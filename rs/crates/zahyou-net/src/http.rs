//! HTTP の共通部分: エージェントの作り方、本文の読み取り、続きから落とす処理。

use crate::NetError;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};
use ureq::Agent;

/// 1 回に読む大きさ。中止はこの単位で見る。
pub(crate) const CHUNK: usize = 1 << 20;

/// 全体の待ち時間を決めたエージェント。`status_err` が偽なら 4xx/5xx も Ok で返す。
pub(crate) fn agent(timeout: Duration, status_err: bool) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(status_err)
        .build()
        .into()
}

/// ダウンロード用のエージェント。全体の上限は付けず、つなぐ・応答を待つ・本文を受ける の段ごとに上限を置く。
/// 本文の上限は 1 回の要求ぶん(区切りの大きさ)にかかる。
pub(crate) fn download_agent(recv_body: Duration) -> Agent {
    Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .timeout_recv_body(Some(recv_body))
        .http_status_as_error(false)
        .build()
        .into()
}

/// ureq の誤りを 1 行の説明にする(Python 版の `type(e).__name__` の代わり)。
pub(crate) fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(c) => format!("HTTP {c}"),
        ureq::Error::Timeout(t) => format!("時間切れ ({t})"),
        ureq::Error::Io(io) => format!("通信の誤り ({io})"),
        ureq::Error::HostNotFound => "宛先が見つかりません".into(),
        other => other.to_string(),
    }
}

/// 本文を文字列で読む(上限 `limit` バイト)。
pub(crate) fn read_text(resp: &mut ureq::http::Response<ureq::Body>, limit: u64) -> Result<String, String> {
    resp.body_mut()
        .with_config()
        .limit(limit)
        .read_to_string()
        .map_err(|e| describe(&e))
}

/// HEAD で大きさ(Content-Length)を聞く。
pub(crate) fn head_size(agent: &Agent, url: &str) -> Result<u64, String> {
    let resp = agent
        .head(url)
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(|e| describe(&e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    resp.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .ok_or_else(|| "Content-Length がありません".to_string())
}

/// 中止を見ながら眠る。
pub(crate) fn sleep_with_stop(d: Duration, stop: &dyn Fn() -> bool) -> bool {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if stop() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50).min(end - Instant::now()));
    }
    !stop()
}

/// 続きから落とす処理の設定。
#[derive(Clone, Debug)]
pub(crate) struct ResumeOpts {
    /// 1 回の要求で頼む大きさ(Range の幅)
    pub segment: u64,
    /// 失敗してよい回数(Python 版の retries=5)
    pub retries: u32,
    /// 待ち時間の単位。n 回目の失敗のあと min(2^n, 20) × これ だけ待つ
    pub retry_unit: Duration,
    /// 1 区切りの本文を受け取る上限
    pub segment_timeout: Duration,
    /// 追加のヘッダー
    pub headers: Vec<(String, String)>,
}

impl Default for ResumeOpts {
    fn default() -> Self {
        ResumeOpts {
            segment: 8 << 20,
            retries: 5,
            retry_unit: Duration::from_secs(1),
            segment_timeout: Duration::from_secs(240),
            headers: Vec::new(),
        }
    }
}

/// 1 回の要求の結果。
enum Step {
    /// 進んだ(続きを頼む)
    Progress,
    /// サーバーが Range に応えない。最初から全体を頼み直す
    NoRange,
}

/// `url` を `part` へ落とす。`part` に途中まであれば Range で続きから。
///
/// `total` が分かっていれば(Some)、`part` がその大きさになったら終わり。分からなければ(None)
/// 全体を 1 回で頼み、終わりまで読む。`account(+n / -n)` で進み具合(バイト)を伝える。
/// 戻り値 Ok は `part` が揃ったこと。
#[allow(clippy::too_many_arguments)]
pub(crate) fn resume_download(
    url: &str,
    part: &Path,
    total: Option<u64>,
    opts: &ResumeOpts,
    account: &mut dyn FnMut(i64),
    stop: &dyn Fn() -> bool,
    log: &dyn Fn(&str),
    name: &str,
) -> Result<(), NetError> {
    let seg_agent = download_agent(opts.segment_timeout);
    let mut range_ok = total.is_some();
    let mut failures = 0u32;
    loop {
        if stop() {
            return Err(NetError::Cancelled);
        }
        let mut have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if let Some(t) = total {
            if have > t {
                // 壊れている(向こうより大きい)
                std::fs::remove_file(part).map_err(|e| NetError::Failed(format!("    {name}: 消せません ({e})")))?;
                account(-(have as i64));
                have = 0;
            }
            if have == t {
                if t == 0 {
                    File::create(part).map_err(|e| NetError::Failed(format!("書き込めません ({e})")))?;
                }
                return Ok(());
            }
        }
        let r = if range_ok {
            request_segment(&seg_agent, url, part, have, total.unwrap_or(0), opts, account, stop)
        } else {
            request_whole(url, part, have, total, opts, account, stop)
        };
        match r {
            Ok(Step::Progress) => {
                if total.is_none() {
                    return Ok(());
                }
            }
            Ok(Step::NoRange) => {
                range_ok = false;
            }
            Err(NetError::Cancelled) => return Err(NetError::Cancelled),
            Err(NetError::Failed(why)) => {
                if stop() {
                    return Err(NetError::Cancelled);
                }
                failures += 1;
                log(&format!("    {name}: {why} ({failures}/{}) 続きから再開します", opts.retries));
                if failures >= opts.retries {
                    log(&format!("    {name}: あきらめました。"));
                    return Err(NetError::Failed(format!("{name}: あきらめました ({why})")));
                }
                let wait = opts.retry_unit.saturating_mul(2u32.saturating_pow(failures).min(20));
                if !sleep_with_stop(wait, stop) {
                    return Err(NetError::Cancelled);
                }
            }
        }
    }
}

/// Range で `have` から 1 区切りぶんを頼む。
#[allow(clippy::too_many_arguments)]
fn request_segment(
    agent: &Agent,
    url: &str,
    part: &Path,
    have: u64,
    total: u64,
    opts: &ResumeOpts,
    account: &mut dyn FnMut(i64),
    stop: &dyn Fn() -> bool,
) -> Result<Step, NetError> {
    // 呼ぶ側は have < total を保証している。区切りが 0 でも 1 バイトずつは進む
    let end = have.saturating_add(opts.segment.max(1)).min(total) - 1;
    let mut req = agent
        .get(url)
        .header("Range", &format!("bytes={have}-{end}"))
        .header("Accept-Encoding", "identity");
    for (k, v) in &opts.headers {
        req = req.header(k, v);
    }
    let mut resp = req.call().map_err(|e| NetError::Failed(describe(&e)))?;
    match resp.status().as_u16() {
        206 => {
            // Content-Range: bytes a-b/c の a が have と合うか
            let start = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().strip_prefix("bytes "))
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.trim().parse::<u64>().ok());
            if start != Some(have) {
                return Err(NetError::Failed(format!("続きの位置が合いません ({start:?} != {have})")));
            }
            let want = end - have + 1;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(part)
                .map_err(|e| NetError::Failed(format!("書き込めません ({e})")))?;
            let got = copy_body(resp.body_mut().as_reader(), file, account, stop)?;
            if got != want {
                return Err(NetError::Failed(format!("途中で切れました ({got} / {want} バイト)")));
            }
            Ok(Step::Progress)
        }
        200 => Ok(Step::NoRange),
        s => Err(NetError::Failed(format!("HTTP {s}"))),
    }
}

/// Range を使わずに全体を頼む(サーバーが Range に応えないとき・大きさが分からないとき)。
fn request_whole(
    url: &str,
    part: &Path,
    have: u64,
    total: Option<u64>,
    opts: &ResumeOpts,
    account: &mut dyn FnMut(i64),
    stop: &dyn Fn() -> bool,
) -> Result<Step, NetError> {
    // 本文の上限は大きさに合わせて伸ばす(20 KB/s を下回ったら打ち切り)
    let budget = match total {
        Some(t) => opts.segment_timeout.max(Duration::from_secs(60 + t / 20_000)),
        None => Duration::from_secs(3600),
    };
    let agent = download_agent(budget);
    let mut req = agent.get(url).header("Accept-Encoding", "identity");
    for (k, v) in &opts.headers {
        req = req.header(k, v);
    }
    let mut resp = req.call().map_err(|e| NetError::Failed(describe(&e)))?;
    let s = resp.status().as_u16();
    if s != 200 {
        return Err(NetError::Failed(format!("HTTP {s}")));
    }
    // 最初から書き直す
    let file = File::create(part).map_err(|e| NetError::Failed(format!("書き込めません ({e})")))?;
    account(-(have as i64));
    let got = copy_body(resp.body_mut().as_reader(), file, account, stop)?;
    if let Some(t) = total
        && got != t
    {
        return Err(NetError::Failed(format!("途中で切れました ({got} / {t} バイト)")));
    }
    Ok(Step::Progress)
}

/// 本文をファイルへ写す。1 MiB ずつ読み、そのたびに中止を見る。
fn copy_body(
    mut reader: impl Read,
    mut file: File,
    account: &mut dyn FnMut(i64),
    stop: &dyn Fn() -> bool,
) -> Result<u64, NetError> {
    let mut buf = vec![0u8; CHUNK];
    let mut got = 0u64;
    loop {
        if stop() {
            let _ = file.flush();
            return Err(NetError::Cancelled);
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = file.flush();
                return Err(NetError::Failed(format!("受信の誤り ({e})")));
            }
        };
        file.write_all(&buf[..n])
            .map_err(|e| NetError::Failed(format!("書き込めません ({e})")))?;
        got += n as u64;
        account(n as i64);
    }
    file.flush().map_err(|e| NetError::Failed(format!("書き込めません ({e})")))?;
    Ok(got)
}

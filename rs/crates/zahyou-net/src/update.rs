//! GitHub の Releases に新しい zahyou.exe があるか調べて、入れ替える(Python の gui/zahyou_update.py)。
//!
//! ```text
//! let st = check_update(exe, APP_VER, &stop);            // 調べるだけ(版名が -dev で終わる試作版は調べない)
//! let new = download_update(rel, exe, &progress, &stop, &log)?;   // zahyou.exe.new へ落として中身を確かめる
//! apply_update(exe, &new, Some(&[]), &log)?;             // 改名して起動し直す(呼んだ側はすぐ閉じる)
//! cleanup_old_versions(exe, ...);                        // 次の起動で zahyou.exe.old を消す
//! ```
//!
//! 見分け方は <b>SHA256 の突き合わせ</b>。GitHub の API は資産ごとに digest (sha256:...) を返すので、
//! 動いている exe のハッシュと比べれば「同じ版を建て直したもの」まで正確に分かる。版名だけでは分からない。
//!
//! 入れ替えは改名方式。Windows では動いている exe を上書きも削除もできないが、<b>改名はできる</b>。
//!   1. 新しいものを zahyou.exe.new として横に置く(大きさと SHA256 を確かめる)
//!   2. 動いている zahyou.exe を zahyou.exe.old へ改名する
//!   3. zahyou.exe.new を zahyou.exe へ改名する(失敗したら .old を戻す)
//!   4. 新しい zahyou.exe を起動し、古いプロセスは閉じる
//!   5. 次の起動で zahyou.exe.old を消す(古いプロセスが閉じるまでは消せない)
//!
//! v6 は PowerShell に任せていたが、PyInstaller の目印の受け渡しで起動に失敗したことがある(#23)。こちらは PowerShell を使わない。

use crate::NetError;
use crate::http::{self, ResumeOpts};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "Akashi3060/zahyou";
pub const API_URL: &str = "https://api.github.com/repos/Akashi3060/zahyou/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/Akashi3060/zahyou/releases/latest";
pub const ASSET: &str = "zahyou.exe";
const USER_AGENT: &str = "zahyou-updater";
const ACCEPT: &str = "application/vnd.github+json";

/// 確認の結果の種類(Python の state)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateState {
    /// 新しい版がある
    Available,
    /// 最新版
    UpToDate,
    /// 試作版(版名が -dev で終わる)なので確かめない
    Dev,
    /// 確かめられなかった
    Error,
    /// 中止した
    Cancelled,
}

/// Releases の最新版の資産。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    /// 公開日(YYYY-MM-DD)
    pub published: String,
    pub url: String,
    pub size: u64,
    /// 資産の SHA256(小文字の 16 進。API が digest を返さないときは空)
    pub sha256: String,
    /// 資産の更新日(YYYY-MM-DD)
    pub updated: String,
}

/// 確認の結果。`message` は画面にそのまま出せる 1 行。
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateCheck {
    pub state: UpdateState,
    pub message: String,
    pub release: Option<ReleaseInfo>,
    pub here_sha256: Option<String>,
    pub here_size: Option<u64>,
    /// 新しい版はあるが、exe の置き場所に書き込めないので入れ替えられない理由
    pub blocked: Option<String>,
}

impl UpdateCheck {
    fn simple(state: UpdateState, message: &str) -> UpdateCheck {
        UpdateCheck { state, message: message.into(), release: None, here_sha256: None, here_size: None, blocked: None }
    }
}

/// ファイルの SHA256(小文字の 16 進)。`stop()` が真になったら None。
pub fn sha256_file(path: &Path, stop: &dyn Fn() -> bool) -> Result<Option<String>, String> {
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 22];
    loop {
        if stop() {
            return Ok(None);
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        h.update(&buf[..n]);
    }
    Ok(Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

/// GitHub の API の応答から資産を取り出す(Python の `latest()`)。
pub fn parse_release(json: &str) -> Result<ReleaseInfo, String> {
    let data: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("応答を読めません ({e})"))?;
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let head10 = |x: String| x.chars().take(10).collect::<String>();
    let asset = data
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|a| a.iter().find(|x| x.get("name").and_then(|n| n.as_str()) == Some(ASSET)))
        .ok_or_else(|| format!("{ASSET} が Releases にありません。"))?;
    let url = asset
        .get("browser_download_url")
        .and_then(|x| x.as_str())
        .ok_or("資産に browser_download_url がありません")?
        .to_string();
    let digest = s(asset, "digest");
    Ok(ReleaseInfo {
        tag: s(&data, "tag_name"),
        name: s(&data, "name"),
        published: head10(s(&data, "published_at")),
        url,
        size: asset.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
        sha256: if digest.is_empty() { String::new() } else { digest.rsplit(':').next().unwrap_or("").to_lowercase() },
        updated: head10(s(asset, "updated_at")),
    })
}

/// Releases の最新版を 1 つ調べる。
pub fn latest_release(api_url: &str, timeout: Duration) -> Result<ReleaseInfo, String> {
    let agent = http::agent(timeout, true);
    let mut resp = agent
        .get(api_url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", ACCEPT)
        .call()
        .map_err(|e| http::describe(&e))?;
    let text = http::read_text(&mut resp, 8 << 20)?;
    parse_release(&text)
}

/// 更新があるか調べる(Python の `check`。待ち時間 20 秒)。
///
/// `version`(画面の APP_VER)が "-dev" で終わる試作版は確かめない。見分けは最新版の資産との SHA256 の
/// 突き合わせなので、試作版の exe はいつも「新しい版があります」になり、押すと最新版へ入れ替わってしまう。
pub fn check_update(current_exe: &Path, version: &str, stop: &dyn Fn() -> bool) -> UpdateCheck {
    check_update_at(API_URL, current_exe, version, Duration::from_secs(20), stop)
}

/// [`check_update`] の問い合わせ先と待ち時間を指定する版(試験用)。
pub fn check_update_at(api_url: &str, current_exe: &Path, version: &str, timeout: Duration, stop: &dyn Fn() -> bool) -> UpdateCheck {
    if version.ends_with("-dev") {
        return UpdateCheck::simple(UpdateState::Dev, "試作版のため更新の確認はしません。");
    }
    let info = match latest_release(api_url, timeout) {
        Ok(i) => i,
        Err(e) => {
            return UpdateCheck::simple(
                UpdateState::Error,
                &format!("更新を確認できませんでした ({e})。インターネットにつながっているか確かめてください。"),
            );
        }
    };
    let here_size = match std::fs::metadata(current_exe) {
        Ok(m) => m.len(),
        Err(e) => {
            return UpdateCheck::simple(UpdateState::Error, &format!("動いている exe を読めません ({}: {e})", current_exe.display()));
        }
    };
    let mut out = UpdateCheck::simple(UpdateState::Error, "");
    out.here_size = Some(here_size);
    let same = if !info.sha256.is_empty() {
        match sha256_file(current_exe, stop) {
            Ok(Some(h)) => {
                let same = h == info.sha256;
                out.here_sha256 = Some(h);
                same
            }
            Ok(None) => return UpdateCheck::simple(UpdateState::Cancelled, "中止しました。"),
            Err(e) => return UpdateCheck::simple(UpdateState::Error, &format!("動いている exe を読めません ({e})")),
        }
    } else {
        // digest が無い API のとき
        here_size == info.size
    };
    if same {
        out.state = UpdateState::UpToDate;
        out.message = format!("最新版です ({} / {} 版)。", info.tag, info.updated);
    } else {
        out.state = UpdateState::Available;
        out.message = format!("新しい版があります: {} ({} 版 / {:.0} MB)", info.tag, info.updated, info.size as f64 / 1e6);
        out.blocked = update_blocked_reason(current_exe);
    }
    out.release = Some(info);
    out
}

/// exe の置き場所に書き込めない(Program Files など)なら、その理由。書き込めるなら None。
pub fn update_blocked_reason(current_exe: &Path) -> Option<String> {
    let dir = current_exe.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let probe = dir.join(".zahyou_write_test");
    let r = std::fs::write(&probe, b"0").and_then(|_| std::fs::remove_file(&probe));
    match r {
        Ok(()) => None,
        Err(e) => Some(format!(
            "{} に書き込めないので、この場所では入れ替えられません ({e})。書ける場所へ移してから、もう一度お試しください。",
            dir.display()
        )),
    }
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// 新しい exe を「zahyou.exe.new」として横へ落とし、大きさと SHA256 を確かめる(Python の `download`)。
///
/// 1 MiB ずつ書き、そのたびに `stop()` を見る。途中で切れたら Range で続きから(5 回まで)。
/// 中止・失敗のときは .new を消す。
pub fn download_update(
    info: &ReleaseInfo,
    current_exe: &Path,
    progress: &dyn Fn(u64, u64),
    stop: &dyn Fn() -> bool,
    log: &dyn Fn(&str),
) -> Result<PathBuf, NetError> {
    download_update_opts(info, current_exe, Duration::from_secs(1), progress, stop, log)
}

/// [`download_update`] の再試行の待ち時間の単位を指定する版(試験用)。
pub fn download_update_opts(
    info: &ReleaseInfo,
    current_exe: &Path,
    retry_unit: Duration,
    progress: &dyn Fn(u64, u64),
    stop: &dyn Fn() -> bool,
    log: &dyn Fn(&str),
) -> Result<PathBuf, NetError> {
    if let Some(why) = update_blocked_reason(current_exe) {
        log(&why);
        return Err(NetError::Failed(why));
    }
    let dest = with_suffix(current_exe, ".new");
    let total = info.size;
    log(&format!("新しい版を落としています ({:.0} MB)...", total as f64 / 1e6));
    // 前に途中で止めたものは使わない(中身の出どころが分からないので最初から)
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(|e| NetError::Failed(format!("{} を消せません ({e})", dest.display())))?;
    }
    let done = std::cell::Cell::new(0i64);
    let mut account = |n: i64| {
        done.set((done.get() + n).max(0));
        progress(done.get() as u64, total);
    };
    let opts = ResumeOpts {
        retry_unit,
        headers: vec![("User-Agent".into(), USER_AGENT.into()), ("Accept".into(), ACCEPT.into())],
        ..ResumeOpts::default()
    };
    let known = if total > 0 { Some(total) } else { None };
    let r = http::resume_download(&info.url, &dest, known, &opts, &mut account, stop, log, ASSET);
    let remove = || {
        let _ = std::fs::remove_file(&dest);
    };
    match r {
        Ok(()) => {}
        Err(NetError::Cancelled) => {
            log("中止しました。");
            remove();
            return Err(NetError::Cancelled);
        }
        Err(NetError::Failed(e)) => {
            log(&format!("落とせませんでした: {e}"));
            remove();
            return Err(NetError::Failed(format!("落とせませんでした: {e}")));
        }
    }
    let got = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if total > 0 && got != total {
        let m = format!("大きさが違います ({got} != {total})。捨てます。");
        log(&m);
        remove();
        return Err(NetError::Failed(m));
    }
    if !info.sha256.is_empty() {
        log("中身を確かめています...");
        let h = match sha256_file(&dest, stop) {
            Ok(Some(h)) => h,
            Ok(None) => {
                log("中止しました。");
                remove();
                return Err(NetError::Cancelled);
            }
            Err(e) => {
                remove();
                return Err(NetError::Failed(e));
            }
        };
        if h != info.sha256 {
            let m = format!("中身が違います。捨てます。\n  こちら {h}\n  向こう {}", info.sha256);
            log(&m);
            remove();
            return Err(NetError::Failed(m));
        }
        log(&format!("  SHA256 一致: {}…", &h[..16]));
    }
    Ok(dest)
}

/// 入れ替えの結果。
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyOutcome {
    /// 退避した古い exe(次の起動で [`cleanup_old_versions`] が消す)
    pub old: PathBuf,
    /// 起動し直した新しいプロセスの番号(起動しなかった・できなかったときは None)
    pub relaunched: Option<u32>,
}

/// 改名方式で入れ替え、`relaunch` が Some なら新しい exe をその引数で起動する(Python の `apply_update`)。
///
/// 設定は呼ぶ前に保存しておくこと。Ok が返ったら、呼んだ側はすぐアプリを閉じる。
/// 新しいものを置けなかったときは、退避した今のものを元の名前に戻す(消えて無くなることはない)。
pub fn apply_update(current_exe: &Path, new_file: &Path, relaunch: Option<&[OsString]>, log: &dyn Fn(&str)) -> Result<ApplyOutcome, NetError> {
    if !new_file.is_file() {
        let m = "入れ替えるものがありません。".to_string();
        log(&m);
        return Err(NetError::Failed(m));
    }
    // 前回の .old が消えずに残っていれば消す。消せなければ別の名前に退避する
    let mut old = with_suffix(current_exe, ".old");
    if old.exists() && std::fs::remove_file(&old).is_err() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        old = with_suffix(current_exe, &format!(".old-{stamp}"));
        let mut k = 1;
        while old.exists() {
            old = with_suffix(current_exe, &format!(".old-{stamp}-{k}"));
            k += 1;
        }
    }
    log("入れ替えて、起動し直します...");
    if let Err(e) = std::fs::rename(current_exe, &old) {
        let m = format!("今の版を退避できません ({e})。");
        log(&m);
        return Err(NetError::Failed(m));
    }
    if let Err(e) = std::fs::rename(new_file, current_exe) {
        // 戻せるなら戻す
        let back = std::fs::rename(&old, current_exe);
        let m = match back {
            Ok(()) => format!("新しい版を置けませんでした ({e})。今の版はそのままです。"),
            Err(e2) => format!("新しい版を置けませんでした ({e})。今の版は {} に残っています ({e2})。", old.display()),
        };
        log(&m);
        return Err(NetError::Failed(m));
    }
    let mut relaunched = None;
    if let Some(args) = relaunch {
        match std::process::Command::new(current_exe).args(args).spawn() {
            Ok(child) => relaunched = Some(child.id()),
            Err(e) => log(&format!("入れ替えましたが、起動し直せませんでした ({e})。手で起動してください。")),
        }
    }
    Ok(ApplyOutcome { old, relaunched })
}

/// 前の版(zahyou.exe.old・zahyou.exe.old-*)を消す。起動の直後に呼ぶ。
///
/// 古いプロセスがまだ閉じきっていないと消せないので、`wait` の間だけ 0.2 秒ごとにやり直す。
/// 戻り値は消せなかったファイル。
pub fn cleanup_old_versions(current_exe: &Path, wait: Duration) -> Vec<PathBuf> {
    let Some(dir) = current_exe.parent() else {
        return Vec::new();
    };
    let Some(base) = current_exe.file_name().map(|s| s.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let prefix = format!("{base}.old").to_lowercase();
    let list = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                        n == prefix || n.starts_with(&format!("{prefix}-"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let end = std::time::Instant::now() + wait;
    loop {
        let left: Vec<PathBuf> = list(dir).into_iter().filter(|p| std::fs::remove_file(p).is_err()).collect();
        if left.is_empty() || std::time::Instant::now() >= end {
            return left;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

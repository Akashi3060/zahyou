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
//!
//! # どの版を探すか(版名 = 画面の APP_VER で決まる)
//!
//! | いまの版 | 問い合わせ先 | 新しい版の見つけ方 |
//! |---|---|---|
//! | `-dev` で終わる(試作版) | 問い合わせない | — |
//! | プレリリース(`v7.0.0-beta.1` など。β版) | Releases の一覧(プレリリースも含む、`?per_page=100`) | 大きな版(major)が同じで、semver で今より新しいもののうち最も新しいもの |
//! | 正式版(`v6`・`v7.0.0` など) | Releases の latest(プレリリースを含まない) | latest の 1 つ |
//!
//! どちらも最後は資産 zahyou.exe の digest(SHA256)と動いている exe を比べる。
//!
//! v7 のβ版は GitHub の<b>プレリリース</b>として出す。v6 の更新係(gui/zahyou_update.py)は latest しか見ないので、
//! v6 の利用者にβ版は出てこない。β版の利用者は一覧から次のβ版・v7 の正式版を受け取る。
//! 一覧では、下書き・`-dev` で終わるタグ・資産 zahyou.exe の無いもの・大きな版の違うもの(v6 など)は使わない。
//! 正式版の latest が今より古い版(semver で比べて)を指していたら、入れ替えに誘わない(古い版へ戻さない)。

use crate::NetError;
use crate::http::{self, ResumeOpts};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "Akashi3060/zahyou";
/// 最新の正式版(プレリリース・下書きを含まない)
pub const API_URL: &str = "https://api.github.com/repos/Akashi3060/zahyou/releases/latest";
/// Releases の一覧(プレリリースを含む。新しい順に 100 件まで)。β版の更新の確認で使う。[`releases_list_url`]`(API_URL)` と同じ
pub const RELEASES_API_URL: &str = "https://api.github.com/repos/Akashi3060/zahyou/releases?per_page=100";
pub const RELEASES_PAGE: &str = "https://github.com/Akashi3060/zahyou/releases/latest";
pub const ASSET: &str = "zahyou.exe";
const USER_AGENT: &str = "zahyou-updater";
const ACCEPT: &str = "application/vnd.github+json";
/// 一覧の応答の上限(100 件ぶんの説明文と資産の表。latest の 1 件は 8 MiB)
const LIST_LIMIT: u64 = 16 << 20;

// ================================================================== 版の比べ方 ===

/// 版(semver 2.0 の形)。先頭の "v" は省ける。"v6"・"v6.2" のように欠けた数は 0 とみなす。
/// ビルドの情報("+…")は読み飛ばし、比べるときにも使わない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// プレリリースの部分を "." で分けたもの("beta.1" なら ["beta", "1"])。空なら正式版
    pub pre: Vec<String>,
}

impl Version {
    /// プレリリース(-beta.1・-rc.1・-dev など)か
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

/// 版名を読む。semver の形でなければ None(壊れた文字列でも panic しない)。
///
/// 読めるもの: "7.0.0-beta.1"・"v7.0.0-beta.1"・"v7.0.0"・"v6.2"・"v6"・"v7-dev"・"7.0.0+build.5"。
/// 読めないもの: 空・数でない部分・数が 4 つ以上・空の区切り("7..0"・"7.0.0-beta..1")・先頭が 0 の数のプレリリース("-01")・
/// 英数字と "-" 以外の文字(全角の数字や「β」)。
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let s = s.split_once('+').map_or(s, |(a, _)| a);
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (s, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut nums = [0u64; 3];
    for (n, p) in nums.iter_mut().zip(&parts) {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *n = p.parse().ok()?;
    }
    let pre = match pre {
        None => Vec::new(),
        Some(p) => {
            let ids: Vec<String> = p.split('.').map(str::to_string).collect();
            let bad = |x: &String| {
                x.is_empty()
                    || !x.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || (x.len() > 1 && x.starts_with('0') && x.bytes().all(|b| b.is_ascii_digit()))
            };
            if ids.iter().any(bad) {
                return None;
            }
            ids
        }
    };
    Some(Version { major: nums[0], minor: nums[1], patch: nums[2], pre })
}

/// プレリリースの 1 区切りを比べる(数だけのものは数として、数は英字を含むものより前)
fn cmp_pre_id(a: &str, b: &str) -> Ordering {
    let num = |x: &str| x.bytes().all(|c| c.is_ascii_digit());
    match (num(a), num(b)) {
        // 先頭の 0 は読むときに拒んでいるので、桁数・文字の順で数の大小になる(u64 を超える数でも)
        (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
    }
}

impl Ord for Version {
    /// semver 2.0 の順: 7.0.0-alpha < 7.0.0-alpha.1 < 7.0.0-beta.1 < 7.0.0-beta.2 < 7.0.0-beta.11 < 7.0.0-rc.1 < 7.0.0 < 7.0.1
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // 正式版はプレリリースより新しい
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self
                    .pre
                    .iter()
                    .zip(&other.pre)
                    .map(|(a, b)| cmp_pre_id(a, b))
                    .find(|o| *o != Ordering::Equal)
                    .unwrap_or_else(|| self.pre.len().cmp(&other.pre.len())),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 試作版(版名が "-dev" で終わる)か。試作版は更新の確認をしない
pub fn is_dev_version(version: &str) -> bool {
    version.trim().ends_with("-dev")
}

/// 版名に添える言葉。β版(-dev 以外のプレリリース。-beta.N・-rc.N など)なら "β版"、正式版・試作版・読めない版名は None。
/// 画面の題や［使い方］の「いまの版」に「v7.0.0-beta.1 (β版)」のように添えるためのもの。
pub fn prerelease_label(version: &str) -> Option<&'static str> {
    if is_dev_version(version) {
        return None;
    }
    parse_version(version).filter(Version::is_prerelease).map(|_| "β版")
}

/// latest の URL(…/releases/latest)から一覧の URL(…/releases?per_page=100)を作る。
/// 末尾が /latest でなければ、そのまま per_page を足す(試験の偽の GitHub でも同じ形で問い合わせる)。
pub fn releases_list_url(latest_url: &str) -> String {
    let base = latest_url.strip_suffix("/latest").unwrap_or(latest_url);
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}per_page=100")
}

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
    release_from_value(&data)
}

/// Releases の 1 件(JSON の値)から資産 zahyou.exe を取り出す。
fn release_from_value(data: &serde_json::Value) -> Result<ReleaseInfo, String> {
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
        tag: s(data, "tag_name"),
        name: s(data, "name"),
        published: head10(s(data, "published_at")),
        url,
        size: asset.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
        sha256: if digest.is_empty() { String::new() } else { digest.rsplit(':').next().unwrap_or("").to_lowercase() },
        updated: head10(s(asset, "updated_at")),
    })
}

/// GitHub の API に GET して本文を読む。
fn get_text(url: &str, timeout: Duration, limit: u64) -> Result<String, String> {
    let agent = http::agent(timeout, true);
    let mut resp = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", ACCEPT)
        .call()
        .map_err(|e| http::describe(&e))?;
    http::read_text(&mut resp, limit)
}

/// Releases の最新版を 1 つ調べる。
pub fn latest_release(api_url: &str, timeout: Duration) -> Result<ReleaseInfo, String> {
    parse_release(&get_text(api_url, timeout, 8 << 20)?)
}

/// Releases の一覧から、β版の更新に使うものを選んだ結果([`select_prerelease_update`])。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrereleaseChoice {
    /// 今より新しい版のうち、最も新しいもの(資産 zahyou.exe があるものだけ)
    pub newer: Option<ReleaseInfo>,
    /// 今と同じ版のもの(あれば。最新版の文に公開日を出すため)
    pub same: Option<ReleaseInfo>,
    /// 今より新しく、`newer` よりも新しいのに資産 zahyou.exe がまだ無いもののタグ(資産を上げている途中など)
    pub pending: Option<String>,
}

/// Releases の一覧(GitHub の API の応答。プレリリースを含む)から、`current` と大きな版(major)が同じで、
/// semver で今より新しいもののうち最も新しいものを選ぶ。
///
/// 使わないもの: 下書き(draft)・タグを版として読めないもの・`-dev` で終わるタグ・大きな版の違うもの(v6 など)。
/// プレリリースかどうか(prerelease)は問わない(β版の利用者は次のβ版も正式版も受け取る)。
/// 一覧として読めない応答だけを誤りにする(1 件ずつの壊れは、その 1 件を使わないだけ)。
pub fn select_prerelease_update(json: &str, current: &Version) -> Result<PrereleaseChoice, String> {
    let data: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("応答を読めません ({e})"))?;
    let list = data.as_array().ok_or("応答を読めません (Releases の一覧ではありません)")?;
    let mut out = PrereleaseChoice::default();
    let mut best: Option<Version> = None;
    let mut pending: Option<Version> = None;
    for rel in list {
        if rel.get("draft").and_then(|d| d.as_bool()).unwrap_or(false) {
            continue;
        }
        let tag = rel.get("tag_name").and_then(|t| t.as_str()).unwrap_or("");
        if is_dev_version(tag) {
            continue;
        }
        let Some(v) = parse_version(tag) else { continue };
        if v.major != current.major {
            continue;
        }
        match v.cmp(current) {
            Ordering::Less => {}
            Ordering::Equal => {
                if out.same.is_none() {
                    out.same = release_from_value(rel).ok();
                }
            }
            Ordering::Greater => match release_from_value(rel) {
                Ok(info) => {
                    if best.as_ref().is_none_or(|b| v > *b) {
                        best = Some(v);
                        out.newer = Some(info);
                    }
                }
                Err(_) => {
                    if pending.as_ref().is_none_or(|p| v > *p) {
                        pending = Some(v);
                        out.pending = Some(tag.to_string());
                    }
                }
            },
        }
    }
    // 資産のあるものより古い「準備中」は知らせない
    if let (Some(b), Some(p)) = (&best, &pending)
        && p < b
    {
        out.pending = None;
    }
    Ok(out)
}

/// 更新があるか調べる(Python の `check`。待ち時間 20 秒)。
///
/// `version`(画面の APP_VER)が "-dev" で終わる試作版は確かめない。見分けは最新版の資産との SHA256 の
/// 突き合わせなので、試作版の exe はいつも「新しい版があります」になり、押すと最新版へ入れ替わってしまう。
/// β版(プレリリースの版名)は、プレリリースも含めた一覧から同じ大きな版の新しいものを探す(モジュールの説明)。
pub fn check_update(current_exe: &Path, version: &str, stop: &dyn Fn() -> bool) -> UpdateCheck {
    check_update_at(API_URL, current_exe, version, Duration::from_secs(20), stop)
}

/// [`check_update`] の問い合わせ先と待ち時間を指定する版(試験用)。
///
/// `api_url` は latest の URL。β版の一覧は [`releases_list_url`] でそこから作る。
pub fn check_update_at(api_url: &str, current_exe: &Path, version: &str, timeout: Duration, stop: &dyn Fn() -> bool) -> UpdateCheck {
    if is_dev_version(version) {
        return UpdateCheck::simple(UpdateState::Dev, "試作版のため更新の確認はしません。");
    }
    let current = parse_version(version);
    if let Some(cur) = current.as_ref().filter(|v| v.is_prerelease()) {
        return check_prerelease(&releases_list_url(api_url), current_exe, version, cur, timeout, stop);
    }
    let info = match latest_release(api_url, timeout) {
        Ok(i) => i,
        Err(e) => return cannot_check(&e),
    };
    // latest が今より古い版なら(v7 の正式版を使っているときに v6 の修正版が latest になった、など)、戻さない
    if let (Some(cur), Some(tag)) = (&current, parse_version(&info.tag))
        && tag < *cur
    {
        let mut out = UpdateCheck::simple(
            UpdateState::UpToDate,
            &format!("最新版です ({version})。公開されている最新版 {} より新しい版を使っています。", info.tag),
        );
        out.release = Some(info);
        return out;
    }
    compare_with_release(info, current_exe, stop)
}

/// 問い合わせに失敗したときの結果
fn cannot_check(e: &str) -> UpdateCheck {
    UpdateCheck::simple(
        UpdateState::Error,
        &format!("更新を確認できませんでした ({e})。インターネットにつながっているか確かめてください。"),
    )
}

/// β版の更新の確認: 一覧から今より新しい版を選び、あればその資産と動いている exe を比べる。
fn check_prerelease(
    list_url: &str,
    current_exe: &Path,
    version: &str,
    current: &Version,
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> UpdateCheck {
    let choice = match get_text(list_url, timeout, LIST_LIMIT).and_then(|t| select_prerelease_update(&t, current)) {
        Ok(c) => c,
        Err(e) => return cannot_check(&e),
    };
    if let Some(info) = choice.newer {
        return compare_with_release(info, current_exe, stop);
    }
    let mut message = match &choice.same {
        Some(r) => format!("最新版です ({} / {} 版)。", r.tag, r.updated),
        None => format!("最新版です ({version})。"),
    };
    if let Some(p) = &choice.pending {
        message.push_str(&format!(" {p} は公開の準備中です ({ASSET} がまだありません)。"));
    }
    let mut out = UpdateCheck::simple(UpdateState::UpToDate, &message);
    out.release = choice.same;
    out
}

/// 新しい版の候補(`info`)の資産と動いている exe を比べる(digest があれば SHA256、無ければ大きさ)。
fn compare_with_release(info: ReleaseInfo, current_exe: &Path, stop: &dyn Fn() -> bool) -> UpdateCheck {
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

// ================================================================== 試験 ===

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        parse_version(s).unwrap_or_else(|| panic!("読めない: {s}"))
    }

    #[test]
    fn parse_versions() {
        assert_eq!(v("7.0.0-beta.1"), Version { major: 7, minor: 0, patch: 0, pre: vec!["beta".into(), "1".into()] });
        assert_eq!(v("v7.0.0-beta.1"), v("7.0.0-beta.1"));
        assert_eq!(v("V7.0.0"), Version { major: 7, minor: 0, patch: 0, pre: vec![] });
        assert_eq!(v("v6"), Version { major: 6, minor: 0, patch: 0, pre: vec![] });
        assert_eq!(v("v6.2"), Version { major: 6, minor: 2, patch: 0, pre: vec![] });
        assert_eq!(v("v7-dev").pre, vec!["dev".to_string()]);
        assert_eq!(v(" 7.0.0+build.5 "), v("7.0.0"), "ビルドの情報は読み飛ばす");
        assert_eq!(v("7.0.0-x-y.1").pre, vec!["x-y".to_string(), "1".to_string()]);
        assert!(v("7.0.0-beta.1").is_prerelease());
        assert!(!v("7.0.0").is_prerelease());
        for bad in [
            "", "v", "-", "-beta", "7.", ".7", "7..0", "7.0.0.0", "7.0.0-", "7.0.0-beta..1", "7.0.0-beta.", "7.0.0-01", "abc", "v7.x",
            "７.0.0", "v7.0.0-β", "7.0.0-beta 1", "99999999999999999999.0.0", "+1", "v+",
        ] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        // 先頭が 0 の数でも、プレリリースの "0" そのものと英字混じりは読める
        assert_eq!(v("7.0.0-0").pre, vec!["0".to_string()]);
        assert_eq!(v("7.0.0-0a").pre, vec!["0a".to_string()]);
    }

    #[test]
    fn semver_order() {
        // semver 2.0 の 11 節の例と、zahyou のβ版の並び
        let chain = [
            "6", "6.2", "7.0.0-alpha", "7.0.0-alpha.1", "7.0.0-alpha.beta", "7.0.0-beta", "7.0.0-beta.1", "7.0.0-beta.2",
            "7.0.0-beta.10", "7.0.0-beta.11", "7.0.0-rc.1", "7.0.0", "7.0.1", "7.1.0-beta.1", "7.1.0", "8.0.0-beta.1",
        ];
        for w in chain.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
            assert!(v(w[1]) > v(w[0]), "{} > {}", w[1], w[0]);
        }
        for a in chain {
            assert_eq!(v(a).cmp(&v(a)), Ordering::Equal);
        }
        assert_eq!(v("v7.0.0-beta.1").cmp(&v("7.0.0-beta.1+abc")), Ordering::Equal);
        // u64 を超える数の区切りでも数として比べる
        assert!(v("7.0.0-beta.99999999999999999999") > v("7.0.0-beta.9"));
        assert!(v("7.0.0-beta.9") < v("7.0.0-beta.a"), "数は英字を含むものより前");
    }

    #[test]
    fn labels_and_urls() {
        assert_eq!(prerelease_label("v7.0.0-beta.1"), Some("β版"));
        assert_eq!(prerelease_label("v7.0.0-rc.1"), Some("β版"));
        assert_eq!(prerelease_label("v7.0.0"), None);
        assert_eq!(prerelease_label("v6"), None);
        assert_eq!(prerelease_label("v7-dev"), None);
        assert_eq!(prerelease_label("v7.0.0-dev"), None);
        assert_eq!(prerelease_label("おかしな版"), None);
        assert!(is_dev_version("v7-dev") && is_dev_version("v7.0.0-dev") && !is_dev_version("v7.0.0-beta.1"));
        assert_eq!(releases_list_url(API_URL), RELEASES_API_URL);
        assert_eq!(releases_list_url("http://127.0.0.1:1/repos/x/releases/latest"), "http://127.0.0.1:1/repos/x/releases?per_page=100");
        assert_eq!(releases_list_url("http://h/r?a=1"), "http://h/r?a=1&per_page=100");
    }

    /// 壊れた一覧・おかしな項目でも panic せず、一覧として読めないときだけ誤り
    #[test]
    fn select_from_broken_lists() {
        let cur = v("7.0.0-beta.1");
        assert!(select_prerelease_update("not json", &cur).is_err());
        assert!(select_prerelease_update("{\"tag_name\": \"v7.0.0\"}", &cur).unwrap_err().contains("一覧ではありません"));
        assert_eq!(select_prerelease_update("[]", &cur).unwrap(), PrereleaseChoice::default());
        let odd = r#"[1, null, "x", {}, {"tag_name": 7}, {"tag_name": "v7.0.0", "assets": "x"},
                      {"tag_name": "v7.0.0-beta.2", "draft": "yes", "assets": [{"name": "zahyou.exe"}]},
                      {"tag_name": "v7.0.0-beta.3", "assets": [{"name": "zahyou.exe", "browser_download_url": 5}]}]"#;
        let c = select_prerelease_update(odd, &cur).unwrap();
        assert_eq!(c.newer, None);
        // 資産の表が壊れているものは「準備中」(新しいもののうち最も新しいタグ)
        assert_eq!(c.pending.as_deref(), Some("v7.0.0"));
    }
}

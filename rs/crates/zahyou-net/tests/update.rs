//! 更新の確認・ダウンロード・入れ替えの試験(localhost のサーバーと、一時フォルダーの偽の exe)。
mod common;

use common::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zahyou_net::update::*;
use zahyou_net::NetError;

fn sha_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn release_json(url: &str, size: usize, digest: Option<&str>) -> String {
    let d = digest.map(|d| format!(", \"digest\": \"sha256:{d}\"")).unwrap_or_default();
    format!(
        "{{\"tag_name\": \"v6.2\", \"name\": \"zahyou v6.2\", \"published_at\": \"2026-09-30T12:34:56Z\", \"assets\": [\
         {{\"name\": \"zahyou.exe\", \"size\": {size}, \"browser_download_url\": \"{url}\", \"updated_at\": \"2026-09-30T13:00:00Z\"{d}}}]}}"
    )
}

#[test]
fn check_states_and_messages() {
    let dir = temp_dir("upd-check");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"fake exe").unwrap();
    let here = sha_hex(b"fake exe");
    let py = python_values();
    if let Some(py) = &py {
        assert_eq!(here, py["update"]["messages"]["fake_exe_sha256"].as_str().unwrap());
    }
    let body = Arc::new(Mutex::new(String::new()));
    let b = body.clone();
    let srv = TestServer::start(move |r| {
        assert_eq!(r.header("user-agent"), Some("zahyou-updater"));
        assert_eq!(r.header("accept"), Some("application/vnd.github+json"));
        Resp::json(&b.lock().unwrap())
    });
    let api = format!("{}/repos/x/releases/latest", srv.url());
    let check = |ver: &str| check_update_at(&api, &exe, ver, Duration::from_secs(5), &|| false);

    // 新しい版がある(Python の文言と同じ)
    *body.lock().unwrap() = release_json("https://x/zahyou.exe", 107288075, Some("5F771F339432588395171ACD2FD8EC8C7C94080419A38B0D703594C5238E3605"));
    let st = check("v6");
    assert_eq!(st.state, UpdateState::Available);
    assert_eq!(st.here_sha256.as_deref(), Some(here.as_str()));
    assert_eq!(st.blocked, None);
    if let Some(py) = &py {
        assert_eq!(st.message, py["update"]["messages"]["available"].as_str().unwrap());
    }
    // 同じ中身 → 最新版
    *body.lock().unwrap() = release_json("https://x/zahyou.exe", 8, Some(&here));
    let st = check("v6");
    assert_eq!(st.state, UpdateState::UpToDate, "{}", st.message);
    if let Some(py) = &py {
        assert_eq!(st.message, py["update"]["messages"]["up_to_date"].as_str().unwrap());
    }
    // digest が無いときは大きさで比べる
    *body.lock().unwrap() = release_json("https://x/zahyou.exe", 8, None);
    assert_eq!(check("v6").state, UpdateState::UpToDate);
    *body.lock().unwrap() = release_json("https://x/zahyou.exe", 9, None);
    assert_eq!(check("v6").state, UpdateState::Available);
    // 試作版は確かめない(問い合わせもしない)
    srv.clear();
    let st = check("v7-dev");
    assert_eq!(st.state, UpdateState::Dev);
    assert!(srv.requests().is_empty());
    if let Some(py) = &py {
        assert_eq!(st.message, py["update"]["messages"]["dev"].as_str().unwrap());
    }
    // 資産が無い・つながらない → Error
    *body.lock().unwrap() = "{\"assets\": []}".into();
    let st = check("v6");
    assert_eq!(st.state, UpdateState::Error);
    assert!(st.message.contains("zahyou.exe が Releases にありません"), "{}", st.message);
    let st = check_update_at("http://127.0.0.1:9/none", &exe, "v6", Duration::from_secs(3), &|| false);
    assert_eq!(st.state, UpdateState::Error);
    assert!(st.message.starts_with("更新を確認できませんでした"));
    // 中止
    *body.lock().unwrap() = release_json("https://x/zahyou.exe", 8, Some(&here));
    let st = check_update_at(&api, &exe, "v6", Duration::from_secs(5), &|| true);
    assert_eq!(st.state, UpdateState::Cancelled);
    // 書き込めない置き場所(親フォルダーが無い)
    assert!(update_blocked_reason(&dir.join("no-such-dir").join("zahyou.exe")).is_some());
    assert!(update_blocked_reason(&exe).is_none());
}

#[test]
fn download_checks_size_and_sha() {
    // 3.5 MiB の偽の exe(1 MiB ずつ書く・区切り 8 MiB の中で 1 回切れる)
    let data: Arc<Vec<u8>> = Arc::new((0..3_500_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect());
    let cuts = Counter::default();
    let (d, c) = (data.clone(), cuts.clone());
    let srv = TestServer::start(move |r| {
        if r.path != "/dl/zahyou.exe" {
            return Resp::new(404, "no");
        }
        let resp = serve_bytes(r, &d, true);
        if r.method == "GET" && c.bump() == 1 {
            return resp.cut(1_200_000);
        }
        resp
    });
    let dir = temp_dir("upd-dl");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"old").unwrap();
    let info = ReleaseInfo {
        tag: "v9".into(),
        url: format!("{}/dl/zahyou.exe", srv.url()),
        size: data.len() as u64,
        sha256: sha_hex(&data),
        ..ReleaseInfo::default()
    };
    let log = Mutex::new(Vec::<String>::new());
    let prog = Mutex::new((0u64, 0u64));
    let lg = |m: &str| log.lock().unwrap().push(m.to_string());
    let new = download_update_opts(&info, &exe, Duration::from_millis(1), &|a, b| *prog.lock().unwrap() = (a, b), &|| false, &lg).unwrap();
    assert_eq!(new, dir.join("zahyou.exe.new"));
    assert_eq!(std::fs::read(&new).unwrap(), *data);
    assert_eq!(*prog.lock().unwrap(), (data.len() as u64, data.len() as u64));
    let reqs = srv.requests();
    assert_eq!(reqs[1].header("range"), Some("bytes=1200000-3499999"), "切れたところから続ける");
    assert_eq!(reqs[0].header("user-agent"), Some("zahyou-updater"));
    let l = log.lock().unwrap().clone();
    assert!(l.iter().any(|m| m.starts_with("  SHA256 一致: ")), "{l:?}");

    // SHA256 が違う → 捨てる
    let bad = ReleaseInfo { sha256: "00".repeat(32), ..info.clone() };
    let r = download_update_opts(&bad, &exe, Duration::from_millis(1), &|_, _| {}, &|| false, &lg);
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m.starts_with("中身が違います")), "{r:?}");
    assert!(!dir.join("zahyou.exe.new").exists());
    // 大きさが違う(向こうの表示より実物が小さい)→ あきらめる・捨てる
    let big = ReleaseInfo { size: data.len() as u64 + 10, ..info.clone() };
    let r = download_update_opts(&big, &exe, Duration::from_millis(1), &|_, _| {}, &|| false, &lg);
    assert!(r.is_err(), "{r:?}");
    assert!(!dir.join("zahyou.exe.new").exists());
    // 中止 → 捨てる
    let r = download_update_opts(&info, &exe, Duration::from_millis(1), &|_, _| {}, &|| true, &lg);
    assert_eq!(r, Err(NetError::Cancelled));
    assert!(!dir.join("zahyou.exe.new").exists());
    // 書き込めない場所
    let r = download_update_opts(&info, &dir.join("nodir").join("zahyou.exe"), Duration::from_millis(1), &|_, _| {}, &|| false, &lg);
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m.contains("書き込めない")), "{r:?}");
}

fn system_exe(name: &str) -> Option<PathBuf> {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let p = root.join("System32").join(name);
    p.exists().then_some(p)
}

/// 動いている exe を改名して入れ替え、起動し直す。偽の exe は Windows の ping.exe と cmd.exe の写し。
#[test]
fn apply_swaps_running_exe_and_relaunches() {
    let (Some(ping), Some(cmd)) = (system_exe("PING.EXE"), system_exe("cmd.exe")) else {
        eprintln!("SKIP: Windows の ping.exe / cmd.exe がありません");
        return;
    };
    let dir = temp_dir("upd-apply");
    let exe = dir.join("zahyou.exe");
    let new = dir.join("zahyou.exe.new");
    std::fs::copy(&ping, &exe).unwrap();
    std::fs::copy(&cmd, &new).unwrap();
    // 「今の版」を動かしておく(約 3 秒)
    let mut running = Command::new(&exe)
        .args(["-n", "4", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // 動いている exe は消せない(前提の確認)
    assert!(std::fs::remove_file(&exe).is_err(), "動いている exe を消せてしまった");
    let log = Mutex::new(Vec::<String>::new());
    let args: Vec<OsString> = vec!["/c".into(), "exit".into(), "0".into()];
    let out = apply_update(&exe, &new, Some(&args), &|m| log.lock().unwrap().push(m.into())).unwrap();
    assert_eq!(out.old, dir.join("zahyou.exe.old"));
    assert!(out.relaunched.is_some(), "起動し直せていない: {:?}", log.lock().unwrap());
    assert_eq!(std::fs::read(&exe).unwrap(), std::fs::read(&cmd).unwrap(), "新しい版が本名になっている");
    assert_eq!(std::fs::read(&out.old).unwrap(), std::fs::read(&ping).unwrap(), "今の版は .old に残る");
    assert!(!new.exists());
    // 古いプロセスが動いている間は .old を消せない → 終わったら消える
    let left = cleanup_old_versions(&exe, Duration::from_millis(100));
    assert_eq!(left, vec![out.old.clone()], "動いている間は消えない");
    running.wait().unwrap();
    let left = cleanup_old_versions(&exe, Duration::from_secs(5));
    assert!(left.is_empty(), "{left:?}");
    assert!(!out.old.exists());

    // 入れ替えるものが無い
    let r = apply_update(&exe, &dir.join("none.new"), None, &|_| {});
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m == "入れ替えるものがありません。"));
}

#[test]
fn apply_without_relaunch_and_leftover_old() {
    let dir = temp_dir("upd-apply2");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"v1").unwrap();
    std::fs::write(dir.join("zahyou.exe.new"), b"v2").unwrap();
    // 前回の .old が残っている(消せる)
    std::fs::write(dir.join("zahyou.exe.old"), b"v0").unwrap();
    let out = apply_update(&exe, &dir.join("zahyou.exe.new"), None, &|_| {}).unwrap();
    assert_eq!(out.relaunched, None);
    assert_eq!(std::fs::read(&exe).unwrap(), b"v2");
    assert_eq!(std::fs::read(&out.old).unwrap(), b"v1");
    std::fs::write(dir.join("zahyou.exe.old-123"), b"x").unwrap();
    std::fs::write(dir.join("zahyou.exe.older"), b"keep").unwrap();
    assert!(cleanup_old_versions(&exe, Duration::ZERO).is_empty());
    assert!(!dir.join("zahyou.exe.old").exists() && !dir.join("zahyou.exe.old-123").exists());
    assert!(dir.join("zahyou.exe.older").exists(), "関係ないファイルは消さない");
    let _ = Path::new("");
}

/// GitHub の資産の URL は別のホストへ 302 で飛ばす。飛んだ先にも Range と User-Agent が届き、続きから落とせること。
#[test]
fn download_follows_redirect_with_range() {
    let data: Arc<Vec<u8>> = Arc::new((0..2_600_000u32).map(|i| (i.wrapping_mul(40503) >> 7) as u8).collect());
    let cuts = Counter::default();
    let (d, c) = (data.clone(), cuts.clone());
    let srv = TestServer::start(move |r| {
        if r.path == "/releases/download/v9/zahyou.exe" {
            let mut resp = Resp::new(302, Vec::new());
            resp.headers.push(("Location".into(), "/objects/abc?sig=1".into()));
            return resp;
        }
        if r.path != "/objects/abc?sig=1" {
            return Resp::new(404, "no");
        }
        let resp = serve_bytes(r, &d, true);
        if c.bump() == 1 {
            return resp.cut(700_000);
        }
        resp
    });
    let dir = temp_dir("upd-redirect");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"old").unwrap();
    let info = ReleaseInfo {
        url: format!("{}/releases/download/v9/zahyou.exe", srv.url()),
        size: data.len() as u64,
        sha256: sha_hex(&data),
        ..ReleaseInfo::default()
    };
    let new = download_update_opts(&info, &exe, Duration::from_millis(1), &|_, _| {}, &|| false, &|m| eprintln!("[log] {m}")).unwrap();
    assert_eq!(std::fs::read(&new).unwrap(), *data);
    let got: Vec<(String, Option<String>, Option<String>)> = srv
        .requests()
        .iter()
        .map(|q| (q.path.clone(), q.header("range").map(String::from), q.header("user-agent").map(String::from)))
        .collect();
    for g in &got {
        eprintln!("{g:?}");
    }
    let objs: Vec<_> = got.iter().filter(|g| g.0.starts_with("/objects/")).collect();
    assert_eq!(objs[0].1.as_deref(), Some("bytes=0-2599999"), "飛んだ先にも Range が届く");
    assert_eq!(objs[1].1.as_deref(), Some("bytes=700000-2599999"), "切れたところから");
    assert!(objs.iter().all(|g| g.2.as_deref() == Some("zahyou-updater")));
}

// ------------------------------------------------------------- β版の更新の確認 ---

/// Releases の 1 件(一覧の項目)。`asset` が None なら資産 zahyou.exe が無い。
fn listed(tag: &str, prerelease: bool, draft: bool, asset: Option<(usize, &str)>) -> String {
    let assets = match asset {
        Some((size, digest)) => format!(
            "[{{\"name\": \"zahyou.exe\", \"size\": {size}, \"browser_download_url\": \"https://x/{tag}/zahyou.exe\", \
             \"updated_at\": \"2026-11-01T00:00:00Z\", \"digest\": \"sha256:{digest}\"}}]"
        ),
        None => "[{\"name\": \"notes.txt\", \"size\": 1, \"browser_download_url\": \"https://x/notes.txt\"}]".into(),
    };
    format!(
        "{{\"tag_name\": \"{tag}\", \"name\": \"zahyou {tag}\", \"draft\": {draft}, \"prerelease\": {prerelease}, \
         \"published_at\": \"2026-11-01T00:00:00Z\", \"assets\": {assets}}}"
    )
}

/// 偽の GitHub: latest(プレリリースを含まない)と一覧(プレリリースを含む)を別々に返す。
/// latest には、GitHub の説明のとおり一覧(新しい順)のうちプレリリースでも下書きでもない最初のものを返す。
struct FakeGitHub {
    srv: TestServer,
    list: Arc<Mutex<Vec<String>>>,
}

impl FakeGitHub {
    fn start() -> FakeGitHub {
        let list: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let l = list.clone();
        let srv = TestServer::start(move |r| {
            assert_eq!(r.header("user-agent"), Some("zahyou-updater"));
            assert_eq!(r.header("accept"), Some("application/vnd.github+json"));
            let items = l.lock().unwrap().clone();
            match r.path.as_str() {
                LIST_PATH => Resp::json(&format!("[{}]", items.join(","))),
                LATEST_PATH => {
                    let full = items.iter().find(|j| j.contains("\"prerelease\": false") && j.contains("\"draft\": false"));
                    match full {
                        Some(j) => Resp::json(j),
                        None => Resp::new(404, "{\"message\": \"Not Found\"}"),
                    }
                }
                _ => Resp::new(404, "{\"message\": \"Not Found\"}"),
            }
        });
        FakeGitHub { srv, list }
    }
    fn set(&self, items: &[String]) {
        *self.list.lock().unwrap() = items.to_vec();
        self.srv.clear();
    }
    fn latest_url(&self) -> String {
        format!("{}{LATEST_PATH}", self.srv.url())
    }
    fn paths(&self) -> Vec<String> {
        self.srv.requests().iter().map(|r| r.path.clone()).collect()
    }
}

const LIST_PATH: &str = "/repos/x/releases?per_page=100";
const LATEST_PATH: &str = "/repos/x/releases/latest";

#[test]
fn beta_checks_prereleases_in_the_list() {
    let dir = temp_dir("upd-beta");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"fake beta exe").unwrap();
    let here = sha_hex(b"fake beta exe");
    let other = sha_hex(b"another build");
    let gh = FakeGitHub::start();
    let api = gh.latest_url();
    let check = |ver: &str| check_update_at(&api, &exe, ver, Duration::from_secs(5), &|| false);
    let beta1 = "v7.0.0-beta.1";

    // 1. プレリリースだけ: 次のβ版がある → 新しい版あり(一覧だけを問い合わせ、latest は見ない)
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((14_000_000, &other))), listed(beta1, true, false, Some((13_900_000, &here)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::Available, "{}", st.message);
    assert_eq!(st.release.as_ref().unwrap().tag, "v7.0.0-beta.2");
    assert_eq!(st.release.as_ref().unwrap().url, "https://x/v7.0.0-beta.2/zahyou.exe");
    assert_eq!(st.release.as_ref().unwrap().sha256, other);
    assert_eq!(st.here_sha256.as_deref(), Some(here.as_str()));
    assert_eq!(st.message, "新しい版があります: v7.0.0-beta.2 (2026-11-01 版 / 14 MB)");
    assert_eq!(gh.paths(), vec![LIST_PATH.to_string()]);

    // 2. 正式版だけ: v7 の正式版はβ版より新しい → 新しい版あり
    gh.set(&[listed("v7.0.0", false, false, Some((14_100_000, &other)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::Available, "{}", st.message);
    assert_eq!(st.release.unwrap().tag, "v7.0.0");

    // 3. 両方(新しい順に並ぶが、いちばん上が最も新しい版とは限らないので semver で選ぶ)
    gh.set(&[
        listed("v7.0.0-beta.3", true, false, Some((1, &other))),
        listed("v6.3", false, false, Some((107_000_000, &other))),
        listed("v7.0.0", false, false, Some((2, &other))),
        listed("v7.0.0-beta.2", true, false, Some((3, &other))),
        listed(beta1, true, false, Some((4, &here))),
    ]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::Available);
    assert_eq!(st.release.unwrap().tag, "v7.0.0", "beta.3 より正式版 v7.0.0 が新しい");
    // beta.10 は beta.2 より新しい(文字の順ではない)
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((1, &other))), listed("v7.0.0-beta.10", true, false, Some((2, &other)))]);
    assert_eq!(check(beta1).release.unwrap().tag, "v7.0.0-beta.10");
    // 正式版のあとのβ版(v7.1.0-beta.1)も、β版の利用者には出す
    gh.set(&[listed("v7.1.0-beta.1", true, false, Some((1, &other))), listed("v7.0.0", false, false, Some((2, &other)))]);
    assert_eq!(check(beta1).release.unwrap().tag, "v7.1.0-beta.1");

    // 4. v6 のタグだけ → 新しい版なし(exe のハッシュも計算しない)
    gh.set(&[listed("v6.3", false, false, Some((107_000_000, &other))), listed("v6.2", false, false, Some((107_000_000, &other)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::UpToDate, "{}", st.message);
    assert_eq!(st.message, "最新版です (v7.0.0-beta.1)。");
    assert_eq!(st.release, None);
    assert_eq!(st.here_sha256, None);
    assert_eq!(gh.paths(), vec![LIST_PATH.to_string()]);
    // 一覧が空でも同じ
    gh.set(&[]);
    assert_eq!(check(beta1).state, UpdateState::UpToDate);

    // 5. 今と同じ版だけ → 最新版(公開日つき)。古いβ版・α版も新しい版ではない
    gh.set(&[listed(beta1, true, false, Some((4, &here))), listed("v7.0.0-alpha.3", true, false, Some((5, &other)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::UpToDate);
    assert_eq!(st.message, "最新版です (v7.0.0-beta.1 / 2026-11-01 版)。");
    assert_eq!(st.release.unwrap().tag, beta1);
    gh.set(&[listed(beta1, true, false, Some((4, &other)))]);
    assert_eq!(check("v7.0.0-beta.2").state, UpdateState::UpToDate, "beta.2 から見て beta.1 は古い");

    // 6. 使わないもの: 下書き・-dev で終わるタグ(semver では beta より後ろ)・v8・読めないタグ
    gh.set(&[
        listed("v7.0.0-beta.9", true, true, Some((1, &other))),
        listed("v7.0.0-dev", true, false, Some((1, &other))),
        listed("v8.0.0", false, false, Some((1, &other))),
        listed("v7.0.0-β2", true, false, Some((1, &other))),
        listed("latest", false, false, Some((1, &other))),
    ]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::UpToDate, "{}", st.message);
    assert_eq!(st.message, "最新版です (v7.0.0-beta.1)。");

    // 7. 新しい版の資産が同じ中身 → 最新版
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((13, &here)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::UpToDate);
    assert_eq!(st.message, "最新版です (v7.0.0-beta.2 / 2026-11-01 版)。");

    // 8. 資産がまだ無い新しい版 → 出さない(準備中と知らせる)。資産のある、それより古い新しい版があればそれを出す
    gh.set(&[listed("v7.0.0-beta.3", true, false, None), listed(beta1, true, false, Some((4, &here)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::UpToDate);
    assert_eq!(st.message, "最新版です (v7.0.0-beta.1 / 2026-11-01 版)。 v7.0.0-beta.3 は公開の準備中です (zahyou.exe がまだありません)。");
    gh.set(&[listed("v7.0.0-beta.3", true, false, None), listed("v7.0.0-beta.2", true, false, Some((1, &other)))]);
    let st = check(beta1);
    assert_eq!(st.state, UpdateState::Available);
    assert_eq!(st.release.unwrap().tag, "v7.0.0-beta.2");

    // 9. 中止(ハッシュの計算中)・つながらない
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((1, &other)))]);
    let st = check_update_at(&api, &exe, beta1, Duration::from_secs(5), &|| true);
    assert_eq!(st.state, UpdateState::Cancelled);
    let st = check_update_at("http://127.0.0.1:9/repos/x/releases/latest", &exe, beta1, Duration::from_secs(3), &|| false);
    assert_eq!(st.state, UpdateState::Error);
    assert!(st.message.starts_with("更新を確認できませんでした"), "{}", st.message);
}

/// 一覧の応答が壊れている・一覧でない → Error(panic しない)
#[test]
fn beta_check_with_broken_responses() {
    let dir = temp_dir("upd-beta-broken");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"x").unwrap();
    let body = Arc::new(Mutex::new(String::new()));
    let b = body.clone();
    let srv = TestServer::start(move |_| Resp::json(&b.lock().unwrap()));
    let api = format!("{}/repos/x/releases/latest", srv.url());
    for (bad, want) in [
        ("not json", "応答を読めません"),
        ("{\"message\": \"API rate limit exceeded\"}", "一覧ではありません"),
        ("", "応答を読めません"),
        ("[", "応答を読めません"),
    ] {
        *body.lock().unwrap() = bad.into();
        let st = check_update_at(&api, &exe, "v7.0.0-beta.1", Duration::from_secs(5), &|| false);
        assert_eq!(st.state, UpdateState::Error, "{bad:?}");
        assert!(st.message.contains(want), "{bad:?}: {}", st.message);
    }
    // 壊れた項目だけなら、その項目を使わないだけ(誤りにしない)
    *body.lock().unwrap() = "[null, 3, {\"tag_name\": \"v7.0.0\", \"assets\": null}]".into();
    let st = check_update_at(&api, &exe, "v7.0.0-beta.1", Duration::from_secs(5), &|| false);
    assert_eq!(st.state, UpdateState::UpToDate, "{}", st.message);
}

/// 試作版(-dev で終わる)は、β版の形でも v7-dev の形でも問い合わせない
#[test]
fn dev_versions_never_ask() {
    let dir = temp_dir("upd-dev");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"x").unwrap();
    let gh = FakeGitHub::start();
    gh.set(&[listed("v7.0.0", false, false, Some((1, &sha_hex(b"y"))))]);
    for ver in ["v7-dev", "v7.0.0-dev", "v7.0.0-beta.1-dev", "7.1.0-dev"] {
        let st = check_update_at(&gh.latest_url(), &exe, ver, Duration::from_secs(5), &|| false);
        assert_eq!(st.state, UpdateState::Dev, "{ver}");
        assert_eq!(st.message, "試作版のため更新の確認はしません。");
    }
    assert!(gh.paths().is_empty(), "問い合わせてしまった: {:?}", gh.paths());
}

/// 正式版(プレリリースでない版名)は今までどおり latest だけを見る。latest が今より古い版なら戻さない
#[test]
fn release_versions_use_latest_only() {
    let dir = temp_dir("upd-stable");
    let exe = dir.join("zahyou.exe");
    std::fs::write(&exe, b"fake v7 exe").unwrap();
    let here = sha_hex(b"fake v7 exe");
    let other = sha_hex(b"other");
    let gh = FakeGitHub::start();
    let api = gh.latest_url();
    let check = |ver: &str| check_update_at(&api, &exe, ver, Duration::from_secs(5), &|| false);

    // プレリリースは latest に出てこない(v6 の利用者にβ版は出ない)
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((1, &other))), listed("v6.3", false, false, Some((2, &other)))]);
    let st = check("v6.2");
    assert_eq!(st.state, UpdateState::Available, "{}", st.message);
    assert_eq!(st.release.unwrap().tag, "v6.3");
    assert_eq!(gh.paths(), vec![LATEST_PATH.to_string()]);
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((1, &other))), listed("v6.3", false, false, Some((2, &here)))]);
    assert_eq!(check("v6.3").state, UpdateState::UpToDate);
    // β版しか無ければ(latest が 404)、正式版の利用者には確かめられないと出る(v6 の更新係も同じ)
    gh.set(&[listed("v7.0.0-beta.2", true, false, Some((1, &other)))]);
    assert_eq!(check("v6.3").state, UpdateState::Error);

    // v7 の正式版を使っているとき、latest が v6 の修正版を指したら入れ替えに誘わない
    gh.set(&[listed("v6.3", false, false, Some((2, &other)))]);
    let st = check("v7.0.0");
    assert_eq!(st.state, UpdateState::UpToDate, "{}", st.message);
    assert_eq!(st.message, "最新版です (v7.0.0)。公開されている最新版 v6.3 より新しい版を使っています。");
    assert_eq!(st.here_sha256, None);
    // v7 の次の正式版は出す
    gh.set(&[listed("v7.0.1", false, false, Some((3, &other)))]);
    assert_eq!(check("v7.0.0").state, UpdateState::Available);
    // 同じ版を建て直したもの(中身が違う)は今までどおり出す
    gh.set(&[listed("v7.0.0", false, false, Some((3, &other)))]);
    assert_eq!(check("v7.0.0").state, UpdateState::Available);
    gh.set(&[listed("v7.0.0", false, false, Some((3, &here)))]);
    assert_eq!(check("v7.0.0").state, UpdateState::UpToDate);
    assert!(gh.paths().iter().all(|p| p == LATEST_PATH), "{:?}", gh.paths());
}

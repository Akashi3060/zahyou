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

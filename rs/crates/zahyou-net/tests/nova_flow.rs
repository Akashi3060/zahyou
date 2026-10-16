//! nova.astrometry.net の手順の試験。localhost に偽の nova を立て、送った要求の並びと本文を
//! Python 版(astroquery を差し替えて記録したもの、testdata/net_python.json)と比べる。本物の nova へは送らない。
mod common;

use common::*;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zahyou_net::nova::*;
use zahyou_net::NetError;

/// 偽の nova。`script(path, 何回目)` が応答を決める。
fn fake_nova(script: impl Fn(&str, usize) -> Resp + Send + Sync + 'static) -> TestServer {
    let seen: Arc<Mutex<std::collections::HashMap<String, usize>>> = Arc::default();
    TestServer::start(move |r| {
        let mut s = seen.lock().unwrap();
        let k = s.entry(r.path.clone()).or_insert(0);
        *k += 1;
        script(&r.path, *k)
    })
}

fn client(srv: &TestServer) -> NovaClient {
    let mut c = NovaClient::new(&srv.url());
    c.poll = Duration::from_millis(5);
    c
}

/// Python の記録(URL は https://nova.astrometry.net/...)を (方法, パス, 本文) にする。
fn py_requests(v: &Value) -> Vec<(String, String, Option<String>)> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let url = r["url"].as_str().unwrap();
            (
                r["method"].as_str().unwrap().to_string(),
                url.trim_start_matches("https://nova.astrometry.net").to_string(),
                r["body"].as_str().map(String::from),
            )
        })
        .collect()
}

fn rs_requests(srv: &TestServer) -> Vec<(String, String, Option<String>)> {
    srv.requests()
        .iter()
        .map(|q| (q.method.clone(), q.path.clone(), (!q.body.is_empty()).then(|| String::from_utf8_lossy(&q.body).into_owned())))
        .collect()
}

#[test]
fn source_list_flow_matches_python() {
    let Some(py) = python_values() else { return };
    let nv = &py["nova"];
    let wcs = nv["wcs_text"].as_str().unwrap().to_string();
    let src: Vec<(f64, f64)> = nv["sources"].as_array().unwrap().iter().map(|s| (s["x"].as_f64().unwrap(), s["y"].as_f64().unwrap())).collect();
    let want_cards: Vec<String> = serde_json::from_value(nv["wcs_cards"].clone()).unwrap();
    for case in nv["cases"].as_array().unwrap() {
        let w = wcs.clone();
        let srv = fake_nova(move |path, n| match path {
            "/api/login" => Resp::json("{\"status\": \"success\", \"session\": \"sess-abc\"}"),
            "/api/url_upload" => Resp::json("{\"status\": \"success\", \"subid\": 4242, \"hash\": \"x\"}"),
            "/api/submissions/4242" => Resp::json("{\"processing_started\": \"x\", \"jobs\": [777]}"),
            "/api/jobs/777/info" if n == 1 => Resp::json("{\"status\": \"solving\"}"),
            "/api/jobs/777/info" => Resp::json("{\"status\": \"success\"}"),
            "/wcs_file/777" => Resp::new(200, w.clone().into_bytes()),
            _ => Resp::new(404, "no"),
        });
        let hint = case["hint"].as_array().map(|h| (h[0].as_f64().unwrap(), h[1].as_f64().unwrap()));
        let req = OnlineRequest {
            api_key: "KEY-0123456789",
            image_path: None,
            sources: Some(&src),
            width: 968,
            height: 548,
            timeout_s: 120.0,
            hint_radec: hint,
            hint_radius_deg: 5.0,
            scale_hint: case["scale"].as_f64(),
        };
        let mut c = client(&srv);
        let sol = solve_online_with(&mut c, &req, &|| false, &|m| eprintln!("[log] {m}")).unwrap().unwrap();
        assert!(sol.from_source_list);
        assert_eq!((sol.submission_id, sol.job_id), (4242, 777));
        assert_eq!(sol.cards, want_cards);
        assert_eq!(rs_requests(&srv), py_requests(&case["requests"]), "要求の並び (hint {hint:?})");
        // 送り方(form)
        let reqs = srv.requests();
        assert_eq!(reqs[0].header("content-type"), Some("application/x-www-form-urlencoded"));
    }
}

#[test]
fn image_upload_flow_matches_python() {
    let Some(py) = python_values() else { return };
    let up = &py["nova"]["image_upload"];
    let srv = fake_nova(|path, n| match path {
        "/api/login" => Resp::json("{\"status\": \"success\", \"session\": \"sess-img\"}"),
        "/api/upload" => Resp::json("{\"status\": \"success\", \"subid\": 9}"),
        "/api/submissions/9" if n == 1 => Resp::json("{\"jobs\": [null]}"),
        "/api/submissions/9" => Resp::json("{\"jobs\": [55]}"),
        "/api/jobs/55/info" => Resp::json("{\"status\": \"failure\"}"),
        _ => Resp::new(404, "no"),
    });
    let dir = temp_dir("nova-img");
    let img = dir.join("撮影 01.fits");
    let content = b"SIMPLE  =                    T\0\x01\xff".to_vec();
    std::fs::write(&img, &content).unwrap();
    let req = OnlineRequest {
        api_key: "KEY-0123456789",
        image_path: Some(&img),
        sources: None,
        width: 968,
        height: 548,
        timeout_s: 60.0,
        hint_radec: Some((10.5, -20.25)),
        hint_radius_deg: 5.0,
        scale_hint: Some(1.2345),
    };
    let mut c = client(&srv);
    let r = solve_online_with(&mut c, &req, &|| false, &|m| eprintln!("[log] {m}")).unwrap();
    assert_eq!(r, None, "nova が解けないと答えた");
    // 並び(upload の本文は multipart なので別に見る)
    let got: Vec<(String, String)> = srv.requests().iter().map(|q| (q.method.clone(), q.path.clone())).collect();
    let want: Vec<(String, String)> = py_requests(&up["requests"]).into_iter().map(|(m, p, _)| (m, p)).collect();
    assert_eq!(got, want);
    let q = &srv.requests()[1];
    let ct = q.header("content-type").unwrap();
    let boundary = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
    let body = &q.body;
    let rj = up["requests"][1]["request_json"].as_str().unwrap();
    let part1 = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request-json\"\r\n\r\n{rj}\r\n");
    assert!(body.starts_with(part1.as_bytes()), "request-json の部分が Python と同じ");
    let head2 = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
        up["requests"][1]["files"]["file"].as_str().unwrap()
    );
    let mut rest = body[part1.len()..].to_vec();
    assert!(rest.starts_with(head2.as_bytes()));
    rest.drain(..head2.len());
    let tail = format!("\r\n--{boundary}--\r\n");
    assert!(rest.ends_with(tail.as_bytes()));
    assert_eq!(&rest[..rest.len() - tail.len()], &content[..], "ファイルの中身そのまま");
}

#[test]
fn falls_back_to_image_then_times_out_or_cancels() {
    // 星のリストは「解けない」→ 画像を送る(ログインは 1 回だけ)
    let srv = fake_nova(|path, _| match path {
        "/api/login" => Resp::json("{\"status\": \"success\", \"session\": \"s\"}"),
        "/api/url_upload" => Resp::json("{\"status\": \"success\", \"subid\": 1}"),
        "/api/upload" => Resp::json("{\"status\": \"success\", \"subid\": 2}"),
        "/api/submissions/1" => Resp::json("{\"jobs\": [11]}"),
        "/api/submissions/2" => Resp::json("{\"jobs\": [22]}"),
        "/api/jobs/11/info" => Resp::json("{\"status\": \"failure\"}"),
        "/api/jobs/22/info" => Resp::json("{\"status\": \"success\"}"),
        "/wcs_file/22" => Resp::new(200, format!("{:<80}{:<80}", "CRVAL1  =                 10.0", "END").into_bytes()),
        _ => Resp::new(404, "no"),
    });
    let dir = temp_dir("nova-fb");
    let img = dir.join("a.png");
    std::fs::write(&img, b"png").unwrap();
    let src = [(1.0, 2.0)];
    let mut req = OnlineRequest {
        api_key: "K",
        image_path: Some(&img),
        sources: Some(&src),
        width: 10,
        height: 10,
        timeout_s: 30.0,
        hint_radec: None,
        hint_radius_deg: 5.0,
        scale_hint: None,
    };
    let log = Mutex::new(Vec::<String>::new());
    let sol = solve_online_with(&mut client(&srv), &req, &|| false, &|m| log.lock().unwrap().push(m.into())).unwrap().unwrap();
    assert!(!sol.from_source_list);
    assert_eq!(sol.cards, vec![format!("{:<80}", "CRVAL1  =                 10.0")]);
    let paths: Vec<String> = srv.requests().iter().map(|q| q.path.clone()).collect();
    assert_eq!(paths.iter().filter(|p| *p == "/api/login").count(), 1);
    assert!(log.lock().unwrap().iter().any(|m| m.contains("画像そのものを送ります")));

    // いつまでも solving → 時間切れ
    let srv = fake_nova(|path, _| match path {
        "/api/login" => Resp::json("{\"status\": \"success\", \"session\": \"s\"}"),
        "/api/url_upload" => Resp::json("{\"status\": \"success\", \"subid\": 1}"),
        "/api/submissions/1" => Resp::json("{\"jobs\": []}"),
        _ => Resp::new(404, "no"),
    });
    req.timeout_s = 1.0;
    let t = std::time::Instant::now();
    let r = solve_online_with(&mut client(&srv), &req, &|| false, &|_| {});
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m.contains("終わりませんでした")), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(5));
    // 中止
    let r = solve_online_with(&mut client(&srv), &req, &|| true, &|_| {});
    assert_eq!(r, Err(NetError::Cancelled));
    // ログイン失敗
    let srv = fake_nova(|_, _| Resp::json("{\"status\": \"error\"}"));
    let r = solve_online_with(&mut client(&srv), &req, &|| false, &|_| {});
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m.contains("Unable to log in")), "{r:?}");
    // 手がかりの値がおかしい(astroquery と同じく下限で弾く)
    req.hint_radec = Some((-1.0, 0.0));
    let r = solve_online_with(&mut client(&srv), &req, &|| false, &|_| {});
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m.contains("center_ra")), "{r:?}");
    // API キーが無い
    req.hint_radec = None;
    req.api_key = " ";
    let r = solve_online_with(&mut client(&srv), &req, &|| false, &|_| {});
    assert!(matches!(r, Err(NetError::Failed(ref m)) if m.contains("API キー")), "{r:?}");
}

/// API キーの値はログ・失敗の文・Debug に出ない(nova の応答がキーをそのまま返してきても伏せる)。
/// 値は assert! で比べ、失敗の文にも入れない。
#[test]
fn api_key_is_never_logged() {
    const KEY: &str = "secretkey0123456";
    // 偽の nova: ログインは通し、星のリストの投稿には受け取った本文(キーを含む)をそのまま返す(subid なし)。
    // 状態の問い合わせは失敗させ、その文にもキーを入れる
    let srv = TestServer::start(|r| match r.path.as_str() {
        "/api/login" => {
            let body = String::from_utf8_lossy(&r.body).into_owned();
            assert!(body.contains(KEY), "ログインの本文にキーが入っていない");
            Resp::json("{\"status\": \"success\", \"session\": \"s\"}")
        }
        "/api/url_upload" => Resp::json(&format!("{{\"status\": \"error\", \"errormessage\": \"bad key {KEY}\"}}")),
        "/api/upload" => Resp::json("{\"status\": \"success\", \"subid\": 5}"),
        "/api/submissions/5" => Resp::new(500, format!("key={KEY}")),
        _ => Resp::new(404, KEY),
    });
    let dir = temp_dir("nova-redact");
    let img = dir.join("a.fits");
    std::fs::write(&img, b"SIMPLE  =                    T").unwrap();
    let src = [(1.0, 2.0)];
    let req = OnlineRequest {
        api_key: KEY,
        image_path: Some(&img),
        sources: Some(&src),
        width: 10,
        height: 10,
        timeout_s: 1.0,
        hint_radec: None,
        hint_radius_deg: 5.0,
        scale_hint: None,
    };
    // Debug に値が出ない
    let shown = format!("{req:?}");
    assert!(!shown.contains(KEY), "OnlineRequest の Debug にキーが出た");
    assert!(shown.contains("<伏せ字 16 文字>"), "伏せ字の表示が無い");
    // 星のリストの投稿で失敗(応答にキーが入っている)→ 失敗の文では伏せる
    let log = Mutex::new(Vec::<String>::new());
    let r = solve_online_with(&mut client(&srv), &req, &|| false, &|m| log.lock().unwrap().push(m.into()));
    match &r {
        Err(NetError::Failed(m)) => {
            assert!(!m.contains(KEY), "失敗の文にキーが出た");
            assert!(m.contains(REDACTED) && m.contains("subid"), "伏せた印が無い");
        }
        _ => panic!("失敗するはず: {}", r.is_ok()),
    }
    // 画像を送る道(状態の問い合わせの失敗の文にキー → ログでは伏せる。そのあと時間切れ)
    let req2 = OnlineRequest { sources: None, ..req.clone() };
    let r2 = solve_online_with(&mut client(&srv), &req2, &|| false, &|m| log.lock().unwrap().push(m.into()));
    assert!(matches!(&r2, Err(NetError::Failed(m)) if m.contains("終わりませんでした") && !m.contains(KEY)));
    let lines = log.into_inner().unwrap();
    assert!(lines.iter().any(|l| l.contains("状態を聞けませんでした")), "状態の問い合わせの失敗がログに無い");
    assert!(lines.iter().all(|l| !l.contains(KEY)), "ログにキーが出た");
}

#[test]
fn api_key_helpers() {
    // 既定のキーは v6 と同じ 16 文字(値は書かない)
    assert_eq!(DEFAULT_API_KEY.chars().count(), 16);
    assert!(DEFAULT_API_KEY.chars().all(|c| c.is_ascii_lowercase()));
    let k = ApiKey::default_key();
    assert!(k.is_default() && k.usable() == Some(DEFAULT_API_KEY));
    assert_eq!(k.describe(), "既定のキー (16 文字)");
    assert!(!format!("{k:?}").contains(DEFAULT_API_KEY));
    // 前後の空白は使うときに除く。空白だけなら空
    assert!(ApiKey::new(format!("  {DEFAULT_API_KEY} ")).is_default());
    assert_eq!(ApiKey::new("  ").usable(), None);
    assert_eq!(ApiKey::new("").describe(), "空");
    assert_eq!(ApiKey::new("abc-123").describe(), "既定と違うキー (7 文字)");
    assert!(!ApiKey::new("abc-123").is_default());
    // 初めの値は環境変数、無ければ既定のキー
    match api_key_from_env() {
        Some(e) => assert!(initial_api_key() == e),
        None => assert!(initial_api_key() == DEFAULT_API_KEY && ApiKey::initial().is_default()),
    }
    // 伏せ字: 4 文字以上だけ伏せる(短い値で、ふつうの文字が消えないように)
    assert_eq!(redact_key("a secretkey here secretkey", " secretkey "), format!("a {REDACTED} here {REDACTED}"));
    assert_eq!(redact_key("Key K k", "K"), "Key K k");
    assert_eq!(redact_key("no key", ""), "no key");
}

#[test]
fn internet_check_against_local() {
    let srv = TestServer::start(|r| if r.path == "/api/" { Resp::new(200, "ok") } else { Resp::new(404, "") });
    assert!(zahyou_net::internet_available_at(&srv.url(), Duration::from_secs(4)));
    let srv2 = TestServer::start(|_| Resp::new(500, ""));
    assert!(!zahyou_net::internet_available_at(&srv2.url(), Duration::from_secs(4)));
    let t = std::time::Instant::now();
    assert!(!zahyou_net::internet_available_at("http://127.0.0.1:9", Duration::from_secs(4)));
    eprintln!("つながらない宛先の判定: {:.3} 秒", t.elapsed().as_secs_f64());
}

/// 状態の問い合わせの最中に中止され、そこへ nova が「解けない」と答えても、画像は送らない。
#[test]
fn cancel_with_failure_answer_does_not_upload_image() {
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let c2 = cancel.clone();
    let srv = fake_nova(move |path, _| match path {
        "/api/login" => Resp::json("{\"status\": \"success\", \"session\": \"s\"}"),
        "/api/url_upload" => Resp::json("{\"status\": \"success\", \"subid\": 1}"),
        "/api/submissions/1" => Resp::json("{\"jobs\": [2]}"),
        "/api/jobs/2/info" => {
            // 利用者がこの問い合わせの往復の間に「中止」を押した
            c2.store(true, std::sync::atomic::Ordering::SeqCst);
            Resp::json("{\"status\": \"failure\"}")
        }
        "/api/upload" => Resp::json("{\"status\": \"success\", \"subid\": 3}"),
        _ => Resp::new(404, "no"),
    });
    let dir = temp_dir("nova-cancel-fail");
    let img = dir.join("a.fits");
    std::fs::write(&img, b"SIMPLE  =                    T").unwrap();
    let src = [(1.0, 2.0)];
    let req = OnlineRequest {
        api_key: "K",
        image_path: Some(&img),
        sources: Some(&src),
        width: 10,
        height: 10,
        timeout_s: 30.0,
        hint_radec: None,
        hint_radius_deg: 5.0,
        scale_hint: None,
    };
    let stop = || cancel.load(std::sync::atomic::Ordering::SeqCst);
    let r = solve_online_with(&mut client(&srv), &req, &stop, &|_| {});
    assert_eq!(r, Err(NetError::Cancelled));
    let paths: Vec<String> = srv.requests().iter().map(|q| q.path.clone()).collect();
    assert!(!paths.iter().any(|p| p == "/api/upload"), "中止のあとで画像を送った: {paths:?}");
}

/// 応答が遅いときも、中止すれば応答を待たずにすぐ戻る(状態の問い合わせ・画像の送信)。
#[test]
fn cancel_does_not_wait_for_slow_responses() {
    for slow in ["/api/jobs/2/info", "/api/upload"] {
        let srv = fake_nova(move |path, _| {
            if path == slow {
                std::thread::sleep(Duration::from_secs(5));
            }
            match path {
                "/api/login" => Resp::json("{\"status\": \"success\", \"session\": \"s\"}"),
                "/api/url_upload" => Resp::json("{\"status\": \"success\", \"subid\": 1}"),
                "/api/upload" => Resp::json("{\"status\": \"success\", \"subid\": 1}"),
                "/api/submissions/1" => Resp::json("{\"jobs\": [2]}"),
                "/api/jobs/2/info" => Resp::json("{\"status\": \"solving\"}"),
                _ => Resp::new(404, "no"),
            }
        });
        let dir = temp_dir("nova-cancel-slow");
        let img = dir.join("a.fits");
        std::fs::write(&img, vec![7u8; 200_000]).unwrap();
        let src = [(1.0, 2.0)];
        let req = OnlineRequest {
            api_key: "K",
            image_path: Some(&img),
            // 画像の送信を見るときは星のリストを渡さない(いきなり画像を送る)
            sources: if slow == "/api/upload" { None } else { Some(&src) },
            width: 10,
            height: 10,
            timeout_s: 60.0,
            hint_radec: None,
            hint_radius_deg: 5.0,
            scale_hint: None,
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let set_at: Mutex<Option<std::time::Instant>> = Mutex::new(None);
        let (r, lag) = std::thread::scope(|sc| {
            sc.spawn(|| {
                // 遅い要求を投げた頃に中止する
                std::thread::sleep(Duration::from_millis(600));
                *set_at.lock().unwrap() = Some(std::time::Instant::now());
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let stop = || cancel.load(std::sync::atomic::Ordering::SeqCst);
            let r = solve_online_with(&mut client(&srv), &req, &stop, &|_| {});
            let end = std::time::Instant::now();
            let lag = set_at.lock().unwrap().map(|t| end.saturating_duration_since(t).as_secs_f64());
            (r, lag)
        });
        eprintln!("{slow} が遅いとき: {r:?}、中止から {lag:?} 秒で戻った");
        assert_eq!(r, Err(NetError::Cancelled), "{slow}");
        let lag = lag.expect("中止より先に戻った");
        assert!(lag < 0.5, "{slow}: 中止から戻るまで {lag:.3} 秒");
    }
}

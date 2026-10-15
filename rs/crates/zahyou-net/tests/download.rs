//! 星図データを落とす処理の試験(localhost のサーバーと小さな偽のファイルだけを使う)。
mod common;

use common::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zahyou_net::*;

/// 落とす試験の既定: 区切り 4 KiB(小さなファイルでも何度も Range を頼む)、待ち時間 1 ms。
/// 偽の索引ファイルは表を持たないので、zahyou-index の検査は外す(軽い検査だけ)
fn opts(base: &str) -> FetchOptions<'static> {
    FetchOptions {
        validate: None,
        lite_base: format!("{base}/lite"),
        g41_base: format!("{base}/g41"),
        retry_unit: Duration::from_millis(1),
        segment: 4096,
        ..FetchOptions::default()
    }
}

fn files() -> HashMap<String, Vec<u8>> {
    let mut m = HashMap::new();
    m.insert("/lite/index-5206-00.fits".to_string(), fake_index(5206, 0, 20_000));
    m.insert("/lite/index-5206-01.fits".to_string(), fake_index(5206, 1, 33_333));
    m.insert("/g41/index-4119.fits".to_string(), fake_index(4119, -1, 9_000));
    m
}

fn names() -> Vec<String> {
    vec!["index-4119.fits".into(), "index-5206-00.fits".into(), "index-5206-01.fits".into()]
}

struct Run {
    res: FetchResult,
    log: Vec<String>,
    last_progress: Option<(u64, u64)>,
}

fn run(dest: &Path, o: &FetchOptions, names: &[String], stop_after: Option<u64>) -> Run {
    let log = Mutex::new(Vec::new());
    let prog = Mutex::new(None);
    let stop_flag = std::sync::atomic::AtomicBool::new(false);
    let res = fetch_index_with(
        names,
        dest,
        o,
        &|d, t| {
            *prog.lock().unwrap() = Some((d, t));
            if let Some(n) = stop_after
                && d >= n
            {
                stop_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        },
        &|| stop_flag.load(std::sync::atomic::Ordering::SeqCst),
        &|m| log.lock().unwrap().push(m.to_string()),
    );
    let log = log.into_inner().unwrap();
    for l in &log {
        eprintln!("[log] {l}");
    }
    Run { res, log, last_progress: prog.into_inner().unwrap() }
}

fn assert_files(dest: &Path, m: &HashMap<String, Vec<u8>>) {
    for (path, data) in m {
        let name = path.rsplit('/').next().unwrap();
        let got = std::fs::read(dest.join(name)).unwrap_or_default();
        assert!(got == *data, "{name} の中身が違う ({} / {} バイト)", got.len(), data.len());
        assert!(!dest.join(format!("{name}.part")).exists(), "{name}.part が残っている");
        assert!(check_index_file(&dest.join(name), None).is_ok());
    }
}

#[test]
fn fetch_with_range_and_skip_existing() {
    let m = Arc::new(files());
    let mm = m.clone();
    let srv = TestServer::start(move |r| match mm.get(&r.path) {
        Some(d) => serve_bytes(r, d, true),
        None => Resp::new(404, "no"),
    });
    let dest = temp_dir("fetch-basic");
    let o = opts(&srv.url());
    let r = run(&dest, &o, &names(), None);
    assert!(r.res.ok, "{:?}", r.res);
    assert_eq!((r.res.planned, r.res.done, r.res.already), (3, 3, 0));
    let total: u64 = m.values().map(|d| d.len() as u64).sum();
    assert_eq!(r.res.total_bytes, total);
    assert_eq!(r.last_progress, Some((total, total)), "進み具合が 100% で終わる");
    assert_files(&dest, &m);
    // 区切りごとの Range
    let gets: Vec<String> = srv.requests().iter().filter(|q| q.method == "GET").filter_map(|q| q.header("range").map(String::from)).collect();
    assert!(gets.contains(&"bytes=0-4095".to_string()) && gets.contains(&"bytes=4096-8191".to_string()), "{gets:?}");
    assert!(r.log.iter().any(|l| l.contains("3 ファイル /") && l.contains("へ落とします")));
    assert!(r.log.iter().any(|l| l == "  完了: 3/3 ファイル"));

    // もう一度: すべて落ちている(HEAD だけ)
    srv.clear();
    let r = run(&dest, &o, &names(), None);
    assert!(r.res.ok);
    assert_eq!((r.res.planned, r.res.already), (0, 3));
    assert!(r.log.iter().any(|l| l == "  すべて落ちています (3 ファイル)。"));
    assert!(srv.requests().iter().all(|q| q.method == "HEAD"));
}

#[test]
fn fetch_resumes_after_cut_and_from_part() {
    let m = Arc::new(files());
    let cuts = Counter::default();
    let (mm, c) = (m.clone(), cuts.clone());
    let srv = TestServer::start(move |r| {
        let Some(d) = mm.get(&r.path) else { return Resp::new(404, "no") };
        let resp = serve_bytes(r, d, true);
        // index-5206-01 の GET は最初の 2 回、本文を 1000 バイトで切る
        if r.method == "GET" && r.path.ends_with("5206-01.fits") && c.get() < 2 {
            c.bump();
            return resp.cut(1000);
        }
        resp
    });
    let dest = temp_dir("fetch-resume");
    // index-5206-00 は途中まで(.part)、index-4119 は v6 が残した途中のファイル(本来の名前)
    std::fs::write(dest.join("index-5206-00.fits.part"), &m["/lite/index-5206-00.fits"][..10_000]).unwrap();
    std::fs::write(dest.join("index-4119.fits"), &m["/g41/index-4119.fits"][..5_000]).unwrap();
    let o = opts(&srv.url());
    let r = run(&dest, &o, &names(), None);
    assert!(r.res.ok, "{:?}", r.res);
    assert_files(&dest, &m);
    // 残りのぶんだけを数える
    let total: u64 = m.values().map(|d| d.len() as u64).sum::<u64>() - 10_000 - 5_000;
    assert_eq!(r.res.total_bytes, total);
    let reqs = srv.requests();
    let ranges: Vec<(String, String)> = reqs.iter().filter(|q| q.method == "GET").map(|q| (q.path.clone(), q.header("range").unwrap_or("").to_string())).collect();
    assert!(ranges.contains(&("/lite/index-5206-00.fits".into(), "bytes=10000-14095".into())), "{ranges:?}");
    assert!(ranges.contains(&("/g41/index-4119.fits".into(), "bytes=5000-9095".into())), "{ranges:?}");
    // 切れたあとは、受け取ったところ(1000 バイト)から頼み直す
    assert!(ranges.contains(&("/lite/index-5206-01.fits".into(), "bytes=1000-5095".into())), "{ranges:?}");
    assert_eq!(r.log.iter().filter(|l| l.contains("続きから再開します")).count(), 2);
    assert!(r.log.iter().any(|l| l.contains("index-5206-01.fits") && l.contains("(1/5)")));
}

#[test]
fn fetch_gives_up_rejects_corrupt_and_missing() {
    let mut m = files();
    // INDEXID が名前と違うファイル
    m.insert("/lite/index-5206-02.fits".into(), fake_index(5205, 2, 5_000));
    let m = Arc::new(m);
    let mm = m.clone();
    let srv = TestServer::start(move |r| {
        let Some(d) = mm.get(&r.path) else { return Resp::new(404, "no") };
        let resp = serve_bytes(r, d, true);
        // index-4119 はいつも切れる → 5 回であきらめる
        if r.method == "GET" && r.path.ends_with("4119.fits") {
            return resp.cut(10);
        }
        resp
    });
    let dest = temp_dir("fetch-fail");
    let o = opts(&srv.url());
    let names = vec![
        "index-4119.fits".to_string(),
        "index-5206-02.fits".to_string(),
        "index-5206-03.fits".to_string(), // 404
        "readme.txt".to_string(),         // 名前が違う
    ];
    let r = run(&dest, &o, &names, None);
    assert!(!r.res.ok);
    assert_eq!(r.res.done, 0);
    let mut failed = r.res.failed.clone();
    failed.sort();
    assert_eq!(failed, vec!["index-4119.fits", "index-5206-02.fits", "index-5206-03.fits", "readme.txt"]);
    assert!(r.log.iter().any(|l| l == "    index-4119.fits: あきらめました。"));
    assert_eq!(r.log.iter().filter(|l| l.contains("index-4119.fits") && l.contains("続きから再開します")).count(), 5);
    assert!(r.log.iter().any(|l| l.contains("index-5206-02.fits: 落としたファイルの中身がおかしいので消します (INDEXID が名前と違います (5205))")));
    assert!(r.log.iter().any(|l| l.starts_with("    index-5206-03.fits: 大きさを取れません")));
    assert!(!dest.join("index-5206-02.fits").exists(), "中身のおかしいファイルは消す");
    // 4119 の .part は残る(次に続きから)
    assert_eq!(std::fs::metadata(dest.join("index-4119.fits.part")).unwrap().len(), 50);
}

#[test]
fn fetch_server_without_range_and_cancel() {
    let m = Arc::new(files());
    let mm = m.clone();
    // Range を無視するサーバー
    let srv = TestServer::start(move |r| match mm.get(&r.path) {
        Some(d) => serve_bytes(r, d, false),
        None => Resp::new(404, "no"),
    });
    let dest = temp_dir("fetch-norange");
    std::fs::write(dest.join("index-5206-00.fits.part"), b"garbage").unwrap();
    let o = opts(&srv.url());
    let r = run(&dest, &o, &names(), None);
    assert!(r.res.ok, "{:?}", r.res);
    assert_files(&dest, &m);

    // 中止: 2 万バイト受け取ったところで止める
    let dest = temp_dir("fetch-cancel");
    let r = run(&dest, &o, &names(), Some(20_000));
    assert!(r.res.cancelled && !r.res.ok, "{:?}", r.res);
    assert!(r.log.iter().any(|l| l == "  中止しました。"));
}

#[test]
fn fetch_from_python_http_server() {
    let dir = temp_dir("pyserve-src");
    std::fs::create_dir_all(dir.join("lite")).unwrap();
    std::fs::create_dir_all(dir.join("g41")).unwrap();
    let m = files();
    for (p, d) in &m {
        std::fs::write(dir.join(p.trim_start_matches('/')), d).unwrap();
    }
    let Some(py) = PyServer::start(&dir) else {
        eprintln!("SKIP: python -m http.server を起動できません");
        return;
    };
    let dest = temp_dir("pyserve-dest");
    // 途中まで落ちた .part(http.server は Range に応えないので最初から落とし直す)
    std::fs::write(dest.join("index-5206-01.fits.part"), &m["/lite/index-5206-01.fits"][..3000]).unwrap();
    let o = FetchOptions { segment: 8 << 20, ..opts(&py.url()) };
    let t = std::time::Instant::now();
    let r = run(&dest, &o, &names(), None);
    assert!(r.res.ok, "{:?}", r.res);
    assert_files(&dest, &m);
    eprintln!("python -m http.server から 3 ファイルを {:.3} 秒で落とした", t.elapsed().as_secs_f64());
    let sizes = remote_sizes_urls(
        &[("index-4119.fits".into(), format!("{}/g41/index-4119.fits", py.url())), ("x".into(), format!("{}/nothing", py.url()))],
        8,
        &|| false,
    );
    assert_eq!(sizes, vec![("index-4119.fits".to_string(), Some(m["/g41/index-4119.fits"].len() as u64)), ("x".to_string(), None)]);
}

#[test]
fn plan_and_recommend_examples() {
    // README の例: 800mm / 7.4mm(31.8′)→ 4107〜4109
    let fov = fov_arcmin(800.0, 7.4).unwrap();
    assert_eq!(recommend_scales(fov, None), vec![4107, 4108, 4109]);
    // 35mm / 36mm(3266′)→ 4118, 4119(5200 を勧めない)
    assert_eq!(recommend_scales(fov_arcmin(35.0, 36.0).unwrap(), None), vec![4118, 4119]);
    // 値は Python 版の plan_size(同じ状態のフォルダー)が返したもの
    let d = temp_dir("plan");
    std::fs::write(d.join("index-5206-00.fits"), b"x").unwrap();
    assert_eq!(plan_size(&[5206], &d), 328_553_813);
    assert_eq!(plan_size(&[4107, 5206, 5205], &d), 1_144_345_941);
}

/// 既定の設定では zahyou-index の検査も通す: 形だけの偽物は落としたあとで消され、本物(手元にあれば)は通る。
#[test]
fn fetch_default_validates_with_zahyou_index() {
    let mut m: HashMap<String, Vec<u8>> = HashMap::new();
    m.insert("/lite/index-5206-05.fits".into(), fake_index(5206, 5, 3_000));
    let real = Path::new(r"C:\AstrometryData\index-4119.fits");
    let real_data = std::fs::read(real).ok();
    if let Some(d) = &real_data {
        m.insert("/g41/index-4119.fits".into(), d.clone());
    }
    let m = Arc::new(m);
    let mm = m.clone();
    let srv = TestServer::start(move |r| match mm.get(&r.path) {
        Some(d) => serve_bytes(r, d, true),
        None => Resp::new(404, "no"),
    });
    let dest = temp_dir("fetch-validate");
    let o = FetchOptions { validate: FetchOptions::default().validate, ..opts(&srv.url()) };
    assert!(o.validate.is_some(), "既定は zahyou-index の検査つき");
    let mut names = vec!["index-5206-05.fits".to_string()];
    if real_data.is_some() {
        names.push("index-4119.fits".into());
    }
    let r = run(&dest, &FetchOptions { segment: 64 << 10, ..o }, &names, None);
    assert!(!r.res.ok);
    assert_eq!(r.res.failed, vec!["index-5206-05.fits".to_string()]);
    let why = r.log.iter().find(|l| l.contains("index-5206-05.fits: 落としたファイルの中身がおかしいので消します")).expect("消した記録");
    eprintln!("zahyou-index が偽物を弾いた理由: {why}");
    assert!(!dest.join("index-5206-05.fits").exists());
    // 軽い検査だけなら通る(表が無いことまでは見ない)
    std::fs::write(dest.join("x.bin"), &m["/lite/index-5206-05.fits"]).unwrap();
    let p = dest.join("index-5206-05.fits");
    std::fs::rename(dest.join("x.bin"), &p).unwrap();
    assert!(check_index_file(&p, None).is_ok());
    assert!(check_index_file_full(&p).is_err());
    match real_data {
        None => eprintln!("SKIP: {} が無いので本物の索引ファイルを落とす試験は省く", real.display()),
        Some(d) => {
            assert_eq!(r.res.done, 1);
            assert_eq!(std::fs::read(dest.join("index-4119.fits")).unwrap(), d);
            assert!(validate_with_index(&dest.join("index-4119.fits")).is_ok());
            eprintln!("本物の index-4119.fits ({} バイト) を落として zahyou-index の検査に通った", d.len());
        }
    }
}

/// 空き容量が足りなければ、落とし始める前にやめる(HEAD で 10 PB のふりをするサーバー)。
#[test]
fn fetch_stops_when_disk_is_too_small() {
    let srv = TestServer::start(|r| {
        if r.method != "HEAD" {
            return Resp::new(500, "GET してはいけない");
        }
        let mut resp = Resp::new(200, Vec::new());
        resp.claim_len = Some(10u64.pow(16));
        resp
    });
    let dest = temp_dir("fetch-nospace");
    let r = run(&dest, &opts(&srv.url()), &["index-5206-00.fits".to_string()], None);
    assert!(!r.res.ok);
    assert_eq!(r.res.failed, vec!["index-5206-00.fits".to_string()]);
    assert!(r.log.iter().any(|l| l.starts_with("  空き容量が足りません (要 9094.9 TB / 空き ")), "{:?}", r.log);
    assert!(srv.requests().iter().all(|q| q.method == "HEAD"), "GET していない");
    assert!(!dest.join("index-5206-00.fits.part").exists());
}

/// 速さの物差し(手で走らせるとき): ZAHYOU_NET_BENCH_BASE の `<base>/lite/index-5206-NN.fits` を
/// ZAHYOU_NET_BENCH_DEST へ落とす。Python 版と同じサーバー・同じファイルで比べる(scratch の bench_fetch.py)。
#[test]
#[ignore]
fn fetch_bench_local() {
    let (Some(base), Some(dest)) = (std::env::var_os("ZAHYOU_NET_BENCH_BASE"), std::env::var_os("ZAHYOU_NET_BENCH_DEST")) else {
        eprintln!("SKIP: ZAHYOU_NET_BENCH_BASE / ZAHYOU_NET_BENCH_DEST が無い");
        return;
    };
    let base = base.to_string_lossy().into_owned();
    let dest = std::path::PathBuf::from(dest);
    let _ = std::fs::remove_dir_all(&dest);
    let o = FetchOptions { validate: None, lite_base: format!("{base}/lite"), ..FetchOptions::default() };
    let t = std::time::Instant::now();
    let r = fetch_index_with(&index_names(5206), &dest, &o, &|_, _| {}, &|| false, &|_| {});
    let secs = t.elapsed().as_secs_f64();
    let total: u64 = std::fs::read_dir(&dest).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum();
    eprintln!("Rust fetch_index: ok={} {:.0} MB {secs:.2} s ({:.0} MB/s)", r.ok, total as f64 / 1e6, total as f64 / 1e6 / secs);
    assert!(r.ok);
}

//! 速さの物差し: Python 基準実装(dev/solver_core.py の sv_solve)と同じ星・同じ条件・同じ機械で解法の時間を比べる。
//!
//! ```text
//! cargo run --release -p zahyou-solver --example bench_solver [-- [--repeat 3] [--cases ra0,few20] [--out 結果.json]
//!                                                                    [--python py.json]]
//! python -B rs/testdata/bench_solver.py --out py.json        (Python 側を同じ星・同じ条件で測り直す)
//! ```
//!
//! 測るもの(星と条件は rs/testdata の solver_real.json・solver_synth.json。gen_solver.py が Python で作ったもの)
//! - 基準の実写 real_orig(968×548、深い検出の星)を ブラインド・ヒント(目標 5°)・スケール既知(正解 ×0.75〜1.25)で
//! - 合成の難しい場合: ra0(赤経 0° をまたぐ、ブラインド)・tile_corner(タイルの角)・polein(視野に極)・few20(星 20 個)、
//!   ほかに bb(全天ブラインド)・w25deg(横 25°)・many300(星 300 個)・rot090
//! - 利用者の経路 solve_offline(画像が手元にあれば。zahyou-image の読み込み・前処理・検出は含めない)
//! - 解けない星野(rand60、締め切り 5 秒)の締め切りの超過
//!
//! Python の秒は、--python を渡せば rs/testdata/bench_solver.py が書いた JSON(同じ名前の項目。repeat 回の最短)、
//! 無ければ solver_*.json に入っている参考値(gen_solver.py が同じ PC で測ったもの。実写の 3 条件は 3 回の最短、
//! 合成は trace つきの 1 回なので少し遅め)。
//! Rust は --repeat 回(既定 3)の最短。Python と同じ答え(索引と対数オッズ)になったかも出す。
//! 星図データは C:\AstrometryData(ZAHYOU_INDEX_DIR)、基準値は rs/testdata(ZAHYOU_TESTDATA)。
//! 画像は $ZAHYOU_TEST_IMAGES(既定はホームフォルダー)。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zahyou_image::{DetectOpts, detect_sources, load_image_any, preprocess};
use zahyou_index::Catalog;
use zahyou_solver::{Control, FieldStar, OfflineRequest, Params, SolveRequest, Solution};

fn index_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_INDEX_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\AstrometryData"))
}

fn testdata_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("testdata"))
}

fn load(name: &str) -> Option<Value> {
    let s = std::fs::read_to_string(testdata_dir().join(name)).ok()?;
    serde_json::from_str(&s).ok()
}

fn f64s(v: &Value) -> Vec<f64> {
    fn walk(v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            x => out.push(x.as_f64().unwrap_or(f64::NAN)),
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

fn hint_of(v: &Value) -> Option<(f64, f64, f64)> {
    let a = f64s(v);
    (a.len() == 3).then(|| (a[0], a[1], a[2]))
}

/// 系列 ids だけの目録(Python の restrict)。
fn restricted(ids: &[u32]) -> Catalog {
    let mut c = Catalog::scan(index_dir());
    let files = std::mem::take(&mut c.files);
    c.series.clear();
    for f in files {
        if ids.contains(&f.indexid) {
            let k = c.files.len();
            c.series.entry(f.indexid).or_default().push(k);
            c.files.push(f);
        }
    }
    c
}

/// 1 回解く(締め切り sec 秒)。戻り値 (解, 秒)。
fn solve_timed(req: &SolveRequest, cat: &Catalog, sec: f64) -> (Option<Solution>, f64, bool) {
    let cancel = AtomicBool::new(false);
    let log = |_: &str| {};
    let ctl = Control { deadline: Some(Instant::now() + Duration::from_secs_f64(sec)), cancel: &cancel, log: &log };
    let t0 = Instant::now();
    let r = zahyou_solver::solve(req, cat, &Params::default(), &ctl);
    let dt = t0.elapsed().as_secs_f64();
    match r {
        Ok(s) => (s, dt, false),
        Err(_) => (None, dt, true),
    }
}

struct Row {
    name: String,
    what: String,
    rust_s: f64,
    python_s: Option<f64>,
    note: String,
    same: Option<bool>,
}

#[allow(clippy::too_many_arguments)]
fn bench_req(
    rows: &mut Vec<Row>,
    name: &str,
    what: &str,
    req: &SolveRequest,
    cat: &Catalog,
    repeat: usize,
    py_s: Option<f64>,
    py_index: Option<&str>,
    py_lo: Option<f64>,
) {
    let mut best = f64::INFINITY;
    let mut last = None;
    let mut err = false;
    for k in 0..repeat.max(1) {
        let (s, dt, e) = solve_timed(req, cat, 600.0);
        best = best.min(dt);
        err |= e;
        last = s;
        // 1 回で 20 秒を超えるものは繰り返さない
        if k == 0 && dt > 20.0 {
            break;
        }
    }
    let (note, same) = match &last {
        Some(s) => {
            let same = py_index.map(|pi| pi == s.index_name && py_lo.is_some_and(|l| (l - s.logodds).abs() <= 1e-6));
            (format!("{} 対数オッズ {:.2}・一致 {}・quad {}・仮説 {}", s.index_name, s.logodds, s.nmatch, s.quads_tried, s.hypotheses), same)
        }
        None => (if err { "エラー(中止)".to_string() } else { "解けず".to_string() }, py_index.map(|_| false)),
    };
    rows.push(Row { name: name.into(), what: what.into(), rust_s: best, python_s: py_s, note, same });
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut repeat = 3usize;
    let mut out_path: Option<PathBuf> = None;
    let mut pick: Option<Vec<String>> = None;
    let mut py_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repeat" => {
                repeat = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(3);
                i += 1;
            }
            "--out" => {
                out_path = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--python" => {
                py_path = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--cases" => {
                pick = args.get(i + 1).map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
                i += 1;
            }
            a if a.ends_with(".json") => out_path = Some(PathBuf::from(a)),
            a => {
                eprintln!("知らない引数: {a}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let want = |n: &str| pick.as_ref().is_none_or(|p| p.iter().any(|x| x == n));
    let t_scan = Instant::now();
    let cat = Catalog::scan(index_dir());
    let scan_s = t_scan.elapsed().as_secs_f64();
    if cat.files.is_empty() {
        eprintln!("星図データが無い({})", index_dir().display());
        return ExitCode::from(1);
    }
    eprintln!("星図データ {} 本(走査 {:.1} ms)、repeat {repeat}", cat.files.len(), scan_s * 1e3);
    let mut rows: Vec<Row> = Vec::new();
    // bench_solver.py の結果(名前 → 秒)
    let py_meas: std::collections::HashMap<String, f64> = match &py_path {
        None => Default::default(),
        Some(p) => match std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            Some(d) => d["rows"]
                .as_array()
                .map(|a| a.iter().filter_map(|r| Some((r["name"].as_str()?.to_string(), r["python_seconds"].as_f64()?))).collect())
                .unwrap_or_default(),
            None => {
                eprintln!("{} を読めない", p.display());
                return ExitCode::from(1);
            }
        },
    };

    // ---- 基準の実写(星のリストで 3 条件) ----
    if let Some(real) = load("solver_real.json")
        && let Some(img) = real["images"].as_array().and_then(|a| a.iter().find(|i| i.get("bench").is_some_and(|b| b.is_array())))
    {
        let w = img["width"].as_u64().unwrap() as usize;
        let h = img["height"].as_u64().unwrap() as usize;
        let stars: Vec<FieldStar> = f64s(&img["stars_deep"]).chunks(3).map(|c| FieldStar { x: c[0], y: c[1], flux: c[2] }).collect();
        for b in img["bench"].as_array().unwrap() {
            let label = b["label"].as_str().unwrap_or("?");
            let name = format!("{}:{label}", img["tag"].as_str().unwrap_or("?"));
            if !want(&name) && !want(label) && pick.is_some() {
                continue;
            }
            let req = SolveRequest {
                stars: stars.clone(),
                width: w,
                height: h,
                scale_lo: b["scale_lo"].as_f64(),
                scale_hi: b["scale_hi"].as_f64(),
                hint: hint_of(&b["hint"]),
            };
            let what = format!("{w}×{h} 実写・深い検出の星 {} 個・{}", stars.len(), match label {
                "blind" => "ブラインド",
                "hint" => "ヒント 5°",
                "scale" => "スケール既知",
                _ => label,
            });
            bench_req(&mut rows, &name, &what, &req, &cat, repeat, b["python_seconds"].as_f64(), b["index"].as_str(), b["logodds"].as_f64());
        }
        // 利用者の経路(画像があれば。検出までは時間に入れない)
        let rel = img["path"].as_str().unwrap_or("");
        let base = std::env::var_os("ZAHYOU_TEST_IMAGES")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_default();
        let path = rel.strip_prefix("$ZAHYOU_TEST_IMAGES/").map(|r| r.split('/').fold(base, |p, s| p.join(s)));
        if let Some(path) = path.filter(|p| p.is_file())
            && (pick.is_none() || want("solve_offline"))
            && let Ok(l) = load_image_any(&path)
        {
            let prep = preprocess(&l.data, true);
            let (src, _) = detect_sources(&prep, &DetectOpts::with_min_sources(60));
            let mut best = f64::INFINITY;
            let mut note = String::new();
            for _ in 0..repeat.max(1) {
                let req = OfflineRequest { prep: &prep, sources: &src, width: w, height: h, scale: img["pixel_scale"].as_f64(), hint: None, timeout_s: 300.0 };
                let cancel = AtomicBool::new(false);
                let log = |_: &str| {};
                let ctl = Control { deadline: None, cancel: &cancel, log: &log };
                let t0 = Instant::now();
                let r = zahyou_solver::solve_offline(&req, &cat, &Params::default(), &ctl);
                best = best.min(t0.elapsed().as_secs_f64());
                note = match r {
                    Ok(Some(s)) => format!("{} 対数オッズ {:.2}(深い検出は Rust の zahyou-image)", s.index_name, s.logodds),
                    Ok(None) => "解けず".into(),
                    Err(e) => format!("{e:?}"),
                };
            }
            rows.push(Row {
                name: format!("{}:solve_offline", img["tag"].as_str().unwrap_or("?")),
                what: format!("{w}×{h} 実写・利用者の経路(ヒントなし・焦点距離なし。検出は含めない)"),
                rust_s: best,
                python_s: img["solve_offline"]["seconds"].as_f64(),
                note,
                same: None,
            });
        }
    } else {
        eprintln!("solver_real.json に bench が無い(実写の 3 条件は飛ばす)");
    }

    // ---- 合成の難しい場合 ----
    if let Some(syn) = load("solver_synth.json") {
        let names = ["ra0", "tile_corner", "polein", "few20", "bb", "w25deg", "many300", "rot090"];
        for c in syn["cases"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap_or("");
            if !names.contains(&name) || !want(name) || c.get("skipped").is_some() {
                continue;
            }
            let p: Vec<f64> = f64s(&c["xy"]);
            let f: Vec<f64> = f64s(&c["flux"]);
            let stars: Vec<FieldStar> = p.chunks(2).zip(f).map(|(xy, fl)| FieldStar { x: xy[0], y: xy[1], flux: fl }).collect();
            let w = c["width"].as_u64().unwrap() as usize;
            let h = c["height"].as_u64().unwrap() as usize;
            let req = SolveRequest { stars, width: w, height: h, scale_lo: c["scale_lo"].as_f64(), scale_hi: c["scale_hi"].as_f64(), hint: hint_of(&c["hint"]) };
            let only: Vec<u32> = c["only"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as u32).collect()).unwrap_or_default();
            let rc;
            let cref = if only.is_empty() {
                &cat
            } else {
                rc = restricted(&only);
                &rc
            };
            let py = &c["python"];
            let what = format!("{w}×{h} 合成・{}", c["what"].as_str().unwrap_or(""));
            bench_req(&mut rows, name, &what, &req, cref, repeat, py["seconds"].as_f64(), py["solution"]["index"].as_str(), py["solution"]["logodds"].as_f64());
        }
        // 解けない星野の締め切りの超過
        if let Some(c) = syn["cases"].as_array().unwrap().iter().find(|c| c["name"] == "rand60")
            && want("rand60")
        {
            let p: Vec<f64> = f64s(&c["xy"]);
            let f: Vec<f64> = f64s(&c["flux"]);
            let stars: Vec<FieldStar> = p.chunks(2).zip(f).map(|(xy, fl)| FieldStar { x: xy[0], y: xy[1], flux: fl }).collect();
            let req = SolveRequest { stars, width: c["width"].as_u64().unwrap() as usize, height: c["height"].as_u64().unwrap() as usize, ..Default::default() };
            let (s, dt, _) = solve_timed(&req, &cat, 5.0);
            rows.push(Row {
                name: "rand60:deadline".into(),
                what: "乱数の星 60 個・ブラインド・締め切り 5 秒".into(),
                rust_s: dt,
                python_s: None,
                note: format!("{}、超過 {:+.3} 秒", if s.is_some() { "解が出た(誤り)" } else { "解なし" }, dt - 5.0),
                same: None,
            });
        }
    }

    // Python の秒を測り直した値に差し替える
    let mut replaced = 0;
    for r in rows.iter_mut() {
        if let Some(&s) = py_meas.get(&r.name) {
            r.python_s = Some(s);
            replaced += 1;
        }
    }
    if py_path.is_some() {
        eprintln!("Python の秒: bench_solver.py の測り直し {replaced} 項目、残りは JSON の参考値");
    }

    // ---- 表 ----
    println!("{:<26} {:>10} {:>10} {:>8}  内容", "項目", "Rust [s]", "Python [s]", "倍率");
    let mut arr = Vec::new();
    for r in &rows {
        // 締め切りまで探す項目は倍率を出さない(どちらも締め切りの秒になる)
        let ratio = r.python_s.filter(|_| !r.name.ends_with(":deadline")).map(|p| p / r.rust_s);
        println!(
            "{:<26} {:>10.4} {:>10} {:>8}  {}  {}{}",
            r.name,
            r.rust_s,
            r.python_s.map(|p| format!("{p:.3}")).unwrap_or_else(|| "-".into()),
            ratio.map(|x| format!("{x:.1}×")).unwrap_or_else(|| "-".into()),
            r.what,
            r.note,
            match r.same {
                Some(true) => "(Python と同じ解)",
                Some(false) => "(Python と違う)",
                None => "",
            }
        );
        arr.push(json!({"name": r.name, "what": r.what, "rust_seconds": r.rust_s, "python_seconds": r.python_s,
                        "speedup": ratio, "note": r.note, "same_as_python": r.same}));
    }
    let doc = json!({"index_dir": index_dir().display().to_string(), "index_files": cat.files.len(), "repeat": repeat, "rows": arr});
    if let Some(p) = out_path {
        if let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(&doc).unwrap()) {
            eprintln!("{} に書けない: {e}", p.display());
            return ExitCode::from(1);
        }
        eprintln!("書いた: {}", p.display());
    }
    ExitCode::SUCCESS
}

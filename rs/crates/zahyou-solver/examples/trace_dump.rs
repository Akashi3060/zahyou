//! 前後比較の物差し: 合成星野と実写の星のリスト(rs/testdata の solver_synth.json・solver_real.json)を解き、
//! trace(途中の値)から時刻だけを除いた JSON を 1 件 1 ファイルに書く。
//!
//! 解法の中を速くしたとき、変える前と変えた後で同じフォルダー構成に書かせ、ファイルをバイト単位で比べる
//! (束ごとの quad・当たり・仮説の数、足切りの数、先頭 20000 個の仮説の 4 点の WCS と対数オッズ、tune-up、解の WCS と一致)。
//! 実数は serde_json の最短の桁で書くので、解の WCS・対数オッズ・一致のずれはビットまで比べられる
//! (仮説の対数オッズとスケールだけは trace の約束どおり 4 桁・6 桁に丸めてある)。
//!
//! ```text
//! cargo run --release -p zahyou-solver --example trace_dump -- --out <フォルダー> [--cases ra0,s5202] [--deadline 秒]
//! ```
//!
//! 締め切りで止まった場合("status": "deadline")は、どこで止まったかが時間で決まるので、比べる側は
//! 仮説の記録の共通の先頭だけを比べること。星図データは C:\AstrometryData(ZAHYOU_INDEX_DIR)。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::Value;
use zahyou_index::Catalog;
use zahyou_solver::{Control, FieldStar, Params, SolveOptions, SolveRequest};

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

fn opt_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn hint_of(v: &Value) -> Option<(f64, f64, f64)> {
    let a = f64s(v);
    (a.len() == 3).then(|| (a[0], a[1], a[2]))
}

fn stars_xy_flux(xy: &Value, flux: &Value) -> Vec<FieldStar> {
    // 試験(tests/common の stars_xy_flux)と同じ組み立て
    let p = f64s(xy);
    let f = if flux.is_null() { vec![f64::NAN; p.len() / 2] } else { f64s(flux) };
    p.chunks(2).zip(f).map(|(q, fl)| FieldStar { x: q[0], y: q[1], flux: fl }).collect()
}

fn stars_xyf(v: &Value) -> Vec<FieldStar> {
    f64s(v).chunks(3).map(|q| FieldStar { x: q[0], y: q[1], flux: q[2] }).collect()
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

/// 1 件を解いて、時刻を除いた trace を out に書く。戻り値は 1 行の要約。
fn dump_one(name: &str, req: &SolveRequest, cat: &Catalog, deadline_s: f64, out: &Path) -> String {
    let tmp = out.join(format!("{name}.raw.json"));
    let _ = std::fs::remove_file(&tmp);
    let cancel = AtomicBool::new(false);
    let log = |_: &str| {};
    let ctl =
        Control { deadline: Some(Instant::now() + Duration::from_secs_f64(deadline_s)), cancel: &cancel, log: &log };
    let opts = SolveOptions { trace: Some(tmp.clone()) };
    let t0 = Instant::now();
    let r = zahyou_solver::solve_detailed(req, cat, &Params::default(), &ctl, &opts);
    let dt = t0.elapsed().as_secs_f64();
    let Some(mut t) = std::fs::read_to_string(&tmp).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
        return format!("{name}: trace が無い({r:?})");
    };
    let _ = std::fs::remove_file(&tmp);
    if let Some(o) = t.as_object_mut() {
        o.remove("seconds");
        if let Some(Value::Array(fl)) = o.get_mut("flushes") {
            for f in fl {
                if let Some(m) = f.as_object_mut() {
                    m.remove("t");
                }
            }
        }
        // solve_detailed の解(trace の solution と同じ値のはずだが、一致の並びもビットで残す)
        if let Ok(Some(d)) = &r {
            let m: Vec<Value> = d
                .matches
                .iter()
                .map(|m| {
                    serde_json::json!([m.field, m.star.map(f64::to_bits), m.dist_px.to_bits()])
                })
                .collect();
            o.insert("detail_matches_bits".into(), Value::Array(m));
            o.insert("detail_logodds_bits".into(), serde_json::json!(d.solution.logodds.to_bits()));
        }
    }
    let status = t["status"].as_str().unwrap_or("?").to_string();
    let text = serde_json::to_string(&t).unwrap_or_default();
    let _ = std::fs::write(out.join(format!("{name}.json")), text);
    format!("{name}: {status}・仮説 {}・{dt:.2} 秒", t["hypotheses"])
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out: Option<PathBuf> = None;
    let mut cases: Option<Vec<String>> = None;
    let mut deadline_s = 120.0;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                out = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--cases" => {
                cases = args.get(i + 1).map(|s| s.split(',').map(str::to_string).collect());
                i += 1;
            }
            "--deadline" => {
                deadline_s = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(deadline_s);
                i += 1;
            }
            a => {
                eprintln!("知らない引数 {a}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }
    let Some(out) = out else {
        eprintln!("--out <フォルダー> が要る");
        return ExitCode::FAILURE;
    };
    let _ = std::fs::create_dir_all(&out);
    let want = |n: &str| cases.as_ref().is_none_or(|c| c.iter().any(|x| x == n));
    let all = Catalog::scan(index_dir());
    if let Some(doc) = load("solver_synth.json") {
        for c in doc["cases"].as_array().into_iter().flatten() {
            let name = c["name"].as_str().unwrap_or("?");
            if !want(name) || c.get("skipped").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let only: Vec<u32> = c["only"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as u32).collect())
                .unwrap_or_default();
            let req = SolveRequest {
                stars: stars_xy_flux(&c["xy"], &c["flux"]),
                width: c["width"].as_u64().unwrap_or(0) as usize,
                height: c["height"].as_u64().unwrap_or(0) as usize,
                scale_lo: opt_f64(&c["scale_lo"]),
                scale_hi: opt_f64(&c["scale_hi"]),
                hint: hint_of(&c["hint"]),
            };
            let line = if only.is_empty() {
                dump_one(name, &req, &all, deadline_s, &out)
            } else {
                let r = restricted(&only);
                let l = dump_one(name, &req, &r, deadline_s, &out);
                r.close();
                l
            };
            println!("{line}");
        }
    }
    if let Some(doc) = load("solver_real.json") {
        for img in doc["images"].as_array().into_iter().flatten() {
            let name = img["name"].as_str().unwrap_or("?");
            if !want(name) || img.get("skipped").is_some_and(|v| !v.is_null()) {
                continue;
            }
            for a in img["attempts"].as_array().into_iter().flatten() {
                let stars = match a["stars"].as_str() {
                    Some("deep") => stars_xyf(&img["stars_deep"]),
                    _ => match img.get("stars_standard") {
                        Some(v) if !v.is_null() => stars_xyf(v),
                        _ => continue,
                    },
                };
                let req = SolveRequest {
                    stars,
                    width: img["width"].as_u64().unwrap_or(0) as usize,
                    height: img["height"].as_u64().unwrap_or(0) as usize,
                    scale_lo: opt_f64(&a["scale_lo"]),
                    scale_hi: opt_f64(&a["scale_hi"]),
                    hint: hint_of(&a["hint"]),
                };
                let tag = format!("{name}#{}", a["attempt"]);
                println!("{}", dump_one(&tag, &req, &all, deadline_s, &out));
            }
        }
    }
    all.close();
    ExitCode::SUCCESS
}

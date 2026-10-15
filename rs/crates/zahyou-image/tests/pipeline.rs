//! 実画像の通しの照合(rs/testdata/image_pipeline.json、gen_image.py の gen_pipeline)。
//!
//! dev/tcase の全画像と手元の実写について、読み込み・前処理(焼き込み文字のマスクあり/なし)・
//! 検出(min_sources 12 / 60)・深い検出の結果を Python と比べる。画像が手元に無い(または中身が違う)ときは SKIP。
//! 許す差: 星の位置 1e-4 px、σ 相対 1e-6、光量 相対 1e-9、伸び・FWHM・SNR 相対 1e-6(実際はほぼビット一致)。

mod common;

use std::path::PathBuf;
use std::time::Instant;

use common::*;
use zahyou_image::{DetectOpts, Source, detect_sources, detect_sources_deep, load_image_any, preprocess};

const POS_TOL: f64 = 1e-4;
const SIGMA_TOL: f64 = 1e-6;

fn star_issues(label: &str, py: &Value, got: &(Vec<Source>, Option<f64>), worst: &mut [f64; 2]) -> Vec<String> {
    let mut out = Vec::new();
    let pn = py["n"].as_u64().unwrap() as usize;
    if pn != got.0.len() {
        out.push(format!("{label}: 星の数 {} ≠ {pn}", got.0.len()));
    }
    let pthr = py["thr"].as_f64();
    if pthr != got.1 {
        out.push(format!("{label}: しきい値 {:?} ≠ {pthr:?}", got.1));
    }
    for (k, (p, s)) in py["stars"].as_array().unwrap().iter().zip(got.0.iter()).enumerate() {
        let p: Vec<f64> = p.as_array().unwrap().iter().map(f).collect();
        let dpos = (p[0] - s.x).abs().max((p[1] - s.y).abs());
        worst[0] = worst[0].max(dpos);
        worst[1] = worst[1].max(rel(p[2], s.flux));
        let bad = dpos > POS_TOL
            || rel(p[2], s.flux) > 1e-9
            || p[3] != s.peak
            || p[4] as usize != s.area
            || rel(p[5], s.elong) > 1e-6
            || rel(p[6], s.fwhm) > 1e-6
            || rel(p[7], s.snr) > 1e-6;
        if bad {
            out.push(format!(
                "{label}: 星 #{k} Python {p:?} / Rust ({}, {}, {}, {}, {}, {}, {}, {})",
                s.x, s.y, s.flux, s.peak, s.area, s.elong, s.fwhm, s.snr
            ));
            if out.len() > 6 {
                break;
            }
        }
    }
    out
}

#[test]
fn pipeline_matches_python() {
    let j = load_json("image_pipeline.json");
    let dev = zahyou_dev();
    let mut issues: Vec<String> = Vec::new();
    let mut n_run = 0;
    let mut n_skip = 0;
    let mut worst = [0.0f64; 2];
    let mut sigma_worst = 0.0f64;
    let mut t_rs = [0.0f64; 4];
    let mut t_py = [0.0f64; 4];
    println!(
        "{:44} {:>8} {:>8} {:>8} {:>8}  (Rust 秒 / Python 秒: 読み込み 前処理 検出×2 深い検出)",
        "画像", "", "", "", ""
    );
    for it in j["items"].as_array().unwrap() {
        let name = it["name"].as_str().unwrap();
        let path: PathBuf = if it["base"] == "tcase" { dev.join("tcase").join(name) } else { PathBuf::from(name) };
        let short: String = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(bytes) = std::fs::read(&path) else {
            println!("  SKIP {short} (手元に無い)");
            n_skip += 1;
            continue;
        };
        if bytes.len() as u64 != it["file_size"].as_u64().unwrap()
            || ahash_u8(bytes.iter().copied()) != it["file_hash"].as_str().unwrap()
        {
            println!("  SKIP {short} (Python で値を作ったときと中身が違う)");
            n_skip += 1;
            continue;
        }
        drop(bytes);
        let t0 = Instant::now();
        let r = load_image_any(&path);
        let t_load = t0.elapsed().as_secs_f64();
        let py_ok = it["ok"].as_bool().unwrap();
        let l = match (py_ok, r) {
            (false, Err(e)) => {
                println!("  {short:42} Err (Python も Err): {e}");
                n_run += 1;
                continue;
            }
            (false, Ok(l)) => {
                issues.push(format!("{short}: Python は読めないのに Rust は読めた ({})", l.note));
                continue;
            }
            (true, Err(e)) => {
                issues.push(format!("{short}: Python は読めたのに Rust は Err: {e}"));
                continue;
            }
            (true, Ok(l)) => l,
        };
        n_run += 1;
        let mut mine: Vec<String> = Vec::new();
        let shape = (l.data.height, l.data.width);
        if l.note != it["note"].as_str().unwrap() {
            mine.push(format!("{short}: note '{}' ≠ '{}'", l.note, it["note"].as_str().unwrap()));
        }
        if l.flip_y != it["flip_y"].as_bool().unwrap() {
            mine.push(format!("{short}: flip_y が違う"));
        }
        let data_diff = cmp_stats(&format!("{short}.data"), &it["data"], shape, &l.data.data);
        mine.extend(data_diff);
        mine.extend(cmp_stats(&format!("{short}.display"), &it["display"], shape, &l.display.data));

        let t1 = Instant::now();
        let prep = preprocess(&l.data, true);
        let t_prep = t1.elapsed().as_secs_f64();
        let pp = &it["prep"];
        let ds = rel(prep.sigma, f(&pp["sigma"]));
        sigma_worst = sigma_worst.max(ds);
        let mask_count = prep.mask.iter().filter(|&&m| m).count();
        let check_prep = |mine: &mut Vec<String>| {
            if ds > SIGMA_TOL {
                mine.push(format!("{short}: σ {} ≠ {} (相対 {ds:.2e})", prep.sigma, f(&pp["sigma"])));
            }
            if mask_count as u64 != pp["mask_count"].as_u64().unwrap() {
                mine.push(format!("{short}: マスク {mask_count} ≠ {}", pp["mask_count"]));
            }
            if ahash_u8(prep.mask.iter().map(|&m| m as u8)) != pp["mask_hash"].as_str().unwrap() {
                mine.push(format!("{short}: マスクの位置が違う"));
            }
            let info: Vec<&str> = pp["info"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            if prep.info.iter().map(|s| s.as_str()).collect::<Vec<_>>() != info {
                mine.push(format!("{short}: info {:?} ≠ {info:?}", prep.info));
            }
            mine.extend(cmp_stats(&format!("{short}.sub"), &pp["sub"], shape, &prep.sub.data));
        };
        let p2 = preprocess(&l.data, false);
        let pn = &it["prep_no_overlay"];
        let t2 = Instant::now();
        let d12 = detect_sources(&prep, &DetectOpts::default());
        let d60 = detect_sources(&prep, &DetectOpts::with_min_sources(60));
        let t_det = t2.elapsed().as_secs_f64();
        let t3 = Instant::now();
        let deep = detect_sources_deep(&prep, &DetectOpts::deep());
        let t_deep = t3.elapsed().as_secs_f64();

        check_prep(&mut mine);
        if rel(p2.sigma, f(&pn["sigma"])) > SIGMA_TOL {
            mine.push(format!("{short}: (マスクなし) σ {} ≠ {}", p2.sigma, f(&pn["sigma"])));
        }
        if p2.mask.iter().filter(|&&m| m).count() as u64 != pn["mask_count"].as_u64().unwrap() {
            mine.push(format!("{short}: (マスクなし) マスクの数が違う"));
        }
        mine.extend(star_issues(&format!("{short} det12"), &it["det12"], &d12, &mut worst));
        mine.extend(star_issues(&format!("{short} det60"), &it["det60"], &d60, &mut worst));
        mine.extend(star_issues(&format!("{short} deep"), &it["deep"], &deep, &mut worst));

        let ps = &it["py_seconds"];
        let pyt = [f(&ps["load"]), f(&ps["preprocess"]), f(&ps["detect_x2"]), f(&ps["deep"])];
        let rst = [t_load, t_prep, t_det, t_deep];
        for k in 0..4 {
            t_rs[k] += rst[k];
            t_py[k] += pyt[k];
        }
        let status = if mine.is_empty() { "一致" } else { "✗" };
        println!(
            "  {short:42} {status} 星 {}・{}・{}  {:.3}/{:.3} {:.3}/{:.3} {:.3}/{:.3} {:.3}/{:.3}",
            d12.0.len(),
            d60.0.len(),
            deep.0.len(),
            rst[0],
            pyt[0],
            rst[1],
            pyt[1],
            rst[2],
            pyt[2],
            rst[3],
            pyt[3]
        );
        for m in &mine {
            println!("      ✗ {m}");
        }
        issues.extend(mine);
    }
    println!(
        "通し {n_run} 件、SKIP {n_skip} 件、食い違い {} 件。星の位置の差 最大 {:.2e} px、光量の相対差 最大 {:.2e}、σ の相対差 最大 {:.2e}",
        issues.len(),
        worst[0],
        worst[1],
        sigma_worst
    );
    println!(
        "合計時間 Rust / Python: 読み込み {:.3}/{:.3} 秒、前処理 {:.3}/{:.3} 秒、検出×2 {:.3}/{:.3} 秒、深い検出 {:.3}/{:.3} 秒(試験は opt-level 2 のビルド)",
        t_rs[0], t_py[0], t_rs[1], t_py[1], t_rs[2], t_py[2], t_rs[3], t_py[3]
    );
    assert!(issues.is_empty(), "食い違い:\n{}", issues.join("\n"));
}

/// 6000×4000 の合成画像(gen_image.py --big DIR で作る。critic の prof_big.py と同じ作り方)。
/// 環境変数 ZAHYOU_BIG_DIR に DIR を入れたときだけ回す(無ければ SKIP)。
#[test]
fn big_synthetic_matches_python() {
    let Some(dir) = std::env::var_os("ZAHYOU_BIG_DIR").map(PathBuf::from) else {
        println!("SKIP: ZAHYOU_BIG_DIR が無い(gen_image.py --big <DIR> で作り、そのフォルダーを入れる)");
        return;
    };
    let Ok(s) = std::fs::read_to_string(dir.join("big_expected.json")) else {
        println!("SKIP: {} に big_expected.json が無い", dir.display());
        return;
    };
    let j: Value = serde_json::from_str(&s).unwrap();
    let path = dir.join("big6000x4000.fits");
    let t0 = Instant::now();
    let l = load_image_any(&path).expect("読み込み");
    let t_load = t0.elapsed().as_secs_f64();
    let shape = (l.data.height, l.data.width);
    let mut issues: Vec<String> = Vec::new();
    issues.extend(cmp_stats("load", &j["load_data"], shape, &l.data.data));
    let t1 = Instant::now();
    let prep = preprocess(&l.data, true);
    let t_prep = t1.elapsed().as_secs_f64();
    if rel(prep.sigma, f(&j["sigma"])) > SIGMA_TOL {
        issues.push(format!("σ {} ≠ {}", prep.sigma, f(&j["sigma"])));
    }
    if prep.mask.iter().filter(|&&m| m).count() as u64 != j["mask_count"].as_u64().unwrap() {
        issues.push("マスクの数が違う".into());
    }
    let info: Vec<&str> = j["info"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    if prep.info.iter().map(|s| s.as_str()).collect::<Vec<_>>() != info {
        issues.push(format!("info {:?} ≠ {info:?}", prep.info));
    }
    issues.extend(cmp_stats("sub", &j["sub"], shape, &prep.sub.data));
    let mut worst = [0.0f64; 2];
    let t2 = Instant::now();
    let d12 = detect_sources(&prep, &DetectOpts::default());
    let t_det = t2.elapsed().as_secs_f64();
    let d60 = detect_sources(&prep, &DetectOpts::with_min_sources(60));
    let t3 = Instant::now();
    let deep = detect_sources_deep(&prep, &DetectOpts::deep());
    let t_deep = t3.elapsed().as_secs_f64();
    issues.extend(star_issues("det12", &j["det12"], &d12, &mut worst));
    issues.extend(star_issues("det60", &j["det60"], &d60, &mut worst));
    issues.extend(star_issues("deep", &j["deep"], &deep, &mut worst));
    let ps = &j["py_seconds"];
    println!(
        "6000×4000: 食い違い {} 件、星の位置の差 最大 {:.2e} px。Rust / Python 秒: 読み込み {t_load:.3}/{:.3} 前処理 {t_prep:.3}/{:.3} 検出 {t_det:.3}/{:.3} 深い検出 {t_deep:.3}/{:.3}(試験は opt-level 2)",
        issues.len(),
        worst[0],
        f(&ps["load"]),
        f(&ps["preprocess"]),
        f(&ps["detect12"]),
        f(&ps["deep"])
    );
    assert!(issues.is_empty(), "{}", issues.join("\n"));
}

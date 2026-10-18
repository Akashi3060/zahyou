//! 実写(dev/solver_truth.json の陽性の画像)を解く。
//!
//! 1. 星のリストで(画像が無くても回る): rs/testdata/solver_real.json に Python 基準実装の検出の星
//!    (load_image_any → preprocess → detect_sources_deep / detect_sources(min_sources=60))と、solve_offline と同じ
//!    試行のはしごを sv_solve で順に解いた結果が入っている。同じ星・同じ条件で zahyou_solver::solve を呼び、
//!    Python と同じ答え(同じ索引・対数オッズ 1e-6・一致数・最終の WCS 1e-6 px・試した quad の数・仮説の数)か、
//!    Python が quad を試し尽くして解けなかった試行は Rust も解なしか、を確かめる。解法の crate が trace を出せれば
//!    途中の値(解いた quad の 4 点・束ごとの数・最初の 24 個の仮説の WCS と対数オッズ)も Python と比べる。続いて正解(solver_truth.json の
//!    truth。criteria に criteria_override を重ね、accept に truth_tan があればそれも合格)に対して設計書 8 節の基準。
//! 2. 利用者の経路で(画像が手元にあるときだけ): zahyou-image で読み込み・前処理・検出(min_sources 60)をして
//!    zahyou_solver::solve_offline。正解の基準と、基準の画像は目標までのずれ(1.583′・0.918′ 東・1.290′ 北)も。
//!    Rust の深い検出は Python と同じとは限らない(この試験は正解とだけ比べる)。
//!
//! ```text
//! cargo test -p zahyou-solver --test real -- --nocapture             基準の画像 real_orig(+ヒント)だけ
//! cargo test -p zahyou-solver --test real -- --ignored --nocapture   全部
//! ```
//! 画像は $ZAHYOU_TEST_IMAGES(既定はホームフォルダー)かリポジトリからの相対(zahyou-rs に無ければ ZAHYOU_PY_REPO)。
//! 星図データ・基準値・画像が無いものは SKIP。解けてはいけない画像は tests/negative.rs。

mod common;

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use common::*;
use zahyou_image::{DetectOpts, detect_sources, load_image_any, preprocess};
use zahyou_solver::{Control, OfflineRequest, Params, SolveDetail, SolveRequest};

const QUICK: &[&str] = &["real_orig", "real_orig+hint"];

/// solver_truth.json の画像の項目(tag で引く)。
fn truth_entry<'a>(doc: &'a Value, tag: &str) -> Option<&'a Value> {
    doc["images"].as_array()?.iter().find(|e| e["tag"].as_str() == Some(tag))
}

/// 解の WCS を正解と比べる(Python の test_solver_real.evaluate と同じ。truth で外れ、accept に truth_tan があれば
/// truth_tan でも比べる)。
pub fn judge_truth(wcs: &zahyou_wcs::Wcs, e: &Value, crit: &Value) -> Result<String, String> {
    let w = e["width"].as_u64().unwrap() as usize;
    let h = e["height"].as_u64().unwrap() as usize;
    let tol = Tol::from_criteria(crit, e.get("criteria_override"));
    let want = truth_measure(&e["truth"]);
    let Some(cmp) = compare_wcs(wcs, w, h, &want, tol) else {
        return Err("解の WCS を測れない".into());
    };
    if cmp.ok {
        return Ok(describe(&cmp));
    }
    let accept_tan = e["accept"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some("truth_tan")));
    if accept_tan && e.get("truth_tan").is_some_and(|t| !t.is_null()) {
        let want2 = truth_measure(&e["truth_tan"]);
        if let Some(c2) = compare_wcs(wcs, w, h, &want2, tol)
            && c2.ok
        {
            return Ok(format!("TAN の参考値に合格(正解とは {}): {}", describe(&cmp), describe(&c2)));
        }
    }
    Err(format!("外れ {:?}: {}", cmp.failed, describe(&cmp)))
}

/// 目標までのずれを基準値(小数 3 桁の表示)と比べる(Python の check_offsets と同じ。許容 2″ + 表示の丸め)。
fn check_offsets(base: &Value, wcs: &zahyou_wcs::Wcs, w: usize, h: usize) -> Result<String, String> {
    let c = wcs.pix_to_radec((w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0);
    let t = (base["ra_deg"].as_f64().unwrap(), base["dec_deg"].as_f64().unwrap());
    let (sep, de, dn) = zahyou_wcs::spherical_offsets(c, t);
    let am = |r: f64| r.to_degrees() * 60.0;
    let (sep, de, dn) = (am(sep), am(de), am(dn));
    let b = |k: &str| base[k].as_f64().unwrap();
    let d = (sep - b("sep_arcmin")).abs().max((de - b("east_arcmin")).abs()).max((dn - b("north_arcmin")).abs());
    let same = (format!("{sep:.3}"), format!("{de:.3}"), format!("{dn:.3}"))
        == (format!("{:.3}", b("sep_arcmin")), format!("{:.3}", b("east_arcmin")), format!("{:.3}", b("north_arcmin")));
    let msg = format!(
        "目標まで {sep:.3}′・{de:.3}′ 東・{dn:.3}′ 北(差 ≤ {:.2}″、表示{})",
        d * 60.0,
        if same { "も一致" } else { "は違う" }
    );
    if d <= 2.0 / 60.0 + 5e-4 { Ok(msg) } else { Err(msg) }
}

/// 星のリストで、試行のはしごを Python と同じ順に解く。
fn stars_case(img: &Value, truth_doc: &Value, bad: &mut Vec<String>) {
    let name = img["name"].as_str().unwrap_or("?");
    if let Some(why) = img.get("skipped").and_then(|v| v.as_str()) {
        skip(name, &format!("gen_solver.py: {why}"));
        return;
    }
    let tag = img["tag"].as_str().unwrap();
    let Some(e) = truth_entry(truth_doc, tag) else {
        bad.push(format!("{name}: solver_truth.json に無い"));
        return;
    };
    let crit = &truth_doc["criteria"];
    let w = img["width"].as_u64().unwrap() as usize;
    let h = img["height"].as_u64().unwrap() as usize;
    let cat = catalog().unwrap();
    for a in img["attempts"].as_array().unwrap() {
        let stars = match a["stars"].as_str() {
            Some("deep") => stars_xyf(&img["stars_deep"]),
            _ => match img.get("stars_standard") {
                Some(v) if !v.is_null() => stars_xyf(v),
                _ => {
                    bad.push(format!("{name}: 標準の検出の星が JSON に無い"));
                    return;
                }
            },
        };
        let label = format!("{name} [{}] {}", a["attempt"], a["label"].as_str().unwrap_or(""));
        let req = SolveRequest {
            stars,
            width: w,
            height: h,
            scale_lo: opt_f64(&a["scale_lo"]),
            scale_hi: opt_f64(&a["scale_hi"]),
            hint: hint_of(&a["hint"]),
        };
        let buf = LogBuf::default();
        let (res, dt, trace) = solve_traced(&req, &cat, &Params::default(), solve_deadline_s(), &buf);
        let status = a["status"].as_str().unwrap_or("");
        let det: Option<SolveDetail> = match res {
            Ok(s) => s,
            Err(err) => {
                row("FAIL", &label, &format!("エラー {err:?}"));
                bad.push(format!("{label}: エラー {err:?}"));
                return;
            }
        };
        let sol = det.as_ref().map(|d| &d.solution);
        match (status, sol) {
            ("solved", Some(s)) => {
                let d = detail_diff(det.as_ref().unwrap(), &a["trace"], w, h);
                if d.is_empty() {
                    row("PASS", &label, &format!("Python と一致({} 対数オッズ {:.2}・一致 {}、{dt:.3} 秒 / Python {:.3} 秒)",
                        s.index_name, s.logodds, s.nmatch, a["seconds"].as_f64().unwrap_or(f64::NAN)));
                } else {
                    row("FAIL", &label, &d.join("、"));
                    bad.push(format!("{label} Python: {}", d.join("、")));
                }
                if let Some(t) = &trace {
                    let d = trace_diff(t, &a["trace"]);
                    if d.is_empty() {
                        row("PASS", &format!("{name} trace"), "途中の値も一致");
                    } else {
                        row("FAIL", &format!("{name} trace"), &d.join("、"));
                        bad.push(format!("{label} trace: {}", d.join("、")));
                    }
                }
                match judge_truth(&s.wcs, e, crit) {
                    Ok(m) => row("PASS", &format!("{name} 正解"), &m),
                    Err(m) => {
                        row("FAIL", &format!("{name} 正解"), &m);
                        bad.push(format!("{name} 正解: {m}"));
                    }
                }
            }
            ("solved", None) => {
                let m = format!("解けなかった(Python は {} で解いた。{dt:.2} 秒、ログ: {})", a["trace"]["solution"]["index"], buf.tail(2));
                row("FAIL", &label, &m);
                bad.push(format!("{label}: {m}"));
            }
            ("unsolved", None) => {
                // quad を試し尽くしたので、試した quad と仮説の数も Python と同じはず
                let counts = trace.as_ref().map(|t| {
                    (t["quads_tried"].as_u64(), t["hypotheses"].as_u64())
                        == (a["quads_tried"].as_u64(), a["hypotheses"].as_u64())
                });
                let m = format!("Python と同じく解なし({dt:.2} 秒{})", match &trace {
                    Some(t) => format!("、quad {}・仮説 {} / Python {}・{}", t["quads_tried"], t["hypotheses"], a["quads_tried"], a["hypotheses"]),
                    None => String::new(),
                });
                if counts == Some(false) {
                    row("FAIL", &label, &m);
                    bad.push(format!("{label}: 試した数が違う {m}"));
                } else {
                    row("PASS", &label, &m);
                }
            }
            ("unsolved", Some(s)) => {
                bad.push(format!("{label}: Python は解なしなのに {} で解けた", s.index_name));
                row("FAIL", &label, "Python は解なしなのに解けた");
            }
            _ => row("----", &label, &format!("Python は {status}(突き合わせない)")),
        }
        if sol.is_some() {
            break; // 解けたらはしごを降りない(solve_offline と同じ)
        }
    }
}

/// 利用者の経路(zahyou-image → solve_offline)で 1 枚。
fn offline_case(img: &Value, truth_doc: &Value, bad: &mut Vec<String>) {
    let name = img["name"].as_str().unwrap_or("?");
    let tag = img["tag"].as_str().unwrap_or("?");
    let Some(e) = truth_entry(truth_doc, tag) else { return };
    let path = image_path(e["path"].as_str().unwrap());
    if !path.is_file() {
        skip(name, &format!("画像が無い({})", path.display()));
        return;
    }
    if sha256_file(&path).as_deref() != e["sha256"].as_str() {
        skip(name, "画像の中身が正解を作ったときと違う(sha256)");
        return;
    }
    let w = e["width"].as_u64().unwrap() as usize;
    let h = e["height"].as_u64().unwrap() as usize;
    let t0 = Instant::now();
    let loaded = match load_image_any(&path) {
        Ok(l) => l,
        Err(err) => {
            bad.push(format!("{name}: 読めない {err}"));
            return;
        }
    };
    if (loaded.data.width, loaded.data.height) != (w, h) {
        bad.push(format!("{name}: 画像の大きさ {}×{} が正解 {w}×{h} と違う", loaded.data.width, loaded.data.height));
        return;
    }
    let prep = preprocess(&loaded.data, true);
    let (src, _) = detect_sources(&prep, &DetectOpts::with_min_sources(60));
    let t_det = t0.elapsed().as_secs_f64();
    let timeout = solve_deadline_s();
    let req = OfflineRequest {
        prep: &prep,
        sources: &src,
        width: w,
        height: h,
        scale: opt_f64(&img["pixel_scale"]),
        hint: hint_of(&img["hint"]),
        timeout_s: timeout,
    };
    let cancel = AtomicBool::new(false);
    let buf = LogBuf::default();
    let log = |s: &str| buf.push(s);
    let ctl = Control { deadline: None, cancel: &cancel, log: &log };
    let t1 = Instant::now();
    let r = zahyou_solver::solve_offline(&req, &catalog().unwrap(), &Params::default(), &ctl);
    let dt = t1.elapsed().as_secs_f64();
    let label = format!("{name} 利用者の経路");
    let py_s = img["solve_offline"]["seconds"].as_f64().unwrap_or(f64::NAN);
    match r {
        Ok(Some(s)) => {
            let mut m = judge_truth(&s.wcs, e, &truth_doc["criteria"]);
            let base = &truth_doc["target_offset_baseline"];
            if base["image"].as_str() == Some(tag) {
                let o = check_offsets(base, &s.wcs, w, h);
                m = match (m, o) {
                    (Ok(a), Ok(b)) => Ok(format!("{a}  {b}")),
                    (Err(a), Ok(b)) | (Ok(b), Err(a)) => Err(format!("{a}  {b}")),
                    (Err(a), Err(b)) => Err(format!("{a}  {b}")),
                };
            }
            let tail = format!("  検出 {t_det:.3} 秒・解 {dt:.3} 秒(Python {py_s:.2} 秒)、星 {} 個、{}", src.len(), s.index_name);
            match m {
                Ok(m) => row("PASS", &label, &(m + &tail)),
                Err(m) => {
                    row("FAIL", &label, &(m.clone() + &tail));
                    bad.push(format!("{label}: {m}"));
                }
            }
        }
        Ok(None) => {
            let m = format!("解けなかった({dt:.2} 秒、ログ: {})", buf.tail(3));
            row("FAIL", &label, &m);
            bad.push(format!("{label}: {m}"));
        }
        Err(err) => {
            bad.push(format!("{label}: エラー {err:?}"));
        }
    }
}

fn run(names: Option<&[&str]>, label: &str, offline: bool) {
    let Some(doc) = load_testdata("solver_real.json") else {
        skip(label, "rs/testdata/solver_real.json が無い(gen_solver.py で作る)");
        return;
    };
    let Some(truth_doc) = load_truth() else {
        skip(label, "dev/solver_truth.json が無い");
        return;
    };
    if catalog().is_none() {
        skip(label, &format!("星図データが無い({})", index_dir().display()));
        return;
    }
    let _g = heavy();
    let mut bad = Vec::new();
    let mut n = 0;
    for img in doc["images"].as_array().unwrap() {
        let name = img["name"].as_str().unwrap_or("");
        if img["expect_solve"].as_bool() != Some(true) {
            continue;
        }
        if let Some(ns) = names
            && !ns.contains(&name)
        {
            continue;
        }
        n += 1;
        if offline {
            offline_case(img, &truth_doc, &mut bad);
        } else {
            stars_case(img, &truth_doc, &mut bad);
        }
    }
    eprintln!("  {label}: {n} 件、食い違い・外れ {} 件", bad.len());
    assert!(bad.is_empty(), "{} 件が合わない:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn real_stars_quick_match_python_and_truth() {
    run(Some(QUICK), "real_stars_quick", false);
}

#[test]
#[ignore]
fn real_stars_all_match_python_and_truth() {
    run(None, "real_stars_all", false);
}

#[test]
fn real_offline_quick_meets_truth() {
    run(Some(QUICK), "real_offline_quick", true);
}

#[test]
#[ignore]
fn real_offline_all_meets_truth() {
    run(None, "real_offline_all", true);
}

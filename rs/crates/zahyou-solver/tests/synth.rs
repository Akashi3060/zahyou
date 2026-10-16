//! 合成星野(rs/testdata/solver_synth.json。dev/solver_synth.py が種で作った正解つきの星野)を解き、
//! (1) Python 基準実装の sv_solve と同じ答えか(同じ索引・対数オッズ 1e-6・一致数・最終の WCS 1e-6 px・
//!     試した quad の数・検証した仮説の数)、(2) 正解に対して設計書 8 節の基準(中心 2″ か 0.5 px・スケール 0.2 %・
//!     回転 0.1°・鏡像)を満たすか、(3) 途中の値(trace)が Python と同じか(解いた quad の 4 点、使った索引、
//!     束ごとの quad・当たり・仮説の数、最初の 24 個の仮説の 4 点の WCS と対数オッズ、解いた仮説の順番)を確かめる。
//!     (3) は解法の crate が trace を出せるときだけ(出せなければ「----」と出して飛ばす)。
//!
//! ```text
//! cargo test -p zahyou-solver --test synth -- --nocapture                 速い場合だけ(Python で 2 秒未満)
//! cargo test -p zahyou-solver --test synth -- --ignored --nocapture       全部(ra0・tile_corner・polein・w25deg も)
//! ZAHYOU_SYNTH_CASES=ra0,few20 cargo test -p zahyou-solver --test synth -- --ignored --nocapture   名前で選ぶ
//! ```
//!
//! 星図データ(C:\AstrometryData。ZAHYOU_INDEX_DIR)か基準値の JSON が無ければ SKIP と出して通す。
//! Python が締め切りで止まった場合("status": "deadline")は Python との突き合わせをせず、正解との比較だけをする。
//! 乱数の星野(解けてはいけない)は tests/negative.rs。

mod common;

use common::*;
use zahyou_solver::{Params, SolveRequest};

/// 速い場合(Python の sv_solve で 2 秒未満。既定の cargo test で回す)。
const QUICK: &[&str] = &[
    "s5205", "s5206", "s4107", "s4110", "t4111", "t4113", "t4117", "t4119", "w10deg", "mir000", "mir152", "hs", "hb",
    "bs", "bb", "sip_pin", "sip_bar", "few20", "pole89_h", "south60_h",
];

/// 1 件の結果。
struct Outcome {
    name: String,
    /// Python と同じ答えか(None = 突き合わせない)
    python: Option<Result<String, String>>,
    /// 正解の基準を満たすか(None = 解けてはいけない場合)
    truth: Option<Result<String, String>>,
    /// 途中の値が Python と同じか(None = 突き合わせない・trace が無い)
    trace: Option<Result<String, String>>,
}

fn run_case(c: &Value) -> Outcome {
    let name = c["name"].as_str().unwrap_or("?").to_string();
    let mut out = Outcome { name: name.clone(), python: None, truth: None, trace: None };
    if let Some(why) = c.get("skipped").and_then(|v| v.as_str()) {
        out.python = Some(Ok(format!("SKIP(gen_solver.py: {why})")));
        return out;
    }
    let w = c["width"].as_u64().unwrap() as usize;
    let h = c["height"].as_u64().unwrap() as usize;
    let only: Vec<u32> = c["only"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as u32).collect()).unwrap_or_default();
    let req = SolveRequest {
        stars: stars_xy_flux(&c["xy"], &c["flux"]),
        width: w,
        height: h,
        scale_lo: opt_f64(&c["scale_lo"]),
        scale_hi: opt_f64(&c["scale_hi"]),
        hint: hint_of(&c["hint"]),
    };
    let restricted;
    let all;
    let cat = if only.is_empty() {
        all = catalog().expect("星図データ");
        &*all
    } else {
        restricted = restricted_catalog(&only);
        &restricted
    };
    let buf = LogBuf::default();
    let (res, dt, trace) = solve_traced(&req, cat, &Params::default(), solve_deadline_s(), &buf);
    let py = &c["python"];
    let status = py["status"].as_str().unwrap_or("");
    // (3) 途中の値(Python が締め切りで止まったものは比べない)
    if matches!(status, "solved" | "unsolved") {
        out.trace = trace.map(|t| {
            let d = trace_diff(&t, py);
            if d.is_empty() {
                Ok(format!("途中の値も一致(束 {}・仮説の記録 {} 個を比べた)", py["flushes"].as_array().map_or(0, |a| a.len()),
                    py["hyp_first"].as_array().map_or(0, |a| a.len())))
            } else {
                Err(d.join("、"))
            }
        });
    }
    let det = match res {
        Ok(s) => s,
        Err(e) => {
            out.python = Some(Err(format!("エラー {e:?}({dt:.2} 秒)")));
            return out;
        }
    };
    let sol = det.as_ref().map(|d| &d.solution);
    // (1) Python と同じ答えか
    out.python = Some(match (status, &sol) {
        ("solved", Some(s)) => {
            let bad = detail_diff(det.as_ref().unwrap(), py, w, h);
            if bad.is_empty() {
                Ok(format!("Python と一致({} 対数オッズ {:.2}・一致 {}・quad {}、{dt:.2} 秒 / Python {:.2} 秒)",
                    s.index_name, s.logodds, s.nmatch, s.quads_tried, py["seconds"].as_f64().unwrap_or(f64::NAN)))
            } else {
                Err(bad.join("、") + &format!("({dt:.2} 秒)"))
            }
        }
        ("solved", None) => Err(format!("解けなかった(Python は {} で解いた。{dt:.2} 秒、ログ: {})",
            py["solution"]["index"], buf.tail(2))),
        ("unsolved", None) => Ok(format!("Python と同じく解なし({dt:.2} 秒)")),
        ("unsolved", Some(s)) => Err(format!("Python は解なしなのに {} で解けた", s.index_name)),
        _ => Ok(format!("Python は {status}(突き合わせない)")),
    });
    // (2) 正解の基準
    if c["expect_solve"].as_bool() == Some(true) {
        let want = truth_measure(&c["truth_summary"]);
        out.truth = Some(match &sol {
            None => Err(format!("解けなかった({dt:.2} 秒)")),
            Some(s) => match compare_wcs(&s.wcs, w, h, &want, TOL_DEFAULT) {
                Some(cmp) if cmp.ok => Ok(describe(&cmp)),
                Some(cmp) => Err(format!("外れ {:?}: {}", cmp.failed, describe(&cmp))),
                None => Err("解の WCS を測れない".into()),
            },
        });
    }
    out
}

fn run(names: Option<&[&str]>, label: &str) {
    let Some(doc) = load_testdata("solver_synth.json") else {
        skip(label, "rs/testdata/solver_synth.json が無い(gen_solver.py で作る)");
        return;
    };
    if catalog().is_none() {
        skip(label, &format!("星図データが無い({})", index_dir().display()));
        return;
    }
    let pick: Option<Vec<String>> = std::env::var("ZAHYOU_SYNTH_CASES").ok().map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
    let _g = heavy();
    let mut bad = Vec::new();
    let mut n = 0;
    for c in doc["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap_or("");
        if c["expect_solve"].as_bool() == Some(false) {
            continue; // 乱数の星野は negative.rs
        }
        if let Some(p) = &pick {
            if !p.iter().any(|x| x == name) {
                continue;
            }
        } else if let Some(ns) = names
            && !ns.contains(&name)
        {
            continue;
        }
        n += 1;
        let o = run_case(c);
        for (aspect, r) in [("Python", &o.python), ("正解", &o.truth), ("trace", &o.trace)] {
            match r {
                Some(Ok(m)) => row("PASS", &format!("{} {aspect}", o.name), m),
                Some(Err(m)) => {
                    row("FAIL", &format!("{} {aspect}", o.name), m);
                    bad.push(format!("{} {aspect}: {m}", o.name));
                }
                None => {}
            }
        }
    }
    eprintln!("  {label}: {n} 件、食い違い・外れ {} 件", bad.len());
    assert!(bad.is_empty(), "{} 件が合わない:\n{}", bad.len(), bad.join("\n"));
}

/// 速い場合(Python で 2 秒未満)を Python・正解と突き合わせる。
#[test]
fn synth_quick_matches_python_and_truth() {
    run(Some(QUICK), "synth_quick");
}

/// 全部の場合(重い。cargo test -- --ignored で回す)。
#[test]
#[ignore]
fn synth_all_matches_python_and_truth() {
    run(None, "synth_all");
}

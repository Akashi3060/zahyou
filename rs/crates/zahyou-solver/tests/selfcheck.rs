//! 物差しの自己点検(Python の test_solver_synth の「偽のソルバー」と同じ考え方): 突き合わせの関数
//! (common の solution_diff・detail_diff・trace_diff・compare)が、Python の値そのものなら一致とし、
//! 1 か所ずらすとそこだけを食い違いとして挙げるかを確かめる。解法は呼ばない(星図データも要らない)。
//!
//! ```text
//! cargo test -p zahyou-solver --test selfcheck -- --nocapture
//! ```

mod common;

use common::*;
use zahyou_solver::{SolveDetail, Solution, StarMatch};

/// 場合 hs(ヒント+スケール既知。仮説 1 個で解け、使う索引は 7 本で名前も JSON にある)。
fn case_hs() -> Option<Value> {
    let doc = load_testdata("solver_synth.json")?;
    doc["cases"].as_array()?.iter().find(|c| c["name"] == "hs" && c.get("skipped").is_none()).cloned()
}

/// Python の要点から、Rust の trace と同じ形の JSON を作る(偽の Rust)。
fn fake_trace(py: &Value) -> Value {
    let mut log: Vec<Value> = py["hyp_first"].as_array().cloned().unwrap_or_default();
    if let Some(o) = py["solved_hyp"]["ordinal"].as_u64() {
        let mut h = py["solved_hyp"].clone();
        h.as_object_mut().unwrap().remove("ordinal");
        if (o as usize) < log.len() {
            log[o as usize] = h;
        } else {
            log.push(h);
        }
    }
    serde_json::json!({
        "status": py["status"], "files": py["files"], "scale_used": py["scale_used"],
        "quads_tried": py["quads_tried"], "hits": py["hits"], "hypotheses": py["hypotheses"],
        "quick_rejected": py["quick_rejected"], "logodds_max": py["logodds_max"], "flushes": py["flushes"],
        "tuneups": py["tuneups"], "hypotheses_log": log,
        "solution": {"index": py["solution"]["index"], "quad_field": py["solution"]["quad_field"],
                     "quad_index": py["solution"]["quad_index"]},
    })
}

/// Python の解から、偽の Rust の解。
fn fake_detail(py: &Value) -> SolveDetail {
    let s = &py["solution"];
    let u = |k: &str| s[k].as_u64().unwrap() as usize;
    SolveDetail {
        solution: Solution {
            wcs: wcs_from_json(&s["wcs"]),
            logodds: s["logodds"].as_f64().unwrap(),
            nmatch: u("nmatch"),
            ndistract: u("ndistract"),
            nfield: u("nfield"),
            nref: u("nref"),
            index_name: s["index"].as_str().unwrap().to_string(),
            quads_tried: py["quads_tried"].as_u64().unwrap(),
            hypotheses: py["hypotheses"].as_u64().unwrap(),
            seconds: 0.0,
        },
        quad_field: usizes(&s["quad_field"]),
        quad_index: usizes(&s["quad_index"]).into_iter().map(|v| v as u32).collect(),
        matches: s["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| StarMatch { field: m[0].as_u64().unwrap() as usize, star: [0.0, 0.0, 1.0], dist_px: m[1].as_f64().unwrap() })
            .collect(),
    }
}

#[test]
fn trace_diff_flags_only_what_changed() {
    let Some(c) = case_hs() else {
        skip("trace_diff_flags_only_what_changed", "solver_synth.json の hs が無い");
        return;
    };
    let py = &c["python"];
    let base = fake_trace(py);
    let d = trace_diff(&base, py);
    assert!(d.is_empty(), "Python の値そのものなのに食い違い: {d:?}");
    // (ずらし方, 出るはずの言葉)
    type Edit = fn(&mut Value);
    let edits: Vec<(&str, Edit, &str)> = vec![
        ("解いた quad の画像の星", |t| t["solution"]["quad_field"][0] = serde_json::json!(999), "解いた quad の quad_field"),
        ("解いた quad の索引の星", |t| t["solution"]["quad_index"][1] = serde_json::json!(1), "解いた quad の quad_index"),
        ("索引の一覧", |t| t["files"][0] = serde_json::json!("index-9999"), "使った索引"),
        ("スケールの範囲", |t| t["scale_used"][0] = serde_json::json!(t["scale_used"][0].as_f64().unwrap() * (1.0 + 1e-9)), "スケールの範囲"),
        ("quad の数", |t| t["quads_tried"] = serde_json::json!(t["quads_tried"].as_u64().unwrap() + 1), "quads_tried"),
        ("当たりの数", |t| t["hits"] = serde_json::json!(t["hits"].as_u64().unwrap() + 1), "hits"),
        ("束の経過", |t| t["flushes"][0]["hits"] = serde_json::json!(t["flushes"][0]["hits"].as_u64().unwrap() + 1), "束 0"),
        ("仮説の CRPIX", |t| t["hypotheses_log"][0]["crpix"][0] = serde_json::json!(t["hypotheses_log"][0]["crpix"][0].as_f64().unwrap() + 2e-6), "CRPIX"),
        ("仮説の CRVAL", |t| t["hypotheses_log"][0]["crval"][1] = serde_json::json!(t["hypotheses_log"][0]["crval"][1].as_f64().unwrap() + 1e-9), "CRVAL"),
        ("仮説の対数オッズ(丸めの外)", |t| t["hypotheses_log"][0]["logodds"] = serde_json::json!(t["hypotheses_log"][0]["logodds"].as_f64().unwrap() + 1e-4), "対数オッズ"),
        ("仮説の一致数", |t| t["hypotheses_log"][0]["nmatch"] = serde_json::json!(0), "nmatch"),
        ("仮説の鏡像", |t| t["hypotheses_log"][0]["mirror"] = serde_json::json!(!t["hypotheses_log"][0]["mirror"].as_bool().unwrap()), "mirror"),
        ("仮説が足りない", |t| t["hypotheses_log"] = serde_json::json!([]), "Rust の記録に無い"),
    ];
    let mut bad = Vec::new();
    for (what, f, want) in edits {
        let mut t = base.clone();
        f(&mut t);
        let d = trace_diff(&t, py);
        let ok = !d.is_empty() && d.iter().any(|m| m.contains(want));
        row(if ok { "PASS" } else { "FAIL" }, what, &d.join("、"));
        if !ok {
            bad.push(format!("{what}: {d:?}"));
        }
    }
    // 丸め: 4 桁に丸めた対数オッズは、丸めの半分までのずれなら一致とする(Rust の trace は Python と同じく丸めて書く)
    let mut t = base.clone();
    let lo = t["hypotheses_log"][0]["logodds"].as_f64().unwrap();
    t["hypotheses_log"][0]["logodds"] = serde_json::json!((lo * 1e4).round() / 1e4);
    let d = trace_diff(&t, py);
    row(if d.is_empty() { "PASS" } else { "FAIL" }, "4 桁に丸めた対数オッズ", &d.join("、"));
    if !d.is_empty() {
        bad.push(format!("丸めた対数オッズで食い違い: {d:?}"));
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn detail_diff_flags_only_what_changed() {
    let Some(c) = case_hs() else {
        skip("detail_diff_flags_only_what_changed", "solver_synth.json の hs が無い");
        return;
    };
    let py = &c["python"];
    let (w, h) = (c["width"].as_u64().unwrap() as usize, c["height"].as_u64().unwrap() as usize);
    let base = fake_detail(py);
    let d = detail_diff(&base, py, w, h);
    assert!(d.is_empty(), "Python の値そのものなのに食い違い: {d:?}");
    type Edit = fn(&mut SolveDetail);
    let edits: Vec<(&str, Edit, &str)> = vec![
        ("索引", |s| s.solution.index_name = "index-5205-14".into(), "索引"),
        ("対数オッズ +2e-6", |s| s.solution.logodds += 2e-6, "対数オッズ"),
        ("一致数", |s| s.solution.nmatch += 1, "nmatch"),
        ("CRPIX +1e-5 px", |s| s.solution.wcs.crpix[0] += 1e-5, "WCS"),
        ("CD の回転 1e-8 rad(隅で約 5e-6 px)", |s| {
            let c = s.solution.wcs.cd;
            let e = 1e-8f64;
            s.solution.wcs.cd = [[c[0][0] + e * c[0][1], c[0][1] - e * c[0][0]], [c[1][0] + e * c[1][1], c[1][1] - e * c[1][0]]];
        }, "WCS"),
        ("quad", |s| s.quad_field.swap(0, 1), "解いた quad"),
        ("最後の一致のずれ", |s| s.matches[0].dist_px += 2e-6, "最後の一致"),
        ("試した quad の数", |s| s.solution.quads_tried += 1, "quads_tried"),
    ];
    let mut bad = Vec::new();
    for (what, f, want) in edits {
        let mut s = base.clone();
        f(&mut s);
        let d = detail_diff(&s, py, w, h);
        let ok = !d.is_empty() && d.iter().any(|m| m.contains(want));
        row(if ok { "PASS" } else { "FAIL" }, what, &d.join("、"));
        if !ok {
            bad.push(format!("{what}: {d:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn truth_compare_flags_each_item() {
    // 正解そのもの → 合格、許容を少し超えてずらす → その項目だけ外れ(中心・スケール・回転・鏡像)
    let Some(c) = case_hs() else {
        skip("truth_compare_flags_each_item", "solver_synth.json の hs が無い");
        return;
    };
    let (w, h) = (c["width"].as_u64().unwrap() as usize, c["height"].as_u64().unwrap() as usize);
    let truth = wcs_from_json(&c["truth"]);
    let want = truth_measure(&c["truth_summary"]);
    let ok = compare_wcs(&truth, w, h, &want, TOL_DEFAULT).unwrap();
    assert!(ok.ok, "正解そのものが外れ: {}", describe(&ok));
    let s = want.scale_arcsec;
    let mut bad = Vec::new();
    let rot = |wcs: &zahyou_wcs::Wcs, deg: f64| {
        let (sn, cs) = deg.to_radians().sin_cos();
        let c = wcs.cd;
        let mut w2 = wcs.clone();
        w2.cd = [[cs * c[0][0] - sn * c[1][0], cs * c[0][1] - sn * c[1][1]], [sn * c[0][0] + cs * c[1][0], sn * c[0][1] + cs * c[1][1]]];
        w2
    };
    let mut shifted = truth.clone();
    shifted.crpix[0] += 2.2 / s; // 2.2″ ずらす(許容 2″)
    let mut scaled = truth.clone();
    scaled.cd = scaled.cd.map(|r| r.map(|v| v * 1.0025));
    let mut mirrored = truth.clone();
    mirrored.cd = [[truth.cd[0][1], truth.cd[0][0]], [truth.cd[1][1], truth.cd[1][0]]];
    for (what, wcs, item) in [("中心 2.2″", shifted, "center"), ("スケール 0.25 %", scaled, "scale"), ("回転 0.12°", rot(&truth, 0.12), "rot")] {
        let cmp = compare_wcs(&wcs, w, h, &want, TOL_DEFAULT).unwrap();
        let ok = cmp.failed == vec![item];
        row(if ok { "PASS" } else { "FAIL" }, what, &format!("{:?} {}", cmp.failed, describe(&cmp)));
        if !ok {
            bad.push(format!("{what}: {:?}", cmp.failed));
        }
    }
    let cmp = compare_wcs(&mirrored, w, h, &want, TOL_DEFAULT).unwrap();
    let ok = cmp.failed.contains(&"parity");
    row(if ok { "PASS" } else { "FAIL" }, "鏡像", &format!("{:?}", cmp.failed));
    if !ok {
        bad.push(format!("鏡像: {:?}", cmp.failed));
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

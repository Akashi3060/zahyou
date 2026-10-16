//! 解法の部品を Python 基準実装(dev/solver_core.py)の値と突き合わせる(rs/testdata/solver_parts.json。
//! gen_solver.py が作る。入力は小数 1〜3 桁に丸めた値なので、Rust も同じ入力で計算する)。
//!
//! - 星の並べ替え(sv_order_stars・_SvField の order・nq・検証の星の並び vidx。明るさの同じ値・NaN・箱の縁を含む)
//! - newpoint の quad 作り(足した星 n ごとの sv_quads_newest の並び。4 星・3 星。同じ位置の星を含む)と、
//!   それが総当たり(AB の長さが範囲内、内側の星が AB を直径とする円の中)と一致すること
//! - 画像側のコード(sv_field_codes)と照合に投げる形(sv_code_variants。正準化の境界の近く・鏡像・parity 3 通り)
//! - quad の AB の範囲(sv_quad_ab_range)・一致の重み・F 分布の点・HEALPix の 1 辺
//! - 4 点の当てはめ(sv_fit_similarity_many の 1 組ずつ)
//! - 検証の積み上げ(sv_verify_core。1 対 1 の付け替え・偽物・打ち切りを含む 60 仮説)
//! - 検証の対数オッズ(sv_verify_wcs → zahyou_solver::verify_wcs。合成星野で真の WCS・乱数・回転・鏡像・ずらしたもの。
//!   さらに Python の解の quad と WCS を渡すと Python の解の対数オッズになること。索引が要る)
//!
//! ```text
//! cargo test -p zahyou-solver --test parts -- --nocapture
//! ```

mod common;

use common::*;
use zahyou_solver::parts::{
    code_variants, f_upper, field_codes, field_order, fit_similarity_quad, healpix_side_rad, logodds_weight, order_stars,
    quad_ab_range, quad_geometry, quads_newest, verify_core,
};
use zahyou_solver::{Params, Parity};

fn parts() -> Option<Value> {
    let v = load_testdata("solver_parts.json");
    if v.is_none() {
        skip("parts", "rs/testdata/solver_parts.json が無い(gen_solver.py で作る)");
    }
    v
}

fn pts2(v: &Value) -> Vec<[f64; 2]> {
    f64s(v).chunks(2).map(|c| [c[0], c[1]]).collect()
}

#[test]
fn order_matches_python() {
    let Some(d) = parts() else { return };
    let mut bad = Vec::new();
    let cases = d["order"].as_array().unwrap();
    for (k, c) in cases.iter().enumerate() {
        let xy = pts2(&c["xy"]);
        let flux: Option<Vec<f64>> = (!c["flux"].is_null()).then(|| f64s(&c["flux"]));
        let w = c["width"].as_f64().unwrap();
        let h = c["height"].as_f64().unwrap();
        let nb = c["nboxes"].as_u64().unwrap() as usize;
        let got = order_stars(&xy, flux.as_deref(), w, h, nb);
        if got != usizes(&c["order"]) {
            bad.push(format!("場合 {k}(星 {}・箱 {nb}): order_stars が違う", xy.len()));
        }
        let p = Params { uniformize_boxes: nb, ..Params::default() };
        let fo = field_order(&xy, flux.as_deref(), w, h, &p);
        if fo.order != usizes(&c["order"]) || fo.nq != c["nq"].as_u64().unwrap() as usize || fo.verify != usizes(&c["verify"]) {
            bad.push(format!("場合 {k}: field_order(order・nq・verify)が違う"));
        }
    }
    eprintln!("  星の並べ替え {} 通り、食い違い {}", cases.len(), bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn newpoint_matches_python_and_brute_force() {
    let Some(d) = parts() else { return };
    let mut bad = Vec::new();
    let mut total = 0;
    for (k, c) in d["newpoint"].as_array().unwrap().iter().enumerate() {
        let p = pts2(&c["points"]);
        let dq = c["dimquads"].as_u64().unwrap() as usize;
        let (lo, hi) = (c["ab_lo"].as_f64().unwrap(), c["ab_hi"].as_f64().unwrap());
        let g = quad_geometry(&p, c["codetol"].as_f64().unwrap());
        let mut all = Vec::new();
        for (n, want) in c["quads"].as_array().unwrap().iter().enumerate() {
            let got = quads_newest(n, dq, &g, lo * lo, hi * hi);
            let want: Vec<Vec<usize>> = want.as_array().unwrap().iter().map(usizes).collect();
            let got2: Vec<Vec<usize>> = got.iter().map(|q| q[..dq].to_vec()).collect();
            if got2 != want {
                bad.push(format!("場合 {k}({dq} 星)n = {n}: Python {} 個 / Rust {} 個、並びか中身が違う", want.len(), got2.len()));
            }
            if dq == 3 && got.iter().any(|q| q[3] != usize::MAX) {
                bad.push(format!("場合 {k} n = {n}: 3 星の 4 つめが usize::MAX でない"));
            }
            all.extend(got2);
        }
        total += all.len();
        // 総当たり: a < b、|AB|² が範囲内、内側の星は a・b 以外で円の中(番号の組で比べる)
        let n = p.len();
        let mut want = std::collections::BTreeSet::new();
        for a in 0..n {
            for b in a + 1..n {
                let l = g.l2(a, b);
                if !(lo * lo <= l && l <= hi * hi) {
                    continue;
                }
                let ins: Vec<usize> = (0..n).filter(|&k| k != a && k != b && g.inside(a, b, k)).collect();
                if dq == 3 {
                    ins.iter().for_each(|&c| {
                        want.insert(vec![a, b, c]);
                    });
                } else {
                    for (i, &c) in ins.iter().enumerate() {
                        for &dd in &ins[i + 1..] {
                            want.insert(vec![a, b, c, dd]);
                        }
                    }
                }
            }
        }
        let got: std::collections::BTreeSet<Vec<usize>> = all
            .iter()
            .map(|q| {
                let (mut ab, mut inner) = (vec![q[0], q[1]], q[2..].to_vec());
                ab.sort();
                inner.sort();
                ab.extend(inner);
                ab
            })
            .collect();
        if got.len() != all.len() || got != want || c["total"].as_u64() != Some(all.len() as u64) {
            bad.push(format!("場合 {k}: newpoint {} 個(重複 {})≠ 総当たり {} 個", all.len(), all.len() - got.len(), want.len()));
        }
    }
    eprintln!("  newpoint の quad 計 {total} 個、食い違い {}", bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn codes_and_variants_match_python() {
    let Some(d) = parts() else { return };
    let mut bad = Vec::new();
    let mut worst: f64 = 0.0;
    for c in d["codes"].as_array().unwrap() {
        let dq = c["dimquads"].as_u64().unwrap() as usize;
        let p = pts2(&c["points"]);
        let quads: Vec<[usize; 4]> = c["quads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| {
                let v = usizes(q);
                [v[0], v[1], v[2], if dq == 4 { v[3] } else { usize::MAX }]
            })
            .collect();
        let codes = field_codes(&p, &quads, dq);
        let want = f64s(&c["codes"]);
        let dcode = 2 * (dq - 2);
        for (i, cd) in codes.iter().enumerate() {
            for j in 0..dcode {
                worst = worst.max((cd[j] - want[i * dcode + j]).abs());
            }
            if dq == 3 && (cd[2] != 0.0 || cd[3] != 0.0) {
                bad.push(format!("3 星のコードの後ろ 2 つが 0 でない(行 {i})"));
            }
        }
        if worst > 0.0 {
            bad.push(format!("{dq} 星: コードが Python と {worst:.1e} 違う(ビットまで同じはず)"));
        }
        let tol = c["codetol"].as_f64().unwrap();
        for (name, parity) in [("both", Parity::Both), ("normal", Parity::Normal), ("mirror", Parity::Mirror)] {
            // Python の codes(丸めなしの値)から形を作る(コードの差で入れ替えの判定が変わらないように)
            let pc: Vec<[f64; 4]> = want
                .chunks(dcode)
                .map(|v| if dq == 4 { [v[0], v[1], v[2], v[3]] } else { [v[0], v[1], 0.0, 0.0] })
                .collect();
            let got = code_variants(&pc, dq, tol, parity);
            let w = &c["variants"][name];
            let wq = f64s(&w["q"]);
            let wrow = usizes(&w["row"]);
            let wperm: Vec<Vec<usize>> = w["perm"].as_array().unwrap().iter().map(usizes).collect();
            let wmir: Vec<bool> = w["mirror"].as_array().unwrap().iter().map(|v| v.as_bool().unwrap()).collect();
            let mut ok = got.row == wrow && got.mirror == wmir && got.q.len() == wrow.len();
            if ok {
                for (i, q) in got.q.iter().enumerate() {
                    ok &= q[..dcode] == wq[i * dcode..(i + 1) * dcode];
                    ok &= got.perm[i][..dq] == wperm[i][..];
                    if dq == 3 {
                        ok &= got.perm[i][3] == usize::MAX && q[2] == 0.0 && q[3] == 0.0;
                    }
                }
            }
            if !ok {
                bad.push(format!("{dq} 星 parity {name}: 形が違う(Python {} 個 / Rust {} 個)", wrow.len(), got.row.len()));
            }
        }
    }
    eprintln!("  コードの差の最大 {worst:.1e}、食い違い {}", bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn small_formulas_match_python() {
    let Some(d) = parts() else { return };
    let mut bad = Vec::new();
    for r in d["ab_range"].as_array().unwrap() {
        let g = |k: &str| r[k].as_f64().unwrap();
        let (lo, hi) = quad_ab_range(
            g("scale_lo_rad"),
            g("scale_hi_rad"),
            g("s_lo"),
            g("s_hi"),
            g("width"),
            g("height"),
            g("codetol"),
            g("quad_min_frac"),
        );
        if lo != g("lo") || hi != g("hi") {
            bad.push(format!("quad_ab_range({}): ({lo}, {hi}) ≠ Python ({}, {})", r["indexid"], g("lo"), g("hi")));
        }
    }
    let lw = &d["logodds_weight"];
    for (lo, w) in f64s(&lw["lo"]).into_iter().zip(f64s(&lw["w"])) {
        let got = logodds_weight(lo);
        if (got - w).abs() > 1e-15 * w.abs().max(1e-300) && got != w {
            bad.push(format!("logodds_weight({lo}) = {got} ≠ Python {w}"));
        }
    }
    for r in d["f_upper"].as_array().unwrap() {
        let (d1, d2, z) = (r["d1"].as_f64().unwrap(), r["d2"].as_f64().unwrap(), r["z"].as_f64().unwrap());
        let want = r["f"].as_f64().unwrap_or(f64::INFINITY); // null は +∞(JSON に書けないため)
        let got = f_upper(d1, d2, z);
        let same = if want.is_infinite() { got.is_infinite() && got > 0.0 } else { (got - want).abs() <= 1e-13 * want.abs() };
        if !same {
            bad.push(format!("f_upper({d1}, {d2}) = {got} ≠ Python {want}"));
        }
    }
    for r in d["healpix_side_rad"].as_array().unwrap() {
        let n = r["nside"].as_u64().unwrap() as u32;
        let want = r["side"].as_f64().unwrap();
        if (healpix_side_rad(n) - want).abs() > 1e-16 {
            bad.push(format!("healpix_side_rad({n}) ≠ Python"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn similarity_matches_python() {
    let Some(d) = parts() else { return };
    let s = &d["similarity"];
    let field = f64s(&s["field"]);
    let stars = f64s(&s["stars"]);
    let crval = f64s(&s["crval"]);
    let crpix = f64s(&s["crpix"]);
    let cd = f64s(&s["cd"]);
    let scale = f64s(&s["scale"]);
    let okv: Vec<bool> = s["ok"].as_array().unwrap().iter().map(|v| v.as_bool().unwrap()).collect();
    let (mut dcd, mut dcv, mut dcp, mut dsc): (f64, f64, f64, f64) = (0.0, 0.0, 0.0, 0.0);
    let mut bad = Vec::new();
    for h in 0..okv.len() {
        let f: Vec<[f64; 2]> = (0..4).map(|k| [field[h * 8 + 2 * k], field[h * 8 + 2 * k + 1]]).collect();
        let st: Vec<[f64; 3]> = (0..4).map(|k| [stars[h * 12 + 3 * k], stars[h * 12 + 3 * k + 1], stars[h * 12 + 3 * k + 2]]).collect();
        let r = fit_similarity_quad(&f, &st);
        if r.ok != okv[h] {
            bad.push(format!("組 {h}: ok {} ≠ Python {}", r.ok, okv[h]));
        }
        let cmax = (0..4).map(|k| cd[h * 4 + k].abs()).fold(0.0, f64::max);
        for k in 0..4 {
            dcd = dcd.max((r.cd[k / 2][k % 2] - cd[h * 4 + k]).abs() / cmax);
        }
        for k in 0..3 {
            dcv = dcv.max((r.crval[k] - crval[h * 3 + k]).abs());
        }
        dcp = dcp.max((r.crpix[0] - crpix[h * 2]).abs()).max((r.crpix[1] - crpix[h * 2 + 1]).abs());
        dsc = dsc.max((r.scale / scale[h] - 1.0).abs());
    }
    eprintln!("  4 点の当てはめ {} 組: CD の相対差 {dcd:.1e}、CRVAL {dcv:.1e}、CRPIX {dcp:.1e} px、スケール {dsc:.1e}", okv.len());
    assert!(bad.is_empty() && dcd <= 1e-12 && dcv <= 1e-15 && dcp <= 1e-12 && dsc <= 1e-13, "{}", bad.join("\n"));
}

#[test]
fn accumulate_matches_python() {
    let Some(d) = parts() else { return };
    let a = &d["accumulate"];
    let test_xy = pts2(&a["test_xy"]);
    let ref_xy = pts2(&a["ref_xy"]);
    let rstart = usizes(&a["ref_start"]);
    let rcount = usizes(&a["ref_count"]);
    let logbg = f64s(&a["logbg"]);
    let p = Params { distractor: a["distractor"].as_f64().unwrap(), logodds_bail: a["logodds_bail"].as_f64().unwrap(), ..Params::default() };
    let rows = a["test_idx"].as_array().unwrap();
    let sig2 = a["sig2"].as_array().unwrap();
    let mut bad = Vec::new();
    let (mut nm, mut nc, mut nd) = (0, 0, 0);
    let mut dmax: f64 = 0.0;
    for h in 0..rows.len() {
        let ti = i64s(&rows[h]);
        let s2 = f64s(&sig2[h]);
        let refs = &ref_xy[rstart[h]..rstart[h] + rcount[h]];
        let r = verify_core(&test_xy, &ti, &s2, refs, logbg[h], &p, true);
        let want = a["best"][h].as_f64().unwrap_or(f64::NEG_INFINITY);
        let d_ = if want.is_finite() { (r.best - want).abs() } else if r.best == want { 0.0 } else { f64::INFINITY };
        dmax = dmax.max(d_);
        let get = |k: &str| a[k][h].as_i64().unwrap();
        let same_counts = r.nmatch as i64 == get("nmatch")
            && r.ndistract as i64 == get("ndistract")
            && r.nconflict as i64 == get("nconflict")
            && r.nused as i64 == get("nused")
            && r.best_col == get("best_col");
        let wm: Vec<(usize, usize, usize, f64)> = a["matches"][h]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                let v = f64s(m);
                (v[0] as usize, v[1] as usize, v[2] as usize - rstart[h], v[3])
            })
            .collect();
        let same_m = wm.len() == r.matches.len()
            && wm.iter().zip(&r.matches).all(|(w, g)| w.0 == g.col && w.1 == g.test && w.2 == g.reference && (w.3 - g.logodds).abs() <= 1e-9);
        if d_ > 1e-9 || !same_counts || !same_m {
            bad.push(format!(
                "仮説 {h}: 対数オッズ {:.6} / Python {want:.6}、一致 {}/{}、偽物 {}/{}、取り合い {}/{}、一致の記録 {}",
                r.best,
                r.nmatch,
                get("nmatch"),
                r.ndistract,
                get("ndistract"),
                r.nconflict,
                get("nconflict"),
                if same_m { "同じ" } else { "違う" }
            ));
        }
        nm += r.nmatch;
        nc += r.nconflict;
        nd += r.ndistract;
    }
    eprintln!("  積み上げ {} 仮説: 対数オッズの差の最大 {dmax:.1e}、一致 {nm}・偽物 {nd}・取り合い {nc}、食い違い {}", rows.len(), bad.len());
    assert!(nc > 5, "取り合い(付け替え)が起きる場合が入っていない");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// 索引の名前("index-5202-20" / "index-4107")から系列の番号。
fn series_of(name: &str) -> Option<u32> {
    name.strip_prefix("index-")?.split('-').next()?.parse().ok()
}

#[test]
fn verify_wcs_matches_python() {
    let Some(d) = parts() else { return };
    let rows = d["verify"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        skip("verify_wcs_matches_python", "solver_parts.json に verify が無い(作ったときに星図データが無かった)");
        return;
    }
    let Some(cat) = catalog() else {
        skip("verify_wcs_matches_python", "星図データが無い");
        return;
    };
    let p = Params::default();
    let mut bad = Vec::new();
    let mut n = 0;
    for r in &rows {
        let iid = r["indexid"].as_u64().unwrap() as u32;
        let w = r["width"].as_u64().unwrap() as usize;
        let h = r["height"].as_u64().unwrap() as usize;
        let stars = stars_xy_flux(&r["xy"], &r["flux"]);
        for c in r["cases"].as_array().unwrap() {
            n += 1;
            let label = format!("{iid} {}", c["label"].as_str().unwrap_or("?"));
            let wcs = wcs_from_json(&c["wcs"]);
            let got = match zahyou_solver::verify_wcs(&stars, w, h, &cat, iid, &wcs, None, &p) {
                Ok(g) => g,
                Err(e) => {
                    bad.push(format!("{label}: Err {e}"));
                    continue;
                }
            };
            let want = c["logodds"].as_f64().unwrap_or(f64::NEG_INFINITY);
            let same_lo = if want.is_finite() { (got.logodds - want).abs() <= 1e-6 } else { got.logodds == want };
            let counts = [("nmatch", got.nmatch), ("ndistract", got.ndistract), ("nfield", got.nfield), ("nref", got.nref)];
            let bad_counts: Vec<String> = counts
                .iter()
                .filter(|(k, v)| c[*k].as_u64() != Some(*v as u64))
                .map(|(k, v)| format!("{k} {v} ≠ Python {}", c[*k]))
                .collect();
            let pm: Vec<(u64, f64)> =
                c["matches"].as_array().unwrap().iter().map(|m| (m[0].as_u64().unwrap(), m[1].as_f64().unwrap())).collect();
            let same_m = pm.len() == got.matches.len()
                && pm.iter().zip(&got.matches).all(|(a, b)| a.0 == b.field as u64 && (a.1 - b.dist_px).abs() <= 1e-6);
            if !same_lo || !bad_counts.is_empty() || !same_m {
                bad.push(format!(
                    "{label}: 対数オッズ {:.9} / Python {want:.9}、{}、一致の並び {}",
                    got.logodds,
                    bad_counts.join("・"),
                    if same_m { "同じ" } else { "違う" }
                ));
            } else {
                row("PASS", &label, &format!("対数オッズ {:.4}・一致 {}・偽物 {}", got.logodds, got.nmatch, got.ndistract));
            }
        }
    }
    eprintln!("  検証の対数オッズ {n} 通り、食い違い {}", bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Python の解(quad の 4 点・索引・最終の WCS)をそのまま verify_wcs に渡すと、Python の解の対数オッズになる
/// (Python の sv_verify_wcs の約束)。合成星野の Python の解で確かめる(探索を通さない検証の突き合わせ)。
#[test]
fn verify_wcs_with_quad_gives_python_solution_logodds() {
    let label = "verify_wcs_with_quad_gives_python_solution_logodds";
    let (Some(doc), Some(cat)) = (load_testdata("solver_synth.json"), catalog()) else {
        skip(label, "solver_synth.json か星図データが無い");
        return;
    };
    let p = Params::default();
    let mut bad = Vec::new();
    let mut n = 0;
    for c in doc["cases"].as_array().unwrap() {
        let s = &c["python"]["solution"];
        if !s.is_object() {
            continue;
        }
        let name = c["name"].as_str().unwrap_or("?");
        let index = s["index"].as_str().unwrap();
        let Some(iid) = series_of(index) else {
            bad.push(format!("{name}: 索引の名前 {index} から系列が分からない"));
            continue;
        };
        let w = c["width"].as_u64().unwrap() as usize;
        let h = c["height"].as_u64().unwrap() as usize;
        let stars = stars_xy_flux(&c["xy"], &c["flux"]);
        let qf = usizes(&s["quad_field"]);
        let qi: Vec<u32> = usizes(&s["quad_index"]).into_iter().map(|v| v as u32).collect();
        let wcs = wcs_from_json(&s["wcs"]);
        n += 1;
        match zahyou_solver::verify_wcs(&stars, w, h, &cat, iid, &wcs, Some((&qf, index, &qi)), &p) {
            Ok(g) => {
                let want = s["logodds"].as_f64().unwrap();
                let ok = (g.logodds - want).abs() <= 1e-6
                    && Some(g.nmatch as u64) == s["nmatch"].as_u64()
                    && Some(g.nfield as u64) == s["nfield"].as_u64()
                    && Some(g.nref as u64) == s["nref"].as_u64();
                let m = format!("{index} 対数オッズ {:.9} / Python {want:.9}・一致 {} / {}", g.logodds, g.nmatch, s["nmatch"]);
                if ok {
                    row("PASS", name, &m);
                } else {
                    row("FAIL", name, &m);
                    bad.push(format!("{name}: {m}"));
                }
            }
            Err(e) => bad.push(format!("{name}: Err {e}")),
        }
    }
    eprintln!("  Python の解 {n} 件、食い違い {}", bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

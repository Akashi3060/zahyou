//! Python 基準実装(dev/solver_wcs.py と astropy)との突き合わせ。値は rs/testdata/gen_wcs.py が作った wcs_*.json。
//!
//! 一致の度合いを `cargo test -p zahyou-wcs -- --nocapture` で表示する。JSON が無ければ SKIP と出して通す。

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use zahyou_wcs::*;

/// 相対誤差の許容(依頼の基準)。
const REL: f64 = 1e-9;
/// 画素で測る誤差の許容。実測は 1e-10 px 前後(2026-10-15)なので、100 倍の余裕をみた値。
const PX: f64 = 1e-8;

fn load(name: &str) -> Option<Value> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(name);
    match fs::read_to_string(&p) {
        Ok(s) => Some(serde_json::from_str(&s).unwrap_or_else(|e| panic!("{} を読めない: {e}", p.display()))),
        Err(_) => {
            eprintln!("SKIP: {} が無い(rs/testdata/gen_wcs.py で作る)", p.display());
            None
        }
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}
fn v2(v: &Value) -> [f64; 2] {
    [f(&v[0]), f(&v[1])]
}
fn v3(v: &Value) -> Vec3 {
    [f(&v[0]), f(&v[1]), f(&v[2])]
}
fn vecf(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(f).collect()).unwrap_or_default()
}
fn pts2(v: &Value) -> Vec<[f64; 2]> {
    v.as_array().unwrap().iter().map(v2).collect()
}
fn pts3(v: &Value) -> Vec<Vec3> {
    v.as_array().unwrap().iter().map(v3).collect()
}

/// JSON の WCS をフィールドそのままで作る(正規化し直さない = Python と同じビット)。
fn wcs_of(v: &Value) -> Wcs {
    let sip = if v["sip"].is_null() {
        None
    } else {
        let s = &v["sip"];
        Some(Sip {
            order: s["order"].as_u64().unwrap() as usize,
            a: vecf(&s["a"]),
            b: vecf(&s["b"]),
            ap_order: s["ap_order"].as_u64().unwrap() as usize,
            ap: vecf(&s["ap"]),
            bp: vecf(&s["bp"]),
        })
    };
    Wcs { crval: v3(&v["crval"]), crpix: v2(&v["crpix"]), cd: [v2(&v["cd"][0]), v2(&v["cd"][1])], sip }
}

/// 最大値を覚えておく。
#[derive(Default)]
struct Worst(Vec<(String, f64)>);
impl Worst {
    fn put(&mut self, k: &str, v: f64) {
        let v = if v.is_nan() { f64::INFINITY } else { v };
        match self.0.iter_mut().find(|(n, _)| n == k) {
            Some((_, w)) => *w = w.max(v),
            None => self.0.push((k.to_string(), v)),
        }
    }
    fn show(&self, title: &str) {
        eprintln!("[{title}]");
        for (k, v) in &self.0 {
            eprintln!("    {k:<44} {v:.2e}");
        }
    }
}

/// 相対誤差(分母は max(|b|, floor))。
fn rel(a: f64, b: f64, floor: f64) -> f64 {
    (a - b).abs() / b.abs().max(floor)
}

/// 2 つの単位ベクトルのなす角(ラジアン)。
fn ang(a: Vec3, b: Vec3) -> f64 {
    angle_between(a, b)
}

/// CD の相対誤差(最大の成分の差 / 最大の成分)。
fn cd_rel(a: &Mat2, b: &Mat2) -> f64 {
    let m = b.iter().flatten().fold(0.0f64, |x, y| x.max(y.abs()));
    let d = (0..2).flat_map(|i| (0..2).map(move |j| (i, j))).fold(0.0f64, |x, (i, j)| x.max((a[i][j] - b[i][j]).abs()));
    d / m
}

/// SIP の係数の差を、半径 L での px で測る。戻り値 (差の最大 [px], 係数 × L^(p+q) の最大 [px])。
fn sip_px(a: &[f64], b: &[f64], n: usize, l: f64) -> (f64, f64) {
    if a.len() != b.len() {
        return (f64::INFINITY, 0.0);
    }
    let m = n + 1;
    let (mut d, mut s) = (0.0f64, 0.0f64);
    for p in 0..m {
        for q in 0..m {
            let k = l.powi((p + q) as i32);
            d = d.max((a[p * m + q] - b[p * m + q]).abs() * k);
            s = s.max(b[p * m + q].abs() * k);
        }
    }
    (d, s)
}

/// SIP の係数を比べる。px の差はいつも、相対誤差は SIP の効きが 0.01 px 以上のときだけ記録する
/// (真の WCS が TAN で、当てた SIP が投影の丸めぶんしか無いときは、相対誤差に意味が無い)。
fn put_sip(w: &mut Worst, key: &str, pairs: [(&[f64], &[f64]); 2], n: usize, l: f64) {
    let (mut d, mut s) = (0.0f64, 0.0f64);
    for (a, b) in pairs {
        let (dd, ss) = sip_px(a, b, n, l);
        d = d.max(dd);
        s = s.max(ss);
    }
    w.put(&format!("{key} [px]"), d);
    if s >= 0.01 {
        w.put(&format!("{key} 相対"), d / s);
    }
}

fn compare_wcs(w: &mut Worst, tag: &str, got: &Wcs, exp: &Wcs, l: f64) {
    let scale = exp.pixel_scale_arcsec() / ARCSEC_PER_RAD; // ラジアン/px
    w.put(&format!("{tag} CRVAL [px]"), ang(got.crval, exp.crval) / scale);
    w.put(&format!("{tag} CRVAL 相対"), ang(got.crval, exp.crval));
    w.put(&format!("{tag} CRPIX 相対"), rel(got.crpix[0], exp.crpix[0], 1.0).max(rel(got.crpix[1], exp.crpix[1], 1.0)));
    w.put(&format!("{tag} CD 相対"), cd_rel(&got.cd, &exp.cd));
    match (&got.sip, &exp.sip) {
        (None, None) => {}
        (Some(g), Some(e)) => {
            w.put(&format!("{tag} 次数の不一致"), if g.order == e.order && g.ap_order == e.ap_order { 0.0 } else { f64::INFINITY });
            put_sip(w, &format!("{tag} SIP A,B"), [(&g.a, &e.a), (&g.b, &e.b)], e.order, l);
            if e.has_inverse() {
                put_sip(w, &format!("{tag} SIP AP,BP"), [(&g.ap, &e.ap), (&g.bp, &e.bp)], e.ap_order, l);
            }
        }
        _ => w.put(&format!("{tag} SIP の有無の不一致"), f64::INFINITY),
    }
}

// ---------------------------------------------------------------- 投影 ---

#[test]
fn projection_and_cards_match_python() {
    let Some(d) = load("wcs_proj.json") else { return };
    let mut w = Worst::default();
    let cases = d["cases"].as_array().unwrap();
    for c in cases {
        let wcs = wcs_of(&c["wcs"]);
        let scale = f(&c["scale_arcsec"]);
        let srad = scale / ARCSEC_PER_RAD;
        let xs = vecf(&c["x"]);
        let ys = vecf(&c["y"]);
        let pts: Vec<[f64; 2]> = xs.iter().zip(&ys).map(|(x, y)| [*x, *y]).collect();
        let batch = wcs.pix_to_xyz_batch(&pts);
        for (i, (&x, &y)) in xs.iter().zip(&ys).enumerate() {
            let (xi, eta) = wcs.pix_to_iwc(x, y);
            let e = v2(&c["iwc"][i]);
            w.put("画素→中間世界座標 [px]", (xi - e[0]).hypot(eta - e[1]) / scale * 3600.0);
            let xyz = wcs.pix_to_xyz(x, y);
            let ex = v3(&c["xyz"][i]);
            w.put("画素→天 単位ベクトル 相対", (0..3).fold(0.0f64, |m, k| m.max((xyz[k] - ex[k]).abs())));
            w.put("画素→天 [px]", ang(xyz, ex) / srad);
            w.put("画素→天 一括と 1 点ずつの差", ang(batch[i], xyz));
            let (ra, dec) = wcs.pix_to_radec(x, y);
            let er = v2(&c["radec"][i]);
            w.put("画素→赤経赤緯 [px]", ang(radec_to_xyz(ra, dec), radec_to_xyz(er[0], er[1])) / srad);
            let dra = (ra - er[0] + 180.0).rem_euclid(360.0) - 180.0;
            w.put("画素→赤経赤緯 相対(度)", rel(dra, 0.0, 1.0).max(rel(dec, er[1], 1.0)));
            for (key, got) in [
                ("pix", wcs.xyz_to_pix(ex)),
                ("pix_exact", wcs.xyz_to_pix_exact(ex)),
                ("pix_from_radec", wcs.radec_to_pix(er[0], er[1])),
            ] {
                let exp = &c[key][i];
                match (got, exp.is_null()) {
                    (Some((px, py)), false) => {
                        let e = v2(exp);
                        w.put(&format!("天→画素({key}) [px]"), (px - e[0]).hypot(py - e[1]));
                        w.put(&format!("天→画素({key}) 相対"), rel(px, e[0], 1.0).max(rel(py, e[1], 1.0)));
                    }
                    (None, true) => {}
                    _ => w.put(&format!("天→画素({key}) 成否の不一致"), f64::INFINITY),
                }
            }
        }
        let anti = [-wcs.crval[0], -wcs.crval[1], -wcs.crval[2]];
        w.put("接点の反対側が投影された", if wcs.xyz_to_pix(anti).is_none() && !c["behind_ok"].as_bool().unwrap() { 0.0 } else { f64::INFINITY });
        w.put("スケール 相対", rel(wcs.pixel_scale_arcsec(), scale, 1e-300));
        w.put("鏡像の不一致", if wcs.parity() as i64 == c["parity"].as_i64().unwrap() { 0.0 } else { f64::INFINITY });
        let (ra0, de0) = wcs.crval_radec();
        let e0 = v2(&c["crval_radec"]);
        w.put("crval_radec 相対(度)", rel((ra0 - e0[0] + 180.0).rem_euclid(360.0) - 180.0, 0.0, 1.0).max(rel(de0, e0[1], 1.0)));

        // ヘッダーのカード
        let comments: Vec<String> = c["comments"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
        let cards = wcs.header_cards(c["width"].as_u64().unwrap() as usize, c["height"].as_u64().unwrap() as usize, &comments);
        let exp = c["cards"].as_array().unwrap();
        assert_eq!(cards.len(), exp.len(), "カードの数");
        for (got, e) in cards.iter().zip(exp) {
            let key = e["k"].as_str().unwrap();
            assert_eq!(got.key, key);
            if key == "COMMENT" {
                assert_eq!(got.value, CardValue::None);
                assert_eq!(got.comment, e["v"].as_str().unwrap());
                continue;
            }
            assert_eq!(got.comment, e["c"].as_str().unwrap(), "{key} の注記");
            let ev = &e["v"];
            match &got.value {
                CardValue::Str(s) => assert_eq!(s, ev.as_str().unwrap(), "{key}"),
                CardValue::Int(i) => assert_eq!(Some(*i), ev.as_i64(), "{key}"),
                CardValue::Float(x) => {
                    assert!(ev.is_f64(), "{key} は Python では実数でない");
                    let r = rel(*x, f(ev), 1e-300);
                    if key == "CRVAL1" || key == "CRVAL2" {
                        w.put("カード CRVAL 相対", r);
                    } else {
                        w.put("カード CRVAL 以外の実数 相対(0 のはず)", r);
                    }
                }
                other => panic!("{key} の値の型 {other:?}"),
            }
        }
    }
    // COMMENT の折り返し
    let tan = Wcs::tan(radec_to_xyz(10.0, 20.0), [5.0, 5.0], [[1e-4, 0.0], [0.0, 1e-4]]);
    let mut nwrap = 0;
    for c in d["wraps"].as_array().unwrap() {
        let text = c["text"].as_str().unwrap().to_string();
        let got: Vec<String> = tan.header_cards(10, 10, std::slice::from_ref(&text)).into_iter().filter(|c| c.key == "COMMENT").map(|c| c.comment).collect();
        let exp: Vec<String> = c["lines"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
        assert_eq!(got, exp, "折り返し: {text:?}");
        nwrap += 1;
    }
    w.show(&format!("投影とカード: {} 件(TAN・SIP 2 次・3 次)、折り返し {nwrap} 件はすべて一致", cases.len()));
    for (k, v) in &w.0 {
        let tol = if k.contains("[px]") { PX } else if k.contains("0 のはず") { 0.0 } else { REL };
        assert!(*v <= tol, "{k} = {v:e}(許容 {tol:e})");
    }
}

// -------------------------------------------------------------- 当てはめ ---

#[test]
fn fits_match_python() {
    let Some(d) = load("wcs_fit.json") else { return };
    let mut w = Worst::default();
    let sim = d["similarity"].as_array().unwrap();
    for c in sim {
        let field = pts2(&c["field"]);
        let stars = pts3(&c["stars"]);
        let exp = wcs_of(&c["expect"]);
        let got = fit_similarity(&field, &stars).expect("相似変換が失敗した");
        let l = field.iter().map(|p| (p[0] - exp.crpix[0]).hypot(p[1] - exp.crpix[1])).fold(1.0, f64::max);
        compare_wcs(&mut w, "相似", &got, &exp, l);
    }
    let lsq = d["lsq"].as_array().unwrap();
    let mut nerr = 0;
    for c in lsq {
        let field = pts2(&c["field"]);
        let stars = pts3(&c["stars"]);
        let weights = if c["weights"].is_null() { None } else { Some(vecf(&c["weights"])) };
        let crpix = if c["crpix"].is_null() { None } else { Some(v2(&c["crpix"])) };
        let crval = if c["crval"].is_null() { None } else { Some(v3(&c["crval"])) };
        let size = if c["size"].is_null() { None } else { Some((f(&c["size"][0]), f(&c["size"][1]))) };
        let order = c["sip_order"].as_u64().unwrap() as usize;
        let iters = c["iterations"].as_u64().unwrap() as usize;
        let got = try_fit_wcs_lsq(&field, &stars, weights.as_deref(), crpix, order, crval, iters, size);
        let note = c["note"].as_str().unwrap();
        match (got, c["expect"].is_null()) {
            (Ok(g), false) => {
                let exp = wcs_of(&c["expect"]);
                let l = size.map_or_else(
                    || field.iter().map(|p| (p[0] - exp.crpix[0]).hypot(p[1] - exp.crpix[1])).fold(1.0, f64::max),
                    |(a, b)| 0.5 * a.hypot(b),
                );
                compare_wcs(&mut w, &format!("最小二乗 SIP{}", exp.a_order()), &g, &exp, l);
                let mut one = Worst::default();
                compare_wcs(&mut one, "", &g, &exp, l);
                if one.0.iter().any(|(_, v)| *v > 1e-10) {
                    eprintln!("    注意 {note}: {:?}", one.0.iter().filter(|(_, v)| *v > 1e-10).collect::<Vec<_>>());
                    if let (Some(gs), Some(es)) = (&g.sip, &exp.sip) {
                        eprintln!("      A(Rust)   {:?}\n      A(Python) {:?}\n      B(Rust)   {:?}\n      B(Python) {:?}", gs.a, es.a, gs.b, es.b);
                    }
                }
                // 当てた WCS どうしで、点の位置の天と画素を比べる
                let srad = exp.pixel_scale_arcsec() / ARCSEC_PER_RAD;
                for p in field.iter().filter(|p| p[0].is_finite()) {
                    w.put("最小二乗 当てた WCS の画素→天 [px]", ang(g.pix_to_xyz(p[0], p[1]), exp.pix_to_xyz(p[0], p[1])) / srad);
                    let s = exp.pix_to_xyz(p[0], p[1]);
                    if let (Some(a), Some(b)) = (g.xyz_to_pix(s), exp.xyz_to_pix(s)) {
                        w.put("最小二乗 当てた WCS の天→画素 [px]", (a.0 - b.0).hypot(a.1 - b.1));
                    }
                }
            }
            (Err(e), true) => {
                nerr += 1;
                eprintln!("    縮退で失敗(Python も失敗): {note}: {e} / Python: {}", c["error"].as_str().unwrap_or(""));
            }
            (Ok(_), true) => panic!("{note}: Python は失敗したのに Rust は解けた"),
            (Err(e), false) => panic!("{note}: Python は解けたのに Rust は失敗した: {e}"),
        }
    }
    let inv = d["sip_inverse"].as_array().unwrap();
    for c in inv {
        let wcs = wcs_of(&c["wcs"]);
        let order = c["order"].as_u64().map(|x| x as usize);
        let got = if c["size"].is_null() {
            let b = vecf(&c["bounds"]);
            fit_sip_inverse_bounds(&wcs, [b[0], b[1], b[2], b[3]], order).unwrap()
        } else {
            fit_sip_inverse(&wcs, f(&c["size"][0]), f(&c["size"][1]), order).unwrap()
        };
        let exp = wcs_of(&c["expect"]);
        let l = 0.5 * (exp.crpix[0] * 2.0).hypot(exp.crpix[1] * 2.0);
        compare_wcs(&mut w, "SIP の逆変換", &got, &exp, l);
    }
    w.show(&format!(
        "当てはめ: 相似 {} 件、最小二乗 {} 件(うち縮退で失敗 {nerr} 件はどちらも失敗)、逆変換 {} 件",
        sim.len(),
        lsq.len(),
        inv.len()
    ));
    for (k, v) in &w.0 {
        let tol = if k.contains("[px]") { PX } else { REL };
        assert!(*v <= tol, "{k} = {v:e}(許容 {tol:e})");
    }
}

// ------------------------------------------------------------ ヘッダー ---

fn card_of(v: &Value) -> Card {
    let k = v["k"].as_str().unwrap();
    let c = v["c"].as_str().unwrap_or("");
    let val = &v["v"];
    if k == "COMMENT" {
        return Card::commentary(k, val.as_str().unwrap_or(""));
    }
    let value = if let Some(s) = val.as_str() {
        CardValue::Str(s.to_string())
    } else if let Some(b) = val.as_bool() {
        CardValue::Bool(b)
    } else if val.is_i64() || val.is_u64() {
        CardValue::Int(val.as_i64().unwrap())
    } else {
        CardValue::Float(f(val))
    };
    Card::new(k, value, c)
}

#[test]
fn header_reading_matches_astropy() {
    let Some(d) = load("wcs_header.json") else { return };
    let mut w = Worst::default();
    let cases = d["cases"].as_array().unwrap();
    let (mut nok, mut nbad) = (0, 0);
    for c in cases {
        let note = c["note"].as_str().unwrap();
        let cards: Vec<Card> = c["cards"].as_array().unwrap().iter().map(card_of).collect();
        let got = Wcs::from_header_cards(&cards);
        if !c["ok"].as_bool().unwrap() {
            match got {
                Err(e) => eprintln!("    読めない(期待どおり): {note}: {e}"),
                Ok(g) => panic!("{note}: 読めないはずが読めた: {g}"),
            }
            nbad += 1;
            continue;
        }
        let g = got.unwrap_or_else(|e| panic!("{note}: 読めなかった: {e}"));
        nok += 1;
        let scale = f(&c["scale_arcsec"]);
        let srad = scale / ARCSEC_PER_RAD;
        let xs = vecf(&c["x"]);
        let ys = vecf(&c["y"]);
        let mut worst_case = 0.0f64;
        for i in 0..xs.len() {
            let e = v2(&c["radec"][i]);
            let ex = radec_to_xyz(e[0], e[1]);
            let err = ang(g.pix_to_xyz(xs[i], ys[i]), ex);
            worst_case = worst_case.max(err / srad);
            w.put("画素→天(astropy の all_pix2world)[px]", err / srad);
            w.put("画素→天(astropy) 相対", err);
            let b = v2(&c["back"][i]);
            match g.radec_to_pix_exact(e[0], e[1]) {
                Some((px, py)) => {
                    w.put("天→画素(astropy の all_world2pix)[px]", (px - b[0]).hypot(py - b[1]));
                    w.put("天→画素(astropy) 相対", rel(px, b[0], 1.0).max(rel(py, b[1], 1.0)));
                }
                None => w.put("天→画素 が解けなかった", f64::INFINITY),
            }
        }
        if worst_case > 1e-6 {
            eprintln!("    ずれ大: {note}: {worst_case:e} px");
        }
        w.put("スケール 相対", rel(g.pixel_scale_arcsec(), scale, 1e-300));
        w.put("鏡像の不一致", if (g.parity() > 0) == c["det_positive"].as_bool().unwrap() { 0.0 } else { f64::INFINITY });
    }
    w.show(&format!("画像のヘッダーを読む: 読める {nok} 件(astropy と比べる)、読めない {nbad} 件(Err を確かめる)"));
    for (k, v) in &w.0 {
        let tol = if k.contains("[px]") { PX } else { REL };
        assert!(*v <= tol, "{k} = {v:e}(許容 {tol:e})");
    }
}

// ---------------------------------------------------------------- ずれ ---

#[test]
fn offsets_match_astropy() {
    let Some(d) = load("wcs_offsets.json") else { return };
    let mut w = Worst::default();
    let pairs = d["pairs"].as_array().unwrap();
    for p in pairs {
        let c = v2(&p["center"]);
        let t = v2(&p["target"]);
        let (sep, dra, ddec) = spherical_offsets((c[0], c[1]), (t[0], t[1]));
        // 角は絶対誤差(ラジアン)と、角距離に対する相対誤差の両方を見る
        let s = f(&p["sep"]);
        for (k, a, b) in [("角距離", sep, s), ("赤経方向", dra, f(&p["dra"])), ("赤緯方向", ddec, f(&p["ddec"]))] {
            w.put(&format!("{k} 絶対 [rad]"), (a - b).abs());
            if s > 1e-6 {
                w.put(&format!("{k} 相対(角距離 > 1e-6 rad)"), (a - b).abs() / s);
            }
        }
        let pa = position_angle((c[0], c[1]), (t[0], t[1]));
        let dpa = ((pa - f(&p["pa"])) + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
        // 位置角は、同じ点(角距離 0)と正反対の点(角距離 π)では定まらないので比べない。
        // 近い点では成分の桁落ちで、どちらの実装も ε/角距離 くらいの誤差を持つので、
        // 「位置角の差 × sin(角距離)」(= 目標の位置のずれ、ラジアン)で比べる。離れた点は位置角そのものも比べる
        if s > 0.0 && s < std::f64::consts::PI - 1e-9 {
            w.put("位置角の差 × sin(角距離) [rad]", dpa.abs() * s.sin());
            if s > 1e-3 && s < std::f64::consts::PI - 1e-3 {
                w.put("位置角 [rad](角距離 1e-3〜π−1e-3 rad)", dpa.abs());
            }
        }
        let (ra, dec) = offset_by((c[0], c[1]), f(&p["pa"]), s);
        let e = v2(&p["offset_by"]);
        w.put("offset_by [rad]", ang(radec_to_xyz(ra, dec), radec_to_xyz(e[0], e[1])));
    }
    w.show(&format!("目標までのずれ: {} 組(astropy と比べる)", pairs.len()));
    for (k, v) in &w.0 {
        let tol = if k.contains("相対") { REL } else { 1e-12 };
        assert!(*v <= tol, "{k} = {v:e}(許容 {tol:e})");
    }
}

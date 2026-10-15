//! Python 基準実装(dev/zahyou_plot.py と matplotlib・astropy・reproject)との突き合わせ。
//! 値は rs/testdata/gen_plot.py が作った plot_*.json。
//!
//! 一致の度合いを `cargo test -p zahyou-plot --test python_ref -- --nocapture` で表示する。JSON が無ければ SKIP と出して通す。

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use zahyou_image::Image2D;
use zahyou_plot::axes::{format_tick, select_step_degree, select_step_hour};
use zahyou_plot::*;
use zahyou_wcs::{Card, CardValue, Wcs};

fn load(name: &str) -> Option<Value> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(name);
    match fs::read_to_string(&p) {
        Ok(s) => Some(serde_json::from_str(&s).unwrap_or_else(|e| panic!("{} を読めない: {e}", p.display()))),
        Err(_) => {
            eprintln!("SKIP: {} が無い(rs/testdata/gen_plot.py で作る)", p.display());
            None
        }
    }
}

/// 数(有限でない値は "inf" / "-inf" / "nan" の文字列で入っている)。
fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) if s == "inf" => f64::INFINITY,
        Value::String(s) if s == "-inf" => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap_or(f64::NAN),
    }
}

fn v2(v: &Value) -> [f64; 2] {
    [num(&v[0]), num(&v[1])]
}

fn f32s(v: &Value) -> Vec<f32> {
    let mut out: Vec<f32> =
        v["v"].as_array().unwrap().iter().map(|x| x.as_f64().map_or(f32::NAN, |f| f as f32)).collect();
    for (key, val) in [("nan", f32::NAN), ("posinf", f32::INFINITY), ("neginf", f32::NEG_INFINITY)] {
        for i in v[key].as_array().unwrap() {
            out[i.as_u64().unwrap() as usize] = val;
        }
    }
    out
}

fn cards(v: &Value) -> Vec<Card> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let val = &c["v"];
            let value = if let Some(s) = val.as_str() {
                CardValue::Str(s.to_string())
            } else if let Some(b) = val.as_bool() {
                CardValue::Bool(b)
            } else if let Some(i) = val.as_i64() {
                CardValue::Int(i)
            } else {
                CardValue::Float(val.as_f64().unwrap())
            };
            Card::new(c["k"].as_str().unwrap(), value, c["c"].as_str().unwrap_or(""))
        })
        .collect()
}

fn wcs_of(v: &Value) -> Wcs {
    Wcs::from_header_cards(&cards(v)).expect("WCS を読めない")
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn rel(a: f64, b: f64) -> f64 {
    if a == b { 0.0 } else { (a - b).abs() / a.abs().max(b.abs()) }
}

/// Python と同じ大きな配列(整数のハッシュ → 4 乗 → float32)。
fn big_series(n: usize) -> Vec<f32> {
    (0..n as u64)
        .map(|i| {
            let h = (i.wrapping_mul(2654435761)) % (1u64 << 32);
            let x = h as f64 / 4294967296.0;
            (x * x * x * x * 5000.0 + 100.0) as f32
        })
        .collect()
}

#[test]
fn norm_and_gray() {
    let Some(j) = load("plot_norm.json") else { return };
    let lut: Vec<u8> = j["lut"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
    assert_eq!(lut, GRAY_LUT.to_vec(), "灰色の色表");
    let mut npx = 0usize;
    for c in j["cases"].as_array().unwrap() {
        let note = c["note"].as_str().unwrap();
        let shape = c["shape"].as_array().unwrap();
        let (h, w) = (shape[0].as_u64().unwrap() as usize, shape.get(1).map_or(1, |v| v.as_u64().unwrap() as usize));
        let data = f32s(&c["data"]);
        let img = Image2D::new(w, h, data.clone()).unwrap();
        let norm = compute_norm(&data);
        if c["norm"].is_null() {
            assert!(norm.is_none(), "{note}: Python は None");
            assert!(gray_rgba(&img, None).iter().all(|&b| b == 0));
            continue;
        }
        let n = norm.unwrap_or_else(|| panic!("{note}: Rust が None"));
        let (vmin, vmax) = (num(&c["norm"][0]), num(&c["norm"][1]));
        assert_eq!((n.vmin.to_bits(), n.vmax.to_bits()), (vmin.to_bits(), vmax.to_bits()), "{note}: vmin/vmax");
        let rgba = gray_rgba(&img, Some(&n));
        let gray = c["gray"].as_str().unwrap().as_bytes();
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            let hx = std::str::from_utf8(&gray[i * 2..i * 2 + 2]).unwrap();
            if hx == "--" {
                assert_eq!(px, [0, 0, 0, 0], "{note}: 画素 {i} は透明のはず");
            } else {
                let g = u8::from_str_radix(hx, 16).unwrap();
                assert_eq!(px, [g, g, g, 255], "{note}: 画素 {i}");
            }
            npx += 1;
        }
        println!("伸張 {note}: vmin/vmax と 8 bit がビット一致({} 画素)", w * h);
    }
    for b in j["big"].as_array().unwrap() {
        let n = b["n"].as_u64().unwrap() as usize;
        let v = big_series(n);
        let p = percentile_f32(&v, 99.9);
        assert_eq!(p as f64, num(&b["p999"]), "n={n}: 99.9 パーセント点");
        let t0 = std::time::Instant::now();
        let nm = compute_norm(&v).unwrap();
        assert_eq!((nm.vmin, nm.vmax), (num(&b["norm"][0]), num(&b["norm"][1])), "n={n}: vmin/vmax");
        println!("伸張 大きな配列 n={n}: パーセント点・vmin・vmax がビット一致({:.3} 秒)", t0.elapsed().as_secs_f64());
    }
    println!("伸張: 小さい画像 {npx} 画素の 8 bit がすべて一致");
}

#[test]
fn text_formats() {
    let Some(j) = load("plot_text.json") else { return };
    let hms = j["hmsdms"].as_array().unwrap();
    for e in hms {
        let s = hmsdms(num(&e["ra"]), num(&e["dec"]), e["p"].as_u64().unwrap() as usize);
        assert_eq!(s, e["s"].as_str().unwrap(), "hmsdms({}, {}, {})", e["ra"], e["dec"], e["p"]);
    }
    println!("hmsdms: {} 件が一致", hms.len());
    let mut nfmt = 0;
    for e in j["format"].as_array().unwrap() {
        let coord = e["coord"].as_u64().unwrap() as usize;
        let sp = num(&e["spacing"]);
        for (v, t) in e["values"].as_array().unwrap().iter().zip(strs(&e["text"])) {
            assert_eq!(format_tick(coord, num(v), sp), t, "目盛りの書式 coord={coord} 間隔={sp} 値={v}");
            nfmt += 1;
        }
    }
    println!("目盛りの書式: {nfmt} 件が一致");
    let steps = j["steps"].as_array().unwrap();
    for e in steps {
        let dv = num(&e["dv"]);
        assert_eq!(select_step_degree(dv), num(&e["deg"]), "select_step_degree({dv})");
        assert!(rel(select_step_hour(dv), num(&e["hour"])) < 1e-15, "select_step_hour({dv})");
    }
    println!("目盛りの間隔: {} 件が一致", steps.len());
    let offs = j["offsets"].as_array().unwrap();
    for e in offs {
        let c = v2(&e["center"]);
        let t = v2(&e["target"]);
        let o = Offsets::new((c[0], c[1]), (t[0], t[1]));
        assert_eq!(o.report_lines(), strs(&e["lines"]), "report_offsets {c:?} → {t:?}");
        assert_eq!(title_north_up((c[0], c[1]), Some((t[0], t[1]))), e["title"].as_str().unwrap());
        let sub = title_original(Some((t[0], t[1])));
        assert_eq!(sub.lines().nth(1).unwrap(), e["subtitle"].as_str().unwrap());
    }
    println!("ずれの文・題: {} 件が一致", offs.len());
    let summ = j["summary"].as_array().unwrap();
    for e in summ {
        let w = wcs_of(&e["cards"]);
        let s = wcs_summary(
            &w,
            e["width"].as_u64().unwrap() as usize,
            e["height"].as_u64().unwrap() as usize,
            e["flip_y"].as_bool().unwrap(),
        );
        assert_eq!(s.lines(), strs(&e["lines"]));
    }
    println!("解析結果の文: {} 件が一致", summ.len());
}

fn side_of(c: &str) -> Side {
    match c {
        "b" => Side::Bottom,
        "r" => Side::Right,
        "t" => Side::Top,
        _ => Side::Left,
    }
}

#[test]
fn wcsaxes_grid_ticks_labels() {
    let Some(j) = load("plot_axes.json") else { return };
    let mut worst_px = 0.0f64;
    let mut worst_ang = 0.0f64;
    let mut worst_range = 0.0f64;
    for c in j["cases"].as_array().unwrap() {
        let note = c["note"].as_str().unwrap();
        let wcs = wcs_of(&c["cards"]);
        let (w, h) = (c["width"].as_u64().unwrap() as usize, c["height"].as_u64().unwrap() as usize);
        let flip = c["flip_y"].as_bool().unwrap();
        // 拡大・移動した表示は、Python の get_xlim・get_ylim(画面の左右・下上の順)の範囲で選び直す
        let g = if c["view"].is_null() {
            wcs_grid(&wcs, w, h, flip)
        } else {
            let (xl, yl) = (v2(&c["xlim"]), v2(&c["ylim"]));
            grid_for_view(&wcs, w, h, &View::new((xl[0], xl[1]), (yl[0], yl[1])))
        };
        for (ci, pc) in c["coords"].as_array().unwrap().iter().enumerate() {
            let r = v2(&pc["range"]);
            for k in 0..2 {
                let d = (r[k] - [g.ranges[ci].0, g.ranges[ci].1][k]).abs();
                worst_range = worst_range.max(d);
                assert!(d < 1e-9, "{note}: 範囲 coord {ci}: Python {r:?} Rust {:?}", g.ranges[ci]);
            }
            let vals: Vec<f64> = pc["values"].as_array().unwrap().iter().map(num).collect();
            assert_eq!(vals.len(), g.tick_values[ci].len(), "{note}: coord {ci} の目盛りの数");
            for (a, b) in vals.iter().zip(&g.tick_values[ci]) {
                assert!((a - b).abs() < 1e-9, "{note}: 目盛りの値 {a} / {b}");
            }
            assert!(rel(num(&pc["spacing"]), g.spacing[ci]) < 1e-15, "{note}: 間隔");
            // 各辺の目盛り
            for side in ["b", "r", "t", "l"] {
                let pt = &pc["ticks"][side];
                let pw: Vec<f64> = pt["world"].as_array().unwrap().iter().map(num).collect();
                let pp: Vec<[f64; 2]> = pt["pixel"].as_array().unwrap().iter().map(v2).collect();
                let pa: Vec<f64> = pt["angle"].as_array().unwrap().iter().map(num).collect();
                let mine: Vec<&Tick> = g.ticks.iter().filter(|t| t.coord == ci && t.side == side_of(side)).collect();
                assert_eq!(pw.len(), mine.len(), "{note}: coord {ci} 辺 {side} の目盛りの数");
                for (k, t) in mine.iter().enumerate() {
                    assert!((t.world - pw[k]).abs() < 1e-9, "{note}: 目盛りの値");
                    let d = (t.pos[0] - pp[k][0]).hypot(t.pos[1] - pp[k][1]);
                    worst_px = worst_px.max(d);
                    assert!(d < 1e-6, "{note}: coord {ci} 辺 {side} 目盛り {k} の位置 {:?} / {:?}", t.pos, pp[k]);
                    // 上下反転の図では astropy の角度は左右の鏡像(データの +y を画面の上とみなして差分を取る。
                    // Rust は画面の向きで正しく出す)。鏡像に直し、向き(180° の区別なし)で比べる
                    let (mine_a, py_a, modulo) =
                        if flip { (t.angle_deg, 180.0 - pa[k], 180.0) } else { (t.angle_deg, pa[k], 360.0) };
                    let mut da = (mine_a - py_a).rem_euclid(modulo);
                    if da > modulo / 2.0 {
                        da = modulo - da;
                    }
                    worst_ang = worst_ang.max(da);
                    assert!(da < 0.1, "{note}: 目盛りの向き {} / {}", t.angle_deg, pa[k]);
                }
            }
            // 文字の辺と文字
            let sides = strs(&pc["label_sides"]);
            assert_eq!(sides.len(), 1, "{note}: 文字の辺は 1 つ");
            let side = side_of(&sides[0]);
            let mine: Vec<&TickLabel> = g.labels.iter().filter(|l| l.coord == ci).collect();
            let texts = strs(&pc["labels"][&sides[0]]["text"]);
            let texts: Vec<&String> = texts.iter().filter(|t| !t.is_empty()).collect();
            assert!(mine.iter().all(|l| l.side == side), "{note}: coord {ci} の文字は辺 {} のはず", sides[0]);
            let got: Vec<&String> = mine.iter().map(|l| &l.text).collect();
            assert_eq!(got, texts, "{note}: coord {ci} の文字");
            // 格子の線(枠で切った区間の数・点の数・両端)
            let mut lines = g.lines.iter().filter(|l| l.coord == ci).peekable();
            for gl in pc["grid"].as_array().unwrap() {
                let wv = num(&gl["world"]);
                for piece in gl["pieces"].as_array().unwrap() {
                    let l = lines.next().unwrap_or_else(|| panic!("{note}: coord {ci} 値 {wv} の線が足りない"));
                    assert!((l.world - wv).abs() < 1e-9, "{note}: 線の値 {} / {wv}", l.world);
                    assert_eq!(
                        l.points.len() as u64,
                        piece["n"].as_u64().unwrap(),
                        "{note}: coord {ci} 値 {wv} の点の数"
                    );
                    let (f, la) = (v2(&piece["first"]), v2(&piece["last"]));
                    let d0 = (l.points[0][0] - f[0]).hypot(l.points[0][1] - f[1]);
                    let d1 = (l.points[l.points.len() - 1][0] - la[0]).hypot(l.points[l.points.len() - 1][1] - la[1]);
                    worst_px = worst_px.max(d0).max(d1);
                    assert!(d0 < 1e-6 && d1 < 1e-6, "{note}: 線の端 {d0:e} {d1:e}");
                }
            }
            assert!(lines.next().is_none(), "{note}: coord {ci} の線が多い");
        }
        let axis: Vec<(usize, Side)> = g.axis_labels.iter().map(|a| (a.coord, a.side)).collect();
        println!(
            "WCSAxes {note}: 範囲・目盛り {}+{} 本・文字・格子が一致(軸名 {:?})",
            g.tick_values[0].len(),
            g.tick_values[1].len(),
            axis
        );
    }
    println!(
        "WCSAxes: 位置の差 最大 {worst_px:.2e} px、目盛りの向きの差 最大 {worst_ang:.2e}°、範囲の差 最大 {worst_range:.2e}°"
    );
}

#[test]
fn north_up_and_markers() {
    let Some(j) = load("plot_reproject.json") else { return };
    let mut worst_val = 0.0f64;
    let mut worst_px = 0.0f64;
    let mut worst_norm = 0.0f64;
    for c in j["cases"].as_array().unwrap() {
        let note = c["note"].as_str().unwrap();
        let wcs = wcs_of(&c["cards"]);
        let (w, h) = (c["width"].as_u64().unwrap() as usize, c["height"].as_u64().unwrap() as usize);
        let img = Image2D::new(w, h, f32s(&c["data"])).unwrap();
        // 出力の WCS と大きさ
        let (out, wo, ho) = north_up_frame(&wcs, w, h).unwrap();
        let so = c["shape_out"].as_array().unwrap();
        assert_eq!((ho as u64, wo as u64), (so[0].as_u64().unwrap(), so[1].as_u64().unwrap()), "{note}: 大きさ");
        let cpx = v2(&c["out_crpix"]);
        assert!(
            (out.crpix[0] - (cpx[0] - 1.0)).abs() < 1e-7 && (out.crpix[1] - (cpx[1] - 1.0)).abs() < 1e-7,
            "{note}: CRPIX"
        );
        let crv = v2(&c["out_crval"]);
        let (ra, dec) = out.crval_radec();
        assert!((ra - crv[0]).abs() < 1e-10 && (dec - crv[1]).abs() < 1e-10, "{note}: CRVAL");
        let cd = v2(&c["out_cdelt"]);
        assert!(rel(out.cd[0][0], cd[0]) < 1e-14 && rel(out.cd[1][1], cd[1]) < 1e-14, "{note}: CDELT");
        // 値
        let nu = reproject_north_up(&img, &wcs).unwrap();
        let py: Vec<Option<f64>> = c["rep"].as_array().unwrap().iter().map(|v| v.as_f64()).collect();
        let finite: Vec<f32> = img.data.iter().copied().filter(|v| v.is_finite()).collect();
        let span = finite.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))
            - finite.iter().fold(f32::INFINITY, |a, &b| a.min(b));
        let mut mismatch_nan = 0;
        let mut worst = 0.0f64;
        for (k, (r, p)) in nu.image.data.iter().zip(&py).enumerate() {
            match p {
                None => {
                    if !r.is_nan() {
                        mismatch_nan += 1;
                        eprintln!("{note}: 画素 {k} Python は NaN、Rust は {r}");
                    }
                }
                Some(v) => {
                    if r.is_nan() {
                        mismatch_nan += 1;
                        eprintln!("{note}: 画素 {k} Python は {v}、Rust は NaN");
                    } else {
                        // 画像の値の幅に対する差(±∞ を最大値にした画素の近くは値そのものに対して)
                        worst = worst.max((*r as f64 - v).abs() / (span as f64).max(v.abs()));
                    }
                }
            }
        }
        assert_eq!(mismatch_nan, 0, "{note}: NaN の位置");
        worst_val = worst_val.max(worst);
        assert!(worst < 2e-6, "{note}: 値の差(画像の値の幅に対して){worst:e}");
        if let Some(nv) = c["norm"].as_array() {
            let n = compute_norm_f64(&nu.image.data).unwrap();
            let (a, b) = (num(&nv[0]), num(&nv[1]));
            let d = rel(n.vmin, a).max(rel(n.vmax, b));
            worst_norm = worst_norm.max(d);
            assert!(d < 1e-6, "{note}: 北が上の図の伸張 {n:?} / {a} {b}");
        }
        // 2 つの図の印(中心・矢印・目標の円・TARGET)と題
        let center = v2(&c["center"]);
        for m in c["markers"].as_array().unwrap() {
            let target = m["target"].as_array().map(|t| (num(&t[0]), num(&t[1])));
            let f1 = figure_original(&img, &wcs, &[], target, true, false).unwrap();
            let f2 = figure_north_up(&img, &wcs, target).unwrap();
            for (fig, pm, what) in [(&f1, &m["original"], "図 1"), (&f2, &m["north_up"], "図 2")] {
                let d = check_markers(fig, pm, &format!("{note} {what} 目標 {target:?}"));
                worst_px = worst_px.max(d);
            }
            if let Some(t) = m["title_north_up"].as_str() {
                assert_eq!(f2.title, t, "{note}: 図 2 の題");
            }
            assert_eq!(f2.title, title_north_up((center[0], center[1]), target));
            assert_eq!(f1.title, title_original(target));
        }
        println!("北が上 {note}: {w}×{h} → {wo}×{ho}、NaN の位置が一致、値の差 最大 {worst:.1e}(画像の値の幅に対して)");
    }
    println!(
        "北が上: 値の差 最大 {worst_val:.1e}、伸張の vmin/vmax の相対差 最大 {worst_norm:.1e}、印の位置の差 最大 {worst_px:.1e} px"
    );
}

/// 図の表示リストの印を、Python の _draw_markers が置いたものと比べる。戻り値は位置の差の最大(px)。
fn check_markers(fig: &Figure, pm: &Value, what: &str) -> f64 {
    let mut worst = 0.0f64;
    // SIP の図では astropy の all_world2pix が 1e-4 px で反復を止めるので、その分だけ許す
    let sip = fig.wcs.has_sip();
    let mut close = |a: [f64; 2], b: [f64; 2], tol: f64, name: &str| {
        let d = (a[0] - b[0]).hypot(a[1] - b[1]);
        let scale = 1.0f64.max(b[0].abs().max(b[1].abs()));
        let ok = d / scale < tol || (sip && d < 2e-4);
        assert!(ok, "{what}: {name} Rust {a:?} / Python {b:?}");
        worst = worst.max(d);
    };
    let cross = fig.prims.iter().find_map(|p| if let Prim::Cross { center, .. } = p { Some(*center) } else { None });
    close(cross.expect("中心の ×"), v2(&pm["center"]), 1e-7, "中心");
    if pm["diverged"].as_bool() == Some(true) {
        // astropy の all_world2pix が発散した(SIP の外挿が破綻する画像の外の目標)。Python の値は当てにならないので、
        // Rust は TAN だけの解の向きへ矢印を引いていることを確かめる
        let t = fig.target_pixel.expect("目標の画素");
        let tan =
            fig.wcs.tan_only().radec_to_pix(fig.wcs.pix_to_radec(t[0], t[1]).0, fig.wcs.pix_to_radec(t[0], t[1]).1);
        assert!(tan.is_some(), "{what}: TAN で写せる目標のはず");
        assert_eq!(fig.target_inside, Some(false), "{what}: 画面の外のはず");
        println!("  {what}: astropy が発散(Python の矢印は当てにならない)→ Rust は TAN だけの解 {t:?} へ");
        return worst;
    }
    let arrow =
        fig.prims.iter().find_map(|p| if let Prim::Arrow { from, to, .. } = p { Some((*from, *to)) } else { None });
    match (&pm["arrow"], arrow) {
        (Value::Null, None) => {}
        (Value::Null, Some(a)) => panic!("{what}: Rust だけ矢印 {a:?}"),
        (pa, Some((f, t))) => {
            close(f, v2(&pa["from"]), 1e-7, "矢印の根元");
            close(t, v2(&pa["to"]), 1e-7, "矢印の先");
        }
        (pa, None) => {
            // Python は根元と先が同じでも矢印の注記を作る(描かれない)
            let (f, t) = (v2(&pa["from"]), v2(&pa["to"]));
            assert!((f[0] - t[0]).abs() < 1e-9 && (f[1] - t[1]).abs() < 1e-9, "{what}: Python だけ矢印");
        }
    }
    let inside = pm["inside"].as_bool();
    assert_eq!(fig.target_inside, inside, "{what}: 画面内か");
    let circle = fig.prims.iter().find_map(|p| {
        if let Prim::Circle { center, diameter_pt, .. } = p {
            if *diameter_pt > 20.0 { Some(*center) } else { None }
        } else {
            None
        }
    });
    match (&pm["circle"], circle) {
        (Value::Null, None) => {}
        (pc, Some(c)) if !pc.is_null() => close(c, v2(pc), 1e-7, "目標の円"),
        (pc, c) => panic!("{what}: 目標の円 Python {pc} / Rust {c:?}"),
    }
    let label = fig.prims.iter().find_map(|p| {
        if let Prim::Text { anchor, text, .. } = p { if text == "TARGET" { Some(*anchor) } else { None } } else { None }
    });
    match (&pm["label"], label) {
        (Value::Null, None) => {}
        (pl, Some(l)) if !pl.is_null() => close(l, v2(pl), 1e-7, "TARGET"),
        (pl, l) => panic!("{what}: TARGET Python {pl} / Rust {l:?}"),
    }
    worst
}

/// 外接矩形 [左, 上, 右, 下] の各辺の差の最大(px)。
fn box_diff(a: [f64; 4], b: &Value) -> f64 {
    (0..4).map(|k| (a[k] - num(&b[k])).abs()).fold(0.0, f64::max)
}

#[test]
fn layout_matches_matplotlib() {
    let Some(j) = load("plot_layout.json") else { return };
    // Python は游ゴシック(Yu Gothic Regular = YuGothR.ttc の 0 番)で描いている。違うフォントなら文字の大きさが違うので比べない
    let fonts = match Fonts::system() {
        Some(f) if f.description.starts_with("YuGothR.ttc#0") && j["yu_gothic"].as_bool() == Some(true) => f,
        _ => {
            eprintln!("SKIP: 游ゴシックが無い(Python と同じ文字の大きさで比べられない)");
            return;
        }
    };
    let opts = RenderOptions::default();
    let (mut worst_axes, mut worst_title, mut worst_label) = (0.0f64, 0.0f64, 0.0f64);
    for c in j["cases"].as_array().unwrap() {
        let note = c["note"].as_str().unwrap();
        let wcs = wcs_of(&c["cards"]);
        let (w, h) = (c["width"].as_u64().unwrap() as usize, c["height"].as_u64().unwrap() as usize);
        let flip = c["flip_y"].as_bool().unwrap();
        let target = c["target"].as_array().map(|t| (num(&t[0]), num(&t[1])));
        let img = Image2D::new(w, h, vec![0.0; w * h]).unwrap();
        let mut figs = vec![figure_original(&img, &wcs, &[], target, flip, false).unwrap()];
        if target.is_some() {
            figs.push(figure_north_up(&img, &wcs, target).unwrap());
        }
        let py = c["figures"].as_array().unwrap();
        assert_eq!(py.len(), figs.len(), "{note}: 図の数");
        for (k, (fig, p)) in figs.iter().zip(py).enumerate() {
            let what = format!("{note} 図 {}", k + 1);
            let l = layout_with(fig, &opts, Some(fonts));
            // 図の大きさ(Python は 12 × (12·ny/nx + 1.2) インチの小数のまま、Rust は px に丸める)
            let fs = v2(&p["fig"]);
            assert!(
                (l.size.0 as f64 - fs[0]).abs() <= 0.5 && (l.size.1 as f64 - fs[1]).abs() <= 0.5,
                "{what}: 図の大きさ"
            );
            let (bw, bh) = l.axes.size();
            let ab = [l.axes.x0, l.axes.y0, l.axes.x0 + bw, l.axes.y0 + bh];
            // 上の辺に目盛りの文字があると(極のそば)、Python 版は題を上の文字・軸名に重ねて読めなくしてしまう。
            // Rust は題をその上へ逃がして上の余白を足すので、軸の矩形は同じにならない。重ならないことと、
            // 軸が Python より大きく縮んでいないことだけを確かめる
            let top_labels = fig.tick_labels.iter().any(|t| t.side == Side::Top);
            if top_labels {
                let t = l.title.expect("題");
                let top_deco = l
                    .tick_labels
                    .iter()
                    .zip(&fig.tick_labels)
                    .filter(|(_, tl)| tl.side == Side::Top)
                    .map(|(b, _)| b[1])
                    .chain(l.axis_labels.iter().filter(|a| a.1 == Side::Top).map(|a| a.2[1]))
                    .fold(f64::INFINITY, f64::min);
                assert!(t[3] <= top_deco + 1e-9, "{what}: 題 {t:?} が上の文字(上端 {top_deco})に重なる");
                assert!(t[1] >= 0.0, "{what}: 題が図からはみ出す {t:?}");
                let pw = num(&p["axes"][2]) - num(&p["axes"][0]);
                assert!(bw >= 0.95 * pw, "{what}: 軸の幅 Rust {bw:.1} / Python {pw:.1}");
                println!(
                    "配置 {what}: 上の辺に目盛りの文字(Python は題が重なる)。Rust の軸の幅 {bw:.1} px / Python {pw:.1} px"
                );
                continue;
            }
            let d = box_diff(ab, &p["axes"]);
            println!("配置 {what}: 軸の矩形の差 {d:.2} px");
            worst_axes = worst_axes.max(d);
            // 実測 0.7 px 以下。題は matplotlib がヒンティングした字の高さ("lp" が 17 px、丸めなければ 17.95 px)の分だけずれる
            assert!(d < 1.0, "{what}: 軸の矩形 Rust {ab:?} / Python {}", p["axes"]);
            {
                let t = l.title.expect("題");
                let dt = box_diff(t, &p["title"]);
                worst_title = worst_title.max(dt);
                assert!(dt < 3.0, "{what}: 題 Rust {t:?} / Python {}", p["title"]);
            }
            for pa in p["axislabels"].as_array().unwrap() {
                let coord = pa["coord"].as_u64().unwrap() as usize;
                let mine = l
                    .axis_labels
                    .iter()
                    .find(|a| a.0 == coord)
                    .unwrap_or_else(|| panic!("{what}: 軸名 {coord} が無い"));
                let sides = strs(&pa["axes"]);
                assert_eq!(sides, [mine.1.letter().to_string()], "{what}: 軸名 {coord} の辺");
                let da = box_diff(mine.2, &pa["box"]);
                worst_label = worst_label.max(da);
                assert!(da < 2.0, "{what}: 軸名 {coord} Rust {:?} / Python {}", mine.2, pa["box"]);
            }
        }
    }
    println!("配置: 軸の矩形の差 最大 {worst_axes:.2} px、題 {worst_title:.2} px、軸名 {worst_label:.2} px");
}

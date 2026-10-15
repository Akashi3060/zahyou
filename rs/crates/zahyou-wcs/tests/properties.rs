//! Python 版の試験(dev/test_solver_wcs.py)の 12〜14 と同じ性質を、乱数の WCS で確かめる。
//! あわせて、壊れた入力(NaN・巨大な値・壊れたヘッダー)でパニックしないことを確かめる。照合用の JSON は使わない。

use zahyou_wcs::*;

/// 試験用の決まった乱数(xorshift64*)。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }
    fn normal(&mut self) -> f64 {
        let u1 = self.uniform(1e-300, 1.0);
        let u2 = self.uniform(0.0, 1.0);
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    fn unit(&mut self) -> Vec3 {
        normalize([self.normal(), self.normal(), self.normal()])
    }
}

const SPECIAL: [(f64, f64); 9] = [
    (0.0, 90.0),
    (0.0, -90.0),
    (137.0, 89.9999),
    (300.0, -89.99),
    (359.9995, 0.0),
    (0.0004, -45.0),
    (359.99, 61.0),
    (250.0, -75.0),
    (83.8, -5.4),
];

/// Python の random_config と同じ作り方の乱数の WCS(0.3〜300″/px、視野は 100° まで)。
fn random_config(rng: &mut Rng, i: usize, sip_order: usize, scale: Option<f64>) -> (Wcs, f64, f64) {
    let crval = if i < SPECIAL.len() { radec_to_xyz(SPECIAL[i].0, SPECIAL[i].1) } else { rng.unit() };
    let scale = scale.unwrap_or_else(|| 10f64.powf(rng.uniform(0.3f64.log10(), 300f64.log10())));
    let (mut w, mut h) = (rng.uniform(300.0, 4000.0).floor(), rng.uniform(200.0, 3000.0).floor());
    let lim = 100.0 / (scale / 3600.0);
    let f = (lim / w.hypot(h)).min(1.0);
    w = (w * f).floor().max(50.0);
    h = (h * f).floor().max(40.0);
    let par = if i.is_multiple_of(2) { 1.0 } else { -1.0 };
    let skew = if i.is_multiple_of(3) { 0.02 * rng.normal() } else { 0.0 };
    let t = rng.uniform(0.0, std::f64::consts::TAU);
    let (s, c) = t.sin_cos();
    let k = scale / 3600.0;
    // rot(t) · diag(1, par) · [[1, skew], [0, 1]]
    let cd = [[k * c, k * (c * skew - s * par)], [k * s, k * (s * skew + c * par)]];
    let crpix = [rng.uniform(0.2, 0.8) * w, rng.uniform(0.2, 0.8) * h];
    let sip = if sip_order >= 2 {
        let m = sip_order + 1;
        let half = 0.5 * w.hypot(h);
        let amp = [3.0, 1.0, 0.5];
        let mut a = vec![0.0; m * m];
        let mut b = vec![0.0; m * m];
        for d in 2..=sip_order {
            for p in 0..=d {
                a[p * m + d - p] = rng.normal() * amp[d - 2] / half.powi(d as i32);
                b[p * m + d - p] = rng.normal() * amp[d - 2] / half.powi(d as i32);
            }
        }
        Some(Sip::new(sip_order, a, b).unwrap())
    } else {
        None
    };
    (Wcs::new(crval, crpix, cd, sip), w, h)
}

fn grid_error(fit: &Wcs, truth: &Wcs, w: f64, h: f64) -> f64 {
    let mut worst = 0.0f64;
    for i in 0..21 {
        for j in 0..21 {
            let x = -0.5 + w * i as f64 / 20.0;
            let y = -0.5 + h * j as f64 / 20.0;
            worst = worst.max(angle_between(fit.pix_to_xyz(x, y), truth.pix_to_xyz(x, y)));
        }
    }
    worst * ARCSEC_PER_RAD / truth.pixel_scale_arcsec()
}

#[test]
fn projection_roundtrip_within_60_degrees() {
    // Python の 01: 接点(極ちょうど・特別な場所・乱数)のまわり 60° 以内の星を、投影を通さずに作って
    // 球 → 平面 → 球、平面 → 球 → 平面の往復と、心射の性質 |(ξ, η)| = tan θ を確かめる
    let mut rng = Rng(101);
    let mut centers: Vec<Vec3> = vec![[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
    centers.extend(SPECIAL[2..].iter().map(|&(ra, de)| radec_to_xyz(ra, de)));
    centers.extend((0..20).map(|_| rng.unit()));
    let (mut worst_xyz, mut worst_tan, mut worst_xi) = (0.0f64, 0.0f64, 0.0f64);
    for r in centers {
        let (e, n) = tan_basis(r);
        for _ in 0..400 {
            let th = 60f64.to_radians() * rng.uniform(0.0, 1.0).sqrt();
            let phi = rng.uniform(0.0, std::f64::consts::TAU);
            let d = add(scale(e, phi.cos()), scale(n, phi.sin()));
            let s = add(scale(r, th.cos()), scale(d, th.sin()));
            let (xi, eta) = tan_project(s, r).expect("60° 以内は投影できる");
            let back = tan_deproject(xi, eta, r);
            worst_xyz = worst_xyz.max(angle_between(back, s) * ARCSEC_PER_RAD);
            worst_tan = worst_tan.max((xi.hypot(eta) - th.tan()).abs() / (1.0 + th.tan()));
            let (x2, e2) = tan_project(back, r).expect("戻した点も投影できる");
            worst_xi = worst_xi.max((x2 - xi).hypot(e2 - eta) * ARCSEC_PER_RAD);
        }
    }
    eprintln!("投影の往復: 球→平面→球 {worst_xyz:.1e}″  平面→球→平面 {worst_xi:.1e}″  |ξη|=tanθ の相対誤差 {worst_tan:.1e}");
    assert!(worst_xyz < 1e-6 && worst_tan < 1e-12 && worst_xi < 1e-6);
}

#[test]
fn projection_orientation_wrap_and_hemisphere() {
    // Python の 02: 東 = 赤経が増える向き(ξ > 0)、北 = 赤緯が増える向き(η > 0)。赤経 0° をまたいでも連続。
    // 接点から 90° を少しでも越えた点は投影しない
    for (ra0, de0) in [(359.995, 0.0), (0.002, -40.0), (180.0, 60.0), (250.0, -75.0)] {
        let r = radec_to_xyz(ra0, de0);
        let d = 0.01;
        let ce = f64::cos(de0.to_radians());
        let (xi_e, eta_e) = tan_project(radec_to_xyz(ra0 + d / ce, de0), r).unwrap();
        let (xi_n, eta_n) = tan_project(radec_to_xyz(ra0, de0 + d), r).unwrap();
        assert!((xi_e.to_degrees() - d).abs() < 1e-6 && eta_e.to_degrees().abs() < 1e-5, "({ra0},{de0}) 東");
        assert!((eta_n.to_degrees() - d).abs() < 1e-6 && xi_n.to_degrees().abs() < 1e-9, "({ra0},{de0}) 北");
    }
    let r = radec_to_xyz(0.0, 0.0);
    let (xa, _) = tan_project(radec_to_xyz(359.99, 0.0), r).unwrap();
    let (xb, _) = tan_project(radec_to_xyz(0.01, 0.0), r).unwrap();
    assert!(((xb - xa).to_degrees() - 0.02).abs() < 1e-9);
    assert!(tan_project([-r[0], -r[1], -r[2]], r).is_none());
    assert!(tan_project(r, r).is_some());
    assert!(tan_project(radec_to_xyz(90.001, 0.0), r).is_none());
    assert!(tan_project(radec_to_xyz(89.999, 0.0), r).is_some());
    // WCS を通しても同じ向き: CD = diag(−s, s)(東が左・北が上の普通の画像)なら、東の星は x が減る側、北の星は y が増える側
    let s = 1.0 / 3600.0;
    let w = Wcs::tan(radec_to_xyz(359.9995, 0.0), [500.0, 400.0], [[-s, 0.0], [0.0, s]]);
    let (x_e, y_e) = w.radec_to_pix(0.0005, 0.0).unwrap();
    let (x_n, y_n) = w.radec_to_pix(359.9995, 0.0005).unwrap();
    assert!((x_e - (500.0 - 3.6)).abs() < 1e-6 && (y_e - 400.0).abs() < 1e-9, "({x_e}, {y_e})");
    assert!((x_n - 500.0).abs() < 1e-9 && (y_n - (400.0 + 1.8)).abs() < 1e-6, "({x_n}, {y_n})");
    assert_eq!(w.parity(), -1);
}

#[test]
fn sip_roundtrip_newton_and_inverse() {
    // Python の 14: ニュートン法と AP/BP の往復(2・3 次、画像の外側 10% まで)
    let mut rng = Rng(1401);
    let (mut newton, mut apbp, mut sky) = (0.0f64, 0.0f64, 0.0f64);
    let mut nbad = 0;
    for i in 0..40usize {
        let order = if i.is_multiple_of(2) { 2 } else { 3 };
        let (wcs, w, h) = random_config(&mut rng, i, order, None);
        let winv = fit_sip_inverse(&wcs, w, h, None).unwrap();
        assert_eq!(winv.ap_order(), order + 1);
        for _ in 0..300 {
            let x = rng.uniform(-0.1 * w, 1.1 * w);
            let y = rng.uniform(-0.1 * h, 1.1 * h);
            let s = wcs.pix_to_xyz(x, y);
            match (wcs.xyz_to_pix(s), winv.xyz_to_pix_exact(s), winv.xyz_to_pix(s)) {
                (Some(a), Some(b), Some(c)) => {
                    newton = newton.max((a.0 - x).hypot(a.1 - y)).max((b.0 - x).hypot(b.1 - y));
                    if (-0.5..=w - 0.5).contains(&x) && (-0.5..=h - 0.5).contains(&y) {
                        apbp = apbp.max((c.0 - x).hypot(c.1 - y));
                    }
                }
                _ => nbad += 1,
            }
            // 天 → 画素 → 天
            let st = wcs.pix_to_xyz(rng.uniform(0.0, w), rng.uniform(0.0, h));
            let (px, py) = wcs.xyz_to_pix(st).unwrap();
            sky = sky.max(angle_between(wcs.pix_to_xyz(px, py), st) * ARCSEC_PER_RAD);
        }
    }
    eprintln!("SIP の往復: ニュートン {newton:.1e} px  AP/BP {apbp:.1e} px  天→画素→天 {sky:.1e}″  解けなかった点 {nbad}");
    assert!(newton < 1e-8 && apbp < 0.01 && sky < 1e-6 && nbad == 0);
}

#[test]
fn lsq_noise_free_recovers_truth_everywhere() {
    // Python の 10・11: 雑音なし・CRPIX = 真の値なら、接点を十分に動かせば(30 回)視野が広くても元に戻る
    let mut rng = Rng(1001);
    for order in [0usize, 2] {
        let mut worst = 0.0f64;
        for i in 0..30usize {
            let (truth, w, h) = random_config(&mut rng, i, order, None);
            let f: Vec<[f64; 2]> = (0..400).map(|_| [rng.uniform(-0.5, w - 0.5), rng.uniform(-0.5, h - 0.5)]).collect();
            let s: Vec<Vec3> = f.iter().map(|p| truth.pix_to_xyz(p[0], p[1])).collect();
            let fit = fit_wcs_lsq(&f, &s, None, Some(truth.crpix), order, None, 30, Some((w, h))).unwrap();
            worst = worst.max(grid_error(&fit, &truth, w, h));
            assert_eq!(fit.parity(), truth.parity());
        }
        eprintln!("最小二乗(雑音なし、SIP {order} 次、30 件): 格子のずれ {worst:.1e} px");
        assert!(worst < 1e-6);
    }
}

#[test]
fn lsq_zero_weights_are_ignored() {
    // Python の 13: 重み 0 の外れ値は無視され、その点を消して当てたものと同じになる
    let mut rng = Rng(1301);
    let (truth, w, h) = random_config(&mut rng, 20, 2, Some(1.46));
    let n = 300;
    let mut f: Vec<[f64; 2]> = (0..n).map(|_| [rng.uniform(0.0, w), rng.uniform(0.0, h)]).collect();
    let s: Vec<Vec3> = f.iter().map(|p| truth.pix_to_xyz(p[0], p[1])).collect();
    let mut wt = vec![0.0; n];
    for i in 0..n {
        if rng.uniform(0.0, 1.0) < 0.15 {
            f[i][0] += rng.uniform(5.0, 50.0);
            f[i][1] += rng.uniform(5.0, 50.0);
        } else {
            wt[i] = rng.uniform(0.5, 2.0);
        }
    }
    let fit_w = fit_wcs_lsq(&f, &s, Some(&wt), Some(truth.crpix), 2, None, 3, Some((w, h))).unwrap();
    let fit_u = fit_wcs_lsq(&f, &s, None, Some(truth.crpix), 2, None, 3, Some((w, h))).unwrap();
    let keep: Vec<usize> = (0..n).filter(|&i| wt[i] > 0.0).collect();
    let fk: Vec<[f64; 2]> = keep.iter().map(|&i| f[i]).collect();
    let sk: Vec<Vec3> = keep.iter().map(|&i| s[i]).collect();
    let wk: Vec<f64> = keep.iter().map(|&i| wt[i]).collect();
    let fit_d = fit_wcs_lsq(&fk, &sk, Some(&wk), Some(truth.crpix), 2, None, 3, Some((w, h))).unwrap();
    let (ew, eu, ed) = (grid_error(&fit_w, &truth, w, h), grid_error(&fit_u, &truth, w, h), grid_error(&fit_w, &fit_d, w, h));
    eprintln!("重み: 重みあり {ew:.1e} px  重みなし {eu:.2} px  重み 0 の点を消したものとの差 {ed:.1e} px");
    assert!(ew < 1e-6 && eu > 1.0 && ed < 1e-9);
}

#[test]
fn header_roundtrip_random() {
    // header_cards → from_header_cards で同じ空を指す(極・赤経 0°・SIP の逆変換あり/なし)
    let mut rng = Rng(601);
    for i in 0..60usize {
        let order = [0, 2, 3][i % 3];
        let (mut wcs, w, h) = random_config(&mut rng, i, order, None);
        if order >= 2 && i.is_multiple_of(2) {
            wcs = fit_sip_inverse(&wcs, w, h, None).unwrap();
        }
        let cards = wcs.header_cards(w as usize, h as usize, &["round trip".to_string()]);
        let back = Wcs::from_header_cards(&cards).unwrap();
        assert_eq!(back.sip, wcs.sip);
        let e = grid_error(&back, &wcs, w, h);
        assert!(e < 1e-6, "{i}: {e} px {wcs}");
    }
}

#[test]
fn broken_inputs_never_panic() {
    let mut rng = Rng(99);
    let weird = [f64::NAN, f64::INFINITY, -f64::INFINITY, 0.0, -0.0, 1e308, -1e308, 1e-308, 90.0, -90.0, 1.0];
    let pick = |rng: &mut Rng| weird[(rng.next() % weird.len() as u64) as usize];
    // 投影
    for _ in 0..2000 {
        let sip = if rng.next().is_multiple_of(2) {
            Some(Sip {
                order: (rng.next() % 12) as usize,
                a: (0..(rng.next() % 20)).map(|_| pick(&mut rng)).collect(),
                b: (0..(rng.next() % 20)).map(|_| pick(&mut rng)).collect(),
                ap_order: (rng.next() % 12) as usize,
                ap: (0..(rng.next() % 20)).map(|_| pick(&mut rng)).collect(),
                bp: (0..(rng.next() % 20)).map(|_| pick(&mut rng)).collect(),
            })
        } else {
            None
        };
        let wcs = Wcs {
            crval: [pick(&mut rng), pick(&mut rng), pick(&mut rng)],
            crpix: [pick(&mut rng), pick(&mut rng)],
            cd: [[pick(&mut rng), pick(&mut rng)], [pick(&mut rng), pick(&mut rng)]],
            sip,
        };
        let _ = wcs.pix_to_radec(pick(&mut rng), pick(&mut rng));
        let _ = wcs.radec_to_pix(pick(&mut rng), pick(&mut rng));
        let _ = wcs.radec_to_pix_exact(pick(&mut rng), pick(&mut rng));
        let _ = wcs.crval_radec();
        let _ = wcs.pixel_scale_arcsec();
        let _ = wcs.header_cards(10, 10, &["x".to_string()]);
        let _ = fit_sip_inverse(&wcs, pick(&mut rng), pick(&mut rng), None);
        let _ = spherical_offsets((pick(&mut rng), pick(&mut rng)), (pick(&mut rng), pick(&mut rng)));
        let _ = position_angle((pick(&mut rng), pick(&mut rng)), (pick(&mut rng), pick(&mut rng)));
        let _ = offset_by((pick(&mut rng), pick(&mut rng)), pick(&mut rng), pick(&mut rng));
    }
    // 巨大な次数(フィールドを直接書いたもの・引数)
    let huge = Wcs {
        crval: [0.0, 0.0, 1.0],
        crpix: [1.0, 1.0],
        cd: [[1e-4, 0.0], [0.0, 1e-4]],
        sip: Some(Sip { order: usize::MAX, a: vec![1e-9; 4], b: vec![], ap_order: usize::MAX, ap: vec![1.0], bp: vec![1.0] }),
    };
    let _ = huge.header_cards(10, 10, &[]);
    let _ = huge.pix_to_radec(3.0, 4.0);
    let _ = huge.radec_to_pix(3.0, 89.9);
    assert!(fit_sip_inverse(&huge, 10.0, 10.0, None).is_err());
    assert!(fit_sip_inverse(&huge, 10.0, 10.0, Some(usize::MAX)).is_err());
    let f3 = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
    let s3 = [radec_to_xyz(0.0, 0.0), radec_to_xyz(0.01, 0.0), radec_to_xyz(0.0, 0.01)];
    assert!(fit_wcs_lsq(&f3, &s3, None, None, usize::MAX, None, usize::MAX, None).is_none());
    // 当てはめ
    for _ in 0..500 {
        let n = (rng.next() % 12) as usize;
        let f: Vec<[f64; 2]> = (0..n).map(|_| [pick(&mut rng), pick(&mut rng)]).collect();
        let s: Vec<Vec3> = (0..n).map(|_| [pick(&mut rng), pick(&mut rng), pick(&mut rng)]).collect();
        let wt: Vec<f64> = (0..n).map(|_| pick(&mut rng)).collect();
        let _ = fit_similarity(&f, &s);
        let order = (rng.next() % 5) as usize;
        let _ = fit_wcs_lsq(&f, &s, Some(&wt), None, order, None, 3, None);
        let _ = fit_wcs_lsq(&f, &s, None, Some([pick(&mut rng), pick(&mut rng)]), order, Some([pick(&mut rng), 0.0, 1.0]), 2, Some((pick(&mut rng), 100.0)));
    }
    // 壊れたヘッダー: よく使うキーに、いろいろな型の値を入れる
    let keys = [
        "CTYPE1", "CTYPE2", "CRVAL1", "CRVAL2", "CRPIX1", "CRPIX2", "CD1_1", "CD1_2", "CD2_1", "CD2_2", "CDELT1",
        "CDELT2", "CROTA2", "PC1_1", "PC2_2", "LONPOLE", "CUNIT1", "A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER",
        "A_2_0", "B_0_2", "AP_0_0", "A_1_0", "BP_9_0",
    ];
    let strs = ["RA---TAN", "DEC--TAN", "RA---TAN-SIP", "DEC--TAN-SIP", "", "RA", "日本語の文字列です", "deg", "RA---TAN-SIPX"];
    // 正しい TAN-SIP(逆変換つき)のヘッダーを、カードを消す・値を替える・足すで崩す
    let mut sip_a = vec![0.0; 16];
    let mut sip_b = vec![0.0; 16];
    sip_a[2 * 4] = 1e-6;
    sip_b[3] = -2e-9;
    let good = Wcs::new(radec_to_xyz(83.8, -5.4), [500.0, 400.0], [[-4e-4, 1e-5], [1e-5, 4e-4]], Some(Sip::new(3, sip_a, sip_b).unwrap()));
    let good = fit_sip_inverse(&good, 1000.0, 800.0, None).unwrap();
    let base = good.header_cards(1000, 800, &["base".to_string()]);
    let mut nok = 0;
    for _ in 0..20000 {
        let mut cards = base.clone();
        let nmut = 1 + rng.next() % 4;
        for _ in 0..nmut {
            let v = match rng.next() % 5 {
                0 => CardValue::Str(strs[(rng.next() % strs.len() as u64) as usize].to_string()),
                1 => CardValue::Int((rng.next() % 2_000_000_000) as i64 - 1_000_000_000),
                2 => CardValue::Int((rng.next() % 12) as i64 - 1),
                3 => CardValue::Float(pick(&mut rng)),
                _ => CardValue::Bool(rng.next().is_multiple_of(2)),
            };
            match rng.next() % 3 {
                0 if !cards.is_empty() => {
                    let i = (rng.next() % cards.len() as u64) as usize;
                    cards.remove(i);
                }
                1 if !cards.is_empty() => {
                    let i = (rng.next() % cards.len() as u64) as usize;
                    cards[i].value = v;
                }
                _ => {
                    let k = keys[(rng.next() % keys.len() as u64) as usize];
                    let i = (rng.next() % (cards.len() as u64 + 1)) as usize;
                    cards.insert(i, Card::new(k, v, ""));
                }
            }
        }
        if let Ok(w) = Wcs::from_header_cards(&cards) {
            nok += 1;
            let _ = w.pix_to_radec(1.0, 2.0);
            let _ = w.radec_to_pix(83.8, -5.4);
            let _ = w.radec_to_pix_exact(83.8, -5.4);
            let _ = w.header_cards(10, 10, &[]);
        }
    }
    assert!(nok > 1000, "読めたものが少なすぎて奥まで試せていない({nok})");
    eprintln!("壊れたヘッダー 20000 通り: パニックなし(崩しても読めたもの {nok})");
}

//! 改良の当てはめ(Python の `sv_fit_wcs_lsq` の今の版)と残差。
//!
//! zahyou-wcs の `try_fit_wcs_lsq` は接点を決まった回数だけ動かす版なので、ここでは Python の今の版
//! (残った定数項を画素に直して 1e-8 px 以下になるまで、上限 40 回。上限で 0.01 px を超えて残れば失敗)を写す。
//! 多項式の最小二乗は zahyou-wcs の QR(`linalg::lstsq_qr`)、SIP の逆変換は `fit_sip_inverse` を使う。

use zahyou_wcs::linalg::{det2, inv2, lstsq_qr};
use zahyou_wcs::sip::poly_terms;
use zahyou_wcs::{Mat2, Sip, Vec3, Wcs, fit_sip_inverse, fit_sip_inverse_bounds, tan_basis};

/// 接点を動かす回数の上限(Python の SV_LSQ_MAX_SHIFTS)。
pub(crate) const LSQ_MAX_SHIFTS: usize = 40;
/// CRPIX に残る定数項(画素に直したもの)がこれ以下なら接点を動かすのをやめる。
const LSQ_TOL_PX: f64 = 1e-8;
/// 上限まで動かしてもこれを超えて残れば、当てはめは失敗。
const LSQ_FAIL_PX: f64 = 0.01;

/// 長さ 1 にする(Python の sv_normalize と同じ式。長さ 0 なら NaN)。
#[inline]
pub(crate) fn normalize3(v: Vec3) -> Vec3 {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

/// 接点 r の接平面の (ξ, η)(ラジアン)を単位ベクトルへ(Python の sv_tan_deproject と同じ足し算の順)。
#[inline]
pub(crate) fn deproject(xi: f64, eta: f64, r: Vec3, east: Vec3, north: Vec3) -> Vec3 {
    normalize3([
        r[0] + xi * east[0] + eta * north[0],
        r[1] + xi * east[1] + eta * north[1],
        r[2] + xi * east[2] + eta * north[2],
    ])
}

/// 2×2 の連立一次方程式(部分ピボットつきの LU。numpy の solve と同じやり方)。
fn solve2(a: &Mat2, b: [f64; 2]) -> [f64; 2] {
    let (r0, r1, b0, b1) =
        if a[1][0].abs() > a[0][0].abs() { (a[1], a[0], b[1], b[0]) } else { (a[0], a[1], b[0], b[1]) };
    let l = r1[0] / r0[0];
    let u11 = r1[1] - l * r0[1];
    let y1 = b1 - l * b0;
    let x1 = y1 / u11;
    let x0 = (b0 - r0[1] * x1) / r0[0];
    [x0, x1]
}

/// 2 本の目的値をそれぞれ Σ c[p][q] u^p v^q(min_deg ≤ p+q ≤ order)で重みつき最小二乗に当てる
/// (Python の `_sv_poly_lstsq`)。u, v は最大の絶対値で割ってから当てる。戻り値は行優先の (order+1)² 2 本。
fn poly_lstsq(
    u: &[f64],
    v: &[f64],
    t: &[[f64; 2]],
    order: usize,
    min_deg: usize,
    weights: Option<&[f64]>,
) -> Result<[Vec<f64>; 2], String> {
    let n = u.len();
    let terms = poly_terms(order, min_deg);
    let k = terms.len();
    let mut s = 1.0f64;
    let mut bad = false;
    for i in 0..n {
        let (a, b) = (u[i].abs(), v[i].abs());
        if a.is_nan() || b.is_nan() {
            bad = true;
        }
        s = s.max(a).max(b);
    }
    if bad {
        return Err("当てはめの点に有限でない値がある".to_string());
    }
    let mut a = vec![0.0f64; n * k];
    let mut b = vec![0.0f64; n * 2];
    for i in 0..n {
        let sw = weights.map_or(1.0, |w| w[i].sqrt());
        let us = u[i] / s;
        let vs = v[i] / s;
        for (j, &(p, q)) in terms.iter().enumerate() {
            a[j * n + i] = us.powi(p as i32) * vs.powi(q as i32) * sw;
        }
        b[i] = t[i][0] * sw;
        b[n + i] = t[i][1] * sw;
    }
    let sol = lstsq_qr(&mut a, n, k, &mut b, 2).map_err(|rank| {
        format!("多項式(次数 {order})を当てるには点が足りないか、並びが縮退している(項 {k}、階数 {rank})")
    })?;
    let m = order + 1;
    let mut c0 = vec![0.0f64; m * m];
    let mut c1 = vec![0.0f64; m * m];
    for (j, &(p, q)) in terms.iter().enumerate() {
        let d = s.powi((p + q) as i32);
        c0[p * m + q] = sol[j] / d;
        c1[p * m + q] = sol[k + j] / d;
    }
    Ok([c0, c1])
}

/// 対応する画像の点と星から、TAN(+SIP)を重みつき最小二乗で当てる(Python の `sv_fit_wcs_lsq`)。
///
/// CRPIX は固定(既定は画像の点の重みつき重心)。接点(既定は星の重みつきの和の方向)のまわりで星を
/// 投影し、ξ と η をそれぞれ u, v(画素 − CRPIX)の多項式(0 ≤ p+q ≤ 次数、次数 = max(1, sip_order))で当てる。
/// 定数項を CD⁻¹ で画素に直した大きさが 1e-8 px 以下になるまで接点を動かして当て直す(iterations は上限)。
/// 上限まで動かしても 0.01 px を超えるずれが残れば失敗。sip_order ≥ 2 なら SIP と逆変換(次数 sip_order+1、
/// `size` = (幅, 高さ) の画像全体の格子。無ければ点の範囲)も当てる。重み 0 の点は使わない。
#[allow(clippy::too_many_arguments)]
pub(crate) fn fit_wcs_lsq(
    field: &[[f64; 2]],
    stars: &[Vec3],
    weights: Option<&[f64]>,
    crpix: Option<[f64; 2]>,
    sip_order: usize,
    crval: Option<Vec3>,
    iterations: usize,
    size: Option<(f64, f64)>,
) -> Result<Wcs, String> {
    let n = field.len();
    if n != stars.len() {
        return Err(format!("画像の点と星の数が合わない(画像 {n}、星 {})", stars.len()));
    }
    if let Some(w) = weights
        && w.len() != n
    {
        return Err(format!("重みの数が合わない(重み {}、点 {n})", w.len()));
    }
    let mut w = vec![0.0f64; n];
    let mut f = vec![[0.0f64; 2]; n];
    let mut s = vec![[0.0f64; 3]; n];
    for i in 0..n {
        let wi = weights.map_or(1.0, |ws| ws[i]);
        let si = normalize3(stars[i]);
        let fi = field[i];
        if wi.is_finite() && wi > 0.0 && fi.iter().all(|x| x.is_finite()) && si.iter().all(|x| x.is_finite()) {
            w[i] = wi;
            f[i] = fi;
            s[i] = si;
        }
    }
    if !w.iter().any(|&x| x > 0.0) {
        return Err("重みが正の点が無い".to_string());
    }
    let crpix = match crpix {
        Some(c) => c,
        None => {
            let (mut c, mut ws) = ([0.0f64; 2], 0.0f64);
            for i in 0..n {
                c[0] += w[i] * f[i][0];
                c[1] += w[i] * f[i][1];
                ws += w[i];
            }
            [c[0] / ws, c[1] / ws]
        }
    };
    let crval = match crval {
        Some(c) => c,
        None => {
            let mut c = [0.0f64; 3];
            for i in 0..n {
                for k in 0..3 {
                    c[k] += w[i] * s[i][k];
                }
            }
            if (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() < 1e-12 {
                return Err("星の重みつきの和が 0(接点を決められない)".to_string());
            }
            c
        }
    };
    let mut r = normalize3(crval);
    if !(r.iter().all(|x| x.is_finite()) && crpix.iter().all(|x| x.is_finite())) {
        return Err("接点か CRPIX が有限でない".to_string());
    }
    let order = sip_order.max(1);
    if order >= zahyou_wcs::MAX_SIP_ORDER {
        return Err(format!("SIP の次数 {sip_order} が大きすぎる"));
    }
    let u: Vec<f64> = f.iter().map(|p| p[0] - crpix[0]).collect();
    let v: Vec<f64> = f.iter().map(|p| p[1] - crpix[1]).collect();
    let mut ww = vec![0.0f64; n];
    let mut t = vec![[0.0f64; 2]; n];
    let deg = 180.0 / std::f64::consts::PI;
    let rad = std::f64::consts::PI / 180.0;
    let mut coef;
    let mut cd: Mat2;
    let mut left;
    let mut k = 0usize;
    loop {
        let (east, north) = tan_basis(r);
        for i in 0..n {
            let si = s[i];
            let wp = si[0] * r[0] + si[1] * r[1] + si[2] * r[2];
            if wp > 0.0 {
                ww[i] = w[i];
                let xi = (si[0] * east[0] + si[1] * east[1] + si[2] * east[2]) / wp;
                let eta = (si[0] * north[0] + si[1] * north[1] + si[2] * north[2]) / wp;
                t[i] = [xi * deg, eta * deg];
            } else {
                ww[i] = 0.0;
                t[i] = [0.0, 0.0];
            }
        }
        coef = poly_lstsq(&u, &v, &t, order, 0, Some(&ww))?;
        let m = order + 1;
        cd = [[coef[0][m], coef[0][1]], [coef[1][m], coef[1][1]]];
        if !(det2(&cd).abs() >= 1e-300) || !coef.iter().flatten().all(|x| x.is_finite()) {
            return Err("CD が特異(点が一直線に並んでいる)".to_string());
        }
        // 定数項 = CRPIX が写る (ξ, η)。画素に直して十分小さければ終わり
        let sol = solve2(&cd, [coef[0][0], coef[1][0]]);
        left = sol[0].hypot(sol[1]);
        if left <= LSQ_TOL_PX || k >= iterations {
            break;
        }
        // 接点をそこへ動かして当て直す(最後の当てはめのあとには動かさない)
        r = deproject(coef[0][0] * rad, coef[1][0] * rad, r, east, north);
        k += 1;
    }
    if !(left <= LSQ_FAIL_PX) {
        return Err(format!("接点が定まりません({iterations} 回動かしても CRPIX に {left:.3e} px のずれが残る)"));
    }
    if order < 2 {
        return Ok(Wcs::tan(r, crpix, cd));
    }
    let ci = inv2(&cd).ok_or_else(|| "CD が特異".to_string())?;
    let m = order + 1;
    let mut a = vec![0.0f64; m * m];
    let mut b = vec![0.0f64; m * m];
    for p in 0..m {
        for q in 0..m {
            if p + q < 2 {
                continue;
            }
            let (h0, h1) = (coef[0][p * m + q], coef[1][p * m + q]);
            a[p * m + q] = ci[0][0] * h0 + ci[0][1] * h1;
            b[p * m + q] = ci[1][0] * h0 + ci[1][1] * h1;
        }
    }
    let sip = Sip::new(order, a, b)?;
    let wcs = Wcs::new(r, crpix, cd, Some(sip));
    let out = match size {
        Some((wd, ht)) => fit_sip_inverse(&wcs, wd, ht, Some(order + 1)),
        None => {
            let mut bx = [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY];
            for i in 0..n {
                if w[i] > 0.0 {
                    bx[0] = bx[0].min(f[i][0]);
                    bx[1] = bx[1].max(f[i][0]);
                    bx[2] = bx[2].min(f[i][1]);
                    bx[3] = bx[3].max(f[i][1]);
                }
            }
            fit_sip_inverse_bounds(&wcs, bx, Some(order + 1))
        }
    };
    out.map_err(|e| e.to_string())
}

/// 重みつきの残差の RMS(px。Python の `_sv_wrms`)。使える点が無ければ +∞。
pub(crate) fn wrms(wcs: &Wcs, f: &[[f64; 2]], s: &[Vec3], w: &[f64]) -> f64 {
    let r = wcs.residuals(f, s, false);
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (ri, &wi) in r.iter().zip(w) {
        let r2 = ri[0] * ri[0] + ri[1] * ri[1];
        if r2.is_finite() && wi > 0.0 {
            num += wi * r2;
            den += wi;
        }
    }
    if den > 0.0 { (num / den).sqrt() } else { f64::INFINITY }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zahyou_wcs::{ARCSEC_PER_RAD, radec_to_xyz};

    #[test]
    fn lsq_recovers_wide_field_off_center() {
        // 広い視野(40°)で星が片側だけでも、中心を CRPIX にして当てれば元の TAN に戻る
        let truth = Wcs::tan(radec_to_xyz(120.0, 30.0), [999.5, 649.5], [[-0.02, 0.0], [0.0, 0.02]]);
        let mut f = Vec::new();
        let mut s = Vec::new();
        for i in 0..8 {
            for j in 0..10 {
                let p = [20.0 + 70.0 * i as f64, 30.0 + 120.0 * j as f64];
                f.push(p);
                s.push(truth.pix_to_xyz(p[0], p[1]));
            }
        }
        let start = truth.pix_to_xyz(999.5, 649.5);
        let got = fit_wcs_lsq(&f, &s, None, Some([999.5, 649.5]), 0, Some(start), LSQ_MAX_SHIFTS, None).unwrap();
        for &(x, y) in &[(0.0, 0.0), (1999.0, 1299.0), (999.5, 649.5)] {
            let a = got.pix_to_xyz(x, y);
            let b = truth.pix_to_xyz(x, y);
            assert!(zahyou_wcs::angle_between(a, b) * ARCSEC_PER_RAD < 1e-6);
        }
        assert!(wrms(&got, &f, &s, &vec![1.0; f.len()]) < 1e-6);
        // 点が足りなければ失敗(panic しない)
        assert!(fit_wcs_lsq(&f[..2], &s[..2], None, None, 0, None, 40, None).is_err());
        assert!(fit_wcs_lsq(&f[..2], &s[..3], None, None, 0, None, 40, None).is_err());
    }
}

//! WCS の当てはめ。Python 版 `dev/solver_wcs.py` の `sv_fit_similarity`・`sv_fit_wcs_lsq`・`sv_fit_sip_inverse`
//! (設計書 design-native-solver.md の 3.6・3.8 節)。
//!
//! 小さな行列の解法は自前(2×2 の極分解、QR の最小二乗。linalg.rs)。

use std::fmt;

use crate::geom::{Vec3, norm, tan_basis, tan_deproject, tan_project_with};
use crate::linalg::{Mat2, det2, inv2, lstsq_qr, orthogonal_polar2, transpose2};
use crate::sip::{MAX_SIP_ORDER, Sip, poly_terms};
use crate::wcs::Wcs;

/// 当てはめに失敗した理由(Python 版の ValueError の文)。
#[derive(Clone, Debug, PartialEq)]
pub enum FitError {
    /// 画像の点と星の数が合わない
    CountMismatch { field: usize, stars: usize },
    /// 重みの数が点の数と合わない
    WeightCountMismatch { weights: usize, points: usize },
    /// 点が足りない(相似変換は 2 点以上)
    TooFewPoints(usize),
    /// 重みが正の(使える)点が無い
    NoUsablePoints,
    /// 星の(重みつきの)和が 0 で接点を決められない
    ZeroStarSum,
    /// 接点から 90° 以上離れた星がある
    StarBehindTangent,
    /// 画像の点か星がすべて同じ位置にある
    Coincident,
    /// 多項式を当てるには点が足りないか、並びが縮退している
    RankDeficient { order: usize, terms: usize, rank: usize },
    /// CD が特異(点が一直線に並んでいる)
    SingularCd,
    /// 次数が大きすぎる
    OrderTooHigh(usize),
    /// 入力の範囲や接点が有限でない
    NotFinite,
}

impl fmt::Display for FitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FitError::CountMismatch { field, stars } => {
                write!(f, "画像の点と星の数が合わない(画像 {field}、星 {stars})")
            }
            FitError::WeightCountMismatch { weights, points } => {
                write!(f, "重みの数が合わない(重み {weights}、点 {points})")
            }
            FitError::TooFewPoints(n) => write!(f, "点が 2 未満({n})"),
            FitError::NoUsablePoints => write!(f, "重みが正の点が無い"),
            FitError::ZeroStarSum => write!(f, "星の和が 0(接点を決められない)"),
            FitError::StarBehindTangent => write!(f, "接点から 90° 以上離れた星がある"),
            FitError::Coincident => write!(f, "画像の点か星がすべて同じ位置にある"),
            FitError::RankDeficient { order, terms, rank } => write!(
                f,
                "多項式(次数 {order})を当てるには点が足りないか、並びが縮退している(項 {terms}、階数 {rank})"
            ),
            FitError::SingularCd => write!(f, "CD が特異(点が一直線に並んでいる)"),
            FitError::OrderTooHigh(n) => write!(f, "次数 {n} は扱えない(SIP は {} 次まで)", MAX_SIP_ORDER - 1),
            FitError::NotFinite => write!(f, "入力に有限でない値がある"),
        }
    }
}

impl std::error::Error for FitError {}

// --------------------------------------------------------- 相似変換(4 点) ---

/// 対応する画像の点と星から、回転+等方スケールの TAN を当てる(4 点からの仮の解)。失敗は None。
///
/// 中身は [`try_fit_similarity`]。
pub fn fit_similarity(field: &[[f64; 2]], stars: &[Vec3]) -> Option<Wcs> {
    try_fit_similarity(field, stars).ok()
}

/// 対応する画像の点と星から、回転+等方スケールの TAN を当てる(設計書 3.6 節)。
///
/// 接点 r = 星の単位ベクトルの和の方向、CRPIX = 画像の点の重心。星を r で投影した (ξ, η) と画像の点から
/// それぞれ重心を引き、C = Σ fᵢ pᵢᵀ = U S Vᵀ、R = V Uᵀ(行列式の符号は見ない = 鏡像も表せる)、
/// スケール = √(Σ|pᵢ|² / Σ|fᵢ|²)、CD = スケール × R。
/// C が階数 1 以下(点が一直線、2 点だけなど)のときは向きが一意に決まらず、鏡像なしの側を返す。
pub fn try_fit_similarity(field: &[[f64; 2]], stars: &[Vec3]) -> Result<Wcs, FitError> {
    let n = field.len();
    if n != stars.len() {
        return Err(FitError::CountMismatch { field: n, stars: stars.len() });
    }
    if n < 2 {
        return Err(FitError::TooFewPoints(n));
    }
    let mut tot = [0.0f64; 3];
    let mut s: Vec<Vec3> = Vec::with_capacity(n);
    for v in stars {
        let l = norm(*v);
        let u = [v[0] / l, v[1] / l, v[2] / l];
        tot = [tot[0] + u[0], tot[1] + u[1], tot[2] + u[2]];
        s.push(u);
    }
    if !(tot.iter().all(|x| x.is_finite()) && field.iter().flatten().all(|x| x.is_finite())) {
        return Err(FitError::NotFinite);
    }
    let tn = norm(tot);
    if tn < 1e-12 {
        return Err(FitError::ZeroStarSum);
    }
    let r = [tot[0] / tn, tot[1] / tn, tot[2] / tn];
    let (east, north) = tan_basis(r);
    let mut p: Vec<[f64; 2]> = Vec::with_capacity(n);
    for v in &s {
        match tan_project_with(*v, r, east, north) {
            Some((xi, eta)) => p.push([xi, eta]),
            None => return Err(FitError::StarBehindTangent),
        }
    }
    let nf = n as f64;
    let pm = mean2(&p, nf);
    let fc = mean2(field, nf);
    let mut c: Mat2 = [[0.0; 2]; 2];
    let (mut sf, mut sp) = (0.0f64, 0.0f64);
    for (f, q) in field.iter().zip(&p) {
        let g = [f[0] - fc[0], f[1] - fc[1]];
        let h = [q[0] - pm[0], q[1] - pm[1]];
        sf += g[0] * g[0] + g[1] * g[1];
        sp += h[0] * h[0] + h[1] * h[1];
        for i in 0..2 {
            for j in 0..2 {
                c[i][j] += g[i] * h[j];
            }
        }
    }
    if !(sf > 0.0 && sp > 0.0) {
        return Err(FitError::Coincident);
    }
    // C = U S Vᵀ のとき R = V Uᵀ = (U Vᵀ)ᵀ = (C の直交成分)ᵀ
    let rot = transpose2(&orthogonal_polar2(&c).ok_or(FitError::Coincident)?);
    let k = (sp / sf).sqrt().to_degrees();
    let cd = [[k * rot[0][0], k * rot[0][1]], [k * rot[1][0], k * rot[1][1]]];
    Ok(Wcs::tan(r, fc, cd))
}

fn mean2(pts: &[[f64; 2]], n: f64) -> [f64; 2] {
    let mut m = [0.0, 0.0];
    for p in pts {
        m[0] += p[0];
        m[1] += p[1];
    }
    [m[0] / n, m[1] / n]
}

// ------------------------------------------------------------ 多項式の当てはめ ---

/// 2 本の目的値 (t0, t1) をそれぞれ Σ c[p][q] u^p v^q(min_deg ≤ p+q ≤ order)で重みつき最小二乗に当てる。
///
/// 戻り値は 2 つの (order+1)×(order+1) の係数(行優先)。u, v は最大の絶対値で割ってから当てる
/// (高い次数で桁が開きすぎて解けなくなるのを防ぐ。Python 版の `_sv_poly_lstsq` と同じ)。
fn poly_lstsq(
    u: &[f64],
    v: &[f64],
    t: &[[f64; 2]],
    order: usize,
    min_deg: usize,
    weights: Option<&[f64]>,
) -> Result<[Vec<f64>; 2], FitError> {
    let n = u.len();
    let terms = poly_terms(order, min_deg);
    let k = terms.len();
    let mut s = 1.0f64;
    for i in 0..n {
        s = s.max(u[i].abs()).max(v[i].abs());
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
    let sol = lstsq_qr(&mut a, n, k, &mut b, 2).map_err(|rank| FitError::RankDeficient { order, terms: k, rank })?;
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

/// numpy の linspace と同じ等間隔の点(最後の点はちょうど stop)。
fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return Vec::new();
    }
    if num == 1 {
        return vec![start];
    }
    let div = (num - 1) as f64;
    let delta = stop - start;
    let step = delta / div;
    let mut out: Vec<f64> = (0..num)
        .map(|i| if step == 0.0 { i as f64 / div * delta + start } else { i as f64 * step + start })
        .collect();
    out[num - 1] = stop;
    out
}

// ------------------------------------------------------------ SIP の逆変換 ---

/// SIP の逆変換 AP, BP を、画像全体(x = −0.5 … width−0.5、y = −0.5 … height−0.5)の格子で当てる(設計書 3.8 節)。
///
/// `order` の既定は順変換の次数 + 1。AP, BP を付けた複製を返す(SIP が無ければそのままの複製)。
pub fn fit_sip_inverse(wcs: &Wcs, width: f64, height: f64, order: Option<usize>) -> Result<Wcs, FitError> {
    fit_sip_inverse_bounds(wcs, [-0.5, width - 0.5, -0.5, height - 0.5], order)
}

/// `fit_sip_inverse` の、範囲 (xmin, xmax, ymin, ymax)(0 始まりの画素)を直接渡す版。
///
/// 格子は 10 × (次数+1) 点四方。各点を順変換した (U, V) から元の (u, v) へ戻す多項式(0 ≤ p+q ≤ 次数)を当てる。
pub fn fit_sip_inverse_bounds(wcs: &Wcs, bounds: [f64; 4], order: Option<usize>) -> Result<Wcs, FitError> {
    let Some(sip) = wcs.sip.as_ref() else {
        return Ok(wcs.clone());
    };
    let order = order.unwrap_or(sip.order.saturating_add(1));
    if order > MAX_SIP_ORDER {
        return Err(FitError::OrderTooHigh(order));
    }
    if !bounds.iter().all(|x| x.is_finite()) {
        return Err(FitError::NotFinite);
    }
    let [x0, x1, y0, y1] = bounds;
    let n = 10 * (order + 1);
    let xs = linspace(x0, x1, n);
    let ys = linspace(y0, y1, n);
    let mut uu = Vec::with_capacity(n * n);
    let mut vv = Vec::with_capacity(n * n);
    let mut t = Vec::with_capacity(n * n);
    for y in &ys {
        for x in &xs {
            let u = x - wcs.crpix[0];
            let v = y - wcs.crpix[1];
            let (fa, fb) = sip.forward(u, v);
            let (pu, pv) = (u + fa, v + fb);
            uu.push(pu);
            vv.push(pv);
            t.push([u - pu, v - pv]);
        }
    }
    let [ap, bp] = poly_lstsq(&uu, &vv, &t, order, 0, None)?;
    let sip = sip.without_inverse().with_inverse(order, ap, bp).map_err(|_| FitError::OrderTooHigh(order))?;
    Ok(Wcs { sip: Some(sip), ..wcs.clone() })
}

// ---------------------------------------------------------- 最小二乗の当てはめ ---

/// 対応する画像の点と星から、TAN(+SIP)を重みつき最小二乗で当てる。失敗は None。中身は [`try_fit_wcs_lsq`]。
#[allow(clippy::too_many_arguments)]
pub fn fit_wcs_lsq(
    field: &[[f64; 2]],
    stars: &[Vec3],
    weights: Option<&[f64]>,
    crpix: Option<[f64; 2]>,
    sip_order: usize,
    crval: Option<Vec3>,
    iterations: usize,
    size: Option<(f64, f64)>,
) -> Option<Wcs> {
    try_fit_wcs_lsq(field, stars, weights, crpix, sip_order, crval, iterations, size).ok()
}

/// 対応する画像の点と星から、TAN(+SIP)を重みつき最小二乗で当てる(設計書 3.8 節。Python の `sv_fit_wcs_lsq`)。
///
/// CRPIX は固定(既定は画像の点の重みつき重心)。接点(既定は星の重みつきの和の方向)のまわりで星を
/// 投影し、ξ と η をそれぞれ u, v(画素 − CRPIX)の多項式(0 ≤ p+q ≤ 次数、次数 = max(1, sip_order))で当てる。
/// 定数項は接点のずれなので、接点をそのぶん動かして当て直す(`iterations` 回動かし、当てるのは iterations+1 回。
/// Python の既定は 3)。1 次の項が CD、2 次以上に CD⁻¹ を掛けたものが SIP の A, B。sip_order ≥ 2 なら
/// 逆変換 AP, BP(次数 sip_order+1)も、画像全体(`size` = (幅, 高さ)。無ければ点の範囲)の格子で当てる。
/// 重みが 0・負・有限でない点と、座標が有限でない点は使わない。
#[allow(clippy::too_many_arguments)]
pub fn try_fit_wcs_lsq(
    field: &[[f64; 2]],
    stars: &[Vec3],
    weights: Option<&[f64]>,
    crpix: Option<[f64; 2]>,
    sip_order: usize,
    crval: Option<Vec3>,
    iterations: usize,
    size: Option<(f64, f64)>,
) -> Result<Wcs, FitError> {
    let n = field.len();
    if n != stars.len() {
        return Err(FitError::CountMismatch { field: n, stars: stars.len() });
    }
    if let Some(w) = weights
        && w.len() != n
    {
        return Err(FitError::WeightCountMismatch { weights: w.len(), points: n });
    }
    let order = sip_order.max(1);
    if order >= MAX_SIP_ORDER {
        return Err(FitError::OrderTooHigh(sip_order));
    }
    // 使える点だけ重みを残し、使えない点の座標は 0 にする(Python 版と同じ)
    let mut w = vec![0.0f64; n];
    let mut f = vec![[0.0f64; 2]; n];
    let mut s = vec![[0.0f64; 3]; n];
    for i in 0..n {
        let wi = weights.map_or(1.0, |ws| ws[i]);
        let l = norm(stars[i]);
        let si = [stars[i][0] / l, stars[i][1] / l, stars[i][2] / l];
        let fi = field[i];
        if wi.is_finite() && wi > 0.0 && fi.iter().all(|x| x.is_finite()) && si.iter().all(|x| x.is_finite()) {
            w[i] = wi;
            f[i] = fi;
            s[i] = si;
        }
    }
    let wsum: f64 = w.iter().sum();
    if !w.iter().any(|&x| x > 0.0) {
        return Err(FitError::NoUsablePoints);
    }
    let crpix = match crpix {
        Some(c) => c,
        None => {
            let mut c = [0.0f64; 2];
            for i in 0..n {
                c[0] += w[i] * f[i][0];
                c[1] += w[i] * f[i][1];
            }
            [c[0] / wsum, c[1] / wsum]
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
            if norm(c) < 1e-12 {
                return Err(FitError::ZeroStarSum);
            }
            c
        }
    };
    let cn = norm(crval);
    if !(cn.is_finite() && cn > 0.0 && crpix.iter().all(|x| x.is_finite())) {
        return Err(FitError::NotFinite);
    }
    let mut r = [crval[0] / cn, crval[1] / cn, crval[2] / cn];
    let u: Vec<f64> = f.iter().map(|p| p[0] - crpix[0]).collect();
    let v: Vec<f64> = f.iter().map(|p| p[1] - crpix[1]).collect();
    let mut ww = vec![0.0f64; n];
    let mut t = vec![[0.0f64; 2]; n];
    let mut coef;
    let mut k = 0usize;
    loop {
        let (east, north) = tan_basis(r);
        for i in 0..n {
            match tan_project_with(s[i], r, east, north) {
                Some((xi, eta)) => {
                    ww[i] = w[i];
                    t[i] = [xi.to_degrees(), eta.to_degrees()];
                }
                _ => {
                    ww[i] = 0.0;
                    t[i] = [0.0, 0.0];
                }
            }
        }
        coef = poly_lstsq(&u, &v, &t, order, 0, Some(&ww))?;
        if k == iterations {
            break;
        }
        // 定数項 = CRPIX が写る (ξ, η)。接点をそこへ動かして当て直す。最後の当てはめのあとには動かさない
        // (CD は接点の東・北に対する値で、極のそばでは接点をわずかに動かすだけで東・北の向きが大きく回るため)
        r = tan_deproject(coef[0][0].to_radians(), coef[1][0].to_radians(), r);
        k += 1;
    }
    let m = order + 1;
    let cd: Mat2 = [[coef[0][m], coef[0][1]], [coef[1][m], coef[1][1]]];
    let dabs = det2(&cd).abs();
    if dabs.is_nan() || dabs < 1e-300 {
        return Err(FitError::SingularCd);
    }
    if order < 2 {
        return Ok(Wcs::tan(r, crpix, cd));
    }
    let ci = inv2(&cd).ok_or(FitError::SingularCd)?;
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
    let sip = Sip::new(order, a, b).map_err(|_| FitError::NotFinite)?;
    let wcs = Wcs::new(r, crpix, cd, Some(sip));
    match size {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{ARCSEC_PER_RAD, angle_between, radec_to_xyz};

    fn grid_err(a: &Wcs, b: &Wcs, w: f64, h: f64) -> f64 {
        let mut worst = 0.0f64;
        for i in 0..=10 {
            for j in 0..=10 {
                let (x, y) = (-0.5 + w * i as f64 / 10.0, -0.5 + h * j as f64 / 10.0);
                worst = worst.max(angle_between(a.pix_to_xyz(x, y), b.pix_to_xyz(x, y)));
            }
        }
        worst * ARCSEC_PER_RAD / b.pixel_scale_arcsec()
    }

    #[test]
    fn similarity_recovers_truth() {
        for (i, &(ra, dec)) in [(10.0, 20.0), (0.0, 90.0), (300.0, -89.99), (359.9995, 0.0)].iter().enumerate() {
            let t = (33.0 + 71.0 * i as f64).to_radians();
            let s = 1.5 / 3600.0;
            let par = if i % 2 == 0 { 1.0 } else { -1.0 };
            let f = [[100.0, 200.0], [180.0, 230.0], [140.0, 150.0], [120.0, 260.0]];
            let c = mean2(&f, 4.0);
            let truth = Wcs::tan(radec_to_xyz(ra, dec), c, [[s * t.cos(), -s * par * t.sin()], [s * t.sin(), s * par * t.cos()]]);
            let st: Vec<Vec3> = f.iter().map(|p| truth.pix_to_xyz(p[0], p[1])).collect();
            let fit = fit_similarity(&f, &st).unwrap();
            assert_eq!(fit.parity(), truth.parity());
            assert!(grid_err(&fit, &truth, 300.0, 300.0) < 1e-6);
        }
        assert!(fit_similarity(&[[0.0, 0.0]], &[radec_to_xyz(1.0, 2.0)]).is_none());
        assert!(fit_similarity(&[[0.0, 0.0]; 4], &[radec_to_xyz(1.0, 2.0); 4]).is_none());
        assert!(fit_similarity(&[[0.0, 0.0]; 4], &[radec_to_xyz(1.0, 2.0); 3]).is_none());
        let opp = [radec_to_xyz(0.0, 0.0), radec_to_xyz(180.0, 0.0)];
        assert_eq!(try_fit_similarity(&[[0.0, 0.0], [1.0, 1.0]], &opp), Err(FitError::ZeroStarSum));
        assert!(fit_similarity(&[[f64::NAN, 0.0], [1.0, 1.0]], &[radec_to_xyz(1.0, 2.0), radec_to_xyz(1.1, 2.0)]).is_none());
    }

    #[test]
    fn lsq_sip_recovers_truth_and_rejects_degenerate() {
        let s = 1.46 / 3600.0;
        let t = 0.7f64;
        let cd = [[s * t.cos(), -s * t.sin()], [s * t.sin(), s * t.cos()]];
        let mut a = vec![0.0; 9];
        let mut b = vec![0.0; 9];
        a[2 * 3] = 2e-6;
        a[3 + 1] = -1e-6;
        b[2] = 3e-6;
        let truth = Wcs::new(radec_to_xyz(61.9, 41.97), [484.0, 274.0], cd, Some(Sip::new(2, a, b).unwrap()));
        let mut f = Vec::new();
        let mut k = 0.123f64;
        for _ in 0..80 {
            k = (k * 9.7 + 0.31) % 1.0;
            let x = k * 968.0;
            k = (k * 9.7 + 0.31) % 1.0;
            f.push([x, k * 548.0]);
        }
        let st: Vec<Vec3> = f.iter().map(|p| truth.pix_to_xyz(p[0], p[1])).collect();
        let fit = fit_wcs_lsq(&f, &st, None, Some(truth.crpix), 2, None, 3, Some((968.0, 548.0))).unwrap();
        assert!(grid_err(&fit, &truth, 968.0, 548.0) < 1e-6, "{}", grid_err(&fit, &truth, 968.0, 548.0));
        assert_eq!(fit.ap_order(), 3);
        // 縮退
        assert!(matches!(try_fit_wcs_lsq(&f[..2], &st[..2], None, None, 0, None, 3, None), Err(FitError::RankDeficient { .. })));
        assert!(matches!(try_fit_wcs_lsq(&f[..5], &st[..5], None, None, 2, None, 3, None), Err(FitError::RankDeficient { .. })));
        assert_eq!(try_fit_wcs_lsq(&f, &st, Some(&vec![0.0; 80]), None, 0, None, 3, None), Err(FitError::NoUsablePoints));
        assert!(matches!(try_fit_wcs_lsq(&f, &st, Some(&[1.0]), None, 0, None, 3, None), Err(FitError::WeightCountMismatch { .. })));
        assert!(matches!(try_fit_wcs_lsq(&f, &st[..3], None, None, 0, None, 3, None), Err(FitError::CountMismatch { .. })));
        assert!(matches!(try_fit_wcs_lsq(&f, &st, None, None, 50, None, 3, None), Err(FitError::OrderTooHigh(50))));
        let line: Vec<[f64; 2]> = (0..10).map(|i| [500.0 * i as f64 / 9.0, 300.0 * i as f64 / 9.0]).collect();
        let sl: Vec<Vec3> = line.iter().map(|p| truth.pix_to_xyz(p[0], p[1])).collect();
        assert!(try_fit_wcs_lsq(&line, &sl, None, None, 0, None, 3, None).is_err());
        // 最小限の点数ならちょうど解ける
        assert!(fit_wcs_lsq(&f[..3], &st[..3], None, None, 0, None, 3, None).is_some());
        assert!(fit_wcs_lsq(&f[..6], &st[..6], None, None, 2, None, 3, Some((500.0, 500.0))).is_some());
        // NaN の点は使わない
        let mut f2 = f.clone();
        f2[0] = [f64::NAN, 1.0];
        let fit2 = fit_wcs_lsq(&f2, &st, None, Some(truth.crpix), 2, None, 3, Some((968.0, 548.0))).unwrap();
        assert!(grid_err(&fit2, &truth, 968.0, 548.0) < 1e-6);
    }

    #[test]
    fn linspace_matches_numpy() {
        let v = linspace(-0.5, 967.5, 30);
        assert_eq!(v[0], -0.5);
        assert_eq!(v[29], 967.5);
        assert_eq!(v[1], 1.0 * (968.0 / 29.0) - 0.5);
        assert_eq!(linspace(3.0, 3.0, 4), vec![3.0; 4]);
    }
}

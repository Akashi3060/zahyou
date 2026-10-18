//! 小さな行列の計算(2×2 の特異値分解・直交成分、最小二乗の QR)。
//!
//! 外部の行列ライブラリは使わない。扱うのは 2×2 と、列が十数本(SIP 4 次で 15 項)の縦長の行列だけ。

/// 2×2 の行列(行優先、`m[行][列]`)。
pub type Mat2 = [[f64; 2]; 2];

/// 行列式。
#[inline]
pub fn det2(m: &Mat2) -> f64 {
    m[0][0] * m[1][1] - m[0][1] * m[1][0]
}

/// 逆行列。特異(行列式が 0)か、値が有限でなければ None。
pub fn inv2(m: &Mat2) -> Option<Mat2> {
    let d = det2(m);
    if d == 0.0 || !d.is_finite() {
        return None;
    }
    let r = [[m[1][1] / d, -m[0][1] / d], [-m[1][0] / d, m[0][0] / d]];
    if r.iter().flatten().all(|x| x.is_finite()) { Some(r) } else { None }
}

/// 行列の積 a·b。
#[inline]
pub fn mul2(a: &Mat2, b: &Mat2) -> Mat2 {
    [
        [a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
        [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
    ]
}

/// 転置。
#[inline]
pub fn transpose2(m: &Mat2) -> Mat2 {
    [[m[0][0], m[1][0]], [m[0][1], m[1][1]]]
}

/// 回転行列 [[cos t, −sin t], [sin t, cos t]]。
#[inline]
fn rot(t: f64) -> Mat2 {
    let (s, c) = t.sin_cos();
    [[c, -s], [s, c]]
}

/// 2×2 の特異値分解 m = U · diag(s) · Vᵀ。
///
/// 戻り値 (U, s, V)。s[0] ≥ s[1] ≥ 0、U と V は直交行列(回転か鏡映)。
/// Blinn の閉じた式: m を「回転 Q·Rot(a2)」と「鏡映 R·Refl(a1)」の和に分けると
/// m = Rot(φ) · diag(Q+R, Q−R) · Rot(θ)(φ = (a2+a1)/2、θ = (a2−a1)/2)。Q−R < 0 なら U の 2 列目の符号を返す。
pub fn svd2(m: &Mat2) -> (Mat2, [f64; 2], Mat2) {
    let e = 0.5 * (m[0][0] + m[1][1]);
    let f = 0.5 * (m[0][0] - m[1][1]);
    let g = 0.5 * (m[1][0] + m[0][1]);
    let h = 0.5 * (m[1][0] - m[0][1]);
    let q = e.hypot(h);
    let r = f.hypot(g);
    let a1 = g.atan2(f);
    let a2 = h.atan2(e);
    let theta = 0.5 * (a2 - a1);
    let phi = 0.5 * (a2 + a1);
    let mut u = rot(phi);
    let v = transpose2(&rot(theta));
    let sx = q + r;
    let sy = q - r;
    if sy < 0.0 {
        u[0][1] = -u[0][1];
        u[1][1] = -u[1][1];
    }
    (u, [sx, sy.abs()], v)
}

/// 直交成分(極分解 m = P·S の P = U·Vᵀ。行列式の符号は問わない = 回転か鏡映)。
///
/// det m > 0 なら回転 [[E, −H], [H, E]] / √(E²+H²)、det m < 0 なら鏡映 [[F, G], [G, −F]] / √(F²+G²)
/// (E = (m00+m11)/2、F = (m00−m11)/2、G = (m10+m01)/2、H = (m10−m01)/2)。
/// det m = 0(階数 1 以下)では一意に決まらないので回転の側を返す。m が 0 なら None。
pub fn orthogonal_polar2(m: &Mat2) -> Option<Mat2> {
    let e = 0.5 * (m[0][0] + m[1][1]);
    let f = 0.5 * (m[0][0] - m[1][1]);
    let g = 0.5 * (m[1][0] + m[0][1]);
    let h = 0.5 * (m[1][0] - m[0][1]);
    let q = e.hypot(h);
    let r = f.hypot(g);
    if !(q.is_finite() && r.is_finite()) {
        return None;
    }
    if q >= r {
        if q == 0.0 {
            return None;
        }
        Some([[e / q, -h / q], [h / q, e / q]])
    } else {
        Some([[f / r, g / r], [g / r, -f / r]])
    }
}

/// 最小二乗 min ‖A·X − B‖ を、列の入れ替えつきの Householder QR で解く。
///
/// - `a` は n×k の行列を**列優先**(`a[j * n + i]` = i 行 j 列)で渡す。中身は壊れる。
/// - `b` は n×r の右辺を列優先で渡す。中身は壊れる。
/// - 戻り値は X(k×r、列優先 `x[c * k + j]`)。
///
/// 階数は |R_jj| > ε·max(n, k)·|R_00| を満たす対角の数(numpy の lstsq の既定の rcond と同じ考え方)。
/// 階数が k に満たなければ `Err(階数)`。値が有限でないときも `Err(0)`。
pub fn lstsq_qr(a: &mut [f64], n: usize, k: usize, b: &mut [f64], r: usize) -> Result<Vec<f64>, usize> {
    if k == 0 || a.len() != n * k || b.len() != n * r {
        return Err(0);
    }
    if !a.iter().chain(b.iter()).all(|x| x.is_finite()) {
        return Err(0);
    }
    let mut perm: Vec<usize> = (0..k).collect();
    let mut rdiag = vec![0.0f64; k];
    let steps = n.min(k);
    for j in 0..steps {
        // 残りの行で最も長い列を前へ
        let mut best = j;
        let mut best_n = -1.0;
        for c in j..k {
            let s: f64 = a[c * n + j..c * n + n].iter().map(|x| x * x).sum();
            if s > best_n {
                best_n = s;
                best = c;
            }
        }
        if best != j {
            for i in 0..n {
                a.swap(j * n + i, best * n + i);
            }
            perm.swap(j, best);
        }
        let norm = best_n.max(0.0).sqrt();
        if norm == 0.0 {
            // 残りの列はすべて 0 = これ以上は階数が増えない
            break;
        }
        let x0 = a[j * n + j];
        let alpha = if x0 > 0.0 { -norm } else { norm };
        a[j * n + j] = x0 - alpha;
        let vnorm2: f64 = a[j * n + j..j * n + n].iter().map(|x| x * x).sum();
        rdiag[j] = alpha;
        if vnorm2 == 0.0 {
            continue;
        }
        let (head, tail) = a.split_at_mut((j + 1) * n);
        let v = &head[j * n + j..j * n + n];
        for c in 0..(k - j - 1) {
            let col = &mut tail[c * n + j..c * n + n];
            let d: f64 = v.iter().zip(col.iter()).map(|(p, q)| p * q).sum();
            let f = 2.0 * d / vnorm2;
            for (q, p) in col.iter_mut().zip(v.iter()) {
                *q -= f * p;
            }
        }
        for c in 0..r {
            let col = &mut b[c * n + j..c * n + n];
            let d: f64 = v.iter().zip(col.iter()).map(|(p, q)| p * q).sum();
            let f = 2.0 * d / vnorm2;
            for (q, p) in col.iter_mut().zip(v.iter()) {
                *q -= f * p;
            }
        }
    }
    let r0 = rdiag[0].abs();
    let tol = f64::EPSILON * (n.max(k) as f64) * r0;
    let rank = rdiag.iter().filter(|d| d.abs() > tol && r0 > 0.0).count();
    if rank < k {
        return Err(rank);
    }
    // 後退代入 R·Y = (Qᵀ B) の上 k 行。R の非対角は a の j 行目に残っている
    let mut x = vec![0.0f64; k * r];
    let mut y = vec![0.0f64; k];
    for c in 0..r {
        for j in (0..k).rev() {
            let mut s = b[c * n + j];
            for m in (j + 1)..k {
                s -= a[m * n + j] * y[m];
            }
            y[j] = s / rdiag[j];
        }
        for j in 0..k {
            x[c * k + perm[j]] = y[j];
        }
    }
    if x.iter().all(|v| v.is_finite()) { Ok(x) } else { Err(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Mat2, b: &Mat2, tol: f64) -> bool {
        (0..2).all(|i| (0..2).all(|j| (a[i][j] - b[i][j]).abs() <= tol))
    }

    #[test]
    fn svd2_reconstructs_and_orders() {
        // 回転・鏡映・特異・対角・ゼロに近いものを含めて、m = U diag(s) Vᵀ に戻ることと直交性
        let cases: [Mat2; 8] = [
            [[1.0, 2.0], [3.0, 4.0]],
            [[-3.0e-4, 1.8e-4], [-1.8e-4, -3.6e-4]],
            [[2.0, 0.0], [0.0, -5.0]],
            [[1.0, 2.0], [2.0, 4.0]],
            [[0.0, 0.0], [0.0, 0.0]],
            [[1e-300, 0.0], [0.0, 1e-300]],
            [[0.3, -7.0], [0.2, 9.0]],
            [[-1.0, 0.0], [0.0, 1.0]],
        ];
        for m in cases {
            let (u, s, v) = svd2(&m);
            assert!(s[0] >= s[1] && s[1] >= 0.0, "{m:?} {s:?}");
            let back = mul2(&mul2(&u, &[[s[0], 0.0], [0.0, s[1]]]), &transpose2(&v));
            let scale = m.iter().flatten().fold(0.0f64, |a, x| a.max(x.abs())).max(1e-300);
            assert!(close(&back, &m, 1e-14 * scale), "{m:?} -> {back:?}");
            let i2 = [[1.0, 0.0], [0.0, 1.0]];
            assert!(close(&mul2(&transpose2(&u), &u), &i2, 1e-14));
            assert!(close(&mul2(&transpose2(&v), &v), &i2, 1e-14));
            // 直交成分は U Vᵀ と同じ(行列式が 0 でないとき)
            if det2(&m).abs() > 1e-20 * scale * scale {
                let p = orthogonal_polar2(&m).unwrap();
                assert!(close(&p, &mul2(&u, &transpose2(&v)), 1e-13), "{m:?}");
            }
        }
        assert!(orthogonal_polar2(&[[0.0, 0.0], [0.0, 0.0]]).is_none());
        assert!(inv2(&[[1.0, 2.0], [2.0, 4.0]]).is_none());
        assert!(inv2(&[[f64::NAN, 0.0], [0.0, 1.0]]).is_none());
    }

    #[test]
    fn lstsq_exact_and_rank() {
        // y = 1 + 2x − 3x² を 5 点で(列優先)
        let xs = [-1.0, -0.5, 0.0, 0.5, 1.0];
        let n = xs.len();
        let mut a: Vec<f64> = Vec::new();
        for p in 0..3 {
            a.extend(xs.iter().map(|x: &f64| x.powi(p)));
        }
        let mut b: Vec<f64> = xs.iter().map(|x| 1.0 + 2.0 * x - 3.0 * x * x).collect();
        let sol = lstsq_qr(&mut a, n, 3, &mut b, 1).unwrap();
        assert!((sol[0] - 1.0).abs() < 1e-14 && (sol[1] - 2.0).abs() < 1e-14 && (sol[2] + 3.0).abs() < 1e-14);
        // 同じ列が 2 本 = 階数 2
        let mut a2: Vec<f64> = xs.iter().copied().chain(xs.iter().copied()).chain(xs.iter().map(|_| 1.0)).collect();
        let mut b2 = vec![1.0; n];
        assert_eq!(lstsq_qr(&mut a2, n, 3, &mut b2, 1), Err(2));
        // 行が足りない
        let mut a3 = vec![1.0, 2.0, 3.0, 4.0];
        let mut b3 = vec![1.0, 2.0];
        assert!(lstsq_qr(&mut a3, 2, 2, &mut b3, 1).is_ok());
        let mut a4 = vec![1.0, 2.0];
        let mut b4 = vec![1.0];
        assert_eq!(lstsq_qr(&mut a4, 1, 2, &mut b4, 1), Err(1));
        // 長さの合わない入力・NaN は Err(パニックしない)
        assert!(lstsq_qr(&mut [1.0, 2.0], 3, 1, &mut [1.0], 1).is_err());
        assert!(lstsq_qr(&mut [f64::NAN, 1.0], 2, 1, &mut [1.0, 1.0], 1).is_err());
    }
}

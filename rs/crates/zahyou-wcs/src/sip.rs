//! SIP の多項式(FITS の規約、Shupe et al. 2005)。Python 版 `dev/solver_wcs.py` の `_sv_poly_*` と `_sv_sip_*`。
//!
//! u, v = 画素 − CRPIX として
//!   (ξ, η) = CD · (u + A(u, v), v + B(u, v))            A, B は 2 ≤ p+q ≤ 次数
//!   U, V = CD⁻¹ · (ξ, η)、u = U + AP(U, V)、v = V + BP(U, V)   AP, BP は 0 ≤ p+q ≤ 次数
//! 係数は (次数+1)×(次数+1) の行列を行優先で持ち、添字 [p][q](= `p * (次数+1) + q`)が u^p v^q の係数。

/// 扱う多項式の次数の上限(FITS のキー `A_p_q` が 1 桁ずつで書ける範囲。壊れたヘッダーで巨大な配列を作らないため)。
pub const MAX_SIP_ORDER: usize = 9;

/// SIP の係数。
///
/// - `a`, `b`: 順変換(長さ (order+1)²。p+q < 2 と p+q > order の項は 0)
/// - `ap`, `bp`: 逆変換(長さ (ap_order+1)²。p+q > ap_order の項は 0)。逆変換が無ければ `ap_order = 0` で `ap`・`bp` は空
///
/// 作るときは [`Sip::new`] と [`Sip::with_inverse`] を使う(範囲の外の項を 0 にそろえる)。
/// フィールドを直接書き換えて長さが合わなくなっても、計算はパニックせず、足りない係数を 0 とみなす。
#[derive(Clone, Debug, PartialEq)]
pub struct Sip {
    pub order: usize,
    pub a: Vec<f64>,
    pub b: Vec<f64>,
    pub ap_order: usize,
    pub ap: Vec<f64>,
    pub bp: Vec<f64>,
}

/// 多項式の項 (p, q) の並び(min_deg ≤ p+q ≤ order。次数の低い順、同じ次数では p の大きい順)。
pub fn poly_terms(order: usize, min_deg: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for d in min_deg..=order {
        for p in (0..=d).rev() {
            out.push((p, d - p));
        }
    }
    out
}

/// 係数の行列を (n+1)×(n+1) にそろえ、p+q が [min_deg, n] の外の項を 0 にする。
fn tidy(c: Vec<f64>, n: usize, min_deg: usize, what: &str) -> Result<Vec<f64>, String> {
    let m = n + 1;
    if c.len() != m * m {
        return Err(format!("SIP の係数 {what} の数が (次数+1)² = {} でない({} 個)", m * m, c.len()));
    }
    if !c.iter().all(|x| x.is_finite()) {
        return Err(format!("SIP の係数 {what} に有限でない値がある"));
    }
    let mut c = c;
    for p in 0..m {
        for q in 0..m {
            if p + q < min_deg || p + q > n {
                c[p * m + q] = 0.0;
            }
        }
    }
    Ok(c)
}

impl Sip {
    /// 順変換の係数から作る(逆変換なし)。p+q < 2 と p+q > order の項は 0 にする。
    /// 次数は 2〜[`MAX_SIP_ORDER`](1 次以下は項が無い = SIP なし。呼ぶ側で SIP を付けないこと)。
    pub fn new(order: usize, a: Vec<f64>, b: Vec<f64>) -> Result<Sip, String> {
        if !(2..=MAX_SIP_ORDER).contains(&order) {
            return Err(format!("SIP の次数 {order} は扱えない(2〜{MAX_SIP_ORDER})"));
        }
        Ok(Sip {
            order,
            a: tidy(a, order, 2, "A")?,
            b: tidy(b, order, 2, "B")?,
            ap_order: 0,
            ap: Vec::new(),
            bp: Vec::new(),
        })
    }

    /// 逆変換 AP, BP を付ける(0 ≤ p+q ≤ ap_order)。
    pub fn with_inverse(mut self, ap_order: usize, ap: Vec<f64>, bp: Vec<f64>) -> Result<Sip, String> {
        if ap_order > MAX_SIP_ORDER {
            return Err(format!("SIP の逆変換の次数 {ap_order} は扱えない(0〜{MAX_SIP_ORDER})"));
        }
        self.ap = tidy(ap, ap_order, 0, "AP")?;
        self.bp = tidy(bp, ap_order, 0, "BP")?;
        self.ap_order = ap_order;
        Ok(self)
    }

    /// 逆変換を外した複製。
    pub fn without_inverse(&self) -> Sip {
        Sip { ap_order: 0, ap: Vec::new(), bp: Vec::new(), ..self.clone() }
    }

    /// 逆変換 AP, BP を持っているか。
    pub fn has_inverse(&self) -> bool {
        !self.ap.is_empty() && !self.bp.is_empty()
    }

    /// 係数 A_p_q(範囲の外は 0)。
    pub fn a_coef(&self, p: usize, q: usize) -> f64 {
        coef_at(&self.a, self.order, p, q)
    }
    /// 係数 B_p_q。
    pub fn b_coef(&self, p: usize, q: usize) -> f64 {
        coef_at(&self.b, self.order, p, q)
    }
    /// 係数 AP_p_q。
    pub fn ap_coef(&self, p: usize, q: usize) -> f64 {
        coef_at(&self.ap, self.ap_order, p, q)
    }
    /// 係数 BP_p_q。
    pub fn bp_coef(&self, p: usize, q: usize) -> f64 {
        coef_at(&self.bp, self.ap_order, p, q)
    }

    /// 順変換の補正 (A(u, v), B(u, v))。
    pub fn forward(&self, u: f64, v: f64) -> (f64, f64) {
        (poly_eval(&self.a, self.order, u, v), poly_eval(&self.b, self.order, u, v))
    }

    /// 逆変換の補正 (AP(U, V), BP(U, V))。逆変換が無ければ None。
    pub fn inverse(&self, uu: f64, vv: f64) -> Option<(f64, f64)> {
        if !self.has_inverse() {
            return None;
        }
        Some((poly_eval(&self.ap, self.ap_order, uu, vv), poly_eval(&self.bp, self.ap_order, uu, vv)))
    }
}

#[inline]
fn coef_at(c: &[f64], n: usize, p: usize, q: usize) -> f64 {
    if p > n || q > n {
        return 0.0;
    }
    c.get(p * (n + 1) + q).copied().unwrap_or(0.0)
}

/// 冪 1, x, x², …, x^n(n ≤ MAX_SIP_ORDER + 1)。Python 版と同じく掛け算を繰り返して作る。
#[inline]
fn powers(x: f64, n: usize) -> [f64; MAX_SIP_ORDER + 2] {
    let mut out = [0.0; MAX_SIP_ORDER + 2];
    out[0] = 1.0;
    for i in 1..=n.min(MAX_SIP_ORDER + 1) {
        out[i] = out[i - 1] * x;
    }
    out
}

/// Σ coef[p][q] u^p v^q(p+q ≤ n)。足す順番は Python 版の `_sv_poly_eval` と同じ。
#[allow(clippy::needless_range_loop)] // 添字 p, q で係数と冪を対応させる(Python 版と同じ形)
pub fn poly_eval(coef: &[f64], n: usize, u: f64, v: f64) -> f64 {
    let n = n.min(MAX_SIP_ORDER + 1);
    let up = powers(u, n);
    let vp = powers(v, n);
    let mut out = 0.0;
    for p in 0..=n {
        for q in 0..=(n - p) {
            let c = coef_at(coef, n, p, q);
            if c != 0.0 {
                out += c * up[p] * vp[q];
            }
        }
    }
    out
}

/// Σ coef[p][q] u^p v^q の u と v での偏微分。
#[allow(clippy::needless_range_loop)]
pub fn poly_grad(coef: &[f64], n: usize, u: f64, v: f64) -> (f64, f64) {
    let n = n.min(MAX_SIP_ORDER + 1);
    let up = powers(u, n);
    let vp = powers(v, n);
    let mut du = 0.0;
    let mut dv = 0.0;
    for p in 0..=n {
        for q in 0..=(n - p) {
            let c = coef_at(coef, n, p, q);
            if c == 0.0 {
                continue;
            }
            if p > 0 {
                du += (c * p as f64) * up[p - 1] * vp[q];
            }
            if q > 0 {
                dv += (c * q as f64) * up[p] * vp[q - 1];
            }
        }
    }
    (du, dv)
}

/// u + A(u, v) = U、v + B(u, v) = V をニュートン法で解く(Python 版の `_sv_sip_solve`、最大 50 回)。
///
/// (u0, v0) から始める。|残差| < 1e-7 px で収束したら Some((u, v))。
pub fn sip_solve(sip: &Sip, uu: f64, vv: f64, u0: f64, v0: f64) -> Option<(f64, f64)> {
    if !(uu.is_finite() && vv.is_finite() && u0.is_finite() && v0.is_finite()) {
        return None;
    }
    let (a, b, n) = (&sip.a, &sip.b, sip.order);
    let mut u = u0;
    let mut v = v0;
    for _ in 0..50 {
        let fa = u + poly_eval(a, n, u, v) - uu;
        let fb = v + poly_eval(b, n, u, v) - vv;
        let (au, av) = poly_grad(a, n, u, v);
        let (bu, bv) = poly_grad(b, n, u, v);
        let (j11, j12, j21, j22) = (1.0 + au, av, bu, 1.0 + bv);
        let det = j11 * j22 - j12 * j21;
        let du = (j22 * fa - j12 * fb) / det;
        let dv = (j11 * fb - j21 * fa) / det;
        u -= du;
        v -= dv;
        let step = du.abs() + dv.abs();
        // NaN の歩みでも打ち切る(Python の np.any(step > …) は NaN との比較が偽になって打ち切る)
        if step.is_nan() || step <= 1e-12 * (1.0 + u.abs() + v.abs()) {
            break;
        }
    }
    let fa = u + poly_eval(a, n, u, v) - uu;
    let fb = v + poly_eval(b, n, u, v) - vv;
    if fa.abs() < 1e-7 && fb.abs() < 1e-7 && u.is_finite() && v.is_finite() { Some((u, v)) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_order() {
        assert_eq!(poly_terms(2, 2), vec![(2, 0), (1, 1), (0, 2)]);
        assert_eq!(poly_terms(1, 0), vec![(0, 0), (1, 0), (0, 1)]);
        assert_eq!(poly_terms(3, 0).len(), 10);
    }

    #[test]
    fn eval_grad_and_solve() {
        // A = 1e-5 u² − 2e-6 uv、B = 3e-6 v² + 1e-8 u³ (3 次)
        let mut a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        a[2 * 4] = 1e-5;
        a[4 + 1] = -2e-6;
        b[2] = 3e-6;
        b[3 * 4] = 1e-8;
        a[1] = 0.5; // 1 次の項は new で消える
        let sip = Sip::new(3, a, b).unwrap();
        assert_eq!(sip.a_coef(0, 1), 0.0);
        let (u, v) = (321.0, -123.0);
        let (fa, fb) = sip.forward(u, v);
        assert!((fa - (1e-5 * u * u - 2e-6 * u * v)).abs() < 1e-15);
        assert!((fb - (3e-6 * v * v + 1e-8 * u * u * u)).abs() < 1e-15);
        let (du, dv) = poly_grad(&sip.a, 3, u, v);
        assert!((du - (2e-5 * u - 2e-6 * v)).abs() < 1e-15 && (dv + 2e-6 * u).abs() < 1e-15);
        let (uu, vv) = (u + fa, v + fb);
        let (su, sv) = sip_solve(&sip, uu, vv, uu, vv).unwrap();
        assert!((su - u).abs() < 1e-9 && (sv - v).abs() < 1e-9);
        assert!(sip_solve(&sip, f64::NAN, 0.0, 0.0, 0.0).is_none());
        // 壊れた係数の長さでもパニックしない
        let broken = Sip { order: 3, a: vec![1.0], b: vec![], ap_order: 2, ap: vec![1.0; 3], bp: vec![] };
        let _ = broken.forward(1.0, 2.0);
        assert!(broken.inverse(1.0, 2.0).is_none());
        assert!(Sip::new(1, vec![0.0; 4], vec![0.0; 4]).is_err());
        assert!(Sip::new(2, vec![0.0; 4], vec![0.0; 9]).is_err());
        assert!(Sip::new(12, vec![0.0; 169], vec![0.0; 169]).is_err());
    }
}

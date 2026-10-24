//! 解法の部品(Python 版 dev/solver_core.py の同じ名前の関数)。探索の本体(search.rs)もこれを使う。
//!
//! 試験(Python の値との突き合わせ)のために公開している。値と並びは Python 版と同じ:
//! - 星の並べ替え: `sv_box_order`・`sv_uniformize`・`sv_order_stars`・`_SvField` の検証の星の並び(3.2 節)
//! - quad 作り: `sv_quad_geometry`・`sv_quads_newest`(newpoint 方式、3.3 節)
//! - コード: `sv_field_codes`・`sv_code_variants`(正準化・境界の近くは両方・鏡像、3.4 節)
//! - quad の大きさの範囲: `sv_quad_ab_range`(3.5 節)
//! - 4 点の当てはめ: `sv_fit_similarity_many` の 1 組ぶん(3.6 節)
//! - 検証の積み上げ: `sv_verify_core` の 1 仮説ぶん(最近傍は Python と同じく単精度の距離で選ぶ。3.7 節)
//! - 一致の重み・F 分布の点・HEALPix の 1 辺: `sv_logodds_weight`・`sv_f_upper`・`sv_healpix_side_rad`
//! - 解の判定: `sv_accept_logodds`・`sv_tuneup_accepted`(3.7 節の 6)
//!
//! 座標の約束は zahyou-wcs と同じ(画素は 0 始まり、x = 列、y = 行。CD は度/px)。

use zahyou_wcs::{ARCSEC_PER_RAD, Mat2, Vec3, linalg::orthogonal_polar2, linalg::transpose2, tan_basis};

use crate::{Params, Parity};

/// SIP を付ける F 検定の有意水準(上側 1 %)の標準正規分布の点(Python の SV_SIP_F_Z)。
pub const SIP_F_Z: f64 = 2.3263478740408408;

// ------------------------------------------------------------------ 星の並べ替え ---

/// `np.clip(np.floor(v), 0, n − 1)` を整数にしたもの(NaN は 0)。
#[inline]
fn clip_floor(v: f64, n: usize) -> usize {
    let f = v.floor();
    if !(f >= 0.0) {
        0
    } else if f >= (n - 1) as f64 {
        n - 1
    } else {
        f as usize
    }
}

/// 画像を nx × ny の箱に分け、「各箱の 1 番目を元の順に → 各箱の 2 番目 → …」と並べた順(添字)。
///
/// xy の行の順を「元の順」とする。箱は画素の範囲 −0.5 … width−0.5 を等分したもの(Python の `sv_box_order`)。
pub fn box_order(xy: &[[f64; 2]], width: f64, height: f64, nx: usize, ny: usize) -> Vec<usize> {
    let n = xy.len();
    if n == 0 || (nx <= 1 && ny <= 1) {
        return (0..n).collect();
    }
    let (nx, ny) = (nx.max(1), ny.max(1));
    let nbox = nx.saturating_mul(ny);
    let boxes: Vec<usize> = xy
        .iter()
        .map(|p| {
            let bx = clip_floor((p[0] + 0.5) / width * nx as f64, nx);
            let by = clip_floor((p[1] + 0.5) / height * ny as f64, ny);
            by * nx + bx
        })
        .collect();
    // 箱の中で何番目か(元の順に数える)。箱の数が多すぎるときは並べ替えで数える
    let mut rank = vec![0usize; n];
    if nbox <= 4 * n + 1024 {
        let mut cnt = vec![0usize; nbox];
        for i in 0..n {
            rank[i] = cnt[boxes[i]];
            cnt[boxes[i]] += 1;
        }
    } else {
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by_key(|&i| boxes[i]);
        let mut k = 0;
        while k < n {
            let mut e = k;
            while e < n && boxes[idx[e]] == boxes[idx[k]] {
                rank[idx[e]] = e - k;
                e += 1;
            }
            k = e;
        }
    }
    let mut out: Vec<usize> = (0..n).collect();
    out.sort_by_key(|&i| rank[i]); // 安定(同じ順位は元の順)
    out
}

/// 一様化の箱の数 (NX, NY) = (max(1, round(√(n·W/H))), max(1, round(n/NX)))(round は四捨五入)。
pub fn uniformize_boxes(width: f64, height: f64, nboxes: usize) -> (usize, usize) {
    let n = nboxes.max(1) as f64;
    let nx = round_half_up((n * width / height).sqrt()).max(1);
    let ny = round_half_up(n / nx as f64).max(1);
    (nx, ny)
}

/// 四捨五入(Python の `_sv_round` = floor(v + 0.5))。負・NaN は 0。
fn round_half_up(v: f64) -> usize {
    let f = (v + 0.5).floor();
    if f >= 0.0 && f < usize::MAX as f64 { f as usize } else { 0 }
}

/// 探索の前の一様化(Python の `sv_uniformize`)。
pub fn uniformize(xy: &[[f64; 2]], width: f64, height: f64, nboxes: usize) -> Vec<usize> {
    let (nx, ny) = uniformize_boxes(width, height, nboxes);
    box_order(xy, width, height, nx, ny)
}

/// 明るい順(フラックスの降順。同じ値は入力の順)の添字。flux が無ければ入力の順。有限でない値は最後へ。
pub fn bright_order(n: usize, flux: Option<&[f64]>) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    if let Some(f) = flux {
        // numpy の argsort(−f, stable) と同じ(−0.0 と 0.0 は同じ値として元の順)
        let key = |i: usize| -> f64 {
            let v = f.get(i).copied().unwrap_or(f64::NAN);
            if v.is_finite() { -v } else { f64::INFINITY }
        };
        idx.sort_by(|&a, &b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
    }
    idx
}

/// 明るい順 → 一様化(Python の `sv_order_stars`)。入力の添字を返す。
pub fn order_stars(xy: &[[f64; 2]], flux: Option<&[f64]>, width: f64, height: f64, nboxes: usize) -> Vec<usize> {
    let bright = bright_order(xy.len(), flux);
    let bxy: Vec<[f64; 2]> = bright.iter().map(|&i| xy[i]).collect();
    uniformize(&bxy, width, height, nboxes).into_iter().map(|k| bright[k]).collect()
}

/// 1 回の探索で使う星の並び(Python の `_SvField` の order・nq・vidx)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldOrder {
    /// 明るい順 → 一様化した順(入力の添字)
    pub order: Vec<usize>,
    /// quad 作りに使う星の数(order の先頭から)
    pub nq: usize,
    /// 検証に使う星(order の先頭 max_verify_stars 個を明るい順に並べ直したもの。入力の添字)
    pub verify: Vec<usize>,
}

/// 星の並び(Python の `_SvField`)。
pub fn field_order(xy: &[[f64; 2]], flux: Option<&[f64]>, width: f64, height: f64, params: &Params) -> FieldOrder {
    let n = xy.len();
    let bright = bright_order(n, flux);
    let bxy: Vec<[f64; 2]> = bright.iter().map(|&i| xy[i]).collect();
    let order: Vec<usize> =
        uniformize(&bxy, width, height, params.uniformize_boxes).into_iter().map(|k| bright[k]).collect();
    let nq = n.min(params.max_quad_stars);
    let mut rank = vec![0usize; n];
    for (k, &i) in bright.iter().enumerate() {
        rank[i] = k;
    }
    let mut verify: Vec<usize> = order[..n.min(params.max_verify_stars)].to_vec();
    verify.sort_by_key(|&i| rank[i]);
    FieldOrder { order, nq, verify }
}

// ------------------------------------------------------------------ quad 作り ---

/// quad 作りに使う星の組の表(Python の `sv_quad_geometry`)。
///
/// `l2[a·n + b]` = |P_a − P_b|²、`inside(a, b, k)` = 星 k が AB を直径とする円の中
/// (コードの許容誤差の分だけ広げる: 中心からの距離² ≤ |AB|²·(1/2 + codetol/√2)²)。
#[derive(Clone, Debug)]
pub struct QuadGeometry {
    pub n: usize,
    pub l2: Vec<f64>,
    inside: Vec<bool>,
}

impl QuadGeometry {
    #[inline]
    pub fn l2(&self, a: usize, b: usize) -> f64 {
        self.l2[a * self.n + b]
    }

    #[inline]
    pub fn inside(&self, a: usize, b: usize, k: usize) -> bool {
        self.inside[(a * self.n + b) * self.n + k]
    }
}

/// quad 作りの表を作る。
pub fn quad_geometry(p: &[[f64; 2]], codetol: f64) -> QuadGeometry {
    let n = p.len();
    let rfac = 0.5 + codetol / 2f64.sqrt();
    let mut l2 = vec![0.0; n * n];
    let mut inside = vec![false; n * n * n];
    for a in 0..n {
        for b in 0..n {
            let d = [p[a][0] - p[b][0], p[a][1] - p[b][1]];
            let l = d[0] * d[0] + d[1] * d[1];
            l2[a * n + b] = l;
            let mid = [(p[a][0] + p[b][0]) / 2.0, (p[a][1] + p[b][1]) / 2.0];
            let r2 = l * rfac * rfac;
            let base = (a * n + b) * n;
            for k in 0..n {
                let e = [p[k][0] - mid[0], p[k][1] - mid[1]];
                inside[base + k] = e[0] * e[0] + e[1] * e[1] <= r2;
            }
        }
    }
    QuadGeometry { n, l2, inside }
}

/// 星 n を含み、ほかの星の番号がすべて n 未満の quad(newpoint 方式。Python の `sv_quads_newest`)。
///
/// (a) n を B にする: A = a < n で |AB|² が [ab2_lo, ab2_hi] の中なら、a・n 以外で番号 < n の円の中の星から
///     内側の星を選ぶ(4 星なら c < d の 2 個、3 星なら 1 個)。並びは a → c → d の小さい順。
/// (b) n を内側の星にする: a < b < n で |AB|² が範囲内かつ n が円の中なら、4 星なら残り 1 個を番号 < n の
///     円の中の星から選ぶ([a, b, c, n])。並びは a → b → c の小さい順。(a) の後ろに (b) を並べる。
/// 戻り値の各行は [A, B, 内側の星…](3 星なら 4 つめは使わない = usize::MAX)。
pub fn quads_newest(n: usize, dimquads: usize, g: &QuadGeometry, ab2_lo: f64, ab2_hi: f64) -> Vec<[usize; 4]> {
    let mut out = Vec::new();
    quads_newest_into(n, dimquads, g, ab2_lo, ab2_hi, &mut out);
    out
}

/// [`quads_newest`] の、結果を out の後ろに足す版。
pub fn quads_newest_into(
    n: usize,
    dimquads: usize,
    g: &QuadGeometry,
    ab2_lo: f64,
    ab2_hi: f64,
    out: &mut Vec<[usize; 4]>,
) {
    if n >= g.n || dimquads < 3 || n + 1 < dimquads {
        return;
    }
    let okab = |a: usize, b: usize| {
        let l = g.l2(a, b);
        l >= ab2_lo && l <= ab2_hi
    };
    let x = usize::MAX;
    // (a) n を B にする
    let mut m: Vec<usize> = Vec::with_capacity(n);
    for a in 0..n {
        if !okab(a, n) {
            continue;
        }
        m.clear();
        m.extend((0..n).filter(|&k| k != a && g.inside(a, n, k)));
        if dimquads == 3 {
            for &c in &m {
                out.push([a, n, c, x]);
            }
        } else {
            for (i, &c) in m.iter().enumerate() {
                for &d in &m[i + 1..] {
                    out.push([a, n, c, d]);
                }
            }
        }
    }
    // (b) n を内側の星にする
    for a in 0..n {
        for b in a + 1..n {
            if !(okab(a, b) && g.inside(a, b, n)) {
                continue;
            }
            if dimquads == 3 {
                out.push([a, b, n, x]);
            } else {
                for c in 0..n {
                    if c != a && c != b && g.inside(a, b, c) {
                        out.push([a, b, c, n]);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ コード ---

/// 画像側のコード(ピクセルのまま。A → (0, 0)、B → (1, 1) の相似変換。Python の `sv_field_codes`)。
///
/// s = dx² + dy²、cosθ = (dx + dy)/s、sinθ = (dy − dx)/s、星 P は u, v = P − A として
/// x = u·cosθ + v·sinθ、y = −u·sinθ + v·cosθ。戻り値は (cx, cy, dx, dy)(3 星は (cx, cy, 0, 0))。
pub fn field_code(p: &[[f64; 2]], quad: &[usize; 4], dimquads: usize) -> [f64; 4] {
    let a = p[quad[0]];
    let d = [p[quad[1]][0] - a[0], p[quad[1]][1] - a[1]];
    let s = d[0] * d[0] + d[1] * d[1];
    let cos = (d[0] + d[1]) / s;
    let sin = (d[1] - d[0]) / s;
    let mut out = [0.0; 4];
    for k in 0..dimquads.saturating_sub(2).min(2) {
        let q = p[quad[2 + k]];
        let (iu, iv) = (q[0] - a[0], q[1] - a[1]);
        out[2 * k] = iu * cos + iv * sin;
        out[2 * k + 1] = -iu * sin + iv * cos;
    }
    out
}

/// 画像側のコードをまとめて。
pub fn field_codes(p: &[[f64; 2]], quads: &[[usize; 4]], dimquads: usize) -> Vec<[f64; 4]> {
    quads.iter().map(|q| field_code(p, q, dimquads)).collect()
}

/// 照合に投げるコードの形(Python の `sv_code_variants` の戻り値)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeVariants {
    /// 問い合わせのコード(3 星は後ろ 2 つが 0)
    pub q: Vec<[f64; 4]>,
    /// 元の codes の行
    pub row: Vec<usize>,
    /// 索引の quad の星 A, B, C(, D) に対応する画像側の quad の列(3 星は 4 つめが usize::MAX)
    pub perm: Vec<[usize; 4]>,
    /// 鏡像の形か
    pub mirror: Vec<bool>,
}

/// 照合に投げるコードの形を全部作る(Python の `sv_code_variants`)。
///
/// - 正準化は索引と同じ: 4 星は cx + dx ≤ 1(破れていれば A↔B = 全成分 1 − v)、cx ≤ dx(破れていれば C↔D)。
///   3 星は cx ≤ 1/2。境界から 1.5·codetol 以内なら入れ替えた形も問い合わせる
///   (= 4 通りの並びのうち cx + dx ≤ 1 + 1.5·codetol かつ cx ≤ dx + 1.5·codetol のものを全部)。
/// - 鏡像: (x, y) を入れ替えた形 (cy, cx, dy, dx) にも同じことをする(parity が Both か Mirror)。
///
/// 並びは (鏡像か, A↔B, C↔D) の組ごと(偽が先)に、その中は元の行の順。
pub fn code_variants(codes: &[[f64; 4]], dimquads: usize, codetol: f64, parity: Parity) -> CodeVariants {
    let lim = 1.5 * codetol;
    let mirrors: &[bool] = match parity {
        Parity::Normal => &[false],
        Parity::Mirror => &[true],
        Parity::Both => &[false, true],
    };
    let four = dimquads == 4;
    let orders: &[(bool, bool)] =
        if four { &[(false, false), (false, true), (true, false), (true, true)] } else { &[(false, false), (true, false)] };
    let mut out = CodeVariants::default();
    for &mir in mirrors {
        for &(ab, cd) in orders {
            let mut perm = if ab { [1, 0, 0, 0] } else { [0, 1, 0, 0] };
            if four {
                if cd {
                    perm[2] = 3;
                    perm[3] = 2;
                } else {
                    perm[2] = 2;
                    perm[3] = 3;
                }
            } else {
                perm[2] = 2;
                perm[3] = usize::MAX;
            }
            for (row, code) in codes.iter().enumerate() {
                let base = if mir {
                    if four { [code[1], code[0], code[3], code[2]] } else { [code[1], code[0], 0.0, 0.0] }
                } else {
                    *code
                };
                let mut c = base;
                if ab {
                    let k = if four { 4 } else { 2 };
                    for v in c.iter_mut().take(k) {
                        *v = 1.0 - *v;
                    }
                }
                if four && cd {
                    c = [c[2], c[3], c[0], c[1]];
                }
                let ok = if four { c[0] + c[2] <= 1.0 + lim && c[0] <= c[2] + lim } else { c[0] <= 0.5 + lim };
                if ok {
                    out.q.push(c);
                    out.row.push(row);
                    out.perm.push(perm);
                    out.mirror.push(mir);
                }
            }
        }
    }
    out
}

/// 索引の段で使える quad の AB の長さの範囲 [minAB, maxAB](px)(Python の `sv_quad_ab_range`)。
///
/// fudge = SCALE_U × codetol / (√2 × s_hi)、minAB = max(SCALE_L / s_hi − fudge, quad_min_frac × 短辺)、
/// maxAB = SCALE_U / s_lo + fudge(SCALE は ″、s は ″/px)。
#[allow(clippy::too_many_arguments)]
pub fn quad_ab_range(
    scale_lo_rad: f64,
    scale_hi_rad: f64,
    s_lo: f64,
    s_hi: f64,
    width: f64,
    height: f64,
    codetol: f64,
    quad_min_frac: f64,
) -> (f64, f64) {
    let l = scale_lo_rad * ARCSEC_PER_RAD;
    let u = scale_hi_rad * ARCSEC_PER_RAD;
    let fudge = u * codetol / (2f64.sqrt() * s_hi);
    let lo = py_max(l / s_hi - fudge, quad_min_frac * width.min(height));
    let hi = u / s_lo + fudge;
    (lo, hi)
}

/// Python の max(a, b)(a < b のときだけ b。NaN の扱いも Python と同じ)。
#[inline]
pub(crate) fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

// ------------------------------------------------------------------ 4 点の当てはめ ---

/// 4 点(3 点)からの回転+等方スケールの TAN(Python の `sv_fit_similarity_many` の 1 組ぶん)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimilarityFit {
    /// 接点(星の単位ベクトルの和の方向)
    pub crval: Vec3,
    /// 接点の東・北(極では東 = +y 軸)
    pub east: Vec3,
    pub north: Vec3,
    /// 画像の点の重心
    pub crpix: [f64; 2],
    /// 度/px
    pub cd: Mat2,
    /// ″/px
    pub scale: f64,
    /// 縮退していない・接点から 90° を越える星が無い
    pub ok: bool,
}

/// numpy の pairwise 和の小さい場合(8 個なら 8 つの部分和を ((0+1)+(2+3))+((4+5)+(6+7))、8 個未満は順に)。
fn np_sum(v: &[f64]) -> f64 {
    if v.len() == 8 {
        ((v[0] + v[1]) + (v[2] + v[3])) + ((v[4] + v[5]) + (v[6] + v[7]))
    } else {
        let mut s = 0.0;
        for x in v {
            s += x;
        }
        s
    }
}

/// 4 点(3 点)から回転+等方スケールを当てる(行列式の符号は見ない = 鏡像も表せる)。
///
/// 接点 r = 星の単位ベクトルの和の方向、CRPIX = 画像の点の重心。星を r で投影した (ξ, η) と画像の点から
/// それぞれ重心を引き、C = Σ fᵢ pᵢᵀ = U S Vᵀ、R = V Uᵀ、スケール = √(Σ|pᵢ|² / Σ|fᵢ|²)、CD = スケール × R。
pub fn fit_similarity_quad(field: &[[f64; 2]], stars: &[Vec3]) -> SimilarityFit {
    let k = field.len().min(stars.len()).min(4);
    let mut s = [[0.0f64; 3]; 4];
    let mut tot = [0.0f64; 3];
    for i in 0..k {
        let v = stars[i];
        let nrm = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        s[i] = [v[0] / nrm, v[1] / nrm, v[2] / nrm];
        for j in 0..3 {
            tot[j] += s[i][j];
        }
    }
    let tn = (tot[0] * tot[0] + tot[1] * tot[1] + tot[2] * tot[2]).sqrt();
    let mut ok = tn > 1e-12 && k >= 2;
    let d = if ok { tn } else { 1.0 };
    let r = [tot[0] / d, tot[1] / d, tot[2] / d];
    let (east, north) = tan_basis(r);
    let mut p = [[0.0f64; 2]; 4];
    for i in 0..k {
        let w = s[i][0] * r[0] + s[i][1] * r[1] + s[i][2] * r[2];
        if !(w > 0.0) {
            ok = false;
        }
        let ws = if w > 0.0 { w } else { 1.0 };
        p[i] = [
            (s[i][0] * east[0] + s[i][1] * east[1] + s[i][2] * east[2]) / ws,
            (s[i][0] * north[0] + s[i][1] * north[1] + s[i][2] * north[2]) / ws,
        ];
    }
    let kf = k.max(1) as f64;
    let mut pm = [0.0f64; 2];
    let mut fc = [0.0f64; 2];
    for i in 0..k {
        pm[0] += p[i][0];
        pm[1] += p[i][1];
        fc[0] += field[i][0];
        fc[1] += field[i][1];
    }
    let pm = [pm[0] / kf, pm[1] / kf];
    let fc = [fc[0] / kf, fc[1] / kf];
    let mut gsq = [0.0f64; 8];
    let mut psq = [0.0f64; 8];
    let mut c: Mat2 = [[0.0; 2]; 2];
    let mut gs = [[0.0f64; 2]; 4];
    let mut ps = [[0.0f64; 2]; 4];
    for i in 0..k {
        let g = [field[i][0] - fc[0], field[i][1] - fc[1]];
        let h = [p[i][0] - pm[0], p[i][1] - pm[1]];
        gs[i] = g;
        ps[i] = h;
        gsq[2 * i] = g[0] * g[0];
        gsq[2 * i + 1] = g[1] * g[1];
        psq[2 * i] = h[0] * h[0];
        psq[2 * i + 1] = h[1] * h[1];
    }
    for a in 0..2 {
        for b in 0..2 {
            let mut v = 0.0;
            for i in 0..k {
                v += gs[i][a] * ps[i][b];
            }
            c[a][b] = v;
        }
    }
    let sf = np_sum(&gsq[..2 * k]);
    let sp = np_sum(&psq[..2 * k]);
    if !(sf > 0.0 && sp > 0.0) {
        ok = false;
    }
    let rot = match orthogonal_polar2(&c) {
        Some(q) => transpose2(&q),
        None => {
            ok = false;
            [[1.0, 0.0], [0.0, 1.0]]
        }
    };
    let sc = if ok { (sp / sf).sqrt() } else { 1.0 };
    let deg = 180.0 / std::f64::consts::PI;
    let cd = [[(sc * rot[0][0]) * deg, (sc * rot[0][1]) * deg], [(sc * rot[1][0]) * deg, (sc * rot[1][1]) * deg]];
    let ok = ok && cd.iter().flatten().all(|v| v.is_finite());
    SimilarityFit { crval: r, east, north, crpix: fc, cd, scale: sc * ARCSEC_PER_RAD, ok }
}

// ------------------------------------------------------------------ 検証の積み上げ ---

/// 検証の積み上げの 1 つの一致(Python の `sv_verify_core` の record の 1 行)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoreMatch {
    /// 並びの何列目か
    pub col: usize,
    /// 画像の星(test_xy の行)
    pub test: usize,
    /// 参照星(この仮説の参照星の並びの中の番号)
    pub reference: usize,
    /// 1 つの一致の対数オッズ(logfg − logd)
    pub logodds: f64,
}

/// 検証の積み上げの結果(Python の `sv_verify_core` の 1 仮説ぶん)。
#[derive(Clone, Debug, PartialEq)]
pub struct CoreResult {
    /// 並びの先頭から何個かまでを使ったときの対数オッズの最大(使える星が無ければ −∞)
    pub best: f64,
    pub nmatch: usize,
    pub ndistract: usize,
    pub nconflict: usize,
    pub nused: usize,
    /// best になった列(無ければ −1)
    pub best_col: i64,
    /// record のときだけ: best の列までに組になっていたもの(参照星ごとに最後の組。列の順)
    pub matches: Vec<CoreMatch>,
}

/// 単精度の座標の丸めの分も含めた、最近傍の候補を集める半径の余裕(px)。
#[inline]
fn nn_margin(r5: f64, coord_max: f64) -> f64 {
    // 座標を単精度にした誤差(1 座標あたり |c|·2⁻²⁴ 以下)が差と 2 乗に乗る分を、十分に広く見る
    8.0 * (coord_max + 1.0) * f32::EPSILON as f64 + r5 * 1e-5 + 1e-3
}

/// 画像の星ごとに 5σ 以内で最も近い参照星(Python の `_sv_nearest` の 1 仮説ぶん)。
///
/// Python と同じく、最近傍は単精度で計算した距離² の最小(同じ値なら番号の小さいほう)で選び、
/// その距離を倍精度で測り直して 25σ² 以内なら採る。候補は x で並べた参照星から 5σ + 余裕の範囲だけを見る
/// (範囲の外の星が単精度の最小になることはない。どの星も 5σ 以内に無ければ結果はどのみち「無し」)。
/// 戻り値は列ごとの (参照星の番号 か −1, 距離²)。
pub(crate) fn nearest_refs(
    test_xy: &[[f64; 2]],
    test_idx: &[i64],
    sig2: &[f64],
    ref_xy: &[[f64; 2]],
) -> (Vec<i64>, Vec<f64>) {
    let nt = test_idx.len();
    let mut nn = vec![-1i64; nt];
    let mut nd2 = vec![f64::INFINITY; nt];
    if ref_xy.is_empty() || nt == 0 {
        return (nn, nd2);
    }
    // x の順に並べた参照星(番号つき)。比較は (x, 番号)。x に NaN が無ければこれは全順序で番号は重ならないので、
    // 安定でも不安定でも並べた結果は同じ(不安定のほうが軽い)。NaN があると全順序でなくなるので、今までどおり安定に
    let cmp = |a: &(f64, u32), b: &(f64, u32)| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1));
    let mut xs: Vec<(f64, u32)> = ref_xy.iter().enumerate().map(|(i, p)| (p[0], i as u32)).collect();
    if xs.iter().any(|e| e.0.is_nan()) {
        xs.sort_by(cmp);
    } else {
        xs.sort_unstable_by(cmp);
    }
    let mut coord_max: f64 = 0.0;
    for p in ref_xy {
        coord_max = coord_max.max(p[0].abs()).max(p[1].abs());
    }
    let r32: Vec<[f32; 2]> = ref_xy.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
    for i in 0..nt {
        let t = test_idx[i];
        if t < 0 {
            continue;
        }
        let Some(txy) = test_xy.get(t as usize) else { continue };
        let lim = 25.0 * sig2.get(i).copied().unwrap_or(f64::NAN);
        if !(lim >= 0.0) {
            continue;
        }
        let cm = coord_max.max(txy[0].abs()).max(txy[1].abs());
        let rad = lim.sqrt() + nn_margin(lim.sqrt(), cm);
        let t32 = [txy[0] as f32, txy[1] as f32];
        let lo = txy[0] - rad;
        let hi = txy[0] + rad;
        let start = xs.partition_point(|e| e.0 < lo);
        let mut best: Option<(f32, u32)> = None;
        for &(x, j) in &xs[start..] {
            if x > hi {
                break;
            }
            let ry = ref_xy[j as usize][1];
            if (ry - txy[1]).abs() > rad {
                continue;
            }
            let r = r32[j as usize];
            let dx = t32[0] - r[0];
            let mut dd = dx * dx;
            let dy = t32[1] - r[1];
            dd += dy * dy;
            best = match best {
                None => Some((dd, j)),
                Some((bd, bj)) if dd < bd || (dd == bd && j < bj) => Some((dd, j)),
                b => b,
            };
        }
        if let Some((_, j)) = best {
            let r = ref_xy[j as usize];
            let e = [txy[0] - r[0], txy[1] - r[1]];
            let m = e[0] * e[0] + e[1] * e[1];
            if m <= lim {
                nn[i] = j as i64;
                nd2[i] = m;
            }
        }
    }
    (nn, nd2)
}

/// 対数オッズの積み上げ(Python の `sv_verify_core` を 1 仮説ぶん。設計書 3.7 節の 5)。
///
/// test_xy は画像の星(検証に使う全部)、test_idx はこの仮説の並び(test_xy の行。−1 は詰め物)、
/// sig2 は列ごとの σ²(px²)、ref_xy はこの仮説の参照星(sweep の順)、logbg = −ln(有効面積)。
///
/// logd(μ) = ln(d + (1 − d)·μ/NR) + logbg、logfg = ln((1 − d)/(2π σᵢ² NR)) − dist²/(2σᵢ²)。
/// logfg < logd(μ) なら偽物。最近傍がすでに別の星 j の組なら、付け替えたほうが尤度が上がるときだけ付け替える
/// (1 対 1)。logodds += (その星の項) − logbg。bail を割ったら打ち切り。record なら一致も返す。
pub fn verify_core(
    test_xy: &[[f64; 2]],
    test_idx: &[i64],
    sig2: &[f64],
    ref_xy: &[[f64; 2]],
    logbg: f64,
    params: &Params,
    record: bool,
) -> CoreResult {
    let (nn, nd2) = nearest_refs(test_xy, test_idx, sig2, ref_xy);
    accumulate(test_idx, sig2, &nn, &nd2, ref_xy.len(), logbg, params, record)
}

/// 積み上げの本体(最近傍は済ませたもの)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn accumulate(
    test_idx: &[i64],
    sig2: &[f64],
    nn: &[i64],
    nd2: &[f64],
    nref: usize,
    logbg: f64,
    params: &Params,
    record: bool,
) -> CoreResult {
    let nt = test_idx.len();
    let mut res = CoreResult {
        best: f64::NEG_INFINITY,
        nmatch: 0,
        ndistract: 0,
        nconflict: 0,
        nused: 0,
        best_col: -1,
        matches: Vec::new(),
    };
    if nt == 0 || nref == 0 {
        // 参照星が無い仮説は検証できない(Python でも best は −∞ のまま、数は 0)
        return res;
    }
    let d = params.distractor;
    let bail = params.logodds_bail;
    let nr = nref as f64;
    let lnfg0 = ((1.0 - d) / (2.0 * std::f64::consts::PI * nr)).ln();
    // 偽物の尤度 logd(μ)(Python と同じ式)。μ は 0 から 1.0 ずつ増える整数なので、列ごとに ln を取り直さず、
    // μ が増えたときだけ計算する。ld_cur = logd(μ)、ld_prev = logd(max(μ − 1, 0))(μ = 0 では両方 logd(0))。
    // μ − 1 は 2^53 未満の整数どうしの引き算で正確なので、ld_prev は今までの列ごとの logd((μ − 1).max(0)) とビットまで同じ。
    // 付け替えの候補の列 j の logd(μ_j) は、j が組になったときのその列の ld そのもの(同じ式・同じ μ)なので ld_at に覚える
    let logd = |x: f64| (d + (1.0 - d) * x / nr).ln() + logbg;
    let mut ld_cur = logd(0.0);
    let mut ld_prev = ld_cur;
    let mut owner = vec![-1i64; nref.max(1)];
    let mut fg_at = vec![0.0f64; nt];
    let mut ld_at = vec![0.0f64; nt];
    let mut sacc = vec![0.0f64; nt + 1];
    let mut lo = 0.0f64;
    let mut mu = 0.0f64;
    let (mut nmatch, mut ndis, mut ncon, mut nused) = (0usize, 0usize, 0usize, 0usize);
    let mut events: Vec<(usize, usize, usize, f64)> = Vec::new(); // (列, 画像の星, 参照星, 1 つの一致の対数オッズ)
    for i in 0..nt {
        let ti = test_idx[i];
        if ti < 0 {
            sacc[i + 1] = sacc[i];
            continue;
        }
        let s2 = sig2[i];
        let k = nn[i];
        let has = k >= 0 && (k as usize) < nref;
        let logfg = if has { lnfg0 - s2.ln() - nd2[i] / (2.0 * s2) } else { f64::NEG_INFINITY };
        let ld = ld_cur;
        let ld_m1 = ld_prev;
        let isdis = logfg < ld;
        let kk = if has { k as usize } else { 0 };
        let j = if has { owner[kk] } else { -1 };
        let conflict = !isdis && j >= 0;
        let newm = !isdis && j < 0;
        let mut do_sw = false;
        let mut switch = 0.0;
        if conflict {
            let jj = j as usize;
            let ldj = ld_at[jj];
            switch = logfg + (ldj - fg_at[jj]) + (sacc[i] - sacc[jj + 1]);
            do_sw = switch > ld;
        }
        let keepc = conflict && !do_sw;
        let term = if isdis || keepc {
            ld
        } else if do_sw {
            switch
        } else {
            logfg
        };
        if newm || do_sw {
            owner[kk] = i as i64;
            fg_at[i] = logfg;
            ld_at[i] = ld;
            if record {
                events.push((i, ti as usize, kk, logfg - ld));
            }
        }
        if newm {
            mu += 1.0;
            nmatch += 1;
            ld_prev = ld_cur;
            ld_cur = logd(mu);
        }
        if isdis {
            ndis += 1;
        }
        if conflict {
            ncon += 1;
        }
        nused += 1;
        sacc[i + 1] = sacc[i] + if isdis || keepc { ld_m1 - ld } else { 0.0 };
        lo = lo + term - logbg;
        if lo > res.best {
            res.best = lo;
            res.nmatch = nmatch;
            res.ndistract = ndis;
            res.nconflict = ncon;
            res.nused = nused;
            res.best_col = i as i64;
        }
        if lo < bail {
            break;
        }
    }
    if record && res.best_col >= 0 {
        // best の列の時点で組になっていたもの(参照星ごとに、best の列までで最後に組になった星)
        let bc = res.best_col as usize;
        let mut last: Vec<Option<(usize, usize, usize, f64)>> = vec![None; nref.max(1)];
        for e in events {
            if e.0 <= bc {
                last[e.2] = Some(e);
            }
        }
        let mut m: Vec<CoreMatch> = last
            .into_iter()
            .flatten()
            .map(|(col, test, reference, logodds)| CoreMatch { col, test, reference, logodds })
            .collect();
        m.sort_by_key(|c| c.col);
        res.matches = m;
    }
    res
}

// ------------------------------------------------------------------ 小さな式 ---

/// 1 つの一致の対数オッズ → 改良の重み(lo > 40 は 1、lo < −700 は 0、ほかは 1 / (1 + e^−lo))。
pub fn logodds_weight(lo: f64) -> f64 {
    if lo > 40.0 {
        1.0
    } else if lo < -700.0 {
        0.0
    } else {
        1.0 / (1.0 + (-lo.clamp(-700.0, 40.0)).exp())
    }
}

/// F 分布(自由度 d1, d2)の上側の点。z は同じ確率の標準正規分布の点(Python の `sv_f_upper`)。
///
/// Paulson の近似(F^(1/3) を正規分布で近似する式を F について解く)。解が無ければ +∞。
pub fn f_upper(d1: f64, d2: f64, z: f64) -> f64 {
    let (a, b) = (1.0 - 2.0 / (9.0 * d2), 1.0 - 2.0 / (9.0 * d1));
    let (c2, c1) = (2.0 / (9.0 * d2), 2.0 / (9.0 * d1));
    let aa = a * a - z * z * c2;
    let bb = a * b;
    let cc = b * b - z * z * c1;
    if aa <= 0.0 || bb * bb - aa * cc < 0.0 {
        return f64::INFINITY;
    }
    ((bb + (bb * bb - aa * cc).sqrt()) / aa).powi(3)
}

/// HEALPix の 1 辺の目安(ラジアン)= √(4π / 12) / nside。
pub fn healpix_side_rad(nside: u32) -> f64 {
    (4.0 * std::f64::consts::PI / 12.0).sqrt() / nside as f64
}

// ------------------------------------------------------------------ 解の判定 ---

/// この探索で nhyp 個の仮説を試したところで、解とみなす対数オッズ(Python の `sv_accept_logodds`。設計書 3.7 節の 6)。
///
/// nhyp が accept_hyps0 以下なら logodds_solve、超えたら logodds_solve + ln(nhyp / accept_hyps0)。
/// 考え方: 偶然の仮説の尤度比が K を超える見込みは 1 つあたり 1/K 以下なので、当て直す前の尤度比に k 個目で 1e9·k/1e5 を
/// 求めれば、探索全体の見込みは 1e−4·(1 + ln(N/1e5)) 以下になる。ただし実装はこの値を最後の改良(一致した星への当て直し)
/// の後の対数オッズと比べ、tune-up を通らない仮説の入口は固定の logodds_solve のままなので、厳密な上限ではない(その経路で
/// 保証されるのは固定の 1e9 と同じ N/1e9 まで)。当て直しの上げ幅が ln(N/1e5) より小さい偶然の解を止める目安で、
/// tune-up 経由の偶然の解は `tuneup_accepted` が受け持つ。
/// 代償: 対数オッズ L の本物の解でも、初めて出るのが N > accept_hyps0·e^(L − logodds_solve) 個目なら捨てる
/// (L = 24 で 2.7e6、26.9 で 4.8e7)。N は探索 1 回で全部の系列にまたがって数えるので、星図データが多いほど・速いほど
/// 同じ締め切りの中で基準が厳しくなる(比較の画像の正しい解に L < 50 かつ N > 1e5 は無いが、星の少ない画像に偽の検出を足して
/// 作ると、仮説 1e5 個を超えて初めて出る 21〜22 の正しい解を落とす。docs/rust-port.md 14.6)。
pub fn accept_logodds(p: &Params, nhyp: u64) -> f64 {
    let n = nhyp as f64;
    if p.accept_hyps0 > 0.0 && n > p.accept_hyps0 {
        p.logodds_solve + (n / p.accept_hyps0).ln()
    } else {
        p.logodds_solve
    }
}

/// この探索で nhyp 個の仮説を試したところで、tune-up(1 次の改良で検証し直す)で solve に届いた仮説を解にしてよいか
/// (Python の `sv_tuneup_accepted`)。
///
/// 改良は検証に使ったのと同じ星に WCS を当て直すので、それだけでオッズが上がる(偶然の仮説で 4〜8.5)。
/// 本物の仮説なら改良で新しい星が並ぶので、nhyp が tune_gain_hyps0 を超えたら、改良後の一致数が 4 点の検証の一致数より
/// tune_min_gain 以上多いことを求める。それまでは求めない: 星の少ない画像の本物の解は、星が全部一致済みで増えようがない
/// (星 8 個の 3I_00010 は仮説 4,553 個目に tune-up で 21.9・一致 4 → 4 で解ける。星 17〜31 個の画像に偽の検出が数個混ざると、
/// quad を試し尽くす探索でも、仮説 1.0e5〜2.4e5 個目に同じ形の正しい解が出る)。見つかった誤った解は、どれも 266 万個目より後。
/// 代償: tune_gain_hyps0 を超えてから tune-up で上がり、一致が増えない本物の候補も捨てる(比較の画像では s-30d-r3 の
/// 仮説 582 万個目の 1 候補で、別の仮説で先に解けていて結果は変わらない)。区切りが 1e5 だったときは、星の少ない画像に
/// 偽の検出を足して作ると 1e5 個のすぐ上で正しい解を落としたので、1e6 にした(docs/rust-port.md 14.6・14.7)。
pub fn tuneup_accepted(p: &Params, nhyp: u64, nmatch_before: usize, nmatch_after: usize) -> bool {
    if !(p.tune_min_gain > 0 && (nhyp as f64) > p.tune_gain_hyps0) {
        return true;
    }
    nmatch_after >= nmatch_before.saturating_add(p.tune_min_gain)
}

/// solve を超えて(tune-up で届いたものを含む)改良まで済ませたのに、改良の後の判定で解にしなかった候補のまとめ
/// (Python の `sv_passed_add`・`sv_passed_note` と st の passed_*)。探索の終わりの「解けませんでした」「解けず」の行に
/// 件数と最大だけを出す(1 候補ごとに出すと、乱数の星野で行が増えるので)。
#[derive(Clone, Debug, Default)]
pub struct PassedOver {
    /// 改良の後の対数オッズが `accept_logodds` に届かなかった数と、その最大・その候補のときに要った値
    pub short: u64,
    pub short_best: f64,
    pub short_need: f64,
    /// 値は届いたが、tune-up で上がって改良で一致が増えなかった(`tuneup_accepted`)数と、その改良の後の対数オッズの最大
    pub nogain: u64,
    pub nogain_best: f64,
}

impl PassedOver {
    /// 改良の後の対数オッズ logodds が、要った値 need に届かなかった候補を数える。
    pub fn add_short(&mut self, logodds: f64, need: f64) {
        if self.short == 0 || logodds > self.short_best {
            self.short_best = logodds;
            self.short_need = need;
        }
        self.short += 1;
    }

    /// tune-up で上がって改良で一致が増えなかった候補(改良の後の対数オッズ logodds)を数える。
    pub fn add_nogain(&mut self, logodds: f64) {
        if self.nogain == 0 || logodds > self.nogain_best {
            self.nogain_best = logodds;
        }
        self.nogain += 1;
    }

    /// 行の末尾に足す文(見送った候補が無ければ空)。
    pub fn note(&self) -> String {
        let mut s = String::new();
        if self.short > 0 {
            s.push_str(&format!(
                "。改良の後の対数オッズが足りず見送った候補 {} 件(最大 {:.1}、要った値 {:.1})",
                crate::pyfmt::fmt_int(self.short),
                self.short_best,
                self.short_need
            ));
        }
        if self.nogain > 0 {
            s.push_str(&format!(
                "。tune-up で一致が増えず見送った候補 {} 件(改良の後の対数オッズの最大 {:.1})",
                crate::pyfmt::fmt_int(self.nogain),
                self.nogain_best
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_example_from_python_case_01() {
        // 968×548・10 箱 → 4 × 3、隅の明るい 2 個と暗い 1 個 → [0, 2, 1]
        assert_eq!(uniformize_boxes(968.0, 548.0, 10), (4, 3));
        let xy = [[10.0, 10.0], [20.0, 10.0], [900.0, 500.0]];
        let o = order_stars(&xy, Some(&[3.0, 2.0, 1.0]), 968.0, 548.0, 10);
        assert_eq!(o, vec![0, 2, 1]);
        // 非有限の明るさは最後、同じ値は元の順
        let o = bright_order(4, Some(&[1.0, f64::NAN, 1.0, 5.0]));
        assert_eq!(o, vec![3, 0, 2, 1]);
    }

    #[test]
    fn newpoint_is_complete_and_unique() {
        // 総当たりで作った quad の集合と一致し、重複が無い
        let mut s = 12345u64;
        let mut rnd = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let p: Vec<[f64; 2]> = (0..14).map(|_| [rnd() * 900.0, rnd() * 600.0]).collect();
        let g = quad_geometry(&p, 0.01);
        let (lo2, hi2) = (60.0f64 * 60.0, 700.0f64 * 700.0);
        for dq in [3usize, 4] {
            let mut got = std::collections::BTreeSet::new();
            for n in 0..p.len() {
                for q in quads_newest(n, dq, &g, lo2, hi2) {
                    let mut inner: Vec<usize> = q[2..dq].to_vec();
                    inner.sort();
                    let key = (q[0].min(q[1]), q[0].max(q[1]), inner);
                    assert!(got.insert(key), "重複 {q:?}");
                }
            }
            let mut want = std::collections::BTreeSet::new();
            let n = p.len();
            for a in 0..n {
                for b in a + 1..n {
                    let l = g.l2(a, b);
                    if !(l >= lo2 && l <= hi2) {
                        continue;
                    }
                    let ins: Vec<usize> = (0..n).filter(|&k| k != a && k != b && g.inside(a, b, k)).collect();
                    if dq == 3 {
                        for &c in &ins {
                            want.insert((a, b, vec![c]));
                        }
                    } else {
                        for (i, &c) in ins.iter().enumerate() {
                            for &d in &ins[i + 1..] {
                                want.insert((a, b, vec![c, d]));
                            }
                        }
                    }
                }
            }
            assert_eq!(got, want);
        }
    }

    #[test]
    fn codes_are_invariant_and_canonical() {
        // 相似変換で動かしてもコードは同じ。正準化した形は 4 通りのうち少なくとも 1 つ
        let p = [[0.0, 0.0], [100.0, 100.0], [30.0, 60.0], [70.0, 50.0]];
        let q = [0usize, 1, 2, 3];
        let c = field_code(&p, &q, 4);
        let th: f64 = 0.7;
        let s = 2.3;
        let t: Vec<[f64; 2]> = p
            .iter()
            .map(|v| [s * (th.cos() * v[0] - th.sin() * v[1]) + 17.0, s * (th.sin() * v[0] + th.cos() * v[1]) - 4.0])
            .collect();
        let c2 = field_code(&t, &q, 4);
        for k in 0..4 {
            assert!((c[k] - c2[k]).abs() < 1e-12);
        }
        let v = code_variants(&[c], 4, 0.01, Parity::Both);
        assert!(!v.q.is_empty());
        for qq in &v.q {
            assert!(qq[0] + qq[2] <= 1.0 + 0.015 && qq[0] <= qq[2] + 0.015);
        }
    }

    #[test]
    fn small_formulas() {
        assert_eq!(logodds_weight(41.0), 1.0);
        assert_eq!(logodds_weight(-701.0), 0.0);
        assert!((logodds_weight(0.0) - 0.5).abs() < 1e-15);
        // F(6, 30) の上側 1 % は 3.47(表の値)。近似の差は 1 % 以内
        assert!((f_upper(6.0, 30.0, SIP_F_Z) / 3.47 - 1.0).abs() < 0.01);
        assert!(f_upper(1.0, 1.0, SIP_F_Z).is_infinite());
        assert!((healpix_side_rad(1) - (std::f64::consts::PI / 3.0).sqrt()).abs() < 1e-15);
    }

    #[test]
    fn accumulate_switches_one_to_one() {
        // 参照星 1 個を 2 つの画像の星が取り合う。近いほうが後から来ても、付け替えで μ が戻る分(偽物の尤度の
        // 下がり)のほうが大きければ付け替えない。値は Python の sv_verify_core と同じ
        let p = Params::default();
        let test_xy = [[10.0, 10.0], [10.2, 10.0], [50.0, 50.0]];
        let refs = [[10.19, 10.0], [50.0, 50.1]];
        let logbg = -(300.0f64 * 300.0).ln();
        let r = verify_core(&test_xy, &[0, 1, 2], &[1.0, 1.0, 1.0], &refs, logbg, &p, true);
        assert_eq!(r.nconflict, 1);
        let m: Vec<(usize, usize)> = r.matches.iter().map(|m| (m.test, m.reference)).collect();
        assert_eq!(m, vec![(0, 0), (2, 1)]);
        assert!((r.best - 16.684663630536924).abs() < 1e-12, "{}", r.best);
        let r = verify_core(&test_xy, &[1, 0, 2], &[1.0, 1.0, 1.0], &refs, logbg, &p, true);
        let m: Vec<(usize, usize)> = r.matches.iter().map(|m| (m.test, m.reference)).collect();
        assert_eq!(m, vec![(1, 0), (2, 1)]);
        assert!((r.best - 16.702663630536925).abs() < 1e-12, "{}", r.best);
        // 参照星が無ければ −∞
        let r = verify_core(&test_xy, &[0, 1], &[1.0, 1.0], &[], 0.0, &p, false);
        assert!(r.best.is_infinite() && r.best < 0.0);
    }

    // ---- 変える前の最近傍と積み上げ(参照実装)。今の形と to_bits まで同じ答えになることを確かめる ----

    fn nearest_refs_ref(
        test_xy: &[[f64; 2]],
        test_idx: &[i64],
        sig2: &[f64],
        ref_xy: &[[f64; 2]],
    ) -> (Vec<i64>, Vec<f64>) {
        let nt = test_idx.len();
        let mut nn = vec![-1i64; nt];
        let mut nd2 = vec![f64::INFINITY; nt];
        if ref_xy.is_empty() || nt == 0 {
            return (nn, nd2);
        }
        // x の順に並べた参照星(番号つき)
        let mut xs: Vec<(f64, u32)> = ref_xy.iter().enumerate().map(|(i, p)| (p[0], i as u32)).collect();
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
        let mut coord_max: f64 = 0.0;
        for p in ref_xy {
            coord_max = coord_max.max(p[0].abs()).max(p[1].abs());
        }
        let r32: Vec<[f32; 2]> = ref_xy.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
        for i in 0..nt {
            let t = test_idx[i];
            if t < 0 {
                continue;
            }
            let Some(txy) = test_xy.get(t as usize) else { continue };
            let lim = 25.0 * sig2.get(i).copied().unwrap_or(f64::NAN);
            if !(lim >= 0.0) {
                continue;
            }
            let cm = coord_max.max(txy[0].abs()).max(txy[1].abs());
            let rad = lim.sqrt() + nn_margin(lim.sqrt(), cm);
            let t32 = [txy[0] as f32, txy[1] as f32];
            let lo = txy[0] - rad;
            let hi = txy[0] + rad;
            let start = xs.partition_point(|e| e.0 < lo);
            let mut best: Option<(f32, u32)> = None;
            for &(x, j) in &xs[start..] {
                if x > hi {
                    break;
                }
                let ry = ref_xy[j as usize][1];
                if (ry - txy[1]).abs() > rad {
                    continue;
                }
                let r = r32[j as usize];
                let dx = t32[0] - r[0];
                let mut dd = dx * dx;
                let dy = t32[1] - r[1];
                dd += dy * dy;
                best = match best {
                    None => Some((dd, j)),
                    Some((bd, bj)) if dd < bd || (dd == bd && j < bj) => Some((dd, j)),
                    b => b,
                };
            }
            if let Some((_, j)) = best {
                let r = ref_xy[j as usize];
                let e = [txy[0] - r[0], txy[1] - r[1]];
                let m = e[0] * e[0] + e[1] * e[1];
                if m <= lim {
                    nn[i] = j as i64;
                    nd2[i] = m;
                }
            }
        }
        (nn, nd2)
    }

    #[allow(clippy::too_many_arguments)]
    fn accumulate_ref(
        test_idx: &[i64],
        sig2: &[f64],
        nn: &[i64],
        nd2: &[f64],
        nref: usize,
        logbg: f64,
        params: &Params,
        record: bool,
    ) -> CoreResult {
        let nt = test_idx.len();
        let mut res = CoreResult {
            best: f64::NEG_INFINITY,
            nmatch: 0,
            ndistract: 0,
            nconflict: 0,
            nused: 0,
            best_col: -1,
            matches: Vec::new(),
        };
        if nt == 0 || nref == 0 {
            // 参照星が無い仮説は検証できない(Python でも best は −∞ のまま、数は 0)
            return res;
        }
        let d = params.distractor;
        let bail = params.logodds_bail;
        let nr = nref as f64;
        let lnfg0 = ((1.0 - d) / (2.0 * std::f64::consts::PI * nr)).ln();
        let mut owner = vec![-1i64; nref.max(1)];
        let mut fg_at = vec![0.0f64; nt];
        let mut mu_at = vec![0.0f64; nt];
        let mut sacc = vec![0.0f64; nt + 1];
        let mut lo = 0.0f64;
        let mut mu = 0.0f64;
        let (mut nmatch, mut ndis, mut ncon, mut nused) = (0usize, 0usize, 0usize, 0usize);
        let mut events: Vec<(usize, usize, usize, f64)> = Vec::new(); // (列, 画像の星, 参照星, 1 つの一致の対数オッズ)
        for i in 0..nt {
            let ti = test_idx[i];
            if ti < 0 {
                sacc[i + 1] = sacc[i];
                continue;
            }
            let s2 = sig2[i];
            let k = nn[i];
            let has = k >= 0 && (k as usize) < nref;
            let logfg = if has { lnfg0 - s2.ln() - nd2[i] / (2.0 * s2) } else { f64::NEG_INFINITY };
            let ld = (d + (1.0 - d) * mu / nr).ln() + logbg;
            let ld_m1 = (d + (1.0 - d) * (mu - 1.0).max(0.0) / nr).ln() + logbg;
            let isdis = logfg < ld;
            let kk = if has { k as usize } else { 0 };
            let j = if has { owner[kk] } else { -1 };
            let conflict = !isdis && j >= 0;
            let newm = !isdis && j < 0;
            let mut do_sw = false;
            let mut switch = 0.0;
            if conflict {
                let jj = j as usize;
                let ldj = (d + (1.0 - d) * mu_at[jj] / nr).ln() + logbg;
                switch = logfg + (ldj - fg_at[jj]) + (sacc[i] - sacc[jj + 1]);
                do_sw = switch > ld;
            }
            let keepc = conflict && !do_sw;
            let term = if isdis || keepc {
                ld
            } else if do_sw {
                switch
            } else {
                logfg
            };
            if newm || do_sw {
                owner[kk] = i as i64;
                fg_at[i] = logfg;
                mu_at[i] = mu;
                if record {
                    events.push((i, ti as usize, kk, logfg - ld));
                }
            }
            if newm {
                mu += 1.0;
                nmatch += 1;
            }
            if isdis {
                ndis += 1;
            }
            if conflict {
                ncon += 1;
            }
            nused += 1;
            sacc[i + 1] = sacc[i] + if isdis || keepc { ld_m1 - ld } else { 0.0 };
            lo = lo + term - logbg;
            if lo > res.best {
                res.best = lo;
                res.nmatch = nmatch;
                res.ndistract = ndis;
                res.nconflict = ncon;
                res.nused = nused;
                res.best_col = i as i64;
            }
            if lo < bail {
                break;
            }
        }
        if record && res.best_col >= 0 {
            // best の列の時点で組になっていたもの(参照星ごとに、best の列までで最後に組になった星)
            let bc = res.best_col as usize;
            let mut last: Vec<Option<(usize, usize, usize, f64)>> = vec![None; nref.max(1)];
            for e in events {
                if e.0 <= bc {
                    last[e.2] = Some(e);
                }
            }
            let mut m: Vec<CoreMatch> = last
                .into_iter()
                .flatten()
                .map(|(col, test, reference, logodds)| CoreMatch { col, test, reference, logodds })
                .collect();
            m.sort_by_key(|c| c.col);
            res.matches = m;
        }
        res
    }

    /// 試験用の乱数(xorshift)。
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn same_core(a: &CoreResult, b: &CoreResult) -> bool {
        a.best.to_bits() == b.best.to_bits()
            && (a.nmatch, a.ndistract, a.nconflict, a.nused, a.best_col) == (b.nmatch, b.ndistract, b.nconflict, b.nused, b.best_col)
            && a.matches.len() == b.matches.len()
            && a.matches.iter().zip(&b.matches).all(|(x, y)| {
                (x.col, x.test, x.reference) == (y.col, y.test, y.reference) && x.logodds.to_bits() == y.logodds.to_bits()
            })
    }

    /// 最近傍(並べ替えを不安定にした)と積み上げ(ln を μ が増えたときだけ計算する)が、変える前の形と to_bits まで同じ。
    /// 乱数の星野で、一致・偽物・取り合い(付け替えあり・なし)・詰め物(−1)・打ち切り・同じ x の参照星・NaN を混ぜる。
    #[test]
    fn nearest_and_accumulate_match_reference() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut nconf, mut nbail, mut nm) = (0usize, 0usize, 0usize);
        for it in 0..20_000 {
            let nt = 1 + (rng.next() * 120.0) as usize;
            let nref = (rng.next() * 80.0) as usize;
            let w = 50.0 + rng.next() * 500.0;
            let test_xy: Vec<[f64; 2]> = (0..nt).map(|_| [rng.next() * w, rng.next() * w]).collect();
            // 参照星: 画像の星の近く(一致)・同じ画像の星の近くに 2 つ(取り合いの元)・ばらばら(偽物)・同じ x
            let mut ref_xy: Vec<[f64; 2]> = Vec::with_capacity(nref);
            for k in 0..nref {
                let t = test_xy[(rng.next() * nt as f64) as usize % nt];
                let p = match k % 5 {
                    0 | 1 => [t[0] + (rng.next() - 0.5) * 3.0, t[1] + (rng.next() - 0.5) * 3.0],
                    2 => [rng.next() * w, rng.next() * w],
                    3 => match ref_xy.last() {
                        Some(q) => [q[0], q[1] + (rng.next() - 0.5) * 2.0],
                        None => [t[0], t[1]],
                    },
                    _ => [t[0] + (rng.next() - 0.5) * 0.5, t[1]],
                };
                ref_xy.push(p);
            }
            if it % 97 == 0 && !ref_xy.is_empty() {
                ref_xy[0][0] = f64::NAN; // x が NaN の参照星(並べ替えが安定に戻る道)
            }
            // 並び: 画像の星の番号(ときどき詰め物 −1、ときどき同じ星を 2 回)
            let test_idx: Vec<i64> = (0..nt)
                .map(|i| match (rng.next() * 20.0) as usize {
                    0 => -1,
                    1 => ((i + 1) % nt) as i64,
                    _ => i as i64,
                })
                .collect();
            let sig2: Vec<f64> = (0..nt).map(|_| 0.2 + rng.next() * 3.0).collect();
            let logbg = -(w * w * (0.2 + rng.next())).ln();
            // 3 回に 1 回は早めに打ち切る
            let bail = if it % 3 == 0 { -5.0 - rng.next() * 30.0 } else { Params::default().logodds_bail };
            let p = Params { distractor: [0.25, 0.1, 0.5, 0.01][it % 4], logodds_bail: bail, ..Params::default() };
            let (nn, nd2) = nearest_refs(&test_xy, &test_idx, &sig2, &ref_xy);
            let (nn0, nd20) = nearest_refs_ref(&test_xy, &test_idx, &sig2, &ref_xy);
            assert_eq!(nn, nn0, "it={it}");
            assert!(nd2.iter().zip(&nd20).all(|(a, b)| a.to_bits() == b.to_bits()), "it={it}");
            for record in [false, true] {
                let a = accumulate(&test_idx, &sig2, &nn, &nd2, ref_xy.len(), logbg, &p, record);
                let b = accumulate_ref(&test_idx, &sig2, &nn, &nd2, ref_xy.len(), logbg, &p, record);
                assert!(same_core(&a, &b), "it={it} record={record}\n{a:?}\n{b:?}");
                if record {
                    nconf += a.nconflict;
                    nm += a.nmatch;
                    nbail += (a.nused < test_idx.iter().filter(|&&t| t >= 0).count()) as usize;
                }
            }
        }
        eprintln!("最近傍と積み上げが変える前と一致: 一致 {nm}・取り合い {nconf}・打ち切り {nbail}");
        assert!(nconf > 1000 && nbail > 100 && nm > 10_000);
    }
}

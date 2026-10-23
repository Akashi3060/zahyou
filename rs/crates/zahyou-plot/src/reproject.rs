//! 北が上・東が左の図への描き直し。Python 版 `plot_north_up` の前半(出力の WCS と大きさを決め、
//! `reproject_interp`(reproject 0.15、双一次補間)で描き直す)を移したもの。
//!
//! - 出力の WCS: TAN、接点 = 画像中心の天球座標、CD = diag(−scale, +scale)(scale は CD の列の長さの平均)。
//!   四隅を写した範囲が収まるように CRPIX と大きさを決める(大きさは ceil(幅) + 1)。
//! - 画素ごとに: 出力の画素 → 天 → 入力の画素(SIP はニュートン法で正確に解く。astropy の all_world2pix と同じく
//!   近似の逆変換 AP・BP は使わない)→ 入力の画素から出力へ戻して 1 画素以上ずれたら捨てる(reproject の往復の検査)。
//! - 補間: reproject の map_coordinates と scipy の map_coordinates(order=1、mode="constant"、cval=NaN)と同じ規則。
//!   入力の端の外側の半画素は端の画素の値、それより外は NaN。入力の NaN は 0、±∞ は float32 の最大・最小
//!   (Python は `np.nan_to_num(data)` を渡している)。
//! - 結果は float32 で持つ(Python は float64。差は float32 の丸めだけ)。行ごとに rayon で並列に計算する。

use rayon::prelude::*;
use zahyou_image::Image2D;
use zahyou_wcs::linalg::inv2;
use zahyou_wcs::{MAX_SIP_ORDER, Mat2, Sip, Vec3, Wcs, radec_to_xyz, tan_basis, tan_deproject_with, tan_project_with};

use crate::PlotError;

/// 北が上の図の画像と WCS。
#[derive(Clone, Debug)]
pub struct NorthUp {
    /// 描き直した画像(範囲の外は NaN)
    pub image: Image2D,
    /// 出力の WCS(TAN、北が上・東が左)
    pub wcs: Wcs,
}

/// 出力の画素数の上限(入力の画素数に対する倍率。45° 回した画像でも 2 倍で収まる)
const MAX_GROWTH: usize = 6;

/// `proj_plane_pixel_scales(wcs)` の平均(度/px)。CD の各列の長さの平均。
pub fn mean_pixel_scale_deg(wcs: &Wcs) -> f64 {
    let cd = &wcs.cd;
    let s0 = (cd[0][0] * cd[0][0] + cd[1][0] * cd[1][0]).sqrt();
    let s1 = (cd[0][1] * cd[0][1] + cd[1][1] * cd[1][1]).sqrt();
    (s0 + s1) / 2.0
}

/// 北が上の図の WCS と大きさ (幅, 高さ) を決める(`plot_north_up` の前半)。
pub fn north_up_frame(wcs: &Wcs, width: usize, height: usize) -> Result<(Wcs, usize, usize), PlotError> {
    if width == 0 || height == 0 {
        return Err(PlotError::EmptyImage);
    }
    let (nx, ny) = (width as f64, height as f64);
    let scale = mean_pixel_scale_deg(wcs);
    if !(scale.is_finite() && scale > 0.0) {
        return Err(PlotError::BadWcs("画素スケールが求められません".into()));
    }
    let (ra, dec) = wcs.pix_to_radec((nx - 1.0) / 2.0, (ny - 1.0) / 2.0);
    if !(ra.is_finite() && dec.is_finite()) {
        return Err(PlotError::BadWcs("画像中心の座標が求められません".into()));
    }
    let cd = [[-scale, 0.0], [0.0, scale]];
    let base = Wcs::tan(radec_to_xyz(ra, dec), [0.0, 0.0], cd);
    let corners = [(0.0, 0.0), (nx - 1.0, 0.0), (0.0, ny - 1.0), (nx - 1.0, ny - 1.0)];
    let mut px = Vec::with_capacity(4);
    let mut py = Vec::with_capacity(4);
    for (x, y) in corners {
        let v = wcs.pix_to_xyz(x, y);
        match base.xyz_to_pix(v) {
            Some((a, b)) if a.is_finite() && b.is_finite() => {
                px.push(a);
                py.push(b);
            }
            _ => return Err(PlotError::BadWcs("画像の四隅を北が上の図へ写せません".into())),
        }
    }
    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (x0, x1, y0, y1) = (min(&px), max(&px), min(&py), max(&py));
    let w_out = (x1 - x0).ceil() + 1.0;
    let h_out = (y1 - y0).ceil() + 1.0;
    let limit = (MAX_GROWTH * width * height).max(1 << 20) as f64;
    if !(w_out.is_finite() && h_out.is_finite()) || w_out * h_out > limit || w_out > 1e6 || h_out > 1e6 {
        return Err(PlotError::TooLarge { width: w_out, height: h_out });
    }
    // FITS の CRPIX は 1 − min(px)(1 始まり)。ここでは 0 始まりなので −min(px)
    let out = Wcs::tan(radec_to_xyz(ra, dec), [-x0, -y0], cd);
    Ok((out, w_out as usize, h_out as usize))
}

/// 画像を北が上・東が左へ描き直す(`plot_north_up` の `reproject_interp` まで)。
pub fn reproject_north_up(img: &Image2D, wcs: &Wcs) -> Result<NorthUp, PlotError> {
    check_image(img)?;
    let (out, w, h) = north_up_frame(wcs, img.width, img.height)?;
    let image = reproject_interp(img, wcs, &out, w, h);
    Ok(NorthUp { image, wcs: out })
}

pub(crate) fn check_image(img: &Image2D) -> Result<(), PlotError> {
    if img.width == 0 || img.height == 0 {
        return Err(PlotError::EmptyImage);
    }
    if img.width.checked_mul(img.height) != Some(img.data.len()) {
        return Err(PlotError::BadImage(format!(
            "画素の数 {} が大きさ {}×{} と合いません",
            img.data.len(),
            img.width,
            img.height
        )));
    }
    Ok(())
}

/// `np.nan_to_num`(float32): NaN → 0、+∞ → 最大、−∞ → 最小。
#[inline]
fn nan_to_num(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else if v == f32::INFINITY {
        f32::MAX
    } else if v == f32::NEG_INFINITY {
        f32::MIN
    } else {
        v
    }
}

/// 入力の座標(1 軸)を reproject の規則で直す。補間できない座標は None。
#[inline]
fn fix_coord(c: f64, n: usize) -> Option<f64> {
    if c.is_nan() {
        return None;
    }
    let nf = n as f64;
    let mut c = c;
    // 外側の半画素は端の画素の中心へ寄せる
    if (-0.5..0.0).contains(&c) {
        c = 0.0;
    }
    if c < nf - 0.5 && c >= nf - 1.0 {
        c = nf - 1.0;
    }
    // それより外は NaN(n − 0.5 ちょうどは scipy の "constant" が範囲外として NaN を返す)
    if (0.0..=nf - 1.0).contains(&c) { Some(c) } else { None }
}

/// 双一次補間(scipy の map_coordinates(order=1) と同じ重みと足す順)。座標は fix_coord を通したもの。
#[inline]
fn bilinear(data: &[f32], w: usize, h: usize, x: f64, y: f64) -> f64 {
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = x - x0;
    let ty = y - y0;
    // scipy: weights[0] = 1 − t、weights[1] = 1 − weights[0]
    let wx0 = 1.0 - tx;
    let wx1 = 1.0 - wx0;
    let wy0 = 1.0 - ty;
    let wy1 = 1.0 - wy0;
    let (ix0, iy0) = (x0 as usize, y0 as usize);
    let ix1 = (ix0 + 1).min(w - 1);
    let iy1 = (iy0 + 1).min(h - 1);
    let at = |ix: usize, iy: usize| nan_to_num(data[iy * w + ix]) as f64;
    let mut t = 0.0;
    t += at(ix0, iy0) * wy0 * wx0;
    t += at(ix1, iy0) * wy0 * wx1;
    t += at(ix0, iy1) * wy1 * wx0;
    t += at(ix1, iy1) * wy1 * wx1;
    t
}

/// SIP の順変換の項(係数が 0 でないもの)。値と偏微分を 1 回の計算で求める。
struct SipTerms {
    n: usize,
    /// (p, q, A_pq, B_pq)
    terms: Vec<(usize, usize, f64, f64)>,
    /// 逆変換(AP・BP)。行の先頭の初めの値に使う
    inv: Option<Sip>,
}

impl SipTerms {
    fn new(s: &Sip) -> SipTerms {
        let n = s.order.min(MAX_SIP_ORDER);
        let mut terms = Vec::new();
        for p in 0..=n {
            for q in 0..=(n - p) {
                let (a, b) = (s.a_coef(p, q), s.b_coef(p, q));
                if a != 0.0 || b != 0.0 {
                    terms.push((p, q, a, b));
                }
            }
        }
        SipTerms { n, terms, inv: if s.has_inverse() { Some(s.clone()) } else { None } }
    }

    /// (A, B, ∂A/∂u, ∂A/∂v, ∂B/∂u, ∂B/∂v)。
    #[inline]
    fn eval(&self, u: f64, v: f64) -> [f64; 6] {
        let mut up = [1.0f64; MAX_SIP_ORDER + 1];
        let mut vp = [1.0f64; MAX_SIP_ORDER + 1];
        for i in 1..=self.n {
            up[i] = up[i - 1] * u;
            vp[i] = vp[i - 1] * v;
        }
        let mut r = [0.0f64; 6];
        for &(p, q, a, b) in &self.terms {
            let m = up[p] * vp[q];
            r[0] += a * m;
            r[1] += b * m;
            if p > 0 {
                let mu = p as f64 * up[p - 1] * vp[q];
                r[2] += a * mu;
                r[4] += b * mu;
            }
            if q > 0 {
                let mv = q as f64 * up[p] * vp[q - 1];
                r[3] += a * mv;
                r[5] += b * mv;
            }
        }
        r
    }

    /// u + A(u, v) = U、v + B(u, v) = V をニュートン法で解く。`guess` が無ければ逆変換(無ければ (U, V))から始める。
    ///
    /// 1 回の修正が 1e-5 px より小さくなったら止める(ニュートン法は 2 乗で収束するので、残りの誤差は
    /// 曲がりの大きさ × 1e-10 px 程度。zahyou-wcs の sip_solve との差は 1e-9 px 未満)。
    #[inline]
    fn solve(&self, uu: f64, vv: f64, guess: Option<(f64, f64)>) -> Option<(f64, f64)> {
        let (mut u, mut v) = match guess {
            Some(g) => g,
            None => match self.inv.as_ref().and_then(|s| s.inverse(uu, vv)) {
                Some((pa, pb)) => (uu + pa, vv + pb),
                None => (uu, vv),
            },
        };
        for _ in 0..50 {
            let [a, b, au, av, bu, bv] = self.eval(u, v);
            let fa = u + a - uu;
            let fb = v + b - vv;
            let (j11, j12, j21, j22) = (1.0 + au, av, bu, 1.0 + bv);
            let det = j11 * j22 - j12 * j21;
            let du = (j22 * fa - j12 * fb) / det;
            let dv = (j11 * fb - j21 * fa) / det;
            u -= du;
            v -= dv;
            let step = du.abs() + dv.abs();
            if !step.is_finite() {
                return None;
            }
            if step < 1e-5 {
                return Some((u, v));
            }
        }
        None
    }
}

/// 出力の画素 → 入力の画素の写し(行ごとに使う下ごしらえ)。
struct Mapper {
    out_r: Vec3,
    out_e: Vec3,
    out_n: Vec3,
    out_cd: Mat2,
    out_crpix: [f64; 2],
    in_r: Vec3,
    in_e: Vec3,
    in_n: Vec3,
    in_cdi: Mat2,
    in_crpix: [f64; 2],
    sip: Option<SipTerms>,
    /// SIP の解が線形の解からこれ以上離れたら別の根とみなす(入力の画像の対角線)
    lim: f64,
}

impl Mapper {
    fn new(wcs_in: &Wcs, wcs_out: &Wcs, w: usize, h: usize) -> Option<Mapper> {
        let (out_e, out_n) = tan_basis(wcs_out.crval);
        let (in_e, in_n) = tan_basis(wcs_in.crval);
        Some(Mapper {
            out_r: wcs_out.crval,
            out_e,
            out_n,
            out_cd: wcs_out.cd,
            out_crpix: wcs_out.crpix,
            in_r: wcs_in.crval,
            in_e,
            in_n,
            in_cdi: inv2(&wcs_in.cd)?,
            in_crpix: wcs_in.crpix,
            sip: wcs_in.sip.as_ref().map(SipTerms::new),
            lim: (w as f64).hypot(h as f64).max(1.0),
        })
    }

    /// 出力の行 y の各画素が写る入力の画素(写せなければ None)。
    fn row(&self, y: f64, out: &mut [Option<(f64, f64)>]) {
        // 直前の 2 画素の解(SIP の初めの値を外挿で作る)
        let mut prev: [Option<(f64, f64)>; 2] = [None, None];
        let cd = &self.out_cd;
        let dy = y - self.out_crpix[1];
        for (j, o) in out.iter_mut().enumerate() {
            let dx = j as f64 - self.out_crpix[0];
            // 出力の画素 → 中間世界座標(度)→ 天(出力は SIP なしの TAN)
            let xi = cd[0][0] * dx + cd[0][1] * dy;
            let eta = cd[1][0] * dx + cd[1][1] * dy;
            let s = tan_deproject_with(xi.to_radians(), eta.to_radians(), self.out_r, self.out_e, self.out_n);
            *o = None;
            let Some((pxi, peta)) = tan_project_with(s, self.in_r, self.in_e, self.in_n) else {
                prev = [None, None];
                continue;
            };
            let (pxi, peta) = (pxi.to_degrees(), peta.to_degrees());
            let ci = &self.in_cdi;
            let uu = ci[0][0] * pxi + ci[0][1] * peta;
            let vv = ci[1][0] * pxi + ci[1][1] * peta;
            if !(uu.is_finite() && vv.is_finite()) {
                prev = [None, None];
                continue;
            }
            let (u, v) = match &self.sip {
                None => (uu, vv),
                Some(sip) => {
                    let guess = match prev {
                        [Some(a), Some(b)] => Some((2.0 * b.0 - a.0, 2.0 * b.1 - a.1)),
                        [None, Some(b)] => Some(b),
                        _ => None,
                    };
                    match sip.solve(uu, vv, guess) {
                        Some((u, v)) if (u - uu).hypot(v - vv) <= self.lim => (u, v),
                        _ => {
                            prev = [None, None];
                            continue;
                        }
                    }
                }
            };
            prev = [prev[1], Some((u, v))];
            let (x, yy) = (u + self.in_crpix[0], v + self.in_crpix[1]);
            if x.is_finite() && yy.is_finite() {
                *o = Some((x, yy));
            }
        }
    }
}

/// `reproject_interp((data, wcs_in), wcs_out, shape_out=(height_out, width_out))` の値(float32 に丸める)。
///
/// 写せない画素・入力の範囲の外は NaN。`img` の大きさが合わなければ全部 NaN。
///
/// reproject は入力の画素 → 出力の画素へ戻して 1 画素以上ずれた点を捨てる(往復の検査)が、ここでは
/// 入力の画素をニュートン法で解いた式そのものが「戻すと一致する」条件なので、解けた点は必ず通る(省く)。
/// 解けなかった点(ニュートン法が収束しない・反対の半球)は NaN。
pub fn reproject_interp(img: &Image2D, wcs_in: &Wcs, wcs_out: &Wcs, width_out: usize, height_out: usize) -> Image2D {
    // 既定値は NaN(写せない画素・範囲の外は下のループが continue してこのまま残る)。NaN は 0 でないので
    // vec! だと 1 本のスレッドで全部を埋める(24 MP で 96 MB)。添字つきの並列の collect なら、予約した領域へ
    // 添字の順に並列で書く。書くのは f32::NAN の定数そのもの(0x7FC00000)なので、vec! の NaN と同じビット。
    // 「既定値 NaN」の形は保つ(0 で確保して写せない画素に NaN を書く形は、書き忘れた経路が黙って黒になる)
    let mut out: Vec<f32> = (0..width_out * height_out).into_par_iter().map(|_| f32::NAN).collect();
    let (w, h) = (img.width, img.height);
    if check_image(img).is_err() || width_out == 0 {
        return Image2D { width: width_out, height: height_out, data: out };
    }
    let Some(map) = Mapper::new(wcs_in, wcs_out, w, h) else {
        return Image2D { width: width_out, height: height_out, data: out };
    };
    // 行の作業用の Vec(6000 画素で 144 KB)は並列の塊ごとに 1 本だけ確保して使い回す。Mapper::row は
    // 各画素の先頭で必ず `*o = None` を書いてから進むので、前の行の値が残っていても結果は同じ
    out.par_chunks_mut(width_out).enumerate().for_each_init(
        || vec![None::<(f64, f64)>; width_out],
        |pix, (row, dst)| {
            map.row(row as f64, pix);
            for (d, p) in dst.iter_mut().zip(pix.iter()) {
                let Some((xi, yi)) = *p else { continue };
                let (Some(cx), Some(cy)) = (fix_coord(xi, w), fix_coord(yi, h)) else { continue };
                *d = bilinear(&img.data, w, h, cx, cy) as f32;
            }
        },
    );
    Image2D { width: width_out, height: height_out, data: out }
}

/// `reproject_interp` を zahyou-wcs の関数だけで素直に書いたもの(試験で速い版と比べる)。往復の検査も行う。
#[doc(hidden)]
pub fn reproject_interp_reference(
    img: &Image2D,
    wcs_in: &Wcs,
    wcs_out: &Wcs,
    width_out: usize,
    height_out: usize,
) -> Image2D {
    let mut out = vec![f32::NAN; width_out * height_out];
    let (w, h) = (img.width, img.height);
    if check_image(img).is_err() || width_out == 0 {
        return Image2D { width: width_out, height: height_out, data: out };
    }
    out.par_chunks_mut(width_out).enumerate().for_each(|(row, dst)| {
        let y = row as f64;
        let pts: Vec<[f64; 2]> = (0..width_out).map(|j| [j as f64, y]).collect();
        let sky: Vec<Vec3> = wcs_out.pix_to_xyz_batch(&pts);
        let inp = wcs_in.xyz_to_pix_batch(&sky, true);
        // 往復の検査: 入力の画素 → 天 → 出力の画素が元から 1 画素以上ずれたら捨てる
        let back_in: Vec<[f64; 2]> = inp.iter().map(|p| p.map_or([f64::NAN, f64::NAN], |(a, b)| [a, b])).collect();
        let back_sky = wcs_in.pix_to_xyz_batch(&back_in);
        let back = wcs_out.xyz_to_pix_batch(&back_sky, true);
        for (j, d) in dst.iter_mut().enumerate() {
            let Some((xi, yi)) = inp[j] else { continue };
            // reproject は差が 1 を超えたときだけ捨てる(戻せず NaN になった点は比べられないので残す)
            if let Some((bx, by)) = back[j]
                && ((bx - j as f64).abs() > 1.0 || (by - y).abs() > 1.0)
            {
                continue;
            }
            let (Some(cx), Some(cy)) = (fix_coord(xi, w), fix_coord(yi, h)) else { continue };
            *d = bilinear(&img.data, w, h, cx, cy) as f32;
        }
    });
    Image2D { width: width_out, height: height_out, data: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_rules_match_scipy() {
        // reproject 0.15 + scipy 1.16 で確かめた値(3×4 の 0..11): (行, 列) → 値
        let img = Image2D::new(4, 3, (0..12).map(|v| v as f32).collect()).unwrap();
        let s = |r: f64, c: f64| match (fix_coord(c, 4), fix_coord(r, 3)) {
            (Some(x), Some(y)) => bilinear(&img.data, 4, 3, x, y),
            _ => f64::NAN,
        };
        assert_eq!(s(2.0, 3.0), 11.0);
        assert_eq!(s(2.0, 2.5), 10.5);
        assert!(s(2.5, 1.0).is_nan());
        assert_eq!(s(2.4999, 1.0), 9.0);
        assert_eq!(s(-0.5, 1.0), 1.0);
        assert!(s(-0.50001, 1.0).is_nan());
        assert_eq!(s(0.0, -0.2), 0.0);
        assert_eq!(s(1.5, 3.0), 9.0);
        assert!(s(f64::NAN, 1.0).is_nan());
        assert!(s(2.9999, 3.4999).is_nan());
        assert_eq!(s(1.25, 2.75), 7.75);
    }

    #[test]
    fn fast_path_matches_reference() {
        use zahyou_wcs::fit_sip_inverse;
        // 強くゆがんだ SIP 3 次(逆変換あり・なし)と TAN で、速い版と素直な版が同じ値になる
        let (w, h) = (300usize, 200usize);
        let data: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 1000) as f32 + (i % w) as f32 * 0.5).collect();
        let img = Image2D::new(w, h, data).unwrap();
        let s = 2.0 / 3600.0;
        let t = 0.6f64;
        let cd = [[-s * t.cos(), s * t.sin()], [s * t.sin(), s * t.cos()]];
        let m = 4;
        let mut a = vec![0.0; m * m];
        let mut b = vec![0.0; m * m];
        a[2 * m] = 3e-5;
        a[m + 1] = -2e-5;
        b[2] = 2.5e-5;
        a[3 * m] = 4e-8;
        b[3] = -5e-8;
        b[m + 2] = 3e-8;
        let sip = Sip::new(3, a, b).unwrap();
        let base = Wcs::new(radec_to_xyz(10.0, 60.0), [140.0, 110.0], cd, Some(sip));
        let inv = fit_sip_inverse(&base, w as f64, h as f64, None).unwrap();
        for wcs in [base.clone(), inv, base.tan_only()] {
            let (out, wo, ho) = north_up_frame(&wcs, w, h).unwrap();
            let fast = reproject_interp(&img, &wcs, &out, wo, ho);
            let refr = reproject_interp_reference(&img, &wcs, &out, wo, ho);
            let mut worst = 0.0f64;
            for (x, y) in fast.data.iter().zip(&refr.data) {
                assert_eq!(x.is_nan(), y.is_nan());
                if !x.is_nan() {
                    worst = worst.max((x - y).abs() as f64);
                }
            }
            // 値は 0〜1150 ほど。float32 の丸め(1e-4)程度まで同じ
            assert!(worst <= 2e-4, "速い版と素直な版の差 {worst}");
        }
    }

    /// 旧版の reproject_interp(vec! の NaN 埋め、行ごとに作業用の Vec を確保)。新版と比べる参照
    fn reproject_interp_old(
        img: &Image2D,
        wcs_in: &Wcs,
        wcs_out: &Wcs,
        width_out: usize,
        height_out: usize,
    ) -> Image2D {
        let mut out = vec![f32::NAN; width_out * height_out];
        let (w, h) = (img.width, img.height);
        if check_image(img).is_err() || width_out == 0 {
            return Image2D { width: width_out, height: height_out, data: out };
        }
        let Some(map) = Mapper::new(wcs_in, wcs_out, w, h) else {
            return Image2D { width: width_out, height: height_out, data: out };
        };
        out.par_chunks_mut(width_out).enumerate().for_each(|(row, dst)| {
            let mut pix: Vec<Option<(f64, f64)>> = vec![None; width_out];
            map.row(row as f64, &mut pix);
            for (d, p) in dst.iter_mut().zip(&pix) {
                let Some((xi, yi)) = *p else { continue };
                let (Some(cx), Some(cy)) = (fix_coord(xi, w), fix_coord(yi, h)) else { continue };
                *d = bilinear(&img.data, w, h, cx, cy) as f32;
            }
        });
        Image2D { width: width_out, height: height_out, data: out }
    }

    fn same_bits(a: &Image2D, b: &Image2D) -> bool {
        a.width == b.width
            && a.height == b.height
            && a.data.len() == b.data.len()
            && a.data.iter().zip(&b.data).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    #[test]
    fn parallel_fill_matches_old() {
        use zahyou_wcs::fit_sip_inverse;
        // SIP(逆変換あり・なし)・TAN・回転・反転の WCS、NaN と ±∞ を含む画像で、全画素が旧版と to_bits まで同じ。
        // 写せない画素(外側)の NaN の位置とビットも同じ
        let (w, h) = (420usize, 260usize);
        let mut data: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 1000) as f32 + (i % w) as f32 * 0.5).collect();
        data[5 * w + 7] = f32::NAN;
        data[100 * w + 200] = f32::INFINITY;
        data[150 * w + 3] = f32::NEG_INFINITY;
        let img = Image2D::new(w, h, data).unwrap();
        let s = 2.0 / 3600.0;
        let m = 4;
        let mut a = vec![0.0; m * m];
        let mut b = vec![0.0; m * m];
        a[2 * m] = 3e-5;
        a[m + 1] = -2e-5;
        b[2] = 2.5e-5;
        a[3 * m] = 4e-8;
        b[3] = -5e-8;
        let sip = Sip::new(3, a, b).unwrap();
        let mut checked = 0;
        for (t, par, dec) in [(0.6f64, 1.0f64, 60.0f64), (0.785, -1.0, -30.0), (0.0, 1.0, 89.99), (2.5, -1.0, 0.0)] {
            let cd = [[-s * t.cos(), par * s * t.sin()], [s * t.sin(), par * s * t.cos()]];
            let base = Wcs::new(radec_to_xyz(10.0, dec), [200.0, 120.0], cd, Some(sip.clone()));
            let inv = fit_sip_inverse(&base, w as f64, h as f64, None).unwrap();
            for wcs in [base.clone(), inv, base.tan_only()] {
                let (out, wo, ho) = north_up_frame(&wcs, w, h).unwrap();
                let new = reproject_interp(&img, &wcs, &out, wo, ho);
                let old = reproject_interp_old(&img, &wcs, &out, wo, ho);
                assert!(same_bits(&new, &old), "t={t} par={par} dec={dec}");
                assert!(new.data.iter().any(|v| v.is_nan()) && new.data.iter().any(|v| !v.is_nan()));
                checked += 1;
            }
        }
        assert_eq!(checked, 12);
        // 早く抜ける経路: 大きさの合わない画像・特異な CD・幅 0・高さ 0
        let wcs = Wcs::tan(radec_to_xyz(10.0, 20.0), [200.0, 120.0], [[-s, 0.0], [0.0, s]]);
        let (out, wo, ho) = north_up_frame(&wcs, w, h).unwrap();
        let broken = Image2D { width: w, height: h, data: vec![1.0; 10] };
        assert!(same_bits(
            &reproject_interp(&broken, &wcs, &out, wo, ho),
            &reproject_interp_old(&broken, &wcs, &out, wo, ho)
        ));
        let singular = Wcs::tan(radec_to_xyz(10.0, 20.0), [200.0, 120.0], [[0.0, 0.0], [0.0, 0.0]]);
        assert!(same_bits(
            &reproject_interp(&img, &singular, &out, wo, ho),
            &reproject_interp_old(&img, &singular, &out, wo, ho)
        ));
        for (a, b) in [(0usize, 50usize), (50, 0), (0, 0), (1, 1)] {
            assert!(same_bits(
                &reproject_interp(&img, &wcs, &out, a, b),
                &reproject_interp_old(&img, &wcs, &out, a, b)
            ));
        }
    }

    #[test]
    fn nan_and_inf_inputs() {
        let img = Image2D::new(2, 1, vec![f32::NAN, f32::INFINITY]).unwrap();
        assert_eq!(bilinear(&img.data, 2, 1, 0.0, 0.0), 0.0);
        assert_eq!(bilinear(&img.data, 2, 1, 1.0, 0.0), f32::MAX as f64);
    }
}

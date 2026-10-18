//! TAN(+SIP)の WCS。Python 版 `dev/solver_wcs.py` の `SvWcs`(設計書 design-native-solver.md の 3.6・3.8・3.9 節)。
//!
//! - 画素は 0 始まり(x = 列、y = 行)。FITS のヘッダーに書くときだけ CRPIX に 1 を足す(header.rs)。
//! - CD は度/px、中間世界座標 (ξ = 東, η = 北) も度(FITS と同じ)。投影の関数(geom.rs)はラジアンのまま。

use std::fmt;

use crate::geom::{
    Vec3, normalize, radec_to_xyz, tan_basis, tan_deproject_with, tan_project_with, xyz_to_radec,
};
use crate::linalg::{Mat2, det2, inv2};
use crate::sip::{Sip, sip_solve};

/// TAN(+SIP)の WCS。
///
/// - `crval`: 接点(単位ベクトル。[`Wcs::new`] が長さ 1 にそろえる)
/// - `crpix`: 接点が写る画素(**0 始まり**)
/// - `cd`: 画素 → 中間世界座標 (ξ = 東, η = 北)。度/px。`cd[i][j]` が FITS の `CD{i+1}_{j+1}`
/// - `sip`: SIP の係数。無ければ None(SIP があって逆変換が無いときは、天 → 画素をニュートン法で解く)
#[derive(Clone, Debug, PartialEq)]
pub struct Wcs {
    pub crval: Vec3,
    pub crpix: [f64; 2],
    pub cd: Mat2,
    pub sip: Option<Sip>,
}

/// 同じ WCS で多数の点を投影するための下ごしらえ(接点の東・北と CD⁻¹)。
#[derive(Clone, Copy)]
struct Frame {
    r: Vec3,
    east: Vec3,
    north: Vec3,
    cdi: Option<Mat2>,
}

impl Wcs {
    /// 作る。接点は長さ 1 にそろえる。SIP の次数が 2 未満なら SIP なしにする(Python 版と同じ)。
    pub fn new(crval: Vec3, crpix: [f64; 2], cd: Mat2, sip: Option<Sip>) -> Wcs {
        Wcs { crval: normalize(crval), crpix, cd, sip: sip.filter(|s| s.order >= 2) }
    }

    /// SIP の無い TAN の WCS を作る。
    pub fn tan(crval: Vec3, crpix: [f64; 2], cd: Mat2) -> Wcs {
        Wcs::new(crval, crpix, cd, None)
    }

    /// SIP があるか。
    pub fn has_sip(&self) -> bool {
        self.sip.is_some()
    }

    /// SIP の順変換の次数(SIP が無ければ 0)。
    pub fn a_order(&self) -> usize {
        self.sip.as_ref().map_or(0, |s| s.order)
    }

    /// SIP の逆変換の次数(無ければ 0)。
    pub fn ap_order(&self) -> usize {
        self.sip.as_ref().filter(|s| s.has_inverse()).map_or(0, |s| s.ap_order)
    }

    /// SIP の逆変換を外した複製(天 → 画素はニュートン法になる)。
    pub fn without_sip_inverse(&self) -> Wcs {
        Wcs { sip: self.sip.as_ref().map(|s| s.without_inverse()), ..self.clone() }
    }

    /// SIP を外した複製(TAN だけ)。
    pub fn tan_only(&self) -> Wcs {
        Wcs { sip: None, ..self.clone() }
    }

    /// 接点の (赤経, 赤緯)(度)。極ちょうどでは赤経 0(`tan_basis` の東の取り方と合わせる)。
    pub fn crval_radec(&self) -> (f64, f64) {
        let r = self.crval;
        if r[0].hypot(r[1]) < 1e-15 {
            return (0.0, if r[2] > 0.0 { 90.0 } else { -90.0 });
        }
        xyz_to_radec(r)
    }

    /// 画素スケール(″/px)= √|det CD| × 3600。
    pub fn pixel_scale_arcsec(&self) -> f64 {
        det2(&self.cd).abs().sqrt() * 3600.0
    }

    /// det CD の符号(+1 / −1)。配列座標での鏡像。zahyou の表示の「正 (鏡像なし)」は +1(画面の向きは flip_y と組み合わせる)。
    pub fn parity(&self) -> i32 {
        if det2(&self.cd) >= 0.0 { 1 } else { -1 }
    }

    fn frame(&self) -> Frame {
        let (east, north) = tan_basis(self.crval);
        Frame { r: self.crval, east, north, cdi: inv2(&self.cd) }
    }

    /// 画素(0 始まり)→ 中間世界座標 (ξ, η)(度)。SIP があれば掛ける。
    pub fn pix_to_iwc(&self, x: f64, y: f64) -> (f64, f64) {
        let mut u = x - self.crpix[0];
        let mut v = y - self.crpix[1];
        if let Some(s) = &self.sip {
            let (fa, fb) = s.forward(u, v);
            u += fa;
            v += fb;
        }
        let cd = &self.cd;
        (cd[0][0] * u + cd[0][1] * v, cd[1][0] * u + cd[1][1] * v)
    }

    fn iwc_to_pix_in(&self, cdi: Option<Mat2>, xi: f64, eta: f64, exact: bool) -> Option<(f64, f64)> {
        let ci = cdi?;
        let uu = ci[0][0] * xi + ci[0][1] * eta;
        let vv = ci[1][0] * xi + ci[1][1] * eta;
        if !(uu.is_finite() && vv.is_finite()) {
            return None;
        }
        let (u, v) = match &self.sip {
            None => (uu, vv),
            Some(s) => {
                let (u0, v0) = match s.inverse(uu, vv) {
                    Some((pa, pb)) => (uu + pa, vv + pb),
                    None => (uu, vv),
                };
                if exact || !s.has_inverse() { sip_solve(s, uu, vv, u0, v0)? } else { (u0, v0) }
            }
        };
        let (x, y) = (u + self.crpix[0], v + self.crpix[1]);
        if x.is_finite() && y.is_finite() { Some((x, y)) } else { None }
    }

    /// 中間世界座標 (ξ, η)(度)→ 画素(0 始まり)。
    ///
    /// SIP の逆変換 (AP, BP) があればそれを使う(近似)。逆変換が無ければ順変換をニュートン法で解く。
    /// CD が特異か、ニュートン法が収束しなければ None。
    pub fn iwc_to_pix(&self, xi: f64, eta: f64) -> Option<(f64, f64)> {
        self.iwc_to_pix_in(inv2(&self.cd), xi, eta, false)
    }

    /// `iwc_to_pix` の、逆変換があっても順変換をニュートン法で解く版(Python の `exact=True`)。
    pub fn iwc_to_pix_exact(&self, xi: f64, eta: f64) -> Option<(f64, f64)> {
        self.iwc_to_pix_in(inv2(&self.cd), xi, eta, true)
    }

    fn pix_to_xyz_in(&self, fr: &Frame, x: f64, y: f64) -> Vec3 {
        let (xi, eta) = self.pix_to_iwc(x, y);
        tan_deproject_with(xi.to_radians(), eta.to_radians(), fr.r, fr.east, fr.north)
    }

    fn xyz_to_pix_in(&self, fr: &Frame, v: Vec3, exact: bool) -> Option<(f64, f64)> {
        let (xi, eta) = tan_project_with(v, fr.r, fr.east, fr.north)?;
        self.iwc_to_pix_in(fr.cdi, xi.to_degrees(), eta.to_degrees(), exact)
    }

    /// 画素(0 始まり)→ 単位ベクトル。
    pub fn pix_to_xyz(&self, x: f64, y: f64) -> Vec3 {
        self.pix_to_xyz_in(&self.frame(), x, y)
    }

    /// 単位ベクトル → 画素(0 始まり)。
    ///
    /// 接点の反対側の半球の点、SIP をニュートン法で解けなかった点、CD が特異なときは None。
    /// SIP の逆変換があればそれを使う(近似。画像の外では外れていく)。正確に解くなら [`Wcs::xyz_to_pix_exact`]。
    pub fn xyz_to_pix(&self, v: Vec3) -> Option<(f64, f64)> {
        self.xyz_to_pix_in(&self.frame(), v, false)
    }

    /// `xyz_to_pix` の、SIP をいつもニュートン法で解く版(Python の `exact=True`)。
    pub fn xyz_to_pix_exact(&self, v: Vec3) -> Option<(f64, f64)> {
        self.xyz_to_pix_in(&self.frame(), v, true)
    }

    /// 画素(0 始まり)→ (赤経 [0, 360), 赤緯)(度)。
    pub fn pix_to_radec(&self, x: f64, y: f64) -> (f64, f64) {
        xyz_to_radec(self.pix_to_xyz(x, y))
    }

    /// (赤経, 赤緯)(度)→ 画素(0 始まり)。投影できなければ None。
    pub fn radec_to_pix(&self, ra_deg: f64, dec_deg: f64) -> Option<(f64, f64)> {
        self.xyz_to_pix(radec_to_xyz(ra_deg, dec_deg))
    }

    /// `radec_to_pix` の、SIP をいつもニュートン法で解く版。
    pub fn radec_to_pix_exact(&self, ra_deg: f64, dec_deg: f64) -> Option<(f64, f64)> {
        self.xyz_to_pix_exact(radec_to_xyz(ra_deg, dec_deg))
    }

    /// 多数の画素 → 単位ベクトル(接点の基底を 1 回だけ計算する)。
    pub fn pix_to_xyz_batch(&self, pts: &[[f64; 2]]) -> Vec<Vec3> {
        let fr = self.frame();
        pts.iter().map(|p| self.pix_to_xyz_in(&fr, p[0], p[1])).collect()
    }

    /// 多数の単位ベクトル → 画素(接点の基底と CD⁻¹ を 1 回だけ計算する)。`exact` は `xyz_to_pix_exact` と同じ。
    pub fn xyz_to_pix_batch(&self, stars: &[Vec3], exact: bool) -> Vec<Option<(f64, f64)>> {
        let fr = self.frame();
        stars.iter().map(|s| self.xyz_to_pix_in(&fr, *s, exact)).collect()
    }

    /// 画素の残差 = 画像の点 − 星をこの WCS で画像へ投影した位置(Python の `sv_wcs_residuals`)。
    /// 投影できない星の行は NaN。数の合わない分は短い方に合わせる。
    pub fn residuals(&self, field: &[[f64; 2]], stars: &[Vec3], exact: bool) -> Vec<[f64; 2]> {
        let fr = self.frame();
        field
            .iter()
            .zip(stars)
            .map(|(f, s)| match self.xyz_to_pix_in(&fr, *s, exact) {
                Some((x, y)) => [f[0] - x, f[1] - y],
                None => [f64::NAN, f64::NAN],
            })
            .collect()
    }
}

impl fmt::Display for Wcs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ra, dec) = self.crval_radec();
        write!(
            f,
            "Wcs(crval=({ra:.6}, {dec:+.6}), crpix=({:.2}, {:.2}), {:.4} arcsec/px, parity {:+}",
            self.crpix[0],
            self.crpix[1],
            self.pixel_scale_arcsec(),
            self.parity()
        )?;
        if self.has_sip() {
            write!(f, ", SIP {}/{}", self.a_order(), self.ap_order())?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{angle_between, radec_to_xyz, ARCSEC_PER_RAD};

    fn sample_sip(order: usize, half: f64) -> Sip {
        let m = order + 1;
        let mut a = vec![0.0; m * m];
        let mut b = vec![0.0; m * m];
        let mut k = 0.37;
        for p in 0..m {
            for q in 0..m {
                let d = p + q;
                if (2..=order).contains(&d) {
                    k = (k * 7.3 + 0.11) % 1.0;
                    a[p * m + q] = (k - 0.5) * 3.0 / half.powi(d as i32);
                    b[p * m + q] = (0.5 - k * k) * 3.0 / half.powi(d as i32);
                }
            }
        }
        Sip::new(order, a, b).unwrap()
    }

    #[test]
    fn roundtrip_poles_and_ra_zero() {
        // 極ちょうど・極のそば・赤経 0° のまたぎ・南天で、画素 → 天 → 画素
        let centers = [(0.0, 90.0), (0.0, -90.0), (137.0, 89.9999), (300.0, -89.99), (359.9995, 0.0), (0.0004, -45.0)];
        for (i, &(ra, dec)) in centers.iter().enumerate() {
            let t = (i as f64 * 47.0).to_radians();
            let s = 2.0 / 3600.0;
            let par = if i % 2 == 0 { 1.0 } else { -1.0 };
            let cd = [[s * t.cos(), -s * par * t.sin()], [s * t.sin(), s * par * t.cos()]];
            for sip in [None, Some(sample_sip(3, 1500.0))] {
                let w = Wcs::new(radec_to_xyz(ra, dec), [1000.0, 700.0], cd, sip);
                for &(x, y) in &[(0.0, 0.0), (1999.0, 1399.0), (1000.0, 700.0), (-200.0, 1500.0)] {
                    let v = w.pix_to_xyz(x, y);
                    let (px, py) = w.xyz_to_pix(v).unwrap();
                    assert!((px - x).abs() < 1e-8 && (py - y).abs() < 1e-8, "{ra},{dec} ({x},{y}) -> ({px},{py})");
                    let (ra2, de2) = w.pix_to_radec(x, y);
                    let back = radec_to_xyz(ra2, de2);
                    assert!(angle_between(back, v) * ARCSEC_PER_RAD < 1e-9);
                }
                assert_eq!(w.parity(), if par > 0.0 { 1 } else { -1 });
                assert!((w.pixel_scale_arcsec() - 2.0).abs() < 1e-12);
                // 接点の反対側は None
                let anti = [-w.crval[0], -w.crval[1], -w.crval[2]];
                assert!(w.xyz_to_pix(anti).is_none());
            }
        }
        let p = Wcs::tan([0.0, 0.0, 2.0], [0.0, 0.0], [[1e-4, 0.0], [0.0, 1e-4]]);
        assert_eq!(p.crval_radec(), (0.0, 90.0));
        assert_eq!(p.crval, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn singular_and_broken_inputs_do_not_panic() {
        let w = Wcs::tan(radec_to_xyz(10.0, 20.0), [5.0, 5.0], [[0.0, 0.0], [0.0, 0.0]]);
        assert!(w.radec_to_pix(10.0, 20.0).is_none());
        let w = Wcs::tan(radec_to_xyz(10.0, 20.0), [5.0, 5.0], [[1e-4, 0.0], [0.0, 1e-4]]);
        assert!(w.xyz_to_pix([f64::NAN, 0.0, 1.0]).is_none());
        assert!(w.xyz_to_pix([0.0, 0.0, 0.0]).is_none());
        let r = w.residuals(&[[1.0, 2.0], [0.0, 0.0]], &[w.pix_to_xyz(1.0, 2.0), [-w.crval[0], -w.crval[1], -w.crval[2]]], false);
        assert!(r[0][0].abs() < 1e-9 && r[0][1].abs() < 1e-9 && r[1][0].is_nan());
        let s = format!("{w}");
        assert!(s.contains("0.3600 arcsec/px"), "{s}");
    }
}

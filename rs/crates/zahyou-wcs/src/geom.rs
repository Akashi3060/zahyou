//! 天球の単位ベクトルと TAN(心射)投影。Python 版 `dev/solver_wcs.py` の冒頭の関数と同じ式。
//!
//! - 天球の点は単位ベクトル (x, y, z) = (cosδ cosα, cosδ sinα, sinδ)(赤道座標、ICRS)。
//! - 接平面の座標は (ξ, η) = (東, 北)、単位はラジアン。FITS の中間世界座標と同じ向き。

/// 3 次元ベクトル。
pub type Vec3 = [f64; 3];

/// 1 ラジアンあたりの秒角。
pub const ARCSEC_PER_RAD: f64 = 180.0 / std::f64::consts::PI * 3600.0;

#[inline]
pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
pub fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// 長さ 1 にする。
#[inline]
pub fn normalize(a: Vec3) -> Vec3 {
    let n = norm(a);
    [a[0] / n, a[1] / n, a[2] / n]
}

#[inline]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// 赤経・赤緯(度)→ 単位ベクトル。
pub fn radec_to_xyz(ra_deg: f64, dec_deg: f64) -> Vec3 {
    let (ra, dec) = (ra_deg.to_radians(), dec_deg.to_radians());
    let cd = dec.cos();
    [cd * ra.cos(), cd * ra.sin(), dec.sin()]
}

/// 単位ベクトル(長さは問わない)→ (赤経 [0, 360) 度, 赤緯 度)。
pub fn xyz_to_radec(v: Vec3) -> (f64, f64) {
    let ra = v[1].atan2(v[0]).to_degrees().rem_euclid(360.0);
    let dec = v[2].atan2(v[0].hypot(v[1])).to_degrees();
    // rem_euclid は 360 ちょうどを返すことがある(-0 付近)
    (if ra >= 360.0 { 0.0 } else { ra }, dec)
}

/// 2 つのベクトルのなす角(ラジアン)。小さな角でも桁落ちしない atan2 の形。
pub fn angle_between(a: Vec3, b: Vec3) -> f64 {
    norm(cross(a, b)).atan2(dot(a, b))
}

/// 角距離(ラジアン)→ 単位球の上の弦の長さ。
pub fn chord_from_angle(rad: f64) -> f64 {
    2.0 * (rad / 2.0).sin()
}

/// 弦の長さ → 角距離(ラジアン)。
pub fn angle_from_chord(chord: f64) -> f64 {
    2.0 * (chord / 2.0).clamp(0.0, 1.0).asin()
}

/// 接点 r(単位ベクトル)での (東, 北) の単位ベクトル。
///
/// 東 = normalize(−r_y, r_x, 0)、北 = r × 東。極(r_x = r_y = 0)では東を +y 軸にとる。
pub fn tan_basis(r: Vec3) -> (Vec3, Vec3) {
    let h = r[0].hypot(r[1]);
    let east = if h < 1e-15 { [0.0, 1.0, 0.0] } else { [-r[1] / h, r[0] / h, 0.0] };
    let north = cross(r, east);
    (east, north)
}

/// 単位ベクトル s を接点 r で TAN 投影する。戻り値 (ξ 東, η 北)、単位はラジアン。
/// r の反対側の半球(s·r ≤ 0)は投影できないので None。
pub fn tan_project(s: Vec3, r: Vec3) -> Option<(f64, f64)> {
    let (east, north) = tan_basis(r);
    tan_project_with(s, r, east, north)
}

/// 基底を先に計算しておく版(同じ接点で多数の点を投影するとき)。
#[inline]
pub fn tan_project_with(s: Vec3, r: Vec3, east: Vec3, north: Vec3) -> Option<(f64, f64)> {
    let w = dot(s, r);
    if w <= 0.0 {
        return None;
    }
    Some((dot(s, east) / w, dot(s, north) / w))
}

/// 接平面の (ξ, η)(ラジアン)を接点 r から単位ベクトルへ戻す。
pub fn tan_deproject(xi: f64, eta: f64, r: Vec3) -> Vec3 {
    let (east, north) = tan_basis(r);
    tan_deproject_with(xi, eta, r, east, north)
}

/// 基底を先に計算しておく版(`tan_deproject` と同じ式)。
#[inline]
pub fn tan_deproject_with(xi: f64, eta: f64, r: Vec3, east: Vec3, north: Vec3) -> Vec3 {
    normalize(add(r, add(scale(east, xi), scale(north, eta))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_known_values() {
        // Python の solver_wcs と astropy で確かめた値(1″ = 4.848e-6 rad)
        let r = radec_to_xyz(61.89, 41.96);
        let s = radec_to_xyz(61.90, 41.96);
        let (xi, eta) = tan_project(s, r).unwrap();
        assert!((xi.to_degrees() * 3600.0 - 26.770_024_4).abs() < 1e-6);
        assert!((eta.to_degrees() * 3600.0 - 0.001_561_96).abs() < 1e-7);
        let back = tan_deproject(xi, eta, r);
        assert!(norm(sub(back, s)) < 1e-15);
        let (ra, dec) = xyz_to_radec(r);
        assert!((ra - 61.89).abs() < 1e-12 && (dec - 41.96).abs() < 1e-12);
        assert!(tan_project(scale(r, -1.0), r).is_none());
        // 極
        let p = [0.0, 0.0, 1.0];
        let q = radec_to_xyz(10.0, 89.0);
        assert!(tan_project(q, p).is_some());
        assert!((angle_between(p, q).to_degrees() - 1.0).abs() < 1e-12);
    }
}

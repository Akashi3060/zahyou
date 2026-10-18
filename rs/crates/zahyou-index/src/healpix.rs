//! HEALPix(astrometry.net の xy 方式の番号)。Python 版 `sv_healpix_xy_*` と同じ式・同じ演算の順。
//!
//! 番号 = 基本番号(0〜11)× nside² + x × nside + y。(x, y) は基本ピクセルの中の座標で、
//! 標準の NESTED の副番号は x を偶数ビット・y を奇数ビットに混ぜたもの。nside は 2 の累乗でなくてよい。
//! 式は Górski et al. (2005) の HEALPix の投影(赤道帯 |z| ≤ 2/3 は円筒、極帯は σ = √(3(1−|z|)))。
//! 基本ピクセルの中で x は北東、y は北西へ増える((0, 0) が南の角、(nside, nside) が北の角)。

use std::f64::consts::PI;

use zahyou_wcs::{angle_between, Vec3};

/// 基本ピクセルの中心の、HEALPix の平面(x_s, y_s)での位置。
fn face_center(face: u64) -> (f64, f64) {
    let row = face / 4;
    let col = (face % 4) as f64;
    let xc = if row == 1 { col * (PI / 2.0) } else { PI / 4.0 + col * (PI / 2.0) };
    let yc = if row == 0 {
        PI / 4.0
    } else if row == 1 {
        0.0
    } else {
        -PI / 4.0
    };
    (xc, yc)
}

/// numpy の np.sign(0 は 0、NaN は NaN)。
fn np_sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x * 0.0
    }
}

/// xy 方式の番号 hp のピクセルの中の点(dx, dy ∈ [0, 1]。0.5, 0.5 が中心)の単位ベクトル。
/// nside = 0 は扱えないので NaN を返す。
pub fn healpix_xy_point(hp: u64, nside: u32, dx: f64, dy: f64) -> Vec3 {
    if nside == 0 {
        return [f64::NAN; 3];
    }
    let ns = nside as u64;
    let n2 = ns * ns;
    let face = hp / n2;
    let rest = hp % n2;
    let x = (rest / ns) as f64 + dx;
    let y = (rest % ns) as f64 + dy;
    let nsf = ns as f64;
    let (xc, yc) = face_center(face);
    let xs = xc + (PI / 4.0) * (x - y) / nsf;
    let ys = yc + (PI / 4.0) * ((x + y) / nsf - 1.0);
    let ays = ys.abs();
    let (z, sth, phi) = if ays > PI / 4.0 {
        // 極帯: σ = 2 − 4|y_s|/π、z = ±(1 − σ²/3)、φ = x_c + (x_s − x_c)/σ
        let sig = 2.0 - 4.0 * ays / PI;
        let phi = if sig > 0.0 { xc + (xs - xc) / sig } else { xc };
        let z = np_sign(ys) * (1.0 - sig * sig / 3.0);
        let sth = sig * (0f64.max((2.0 - sig * sig / 3.0) / 3.0)).sqrt(); // √(1 − z²) を桁落ちなしに
        (z, sth, phi)
    } else {
        // 赤道帯: z = 8 y_s / (3π)、φ = x_s
        let z = ys * (8.0 / (3.0 * PI));
        (z, (0f64.max(1.0 - z * z)).sqrt(), xs)
    };
    [sth * phi.cos(), sth * phi.sin(), z]
}

/// xy 方式の番号 hp のピクセルの中心の単位ベクトル。
pub fn healpix_xy_center(hp: u32, nside: u32) -> Vec3 {
    healpix_xy_point(hp as u64, nside, 0.5, 0.5)
}

/// ベクトル(長さは問わない)が入る xy 方式の HEALPix の番号(u64)。nside = 0 や NaN では u64::MAX。
pub fn healpix_xy_from_xyz_u64(v: Vec3, nside: u32) -> u64 {
    if nside == 0 {
        return u64::MAX;
    }
    let ns = nside as i64;
    let nsf = ns as f64;
    let norm = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let z = v[2] / norm;
    let za = z.abs();
    if !za.is_finite() {
        return u64::MAX;
    }
    let sth = v[0].hypot(v[1]) / norm;
    // φ / (π/2) ∈ [0, 4)。np.mod(x, 4) は 4 を足して非負にする(−0 は +0)
    let mut tt = (v[1].atan2(v[0]) * (2.0 / PI)) % 4.0;
    if tt < 0.0 {
        tt += 4.0;
    }
    if tt >= 4.0 {
        tt = 0.0;
    }
    let (face, x, y) = if za <= 2.0 / 3.0 {
        // 赤道帯: 斜めの 2 本の格子線の番号から基本ピクセルと (x, y) を出す
        let t1 = nsf * (0.5 + tt);
        let t2 = nsf * (z * 0.75);
        let jp = (t1 - t2).floor() as i64; // 右上がりの線の番号
        let jm = (t1 + t2).floor() as i64; // 右下がりの線の番号
        let ifp = jp.div_euclid(ns);
        let ifm = jm.div_euclid(ns);
        let face = if ifp == ifm {
            ifp | 4
        } else if ifp < ifm {
            ifp
        } else {
            ifm + 8
        };
        (face, jm.rem_euclid(ns), ns - jp.rem_euclid(ns) - 1)
    } else {
        // 極帯
        let ntt = tt.floor().min(3.0);
        let tp = tt - ntt;
        let tmp = nsf * sth * (3.0 / (1.0 + za)).sqrt(); // = nside·√(3(1 − |z|))
        let jp2 = ((tp * tmp).floor() as i64).min(ns - 1);
        let jm2 = (((1.0 - tp) * tmp).floor() as i64).min(ns - 1);
        if z > 0.0 {
            (ntt as i64, ns - jm2 - 1, ns - jp2 - 1)
        } else {
            (ntt as i64 + 8, jp2, jm2)
        }
    };
    let hp = face as i128 * (ns as i128 * ns as i128) + x as i128 * ns as i128 + y as i128;
    u64::try_from(hp).unwrap_or(u64::MAX)
}

/// ベクトルが入る xy 方式の HEALPix の番号。nside が大きすぎて u32 に入らないとき・nside = 0・NaN は u32::MAX。
pub fn healpix_xy_from_xyz(v: Vec3, nside: u32) -> u32 {
    u32::try_from(healpix_xy_from_xyz_u64(v, nside)).unwrap_or(u32::MAX)
}

/// ピクセル hp の外接半径(中心から境界までの最大の角距離、ラジアン)。
///
/// 境界を辺ごとに samples 点で調べた最大に、取りこぼしの分の余裕(0.2 % + 1e-5 rad)を足す
/// (安全側。中心からいちばん遠い点は必ず境界にある)。Python 版の既定は samples = 64。
pub fn healpix_xy_radius_samples(hp: u32, nside: u32, samples: u32) -> f64 {
    let c = healpix_xy_center(hp, nside);
    let n = samples.max(1);
    let mut r = 0.0f64;
    let mut found_nan = false;
    for edge in 0..4 {
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let (dx, dy) = match edge {
                0 => (t, 0.0),
                1 => (1.0, t),
                2 => (t, 1.0),
                _ => (0.0, t),
            };
            let a = angle_between(healpix_xy_point(hp as u64, nside, dx, dy), c);
            if a.is_nan() {
                found_nan = true;
            } else if a > r {
                r = a;
            }
        }
    }
    if found_nan {
        return f64::NAN; // np.max は NaN を返す
    }
    r * 1.002 + 1e-5
}

/// ピクセル hp の外接半径(samples = 64)。
pub fn healpix_xy_radius(hp: u32, nside: u32) -> f64 {
    healpix_xy_radius_samples(hp, nside, 64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zahyou_wcs::xyz_to_radec;

    #[test]
    fn roundtrip_and_table() {
        // 中心 → 番号の往復と、ピクセルの中の点(nside が 2 の累乗でないものも)
        for nside in [1u32, 2, 3, 5, 7, 16] {
            for hp in 0..12 * nside * nside {
                assert_eq!(healpix_xy_from_xyz(healpix_xy_center(hp, nside), nside), hp);
                for k in 1..8 {
                    let dx = 0.01 + 0.98 * (k as f64 / 8.0);
                    let dy = 0.99 - 0.98 * ((k * 3 % 8) as f64 / 8.0);
                    let p = healpix_xy_point(hp as u64, nside, dx, dy);
                    assert_eq!(healpix_xy_from_xyz(p, nside), hp, "nside {nside} hp {hp} ({dx}, {dy})");
                }
            }
        }
        // index-format.md 12 節の表(nside 2 の中心)
        let table = [
            (0, 45.0, 19.47),
            (1, 22.5, 41.81),
            (3, 45.0, 66.44),
            (17, 337.5, 0.0),
            (25, 157.5, 0.0),
            (36, 135.0, -66.44),
            (40, 225.0, -66.44),
            (47, 315.0, -19.47),
        ];
        for (hp, ra, dec) in table {
            let (r, d) = xyz_to_radec(healpix_xy_center(hp, 2));
            assert!(((r - ra + 180.0).rem_euclid(360.0) - 180.0).abs() < 0.01 && (d - dec).abs() < 0.01);
        }
        // 極・境目・nside 0 でも panic しない
        for v in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [1.0, -1e-17, 0.0], [0.6, 0.0, 0.8]] {
            for nside in [1u32, 2, 3, 1024] {
                let h = healpix_xy_from_xyz(v, nside);
                assert!(h < 12 * nside * nside);
            }
        }
        assert_eq!(healpix_xy_from_xyz([f64::NAN, 0.0, 0.0], 2), u32::MAX);
        assert_eq!(healpix_xy_from_xyz([0.0, 0.0, 0.0], 2), u32::MAX);
        assert_eq!(healpix_xy_from_xyz([1.0, 0.0, 0.0], 0), u32::MAX);
        assert!(healpix_xy_center(5, 0)[0].is_nan());
    }

    #[test]
    fn radius_is_safe_and_tight() {
        // 外接半径は境界を細かく調べた最大以上、かつ 1 % 以内
        for nside in [2u32, 3] {
            for hp in 0..12 * nside * nside {
                let dense = (healpix_xy_radius_samples(hp, nside, 2000) - 1e-5) / 1.002;
                let ratio = healpix_xy_radius(hp, nside) / dense;
                assert!((1.0..1.01).contains(&ratio), "nside {nside} hp {hp} ratio {ratio}");
            }
        }
    }
}

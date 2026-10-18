//! 画像中心から目標までのずれと位置角。Python 版 `dev/zahyou_plot.py` の `report_offsets`
//! (astropy の `separation`・`spherical_offsets_to`・`position_angle`・`directional_offset_by`)と同じ式。
//!
//! 角はすべてラジアンで返す(分角にするなら `.to_degrees() * 60.0`)。

use std::f64::consts::{FRAC_PI_2, TAU};

/// 中心 (α0, δ0) から目標 (α, δ)(度)までのずれ。戻り値 (角距離, 赤経方向 東が正, 赤緯方向 北が正)、ラジアン。
///
/// 目標を、中心を原点とする座標系(astropy の SkyOffsetFrame)へ回して読む。Δα = α − α0 として
///   x = cosδ·cosΔα·cosδ0 + sinδ·sinδ0
///   y = cosδ·sinΔα
///   z = sinδ·cosδ0 − cosδ·cosΔα·sinδ0
/// 赤経方向 = atan2(y, x)、赤緯方向 = asin(z)(桁落ちしないよう atan2(z, √(x²+y²)) で計算する。値は同じ)、
/// 角距離 = atan2(√(y²+z²), x)。
pub fn spherical_offsets(center_radec: (f64, f64), target_radec: (f64, f64)) -> (f64, f64, f64) {
    let (x, y, z) = offset_frame(center_radec, target_radec);
    let sep = y.hypot(z).atan2(x);
    let dra = y.atan2(x);
    let ddec = z.atan2(x.hypot(y));
    (sep, dra, ddec)
}

/// 中心から見た目標の位置角(北から東回り、ラジアン、[0, 2π))。astropy の `position_angle` と同じ。
pub fn position_angle(center_radec: (f64, f64), target_radec: (f64, f64)) -> f64 {
    let (_, y, z) = offset_frame(center_radec, target_radec);
    let pa = y.atan2(z).rem_euclid(TAU);
    if pa >= TAU { 0.0 } else { pa }
}

/// 中心から位置角 `pa`(北から東回り)の向きへ角距離 `dist` だけ進んだ点 (赤経 [0, 360), 赤緯)(度)。
/// 角はラジアン。astropy の `directional_offset_by` と同じ式(極では経度を位置角から決める)。
pub fn offset_by(center_radec: (f64, f64), pa: f64, dist: f64) -> (f64, f64) {
    let (lon, lat) = (center_radec.0.to_radians(), center_radec.1.to_radians());
    let (sin_a, cos_a) = dist.sin_cos();
    let cos_c = lat.sin();
    let sin_c = lat.cos();
    let (sin_b, cos_b_ang) = pa.sin_cos();
    let cos_b = cos_c * cos_a + sin_c * sin_a * cos_b_ang;
    let xsin_a = sin_a * sin_b * sin_c;
    let xcos_a = cos_a - cos_b * cos_c;
    let mut a = xsin_a.atan2(xcos_a);
    if sin_c < 1e-12 {
        // 極では経度が定まらないので、位置角から決める(astropy と同じ)
        a = FRAC_PI_2 + cos_c * (FRAC_PI_2 - pa);
    }
    let ra = (lon + a).to_degrees().rem_euclid(360.0);
    let dec = cos_b.clamp(-1.0, 1.0).asin().to_degrees();
    (if ra >= 360.0 { 0.0 } else { ra }, dec)
}

/// 目標を中心の座標系へ回したときの (x, y, z)(中心が +x、東が +y、北が +z)。
fn offset_frame(center: (f64, f64), target: (f64, f64)) -> (f64, f64, f64) {
    let (a0, d0) = (center.0.to_radians(), center.1.to_radians());
    let (a, d) = (target.0.to_radians(), target.1.to_radians());
    let (sd0, cd0) = d0.sin_cos();
    let (sd, cd) = d.sin_cos();
    let (sda, cda) = (a - a0).sin_cos();
    let x = cd * cda * cd0 + sd * sd0;
    let y = cd * sda;
    let z = sd * cd0 - cd * cda * sd0;
    (x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_value_from_python() {
        // テスト画像の中心から UCAC4 660-021020 まで。Python 版(astropy 7.1)の全桁:
        //   separation 1.5833445902411678′、spherical_offsets_to 0.9177253671708191′ / 1.2902558976588177′
        // 依頼書の「1.5833′ / 0.9177′ / 1.2902′」は小数 4 桁で切り捨てた値(四捨五入なら赤緯は 1.2903′)
        let (sep, dra, ddec) = spherical_offsets((61.891414, 41.964760), (61.91199157844, 41.98626242835));
        let am = |r: f64| r.to_degrees() * 60.0;
        assert!((am(sep) - 1.5833445902411678).abs() < 1e-10, "{}", am(sep));
        assert!((am(dra) - 0.9177253671708191).abs() < 1e-10, "{}", am(dra));
        assert!((am(ddec) - 1.2902558976588177).abs() < 1e-10, "{}", am(ddec));
        let trunc4 = |v: f64| format!("{:.4}", (v * 1e4).trunc() / 1e4);
        assert_eq!([trunc4(am(sep)), trunc4(am(dra)), trunc4(am(ddec))].join(" "), "1.5833 0.9177 1.2902");
        assert!(dra > 0.0 && ddec > 0.0); // 東・北
        // 位置角は東寄りの北(0〜90°)で、そこへ角距離だけ進むと目標に戻る
        let pa = position_angle((61.891414, 41.964760), (61.91199157844, 41.98626242835));
        assert!(pa > 0.0 && pa < FRAC_PI_2);
        let (ra, dec) = offset_by((61.891414, 41.964760), pa, sep);
        assert!((ra - 61.91199157844).abs() < 1e-9 && (dec - 41.98626242835).abs() < 1e-9);
    }

    #[test]
    fn signs_wrap_and_poles() {
        // 赤経 0° のまたぎ: 359.99° から 0.01° は東へ 0.02°
        let (_, dra, ddec) = spherical_offsets((359.99, 0.0), (0.01, 0.0));
        assert!((dra.to_degrees() - 0.02).abs() < 1e-9 && ddec.abs() < 1e-12);
        // 西・南は負
        let (_, dra, ddec) = spherical_offsets((10.0, 20.0), (9.9, 19.9));
        assert!(dra < 0.0 && ddec < 0.0);
        // 真北の位置角は 0、真東は 90°
        assert!(position_angle((10.0, 20.0), (10.0, 21.0)).abs() < 1e-12);
        assert!((position_angle((10.0, 0.0), (11.0, 0.0)).to_degrees() - 90.0).abs() < 1e-9);
        // 真南は 180°、真西は 270°([0, 360) に入る)
        assert!((position_angle((10.0, 20.0), (10.0, 19.0)).to_degrees() - 180.0).abs() < 1e-9);
        assert!((position_angle((10.0, 0.0), (9.0, 0.0)).to_degrees() - 270.0).abs() < 1e-9);
        // 極から: 角距離は余緯度
        let (sep, _, _) = spherical_offsets((0.0, 90.0), (123.0, 80.0));
        assert!((sep.to_degrees() - 10.0).abs() < 1e-12);
        let (ra, dec) = offset_by((0.0, 90.0), 0.3, 0.1);
        assert!(ra.is_finite() && (dec - (90.0 - 0.1f64.to_degrees())).abs() < 1e-9);
        // 同じ点
        let (sep, dra, ddec) = spherical_offsets((10.0, 20.0), (10.0, 20.0));
        assert_eq!((sep, dra, ddec), (0.0, 0.0, 0.0));
    }
}

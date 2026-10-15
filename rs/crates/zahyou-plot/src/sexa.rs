//! 角度を 60 進の文字列にする。astropy の `Angle.to_string`(`_decimal_to_sexagesimal_string`)と
//! `SkyCoord.to_string("hmsdms", precision=…)` を移したもの(astropy 7.1)。
//!
//! 度 → 時の換算は astropy の単位換算と同じ係数を掛ける(1/15 ではなく `u.deg.to(u.hourangle)` =
//! 0.06666666666666668)。四捨五入の境目で 1 桁ずれないよう、式も途中の順序も astropy のまま。

/// `u.deg.to(u.hourangle)`(astropy の単位換算の係数そのもの)
pub const DEG_TO_HOURANGLE: f64 = 0.06666666666666668;
/// `u.hourangle.to(u.deg)`
pub const HOURANGLE_TO_DEG: f64 = 14.999999999999998;
/// `u.arcmin.to(u.deg)`
pub const ARCMIN_TO_DEG: f64 = 0.016666666666666666;
/// `u.arcsec.to(u.deg)`
pub const ARCSEC_TO_DEG: f64 = 0.0002777777777777778;

/// `_decimal_to_sexagesimal(a)`: (度 or 時, 分, 秒)。負の数は 3 つとも負(度・時の 0 は −0.0)。
pub fn decimal_to_sexagesimal(a: f64) -> (f64, f64, f64) {
    let sign = 1.0f64.copysign(a);
    let ab = a.abs();
    // np.modf: (小数部, 整数部)
    let d = ab.trunc();
    let df = ab - d;
    let m60 = df * 60.0;
    let m = m60.trunc();
    let mf = m60 - m;
    let s = mf * 60.0;
    ((sign * d).floor(), sign * m.floor(), sign * s)
}

/// `_decimal_to_sexagesimal_string(angle, precision, pad, sep, fields)`。
///
/// - `precision`: 秒の小数の桁(None なら 8 桁で書いて末尾の 0 を落とす)
/// - `pad`: 先頭の数を 2 桁(負なら符号込みで 3 桁)にそろえる
/// - `sep`: 3 つの区切り(例 ["h", "m", "s"])
/// - `fields`: 何項目まで書くか(1〜3。範囲の外は 3 とみなす)
///
/// 繰り上がりは astropy と同じく、秒が `60 − 10^(−precision)` 以上なら繰り上げる(precision = 0 なら 59 秒以上)。
pub fn sexagesimal_string(angle: f64, precision: Option<usize>, pad: bool, sep: [&str; 3], fields: usize) -> String {
    if angle.is_nan() {
        return "nan".to_string();
    }
    let fields = if (1..=3).contains(&fields) { fields } else { 3 };
    let (v0, v1, v2) = decimal_to_sexagesimal(angle);
    let sign = 1.0f64.copysign(v0);
    let mut v = [v0.abs(), v1.abs(), v2.abs()];
    let width = if pad { if sign < 0.0 { 3 } else { 2 } } else { 0 };
    let p = precision.map(|p| p.min(17));
    let rounding_thresh = 60.0 - 10f64.powi(-(p.unwrap_or(8) as i32));
    if fields == 3 && v[2] >= rounding_thresh {
        v[2] = 0.0;
        v[1] += 1.0;
    } else if fields < 3 && v[2] >= 30.0 {
        v[1] += 1.0;
    }
    if fields >= 2 && v[1] >= 60.0 {
        v[1] = 0.0;
        v[0] += 1.0;
    } else if fields < 2 && v[1] >= 30.0 {
        v[0] += 1.0;
    }
    let first = v[0].copysign(sign);
    let mut out = format!("{first:0width$.0}{}", sep[0]);
    if fields >= 2 {
        // int(values[1]) を 2 桁で
        out.push_str(&format!("{:02}{}", v[1] as i64, sep[1]));
    }
    if fields == 3 {
        let mut last = match p {
            None => {
                let s = format!("{:.8}", v[2].abs());
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            Some(p) => format!("{:.p$}", v[2].abs()),
        };
        if last.len() == 1 || last.as_bytes().get(1) == Some(&b'.') {
            last.insert(0, '0');
        }
        out.push_str(&last);
        out.push_str(sep[2]);
    }
    out
}

/// 経度を [0, 360) にそろえる(astropy の Longitude と同じ。360 ちょうどに丸まったら 0)。
pub fn wrap_longitude(ra_deg: f64) -> f64 {
    if !ra_deg.is_finite() || (0.0..360.0).contains(&ra_deg) {
        return ra_deg;
    }
    let mut a = ra_deg - (ra_deg / 360.0).floor() * 360.0;
    if a >= 360.0 {
        a -= 360.0;
    }
    if a < 0.0 {
        a += 360.0;
    }
    a
}

/// `SkyCoord(ra, dec).to_string("hmsdms", precision=precision)`。例 `04h07m38.88s +41d58m00.12s`。
pub fn hmsdms(ra_deg: f64, dec_deg: f64, precision: usize) -> String {
    let ra = wrap_longitude(ra_deg);
    let lon = sexagesimal_string(ra * DEG_TO_HOURANGLE, Some(precision), true, ["h", "m", "s"], 3);
    let mut lat = sexagesimal_string(dec_deg, Some(precision), true, ["d", "m", "s"], 3);
    if !lat.starts_with('-') {
        lat.insert(0, '+');
    }
    format!("{lon} {lat}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_strings() {
        // astropy 7.1 で確かめた値(gen_plot.py の plot_text.json で多数を照合する)
        assert_eq!(hmsdms(61.91199157844, 41.98626242835, 2), "04h07m38.88s +41d59m10.54s");
        assert_eq!(hmsdms(0.0, -0.5, 2), "00h00m00.00s -00d30m00.00s");
        assert_eq!(hmsdms(359.9999999999, 89.99999999, 2), "24h00m00.00s +90d00m00.00s");
        assert_eq!(sexagesimal_string(4.125, Some(0), false, ["h", "m", "s"], 2), "4h08m");
        assert_eq!(sexagesimal_string(-0.25, Some(0), false, ["°", "'", "\""], 2), "-0°15'");
        assert_eq!(sexagesimal_string(41.0, Some(0), false, ["°", "'", "\""], 1), "41°");
        assert_eq!(sexagesimal_string(f64::NAN, Some(0), false, ["°", "'", "\""], 1), "nan");
    }
}

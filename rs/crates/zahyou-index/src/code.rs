//! 索引に入っている quad のコードを星から計算する(index-format.md 8.1。Python 版 `sv_index_codes_from_xyz`)。
//!
//! 投影中心 = A と B の中点の方向、x = 北・y = 東の心射投影、A → (0, 0)・B → (1, 1) の相似変換。
//! この式を符号化すると、保存されている整数と全 quad で一致する(試験で確かめる)。

use zahyou_wcs::{add, cross, dot, normalize, Vec3};

/// 複素数の割り算(numpy と同じ Smith の方法。保存値と整数まで一致させるため演算の順も同じにする)。
fn cdiv(ar: f64, ai: f64, br: f64, bi: f64) -> (f64, f64) {
    let (bra, bia) = (br.abs(), bi.abs());
    if bra >= bia {
        if bra == 0.0 && bia == 0.0 {
            return (ar / bra, ai / bra);
        }
        let rat = bi / br;
        let scl = 1.0 / (br + bi * rat);
        ((ar + ai * rat) * scl, (ai - ar * rat) * scl)
    } else {
        let rat = br / bi;
        let scl = 1.0 / (bi + br * rat);
        ((ar * rat + ai) * scl, (ai * rat - ar) * scl)
    }
}

/// quad の星(単位ベクトル。並びは A, B, C(, D))から、索引のコードを計算する。
///
/// 戻り値は (cx, cy, dx, dy)。三角形(星 3 つ)は (cx, cy, 0, 0)。星の数が 3 か 4 でなければ None。
pub fn quad_code_from_xyz(stars: &[Vec3]) -> Option<[f64; 4]> {
    let n = stars.len();
    if n != 3 && n != 4 {
        return None;
    }
    let c = normalize(add(stars[0], stars[1]));
    let h = c[0].hypot(c[1]);
    let east = if h > 0.0 { [-c[1] / h, c[0] / h, 0.0] } else { [0.0, 1.0, 0.0] };
    let north = cross(c, east);
    let mut z = [(0.0, 0.0); 4];
    for (k, s) in stars.iter().enumerate() {
        let w = dot(*s, c);
        z[k] = (dot(*s, north) / w, dot(*s, east) / w);
    }
    let (br, bi) = (z[1].0 - z[0].0, z[1].1 - z[0].1);
    let mut out = [0.0; 4];
    for k in 2..n {
        let (qr, qi) = cdiv(z[k].0 - z[0].0, z[k].1 - z[0].1, br, bi);
        // × (1 + i)
        out[2 * (k - 2)] = qr - qi;
        out[2 * (k - 2) + 1] = qr + qi;
    }
    Some(out)
}

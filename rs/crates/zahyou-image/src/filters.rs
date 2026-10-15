//! scipy.ndimage と同じ値になる 2 次元フィルター(float32 の画像、行優先)。
//!
//! scipy は 1 次元フィルターを軸 0(列)→ 軸 1(行)の順にかけ、途中の結果を入力と同じ float32 に丸める。
//! 各行の計算は float64 で、端の延長(nearest・reflect)も同じ規則。ここでは計算の順序まで写して
//! ビット単位で同じ値を出す(image_kernels.json で確かめる)。

use rayon::prelude::*;

/// gaussian_filter(sigma=1, truncate=4) の重み(半径 4)。scipy の _gaussian_kernel1d が numpy で計算した値を
/// そのまま書く(exp の実装の違いで最下位ビットがずれないように)。添字は中心からの距離。
const GAUSS1: [f64; 5] = [
    f64::from_bits(0x3FD9884A307594FB), // 0x1.9884a307594fbp-2
    f64::from_bits(0x3FCEF8EB9AD499BA), // 0x1.ef8eb9ad499bap-3
    f64::from_bits(0x3FABA4B99D1799AB), // 0x1.ba4b99d1799abp-5
    f64::from_bits(0x3F722724CB7EB269), // 0x1.22724cb7eb269p-8
    f64::from_bits(0x3F218A9C4FD536C6), // 0x1.18a9c4fd536c6p-13
];

/// scipy の mode="reflect"(d c b a | a b c d | d c b a)。長さ n の外の添字を中へ折り返す。
#[inline]
fn reflect_index(i: isize, n: usize) -> usize {
    let n = n as isize;
    if n <= 1 {
        return 0;
    }
    let p = 2 * n;
    let m = i.rem_euclid(p);
    (if m < n { m } else { p - 1 - m }) as usize
}

#[inline]
fn nearest_index(i: isize, n: usize) -> usize {
    i.clamp(0, n as isize - 1) as usize
}

/// 対称な重み w[0..=r](w[0] が中心)の相関を 1 本の線にかける。scipy の NI_Correlate1D の対称の枝と同じ順:
/// o = x[0]*w0、続いて外側から o += (x[-j] + x[+j]) * w[j](j = r .. 1)。
#[inline]
fn sym_line(ext: &[f64], r: usize, w: &[f64], out: &mut [f32]) {
    for (l, o) in out.iter_mut().enumerate() {
        let c = l + r;
        let mut acc = ext[c] * w[0];
        for j in (1..=r).rev() {
            acc += (ext[c - j] + ext[c + j]) * w[j];
        }
        *o = acc as f32;
    }
}

/// 対称な重みの分離型フィルター(軸 0 → 軸 1)。`index` は端の延長の規則。
fn separable_sym(data: &[f32], ny: usize, nx: usize, w: &[f64], index: fn(isize, usize) -> usize) -> Vec<f32> {
    let r = w.len() - 1;
    let mut tmp = vec![0.0f32; ny * nx];
    // 軸 0: 出力の行 y は入力の行 y-r..y+r から決まる(行ごとに並列)
    tmp.par_chunks_mut(nx.max(1)).enumerate().for_each(|(y, orow)| {
        let rows: Vec<&[f32]> = (0..=2 * r)
            .map(|k| {
                let yy = index(y as isize + k as isize - r as isize, ny);
                &data[yy * nx..yy * nx + nx]
            })
            .collect();
        for x in 0..nx {
            let mut acc = rows[r][x] as f64 * w[0];
            for j in (1..=r).rev() {
                acc += (rows[r - j][x] as f64 + rows[r + j][x] as f64) * w[j];
            }
            orow[x] = acc as f32;
        }
    });
    // 軸 1: 行ごと
    let mut out = vec![0.0f32; ny * nx];
    out.par_chunks_mut(nx.max(1)).enumerate().for_each(|(y, orow)| {
        let row = &tmp[y * nx..y * nx + nx];
        let ext: Vec<f64> = (0..nx + 2 * r).map(|e| row[index(e as isize - r as isize, nx)] as f64).collect();
        sym_line(&ext, r, w, orow);
    });
    out
}

/// `ndimage.gaussian_filter(a, sigma=1.0)`(mode="reflect"、truncate=4)。
pub fn gaussian1(data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
    if ny == 0 || nx == 0 {
        return Vec::new();
    }
    separable_sym(data, ny, nx, &GAUSS1, reflect_index)
}

/// ベイヤー用の 3×3 二項フィルター: `correlate1d(a, [0.25, 0.5, 0.25], axis=0/1, mode="nearest")`。
pub fn binomial3(data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
    if ny == 0 || nx == 0 {
        return Vec::new();
    }
    separable_sym(data, ny, nx, &[0.5, 0.25], nearest_index)
}

/// `ndimage.uniform_filter(a, size=3, mode="nearest")`。scipy は線ごとに float64 の和を持ち回し
/// (`tmp += 新 - 旧`)、毎回 3 で割る。その丸めの積み重ねまで同じにする。
pub fn uniform3(data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
    if ny == 0 || nx == 0 {
        return Vec::new();
    }
    // 軸 0: 列ごとの持ち回し。縦の帯(幅 STRIP)ごとに並列に計算して、あとで行へ並べ直す
    const STRIP: usize = 256;
    let strips: Vec<(usize, Vec<f32>)> = (0..nx.div_ceil(STRIP))
        .into_par_iter()
        .map(|s| {
            let x0 = s * STRIP;
            let x1 = (x0 + STRIP).min(nx);
            let w = x1 - x0;
            let mut buf = vec![0.0f32; ny * w];
            let mut tmp = vec![0.0f64; w];
            let at = |y: isize, x: usize| data[nearest_index(y, ny) * nx + x0 + x] as f64;
            for x in 0..w {
                let mut t = 0.0f64;
                t += at(-1, x);
                t += at(0, x);
                t += at(1, x);
                tmp[x] = t;
                buf[x] = (t / 3.0) as f32;
            }
            for l in 1..ny {
                let (yn, yo) = (l as isize + 1, l as isize - 2);
                for x in 0..w {
                    tmp[x] += at(yn, x) - at(yo, x);
                    buf[l * w + x] = (tmp[x] / 3.0) as f32;
                }
            }
            (x0, buf)
        })
        .collect();
    let mut t0 = vec![0.0f32; ny * nx];
    t0.par_chunks_mut(nx).enumerate().for_each(|(y, orow)| {
        for (x0, buf) in &strips {
            let w = (STRIP).min(nx - x0);
            orow[*x0..x0 + w].copy_from_slice(&buf[y * w..y * w + w]);
        }
    });
    drop(strips);
    // 軸 1: 行ごと
    let mut out = vec![0.0f32; ny * nx];
    out.par_chunks_mut(nx).enumerate().for_each(|(y, orow)| {
        let row = &t0[y * nx..y * nx + nx];
        let at = |x: isize| row[nearest_index(x, nx)] as f64;
        let mut t = 0.0f64;
        t += at(-1);
        t += at(0);
        t += at(1);
        orow[0] = (t / 3.0) as f32;
        for (l, o) in orow.iter_mut().enumerate().skip(1) {
            t += at(l as isize + 1) - at(l as isize - 2);
            *o = (t / 3.0) as f32;
        }
    });
    out
}

/// 9 個の中央値(5 番目)。
#[inline]
fn median9(mut v: [f32; 9]) -> f32 {
    let (_, m, _) = v.select_nth_unstable_by(4, |a, b| a.total_cmp(b));
    *m
}

/// `ndimage.median_filter(a, size=3, mode="nearest")` の (x, y) の 1 画素ぶん。
#[inline]
pub fn median3_at(data: &[f32], ny: usize, nx: usize, x: usize, y: usize) -> f32 {
    let mut v = [0.0f32; 9];
    let mut k = 0;
    for dy in -1isize..=1 {
        let yy = nearest_index(y as isize + dy, ny);
        for dx in -1isize..=1 {
            let xx = nearest_index(x as isize + dx, nx);
            v[k] = data[yy * nx + xx];
            k += 1;
        }
    }
    median9(v)
}

/// `ndimage.median_filter(a, size=3, mode="nearest")`(画像全体)。
pub fn median3(data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; ny * nx];
    if nx == 0 {
        return out;
    }
    out.par_chunks_mut(nx).enumerate().for_each(|(y, orow)| {
        for (x, o) in orow.iter_mut().enumerate() {
            *o = median3_at(data, ny, nx, x, y);
        }
    });
    out
}

/// 1 軸ぶんの線形補間の表(入力の添字 2 つと重み 2 つ)。scipy の NI_ZoomShift(order=1、grid_mode=False)と同じ:
/// 座標 cc = k * zoom、zoom = (入力-1)/(出力-1)、start = floor(cc)、重み (1-t, t)、範囲外の添字は鏡映で折り返す。
fn zoom_axis(n_in: usize, n_out: usize) -> Vec<(usize, usize, f64, f64)> {
    let z = if n_out > 1 { (n_in as f64 - 1.0) / (n_out as f64 - 1.0) } else { 1.0 };
    let mirror = |i: isize| -> usize {
        let len = n_in as isize;
        if len <= 1 {
            return 0;
        }
        let s2 = 2 * len - 2;
        let mut i = i;
        if i < 0 {
            i = s2 * (-i / s2) + i;
            i = if i <= 1 - len { i + s2 } else { -i };
        } else if i >= len {
            i -= s2 * (i / s2);
            if i >= len {
                i = s2 - i;
            }
        }
        i as usize
    };
    (0..n_out)
        .map(|k| {
            let cc = k as f64 * z;
            let st = cc.floor();
            let t = cc - st;
            let s = st as isize;
            (mirror(s), mirror(s + 1), 1.0 - t, t)
        })
        .collect()
}

/// `ndimage.zoom(c, (oy/by, ox/bx), order=1)` の本体(出力の大きさは呼ぶ側で決める)。
pub fn zoom_linear(c: &[f32], by: usize, bx: usize, oy: usize, ox: usize) -> Vec<f32> {
    let ay = zoom_axis(by, oy);
    let ax = zoom_axis(bx, ox);
    let mut out = vec![0.0f32; oy * ox];
    if ox == 0 {
        return out;
    }
    out.par_chunks_mut(ox).enumerate().for_each(|(i, orow)| {
        let (y0, y1, wy0, wy1) = ay[i];
        for (j, o) in orow.iter_mut().enumerate() {
            let (x0, x1, wx0, wx1) = ax[j];
            let mut t = 0.0f64;
            t += c[y0 * bx + x0] as f64 * wy0 * wx0;
            t += c[y0 * bx + x1] as f64 * wy0 * wx1;
            t += c[y1 * bx + x0] as f64 * wy1 * wx0;
            t += c[y1 * bx + x1] as f64 * wy1 * wx1;
            *o = t as f32;
        }
    });
    out
}

/// scipy の zoom が出す大きさ: `int(round(n * (target / n)))`(Python の round は偶数丸め)。
pub fn zoom_out_len(n_in: usize, target: usize) -> usize {
    let f = target as f64 / n_in as f64;
    (n_in as f64 * f).round_ties_even() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflect_matches_scipy_rule() {
        // n=2 [a b] → 左へ a b b a、右へ b a a b
        let n = 2;
        let left: Vec<usize> = (-4..0).map(|i| reflect_index(i, n)).collect();
        assert_eq!(left, vec![0, 1, 1, 0]);
        let right: Vec<usize> = (2..6).map(|i| reflect_index(i, n)).collect();
        assert_eq!(right, vec![1, 0, 0, 1]);
    }
}

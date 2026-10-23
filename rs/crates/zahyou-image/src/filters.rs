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

/// 対称な重みの分離型フィルター(軸 0 → 軸 1)。`index` は端の延長の規則。
/// 画素ごとの計算は scipy の NI_Correlate1D の対称の枝と同じ順: o = x[0]*w0、続いて外側から
/// o += (x[-j] + x[+j]) * w[j](j = r .. 1)を float64 で行い、軸ごとに float32 に丸める。
/// 速さのため、行ごとに「重み 1 つ分を行全体に足す」を外側から順に繰り返す(画素ごとの足す順は同じ)。
/// 出力は 1 枚だけ確保し、軸 1 はその行の写しから計算して同じ行へ書き戻す。
fn separable_sym(data: &[f32], ny: usize, nx: usize, w: &[f64], index: fn(isize, usize) -> usize) -> Vec<f32> {
    let mut out = Vec::new();
    separable_sym_into(data, ny, nx, w, index, &mut out);
    out
}

/// `separable_sym` の書き先を受け取る版。out の長さが ny×nx 以上なら切り詰めて使う(確保もゼロ埋めもしない)。
/// 軸 0 が全画素を書く(各行の nx 個すべて)ので、前の中身は残らず、値は新しく確保したときと同じ。
fn separable_sym_into(
    data: &[f32],
    ny: usize,
    nx: usize,
    w: &[f64],
    index: fn(isize, usize) -> usize,
    out: &mut Vec<f32>,
) {
    let r = w.len() - 1;
    let nxc = nx.max(1);
    if out.len() >= ny * nx {
        out.truncate(ny * nx);
    } else {
        // 古い配列を先に捨ててから確保する(一瞬でも 2 本持たない)
        drop(std::mem::take(out));
        *out = vec![0.0f32; ny * nx];
    }
    // 軸 0: 出力の行 y は入力の行 y-r..y+r から決まる(行ごとに並列)
    out.par_chunks_mut(nxc).enumerate().for_each_init(
        || vec![0.0f64; nx],
        |acc, (y, orow)| {
            let row = |k: isize| {
                let yy = index(y as isize + k, ny);
                &data[yy * nx..yy * nx + nx]
            };
            for (a, &v) in acc.iter_mut().zip(row(0)) {
                *a = v as f64 * w[0];
            }
            for j in (1..=r).rev() {
                let (lo, hi, wj) = (row(-(j as isize)), row(j as isize), w[j]);
                for ((a, &p), &q) in acc.iter_mut().zip(lo).zip(hi) {
                    *a += (p as f64 + q as f64) * wj;
                }
            }
            for (o, &a) in orow.iter_mut().zip(acc.iter()) {
                *o = a as f32;
            }
        },
    );
    // 軸 1: 行ごと(行を端で延ばした float64 の写しから計算し、同じ行へ書き戻す)
    out.par_chunks_mut(nxc).for_each_init(
        || (vec![0.0f64; nx + 2 * r], vec![0.0f64; nx]),
        |(ext, acc), orow| {
            for (e, slot) in ext[..r].iter_mut().enumerate() {
                *slot = orow[index(e as isize - r as isize, nx)] as f64;
            }
            for (slot, &v) in ext[r..r + nx].iter_mut().zip(orow.iter()) {
                *slot = v as f64;
            }
            for (k, slot) in ext[r + nx..].iter_mut().enumerate() {
                *slot = orow[index((nx + k) as isize, nx)] as f64;
            }
            for (a, &v) in acc.iter_mut().zip(&ext[r..r + nx]) {
                *a = v * w[0];
            }
            for j in (1..=r).rev() {
                let wj = w[j];
                let (lo, hi) = (&ext[r - j..r - j + nx], &ext[r + j..r + j + nx]);
                for ((a, &p), &q) in acc.iter_mut().zip(lo).zip(hi) {
                    *a += (p + q) * wj;
                }
            }
            for (o, &a) in orow.iter_mut().zip(acc.iter()) {
                *o = a as f32;
            }
        },
    );
}

/// `ndimage.gaussian_filter(a, sigma=1.0)`(mode="reflect"、truncate=4)。
pub fn gaussian1(data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
    if ny == 0 || nx == 0 {
        return Vec::new();
    }
    separable_sym(data, ny, nx, &GAUSS1, reflect_index)
}

/// scipy の `_gaussian_kernel1d(sigma, 0, radius)` の重みの半分(添字は中心からの距離、[0] が中心)。
/// 半径は `int(4 * sigma + 0.5)`(truncate=4)。scipy と同じく `exp(-0.5 / sigma² * x²)` を numpy と同じ
/// ペアワイズ和で割る。exp は numpy(この機械では UCRT の exp)と同じ値を返すことを確かめてある。
/// sigma が有限の正の数でない、または大きすぎる(半径 1000 超)ときは None。
pub fn gaussian_weights(sigma: f64) -> Option<Vec<f64>> {
    if !(sigma.is_finite() && sigma > 0.0) {
        return None;
    }
    let lw = 4.0 * sigma + 0.5;
    if lw >= 1001.0 {
        return None;
    }
    let r = lw as usize; // Python の int(): 正の数は切り捨て
    let c = -0.5 / (sigma * sigma);
    let phi: Vec<f64> = (0..=2 * r)
        .map(|i| {
            let x = i as f64 - r as f64;
            (c * (x * x)).exp()
        })
        .collect();
    let total = crate::stats::sum_f64(&phi);
    Some((0..=r).map(|j| phi[r + j] / total).collect())
}

/// `ndimage.gaussian_filter(a, sigma)`(mode="reflect"、truncate=4)。軸 0 → 軸 1 の順で、途中を float32 に丸める。
/// sigma が使えない値なら入力をそのまま返す(scipy は sigma ≤ 1e-15 なら何もしない)。
pub fn gaussian(data: &[f32], ny: usize, nx: usize, sigma: f64) -> Vec<f32> {
    let mut out = Vec::new();
    gaussian_into(data, ny, nx, sigma, &mut out);
    out
}

/// `gaussian` の書き先を受け取る版(深い検出が、使い終えた同じ大きさの配列を書き先に回す)。
/// out の前の中身は値に効かない(separable_sym_into は全画素を書き、sigma が使えないときは data の写しにする)。
pub(crate) fn gaussian_into(data: &[f32], ny: usize, nx: usize, sigma: f64, out: &mut Vec<f32>) {
    if ny == 0 || nx == 0 || data.len() != ny * nx {
        out.clear();
        return;
    }
    match gaussian_weights(sigma) {
        Some(w) if sigma > 1e-15 => separable_sym_into(data, ny, nx, &w, reflect_index, out),
        _ => {
            out.clear();
            out.extend_from_slice(data);
        }
    }
}

/// `ndimage.maximum_filter(a, size=2h+1)`(mode="reflect")。最大値は端を鏡映で延ばしても窓を画像の内側に
/// 縮めても同じなので、窓を縮めて計算する(NaN を含まない画像に使う)。深い検出は極大を 1 点ずつ調べる
/// (deep.rs の is_local_max)ので、これは照合の試験だけで使う。
#[cfg(test)]
pub(crate) fn maximum_filter(data: &[f32], ny: usize, nx: usize, h: usize) -> Vec<f32> {
    if ny == 0 || nx == 0 || data.len() != ny * nx {
        return Vec::new();
    }
    #[inline]
    fn mx(a: f32, b: f32) -> f32 {
        if b > a { b } else { a }
    }
    // 軸 0: 行 y の出力は行 y-h..=y+h(画像の内側)の最大値
    let mut tmp = vec![0.0f32; ny * nx];
    tmp.par_chunks_mut(nx).enumerate().for_each(|(y, orow)| {
        let y0 = y.saturating_sub(h);
        let y1 = (y + h).min(ny - 1);
        orow.copy_from_slice(&data[y0 * nx..y0 * nx + nx]);
        for yy in y0 + 1..=y1 {
            let row = &data[yy * nx..yy * nx + nx];
            for (o, &v) in orow.iter_mut().zip(row) {
                *o = mx(*o, v);
            }
        }
    });
    // 軸 1: 列 x の出力は列 x-h..=x+h(画像の内側)の最大値。ずらした行との最大を h 回ずつ取る
    let mut out = tmp.clone();
    out.par_chunks_mut(nx).enumerate().for_each(|(y, orow)| {
        let row = &tmp[y * nx..y * nx + nx];
        for d in 1..=h.min(nx - 1) {
            for (o, &v) in orow[..nx - d].iter_mut().zip(&row[d..]) {
                *o = mx(*o, v);
            }
            for (o, &v) in orow[d..].iter_mut().zip(&row[..nx - d]) {
                *o = mx(*o, v);
            }
        }
    });
    out
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
    let strips = uniform3_strips(data, ny, nx);
    // 軸 1: 行ごと。軸 0 の結果の行を帯から集めてかける(全面の並べ直しの配列は作らない)
    let mut out = vec![0.0f32; ny * nx];
    out.par_chunks_mut(nx).enumerate().for_each_init(
        || vec![0.0f32; nx],
        |t0, (y, orow)| {
            uniform3_gather_row(&strips, y, t0);
            uniform3_row(t0, orow);
        },
    );
    out
}

/// uniform3 の軸 0 の結果の縦の帯 1 本(列 x0..x0+w、行優先の ny×w)。
pub(crate) struct Strip {
    x0: usize,
    w: usize,
    buf: Vec<f32>,
}

/// 縦の帯の幅。列ごとに独立に計算するので幅は値に効かない。帯の数を、幅 256 以下になるスレッド数の倍数にして
/// 最後の周回で遊ぶスレッドを減らす(細すぎると 1 列あたりの手間が増えるので 32 列以上)。
fn strip_width(nx: usize) -> usize {
    let t = rayon::current_num_threads().max(1);
    let n = nx.div_ceil(256).div_ceil(t).max(1) * t;
    nx.div_ceil(n).max(32)
}

/// uniform3 の軸 0: 列ごとの持ち回しの和(scipy と同じく `tmp += 新 - 旧` を float64 で、毎回 3 で割って float32)。
/// 縦の帯ごとに並列に計算する。ny・nx は 1 以上、data は ny×nx 以上。
pub(crate) fn uniform3_strips(data: &[f32], ny: usize, nx: usize) -> Vec<Strip> {
    let sw = strip_width(nx);
    (0..nx.div_ceil(sw))
        .into_par_iter()
        .map(|s| {
            let x0 = s * sw;
            let x1 = (x0 + sw).min(nx);
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
            Strip { x0, w, buf }
        })
        .collect()
}

/// 帯から軸 0 の結果の行 y を t0row(長さ nx)へ集める。
#[inline]
pub(crate) fn uniform3_gather_row(strips: &[Strip], y: usize, t0row: &mut [f32]) {
    for s in strips {
        t0row[s.x0..s.x0 + s.w].copy_from_slice(&s.buf[y * s.w..y * s.w + s.w]);
    }
}

/// uniform3 の軸 1 の 1 行ぶん: t0row(軸 0 の結果の行、長さ 1 以上)の持ち回しの和を orow(同じ長さ)へ。
#[inline]
pub(crate) fn uniform3_row(t0row: &[f32], orow: &mut [f32]) {
    let nx = t0row.len();
    let at = |x: isize| t0row[nearest_index(x, nx)] as f64;
    let mut t = 0.0f64;
    t += at(-1);
    t += at(0);
    t += at(1);
    orow[0] = (t / 3.0) as f32;
    for (l, o) in orow.iter_mut().enumerate().skip(1) {
        t += at(l as isize + 1) - at(l as isize - 2);
        *o = (t / 3.0) as f32;
    }
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

/// zoom の 1 画素: 4 点の重みつきの和を float64 で足して float32 に丸める。zoom_linear と zoom_linear_sub で共通。
#[inline]
fn zoom_px(
    c: &[f32],
    bx: usize,
    (y0, y1, wy0, wy1): (usize, usize, f64, f64),
    (x0, x1, wx0, wx1): (usize, usize, f64, f64),
) -> f32 {
    let mut t = 0.0f64;
    t += c[y0 * bx + x0] as f64 * wy0 * wx0;
    t += c[y0 * bx + x1] as f64 * wy0 * wx1;
    t += c[y1 * bx + x0] as f64 * wy1 * wx0;
    t += c[y1 * bx + x1] as f64 * wy1 * wx1;
    t as f32
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
        for (o, &a) in orow.iter_mut().zip(ax.iter()) {
            *o = zoom_px(c, bx, ay[i], a);
        }
    });
    out
}

/// `data - zoom_linear(c, by, bx, ny, nx)`(data は ny×nx)。背景の配列を作らずに、引いた値を直接書く。
/// 各画素は zoom_linear と同じ zoom_px の値を float32 で引くだけなので、背景を作ってから引くのとビット単位で同じ。
pub(crate) fn zoom_linear_sub(c: &[f32], by: usize, bx: usize, data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
    let ay = zoom_axis(by, ny);
    let ax = zoom_axis(bx, nx);
    let mut out = vec![0.0f32; ny * nx];
    if nx == 0 {
        return out;
    }
    out.par_chunks_mut(nx).zip(data.par_chunks(nx)).enumerate().for_each(|(i, (orow, drow))| {
        for ((o, &d), &a) in orow.iter_mut().zip(drow).zip(ax.iter()) {
            *o = d - zoom_px(c, bx, ay[i], a);
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
pub(crate) mod tests {
    use super::*;

    /// 2026-10-24 までの uniform3(帯の幅 256、軸 0 の結果を全面の t0 へ並べ直してから軸 1)。照合の基準
    pub(crate) fn uniform3_ref(data: &[f32], ny: usize, nx: usize) -> Vec<f32> {
        if ny == 0 || nx == 0 {
            return Vec::new();
        }
        const STRIP: usize = 256;
        let strips: Vec<(usize, Vec<f32>)> = (0..nx.div_ceil(STRIP))
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
        for (y, orow) in t0.chunks_mut(nx).enumerate() {
            for (x0, buf) in &strips {
                let w = (STRIP).min(nx - x0);
                orow[*x0..x0 + w].copy_from_slice(&buf[y * w..y * w + w]);
            }
        }
        let mut out = vec![0.0f32; ny * nx];
        for (y, orow) in out.chunks_mut(nx).enumerate() {
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
        }
        out
    }

    /// 決まった擬似乱数(xorshift)の画像。値の幅は大きめ(持ち回しの和の丸めの積み重ねが出るように)
    pub(crate) fn rand_vals(n: usize, seed: u64) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 11) as f64 / (1u64 << 53) as f64 * 1.0e4 - 3.0e3) as f32
            })
            .collect()
    }

    #[test]
    fn uniform3_matches_reference() {
        // 帯の幅を変え、全面の並べ直しをやめても、前の版とビット単位で同じ(帯の境目をまたぐ幅・1 行・1 列・NaN 入り)
        for (ny, nx) in
            [(1usize, 1usize), (1, 700), (700, 1), (2, 3), (37, 300), (5, 257), (64, 256), (90, 1030), (33, 6001)]
        {
            let mut d = rand_vals(ny * nx, (ny * 7919 + nx) as u64);
            if d.len() > 50 {
                d[17] = f32::NAN;
                d[40] = f32::INFINITY;
            }
            let want = uniform3_ref(&d, ny, nx);
            let got = uniform3(&d, ny, nx);
            assert_eq!(got.len(), want.len());
            assert!(got.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()), "{ny}x{nx}");
        }
    }

    #[test]
    fn zoom_sub_matches_zoom_then_subtract() {
        // 背景を作ってから引くのと、引いた値を直接書くのが同じ
        for (by, bx, ny, nx) in [(3usize, 3usize, 200usize, 250usize), (5, 9, 333, 600), (62, 93, 4000, 6000)] {
            let c = rand_vals(by * bx, 3);
            let d = rand_vals(ny * nx, 4);
            let back = zoom_linear(&c, by, bx, ny, nx);
            let want: Vec<f32> = d.iter().zip(&back).map(|(&d, &b)| d - b).collect();
            let got = zoom_linear_sub(&c, by, bx, &d, ny, nx);
            assert!(got.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()), "{ny}x{nx}");
        }
    }

    #[test]
    fn gaussian_into_ignores_old_contents() {
        // 書き先に前の値(NaN・長い・短い)が残っていても、新しく作ったものと同じ。sigma が使えないときは写し
        let (ny, nx) = (61, 77);
        let d = rand_vals(ny * nx, 9);
        for sigma in [1.0, 1.7, 3.2, 0.0, f64::NAN, 1e-20] {
            let want = gaussian(&d, ny, nx, sigma);
            for init in [Vec::new(), vec![f32::NAN; ny * nx], vec![5.0f32; 10], vec![-1.0f32; ny * nx + 99]] {
                let mut out = init;
                gaussian_into(&d, ny, nx, sigma, &mut out);
                assert_eq!(out.len(), want.len());
                assert!(out.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()), "sigma={sigma}");
            }
        }
        let mut out = vec![1.0f32; 5];
        gaussian_into(&d, ny, nx + 1, 1.0, &mut out);
        assert!(out.is_empty());
    }

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

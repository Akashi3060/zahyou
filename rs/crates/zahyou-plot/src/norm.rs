//! 表示の伸張。Python 版 `dev/zahyou_plot.py` の `_norm`(astropy の ImageNormalize + AsinhStretch(1.0))と、
//! matplotlib の灰色の色表(`imshow(..., cmap="gray")`、256 段)で 8 bit にするところ。
//!
//! 範囲は「背景の中央値 − 1σ」から「max(99.9 パーセント点, 中央値 + 12σ)」まで(中央値と σ は 3σ クリップ)。
//! 値 v は x = (v − vmin) / (vmax − vmin)、y = asinh(x) / asinh(1) で写し、matplotlib と同じく
//! y·256 を切り捨てた段の色にする(0 未満は黒、1 以上は白、有限でない値は透明)。
//!
//! 途中の型も Python 版と同じにしてある:
//! - σ クリップは float32 の画像に対する astropy の値(zahyou_image::kernels::sigma_clipped_stats がビット一致)
//! - `np.percentile(vals, 99.9)` は float32 の配列なので **float32 で** 計算される
//!   (numpy 2 は q を配列の型で割り、位置 (n−1)·q も、補間も float32)
//! - 伸張の式は float64(ImageNormalize は値を float64 にしてから計算する)

use std::borrow::Cow;

use rayon::prelude::*;
use zahyou_image::Image2D;
use zahyou_image::kernels::{sigma_clipped_stats, std_f32};

/// `np.arcsinh(1.0)`(AsinhStretch(a=1.0) で割る数)
const ASINH_1: f64 = 0.881373587019543;

/// これより長い配列は、パーセント点の選択をヒストグラムで行う(短いものは並べ替え)
const RADIX_MIN: usize = 1 << 16;
/// 並列の処理単位(要素数)
const CHUNK: usize = 1 << 16;

/// 伸張の範囲(Python の `_norm` が返す ImageNormalize の vmin・vmax)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Norm {
    pub vmin: f64,
    pub vmax: f64,
}

impl Norm {
    /// 値を 0〜1 の明るさへ(ImageNormalize の __call__ と同じ式。0〜1 の外にもなる)。
    #[inline]
    pub fn unit(&self, v: f64) -> f64 {
        let x = (v - self.vmin) / (self.vmax - self.vmin);
        // AsinhStretch: x / a(a = 1.0)→ arcsinh → / arcsinh(1 / a)
        (x / 1.0).asinh() / ASINH_1
    }

    /// 値を 8 bit の灰色へ(matplotlib の `cmap(norm(v), bytes=True)` の R)。有限でない値は None(透明)。
    #[inline]
    pub fn gray8(&self, v: f32) -> Option<u8> {
        if !v.is_finite() {
            return None;
        }
        lut_index(self.unit(v as f64)).map(|i| GRAY_LUT[i])
    }
}

/// matplotlib の Colormap.__call__ の段の選び方(N = 256)。NaN は None(「悪い値」= 透明)。
#[inline]
fn lut_index(y: f64) -> Option<usize> {
    let mut xa = y * 256.0;
    if xa == 256.0 {
        xa = 255.0;
    }
    if xa.is_nan() {
        return None;
    }
    if xa < 0.0 {
        return Some(0); // 「下回り」の色 = 先頭の色
    }
    if xa >= 256.0 {
        return Some(255); // 「上回り」の色 = 最後の色
    }
    Some(xa as usize)
}

/// matplotlib の灰色の色表を 8 bit にしたもの(`plt.get_cmap("gray")(x, bytes=True)`)。
///
/// 色表は linspace(0, 1, 256) で、bytes にするとき 255 倍して**切り捨てる**ので、
/// 33 → 32 のように 24 か所が 1 小さい(matplotlib と同じ)。
pub const GRAY_LUT: [u8; 256] = gray_lut_const();

const fn gray_lut_const() -> [u8; 256] {
    let mut lut = [0u8; 256];
    let step = 1.0f64 / 255.0;
    let mut i = 0;
    while i < 255 {
        // numpy の linspace: i·step(+ 0)。最後の 1 点だけ stop(1.0)ちょうど
        let x = i as f64 * step;
        lut[i] = (x * 255.0) as u8;
        i += 1;
    }
    lut[255] = 255;
    lut
}

/// Python の `_norm(data)`。有限な画素が 1 つも無ければ None。
///
/// `data` は表示する画像の画素(並びは問わない。NaN・±∞ を含んでよい)。
pub fn compute_norm(data: &[f32]) -> Option<Norm> {
    let vals = finite_values(data);
    norm_of_finite(&vals)
}

/// 有限な値だけを、並びを保ったまま集める(`data[np.isfinite(data)]`)。
///
/// 全部有限なら写さずに `data` をそのまま返す(図 1 の表示用画像はふつう NaN を含まない)。写した配列は
/// 元と同じ並び・同じ値になるだけなので、後の和(区切りは配列の先頭からの位置で決まる)・σ クリップ・
/// パーセント点はビット単位で同じ。NaN・±∞ を含むとき(北が上の図は外側がいつも NaN)は、CHUNK ごとに
/// 個数を数えて合計の長さで 1 回だけ確保し、塊ごとの出力の区切りへ並列に書く(旧版は塊ごとの Vec を作って
/// から 1 本のスレッドで concat していた)。数える述語と書く述語は同じ is_finite なので区切りの長さは合い、
/// 並びも旧版と同じ。
pub(crate) fn finite_values(data: &[f32]) -> Cow<'_, [f32]> {
    if data.len() < CHUNK {
        if data.iter().all(|x| x.is_finite()) {
            return Cow::Borrowed(data);
        }
        return Cow::Owned(data.iter().copied().filter(|x| x.is_finite()).collect());
    }
    if data.par_iter().all(|x| x.is_finite()) {
        return Cow::Borrowed(data);
    }
    let counts: Vec<usize> = data.par_chunks(CHUNK).map(|c| c.iter().filter(|x| x.is_finite()).count()).collect();
    let total: usize = counts.iter().sum();
    let mut out = vec![0.0f32; total];
    let mut parts: Vec<&mut [f32]> = Vec::with_capacity(counts.len());
    let mut rest: &mut [f32] = &mut out;
    for &c in &counts {
        let (h, t) = rest.split_at_mut(c);
        parts.push(h);
        rest = t;
    }
    data.par_chunks(CHUNK).zip(parts.into_par_iter()).for_each(|(c, o)| {
        // zip なので、仮に長さが食い違っても添字の外へは書かない(述語が同じなので食い違わない)
        for (d, x) in o.iter_mut().zip(c.iter().copied().filter(|x| x.is_finite())) {
            *d = x;
        }
    });
    Cow::Owned(out)
}

/// 北が上の図の `_norm`。Python では reproject の結果が **float64** の配列なので、σ クリップ・`np.std`・
/// `np.percentile` が float64 で計算される(q = 99.9/100、位置 (n−1)·q、補間とも float64)。
/// Rust は描き直した画像を float32 で持つので、値を float64 にして同じ式で計算する(差は float32 の丸め程度)。
/// float64 なので、±∞ を float32 の最大値にした画素があっても 2 乗の和があふれない(Python と同じ)。
pub fn compute_norm_f64(data: &[f32]) -> Option<Norm> {
    // NaN を含む data を直接読む形にはしない(和の 8192 個の区切りが写した配列とずれて、最後の桁が変わる)。
    // finite_values は写した配列と同じ並び・同じ値を返す(全部有限なら data そのもの)ので、区切りも同じ
    let vals = finite_values(data);
    if vals.is_empty() {
        return None;
    }
    let (med, mut sigma) = clip_stats_f64(&vals);
    if !sigma.is_finite() || sigma <= 0.0 {
        let s = std_f64(&vals);
        sigma = if s == 0.0 { 1.0 } else { s };
    }
    let p = percentile_f64(&vals, 99.9);
    Some(finish(med, sigma, p))
}

/// `_norm` の本体(有限な値だけの配列に対して)。
pub(crate) fn norm_of_finite(vals: &[f32]) -> Option<Norm> {
    if vals.is_empty() {
        return None;
    }
    let st = sigma_clipped_stats(vals);
    let med = st.median as f64;
    let mut sigma = st.std as f64;
    if !sigma.is_finite() || sigma <= 0.0 {
        // float(np.std(vals)) or 1.0
        let s = std_f32(vals) as f64;
        sigma = if s == 0.0 { 1.0 } else { s };
    }
    Some(finish(med, sigma, percentile_f32(vals, 99.9) as f64))
}

/// vmin = 中央値 − 1σ、vmax = max(99.9 パーセント点, 中央値 + 12σ)。
fn finish(med: f64, sigma: f64, p: f64) -> Norm {
    let lo = med - 1.0 * sigma;
    let b = med + 12.0 * sigma;
    // Python の max(a, b): b が大きいときだけ b(等しい・比べられないときは a)
    let mut hi = if b > p { b } else { p };
    // Python の `if not (hi > lo)`(どちらかが NaN のときも)
    if hi.is_nan() || lo.is_nan() || hi <= lo {
        hi = lo + 1e-6;
    }
    Norm { vmin: lo, vmax: hi }
}

/// float64 の和(8192 個ずつの区切りの和を、並列に求めてから先頭から順に足す)。
fn sum_f64_of<F: Fn(f32) -> f64 + Sync>(vals: &[f32], f: F) -> f64 {
    let parts: Vec<f64> = vals.par_chunks(8192).map(|c| sum8(c, &f)).collect();
    parts.iter().sum()
}

/// `np.std`(float64、ddof = 0)。
fn std_f64(vals: &[f32]) -> f64 {
    if vals.is_empty() {
        return f64::NAN;
    }
    let n = vals.len() as f64;
    let mean = sum_f64_of(vals, |x| x as f64) / n;
    let ss = sum_f64_of(vals, |x| {
        let d = x as f64 - mean;
        d * d
    });
    (ss / n).sqrt()
}

/// float64 の和(8192 個ずつの区切りの和を順に足す。区切りの中は 8 本の部分和)。
#[inline]
fn sum8<F: Fn(f32) -> f64>(c: &[f32], f: &F) -> f64 {
    let mut r = [0.0f64; 8];
    let mut it = c.chunks_exact(8);
    for g in &mut it {
        for j in 0..8 {
            r[j] += f(g[j]);
        }
    }
    let mut t = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
    for &x in it.remainder() {
        t += f(x);
    }
    t
}

/// 並列の塊(ヒストグラムを持つので大きめ)
const BIG: usize = 1 << 20;

/// 有限な float32 の配列のうち、float64 にして [lo, hi] に入る値の (個数, 中央値, σ)(`np.median`・`np.std`)。
///
/// 読むのは 2 回だけ: 1 回目で個数・鍵の上位 16 bit のヒストグラム・和、2 回目で中央値の入る段の下位 16 bit の
/// ヒストグラムと偏差の 2 乗の和。値を写して集め直さない(σ クリップで残る値は「元の配列のうち、それまでの
/// 境界の共通部分に入る値」なので、区間で選べば同じ集合になる)。
fn median_std_in(vals: &[f32], lo: f64, hi: f64) -> (usize, f64, f64) {
    let keep = |x: f32| {
        let v = x as f64;
        v >= lo && v <= hi
    };
    let parts: Vec<(Vec<u32>, f64, usize)> = vals
        .par_chunks(BIG)
        .map(|c| {
            let mut h = vec![0u32; 65536];
            let mut sum = 0.0f64;
            let mut cnt = 0usize;
            for sub in c.chunks(8192) {
                let mut r = [0.0f64; 8];
                let mut j = 0;
                for &x in sub {
                    if keep(x) {
                        h[(f32_key(x) >> 16) as usize] += 1;
                        r[j & 7] += x as f64;
                        j += 1;
                    }
                }
                cnt += j;
                sum += ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
            }
            (h, sum, cnt)
        })
        .collect();
    let mut h1 = vec![0u32; 65536];
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for (h, s, c) in &parts {
        for (a, b) in h1.iter_mut().zip(h) {
            *a += *b;
        }
        sum += s;
        n += c;
    }
    if n == 0 {
        return (0, f64::NAN, f64::NAN);
    }
    let mean = sum / n as f64;
    let ks: Vec<usize> = if n % 2 == 1 { vec![n / 2] } else { vec![n / 2 - 1, n / 2] };
    let bins: Vec<(usize, usize)> = ks.iter().map(|&k| find_bin(&h1, k)).collect();
    let (b_a, b_b) = (bins[0].0 as u32, bins[bins.len() - 1].0 as u32);
    let parts2: Vec<(Vec<u32>, Vec<u32>, f64)> = vals
        .par_chunks(BIG)
        .map(|c| {
            let mut ha = vec![0u32; 65536];
            let mut hb = if b_b != b_a { vec![0u32; 65536] } else { Vec::new() };
            let mut ss = 0.0f64;
            for sub in c.chunks(8192) {
                let mut r = [0.0f64; 8];
                let mut j = 0;
                for &x in sub {
                    if keep(x) {
                        let key = f32_key(x);
                        let top = key >> 16;
                        if top == b_a {
                            ha[(key & 0xFFFF) as usize] += 1;
                        } else if top == b_b {
                            hb[(key & 0xFFFF) as usize] += 1;
                        }
                        let d = x as f64 - mean;
                        r[j & 7] += d * d;
                        j += 1;
                    }
                }
                ss += ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
            }
            (ha, hb, ss)
        })
        .collect();
    let mut ha = vec![0u32; 65536];
    let mut hb = vec![0u32; 65536];
    let mut ss = 0.0f64;
    for (a, b, s) in &parts2 {
        for (x, y) in ha.iter_mut().zip(a) {
            *x += *y;
        }
        for (x, y) in hb.iter_mut().zip(b) {
            *x += *y;
        }
        ss += s;
    }
    let pick = |i: usize| {
        let (bin, base) = bins[i];
        let h = if bin as u32 == b_a { &ha } else { &hb };
        let (low, _) = find_bin(h, ks[i] - base);
        key_to_f32(((bin as u32) << 16) | low as u32) as f64
    };
    let med = if ks.len() == 1 { pick(0) } else { (pick(0) + pick(1)) / 2.0 };
    (n, med, (ss / n as f64).sqrt())
}

/// astropy の sigma_clipped_stats(sigma=3、maxiters=5、中心は median、std は ddof=0)の (中央値, σ) を float64 で。
/// 境界 [中央値 − 3σ, 中央値 + 3σ] の外を捨て、何も変わらなくなるか 5 回で止める。
fn clip_stats_f64(vals: &[f32]) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    let (mut n, mut med, mut sd) = median_std_in(vals, lo, hi);
    for _ in 0..5 {
        if n == 0 {
            return (f64::NAN, f64::NAN);
        }
        // 残す区間は、それまでの区間との共通部分(集合は入れ子になる)
        let (l2, h2) = (med - sd * 3.0, med + sd * 3.0);
        let (nl, nh) = (if l2 > lo { l2 } else { lo }, if h2 < hi { h2 } else { hi });
        if nl.is_nan() || nh.is_nan() || nl > nh {
            return (f64::NAN, f64::NAN);
        }
        let (n2, m2, s2) = median_std_in(vals, nl, nh);
        if n2 == n {
            // 何も変わらない(値の集合が同じなので統計も同じ)
            break;
        }
        (lo, hi, n, med, sd) = (nl, nh, n2, m2, s2);
    }
    (med, sd)
}

/// `np.percentile(vals, q)`(vals は float32 の 1 次元。method="linear")。空なら NaN。
///
/// numpy 2 は float32 の配列に対して、q を `float32(100)` で割り、位置 (n−1)·q も float32 で計算する
/// (n−1 は Python の整数なので float32 に丸められる)。補間 `_lerp` も float32。
/// NaN を含む配列は渡さないこと(`_norm` は有限な値だけを渡す)。
pub fn percentile_f32(vals: &[f32], q: f64) -> f32 {
    let n = vals.len();
    if n == 0 {
        return f32::NAN;
    }
    let qq = (q as f32) / 100.0f32;
    let vi = ((n - 1) as f32) * qq;
    let last = n - 1;
    // _get_indexes: 範囲の外は端の値。NaN の位置は最後の値
    let (pi, ni) = if vi.is_nan() || vi >= (n - 1) as f32 {
        (last, last)
    } else if vi < 0.0 {
        (0, 0)
    } else {
        let p = vi.floor();
        let nx = p + 1.0f32; // float32 の足し算(2^24 を超えると丸められる。numpy と同じ)
        ((p as usize).min(last), (nx as usize).min(last))
    };
    // gamma = 位置 − 前の添字(float64 で引いて float32 にする。範囲の外では前の添字が −1 扱いだが、
    // そのとき前後は同じ値なので結果に効かない)
    let gamma = (vi as f64 - pi as f64) as f32;
    let picked = kth_many(vals, &[pi, ni]);
    let (a, b) = (picked[0], picked[1]);
    let diff = b - a;
    if gamma >= 0.5 { b - diff * (1.0f32 - gamma) } else { a + diff * gamma }
}

/// `np.percentile(vals, q)` を float64 の配列として計算した値(値は float32 を float64 にしたもの)。空なら NaN。
pub fn percentile_f64(vals: &[f32], q: f64) -> f64 {
    let n = vals.len();
    if n == 0 {
        return f64::NAN;
    }
    let qq = q / 100.0;
    let vi = (n - 1) as f64 * qq;
    let last = n - 1;
    let (pi, ni) = if vi.is_nan() || vi >= (n - 1) as f64 {
        (last, last)
    } else if vi < 0.0 {
        (0, 0)
    } else {
        let p = vi.floor();
        ((p as usize).min(last), ((p + 1.0) as usize).min(last))
    };
    let gamma = vi - pi as f64;
    let picked = kth_many(vals, &[pi, ni]);
    let (a, b) = (picked[0] as f64, picked[1] as f64);
    let diff = b - a;
    if gamma >= 0.5 { b - diff * (1.0 - gamma) } else { a + diff * gamma }
}

/// float32 の大小と同じ順になる 32 bit の鍵(負の数はビットを反転、正の数は符号ビットを立てる)。
#[inline]
fn f32_key(x: f32) -> u32 {
    let b = x.to_bits();
    if b & 0x8000_0000 != 0 { !b } else { b | 0x8000_0000 }
}

/// 鍵のヒストグラム(65536 段)。`bin` が None の値は数えない。
fn hist16<F: Fn(u32) -> Option<u32> + Sync>(vals: &[f32], bin: F) -> Vec<u32> {
    vals.par_chunks(CHUNK)
        .fold(
            || vec![0u32; 65536],
            |mut h, c| {
                for &x in c {
                    if let Some(b) = bin(f32_key(x)) {
                        h[b as usize] += 1;
                    }
                }
                h
            },
        )
        .reduce(
            || vec![0u32; 65536],
            |mut a, b| {
                for (x, y) in a.iter_mut().zip(b) {
                    *x += y;
                }
                a
            },
        )
}

/// ヒストグラムで、順位 k の入る段と、その段より前の個数。
fn find_bin(h: &[u32], k: usize) -> (usize, usize) {
    let mut cum = 0usize;
    for (i, &c) in h.iter().enumerate() {
        if cum + c as usize > k {
            return (i, cum);
        }
        cum += c as usize;
    }
    (h.len() - 1, cum)
}

/// 小さい方から k 番目(0 始まり)の値を、複数の k について(NaN を含まない配列)。
/// 長い配列は鍵(32 bit)の上位 16 bit・下位 16 bit の 2 段のヒストグラムで正確に選ぶ(並べ替えない)。
fn kth_many(vals: &[f32], ks: &[usize]) -> Vec<f32> {
    let n = vals.len();
    if n == 0 {
        return vec![f32::NAN; ks.len()];
    }
    if n < RADIX_MIN {
        let mut v = vals.to_vec();
        v.sort_unstable_by(|a, b| a.total_cmp(b));
        return ks.iter().map(|&k| v[k.min(n - 1)]).collect();
    }
    let h1 = hist16(vals, |key| Some(key >> 16));
    let mut cache: Vec<(u32, Vec<u32>)> = Vec::new();
    ks.iter()
        .map(|&k| {
            let k = k.min(n - 1);
            let (b1, base) = find_bin(&h1, k);
            let b1u = b1 as u32;
            if !cache.iter().any(|(b, _)| *b == b1u) {
                let h2 = hist16(vals, |key| if key >> 16 == b1u { Some(key & 0xFFFF) } else { None });
                cache.push((b1u, h2));
            }
            let h2 = &cache.iter().find(|(b, _)| *b == b1u).unwrap().1;
            let (lo, _) = find_bin(h2, k - base);
            key_to_f32((b1u << 16) | lo as u32)
        })
        .collect()
}

#[inline]
fn key_to_f32(k: u32) -> f32 {
    if k & 0x8000_0000 != 0 { f32::from_bits(k & 0x7FFF_FFFF) } else { f32::from_bits(!k) }
}

/// 伸張して 8 bit の段にする速い写し(結果は [`Norm::gray8`] と同じ)。
///
/// 段の境目 y = k/256(k = 1〜255)を x = (v − vmin)/(vmax − vmin) の値 X_k = sinh(k·asinh(1)/256) に直しておき、
/// 画素ごとには x を二分探索するだけにする(asinh を呼ばない)。境目から相対 1e-9 以内の x と、x が有限でないときだけ
/// 元の式で計算する(asinh の丸めは 1e-15 程度なので、それより離れた x では段が変わらない)。
pub(crate) struct GrayMap {
    norm: Norm,
    /// X_1〜X_255(増加列)
    xs: [f64; 255],
}

/// 境目のまわりで元の式に戻す幅(相対)
const GUARD: f64 = 1e-9;

impl GrayMap {
    pub(crate) fn new(norm: &Norm) -> GrayMap {
        let mut xs = [0.0f64; 255];
        for (k, x) in xs.iter_mut().enumerate() {
            *x = ((k + 1) as f64 * ASINH_1 / 256.0).sinh();
        }
        GrayMap { norm: *norm, xs }
    }

    /// `Norm::gray8(v)` と同じ値。
    #[inline]
    pub(crate) fn gray8(&self, v: f32) -> Option<u8> {
        if !v.is_finite() {
            return None;
        }
        let n = &self.norm;
        let x = (v as f64 - n.vmin) / (n.vmax - n.vmin);
        if !x.is_finite() {
            return n.gray8(v);
        }
        // 段 = x 以下の境目の数
        let k = self.xs.partition_point(|&t| t <= x);
        let near = |i: usize| (x - self.xs[i]).abs() <= GUARD * self.xs[i];
        if (k > 0 && near(k - 1)) || (k < 255 && near(k)) {
            return n.gray8(v);
        }
        Some(GRAY_LUT[k])
    }
}

/// 画像を伸張して RGBA(1 画素 4 バイト、並びは画像と同じ)にする。灰色なので R = G = B、
/// 有限でない画素は (0, 0, 0, 0)(透明。図では軸の背景色が見える)。`norm` が None なら全部透明。
pub fn gray_rgba(img: &Image2D, norm: Option<&Norm>) -> Vec<u8> {
    let mut out = vec![0u8; img.data.len() * 4];
    let Some(norm) = norm else { return out };
    let map = GrayMap::new(norm);
    out.par_chunks_mut(4 * CHUNK).zip(img.data.par_chunks(CHUNK)).for_each(|(o, src)| {
        for (px, &v) in o.chunks_exact_mut(4).zip(src) {
            if let Some(g) = map.gray8(v) {
                px.copy_from_slice(&[g, g, g, 255]);
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_matches_matplotlib_truncation() {
        // matplotlib 3.10 の get_cmap("gray")(linspace(0,1,256), bytes=True) と同じ(gen_plot.py で全段を照合する)
        assert_eq!(GRAY_LUT[0], 0);
        assert_eq!(GRAY_LUT[32], 32);
        assert_eq!(GRAY_LUT[33], 32);
        assert_eq!(GRAY_LUT[244], 243);
        assert_eq!(GRAY_LUT[255], 255);
        let lower: Vec<usize> = (0..256).filter(|&i| GRAY_LUT[i] as usize != i).collect();
        assert_eq!(lower.len(), 24);
    }

    #[test]
    fn gray_edges() {
        let n = Norm { vmin: 0.0, vmax: 1.0 };
        assert_eq!(n.gray8(-5.0), Some(0));
        assert_eq!(n.gray8(0.0), Some(0));
        assert_eq!(n.gray8(1.0), Some(255));
        assert_eq!(n.gray8(100.0), Some(255));
        assert_eq!(n.gray8(f32::NAN), None);
        assert_eq!(n.gray8(f32::INFINITY), None);
        // 幅が ∞ でもパニックしない(NaN → 透明)
        let w = Norm { vmin: f64::NEG_INFINITY, vmax: f64::INFINITY };
        assert_eq!(w.gray8(1.0), None);
    }

    #[test]
    fn percentile_small_and_large_agree() {
        // 並べ替えとヒストグラムの選び方が同じ値を返す
        let v: Vec<f32> = (0..200_003u32).map(|i| ((i.wrapping_mul(2654435761) >> 7) as f32) * 1e-3 - 5000.0).collect();
        let a = percentile_f32(&v, 99.9);
        let mut s = v.clone();
        s.sort_unstable_by(|a, b| a.total_cmp(b));
        let qq = 99.9f32 / 100.0;
        let vi = ((v.len() - 1) as f32) * qq;
        let p = vi.floor() as usize;
        let g = vi - vi.floor();
        let expect = if g >= 0.5 { s[p + 1] - (s[p + 1] - s[p]) * (1.0 - g) } else { s[p] + (s[p + 1] - s[p]) * g };
        assert_eq!(a.to_bits(), expect.to_bits());
        assert_eq!(percentile_f32(&[3.0], 99.9), 3.0);
        assert!(percentile_f32(&[], 99.9).is_nan());
        assert_eq!(percentile_f32(&[1.0, 2.0], 99.9), 2.0 - 1.0 * (1.0 - 0.999f32));
    }

    #[test]
    fn fast_gray_matches_formula() {
        // 速い写しが元の式と同じ段を返す: 乱数の値、境目の前後の float32、範囲の外、ふつうでない伸張の範囲
        let norms = [
            Norm { vmin: 995.9105882644653, vmax: 6466.2255859375 },
            Norm { vmin: -387.25, vmax: 7886.05 },
            Norm { vmin: 0.0, vmax: 1.0 },
            Norm { vmin: -1e30, vmax: 1e30 },
            Norm { vmin: 4.0, vmax: 4.000001 },
            Norm { vmin: f64::NEG_INFINITY, vmax: f64::INFINITY },
            Norm { vmin: 1.0, vmax: 1.0 },
        ];
        let mut st: u64 = 7;
        for n in norms {
            let map = GrayMap::new(&n);
            let mut vals: Vec<f32> = Vec::new();
            for _ in 0..200_000 {
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let u = (st >> 11) as f64 / (1u64 << 53) as f64;
                vals.push((n.vmin + (u * 1.4 - 0.2) * (n.vmax - n.vmin)) as f32);
            }
            // 段の境目そのもの(元の式を float32 の上で二分探索して、境目の両側の値)
            for k in 1..256usize {
                let span = n.vmax - n.vmin;
                if !span.is_finite() || span <= 0.0 {
                    break;
                }
                let t = (n.vmin + span * ((k as f64) * ASINH_1 / 256.0).sinh()) as f32;
                let mut v = t;
                for _ in 0..4 {
                    vals.push(v);
                    v = f32::from_bits(v.to_bits().wrapping_sub(1));
                }
                let mut v = t;
                for _ in 0..4 {
                    vals.push(v);
                    v = f32::from_bits(v.to_bits().wrapping_add(1));
                }
            }
            vals.extend([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, f32::MIN, 0.0, -0.0]);
            for &v in &vals {
                assert_eq!(map.gray8(v), n.gray8(v), "{n:?} の {v}");
            }
        }
    }

    /// 旧版の finite_values(塊ごとに filter().collect() してから concat)。新版と比べる参照
    fn finite_values_old(data: &[f32]) -> Vec<f32> {
        if data.len() < CHUNK {
            return data.iter().copied().filter(|x| x.is_finite()).collect();
        }
        let parts: Vec<Vec<f32>> =
            data.par_chunks(CHUNK).map(|c| c.iter().copied().filter(|x| x.is_finite()).collect()).collect();
        parts.concat()
    }

    fn compute_norm_old(data: &[f32]) -> Option<Norm> {
        norm_of_finite(&finite_values_old(data))
    }

    fn compute_norm_f64_old(data: &[f32]) -> Option<Norm> {
        let vals = finite_values_old(data);
        if vals.is_empty() {
            return None;
        }
        let (med, mut sigma) = clip_stats_f64(&vals);
        if !sigma.is_finite() || sigma <= 0.0 {
            let s = std_f64(&vals);
            sigma = if s == 0.0 { 1.0 } else { s };
        }
        let p = percentile_f64(&vals, 99.9);
        Some(finish(med, sigma, p))
    }

    fn bits(n: Option<Norm>) -> Option<(u64, u64)> {
        n.map(|n| (n.vmin.to_bits(), n.vmax.to_bits()))
    }

    #[test]
    fn finite_values_matches_old() {
        // NaN・±∞ を混ぜた配列(塊の境目をまたぐ長さ、塊がまるごと非有限、先頭・末尾が非有限)と、全部有限の配列で、
        // 新版の写し(または写さない)が旧版と要素ごとに to_bits まで同じ。伸張の範囲(float32 と float64)も同じ
        let mut st: u64 = 12345;
        let mut rnd = || {
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (st >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut cases: Vec<Vec<f32>> = Vec::new();
        for (n, bad) in
            [(250_001usize, 0.01f64), (200_000, 0.3), (70_000, 0.0), (65_536, 0.5), (65_535, 0.2), (1000, 0.1)]
        {
            let v: Vec<f32> = (0..n)
                .map(|i| {
                    let u = rnd();
                    if u < bad / 3.0 {
                        f32::NAN
                    } else if u < bad * 2.0 / 3.0 {
                        f32::INFINITY
                    } else if u < bad {
                        f32::NEG_INFINITY
                    } else {
                        (1000.0 + 50.0 * (rnd() - 0.5) + if i % 997 == 0 { 3e4 } else { 0.0 }) as f32
                    }
                })
                .collect();
            cases.push(v);
        }
        // 2 つ目の塊がまるごと NaN、先頭と末尾が NaN(北が上の図の外側の形)
        let mut v: Vec<f32> = (0..300_000).map(|i| (i % 4093) as f32 * 0.25 - 100.0).collect();
        for x in &mut v[CHUNK..2 * CHUNK] {
            *x = f32::NAN;
        }
        v[0] = f32::NAN;
        let last = v.len() - 1;
        v[last] = f32::NEG_INFINITY;
        cases.push(v);
        cases.push(vec![f32::NAN; 200_000]);
        cases.push(vec![]);
        for (k, d) in cases.iter().enumerate() {
            let new = finite_values(d);
            let old = finite_values_old(d);
            assert_eq!(new.len(), old.len(), "例 {k}");
            assert!(new.iter().zip(&old).all(|(a, b)| a.to_bits() == b.to_bits()), "例 {k}");
            // 全部有限なら写さない
            if d.iter().all(|x| x.is_finite()) {
                assert!(matches!(new, Cow::Borrowed(_)), "例 {k}");
            }
            assert_eq!(bits(compute_norm(d)), bits(compute_norm_old(d)), "例 {k}");
            assert_eq!(bits(compute_norm_f64(d)), bits(compute_norm_f64_old(d)), "例 {k}");
        }
    }

    #[test]
    fn norm_degenerate() {
        assert!(compute_norm(&[]).is_none());
        assert!(compute_norm(&[f32::NAN, f32::INFINITY]).is_none());
        // 一様な画像: σ = 0 → np.std も 0 → σ = 1
        let n = compute_norm(&[5.0; 100]).unwrap();
        assert_eq!(n.vmin, 4.0);
        assert_eq!(n.vmax, 17.0);
    }
}

//! 深い検出(Python の zahyou_core.detect_sources_deep、2026-10-16 版)。解析エンジンに渡す星を、
//! 図に出す星(detect_sources)より暗いところまで拾う。
//!
//!   1. σ=1 でぼかした画像の明るい星の半値の面積から、星像の幅 σ_psf を測る
//!   2. 画像を s = 1.25 σ_psf(測れなければ 1.5、1〜8 に収める)のガウスでぼかし、升ごとの中央値で背景を引く。
//!      雑音は升ごとの MAD で場所ごとに測る
//!   3. 局所の極大のうち、その場所の雑音の u 倍を超えるものを候補にする(u は雑音だけの画像で偽の極大が
//!      画像全体で target_false 個ほどになる値)
//!   4. 明るい順に見て、谷が浅いもの(同じ星の肩)・伸びたもの・鋭すぎるもの(ホットピクセル)・
//!      まわりの輪より十分高くないものを捨てる
//!   5. 位置はぼかした像の 3×3 に(対数の)放物線を当てて小数まで出す
//!
//! 値は Python 版と同じになるように、計算の順序と途中の型まで写してある:
//! - numpy 2 の NEP 50: float32 の配列・スカラーと Python の float の比較・積は、float を float32 に丸めて float32 で行う
//!   (`sm > u * nmap` は `f32(u) * nmap` を float32 で計算して比べる)
//! - 中央値は float32 の中央 2 個を float32 で足して 2 で割る(np.median・np.nanmedian・np.ma.median で同じ値)
//! - Python の `max(a, b)` / `min(a, b)` は「後ろが厳密に大きい(小さい)ときだけ後ろ」(NaN の扱いが f64::max と違う)
//! - `x ** 2`(Python の float)は C の pow なので `powf(2.0)` で計算する(x * x と最下位ビットが違うことがある)
//! - exp・ln・cos・sin は Python / numpy と同じ値になることを確かめてある(この機械の UCRT)
//!
//! 同じ値の候補(並べ替えの同点)は、元の並び(行優先)の順に見る(安定な並べ替え)。基準実装の Python も
//! 2026-10-16 から `np.argsort(..., kind="stable")` で同じ順にした(それまでは安定でなく、飽和して頂が平らな星で
//! 位置が 1 px 変わることがあった)。

use std::borrow::Cow;
use std::f64::consts::PI;
use std::time::Instant;

use rayon::prelude::*;

use crate::filters::{gaussian, gaussian_into, median3};
use crate::stats::{f32_key, key_to_f32, median_finite_f32, sigma_clipped_stats};
use crate::{DeepInfo, DeepOpts, DeepRejected, DetectOpts, Image2D, Prep, Source};

/// 偽の極大の数の係数(白色雑音で数えて合わせた値。Python の `_DEEP_FALSE_RATE`)
const DEEP_FALSE_RATE: f64 = 0.045;
/// σ クリップに使う値の数の上限(多いときは等間隔に間引く。Python の `_clipped_stats` の limit)
const CLIP_LIMIT: usize = 500_000;

/// Python の `max(a, b)`: b が a より大きいときだけ b。
#[inline]
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// Python の `min(a, b)`: b が a より小さいときだけ b。
#[inline]
fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Python の float の `x ** 2`(C の pow。定数の 2.0 だと LLVM が x * x に置き換えるので black_box で隠す)。
#[inline]
fn py_sq(x: f64) -> f64 {
    x.powf(std::hint::black_box(2.0))
}

/// Python の `round(x, 2)`(10 進で最も近い値。ちょうど半分は偶数側)。
fn py_round2(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.2}").parse::<f64>().unwrap_or(x)
}

/// 升ごとに求める値。
#[derive(Clone, Copy)]
enum BlockFn {
    /// 有効な画素の中央値(Python の `_median_last`)
    Median,
    /// MAD から出した標準偏差 1.4826 × MAD。有効な画素が升の 1/4 未満なら NaN(`_mad_sigma_last`)
    MadSigma,
}

/// 鍵(f32_key)の並びの中央値。`np.median`(float32)と同じ値: 奇数個なら中央の値、偶数個なら中央 2 個を
/// float32 で足して 2 で割る。空なら NaN。keys の並びは変わる。
fn keys_median(keys: &mut [u32]) -> f32 {
    let n = keys.len();
    if n == 0 {
        return f32::NAN;
    }
    if n % 2 == 1 {
        let (_, v, _) = keys.select_nth_unstable(n / 2);
        0.0f32 + key_to_f32(*v)
    } else {
        let (_, lo, right) = keys.select_nth_unstable(n / 2 - 1);
        let lo = *lo;
        let hi = right.iter().copied().min().unwrap_or(lo);
        let s = (0.0f32 + key_to_f32(lo)) + key_to_f32(hi);
        (s as f64 / 2.0) as f32
    }
}

/// `_block_map` の升の値(3×3 の中央値でならしたもの)と、引き伸ばしの寸法。
struct BlockGrid {
    coarse: Vec<f32>,
    by: usize,
    bx: usize,
    oy: usize,
    ox: usize,
    bsz: usize,
}

/// Python の `_block_map` の前半: bsz 画素の升ごとに `keep == want` の画素の値を出し、値の無い升を
/// 有限な升の中央値で埋め、3×3 の中央値でならす。升が縦横 3 つ未満、または有限な升が 1 つも無ければ None。
/// Python はマスクの画素を NaN にして nanmedian を取るので、ここでも NaN の画素は数えない
/// (MAD の「有効な画素が升の 1/4」は `np.isfinite` の数なので inf も数えない)。
fn block_grid(
    img: &[f32],
    keep: &[bool],
    want: bool,
    (ny, nx): (usize, usize),
    bsz: usize,
    kind: BlockFn,
) -> Option<BlockGrid> {
    if bsz == 0 {
        return None;
    }
    let (by, bx) = (ny / bsz, nx / bsz);
    if by < 3 || bx < 3 {
        return None;
    }
    let (h, w) = (by * bsz, bx * bsz);
    let (oy, ox) = ((ny - h) / 2, (nx - w) / 2);
    let need = bsz * bsz / 4;
    let coarse: Vec<f32> = (0..by * bx)
        .into_par_iter()
        .map_init(
            || Vec::with_capacity(bsz * bsz),
            |keys: &mut Vec<u32>, t| {
                let (i, j) = (t / bx, t % bx);
                keys.clear();
                let mut n_fin = 0usize;
                for yy in oy + i * bsz..oy + (i + 1) * bsz {
                    let base = yy * nx;
                    for xx in ox + j * bsz..ox + (j + 1) * bsz {
                        let v = img[base + xx];
                        if keep[base + xx] == want && !v.is_nan() {
                            keys.push(f32_key(v));
                            n_fin += v.is_finite() as usize;
                        }
                    }
                }
                match kind {
                    BlockFn::Median => keys_median(keys),
                    BlockFn::MadSigma => {
                        if keys.is_empty() {
                            return f32::NAN;
                        }
                        let med = keys_median(keys);
                        for k in keys.iter_mut() {
                            *k = f32_key((key_to_f32(*k) - med).abs());
                        }
                        // inf - inf などで出た NaN は nanmedian と同じく除く
                        keys.retain(|&k| !key_to_f32(k).is_nan());
                        let mad = keys_median(keys);
                        if n_fin >= need { 1.4826f32 * mad } else { f32::NAN }
                    }
                }
            },
        )
        .collect();
    let good: Vec<f32> = coarse.iter().copied().filter(|v| v.is_finite()).collect();
    if good.is_empty() {
        return None;
    }
    let fill = median_finite_f32(&good);
    let coarse: Vec<f32> = coarse.iter().map(|&v| if v.is_finite() { v } else { fill }).collect();
    let coarse = median3(&coarse, by, bx);
    Some(BlockGrid { coarse, by, bx, oy, ox, bsz })
}

impl BlockGrid {
    /// Python の `_block_map` の後半: 升の中心を節点に双一次で引き伸ばす(外側は端の値)。値は
    /// rows = coarse[iy] (1 - wy) + coarse[iy + 1] wy(float64)、out = rows[ix] (1 - wx) + rows[ix + 1] wx を float32 に。
    /// `subtract` なら out の値を img から引き(`img - back`)、そうでなければ img に書く。
    fn expand(&self, img: &mut [f32], (ny, nx): (usize, usize), subtract: bool) {
        let (bx, bsz) = (self.bx, self.bsz);
        let ay = interp_axis(ny, self.oy, bsz, self.by);
        let ax = interp_axis(nx, self.ox, bsz, bx);
        let coarse = &self.coarse;
        img.par_chunks_mut(nx).enumerate().for_each_init(
            || vec![0.0f64; bx],
            |rows, (y, orow)| {
                let (iy, wy) = ay[y];
                let c0 = &coarse[iy * bx..iy * bx + bx];
                let c1 = &coarse[(iy + 1) * bx..(iy + 1) * bx + bx];
                for ((r, &a), &b) in rows.iter_mut().zip(c0).zip(c1) {
                    *r = a as f64 * (1.0 - wy) + b as f64 * wy;
                }
                for (o, &(ix, wx)) in orow.iter_mut().zip(ax.iter()) {
                    let v = (rows[ix] * (1.0 - wx) + rows[ix + 1] * wx) as f32;
                    if subtract {
                        *o -= v;
                    } else {
                        *o = v;
                    }
                }
            },
        );
    }
}

/// `img = img - _block_map(img, keep == want, bsz, _median_last)`。升が足りなければ
/// `img - float(np.median(img[keep == want]))`(Python の `back if back is not None else ...`)。
fn subtract_background(img: &mut [f32], keep: &[bool], want: bool, dims: (usize, usize), bsz: usize) {
    match block_grid(img, keep, want, dims, bsz, BlockFn::Median) {
        Some(g) => g.expand(img, dims, true),
        None => {
            let m = median_finite_f32(&gather(img, keep, want));
            img.par_iter_mut().for_each(|a| *a -= m);
        }
    }
}

/// `_block_map` の 1 軸ぶんの補間の表 (下の節点の番号, 重み)。
/// f = clip((k - (o + 0.5 bsz - 0.5)) / bsz, 0, nb - 1)、i = min(int(f), nb - 2)、重み f - i。
fn interp_axis(n: usize, o: usize, bsz: usize, nb: usize) -> Vec<(usize, f64)> {
    let c = (o as f64 + 0.5 * bsz as f64) - 0.5;
    let hi = (nb - 1) as f64;
    (0..n)
        .map(|k| {
            let mut f = (k as f64 - c) / bsz as f64;
            if f < 0.0 {
                f = 0.0;
            }
            if f > hi {
                f = hi;
            }
            let i = (f as usize).min(nb - 2);
            (i, f - i as f64)
        })
        .collect()
}

/// 場所ごとの雑音の地図: 一辺 nbox の升ごとの MAD の σ(block_grid の MadSigma)を引き伸ばし、0.7 ns を下限にする
/// (星で埋まった升で雑音を低く見積もらない)。升が足りなければ全面 ns。
/// 升の地図があるときは expand が全画素を上書きするので、確保は 0 埋め(OS のゼロページで、直列に埋めない)でよい。
/// 前の版は全面を ns で埋めてから上書きしていたが、上書きされる値は同じ。
fn noise_map(sm: &[f32], bad: &[bool], dims: (usize, usize), nbox: usize, ns: f64) -> Vec<f32> {
    let mut nmap = vec![0.0f32; dims.0 * dims.1];
    match block_grid(sm, bad, false, dims, nbox, BlockFn::MadSigma) {
        Some(g) => g.expand(&mut nmap, dims, false),
        None => {
            let v = ns as f32;
            nmap.par_iter_mut().for_each(|a| *a = v);
        }
    }
    let floor = (0.7 * ns) as f32;
    nmap.par_iter_mut().for_each(|v| {
        if v.is_nan() || floor.is_nan() {
            *v = f32::NAN;
        } else if floor > *v {
            *v = floor;
        }
    });
    nmap
}

/// 並列の処理単位(画素数)
const CHUNK: usize = 1 << 16;

/// Python の `_clipped_stats(img[keep == want])`: σ クリップ(3σ・5 回)の (中央値, 標準偏差)。
/// 集めた値が 50 万個を超えるときは `ceil(個数 / 50万)` おきに間引く(Python の `values[::step]`)。
/// 全部を集めてから間引くと 2400 万画素で 96 MB を書くので、塊ごとの個数から順位を出し、残すものだけを集める。
fn clipped_masked(img: &[f32], keep: &[bool], want: bool) -> (f64, f64) {
    let counts: Vec<usize> = keep.par_chunks(CHUNK).map(|c| c.iter().filter(|&&k| k == want).count()).collect();
    let total: usize = counts.iter().sum();
    let step = if total > CLIP_LIMIT { total.div_ceil(CLIP_LIMIT) } else { 1 };
    let mut starts = Vec::with_capacity(counts.len());
    let mut acc = 0usize;
    for &c in &counts {
        starts.push(acc);
        acc += c;
    }
    let parts: Vec<Vec<f32>> = img
        .par_chunks(CHUNK)
        .zip(keep.par_chunks(CHUNK))
        .zip(starts.par_iter())
        .map(|((ic, kc), &st)| {
            let mut out = Vec::new();
            // 次に残すまでに飛ばす個数(順位が step の倍数のものを残す)
            let mut skip = (step - st % step) % step;
            for (&v, &k) in ic.iter().zip(kc) {
                if k == want {
                    if skip == 0 {
                        out.push(v);
                        skip = step - 1;
                    } else {
                        skip -= 1;
                    }
                }
            }
            out
        })
        .collect();
    let st = sigma_clipped_stats(&parts.concat());
    (st.median as f64, st.std as f64)
}

/// 条件を満たす画素の値を、行優先の順に集める。
fn gather(img: &[f32], keep: &[bool], want: bool) -> Vec<f32> {
    img.par_iter().zip(keep.par_iter()).filter(|(_, k)| **k == want).map(|(v, _)| *v).collect()
}

/// (2h+1)² の窓の中の (dy, dx)(中心を除く)を、中心から近い順(チェビシェフ距離、同じ距離は上下左右を先に)に並べる。
fn window_offsets(h: usize) -> Vec<(isize, isize)> {
    let h = h as isize;
    let mut v: Vec<(isize, isize)> =
        (-h..=h).flat_map(|dy| (-h..=h).map(move |dx| (dy, dx))).filter(|&(dy, dx)| dy != 0 || dx != 0).collect();
    v.sort_by_key(|&(dy, dx)| (dy.abs().max(dx.abs()), dy.abs() + dx.abs()));
    v
}

/// `img[y, x] == ndimage.maximum_filter(img, size=2h+1)[y, x]`(mode="reflect")を 1 点だけ調べる。
/// 窓の中に img[y, x] より大きい値が無ければ極大。reflect で延ばした値は画像の内側に縮めた窓の中の値なので、
/// 窓を画像の内側に縮めて見る。近い画素から見るので、極大でない点はたいてい数回で抜ける。
/// 画像全体の最大値フィルターを作らずに済む(極大を探すのは雑音の数倍を超える画素だけ)。NaN を含まない画像に使う。
#[inline]
fn is_local_max(img: &[f32], ny: usize, nx: usize, y: usize, x: usize, offs: &[(isize, isize)]) -> bool {
    let v = img[y * nx + x];
    for &(dy, dx) in offs {
        let (yy, xx) = (y as isize + dy, x as isize + dx);
        if yy < 0 || xx < 0 || yy >= ny as isize || xx >= nx as isize {
            continue;
        }
        if img[yy as usize * nx + xx as usize] > v {
            return false;
        }
    }
    true
}

/// 行 y0..y1・列 x0..x1 で、`pass(画素の番号, 値)` を満たす (2h+1)² の窓の極大を、行優先の順に集める。
fn local_peaks<P: Fn(usize, f32) -> bool + Sync>(
    img: &[f32],
    (ny, nx): (usize, usize),
    h: usize,
    (y0, y1): (usize, usize),
    (x0, x1): (usize, usize),
    pass: P,
) -> Vec<(usize, usize, f32)> {
    let offs = window_offsets(h);
    (y0..y1.max(y0))
        .into_par_iter()
        .flat_map_iter(|y| {
            let base = y * nx;
            let (offs, pass) = (&offs, &pass);
            (x0..x1.max(x0)).filter_map(move |x| {
                let v = img[base + x];
                (pass(base + x, v) && is_local_max(img, ny, nx, y, x, offs)).then_some((y, x, v))
            })
        })
        .collect()
}

/// Python の `_sharpness`: 中心の値 / 上下左右の平均(平均が 0 以下なら inf)。
#[inline]
fn sharpness(work: &[f32], nx: usize, y: usize, x: usize) -> f64 {
    let n4 = (work[(y - 1) * nx + x] as f64
        + work[(y + 1) * nx + x] as f64
        + work[y * nx + x - 1] as f64
        + work[y * nx + x + 1] as f64)
        / 4.0;
    if n4 > 0.0 { work[y * nx + x] as f64 / n4 } else { f64::INFINITY }
}

/// `np.percentile(values, q)`(既定の linear)。virtual = (n - 1) q / 100 の前後を numpy の `_lerp` と同じ式で補間する。
fn np_percentile(values: &[f64], q: f64) -> f64 {
    let n = values.len();
    if n == 0 {
        return f64::NAN;
    }
    if values.iter().any(|v| v.is_nan()) {
        return f64::NAN; // numpy も NaN を返す
    }
    let mut a = values.to_vec();
    a.sort_by(|x, y| x.total_cmp(y));
    let quant = q / 100.0;
    let virt = (n - 1) as f64 * quant;
    let (prev, next, prev_f) = if virt >= (n - 1) as f64 {
        (n - 1, n - 1, -1.0) // numpy は添字 -1(最後の値)にし、gamma = virt - (-1)
    } else {
        let p = virt.floor();
        (p as usize, p as usize + 1, p)
    };
    let gamma = virt - prev_f;
    let (lo, hi) = (a[prev], a[next]);
    let diff = hi - lo;
    if gamma >= 0.5 { hi - diff * (1.0 - gamma) } else { lo + diff * gamma }
}

/// Python の `_psf_sigma`: 明るい星(σ=1 でぼかした画像で雑音の 10 倍以上)の半値の面積から星像の σ を測る。
/// 戻り値は (σ か None, 使った星の数, 鋭さの上限)。
fn psf_sigma(
    sm1: &[f32],
    work: &[f32],
    mask: &[bool],
    ny: usize,
    nx: usize,
    ns1: f64,
    sigma: f64,
) -> (Option<f64>, usize, f64) {
    const MAX_STARS: usize = 40;
    const MIN_SNR: f64 = 10.0;
    let thr = (MIN_SNR * ns1) as f32;
    // sm1 == maximum_filter(sm1, size=7) で、雑音の 10 倍を超える有効な画素。縁の 2 画素は除く
    let mut pk =
        local_peaks(sm1, (ny, nx), 3, (2, ny.saturating_sub(2)), (2, nx.saturating_sub(2)), |i, v| v > thr && !mask[i]);
    if pk.len() < 3 {
        return (None, 0, f64::INFINITY);
    }
    // 明るい順(同じ値は元の順。基準実装も安定な並べ替え。冒頭の注記)
    pk.sort_by(|a, b| b.2.total_cmp(&a.2));
    pk.truncate(MAX_STARS);
    let thr_sharp = (10.0 * sigma) as f32;
    let mut found: Vec<(f64, Option<f64>)> = Vec::new();
    let mut seen = Vec::new();
    let mut stack = Vec::new();
    for &(y, x, vf) in &pk {
        let v = vf as f64;
        let sharp = if work[y * nx + x] > thr_sharp { Some(sharpness(work, nx, y, x)) } else { None };
        let half = (0.5 * v) as f32;
        let top = (0.98 * v) as f32;
        for r in [8usize, 16, 32] {
            // 窓 [y-r, y+r] × [x-r, x+r](画像の内側)で、中心を含む「半値より上」の 4 連結の領域
            let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(ny));
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(nx));
            let (hh, ww) = (y1 - y0, x1 - x0);
            if sm1[y * nx + x] <= half {
                break;
            }
            seen.clear();
            seen.resize(hh * ww, false);
            stack.clear();
            stack.push((y - y0, x - x0));
            seen[(y - y0) * ww + (x - x0)] = true;
            let (mut cnt, mut cnt_top) = (0usize, 0usize);
            let (mut rmin, mut rmax, mut cmin, mut cmax) = (usize::MAX, 0usize, usize::MAX, 0usize);
            while let Some((ry, rx)) = stack.pop() {
                let val = sm1[(y0 + ry) * nx + x0 + rx];
                cnt += 1;
                if val > top {
                    cnt_top += 1;
                }
                rmin = rmin.min(ry);
                rmax = rmax.max(ry);
                cmin = cmin.min(rx);
                cmax = cmax.max(rx);
                let mut push = |py: usize, px: usize| {
                    let k = py * ww + px;
                    if !seen[k] && sm1[(y0 + py) * nx + x0 + px] > half {
                        seen[k] = true;
                        stack.push((py, px));
                    }
                };
                if ry > 0 {
                    push(ry - 1, rx);
                }
                if ry + 1 < hh {
                    push(ry + 1, rx);
                }
                if rx > 0 {
                    push(ry, rx - 1);
                }
                if rx + 1 < ww {
                    push(ry, rx + 1);
                }
            }
            let touches = rmin == 0 || cmin == 0 || rmax == hh - 1 || cmax == ww - 1;
            if touches {
                if r < 32 {
                    continue;
                }
                break; // 65 画素の窓より大きい = 星ではない
            }
            if cnt_top > 4 {
                break; // 頂が平ら = 飽和
            }
            let fwhm = 2.0 * (cnt as f64 / PI).sqrt();
            found.push((py_max(py_sq(fwhm / 2.3548) - 1.0, 0.0).sqrt(), sharp));
            break;
        }
    }
    let sharps: Vec<f64> = found.iter().filter_map(|&(_, s)| s).collect();
    let limit = if sharps.len() >= 3 { py_max(2.5, 2.0 * np_percentile(&sharps, 25.0)) } else { f64::INFINITY };
    let widths: Vec<f64> =
        found.iter().filter(|&&(w, s)| s.is_none_or(|s| s <= limit) && w > 0.3).map(|&(w, _)| w).collect();
    if widths.len() < 3 {
        return (None, widths.len(), limit);
    }
    (Some(np_percentile(&widths, 40.0)), widths.len(), limit)
}

/// `ndimage.binary_dilation(invalid, iterations=n)`(十字の構造要素、外側は偽)= 無効な画素から
/// 市街地距離 n 以内。各行の「行の中で最も近い無効な画素までの距離」g を求め、列の方向に
/// d(y) = min(g(yy) + |yy - y|) を前向き・後ろ向きの 2 回の掃引で出す。n より遠いものは要らないので、
/// 行の帯ごとに上下 n 行だけ広げて並列に掃引する。距離は 255 で頭打ちの u8(n ≤ 17 なので足りる)。
fn dilate_l1(invalid: &[bool], ny: usize, nx: usize, n: usize) -> Vec<bool> {
    // 初めの値は使わない(各行の最初のループが全要素を書く)ので、0 埋めの確保にする(直列に埋めない)
    let mut g = vec![0u8; ny * nx];
    g.par_chunks_mut(nx).enumerate().for_each(|(y, row)| {
        let inv = &invalid[y * nx..y * nx + nx];
        let mut d = u8::MAX;
        for (o, &b) in row.iter_mut().zip(inv) {
            d = if b { 0 } else { d.saturating_add(1) };
            *o = d;
        }
        d = u8::MAX;
        for (o, &b) in row.iter_mut().zip(inv).rev() {
            d = if b { 0 } else { d.saturating_add(1) };
            *o = (*o).min(d);
        }
    });
    const BAND: usize = 32;
    // 255 以上は「遠い」としか分からないので、n が 254 を超えるなら全部を遠いとみなす前に 254 で止める
    let n8 = n.min(254) as u8;
    let mut out = vec![false; ny * nx];
    out.par_chunks_mut(nx * BAND).enumerate().for_each(|(bi, ochunk)| {
        let a = bi * BAND;
        let b = (a + BAND).min(ny);
        let (lo, hi) = (a.saturating_sub(n), (b + n).min(ny));
        let mut f: Vec<u8> = g[lo * nx..hi * nx].to_vec();
        for y in 1..hi - lo {
            let (prev, cur) = f.split_at_mut(y * nx);
            for (c, &p) in cur[..nx].iter_mut().zip(&prev[(y - 1) * nx..]) {
                *c = (*c).min(p.saturating_add(1));
            }
        }
        for y in (0..hi - lo - 1).rev() {
            let (cur, next) = f.split_at_mut((y + 1) * nx);
            for (c, &q) in cur[y * nx..].iter_mut().zip(&next[..nx]) {
                *c = (*c).min(q.saturating_add(1));
            }
        }
        for (o, &d) in ochunk.iter_mut().zip(&f[(a - lo) * nx..(b - lo) * nx]) {
            *o = d <= n8;
        }
    });
    out
}

/// Python の `_false_peaks`: 雑音だけの画像を σ=s でぼかしたとき、雑音の u 倍を超える局所の極大の数の見込み。
#[inline]
fn false_peaks(u: f64, area: f64, s: f64) -> f64 {
    DEEP_FALSE_RATE * area / (s * s) * u * (-0.5 * u * u).exp()
}

/// Python の `_peak_offset`: 3 点(左・中央・右)の極大の位置(-0.5〜0.5)。全部正なら対数の放物線、
/// そうでなければ普通の放物線。
fn peak_offset(a: f64, b: f64, c: f64) -> f64 {
    if a > 0.0 && b > 0.0 && c > 0.0 {
        let (la, lb, lc) = (a.ln(), b.ln(), c.ln());
        let den = la - 2.0 * lb + lc;
        if den < 0.0 {
            return py_min(py_max(0.5 * (la - lc) / den, -0.5), 0.5);
        }
    }
    let den = a - 2.0 * b + c;
    if den < 0.0 {
        return py_min(py_max(0.5 * (a - c) / den, -0.5), 0.5);
    }
    0.0
}

/// `np.linspace(a, b, m)` の k 番目(最後は b そのもの。間隔が 0 なら a)。
#[inline]
fn linspace_at(a: f64, b: f64, m: usize, k: usize) -> f64 {
    if k + 1 == m {
        return b;
    }
    let div = (m - 1) as f64;
    let delta = b - a;
    let step = delta / div;
    if step == 0.0 { (k as f64 / div) * delta + a } else { k as f64 * step + a }
}

/// (y0, x0) から (y1, x1) までの線を m 点で `ndimage.map_coordinates(sm, ..., order=1)` したときの最小値。
/// 双一次の重みは scipy と同じ (1 - t, 1 - (1 - t))、足す順は (下, 左) (下, 右) (上, 左) (上, 右)。
fn line_min(sm: &[f32], ny: usize, nx: usize, p0: (usize, usize), p1: (usize, usize), m: usize) -> f32 {
    let mut lo = f32::INFINITY;
    for k in 0..m {
        let cy = linspace_at(p0.0 as f64, p1.0 as f64, m, k);
        let cx = linspace_at(p0.1 as f64, p1.1 as f64, m, k);
        let (fy, fx) = (cy.floor(), cx.floor());
        let (wy0, wx0) = (1.0 - (cy - fy), 1.0 - (cx - fx));
        let (wy1, wx1) = (1.0 - wy0, 1.0 - wx0);
        let iy0 = (fy.max(0.0) as usize).min(ny - 1);
        let ix0 = (fx.max(0.0) as usize).min(nx - 1);
        let (iy1, ix1) = ((iy0 + 1).min(ny - 1), (ix0 + 1).min(nx - 1));
        let mut t = 0.0f64;
        t += sm[iy0 * nx + ix0] as f64 * wy0 * wx0;
        t += sm[iy0 * nx + ix1] as f64 * wy0 * wx1;
        t += sm[iy1 * nx + ix0] as f64 * wy1 * wx0;
        t += sm[iy1 * nx + ix1] as f64 * wy1 * wx1;
        let v = t as f32;
        if v < lo || v.is_nan() {
            lo = v;
        }
    }
    lo
}

/// 深い検出の本体(Python の `detect_sources_deep(sub, mask, sigma, max_sources, target_false, min_u, info)`)。
/// 戻り値は (明るい順の星, しきい値 u, 測った値)。星が探せない大きさ・マスクのときは ([], None, None)。
pub fn detect_deep_raw(
    sub: &Image2D,
    mask: &[bool],
    sigma: f64,
    opts: &DeepOpts,
) -> (Vec<Source>, Option<f64>, Option<DeepInfo>) {
    let t_start = Instant::now();
    let (ny, nx) = (sub.height, sub.width);
    let npx = ny * nx;
    if npx == 0 || sub.data.len() != npx || mask.len() != npx {
        return (Vec::new(), None, None);
    }
    // 有効な画素 = mask が偽(Python の valid = ~mask)。全面の配列を増やさないよう mask をそのまま使う
    let n_valid = mask.par_iter().filter(|&&m| !m).count();
    if ny < 16 || nx < 16 || n_valid < 256 {
        return (Vec::new(), None, None);
    }
    let dims = (ny, nx);
    // マスクの画素を 0 にした画像。マスクが空なら sub そのもの(写さない。`if m {0} else {s}` は m が全部偽なら s のビットのまま)
    let work: Cow<[f32]> = if n_valid == npx {
        Cow::Borrowed(&sub.data)
    } else {
        Cow::Owned(sub.data.par_iter().zip(mask.par_iter()).map(|(&s, &m)| if m { 0.0 } else { s }).collect())
    };

    // 1. 星像の幅
    let mut sm1 = gaussian(&work, ny, nx, 1.0);
    subtract_background(&mut sm1, mask, false, dims, 32);
    let mut ns1 = clipped_masked(&sm1, mask, false).1;
    if !ns1.is_finite() || ns1 <= 0.0 {
        ns1 = 0.28 * sigma;
    }
    let (s_psf, n_psf, sharp_max) = psf_sigma(&sm1, &work, mask, ny, nx, ns1, sigma);
    // Python の `s_psf if s_psf else ...`(0 は偽)
    let s_psf_true = s_psf.filter(|&p| p != 0.0);
    let s = py_min(py_max(s_psf_true.map_or(1.5, |p| 1.25 * p), 1.0), 8.0);

    // 2. ぼかす・背景を引く・雑音を測る
    // 使い終えた sm1(同じ ny×nx)を書き先に回す。ガウスは全画素を書くので、前の中身は値に効かない(gaussian_into)
    let mut sm = sm1;
    gaussian_into(&work, ny, nx, s, &mut sm);
    let bsz = ((8.0 * s).round_ties_even() as usize).max(16);
    subtract_background(&mut sm, mask, false, dims, bsz);
    // マスクの縁はぼかしでマスクの 0 が混ざって沈むので、ぼかしの幅だけ広げて除く。
    // マスクが空なら mask そのもの(全部偽で、前の版の `vec![false; npx]` と同じ中身)
    let bad: Cow<[bool]> = if n_valid == npx {
        Cow::Borrowed(mask)
    } else {
        Cow::Owned(dilate_l1(mask, ny, nx, (2.0 * s).ceil() as usize + 1))
    };
    let n_good = if n_valid == npx { npx } else { bad.par_iter().filter(|&&b| !b).count() };
    if n_good < 256 {
        return (Vec::new(), None, None);
    }
    let (med, mut ns) = clipped_masked(&sm, &bad, false);
    if !ns.is_finite() || ns <= 0.0 {
        ns = py_max(ns1, 1e-12);
    }
    let med32 = med as f32;
    sm.par_iter_mut().for_each(|a| *a -= med32);
    let nbox = ((16.0 * s).round_ties_even() as usize).max(32);
    let nmap = noise_map(&sm, &bad, dims, nbox, ns);

    // 3. しきい値と候補
    let area = n_good as f64;
    let mut u = opts.min_u;
    let mut steps = 0u32;
    // 既定の条件なら 80 回以内(4.0 → 8.0)。上限は極端な min_u で回り続けないため
    while u < 8.0 && false_peaks(u, area, s) > opts.target_false && steps < 100_000 {
        u += 0.05;
        steps += 1;
    }
    let u = py_round2(u);
    let h = s.ceil() as usize;
    let uf = u as f32;
    let edge = h.max(2);
    // sm == maximum_filter(sm, size=2h+1) で、その場所の雑音の u 倍を超え、マスクの縁でない画素
    let mut cand =
        local_peaks(&sm, (ny, nx), h, (edge, ny.saturating_sub(edge)), (edge, nx.saturating_sub(edge)), |i, v| {
            v > uf * nmap[i] && !bad[i]
        });
    // 明るい順(同じ値は元の行優先の順。基準実装も安定な並べ替え。冒頭の注記)
    cand.sort_by(|a, b| b.2.total_cmp(&a.2));
    cand.truncate(4000.max(opts.max_sources.saturating_mul(10)));
    let n = cand.len();
    let ys: Vec<usize> = cand.iter().map(|c| c.0).collect();
    let xs: Vec<usize> = cand.iter().map(|c| c.1).collect();
    let vals: Vec<f64> = cand.iter().map(|c| c.2 as f64).collect();
    drop(cand);

    // 4. 明るい順に選ぶ
    let mut kept_idx: Vec<usize> = Vec::new();
    let mut rejected = DeepRejected::default();
    // 局所の背景を測る輪: ぼかした星像の σ (√(σ_psf² + s²)) の 3 倍の半径
    let sp = s_psf_true.unwrap_or(s / 1.25);
    let r_ring = 3.0 * (sp * sp + s * s).sqrt();
    let nang = ((2.0 * PI * r_ring) as usize).max(16);
    let step = (2.0 * PI) / nang as f64;
    let ring: Vec<(isize, isize)> = (0..nang)
        .map(|k| {
            let a = k as f64 * step + 0.0;
            ((r_ring * a.sin()).round_ties_even() as isize, (r_ring * a.cos()).round_ties_even() as isize)
        })
        .collect();
    let reach = |v: f64| -> f64 { py_min(py_max(4.0 * s, 2.0 * s * py_max(v / (u * ns), 1.0).sqrt()), 40.0 * s) };
    let thr_sharp = (10.0 * sigma) as f32;
    let mut ring_vals: Vec<f32> = Vec::with_capacity(nang);
    if n > 0 {
        let r_max = reach(vals[0]);
        let r2 = r_max * r_max; // scipy の cKDTree は距離の 2 乗を r² と比べる
        // 残した星を一辺 cell 画素の升目に入れ、半径 r_max の中にかかる升だけを見る。どの星で肩と分かっても
        // 結果は同じなので、見る順は Python(cKDTree)と違ってよい
        let rr = (r_max.ceil() as usize).min(ny.max(nx));
        let cell = rr.max(32);
        let (gw, gh) = (nx / cell + 1, ny / cell + 1);
        let mut grid: Vec<Vec<u32>> = vec![Vec::new(); gw * gh];
        for i in 0..n {
            let (y, x, v) = (ys[i], xs[i], vals[i]);
            if work[y * nx + x] > thr_sharp && sharpness(&work, nx, y, x) > sharp_max {
                rejected.sharp += 1;
                continue;
            }
            let noise = nmap[y * nx + x] as f64;
            let mut shoulder = false;
            let (gx0, gx1) = (x.saturating_sub(rr) / cell, (x + rr).min(nx - 1) / cell);
            let (gy0, gy1) = (y.saturating_sub(rr) / cell, (y + rr).min(ny - 1) / cell);
            'cells: for gy in gy0..=gy1 {
                for gx in gx0..=gx1 {
                    for &j in &grid[gy * gw + gx] {
                        let j = j as usize;
                        let dx = x as i64 - xs[j] as i64;
                        let dy = y as i64 - ys[j] as i64;
                        let d2 = (dx * dx + dy * dy) as f64;
                        if d2 > r2 {
                            continue;
                        }
                        let d = d2.sqrt(); // 整数の座標なので math.hypot と同じ値
                        if d > reach(vals[j]) {
                            continue;
                        }
                        let m = (2.0 * d).ceil() as usize + 2;
                        if v - (line_min(&sm, ny, nx, (y, x), (ys[j], xs[j]), m) as f64) < 3.0 * noise {
                            shoulder = true;
                            break 'cells;
                        }
                    }
                }
            }
            if shoulder {
                rejected.saddle += 1;
                continue;
            }
            let c = |dy: usize, dx: usize| sm[(y + dy - 1) * nx + (x + dx - 1)] as f64;
            let dxx = c(1, 2) - 2.0 * c(1, 1) + c(1, 0);
            let dyy = c(2, 1) - 2.0 * c(1, 1) + c(0, 1);
            let dxy = (c(2, 2) - c(2, 0) - c(0, 2) + c(0, 0)) / 4.0;
            let (tr, det) = (dxx + dyy, dxx * dyy - dxy * dxy);
            let mut elong = f64::INFINITY;
            if det > 0.0 && tr < 0.0 {
                let root = py_max(tr * tr / 4.0 - det, 0.0).sqrt();
                let (l1, l2) = (tr / 2.0 - root, tr / 2.0 + root); // どちらも負。l1 が急なほう
                if l2 < 0.0 {
                    elong = (l1 / l2).sqrt();
                }
            }
            if elong > 4.0 {
                rejected.elong += 1;
                continue;
            }
            ring_vals.clear();
            for &(dy, dx) in &ring {
                let ry = (y as isize + dy).clamp(0, ny as isize - 1) as usize;
                let rx = (x as isize + dx).clamp(0, nx as isize - 1) as usize;
                if !bad[ry * nx + rx] {
                    ring_vals.push(sm[ry * nx + rx]); // マスクの側は沈んでいるので使わない
                }
            }
            if ring_vals.len() >= 8 && v - (median_finite_f32(&ring_vals) as f64) < 3.0 * noise {
                rejected.ring += 1;
                continue;
            }
            grid[(y / cell) * gw + x / cell].push(i as u32);
            kept_idx.push(i);
        }
    }

    // 5. 位置と明るさ
    let fwhm = 2.3548 * sp;
    let area_px = (PI * fwhm * fwhm / 4.0).round_ties_even();
    let mut out: Vec<Source> = kept_idx
        .iter()
        .map(|&i| {
            let (y, x, v) = (ys[i], xs[i], vals[i]);
            let c = |dy: usize, dx: usize| sm[(y + dy - 1) * nx + (x + dx - 1)] as f64;
            let ox = peak_offset(c(1, 0), c(1, 1), c(1, 2));
            let oy = peak_offset(c(0, 1), c(1, 1), c(2, 1));
            Source {
                x: x as f64 + ox,
                y: y as f64 + oy,
                flux: v * 2.0 * PI * (sp * sp + s * s),
                peak: sub.data[y * nx + x] as f64,
                area: if area_px.is_finite() && area_px > 0.0 { area_px as usize } else { 0 },
                elong: 1.0,
                fwhm,
                snr: v / nmap[y * nx + x] as f64,
            }
        })
        .collect();
    // 明るい順(同じ光量は元の順を保つ = Python の安定な sort)
    out.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));
    let info = DeepInfo {
        s_psf,
        n_psf,
        s,
        u,
        noise: ns,
        candidates: n,
        kept: out.len(),
        rejected,
        sharp_max,
        seconds: t_start.elapsed().as_secs_f64(),
    };
    out.truncate(opts.max_sources);
    (out, Some(u), Some(info))
}

/// 深い検出(`detect_sources_deep(prep.sub, prep.mask, prep.sigma, ...)`)。戻り値は (星, しきい値 u, 測った値)。
pub fn detect_deep(prep: &Prep, opts: &DeepOpts) -> (Vec<Source>, Option<f64>, Option<DeepInfo>) {
    detect_deep_raw(&prep.sub, &prep.mask, prep.sigma, opts)
}

/// 以前の形の呼び出し(`detect_sources_deep(prep, &DetectOpts::deep())`)。使うのは `opts.max_sources` だけで、
/// ほかは `DeepOpts` の既定値。戻り値は (星, しきい値 u)。
pub fn detect_sources_deep(prep: &Prep, opts: &DetectOpts) -> (Vec<Source>, Option<f64>) {
    let (found, u, _) = detect_deep(prep, &DeepOpts { max_sources: opts.max_sources, ..DeepOpts::default() });
    (found, u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_helpers() {
        assert_eq!(py_round2(4.050000000000001), 4.05);
        assert_eq!(py_round2(4.3999999999999995), 4.4);
        assert_eq!(py_max(2.5, f64::NAN), 2.5);
        assert!(py_min(f64::NAN, 0.5).is_nan());
        // np.percentile([1, 2, 3, 4], 25) = 1.75、40 → 2.2、端と inf
        assert_eq!(np_percentile(&[4.0, 1.0, 3.0, 2.0], 25.0), 1.75);
        assert_eq!(np_percentile(&[4.0, 1.0, 3.0, 2.0], 40.0), 2.2);
        assert_eq!(np_percentile(&[5.0], 40.0), 5.0);
        // numpy の _lerp は t ≥ 0.5 で b - (b - a)(1 - t) を使うので、inf を挟むと NaN(numpy 2.2.6 で確かめた)
        assert!(np_percentile(&[1.0, f64::INFINITY, f64::INFINITY], 25.0).is_nan());
        assert_eq!(np_percentile(&[1.0, 2.0, f64::INFINITY, f64::INFINITY], 25.0), 1.75);
        // その NaN は Python の max(2.5, 2.0 * nan) で 2.5 になる
        assert_eq!(py_max(2.5, 2.0 * f64::NAN), 2.5);
    }

    #[test]
    fn dilation_is_l1_ball() {
        let (ny, nx) = (9, 11);
        let mut inv = vec![false; ny * nx];
        inv[4 * nx + 5] = true;
        inv[0] = true;
        let d = dilate_l1(&inv, ny, nx, 2);
        for y in 0..ny {
            for x in 0..nx {
                let near = (y.abs_diff(4) + x.abs_diff(5) <= 2) || (y + x <= 2);
                assert_eq!(d[y * nx + x], near, "({y}, {x})");
            }
        }
    }

    /// 決まった擬似乱数(xorshift)
    fn rand_img(ny: usize, nx: usize, seed: u64, levels: u32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..ny * nx)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                // 値の種類を減らして同じ値(平らな頂)も作る
                ((s >> 11) % levels as u64) as f32 - 3.0
            })
            .collect()
    }

    #[test]
    fn sparse_local_max_matches_full_filter() {
        // 端の近く・同じ値の頂・窓が画像より大きい場合も含めて、全面の maximum_filter と同じ点を選ぶ
        for (ny, nx, h, levels, seed) in
            [(37, 53, 1, 7, 1u64), (40, 41, 2, 5, 2), (31, 64, 3, 50, 3), (25, 30, 8, 9, 4), (9, 7, 8, 4, 5)]
        {
            let img = rand_img(ny, nx, seed, levels);
            let mx = crate::filters::maximum_filter(&img, ny, nx, h);
            let want: Vec<(usize, usize, f32)> = (0..ny)
                .flat_map(|y| (0..nx).map(move |x| (y, x)))
                .filter(|&(y, x)| img[y * nx + x] == mx[y * nx + x] && img[y * nx + x] > 0.0)
                .map(|(y, x)| (y, x, img[y * nx + x]))
                .collect();
            let got = local_peaks(&img, (ny, nx), h, (0, ny), (0, nx), |_, v| v > 0.0);
            assert_eq!(got, want, "{ny}x{nx} h={h}");
        }
    }

    #[test]
    fn dilation_bands_match_brute_force() {
        // 帯(32 行)の境目をまたぐ大きさで、総当たりの市街地距離と同じ
        for (ny, nx, n, seed) in [(100, 37, 5, 7u64), (70, 20, 17, 8), (33, 9, 1, 9), (5, 80, 3, 10)] {
            let inv: Vec<bool> = rand_img(ny, nx, seed, 97).iter().map(|&v| v > 91.0).collect();
            let d = dilate_l1(&inv, ny, nx, n);
            for y in 0..ny {
                for x in 0..nx {
                    let near =
                        (0..ny).any(|yy| (0..nx).any(|xx| inv[yy * nx + xx] && yy.abs_diff(y) + xx.abs_diff(x) <= n));
                    assert_eq!(d[y * nx + x], near, "{ny}x{nx} n={n} ({y}, {x})");
                }
            }
        }
    }

    #[test]
    fn noise_map_matches_reference() {
        // 2026-10-24 までの雑音の地図(全面を ns で埋めてから升の地図で上書きし、下限をかける)とビット単位で同じ。
        // 升の地図が作れる大きさ・作れない大きさ(升が 3 つ未満)・マスク入り・ns が NaN のとき
        fn noise_map_ref(sm: &[f32], bad: &[bool], dims: (usize, usize), nbox: usize, ns: f64) -> Vec<f32> {
            let mut nmap = vec![ns as f32; dims.0 * dims.1];
            if let Some(g) = block_grid(sm, bad, false, dims, nbox, BlockFn::MadSigma) {
                g.expand(&mut nmap, dims, false);
            }
            let floor = (0.7 * ns) as f32;
            nmap.iter_mut().for_each(|v| {
                if v.is_nan() || floor.is_nan() {
                    *v = f32::NAN;
                } else if floor > *v {
                    *v = floor;
                }
            });
            nmap
        }
        let mut n_grid = 0;
        for (ny, nx, nbox, seed) in
            [(100usize, 130usize, 32usize, 1u64), (64, 64, 32, 2), (200, 90, 20, 3), (50, 50, 32, 4)]
        {
            let sm: Vec<f32> = rand_img(ny, nx, seed, 1000).iter().map(|&v| v * 0.01).collect();
            for masked in [false, true] {
                let bad: Vec<bool> = (0..ny * nx).map(|i| masked && (i % 13 == 0 || i < 3 * nx)).collect();
                for ns in [1.0f64, 3.7, 1e-12, f64::NAN] {
                    let want = noise_map_ref(&sm, &bad, (ny, nx), nbox, ns);
                    let got = noise_map(&sm, &bad, (ny, nx), nbox, ns);
                    assert!(got.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()), "{ny}x{nx} ns={ns}");
                }
                n_grid += block_grid(&sm, &bad, false, (ny, nx), nbox, BlockFn::MadSigma).is_some() as usize;
            }
        }
        assert!((2..8).contains(&n_grid), "升の地図あり {n_grid} 件");
    }

    #[test]
    fn strided_clip_matches_python_slicing() {
        // 50 万個を超えると ceil(個数 / 50 万) おきに間引く(values[::step])。全部を集めて間引いたものと同じ
        let (ny, nx) = (1100, 1000);
        let img = rand_img(ny, nx, 11, 1000);
        let keep: Vec<bool> = rand_img(ny, nx, 12, 10).iter().map(|&v| v > -2.0).collect();
        let all = gather(&img, &keep, true);
        assert!(all.len() > CLIP_LIMIT);
        let step = (all.len() as f64 / CLIP_LIMIT as f64).ceil() as usize;
        let sub: Vec<f32> = all.iter().step_by(step).copied().collect();
        let st = sigma_clipped_stats(&sub);
        assert_eq!(clipped_masked(&img, &keep, true), (st.median as f64, st.std as f64));
    }

    #[test]
    fn broken_inputs_do_not_panic() {
        let opts = DeepOpts::default();
        let (ny, nx) = (40, 50);
        let mut img = Image2D { width: nx, height: ny, data: rand_img(ny, nx, 3, 100) };
        img.data[5] = f32::NAN;
        img.data[77] = f32::INFINITY;
        img.data[300] = f32::NEG_INFINITY;
        let mask = vec![false; ny * nx];
        let _ = detect_deep_raw(&img, &mask, 1.0, &opts);
        let _ = detect_deep_raw(&img, &mask, f64::NAN, &opts);
        let _ = detect_deep_raw(&img, &mask, 0.0, &opts);
        // 極端な条件
        for o in [
            DeepOpts { max_sources: usize::MAX, target_false: f64::NEG_INFINITY, min_u: -1e300 },
            DeepOpts { max_sources: 0, target_false: f64::NAN, min_u: f64::NAN },
            DeepOpts { max_sources: 1, target_false: 0.0, min_u: f64::INFINITY },
        ] {
            let _ = detect_deep_raw(&img, &mask, 1.0, &o);
        }
        // ほとんどマスク・全部マスク・大きさの食い違い・小さすぎ
        let mut m2 = vec![true; ny * nx];
        for v in m2.iter_mut().take(300) {
            *v = false;
        }
        let _ = detect_deep_raw(&img, &m2, 1.0, &opts);
        assert!(detect_deep_raw(&img, &vec![true; ny * nx], 1.0, &opts).0.is_empty());
        assert!(detect_deep_raw(&img, &mask[..10], 1.0, &opts).0.is_empty());
        let tiny = Image2D { width: 15, height: 40, data: vec![1.0; 600] };
        assert!(detect_deep_raw(&tiny, &vec![false; 600], 1.0, &opts).0.is_empty());
    }

    #[test]
    fn peak_offset_matches_gaussian() {
        // ガウスの 3 点なら対数の放物線で真の位置が出る
        let g = |t: f64| (-(t - 0.3) * (t - 0.3) / 2.0).exp();
        let o = peak_offset(g(-1.0), g(0.0), g(1.0));
        assert!((o - 0.3).abs() < 1e-12, "{o}");
        assert_eq!(peak_offset(1.0, 1.0, 1.0), 0.0);
    }
}

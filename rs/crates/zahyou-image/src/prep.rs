//! 前処理(Python の zahyou_core.preprocess とその下請け)。
//!
//! 背景を引き、ノイズ σ を測り、ホットピクセルを消し、縁と焼き込み文字をマスクする。
//! 比較のしきい値は Python と同じく float32 に丸めてから比べる(numpy 2 では float32 の配列と
//! Python の float の比較は float32 で行われる)。

use rayon::prelude::*;

use crate::filters::{median3, median3_at, uniform3, zoom_linear, zoom_out_len};
use crate::stats::{filter_ordered, median_f32, sigma_clipped_std, std_f32};
use crate::{Image2D, Prep};

/// `_background(data, box=64)`: 粗いグリッドの中央値を 3×3 の中央値フィルターでならし、線形補間で伸ばす。
pub fn background(img: &Image2D) -> Vec<f32> {
    let (ny, nx) = (img.height, img.width);
    let data = &img.data;
    const BOX: usize = 64;
    let by = (ny / BOX).max(1);
    let bx = (nx / BOX).max(1);
    if by < 3 || bx < 3 {
        let m = median_f32(data);
        return vec![m; ny * nx];
    }
    // np.linspace(0, n, b + 1).astype(int): i * (n / b) を切り捨て、最後は n
    let edges = |n: usize, b: usize| -> Vec<usize> {
        let step = n as f64 / b as f64;
        let mut v: Vec<usize> = (0..=b).map(|i| (i as f64 * step) as usize).collect();
        v[b] = n;
        v
    };
    let ys = edges(ny, by);
    let xs = edges(nx, bx);
    let coarse: Vec<f32> = (0..by * bx)
        .into_par_iter()
        .map(|t| {
            let (i, j) = (t / bx, t % bx);
            let (y0, y1, x0, x1) = (ys[i], ys[i + 1], xs[j], xs[j + 1]);
            if y1 <= y0 || x1 <= x0 {
                return 0.0;
            }
            let mut tile = Vec::with_capacity((y1 - y0) * (x1 - x0));
            for y in y0..y1 {
                tile.extend_from_slice(&data[y * nx + x0..y * nx + x1]);
            }
            median_f32(&tile)
        })
        .collect();
    let coarse = median3(&coarse, by, bx);
    let oy = zoom_out_len(by, ny);
    let ox = zoom_out_len(bx, nx);
    let back = zoom_linear(&coarse, by, bx, oy, ox);
    if oy == ny && ox == nx {
        return back;
    }
    // zoom の丸めで大きさがずれたとき(Python と同じく、はみ出しは捨て、足りない所は coarse の中央値)
    let fill = median_f32(&coarse);
    let mut out = vec![fill; ny * nx];
    let h = ny.min(oy);
    let w = nx.min(ox);
    for y in 0..h {
        out[y * nx..y * nx + w].copy_from_slice(&back[y * ox..y * ox + w]);
    }
    out
}

/// `_kill_hot_pixels(sub, sigma)`: 隣 8 画素の和が中心の 30% 未満で、8σ を超える 1 画素だけの突出を
/// 3×3 の中央値で置き換える。置き換えた画素数を返す(0 なら何もしていない)。
pub fn kill_hot_pixels(sub: &mut [f32], ny: usize, nx: usize, sigma: f64) -> usize {
    let thr = (8.0 * sigma) as f32;
    // まず 8σ を超える画素があるかだけ見る(無ければ 3×3 の平均は要らない)
    if !sub.par_iter().any(|&v| v > thr) {
        return 0;
    }
    let uni = uniform3(sub, ny, nx);
    let hot: Vec<usize> = sub
        .par_iter()
        .zip(uni.par_iter())
        .enumerate()
        .filter_map(|(i, (&s, &u))| {
            let neigh = u * 9.0f32 - s;
            if s > thr && neigh < 0.30f32 * s { Some(i) } else { None }
        })
        .collect();
    let n = hot.len();
    if n == 0 || n as f64 > 0.05 * sub.len() as f64 {
        return 0;
    }
    // 置き換えは元の値の 3×3 中央値(全部を計算してから書く)
    let med: Vec<f32> = hot.par_iter().map(|&i| median3_at(sub, ny, nx, i % nx, i / nx)).collect();
    for (&i, &m) in hot.iter().zip(med.iter()) {
        sub[i] = m;
    }
    n
}

#[inline]
fn ptp(v: impl Iterator<Item = f32>) -> f32 {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    let mut nan = false;
    for x in v {
        if x.is_nan() {
            nan = true;
        }
        lo = lo.min(x);
        hi = hi.max(x);
    }
    if nan { f32::NAN } else { hi - lo }
}

/// `_mask_dead_edges(data, mask, frac=0.02)`: 平らな縁(ptp < 1e-6)や、全体の中央値の 5% 未満の縁の行・列をマスクする。
pub fn mask_dead_edges(img: &Image2D, mask: &mut [bool], med: f64) {
    let (ny, nx) = (img.height, img.width);
    let data = &img.data;
    let nyr = ((ny as f64 * 0.02) as usize).max(1);
    for y in 0..nyr {
        for row in [y, ny - 1 - y] {
            let line = &data[row * nx..row * nx + nx];
            let p = ptp(line.iter().copied()) as f64;
            let m = median_f32(line) as f64;
            if p < 1e-6 || m < med * 0.05 {
                mask[row * nx..row * nx + nx].iter_mut().for_each(|v| *v = true);
            }
        }
    }
    let nxr = ((nx as f64 * 0.02) as usize).max(1);
    for x in 0..nxr {
        for col in [x, nx - 1 - x] {
            let line: Vec<f32> = (0..ny).map(|y| data[y * nx + col]).collect();
            let p = ptp(line.iter().copied()) as f64;
            let m = median_f32(&line) as f64;
            if p < 1e-6 || m < med * 0.05 {
                for y in 0..ny {
                    mask[y * nx + col] = true;
                }
            }
        }
    }
}

/// `_mask_overlay_bands(data, mask, max_band=0.08)`: 縁に張り付き、横に広く散らばった飽和に近い画素の帯
/// (SharpCap などの日時の焼き込み)をマスクする。マスクした画素数(Python と同じ数え方)を返す。
pub fn mask_overlay_bands(data: &[f32], ny: usize, nx: usize, mask: &mut [bool]) -> usize {
    let med = median_f32(data) as f64;
    let mut sigma = sigma_clipped_std(data) as f64;
    if sigma == 0.0 {
        sigma = 1.0; // Python: `float(...) or 1.0`(NaN はそのまま)
    }
    let thr = (med + 8.0 * sigma) as f32;
    let hot = |i: usize| data[i] > thr;
    if !data.par_iter().any(|&v| v > thr) {
        return 0;
    }
    let mut masked = 0usize;
    let band_rows = ((ny as f64 * 0.08) as usize).max(3);
    let band_cols = ((nx as f64 * 0.08) as usize).max(3);

    // 1 本の線の (個数, 最小の位置, 最大の位置)
    let row_stat = |y: usize| -> (usize, usize, usize) {
        let mut c = 0;
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for x in 0..nx {
            if hot(y * nx + x) {
                c += 1;
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        (c, lo, hi)
    };
    let col_stat = |x: usize| -> (usize, usize, usize) {
        let mut c = 0;
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for y in 0..ny {
            if hot(y * nx + x) {
                c += 1;
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
        (c, lo, hi)
    };
    // seq: 端から内側へ。条件を満たす線が続く間を streak とし、途切れたら止める
    let scan = |stat: &dyn Fn(usize) -> (usize, usize, usize),
                length: usize,
                span: usize,
                band: usize|
     -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let forward: Vec<usize> = (0..band.min(length)).collect();
        let backward: Vec<usize> = (0..band.min(length)).map(|k| length - 1 - k).collect();
        for seq in [forward, backward] {
            let mut streak: Vec<usize> = Vec::new();
            for i in seq {
                let (c, lo, hi) = stat(i);
                if c >= 12 && ((hi - lo) as f64) > 0.10 * span as f64 {
                    streak.push(i);
                } else if !streak.is_empty() {
                    break;
                }
            }
            if !streak.is_empty() {
                let mn = *streak.iter().min().unwrap_or(&0);
                let mx = *streak.iter().max().unwrap_or(&0);
                let lo = mn.saturating_sub(3);
                let hi = (mx + 3).min(length - 1);
                spans.push((lo, hi));
            }
        }
        spans
    };
    for (lo, hi) in scan(&row_stat, ny, nx, band_rows) {
        for y in lo..=hi {
            mask[y * nx..y * nx + nx].iter_mut().for_each(|v| *v = true);
        }
        masked += (hi - lo + 1) * nx;
    }
    for (lo, hi) in scan(&col_stat, nx, ny, band_cols) {
        for y in 0..ny {
            for x in lo..=hi {
                mask[y * nx + x] = true;
            }
        }
        masked += (hi - lo + 1) * ny;
    }
    masked
}

/// `preprocess(data, mask_overlay)`: 背景を引き、マスクを作る。
pub fn preprocess(img: &Image2D, mask_overlay: bool) -> Prep {
    let (ny, nx) = (img.height, img.width);
    let Some(n) = ny.checked_mul(nx).filter(|&n| n == img.data.len()) else {
        // 大きさと画素の数が合わない画像(呼ぶ側の誤り)。落ちずに空の結果を返す
        let info = vec![format!("画像の大きさ {nx}×{ny} と画素の数 {} が合いません", img.data.len())];
        return Prep { sub: Image2D::default(), mask: Vec::new(), sigma: 1.0, info };
    };
    let mut mask = vec![false; n];
    let mut info = Vec::new();
    if n == 0 {
        return Prep { sub: img.clone(), mask, sigma: 1.0, info };
    }
    let back = background(img);
    let mut sub: Vec<f32> = img.data.par_iter().zip(back.par_iter()).map(|(&d, &b)| d - b).collect();
    drop(back);
    let mut sigma = sigma_clipped_std(&sub) as f64;
    if !sigma.is_finite() || sigma <= 0.0 {
        let s = std_f32(&sub) as f64;
        sigma = if s == 0.0 { 1.0 } else { s };
    }

    // 焼き込み文字の判定より先に外す(ホットピクセルが行に散っていると文字列と誤判定するため)
    let n_hot = kill_hot_pixels(&mut sub, ny, nx, sigma);
    if n_hot > 0 {
        info.push(format!("ホットピクセル {n_hot} 画素を平滑化"));
    }

    let med_data = median_f32(&img.data);
    mask_dead_edges(img, &mut mask, med_data as f64);
    if mask_overlay {
        let shifted: Vec<f32> = sub.par_iter().map(|&s| s + med_data).collect();
        let k = mask_overlay_bands(&shifted, ny, nx, &mut mask);
        if k > 0 {
            info.push(format!("焼き込みオーバーレイとみられる {k} 画素をマスク"));
        }
    }

    let n_masked = mask.par_iter().filter(|&&m| m).count();
    let s2 = if n_masked < n {
        // sub[~mask](順序を保つ)
        let refv: Vec<f32> = if n_masked == 0 {
            Vec::new()
        } else {
            let pairs: Vec<f32> =
                sub.par_iter().zip(mask.par_iter()).map(|(&s, &m)| if m { f32::NAN } else { s }).collect();
            // マスクの画素だけを除く(NaN の印で選ぶ。sub 自体は有限)
            filter_ordered(&pairs, |v| !v.is_nan())
        };
        let r: &[f32] = if n_masked == 0 { &sub } else { &refv };
        sigma_clipped_std(r) as f64
    } else {
        sigma_clipped_std(&sub) as f64
    };
    if s2.is_finite() && s2 > 0.0 {
        sigma = s2;
    }
    if n_masked > 0 {
        info.push(format!("マスク合計 {n_masked} / {n} 画素"));
    }
    Prep { sub: Image2D { width: nx, height: ny, data: sub }, mask, sigma, info }
}

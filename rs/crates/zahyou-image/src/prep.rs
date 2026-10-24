//! 前処理(Python の zahyou_core.preprocess とその下請け)。
//!
//! 背景を引き、ノイズ σ を測り、ホットピクセルを消し、縁と焼き込み文字をマスクする。
//! 比較のしきい値は Python と同じく float32 に丸めてから比べる(numpy 2 では float32 の配列と
//! Python の float の比較は float32 で行われる)。

use rayon::prelude::*;

use crate::filters::{
    median3, median3_at, uniform3_gather_row, uniform3_row, uniform3_strips, zoom_linear, zoom_linear_sub, zoom_out_len,
};
use crate::stats::{median_f32, sigma_clipped_std, std_f32, unmasked};
use crate::{Image2D, Prep};

/// 背景の粗い升目(`_background` の前半)。
enum Coarse {
    /// 升が縦横 3 つ未満: 背景は全体の中央値 1 つ
    Flat(f32),
    /// 升ごとの中央値を 3×3 の中央値でならしたもの(by×bx)
    Grid { coarse: Vec<f32>, by: usize, bx: usize },
}

/// `_background(data, box=64)` の前半: 64 画素の升ごとの中央値を 3×3 の中央値フィルターでならす。
fn background_coarse(img: &Image2D) -> Coarse {
    let (ny, nx) = (img.height, img.width);
    let data = &img.data;
    const BOX: usize = 64;
    let by = (ny / BOX).max(1);
    let bx = (nx / BOX).max(1);
    if by < 3 || bx < 3 {
        return Coarse::Flat(median_f32(data));
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
    Coarse::Grid { coarse: median3(&coarse, by, bx), by, bx }
}

/// `_background` の後半: 升目を線形補間で ny×nx に伸ばす。
fn background_from_grid(coarse: &[f32], by: usize, bx: usize, ny: usize, nx: usize) -> Vec<f32> {
    let oy = zoom_out_len(by, ny);
    let ox = zoom_out_len(bx, nx);
    let back = zoom_linear(coarse, by, bx, oy, ox);
    if oy == ny && ox == nx {
        return back;
    }
    // zoom の丸めで大きさがずれたとき(Python と同じく、はみ出しは捨て、足りない所は coarse の中央値)
    let fill = median_f32(coarse);
    let mut out = vec![fill; ny * nx];
    let h = ny.min(oy);
    let w = nx.min(ox);
    for y in 0..h {
        out[y * nx..y * nx + w].copy_from_slice(&back[y * ox..y * ox + w]);
    }
    out
}

/// `_background(data, box=64)`: 粗いグリッドの中央値を 3×3 の中央値フィルターでならし、線形補間で伸ばす。
pub fn background(img: &Image2D) -> Vec<f32> {
    let (ny, nx) = (img.height, img.width);
    match background_coarse(img) {
        Coarse::Flat(m) => vec![m; ny * nx],
        Coarse::Grid { coarse, by, bx } => background_from_grid(&coarse, by, bx, ny, nx),
    }
}

/// `data - _background(data)`(前処理の sub)。ふつうの大きさ(zoom の出力が画像と同じ大きさ)では背景の配列を作らず、
/// 引いた値を直接書く(zoom_linear_sub)。どの枝も各画素は背景の値 b(float32)を作ってから `d - b` するのと同じ値。
/// img.data は ny×nx(preprocess が確かめてから呼ぶ)。
fn background_sub(img: &Image2D) -> Vec<f32> {
    let (ny, nx) = (img.height, img.width);
    let data = &img.data;
    match background_coarse(img) {
        Coarse::Flat(m) => data.par_iter().map(|&d| d - m).collect(),
        Coarse::Grid { coarse, by, bx } => {
            if zoom_out_len(by, ny) == ny && zoom_out_len(bx, nx) == nx {
                zoom_linear_sub(&coarse, by, bx, data, ny, nx)
            } else {
                let mut b = background_from_grid(&coarse, by, bx, ny, nx);
                b.par_iter_mut().zip(data.par_iter()).for_each(|(b, &d)| *b = d - *b);
                b
            }
        }
    }
}

/// `_kill_hot_pixels(sub, sigma)`: 隣 8 画素の和が中心の 30% 未満で、8σ を超える 1 画素だけの突出を
/// 3×3 の中央値で置き換える。置き換えた画素数を返す(0 なら何もしていない)。
/// 「0 を返すのは何も書き換えなかったときだけ」は preprocess が頼っている(最後の σ の使い回し)ので崩さない。
pub fn kill_hot_pixels(sub: &mut [f32], ny: usize, nx: usize, sigma: f64) -> usize {
    let thr = (8.0 * sigma) as f32;
    // まず 8σ を超える画素があるかだけ見る(無ければ 3×3 の平均は要らない)
    if !sub.par_iter().any(|&v| v > thr) {
        return 0;
    }
    if ny == 0 || nx == 0 || ny.checked_mul(nx).is_none_or(|n| n > sub.len()) {
        // 空(前の版も 3×3 の平均が空で何もしなかった)か、大きさと画素の数が合わない(呼ぶ側の誤り)。落ちずに何もしない
        return 0;
    }
    // 3×3 の平均 u は行ごとに作って、その行の判定に使うだけ(全面の u の配列を作らない)。
    // u の値は uniform3 と同じ部品(軸 0 の帯 → 行を集める → 軸 1)で計算する。hot は行の順・行の中は x の昇順に
    // つなぐので、全面を行優先に走査したときと同じ並び
    let strips = uniform3_strips(sub, ny, nx);
    let s_all: &[f32] = sub;
    let rows: Vec<Vec<usize>> = (0..ny)
        .into_par_iter()
        .map_init(
            || (vec![0.0f32; nx], vec![0.0f32; nx]),
            |(t0, u), y| {
                uniform3_gather_row(&strips, y, t0);
                uniform3_row(t0, u);
                let row = &s_all[y * nx..y * nx + nx];
                row.iter()
                    .zip(u.iter())
                    .enumerate()
                    .filter_map(|(x, (&s, &u))| {
                        let neigh = u * 9.0f32 - s;
                        if s > thr && neigh < 0.30f32 * s { Some(y * nx + x) } else { None }
                    })
                    .collect()
            },
        )
        .collect();
    drop(strips);
    let hot: Vec<usize> = rows.concat();
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

/// 縁の 1 本の線(行か列の値を上から・左から順に並べたもの)が死んでいるか: 平ら(ptp < 1e-6)か、中央値が全体の 5% 未満。
fn dead_line(line: &[f32], med: f64) -> bool {
    let p = ptp(line.iter().copied()) as f64;
    let m = median_f32(line) as f64;
    p < 1e-6 || m < med * 0.05
}

/// `_mask_dead_edges(data, mask, frac=0.02)`: 平らな縁(ptp < 1e-6)や、全体の中央値の 5% 未満の縁の行・列をマスクする。
pub fn mask_dead_edges(img: &Image2D, mask: &mut [bool], med: f64) {
    let (ny, nx) = (img.height, img.width);
    if ny == 0 || nx == 0 {
        return; // 縁が無い(前の版は ny - 1 があふれた)
    }
    let data = &img.data;
    // 判定は行・列ごとに独立(data だけを読む)なので並列に求め、あとで印を立てる。印は立てるだけなので、
    // 前と後ろの範囲が重なる小さな画像で同じ線を 2 回判定しても結果は同じ
    let nyr = ((ny as f64 * 0.02) as usize).max(1);
    let rows: Vec<usize> = (0..nyr).flat_map(|y| [y, ny - 1 - y]).collect();
    let dead_rows: Vec<bool> = rows.par_iter().map(|&row| dead_line(&data[row * nx..row * nx + nx], med)).collect();
    for (&row, _) in rows.iter().zip(&dead_rows).filter(|(_, d)| **d) {
        mask[row * nx..row * nx + nx].iter_mut().for_each(|v| *v = true);
    }
    // 縁の列(左 nxr 本と右 nxr 本 = 前の版の x と nx-1-x の全部)。列の値を上から順に並べた配列を作ってから判定する。
    // 列の値は G 列ずつ、行を上から読んで集める(1 行で G 個続けて読むので、nx 飛びの読みが G 分の 1 になる)
    let nxr = ((nx as f64 * 0.02) as usize).max(1);
    let cols: Vec<usize> = (0..nxr).chain(nx.saturating_sub(nxr)..nx).collect();
    const G: usize = 16;
    let mut colbuf = vec![0.0f32; cols.len() * ny];
    colbuf.par_chunks_mut(G * ny).zip(cols.par_chunks(G)).for_each(|(buf, cs)| {
        for y in 0..ny {
            let row = &data[y * nx..y * nx + nx];
            for (k, &c) in cs.iter().enumerate() {
                buf[k * ny + y] = row[c];
            }
        }
    });
    let dead_cols: Vec<bool> = colbuf.par_chunks(ny).map(|line| dead_line(line, med)).collect();
    drop(colbuf);
    if dead_cols.iter().any(|&d| d) {
        let marked: Vec<usize> = cols.iter().zip(&dead_cols).filter(|(_, d)| **d).map(|(&c, _)| c).collect();
        mask[..ny * nx].par_chunks_mut(nx).for_each(|mrow| {
            for &c in &marked {
                mrow[c] = true;
            }
        });
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

    // 1 本の線の (個数, 最小の位置, 最大の位置)。下の scan が見るのは端から band 本(前と後ろ)だけなので、
    // その分を先に並列で求めて表にしておく(scan が途中で止める先の線も求めるが、値は純粋な関数なので同じ)
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
    let br = band_rows.min(ny);
    let rows_fwd: Vec<(usize, usize, usize)> = (0..br).into_par_iter().map(row_stat).collect();
    let rows_bwd: Vec<(usize, usize, usize)> = (0..br).into_par_iter().map(|k| row_stat(ny - 1 - k)).collect();
    // 行 i の値(前から br 本は rows_fwd、後ろから br 本は rows_bwd。重なる所はどちらも同じ値)
    let row_tab = |i: usize| if i < br { rows_fwd[i] } else { rows_bwd[ny - 1 - i] };
    // 列は、端の bc 列(左は col_acc[k] = 列 k、右は col_acc[bc + k] = 列 nx-1-k)を行の順に読んで数える。
    // 個数の和・位置の最小・最大は足し合わせる順に依らない
    let bc = band_cols.min(nx);
    let empty = (0usize, usize::MAX, 0usize);
    let add = |a: &mut (usize, usize, usize), y: usize| {
        a.0 += 1;
        a.1 = a.1.min(y);
        a.2 = a.2.max(y);
    };
    let col_acc: Vec<(usize, usize, usize)> = (0..ny)
        .into_par_iter()
        .fold(
            || vec![empty; 2 * bc],
            |mut acc, y| {
                let row = &data[y * nx..y * nx + nx];
                for k in 0..bc {
                    if row[k] > thr {
                        add(&mut acc[k], y);
                    }
                    if row[nx - 1 - k] > thr {
                        add(&mut acc[bc + k], y);
                    }
                }
                acc
            },
        )
        .reduce(
            || vec![empty; 2 * bc],
            |mut a, b| {
                for (x, y) in a.iter_mut().zip(&b) {
                    x.0 += y.0;
                    x.1 = x.1.min(y.1);
                    x.2 = x.2.max(y.2);
                }
                a
            },
        );
    let col_tab = |i: usize| if i < bc { col_acc[i] } else { col_acc[bc + (nx - 1 - i)] };
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
    for (lo, hi) in scan(&row_tab, ny, nx, band_rows) {
        for y in lo..=hi {
            mask[y * nx..y * nx + nx].iter_mut().for_each(|v| *v = true);
        }
        masked += (hi - lo + 1) * nx;
    }
    for (lo, hi) in scan(&col_tab, nx, ny, band_cols) {
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
    let mut sub: Vec<f32> = background_sub(img);
    // ホットピクセルを 1 つも書き換えず、σ を測る配列が sub 全体なら、最後の σ はこれと同じ配列・同じ関数になる(下で使い回す)
    let s_first = sigma_clipped_std(&sub) as f64;
    let mut sigma = s_first;
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
    let s2 = if n_masked == 0 || n_masked == n {
        // σ は sub 全体で測る(全部マスクのときも前の版と同じく sub 全体)
        // kill_hot_pixels が 0 を返すのは何も書き換えなかったときだけ(書いたら書いた数を返す)なので、
        // n_hot == 0 なら sub は最初の σ を測ったときのまま = 同じ配列に同じ決定的な関数で、値も同じ
        if n_hot == 0 { s_first } else { sigma_clipped_std(&sub) as f64 }
    } else {
        // sub[~mask](順序を保つ)。前の版はマスクの画素を NaN にしてから NaN でないものを集めていたので、
        // マスクでない位置の NaN も落ちていたが、σ クリップは最初に非有限値を順序を保って除くので同じ値になる。
        // 書き先はちょうどの長さで新しく作る。焼き込み判定で使い終えた shifted(容量 n)を書き先に回すと、容量 n の配列を
        // 持ったまま σ クリップが配列を作るので、σ クリップの残す配列が焼き込み判定のときより長いと私有メモリの最大が
        // 増えうる。新しく作れば、ここで生きる配列は前の版の最後の σ と同じ(pairs が無いぶん、集める瞬間は小さい)
        let refv = unmasked(&sub, &mask);
        sigma_clipped_std(&refv) as f64
    };
    if s2.is_finite() && s2 > 0.0 {
        sigma = s2;
    }
    if n_masked > 0 {
        info.push(format!("マスク合計 {n_masked} / {n} 画素"));
    }
    Prep { sub: Image2D { width: nx, height: ny, data: sub }, mask, sigma, info }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::tests::{rand_vals, uniform3_ref};

    /// 2026-10-24 までの kill_hot_pixels(全面の 3×3 平均を作ってから判定)。照合の基準
    fn kill_hot_pixels_ref(sub: &mut [f32], ny: usize, nx: usize, sigma: f64) -> usize {
        let thr = (8.0 * sigma) as f32;
        if !sub.iter().any(|&v| v > thr) {
            return 0;
        }
        let uni = uniform3_ref(sub, ny, nx);
        let hot: Vec<usize> = sub
            .iter()
            .zip(uni.iter())
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
        let med: Vec<f32> = hot.iter().map(|&i| median3_at(sub, ny, nx, i % nx, i / nx)).collect();
        for (&i, &m) in hot.iter().zip(med.iter()) {
            sub[i] = m;
        }
        n
    }

    /// 2026-10-24 までの mask_dead_edges(1 本ずつ直列に判定)。照合の基準
    fn mask_dead_edges_ref(img: &Image2D, mask: &mut [bool], med: f64) {
        let (ny, nx) = (img.height, img.width);
        let data = &img.data;
        let nyr = ((ny as f64 * 0.02) as usize).max(1);
        for y in 0..nyr {
            for row in [y, ny - 1 - y] {
                if dead_line(&data[row * nx..row * nx + nx], med) {
                    mask[row * nx..row * nx + nx].iter_mut().for_each(|v| *v = true);
                }
            }
        }
        let nxr = ((nx as f64 * 0.02) as usize).max(1);
        for x in 0..nxr {
            for col in [x, nx - 1 - x] {
                let line: Vec<f32> = (0..ny).map(|y| data[y * nx + col]).collect();
                if dead_line(&line, med) {
                    for y in 0..ny {
                        mask[y * nx + col] = true;
                    }
                }
            }
        }
    }

    /// 2026-10-24 までの mask_overlay_bands(scan が見る線ごとにその場で数える)。照合の基準
    fn mask_overlay_bands_ref(data: &[f32], ny: usize, nx: usize, mask: &mut [bool]) -> usize {
        let med = median_f32(data) as f64;
        let mut sigma = sigma_clipped_std(data) as f64;
        if sigma == 0.0 {
            sigma = 1.0;
        }
        let thr = (med + 8.0 * sigma) as f32;
        let hot = |i: usize| data[i] > thr;
        if !data.iter().any(|&v| v > thr) {
            return 0;
        }
        let mut masked = 0usize;
        let band_rows = ((ny as f64 * 0.08) as usize).max(3);
        let band_cols = ((nx as f64 * 0.08) as usize).max(3);
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
        let scan = |stat: &dyn Fn(usize) -> (usize, usize, usize), length: usize, span: usize, band: usize| {
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
                    spans.push((mn.saturating_sub(3), (mx + 3).min(length - 1)));
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

    /// 雑音・ホットピクセル(縁・角を含む)・焼き込みの帯・死んだ縁の行と列を置いた画像
    fn test_image(ny: usize, nx: usize, seed: u64, hot: usize, band: bool, dead: bool) -> Image2D {
        let noise = rand_vals(ny * nx, seed);
        let mut d: Vec<f32> = noise.iter().map(|&v| 1000.0 + v * 0.01).collect();
        let pos = rand_vals(hot.max(1), seed + 1);
        for (k, &p) in pos.iter().enumerate().take(hot) {
            let i = match k % 5 {
                0 => 0,
                1 => nx - 1,
                2 => (ny - 1) * nx,
                3 => ny * nx - 1,
                _ => ((p as f64 + 3.0e3) / 1.0e4 * (ny * nx) as f64) as usize % (ny * nx),
            };
            d[i] += 5000.0;
        }
        if band && ny > 8 && nx > 2 {
            // 上の縁の横に広い焼き込み(右の縁の縦にも)
            for y in 1..4 {
                for x in (0..nx).step_by(5) {
                    d[y * nx + x] = 65000.0;
                }
            }
            for y in (0..ny).step_by(3) {
                d[y * nx + nx - 2] = 65000.0;
            }
        }
        if dead {
            for v in d[..nx].iter_mut() {
                *v = 0.0;
            }
            for y in 0..ny {
                d[y * nx + nx - 1] = 3.0;
                if y % 2 == 0 {
                    d[y * nx] = 7.0;
                }
            }
        }
        Image2D { width: nx, height: ny, data: d }
    }

    /// 1 つの縁(0 上・1 下・2 左・3 右)から 2 本目の線に、5 画素おきの焼き込みを置いた画像(ny, nx ≥ 2)
    fn band_image(ny: usize, nx: usize, seed: u64, side: usize) -> Image2D {
        let mut img = test_image(ny, nx, seed, 0, false, false);
        let d = &mut img.data;
        if side < 2 {
            let y = if side == 0 { 1 } else { ny - 2 };
            for x in (0..nx).step_by(5) {
                d[y * nx + x] = 65000.0;
            }
        } else {
            let x = if side == 2 { 1 } else { nx - 2 };
            for y in (0..ny).step_by(5) {
                d[y * nx + x] = 65000.0;
            }
        }
        img
    }

    const SHAPES: [(usize, usize); 9] =
        [(1, 1), (1, 300), (300, 1), (2, 3), (9, 9), (37, 300), (130, 257), (201, 700), (64, 1100)];

    #[test]
    fn hot_pixels_match_reference() {
        // 行ごとの 3×3 平均で判定しても、前の版と同じ画素を同じ値で置き換える(5% 超で何もしない場合・ホットピクセル無しも)
        let mut n_fixed = 0;
        for (ny, nx) in SHAPES {
            for (hot, sigma) in [(0usize, 1.0f64), (7, 1.0), (40, 1.0), (7, 1e9), (7, 0.001)] {
                let img = test_image(ny, nx, (ny * 31 + nx) as u64, hot, false, false);
                // 背景を引いたあとの値(0 のまわり)にする
                let sub: Vec<f32> = img.data.iter().map(|&v| v - 1000.0).collect();
                let mut a = sub.clone();
                let mut b = sub;
                let na = kill_hot_pixels(&mut a, ny, nx, sigma);
                let nb = kill_hot_pixels_ref(&mut b, ny, nx, sigma);
                assert_eq!(na, nb, "{ny}x{nx} hot={hot} sigma={sigma}");
                assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "{ny}x{nx} hot={hot}");
                n_fixed += (na > 0) as usize;
            }
        }
        assert!(n_fixed >= 5, "置き換えた場合 {n_fixed} 件");
        // 大きさと画素の数が合わない・空は何もしない(落ちない)
        let mut v = vec![1.0e6f32; 10];
        assert_eq!(kill_hot_pixels(&mut v, 4, 4, 1.0), 0);
        assert_eq!(kill_hot_pixels(&mut v, 0, 10, 1.0), 0);
        assert_eq!(kill_hot_pixels(&mut v, usize::MAX, 3, 1.0), 0);
    }

    #[test]
    fn edge_masks_match_reference() {
        // 縁の死んだ行・列と焼き込みの帯のマスクが、前の版(直列に 1 本ずつ)と同じ
        let (mut n_dead, mut n_band) = (0, 0);
        for (ny, nx) in SHAPES {
            for (band, dead) in [(false, false), (true, false), (false, true), (true, true)] {
                let img = test_image(ny, nx, (ny * 13 + nx) as u64, 3, band, dead);
                for med in [1000.0f64, 0.0, 1.0e9, f64::NAN] {
                    let mut a = vec![false; ny * nx];
                    let mut b = vec![false; ny * nx];
                    mask_dead_edges(&img, &mut a, med);
                    mask_dead_edges_ref(&img, &mut b, med);
                    assert_eq!(a, b, "dead {ny}x{nx} band={band} dead={dead} med={med}");
                    n_dead += (med == 1000.0 && a.iter().any(|&m| m)) as usize;
                }
                let mut a = vec![false; ny * nx];
                let mut b = vec![false; ny * nx];
                let ka = mask_overlay_bands(&img.data, ny, nx, &mut a);
                let kb = mask_overlay_bands_ref(&img.data, ny, nx, &mut b);
                assert_eq!((ka, &a), (kb, &b), "overlay {ny}x{nx} band={band} dead={dead}");
                n_band += (ka > 0) as usize;
            }
        }
        // 印が立つ場合を通したこと
        assert!(n_dead >= 5 && n_band >= 5, "死んだ縁 {n_dead} 件・焼き込み {n_band} 件");
        // 4 つの縁それぞれに帯を置き、前から・後ろから見る行の表引きと、左右の列の数え上げを全部、帯が立つ線で通す
        // (上の test_image の帯は上の行と右の列にしか無い)
        let mut hit = [0usize; 4];
        for (ny, nx) in SHAPES.into_iter().filter(|&(ny, nx)| ny >= 9 && nx >= 9) {
            for (side, h) in hit.iter_mut().enumerate() {
                let img = band_image(ny, nx, (ny * 7 + nx + side) as u64, side);
                let mut a = vec![false; ny * nx];
                let mut b = vec![false; ny * nx];
                let ka = mask_overlay_bands(&img.data, ny, nx, &mut a);
                let kb = mask_overlay_bands_ref(&img.data, ny, nx, &mut b);
                assert_eq!((ka, &a), (kb, &b), "overlay side={side} {ny}x{nx}");
                *h += (ka > 0) as usize;
            }
        }
        assert!(hit.iter().all(|&h| h >= 1), "縁(上・下・左・右)ごとに帯が立った形の数 {hit:?}");
        // 空の画像で落ちない
        mask_dead_edges(&Image2D { width: 0, height: 5, data: Vec::new() }, &mut [], 1.0);
        mask_dead_edges(&Image2D { width: 5, height: 0, data: Vec::new() }, &mut [], 1.0);
    }

    #[test]
    fn background_sub_matches_background() {
        // 背景を作ってから引くのと同じ(升が 3 つ未満の平らな背景も)
        for (ny, nx, seed) in [(100usize, 100usize, 1u64), (192, 192, 2), (300, 517, 3), (1000, 1500, 4), (64, 1300, 5)]
        {
            let img = test_image(ny, nx, seed, 9, true, false);
            let back = background(&img);
            let want: Vec<f32> = img.data.iter().zip(&back).map(|(&d, &b)| d - b).collect();
            let got = background_sub(&img);
            assert!(got.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()), "{ny}x{nx}");
        }
    }
}

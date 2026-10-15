//! 星の検出(Python の zahyou_core.detect_sources・_measure・detect_sources_deep)。
//!
//! 平滑化した画像をしきい値で 2 値にし、4 連結でまとめ(scipy の label と同じく、ラスター順で最初に
//! 現れた順に番号)、領域ごとに重心・光量・伸びを測る。しきい値は 12σ から下げていき、
//! min_sources 個そろったところで止める。

use rayon::prelude::*;

use crate::filters::gaussian1;
use crate::stats::{filter_ordered, sigma_clipped_std};
use crate::{DetectOpts, Image2D, Prep, Source};

/// 横に続く 1 本(行 y の x0..=x1)。
#[derive(Clone, Copy)]
struct Run {
    y: u32,
    x0: u32,
    x1: u32,
}

/// 2 値画像(smooth > thr)の行ごとの連なり。
fn runs_above(smooth: &[f32], ny: usize, nx: usize, thr: f32) -> Vec<Vec<Run>> {
    (0..ny)
        .into_par_iter()
        .map(|y| {
            let row = &smooth[y * nx..y * nx + nx];
            let mut v = Vec::new();
            let mut x = 0;
            while x < nx {
                if row[x] > thr {
                    let s = x;
                    while x < nx && row[x] > thr {
                        x += 1;
                    }
                    v.push(Run { y: y as u32, x0: s as u32, x1: (x - 1) as u32 });
                } else {
                    x += 1;
                }
            }
            v
        })
        .collect()
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        let p = parent[i as usize];
        parent[i as usize] = parent[p as usize];
        i = p;
    }
    i
}

/// 4 連結の成分。戻り値は (各 run の成分番号(0 始まり、ラスター順で最初に現れた順), 成分の数, 全 run)。
fn label_runs(rows: Vec<Vec<Run>>) -> (Vec<u32>, usize, Vec<Run>) {
    let total: usize = rows.iter().map(|r| r.len()).sum();
    let mut runs: Vec<Run> = Vec::with_capacity(total);
    let mut row_start: Vec<usize> = Vec::with_capacity(rows.len() + 1);
    for r in rows {
        row_start.push(runs.len());
        runs.extend(r);
    }
    row_start.push(runs.len());
    let mut parent: Vec<u32> = (0..runs.len() as u32).collect();
    for y in 1..row_start.len() - 1 {
        let (p0, p1) = (row_start[y - 1], row_start[y]);
        let (c0, c1) = (p1, row_start[y + 1]);
        // 上の行の run と横の範囲が 1 列でも重なれば同じ成分(斜めはつながない)
        let mut j = p0;
        for i in c0..c1 {
            let a = runs[i];
            while j < p1 && runs[j].x1 < a.x0 {
                j += 1;
            }
            let mut k = j;
            while k < p1 && runs[k].x0 <= a.x1 {
                let ra = find(&mut parent, i as u32);
                let rb = find(&mut parent, k as u32);
                if ra != rb {
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    parent[hi as usize] = lo;
                }
                k += 1;
            }
        }
    }
    // 番号はラスター順で最初に現れた順(run はラスター順に並んでいる)
    let mut comp = vec![u32::MAX; runs.len()];
    let mut root_label = vec![u32::MAX; runs.len()];
    let mut n = 0u32;
    for (i, c) in comp.iter_mut().enumerate() {
        let r = find(&mut parent, i as u32) as usize;
        if root_label[r] == u32::MAX {
            root_label[r] = n;
            n += 1;
        }
        *c = root_label[r];
    }
    (comp, n as usize, runs)
}

/// `ndimage.label(binary)`(4 連結)。戻り値は (各画素の番号(0 は背景、1 から), 成分の数)。
/// 番号はラスター順で最初に現れた順(scipy と同じ)。
pub(crate) fn label4(binary: &[bool], ny: usize, nx: usize) -> (Vec<u32>, usize) {
    let mut labels = vec![0u32; ny * nx];
    if ny == 0 || nx == 0 || binary.len() != ny * nx {
        return (labels, 0);
    }
    let rows: Vec<Vec<Run>> = (0..ny)
        .map(|y| {
            let row = &binary[y * nx..y * nx + nx];
            let mut v = Vec::new();
            let mut x = 0;
            while x < nx {
                if row[x] {
                    let s = x;
                    while x < nx && row[x] {
                        x += 1;
                    }
                    v.push(Run { y: y as u32, x0: s as u32, x1: (x - 1) as u32 });
                } else {
                    x += 1;
                }
            }
            v
        })
        .collect();
    let (comp, n, runs) = label_runs(rows);
    for (r, &c) in runs.iter().zip(comp.iter()) {
        let y = r.y as usize;
        for x in r.x0 as usize..=r.x1 as usize {
            labels[y * nx + x] = c + 1;
        }
    }
    (labels, n)
}

/// `_measure`: 1 つの領域の重心・光量・広がり。光量が 0 以下なら None。
fn measure(work: &[f32], nx: usize, runs: &[Run]) -> Option<Source> {
    let mut tot = 0.0f64;
    let mut sx = 0.0f64;
    let mut sy = 0.0f64;
    let mut peak = f64::NEG_INFINITY;
    let mut area = 0usize;
    for r in runs {
        let y = r.y as usize;
        for x in r.x0 as usize..=r.x1 as usize {
            let w = work[y * nx + x] as f64;
            tot += w;
            sx += w * x as f64;
            sy += w * y as f64;
            if w > peak {
                peak = w;
            }
            area += 1;
        }
    }
    if tot.is_nan() || tot <= 0.0 {
        return None;
    }
    let cx = sx / tot;
    let cy = sy / tot;
    let (mut vx, mut vy, mut vxy) = (0.0f64, 0.0f64, 0.0f64);
    for r in runs {
        let y = r.y as usize;
        let dy = y as f64 - cy;
        for x in r.x0 as usize..=r.x1 as usize {
            let w = work[y * nx + x] as f64;
            let dx = x as f64 - cx;
            vx += w * dx * dx;
            vy += w * dy * dy;
            vxy += w * dx * dy;
        }
    }
    vx /= tot;
    vy /= tot;
    vxy /= tot;
    let tr = vx + vy;
    let det = vx * vy - vxy * vxy;
    let root = (tr * tr / 4.0 - det).max(0.0).sqrt();
    let a2 = (tr / 2.0 + root).max(1e-6);
    let b2 = (tr / 2.0 - root).max(1e-6);
    Some(Source {
        x: cx,
        y: cy,
        flux: tot,
        peak,
        area,
        elong: (a2 / b2).sqrt(),
        fwhm: 2.3548 * ((vx + vy) / 2.0).max(1e-6).sqrt(),
        snr: 0.0,
    })
}

/// `detect_sources(sub, mask, sigma, ...)` の本体。戻り値は (明るい順の星, 使ったしきい値(σ の倍数))。
pub fn detect_raw(sub: &Image2D, mask: &[bool], sigma: f64, opts: &DetectOpts) -> (Vec<Source>, Option<f64>) {
    let (ny, nx) = (sub.height, sub.width);
    let n = ny * nx;
    if n == 0 || sub.data.len() != n || mask.len() != n {
        return (Vec::new(), None);
    }
    let work: Vec<f32> = sub.data.par_iter().zip(mask.par_iter()).map(|(&s, &m)| if m { 0.0 } else { s }).collect();
    let smooth = gaussian1(&work, ny, nx);
    let smooth_sigma = sigma / 2.0; // 平滑化でノイズが下がる分

    let mut best: Vec<Source> = Vec::new();
    let mut used: Option<f64> = None;
    for thr in [12.0f64, 8.0, 6.0, 5.0, 4.0, 3.0, 2.5, 2.0] {
        let t32 = (thr * smooth_sigma) as f32;
        let n_on: usize = smooth.par_iter().filter(|&&v| v > t32).count();
        if n_on == 0 {
            continue;
        }
        if n_on as f64 > 0.25 * n as f64 {
            break; // 何かおかしい(雲・月明かり)ので下げない
        }
        let rows = runs_above(&smooth, ny, nx, t32);
        let (comp, ncomp, runs) = label_runs(rows);
        if ncomp == 0 {
            continue;
        }
        if ncomp > 20000 {
            break; // ノイズが噴き出している
        }
        // 成分ごとの run の並び(番号順)
        let mut count = vec![0usize; ncomp + 1];
        for &c in &comp {
            count[c as usize + 1] += 1;
        }
        for i in 0..ncomp {
            count[i + 1] += count[i];
        }
        let mut order = vec![0usize; runs.len()];
        let mut fill = count.clone();
        for (i, &c) in comp.iter().enumerate() {
            order[fill[c as usize]] = i;
            fill[c as usize] += 1;
        }
        let comp_runs = |c: usize| -> Vec<Run> { order[count[c]..count[c + 1]].iter().map(|&i| runs[i]).collect() };
        let keep: Vec<usize> = (0..ncomp)
            .filter(|&c| {
                let a: usize =
                    order[count[c]..count[c + 1]].iter().map(|&i| (runs[i].x1 - runs[i].x0 + 1) as usize).sum();
                a >= opts.min_area
            })
            .collect();
        if keep.is_empty() {
            continue;
        }
        let measured: Vec<Option<Source>> = keep.par_iter().map(|&c| measure(&work, nx, &comp_runs(c))).collect();
        let mut cand: Vec<Source> = Vec::new();
        for mut c in measured.into_iter().flatten() {
            // 開口 SNR。ノイズのこぶは面積のわりに光量が無いのでここで落ちる
            c.snr = c.flux / (sigma * (c.area.max(1) as f64).sqrt());
            if c.snr < opts.min_snr || c.elong > opts.max_elongation {
                continue;
            }
            if !(2.0 <= c.x && c.x <= nx as f64 - 3.0 && 2.0 <= c.y && c.y <= ny as f64 - 3.0) {
                continue;
            }
            cand.push(c);
        }
        if cand.len() > best.len() {
            best = cand.clone();
            used = Some(thr);
        }
        if cand.len() >= opts.min_sources {
            best = cand;
            used = Some(thr);
            break;
        }
    }
    // 明るい順(同じ光量は元の順を保つ = Python の安定な sort)
    best.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));
    best.truncate(opts.max_sources);
    (best, used)
}

/// 標準の検出(`detect_sources(prep.sub, prep.mask, prep.sigma, ...)`)。
pub fn detect_sources(prep: &Prep, opts: &DetectOpts) -> (Vec<Source>, Option<f64>) {
    detect_raw(&prep.sub, &prep.mask, prep.sigma, opts)
}

/// 深い検出(`detect_sources_deep`)。画像を 2×2 にまとめてから同じ検出をかけ、座標を元の画素へ戻す
/// ((bx, by) → (2bx+0.5, 2by+0.5))。4 画素の和で星の光は 4 倍、白色雑音は 2 倍なので SNR が 2 倍になる。
/// 雑音はまとめた画像で測り直す。Python 版はまとめた画像に min_sources=60 で検出をかける
/// (ほかは既定値)ので、同じにするには `DetectOpts { min_sources: 60, ..Default::default() }` を渡す。
pub fn detect_sources_deep(prep: &Prep, opts: &DetectOpts) -> (Vec<Source>, Option<f64>) {
    let (ny, nx) = (prep.sub.height, prep.sub.width);
    let (h, w) = (ny / 2 * 2, nx / 2 * 2); // 奇数の端の 1 行・1 列は捨てる
    if h < 16 || w < 16 || prep.mask.len() != ny * nx || prep.sub.data.len() != ny * nx {
        return (Vec::new(), None);
    }
    let (h2, w2) = (h / 2, w / 2);
    let s = &prep.sub.data;
    let m = &prep.mask;
    let mut s2 = vec![0.0f32; h2 * w2];
    let mut m2 = vec![false; h2 * w2];
    s2.par_chunks_mut(w2).zip(m2.par_chunks_mut(w2)).enumerate().for_each(|(i, (srow, mrow))| {
        let r0 = 2 * i * nx;
        let r1 = (2 * i + 1) * nx;
        for j in 0..w2 {
            let (a, b, c, d) = (r0 + 2 * j, r0 + 2 * j + 1, r1 + 2 * j, r1 + 2 * j + 1);
            // numpy の reshape(h/2, 2, w/2, 2).sum(axis=(1, 3)) と同じ順: (a00 + a01) + (a10 + a11)
            srow[j] = (s[a] + s[b]) + (s[c] + s[d]);
            mrow[j] = m[a] || m[b] || m[c] || m[d];
        }
    });
    let any_free = m2.iter().any(|&v| !v);
    let sigma2 = {
        let all_masked = !any_free;
        let v = if all_masked || !m2.iter().any(|&v| v) {
            sigma_clipped_std(&s2)
        } else {
            let marked: Vec<f32> = s2.iter().zip(m2.iter()).map(|(&v, &k)| if k { f32::NAN } else { v }).collect();
            sigma_clipped_std(&filter_ordered(&marked, |v| !v.is_nan()))
        } as f64;
        if !v.is_finite() || v <= 0.0 { 2.0 * prep.sigma } else { v }
    };
    let img2 = Image2D { width: w2, height: h2, data: s2 };
    let (mut found, thr) = detect_raw(&img2, &m2, sigma2, opts);
    for c in &mut found {
        c.x = 2.0 * c.x + 0.5;
        c.y = 2.0 * c.y + 0.5;
        c.fwhm *= 2.0;
        c.area *= 4;
    }
    (found, thr)
}

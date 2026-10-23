//! numpy / astropy と同じ値になる統計の部品。
//!
//! Python 版は float32 の配列を numpy で集計している。numpy の縮約は配列を 8192 個(バッファーの大きさ)ずつに
//! 区切り、区切りごとのペアワイズ和(8 本の部分和・128 個で打ち切り)を、恒等元 0 から先頭の区切りの順に足す。
//! 平均・分散は float32 の和を float64 で割って float32 に丸める。
//! σ クリップの境界も float32 で計算される(NEP 50: float32 × Python の float は float32)。
//! ここではその順序を 1 つずつ写して、ビット単位で同じ値を出す(gen_image.py の image_kernels.json で確かめる)。
//! 速さのため、区切りごとの和と中央値の選択(ヒストグラム 2 回)は rayon で並列にする。並列にしても
//! 足す順序は変わらないので結果は同じ。

use rayon::prelude::*;

/// numpy の縮約の区切り(np.getbufsize() の既定値)
const NP_BUF: usize = 8192;
/// これより長い配列は区切りごとの和を並列に計算する
const PAR_SUM_MIN: usize = 1 << 16;
/// これより長い配列は中央値をヒストグラムで選ぶ(短いものは並べ替えの選択)
const RADIX_MIN: usize = 1 << 15;
/// 並列の処理単位(要素数)
const CHUNK: usize = 1 << 16;
/// σ クリップの読みを並列にする塊の最小(8192 の倍数。和の区切りがそろう)
const SUPER_MIN: usize = NP_BUF * 8;

/// σ クリップの読みを並列にする塊の大きさ(8192 の倍数)。塊ごとに 65536 個のヒストグラムを持つので、
/// 塊はスレッド数の 2 倍まで、1 つ SUPER_MIN 個以上にする(50 万個でも 7 つに分けて並列に読む)。
fn super_len(n: usize) -> usize {
    let parts = (n / SUPER_MIN).clamp(1, 2 * rayon::current_num_threads().max(1));
    n.div_ceil(parts).div_ceil(NP_BUF).max(1) * NP_BUF
}

/// 塊ごとのヒストグラムを足し合わせる(ビンを分けて並列に)。
fn merge_hists(hs: &[&Vec<u32>]) -> Vec<u32> {
    let mut out = vec![0u32; 65536];
    out.par_chunks_mut(4096).enumerate().for_each(|(i, o)| {
        for h in hs {
            for (a, b) in o.iter_mut().zip(&h[i * 4096..(i + 1) * 4096]) {
                *a += *b;
            }
        }
    });
    out
}

/// float32 の大小と同じ順になる 32 bit の鍵(負の数はビットを反転、正の数は符号ビットを立てる)。
#[inline]
pub(crate) fn f32_key(x: f32) -> u32 {
    let b = x.to_bits();
    if b & 0x8000_0000 != 0 { !b } else { b | 0x8000_0000 }
}

#[inline]
pub(crate) fn key_to_f32(k: u32) -> f32 {
    if k & 0x8000_0000 != 0 { f32::from_bits(k & 0x7FFF_FFFF) } else { f32::from_bits(!k) }
}

/// numpy の pairwise_sum(float32、各要素に `f` をかけてから足す)。1 区切り(8192 個以下)に使う。
#[inline]
fn pw_map<F: Fn(f32) -> f32 + Sync>(a: &[f32], f: &F) -> f32 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0f32;
        for &x in a {
            res += f(x);
        }
        res
    } else if n <= 128 {
        let mut r = [f(a[0]), f(a[1]), f(a[2]), f(a[3]), f(a[4]), f(a[5]), f(a[6]), f(a[7])];
        let mut i = 8;
        let end = n - n % 8;
        while i < end {
            for j in 0..8 {
                r[j] += f(a[i + j]);
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += f(a[i]);
            i += 1;
        }
        res
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        let (l, r) = a.split_at(n2);
        pw_map(l, f) + pw_map(r, f)
    }
}

/// numpy の縮約(float32): 8192 個ずつのペアワイズ和を、0 から先頭の区切りの順に足す。
fn np_sum_map<F: Fn(f32) -> f32 + Sync>(a: &[f32], f: &F) -> f32 {
    let mut res = 0.0f32;
    if a.len() >= PAR_SUM_MIN {
        let parts: Vec<f32> = a.par_chunks(NP_BUF).map(|c| pw_map(c, f)).collect();
        for p in parts {
            res += p;
        }
    } else {
        for c in a.chunks(NP_BUF) {
            res += pw_map(c, f);
        }
    }
    res
}

/// `np.sum(a)`(a は float32 の 1 次元)。
pub fn sum_f32(a: &[f32]) -> f32 {
    np_sum_map(a, &|x| x)
}

/// float64 のペアワイズ和(`np.sum` の float64 版)。
pub fn sum_f64(a: &[f64]) -> f64 {
    fn pw(a: &[f64]) -> f64 {
        let n = a.len();
        if n < 8 {
            let mut res = 0.0;
            for &x in a {
                res += x;
            }
            res
        } else if n <= 128 {
            let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
            let mut i = 8;
            let end = n - n % 8;
            while i < end {
                for j in 0..8 {
                    r[j] += a[i + j];
                }
                i += 8;
            }
            let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
            while i < n {
                res += a[i];
                i += 1;
            }
            res
        } else {
            let mut n2 = n / 2;
            n2 -= n2 % 8;
            pw(&a[..n2]) + pw(&a[n2..])
        }
    }
    let mut res = 0.0;
    for c in a.chunks(NP_BUF) {
        res += pw(c);
    }
    res
}

/// `np.mean(a)`(float32)。和を float64 で個数で割り、float32 に丸める。空なら NaN。
pub fn mean_f32(a: &[f32]) -> f32 {
    if a.is_empty() {
        return f32::NAN;
    }
    (sum_f32(a) as f64 / a.len() as f64) as f32
}

/// `np.std(a)` / `np.nanstd(a)`(float32、ddof=0、NaN を含まない配列)。空なら NaN。
pub fn std_f32(a: &[f32]) -> f32 {
    if a.is_empty() {
        return f32::NAN;
    }
    let n = a.len() as f64;
    let m = (sum_f32(a) as f64 / n) as f32;
    let ss = np_sum_map(a, &|x| {
        let d = x - m;
        d * d
    });
    ((ss as f64 / n) as f32).sqrt()
}

/// 小さい配列: 並べ替えの選択で k 番目と k+1 番目(k+1 が無ければ k 番目)を返す。
fn select_small(a: &[f32], k: usize) -> (f32, f32) {
    let mut v: Vec<u32> = a.iter().map(|&x| f32_key(x)).collect();
    let (_, kth, right) = v.select_nth_unstable(k);
    let kv = *kth;
    let next = right.iter().copied().min().unwrap_or(kv);
    (key_to_f32(kv), key_to_f32(next))
}

fn hist16<F: Fn(u32) -> Option<u32> + Sync>(a: &[f32], bin: F) -> Vec<u32> {
    a.par_chunks(CHUNK)
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
                for (x, y) in a.iter_mut().zip(b.iter()) {
                    *x += *y;
                }
                a
            },
        )
}

/// 大きい配列: 鍵の上位 16 bit と下位 16 bit のヒストグラムで、k 番目と k+1 番目を正確に選ぶ。
fn select_radix(a: &[f32], k: usize) -> (f32, f32) {
    let n = a.len();
    let h1 = hist16(a, |key| Some(key >> 16));
    // k 番目の入る上位ビン
    let mut cum = 0usize;
    let mut b1 = 0usize;
    for (i, &c) in h1.iter().enumerate() {
        if cum + c as usize > k {
            b1 = i;
            break;
        }
        cum += c as usize;
    }
    let base = cum; // b1 より前の個数
    let in_b1 = h1[b1] as usize;
    let next_same_bin = k + 1 < n && k + 1 - base < in_b1;
    // k+1 番目が別のビンにあるなら、その次の空でないビン
    let b2 = if !next_same_bin && k + 1 < n { (b1 + 1..65536).find(|&i| h1[i] > 0) } else { None };
    let b1u = b1 as u32;
    let h2 = hist16(a, |key| if key >> 16 == b1u { Some(key & 0xFFFF) } else { None });
    let r = k - base;
    let mut cum2 = 0usize;
    let mut lo = 0usize;
    for (i, &c) in h2.iter().enumerate() {
        if cum2 + c as usize > r {
            lo = i;
            break;
        }
        cum2 += c as usize;
    }
    let kkey = (b1u << 16) | lo as u32;
    let kv = key_to_f32(kkey);
    if k + 1 >= n {
        return (kv, kv);
    }
    if next_same_bin {
        // 同じ下位ビンにまだ残っているか
        if cum2 + h2[lo] as usize > r + 1 {
            return (kv, kv);
        }
        let l2 = (lo + 1..65536).find(|&i| h2[i] > 0).unwrap_or(lo);
        return (kv, key_to_f32((b1u << 16) | l2 as u32));
    }
    match b2 {
        Some(b2) => {
            let b2u = b2 as u32;
            let m = a
                .par_chunks(CHUNK)
                .map(|c| c.iter().map(|&x| f32_key(x)).filter(|key| key >> 16 == b2u).min().unwrap_or(u32::MAX))
                .min()
                .unwrap_or(kkey);
            (kv, key_to_f32(m))
        }
        None => (kv, kv),
    }
}

/// k 番目(0 始まり)と k+1 番目に小さい値。NaN を含まない配列に使う。
pub(crate) fn kth_pair(a: &[f32], k: usize) -> (f32, f32) {
    if a.len() < RADIX_MIN || a.len() > u32::MAX as usize { select_small(a, k) } else { select_radix(a, k) }
}

/// NaN を含まない配列の `np.median`。偶数個なら中央 2 個を float32 で足して 2 で割る。空なら NaN。
pub fn median_finite_f32(a: &[f32]) -> f32 {
    let n = a.len();
    if n == 0 {
        return f32::NAN;
    }
    if n % 2 == 1 {
        let (v, _) = kth_pair(a, n / 2);
        0.0f32 + v
    } else {
        let (lo, hi) = kth_pair(a, n / 2 - 1);
        let s = (0.0f32 + lo) + hi;
        (s as f64 / 2.0) as f32
    }
}

/// `np.median(a)`(float32)。NaN が 1 つでもあれば NaN(numpy と同じ)。
pub fn median_f32(a: &[f32]) -> f32 {
    if a.par_iter().any(|x| x.is_nan()) {
        return f32::NAN;
    }
    median_finite_f32(a)
}

/// 順序を保ったまま、条件を満たす要素だけを集める(並列)。
pub(crate) fn filter_ordered<P: Fn(f32) -> bool + Sync>(a: &[f32], keep: P) -> Vec<f32> {
    let counts = count_kept(a, &keep);
    filter_with_counts(a, keep, &counts)
}

/// 区切り CHUNK ごとの、条件を満たす要素の数。
fn count_kept<P: Fn(f32) -> bool + Sync>(a: &[f32], keep: &P) -> Vec<usize> {
    a.par_chunks(CHUNK).map(|c| c.iter().filter(|&&x| keep(x)).count()).collect()
}

/// 書き先 out(長さ Σcounts)を、区切りごとの書き場所(長さ counts[i])に分ける。
fn split_parts<'a>(out: &'a mut [f32], counts: &[usize]) -> Vec<&'a mut [f32]> {
    let mut parts: Vec<&mut [f32]> = Vec::with_capacity(counts.len());
    let mut rest: &mut [f32] = out;
    for &c in counts {
        let (h, t) = rest.split_at_mut(c);
        parts.push(h);
        rest = t;
    }
    parts
}

/// `filter_ordered` の書き出し(区切り CHUNK ごとの個数を数え済みのとき)。書き先はちょうどの長さで新しく作る。
fn filter_with_counts<P: Fn(f32) -> bool + Sync>(a: &[f32], keep: P, counts: &[usize]) -> Vec<f32> {
    let total: usize = counts.iter().sum();
    let mut out = vec![0.0f32; total];
    a.par_chunks(CHUNK).zip(split_parts(&mut out, counts).into_par_iter()).for_each(|(c, o)| {
        let mut j = 0;
        for &x in c {
            if keep(x) {
                o[j] = x;
                j += 1;
            }
        }
    });
    out
}

/// `a[~mask]`(マスクが偽の要素)を順序を保って集める(並列)。前処理の最後の σ で使う。
/// a と mask は同じ長さ(違えば短いほうまで)。書き先はちょうどの長さで新しく作る。
pub(crate) fn unmasked(a: &[f32], mask: &[bool]) -> Vec<f32> {
    let n = a.len().min(mask.len());
    let (a, mask) = (&a[..n], &mask[..n]);
    let counts: Vec<usize> = mask.par_chunks(CHUNK).map(|c| c.iter().filter(|&&m| !m).count()).collect();
    let total: usize = counts.iter().sum();
    let mut out = vec![0.0f32; total];
    a.par_chunks(CHUNK).zip(mask.par_chunks(CHUNK)).zip(split_parts(&mut out, &counts).into_par_iter()).for_each(
        |((c, mc), o)| {
            let mut j = 0;
            for (&x, &m) in c.iter().zip(mc) {
                if !m {
                    o[j] = x;
                    j += 1;
                }
            }
        },
    );
    out
}

/// `sigma_clipped_stats(data, sigma=3.0)` の戻り値(mean, median, std)。値は float32。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipStats {
    pub mean: f32,
    pub median: f32,
    pub std: f32,
}

/// astropy の sigma_clipped_stats(sigma=3、maxiters=5、中心は median、std は ddof=0)。
/// 有限でない値は最初に除く。境界は float32 で `cen - std*3`・`cen + std*3`、残すのは境界を含む内側。
/// 何も変わらなくなるか 5 回で止める。
pub fn sigma_clipped_stats(data: &[f32]) -> ClipStats {
    clip_core(data, true)
}

/// σ クリップの std だけ(前処理で使う。最後の平均・中央値を計算しない)。
pub fn sigma_clipped_std(data: &[f32]) -> f32 {
    clip_core(data, false).std
}

/// 大きい配列の 1 回目の読み: numpy と同じ和(8192 個ずつのペアワイズ和を順に足す)と、鍵の上位 16 bit のヒストグラム。
/// 区切りの塊(super_len 個 = 8192 の倍数)ごとに並列に計算し、和は先頭の区切りから順に足す。
fn pass_sum_hist(cur: &[f32]) -> (f32, Vec<u32>) {
    let parts: Vec<(Vec<f32>, Vec<u32>)> = cur
        .par_chunks(super_len(cur.len()))
        .map(|sc| {
            let mut h = vec![0u32; 65536];
            let sums: Vec<f32> = sc
                .chunks(NP_BUF)
                .map(|b| {
                    for &x in b {
                        h[(f32_key(x) >> 16) as usize] += 1;
                    }
                    pw_map(b, &|x| x)
                })
                .collect();
            (sums, h)
        })
        .collect();
    let mut res = 0.0f32;
    for (sums, _) in &parts {
        for &v in sums {
            res += v;
        }
    }
    (res, merge_hists(&parts.iter().map(|p| &p.1).collect::<Vec<_>>()))
}

/// 大きい配列の中央値と std(numpy の median・std と同じ値)を、1 回目の読み(和・上位ヒストグラム)の結果と
/// もう 1 回の読み(二乗の和・中央値の入る上位ビンの下位ヒストグラム)で求める。
fn median_std_big(cur: &[f32], sum: f32, h1: &[u32]) -> (f32, f32) {
    let n = cur.len();
    let m = (sum as f64 / n as f64) as f32;
    let even = n.is_multiple_of(2);
    let k = if even { n / 2 - 1 } else { n / 2 };
    // k 番目の入る上位ビン
    let mut cum = 0usize;
    let mut b1 = 0usize;
    for (i, &c) in h1.iter().enumerate() {
        if cum + c as usize > k {
            b1 = i;
            break;
        }
        cum += c as usize;
    }
    let base = cum;
    let next_same_bin = even && k + 1 - base < h1[b1] as usize;
    // k+1 番目が別の上位ビンにあるなら、その次の空でないビンの最小値が要る
    let b2: Option<u32> =
        if even && !next_same_bin { (b1 + 1..65536).find(|&i| h1[i] > 0).map(|i| i as u32) } else { None };
    let b1u = b1 as u32;
    let parts: Vec<(Vec<f32>, Vec<u32>, u32)> = cur
        .par_chunks(super_len(n))
        .map(|sc| {
            let mut h = vec![0u32; 65536];
            let mut mn = u32::MAX;
            let sums: Vec<f32> = sc
                .chunks(NP_BUF)
                .map(|b| {
                    for &x in b {
                        let key = f32_key(x);
                        let top = key >> 16;
                        if top == b1u {
                            h[(key & 0xFFFF) as usize] += 1;
                        } else if Some(top) == b2 && key < mn {
                            mn = key;
                        }
                    }
                    pw_map(b, &|x| {
                        let d = x - m;
                        d * d
                    })
                })
                .collect();
            (sums, h, mn)
        })
        .collect();
    let mut ss = 0.0f32;
    let mut mn = u32::MAX;
    for (sums, _, m2) in &parts {
        for &v in sums {
            ss += v;
        }
        mn = mn.min(*m2);
    }
    let h2 = merge_hists(&parts.iter().map(|p| &p.1).collect::<Vec<_>>());
    let std = ((ss as f64 / n as f64) as f32).sqrt();
    // 下位ヒストグラムで k 番目
    let r = k - base;
    let mut cum2 = 0usize;
    let mut lo = 0usize;
    for (i, &c) in h2.iter().enumerate() {
        if cum2 + c as usize > r {
            lo = i;
            break;
        }
        cum2 += c as usize;
    }
    let kv = key_to_f32((b1u << 16) | lo as u32);
    if !even {
        return (0.0f32 + kv, std);
    }
    let next = if next_same_bin {
        if cum2 + h2[lo] as usize > r + 1 {
            kv
        } else {
            let l2 = (lo + 1..65536).find(|&i| h2[i] > 0).unwrap_or(lo);
            key_to_f32((b1u << 16) | l2 as u32)
        }
    } else if mn != u32::MAX {
        key_to_f32(mn)
    } else {
        kv
    };
    let s = (0.0f32 + kv) + next;
    ((s as f64 / 2.0) as f32, std)
}

/// 中央値・ヒストグラムを読む回数を減らした版(pass_sum_hist + median_std_big)を使う長さ。
/// median_finite_f32 がヒストグラムで選ぶ長さ(kth_pair の select_radix)と同じ範囲。
#[inline]
fn is_big(n: usize) -> bool {
    (RADIX_MIN..=u32::MAX as usize).contains(&n)
}

/// 大きい配列の読み(pass_sum_hist・median_std_big)が配列を分ける塊の数(n > 0)。読みの間に同時に生きる
/// ヒストグラムは (この数 + 2) 本(1 本 65536 × u32 = 256 KiB)。塊の数は長さに単調でない: super_len の切り上げのため、
/// 長さが 8192 × p(p は塊の目標数 = clamp(n / 65536, 1, 2 × スレッド数))の倍数ちょうどのとき塊が p 個で、
/// 1 つ長いと 1〜2 個減ることがある(例: 20 スレッドで 655360 個は 10 個、655361 個は 9 個)。
fn n_parts(n: usize) -> usize {
    n.div_ceil(super_len(n))
}

fn clip_core(data: &[f32], full: bool) -> ClipStats {
    // 残す値の配列は回ごとにちょうどの長さで新しく作り、作り終えてから前の配列を捨てる(2026-10-24 までの版と同じ確保の仕方)。
    // 同時に生きるのは、書き出しの瞬間の新旧 2 本か、統計の読みの間の 1 本 + ヒストグラム((塊の数 + 2) × 256 KiB、
    // 20 スレッドで最大 42 本 = 10.5 MiB)のどちらか。使い終えた配列を次の書き先に取っておく形(配列 2 本の使い回し)は、
    // 3 回目からの統計の読みで配列 2 本とヒストグラムが重なり、私有メモリの最大がヒストグラムの分だけ増えるので採らない。
    let all_finite = data.par_iter().all(|x| x.is_finite());
    let mut owned: Option<Vec<f32>> = if all_finite { None } else { Some(filter_ordered(data, |x| x.is_finite())) };
    // 直近に境界を計算した配列の std と、大きい配列ならその回の (中心, 和)。最後に変化が無かったときに使い回す
    let mut last: Option<(f32, Option<(f32, f32)>)> = None;
    // ループの最後の統計の読みが大きい配列の版だったときの塊の数(clip_finish で最後の読みのメモリを比べる)
    let mut last_parts: Option<usize> = None;
    for _ in 0..5 {
        let cur: &[f32] = owned.as_deref().unwrap_or(data);
        let size = cur.len();
        // 大きい配列は読む回数を減らした版(値は同じ)。小さい配列はそのまま
        let (cen, sd, sum) = if is_big(size) {
            last_parts = Some(n_parts(size));
            let (sum, h1) = pass_sum_hist(cur);
            let (c, s) = median_std_big(cur, sum, &h1);
            (c, s, Some(sum))
        } else {
            last_parts = None;
            (median_finite_f32(cur), std_f32(cur), None)
        };
        let lo = cen - sd * 3.0f32;
        let hi = cen + sd * 3.0f32;
        let keep = |x: f32| x >= lo && x <= hi;
        let counts = count_kept(cur, &keep);
        let kept: usize = counts.iter().sum();
        if kept == size {
            last = Some((sd, sum.map(|s| (cen, s))));
            break;
        }
        let next = filter_with_counts(cur, keep, &counts);
        owned = Some(next);
        last = None;
    }
    let cur: &[f32] = owned.as_deref().unwrap_or(data);
    clip_finish(cur, full, last, last_parts)
}

/// σ クリップの最後の値。cur は最後の配列、last は収束して抜けたときのその回の (std, 大きい配列なら (中心, 和))、
/// last_parts はループの最後の統計の読みが大きい配列の版だったときの塊の数。
fn clip_finish(
    cur: &[f32],
    full: bool,
    last: Option<(f32, Option<(f32, f32)>)>,
    last_parts: Option<usize>,
) -> ClipStats {
    let n = cur.len();
    match last {
        // 収束して抜けた大きい配列: その回の値がそのまま答え(cur はその回と同じ配列)。
        // median_std_big の中心は median_finite_f32 と、pass_sum_hist の和は sum_f32 とビット一致する
        // (fused_median_std_matches)。mean_f32 は (sum_f32 as f64 / n as f64) as f32 なので同じ式で作る。何も確保しない
        Some((sd, Some((cen, sum)))) if full => {
            ClipStats { mean: (sum as f64 / n as f64) as f32, median: cen, std: sd }
        }
        Some((sd, _)) if full => ClipStats { mean: mean_f32(cur), median: median_finite_f32(cur), std: sd },
        Some((sd, _)) => ClipStats { mean: f32::NAN, median: f32::NAN, std: sd },
        // 5 回で収束しなかった大きい配列: 最後の配列の中央値・std・平均を 2 回の読みで出す(値は上と同じ理由で同じ)。
        // メモリ: ループの最後(5 回目)の読みは、cur より長い 1 つ前の配列とヒストグラム (last_parts + 2) 本を持っていた。
        // cur の塊の数がそれ以下なら、この読み(cur + (塊の数 + 2) 本)はそれより小さく、前の版の最大を超えない。
        // 塊の数は長さに単調でない(n_parts)ので、超えるときは前の版と同じ計算にする(値は同じ)
        None if full && is_big(n) && last_parts.is_some_and(|p| n_parts(n) <= p) => {
            let (sum, h1) = pass_sum_hist(cur);
            let (med, sd) = median_std_big(cur, sum, &h1);
            ClipStats { mean: (sum as f64 / n as f64) as f32, median: med, std: sd }
        }
        None if full => ClipStats { mean: mean_f32(cur), median: median_finite_f32(cur), std: std_f32(cur) },
        None => ClipStats { mean: f32::NAN, median: f32::NAN, std: std_f32(cur) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_order() {
        let v = [-f32::INFINITY, -3.5, -1e-30, 0.0, 1e-30, 2.0, f32::INFINITY];
        for w in v.windows(2) {
            assert!(f32_key(w[0]) < f32_key(w[1]));
            assert_eq!(key_to_f32(f32_key(w[0])), w[0]);
        }
    }

    #[test]
    fn fused_median_std_matches() {
        // 中央の 2 つが同じ上位ビン・別の上位ビン・奇数個のどれでも、ふつうの経路と同じ値
        // 600001 個は塊が複数(super_len)になる大きさ
        for (n, scale) in
            [(70001usize, 0.37f32), (70000, 0.37), (65536, 1e-3), (100000, 3.0e3), (40000, 1.0), (600001, 0.011)]
        {
            let mut a: Vec<f32> = (0..n).map(|i| (((i * 7919) % 10007) as f32 - 5000.0) * scale).collect();
            if n == 40000 {
                // 半分を 1.0、半分を 2.0(中央の 2 つが別の上位ビン)
                for (i, v) in a.iter_mut().enumerate() {
                    *v = if i % 2 == 0 { 1.0 } else { 2.0 };
                }
            }
            let (sum, h1) = pass_sum_hist(&a);
            assert_eq!(sum.to_bits(), sum_f32(&a).to_bits(), "n={n}");
            let (med, sd) = median_std_big(&a, sum, &h1);
            assert_eq!(med.to_bits(), median_finite_f32(&a).to_bits(), "n={n}");
            assert_eq!(sd.to_bits(), std_f32(&a).to_bits(), "n={n}");
        }
    }

    /// 決まった擬似乱数(xorshift)の [0, 1)
    fn rng(seed: u64) -> impl FnMut() -> f64 {
        let mut s = seed | 1;
        move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// 2026-10-24 までの clip_core(最後の平均・中央値・std を毎回別々に計算し直す版)。
    /// 照合の基準。戻り値の 2 つ目は回った回数と、収束して抜けたか。
    fn clip_core_ref(data: &[f32], full: bool) -> (ClipStats, (usize, bool)) {
        fn filter_ref<P: Fn(f32) -> bool + Sync>(a: &[f32], keep: P, counts: &[usize]) -> Vec<f32> {
            let total: usize = counts.iter().sum();
            let mut out = vec![0.0f32; total];
            let mut parts: Vec<&mut [f32]> = Vec::with_capacity(counts.len());
            let mut rest: &mut [f32] = &mut out;
            for &c in counts {
                let (h, t) = rest.split_at_mut(c);
                parts.push(h);
                rest = t;
            }
            a.par_chunks(CHUNK).zip(parts.into_par_iter()).for_each(|(c, o)| {
                let mut j = 0;
                for &x in c {
                    if keep(x) {
                        o[j] = x;
                        j += 1;
                    }
                }
            });
            out
        }
        let all_finite = data.par_iter().all(|x| x.is_finite());
        let mut owned: Option<Vec<f32>> = if all_finite {
            None
        } else {
            let counts: Vec<usize> =
                data.par_chunks(CHUNK).map(|c| c.iter().filter(|&&x| x.is_finite()).count()).collect();
            Some(filter_ref(data, |x| x.is_finite(), &counts))
        };
        let mut last_std: Option<f32> = None;
        let mut iters = (0, false);
        for _ in 0..5 {
            iters.0 += 1;
            let cur: &[f32] = owned.as_deref().unwrap_or(data);
            let size = cur.len();
            let (cen, sd) = if (RADIX_MIN..=u32::MAX as usize).contains(&size) {
                let (sum, h1) = pass_sum_hist(cur);
                median_std_big(cur, sum, &h1)
            } else {
                (median_finite_f32(cur), std_f32(cur))
            };
            let lo = cen - sd * 3.0f32;
            let hi = cen + sd * 3.0f32;
            let keep = |x: f32| x >= lo && x <= hi;
            let counts: Vec<usize> = cur.par_chunks(CHUNK).map(|c| c.iter().filter(|&&x| keep(x)).count()).collect();
            let kept: usize = counts.iter().sum();
            if kept == size {
                last_std = Some(sd);
                iters.1 = true;
                break;
            }
            let next = filter_ref(cur, keep, &counts);
            owned = Some(next);
            last_std = None;
        }
        let cur: &[f32] = owned.as_deref().unwrap_or(data);
        let std = match last_std {
            Some(s) => s,
            None => std_f32(cur),
        };
        let st = if full {
            ClipStats { mean: mean_f32(cur), median: median_finite_f32(cur), std }
        } else {
            ClipStats { mean: f32::NAN, median: f32::NAN, std }
        };
        (st, iters)
    }

    #[test]
    fn clip_matches_reference() {
        // 最後の値を使い回す枝(収束して抜けた大きい配列・5 回回り切った大きい配列)を含め、前の版とビット単位で同じ。
        // 5 回で収束しない大小の配列・途中で収束する配列・非有限値混じり・同じ値ばかり・区切りの前後の長さ
        let mut cases: Vec<Vec<f32>> = Vec::new();
        for &n in &[0usize, 1, 2, 7, 100, 8191, 8192, 8193, 32767, 32768, 32769, 65535, 65536, 65537, 200_001] {
            let mut r = rng(n as u64 + 3);
            // ほぼ正規分布(4 つの一様の和)。3σ で切ると毎回少しずつ外れて、5 回では収束しないことが多い
            let g: Vec<f32> = (0..n).map(|_| ((r() + r() + r() + r() - 2.0) * 10.0 + 100.0) as f32).collect();
            cases.push(g.clone());
            // 外れ値入り(7 個に 1 個)
            cases
                .push(g.iter().enumerate().map(|(i, &v)| if i % 7 == 0 { v + (i % 5000) as f32 } else { v }).collect());
            // 非有限値入り
            cases.push(
                g.iter()
                    .enumerate()
                    .map(|(i, &v)| match i % 97 {
                        0 => f32::NAN,
                        13 => f32::INFINITY,
                        41 => f32::NEG_INFINITY,
                        _ => v,
                    })
                    .collect(),
            );
            // 一様分布(数回で収束する)
            cases.push((0..n).map(|_| r() as f32).collect());
        }
        let mut r = rng(99);
        cases.push((0..1_000_000).map(|i| if i % 7 == 0 { 5000.0 } else { (r() * 50.0) as f32 }).collect());
        cases.push((0..2_000_000).map(|_| ((r() + r() + r() + r() - 2.0) * 3.0) as f32).collect());
        cases.push(vec![3.5f32; 100_000]);
        cases.push(vec![3.5f32; 100]);
        cases.push(vec![f32::NAN; 70_000]);
        cases.push(vec![f32::INFINITY; 50]);
        cases.push((0..80_000).map(|i| if i % 2 == 0 { 1.0 } else { 2.0 }).collect());
        let (mut n_unconv_big, mut n_conv_big) = (0, 0);
        for a in &cases {
            for full in [true, false] {
                let (want, (iters, conv)) = clip_core_ref(a, full);
                let got = clip_core(a, full);
                let n = a.len();
                assert_eq!(got.mean.to_bits(), want.mean.to_bits(), "mean n={n} full={full}");
                assert_eq!(got.median.to_bits(), want.median.to_bits(), "median n={n} full={full}");
                assert_eq!(got.std.to_bits(), want.std.to_bits(), "std n={n} full={full}");
                if n >= RADIX_MIN && full {
                    if conv {
                        n_conv_big += 1;
                    } else {
                        assert_eq!(iters, 5);
                        n_unconv_big += 1;
                    }
                }
            }
        }
        // 両方の枝(収束して抜けた・5 回回り切った)を大きい配列で通したこと
        assert!(n_unconv_big >= 3 && n_conv_big >= 3, "収束しない {n_unconv_big} / 収束 {n_conv_big}");
    }

    #[test]
    fn clip_finish_both_paths_match() {
        // 5 回で収束しなかった大きい配列の最後の計算: 2 回の読み(塊の数が前の読み以下のとき)と、
        // 前の版と同じ計算(塊が増えるとき)のどちらを通っても、前の版の値とビット単位で同じ
        let mut r = rng(17);
        for n in [RADIX_MIN, 65_536, 200_001, 655_360, 655_361, 1_310_720] {
            let a: Vec<f32> = (0..n).map(|_| ((r() + r() + r() - 1.5) * 7.0 + 30.0) as f32).collect();
            let want = ClipStats { mean: mean_f32(&a), median: median_finite_f32(&a), std: std_f32(&a) };
            // last_parts = MAX なら 2 回の読み、Some(0) と None なら前の版と同じ計算
            for lp in [Some(usize::MAX), Some(0), None] {
                let got = clip_finish(&a, true, None, lp);
                assert_eq!(got.mean.to_bits(), want.mean.to_bits(), "mean n={n} {lp:?}");
                assert_eq!(got.median.to_bits(), want.median.to_bits(), "median n={n} {lp:?}");
                assert_eq!(got.std.to_bits(), want.std.to_bits(), "std n={n} {lp:?}");
            }
            let got = clip_finish(&a, false, None, Some(usize::MAX));
            assert!(got.mean.is_nan() && got.median.is_nan());
            assert_eq!(got.std.to_bits(), want.std.to_bits(), "std n={n} full=false");
        }
        // 塊の数が長さに単調でないこと(clip_finish が比べる理由)。スレッド数を決めて確かめる
        let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build().expect("rayon のスレッドの束");
        pool.install(|| {
            assert_eq!(n_parts(655_360), 10);
            assert_eq!(n_parts(655_361), 9);
        });
    }

    #[test]
    fn unmasked_matches_nan_marking() {
        // 前処理の最後の σ: 前の版はマスクの画素を NaN にしてから NaN でないものを集めていた。
        // マスクでない位置に NaN が無ければ配列が同じ、あっても σ クリップ(最初に非有限値を順序を保って除く)は同じ
        let mut r = rng(8);
        for (n, with_nan) in [(1000usize, false), (300_000, false), (300_000, true), (65_536, true)] {
            let mut a: Vec<f32> = (0..n).map(|_| ((r() + r() + r() - 1.5) * 20.0) as f32).collect();
            if with_nan {
                for (i, v) in a.iter_mut().enumerate() {
                    if i % 101 == 0 {
                        *v = f32::NAN;
                    } else if i % 103 == 0 {
                        *v = f32::INFINITY;
                    }
                }
            }
            let mask: Vec<bool> = (0..n).map(|i| (i / 3000) % 4 == 1 || i % 17 == 0).collect();
            let pairs: Vec<f32> = a.iter().zip(&mask).map(|(&s, &m)| if m { f32::NAN } else { s }).collect();
            let old = filter_ordered(&pairs, |v| !v.is_nan());
            let new = unmasked(&a, &mask);
            assert_eq!(new.len(), mask.iter().filter(|&&m| !m).count());
            if !with_nan {
                assert_eq!(new.len(), old.len());
                assert!(new.iter().zip(&old).all(|(x, y)| x.to_bits() == y.to_bits()));
            }
            assert_eq!(sigma_clipped_std(&new).to_bits(), sigma_clipped_std(&old).to_bits(), "n={n}");
        }
        // 長さが違えば短いほうまで(落ちない)
        assert_eq!(unmasked(&[1.0, 2.0, 3.0], &[false, true]), vec![1.0]);
        assert!(unmasked(&[], &[false]).is_empty());
    }

    #[test]
    fn radix_matches_sort() {
        let n = 70001usize;
        let a: Vec<f32> = (0..n).map(|i| (((i * 7919) % 10007) as f32 - 5000.0) * 0.37).collect();
        let mut s = a.clone();
        s.sort_by(|x, y| x.total_cmp(y));
        for &k in &[0usize, 1, 17, n / 2 - 1, n / 2, n - 2, n - 1] {
            let (x, y) = select_radix(&a, k);
            assert_eq!(x, s[k], "k={k}");
            assert_eq!(y, s[(k + 1).min(n - 1)], "k+1={k}");
            let (x2, y2) = select_small(&a, k);
            assert_eq!((x, y), (x2, y2));
        }
    }
}

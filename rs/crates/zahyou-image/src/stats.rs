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
/// σ クリップの読みを並列にする塊(8192 の倍数。和の区切りがそろう)
const SUPER: usize = NP_BUF * 64;

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
    let counts: Vec<usize> = a.par_chunks(CHUNK).map(|c| c.iter().filter(|&&x| keep(x)).count()).collect();
    filter_with_counts(a, keep, &counts)
}

/// `filter_ordered` の書き出し(区切り CHUNK ごとの個数を数え済みのとき)。
fn filter_with_counts<P: Fn(f32) -> bool + Sync>(a: &[f32], keep: P, counts: &[usize]) -> Vec<f32> {
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
/// 区切りの塊(SUPER 個 = 8192 の倍数)ごとに並列に計算し、和は先頭の区切りから順に足す。
fn pass_sum_hist(cur: &[f32]) -> (f32, Vec<u32>) {
    let parts: Vec<(Vec<f32>, Vec<u32>)> = cur
        .par_chunks(SUPER)
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
    let mut h1 = vec![0u32; 65536];
    for (sums, h) in &parts {
        for &v in sums {
            res += v;
        }
        for (a, b) in h1.iter_mut().zip(h.iter()) {
            *a += *b;
        }
    }
    (res, h1)
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
        .par_chunks(SUPER)
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
    let mut h2 = vec![0u32; 65536];
    let mut mn = u32::MAX;
    for (sums, h, m2) in &parts {
        for &v in sums {
            ss += v;
        }
        for (a, b) in h2.iter_mut().zip(h.iter()) {
            *a += *b;
        }
        mn = mn.min(*m2);
    }
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

fn clip_core(data: &[f32], full: bool) -> ClipStats {
    let all_finite = data.par_iter().all(|x| x.is_finite());
    let mut owned: Option<Vec<f32>> = if all_finite { None } else { Some(filter_ordered(data, |x| x.is_finite())) };
    // 直近の境界を計算した配列の std(最後に変化が無かったときに使い回す)
    let mut last_std: Option<f32> = None;
    for _ in 0..5 {
        let cur: &[f32] = owned.as_deref().unwrap_or(data);
        let size = cur.len();
        // 大きい配列は読む回数を減らした版(値は同じ)。小さい配列はそのまま
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
            break;
        }
        let next = filter_with_counts(cur, keep, &counts);
        owned = Some(next);
        last_std = None;
    }
    let cur: &[f32] = owned.as_deref().unwrap_or(data);
    let std = match last_std {
        Some(s) => s,
        None => std_f32(cur),
    };
    if full {
        ClipStats { mean: mean_f32(cur), median: median_finite_f32(cur), std }
    } else {
        ClipStats { mean: f32::NAN, median: f32::NAN, std }
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
        for (n, scale) in [(70001usize, 0.37f32), (70000, 0.37), (65536, 1e-3), (100000, 3.0e3), (40000, 1.0)] {
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

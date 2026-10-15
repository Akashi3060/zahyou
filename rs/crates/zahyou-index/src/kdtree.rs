//! kd-tree(ファイルに入っている木をそのまま使う。作り直さない)。
//!
//! 節点はヒープ順(根 = 0、子 = 2i+1 と 2i+2)。内部節点は 0 … ninterior−1、葉はその後ろで全部同じ深さ。
//! 葉 k のデータは lr[k−1]+1 … lr[k](lr[−1] = −1)。split の下位 dim_bits ビットが分割次元、
//! 残り(下位を 0 にしたもの)が分割位置。左の部分木は x[dim] ≤ 位置、右は x[dim] ≥ 位置(両側とも等号あり)。
//! 実数 = minval + 整数 / scale(index-format.md 6 節)。

use rayon::prelude::*;

use crate::layout::{KdType, TreeLayout};
use crate::pyfmt::float_repr;

/// 開くときの表の検査を並列にする区切り(u32 で 64 KiB = mmap のページ 16 枚)。
const CHECK_CHUNK: usize = 16384;

/// 木の整数の型(u16 か u32)。
pub(crate) trait KdInt: bytemuck::Pod + Send + Sync {
    fn to_u64(self) -> u64;
    fn to_f64(self) -> f64;
}

impl KdInt for u16 {
    #[inline(always)]
    fn to_u64(self) -> u64 {
        self as u64
    }
    #[inline(always)]
    fn to_f64(self) -> f64 {
        self as f64
    }
}

impl KdInt for u32 {
    #[inline(always)]
    fn to_u64(self) -> u64 {
        self as u64
    }
    #[inline(always)]
    fn to_f64(self) -> f64 {
        self as f64
    }
}

/// 型付きの整数の並び(mmap の上のビュー)。
#[derive(Clone, Copy)]
pub(crate) enum Ints<'a> {
    U16(&'a [u16]),
    U32(&'a [u32]),
}

impl<'a> Ints<'a> {
    /// バイト列を型付きのビューにする(位置が揃っていない・長さが合わないときは Err)。
    pub(crate) fn cast(bytes: &'a [u8], ty: KdType) -> Result<Ints<'a>, String> {
        let err = |e: bytemuck::PodCastError| format!("表を整数の並びとして読めません({e:?})");
        Ok(match ty {
            KdType::U16 => Ints::U16(bytemuck::try_cast_slice(bytes).map_err(err)?),
            KdType::U32 => Ints::U32(bytemuck::try_cast_slice(bytes).map_err(err)?),
        })
    }

    #[inline]
    pub(crate) fn get_u64(&self, i: usize) -> Option<u64> {
        match self {
            Ints::U16(s) => s.get(i).map(|v| *v as u64),
            Ints::U32(s) => s.get(i).map(|v| *v as u64),
        }
    }
}

/// 木 1 本の形と、整数 ↔ 実数の変換の値。表の中身は持たない(mmap から毎回ビューを作る)。
#[derive(Clone, Debug)]
pub(crate) struct TreeMeta {
    pub ndim: usize,
    pub levels: usize,
    pub ninterior: usize,
    pub mask: u64,
    /// 根の箱は整数の全域 [0, 2^bits − 1]
    pub full: f64,
    pub minval: [f64; 4],
    pub scale: f64,
    pub int_type: KdType,
    pub data_type: KdType,
    pub lr: (usize, usize),
    pub split: (usize, usize),
    pub data: (usize, usize),
}

/// mmap の上の木 1 本(ビュー)。
#[derive(Clone, Copy)]
pub(crate) struct Tree<'a> {
    pub meta: &'a TreeMeta,
    pub lr: &'a [u32],
    pub split: Ints<'a>,
    pub data: Ints<'a>,
}

fn sub(bytes: &[u8], off: u64, len: u64) -> Result<&[u8], String> {
    let s = usize::try_from(off).map_err(|_| "表の位置がファイルの外です".to_string())?;
    let n = usize::try_from(len).map_err(|_| "表の位置がファイルの外です".to_string())?;
    let e = s.checked_add(n).ok_or_else(|| "表の位置がファイルの外です".to_string())?;
    bytes.get(s..e).ok_or_else(|| "表の位置がファイルの外です".to_string())
}

impl TreeMeta {
    /// 検査済みのヘッダー(lay)と mmap したバイト列から木を組み立て、表の中身を検査する。
    ///
    /// Python 版 SvKdTree と同じ検査(範囲の表が有限で scale > 0)に加えて、Rust で安全に読むための検査
    /// (lr が単調でデータの数と合う、分割次元が次元の数より小さい、表が型の境界に揃っている)。
    pub(crate) fn build(name: &str, lay: &TreeLayout, bytes: &[u8]) -> Result<TreeMeta, String> {
        let d = lay.ndim;
        if d == 0 || d > 4 {
            return Err(format!("{name} の木の次元 {d} は扱えません"));
        }
        let nn = lay.nnodes;
        let levels = (usize::BITS - nn.leading_zeros()) as usize; // nn = 2^levels − 1
        let ninterior = (nn - 1) / 2;
        let nleaves = nn.div_ceil(2);
        let dim_bits = (usize::BITS - (d - 1).leading_zeros()).max(1); // ⌈log₂D⌉(D = 2 で 1、3・4 で 2)
        let mask = (1u64 << dim_bits) - 1;
        let full = match lay.data_type {
            KdType::U16 => u16::MAX as f64,
            KdType::U32 => u32::MAX as f64,
        };

        // 範囲の表: minval[D], maxval[D], scale(小さいので写して読む)
        let rb = sub(bytes, lay.range, 8 * (2 * d as u64 + 1))?;
        let rng: Vec<f64> = rb
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect();
        let scale = rng[2 * d];
        if !(rng.iter().all(|v| v.is_finite()) && scale > 0.0) {
            return Err(format!("{name} の木の範囲の表がおかしい(scale = {})", float_repr(scale)));
        }
        let mut minval = [0.0; 4];
        minval[..d].copy_from_slice(&rng[..d]);

        let ilen = lay.int_type.bytes();
        let dlen = lay.data_type.bytes();
        let span = |off: u64, n: u64| -> Result<(usize, usize), String> {
            sub(bytes, off, n)?;
            Ok((off as usize, n as usize))
        };
        let meta = TreeMeta {
            ndim: d,
            levels,
            ninterior,
            mask,
            full,
            minval,
            scale,
            int_type: lay.int_type,
            data_type: lay.data_type,
            lr: span(lay.lr, 4 * nleaves as u64)?,
            split: span(lay.split, ilen * ninterior as u64)?,
            data: span(lay.data, dlen * lay.ndata as u64 * d as u64)?,
        };
        let t = meta.view(bytes)?;

        // 表の全件を見る検査は、時間の大半が mmap のページの初回の読み込みなので、区切って並列にする
        // (4107 で 0.9 → 0.5 ms。結果は 1 本のスレッドで見たときと同じ)
        //
        // lr: 葉 k は lr[k−1]+1 … lr[k]。単調(空の葉は許す)で、最後がデータの数 − 1
        let n = lay.ndata as u64;
        let lr_ok = t.lr.par_chunks(CHECK_CHUNK).all(|c| c.is_sorted() && c.iter().all(|&v| (v as u64) < n))
            && (CHECK_CHUNK..t.lr.len()).step_by(CHECK_CHUNK).all(|i| t.lr[i - 1] <= t.lr[i]);
        if !lr_ok {
            return Err(format!("{name} の木の葉の表(lr)がおかしい"));
        }
        if t.lr.last().map(|&v| v as u64) != Some(n - 1) {
            return Err(format!("{name} の木の葉の表(lr)がデータの数と合いません"));
        }
        // 分割次元(D = 3 なら 2 ビットで 0〜3 が表せるので、3 が無いことを確かめる)
        if (mask + 1) as usize > d {
            let bad = |raw: u64| (raw & mask) as usize >= d;
            let found = match t.split {
                Ints::U16(s) => s.par_chunks(2 * CHECK_CHUNK).any(|c| c.iter().any(|&v| bad(v as u64))),
                Ints::U32(s) => s.par_chunks(CHECK_CHUNK).any(|c| c.iter().any(|&v| bad(v as u64))),
            };
            if found {
                return Err(format!("{name} の木の分割の表に次元の外の値があります"));
            }
        }
        Ok(meta)
    }

    /// mmap したバイト列の上に型付きのビューを作る。
    pub(crate) fn view<'a>(&'a self, bytes: &'a [u8]) -> Result<Tree<'a>, String> {
        let get = |(o, n): (usize, usize)| bytes.get(o..o + n).ok_or_else(|| "表の位置がファイルの外です".to_string());
        let lr: &[u32] = bytemuck::try_cast_slice(get(self.lr)?)
            .map_err(|e| format!("表を整数の並びとして読めません({e:?})"))?;
        Ok(Tree {
            meta: self,
            lr,
            split: Ints::cast(get(self.split)?, self.int_type)?,
            data: Ints::cast(get(self.data)?, self.data_type)?,
        })
    }

    /// 実数 → 木の単位(丸めない実数)。
    #[inline(always)]
    pub(crate) fn to_tree(&self, k: usize, x: f64) -> f64 {
        (x - self.minval[k]) * self.scale
    }

    /// 木の整数 → 実数。
    #[inline(always)]
    pub(crate) fn to_real(&self, k: usize, v: f64) -> f64 {
        self.minval[k] + v / self.scale
    }
}

/// 探索の途中の節点(箱と、問い合わせから箱までの次元ごとの距離の 2 乗)。
#[derive(Clone, Copy)]
pub(crate) struct Frame<const D: usize> {
    node: usize,
    lo: [f64; D],
    hi: [f64; D],
    g2: [f64; D],
}

/// 木の単位の問い合わせ qt から距離 √r2 以内に届く葉を、左から順に leaf(データの始め, 終わり) で知らせる。
///
/// 節点の箱は根の全域から split の位置で狭めていき、箱までの最短距離が √r2 を超えた枝は捨てる
/// (Python 版 ball_candidates と同じ規則。1 問ずつ深さ優先でたどる)。
#[inline]
pub(crate) fn ball_visit<S: KdInt, const D: usize>(
    meta: &TreeMeta,
    lr: &[u32],
    split: &[S],
    qt: &[f64; D],
    r2: f64,
    stack: &mut Vec<Frame<D>>,
    mut leaf: impl FnMut(usize, usize),
) {
    // f64::max は NaN を無視するので、NaN の問い合わせはどの枝も刈られず木を全部たどってしまう。
    // 答えは必ず空なので(Python 版も NaN で枝が刈られて空)、ここで返す
    if r2.is_nan() || qt.iter().any(|v| v.is_nan()) {
        return;
    }
    let mut root = Frame { node: 0, lo: [0.0; D], hi: [meta.full; D], g2: [0.0; D] };
    let mut gs = 0.0;
    for (k, &q) in qt.iter().enumerate() {
        let g = (root.lo[k] - q).max(q - root.hi[k]).max(0.0);
        root.g2[k] = g * g;
        gs += root.g2[k];
    }
    if gs > r2 {
        return;
    }
    stack.clear();
    stack.push(root);
    while let Some(f) = stack.pop() {
        if f.node >= meta.ninterior {
            // 葉(lr は開くときに単調でデータの数の内と確かめてある)
            let k = f.node - meta.ninterior;
            let Some(&last) = lr.get(k) else { continue };
            let start = if k == 0 {
                0
            } else {
                match lr.get(k - 1) {
                    Some(&p) => p as usize + 1,
                    None => continue,
                }
            };
            let end = last as usize + 1;
            if start <= end {
                leaf(start, end);
            }
            continue;
        }
        let Some(raw) = split.get(f.node).map(|v| v.to_u64()) else { continue };
        let dim = (raw & meta.mask) as usize;
        if dim >= D {
            continue; // 開くときに確かめてあるので起きない
        }
        let val = (raw & !meta.mask) as f64;
        let q = qt[dim];
        let mut rest = 0.0;
        for k in 0..D {
            if k != dim {
                rest += f.g2[k];
            }
        }
        // 右の子を先に積む(左から順に葉を出すため)
        let lo_r = f.lo[dim].max(val);
        let gr = (lo_r - q).max(q - f.hi[dim]).max(0.0);
        if rest + gr * gr <= r2 {
            let mut c = f;
            c.node = 2 * f.node + 2;
            c.lo[dim] = lo_r;
            c.g2[dim] = gr * gr;
            stack.push(c);
        }
        let hi_l = f.hi[dim].min(val);
        let gl = (f.lo[dim] - q).max(q - hi_l).max(0.0);
        if rest + gl * gl <= r2 {
            let mut c = f;
            c.node = 2 * f.node + 1;
            c.hi[dim] = hi_l;
            c.g2[dim] = gl * gl;
            stack.push(c);
        }
    }
}

/// コード木: 問い合わせ q(実数)から radius 以内のデータの番号を out に足す(昇順)。
///
/// 最終判定は Python 版と同じ式・同じ順(実数 c = minval + 整数/scale、Σ(c − q)² ≤ radius²)。
#[allow(clippy::too_many_arguments)] // 木の表(mmap のビュー)と作業用の領域をそのまま渡すため
pub(crate) fn ball_query<S: KdInt, T: KdInt, const D: usize>(
    meta: &TreeMeta,
    lr: &[u32],
    split: &[S],
    data: &[T],
    q: &[f64; D],
    radius: f64,
    r_t: f64,
    stack: &mut Vec<Frame<D>>,
    out: &mut Vec<u32>,
) {
    let mut qt = [0.0; D];
    for k in 0..D {
        qt[k] = meta.to_tree(k, q[k]);
    }
    let rr = radius * radius;
    let start_len = out.len();
    ball_visit(meta, lr, split, &qt, r_t * r_t, stack, |s, e| {
        let Some(rows) = data.get(s * D..e * D) else { return };
        for (i, row) in rows.chunks_exact(D).enumerate() {
            let mut d2 = 0.0;
            for k in 0..D {
                let t = meta.to_real(k, row[k].to_f64()) - q[k];
                d2 += t * t;
            }
            if d2 <= rr {
                out.push((s + i) as u32);
            }
        }
    });
    // 葉は左から順なので並んでいるはずだが、念のため
    let mine = &mut out[start_len..];
    if !mine.is_sorted() {
        mine.sort_unstable();
    }
}

/// 星の木: 単位ベクトル c(正規化済み)から弦 chord 以内の星の番号を out に足す。
///
/// 判定は Python 版と同じ(星を実数に戻して長さ 1 に正規化し、Σ(s − c)² ≤ chord²)。
pub(crate) fn cone_query<S: KdInt, T: KdInt>(
    meta: &TreeMeta,
    lr: &[u32],
    split: &[S],
    data: &[T],
    c: &[f64; 3],
    chord: f64,
    out: &mut Vec<u32>,
) {
    let mut qt = [0.0; 3];
    for k in 0..3 {
        qt[k] = meta.to_tree(k, c[k]);
    }
    // 量子化(3 × 0.25 目盛り²)と、正規化で動く分(|xyz| − 1 ≤ 4e-10 ≒ 0.9 目盛り)を広げる
    let r_t = ((chord * meta.scale).powi(2) + 0.75).sqrt() + 4.0;
    let cc = chord * chord;
    let mut stack = Vec::with_capacity(2 * meta.levels + 2);
    ball_visit(meta, lr, split, &qt, r_t * r_t, &mut stack, |s, e| {
        let Some(rows) = data.get(s * 3..e * 3) else { return };
        for (i, row) in rows.chunks_exact(3).enumerate() {
            let v = star_unit(meta, row);
            let mut d2 = 0.0;
            for k in 0..3 {
                let t = v[k] - c[k];
                d2 += t * t;
            }
            if d2 <= cc {
                out.push((s + i) as u32);
            }
        }
    });
}

/// 星の整数 3 つ → 長さ 1 に正規化した単位ベクトル(Python 版 star_xyz と同じ演算の順)。
#[inline(always)]
pub(crate) fn star_unit<T: KdInt>(meta: &TreeMeta, row: &[T]) -> [f64; 3] {
    let v = [
        meta.to_real(0, row[0].to_f64()),
        meta.to_real(1, row[1].to_f64()),
        meta.to_real(2, row[2].to_f64()),
    ];
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}


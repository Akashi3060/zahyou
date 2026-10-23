//! kd-tree(ファイルに入っている木をそのまま使う。作り直さない)。
//!
//! 節点はヒープ順(根 = 0、子 = 2i+1 と 2i+2)。内部節点は 0 … ninterior−1、葉はその後ろで全部同じ深さ。
//! 葉 k のデータは lr[k−1]+1 … lr[k](lr[−1] = −1)。split の下位 dim_bits ビットが分割次元、
//! 残り(下位を 0 にしたもの)が分割位置。左の部分木は x[dim] ≤ 位置、右は x[dim] ≥ 位置(両側とも等号あり)。
//! 実数 = minval + 整数 / scale(index-format.md 6 節)。

use std::cell::Cell;
use std::ops::ControlFlow;

use rayon::prelude::*;

use crate::layout::{KdType, TreeLayout};
use crate::pyfmt::float_repr;

/// 開くときの表の検査を並列にする区切り(u32 で 64 KiB = mmap のページ 16 枚)。
const CHECK_CHUNK: usize = 16384;

/// 葉の前ふるい(実数に戻す前に、木の単位で距離を見て遠いものを捨てる)を使ってよい木の範囲の表の値。
/// 前ふるいの余裕の根拠(丸め誤差の上限)はこの範囲を前提にしている(code_far・star_far の説明)。
/// 本物の索引は minval ≈ −1〜0・scale ≈ 2^31〜2^32(u32)か 2^15〜2^16(u16)で、どれもこの中に入る。
const PREFILTER_MINVAL_MAX: f64 = 4.0;
const PREFILTER_SCALE_MIN: f64 = 64.0;
const PREFILTER_SCALE_MAX: f64 = 68_719_476_736.0; // 2^36

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
    /// 葉の前ふるいを使ってよいか(範囲の表が max|minval| ≤ 4、64 ≤ scale ≤ 2^36。[`prefilter_allowed`])。
    /// 使わない木では、今までどおり葉の全部を実数に戻して判定する(答えはどちらでも同じ)
    pub prefilter_ok: bool,
}

/// 範囲の表が、葉の前ふるいの余裕の根拠の前提(code_far・star_far の説明)に入るか。
pub(crate) fn prefilter_allowed(minval: &[f64], scale: f64) -> bool {
    minval.iter().all(|m| m.abs() <= PREFILTER_MINVAL_MAX) && (PREFILTER_SCALE_MIN..=PREFILTER_SCALE_MAX).contains(&scale)
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
            prefilter_ok: prefilter_allowed(&minval[..d], scale),
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
/// leaf が Break を返したら、そこでたどるのをやめて Break を返す。
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
    mut leaf: impl FnMut(usize, usize) -> ControlFlow<()>,
) -> ControlFlow<()> {
    // f64::max は NaN を無視するので、NaN の問い合わせはどの枝も刈られず木を全部たどってしまう。
    // 答えは必ず空なので(Python 版も NaN で枝が刈られて空)、ここで返す
    if r2.is_nan() || qt.iter().any(|v| v.is_nan()) {
        return ControlFlow::Continue(());
    }
    let mut root = Frame { node: 0, lo: [0.0; D], hi: [meta.full; D], g2: [0.0; D] };
    let mut gs = 0.0;
    for (k, &q) in qt.iter().enumerate() {
        let g = (root.lo[k] - q).max(q - root.hi[k]).max(0.0);
        root.g2[k] = g * g;
        gs += root.g2[k];
    }
    if gs > r2 {
        return ControlFlow::Continue(());
    }
    // 今の節点 f を持ってたどる。子が両方通るときだけ右をスタックに積んで左へ進み、片方だけなら積まずにその子へ進む
    // (節点ごとに積んで降ろすのをやめる)。左の部分木を全部たどってから右へ進むのは、左右とも積んで左から降ろす形と
    // 同じなので、葉を知らせる順(左から)も、子の箱と距離の式も今までと同じ(試験 ball_visit_matches_reference)
    stack.clear();
    let mut f = root;
    loop {
        if f.node >= meta.ninterior {
            // 葉(lr は開くときに単調でデータの数の内と確かめてある)
            let k = f.node - meta.ninterior;
            if let Some(&last) = lr.get(k) {
                let start = if k == 0 { Some(0) } else { lr.get(k - 1).map(|&p| p as usize + 1) };
                if let Some(start) = start {
                    let end = last as usize + 1;
                    if start <= end {
                        leaf(start, end)?;
                    }
                }
            }
        } else if let Some(raw) = split.get(f.node).map(|v| v.to_u64())
            // 分割次元が次元の外になることは、開くときに確かめてあるので起きない
            && ((raw & meta.mask) as usize) < D
        {
            let dim = (raw & meta.mask) as usize;
            let val = (raw & !meta.mask) as f64;
            let q = qt[dim];
            let mut rest = 0.0;
            for k in 0..D {
                if k != dim {
                    rest += f.g2[k];
                }
            }
            let lo_r = f.lo[dim].max(val);
            let gr = (lo_r - q).max(q - f.hi[dim]).max(0.0);
            let go_r = rest + gr * gr <= r2;
            let hi_l = f.hi[dim].min(val);
            let gl = (f.lo[dim] - q).max(q - hi_l).max(0.0);
            let go_l = rest + gl * gl <= r2;
            if go_r {
                // 右の子(今の節点から作る)
                let mut right = f;
                right.node = 2 * f.node + 2;
                right.lo[dim] = lo_r;
                right.g2[dim] = gr * gr;
                if !go_l {
                    f = right;
                    continue;
                }
                stack.push(right);
            }
            if go_l {
                // 左の子へ(今の節点を書き換える。右の子は書き換える前に作ってある)
                f.node = 2 * f.node + 1;
                f.hi[dim] = hi_l;
                f.g2[dim] = gl * gl;
                continue;
            }
        }
        // 葉か、子がどちらも通らない: 積んでおいた右の子へ
        match stack.pop() {
            Some(n) => f = n,
            None => break,
        }
    }
    ControlFlow::Continue(())
}

/// コードの実数の判定(Python 版と同じ式・同じ順): 実数 c = minval + 整数/scale、Σ(c − q)² ≤ rr(= radius²)。
#[inline(always)]
fn code_in<T: KdInt, const D: usize>(meta: &TreeMeta, row: &[T], q: &[f64; D], rr: f64) -> bool {
    let mut d2 = 0.0;
    for k in 0..D {
        let t = meta.to_real(k, row[k].to_f64()) - q[k];
        d2 += t * t;
    }
    d2 <= rr
}

/// コードの前ふるいのしきい t² (木の単位)。t = radius·scale·(1 + 1e-12) + 1 目盛り。[`code_far`] の説明。
#[inline(always)]
fn code_far_t2(meta: &TreeMeta, radius: f64) -> f64 {
    let t = radius * meta.scale * (1.0 + 1e-12) + 1.0;
    t * t
}

/// コードの前ふるい: 実数の判定([`code_in`])で必ず外れるコードか。true なら判定せずに捨ててよい。
///
/// 木の単位(整数のまま)で問い合わせ qt との距離² dt2 = Σ(整数 − qt)² を見て、t2 より遠ければ捨てる。
/// 余裕の根拠: a = radius·scale とする。実数の判定で入るコードは、正確な値で木の単位の距離 ≤ a(1 + 数 ε) で、
/// 計算の丸めは prefilter_allowed の範囲(|minval| ≤ 4、scale ≤ 2^36、整数 < 2^32)では 1 成分あたり
/// 2^-53 × (4·2^32 + 4·2^36) ≈ 3e-5 目盛り + 距離の相対 数 ε(実数に戻す割り算と足し算、qt の引き算と掛け算、
/// 差と 2 乗の和)にとどまる。しきい t = a(1 + 1e-12) + 1 には相対 1e-12 と 1 目盛りの余裕があり、どちらも
/// 誤差(相対 1e-15、絶対 1e-4 目盛り)より桁違いに大きいので、入るコードを捨てることは無い(a がどれだけ大きくても、
/// 0 でも)。a が無限大なら t2 も無限大で何も捨てない。残ったコードの判定は今の式のままなので、答えは前ふるいなしと同じ。
/// 枝刈りの r_t = √(a² + 0.25D)(1 + 1e-9) + 1e-6 を使わないのは、a が 2 万目盛り前後で r_t − a が 5e-5 目盛りまで
/// 縮み、壊れた範囲の表では丸めの上限に近づくため(t は r_t より少し大きく、捨てる量がわずかに減るだけ)。
#[inline(always)]
fn code_far<T: KdInt, const D: usize>(row: &[T], qt: &[f64; D], t2: f64) -> bool {
    let mut dt2 = 0.0;
    for k in 0..D {
        let t = row[k].to_f64() - qt[k];
        dt2 += t * t;
    }
    dt2 > t2
}

/// コード木: 問い合わせ q(実数)から radius 以内のデータの番号を out に足す(昇順)。
///
/// 最終判定は Python 版と同じ式・同じ順(実数 c = minval + 整数/scale、Σ(c − q)² ≤ radius²)。prefilter は
/// 葉の前ふるい([`code_far`])を使うか(本番は meta.prefilter_ok。試験は false でも同じ答えになることを確かめる)。
#[allow(clippy::too_many_arguments)] // 木の表(mmap のビュー)と作業用の領域をそのまま渡すため
pub(crate) fn ball_query<S: KdInt, T: KdInt, const D: usize>(
    meta: &TreeMeta,
    lr: &[u32],
    split: &[S],
    data: &[T],
    q: &[f64; D],
    radius: f64,
    r_t: f64,
    prefilter: bool,
    stack: &mut Vec<Frame<D>>,
    out: &mut Vec<u32>,
) {
    let mut qt = [0.0; D];
    for k in 0..D {
        qt[k] = meta.to_tree(k, q[k]);
    }
    let rr = radius * radius;
    let t2 = code_far_t2(meta, radius);
    let start_len = out.len();
    let _ = ball_visit(meta, lr, split, &qt, r_t * r_t, stack, |s, e| {
        let Some(rows) = data.get(s * D..e * D) else { return ControlFlow::Continue(()) };
        for (i, row) in rows.chunks_exact(D).enumerate() {
            // 候補の 99 % 以上は外れるので、割り算の要る実数の判定の前に木の単位で捨てる
            if prefilter && code_far(row, &qt, t2) {
                continue;
            }
            if code_in(meta, row, q, rr) {
                out.push((s + i) as u32);
            }
        }
        ControlFlow::Continue(())
    });
    // 葉は左から順なので並んでいるはずだが、念のため
    let mine = &mut out[start_len..];
    if !mine.is_sorted() {
        mine.sort_unstable();
    }
}

thread_local! {
    /// 星の円錐のたどりのスタック(作業スレッドごとに使い回す。円錐は解法の足切りで数千万回引くので、
    /// 毎回ヒープに取って捨てない)。使う間は取り出して空を置き、終わったら戻す(取り合いも panic も無い)。
    static CONE_STACK: Cell<Vec<Frame<3>>> = const { Cell::new(Vec::new()) };
}

/// 星の前ふるい: 実数の判定(正規化して Σ(v − c)² ≤ chord²)で必ず外れる星か。true なら判定せずに捨ててよい。
///
/// 捨てるのは、木の単位で問い合わせから r_t より遠く(dt2 > r2)、かつ木の単位のノルム² n2 = Σ(row − o)²
/// (o = 原点の木の単位 = −minval·scale。n2 = scale²·|x|²、x = 実数に戻した星)が |n2 − scale²| ≤ tol = 2·scale の星だけ。
/// 後者は ||x|² − 1| ≤ 2/scale なので、正規化で星が動く量は scale·||x| − 1| = scale·||x|² − 1| / (|x| + 1) ≤ 2 目盛り以下。
/// 実数の判定で入る星は、木の単位で chord·scale + 2(正規化で動く分)+ 丸め以内にある。r_t = √((chord·scale)² + 0.75) + 4
/// には 4 目盛りの余裕があり、丸めは prefilter_allowed の範囲(|minval| ≤ 4、64 ≤ scale ≤ 2^36、整数 < 2^32、
/// |x| ≥ √(1 − 2/64) なので正規化も安定)では 1e-3 目盛りに届かない(行・qt・o の計算の誤差は
/// 2^-53 × (2^32 + 2·4·2^36) 程度で 1e-4 目盛り、chord·scale ≤ 2^37 の相対誤差 1e-15 も 1e-4 目盛り)。
/// だから実数の判定で入る星を捨てることは無く、答えは前ふるいなしと同じ。単位ベクトルでない星(壊れた索引)は
/// ノルムの条件で捨てずに、今までどおり実数の判定にかける(Python 版も正規化してから判定する)。
#[inline(always)]
#[allow(clippy::neg_cmp_op_on_partial_ord)] // NaN は「遠くない」(捨てない)側へ倒す
fn star_far<T: KdInt>(row: &[T], qt: &[f64; 3], r2: f64, o: &[f64; 3], s2: f64, tol: f64) -> bool {
    let mut dt2 = 0.0;
    for k in 0..3 {
        let t = row[k].to_f64() - qt[k];
        dt2 += t * t;
    }
    if !(dt2 > r2) {
        return false;
    }
    let mut n2 = 0.0;
    for k in 0..3 {
        let t = row[k].to_f64() - o[k];
        n2 += t * t;
    }
    (n2 - s2).abs() <= tol
}

/// 星の円錐の問い合わせ 1 回ぶんの値(葉の前に 1 回だけ計算する。試験も同じ値を使う)。
struct ConeQuery {
    /// 正規化済みの中心(実数)
    c: [f64; 3],
    /// 中心の木の単位と、枝刈りの半径²
    qt: [f64; 3],
    r2: f64,
    /// 弦の長さ²(実数の判定)
    cc: f64,
    /// 前ふるいの値: 原点の木の単位 −minval·scale、scale²、ノルム²の許容 2·scale
    o: [f64; 3],
    s2: f64,
    tol: f64,
}

impl ConeQuery {
    #[inline(always)]
    fn new(meta: &TreeMeta, c: &[f64; 3], chord: f64) -> ConeQuery {
        let mut qt = [0.0; 3];
        for k in 0..3 {
            qt[k] = meta.to_tree(k, c[k]);
        }
        // 量子化(3 × 0.25 目盛り²)と、正規化で動く分(|xyz| − 1 ≤ 4e-10 ≒ 0.9 目盛り)を広げる
        let r_t = ((chord * meta.scale).powi(2) + 0.75).sqrt() + 4.0;
        ConeQuery {
            c: *c,
            qt,
            r2: r_t * r_t,
            cc: chord * chord,
            o: [-meta.minval[0] * meta.scale, -meta.minval[1] * meta.scale, -meta.minval[2] * meta.scale],
            s2: meta.scale * meta.scale,
            tol: 2.0 * meta.scale,
        }
    }

    /// 実数の判定(Python 版と同じ式・同じ順): 星を実数に戻して正規化し、Σ(v − c)² ≤ chord² なら単位ベクトル。
    #[inline(always)]
    fn star_in<T: KdInt>(&self, meta: &TreeMeta, row: &[T]) -> Option<[f64; 3]> {
        let v = star_unit(meta, row);
        let mut d2 = 0.0;
        for (vk, ck) in v.iter().zip(&self.c) {
            let t = vk - ck;
            d2 += t * t;
        }
        (d2 <= self.cc).then_some(v)
    }
}

/// 星の木: 単位ベクトル c(正規化済み)から弦 chord 以内の星を、たどる順(葉の左から = 番号の昇順)に
/// f(番号, 正規化した単位ベクトル) で知らせる。f が Break を返したらそこでやめて Break を返す。
///
/// 判定は Python 版と同じ(星を実数に戻して長さ 1 に正規化し、Σ(s − c)² ≤ chord²)。f に渡す単位ベクトルは
/// その判定に使った値そのもの(= star_unit。IndexData::star_xyz と同じ値)。prefilter は葉の前ふるい([`star_far`])を
/// 使うか(本番は meta.prefilter_ok。試験は false でも同じ答えになることを確かめる)。
#[allow(clippy::too_many_arguments)] // 木の表(mmap のビュー)をそのまま渡すため
pub(crate) fn cone_visit<S: KdInt, T: KdInt>(
    meta: &TreeMeta,
    lr: &[u32],
    split: &[S],
    data: &[T],
    c: &[f64; 3],
    chord: f64,
    prefilter: bool,
    mut f: impl FnMut(u32, [f64; 3]) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let q = ConeQuery::new(meta, c, chord);
    let mut stack = CONE_STACK.try_with(Cell::take).unwrap_or_default();
    let r = ball_visit(meta, lr, split, &q.qt, q.r2, &mut stack, |s, e| {
        let Some(rows) = data.get(s * 3..e * 3) else { return ControlFlow::Continue(()) };
        for (i, row) in rows.chunks_exact(3).enumerate() {
            if prefilter && star_far(row, &q.qt, q.r2, &q.o, q.s2, q.tol) {
                continue;
            }
            if let Some(v) = q.star_in(meta, row) {
                f((s + i) as u32, v)?;
            }
        }
        ControlFlow::Continue(())
    });
    // スタックを戻す(スレッドの終わりで thread_local が無ければ捨てる)
    let _ = CONE_STACK.try_with(|cell| cell.set(stack));
    r
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


/// 変える前の ball_visit(節点ごとに子を積んで降ろす形)。試験で、今の形と葉を知らせる順がまったく同じことを確かめる。
#[cfg(test)]
pub(crate) fn ball_visit_ref<S: KdInt, const D: usize>(
    meta: &TreeMeta,
    lr: &[u32],
    split: &[S],
    qt: &[f64; D],
    r2: f64,
    mut leaf: impl FnMut(usize, usize),
) {
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
    let mut stack = vec![root];
    while let Some(f) = stack.pop() {
        if f.node >= meta.ninterior {
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
            continue;
        }
        let val = (raw & !meta.mask) as f64;
        let q = qt[dim];
        let mut rest = 0.0;
        for k in 0..D {
            if k != dim {
                rest += f.g2[k];
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験用の乱数(xorshift。種を決めれば毎回同じ)。
    pub(crate) struct Rng(pub u64);

    impl Rng {
        pub fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        /// 球面上の一様な単位ベクトル。
        pub fn unit(&mut self) -> [f64; 3] {
            loop {
                let v = [self.next() * 2.0 - 1.0, self.next() * 2.0 - 1.0, self.next() * 2.0 - 1.0];
                let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if n > 0.1 && n <= 1.0 {
                    return [v[0] / n, v[1] / n, v[2] / n];
                }
            }
        }
    }

    fn norm3(v: [f64; 3]) -> [f64; 3] {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / n, v[1] / n, v[2] / n]
    }

    /// 表を持たない、変換の値だけの TreeMeta(前ふるいの式の試験用)。
    fn meta_with(minval: [f64; 4], scale: f64, d: usize) -> TreeMeta {
        TreeMeta {
            ndim: d,
            levels: 1,
            ninterior: 0,
            mask: 0,
            full: u32::MAX as f64,
            minval,
            scale,
            int_type: KdType::U32,
            data_type: KdType::U32,
            lr: (0, 0),
            split: (0, 0),
            data: (0, 0),
            prefilter_ok: prefilter_allowed(&minval[..d], scale),
        }
    }

    #[test]
    fn prefilter_allowed_range() {
        assert!(prefilter_allowed(&[-1.0, -1.0, -1.0], 2147483647.5));
        assert!(prefilter_allowed(&[-4.0, 4.0], 64.0));
        assert!(prefilter_allowed(&[0.0; 4], PREFILTER_SCALE_MAX));
        assert!(!prefilter_allowed(&[-4.5, 0.0, 0.0], 1e9));
        assert!(!prefilter_allowed(&[0.0, 0.0, 0.0], 63.0));
        assert!(!prefilter_allowed(&[0.0, 0.0, 0.0], PREFILTER_SCALE_MAX * 2.0));
        assert!(!prefilter_allowed(&[f64::NAN, 0.0, 0.0], 1e9));
    }

    /// 星の前ふるいの余裕: 範囲の端(|minval| ≤ 4、64 ≤ scale ≤ 2^36)と、単位ベクトルから ±1.2 目盛り伸び縮みした星
    /// (ノルムの条件 2/scale の境目をまたぐ)、円錐の境目(星までの弦 = chord × (1 ± 5e-7))で、実数の判定で入る星を
    /// star_far が捨てないこと。正規化の動きが 1 目盛りを超えて効く場合(木の単位で chord·scale + 1 より遠いのに入る)も
    /// 十分な数を通ること(余裕を 1 目盛りに縮めると落とす場合を、ちゃんと試している)。
    #[test]
    fn star_prefilter_never_drops_a_star_in_the_cone() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut accepted, mut tight, mut far_n) = (0usize, 0usize, 0usize);
        for it in 0..2_000_000 {
            let scale = 2f64.powf(6.0 + 30.0 * rng.next());
            let u = rng.unit();
            let delta = (rng.next() * 2.0 - 1.0) * 1.2 / scale;
            let mut minval = [0.0; 4];
            let mut row = [0u32; 3];
            for k in 0..3 {
                let x = u[k] * (1.0 + delta);
                // minval = x − w/scale ≥ −4 になる整数 w(< 2^32)
                let wmax = ((x + 4.0) * scale).min(u32::MAX as f64);
                let w = (rng.next() * wmax).floor();
                row[k] = w as u32;
                minval[k] = x - w / scale;
            }
            if minval.iter().any(|m| m.abs() > PREFILTER_MINVAL_MAX) {
                continue;
            }
            let meta = meta_with(minval, scale, 3);
            assert!(meta.prefilter_ok);
            let v = star_unit(&meta, &row);
            // 弦: 木の単位で 0〜3 目盛り(正規化の動きが効く)か、0〜1e-3。ときどき遠い中心(捨てる側も通す)
            let chord = if it % 2 == 0 { rng.next() * 3.0 / scale } else { rng.next() * 1e-3 };
            let w = rng.unit();
            let wv = w[0] * v[0] + w[1] * v[1] + w[2] * v[2];
            let t = norm3([w[0] - wv * v[0], w[1] - wv * v[1], w[2] - wv * v[2]]);
            let off = if it % 7 == 0 { chord * (2.0 + 10.0 * rng.next()) + 10.0 / scale } else { chord * (1.0 + (rng.next() - 0.5) * 1e-6) };
            let c = norm3([v[0] + t[0] * off, v[1] + t[1] * off, v[2] + t[2] * off]);
            let q = ConeQuery::new(&meta, &c, chord);
            let accept = q.star_in(&meta, &row).is_some();
            let far = star_far(&row, &q.qt, q.r2, &q.o, q.s2, q.tol);
            assert!(!(accept && far), "scale={scale} minval={minval:?} row={row:?} c={c:?} chord={chord}");
            if accept {
                accepted += 1;
                let mut dt2 = 0.0;
                for (r, t) in row.iter().zip(&q.qt) {
                    let d = *r as f64 - t;
                    dt2 += d * d;
                }
                if dt2.sqrt() > chord * scale + 1.0 {
                    tight += 1;
                }
            }
            far_n += far as usize;
        }
        assert!(accepted > 300_000 && tight > 1000 && far_n > 10_000, "入る {accepted}・きわどい {tight}・捨てる {far_n}");
    }

    /// コードの前ふるいの余裕(D 次元): 範囲の端(|minval| ≤ 4、64 ≤ scale ≤ 2^36、整数 < 2^32)と、球の境目
    /// (コードまでの距離 = radius × (1 ± 1e-12) か、木の単位で 0〜3 目盛りの半径)で、実数の判定で入るコードを
    /// code_far が捨てないこと。戻り値は (入った数, 木の単位で a = radius·scale より遠いのに入った数, 捨てた数)。
    fn code_margin<const D: usize>(seed: u64, n: usize) -> (usize, usize, usize) {
        let mut rng = Rng(seed);
        let (mut accepted, mut beyond, mut far_n) = (0usize, 0usize, 0usize);
        for it in 0..n {
            let scale = 2f64.powf(6.0 + 30.0 * rng.next());
            let mut minval = [0.0; 4];
            let mut row = [0u32; D];
            for k in 0..D {
                minval[k] = (rng.next() * 2.0 - 1.0) * PREFILTER_MINVAL_MAX;
                row[k] = (rng.next() * u32::MAX as f64).floor() as u32;
            }
            let meta = meta_with(minval, scale, D);
            assert!(meta.prefilter_ok);
            let radius = match it % 3 {
                0 => rng.next() * 3.0 / scale,
                1 => rng.next() * 0.05,
                _ => rng.next() * 1e-3,
            };
            let mut dir = [0.0; D];
            let mut nn = 0.0;
            for v in dir.iter_mut() {
                *v = rng.next() * 2.0 - 1.0;
                nn += *v * *v;
            }
            let nn = nn.sqrt().max(1e-3);
            // ほとんどは境目のまわり、ときどき遠く(捨てる側も通す)
            let off = if it % 5 == 0 { radius * (2.0 + 10.0 * rng.next()) + 10.0 / scale } else { radius * (1.0 + (rng.next() - 0.5) * 2e-12) };
            let mut q = [0.0; D];
            let mut qt = [0.0; D];
            for k in 0..D {
                q[k] = meta.to_real(k, row[k] as f64) + dir[k] / nn * off;
                qt[k] = meta.to_tree(k, q[k]);
            }
            let accept = code_in(&meta, &row, &q, radius * radius);
            let far = code_far(&row, &qt, code_far_t2(&meta, radius));
            assert!(!(accept && far), "D={D} scale={scale} minval={minval:?} row={row:?} q={q:?} radius={radius}");
            if accept {
                accepted += 1;
                let mut dt2 = 0.0;
                for (r, t) in row.iter().zip(&qt) {
                    let d = *r as f64 - t;
                    dt2 += d * d;
                }
                beyond += (dt2.sqrt() > radius * scale) as usize;
            }
            far_n += far as usize;
        }
        (accepted, beyond, far_n)
    }

    #[test]
    fn code_prefilter_never_drops_a_code_in_the_ball() {
        let (a4, b4, f4) = code_margin::<4>(0x853C_49E6_748F_EA9B, 1_000_000);
        let (a2, b2, f2) = code_margin::<2>(0xDA94_2042_E4DD_58B5, 1_000_000);
        eprintln!("コードの前ふるい: D=4 入る {a4}・木の単位で a より遠いのに入る {b4}・捨てる {f4}、D=2 {a2}・{b2}・{f2}");
        // 丸めで木の単位の距離が a を超えたのに入るコード(余裕が要る場合)も、ちゃんと試している
        assert!(a4 > 200_000 && a2 > 200_000 && b4 > 1000 && b2 > 1000 && f4 > 100_000 && f2 > 100_000);
    }
}

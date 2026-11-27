//! `load_image_any`・`looks_inverted`(Python 版 dev/zahyou_core.py の同名の関数)。
//!
//! 拡張子が FITS のもの(.fits .fit .fts .fz .fits.gz .fit.gz)は FITS → 画像ファイルの順、それ以外は逆の順で試す
//! (拡張子と中身が食い違うファイル対策)。読めたら
//! 1. 2 次元・16 画素以上かを確かめる
//! 2. 表示用の面は今の配列をそのまま持つ(コピーしない。検出用の面は 3・4・6 で書き換えるときだけ新しい配列にする)
//! 3. ベイヤー配列のキーがあれば 3×3 の二項フィルター [1,2,1]⊗[1,2,1]/16 で市松模様を消す(星の位置はずれない)
//! 4. 非有限値を有限値の中央値で埋める
//! 5. 一様なら Err
//! 6. 白地に黒い星(星図や、明暗を反転した写真)なら検出用の面だけ明暗を反転する
//!
//! Python 版との違い: 拡張子が FITS でないのに中身が FITS のファイルは、Python では Pillow の FITS プラグインが
//! 16 bit を逆のバイト順で読んで値が化けていた。ここでは画像ファイルとして読めないので FITS として正しく読む。

use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;
use zahyou_wcs::{Card, CardValue};

use crate::filters::binomial3;
use crate::fits::{self, get, py_str};
use crate::raster;
use crate::stats::{filter_ordered, median_finite_f32};
use crate::{Image2D, Loaded};

/// 読み込めなかった理由(Display は Python 版と同じ文)。
#[derive(Clone, Debug, PartialEq)]
pub enum LoadError {
    /// どの読み方でも読めなかった。中身は最初に試した読み方のエラー
    Unreadable(String),
    /// 2 次元の画像にならなかった
    NotTwoDimensional(String),
    /// 小さすぎる(高さ, 幅)
    TooSmall { height: usize, width: usize },
    /// 全画素が同じ値
    Uniform,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Unreadable(e) => write!(f, "画像として読み込めませんでした: {e}"),
            LoadError::NotTwoDimensional(s) => write!(f, "2 次元画像になりませんでした: shape={s}"),
            LoadError::TooSmall { height, width } => write!(f, "画像が小さすぎて解析できません: ({height}, {width})"),
            LoadError::Uniform => write!(f, "画像が一様です (全画素が同じ値)。露出やファイルを確認してください。"),
        }
    }
}

impl std::error::Error for LoadError {}

const FITS_EXT: [&str; 6] = [".fits", ".fit", ".fts", ".fz", ".fits.gz", ".fit.gz"];

/// ローダーの結果(2 次元の画素、高さ、幅、ヘッダー、説明、上下反転)。
type Raw = (Vec<f32>, usize, usize, Vec<Card>, Vec<String>, bool);
/// ローダー(FITS か画像ファイル)
type Loader = fn(&Path) -> Result<Raw, String>;

fn via_fits(path: &Path) -> Result<Raw, String> {
    let f = fits::load_fits(path)?;
    Ok((f.data, f.ny, f.nx, f.header, f.notes, f.flip_y))
}

fn via_raster(path: &Path) -> Result<Raw, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("ファイルを読めません: {e}"))?;
    let r = raster::load_raster(&bytes)?;
    // 画像ファイルは 1 行目が上
    Ok((r.data, r.ny, r.nx, Vec::new(), r.notes, true))
}

/// `_bayer_pattern`: BAYERPAT → COLORTYP → CFAIMAGE の順(Python の `or`)。無くて XBAYROFF・BAYOFFX があれば RGGB。
pub(crate) fn bayer_pattern(header: &[Card]) -> Option<&'static str> {
    // Python の真偽: 空文字・0・False は偽。値の無いカード(Undefined)は真
    let truthy = |v: &CardValue| match v {
        CardValue::Bool(b) => *b,
        CardValue::Int(i) => *i != 0,
        CardValue::Float(f) => *f != 0.0,
        CardValue::Str(s) => !s.is_empty(),
        CardValue::None => true,
    };
    // a or b or c: 真の最初のもの。どれも偽なら最後の c(無ければ None)
    let (a, b, c) = (get(header, "BAYERPAT"), get(header, "COLORTYP"), get(header, "CFAIMAGE"));
    let pat = if a.is_some_and(truthy) {
        a
    } else if b.is_some_and(truthy) {
        b
    } else {
        c
    };
    let pat = match pat {
        Some(v) => v,
        None => {
            if get(header, "XBAYROFF").is_some() || get(header, "BAYOFFX").is_some() {
                return Some("RGGB");
            }
            return None;
        }
    };
    let s = match pat {
        CardValue::None => return None, // str(Undefined) は型名で、どの配列とも一致しない
        v => py_str(v),
    };
    match s.trim().to_uppercase().as_str() {
        "RGGB" => Some("RGGB"),
        "BGGR" => Some("BGGR"),
        "GRBG" => Some("GRBG"),
        "GBRG" => Some("GBRG"),
        _ => None,
    }
}

/// `looks_inverted`: 白地に黒い星(星図や、明暗を反転した写真)か。背景(中央値)が明るさの幅の上端に寄っていて、
/// 明るい側の幅(最大値 − 中央値)が暗い側の幅(中央値 − 最小値)の 5% 以下なら真。ただし最小値が孤立した外れ値なら
/// 暗いほうから `DARK_RANK` 番目の値で測る(`inverted_by`)。ベイヤーの平滑化をかけていない面として扱う
/// (読み込みの中では、平滑化したかを渡して判定している)。
pub fn looks_inverted(data: &Image2D) -> bool {
    looks_inverted_with(data, false)
}

/// `looks_inverted` の本体。`smoothed` はベイヤーの 3×3 の平滑化をかけた面か。
fn looks_inverted_with(data: &Image2D, smoothed: bool) -> bool {
    let all_finite = data.data.par_iter().all(|v| v.is_finite());
    let owned;
    let v: &[f32] = if all_finite {
        &data.data
    } else {
        owned = filter_ordered(&data.data, |x| x.is_finite());
        &owned
    };
    if v.is_empty() {
        return false;
    }
    let (lo, hi) = min_max(v);
    inverted_by(data, v, lo, hi, smoothed)
}

/// `looks_inverted` の判定の部分。`img` は面(最も暗い画素の周りを見るのに使う)、`v` はその空でない有限値の並び、
/// `lo`・`hi` は `min_max(v)` の値、`smoothed` はベイヤーの 3×3 の平滑化をかけた面か。
/// 読み込みでは全部有限で `min_max` も済んでいるので、全部有限かの走査と `min_max` を飛ばしてここだけ呼ぶ。
///
/// 最も暗い画素が孤立した外れ値なら、暗い側の幅を、暗いほうから `DARK_RANK`(10)番目の画素の値で測る
/// (中央値 − 10 番目の値。Python 版と同じ)。以下の「背景との差」は 中央値 − 値 で、背景より明るい画素は 0 とする。
/// 孤立した外れ値とみるのは、次の 2 つを満たすとき。
/// 1. 最も暗い画素の背景との差が、10 番目の画素の背景との差の `DARK_OUTLIER`(10)倍を超える
/// 2. 最も暗い値のどの画素でも、周りの画素(隣の 8 画素。平滑化のあとは、その外側の 16 画素 = 5×5 の外周)の
///    背景との差が、すべて 10 番目の画素の背景との差以下
///
/// HST の画像の −20,977 の 1 画素(背景 0.47・最大 179、10 番目の値は −62)のような写真を星図と取り違えないため。
/// 取り違えると雑音を星として検出し、誤答した(2026-11)。10 番目にしたのは、外れ値 1 画素がベイヤーの平滑化で
/// 9 画素に広がるから。2 が要るのは、明暗を反転した写真で星像が小さいと、明るい星のピークの 1 画素が
/// 1 を満たしてしまうから(星なら隣の画素も背景との差が大きい)。
///
/// つまり、孤立した 1〜9 画素の外れ値では判定が変わらない。そのため次のような画像は、外れ値の画素のおかげで
/// 今までは反転していたが、反転しなくなった(外れ値が無ければ今までも反転しなかった画像):
/// - 白地に孤立した黒い画素が 9 個以下しかない画像
/// - 孤立したホットピクセル・宇宙線が 1〜9 個あり、暗い星だけが写った、明暗を反転した写真
/// - ベイヤー配列で星像がとても小さい(FWHM 1 px 前後)、明暗を反転した写真
///
/// 逆に、外れ値が隣り合う塊や、ベイヤー配列の画像で外れ値が 2 個以上のときは、今までどおり反転しうる
/// (Python 版の説明)。
fn inverted_by(img: &Image2D, v: &[f32], lo: f32, hi: f32, smoothed: bool) -> bool {
    let med = median_finite_f32(v) as f64;
    let up = hi as f64 - med;
    let mut down = med - lo as f64;
    if !(down > 0.0 && up <= down * 0.05) {
        // 暗い側の幅を 10 番目の値で測り直しても小さくなるだけなので、反転しない
        return false;
    }
    let k = DARK_RANK.min(v.len());
    if k > 1 {
        let down_k = med - kth_smallest_few(v, k) as f64;
        if down > DARK_OUTLIER * down_k && dark_isolated(img, lo, med, down_k, smoothed) {
            down = down_k;
        }
    }
    down > 0.0 && up <= down * 0.05
}

/// 値が `lo` の画素(最も暗い画素。ここに来るときは 9 個以下)のどれも、周りの画素の背景 `med` との差(`med` − 値。背景より明るい画素は 0)が `down_k` 以下か
/// (Python 版の `_dark_isolated`)。周りは隣の 8 画素、`smoothed` なら 5×5 の外周の 16 画素(縁ではみ出す分は除く)。
/// 非有限値も背景との差 0 とする。全部を見るのは、画像の向き(上下の反転)で結果が変わらないようにするため。
/// 大きさが画素の数と合わない `Image2D`(公開関数に直接渡されたもの)は、周りが分からないので孤立とみない(最小値で測る)。
fn dark_isolated(img: &Image2D, lo: f32, med: f64, down_k: f64, smoothed: bool) -> bool {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 || w.checked_mul(h) != Some(img.data.len()) {
        return false;
    }
    let r = if smoothed { 2 } else { 1 };
    for (i, _) in img.data.iter().enumerate().filter(|&(_, &v)| v == lo) {
        let (y, x) = (i / w, i % w);
        let mut deepest = 0.0f64;
        for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
            for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                if yy.abs_diff(y).max(xx.abs_diff(x)) < r {
                    continue; // 中心(平滑化のあとは中心の 3×3)は見ない
                }
                let val = img.data[yy * w + xx];
                if val.is_finite() {
                    deepest = deepest.max(med - val as f64);
                }
            }
        }
        if deepest > down_k {
            return false;
        }
    }
    true
}

/// `inverted_by`: 最も暗い画素の背景との差が、暗いほうから `DARK_RANK` 番目の画素の背景との差の `DARK_OUTLIER` 倍を
/// 超え、周りから孤立していれば、外れ値とみて `DARK_RANK` 番目の画素の値で測る(Python 版の同じ名前の値と同じ)
const DARK_RANK: usize = 10;
const DARK_OUTLIER: f64 = 10.0;

/// 小さいほうから `k` 番目(1 始まり、`1 ≤ k ≤ v.len()`)の値。NaN を含まない配列に使う。
/// `k` が小さいとき用: ブロックごとに小さいほうから `k` 個を集め、最後にマージする(全画素を 1 回走査するだけ)。
/// 値は `np.partition(v, k - 1)[k - 1]` と同じ(±0 はどちらになるか決めない)。
fn kth_smallest_few(v: &[f32], k: usize) -> f32 {
    fn push(a: &mut Vec<f32>, x: f32, k: usize) {
        if a.len() == k && x >= a[k - 1] {
            return;
        }
        let pos = a.partition_point(|&y| y <= x);
        a.insert(pos, x);
        a.truncate(k);
    }
    let parts: Vec<Vec<f32>> = v
        .par_chunks(1 << 16)
        .map(|c| {
            let mut a = Vec::with_capacity(k + 1);
            for &x in c {
                push(&mut a, x, k);
            }
            a
        })
        .collect();
    let mut all = Vec::with_capacity(k + 1);
    for p in parts {
        for x in p {
            push(&mut all, x, k);
        }
    }
    all[k - 1]
}

fn min_max(v: &[f32]) -> (f32, f32) {
    v.par_chunks(1 << 16)
        .map(|c| c.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &x| (a.min(x), b.max(x))))
        .reduce(|| (f32::INFINITY, f32::NEG_INFINITY), |(a, b), (c, d)| (a.min(c), b.max(d)))
}

/// 非有限値の数と `min_max` を 1 回の走査で(読み込みで全画素を 2 回走査しないため)。
/// 最小・最大の畳み方は `min_max` と同じ(`f32::min`・`max` なので NaN は飛ばし、±inf は混ぜる)で、同じ `v` なら
/// 非有限値があっても `min_max(v)` と同じ値になる。ただしこれは補完の前の面の値なので、読み込みで補完のあとの
/// `min_max(&data)` として使えるのは、補完が要らない(非有限値が 0 個の)ときだけ。補完したら `min_max` で取り直す。
/// 同じ値でもビットが違いうるのは ±0 だけ(0 以外の値は、等しければビットも同じ)。±0 のどちらを選ぶかは
/// 畳み方(ベクトル化の仕方)で変わりうるので、呼ぶ側は最小・最大が 0 のときも `min_max` で取り直す。
fn count_bad_min_max(v: &[f32]) -> (usize, f32, f32) {
    v.par_chunks(1 << 16)
        .map(|c| {
            c.iter().fold((0usize, f32::INFINITY, f32::NEG_INFINITY), |(n, a, b), &x| {
                (n + usize::from(!x.is_finite()), a.min(x), b.max(x))
            })
        })
        .reduce(|| (0, f32::INFINITY, f32::NEG_INFINITY), |(n, a, b), (m, c, d)| (n + m, a.min(c), b.max(d)))
}

/// 画素ごとの変換を面にかける。ほかと共有していない(`Arc` が 1 つだけの)面はその場で書き換え、
/// 表示用の面と共有しているなら、並列の map で新しい配列を作る(`Arc::make_mut` のように直列でコピーしてから
/// 書き換えると、96 MB の直列の memcpy が残る)。どちらも各画素に同じ `f` をかけるだけなので値は同じ。
fn map_plane(img: &mut Arc<Image2D>, f: impl Fn(f32) -> f32 + Sync + Send) {
    if let Some(m) = Arc::get_mut(img) {
        m.data.par_iter_mut().for_each(|v| *v = f(*v));
    } else {
        let data: Vec<f32> = img.data.par_iter().map(|&v| f(v)).collect();
        *img = Arc::new(Image2D { width: img.width, height: img.height, data });
    }
}

/// `load_image_any`: FITS・PNG・JPEG・TIFF・キューブを読み、2 次元の float32 にする。
pub fn load_image_any(path: impl AsRef<Path>) -> Result<Loaded, LoadError> {
    let path = path.as_ref();
    let lower = path.to_string_lossy().to_lowercase();
    let fits_first = FITS_EXT.iter().any(|e| lower.ends_with(e));
    let loaders: [Loader; 2] = if fits_first { [via_fits, via_raster] } else { [via_raster, via_fits] };
    let mut first_error: Option<String> = None;
    let mut got: Option<Raw> = None;
    for l in loaders {
        match l(path) {
            Ok(r) => {
                got = Some(r);
                break;
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    let Some((data, ny, nx, header, notes, flip_y)) = got else {
        return Err(LoadError::Unreadable(first_error.unwrap_or_default()));
    };
    finish(data, ny, nx, header, notes, flip_y)
}

/// 読めた画素から `Loaded` を作る(モジュールの説明の 1〜6)。
///
/// 表示用の面は読み込んだままの配列をそのまま持ち、検出用の面は書き換えが要るときだけ新しい配列にする
/// (以前は最初に `data.clone()` で表示用をコピーしていた。値は同じ: 表示用は今も加工前の値)。
/// - ベイヤー: `binomial3` が新しい配列を返す(表示用はそのまま)
/// - 非有限値の補完・明暗の反転: `map_plane`(共有中なら並列に新しい配列、単独ならその場で)
/// - どれも無ければ検出用と表示用は同じ配列
fn finish(
    data: Vec<f32>,
    ny: usize,
    nx: usize,
    header: Vec<Card>,
    mut notes: Vec<String>,
    flip_y: bool,
) -> Result<Loaded, LoadError> {
    if data.len() != ny * nx {
        return Err(LoadError::NotTwoDimensional(format!("({ny}, {nx})")));
    }
    if ny.min(nx) < 16 {
        return Err(LoadError::TooSmall { height: ny, width: nx });
    }
    let display = Arc::new(Image2D { width: nx, height: ny, data });
    let mut data = Arc::clone(&display);

    let smoothed = bayer_pattern(&header).is_some();
    if let Some(pat) = bayer_pattern(&header) {
        // 2x2 の平均 (v6 まで) は窓が片側に寄るので星の位置が +0.5 px ずれていた。対称な 3x3 の二項カーネルならずれない
        data = Arc::new(Image2D { width: nx, height: ny, data: binomial3(&display.data, ny, nx) });
        notes.push(format!("ベイヤー配列 ({pat}) を 3x3 の二項フィルターで平滑化"));
    }

    // 非有限値の数と最小・最大を 1 回の走査で。補完が要らないときの最小・最大は `min_max(&data)` と同じ値
    // (0 以外ならビットも同じ。0 のときは ±0 の選び方を以前と揃えるため `min_max` で取り直す)
    let (n_bad, mut lo, mut hi) = count_bad_min_max(&data.data);
    // 補完した値が有限か(有限なら、補完のあとは全部有限で、looks_inverted の全部有限かの走査が要らない)
    let mut fill_finite = true;
    if n_bad > 0 {
        let good = filter_ordered(&data.data, |v| v.is_finite());
        let fill = if good.is_empty() { 0.0f32 } else { median_finite_f32(&good) };
        drop(good);
        map_plane(&mut data, |v| if v.is_finite() { v } else { fill });
        notes.push(format!("非有限値 {n_bad} 画素を中央値で補完"));
        fill_finite = fill.is_finite();
        (lo, hi) = min_max(&data.data);
    } else if lo == 0.0 || hi == 0.0 {
        (lo, hi) = min_max(&data.data);
    }
    if hi <= lo {
        return Err(LoadError::Uniform);
    }

    // 星図 (白地に黒い星) は明暗が逆。検出に使う面だけ反転する(表示は元のまま)。
    // 全部有限なら looks_inverted は全部有限かを確かめ、`min_max(&data)` をもう一度求めるだけなので、上の lo・hi を
    // そのまま渡す(上の lo・hi は `min_max(&data)` とビットまで同じ: 補完したときと 0 のときは min_max そのもの、
    // それ以外は 0 以外の同じ値)。補完した値が ±inf(中央値の和のあふれ)のときだけ元の関数を呼ぶ
    let inverted = if fill_finite {
        inverted_by(&data, &data.data, lo, hi, smoothed)
    } else {
        looks_inverted_with(&data, smoothed)
    };
    if inverted {
        let mx = hi;
        map_plane(&mut data, |v| mx - v);
        notes.push("白地に黒い星 (星図) とみて明暗を反転".to_string());
    }

    Ok(Loaded { data, display, header, note: notes.join(" / "), flip_y })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(k: &str, v: CardValue) -> Card {
        Card::new(k, v, "")
    }

    #[test]
    fn bayer_keys() {
        assert_eq!(bayer_pattern(&[h("BAYERPAT", CardValue::Str("rggb ".into()))]), Some("RGGB"));
        assert_eq!(bayer_pattern(&[h("COLORTYP", CardValue::Str("grbg".into()))]), Some("GRBG"));
        assert_eq!(bayer_pattern(&[h("XBAYROFF", CardValue::Int(0))]), Some("RGGB"));
        assert_eq!(bayer_pattern(&[h("BAYERPAT", CardValue::Str("XYZW".into()))]), None);
        // 空の BAYERPAT は偽なので次のキーへ。最後の CFAIMAGE が無ければ None → XBAYROFF を見る
        assert_eq!(
            bayer_pattern(&[h("BAYERPAT", CardValue::Str("".into())), h("XBAYROFF", CardValue::Int(1))]),
            Some("RGGB")
        );
        assert_eq!(
            bayer_pattern(&[h("BAYERPAT", CardValue::Str("".into())), h("COLORTYP", CardValue::Str("BGGR".into()))]),
            Some("BGGR")
        );
        // 最後の CFAIMAGE が空文字なら pat は '' で、XBAYROFF は見ない
        assert_eq!(bayer_pattern(&[h("CFAIMAGE", CardValue::Str("".into())), h("XBAYROFF", CardValue::Int(1))]), None);
        assert_eq!(bayer_pattern(&[]), None);
    }

    /// 以前の後処理(表示用を最初に `clone` し、検出用をその場で書き換える)。`finish` と値を比べる。
    fn old_finish(
        mut data: Vec<f32>,
        ny: usize,
        nx: usize,
        header: &[Card],
        mut notes: Vec<String>,
    ) -> Result<(Vec<f32>, Vec<f32>, String), LoadError> {
        if data.len() != ny * nx {
            return Err(LoadError::NotTwoDimensional(format!("({ny}, {nx})")));
        }
        if ny.min(nx) < 16 {
            return Err(LoadError::TooSmall { height: ny, width: nx });
        }
        let display = data.clone();
        if let Some(pat) = bayer_pattern(header) {
            data = binomial3(&data, ny, nx);
            notes.push(format!("ベイヤー配列 ({pat}) を 3x3 の二項フィルターで平滑化"));
        }
        let n_bad = data.par_iter().filter(|v| !v.is_finite()).count();
        if n_bad > 0 {
            let good = filter_ordered(&data, |v| v.is_finite());
            let fill = if good.is_empty() { 0.0f32 } else { median_finite_f32(&good) };
            data.par_iter_mut().for_each(|v| {
                if !v.is_finite() {
                    *v = fill;
                }
            });
            notes.push(format!("非有限値 {n_bad} 画素を中央値で補完"));
        }
        let (lo, hi) = min_max(&data);
        if hi <= lo {
            return Err(LoadError::Uniform);
        }
        let mut img = Image2D { width: nx, height: ny, data };
        if looks_inverted(&img) {
            let mx = hi;
            img.data.par_iter_mut().for_each(|v| *v = mx - *v);
            notes.push("白地に黒い星 (星図) とみて明暗を反転".to_string());
        }
        Ok((img.data, display, notes.join(" / ")))
    }

    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    /// 決まった擬似乱数(xorshift64*)の [0, 1)
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f32 {
            let x = &mut self.0;
            *x ^= *x >> 12;
            *x ^= *x << 25;
            *x ^= *x >> 27;
            (x.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    #[test]
    fn finish_same_as_before() {
        let bayer = vec![h("BAYERPAT", CardValue::Str("RGGB".into()))];
        let (ny, nx) = (70, 90);
        let n = ny * nx;
        let mut r = Rng(0x1234_5678_9abc_def1);
        // 名前・画素・ヘッダー
        let mut cases: Vec<(&str, Vec<f32>, Vec<Card>)> = Vec::new();
        let noise: Vec<f32> = (0..n).map(|_| 100.0 + 20.0 * r.next()).collect();
        cases.push(("ふつう", noise.clone(), vec![]));
        cases.push(("ベイヤー", noise.clone(), bayer.clone()));
        let mut nan = noise.clone();
        for i in (0..n).step_by(37) {
            nan[i] = if i % 3 == 0 {
                f32::NAN
            } else if i % 3 == 1 {
                f32::INFINITY
            } else {
                f32::NEG_INFINITY
            };
        }
        cases.push(("非有限値", nan.clone(), vec![]));
        cases.push(("ベイヤー+非有限値", nan.clone(), bayer.clone()));
        // 白地に黒い星(反転する)
        let mut chart: Vec<f32> = (0..n).map(|_| 250.0 - r.next()).collect();
        for i in (0..n).step_by(53) {
            chart[i] = 10.0 * r.next();
        }
        cases.push(("反転", chart.clone(), vec![]));
        cases.push(("ベイヤー+反転", chart.clone(), bayer.clone()));
        let mut chart_nan = chart.clone();
        for i in (5..n).step_by(91) {
            chart_nan[i] = f32::NAN;
        }
        cases.push(("非有限値+反転", chart_nan.clone(), vec![]));
        cases.push(("ベイヤー+非有限値+反転", chart_nan, bayer.clone()));
        // 最大が 0 で +0 と -0 が混ざる白地(反転の mx の ±0 が以前と同じか)
        let mut zero: Vec<f32> = (0..n).map(|i| if i % 2 == 0 { 0.0 } else { -0.0 }).collect();
        for i in (0..n).step_by(41) {
            zero[i] = -100.0 * (0.5 + r.next());
        }
        cases.push(("最大が ±0 の反転", zero.clone(), vec![]));
        let mut zero_nan = zero.clone();
        zero_nan[7] = f32::NAN;
        cases.push(("最大が ±0 の反転+非有限値", zero_nan, vec![]));
        // 最小が ±0
        let lowzero: Vec<f32> = (0..n)
            .map(|i| {
                if i % 5 == 0 {
                    -0.0
                } else if i % 5 == 1 {
                    0.0
                } else {
                    r.next()
                }
            })
            .collect();
        cases.push(("最小が ±0", lowzero, vec![]));
        // 補完の中央値があふれて inf になる(大きな値ばかり)
        let mut huge: Vec<f32> = (0..n).map(|_| 3.0e38 + 1.0e37 * r.next()).collect();
        huge[0] = f32::NAN;
        huge[1] = f32::NAN; // 有限値を偶数個にして、中央値を 2 つの和(あふれて inf)にする
        cases.push(("補完値が inf", huge, vec![]));
        // 一様・全部非有限・小さすぎ・形が合わない
        cases.push(("一様", vec![5.0; n], vec![]));
        cases.push(("全部 NaN", vec![f32::NAN; n], vec![]));
        cases.push((
            "非有限値を除くと一様",
            nan.iter().map(|v| if v.is_finite() { 1.0 } else { *v }).collect(),
            vec![],
        ));
        for (name, data, header) in cases {
            let old = old_finish(data.clone(), ny, nx, &header, vec!["前".into()]);
            let new = finish(data, ny, nx, header, vec!["前".into()], true);
            match (old, new) {
                (Ok((od, odisp, onote)), Ok(l)) => {
                    assert_eq!(bits(&od), bits(&l.data.data), "{name}: 検出用の面");
                    assert_eq!(bits(&odisp), bits(&l.display.data), "{name}: 表示用の面");
                    assert_eq!(onote, l.note, "{name}: 説明");
                    assert_eq!((l.data.width, l.data.height, l.display.width, l.display.height), (nx, ny, nx, ny));
                    assert!(l.flip_y);
                }
                (Err(a), Err(b)) => assert_eq!(a, b, "{name}: エラー"),
                (a, b) => panic!("{name}: 以前 {:?} / 今 {:?}", a.map(|x| x.2), b.map(|x| x.note)),
            }
        }
        assert_eq!(
            finish(vec![1.0; 15 * 20], 15, 20, vec![], vec![], false).unwrap_err(),
            LoadError::TooSmall { height: 15, width: 20 }
        );
        assert!(matches!(finish(vec![1.0; 10], 16, 16, vec![], vec![], false), Err(LoadError::NotTwoDimensional(_))));
    }

    #[test]
    fn count_bad_min_max_values() {
        // 最小・最大は、非有限値があっても同じ v の min_max と同じ値(説明のとおり)。0 以外ならビットも同じ。
        // 並列のブロック(1 << 16)の境目をまたぐ長さで、NaN・±inf の有り無しを試す
        let mut r = Rng(0x0dd_ba11_cafe_f00d);
        let n = (1 << 16) * 2 + 333;
        let base: Vec<f32> = (0..n).map(|_| r.next() * 1000.0 - 500.0).collect();
        let mut nan = base.clone();
        for i in (0..n).step_by(97) {
            nan[i] = f32::NAN;
        }
        let mut inf = nan.clone();
        inf[12345] = f32::INFINITY;
        inf[(1 << 16) + 7] = f32::NEG_INFINITY;
        for (name, v, bad) in
            [("有限", &base, 0usize), ("NaN", &nan, n.div_ceil(97)), ("NaN+inf", &inf, n.div_ceil(97) + 2)]
        {
            let (nb, lo, hi) = count_bad_min_max(v);
            let (a, b) = min_max(v);
            assert_eq!(nb, bad, "{name}: 非有限値の数");
            assert_eq!((lo.to_bits(), hi.to_bits()), (a.to_bits(), b.to_bits()), "{name}: 最小・最大");
        }
        assert_eq!(count_bad_min_max(&[f32::NAN; 5]).0, 5);
        let (_, lo, hi) = count_bad_min_max(&[f32::NAN; 5]);
        assert_eq!((lo, hi), min_max(&[f32::NAN; 5]));
    }

    #[test]
    fn plain_image_shares_one_array() {
        // 加工の無い画像では検出用と表示用が同じ配列(コピーを作らない)
        let d: Vec<f32> = (0..400).map(|i| i as f32).collect();
        let l = finish(d, 20, 20, vec![], vec![], false).unwrap();
        assert!(Arc::ptr_eq(&l.data, &l.display));
        // ベイヤーなら別
        let d: Vec<f32> = (0..400).map(|i| i as f32).collect();
        let l = finish(d, 20, 20, vec![h("BAYERPAT", CardValue::Str("RGGB".into()))], vec![], false).unwrap();
        assert!(!Arc::ptr_eq(&l.data, &l.display));
    }

    #[test]
    fn inverted() {
        // 白地(200)に黒い画素が DARK_RANK 個以上あれば星図とみて反転する
        let mut d = vec![200.0f32; 400];
        for (i, v) in d.iter_mut().take(DARK_RANK).enumerate() {
            *v = 10.0 + 4.0 * i as f32;
        }
        assert!(looks_inverted(&Image2D { width: 20, height: 20, data: d.clone() }));
        // 明るい側にも大きく伸びていれば星図ではない
        d[300] = 255.0;
        assert!(!looks_inverted(&Image2D { width: 20, height: 20, data: d }));
    }

    #[test]
    fn inverted_ignores_fewer_dark_outliers_than_the_rank() {
        // 暗い背景の星野(背景 0.5、星 180)に、飛び抜けて暗い画素(HST の画像の −20,977 のような値)を足す。
        // DARK_RANK − 1 個までは無視し、DARK_RANK 個で星図とみる(境目。順位が 1 つずれても気づけるように)
        let (w, h) = (500, 400);
        let mut base = vec![0.5f32; w * h];
        for i in (0..base.len()).step_by(997) {
            base[i] = 180.0;
        }
        let with = |n: usize| {
            let mut d = base.clone();
            for j in 0..n {
                d[12_345 + 101 * j] = -20_976.9 - j as f32;
            }
            d
        };
        for (n, want) in [(1, false), (DARK_RANK - 1, false), (DARK_RANK, true)] {
            let d = with(n);
            assert_eq!(looks_inverted(&Image2D { width: w, height: h, data: d.clone() }), want, "外れ値 {n} 個");
            // 読み込みの経路(全部有限で lo・hi を渡す)も同じ
            let l = finish(d, h, w, vec![], vec![], false).unwrap();
            assert_eq!(l.note.contains("反転"), want, "読み込み: 外れ値 {n} 個");
        }
        // 大きくて星の少ない星図(白地に黒い画素が DARK_RANK 個だけ)も反転する
        let (w2, h2) = (4000, 3000);
        let mut d2 = vec![255.0f32; w2 * h2];
        for j in 0..DARK_RANK {
            d2[1_000_003 * (j + 1)] = 0.0;
        }
        assert!(looks_inverted(&Image2D { width: w2, height: h2, data: d2 }));
    }

    #[test]
    fn inverted_outlier_ratio_boundary() {
        // 背景 1000・最大 1010(明るい側の幅 10)。最も暗い画素は背景より a 暗く、暗いほうから 2〜10 番目は背景との差 40。
        // a ≤ 10 × 40 なら最小値で測る(a = 400 で暗い側の幅 400 → 反転)。超えたら 10 番目の値で測る(40 → 反転しない)
        let img = |a: f32| {
            let mut d = vec![1000.0f32; 400];
            d[0] = 1010.0;
            d[1] = 1000.0 - a;
            for x in d.iter_mut().skip(2).take(DARK_RANK - 1) {
                *x = 960.0;
            }
            Image2D { width: 20, height: 20, data: d }
        };
        assert!(looks_inverted(&img(400.0)));
        assert!(!looks_inverted(&img(401.0)));
        // 飛び抜けていない最小値(星の少ない、明暗を反転した写真: 最も暗い画素の背景との差 300、10 番目は 40)は今までどおり最小値で測る
        assert!(looks_inverted(&img(300.0)));
    }

    #[test]
    fn inverted_keeps_minimum_when_not_isolated() {
        // 上と同じ a = 401(10 番目の 10 倍を超える)でも、隣の画素の背景との差が 10 番目より大きければ
        // 星のピークとみて最小値で測る
        let mut d = vec![1000.0f32; 400];
        d[0] = 1010.0;
        d[45] = 1000.0 - 401.0;
        d[46] = 1000.0 - 120.0;
        for x in d.iter_mut().skip(200).take(DARK_RANK - 2) {
            *x = 960.0;
        }
        assert!(looks_inverted(&Image2D { width: 20, height: 20, data: d }));
        // 明暗を反転した写真で星像が小さいとき: 明るい星のピークの 1 画素の背景との差が 10 番目の 10 倍を超える
        // (隣の画素も背景との差が大きい)→ 反転する
        let (w, h) = (60, 50);
        let mut neg = vec![20_000.0f32; w * h];
        let c = 25 * w + 30;
        neg[c] = 0.0;
        for o in [c - 1, c + 1, c - w, c + w] {
            neg[o] = 20_000.0 - 8_400.0;
        }
        for o in [c - w - 1, c - w + 1, c + w - 1, c + w + 1] {
            neg[o] = 20_000.0 - 3_500.0;
        }
        for i in (0..w * h).step_by(97).skip(1) {
            neg[i] = 20_000.0 - 300.0; // 暗い星(反転した写真では薄い灰色)
        }
        neg[7] = 20_100.0; // 明るい側の幅 100: 10 番目の値(背景との差 300)で測ると 5% の 15 を超えて反転しない。最小値(背景との差 20,000)で測れば反転
        assert!(looks_inverted(&Image2D { width: w, height: h, data: neg }));
    }

    #[test]
    fn inverted_same_minimum_twice_does_not_depend_on_orientation() {
        // 最も暗い値(599)の画素が 2 個。片方は孤立、もう片方は隣の画素も背景との差が大きい(星のピーク)。
        // 上下を反転しても判定は同じ
        let mut d = vec![1000.0f32; 400];
        d[0] = 1010.0;
        d[2 * 20 + 5] = 599.0;
        d[15 * 20 + 10] = 599.0;
        d[15 * 20 + 11] = 880.0;
        for x in d.iter_mut().skip(100).take(DARK_RANK - 3) {
            *x = 960.0;
        }
        let flipped: Vec<f32> = d.chunks(20).rev().flatten().copied().collect();
        let a = looks_inverted(&Image2D { width: 20, height: 20, data: d });
        let b = looks_inverted(&Image2D { width: 20, height: 20, data: flipped });
        assert_eq!(a, b);
        assert!(a, "孤立していない最小値があるので最小値で測って反転する");
    }

    #[test]
    fn inverted_inconsistent_size_does_not_panic() {
        // 大きさと画素の数が合わない Image2D(公開関数に直接渡されたもの)でも panic しない(孤立とみず最小値で測る)
        let mut d = vec![200.0f32; 20];
        d[3] = 10.0;
        assert!(looks_inverted(&Image2D { width: 0, height: 0, data: d.clone() }));
        assert!(looks_inverted(&Image2D { width: 7, height: 0, data: d }));
    }

    #[test]
    fn inverted_bayer_outlier_uses_outer_ring() {
        // ベイヤーの平滑化で外れ値 1 画素が 3×3 に広がる。読み込みでは 5×5 の外周を見るので外れ値とみて反転しない
        let (nx, ny) = (64, 48);
        let mut d = vec![400.0f32; nx * ny];
        for i in (0..nx * ny).step_by(131) {
            d[i] = 2_000.0;
        }
        d[20 * nx + 30] = -2.0e7;
        let bayer = vec![h("BAYERPAT", CardValue::Str("RGGB".into()))];
        let l = finish(d, ny, nx, bayer, vec![], false).unwrap();
        assert!(!l.note.contains("反転"), "{}", l.note);
        // 同じ平滑化後の面を「平滑化していない」として判定すると、隣の 8 画素も背景との差が大きいので星とみて反転する
        assert!(looks_inverted(&l.data));
    }

    #[test]
    fn kth_smallest_few_matches_kth_pair() {
        use crate::stats::kth_pair;
        let mut r = Rng(0x0dd5_eed5_1234_9876);
        for n in [1usize, 2, 9, 10, 11, 1000, 65_536, 65_537, 200_003] {
            let mut v: Vec<f32> = (0..n).map(|_| (r.next() * 50.0).floor()).collect();
            if n > 20 {
                v[n / 3] = -1.0e6;
                v[n / 2] = -1.0e6;
            }
            for k in [1usize, 2, 5, 10] {
                if k <= n {
                    assert_eq!(kth_smallest_few(&v, k), kth_pair(&v, k - 1).0, "n={n} k={k}");
                }
            }
        }
    }
}

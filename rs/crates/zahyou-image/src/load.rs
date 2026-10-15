//! `load_image_any`・`looks_inverted`(Python 版 dev/zahyou_core.py の同名の関数)。
//!
//! 拡張子が FITS のもの(.fits .fit .fts .fz .fits.gz .fit.gz)は FITS → 画像ファイルの順、それ以外は逆の順で試す
//! (拡張子と中身が食い違うファイル対策)。読めたら
//! 1. 2 次元・16 画素以上かを確かめる
//! 2. 表示用に今の値を写す
//! 3. ベイヤー配列のキーがあれば 3×3 の二項フィルター [1,2,1]⊗[1,2,1]/16 で市松模様を消す(星の位置はずれない)
//! 4. 非有限値を有限値の中央値で埋める
//! 5. 一様なら Err
//! 6. 白地に黒い星(星図・ネガ)なら検出用の面だけ明暗を反転する
//!
//! Python 版との違い: 拡張子が FITS でないのに中身が FITS のファイルは、Python では Pillow の FITS プラグインが
//! 16 bit を逆のバイト順で読んで値が化けていた。ここでは画像ファイルとして読めないので FITS として正しく読む。

use std::path::Path;

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

/// 読み込み器の結果(2 次元の画素、高さ、幅、ヘッダー、説明、上下反転)。
type Raw = (Vec<f32>, usize, usize, Vec<Card>, Vec<String>, bool);
/// 読み込み器(FITS か画像ファイル)
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

/// `looks_inverted`: 白地に黒い星(星図・ネガ)か。背景(中央値)が明るさの幅の上端に寄っていて、
/// 上への伸び(最大 − 中央値)が下への伸び(中央値 − 最小)の 5% 以下なら真。
pub fn looks_inverted(data: &Image2D) -> bool {
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
    let med = median_finite_f32(v) as f64;
    let (lo, hi) = min_max(v);
    let up = hi as f64 - med;
    let down = med - lo as f64;
    down > 0.0 && up <= down * 0.05
}

fn min_max(v: &[f32]) -> (f32, f32) {
    v.par_chunks(1 << 16)
        .map(|c| c.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &x| (a.min(x), b.max(x))))
        .reduce(|| (f32::INFINITY, f32::NEG_INFINITY), |(a, b), (c, d)| (a.min(c), b.max(d)))
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
    let Some((mut data, ny, nx, header, mut notes, flip_y)) = got else {
        return Err(LoadError::Unreadable(first_error.unwrap_or_default()));
    };
    if data.len() != ny * nx {
        return Err(LoadError::NotTwoDimensional(format!("({ny}, {nx})")));
    }
    if ny.min(nx) < 16 {
        return Err(LoadError::TooSmall { height: ny, width: nx });
    }
    let display = Image2D { width: nx, height: ny, data: data.clone() };

    if let Some(pat) = bayer_pattern(&header) {
        // 2x2 の平均 (v6 まで) は窓が片側に寄るので星の位置が +0.5 px ずれていた。対称な 3x3 の二項カーネルならずれない
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
    // 星図 (白地に黒い星) は明暗が逆。検出に使う面だけ反転する(表示は元のまま)
    if looks_inverted(&img) {
        let mx = hi;
        img.data.par_iter_mut().for_each(|v| *v = mx - *v);
        notes.push("白地に黒い星 (星図) とみて明暗を反転".to_string());
    }

    Ok(Loaded { data: img, display, header, note: notes.join(" / "), flip_y })
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

    #[test]
    fn inverted() {
        let mut d = vec![200.0f32; 400];
        d[5] = 10.0;
        d[6] = 50.0;
        assert!(looks_inverted(&Image2D { width: 20, height: 20, data: d.clone() }));
        d[7] = 255.0;
        assert!(!looks_inverted(&Image2D { width: 20, height: 20, data: d }));
    }
}

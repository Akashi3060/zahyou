//! タイル圧縮の画像(FITS 規格の Tiled Image Compression。ZIMAGE = T の BINTABLE)を展開する。
//!
//! astropy の `decompress_image_data_section`(`astropy/io/fits/hdu/compressed/_tiled_compression.py`)と
//! 同じ手順・同じ値にする。cfitsio は使わず、規約から自前で書いた。
//!
//! - 表の各行が 1 タイル。COMPRESSED_DATA 列(可変長配列 1PB / 1QB)がヒープ上の圧縮データを指す。
//!   タイルの並びは numpy の順で最後の軸(FITS の軸 1)が最も速い。端のタイルは画像の端で切れる。
//! - 方式: RICE_1(cfitsio の fits_rdecomp と同じ符号)、GZIP_1、GZIP_2(バイトの並べ替え)、NOCOMPRESS。
//!   PLIO_1・HCOMPRESS_1 は未対応(分かりやすい Err)。
//! - 浮動小数の画像は整数に量子化されている(ZSCALE・ZZERO 列)。ZQUANTIZ が NO_DITHER なら
//!   値 = 整数 × ZSCALE + ZZERO、SUBTRACTIVE_DITHER_1/2 なら cfitsio の乱数列(Park–Miller、1 万個の float32)で
//!   値 = (整数 − 乱数 + 0.5) × ZSCALE + ZZERO(_2 は整数 −2147483646 が 0.0)。ZBLANK の画素は NaN。
//! - COMPRESSED_DATA が空の行は GZIP_COMPRESSED_DATA 列(量子化できなかったタイルを浮動小数のまま gzip)を使う。
//! - 返すのは展開した配列(ZBITPIX の型)と、astropy と同じく画像のヘッダーに戻したカード。

use std::io::Read;
use std::sync::OnceLock;

use flate2::read::MultiGzDecoder;
use rayon::prelude::*;
use zahyou_wcs::{Card, CardValue};

use crate::array::{MemArray, Scaling, Typed};
use crate::fits::{Hdu, Src, get, get_int, get_num, get_str};

/// 展開した画素の数の上限(astropy も ZNAXISn を int32 の範囲に限る)
const MAX_PIXELS: u128 = 1 << 31;
/// 一度に並列で展開するタイルの数
const TILE_BATCH: usize = 512;

// ----------------------------------------------------------- 表の列 ---

#[derive(Clone, Debug)]
struct Column {
    name: String,
    /// 行の中の位置(バイト)
    offset: usize,
    /// 型の文字(L X B I J K A E D C M P Q)
    code: u8,
    /// P・Q の要素の型(B I J …)
    elem: u8,
}

/// TFORM の 1 要素の幅(バイト)。P は記述子 2 × int32、Q は 2 × int64。
fn tform_width(code: u8) -> Option<usize> {
    Some(match code {
        b'L' | b'B' | b'A' => 1,
        b'I' => 2,
        b'J' | b'E' => 4,
        b'K' | b'D' | b'C' | b'P' => 8,
        b'M' | b'Q' => 16,
        _ => return None,
    })
}

/// "1PB(399)"・"PB"・"1D"・"16A" などを (繰り返し, 型, 要素の型) に。
fn parse_tform(t: &str) -> Result<(usize, u8, u8), String> {
    let t = t.trim().as_bytes();
    let digits = t.iter().take_while(|c| c.is_ascii_digit()).count();
    let repeat = if digits == 0 {
        1
    } else {
        std::str::from_utf8(&t[..digits])
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or("TFORM の繰り返しが不正です")?
    };
    let code = *t.get(digits).ok_or("TFORM に型がありません")?;
    let code = code.to_ascii_uppercase();
    let elem = if code == b'P' || code == b'Q' {
        t.get(digits + 1).map(|c| c.to_ascii_uppercase()).unwrap_or(b'B')
    } else {
        0
    };
    Ok((repeat, code, elem))
}

fn columns(cards: &[Card], naxis1: usize) -> Result<Vec<Column>, String> {
    let tfields = get_int(cards, "TFIELDS")?.unwrap_or(0);
    if !(0..=999).contains(&tfields) {
        return Err(format!("TFIELDS が範囲外です: {tfields}"));
    }
    let mut cols = Vec::new();
    let mut off = 0usize;
    for i in 1..=tfields {
        let tform = get_str(cards, &format!("TFORM{i}")).ok_or_else(|| format!("TFORM{i} がありません"))?;
        let name = get_str(cards, &format!("TTYPE{i}")).unwrap_or("").trim().to_ascii_uppercase();
        let (repeat, code, elem) = parse_tform(tform)?;
        let width = if code == b'X' {
            repeat.div_ceil(8)
        } else {
            tform_width(code).ok_or_else(|| format!("TFORM{i} の型が不正です: {tform}"))?.saturating_mul(repeat)
        };
        cols.push(Column { name, offset: off, code, elem });
        off = off.saturating_add(width);
    }
    if off > naxis1 {
        return Err(format!("表の列の幅の和 {off} が NAXIS1 = {naxis1} を超えています"));
    }
    Ok(cols)
}

// ---------------------------------------------------- 画像のヘッダー ---

/// 表のヘッダーから画像のヘッダーへ戻すとき捨てるキーか。
fn is_table_key(k: &str) -> bool {
    const FIXED: &[&str] = &[
        "XTENSION", "BITPIX", "NAXIS", "PCOUNT", "GCOUNT", "TFIELDS", "THEAP", "ZIMAGE", "ZCMPTYPE", "ZBITPIX",
        "ZNAXIS", "ZMASKCMP", "ZQUANTIZ", "ZDITHER0", "ZSIMPLE", "ZTENSION", "ZEXTEND", "ZBLOCKED", "ZPCOUNT",
        "ZGCOUNT", "ZHECKSUM", "ZDATASUM", "ZBLANK", "ZSCALE", "ZZERO", "CHECKSUM", "DATASUM",
    ];
    if FIXED.contains(&k) {
        return true;
    }
    const NUMBERED: &[&str] = &[
        "NAXIS", "TTYPE", "TFORM", "TUNIT", "TNULL", "TSCAL", "TZERO", "TDISP", "TDIM", "TBCOL", "ZNAXIS", "ZTILE",
        "ZNAME", "ZVAL",
    ];
    NUMBERED.iter().any(|p| k.len() > p.len() && k.starts_with(p) && k[p.len()..].bytes().all(|c| c.is_ascii_digit()))
}

/// astropy の CompImageHDU.header と同じ形: XTENSION(か SIMPLE)・BITPIX・NAXIS・NAXISn・PCOUNT・GCOUNT、
/// 続いて表の構造と圧縮のキー以外のカード(EXTNAME = 'COMPRESSED_IMAGE' は捨てる)。
fn image_header(cards: &[Card]) -> Result<Vec<Card>, String> {
    let mut out = Vec::new();
    let primary = get(cards, "ZSIMPLE").is_some();
    if primary {
        out.push(Card::new("SIMPLE", CardValue::Bool(true), "conforms to FITS standard"));
    } else {
        let x = get_str(cards, "ZTENSION").unwrap_or("IMAGE").to_string();
        out.push(Card::new("XTENSION", CardValue::Str(x), "Image extension"));
    }
    let zbitpix = get_int(cards, "ZBITPIX")?.ok_or("ZBITPIX がありません")?;
    out.push(Card::new("BITPIX", CardValue::Int(zbitpix), "array data type"));
    let znaxis = get_int(cards, "ZNAXIS")?.ok_or("ZNAXIS がありません")?;
    out.push(Card::new("NAXIS", CardValue::Int(znaxis), "number of array dimensions"));
    for i in 1..=znaxis.clamp(0, 999) {
        let n = get_int(cards, &format!("ZNAXIS{i}"))?.ok_or_else(|| format!("ZNAXIS{i} がありません"))?;
        out.push(Card::new(&format!("NAXIS{i}"), CardValue::Int(n), ""));
    }
    if primary {
        if let Some(v) = get(cards, "ZEXTEND") {
            out.push(Card::new("EXTEND", v.clone(), ""));
        }
    } else {
        let p = get_int(cards, "ZPCOUNT")?.unwrap_or(0);
        let g = get_int(cards, "ZGCOUNT")?.unwrap_or(1);
        out.push(Card::new("PCOUNT", CardValue::Int(p), "number of parameters"));
        out.push(Card::new("GCOUNT", CardValue::Int(g), "number of groups"));
    }
    for c in cards {
        if is_table_key(&c.key) {
            continue;
        }
        if c.key == "EXTNAME" && matches!(&c.value, CardValue::Str(s) if s.trim() == "COMPRESSED_IMAGE") {
            continue;
        }
        out.push(c.clone());
    }
    Ok(out)
}

// ------------------------------------------------------------- RICE_1 ---

/// cfitsio の fits_rdecomp / fits_rdecomp_short / fits_rdecomp_byte。`nx` 個の画素を返す
/// (bytepix 1 は 0..255、2 は int16、4 は int32 の値)。圧縮データを読み切っても足りなければ Err。
pub(crate) fn rice_decode(c: &[u8], nx: usize, nblock: usize, bytepix: usize) -> Result<Vec<i64>, String> {
    let (fsbits, fsmax, bbits): (i32, i32, i32) = match bytepix {
        1 => (3, 6, 8),
        2 => (4, 14, 16),
        4 => (5, 25, 32),
        _ => return Err(format!("RICE_1 の BYTEPIX = {bytepix} には対応していません")),
    };
    if nblock == 0 {
        return Err("RICE_1 の BLOCKSIZE が 0 です".into());
    }
    let mask: u32 = if bytepix == 4 { u32::MAX } else { (1u32 << (8 * bytepix)) - 1 };
    if c.len() < bytepix {
        return Err("RICE_1 の圧縮データが短すぎます".into());
    }
    let mut lastpix: u32 = 0;
    for &b in &c[..bytepix] {
        lastpix = (lastpix << 8) | b as u32;
    }
    let mut p = bytepix;
    let short = || "RICE_1 の圧縮データが途中で切れています".to_string();
    macro_rules! next {
        () => {{
            let v = *c.get(p).ok_or_else(short)? as u32;
            p += 1;
            v
        }};
    }
    let mut out: Vec<i64> = Vec::new();
    out.try_reserve_exact(nx).map_err(|_| "タイルが大きすぎてメモリに載りません".to_string())?;
    let to_i64 = |v: u32| -> i64 {
        match bytepix {
            1 => v as u8 as i64,
            2 => v as u16 as i16 as i64,
            _ => v as i32 as i64,
        }
    };
    let mut b: u32 = next!();
    let mut nbits: i32 = 8;
    let mut i = 0usize;
    while i < nx {
        // ブロックの FS を fsbits ビット読む
        nbits -= fsbits;
        while nbits < 0 {
            b = (b << 8) | next!();
            nbits += 8;
        }
        let fs: i32 = (b >> nbits) as i32 - 1;
        b &= (1u32 << nbits) - 1;
        let imax = (i + nblock).min(nx);
        if fs < 0 {
            // 差がすべて 0
            let v = to_i64(lastpix);
            out.extend(std::iter::repeat_n(v, imax - i));
        } else if fs == fsmax {
            // 差をそのまま bbits ビットで書いてある
            for _ in i..imax {
                let mut k: i32 = bbits - nbits;
                let mut diff: u32 = b.checked_shl(k as u32).unwrap_or(0);
                k -= 8;
                while k >= 0 {
                    b = next!();
                    diff |= b << k;
                    k -= 8;
                }
                if nbits > 0 {
                    b = next!();
                    diff |= b >> (-k);
                    b &= (1u32 << nbits) - 1;
                } else {
                    b = 0;
                }
                diff = if diff & 1 == 0 { diff >> 1 } else { !(diff >> 1) };
                lastpix = diff.wrapping_add(lastpix) & mask;
                out.push(to_i64(lastpix));
            }
        } else {
            // ふつうの Rice 符号: 先頭の 0 の数 + fs ビット
            for _ in i..imax {
                while b == 0 {
                    nbits += 8;
                    b = next!();
                }
                let nzero = nbits - (32 - b.leading_zeros()) as i32;
                nbits -= nzero + 1;
                if !(0..32).contains(&nbits) {
                    return Err("RICE_1 の圧縮データが壊れています".into());
                }
                b ^= 1u32 << nbits;
                nbits -= fs;
                while nbits < 0 {
                    b = (b << 8) | next!();
                    nbits += 8;
                }
                if nbits >= 32 {
                    return Err("RICE_1 の圧縮データが壊れています".into());
                }
                let d0 = ((nzero as u32).wrapping_shl(fs as u32)) | (b >> nbits);
                b &= (1u32 << nbits) - 1;
                let diff = if d0 & 1 == 0 { d0 >> 1 } else { !(d0 >> 1) };
                lastpix = diff.wrapping_add(lastpix) & mask;
                out.push(to_i64(lastpix));
            }
        }
        i = imax;
    }
    Ok(out)
}

// --------------------------------------------------------------- GZIP ---

/// gzip を展開する(`limit` バイトを超えたら Err。圧縮爆弾よけ)。
fn gunzip(c: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut d = MultiGzDecoder::new(c).take(limit as u64 + 1);
    let mut out = Vec::new();
    d.read_to_end(&mut out).map_err(|e| format!("タイルの gzip を展開できません: {e}"))?;
    if out.len() > limit {
        return Err("タイルの gzip を展開した中身が大きすぎます".into());
    }
    Ok(out)
}

/// 展開したバイト列の型を大きさで決める(astropy の `_finalize_array`。cfitsio の imcompress.c と同じ規則)。
/// 戻り値は (値, 1 要素のバイト数)。
fn finalize_bytes(buf: &[u8], tile_size: usize, bitpix: i64, lossless: bool) -> Result<(TileVals, usize), String> {
    let n = buf.len();
    if tile_size > 0 && n == tile_size * 2 {
        return Ok((TileVals::I(buf.chunks_exact(2).map(|r| i16::from_be_bytes([r[0], r[1]]) as i64).collect()), 2));
    }
    if tile_size > 0 && n == tile_size * 4 {
        let v = if bitpix < 0 && lossless {
            TileVals::F(buf.chunks_exact(4).map(|r| f32::from_be_bytes([r[0], r[1], r[2], r[3]]) as f64).collect())
        } else {
            TileVals::I(buf.chunks_exact(4).map(|r| i32::from_be_bytes([r[0], r[1], r[2], r[3]]) as i64).collect())
        };
        return Ok((v, 4));
    }
    if tile_size > 0 && n == tile_size * 8 {
        let v = if bitpix < 0 && lossless {
            TileVals::F(
                buf.chunks_exact(8)
                    .map(|r| f64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]))
                    .collect(),
            )
        } else {
            TileVals::I(
                buf.chunks_exact(8)
                    .map(|r| i64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]))
                    .collect(),
            )
        };
        return Ok((v, 8));
    }
    if n == tile_size {
        return Ok((TileVals::U8(buf.to_vec()), 1));
    }
    Err(format!("タイルを展開した大きさ {n} バイトが画素数 {tile_size} と合いません"))
}

// -------------------------------------------------------- 量子化の乱数 ---

/// cfitsio の fits_init_randoms: Park–Miller の乱数 1 万個(float32)。
fn randoms() -> &'static [f32] {
    static R: OnceLock<Vec<f32>> = OnceLock::new();
    R.get_or_init(|| {
        let a = 16807.0f64;
        let m = 2147483647.0f64;
        let mut seed = 1.0f64;
        let mut v = Vec::with_capacity(10000);
        for _ in 0..10000 {
            let temp = a * seed;
            seed = temp - m * ((temp / m) as i32 as f64);
            v.push((seed / m) as f32);
        }
        v
    })
}

const N_RANDOM: i64 = 10000;
const ZERO_VALUE: i64 = -2147483646;

/// 展開したタイルの値(整数か浮動小数)。
enum TileVals {
    U8(Vec<u8>),
    I(Vec<i64>),
    F(Vec<f64>),
}

impl TileVals {
    fn len(&self) -> usize {
        match self {
            TileVals::U8(v) => v.len(),
            TileVals::I(v) => v.len(),
            TileVals::F(v) => v.len(),
        }
    }

    /// i 番目が整数 z と等しいか(numpy の tile_data == zblank)
    fn eq_num(&self, i: usize, z: i64) -> bool {
        match self {
            TileVals::U8(v) => v[i] as i64 == z,
            TileVals::I(v) => v[i] == z,
            TileVals::F(v) => v[i] == z as f64,
        }
    }
}

/// 量子化した整数 → 浮動小数(`Quantize.decode_quantized`)。`itemsize` は整数の型の大きさ。
fn unquantize(
    q: &TileVals,
    itemsize: usize,
    row: i64,
    scale: f64,
    zero: f64,
    dither: i64,
    out_f32: bool,
) -> Result<Vec<f64>, String> {
    let n = q.len();
    let get = |i: usize| -> f64 {
        match q {
            TileVals::U8(v) => v[i] as f64,
            TileVals::I(v) => v[i] as f64,
            TileVals::F(v) => v[i],
        }
    };
    if dither == -1 {
        // NO_DITHER: numpy で q * scale + zero(float64)
        return Ok((0..n).map(|i| get(i) * scale + zero).collect());
    }
    if !matches!(itemsize, 1 | 2 | 4) || matches!(q, TileVals::F(_)) {
        return Err("量子化された整数の型が不正です".into());
    }
    let r = randoms();
    let mut iseed = (row - 1).rem_euclid(N_RANDOM) as usize;
    let mut nextrand = (r[iseed] * 500.0) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let qi = match q {
            TileVals::U8(v) => v[i] as i64,
            TileVals::I(v) => v[i],
            TileVals::F(_) => 0,
        };
        let v = if dither == 2 && itemsize == 4 && qi == ZERO_VALUE {
            0.0
        } else {
            let x = ((qi as f64 - r[nextrand] as f64) + 0.5) * scale + zero;
            if out_f32 { x as f32 as f64 } else { x }
        };
        out.push(v);
        nextrand += 1;
        if nextrand == N_RANDOM as usize {
            iseed += 1;
            if iseed == N_RANDOM as usize {
                iseed = 0;
            }
            nextrand = (r[iseed] * 500.0) as usize;
        }
    }
    Ok(out)
}

// ------------------------------------------------------------- 本体 ---

/// 表の 1 行から読む値。
fn read_desc(row: &[u8], col: &Column) -> Result<(u64, u64), String> {
    let o = col.offset;
    match col.code {
        b'P' => {
            let r = row.get(o..o + 8).ok_or("表の行が短すぎます")?;
            let n = i32::from_be_bytes([r[0], r[1], r[2], r[3]]);
            let off = i32::from_be_bytes([r[4], r[5], r[6], r[7]]);
            if n < 0 || off < 0 {
                return Err("可変長配列の記述子が負です".into());
            }
            Ok((n as u64, off as u64))
        }
        b'Q' => {
            let r = row.get(o..o + 16).ok_or("表の行が短すぎます")?;
            let n = i64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]);
            let off = i64::from_be_bytes([r[8], r[9], r[10], r[11], r[12], r[13], r[14], r[15]]);
            if n < 0 || off < 0 {
                return Err("可変長配列の記述子が負です".into());
            }
            Ok((n as u64, off as u64))
        }
        _ => Err(format!("{} 列が可変長配列ではありません", col.name)),
    }
}

fn read_num(row: &[u8], col: &Column) -> Result<f64, String> {
    let o = col.offset;
    let g = |w: usize| row.get(o..o + w).ok_or_else(|| "表の行が短すぎます".to_string());
    Ok(match col.code {
        b'D' => {
            let r = g(8)?;
            f64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]])
        }
        b'E' => {
            let r = g(4)?;
            f32::from_be_bytes([r[0], r[1], r[2], r[3]]) as f64
        }
        b'J' => {
            let r = g(4)?;
            i32::from_be_bytes([r[0], r[1], r[2], r[3]]) as f64
        }
        b'I' => {
            let r = g(2)?;
            i16::from_be_bytes([r[0], r[1]]) as f64
        }
        b'K' => {
            let r = g(8)?;
            i64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]) as f64
        }
        b'B' => g(1)?[0] as f64,
        _ => return Err(format!("{} 列の型には対応していません", col.name)),
    })
}

fn elem_size(e: u8) -> u64 {
    match e {
        b'I' => 2,
        b'J' | b'E' => 4,
        b'K' | b'D' => 8,
        _ => 1,
    }
}

/// 圧縮の設定(ZNAMEn / ZVALn の組)。
fn setting(cards: &[Card], name: &str, default: i64) -> Result<i64, String> {
    for i in 1..1000 {
        let Some(n) = get(cards, &format!("ZNAME{i}")) else { break };
        if let CardValue::Str(s) = n
            && s.trim().eq_ignore_ascii_case(name)
        {
            return get_int(cards, &format!("ZVAL{i}"))?.ok_or_else(|| format!("ZVAL{i} がありません"));
        }
    }
    Ok(default)
}

/// 1 タイルぶんの設定。
struct Ctx<'a> {
    table: &'a [u8],
    naxis1: usize,
    heap: usize,
    cmp: String,
    zbitpix: i64,
    blocksize: usize,
    bytepix: usize,
    dither: i64,
    dither_seed: i64,
    quantized: bool,
    zblank_header: Option<i64>,
    c_data: Column,
    c_gzip: Option<Column>,
    c_scale: Option<Column>,
    c_zero: Option<Column>,
    c_blank: Option<Column>,
}

impl Ctx<'_> {
    fn heap_slice(&self, n: u64, off: u64, esize: u64) -> Result<&[u8], String> {
        let bytes = n.checked_mul(esize).ok_or("ヒープの大きさが不正です")?;
        let s = (self.heap as u64).checked_add(off).ok_or("ヒープの位置が不正です")?;
        let e = s.checked_add(bytes).ok_or("ヒープの位置が不正です")?;
        if e > self.table.len() as u64 {
            return Err("タイルの圧縮データがヒープの外を指しています".into());
        }
        Ok(&self.table[s as usize..e as usize])
    }

    /// row 行目のタイル(画素数 tile_size)を展開する。
    fn decode(&self, row: usize, tile_size: usize) -> Result<TileVals, String> {
        let rb = &self.table[row * self.naxis1..(row + 1) * self.naxis1];
        let (n, off) = read_desc(rb, &self.c_data)?;
        let limit = tile_size.saturating_mul(8).max(1);
        if n == 0 {
            // 量子化できなかったタイル: 浮動小数のまま gzip
            let col = self.c_gzip.as_ref().ok_or("COMPRESSED_DATA が空で GZIP_COMPRESSED_DATA 列もありません")?;
            let (gn, goff) = read_desc(rb, col)?;
            let c = self.heap_slice(gn, goff, elem_size(col.elem))?;
            return Ok(finalize_bytes(&gunzip(c, limit)?, tile_size, self.zbitpix, true)?.0);
        }
        let c = self.heap_slice(n, off, elem_size(self.c_data.elem))?;
        let lossless = !self.quantized;
        // (値, 1 要素のバイト数)。量子化の関数は astropy と同じくこの大きさで選ぶ
        let (mut vals, itemsize) = match self.cmp.as_str() {
            "RICE_1" | "RICE_ONE" => {
                if self.c_data.elem != b'B' {
                    return Err("RICE_1 の圧縮データの型が 1PB ではありません".into());
                }
                (TileVals::I(rice_decode(c, tile_size, self.blocksize, self.bytepix)?), self.bytepix)
            }
            "GZIP_1" => finalize_bytes(&gunzip(c, limit)?, tile_size, self.zbitpix, lossless)?,
            "GZIP_2" => {
                let raw = gunzip(c, limit)?;
                let item = if tile_size == 0 { 0 } else { raw.len() / tile_size };
                if item == 0 || raw.len() % item != 0 {
                    return Err("GZIP_2 のタイルの大きさが画素数と合いません".into());
                }
                // 並べ替えを戻す: 上位バイトの列 → 画素ごとのバイト
                let m = raw.len() / item;
                let mut un = vec![0u8; raw.len()];
                for (k, part) in raw.chunks_exact(m).enumerate() {
                    for (j, &b) in part.iter().enumerate() {
                        un[j * item + k] = b;
                    }
                }
                finalize_bytes(&un, tile_size, self.zbitpix, lossless)?
            }
            "NOCOMPRESS" => finalize_bytes(c, tile_size, self.zbitpix, lossless)?,
            "PLIO_1" | "HCOMPRESS_1" => {
                return Err(format!(
                    "タイル圧縮 {} には対応していません (RICE_1・GZIP_1・GZIP_2・NOCOMPRESS のみ)",
                    self.cmp
                ));
            }
            other => return Err(format!("タイル圧縮の方式 '{other}' を知りません")),
        };
        if vals.len() != tile_size {
            return Err(format!("タイルの画素数 {} が {tile_size} と合いません", vals.len()));
        }
        let zblank: Option<i64> = match &self.c_blank {
            Some(col) => Some(read_num(rb, col)? as i64),
            None => self.zblank_header,
        };
        let blank_mask: Option<Vec<bool>> = zblank.map(|z| (0..vals.len()).map(|i| vals.eq_num(i, z)).collect());
        if self.quantized {
            let scale = read_num(rb, self.c_scale.as_ref().ok_or("ZSCALE 列がありません")?)?;
            let zero = match &self.c_zero {
                Some(c) => read_num(rb, c)?,
                None => return Err("ZZERO 列がありません".into()),
            };
            if self.zbitpix > 0 {
                return Err("量子化された画像の ZBITPIX が -32・-64 ではありません".into());
            }
            let row_seed = if self.dither != -1 { row as i64 + self.dither_seed } else { 0 };
            vals = TileVals::F(unquantize(&vals, itemsize, row_seed, scale, zero, self.dither, self.zbitpix == -32)?);
        }
        if let (Some(m), Some(z)) = (blank_mask, zblank) {
            match &mut vals {
                TileVals::F(v) if self.zbitpix < 0 => m.iter().zip(v.iter_mut()).for_each(|(&b, x)| {
                    if b {
                        *x = f64::NAN;
                    }
                }),
                TileVals::F(v) => m.iter().zip(v.iter_mut()).for_each(|(&b, x)| {
                    if b {
                        *x = z as f64;
                    }
                }),
                TileVals::I(v) => m.iter().zip(v.iter_mut()).for_each(|(&b, x)| {
                    if b {
                        *x = z;
                    }
                }),
                TileVals::U8(v) => m.iter().zip(v.iter_mut()).for_each(|(&b, x)| {
                    if b {
                        *x = z as u8;
                    }
                }),
            }
        }
        Ok(vals)
    }
}

/// 出力の配列へ 1 タイルを書く(numpy の image_data[tile_slices] = tile_data。型は代入で変換)。
fn scatter(out: &mut Typed, shape: &[usize], start: &[usize], tshape: &[usize], vals: &TileVals) {
    let nd = shape.len();
    let run = tshape[nd - 1];
    if run == 0 || tshape.contains(&0) {
        return;
    }
    // 先頭側の軸の添字(オドメーター)
    let mut idx = vec![0usize; nd - 1];
    let mut src = 0usize;
    loop {
        let mut dst = 0usize;
        for d in 0..nd - 1 {
            dst = dst * shape[d] + start[d] + idx[d];
        }
        dst = dst * shape[nd - 1] + start[nd - 1];
        macro_rules! put {
            ($o:expr, $conv:expr) => {{
                let o = &mut $o[dst..dst + run];
                match vals {
                    TileVals::U8(v) => {
                        o.iter_mut().zip(&v[src..src + run]).for_each(|(a, &b)| *a = $conv(b as f64, b as i64))
                    }
                    TileVals::I(v) => o.iter_mut().zip(&v[src..src + run]).for_each(|(a, &b)| *a = $conv(b as f64, b)),
                    TileVals::F(v) => o.iter_mut().zip(&v[src..src + run]).for_each(|(a, &b)| *a = $conv(b, b as i64)),
                }
            }};
        }
        let is_f = matches!(vals, TileVals::F(_));
        match out {
            Typed::U8(o) => put!(o, |f: f64, i: i64| if is_f { f as u8 } else { i as u8 }),
            Typed::I16(o) => put!(o, |f: f64, i: i64| if is_f { f as i16 } else { i as i16 }),
            Typed::U16(o) => put!(o, |f: f64, i: i64| if is_f { f as u16 } else { i as u16 }),
            Typed::I32(o) => put!(o, |f: f64, i: i64| if is_f { f as i32 } else { i as i32 }),
            Typed::I64(o) => put!(o, |f: f64, i: i64| if is_f { f as i64 } else { i }),
            Typed::F32(o) => put!(o, |f: f64, i: i64| if is_f { f as f32 } else { i as f32 }),
            Typed::F64(o) => put!(o, |f: f64, i: i64| if is_f { f } else { i as f64 }),
        }
        src += run;
        // 次の行へ
        let mut d = nd - 1;
        loop {
            if d == 0 {
                return;
            }
            d -= 1;
            idx[d] += 1;
            if idx[d] < tshape[d] {
                break;
            }
            idx[d] = 0;
        }
    }
}

/// タイル圧縮の HDU を展開する。戻り値は (展開した配列, 画像のヘッダー)。
pub(crate) fn decompress(src: &mut Src, hdu: &Hdu) -> Result<(MemArray, Vec<Card>), String> {
    let cards = &hdu.cards;
    let header = image_header(cards)?;
    let naxis1 = get_int(cards, "NAXIS1")?.unwrap_or(0);
    let naxis2 = get_int(cards, "NAXIS2")?.unwrap_or(0);
    if naxis1 < 0 || naxis2 < 0 {
        return Err("表の NAXIS1・NAXIS2 が負です".into());
    }
    let (naxis1, naxis2) = (naxis1 as usize, naxis2 as usize);
    let theap = get_int(cards, "THEAP")?.unwrap_or((naxis1 * naxis2) as i64);
    if theap < 0 {
        return Err("THEAP が負です".into());
    }
    let cols = columns(cards, naxis1)?;
    let find = |n: &str| cols.iter().find(|c| c.name == n).cloned();
    let c_data = find("COMPRESSED_DATA").ok_or("COMPRESSED_DATA 列がありません")?;
    let cmp = get_str(cards, "ZCMPTYPE").ok_or("ZCMPTYPE がありません")?.trim().to_string();
    let zbitpix = get_int(cards, "ZBITPIX")?.ok_or("ZBITPIX がありません")?;
    if !matches!(zbitpix, 8 | 16 | 32 | 64 | -32 | -64) {
        return Err(format!("ZBITPIX が不正です: {zbitpix}"));
    }
    // 画像の形(numpy の順)とタイルの形
    let mut shape = crate::fits::axes(cards, "ZNAXIS")?;
    if shape.is_empty() {
        return Err("ZNAXIS が 0 です".into());
    }
    let mut tile: Vec<usize> = Vec::with_capacity(shape.len());
    for (i, &n) in shape.iter().enumerate() {
        let d = if i == 0 { n as i64 } else { 1 };
        let t = get_int(cards, &format!("ZTILE{}", i + 1))?.unwrap_or(d);
        if t <= 0 || t > i64::from(i32::MAX) {
            return Err(format!("ZTILE{} が不正です: {t}", i + 1));
        }
        tile.push(t as usize);
    }
    shape.reverse();
    tile.reverse();
    let total: u128 = shape.iter().map(|&a| a as u128).product();
    if total > MAX_PIXELS {
        return Err(format!("タイル圧縮の画像が大きすぎます ({total} 画素)"));
    }
    let total = total as usize;
    let ntiles: Vec<usize> = shape.iter().zip(&tile).map(|(&n, &t)| n.div_ceil(t)).collect();
    let n_tiles: usize = ntiles.iter().product();
    if total > 0 && n_tiles > naxis2 {
        return Err(format!("表の行の数 {naxis2} がタイルの数 {n_tiles} より少ない"));
    }
    // 表とヒープを読む
    let need = hdu.data_len;
    if need > src.len().saturating_sub(hdu.data_start) {
        return Err(format!(
            "FITS のデータが途中で切れています (圧縮 HDU[{}] に {need} バイト要るところ、{} バイトしかありません)",
            hdu.index,
            src.len().saturating_sub(hdu.data_start)
        ));
    }
    let mut table = Vec::new();
    table.try_reserve_exact(need as usize).map_err(|_| "圧縮データが大きすぎてメモリに載りません".to_string())?;
    table.resize(need as usize, 0u8);
    src.read_at(hdu.data_start, &mut table)?;
    if naxis1.checked_mul(naxis2).is_none_or(|t| t > table.len()) {
        return Err("表の大きさがデータより大きい".into());
    }
    let dither = match get_str(cards, "ZQUANTIZ").map(|s| s.trim().to_ascii_uppercase()) {
        None => -1,
        Some(s) => match s.as_str() {
            "NONE" => 0,
            "NO_DITHER" => -1,
            "SUBTRACTIVE_DITHER_1" => 1,
            "SUBTRACTIVE_DITHER_2" => 2,
            other => return Err(format!("ZQUANTIZ '{other}' を知りません")),
        },
    };
    let blocksize = setting(cards, "BLOCKSIZE", 32)?;
    let bytepix = setting(cards, "BYTEPIX", 4)?;
    if blocksize <= 0 || !(1..=8).contains(&bytepix) {
        return Err("RICE_1 の BLOCKSIZE・BYTEPIX が不正です".into());
    }
    let zblank_header = match get(cards, "ZBLANK") {
        Some(_) => get_num(cards, "ZBLANK")?.map(|v| v as i64),
        None => get_num(&header, "BLANK").ok().flatten().map(|v| v as i64),
    };
    let ctx = Ctx {
        table: &table,
        naxis1,
        heap: theap as usize,
        cmp,
        zbitpix,
        blocksize: blocksize as usize,
        bytepix: bytepix as usize,
        dither,
        dither_seed: get_int(cards, "ZDITHER0")?.unwrap_or(0),
        quantized: find("ZSCALE").is_some(),
        zblank_header,
        c_data,
        c_gzip: find("GZIP_COMPRESSED_DATA"),
        c_scale: find("ZSCALE"),
        c_zero: find("ZZERO"),
        c_blank: find("ZBLANK"),
    };
    // 出力の配列(ZBITPIX の型)
    fn alloc<T: Clone + Default>(n: usize) -> Result<Vec<T>, String> {
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| format!("画像が大きすぎてメモリに載りません ({n} 画素)"))?;
        v.resize(n, T::default());
        Ok(v)
    }
    let mut out = match zbitpix {
        8 => Typed::U8(alloc(total)?),
        16 => Typed::I16(alloc(total)?),
        32 => Typed::I32(alloc(total)?),
        64 => Typed::I64(alloc(total)?),
        -32 => Typed::F32(alloc(total)?),
        _ => Typed::F64(alloc(total)?),
    };
    if total > 0 {
        let nd = shape.len();
        // タイルの番号 → (始まり, 形)
        let geom = |t: usize| -> (Vec<usize>, Vec<usize>) {
            let mut rem = t;
            let mut ti = vec![0usize; nd];
            for d in (0..nd).rev() {
                ti[d] = rem % ntiles[d];
                rem /= ntiles[d];
            }
            let start: Vec<usize> = (0..nd).map(|d| ti[d] * tile[d]).collect();
            let ts: Vec<usize> = (0..nd).map(|d| tile[d].min(shape[d] - start[d])).collect();
            (start, ts)
        };
        let mut t0 = 0;
        while t0 < n_tiles {
            let t1 = (t0 + TILE_BATCH).min(n_tiles);
            let decoded: Vec<Result<TileVals, String>> = (t0..t1)
                .into_par_iter()
                .map(|t| {
                    let (_, ts) = geom(t);
                    ctx.decode(t, ts.iter().product())
                })
                .collect();
            for (k, r) in decoded.into_iter().enumerate() {
                let vals = r.map_err(|e| format!("タイル {} を展開できません: {e}", t0 + k))?;
                let (start, ts) = geom(t0 + k);
                scatter(&mut out, &shape, &start, &ts, &vals);
            }
            t0 = t1;
        }
    }
    let bscale = get_num(&header, "BSCALE")?.unwrap_or(1.0);
    let bzero = get_num(&header, "BZERO")?.unwrap_or(0.0);
    let blank = match get(&header, "BLANK") {
        Some(CardValue::Int(i)) => Some(*i),
        _ => None,
    };
    let scaling = Scaling::new(zbitpix as i32, bscale, bzero, blank, true);
    Ok((MemArray { shape, data: out, scaling }, header))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn randoms_match_cfitsio() {
        // cfitsio の確かめ: 1 万個目の種は 1043618065
        let a = 16807.0f64;
        let m = 2147483647.0f64;
        let mut seed = 1.0f64;
        for _ in 0..10000 {
            let temp = a * seed;
            seed = temp - m * ((temp / m) as i32 as f64);
        }
        assert_eq!(seed as i64, 1043618065);
        assert_eq!(randoms().len(), 10000);
    }

    #[test]
    fn rice_broken_does_not_panic() {
        for len in 0..40 {
            let c: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            for bp in [1, 2, 4] {
                let _ = rice_decode(&c, 100, 32, bp);
            }
        }
    }

    #[test]
    fn table_keys() {
        assert!(is_table_key("TTYPE12"));
        assert!(is_table_key("ZTILE1"));
        assert!(!is_table_key("TTYPEX"));
        assert!(!is_table_key("BZERO"));
    }
}

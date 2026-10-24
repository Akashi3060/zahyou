//! PNG・JPEG・TIFF を読む(Python 版の `_load_pil`。Pillow と同じ値)。
//!
//! Python 版は Pillow で開き、EXIF の向き(`ImageOps.exif_transpose`)を直したうえで、モードが
//! I;16 / I;16B / I;16L / I / F ならそのままの値、RGB / RGBA / P / YCbCr / CMYK / LA なら `convert("RGB")` して
//! 輝度に合成、それ以外(L・1 など)は `convert("F")` にしている。
//! ここでは画素の復号を image クレート(PNG)と tiff クレートに任せ(JPEG は下記)、Pillow のモードの
//! 決め方と値の変換(16 bit のカラーは上位バイト、1 bit は 0/255、パレットは RGB に引く)を写す。
//! 説明の文も Python と同じ(`None (幅, 高さ) mode=…`。exif_transpose が複製を返すので形式名は None)。
//!
//! JPEG は jpeg.rs(libjpeg-turbo と同じ計算)で復号する。対応していない形式(算術符号・12 bit など)だけ
//! image クレート(zune-jpeg)に任せ、そのときは Pillow と最下位が ±1 ずれることがある。

use std::io::Cursor;

use image::codecs::jpeg::JpegDecoder;
use image::codecs::png::PngDecoder;
use image::{DynamicImage, ImageDecoder};
use rayon::prelude::*;
use tiff::ColorType as TiffColor;
use tiff::decoder::{Decoder as TiffDecoder, DecodingResult, Limits as TiffLimits};
use tiff::tags::Tag;

use crate::array::{MemArray, Scaling, Typed, reduce_to_2d};
use crate::jpeg;

/// 画像が使ってよいメモリの上限(バイト)
const MAX_ALLOC: u64 = 4 << 30;

/// Pillow の画像の中身。
enum Pix {
    /// L・1(convert("F") でそのままの値)
    Gray8(Vec<u8>),
    /// I;16・I;16B
    Gray16(Vec<u16>),
    /// I(32 bit 整数。符号付き 16 bit もここ)
    Int(Vec<i32>),
    /// F
    Float(Vec<f32>),
    /// convert("RGB") した 3 面(画素ごとに R, G, B)
    Rgb(Vec<u8>),
}

struct Pil {
    w: usize,
    h: usize,
    mode: String,
    pix: Pix,
}

/// 読み込んだ 2 次元の画像。
pub(crate) struct RasterImage {
    pub data: Vec<f32>,
    pub ny: usize,
    pub nx: usize,
    pub notes: Vec<String>,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ----------------------------------------------------------- 向き ---

/// Pillow の `exif_transpose` の変換(EXIF の Orientation 2..8)。値の並びだけを入れ替える。
/// 出力の行ごとに並列(以前は 1 画素ずつ直列に `extend_from_slice` していた。値を写すだけなので結果は同じ)。
/// 出力は全部の画素に 1 回ずつ書くので、最初の `T::default()` は残らない。
fn transpose<T: Copy + Default + Send + Sync>(
    v: &[T],
    w: usize,
    h: usize,
    ch: usize,
    orient: u8,
) -> (Vec<T>, usize, usize) {
    // 新しい画像の (x, y) が元のどこから来るか。戻り値は (新しい幅, 新しい高さ)
    let (nw, nh) = if matches!(orient, 5..=8) { (h, w) } else { (w, h) };
    let src = |x: usize, y: usize| -> (usize, usize) {
        match orient {
            2 => (w - 1 - x, y),         // FLIP_LEFT_RIGHT
            3 => (w - 1 - x, h - 1 - y), // ROTATE_180
            4 => (x, h - 1 - y),         // FLIP_TOP_BOTTOM
            5 => (y, x),                 // TRANSPOSE
            6 => (y, h - 1 - x),         // ROTATE_270(時計回りに 90°)
            7 => (w - 1 - y, h - 1 - x), // TRANSVERSE
            8 => (w - 1 - y, x),         // ROTATE_90(反時計回りに 90°)
            _ => (x, y),
        }
    };
    let row = nw * ch;
    // 数の型の 0 は OS の 0 のページをそのまま使う(直列に 0 を書かない)。書き込みは下の並列の中
    let mut out = vec![T::default(); row * nh];
    if row > 0 {
        out.par_chunks_mut(row).enumerate().for_each(|(y, o)| {
            for x in 0..nw {
                let (sx, sy) = src(x, y);
                let s = (sy * w + sx) * ch;
                o[x * ch..x * ch + ch].copy_from_slice(&v[s..s + ch]);
            }
        });
    }
    (out, nw, nh)
}

/// 8 bit の RGBA → RGB(A を捨てる)。並列に写すだけ(以前は直列の `flat_map` → `collect` で、伸ばし直しの
/// 写しも入っていた。値は同じ)。4 の倍数でない端数は以前の `chunks_exact(4)` と同じく捨てる。
fn rgba8_to_rgb(v: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; v.len() / 4 * 3];
    out.par_chunks_exact_mut(3).zip(v.par_chunks_exact(4)).for_each(|(o, p)| o.copy_from_slice(&p[..3]));
    out
}

impl Pil {
    fn apply_orientation(self, orient: u8) -> Pil {
        if !(2..=8).contains(&orient) {
            return self;
        }
        let (w, h) = (self.w, self.h);
        let (pix, nw, nh) = match self.pix {
            Pix::Gray8(v) => {
                let (o, a, b) = transpose(&v, w, h, 1, orient);
                (Pix::Gray8(o), a, b)
            }
            Pix::Gray16(v) => {
                let (o, a, b) = transpose(&v, w, h, 1, orient);
                (Pix::Gray16(o), a, b)
            }
            Pix::Int(v) => {
                let (o, a, b) = transpose(&v, w, h, 1, orient);
                (Pix::Int(o), a, b)
            }
            Pix::Float(v) => {
                let (o, a, b) = transpose(&v, w, h, 1, orient);
                (Pix::Float(o), a, b)
            }
            Pix::Rgb(v) => {
                let (o, a, b) = transpose(&v, w, h, 3, orient);
                (Pix::Rgb(o), a, b)
            }
        };
        Pil { w: nw, h: nh, mode: self.mode, pix }
    }
}

// ------------------------------------------------------------- PNG ---

/// PNG の IHDR から Pillow のモードを決める。
fn png_mode(b: &[u8]) -> Result<&'static str, String> {
    if b.len() < 29 || &b[12..16] != b"IHDR" {
        return Err("PNG の IHDR を読めません".into());
    }
    let depth = b[24];
    let ct = b[25];
    Ok(match (ct, depth) {
        (0, 1) => "1",
        (0, 16) => "I;16",
        (0, _) => "L",
        (2, _) => "RGB",
        (3, _) => "P",
        (4, _) => "LA",
        (6, _) => "RGBA",
        _ => return Err(format!("PNG の色の型 {ct} を知りません")),
    })
}

/// 復号した画像 → Pillow のモードの画素。16 bit のカラーは上位バイト(Pillow の RGB;16B など)。
fn from_dynamic(img: DynamicImage, mode: &str) -> Result<Pix, String> {
    let gray_mode = matches!(mode, "L" | "1");
    Ok(match img {
        DynamicImage::ImageLuma8(b) if gray_mode => Pix::Gray8(b.into_raw()),
        DynamicImage::ImageLumaA8(b) if gray_mode => Pix::Gray8(b.into_raw().chunks_exact(2).map(|p| p[0]).collect()),
        DynamicImage::ImageLuma16(b) if mode == "I;16" => Pix::Gray16(b.into_raw()),
        DynamicImage::ImageLumaA16(b) if mode == "I;16" => {
            Pix::Gray16(b.into_raw().chunks_exact(2).map(|p| p[0]).collect())
        }
        DynamicImage::ImageLuma8(b) => Pix::Rgb(b.into_raw().iter().flat_map(|&g| [g, g, g]).collect()),
        DynamicImage::ImageLumaA8(b) => {
            Pix::Rgb(b.into_raw().chunks_exact(2).flat_map(|p| [p[0], p[0], p[0]]).collect())
        }
        DynamicImage::ImageLuma16(b) => Pix::Rgb(b.into_raw().iter().flat_map(|&g| [(g >> 8) as u8; 3]).collect()),
        DynamicImage::ImageLumaA16(b) => {
            Pix::Rgb(b.into_raw().chunks_exact(2).flat_map(|p| [(p[0] >> 8) as u8; 3]).collect())
        }
        DynamicImage::ImageRgb8(b) => Pix::Rgb(b.into_raw()),
        DynamicImage::ImageRgba8(b) => Pix::Rgb(rgba8_to_rgb(&b.into_raw())),
        DynamicImage::ImageRgb16(b) => Pix::Rgb(b.into_raw().iter().map(|&v| (v >> 8) as u8).collect()),
        DynamicImage::ImageRgba16(b) => Pix::Rgb(
            b.into_raw()
                .chunks_exact(4)
                .flat_map(|p| [(p[0] >> 8) as u8, (p[1] >> 8) as u8, (p[2] >> 8) as u8])
                .collect(),
        ),
        other => return Err(format!("対応していない画素の型です: {:?}", other.color())),
    })
}

fn image_limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_alloc = Some(MAX_ALLOC);
    l
}

fn load_png(bytes: &[u8]) -> Result<(Pil, u8), String> {
    let mode = png_mode(bytes)?;
    let mut dec = PngDecoder::new(Cursor::new(bytes)).map_err(err)?;
    dec.set_limits(image_limits()).map_err(err)?;
    let orient = dec.orientation().map(|o| o.to_exif()).unwrap_or(1);
    let (w, h) = dec.dimensions();
    let img = DynamicImage::from_decoder(dec).map_err(err)?;
    let pix = from_dynamic(img, mode)?;
    Ok((Pil { w: w as usize, h: h as usize, mode: mode.to_string(), pix }, orient))
}

// ------------------------------------------------------------ JPEG ---

fn load_jpeg(bytes: &[u8]) -> Result<(Pil, u8), String> {
    // EXIF の向きは image クレートで読む(APP1 の Exif)
    let orient = JpegDecoder::new(Cursor::new(bytes))
        .ok()
        .and_then(|mut d| d.orientation().ok())
        .map(|o| o.to_exif())
        .unwrap_or(1);
    // まず libjpeg-turbo と同じ値になる自前の復号。対応していない形式なら image クレートで読み直す(±1 ずれうる)
    match jpeg::decode(bytes) {
        Ok(j) => {
            let (mode, pix) = match j.channels {
                1 => ("L", Pix::Gray8(j.pix)),
                3 => ("RGB", Pix::Rgb(j.pix)),
                // Pillow は CMYK の JPEG を常に反転して読み("CMYK;I")、convert("RGB") は cmyk2rgb
                _ => (
                    "CMYK",
                    Pix::Rgb(
                        j.pix
                            .chunks_exact(4)
                            .flat_map(|p| cmyk_to_rgb(255 - p[0], 255 - p[1], 255 - p[2], 255 - p[3]))
                            .collect(),
                    ),
                ),
            };
            return Ok((Pil { w: j.w, h: j.h, mode: mode.to_string(), pix }, orient));
        }
        Err(e) if e.starts_with(jpeg::TRUNCATED) => return Err(e),
        Err(_) => {}
    }
    let mut dec = JpegDecoder::new(Cursor::new(bytes)).map_err(err)?;
    dec.set_limits(image_limits()).map_err(err)?;
    let orient = dec.orientation().map(|o| o.to_exif()).unwrap_or(1);
    let (w, h) = dec.dimensions();
    let img = DynamicImage::from_decoder(dec).map_err(err)?;
    let mode = match img {
        DynamicImage::ImageLuma8(_) => "L",
        _ => "RGB",
    };
    let pix = from_dynamic(img, mode)?;
    Ok((Pil { w: w as usize, h: h as usize, mode: mode.to_string(), pix }, orient))
}

// ------------------------------------------------------------ TIFF ---

/// Pillow の cmyk2rgb(MULDIV255 の丸めも同じ)。
fn cmyk_to_rgb(c: u8, m: u8, y: u8, k: u8) -> [u8; 3] {
    let nk = 255 - k as i32;
    let f = |a: u8| -> u8 {
        let tmp = a as i32 * nk + 128;
        let md = ((tmp >> 8) + tmp) >> 8;
        (nk - md).clamp(0, 255) as u8
    };
    [f(c), f(m), f(y)]
}

fn load_tiff(bytes: &[u8]) -> Result<(Pil, u8), String> {
    let big_endian = bytes.starts_with(b"MM");
    let mut limits = TiffLimits::default();
    limits.decoding_buffer_size = MAX_ALLOC as usize;
    limits.intermediate_buffer_size = MAX_ALLOC as usize;
    let mut dec = TiffDecoder::new(Cursor::new(bytes)).map_err(err)?.with_limits(limits);
    let (w, h) = dec.dimensions().map_err(err)?;
    let (w, h) = (w as usize, h as usize);
    let ct = dec.colortype().map_err(err)?;
    let orient = dec.find_tag_unsigned::<u16>(Tag::Orientation).ok().flatten().unwrap_or(1).min(255) as u8;
    let colormap = if matches!(ct, TiffColor::Palette(_)) { dec.get_tag_u32_vec(Tag::ColorMap).ok() } else { None };
    let n = w.checked_mul(h).ok_or("TIFF が大きすぎます")?;
    let res = dec.read_image().map_err(err)?;
    let bad = || format!("対応していない TIFF です ({ct:?})");
    let (mode, pix): (String, Pix) = match (ct, res) {
        (TiffColor::Gray(1), DecodingResult::U8(v)) => {
            // 1 bit は行ごとにバイトへ詰めてある。Pillow の "1" は 0/255
            let stride = w.div_ceil(8);
            let mut out = Vec::with_capacity(n);
            for y in 0..h {
                for x in 0..w {
                    let byte = *v.get(y * stride + x / 8).ok_or_else(bad)?;
                    out.push(if byte >> (7 - x % 8) & 1 == 1 { 255 } else { 0 });
                }
            }
            ("1".into(), Pix::Gray8(out))
        }
        (TiffColor::Gray(8), DecodingResult::U8(v)) => ("L".into(), Pix::Gray8(v)),
        (TiffColor::Gray(16), DecodingResult::U16(v)) => {
            ((if big_endian { "I;16B" } else { "I;16" }).into(), Pix::Gray16(v))
        }
        (TiffColor::Gray(16), DecodingResult::I16(v)) => ("I".into(), Pix::Int(v.iter().map(|&x| x as i32).collect())),
        (TiffColor::Gray(32), DecodingResult::I32(v)) => ("I".into(), Pix::Int(v)),
        // Pillow の I;32N は符号なしを int32 として読む
        (TiffColor::Gray(32), DecodingResult::U32(v)) => ("I".into(), Pix::Int(v.iter().map(|&x| x as i32).collect())),
        (TiffColor::Gray(32), DecodingResult::F32(v)) => ("F".into(), Pix::Float(v)),
        (TiffColor::Gray(64), DecodingResult::F64(v)) => {
            ("F".into(), Pix::Float(v.iter().map(|&x| x as f32).collect()))
        }
        (TiffColor::GrayA(8), DecodingResult::U8(v)) => {
            ("LA".into(), Pix::Rgb(v.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0]]).collect()))
        }
        (TiffColor::RGB(8), DecodingResult::U8(v)) => ("RGB".into(), Pix::Rgb(v)),
        (TiffColor::RGB(16), DecodingResult::U16(v)) => {
            ("RGB".into(), Pix::Rgb(v.iter().map(|&x| (x >> 8) as u8).collect()))
        }
        (TiffColor::RGBA(8), DecodingResult::U8(v)) => ("RGBA".into(), Pix::Rgb(rgba8_to_rgb(&v))),
        (TiffColor::RGBA(16), DecodingResult::U16(v)) => (
            "RGBA".into(),
            Pix::Rgb(
                v.chunks_exact(4).flat_map(|p| [(p[0] >> 8) as u8, (p[1] >> 8) as u8, (p[2] >> 8) as u8]).collect(),
            ),
        ),
        (TiffColor::CMYK(8), DecodingResult::U8(v)) => {
            ("CMYK".into(), Pix::Rgb(v.chunks_exact(4).flat_map(|p| cmyk_to_rgb(p[0], p[1], p[2], p[3])).collect()))
        }
        (TiffColor::Palette(8), DecodingResult::U8(v)) => {
            // Pillow はカラーマップ(16 bit)を 256 で割って 8 bit にする
            let cm = colormap.ok_or("TIFF のカラーマップがありません")?;
            if cm.len() < 3 * 256 {
                return Err("TIFF のカラーマップが短すぎます".into());
            }
            let pal = |c: usize, i: u8| (cm[c * 256 + i as usize] / 256) as u8;
            ("P".into(), Pix::Rgb(v.iter().flat_map(|&i| [pal(0, i), pal(1, i), pal(2, i)]).collect()))
        }
        _ => return Err(bad()),
    };
    let expect = match &pix {
        Pix::Rgb(v) => v.len() / 3,
        Pix::Gray8(v) => v.len(),
        Pix::Gray16(v) => v.len(),
        Pix::Int(v) => v.len(),
        Pix::Float(v) => v.len(),
    };
    if expect != n {
        return Err(bad());
    }
    Ok((Pil { w, h, mode, pix }, orient))
}

// ------------------------------------------------------------- 本体 ---

/// 中身から画像の形式を決めて読む(Pillow と同じく拡張子は見ない)。FITS の中身なら Err(FITS として読む)。
pub(crate) fn load_raster(bytes: &[u8]) -> Result<RasterImage, String> {
    let (pil, orient) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        load_png(bytes)?
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        load_jpeg(bytes)?
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        load_tiff(bytes)?
    } else if bytes.starts_with(b"SIMPLE") {
        return Err("FITS の中身です (PNG・JPEG・TIFF ではありません)".into());
    } else {
        return Err("PNG・JPEG・TIFF のどれでもありません".into());
    };
    let pil = pil.apply_orientation(orient);
    let mut notes = vec![format!("None ({}, {}) mode={}", pil.w, pil.h, pil.mode)];
    let (w, h) = (pil.w, pil.h);
    // np.asarray(im) の形 (高さ, 幅[, 3]) にして `_reduce_to_2d`
    let (shape, data) = match pil.pix {
        Pix::Gray8(v) => (vec![h, w], Typed::U8(v)),
        Pix::Gray16(v) => (vec![h, w], Typed::U16(v)),
        Pix::Int(v) => (vec![h, w], Typed::I32(v)),
        Pix::Float(v) => (vec![h, w], Typed::F32(v)),
        Pix::Rgb(v) => {
            notes.push("カラー 3 面を輝度に合成".to_string());
            (vec![h, w, 3], Typed::U8(v))
        }
    };
    let bitpix = match &data {
        Typed::U8(_) => 8,
        Typed::U16(_) | Typed::I16(_) => 16,
        Typed::I32(_) => 32,
        Typed::I64(_) => 64,
        Typed::F32(_) => -32,
        Typed::F64(_) => -64,
    };
    let mut arr = MemArray { shape, data, scaling: Scaling::raw(bitpix) };
    let (data, ny, nx, note) = reduce_to_2d(&mut arr, 200)?;
    if !note.is_empty() && !notes.contains(&note) {
        notes.push(note);
    }
    Ok(RasterImage { data, ny, nx, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_maps() {
        // 3×2 の画像 [[0,1,2],[3,4,5]] を Pillow の変換にかけた結果
        let v: Vec<u8> = (0..6).collect();
        let t = |o| transpose(&v, 3, 2, 1, o);
        assert_eq!(t(2), (vec![2, 1, 0, 5, 4, 3], 3, 2));
        assert_eq!(t(3), (vec![5, 4, 3, 2, 1, 0], 3, 2));
        assert_eq!(t(4), (vec![3, 4, 5, 0, 1, 2], 3, 2));
        assert_eq!(t(5), (vec![0, 3, 1, 4, 2, 5], 2, 3));
        // ROTATE_270(時計回り): 左下 3 が左上へ
        assert_eq!(t(6), (vec![3, 0, 4, 1, 5, 2], 2, 3));
        assert_eq!(t(7), (vec![5, 2, 4, 1, 3, 0], 2, 3));
        // ROTATE_90(反時計回り): 右上 2 が左上へ
        assert_eq!(t(8), (vec![2, 5, 1, 4, 0, 3], 2, 3));
    }

    /// 以前の直列の並べ替え
    fn old_transpose<T: Copy>(v: &[T], w: usize, h: usize, ch: usize, orient: u8) -> (Vec<T>, usize, usize) {
        let (nw, nh) = if matches!(orient, 5..=8) { (h, w) } else { (w, h) };
        let src = |x: usize, y: usize| -> (usize, usize) {
            match orient {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                8 => (w - 1 - y, x),
                _ => (x, y),
            }
        };
        let mut out = Vec::with_capacity(v.len());
        for y in 0..nh {
            for x in 0..nw {
                let (sx, sy) = src(x, y);
                let s = (sy * w + sx) * ch;
                out.extend_from_slice(&v[s..s + ch]);
            }
        }
        (out, nw, nh)
    }

    #[test]
    fn parallel_transpose_same_as_serial() {
        for &(w, h) in &[(1usize, 1usize), (1, 7), (9, 1), (13, 8), (64, 33)] {
            for &ch in &[1usize, 3] {
                let v: Vec<u8> = (0..w * h * ch).map(|i| (i * 37 % 251) as u8).collect();
                let f: Vec<f32> = (0..w * h * ch).map(|i| i as f32 * 0.5 - 3.0).collect();
                for o in 1..=8u8 {
                    let (a, aw, ah) = transpose(&v, w, h, ch, o);
                    assert_eq!((a, aw, ah), old_transpose(&v, w, h, ch, o), "u8 {w}x{h}x{ch} 向き {o}");
                    let (a, _, _) = transpose(&f, w, h, ch, o);
                    let (b, _, _) = old_transpose(&f, w, h, ch, o);
                    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()) && a.len() == b.len());
                }
            }
        }
        // 空の画像でも panic しない
        assert_eq!(transpose::<u8>(&[], 0, 5, 3, 6), (vec![], 5, 0));
        assert_eq!(transpose::<u8>(&[], 4, 0, 1, 2), (vec![], 4, 0));
    }

    #[test]
    fn rgba_to_rgb_same_as_serial() {
        for n in [0usize, 1, 4, 7, 4 * 1000 + 3] {
            let v: Vec<u8> = (0..n).map(|i| (i * 91 % 256) as u8).collect();
            let old: Vec<u8> = v.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            assert_eq!(rgba8_to_rgb(&v), old, "{n}");
        }
    }
}

//! 読み込んだ多次元配列を「Python 版で numpy が持つ型の値」として取り出す部品。
//!
//! FITS の整数は BZERO/BSCALE/BLANK によって astropy が型を変える(uint16 に読み替える・float32 に
//! 直すなど)。取り出す値はその型に丸めた値にする(float32 になる場合は float32 で掛け算・足し算をする)。

use rayon::prelude::*;

/// astropy がデータに付ける型(BZERO/BSCALE/BLANK の扱い)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScaleKind {
    /// そのまま(整数・実数)
    Raw,
    /// BZERO = 2^(bits-1)・BSCALE = 1 の符号なし整数(uint16/32/64)
    PseudoUnsigned,
    /// BITPIX 8 + BZERO = -128 の符号付き 8 bit
    Int8,
    /// 8/16 bit の整数を float32 にして BSCALE・BZERO(float32 で計算)、BLANK は NaN
    ToF32,
    /// 32/64 bit の整数を float64 にして BSCALE・BZERO、BLANK は NaN
    ToF64,
    /// float32 のデータに float32 で BSCALE・BZERO
    Float32Scaled,
    /// float64 のデータに BSCALE・BZERO
    Float64Scaled,
}

#[derive(Clone, Debug)]
pub(crate) struct Scaling {
    pub kind: ScaleKind,
    pub bitpix: i32,
    pub bscale: f64,
    pub bzero: f64,
    pub blank: Option<i64>,
    /// Python の note に出る dtype の名前(">i2"・"uint16"・"float32" など)
    pub dtype: String,
}

impl Scaling {
    /// astropy の _ImageBaseHDU の規則で型を決める。`native` は圧縮 HDU(展開後はその機械のバイト順)。
    pub fn new(bitpix: i32, bscale: f64, bzero: f64, blank: Option<i64>, native: bool) -> Scaling {
        let blank = if bitpix > 0 { blank } else { None };
        let raw_name = |b: i32| -> &'static str {
            match (b, native) {
                (8, _) => "uint8",
                (16, false) => ">i2",
                (32, false) => ">i4",
                (64, false) => ">i8",
                (-32, false) => ">f4",
                (-64, false) => ">f8",
                (16, true) => "int16",
                (32, true) => "int32",
                (64, true) => "int64",
                (-32, true) => "float32",
                _ => "float64",
            }
        };
        let mk =
            |kind: ScaleKind, dtype: &str| Scaling { kind, bitpix, bscale, bzero, blank, dtype: dtype.to_string() };
        if bzero == 0.0 && bscale == 1.0 && blank.is_none() {
            return mk(ScaleKind::Raw, raw_name(bitpix));
        }
        if bscale == 1.0 && bzero != 0.0 && bitpix > 0 {
            match bitpix {
                16 if bzero == 32768.0 => return mk(ScaleKind::PseudoUnsigned, "uint16"),
                32 if bzero == 2147483648.0 => return mk(ScaleKind::PseudoUnsigned, "uint32"),
                64 if bzero == 9223372036854775808.0 => return mk(ScaleKind::PseudoUnsigned, "uint64"),
                8 if bzero == -128.0 => return mk(ScaleKind::Int8, "int8"),
                _ => {}
            }
        }
        match bitpix {
            8 | 16 => mk(ScaleKind::ToF32, "float32"),
            32 | 64 => mk(ScaleKind::ToF64, "float64"),
            -32 => mk(ScaleKind::Float32Scaled, if native { "float32" } else { ">f4" }),
            _ => mk(ScaleKind::Float64Scaled, if native { "float64" } else { ">f8" }),
        }
    }

    /// そのままの型(画像ファイルの配列など)。
    pub fn raw(bitpix: i32) -> Scaling {
        Scaling::new(bitpix, 1.0, 0.0, None, true)
    }

    /// 整数の生の値 → Python の型の値(float32 で計算する型は float32 に丸めたものを f64 で)。
    #[inline]
    pub fn int_f64(&self, raw: i64) -> f64 {
        match self.kind {
            ScaleKind::Raw => raw as f64,
            ScaleKind::PseudoUnsigned => self.unsigned(raw) as f64,
            ScaleKind::Int8 => (raw - 128) as f64,
            ScaleKind::ToF32 => self.int_f32(raw) as f64,
            ScaleKind::ToF64 | ScaleKind::Float32Scaled | ScaleKind::Float64Scaled => {
                if self.blank == Some(raw) {
                    return f64::NAN;
                }
                let mut v = raw as f64;
                if self.bscale != 1.0 {
                    v *= self.bscale;
                }
                if self.bzero != 0.0 {
                    v += self.bzero;
                }
                v
            }
        }
    }

    #[inline]
    fn unsigned(&self, raw: i64) -> u64 {
        match self.bitpix {
            16 => (raw + 32768) as u64,
            32 => (raw + 2147483648) as u64,
            _ => (raw as u64) ^ (1u64 << 63),
        }
    }

    /// 整数の生の値 → float32(2 次元の画像にするときの np.array(data, dtype=float32))。
    #[inline]
    pub fn int_f32(&self, raw: i64) -> f32 {
        match self.kind {
            ScaleKind::Raw => raw as f32,
            ScaleKind::PseudoUnsigned => self.unsigned(raw) as f32,
            ScaleKind::Int8 => (raw - 128) as f32,
            ScaleKind::ToF32 => {
                if self.blank == Some(raw) {
                    return f32::NAN;
                }
                let mut v = raw as f32;
                if self.bscale != 1.0 {
                    v *= self.bscale as f32;
                }
                if self.bzero != 0.0 {
                    v += self.bzero as f32;
                }
                v
            }
            _ => self.int_f64(raw) as f32,
        }
    }

    /// 実数の生の値(float32 なら float32 の値)→ Python の型の値。
    #[inline]
    pub fn float_f64(&self, raw: f64) -> f64 {
        match self.kind {
            ScaleKind::Float32Scaled => self.float_f32(raw) as f64,
            ScaleKind::Float64Scaled => {
                let mut v = raw;
                if self.bscale != 1.0 {
                    v *= self.bscale;
                }
                if self.bzero != 0.0 {
                    v += self.bzero;
                }
                v
            }
            _ => raw,
        }
    }

    #[inline]
    pub fn float_f32(&self, raw: f64) -> f32 {
        match self.kind {
            ScaleKind::Float32Scaled => {
                let mut v = raw as f32;
                if self.bscale != 1.0 {
                    v *= self.bscale as f32;
                }
                if self.bzero != 0.0 {
                    v += self.bzero as f32;
                }
                v
            }
            _ => self.float_f64(raw) as f32,
        }
    }
}

/// メモリ上の型付きの配列(タイル圧縮の展開結果・画像ファイルの画素)。
pub(crate) enum Typed {
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl Typed {
    pub fn len(&self) -> usize {
        match self {
            Typed::U8(v) => v.len(),
            Typed::I16(v) => v.len(),
            Typed::U16(v) => v.len(),
            Typed::I32(v) => v.len(),
            Typed::I64(v) => v.len(),
            Typed::F32(v) => v.len(),
            Typed::F64(v) => v.len(),
        }
    }

    pub fn fill_f32(&self, sc: &Scaling, start: usize, out: &mut [f32]) {
        let n = out.len();
        macro_rules! ints {
            ($v:expr) => {
                out.par_iter_mut().zip($v[start..start + n].par_iter()).for_each(|(o, &x)| *o = sc.int_f32(x as i64))
            };
        }
        match self {
            Typed::U8(v) => ints!(v),
            Typed::I16(v) => ints!(v),
            Typed::U16(v) => ints!(v),
            Typed::I32(v) => ints!(v),
            Typed::I64(v) => ints!(v),
            Typed::F32(v) => {
                out.par_iter_mut().zip(v[start..start + n].par_iter()).for_each(|(o, &x)| *o = sc.float_f32(x as f64))
            }
            Typed::F64(v) => {
                out.par_iter_mut().zip(v[start..start + n].par_iter()).for_each(|(o, &x)| *o = sc.float_f32(x))
            }
        }
    }

    pub fn fill_f64(&self, sc: &Scaling, start: usize, out: &mut [f64]) {
        let n = out.len();
        macro_rules! ints {
            ($v:expr) => {
                out.par_iter_mut().zip($v[start..start + n].par_iter()).for_each(|(o, &x)| *o = sc.int_f64(x as i64))
            };
        }
        match self {
            Typed::U8(v) => ints!(v),
            Typed::I16(v) => ints!(v),
            Typed::U16(v) => ints!(v),
            Typed::I32(v) => ints!(v),
            Typed::I64(v) => ints!(v),
            Typed::F32(v) => {
                out.par_iter_mut().zip(v[start..start + n].par_iter()).for_each(|(o, &x)| *o = sc.float_f64(x as f64))
            }
            Typed::F64(v) => {
                out.par_iter_mut().zip(v[start..start + n].par_iter()).for_each(|(o, &x)| *o = sc.float_f64(x))
            }
        }
    }
}

/// 多次元配列(numpy の形の順、最後の軸が最も速い)から値を取り出す口。
pub(crate) trait ArraySource {
    fn shape(&self) -> Vec<usize>;
    /// 線形の添字 start から out.len() 個を float32 で。
    fn read_f32(&mut self, start: usize, out: &mut [f32]) -> Result<(), String>;
    /// 線形の添字 start から out.len() 個を、Python の型の値のまま float64 で(キューブの平均用)。
    fn read_f64(&mut self, start: usize, out: &mut [f64]) -> Result<(), String>;
}

/// メモリ上の配列 + 型の規則。
pub(crate) struct MemArray {
    pub shape: Vec<usize>,
    pub data: Typed,
    pub scaling: Scaling,
}

impl ArraySource for MemArray {
    fn shape(&self) -> Vec<usize> {
        self.shape.clone()
    }
    fn read_f32(&mut self, start: usize, out: &mut [f32]) -> Result<(), String> {
        if start + out.len() > self.data.len() {
            return Err("配列の範囲外を読もうとしました".into());
        }
        self.data.fill_f32(&self.scaling, start, out);
        Ok(())
    }
    fn read_f64(&mut self, start: usize, out: &mut [f64]) -> Result<(), String> {
        if start + out.len() > self.data.len() {
            return Err("配列の範囲外を読もうとしました".into());
        }
        self.data.fill_f64(&self.scaling, start, out);
        Ok(())
    }
}

/// Python のタプルの書き方: (600, 900)・(5,)
pub(crate) fn py_tuple(v: &[usize]) -> String {
    match v.len() {
        0 => "()".to_string(),
        1 => format!("({},)", v[0]),
        _ => format!("({})", v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")),
    }
}

/// 輝度の重み: `_COLOR_WEIGHTS[:3] / _COLOR_WEIGHTS[:3].sum()`(float32)。
pub(crate) fn lum_weights() -> [f32; 3] {
    let c = [0.2126f32, 0.7152f32, 0.0722f32];
    let s = ((0.0f32 + c[0]) + c[1]) + c[2];
    [c[0] / s, c[1] / s, c[2] / s]
}

/// 長さ n の、`T::default()`(数なら 0)で埋めた配列。確保できなければ Err(abort しない)。
///
/// `resize` で埋めると 1 本のスレッドで全部のページに初めて書く(24 MP の f32 で 96 MB・ページの割り当て約 2.4 万回)。
/// ここでは予約した領域へ rayon で並列に書く(rayon の Vec への `par_extend` は、長さの分かる反復なら予約済みの
/// 領域へ添字の順に直接書く。予約が足りているので中で確保し直さない)。中身は `resize` と同じ値。
pub(crate) fn alloc_filled<T: Clone + Default + Send>(n: usize) -> Result<Vec<T>, String> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| format!("画像が大きすぎてメモリに載りません ({n} 画素)"))?;
    v.par_extend(rayon::iter::repeat_n(T::default(), n));
    Ok(v)
}

/// 画素ごとに色が並ぶ配列(numpy の形 (高さ, 幅, ch)、ch は 3 か 4)を輝度にする。n は画素の数。
///
/// 以前は全体を float32 の 3〜4 面ぶん(24 MP で 288 MB)に読んでから合成していた。ここでは blk 画素ずつ
/// 使い回しの作業領域に読み、合成した輝度だけを出力に積む。作業領域の値は以前の全体の配列の同じ添字の値と同じ
/// (同じ `read_f32` を同じ添字で呼ぶ)で、輝度の式も括弧まで同じなので、結果はビット単位で同じ。
/// 区切りは必ず画素の境目(添字は ch の倍数)で、最後の端数の区切りも読む。`par_extend` は添字の順に書く。
pub(crate) fn luminance_interleaved(
    src: &mut dyn ArraySource,
    n: usize,
    ch: usize,
    w: [f32; 3],
    blk: usize,
) -> Result<Vec<f32>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| format!("画像が大きすぎてメモリに載りません ({n} 画素)"))?;
    let blk = blk.max(1);
    let mut tmp = vec![0.0f32; blk.min(n) * ch];
    let mut start = 0;
    while start < n {
        let len = blk.min(n - start);
        let t = &mut tmp[..len * ch];
        src.read_f32(start * ch, t)?;
        out.par_extend(t.par_chunks_exact(ch).map(|p| (w[0] * p[0] + w[1] * p[1]) + w[2] * p[2]));
        start += len;
    }
    Ok(out)
}

/// `luminance_interleaved` の 1 区切りの画素数(作業領域は 3 色で 6 MB。rayon の呼び出しは区切りごとに 2 回なので
/// 小さすぎると呼び出しの重さが目立つ)
pub(crate) const LUM_BLOCK: usize = 1 << 19;

/// `_reduce_to_2d(data, max_frames=200)`: 3 次元以上を 2 次元へ。戻り値 (画素, 高さ, 幅, 説明)。
pub(crate) fn reduce_to_2d(
    src: &mut dyn ArraySource,
    max_frames: usize,
) -> Result<(Vec<f32>, usize, usize, String), String> {
    let mut shape = src.shape();
    // 先頭側(最後の 2 軸以外)の大きさ 1 の軸を、3 次元になるまで畳む
    while shape.len() > 3 {
        let lead = shape.len() - 2;
        match shape[..lead].iter().position(|&s| s == 1) {
            Some(i) => {
                shape.remove(i);
            }
            None => break,
        }
    }
    // まだ 3 次元より多ければ先頭の要素(全部 0 番)の 3 次元ブロック(ファイルの先頭から)
    if shape.len() > 3 {
        shape = shape[shape.len() - 3..].to_vec();
    }
    let total: usize = shape.iter().product();
    let alloc = alloc_filled::<f32>;
    match shape.len() {
        2 => {
            let (ny, nx) = (shape[0], shape[1]);
            let mut out = alloc(total)?;
            src.read_f32(0, &mut out)?;
            Ok((out, ny, nx, String::new()))
        }
        3 => {
            let (nz, ny, nx) = (shape[0], shape[1], shape[2]);
            let w = lum_weights();
            if (nz == 3 || nz == 4) && ny > 8 && nx > 8 {
                let plane = ny * nx;
                let mut p0 = alloc(plane)?;
                let mut p1 = alloc(plane)?;
                let mut p2 = alloc(plane)?;
                src.read_f32(0, &mut p0)?;
                src.read_f32(plane, &mut p1)?;
                src.read_f32(2 * plane, &mut p2)?;
                p0.par_iter_mut().zip(p1.par_iter().zip(p2.par_iter())).for_each(|(a, (&b, &c))| {
                    *a = (w[0] * *a + w[1] * b) + w[2] * c;
                });
                return Ok((p0, ny, nx, "カラー 3 面を輝度に合成".to_string()));
            }
            if (nx == 3 || nx == 4) && ny > 8 && nz > 8 {
                // 画素ごとに色が並ぶ (高さ, 幅, 3|4)(カラーの PNG・JPEG・TIFF はここ)
                let out = luminance_interleaved(src, nz * ny, nx, w, LUM_BLOCK)?;
                return Ok((out, nz, ny, "カラー 3 面を輝度に合成".to_string()));
            }
            let used = nz.min(max_frames);
            let plane = ny * nx;
            let mut acc = vec![0.0f64; plane];
            let mut buf = vec![0.0f64; plane];
            for i in 0..used {
                src.read_f64(i * plane, &mut buf)?;
                acc.par_iter_mut().zip(buf.par_iter()).for_each(|(a, &b)| *a += b);
            }
            let d = used as f64;
            let out: Vec<f32> = acc.par_iter().map(|&a| (a / d) as f32).collect();
            Ok((out, ny, nx, format!("{nz} 枚のキューブのうち先頭 {used} 枚を平均 (SNR 稼ぎ)")))
        }
        _ => Err(format!("2 次元画像に変換できない形状です: {}", py_tuple(&shape))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 以前の (高さ, 幅, ch) の合成: 全体を float32 に読んでから輝度に
    fn old_luminance(src: &mut dyn ArraySource, n: usize, ch: usize, w: [f32; 3]) -> Vec<f32> {
        let mut all = alloc_filled::<f32>(n * ch).unwrap();
        src.read_f32(0, &mut all).unwrap();
        let mut out = alloc_filled::<f32>(n).unwrap();
        out.par_iter_mut().enumerate().for_each(|(i, o)| {
            let p = &all[i * ch..i * ch + 3];
            *o = (w[0] * p[0] + w[1] * p[1]) + w[2] * p[2];
        });
        out
    }

    /// 決まった擬似乱数(xorshift64*)
    fn rand_u64(n: usize, seed: u64) -> Vec<u64> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                x.wrapping_mul(0x2545F4914F6CDD1D)
            })
            .collect()
    }

    #[test]
    fn luminance_blocks_same_as_whole() {
        let w = lum_weights();
        for &ch in &[3usize, 4] {
            for &(h, wd) in &[(9usize, 9usize), (23, 37), (64, 50)] {
                let n = h * wd;
                let r = rand_u64(n * ch, (h * 1000 + wd * 10 + ch) as u64);
                // u8(画像ファイル)と float32(FITS の (H,W,3)。NaN・±inf・-0・非正規数も混ざる)と BSCALE つきの uint16
                let arrays: Vec<(Typed, Scaling)> = vec![
                    (Typed::U8(r.iter().map(|&x| (x >> 56) as u8).collect()), Scaling::raw(8)),
                    (Typed::F32(r.iter().map(|&x| f32::from_bits((x >> 32) as u32)).collect()), Scaling::raw(-32)),
                    (
                        Typed::U16(r.iter().map(|&x| (x >> 48) as u16).collect()),
                        Scaling::new(16, 0.75, 3.0, None, true),
                    ),
                ];
                for (data, scaling) in arrays {
                    let mut a = MemArray { shape: vec![h, wd, ch], data, scaling };
                    let old = old_luminance(&mut a, n, ch, w);
                    // 区切り: 1 画素・端数の出る大きさ・ちょうど割り切れる大きさ・全体より大きい
                    for &blk in &[1usize, 7, n / 3, n, n + 5, LUM_BLOCK] {
                        let new = luminance_interleaved(&mut a, n, ch, w, blk).unwrap();
                        assert_eq!(new.len(), n);
                        for i in 0..n {
                            assert_eq!(new[i].to_bits(), old[i].to_bits(), "ch {ch} {h}x{wd} 区切り {blk} 添字 {i}");
                        }
                    }
                    // reduce_to_2d を通しても同じ(形と説明も)
                    let (d, ny, nx, note) = reduce_to_2d(&mut a, 200).unwrap();
                    assert_eq!((ny, nx, note.as_str()), (h, wd, "カラー 3 面を輝度に合成"));
                    assert!(d.iter().zip(&old).all(|(x, y)| x.to_bits() == y.to_bits()));
                }
            }
        }
    }

    #[test]
    fn luminance_out_of_range_is_err() {
        // 形より短い配列(壊れた入力)は以前と同じく Err で、panic しない
        let mut a = MemArray { shape: vec![10, 10, 3], data: Typed::U8(vec![1; 250]), scaling: Scaling::raw(8) };
        assert_eq!(
            luminance_interleaved(&mut a, 100, 3, lum_weights(), 7).unwrap_err(),
            "配列の範囲外を読もうとしました"
        );
        assert!(reduce_to_2d(&mut a, 200).is_err());
    }
}

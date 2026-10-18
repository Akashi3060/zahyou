//! JPEG を Pillow(libjpeg-turbo)と同じ値で復号する。
//!
//! image クレート(zune-jpeg)の復号は IDCT・色差の補間・色変換の丸めが libjpeg-turbo と違い、画素が ±1 ずれる
//! (星の位置が最大 7e-3 px、σ が相対 2e-4 ずれた)。ここでは libjpeg-turbo の既定の復号を写す:
//! - 整数の逆 DCT `jpeg_idct_islow`(jidctint.c。CONST_BITS 13・PASS1_BITS 2、範囲の制限表も同じ)
//! - 色差の補間は既定の "fancy upsampling"(jdsample.c の h2v1・h1v2・h2v2 の三角フィルター。端は複製)
//! - YCbCr → RGB は jdcolor.c の 16 bit 固定小数点の表
//!
//! 扱うのは 8 bit のハフマン符号(ベースライン・拡張・プログレッシブ)で 1 成分(グレー)・3 成分(YCbCr・RGB)・
//! 4 成分(CMYK・YCCK)のもの。それ以外(算術符号・12 bit・ロスレス・プログレッシブで係数が最後まで来ないもの)は Err を返す。
//! 呼ぶ側(raster.rs)はそのとき image クレートで読み直す。データが途中で切れていたら Err(Pillow も Err)。

use rayon::prelude::*;

/// ジグザグの順 → 自然な順(はみ出した位置は 63。libjpeg の jpeg_natural_order と同じ)。
const NATURAL: [usize; 80] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54,
    47, 55, 62, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];

/// データが途中で切れているときのエラー文の頭(Pillow も Err にする。ほかのエラーは image クレートで読み直してよい)
pub(crate) const TRUNCATED: &str = "JPEG のデータが途中で切れています";

/// 復号した画像。`channels` は 1(グレー)・3(R G B)・4(libjpeg の出力の C M Y K。Pillow はこれを反転して使う)。
pub(crate) struct Jpeg {
    pub w: usize,
    pub h: usize,
    pub channels: usize,
    pub pix: Vec<u8>,
}

// ------------------------------------------------------------ ハフマン ---

#[derive(Clone)]
struct Huff {
    /// 先頭 8 bit で引く表: (符号の長さ, 値)。長さ 0 は 9 bit 以上
    fast: Vec<(u8, u8)>,
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    vals: Vec<u8>,
}

impl Huff {
    fn new(counts: &[u8; 16], vals: Vec<u8>) -> Result<Huff, String> {
        let total: usize = counts.iter().map(|&c| c as usize).sum();
        if total > 256 || total > vals.len() {
            return Err("JPEG のハフマン表が不正です".into());
        }
        let mut maxcode = [-1i32; 18];
        let mut valptr = [0i32; 17];
        let mut mincode = [0i32; 17];
        let mut fast = vec![(0u8, 0u8); 256];
        let mut code: i32 = 0;
        let mut k: i32 = 0;
        for l in 1..=16usize {
            let n = counts[l - 1] as i32;
            if n > 0 {
                valptr[l] = k;
                mincode[l] = code;
                for _ in 0..n {
                    if l <= 8 {
                        // この符号で始まる 8 bit の値すべて
                        let shift = 8 - l;
                        let base = (code << shift) as usize;
                        for t in 0..(1usize << shift) {
                            if let Some(e) = fast.get_mut(base + t) {
                                *e = (l as u8, vals[k as usize]);
                            }
                        }
                    }
                    code += 1;
                    k += 1;
                }
                maxcode[l] = code - 1;
            }
            if code > (1 << l) {
                return Err("JPEG のハフマン表が不正です".into());
            }
            code <<= 1;
        }
        maxcode[17] = i32::MAX;
        Ok(Huff { fast, maxcode, valptr, mincode, vals: vals[..total].to_vec() })
    }
}

// ------------------------------------------------------------ ビット ---

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    /// マーカーに当たった(以後は 0 を補う。libjpeg と同じ)
    marker: bool,
    /// ファイルの終わりに当たった(途中で切れている)
    eof: bool,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8], pos: usize) -> Bits<'a> {
        Bits { d, pos, acc: 0, n: 0, marker: false, eof: false }
    }

    fn fill(&mut self) {
        while self.n <= 56 {
            let mut byte = 0u8;
            if !self.marker {
                match self.d.get(self.pos) {
                    None => {
                        self.eof = true;
                        self.marker = true;
                    }
                    Some(&0xFF) => match self.d.get(self.pos + 1) {
                        Some(0) => {
                            byte = 0xFF;
                            self.pos += 2;
                        }
                        None => {
                            self.eof = true;
                            self.marker = true;
                        }
                        Some(_) => self.marker = true,
                    },
                    Some(&b) => {
                        byte = b;
                        self.pos += 1;
                    }
                }
            }
            self.acc |= (byte as u64) << (56 - self.n);
            self.n += 8;
        }
    }

    #[inline]
    fn bits(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        if self.n < k {
            self.fill();
        }
        let v = (self.acc >> (64 - k)) as u32;
        self.acc <<= k;
        self.n -= k;
        v
    }

    #[inline]
    fn bit(&mut self) -> u32 {
        self.bits(1)
    }

    #[inline]
    fn decode(&mut self, h: &Huff) -> Result<u8, String> {
        if self.n < 16 {
            self.fill();
        }
        let peek = (self.acc >> 56) as usize;
        let (l, v) = h.fast[peek];
        if l > 0 {
            self.acc <<= l;
            self.n -= l as u32;
            return Ok(v);
        }
        // 9 bit 以上の符号
        let mut code = self.bits(8) as i32;
        let mut l = 8usize;
        loop {
            code = (code << 1) | self.bit() as i32;
            l += 1;
            if l > 16 {
                return Err("JPEG のハフマン符号が壊れています".into());
            }
            if code <= h.maxcode[l] {
                let idx = h.valptr[l] + code - h.mincode[l];
                return h
                    .vals
                    .get(idx as usize)
                    .copied()
                    .ok_or_else(|| "JPEG のハフマン符号が壊れています".to_string());
            }
        }
    }

    /// 再同期の区切り(RSTn)の前: 余りのビットを捨ててマーカーを読む。
    fn restart(&mut self) -> Result<(), String> {
        self.acc = 0;
        self.n = 0;
        self.marker = false;
        // マーカーの前の詰め物(0xFF の連続)を飛ばす
        while self.d.get(self.pos) == Some(&0xFF) && self.d.get(self.pos + 1) == Some(&0xFF) {
            self.pos += 1;
        }
        match (self.d.get(self.pos), self.d.get(self.pos + 1)) {
            (Some(0xFF), Some(m)) if (0xD0..=0xD7).contains(m) => {
                self.pos += 2;
                Ok(())
            }
            (None, _) | (Some(0xFF), None) => {
                self.eof = true;
                Err(TRUNCATED.into())
            }
            _ => Err("JPEG の RST マーカーが見つかりません".into()),
        }
    }
}

#[inline]
fn extend(v: u32, s: u32) -> i32 {
    if s == 0 {
        return 0;
    }
    let v = v as i32;
    if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
}

// ------------------------------------------------------------ 成分 ---

struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    /// 係数の格子(交互配置の MCU の大きさに切り上げたもの)
    bw: usize,
    bh: usize,
    coefs: Vec<i16>,
    /// 最初の走査のときの量子化表(libjpeg と同じくそこで固定する)
    quant: Option<[u16; 64]>,
    /// 係数ごとの残りの精度(-1 = まだ来ていない、0 = 確定)。プログレッシブの平滑化の判定に使う
    coef_bits: [i32; 64],
    /// 縮小後の大きさ(端の複製の基準)
    dw: usize,
    dh: usize,
}

struct Frame {
    w: usize,
    h: usize,
    progressive: bool,
    comps: Vec<Comp>,
    hmax: usize,
    vmax: usize,
    mcux: usize,
    mcuy: usize,
}

fn be16(b: &[u8], i: usize) -> Result<usize, String> {
    match (b.get(i), b.get(i + 1)) {
        (Some(&x), Some(&y)) => Ok(((x as usize) << 8) | y as usize),
        _ => Err(TRUNCATED.into()),
    }
}

/// 走査 1 つを復号する(ベースラインかプログレッシブ)。戻り値は走査の後ろの位置。
#[allow(clippy::too_many_arguments)]
fn decode_scan(
    d: &[u8],
    pos: usize,
    fr: &mut Frame,
    sc: &[(usize, usize, usize)], // (成分の番号, DC 表, AC 表)
    ss: usize,
    se: usize,
    ah: u32,
    al: u32,
    dc: &[Option<Huff>; 4],
    ac: &[Option<Huff>; 4],
    restart: usize,
) -> Result<usize, String> {
    let mut b = Bits::new(d, pos);
    let mut pred = vec![0i32; sc.len()];
    let mut eobrun: u32 = 0;
    let table = |t: &[Option<Huff>; 4], i: usize| -> Result<Huff, String> {
        t.get(i).and_then(|x| x.clone()).ok_or_else(|| "JPEG のハフマン表がありません".to_string())
    };
    let need_dc = ss == 0 && ah == 0;
    let need_ac = ss > 0;
    let dcs: Vec<Option<Huff>> = sc.iter().map(|&(_, td, _)| if need_dc { table(dc, td).ok() } else { None }).collect();
    let acs: Vec<Option<Huff>> =
        sc.iter().map(|&(_, _, ta)| if need_ac || !fr.progressive { table(ac, ta).ok() } else { None }).collect();
    if !fr.progressive {
        for (k, &(_, td, ta)) in sc.iter().enumerate() {
            if dcs[k].is_none() || acs[k].is_none() {
                return Err(format!("JPEG のハフマン表 {td}/{ta} がありません"));
            }
        }
    } else if (need_dc && dcs.iter().any(|x| x.is_none())) || (need_ac && acs.iter().any(|x| x.is_none())) {
        return Err("JPEG のハフマン表がありません".into());
    }
    // 走査に入る MCU の並び
    let interleaved = sc.len() > 1;
    let (nx, ny) = if interleaved {
        (fr.mcux, fr.mcuy)
    } else {
        let c = &fr.comps[sc[0].0];
        (c.dw.div_ceil(8), c.dh.div_ceil(8))
    };
    let total = nx.checked_mul(ny).ok_or("JPEG が大きすぎます")?;
    let mut done = 0usize;
    for my in 0..ny {
        for mx in 0..nx {
            if restart > 0 && done > 0 && done.is_multiple_of(restart) {
                b.restart()?;
                pred.iter_mut().for_each(|p| *p = 0);
                eobrun = 0;
            }
            done += 1;
            for (k, &(ci, _, _)) in sc.iter().enumerate() {
                let comp = &mut fr.comps[ci];
                let (bh_, bv_) = if interleaved { (comp.h, comp.v) } else { (1, 1) };
                for v in 0..bv_ {
                    for h in 0..bh_ {
                        let (bx, by) = if interleaved { (mx * comp.h + h, my * comp.v + v) } else { (mx, my) };
                        let off = (by * comp.bw + bx) * 64;
                        let blk = comp.coefs.get_mut(off..off + 64).ok_or("JPEG の係数の位置が範囲外です")?;
                        if !fr.progressive {
                            // ベースライン: DC の差分と AC のランレングス
                            let dch = dcs[k].as_ref().ok_or("表がありません")?;
                            let t = b.decode(dch)? as u32;
                            if t > 16 {
                                return Err("JPEG の DC の大きさが不正です".into());
                            }
                            let diff = extend(b.bits(t), t);
                            pred[k] = pred[k].wrapping_add(diff);
                            blk[0] = pred[k] as i16;
                            let ach = acs[k].as_ref().ok_or("表がありません")?;
                            let mut z = 1usize;
                            while z < 64 {
                                let rs = b.decode(ach)?;
                                let r = (rs >> 4) as usize;
                                let s = (rs & 15) as u32;
                                if s != 0 {
                                    z += r;
                                    let v = extend(b.bits(s), s);
                                    blk[NATURAL[z.min(79)]] = v as i16;
                                } else if r == 15 {
                                    z += 15;
                                } else {
                                    break;
                                }
                                z += 1;
                            }
                        } else if ss == 0 {
                            if ah == 0 {
                                let dch = dcs[k].as_ref().ok_or("表がありません")?;
                                let t = b.decode(dch)? as u32;
                                if t > 16 {
                                    return Err("JPEG の DC の大きさが不正です".into());
                                }
                                let diff = extend(b.bits(t), t);
                                pred[k] = pred[k].wrapping_add(diff);
                                blk[0] = (pred[k].wrapping_shl(al)) as i16;
                            } else if b.bit() != 0 {
                                blk[0] |= (1i32 << al) as i16;
                            }
                        } else if ah == 0 {
                            // AC の最初の走査
                            if eobrun > 0 {
                                eobrun -= 1;
                                continue;
                            }
                            let ach = acs[k].as_ref().ok_or("表がありません")?;
                            let mut z = ss;
                            while z <= se {
                                let rs = b.decode(ach)?;
                                let r = (rs >> 4) as u32;
                                let s = (rs & 15) as u32;
                                if s != 0 {
                                    z += r as usize;
                                    let v = extend(b.bits(s), s);
                                    blk[NATURAL[z.min(79)]] = (v.wrapping_shl(al)) as i16;
                                } else if r == 15 {
                                    z += 15;
                                } else {
                                    eobrun = 1u32 << r;
                                    if r != 0 {
                                        eobrun += b.bits(r);
                                    }
                                    eobrun -= 1;
                                    break;
                                }
                                z += 1;
                            }
                        } else {
                            // AC の精度を上げる走査(libjpeg の decode_mcu_AC_refine)
                            let p1: i32 = 1 << al;
                            let m1: i32 = -1i32 << al;
                            let mut z = ss;
                            if eobrun == 0 {
                                let ach = acs[k].as_ref().ok_or("表がありません")?;
                                while z <= se {
                                    let rs = b.decode(ach)?;
                                    let mut r = (rs >> 4) as i32;
                                    let mut s = (rs & 15) as i32;
                                    if s != 0 {
                                        s = if b.bit() != 0 { p1 } else { m1 };
                                    } else if r != 15 {
                                        eobrun = 1u32 << r;
                                        if r != 0 {
                                            eobrun += b.bits(r as u32);
                                        }
                                        break;
                                    }
                                    loop {
                                        let pos = NATURAL[z.min(79)];
                                        let c = blk[pos] as i32;
                                        if c != 0 {
                                            if b.bit() != 0 && (c & p1) == 0 {
                                                blk[pos] = if c >= 0 { (c + p1) as i16 } else { (c + m1) as i16 };
                                            }
                                        } else {
                                            r -= 1;
                                            if r < 0 {
                                                break;
                                            }
                                        }
                                        z += 1;
                                        if z > se {
                                            break;
                                        }
                                    }
                                    if s != 0 {
                                        blk[NATURAL[z.min(79)]] = s as i16;
                                    }
                                    z += 1;
                                }
                            }
                            if eobrun > 0 {
                                while z <= se {
                                    let pos = NATURAL[z.min(79)];
                                    let c = blk[pos] as i32;
                                    if c != 0 && b.bit() != 0 && (c & p1) == 0 {
                                        blk[pos] = if c >= 0 { (c + p1) as i16 } else { (c + m1) as i16 };
                                    }
                                    z += 1;
                                }
                                eobrun -= 1;
                            }
                        }
                    }
                }
            }
            if b.eof {
                return Err(TRUNCATED.into());
            }
        }
    }
    let _ = total;
    if b.eof {
        return Err(TRUNCATED.into());
    }
    // 走査の後ろの次のマーカーを探す(RST と詰め物は飛ばす)
    let mut p = b.pos;
    while p + 1 < d.len() {
        if d[p] == 0xFF && d[p + 1] != 0 && d[p + 1] != 0xFF && !(0xD0..=0xD7).contains(&d[p + 1]) {
            return Ok(p);
        }
        p += 1;
    }
    Err(format!("{TRUNCATED} (EOI がありません)"))
}

// ----------------------------------------------------------- 逆 DCT ---

const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i64 = 2446;
const FIX_0_390180644: i64 = 3196;
const FIX_0_541196100: i64 = 4433;
const FIX_0_765366865: i64 = 6270;
const FIX_0_899976223: i64 = 7373;
const FIX_1_175875602: i64 = 9633;
const FIX_1_501321110: i64 = 12299;
const FIX_1_847759065: i64 = 15137;
const FIX_1_961570560: i64 = 16069;
const FIX_2_053119869: i64 = 16819;
const FIX_2_562915447: i64 = 20995;
const FIX_3_072711026: i64 = 25172;

#[inline]
fn descale(x: i64, n: i32) -> i64 {
    (x + (1i64 << (n - 1))) >> n
}

/// libjpeg の IDCT 後の範囲の制限表(x & 1023 で引く)。
#[inline]
fn range_limit(v: i64) -> u8 {
    let x = (v & 1023) as i32;
    if x < 128 {
        (x + 128) as u8
    } else if x < 512 {
        255
    } else if x < 896 {
        0
    } else {
        (x - 896) as u8
    }
}

/// jpeg_idct_islow(jidctint.c)。係数(自然な順)と量子化表 → 8×8 の画素。
fn idct_islow(c: &[i16], q: &[u16; 64], out: &mut [u8], stride: usize) {
    let mut ws = [0i64; 64];
    for col in 0..8 {
        let dq = |r: usize| c[r * 8 + col] as i64 * q[r * 8 + col] as i64;
        if (1..8).all(|r| c[r * 8 + col] == 0) {
            let dcval = dq(0) << PASS1_BITS;
            for r in 0..8 {
                ws[r * 8 + col] = dcval;
            }
            continue;
        }
        // 偶数の部分
        let z2 = dq(2);
        let z3 = dq(6);
        let z1 = (z2 + z3) * FIX_0_541196100;
        let tmp2 = z1 + z3 * (-FIX_1_847759065);
        let tmp3 = z1 + z2 * FIX_0_765366865;
        let z2 = dq(0);
        let z3 = dq(4);
        let tmp0 = (z2 + z3) << CONST_BITS;
        let tmp1 = (z2 - z3) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        // 奇数の部分
        let (mut t0, mut t1, mut t2, mut t3) = (dq(7), dq(5), dq(3), dq(1));
        let z1 = t0 + t3;
        let z2 = t1 + t2;
        let z3 = t0 + t2;
        let z4 = t1 + t3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        t0 *= FIX_0_298631336;
        t1 *= FIX_2_053119869;
        t2 *= FIX_3_072711026;
        t3 *= FIX_1_501321110;
        let z1 = z1 * (-FIX_0_899976223);
        let z2 = z2 * (-FIX_2_562915447);
        let z3 = z3 * (-FIX_1_961570560) + z5;
        let z4 = z4 * (-FIX_0_390180644) + z5;
        t0 += z1 + z3;
        t1 += z2 + z4;
        t2 += z2 + z3;
        t3 += z1 + z4;
        let n = CONST_BITS - PASS1_BITS;
        ws[col] = descale(tmp10 + t3, n) as i32 as i64;
        ws[7 * 8 + col] = descale(tmp10 - t3, n) as i32 as i64;
        ws[8 + col] = descale(tmp11 + t2, n) as i32 as i64;
        ws[6 * 8 + col] = descale(tmp11 - t2, n) as i32 as i64;
        ws[2 * 8 + col] = descale(tmp12 + t1, n) as i32 as i64;
        ws[5 * 8 + col] = descale(tmp12 - t1, n) as i32 as i64;
        ws[3 * 8 + col] = descale(tmp13 + t0, n) as i32 as i64;
        ws[4 * 8 + col] = descale(tmp13 - t0, n) as i32 as i64;
    }
    for row in 0..8 {
        let w = &ws[row * 8..row * 8 + 8];
        let o = &mut out[row * stride..row * stride + 8];
        if w[1..].iter().all(|&x| x == 0) {
            let v = range_limit(descale(w[0], PASS1_BITS + 3));
            o.iter_mut().for_each(|p| *p = v);
            continue;
        }
        let z2 = w[2];
        let z3 = w[6];
        let z1 = (z2 + z3) * FIX_0_541196100;
        let tmp2 = z1 + z3 * (-FIX_1_847759065);
        let tmp3 = z1 + z2 * FIX_0_765366865;
        let tmp0 = (w[0] + w[4]) << CONST_BITS;
        let tmp1 = (w[0] - w[4]) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        let (mut t0, mut t1, mut t2, mut t3) = (w[7], w[5], w[3], w[1]);
        let z1 = t0 + t3;
        let z2 = t1 + t2;
        let z3 = t0 + t2;
        let z4 = t1 + t3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        t0 *= FIX_0_298631336;
        t1 *= FIX_2_053119869;
        t2 *= FIX_3_072711026;
        t3 *= FIX_1_501321110;
        let z1 = z1 * (-FIX_0_899976223);
        let z2 = z2 * (-FIX_2_562915447);
        let z3 = z3 * (-FIX_1_961570560) + z5;
        let z4 = z4 * (-FIX_0_390180644) + z5;
        t0 += z1 + z3;
        t1 += z2 + z4;
        t2 += z2 + z3;
        t3 += z1 + z4;
        let n = CONST_BITS + PASS1_BITS + 3;
        o[0] = range_limit(descale(tmp10 + t3, n));
        o[7] = range_limit(descale(tmp10 - t3, n));
        o[1] = range_limit(descale(tmp11 + t2, n));
        o[6] = range_limit(descale(tmp11 - t2, n));
        o[2] = range_limit(descale(tmp12 + t1, n));
        o[5] = range_limit(descale(tmp12 - t1, n));
        o[3] = range_limit(descale(tmp13 + t0, n));
        o[4] = range_limit(descale(tmp13 - t0, n));
    }
}

// ------------------------------------------------------------ 補間 ---

/// 縮小された成分の面(行 y、列 x。範囲外は端を複製)。
struct Plane {
    data: Vec<u8>,
    stride: usize,
    dw: usize,
    dh: usize,
}

impl Plane {
    #[inline]
    fn at(&self, x: usize, y: usize) -> i32 {
        let y = y.min(self.dh - 1);
        let x = x.min(self.dw - 1);
        self.data[y * self.stride + x] as i32
    }
}

/// 成分を出力の大きさ(w × h)に伸ばす(libjpeg-turbo の jdsample.c と同じ)。
fn upsample(p: &Plane, fh: usize, fv: usize, w: usize, h: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; w * h];
    let dw = p.dw;
    match (fh, fv) {
        (1, 1) => out.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
            for (x, v) in o.iter_mut().enumerate() {
                *v = p.at(x, y) as u8;
            }
        }),
        (2, 1) if dw > 2 => out.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
            // h2v1_fancy_upsample
            for (x, v) in o.iter_mut().enumerate() {
                let c = x / 2;
                let t = p.at(c, y);
                *v = if x % 2 == 0 {
                    if c == 0 { t } else { (t * 3 + p.at(c - 1, y) + 1) >> 2 }
                } else if c + 1 >= dw {
                    t
                } else {
                    (t * 3 + p.at(c + 1, y) + 2) >> 2
                } as u8;
            }
        }),
        (1, 2) => out.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
            // h1v2_fancy_upsample: 近い行 × 3 + 遠い行(上の出力行は上の行と、下は下の行と)
            let r = y / 2;
            let (nr, bias) = if y % 2 == 0 { (r.saturating_sub(1), 1) } else { (r + 1, 2) };
            for (x, v) in o.iter_mut().enumerate() {
                *v = ((p.at(x, r) * 3 + p.at(x, nr) + bias) >> 2) as u8;
            }
        }),
        (2, 2) if dw > 2 => out.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
            // h2v2_fancy_upsample
            let r = y / 2;
            let nr = if y % 2 == 0 { r.saturating_sub(1) } else { r + 1 };
            let colsum = |c: usize| p.at(c, r) * 3 + p.at(c, nr);
            for (x, v) in o.iter_mut().enumerate() {
                let c = x / 2;
                let t = colsum(c);
                *v = if x % 2 == 0 {
                    if c == 0 { (t * 4 + 8) >> 4 } else { (t * 3 + colsum(c - 1) + 8) >> 4 }
                } else if c + 1 >= dw {
                    (t * 4 + 7) >> 4
                } else {
                    (t * 3 + colsum(c + 1) + 7) >> 4
                } as u8;
            }
        }),
        (fh, fv) if fh >= 1 && fv >= 1 => out.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
            // 整数倍の単純な複製(int_upsample・h2v1_upsample・h2v2_upsample)
            for (x, v) in o.iter_mut().enumerate() {
                *v = p.at(x / fh, y / fv) as u8;
            }
        }),
        _ => return Err("JPEG の色差の倍率に対応していません".into()),
    }
    Ok(out)
}

// ------------------------------------------------------------ 本体 ---

/// jdcolor.c の YCbCr → RGB(16 bit 固定小数点の表)。
fn ycc_tables() -> ([i32; 256], [i32; 256], [i64; 256], [i64; 256]) {
    const SCALEBITS: i32 = 16;
    const ONE_HALF: i64 = 1 << (SCALEBITS - 1);
    let fix = |x: f64| (x * (1i64 << SCALEBITS) as f64 + 0.5) as i64;
    let mut cr_r = [0i32; 256];
    let mut cb_b = [0i32; 256];
    let mut cr_g = [0i64; 256];
    let mut cb_g = [0i64; 256];
    for i in 0..256usize {
        let x = i as i64 - 128;
        cr_r[i] = ((fix(1.40200) * x + ONE_HALF) >> SCALEBITS) as i32;
        cb_b[i] = ((fix(1.77200) * x + ONE_HALF) >> SCALEBITS) as i32;
        cr_g[i] = -fix(0.71414) * x;
        cb_g[i] = -fix(0.34414) * x + ONE_HALF;
    }
    (cr_r, cb_b, cr_g, cb_g)
}

pub(crate) fn decode(d: &[u8]) -> Result<Jpeg, String> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return Err("JPEG ではありません".into());
    }
    let mut qt: [Option<[u16; 64]>; 4] = [None; 4];
    let mut dc: [Option<Huff>; 4] = [None, None, None, None];
    let mut ac: [Option<Huff>; 4] = [None, None, None, None];
    let mut frame: Option<Frame> = None;
    let mut restart = 0usize;
    let mut jfif = false;
    let mut adobe: Option<u8> = None;
    let mut pos = 2usize;
    let mut eoi = false;
    while pos + 1 < d.len() {
        if d[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let m = d[pos + 1];
        if m == 0xFF {
            pos += 1;
            continue;
        }
        pos += 2;
        match m {
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 => {
                eoi = true;
                break;
            }
            _ => {}
        }
        let len = be16(d, pos)?;
        if len < 2 || pos + len > d.len() {
            return Err(TRUNCATED.into());
        }
        let seg = &d[pos + 2..pos + len];
        match m {
            0xC0..=0xC2 => {
                if frame.is_some() {
                    return Err("JPEG の SOF が 2 つあります".into());
                }
                if seg.len() < 6 || seg[0] != 8 {
                    return Err("8 bit 以外の JPEG には対応していません".into());
                }
                let h = be16(seg, 1)?;
                let w = be16(seg, 3)?;
                let nf = seg[5] as usize;
                if w == 0 || h == 0 || !(nf == 1 || nf == 3 || nf == 4) || seg.len() < 6 + nf * 3 {
                    return Err("この JPEG の成分の構成には対応していません".into());
                }
                let mut comps = Vec::new();
                for i in 0..nf {
                    let c = &seg[6 + i * 3..9 + i * 3];
                    let (hs, vs) = ((c[1] >> 4) as usize, (c[1] & 15) as usize);
                    if !(1..=4).contains(&hs) || !(1..=4).contains(&vs) || c[2] > 3 {
                        return Err("JPEG の成分の設定が不正です".into());
                    }
                    comps.push(Comp {
                        id: c[0],
                        h: hs,
                        v: vs,
                        tq: c[2] as usize,
                        bw: 0,
                        bh: 0,
                        coefs: Vec::new(),
                        quant: None,
                        coef_bits: [-1; 64],
                        dw: 0,
                        dh: 0,
                    });
                }
                let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
                let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
                let mcux = w.div_ceil(8 * hmax);
                let mcuy = h.div_ceil(8 * vmax);
                for c in comps.iter_mut() {
                    if hmax % c.h != 0 || vmax % c.v != 0 {
                        return Err("JPEG の色差の倍率が整数ではありません".into());
                    }
                    c.bw = mcux * c.h;
                    c.bh = mcuy * c.v;
                    c.dw = (w * c.h).div_ceil(hmax);
                    c.dh = (h * c.v).div_ceil(vmax);
                    let n = c.bw.checked_mul(c.bh).and_then(|x| x.checked_mul(64)).ok_or("JPEG が大きすぎます")?;
                    if n > (1usize << 30) {
                        return Err("JPEG が大きすぎます".into());
                    }
                    c.coefs.try_reserve_exact(n).map_err(|_| "JPEG が大きすぎてメモリに載りません".to_string())?;
                    c.coefs.resize(n, 0);
                }
                frame = Some(Frame { w, h, progressive: m == 0xC2, comps, hmax, vmax, mcux, mcuy });
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err("算術符号・ロスレス・階層型の JPEG には対応していません".into());
            }
            0xC4 => {
                let mut i = 0;
                while i + 17 <= seg.len() {
                    let tc = seg[i] >> 4;
                    let th = (seg[i] & 15) as usize;
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&seg[i + 1..i + 17]);
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let vals = seg.get(i + 17..i + 17 + n).ok_or("JPEG のハフマン表が途中で切れています")?.to_vec();
                    if th > 3 || tc > 1 {
                        return Err("JPEG のハフマン表の番号が不正です".into());
                    }
                    let hf = Huff::new(&counts, vals)?;
                    if tc == 0 {
                        dc[th] = Some(hf);
                    } else {
                        ac[th] = Some(hf);
                    }
                    i += 17 + n;
                }
            }
            0xDB => {
                let mut i = 0;
                while i < seg.len() {
                    let pq = seg[i] >> 4;
                    let tq = (seg[i] & 15) as usize;
                    if tq > 3 {
                        return Err("JPEG の量子化表の番号が不正です".into());
                    }
                    let mut t = [0u16; 64];
                    for k in 0..64 {
                        let v = if pq == 0 {
                            *seg.get(i + 1 + k).ok_or("JPEG の量子化表が途中で切れています")? as u16
                        } else {
                            be16(seg, i + 1 + 2 * k)? as u16
                        };
                        t[NATURAL[k]] = v;
                    }
                    qt[tq] = Some(t);
                    i += 1 + if pq == 0 { 64 } else { 128 };
                }
            }
            0xDD => restart = be16(seg, 0)?,
            0xE0 => {
                if seg.starts_with(b"JFIF\0") {
                    jfif = true;
                }
            }
            0xEE => {
                if seg.len() >= 12 && seg.starts_with(b"Adobe") {
                    adobe = Some(seg[11]);
                }
            }
            0xDA => {
                let fr = frame.as_mut().ok_or("JPEG の SOS が SOF より前にあります")?;
                let ns = *seg.first().ok_or("JPEG の SOS が短すぎます")? as usize;
                if ns == 0 || ns > 4 || seg.len() < 1 + ns * 2 + 3 {
                    return Err("JPEG の SOS が不正です".into());
                }
                let mut sc = Vec::new();
                for k in 0..ns {
                    let cid = seg[1 + k * 2];
                    let t = seg[2 + k * 2];
                    let ci =
                        fr.comps.iter().position(|c| c.id == cid).ok_or("JPEG の SOS の成分が SOF にありません")?;
                    sc.push((ci, (t >> 4) as usize, (t & 15) as usize));
                }
                let ss = seg[1 + ns * 2] as usize;
                let se = seg[2 + ns * 2] as usize;
                let ah = (seg[3 + ns * 2] >> 4) as u32;
                let al = (seg[3 + ns * 2] & 15) as u32;
                if fr.progressive && (ss > se || se > 63 || (ss == 0 && se != 0) || (ss > 0 && ns != 1) || al > 13) {
                    return Err("JPEG のプログレッシブの走査が不正です".into());
                }
                // 量子化表を固定し、係数の精度を記録する
                for &(ci, _, _) in &sc {
                    let c = &mut fr.comps[ci];
                    if c.quant.is_none() {
                        c.quant = Some(qt[c.tq].ok_or("JPEG の量子化表がありません")?);
                    }
                    let (s0, s1) = if fr.progressive { (ss, se) } else { (0, 63) };
                    for k in s0..=s1.min(63) {
                        c.coef_bits[k] = if fr.progressive { al as i32 } else { 0 };
                    }
                }
                pos = decode_scan(d, pos + len, fr, &sc, ss, se, ah, al, &dc, &ac, restart)?;
                continue;
            }
            _ => {}
        }
        pos += len;
    }
    let mut fr = frame.ok_or("JPEG に画像がありません")?;
    if !eoi {
        return Err(format!("{TRUNCATED} (EOI がありません)"));
    }
    // プログレッシブで低い周波数の係数が最後まで来ていなければ、libjpeg は平滑化をかける(ここでは扱わない)
    for c in &fr.comps {
        if c.quant.is_none() {
            return Err("JPEG に走査の無い成分があります".into());
        }
        if fr.progressive && c.coef_bits[0] >= 0 && c.coef_bits[1..10].iter().any(|&b| b != 0) {
            return Err("係数が最後まで来ていないプログレッシブ JPEG です".into());
        }
    }
    // 逆 DCT → 成分の面
    let mut planes = Vec::new();
    for c in fr.comps.iter_mut() {
        let q = c.quant.ok_or("量子化表がありません")?;
        let stride = c.bw * 8;
        let mut data = vec![0u8; stride * c.bh * 8];
        let coefs = std::mem::take(&mut c.coefs);
        let bw = c.bw;
        data.par_chunks_mut(stride * 8).zip(coefs.par_chunks(bw * 64)).for_each(|(rows, cb)| {
            for bx in 0..bw {
                idct_islow(&cb[bx * 64..bx * 64 + 64], &q, &mut rows[bx * 8..], stride);
            }
        });
        planes.push((Plane { data, stride, dw: c.dw, dh: c.dh }, fr.hmax / c.h, fr.vmax / c.v));
    }
    let (w, h) = (fr.w, fr.h);
    if planes.len() == 1 {
        let (p, fh, fv) = &planes[0];
        let pix = upsample(p, *fh, *fv, w, h)?;
        return Ok(Jpeg { w, h, channels: 1, pix });
    }
    let ups: Vec<Vec<u8>> = planes.iter().map(|(p, fh, fv)| upsample(p, *fh, *fv, w, h)).collect::<Result<_, _>>()?;
    if ups.len() == 4 {
        // 4 成分: Adobe の transform = 2 なら YCCK(jdcolor.c の ycck_cmyk_convert)、それ以外は CMYK のまま
        let mut pix = vec![0u8; w * h * 4];
        if adobe == Some(2) {
            let (cr_r, cb_b, cr_g, cb_g) = ycc_tables();
            let clamp = |v: i32| v.clamp(0, 255) as u8;
            pix.par_chunks_mut(4).enumerate().for_each(|(i, o)| {
                let y = ups[0][i] as i32;
                let cb = ups[1][i] as usize;
                let cr = ups[2][i] as usize;
                o[0] = clamp(255 - (y + cr_r[cr]));
                o[1] = clamp(255 - (y + ((cb_g[cb] + cr_g[cr]) >> 16) as i32));
                o[2] = clamp(255 - (y + cb_b[cb]));
                o[3] = ups[3][i];
            });
        } else {
            pix.par_chunks_mut(4).enumerate().for_each(|(i, o)| {
                for (k, v) in o.iter_mut().enumerate() {
                    *v = ups[k][i];
                }
            });
        }
        return Ok(Jpeg { w, h, channels: 4, pix });
    }
    // 3 成分の色空間(libjpeg の default_decompress_parms と同じ決め方)
    let ids: Vec<u8> = fr.comps.iter().map(|c| c.id).collect();
    let is_rgb = if jfif {
        false
    } else if let Some(t) = adobe {
        t == 0
    } else {
        ids == [b'R', b'G', b'B']
    };
    let mut pix = vec![0u8; w * h * 3];
    if is_rgb {
        pix.par_chunks_mut(3).enumerate().for_each(|(i, o)| {
            o[0] = ups[0][i];
            o[1] = ups[1][i];
            o[2] = ups[2][i];
        });
    } else {
        let (cr_r, cb_b, cr_g, cb_g) = ycc_tables();
        let clamp = |v: i32| v.clamp(0, 255) as u8;
        pix.par_chunks_mut(3).enumerate().for_each(|(i, o)| {
            let y = ups[0][i] as i32;
            let cb = ups[1][i] as usize;
            let cr = ups[2][i] as usize;
            o[0] = clamp(y + cr_r[cr]);
            o[1] = clamp(y + ((cb_g[cb] + cr_g[cr]) >> 16) as i32);
            o[2] = clamp(y + cb_b[cb]);
        });
    }
    Ok(Jpeg { w, h, channels: 3, pix })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_table() {
        assert_eq!(range_limit(0), 128);
        assert_eq!(range_limit(127), 255);
        assert_eq!(range_limit(128), 255);
        assert_eq!(range_limit(-128), 0);
        assert_eq!(range_limit(-129), 0);
        assert_eq!(range_limit(-1), 127);
    }

    #[test]
    fn broken_does_not_panic() {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 8, 0, 16, 0, 16, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
        for i in 0..200 {
            v.push((i * 37 % 251) as u8);
            let _ = decode(&v);
        }
    }
}

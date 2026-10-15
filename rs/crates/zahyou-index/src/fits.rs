//! FITS のヘッダー(自前で読む。FITS 規格の 2880 バイトのブロックと 80 文字のカード)。
//!
//! Python 版 `_sv_fits_value` / `_sv_read_fits_hdus` と同じ読み方・同じエラーの文面にする。
//! データは読まずに飛ばす(位置と大きさだけを返す)。ファイルからでも、mmap したバイト列からでも読める。

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use crate::pyfmt::{float_repr, py_rstrip, py_strip, str_repr};

/// FITS のブロックの大きさ。
pub const FITS_BLOCK: usize = 2880;

/// カードの値(Python 版 `_sv_fits_value` が返す型に合わせる)。
#[derive(Clone, Debug, PartialEq)]
pub enum FitsValue {
    /// 値が空(Python の None)
    Null,
    Bool(bool),
    /// 整数(Python の int は桁数に上限が無いが、ここでは i128 に入るものだけ。入らなければ文字列として残る)
    Int(i128),
    Float(f64),
    Str(String),
}

impl FitsValue {
    /// Python の `repr` と同じ文字列(メッセージ用)。
    pub fn py_repr(&self) -> String {
        match self {
            FitsValue::Null => "None".to_string(),
            FitsValue::Bool(true) => "True".to_string(),
            FitsValue::Bool(false) => "False".to_string(),
            FitsValue::Int(i) => i.to_string(),
            FitsValue::Float(x) => float_repr(*x),
            FitsValue::Str(s) => str_repr(s),
        }
    }

    /// Python の `float(v)`(int・float・bool・数を表す文字列)。None は数にならない。
    pub fn py_float(&self) -> Option<f64> {
        match self {
            FitsValue::Null => None,
            FitsValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            FitsValue::Int(i) => Some(*i as f64),
            FitsValue::Float(x) => Some(*x),
            FitsValue::Str(s) => parse_py_float(py_strip(s)),
        }
    }
}

/// 1 つの HDU のヘッダー(同じキーは最初のものを使う。Python の `setdefault`)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    map: HashMap<String, FitsValue>,
}

impl Header {
    pub fn get(&self, key: &str) -> Option<&FitsValue> {
        self.map.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    fn set_default(&mut self, key: String, v: FitsValue) {
        self.map.entry(key).or_insert(v);
    }

    /// Python 版 `_sv_fits_int`: bool ではない整数でなければエラー(default は無いときの値)。
    pub fn int(&self, key: &str, default: Option<i128>) -> Result<i128, String> {
        match self.map.get(key) {
            Some(FitsValue::Int(i)) => Ok(*i),
            None if default.is_some() => Ok(default.unwrap_or(0)),
            Some(v) => Err(format!("ヘッダーの {key} が整数ではありません({})", v.py_repr())),
            None => Err(format!("ヘッダーの {key} が整数ではありません(None)")),
        }
    }

    /// 文字列の値(無い・文字列でなければ None)。
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.map.get(key) {
            Some(FitsValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// `repr` した値(無ければ "None")。
    pub fn repr(&self, key: &str) -> String {
        self.map.get(key).map(FitsValue::py_repr).unwrap_or_else(|| "None".to_string())
    }
}

/// HDU 1 つ: ヘッダーと、データの先頭の位置・バイト数。
#[derive(Clone, Debug, PartialEq)]
pub struct Hdu {
    pub header: Header,
    pub offset: u64,
    pub size: u64,
}

/// 読み込みのエラー。Io は OS のエラー(呼ぶ側が「読めません(…)」にする)、Bad は形式の問題。
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Bad(String),
}

impl From<String> for ReadError {
    fn from(s: String) -> ReadError {
        ReadError::Bad(s)
    }
}

/// ブロック単位の読み出し(ファイルでも、メモリ上のバイト列でもよい)。
pub trait BlockSource {
    /// pos から最大 buf.len() バイトを読み、読めたバイト数を返す(ファイルの終わりで短くなる)。
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize>;
}

impl BlockSource for File {
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.seek(SeekFrom::Start(pos))?;
        let mut got = 0;
        while got < buf.len() {
            match self.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(got)
    }
}

impl BlockSource for &[u8] {
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.len() as u64;
        if pos >= len {
            return Ok(0);
        }
        let start = pos as usize;
        let n = buf.len().min(self.len() - start);
        buf[..n].copy_from_slice(&self[start..start + n]);
        Ok(n)
    }
}

/// バイト列を ASCII として読み、ASCII 以外の 1 バイトを U+FFFD 1 文字にする(Python の decode("ascii", "replace"))。
fn ascii_replace(b: &[u8]) -> String {
    b.iter().map(|&c| if c < 0x80 { c as char } else { '\u{fffd}' }).collect()
}

/// Python の int(str) が受け付ける 10 進の整数(符号、桁の間の _ は 1 つまで)。
fn parse_py_int(t: &str) -> Option<i128> {
    let (neg, body) = match t.as_bytes().first() {
        Some(b'+') => (false, &t[1..]),
        Some(b'-') => (true, &t[1..]),
        _ => (false, t),
    };
    if body.is_empty() {
        return None;
    }
    let mut digits = String::with_capacity(body.len());
    let mut prev_digit = false;
    let bytes = body.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c.is_ascii_digit() {
            digits.push(c as char);
            prev_digit = true;
        } else if c == b'_' && prev_digit && bytes.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
            prev_digit = false;
        } else {
            return None;
        }
    }
    let v: i128 = digits.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// Python の float(str)(桁の間の _ を許す。inf・nan は大小文字を問わない)。
pub(crate) fn parse_py_float(t: &str) -> Option<f64> {
    if t.is_empty() {
        return None;
    }
    let b = t.as_bytes();
    let mut s = String::with_capacity(t.len());
    for (i, &c) in b.iter().enumerate() {
        if c == b'_' {
            let ok = i > 0 && b[i - 1].is_ascii_digit() && b.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            if !ok {
                return None;
            }
        } else if c.is_ascii() {
            s.push(c as char);
        } else {
            return None;
        }
    }
    // Rust は "infinity"・"inf"・"nan" を受け付ける。Python が受け付けない形("1e" など)はどちらも拒む
    s.parse::<f64>().ok()
}

/// カードの値の部分(11 桁目から後ろ)を値にする(Python 版 `_sv_fits_value`)。
pub fn parse_value(text: &str) -> FitsValue {
    let t = py_strip(text);
    if let Some(rest) = t.strip_prefix('\'') {
        let chars: Vec<char> = rest.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '\'' {
                if chars.get(i + 1) == Some(&'\'') {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                break;
            }
            out.push(ch);
            i += 1;
        }
        return FitsValue::Str(py_rstrip(&out).to_string());
    }
    let mut t = t;
    if let Some((head, _)) = t.split_once('/') {
        t = py_strip(head);
    }
    match t {
        "T" => return FitsValue::Bool(true),
        "F" => return FitsValue::Bool(false),
        "" => return FitsValue::Null,
        _ => {}
    }
    if let Some(i) = parse_py_int(t) {
        return FitsValue::Int(i);
    }
    if let Some(x) = parse_py_float(&t.replace('D', "E")) {
        return FitsValue::Float(x);
    }
    FitsValue::Str(t.to_string())
}

/// ファイルの HDU を頭から順に読む(Python 版 `_sv_read_fits_hdus`)。データは読まずに飛ばす。
///
/// 戻り値 (hdus, end, size)。end = ヘッダーから計算したファイルの長さ(詰め物込み)、size = 実際の大きさ。
/// 最後の HDU の後ろに XTENSION で始まらないものがあれば、そこで止める(end < size になる)。
pub fn read_hdus<S: BlockSource>(src: &mut S, size: u64) -> Result<(Vec<Hdu>, u64, u64), ReadError> {
    let mut hdus: Vec<Hdu> = Vec::new();
    let mut pos: u64 = 0;
    let mut blk = vec![0u8; FITS_BLOCK];
    while pos < size {
        let mut hdr = Header::default();
        let mut nblk: u64 = 0;
        let mut done = false;
        while !done {
            let at = pos
                .checked_add(nblk * FITS_BLOCK as u64)
                .ok_or_else(|| ReadError::Bad("ヘッダーが大きすぎます".to_string()))?;
            let got = src.read_at(at, &mut blk).map_err(ReadError::Io)?;
            let blk = &blk[..got];
            if nblk == 0 {
                let first = ascii_replace(&blk[..blk.len().min(8)]);
                let first = py_strip(&first);
                if hdus.is_empty() && first != "SIMPLE" {
                    return Err(ReadError::Bad(format!("FITS ではありません(先頭が {})", str_repr(first))));
                }
                if !hdus.is_empty() && first != "XTENSION" {
                    return Ok((hdus, pos, size)); // 後ろに余計なもの(呼ぶ側が大きさで気づく)
                }
            }
            if got < FITS_BLOCK {
                return Err(ReadError::Bad(format!(
                    "ヘッダーの途中でファイルが終わっています({} 個目の HDU、{} バイト目から)",
                    hdus.len() + 1,
                    pos
                )));
            }
            nblk += 1;
            for card in blk.chunks_exact(80) {
                let key_s = ascii_replace(&card[..8]);
                let key = py_strip(&key_s);
                if key == "END" {
                    done = true;
                    break;
                }
                if !key.is_empty() && &card[8..10] == b"= " {
                    let v = parse_value(&ascii_replace(&card[10..]));
                    hdr.set_default(key.to_string(), v);
                }
            }
        }
        let naxis = hdr.int("NAXIS", Some(0))?;
        let mut n: i128 = 0;
        let too_big = || ReadError::Bad("ヘッダーのデータの大きさが大きすぎます".to_string());
        if naxis > 0 {
            n = 1;
            let mut k: i128 = 1;
            while k <= naxis {
                let ax = hdr.int(&format!("NAXIS{k}"), None)?;
                n = n.checked_mul(ax).ok_or_else(too_big)?;
                k += 1;
            }
            let bitpix = hdr.int("BITPIX", None)?;
            let gcount = hdr.int("GCOUNT", Some(1))?;
            let pcount = hdr.int("PCOUNT", Some(0))?;
            // Python の // は負の数で切り下げ(abs(bitpix) は負にならないので普通の割り算と同じ)
            let bytes = bitpix.checked_abs().ok_or_else(too_big)? / 8;
            n = bytes
                .checked_mul(gcount)
                .and_then(|v| pcount.checked_add(n).and_then(|p| v.checked_mul(p)))
                .ok_or_else(too_big)?;
        }
        if n < 0 {
            return Err(ReadError::Bad("ヘッダーのデータの大きさが負です".to_string()));
        }
        let n = u64::try_from(n).map_err(|_| too_big())?;
        let data = pos.checked_add(nblk * FITS_BLOCK as u64).ok_or_else(too_big)?;
        hdus.push(Hdu { header: hdr, offset: data, size: n });
        let padded = n.div_ceil(FITS_BLOCK as u64).checked_mul(FITS_BLOCK as u64).ok_or_else(too_big)?;
        pos = data.checked_add(padded).ok_or_else(too_big)?;
    }
    if hdus.is_empty() {
        return Err(ReadError::Bad("空のファイルです".to_string()));
    }
    Ok((hdus, pos, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_like_python() {
        assert_eq!(parse_value(" 'QUAD    '           / kind"), FitsValue::Str("QUAD".into()));
        assert_eq!(parse_value(" 'it''s'"), FitsValue::Str("it's".into()));
        assert_eq!(parse_value("                   4 / dim"), FitsValue::Int(4));
        assert_eq!(parse_value("  -1"), FitsValue::Int(-1));
        assert_eq!(parse_value("  1_000"), FitsValue::Int(1000));
        assert_eq!(parse_value("  0.0063995 / rad"), FitsValue::Float(0.0063995));
        assert_eq!(parse_value("  1.5D3"), FitsValue::Float(1500.0));
        assert_eq!(parse_value("  T"), FitsValue::Bool(true));
        assert_eq!(parse_value("  F / x"), FitsValue::Bool(false));
        assert_eq!(parse_value("    / only a comment"), FitsValue::Null);
        assert_eq!(parse_value("  abc"), FitsValue::Str("abc".into()));
        assert_eq!(parse_value("  'unterminated  "), FitsValue::Str("unterminated".into()));
        assert_eq!(parse_value("  1__0"), FitsValue::Str("1__0".into()));
        assert_eq!(parse_value("  4.0"), FitsValue::Float(4.0));
    }

    #[test]
    fn broken_bytes_do_not_panic() {
        // 何を渡しても panic しない(短い・空・SIMPLE だけ・END の無いヘッダー)
        let mut cases: Vec<Vec<u8>> = vec![vec![], b"SIMPLE".to_vec(), vec![b' '; 2880], vec![0xff; 5000]];
        let mut h = vec![b' '; 2880 * 2];
        h[..6].copy_from_slice(b"SIMPLE");
        cases.push(h.clone());
        let mut card = |i: usize, s: &str| h[i * 80..i * 80 + s.len()].copy_from_slice(s.as_bytes());
        card(1, "NAXIS   = 99999999999999999999999999999999999");
        card(2, "END");
        cases.push(h.clone());
        for c in cases {
            let mut s: &[u8] = &c;
            let _ = read_hdus(&mut s, c.len() as u64);
        }
    }
}

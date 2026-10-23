//! FITS を自前で読む(Python 版の `_load_fits`・`_pick_fits_hdu`。astropy の `fits.open` と同じ意味)。
//!
//! - ヘッダーは 2880 バイトの区切りに 80 文字のカードが 36 枚。END のカードで終わる。
//! - 値の読み方は astropy と同じ: 文字列('' は ' 1 文字、後ろの空白は捨てる)・T/F・整数・実数(D 指数も)、
//!   HIERARCH(ESO 式の長いキー)、CONTINUE(& で続く長い文字列)。値の無いカードは `CardValue::None`。
//! - データの大きさは |BITPIX|/8 × GCOUNT × (PCOUNT + NAXIS1 × … × NAXISn)。次の HDU は 2880 の倍数に切り上げた先。
//! - 画像の HDU(先頭・IMAGE 拡張・タイル圧縮)のうち NAXIS ≥ 2 で、画素数が最も多いもの(同じなら先のもの)を選ぶ。
//! - gzip で固めたファイル(先頭が 1f 8b)は丸ごと展開してから読む(astropy と同じく拡張子ではなく中身で決める)。
//! - 壊れたファイル(ヘッダーの途中で切れている・END が無い・数字が壊れている・データが足りない)は Err。

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::MultiGzDecoder;
use rayon::prelude::*;
use zahyou_wcs::{Card, CardValue};

use crate::array::{ArraySource, ScaleKind, Scaling, py_tuple, reduce_to_2d};
use crate::tile;

pub(crate) const BLOCK: u64 = 2880;
const CARD_LEN: usize = 80;
/// ヘッダーがこれより長ければ壊れているとみなす(36 枚 × 1 万区切り = 36 万枚)
const MAX_HEADER_BLOCKS: u64 = 10_000;

/// バイトの出どころ(ファイルか、展開済みのメモリ)。
pub(crate) enum Src {
    File { f: File, len: u64 },
    Mem(Vec<u8>),
}

impl Src {
    pub fn open(path: &Path) -> Result<Src, String> {
        let mut f = File::open(path).map_err(|e| format!("ファイルを開けません: {e}"))?;
        let len = f.metadata().map_err(|e| format!("ファイルの大きさを読めません: {e}"))?.len();
        let mut magic = [0u8; 2];
        let n = read_up_to(&mut f, &mut magic).map_err(|e| format!("ファイルを読めません: {e}"))?;
        if n == 2 && magic == [0x1f, 0x8b] {
            f.seek(SeekFrom::Start(0)).map_err(|e| format!("ファイルを読めません: {e}"))?;
            return Ok(Src::Mem(gunzip_all(f)?));
        }
        Ok(Src::File { f, len })
    }

    pub fn len(&self) -> u64 {
        match self {
            Src::File { len, .. } => *len,
            Src::Mem(v) => v.len() as u64,
        }
    }

    /// off から buf.len() バイトをちょうど読む。足りなければ Err。
    pub fn read_at(&mut self, off: u64, buf: &mut [u8]) -> Result<(), String> {
        let end = off.checked_add(buf.len() as u64).ok_or("読み出しの位置が大きすぎます")?;
        if end > self.len() {
            return Err(format!(
                "FITS のデータが途中で切れています (必要 {end} バイト、ファイルは {} バイト)",
                self.len()
            ));
        }
        match self {
            Src::File { f, .. } => {
                f.seek(SeekFrom::Start(off)).map_err(|e| format!("ファイルを読めません: {e}"))?;
                f.read_exact(buf).map_err(|e| format!("ファイルを読めません: {e}"))
            }
            Src::Mem(v) => {
                buf.copy_from_slice(&v[off as usize..end as usize]);
                Ok(())
            }
        }
    }

    /// off から最大 buf.len() バイト読む(ファイルの終わりで止まる)。読んだ数を返す。
    fn read_some(&mut self, off: u64, buf: &mut [u8]) -> Result<usize, String> {
        let avail = self.len().saturating_sub(off).min(buf.len() as u64) as usize;
        self.read_at(off, &mut buf[..avail])?;
        Ok(avail)
    }
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// gzip を丸ごと展開する(複数のメンバーも続けて)。メモリが足りなければ Err(abort しない)。
pub(crate) fn gunzip_all(f: File) -> Result<Vec<u8>, String> {
    let mut d = MultiGzDecoder::new(BufReader::new(f));
    let mut out: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = match d.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("gzip を展開できません: {e}")),
        };
        out.try_reserve(n).map_err(|_| "gzip を展開した中身が大きすぎてメモリに載りません".to_string())?;
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

// ------------------------------------------------------------------ カード ---

/// 1 バイト = 1 文字(Latin-1)として文字列にする。
fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let s = b.iter().position(|&c| c != b' ').unwrap_or(b.len());
    let e = b.iter().rposition(|&c| c != b' ').map(|i| i + 1).unwrap_or(s);
    &b[s..e.max(s)]
}

/// 値の欄(`= ` の後ろ)を読む。戻り値は (値, 注記)。
fn parse_value(v: &[u8]) -> (CardValue, String) {
    let s = v.iter().position(|&c| c != b' ').unwrap_or(v.len());
    let v = &v[s..];
    if v.first() == Some(&b'\'') {
        // 文字列: '' は ' 1 文字。閉じの ' の後ろの / から先が注記
        let mut out = Vec::new();
        let mut i = 1;
        let mut closed = None;
        while i < v.len() {
            if v[i] == b'\'' {
                if i + 1 < v.len() && v[i + 1] == b'\'' {
                    out.push(b'\'');
                    i += 2;
                    continue;
                }
                closed = Some(i);
                break;
            }
            out.push(v[i]);
            i += 1;
        }
        while out.last() == Some(&b' ') {
            out.pop();
        }
        let comment = match closed {
            Some(c) => match v[c + 1..].iter().position(|&x| x == b'/') {
                Some(p) => latin1(trim_bytes(&v[c + 2 + p..])),
                None => String::new(),
            },
            None => String::new(),
        };
        return (CardValue::Str(latin1(&out)), comment);
    }
    let (tok, comment) = match v.iter().position(|&c| c == b'/') {
        Some(p) => (trim_bytes(&v[..p]), latin1(trim_bytes(&v[p + 1..]))),
        None => (trim_bytes(v), String::new()),
    };
    (parse_token(tok), comment)
}

/// 文字列以外の値(T/F・整数・実数)。読めなければ None(astropy の Undefined と同じ扱い)。
fn parse_token(tok: &[u8]) -> CardValue {
    if tok.is_empty() {
        return CardValue::None;
    }
    if tok == b"T" {
        return CardValue::Bool(true);
    }
    if tok == b"F" {
        return CardValue::Bool(false);
    }
    if tok.first() == Some(&b'(') {
        // 複素数はそのままの文字で持つ(zahyou では使わない)
        return CardValue::Str(latin1(tok));
    }
    if !tok.iter().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'E' | b'e' | b'D' | b'd')) {
        return CardValue::None;
    }
    let is_int = tok.iter().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-'));
    let text: String = tok.iter().map(|&c| if c == b'D' || c == b'd' { 'E' } else { c as char }).collect();
    if is_int && let Ok(i) = text.parse::<i64>() {
        return CardValue::Int(i);
    }
    match text.parse::<f64>() {
        Ok(f) if !text.is_empty() && text.bytes().any(|c| c.is_ascii_digit()) => CardValue::Float(f),
        _ => CardValue::None,
    }
}

/// 生のカード 1 枚。
struct RawCard {
    key: String,
    value: CardValue,
    comment: String,
    /// CONTINUE カードの文字列(前の文字列の続き)
    cont: Option<String>,
}

fn parse_card(img: &[u8]) -> RawCard {
    let key8 = latin1(trim_bytes(&img[..8])).to_ascii_uppercase();
    // HIERARCH: 「HIERARCH ESO DET CHIP NAME = 'chip-1'」
    if key8 == "HIERARCH"
        && let Some(eq) = img.iter().position(|&c| c == b'=')
        && eq > 8
    {
        let key = latin1(trim_bytes(&img[8..eq])).to_ascii_uppercase();
        let (value, comment) = parse_value(&img[eq + 1..]);
        return RawCard { key, value, comment, cont: None };
    }
    if key8 == "CONTINUE" {
        let (value, comment) = parse_value(&img[8..]);
        let cont = match &value {
            CardValue::Str(s) => Some(s.clone()),
            _ => None,
        };
        return RawCard { key: key8, value: CardValue::None, comment, cont };
    }
    let commentary = matches!(key8.as_str(), "" | "COMMENT" | "HISTORY" | "END");
    if !commentary && &img[8..10] == b"= " {
        let (value, comment) = parse_value(&img[10..]);
        return RawCard { key: key8, value, comment, cont: None };
    }
    let text = latin1(&img[8..]);
    RawCard { key: key8, value: CardValue::None, comment: text.trim_end().to_string(), cont: None }
}

/// 生のカードの並び → Card の並び(CONTINUE をつなぐ)。END は含めない。
fn cards_from_raw(raw: Vec<RawCard>) -> Vec<Card> {
    let mut out: Vec<Card> = Vec::with_capacity(raw.len());
    for rc in raw {
        if rc.key == "END" {
            break;
        }
        if rc.key == "CONTINUE" && rc.cont.is_some() {
            // 直前の文字列が & で終わっていなければ、続きではない(注記として残す)
            let joined = match out.last_mut() {
                Some(Card { value: CardValue::Str(s), comment, .. }) if s.ends_with('&') => {
                    s.pop();
                    s.push_str(rc.cont.as_deref().unwrap_or(""));
                    if !rc.comment.is_empty() {
                        if !comment.is_empty() {
                            comment.push(' ');
                        }
                        comment.push_str(&rc.comment);
                    }
                    true
                }
                _ => false,
            };
            if joined {
                continue;
            }
        }
        out.push(Card { key: rc.key, value: rc.value, comment: rc.comment });
    }
    out
}

/// off から始まるヘッダーを読む。戻り値は (カード, データの始まり)。
fn read_header(src: &mut Src, off: u64) -> Result<(Vec<Card>, u64), String> {
    let mut raw = Vec::new();
    let mut block = vec![0u8; BLOCK as usize];
    let mut pos = off;
    for _ in 0..MAX_HEADER_BLOCKS {
        let n = src.read_some(pos, &mut block)?;
        if n < CARD_LEN {
            return Err(if raw.is_empty() {
                "FITS が空か壊れています (ヘッダーがありません)".to_string()
            } else {
                "FITS のヘッダーに END がありません".to_string()
            });
        }
        for c in block[..n - n % CARD_LEN].chunks(CARD_LEN) {
            let rc = parse_card(c);
            let end = rc.key == "END";
            raw.push(rc);
            if end {
                return Ok((cards_from_raw(raw), pos + BLOCK));
            }
        }
        if n < BLOCK as usize {
            return Err("FITS のヘッダーに END がありません".to_string());
        }
        pos += BLOCK;
    }
    Err("FITS のヘッダーが長すぎます (END が見つかりません)".to_string())
}

// ---------------------------------------------------------------- ヘッダー ---

/// 最初に一致したキーの値。
pub(crate) fn get<'a>(cards: &'a [Card], key: &str) -> Option<&'a CardValue> {
    cards.iter().find(|c| c.key == key).map(|c| &c.value)
}

/// 整数のキー(実数でも整数値なら可)。無ければ None、あっても整数でなければ Err。
pub(crate) fn get_int(cards: &[Card], key: &str) -> Result<Option<i64>, String> {
    match get(cards, key) {
        None => Ok(None),
        Some(CardValue::Int(i)) => Ok(Some(*i)),
        Some(CardValue::Float(f)) if f.fract() == 0.0 && f.abs() < 9.0e18 => Ok(Some(*f as i64)),
        Some(_) => Err(format!("FITS のヘッダーの {key} が整数ではありません")),
    }
}

/// 数のキー。無ければ None、あっても数でなければ Err。
pub(crate) fn get_num(cards: &[Card], key: &str) -> Result<Option<f64>, String> {
    match get(cards, key) {
        None => Ok(None),
        Some(CardValue::Int(i)) => Ok(Some(*i as f64)),
        Some(CardValue::Float(f)) => Ok(Some(*f)),
        Some(_) => Err(format!("FITS のヘッダーの {key} が数ではありません")),
    }
}

pub(crate) fn get_str<'a>(cards: &'a [Card], key: &str) -> Option<&'a str> {
    match get(cards, key) {
        Some(CardValue::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// NAXIS と NAXIS1..n(n は NAXIS)。負や大きすぎる値は Err。
pub(crate) fn axes(cards: &[Card], prefix: &str) -> Result<Vec<usize>, String> {
    let naxis = get_int(cards, prefix)?.unwrap_or(0);
    if !(0..=999).contains(&naxis) {
        return Err(format!("FITS のヘッダーの {prefix} が範囲外です: {naxis}"));
    }
    let mut v = Vec::with_capacity(naxis as usize);
    for i in 1..=naxis {
        let k = format!("{prefix}{i}");
        let n = get_int(cards, &k)?.ok_or_else(|| format!("FITS のヘッダーに {k} がありません"))?;
        if n < 0 || n > i64::from(u32::MAX) {
            return Err(format!("FITS のヘッダーの {k} が範囲外です: {n}"));
        }
        v.push(n as usize);
    }
    Ok(v)
}

// -------------------------------------------------------------------- HDU ---

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Primary,
    Image,
    CompImage,
    Other,
}

pub(crate) struct Hdu {
    pub index: usize,
    pub cards: Vec<Card>,
    pub kind: Kind,
    pub data_start: u64,
    /// データの大きさ(バイト、2880 への詰め物を除く)
    pub data_len: u64,
}

/// HDU のデータの大きさ: |BITPIX|/8 × GCOUNT × (PCOUNT + 軸の積)。軸が無ければ 0。
fn data_size(cards: &[Card]) -> Result<u64, String> {
    let bitpix = get_int(cards, "BITPIX")?.ok_or("FITS のヘッダーに BITPIX がありません")?;
    if !matches!(bitpix, 8 | 16 | 32 | 64 | -32 | -64) {
        return Err(format!("FITS の BITPIX が不正です: {bitpix}"));
    }
    let ax = axes(cards, "NAXIS")?;
    if ax.is_empty() {
        return Ok(0);
    }
    let gcount = get_int(cards, "GCOUNT")?.unwrap_or(1);
    let pcount = get_int(cards, "PCOUNT")?.unwrap_or(0);
    if gcount < 0 || pcount < 0 {
        return Err("FITS のヘッダーの PCOUNT・GCOUNT が負です".to_string());
    }
    let mut n: u64 = 1;
    for &a in &ax {
        n = n.checked_mul(a as u64).ok_or("FITS のデータの大きさが大きすぎます")?;
    }
    let n = n.checked_add(pcount as u64).ok_or("FITS のデータの大きさが大きすぎます")?;
    n.checked_mul(gcount as u64)
        .and_then(|v| v.checked_mul(bitpix.unsigned_abs() / 8))
        .ok_or_else(|| "FITS のデータの大きさが大きすぎます".to_string())
}

/// HDU を順に読む。先頭の HDU が読めなければ Err。2 つ目以降が壊れていればそこで止める(astropy と同じく読めた分は使う)。
pub(crate) fn scan_hdus(src: &mut Src) -> Result<Vec<Hdu>, String> {
    let mut hdus = Vec::new();
    let mut off = 0u64;
    let len = src.len();
    if len == 0 {
        return Err("FITS が空か壊れています (0 バイト)".to_string());
    }
    while off < len {
        let idx = hdus.len();
        let (cards, data_start) = match read_header(src, off) {
            Ok(x) => x,
            Err(e) if idx == 0 => return Err(e),
            Err(_) => break,
        };
        let first = cards.first().map(|c| (c.key.clone(), c.value.clone()));
        let mut kind = match (idx, &first) {
            (0, Some((k, CardValue::Bool(true)))) if k == "SIMPLE" => Kind::Primary,
            (0, _) => Kind::Other,
            (_, Some((k, CardValue::Str(x)))) if k == "XTENSION" => match x.trim().to_ascii_uppercase().as_str() {
                "IMAGE" => Kind::Image,
                "BINTABLE" if get(&cards, "ZIMAGE") == Some(&CardValue::Bool(true)) => Kind::CompImage,
                _ => Kind::Other,
            },
            _ => break, // 拡張のヘッダーではない(末尾のごみ)
        };
        // SIMPLE の無い先頭(ignore_missing_simple)は、標準の HDU として読めないので後ろは見ない
        let nonstandard = idx == 0 && kind == Kind::Other;
        let (size, broken) = match data_size(&cards) {
            Ok(s) => (s, false),
            Err(e) if idx == 0 && !nonstandard => return Err(e),
            Err(_) => (0, true),
        };
        if broken {
            kind = Kind::Other;
        }
        hdus.push(Hdu { index: idx, cards, kind, data_start, data_len: size });
        // 大きさの分からない HDU の後ろは、次の HDU の位置が決まらないので見ない
        if nonstandard || broken {
            break;
        }
        let padded = size.div_ceil(BLOCK).checked_mul(BLOCK).ok_or("FITS のデータの大きさが大きすぎます")?;
        off = match data_start.checked_add(padded) {
            Some(o) => o,
            None => break,
        };
    }
    Ok(hdus)
}

/// 画像として扱う HDU の軸(FITS の順 NAXIS1..n)。タイル圧縮は ZNAXIS。
fn image_axes(h: &Hdu) -> Result<Vec<usize>, String> {
    match h.kind {
        Kind::Primary | Kind::Image => axes(&h.cards, "NAXIS"),
        Kind::CompImage => axes(&h.cards, "ZNAXIS"),
        Kind::Other => Ok(Vec::new()),
    }
}

/// `_pick_fits_hdu`: NAXIS ≥ 2 の画像 HDU のうち画素数が最も多いもの(同じなら先)。
fn pick_hdu(hdus: &[Hdu]) -> Option<usize> {
    let mut best: Option<(usize, u128)> = None;
    for (i, h) in hdus.iter().enumerate() {
        let Ok(ax) = image_axes(h) else { continue };
        if ax.len() < 2 {
            continue;
        }
        let size: u128 = ax.iter().map(|&a| a as u128).product();
        if best.is_none_or(|(_, b)| size > b) {
            best = Some((i, size));
        }
    }
    best.map(|(i, _)| i)
}

// --------------------------------------------------------------- データ ---

/// ファイル上の画像データ(ビッグエンディアン)を、要る区間だけ読む配列。
struct FileArray<'a> {
    src: &'a mut Src,
    start: u64,
    shape: Vec<usize>,
    bitpix: i32,
    sc: Scaling,
    /// ファイルから読んだバイトを置く作業領域(呼ぶたびに作り直さず使い回す。最大 READ_CHUNK 要素ぶん)。
    /// 小さく区切って何度も読む呼び方(画素ごとに色が並ぶ配列の輝度の合成)で、毎回新しいページを触らないため
    scratch: Vec<u8>,
}

/// 一度に読む要素数
const READ_CHUNK: usize = 1 << 22;
/// 変換を並列にする単位
const CONV_CHUNK: usize = 1 << 15;

impl FileArray<'_> {
    fn bpp(&self) -> usize {
        (self.bitpix.unsigned_abs() / 8) as usize
    }

    /// start から out.len() 個を読み、1 要素ぶんのバイト(ビッグエンディアン)に conv をかけて out へ。
    /// conv は静的に渡す(以前の `&dyn Fn` は 1 画素ごとの間接呼び出しで、インライン化もベクトル化もされなかった)。
    fn read_conv<T: Copy + Send, F: Fn(&[u8]) -> T + Sync>(
        &mut self,
        start: usize,
        out: &mut [T],
        conv: F,
    ) -> Result<(), String> {
        let bpp = self.bpp();
        let total: usize = self.shape.iter().product();
        if start.checked_add(out.len()).is_none_or(|e| e > total) {
            return Err("配列の範囲外を読もうとしました".into());
        }
        let need = out.len().min(READ_CHUNK) * bpp;
        if self.scratch.len() < need {
            // vec! の 0 埋めは OS の 0 のページをそのまま使う(resize のように直列に 0 を書かない)
            self.scratch = vec![0u8; need];
        }
        let buf = &mut self.scratch;
        for (ci, chunk) in out.chunks_mut(READ_CHUNK).enumerate() {
            let off = self.start + ((start + ci * READ_CHUNK) as u64) * bpp as u64;
            let b = &mut buf[..chunk.len() * bpp];
            self.src.read_at(off, b)?;
            chunk.par_chunks_mut(CONV_CHUNK).zip(b.par_chunks(CONV_CHUNK * bpp)).for_each(|(o, raw)| {
                for (v, r) in o.iter_mut().zip(raw.chunks_exact(bpp)) {
                    *v = conv(r);
                }
            });
        }
        Ok(())
    }
}

#[inline]
fn be_int(r: &[u8], bitpix: i32) -> i64 {
    match bitpix {
        8 => r[0] as i64,
        16 => i16::from_be_bytes([r[0], r[1]]) as i64,
        32 => i32::from_be_bytes([r[0], r[1], r[2], r[3]]) as i64,
        _ => i64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]),
    }
}

#[inline]
fn be_float(r: &[u8], bitpix: i32) -> f64 {
    if bitpix == -32 {
        f32::from_be_bytes([r[0], r[1], r[2], r[3]]) as f64
    } else {
        f64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]])
    }
}

impl ArraySource for FileArray<'_> {
    fn shape(&self) -> Vec<usize> {
        self.shape.clone()
    }

    fn read_f32(&mut self, start: usize, out: &mut [f32]) -> Result<(), String> {
        let (bp, sc) = (self.bitpix, self.sc.clone());
        // よくある型は、画素ごとの (BITPIX, 型の規則) の分岐を外へ出した専用の閉包にする。
        // 式は下の一般の閉包(be_int / be_float → Scaling::int_f32 / float_f32)をその型について展開したものと同じ:
        // - 整数 Raw: `raw as f32`(raw は be_int の i64)
        // - 16・32 bit の PseudoUnsigned: `Scaling::unsigned` の `(raw + 2^(bits-1)) as u64` を f32 に
        // - float32 Raw: 一般の閉包では be_float の `f32 as f64` と float_f32 の `as f32` の往復(下の注)
        // - float64 Raw: `f64 as f32`(CPU の変換命令。一般の閉包と同じ)
        match (bp, sc.kind) {
            (8, ScaleKind::Raw) => self.read_conv(start, out, |r| r[0] as i64 as f32),
            (16, ScaleKind::Raw) => self.read_conv(start, out, |r| i16::from_be_bytes([r[0], r[1]]) as i64 as f32),
            (16, ScaleKind::PseudoUnsigned) => {
                self.read_conv(start, out, |r| ((i16::from_be_bytes([r[0], r[1]]) as i64 + 32768) as u64) as f32)
            }
            (32, ScaleKind::Raw) => {
                self.read_conv(start, out, |r| i32::from_be_bytes([r[0], r[1], r[2], r[3]]) as i64 as f32)
            }
            (32, ScaleKind::PseudoUnsigned) => self.read_conv(start, out, |r| {
                ((i32::from_be_bytes([r[0], r[1], r[2], r[3]]) as i64 + 2147483648) as u64) as f32
            }),
            // 以前は f32 → f64 → f32 の往復を CPU の変換命令でしていた(`&dyn Fn` の中で型が実行時に決まるので、
            // 往復が省かれなかった)。往復は NaN 以外と静かな NaN ではビットを変えず、信号の NaN は「静か」のビット
            // (0x0040_0000)を立てる(2026-10 に以前の版の release で確かめた: 7f800001 → 7fc00001)。
            // 型が決まった閉包では、コンパイラーが往復を省いてよい(Rust の約束、RFC 3514)ので信号の NaN が
            // そのまま残る。同じビットにするため、往復の結果を整数で明示的に作る
            (-32, ScaleKind::Raw) => self.read_conv(start, out, |r| {
                let b = u32::from_be_bytes([r[0], r[1], r[2], r[3]]);
                f32::from_bits(if b & 0x7fff_ffff > 0x7f80_0000 { b | 0x0040_0000 } else { b })
            }),
            (-64, ScaleKind::Raw) => self
                .read_conv(start, out, |r| f64::from_be_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]) as f32),
            _ if bp > 0 => self.read_conv(start, out, |r| sc.int_f32(be_int(r, bp))),
            _ => self.read_conv(start, out, |r| sc.float_f32(be_float(r, bp))),
        }
    }

    fn read_f64(&mut self, start: usize, out: &mut [f64]) -> Result<(), String> {
        let (bp, sc) = (self.bitpix, self.sc.clone());
        if bp > 0 {
            self.read_conv(start, out, |r| sc.int_f64(be_int(r, bp)))
        } else {
            self.read_conv(start, out, |r| sc.float_f64(be_float(r, bp)))
        }
    }
}

/// FITS から読んだ 2 次元の画像。
pub(crate) struct FitsImage {
    pub data: Vec<f32>,
    pub ny: usize,
    pub nx: usize,
    pub header: Vec<Card>,
    pub notes: Vec<String>,
    pub flip_y: bool,
}

/// BSCALE・BZERO・BLANK から astropy の型の規則を作る。
fn scaling_of(cards: &[Card], bitpix: i32, native: bool) -> Result<Scaling, String> {
    let bscale = get_num(cards, "BSCALE")?.unwrap_or(1.0);
    let bzero = get_num(cards, "BZERO")?.unwrap_or(0.0);
    let blank = match get(cards, "BLANK") {
        Some(CardValue::Int(i)) => Some(*i),
        _ => None,
    };
    Ok(Scaling::new(bitpix, bscale, bzero, blank, native))
}

/// Python の `float(header["BLANK"])`。数・真偽値はそのまま、文字列は数として読めなければ Err。
fn blank_value(cards: &[Card]) -> Result<Option<f64>, String> {
    match get(cards, "BLANK") {
        None => Ok(None),
        Some(CardValue::Int(i)) => Ok(Some(*i as f64)),
        Some(CardValue::Float(f)) => Ok(Some(*f)),
        Some(CardValue::Bool(b)) => Ok(Some(if *b { 1.0 } else { 0.0 })),
        Some(CardValue::Str(s)) => {
            s.trim().parse::<f64>().map(Some).map_err(|_| format!("FITS の BLANK を数として読めません: '{s}'"))
        }
        // 値の無い BLANK(astropy の Undefined)は float() できない
        Some(CardValue::None) => Err("FITS の BLANK に値がありません".to_string()),
    }
}

/// `_load_fits`: いちばん大きい画像 HDU を 2 次元の float32 にする。
pub(crate) fn load_fits(path: &Path) -> Result<FitsImage, String> {
    let mut src = Src::open(path)?;
    let hdus = scan_hdus(&mut src)?;
    let Some(pick) = pick_hdu(&hdus) else {
        return Err("画像データを含む HDU が FITS の中にありません。".to_string());
    };
    let hdu = &hdus[pick];
    let mut notes = Vec::new();
    let (data, ny, nx, note, header) = match hdu.kind {
        Kind::CompImage => {
            let (mut arr, header) = tile::decompress(&mut src, hdu)?;
            notes.push(format!("FITS HDU[{}] {} {}", hdu.index, py_tuple(&arr.shape), arr.scaling.dtype));
            let (d, ny, nx, note) = reduce_to_2d(&mut arr, 200)?;
            (d, ny, nx, note, header)
        }
        _ => {
            let cards = &hdu.cards;
            let bitpix = get_int(cards, "BITPIX")?.unwrap_or(0) as i32;
            let mut shape = axes(cards, "NAXIS")?;
            shape.reverse(); // numpy の順(最後の軸が NAXIS1)
            let sc = scaling_of(cards, bitpix, false)?;
            // 画像 HDU は PCOUNT = 0・GCOUNT = 1 なので、走査で求めた大きさがそのまま画素の分
            let need = hdu.data_len;
            if hdu.data_start.saturating_add(need) > src.len() {
                return Err(format!(
                    "FITS のデータが途中で切れています (HDU[{}] に {need} バイト要るところ、{} バイトしかありません)",
                    hdu.index,
                    src.len().saturating_sub(hdu.data_start)
                ));
            }
            notes.push(format!("FITS HDU[{}] {} {}", hdu.index, py_tuple(&shape), sc.dtype));
            let mut arr = FileArray { src: &mut src, start: hdu.data_start, shape, bitpix, sc, scratch: Vec::new() };
            let (d, ny, nx, note) = reduce_to_2d(&mut arr, 200)?;
            (d, ny, nx, note, hdu.cards.clone())
        }
    };
    if !note.is_empty() {
        notes.push(note);
    }
    let mut data = data;
    // data[data == float(blank)] = nan(float32 の配列と Python の float の比較は float32 で)
    if let Some(b) = blank_value(&header)? {
        let b32 = b as f32;
        data.par_iter_mut().for_each(|v| {
            if *v == b32 {
                *v = f32::NAN;
            }
        });
    }
    // SharpCap / FireCapture は 1 行目が画面の上
    let flip_y = match get(&header, "ROWORDER") {
        None => false,
        Some(v) => py_str(v).to_uppercase().starts_with("TOP"),
    };
    Ok(FitsImage { data, ny, nx, header, notes, flip_y })
}

/// Python の `str(value)`(真偽値は True/False)。
pub(crate) fn py_str(v: &CardValue) -> String {
    match v {
        CardValue::Bool(true) => "True".to_string(),
        CardValue::Bool(false) => "False".to_string(),
        CardValue::Int(i) => i.to_string(),
        CardValue::Float(f) => format!("{f:?}"),
        CardValue::Str(s) => s.clone(),
        CardValue::None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(s: &str) -> Vec<u8> {
        let mut b = s.as_bytes().to_vec();
        b.resize(80, b' ');
        b
    }

    #[test]
    fn values() {
        let c = parse_card(&card("EXPTIME =               1.5D+1 / D exponent"));
        assert_eq!(c.value, CardValue::Float(15.0));
        assert_eq!(c.comment, "D exponent");
        let c = parse_card(&card("LOWER   = 'abc     '           / string with trailing blanks"));
        assert_eq!(c.value, CardValue::Str("abc".into()));
        let c = parse_card(&card("QUOTE   = 'it''s'"));
        assert_eq!(c.value, CardValue::Str("it's".into()));
        let c = parse_card(&card("UNDEF   ="));
        assert_eq!(c.value, CardValue::None);
        let c = parse_card(&card("NEGINT  =                  -42"));
        assert_eq!(c.value, CardValue::Int(-42));
        let c = parse_card(&card("BOOLF   =                    F"));
        assert_eq!(c.value, CardValue::Bool(false));
        let c = parse_card(&card("HIERARCH ESO DET CHIP NAME = 'chip-1'"));
        assert_eq!(c.key, "ESO DET CHIP NAME");
        assert_eq!(c.value, CardValue::Str("chip-1".into()));
        let c = parse_card(&card("NAXIS1  =                  abc"));
        assert_eq!(c.value, CardValue::None);
        let c = parse_card(&card("FLOATD  =                0.0015 / comment / with slash"));
        assert_eq!(c.value, CardValue::Float(0.0015));
        assert_eq!(c.comment, "comment / with slash");
    }

    /// 以前の読み方(画素ごとに `&dyn Fn`、呼ぶたびに作業領域を作る)。read_conv を静的な呼び出しと
    /// 型ごとの専用の閉包にしたので、値がビット単位で同じかをこれと比べる。
    fn old_read_conv<T: Copy + Send + Default>(
        a: &mut FileArray,
        start: usize,
        out: &mut [T],
        conv: &(dyn Fn(&[u8]) -> T + Sync),
    ) -> Result<(), String> {
        let bpp = a.bpp();
        let total: usize = a.shape.iter().product();
        if start.checked_add(out.len()).is_none_or(|e| e > total) {
            return Err("配列の範囲外を読もうとしました".into());
        }
        let mut buf = vec![0u8; out.len().min(READ_CHUNK) * bpp];
        for (ci, chunk) in out.chunks_mut(READ_CHUNK).enumerate() {
            let off = a.start + ((start + ci * READ_CHUNK) as u64) * bpp as u64;
            let b = &mut buf[..chunk.len() * bpp];
            a.src.read_at(off, b)?;
            chunk.par_chunks_mut(CONV_CHUNK).zip(b.par_chunks(CONV_CHUNK * bpp)).for_each(|(o, raw)| {
                for (v, r) in o.iter_mut().zip(raw.chunks_exact(bpp)) {
                    *v = conv(r);
                }
            });
        }
        Ok(())
    }

    fn old_read_f32(a: &mut FileArray, start: usize, out: &mut [f32]) -> Result<(), String> {
        let (bp, sc) = (a.bitpix, a.sc.clone());
        if bp > 0 {
            old_read_conv(a, start, out, &|r| sc.int_f32(be_int(r, bp)))
        } else {
            old_read_conv(a, start, out, &|r| sc.float_f32(be_float(r, bp)))
        }
    }

    fn old_read_f64(a: &mut FileArray, start: usize, out: &mut [f64]) -> Result<(), String> {
        let (bp, sc) = (a.bitpix, a.sc.clone());
        if bp > 0 {
            old_read_conv(a, start, out, &|r| sc.int_f64(be_int(r, bp)))
        } else {
            old_read_conv(a, start, out, &|r| sc.float_f64(be_float(r, bp)))
        }
    }

    /// 決まった擬似乱数のバイト(xorshift64*)。浮動小数として読むと NaN(静か・信号)・±inf・±0・非正規数も混ざる
    fn rand_bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                (x.wrapping_mul(0x2545F4914F6CDD1D) >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn read_conv_same_as_dyn() {
        // (BITPIX, BSCALE, BZERO, BLANK): Raw・PseudoUnsigned・Int8・ToF32・ToF64・Float32Scaled・Float64Scaled
        let cases: &[(i32, f64, f64, Option<i64>)] = &[
            (8, 1.0, 0.0, None),
            (8, 1.0, -128.0, None),
            (8, 2.5, 10.0, Some(7)),
            (16, 1.0, 0.0, None),
            (16, 1.0, 32768.0, None),
            (16, 0.5, -3.0, Some(-1)),
            (32, 1.0, 0.0, None),
            (32, 1.0, 2147483648.0, None),
            (32, 1.5, 100.0, Some(12345)),
            (64, 1.0, 0.0, None),
            (64, 1.0, 9223372036854775808.0, None),
            (64, 3.0, -1.0, None),
            (-32, 1.0, 0.0, None),
            (-32, 2.0, 1.0, None),
            (-64, 1.0, 0.0, None),
            (-64, 0.25, 7.0, None),
        ];
        // 特別な値を必ず含める(先頭に置く)。float32: +0・-0・静かな NaN・信号の NaN・±inf・非正規数・最大
        let special32: [u32; 9] =
            [0, 0x8000_0000, 0x7fc0_0000, 0x7f80_0001, 0x7f80_0000, 0xff80_0000, 1, 0x7f7f_ffff, 0xffc0_0001];
        for (k, &(bp, bscale, bzero, blank)) in cases.iter().enumerate() {
            let bpp = (bp.unsigned_abs() / 8) as usize;
            // 区切り(READ_CHUNK)をまたぐ大きさと、半端な大きさ
            for &n in &[1000usize, READ_CHUNK + 777] {
                let mut bytes = rand_bytes(n * bpp, 0x9E37_79B9_7F4A_7C15 ^ (k as u64 * 1000 + n as u64));
                if bp == -32 {
                    for (i, v) in special32.iter().enumerate() {
                        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
                    }
                }
                let sc = Scaling::new(bp, bscale, bzero, blank, false);
                let mut src = Src::Mem(bytes);
                let mut a = FileArray { src: &mut src, start: 0, shape: vec![n], bitpix: bp, sc, scratch: Vec::new() };
                // 小さい読み → 大きい読み → 途中から(作業領域の使い回しと伸ばし直しを通す)
                for &(start, len) in &[(3usize, 50usize), (0, n), (n / 3, n - n / 3)] {
                    let mut new = vec![0.0f32; len];
                    let mut old = vec![0.0f32; len];
                    a.read_f32(start, &mut new).unwrap();
                    old_read_f32(&mut a, start, &mut old).unwrap();
                    for i in 0..len {
                        assert_eq!(
                            new[i].to_bits(),
                            old[i].to_bits(),
                            "f32 BITPIX {bp} BSCALE {bscale} BZERO {bzero} 添字 {} ({:08x} ≠ {:08x})",
                            start + i,
                            new[i].to_bits(),
                            old[i].to_bits()
                        );
                    }
                    let mut new = vec![0.0f64; len];
                    let mut old = vec![0.0f64; len];
                    a.read_f64(start, &mut new).unwrap();
                    old_read_f64(&mut a, start, &mut old).unwrap();
                    for i in 0..len {
                        assert_eq!(new[i].to_bits(), old[i].to_bits(), "f64 BITPIX {bp} 添字 {}", start + i);
                    }
                }
                // 範囲外は今と同じ Err
                let mut o = vec![0.0f32; 2];
                assert_eq!(a.read_f32(n - 1, &mut o), old_read_f32(&mut a, n - 1, &mut o));
                assert!(a.read_f32(n - 1, &mut o).is_err());
            }
        }
    }

    #[test]
    fn continue_cards() {
        let raw = vec![
            parse_card(&card("LONGSTR = 'xxxx&'")),
            parse_card(&card("CONTINUE  'yy&'")),
            parse_card(&card("CONTINUE  ' end'")),
            parse_card(&card("END")),
        ];
        let cards = cards_from_raw(raw);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].value, CardValue::Str("xxxxyy end".into()));
    }
}

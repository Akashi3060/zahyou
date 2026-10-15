//! 索引ファイルのヘッダーの検査(Python 版 `_sv_index_layout` / `sv_validate_index_file`)。
//!
//! データは読まない(ヘッダーとファイルの大きさだけ)。問題があれば Python 版と同じ文面のエラー。
//! Python 版に無い検査(Rust で安全に読むためのもの)は、Python 版の検査が全部通った後にだけ行う
//! (壊れ方が同じなら、出るメッセージも Python 版と同じになるように)。

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use crate::fits::{read_hdus, BlockSource, FitsValue, Hdu, Header, ReadError};
use crate::pyfmt::{commas, float_repr, str_repr};

/// 全天の索引を表す HEALPIX の値。
pub const ALLSKY_HEALPIX: i32 = -1;

/// u32 の 0x01020304 が 04 03 02 01 と並ぶ = リトルエンディアン。読めるのはこの値だけ。
pub const INDEX_ENDIAN: &str = "04:03:02:01";

/// kd-tree の整数の型(KDT_INT / KDT_DATA)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KdType {
    U16,
    U32,
}

impl KdType {
    fn parse(v: Option<&str>) -> Option<KdType> {
        match v {
            Some("u16") => Some(KdType::U16),
            Some("u32") => Some(KdType::U32),
            _ => None,
        }
    }

    /// 1 個のバイト数。
    pub fn bytes(self) -> u64 {
        match self {
            KdType::U16 => 2,
            KdType::U32 => 4,
        }
    }
}

/// kd-tree 1 本の表の位置(ファイルの先頭からのバイト数)と形。
#[derive(Clone, Debug, PartialEq)]
pub struct TreeLayout {
    pub ndim: usize,
    pub ndata: usize,
    pub nnodes: usize,
    pub int_type: KdType,
    pub data_type: KdType,
    pub lr: u64,
    pub split: u64,
    pub range: u64,
    pub data: u64,
}

/// 等級の表(4100 系だけ。4 列の float32、ビッグエンディアン)。列 MAG の位置。
#[derive(Clone, Debug, PartialEq)]
pub struct MagLayout {
    pub offset: u64,
    pub row_bytes: u64,
    pub col_offset: u64,
}

/// 検査済みのヘッダーの要約。
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub indexid: u32,
    pub healpix: i32,
    pub hpnside: i64,
    pub dimquads: usize,
    pub nquads: usize,
    pub nstars: usize,
    pub scale_l: f64,
    pub scale_u: f64,
    pub size: u64,
    pub quads: u64,
    pub sweep: u64,
    pub codes: TreeLayout,
    pub stars: TreeLayout,
    /// 星の木のヘッダーの JITTER(無ければ None)と CUTNSIDE
    pub jitter: Option<f64>,
    pub cutnside: Option<u32>,
    pub mag: Option<MagLayout>,
}

/// ファイル名の形 `index-NNNN.fits` / `index-NNNN-TT.fits`(大小文字を問わない)。
/// 一致すれば (系列の 4 桁, タイルの 2 桁) の文字列を返す。
pub fn match_index_name(name: &str) -> Option<(&str, Option<&str>)> {
    let b = name.as_bytes();
    if b.len() < 15 || !b[..6].eq_ignore_ascii_case(b"index-") {
        return None;
    }
    let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
    if !digits(&b[6..10]) {
        return None;
    }
    let rest = &b[10..];
    if rest.eq_ignore_ascii_case(b".fits") {
        return Some((&name[6..10], None));
    }
    if rest.len() == 8 && rest[0] == b'-' && digits(&rest[1..3]) && rest[3..].eq_ignore_ascii_case(b".fits") {
        return Some((&name[6..10], Some(&name[11..13])));
    }
    None
}

/// 検査の失敗。Io は OS のエラー、Bad は形式の問題(文面は Python 版と同じ)。
#[derive(Debug)]
pub enum LayoutError {
    Io(std::io::Error),
    Bad(String),
}

impl LayoutError {
    /// 画面に出す文。
    pub fn message(&self) -> String {
        match self {
            LayoutError::Io(e) => format!("読めません({e})"),
            LayoutError::Bad(s) => s.clone(),
        }
    }
}

fn bad<T>(s: String) -> Result<T, LayoutError> {
    Err(LayoutError::Bad(s))
}

/// ファイルを開いてヘッダーを検査する。name は名前の検査に使うファイル名(None ならパスの名前)。
pub fn read_layout(path: &Path, name: Option<&str>) -> Result<Layout, LayoutError> {
    let fname = name_for_check(path, name);
    // 名前の検査はファイルを開く前(Python 版と同じ順)
    if match_index_name(&fname).is_none() {
        return bad("ファイル名が index-NNNN.fits / index-NNNN-TT.fits の形ではありません".to_string());
    }
    let mut f = File::open(path).map_err(LayoutError::Io)?;
    let size = f.metadata().map_err(LayoutError::Io)?.len();
    layout_from_source(&mut f, size, &fname)
}

/// 名前の検査に使うファイル名(パスの最後の要素)。
pub(crate) fn name_for_check(path: &Path, name: Option<&str>) -> String {
    let p = match name {
        Some(n) => Path::new(n).to_path_buf(),
        None => path.to_path_buf(),
    };
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// ブロックの読み出し元(ファイルか mmap したバイト列)からヘッダーを検査する。
pub(crate) fn layout_from_source<S: BlockSource>(src: &mut S, size: u64, fname: &str) -> Result<Layout, LayoutError> {
    let Some((g_id, g_tile)) = match_index_name(fname) else {
        return bad("ファイル名が index-NNNN.fits / index-NNNN-TT.fits の形ではありません".to_string());
    };
    let (hdus, end, size) = match read_hdus(src, size) {
        Ok(v) => v,
        Err(ReadError::Io(e)) => return Err(LayoutError::Io(e)),
        Err(ReadError::Bad(s)) => return bad(s),
    };
    if end != size {
        if size < end {
            return bad(format!(
                "ファイルが途中で切れています(大きさ {} バイト、ヘッダーから計算した長さ {} バイト。ダウンロードし直してください)",
                commas(size as i128),
                commas(end as i128)
            ));
        }
        return bad(format!(
            "大きさがヘッダーから計算した長さと合いません({} バイト、計算 {} バイト)",
            commas(size as i128),
            commas(end as i128)
        ));
    }

    let h0 = &hdus[0].header;
    for key in ["INDEXID", "HEALPIX", "HPNSIDE", "DIMQUADS", "NQUADS", "NSTARS", "SCALE_L", "SCALE_U"] {
        if !h0.contains(key) {
            return bad(format!("索引のヘッダーに {key} がありません"));
        }
    }
    for (i, h) in hdus.iter().enumerate() {
        // 値の空のカードは Python 版では None = 無いのと同じ
        let e = h.header.get("ENDIAN").filter(|v| **v != FitsValue::Null);
        if i == 0 && e.is_none() {
            return bad("索引のヘッダーに ENDIAN がありません".to_string());
        }
        if let Some(v) = e
            && h.header.str("ENDIAN") != Some(INDEX_ENDIAN)
        {
            return bad(format!(
                "ENDIAN = {} の索引は読めません(読めるのは {} だけ)",
                v.py_repr(),
                str_repr(INDEX_ENDIAN)
            ));
        }
    }

    let indexid = h0.int("INDEXID", None).map_err(LayoutError::Bad)?;
    let healpix = h0.int("HEALPIX", None).map_err(LayoutError::Bad)?;
    let hpnside = h0.int("HPNSIDE", None).map_err(LayoutError::Bad)?;
    let dq = h0.int("DIMQUADS", None).map_err(LayoutError::Bad)?;
    let nq = h0.int("NQUADS", None).map_err(LayoutError::Bad)?;
    let ns = h0.int("NSTARS", None).map_err(LayoutError::Bad)?;
    let (sl, su) = match (
        h0.get("SCALE_L").and_then(|v| v.py_float()),
        h0.get("SCALE_U").and_then(|v| v.py_float()),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return bad("SCALE_L / SCALE_U が数ではありません".to_string()),
    };
    let name_id: i128 = g_id.parse().unwrap_or(-1);
    if indexid != name_id {
        return bad(format!("名前と中身の系列が違います(名前 {g_id}、INDEXID = {indexid})"));
    }
    match g_tile {
        None => {
            if healpix != -1 {
                return bad(format!("全天の名前なのにタイルの索引です(HEALPIX = {healpix})"));
            }
        }
        Some(t) => {
            let name_tile: i128 = t.parse().unwrap_or(-1);
            if healpix != name_tile {
                return bad(format!("名前と中身のタイル番号が違います(名前 {t}、HEALPIX = {healpix})"));
            }
            let npix = hpnside.checked_mul(hpnside).and_then(|v| v.checked_mul(12));
            if hpnside < 1 || npix.is_some_and(|n| healpix >= n) {
                return bad(format!(
                    "タイル番号が HEALPix の範囲の外です(HEALPIX = {healpix}、HPNSIDE = {hpnside})"
                ));
            }
        }
    }
    if dq != 3 && dq != 4 {
        return bad(format!("DIMQUADS = {dq} は扱えません(3 か 4)"));
    }
    if nq < 1 || ns < dq {
        return bad(format!("quad か星がありません(NQUADS = {nq}、NSTARS = {ns})"));
    }
    if !(0.0 < sl && sl < su && su < std::f64::consts::PI) {
        return bad(format!(
            "quad の大きさの範囲がおかしい(SCALE_L = {}、SCALE_U = {})",
            float_repr(sl),
            float_repr(su)
        ));
    }

    // 表を TTYPE1 の名前で探す(同じ名前は最初のもの)
    let mut tables: HashMap<String, &Hdu> = HashMap::new();
    for h in &hdus[1..] {
        if let Some(t) = h.header.str("TTYPE1") {
            tables.entry(t.to_string()).or_insert(h);
        }
    }
    let table = |t: &str| -> Result<&Hdu, LayoutError> {
        match tables.get(t) {
            Some(h) => Ok(*h),
            None => bad(format!("表 {t} がありません")),
        }
    };
    let need_bytes = |t: &str, nbytes: i128| -> Result<u64, LayoutError> {
        let h = table(t)?;
        if h.size as i128 != nbytes {
            return bad(format!(
                "表 {t} の大きさが合いません({} バイト、期待 {} バイト)",
                commas(h.size as i128),
                commas(nbytes)
            ));
        }
        Ok(h.offset)
    };
    // i128 で計算する(ヘッダーの値が大きすぎても溢れない範囲)
    let mul = |a: i128, b: i128| a.saturating_mul(b);

    let quads = need_bytes("quads", mul(4 * dq, nq))?;
    let sweep = need_bytes("sweep", ns)?;

    let mut trees: Vec<(TreeLayout, Header)> = Vec::with_capacity(2);
    for (tname, ndim, ndata) in [("codes", 2 * (dq - 2), nq), ("stars", 3, ns)] {
        let kh = &table(&format!("kdtree_header_{tname}"))?.header;
        if kh.str("KDT_NAME") != Some(tname) {
            return bad(format!("kdtree_header_{tname} の KDT_NAME が {} です", kh.repr("KDT_NAME")));
        }
        let d = kh.int("KDT_NDIM", None).map_err(LayoutError::Bad)?;
        let n = kh.int("KDT_NDAT", None).map_err(LayoutError::Bad)?;
        let nn = kh.int("KDT_NNOD", None).map_err(LayoutError::Bad)?;
        if d != ndim || n != ndata {
            return bad(format!("{tname} の木の次元か点の数が合いません(KDT_NDIM = {d}、KDT_NDAT = {n})"));
        }
        if nn < 1 || ((nn + 1) & nn) != 0 {
            return bad(format!("{tname} の木の節点の数 {nn} が 2^L − 1 ではありません"));
        }
        if kh.str("KDT_EXT") != Some("double") {
            return bad(format!("{tname} の木の KDT_EXT = {} は扱えません", kh.repr("KDT_EXT")));
        }
        let (ti, td) = match (KdType::parse(kh.str("KDT_INT")), KdType::parse(kh.str("KDT_DATA"))) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                return bad(format!(
                    "{tname} の木の整数の型 {} / {} は扱えません",
                    kh.repr("KDT_INT"),
                    kh.repr("KDT_DATA")
                ));
            }
        };
        let lr = need_bytes(&format!("kdtree_lr_{tname}"), mul(4, (nn + 1) / 2))?;
        let split = need_bytes(&format!("kdtree_split_{tname}"), mul(ti.bytes() as i128, (nn - 1) / 2))?;
        let range = need_bytes(&format!("kdtree_range_{tname}"), mul(8, 2 * d + 1))?;
        let data = need_bytes(&format!("kdtree_data_{tname}"), mul(mul(td.bytes() as i128, n), d))?;
        // ここまで来れば d・n・nn は表の大きさ(= ファイルの中)と合っているので usize に入る
        let to_usize = |v: i128| usize::try_from(v).unwrap_or(usize::MAX);
        trees.push((
            TreeLayout {
                ndim: to_usize(d),
                ndata: to_usize(n),
                nnodes: to_usize(nn),
                int_type: ti,
                data_type: td,
                lr,
                split,
                range,
                data,
            },
            kh.clone(),
        ));
    }
    let (stars, stars_hdr) = trees.pop().ok_or_else(|| LayoutError::Bad("表 stars がありません".into()))?;
    let (codes, _) = trees.pop().ok_or_else(|| LayoutError::Bad("表 codes がありません".into()))?;

    // ---- ここから Python 版に無い検査(番号を u32 で持つため)----
    if nq > u32::MAX as i128 || ns > u32::MAX as i128 {
        return bad(format!(
            "quad か星が多すぎます(NQUADS = {nq}、NSTARS = {ns}。扱えるのは {} 個まで)",
            commas(u32::MAX as i128)
        ));
    }

    let jitter = match stars_hdr.get("JITTER") {
        Some(FitsValue::Int(i)) => Some(*i as f64),
        Some(FitsValue::Float(x)) => Some(*x),
        _ => None,
    };
    let cutnside = match stars_hdr.get("CUTNSIDE") {
        Some(FitsValue::Int(i)) if *i > 0 => u32::try_from(*i).ok(),
        _ => None,
    };
    let mag = find_mag(&tables, ns);

    Ok(Layout {
        indexid: indexid as u32, // 名前の 4 桁と同じ = 0〜9999
        healpix: healpix as i32, // -1 か名前の 2 桁
        hpnside: hpnside.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
        dimquads: dq as usize,
        nquads: nq as usize,
        nstars: ns as usize,
        scale_l: sl,
        scale_u: su,
        size,
        quads,
        sweep,
        codes,
        stars,
        jitter,
        cutnside,
        mag,
    })
}

/// 等級の表(TTYPE1 = MAG_BT、4 列目 MAG、どれも 1E)。形が合わなければ無いものとして扱う。
fn find_mag(tables: &HashMap<String, &Hdu>, ns: i128) -> Option<MagLayout> {
    let h = tables.get("MAG_BT")?;
    let hd = &h.header;
    let row = hd.int("NAXIS1", None).ok()?;
    let rows = hd.int("NAXIS2", None).ok()?;
    if rows != ns || row != 16 || h.size as i128 != 16 * ns {
        return None;
    }
    for k in 1..=4 {
        if hd.str(&format!("TFORM{k}")).map(|s| s.trim()) != Some("1E") {
            return None;
        }
    }
    if hd.str("TTYPE4") != Some("MAG") {
        return None;
    }
    Some(MagLayout { offset: h.offset, row_bytes: 16, col_offset: 12 })
}

/// 検査に通ったときの要約(Python 版 `sv_validate_index_file` の ok のときの文)。
pub fn summary(lay: &Layout) -> String {
    let lo = lay.scale_l.to_degrees() * 60.0;
    let hi = lay.scale_u.to_degrees() * 60.0;
    let place = if lay.healpix < 0 { "全天".to_string() } else { format!("タイル {}", lay.healpix) };
    let kind = if lay.dimquads == 3 { "三角形" } else { "4 星" };
    format!(
        "{place}、{kind}の quad {} 個・星 {} 個、quad の大きさ {lo:.1}′〜{hi:.1}′",
        commas(lay.nquads as i128),
        commas(lay.nstars as i128)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_names() {
        assert_eq!(match_index_name("index-4107.fits"), Some(("4107", None)));
        assert_eq!(match_index_name("INDEX-5206-02.FITS"), Some(("5206", Some("02"))));
        assert_eq!(match_index_name("index-4107.fits.part"), None);
        assert_eq!(match_index_name("index-4119-copy.fits"), None);
        assert_eq!(match_index_name("index-410.fits"), None);
        assert_eq!(match_index_name("index-5206-2.fits"), None);
        assert_eq!(match_index_name("index-5206-002.fits"), None);
        assert_eq!(match_index_name("index-５２０６.fits"), None);
    }
}

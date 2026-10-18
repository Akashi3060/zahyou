//! 画素スケールの推定と機材の記憶、星図データの段と画角の対応。
//!
//! Python 版 `dev/zahyou_core.py` の `_first_float`・`rig_key`・`remembered_scale`・`remember_scale`・
//! `estimate_pixel_scale`・`implied_focal_length`・`index_coverage`・`blind_scale_bounds`・`missing_index_advice` と、
//! `dev/nb_main.py` の `_existing_wcs_header` を移したもの。文言と判定の順は Python 版と同じ。
//!
//! ヘッダーは `zahyou_wcs::Card` の並び(`zahyou_image::Loaded::header`)。値の読み方は Python(astropy)の
//! `header.get(key)`・`float(header[key])`・`str(...)`・`or` の真偽に合わせる。
//!
//! Python 版との違い(意図したもの)
//! - 値の無いカード(astropy の `Undefined`)は、文字列の候補としては「無い」扱いにする。Python では
//!   `str(Undefined)` がオブジェクトの番地を含む文字列になり、機材の鍵が実行のたびに変わってしまう。
//! - 画像に入っている WCS は `zahyou_wcs::Wcs::from_header_cards` で読む。読めるのは TAN(と SIP)だけで、
//!   ほかの投影(SIN など)は「WCS なし」として次の手がかり(CDELT1 など)へ進む。astropy は SIN なども読む。
#![forbid(unsafe_code)]

use std::path::Path;

use zahyou_wcs::{Card, CardValue, Wcs};

/// 画素サイズ [µm] のキー(上から順に探す)。
pub const PIXEL_SIZE_KEYS: [&str; 5] = ["XPIXSZ", "PIXSIZE1", "XPIXELSZ", "PIXSZ", "PIXSIZEX"];
/// 画素スケールのキー(CDELT1 だけ度なので 3600 倍する)。
pub const SCALE_KEYS: [&str; 6] = ["SECPIX", "SECPIX1", "PIXSCALE", "SCALE", "PLTSCALE", "CDELT1"];
/// 焦点距離 [mm] のキー。
pub const FOCAL_KEYS: [&str; 3] = ["FOCALLEN", "FOCAL", "FOCALLENGTH"];
/// 1 µm / 1 mm を秒角に(206264.806″ / 1000)。Python 版と同じ桁の定数。
pub const ARCSEC_PER_UM_PER_MM: f64 = 206.264806;
/// 画素スケールとして受け付ける範囲 [″/px](両端は含まない)。
pub const SCALE_MIN: f64 = 0.01;
pub const SCALE_MAX: f64 = 3600.0;

/// 星図データの段(番号の末尾 2 桁)→ 画角 [分角] の範囲。Python の `_INDEX_SCALES`。
pub const INDEX_SCALES: [(u32, f64, f64); 20] = [
    (0, 2.0, 2.8),
    (1, 2.8, 4.0),
    (2, 4.0, 5.6),
    (3, 5.6, 8.0),
    (4, 8.0, 11.0),
    (5, 11.0, 16.0),
    (6, 16.0, 22.0),
    (7, 22.0, 30.0),
    (8, 30.0, 42.0),
    (9, 42.0, 60.0),
    (10, 60.0, 85.0),
    (11, 85.0, 120.0),
    (12, 120.0, 170.0),
    (13, 170.0, 240.0),
    (14, 240.0, 340.0),
    (15, 340.0, 480.0),
    (16, 480.0, 680.0),
    (17, 680.0, 1000.0),
    (18, 1000.0, 1400.0),
    (19, 1400.0, 2000.0),
];

fn in_scale_range(s: f64) -> bool {
    SCALE_MIN < s && s < SCALE_MAX
}

// ------------------------------------------------------------------ ヘッダーの値(Python の読み方) ---

/// `header.get(key)`: 最初に一致したカードの値。COMMENT・HISTORY のような値の無いキーは探さない想定。
pub fn header_get<'a>(header: &'a [Card], key: &str) -> Option<&'a CardValue> {
    header.iter().find(|c| c.key == key).map(|c| &c.value)
}

/// Python の真偽(`or` で次へ進むか)。値の無いカードは偽とする(モジュールの説明を参照)。
pub fn py_truthy(v: &CardValue) -> bool {
    match v {
        CardValue::Bool(b) => *b,
        CardValue::Int(i) => *i != 0,
        // NaN は Python では真
        CardValue::Float(f) => *f != 0.0,
        CardValue::Str(s) => !s.is_empty(),
        CardValue::None => false,
    }
}

/// Python の `str(value)`(真偽は True/False、実数は repr)。
pub fn py_str(v: &CardValue) -> String {
    match v {
        CardValue::Bool(true) => "True".to_string(),
        CardValue::Bool(false) => "False".to_string(),
        CardValue::Int(i) => i.to_string(),
        CardValue::Float(f) => zahyou_net::pyfmt::py_float_repr(*f),
        CardValue::Str(s) => s.clone(),
        CardValue::None => String::new(),
    }
}

/// Python の `float(value)`。数にならなければ None(Python の TypeError・ValueError)。
pub fn py_float(v: &CardValue) -> Option<f64> {
    match v {
        CardValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        CardValue::Int(i) => Some(*i as f64),
        CardValue::Float(f) => Some(*f),
        CardValue::Str(s) => parse_py_float(s),
        CardValue::None => None,
    }
}

/// Python の `float(str)` に近い読み方(前後の空白を許し、inf・nan も読む。桁区切りの `_` は読まない)。
pub fn parse_py_float(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || !t.is_ascii() {
        return None;
    }
    // Rust は "1e" や "e5" を拒む(Python も同じ)。"infinity"・"nan" は大文字小文字を問わない(Python も同じ)
    t.parse::<f64>().ok()
}

/// Python の `_first_float`: keys の順に、数として読めて有限かつ 0 でない最初の値とそのキー。
pub fn first_float(header: &[Card], keys: &[&'static str]) -> Option<(f64, &'static str)> {
    for &k in keys {
        if let Some(v) = header_get(header, k) {
            let Some(x) = py_float(v) else { continue };
            if x.is_finite() && x != 0.0 {
                return Some((x, k));
            }
        }
    }
    None
}

/// `a or b or c ...` のうち最初の真の値(どれも偽なら None)。
fn first_truthy<'a>(header: &'a [Card], keys: &[&str]) -> Option<&'a CardValue> {
    keys.iter().filter_map(|k| header_get(header, k)).find(|v| py_truthy(v))
}

// ------------------------------------------------------------------ 機材の記憶 ---

/// 「同じ機材・同じ画像の大きさ」を表す鍵(Python の `rig_key`)。
/// 形は `"{カメラ}|{望遠鏡}|{幅}x{高さ}|bin{ビニング}|f{焦点距離}"`。
pub fn rig_key(header: &[Card], width: usize, height: usize) -> String {
    let cam = first_truthy(header, &["INSTRUME", "CAMID", "CAMERA"]).map(py_str).unwrap_or_else(|| "?".to_string());
    let bin = first_truthy(header, &["XBINNING", "BINX"]).map(py_str).unwrap_or_else(|| "1".to_string());
    let tel = first_truthy(header, &["TELESCOP"]).map(py_str).unwrap_or_default();
    let focal = first_truthy(header, &["FOCALLEN"]).map(py_str).unwrap_or_default();
    // Python は cam と tel だけ strip する(zahyou_net::rig_key が strip する)
    zahyou_net::rig_key(py_strip(&cam), py_strip(&tel), width, height, &bin, &focal)
}

/// Python の `str.strip()`(Unicode の空白を前後から外す)。
fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// 前に同じ機材で解いたときの実測スケール [″/px](`cache_file` の cache.json の rigs)。
pub fn remembered_scale(header: &[Card], width: usize, height: usize, cache_file: &Path) -> Option<f64> {
    zahyou_net::cache::remembered_scale_at(cache_file, &rig_key(header, width, height))
}

/// 解けたので、この機材の画素スケールを覚える(0.01〜3600″/px の外は覚えない)。
pub fn remember_scale(
    header: &[Card],
    width: usize,
    height: usize,
    scale_arcsec: f64,
    cache_file: &Path,
) -> Result<(), String> {
    if !(scale_arcsec != 0.0 && in_scale_range(scale_arcsec)) {
        return Ok(());
    }
    zahyou_net::cache::remember_scale_at(cache_file, &rig_key(header, width, height), scale_arcsec)
}

// ------------------------------------------------------------------ 画素スケールの推定 ---

/// WCS の画素スケール [度](astropy の `proj_plane_pixel_scales`: CD の列ごとの長さ)。
pub fn proj_plane_pixel_scales(wcs: &Wcs) -> [f64; 2] {
    let c = &wcs.cd;
    let col = |j: usize| (c[0][j] * c[0][j] + c[1][j] * c[1][j]).sqrt();
    [col(0), col(1)]
}

/// `float(np.mean(proj_plane_pixel_scales(w)) * 3600.0)`(平均してから秒角にする。estimate_pixel_scale・_sanity_ok の順)。
pub fn wcs_scale_arcsec(wcs: &Wcs) -> f64 {
    let s = proj_plane_pixel_scales(wcs);
    (s[0] + s[1]) / 2.0 * 3600.0
}

/// 画像のヘッダーにある天球の WCS(astropy の `WCS(header).is_celestial`)。読めなければ None。
pub fn header_wcs(header: &[Card]) -> Option<Wcs> {
    if header.is_empty() {
        return None;
    }
    Wcs::from_header_cards(header).ok()
}

/// 画像に元から入っている WCS が使えるなら返す(Python の `_existing_wcs_header`。スケールが 0.01〜3600″/px のものだけ)。
pub fn existing_wcs(header: &[Card]) -> Option<Wcs> {
    let w = header_wcs(header)?;
    let s = wcs_scale_arcsec(&w);
    in_scale_range(s).then_some(w)
}

/// 1 画素の秒角を推定する(Python の `estimate_pixel_scale`)。戻り値 (″/px, 説明)。
///
/// 上ほど信用できる順:
///   1. 使う人が焦点距離を入れた(× ヘッダーの画素サイズ)
///   2. この画像自身が持っている WCS
///   3. ヘッダーの画素スケールのキー
///   4. ヘッダーの焦点距離 × 画素サイズ
///   5. 前に同じ機材で解いたときの実測値(`size` = (幅, 高さ) を渡したときだけ。`cache_file` の cache.json)
///
/// 焦点距離を入れても、ヘッダーに画素サイズが無ければ使わない(PNG・JPEG・TIFF はいつもこれ。Python 版と同じ)。
pub fn estimate_pixel_scale(
    header: &[Card],
    focal_length_mm: Option<f64>,
    size: Option<(usize, usize)>,
    cache_file: &Path,
) -> (Option<f64>, String) {
    // Python の `if focal_length_mm:`(0 と None は偽、NaN は真)
    if let Some(f) = focal_length_mm.filter(|f| *f != 0.0)
        && let Some((pix, pk)) = first_float(header, &PIXEL_SIZE_KEYS)
    {
        let s = ARCSEC_PER_UM_PER_MM * pix / f;
        if in_scale_range(s) {
            let why =
                format!("{pk}={}um と FOCAL_LENGTH_MM={}mm から", zahyou_net::pyfmt::py_float_repr(pix), fmt_g(f));
            return (Some(s), why);
        }
    }

    if let Some(w) = header_wcs(header) {
        let s = wcs_scale_arcsec(&w);
        if in_scale_range(s) {
            return (Some(s), "画像に既にあった WCS から".to_string());
        }
    }

    if let Some((v, k)) = first_float(header, &SCALE_KEYS) {
        let s = v.abs() * if k == "CDELT1" { 3600.0 } else { 1.0 };
        if in_scale_range(s) {
            return (Some(s), format!("ヘッダ {k} から"));
        }
    }

    if let (Some((pix, pk)), Some((focal, fk))) =
        (first_float(header, &PIXEL_SIZE_KEYS), first_float(header, &FOCAL_KEYS))
    {
        let s = ARCSEC_PER_UM_PER_MM * pix / focal;
        if in_scale_range(s) {
            let why = format!("{pk}={}um, {fk}={}mm から", zahyou_net::pyfmt::py_float_repr(pix), fmt_g(focal));
            return (Some(s), why);
        }
    }

    if let Some((w, h)) = size {
        // Python は `if s:`(0 でなければ採る。範囲は見ない)
        if let Some(s) = remembered_scale(header, w, h, cache_file).filter(|s| *s != 0.0) {
            return (Some(s), "前回この機材で解いたときの実測値".to_string());
        }
    }

    (None, "手がかりなし (星図データが対応する画角を総当たりします)".to_string())
}

/// 解けたスケールから焦点距離 [mm] を逆算する(ヘッダーに画素サイズがあるときだけ。Python の `implied_focal_length`)。
pub fn implied_focal_length(header: &[Card], scale_arcsec: f64) -> Option<f64> {
    let (pix, _) = first_float(header, &PIXEL_SIZE_KEYS)?;
    if scale_arcsec == 0.0 {
        return None;
    }
    Some(ARCSEC_PER_UM_PER_MM * pix / scale_arcsec)
}

// ------------------------------------------------------------------ 星図データの段と画角 ---

/// `index-XXXX[-YY].fits` の段(番号の末尾 2 桁)。読めなければ None(Python の index_coverage の読み方)。
pub fn index_scale_of_name(name: &str) -> Option<u32> {
    let core = Path::new(name).file_name()?.to_str()?;
    let rest = core.strip_prefix("index-")?;
    let num = rest.split('-').next()?.split('.').next()?;
    if num.len() < 2 || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    num[num.len() - 2..].parse().ok()
}

/// 段の画角 [分角]。
pub fn index_scale_fov(scale: u32) -> Option<(f64, f64)> {
    INDEX_SCALES.iter().find(|r| r.0 == scale).map(|r| (r.1, r.2))
}

/// 索引ファイルの名前の一覧から、扱える画角 [分角] の範囲(Python の `index_coverage`)。
pub fn index_coverage<S: AsRef<str>>(names: &[S]) -> Option<(f64, f64)> {
    let mut out: Option<(f64, f64)> = None;
    for n in names {
        let Some((lo, hi)) = index_scale_of_name(n.as_ref()).and_then(index_scale_fov) else { continue };
        out = Some(match out {
            None => (lo, hi),
            Some((a, b)) => (a.min(lo), b.max(hi)),
        });
    }
    out
}

/// 画角が全く分からないときに探す画素スケールの範囲 [″/px](Python の `blind_scale_bounds`。横幅で計算する)。
pub fn blind_scale_bounds<S: AsRef<str>>(width_px: usize, names: &[S]) -> (f64, f64) {
    let (lo_fov, hi_fov) = index_coverage(names).unwrap_or((2.0, 2000.0));
    let w = width_px as f64;
    let low = lo_fov * 60.0 / w * 0.9;
    let high = hi_fov * 60.0 / w * 1.1;
    (low.max(0.01), high.min(3600.0))
}

/// この画角を解くにはどの索引を足せばよいか(Python の `missing_index_advice`)。戻り値 (足りているか, 案内文)。
pub fn missing_index_advice<S: AsRef<str>>(fov_arcmin: f64, names: &[S]) -> (bool, Vec<String>) {
    let have: Vec<u32> = names.iter().filter_map(|n| index_scale_of_name(n.as_ref())).collect();
    let need: Vec<u32> = INDEX_SCALES.iter().filter(|r| r.1 <= fov_arcmin && fov_arcmin < r.2).map(|r| r.0).collect();
    let Some(&n0) = need.first() else {
        return (
            false,
            vec![format!(
                "画角 {fov_arcmin:.1}′ は星図データの範囲 (2′〜2000′) の外なので、この画角のままでは解けません。"
            )],
        );
    };
    if have.contains(&n0) {
        return (true, Vec::new());
    }
    let (lo, hi) = index_scale_fov(n0).unwrap_or((0.0, 0.0));
    let series = if n0 <= 6 { 5200 } else { 4100 } + n0;
    let files = if n0 <= 6 { format!("index-{series}-00〜47.fits") } else { format!("index-{series}.fits") };
    (
        false,
        vec![
            format!("画角 {fov_arcmin:.1}′ に対応する星図データ ({}′〜{}′ 段) がありません。", fmt_g(lo), fmt_g(hi)),
            format!("必要なファイル: {files}"),
            format!("取得: ［準備］タブの段の一覧で index-{series} に印を付け、「選んだ段を落とす」を押してください。"),
        ],
    )
}

// ------------------------------------------------------------------ 数の書き方 ---

/// Python の `format(x, "g")`(有効数字 6 桁、末尾の 0 と小数点を外す。指数は 2 桁以上)。
pub fn fmt_g(x: f64) -> String {
    fmt_g_prec(x, 6)
}

/// Python の `format(x, ".{p}g")`。
pub fn fmt_g_prec(x: f64, prec: usize) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    let p = prec.max(1);
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".to_string() } else { "0".to_string() };
    }
    // 有効数字 p 桁に丸めた値の指数(丸めの繰り上がりも含めて決める)
    let e_form = format!("{:.*e}", p - 1, x);
    let (mant, exp) = match e_form.split_once('e') {
        Some((m, e)) => (m.to_string(), e.parse::<i32>().unwrap_or(0)),
        None => (e_form.clone(), 0),
    };
    if exp < -4 || exp >= p as i32 {
        let m = strip_zeros(&mant);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{m}e{sign}{:02}", exp.abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        strip_zeros(&format!("{x:.decimals$}"))
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(k: &str, v: CardValue) -> Card {
        Card::new(k, v, "")
    }

    #[test]
    fn g_format() {
        assert_eq!(fmt_g(8.0), "8");
        assert_eq!(fmt_g(2.5), "2.5");
        assert_eq!(fmt_g(818.0), "818");
        assert_eq!(fmt_g(1e-5), "1e-05");
        assert_eq!(fmt_g(1234567.0), "1.23457e+06");
        assert_eq!(fmt_g(999999.5), "1e+06");
        assert_eq!(fmt_g(0.0001), "0.0001");
        assert_eq!(fmt_g(-0.0), "-0");
        assert_eq!(fmt_g(2.8), "2.8");
    }

    #[test]
    fn first_float_like_python() {
        let h = vec![
            card("XPIXSZ", CardValue::Str(" 5.8 ".into())),
            card("PIXSIZE1", CardValue::Float(3.0)),
            card("FOCALLEN", CardValue::Int(0)),
            card("FOCAL", CardValue::Bool(true)),
        ];
        assert_eq!(first_float(&h, &PIXEL_SIZE_KEYS), Some((5.8, "XPIXSZ")));
        // 0 は飛ばす、真偽は 1.0
        assert_eq!(first_float(&h, &FOCAL_KEYS), Some((1.0, "FOCAL")));
        let bad = vec![card("XPIXSZ", CardValue::Str("abc".into())), card("PIXSZ", CardValue::Float(f64::NAN))];
        assert_eq!(first_float(&bad, &PIXEL_SIZE_KEYS), None);
    }

    #[test]
    fn rig_key_like_python() {
        let h = vec![card("INSTRUME", CardValue::Str("ZWO ASI290MM".into())), card("XBINNING", CardValue::Int(2))];
        assert_eq!(rig_key(&h, 968, 548), "ZWO ASI290MM||968x548|bin2|f");
        assert_eq!(rig_key(&[], 2015, 1001), "?||2015x1001|bin1|f");
        let h2 = vec![
            card("INSTRUME", CardValue::Str(String::new())),
            card("CAMID", CardValue::Str("  cam ".into())),
            card("XBINNING", CardValue::Int(0)),
            card("BINX", CardValue::Float(2.0)),
            card("TELESCOP", CardValue::Str(" C11 ".into())),
            card("FOCALLEN", CardValue::Float(2800.0)),
        ];
        assert_eq!(rig_key(&h2, 10, 20), "cam|C11|10x20|bin2.0|f2800.0");
    }

    #[test]
    fn coverage_and_advice() {
        let names = ["index-4107.fits", "index-5205-02.fits", "index-4119.fits", "readme.txt", "index-x.fits"];
        assert_eq!(index_coverage(&names), Some((11.0, 2000.0)));
        let empty: [&str; 0] = [];
        assert_eq!(index_coverage(&empty), None);
        let (lo, hi) = blind_scale_bounds(968, &names);
        assert!((lo - 11.0 * 60.0 / 968.0 * 0.9).abs() < 1e-15 && (hi - 2000.0 * 60.0 / 968.0 * 1.1).abs() < 1e-12);
        assert_eq!(missing_index_advice(13.35, &names), (true, vec![]));
        let (ok, msgs) = missing_index_advice(9.0, &names);
        assert!(!ok);
        assert_eq!(msgs[0], "画角 9.0′ に対応する星図データ (8′〜11′ 段) がありません。");
        assert_eq!(msgs[1], "必要なファイル: index-5204-00〜47.fits");
        let (ok, msgs) = missing_index_advice(3000.0, &names);
        assert!(!ok && msgs.len() == 1);
    }
}

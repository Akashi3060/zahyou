//! WCS と FITS のヘッダーのカードの行き来。
//!
//! - 書く: Python 版 `SvWcs.to_header_cards` と同じキー・値・注記([`Wcs::header_cards`])。
//! - 読む: 画像に入っている WCS([`Wcs::from_header_cards`])。Python 版は astropy に読ませていたので、
//!   astropy(wcslib)と同じ空を指すように読む。扱えるのは赤道座標の TAN と TAN-SIP だけ。

use crate::card::{Card, CardValue, find_card};
use crate::geom::radec_to_xyz;
use crate::linalg::{Mat2, det2, mul2};
use crate::sip::{MAX_SIP_ORDER, Sip, poly_terms};
use crate::wcs::Wcs;

impl Wcs {
    /// FITS のヘッダーのカード(Python の `to_header_cards` と同じ中身・同じ順番)。
    ///
    /// CRPIX は 1 始まりに直して書く。LONPOLE = 180 を必ず書く(接点が北極ちょうどのとき、FITS の既定は 0 で、
    /// `tan_basis` の向きと 180° ずれるため。それ以外では既定値と同じ)。`comments` は COMMENT のカードにする
    /// (1 枚 70 文字以内に語の切れ目で分け、ASCII の印字できる文字以外は ? に替える。Python の textwrap.wrap と同じ分け方)。
    /// COMMENT のカードは値なし(`CardValue::None`)で、文は `comment` に入る。
    pub fn header_cards(&self, width: usize, height: usize, comments: &[String]) -> Vec<Card> {
        let (ra, dec) = self.crval_radec();
        let sip = self.sip.as_ref();
        let suffix = if sip.is_some() { "-SIP" } else { "" };
        let what = if sip.is_some() {
            "TAN (gnomonic) projection + SIP distortions"
        } else {
            "TAN (gnomonic) projection"
        };
        let f = |k: &str, v: f64, c: &str| Card::new(k, CardValue::Float(v), c);
        let s = |k: &str, v: &str, c: &str| Card::new(k, CardValue::Str(v.to_string()), c);
        let cd = &self.cd;
        let lin = "Linear transformation matrix [deg/px]";
        let mut cards = vec![
            s("CTYPE1", &format!("RA---TAN{suffix}"), what),
            s("CTYPE2", &format!("DEC--TAN{suffix}"), what),
            s("CUNIT1", "deg", "Unit of CRVAL1 and CD1_*"),
            s("CUNIT2", "deg", "Unit of CRVAL2 and CD2_*"),
            s("RADESYS", "ICRS", "Reference frame of the star catalogue"),
            f("LONPOLE", 180.0, "Native longitude of the celestial pole [deg]"),
            f("CRVAL1", ra, "RA  of the reference point [deg]"),
            f("CRVAL2", dec, "Dec of the reference point [deg]"),
            f("CRPIX1", self.crpix[0] + 1.0, "X of the reference point [px, 1-based]"),
            f("CRPIX2", self.crpix[1] + 1.0, "Y of the reference point [px, 1-based]"),
            f("CD1_1", cd[0][0], lin),
            f("CD1_2", cd[0][1], lin),
            f("CD2_1", cd[1][0], lin),
            f("CD2_2", cd[1][1], lin),
            Card::new("IMAGEW", CardValue::Int(width as i64), "Image width [px]"),
            Card::new("IMAGEH", CardValue::Int(height as i64), "Image height [px]"),
        ];
        if let Some(sp) = sip {
            let mut polys: Vec<(&str, &[f64], usize, usize, &str)> = vec![
                ("A", &sp.a, sp.order, 2, "forward, axis 1"),
                ("B", &sp.b, sp.order, 2, "forward, axis 2"),
            ];
            if sp.has_inverse() {
                polys.push(("AP", &sp.ap, sp.ap_order, 0, "inverse, axis 1"));
                polys.push(("BP", &sp.bp, sp.ap_order, 0, "inverse, axis 2"));
            }
            for (name, coef, n, lo, desc) in polys {
                // フィールドを直接書き換えた巨大な次数でも止まらないように(読み書きできる次数は MAX_SIP_ORDER まで)
                let n = n.min(MAX_SIP_ORDER);
                cards.push(Card::new(
                    &format!("{name}_ORDER"),
                    CardValue::Int(n as i64),
                    &format!("SIP polynomial order, {desc}"),
                ));
                for (p, q) in poly_terms(n, lo) {
                    let c = coef.get(p * (n + 1) + q).copied().unwrap_or(0.0);
                    cards.push(f(&format!("{name}_{p}_{q}"), c, ""));
                }
            }
        }
        for text in comments {
            let ascii: String = text.chars().map(|c| if (' '..='~').contains(&c) { c } else { '?' }).collect();
            let mut lines = text_wrap(&ascii, 70);
            if lines.is_empty() {
                lines.push(String::new());
            }
            for line in lines {
                cards.push(Card::commentary("COMMENT", &line));
            }
        }
        cards
    }

    /// 画像のヘッダーに入っている WCS を読む。
    ///
    /// - CTYPE は `RA---TAN` / `DEC--TAN`(後ろに `-SIP` が付いてよい)だけ。ほかの投影・座標系・軸の入れ替えは Err
    /// - CUNIT は無いか `deg` だけ。RADESYS・EQUINOX は見ない(Python 版も ICRS として扱っていた)
    /// - 線形変換は FITS の優先順: CDi_j(どれか 1 つでもあれば。無いものは 0)→ PCi_j と CDELTi → CDELTi と CROTA2 → CDELTi だけ
    /// - LONPOLE が 180 以外なら CD を回して同じ空を指すようにする(北極ちょうどの既定は 0)
    /// - SIP は A_ORDER・B_ORDER があれば読む(astropy 7.1 と同じく、CTYPE に `-SIP` が無くても掛ける。`-SIP` があって A_ORDER が無ければ TAN)。
    ///   AP_ORDER・BP_ORDER は任意。1 次以下の A・B の項が 0 でなければ Err(この WCS の形で表せないため)
    /// - CRVAL は必須、CRPIX は無ければ 0(FITS の既定)。CRPIX は 1 始まりとして読み、0 始まりに直す
    pub fn from_header_cards(cards: &[Card]) -> Result<Wcs, String> {
        let ct1 = ctype(cards, "CTYPE1")?;
        let ct2 = ctype(cards, "CTYPE2")?;
        let _sip1 = parse_ctype(&ct1, "RA", 1)?;
        let _sip2 = parse_ctype(&ct2, "DEC", 2)?;
        for key in ["CUNIT1", "CUNIT2"] {
            if let Some(c) = find_card(cards, key) {
                let unit = match &c.value {
                    CardValue::Str(s) => s.trim().to_ascii_lowercase(),
                    CardValue::None => String::new(),
                    _ => return Err(format!("{key} が文字列でない")),
                };
                if !(unit.is_empty() || unit == "deg") {
                    return Err(format!("{key} = '{unit}' には対応していない(deg だけ)"));
                }
            }
        }
        let ra0 = num(cards, "CRVAL1")?.ok_or("CRVAL1 が無い")?;
        let dec0 = num(cards, "CRVAL2")?.ok_or("CRVAL2 が無い")?;
        if dec0.abs() > 90.0 {
            return Err(format!("CRVAL2 = {dec0} が −90〜90 の外"));
        }
        let crpix = [num(cards, "CRPIX1")?.unwrap_or(0.0) - 1.0, num(cards, "CRPIX2")?.unwrap_or(0.0) - 1.0];

        let mut cd = linear_transform(cards)?;

        // 接点。tan_basis が極とみなす点(赤経・赤緯の東北が定まらない)は、極ちょうどに置く
        let mut crval = radec_to_xyz(ra0, dec0);
        let pole = crval[0].hypot(crval[1]) < 1e-15;
        // LONPOLE(天の極の天球座標系での経度)。天頂投影の既定は、δ0 ≥ 90° なら 0、それ以外は 180
        let phip = num(cards, "LONPOLE")?.unwrap_or(if dec0 >= 90.0 { 0.0 } else { 180.0 });
        // tan_basis の (東, 北) は LONPOLE = 180 の (x, y) と同じ。極ちょうどでは tan_basis が
        // 赤経 0 の向きを使うので、CRVAL1 のぶんも回す
        let delta = if !pole {
            phip - 180.0
        } else if dec0 > 0.0 {
            crval = [0.0, 0.0, 1.0];
            phip - ra0 - 180.0
        } else {
            crval = [0.0, 0.0, -1.0];
            phip + ra0 - 180.0
        };
        let delta = delta.rem_euclid(360.0);
        if delta != 0.0 && delta != 360.0 {
            // (ξ, η) = [[cosΔ, sinΔ], [−sinΔ, cosΔ]] · (x, y)
            let (s, c) = delta.to_radians().sin_cos();
            cd = mul2(&[[c, s], [-s, c]], &cd);
        }
        let d = det2(&cd);
        if !(d.is_finite() && d != 0.0) {
            return Err("CD が特異(行列式が 0)".to_string());
        }

        let sip = read_sip(cards)?;
        Ok(Wcs::new(crval, crpix, cd, sip))
    }
}

/// CTYPEn を読む(前後の空白を除き大文字にする)。
fn ctype(cards: &[Card], key: &str) -> Result<String, String> {
    match find_card(cards, key) {
        None => Err(format!("{key} が無い(画像に WCS が入っていない)")),
        Some(c) => match &c.value {
            CardValue::Str(s) => Ok(s.trim().to_ascii_uppercase()),
            _ => Err(format!("{key} が文字列でない")),
        },
    }
}

/// `RA---TAN`・`DEC--TAN-SIP` の形を調べる。戻り値は `-SIP` が付いているか。
fn parse_ctype(s: &str, want: &str, axis: usize) -> Result<bool, String> {
    let b = s.as_bytes();
    if b.len() < 8 || b[4] != b'-' || !s.is_char_boundary(4) || !s.is_char_boundary(8) {
        return Err(format!("CTYPE{axis} = '{s}' を読めない"));
    }
    let coord = s[..4].trim_end_matches('-');
    let proj = &s[5..8];
    let rest = &s[8..];
    if coord != want {
        return Err(format!(
            "CTYPE{axis} = '{s}': {} 軸には {want} が要る(赤道座標で、1 軸目が赤経・2 軸目が赤緯の WCS だけ読める)",
            axis
        ));
    }
    if proj != "TAN" {
        return Err(format!("CTYPE{axis} = '{s}': 投影 {proj} には対応していない(TAN と TAN-SIP だけ)"));
    }
    match rest {
        "" => Ok(false),
        "-SIP" => Ok(true),
        _ => Err(format!("CTYPE{axis} = '{s}': 歪みの形式 '{rest}' には対応していない(SIP だけ)")),
    }
}

/// 数値のカードを読む。無ければ None、数値でないか有限でなければ Err。
fn num(cards: &[Card], key: &str) -> Result<Option<f64>, String> {
    match find_card(cards, key) {
        None => Ok(None),
        Some(c) => match c.as_f64() {
            Some(v) if v.is_finite() => Ok(Some(v)),
            Some(_) => Err(format!("{key} が有限の数でない")),
            None => Err(format!("{key} の値が数値でない")),
        },
    }
}

/// 次数のカード(0 以上 MAX_SIP_ORDER 以下の整数)を読む。
fn order(cards: &[Card], key: &str) -> Result<Option<usize>, String> {
    match num(cards, key)? {
        None => Ok(None),
        Some(v) if v >= 0.0 && v.fract() == 0.0 && v <= MAX_SIP_ORDER as f64 => Ok(Some(v as usize)),
        Some(v) => Err(format!("{key} = {v} は扱えない(0〜{MAX_SIP_ORDER} の整数)")),
    }
}

/// 線形変換(度/px)を FITS の優先順で読む。
fn linear_transform(cards: &[Card]) -> Result<Mat2, String> {
    let get = |k: &str| num(cards, k);
    let cdk = [get("CD1_1")?, get("CD1_2")?, get("CD2_1")?, get("CD2_2")?];
    if cdk.iter().any(|v| v.is_some()) {
        let v = |i: usize| cdk[i].unwrap_or(0.0);
        return Ok([[v(0), v(1)], [v(2), v(3)]]);
    }
    let cdelt1 = get("CDELT1")?;
    let cdelt2 = get("CDELT2")?;
    if cdelt1.is_none() && cdelt2.is_none() {
        return Err("画素の大きさ(CDi_j か CDELTi)が無い".to_string());
    }
    let (c1, c2) = (cdelt1.unwrap_or(1.0), cdelt2.unwrap_or(1.0));
    let pck = [get("PC1_1")?, get("PC1_2")?, get("PC2_1")?, get("PC2_2")?];
    if pck.iter().any(|v| v.is_some()) {
        let pc = [[pck[0].unwrap_or(1.0), pck[1].unwrap_or(0.0)], [pck[2].unwrap_or(0.0), pck[3].unwrap_or(1.0)]];
        return Ok([[c1 * pc[0][0], c1 * pc[0][1]], [c2 * pc[1][0], c2 * pc[1][1]]]);
    }
    if let Some(rho) = get("CROTA2")? {
        let (s, c) = rho.to_radians().sin_cos();
        return Ok([[c1 * c, -c2 * s], [c1 * s, c2 * c]]);
    }
    Ok([[c1, 0.0], [0.0, c2]])
}

/// SIP の係数を読む。A_ORDER・B_ORDER が無ければ None。
fn read_sip(cards: &[Card]) -> Result<Option<Sip>, String> {
    let (ao, bo) = (order(cards, "A_ORDER")?, order(cards, "B_ORDER")?);
    let n = match (ao, bo) {
        (None, None) => return Ok(None),
        (Some(a), Some(b)) => a.max(b),
        _ => return Err("A_ORDER と B_ORDER の片方しか無い".to_string()),
    };
    if n < 2 {
        return Ok(None);
    }
    let read = |name: &str, n: usize, lo: usize| -> Result<Vec<f64>, String> {
        let m = n + 1;
        let mut c = vec![0.0; m * m];
        for p in 0..m {
            for q in 0..(m - p) {
                let key = format!("{name}_{p}_{q}");
                if let Some(v) = num(cards, &key)? {
                    if p + q < lo {
                        if v != 0.0 {
                            return Err(format!("{key} = {v}: 1 次以下の SIP の項には対応していない"));
                        }
                    } else {
                        c[p * m + q] = v;
                    }
                }
            }
        }
        Ok(c)
    };
    let mut sip = Sip::new(n, read("A", n, 2)?, read("B", n, 2)?)?;
    match (order(cards, "AP_ORDER")?, order(cards, "BP_ORDER")?) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            let m = a.max(b);
            sip = sip.with_inverse(m, read("AP", m, 0)?, read("BP", m, 0)?)?;
        }
        _ => return Err("AP_ORDER と BP_ORDER の片方しか無い".to_string()),
    }
    Ok(Some(sip))
}

// ------------------------------------------------------------ 折り返し ---

#[inline]
fn is_letter(c: u8) -> bool {
    // Python の正規表現の [^\d\W](ASCII では英字と _)
    c.is_ascii_alphabetic() || c == b'_'
}

#[inline]
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[inline]
fn is_word_punct(c: u8) -> bool {
    is_word(c) || matches!(c, b'!' | b'"' | b'\'' | b'&' | b'.' | b',' | b'?')
}

/// Python の textwrap.TextWrapper.wordsep_re で、位置 i から始まる塊の終わり。
/// 入力は空白が ' ' だけの ASCII(header_cards が先にそろえる)。
fn chunk_end(s: &[u8], i: usize) -> usize {
    let n = s.len();
    let at = |k: usize| if k < n { s[k] } else { 0 };
    // 空白の連なり
    if s[i] == b' ' {
        let mut j = i;
        while j < n && s[j] == b' ' {
            j += 1;
        }
        return j;
    }
    // 語のあいだの em ダッシュ: (?<=wp) -{2,} (?=\w)
    if s[i] == b'-' && i > 0 && is_word_punct(s[i - 1]) {
        let mut j = i;
        while j < n && s[j] == b'-' {
            j += 1;
        }
        if j - i >= 2 && j < n && is_word(s[j]) {
            return j;
        }
    }
    // 語(最短一致で伸ばし、ハイフンの切れ目・語の終わり・em ダッシュの手前のどれかで止める)
    let mut e = i + 1;
    loop {
        // ハイフンでつながった語: -(?:(?<=lt{2}-)|(?<=lt-lt-))(?=lt-?lt)
        if e < n && s[e] == b'-' {
            let h = e + 1;
            let behind = (h >= 3 && is_letter(s[h - 3]) && is_letter(s[h - 2]))
                || (h >= 4 && is_letter(s[h - 4]) && s[h - 3] == b'-' && is_letter(s[h - 2]));
            let ahead = is_letter(at(h))
                && ((at(h + 1) == b'-' && is_letter(at(h + 2))) || is_letter(at(h + 1)));
            if behind && ahead {
                return h;
            }
        }
        // 語の終わり: (?=\s|\Z)
        if e >= n || s[e] == b' ' {
            return e;
        }
        // em ダッシュの手前: (?<=wp)(?=-{2,}\w)
        if is_word_punct(s[e - 1]) && s[e] == b'-' && at(e + 1) == b'-' {
            let mut j = e;
            while j < n && s[j] == b'-' {
                j += 1;
            }
            if j < n && is_word(s[j]) {
                return e;
            }
        }
        e += 1;
    }
}

/// Python の `textwrap.wrap(text, width)`(既定の設定)と同じ折り返し。入力は空白が ' ' だけの ASCII を前提にする
/// (それ以外の文字はバイト単位で扱うので、行の長さが文字数とずれることがある)。
pub fn text_wrap(text: &str, width: usize) -> Vec<String> {
    let s = text.as_bytes();
    let width = width.max(1);
    // 塊に分ける
    let mut chunks: Vec<&[u8]> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let j = chunk_end(s, i).max(i + 1).min(s.len());
        chunks.push(&s[i..j]);
        i = j;
    }
    let is_space = |c: &[u8]| c.iter().all(|&b| b == b' ');
    // 後ろから取り出せるように逆順にする(Python と同じ)
    let mut stack: Vec<Vec<u8>> = chunks.into_iter().rev().map(|c| c.to_vec()).collect();
    let mut lines: Vec<String> = Vec::new();
    while !stack.is_empty() {
        let mut cur: Vec<Vec<u8>> = Vec::new();
        let mut cur_len = 0usize;
        if !lines.is_empty() && stack.last().is_some_and(|c| is_space(c)) {
            stack.pop();
        }
        while let Some(c) = stack.last() {
            if cur_len + c.len() <= width {
                cur_len += c.len();
                if let Some(c) = stack.pop() {
                    cur.push(c);
                }
            } else {
                break;
            }
        }
        if stack.last().is_some_and(|c| c.len() > width) {
            // 長すぎる語を切る(できれば最後のハイフンの後ろで)
            let space_left = width.saturating_sub(cur_len);
            if let Some(chunk) = stack.pop() {
                let mut end = space_left;
                if chunk.len() > space_left
                    && let Some(h) = chunk[..space_left.min(chunk.len())].iter().rposition(|&b| b == b'-')
                    && h > 0
                    && chunk[..h].iter().any(|&b| b != b'-')
                {
                    end = h + 1;
                }
                let end = end.min(chunk.len());
                cur.push(chunk[..end].to_vec());
                let rest = chunk[end..].to_vec();
                stack.push(rest);
            }
            // (Python はここで cur_len を数え直すが、使うのは max_lines のときだけなので省く)
        }
        if cur.last().is_some_and(|c| is_space(c)) {
            cur.pop();
        }
        // 中身の無い塊(長い語を切った残りが空)は捨てる
        stack.retain(|c| !c.is_empty());
        if !cur.is_empty() {
            let line: Vec<u8> = cur.concat();
            lines.push(String::from_utf8_lossy(&line).into_owned());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{ARCSEC_PER_RAD, angle_between};

    #[test]
    fn wrap_basics() {
        assert!(text_wrap("", 70).is_empty());
        assert!(text_wrap("   ", 70).is_empty());
        assert_eq!(text_wrap("a b", 70), vec!["a b"]);
        assert_eq!(text_wrap("  lead", 70), vec!["  lead"]);
        let long = "x".repeat(150);
        assert_eq!(text_wrap(&long, 70), vec!["x".repeat(70), "x".repeat(70), "x".repeat(10)]);
        assert_eq!(text_wrap("well-known goof-ball", 12), vec!["well-known", "goof-ball"]);
        assert_eq!(text_wrap("aaaa-bbbb-cccc", 6), vec!["aaaa-", "bbbb-", "cccc"]);
    }

    #[test]
    fn header_roundtrip_and_errors() {
        let cd = [[-3.61e-4, 1.85e-4], [-1.85e-4, -3.61e-4]];
        let mut a = vec![0.0; 9];
        let mut b = vec![0.0; 9];
        a[2 * 3] = 1.4e-6;
        a[3 + 1] = 5.4e-6;
        b[2] = 1.5e-5;
        let sip = Sip::new(2, a, b).unwrap().with_inverse(1, vec![1e-3, 2e-6, 3e-6, 0.0], vec![-1e-3, 0.0, 4e-6, 0.0]).unwrap();
        for w in [
            Wcs::new(radec_to_xyz(61.9, 41.97), [458.1, 280.2], cd, Some(sip.clone())),
            Wcs::tan(radec_to_xyz(0.0, 90.0), [10.0, 20.0], cd),
            Wcs::tan(radec_to_xyz(0.0, -90.0), [10.0, 20.0], cd),
        ] {
            let cards = w.header_cards(968, 548, &["index-5205-02 logodds 116.7".to_string(), "日本語".to_string()]);
            assert_eq!(cards.last().unwrap().comment, "???");
            let r = Wcs::from_header_cards(&cards).unwrap();
            assert_eq!(r.crpix, w.crpix);
            assert_eq!(r.sip, w.sip);
            for &(x, y) in &[(0.0, 0.0), (967.0, 547.0), (100.0, 400.0)] {
                let e = angle_between(r.pix_to_xyz(x, y), w.pix_to_xyz(x, y)) * ARCSEC_PER_RAD;
                assert!(e < 1e-9, "{e}");
            }
        }
        let base = Wcs::tan(radec_to_xyz(10.0, 20.0), [1.0, 2.0], cd).header_cards(10, 10, &[]);
        let with = |key: &str, v: CardValue| -> Vec<Card> {
            let mut c = base.clone();
            match c.iter_mut().find(|c| c.key == key) {
                Some(card) => card.value = v,
                None => c.push(Card::new(key, v, "")),
            }
            c
        };
        let bad = [
            with("CTYPE1", CardValue::Str("RA---SIN".into())),
            with("CTYPE1", CardValue::Str("RA---TPV".into())),
            with("CTYPE1", CardValue::Str("GLON-TAN".into())),
            with("CTYPE1", CardValue::Str("DEC--TAN".into())),
            with("CTYPE2", CardValue::Str("DEC--TAN-ZPX".into())),
            with("CTYPE2", CardValue::Str("DEC".into())),
            with("CTYPE2", CardValue::Str("DE日本語語".into())),
            with("CTYPE1", CardValue::Int(3)),
            with("CRVAL1", CardValue::Str("abc".into())),
            with("CRVAL2", CardValue::Float(f64::NAN)),
            with("CRVAL2", CardValue::Float(95.0)),
            with("CUNIT1", CardValue::Str("rad".into())),
            with("CD1_1", CardValue::Float(0.0)).into_iter().map(|mut c| {
                if c.key.starts_with("CD") { c.value = CardValue::Float(0.0) }
                c
            }).collect(),
            base.iter().filter(|c| c.key != "CRVAL1").cloned().collect(),
            base.iter().filter(|c| c.key != "CTYPE2").cloned().collect(),
            base.iter().filter(|c| !c.key.starts_with("CD")).cloned().collect(),
        ];
        for (i, cards) in bad.iter().enumerate() {
            assert!(Wcs::from_header_cards(cards).is_err(), "{i} が通った");
        }
        // SIP の壊れ方
        let mut sipc = with("CTYPE1", CardValue::Str("RA---TAN-SIP".into()));
        sipc.push(Card::new("A_ORDER", CardValue::Int(2), ""));
        assert!(Wcs::from_header_cards(&sipc).is_err()); // B_ORDER が無い
        sipc.push(Card::new("B_ORDER", CardValue::Int(1_000_000_000), ""));
        assert!(Wcs::from_header_cards(&sipc).is_err()); // 巨大な次数
        sipc.pop();
        sipc.push(Card::new("B_ORDER", CardValue::Int(2), ""));
        assert!(Wcs::from_header_cards(&sipc).unwrap().sip.is_some());
        sipc.push(Card::new("A_1_0", CardValue::Float(1e-3), ""));
        assert!(Wcs::from_header_cards(&sipc).is_err()); // 1 次の項
        sipc.pop();
        sipc.push(Card::new("AP_ORDER", CardValue::Float(2.5), ""));
        assert!(Wcs::from_header_cards(&sipc).is_err());
        assert!(Wcs::from_header_cards(&[]).is_err());
    }
}

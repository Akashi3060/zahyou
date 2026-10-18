//! 天体名 → 座標(Python の `resolve_target_name`)。
//!
//! 1. Sesame(SIMBAD → NED → VizieR の順)で名前を引く。astropy の `SkyCoord.from_name` と同じ URL と読み方
//! 2. VizieR の Gaia DR3(I/355/gaiadr3)で 5″ 以内の星を探し、<b>最も近い星</b>の座標にする
//!    (v5 は先頭行を無条件に採用していた。5″ 以内に複数あると別の星を掴む)
//! 3. だめなら Sesame の座標。Sesame もだめなら、以前引いて cache.json に覚えた座標
//!
//! 一度引いた名前は覚えるので、次からはオフラインでも使える。

use crate::cache::{self, CachedTarget};
use crate::http;
use crate::pyfmt::py_quote;
use std::path::Path;
use std::time::Duration;

/// Sesame の置き場所(astropy の `sesame_url` と同じ 2 つ。前から順に試す)。
pub const SESAME_URLS: [&str; 2] = [
    "https://cds.unistra.fr/cgi-bin/nph-sesame/",
    "http://vizier.cfa.harvard.edu/viz-bin/nph-sesame/",
];
/// VizieR(astroquery の既定のミラー)。astroquery は votable を頼むが、こちらは同じ問い合わせを TSV で受ける。
pub const VIZIER_URL: &str = "https://vizier.cds.unistra.fr/viz-bin/asu-tsv";
/// Gaia DR3 の表
pub const GAIA_CATALOG: &str = "I/355/gaiadr3";
/// 精密化で探す半径 [″]
pub const GAIA_RADIUS_ARCSEC: f64 = 5.0;

/// Sesame の待ち時間(astropy の remote_timeout の既定 10 秒)
const SESAME_TIMEOUT: Duration = Duration::from_secs(10);
/// VizieR の待ち時間(astroquery の既定 60 秒)
const VIZIER_TIMEOUT: Duration = Duration::from_secs(60);

/// 名前を引いた結果。
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// 赤経 [度](ICRS)
    pub ra: f64,
    /// 赤緯 [度](ICRS)
    pub dec: f64,
    /// どこから得た座標か(1 行。ログと画面にそのまま出せる)
    pub note: String,
    /// Sesame が返した座標(オンラインで引けたとき)
    pub sesame: Option<(f64, f64)>,
    /// Gaia DR3 で精密化できたとき、その星の G 等級(表に無ければ None)と候補の数
    pub gaia: Option<(Option<f64>, usize)>,
    /// キャッシュから返したか
    pub from_cache: bool,
}

/// Sesame の URL(astropy と同じく `<置き場所>SNV?<quote(name)>`)。
pub fn sesame_url(base: &str, name: &str) -> String {
    format!("{base}SNV?{}", py_quote(name, "/"))
}

/// Sesame の応答から座標を読む(astropy の `_parse_response`: `%J` で始まる ICRS J2000 の行)。
///
/// astropy と同じく、赤経は [0, 360) に入れ、赤緯が ±90° を超えるものは読めないものとする。
pub fn parse_sesame(text: &str) -> Option<(f64, f64)> {
    let (ra, dec) = parse_sesame_raw(text)?;
    if !(-90.0..=90.0).contains(&dec) || !ra.is_finite() {
        return None;
    }
    Some((ra.rem_euclid(360.0), dec))
}

fn parse_sesame_raw(text: &str) -> Option<(f64, f64)> {
    // 正規表現 %J\s*([0-9\.]+)\s*([\+\-\.0-9]+) の最初の一致
    let mut from = 0;
    while let Some(i) = text[from..].find("%J") {
        let start = from + i + 2;
        let rest = &text[start..];
        let t = rest.trim_start();
        let ra_len = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
        if ra_len > 0 {
            let after = t[ra_len..].trim_start();
            let dec_len = after
                .find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.')))
                .unwrap_or(after.len());
            if dec_len > 0 {
                let ra = t[..ra_len].parse::<f64>().ok()?;
                let dec = after[..dec_len].parse::<f64>().ok()?;
                return Some((ra, dec));
            }
            if ra_len >= 2 {
                // 正規表現の後戻りと同じ: 赤経の最後の 1 文字が赤緯になる
                let ra = t[..ra_len - 1].parse::<f64>().ok()?;
                let dec = t[ra_len - 1..ra_len].parse::<f64>().ok()?;
                return Some((ra, dec));
            }
        }
        from = start;
    }
    None
}

/// Sesame で名前を引く(置き場所を前から順に試す。通信に失敗したら次へ。見つからなければそこで終わり)。
pub fn sesame_lookup(name: &str, bases: &[&str]) -> Result<(f64, f64), String> {
    let agent = http::agent(SESAME_TIMEOUT, true);
    let mut errors = Vec::new();
    for base in bases {
        let url = sesame_url(base, name);
        match agent.get(&url).call() {
            Ok(mut resp) => {
                let text = http::read_text(&mut resp, 1 << 20)?;
                return parse_sesame(&text)
                    .ok_or_else(|| format!("Unable to find coordinates for name '{name}' using {url}"));
            }
            Err(e) => errors.push(format!("{url}: {}", http::describe(&e))),
        }
    }
    Err(format!("Sesame に届きませんでした: {}", errors.join(" / ")))
}

/// ICRS → FK5(J2000)。astroquery の query_region は既定で中心を FK5 に直して送るので、それに合わせる。
/// 行列は astropy の `_icrs_to_fk5_matrix`(USNO circular 179 の B 行列)= Rx(-η0)·Ry(ξ0)·Rz(dα0)。
pub fn icrs_to_fk5_j2000(ra: f64, dec: f64) -> (f64, f64) {
    // 軸まわりの回転(astropy の rotation_matrix と同じ向き)
    fn rot(deg: f64, axis: usize) -> [[f64; 3]; 3] {
        let (s, c) = deg.to_radians().sin_cos();
        let mut r = [[0.0; 3]; 3];
        let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
        r[axis][axis] = 1.0;
        r[a1][a1] = c;
        r[a1][a2] = s;
        r[a2][a1] = -s;
        r[a2][a2] = c;
        r
    }
    fn mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, x) in row.iter_mut().enumerate() {
                *x = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        r
    }
    let eta0 = -19.9 / 3_600_000.0;
    let xi0 = 9.1 / 3_600_000.0;
    let da0 = -22.9 / 3_600_000.0;
    let m = mul(&mul(&rot(-eta0, 0), &rot(xi0, 1)), &rot(da0, 2));
    let (l, b) = (ra.to_radians(), dec.to_radians());
    let v = [b.cos() * l.cos(), b.cos() * l.sin(), b.sin()];
    let w: Vec<f64> = m.iter().map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2]).collect();
    let lon = w[1].atan2(w[0]).to_degrees().rem_euclid(360.0);
    let lat = w[2].atan2(w[0].hypot(w[1])).to_degrees();
    (lon, lat)
}

/// VizieR へ送る本文(astroquery の `Vizier(catalog=..., columns=[RA_ICRS, DE_ICRS, Gmag])`、
/// ROW_LIMIT=50、`query_region(radius=5″)` が作るものと同じ)。`ra`・`dec` は ICRS [度]。
pub fn vizier_query_body(ra: f64, dec: f64) -> String {
    // 中心は FK5(J2000)で送る(astroquery の既定 frame='fk5')。赤経は [0, 360)
    let (ra, dec) = icrs_to_fk5_j2000(ra, dec);
    [
        format!("-source={GAIA_CATALOG}"),
        "-out.max=50".into(),
        "-out=RA_ICRS,DE_ICRS,Gmag".into(),
        format!("-c={ra:.8}{dec:+.8}"),
        format!("-c.rs={}", crate::pyfmt::py_float_repr(GAIA_RADIUS_ARCSEC)),
        "-out.meta=huUD".into(),
        "-out.form=mini".into(),
        "-oc.form=d".into(),
    ]
    .join("\n")
}

/// Gaia DR3 の 1 行。
#[derive(Clone, Debug, PartialEq)]
pub struct GaiaRow {
    pub ra: f64,
    pub dec: f64,
    pub gmag: Option<f64>,
}

/// VizieR の TSV から RA_ICRS / DE_ICRS / Gmag の行を読む。
///
/// 形: `#` で始まる注記 → 列名の行 → 単位の行 → `---` の行 → データの行 → 空行。
pub fn parse_vizier_tsv(text: &str) -> Result<Vec<GaiaRow>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(hi) = lines.iter().position(|l| {
        !l.starts_with('#') && l.split('\t').any(|c| c.trim() == "RA_ICRS") && l.split('\t').any(|c| c.trim() == "DE_ICRS")
    }) else {
        // 表が無い = 該当なし
        return Ok(Vec::new());
    };
    let cols: Vec<&str> = lines[hi].split('\t').map(|c| c.trim()).collect();
    let col = |name: &str| cols.iter().position(|c| *c == name);
    let (ira, idec, ig) = (col("RA_ICRS"), col("DE_ICRS"), col("Gmag"));
    let (Some(ira), Some(idec)) = (ira, idec) else {
        return Err("VizieR の表に RA_ICRS / DE_ICRS がありません".into());
    };
    let Some(dash) = lines[hi..].iter().position(|l| !l.is_empty() && l.chars().all(|c| c == '-' || c == '\t')) else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for l in &lines[hi + dash + 1..] {
        if l.trim().is_empty() || l.starts_with('#') {
            break;
        }
        let f: Vec<&str> = l.split('\t').map(|c| c.trim()).collect();
        let num = |i: usize| f.get(i).and_then(|s| s.parse::<f64>().ok());
        let (Some(ra), Some(dec)) = (num(ira), num(idec)) else {
            return Err(format!("VizieR の行を読めません: {l}"));
        };
        rows.push(GaiaRow { ra, dec, gmag: ig.and_then(num) });
    }
    Ok(rows)
}

/// 2 点の角距離 [度](astropy の `separation` と同じ Vincenty の式)。
pub fn separation_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (l1, b1, l2, b2) = (ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians());
    let dl = l2 - l1;
    let (sdl, cdl) = dl.sin_cos();
    let (s1, c1) = b1.sin_cos();
    let (s2, c2) = b2.sin_cos();
    let num1 = c2 * sdl;
    let num2 = c1 * s2 - s1 * c2 * cdl;
    let den = s1 * s2 + c1 * c2 * cdl;
    num1.hypot(num2).atan2(den).to_degrees()
}

/// Gaia DR3 で 5″ 以内の星を探す。(最も近い星, 候補の数)。該当が無ければ None。
pub fn gaia_nearest(ra: f64, dec: f64, url: &str) -> Result<Option<(GaiaRow, usize)>, String> {
    let agent = http::agent(VIZIER_TIMEOUT, true);
    let mut resp = agent
        .post(url)
        .content_type("application/x-www-form-urlencoded")
        .send(vizier_query_body(ra, dec).as_bytes())
        .map_err(|e| http::describe(&e))?;
    let text = http::read_text(&mut resp, 8 << 20)?;
    let rows = parse_vizier_tsv(&text)?;
    let n = rows.len();
    let best = rows.into_iter().fold(None::<(GaiaRow, f64)>, |best, r| {
        let d = separation_deg(ra, dec, r.ra, r.dec);
        match best {
            // 同じ距離なら先の行(numpy の argmin と同じ)
            Some((b, bd)) if bd <= d => Some((b, bd)),
            _ => Some((r, d)),
        }
    });
    Ok(best.map(|(r, _)| (r, n)))
}

/// 天体名 → (RA, Dec, 説明)。オンラインで引き、結果を既定の cache.json に覚える(設計書 4.4 の API)。
pub fn resolve_target_name(name: &str) -> Result<(f64, f64, String), String> {
    let r = resolve_target(name, true, &cache::cache_path(), &|_| {})?;
    Ok((r.ra, r.dec, r.note))
}

/// 天体名 → 座標(Python の `resolve_target_name(name, online)` と同じ流れ)。
///
/// `online` が偽なら cache.json だけを見る。`log` には Python 版と同じ文言で経過を書く。
pub fn resolve_target(name: &str, online: bool, cache_file: &Path, log: &dyn Fn(&str)) -> Result<Resolved, String> {
    resolve_target_with(name, online, cache_file, &SESAME_URLS, VIZIER_URL, log)
}

/// [`resolve_target`] の問い合わせ先を指定する版(試験用)。
pub fn resolve_target_with(
    name: &str,
    online: bool,
    cache_file: &Path,
    sesame: &[&str],
    vizier: &str,
    log: &dyn Fn(&str),
) -> Result<Resolved, String> {
    let cached = || cache::lookup_target(&cache::load_cache_at(cache_file), name);
    let from_cache = |hit: CachedTarget| Resolved {
        ra: hit.ra,
        dec: hit.dec,
        note: "以前調べた座標".into(),
        sesame: None,
        gaia: None,
        from_cache: true,
    };
    if !online {
        return match cached() {
            Some(hit) => {
                log(&format!("  (オフライン) 以前調べた座標を再利用します: {name}"));
                Ok(from_cache(hit))
            }
            None => {
                let msg = format!("オフラインでは '{name}' を座標に変換できません。赤経赤緯を直接入力してください。");
                log(&format!("  ❌ {msg}"));
                Err(msg)
            }
        };
    }

    log(&format!("\n'{name}' の座標を調べています..."));
    let (sra, sdec) = match sesame_lookup(name, sesame) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("  ❌ 名前を解決できませんでした: {e}"));
            return match cached() {
                Some(hit) => {
                    log("  → 以前調べた座標を使います。");
                    Ok(from_cache(hit))
                }
                None => Err(format!("名前を解決できませんでした: {e}")),
            };
        }
    };
    log(&format!("  - 名前解決 (SIMBAD/Sesame): {}", hmsdms(sra, sdec)));

    let mut out = Resolved {
        ra: sra,
        dec: sdec,
        note: "SIMBAD/Sesame の座標".into(),
        sesame: Some((sra, sdec)),
        gaia: None,
        from_cache: false,
    };
    match gaia_nearest(sra, sdec, vizier) {
        Ok(Some((row, n))) => {
            out.ra = row.ra;
            out.dec = row.dec;
            out.gaia = Some((row.gmag, n));
            let extra = match row.gmag {
                Some(g) => format!(" (G={g:.2}, 候補 {n} 個中で最も近い星)"),
                None => String::new(),
            };
            log(&format!("  - Gaia DR3 で精密化: RA={:.6}° Dec={:.6}°{extra}", row.ra, row.dec));
            out.note = format!("Gaia DR3 で精密化{extra}");
        }
        Ok(None) => {}
        Err(e) => {
            log(&format!("  - Gaia DR3 の照合はできませんでした ({e})。SIMBAD の座標を使います。"));
        }
    }
    if let Err(e) = cache::remember_target_at(cache_file, name, out.ra, out.dec) {
        log(&format!("  - 座標を覚えられませんでした ({e})"));
    }
    Ok(out)
}

/// astropy の `to_string('hmsdms', precision=3)` に近い表示(ログ用)。
fn hmsdms(ra: f64, dec: f64) -> String {
    let h = ra.rem_euclid(360.0) / 15.0;
    let total = (h * 3600.0 * 1000.0).round() as i64; // ミリ秒単位
    let (hh, mm, ss) = (total / 3_600_000, total / 60_000 % 60, (total % 60_000) as f64 / 1000.0);
    let sign = if dec < 0.0 { '-' } else { '+' };
    let t = (dec.abs() * 3600.0 * 1000.0).round() as i64;
    let (dd, dm, ds) = (t / 3_600_000, t / 60_000 % 60, (t % 60_000) as f64 / 1000.0);
    format!("{hh:02}h{mm:02}m{ss:06.3}s {sign}{dd:02}d{dm:02}m{ds:06.3}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sesame_parse() {
        let t = "# x\n%J 061.911993600 +41.986275900 = 04:07:38.87 +41:59:10.5\n";
        assert_eq!(parse_sesame(t), Some((61.9119936, 41.9862759)));
        assert_eq!(parse_sesame("#!Nothing found"), None);
    }

    #[test]
    fn sep_small() {
        let d = separation_deg(10.0, 20.0, 10.0, 20.0 + 1.0 / 3600.0) * 3600.0;
        assert!((d - 1.0).abs() < 1e-9);
    }
}

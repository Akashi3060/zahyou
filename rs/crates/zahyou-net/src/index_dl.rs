//! 星図データ(astrometry.net 形式の index-*.fits)の段・名前・URL と、落とす処理。
//!
//! Python 版 gui/zahyou_env.py の SCALE_FOV / SCALE_MB / index_names / index_url / fov_arcmin /
//! recommended_scales / recommend_scales / remote_sizes / fetch_index / check_index_file を移したもの。
//! 形式の細部は docs/index-format.md の 2〜4 節。

use crate::NetError;
use crate::http::{self, ResumeOpts};
use crate::pyfmt::group_thousands;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// 画角の段(astrometry.net の付番規則。末尾 2 桁が画角 [分角])。(段, 下限′, 上限′)。並びは Python 版の辞書と同じ。
pub const SCALE_FOV: &[(u32, f64, f64)] = &[
    (4107, 22.0, 30.0),
    (4108, 30.0, 42.0),
    (4109, 42.0, 60.0),
    (4110, 60.0, 85.0),
    (4111, 85.0, 120.0),
    (4112, 120.0, 170.0),
    (4113, 170.0, 240.0),
    (4114, 240.0, 340.0),
    (4115, 340.0, 480.0),
    (4116, 480.0, 680.0),
    (4117, 680.0, 1000.0),
    (4118, 1000.0, 1400.0),
    (4119, 1400.0, 2000.0),
    (5200, 2.0, 2.8),
    (5201, 2.8, 4.0),
    (5202, 4.0, 5.6),
    (5203, 5.6, 8.0),
    (5204, 8.0, 11.0),
    (5205, 11.0, 16.0),
    (5206, 16.0, 22.0),
];

/// おおよその容量 [MiB](選ぶ前に見当が付くように。実物とは 2 割ほどずれる)。
pub const SCALE_MB: &[(u32, u64)] = &[
    (4107, 158),
    (4108, 79),
    (4109, 40),
    (4110, 20),
    (4111, 10),
    (4112, 5),
    (4113, 3),
    (4114, 2),
    (4115, 1),
    (4116, 1),
    (4117, 1),
    (4118, 1),
    (4119, 1),
    (5200, 15900),
    (5201, 9300),
    (5202, 4900),
    (5203, 2400),
    (5204, 1200),
    (5205, 620),
    (5206, 320),
];

/// 5200 系の取得元(nside=2 の 48 タイル)。
pub const LITE_BASE: &str = "https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE";
/// 4100 系の取得元(Python 版と同じく http)。
pub const G41_BASE: &str = "http://data.astrometry.net/4100";
/// 5200 系は 48 分割(HEALPix nside=2)。
pub const TILES: u32 = 48;
/// 「おすすめ」= 画角の下限が 4′ 以上の段。
pub const RECOMMENDED_MIN_FOV: f64 = 4.0;

const BLOCK: usize = 2880;
/// リトルエンディアン。ほかの値の索引は読めない
const ENDIAN: &str = "04:03:02:01";

/// 段の画角 (下限′, 上限′)。
pub fn scale_fov(scale: u32) -> Option<(f64, f64)> {
    SCALE_FOV.iter().find(|r| r.0 == scale).map(|r| (r.1, r.2))
}

/// 段のおおよその容量 [MiB]。
pub fn scale_mb(scale: u32) -> Option<u64> {
    SCALE_MB.iter().find(|r| r.0 == scale).map(|r| r.1)
}

/// その段ぜんぶで何 MiB になるか(表に無い段は 0)。
pub fn scales_mb(scales: &[u32]) -> u64 {
    scales.iter().map(|s| scale_mb(*s).unwrap_or(0)).sum()
}

/// Python の `human()`: 1024 ごとに B / KB / MB / GB / TB、小数 1 桁、整数部は 3 桁区切り。
pub fn human(n: f64) -> String {
    let mut n = n;
    for unit in ["B", "KB", "MB", "GB"] {
        if n.abs() < 1024.0 {
            return format!("{} {unit}", group_thousands(&format!("{n:.1}")));
        }
        n /= 1024.0;
    }
    format!("{n:.1} TB")
}

/// その段に含まれるファイル名の一覧。
pub fn index_names(scale: u32) -> Vec<String> {
    if scale >= 5200 {
        (0..TILES).map(|t| format!("index-{scale}-{t:02}.fits")).collect()
    } else {
        vec![format!("index-{scale}.fits")]
    }
}

/// 段とタイル番号から URL(Python の `index_url(scale, tile)`)。
pub fn index_url_for(scale: u32, tile: Option<u32>) -> String {
    url_with(scale, tile, LITE_BASE, G41_BASE)
}

fn url_with(scale: u32, tile: Option<u32>, lite: &str, g41: &str) -> String {
    if scale >= 5200 {
        format!("{}/index-{scale}-{:02}.fits", lite.trim_end_matches('/'), tile.unwrap_or(0))
    } else {
        format!("{}/index-{scale}.fits", g41.trim_end_matches('/'))
    }
}

/// ファイル名から URL。名前が索引ファイルの形でなければ、4100 系の置き場の下にそのまま付ける。
pub fn index_url(name: &str) -> String {
    match parse_index_name(name) {
        Some((s, t)) => index_url_for(s, t),
        None => format!("{G41_BASE}/{name}"),
    }
}

/// `index-NNNN.fits` / `index-NNNN-TT.fits` を (段, タイル) に分ける(Python の `_INDEX_RE`)。
pub fn parse_index_name(name: &str) -> Option<(u32, Option<u32>)> {
    let rest = name.strip_prefix("index-")?.strip_suffix(".fits")?;
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    match rest.split_once('-') {
        None if digits(rest, 4) => Some((rest.parse().ok()?, None)),
        Some((a, b)) if digits(a, 4) && digits(b, 2) => Some((a.parse().ok()?, Some(b.parse().ok()?))),
        _ => None,
    }
}

/// 焦点距離 [mm] とセンサー横幅 [mm] から画角 [分角]。どちらかが 0(または数でない)なら None。
pub fn fov_arcmin(focal_mm: f64, sensor_mm: f64) -> Option<f64> {
    if focal_mm == 0.0 || sensor_mm == 0.0 || !focal_mm.is_finite() || !sensor_mm.is_finite() {
        return None;
    }
    Some((2.0 * (sensor_mm / 2.0 / focal_mm).atan()).to_degrees() * 60.0)
}

/// おすすめの段(画角 4′ 以上)。焦点距離を聞かずに決められる。
pub fn recommended_scales() -> Vec<u32> {
    let mut v: Vec<u32> = SCALE_FOV.iter().filter(|r| r.1 >= RECOMMENDED_MIN_FOV).map(|r| r.0).collect();
    v.sort_unstable();
    v
}

/// その画角を解くのに要る段。導入誤差ぶん、前後 1 段ずつ足す(Python の `recommend_scales`)。
///
/// `fov_short` を渡すと短辺の画角から長辺の画角までをまとめて覆う。「前後 1 段」は画角の順で数える
/// (4100 系と 5200 系は別のはしごなので、番号順では 4119 の隣が 5200 になってしまう)。
pub fn recommend_scales(fov: f64, fov_short: Option<f64>) -> Vec<u32> {
    if fov.is_nan() || fov <= 0.0 {
        return Vec::new();
    }
    // Python の `if fov_short` と同じく 0 は「無し」
    let short = fov_short.filter(|s| *s != 0.0);
    let (lo_fov, hi_fov) = match short {
        Some(s) => (fov.min(s), fov.max(s)),
        None => (fov, fov),
    };
    let mut keys: Vec<(u32, f64, f64)> = SCALE_FOV.to_vec();
    // (下限, 上限) の組で並べる(Python の sorted(key=SCALE_FOV[s]) と同じ。同じ組は元の順)
    keys.sort_by(|a, b| (a.1, a.2).partial_cmp(&(b.1, b.2)).unwrap_or(std::cmp::Ordering::Equal));
    let mut hit: Vec<usize> = (0..keys.len()).filter(|&i| keys[i].2 >= lo_fov && keys[i].1 <= hi_fov).collect();
    if hit.is_empty() {
        // 範囲外なら一番近い段(同じ距離なら先のもの)
        let dist = |i: usize| (keys[i].1 - lo_fov).abs().min((keys[i].2 - hi_fov).abs());
        let mut best = 0;
        for i in 1..keys.len() {
            if dist(i) < dist(best) {
                best = i;
            }
        }
        hit.push(best);
    }
    let mut out: Vec<u32> = hit.iter().map(|&i| keys[i].0).collect();
    for &i in [hit[0], hit[hit.len() - 1]].iter() {
        for j in [i.wrapping_sub(1), i + 1] {
            if j < keys.len() {
                out.push(keys[j].0);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// まだ落ちていないぶんの、おおよその容量 [byte](確認画面の目安用)。
pub fn plan_size(scales: &[u32], dest: &Path) -> u64 {
    let mut need = 0.0f64;
    for &s in scales {
        let names = index_names(s);
        let per = scale_mb(s).unwrap_or(100) as f64 * 1024.0 * 1024.0 / names.len() as f64;
        for name in names {
            if !dest.join(&name).exists() {
                need += per;
            }
        }
    }
    need as u64
}

// ================================================================ 中身の検査 ===

/// FITS のカード 1 枚の値の部分(文字列なら引用符の中、ほかは / の注記を除いたもの)。
fn card_value(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix('\'') {
        let v = match rest.find('\'') {
            Some(end) => &rest[..end],
            None => rest,
        };
        return v.trim_end().to_string();
    }
    t.split('/').next().unwrap_or("").trim().to_string()
}

/// ヘッダーを 1 つ読む(2880 バイトのブロック単位)。戻り値 (カード, 読んだバイト数)。
fn read_header(f: &mut File) -> Result<(HashMap<String, String>, u64), String> {
    let mut cards = HashMap::new();
    let mut block = vec![0u8; BLOCK];
    let mut used = 0u64;
    loop {
        if read_full(f, &mut block)? < BLOCK {
            return Err("ヘッダーの途中でファイルが終わっています".into());
        }
        used += BLOCK as u64;
        for card in block.chunks(80) {
            // ASCII 以外は置き換え文字にする(Python の decode("ascii", "replace"))。1 バイト = 1 文字
            let text: Vec<char> = card.iter().map(|&b| if b.is_ascii() { b as char } else { '\u{fffd}' }).collect();
            let key = text[..8].iter().collect::<String>().trim().to_string();
            if key == "END" {
                return Ok((cards, used));
            }
            if text[8] == '=' && text[9] == ' ' && !cards.contains_key(&key) {
                cards.insert(key, card_value(&text[10..].iter().collect::<String>()));
            }
        }
    }
}

fn read_full(f: &mut File, buf: &mut [u8]) -> Result<usize, String> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(n)
}

fn int_card(h: &HashMap<String, String>, key: &str, default: i64, or_default: bool) -> Result<i64, String> {
    match h.get(key) {
        None => Ok(default),
        Some(v) if or_default && v.is_empty() => Ok(default),
        Some(v) => v
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("{key} の値を整数として読めません ('{v}')")),
    }
}

/// そのヘッダーに続くデータの長さ [バイト](2880 の倍数に詰める前)。
fn data_bytes(h: &HashMap<String, String>) -> Result<u64, String> {
    let naxis = int_card(h, "NAXIS", 0, true)?;
    if naxis == 0 {
        return Ok(0);
    }
    if !(0..=999).contains(&naxis) {
        return Err(format!("NAXIS がおかしい ({naxis})"));
    }
    let mut n: i128 = 1;
    for i in 1..=naxis {
        n *= int_card(h, &format!("NAXIS{i}"), 0, true)? as i128;
        if n.abs() > 1 << 62 {
            return Err("データの長さが大きすぎます".into());
        }
    }
    n += int_card(h, "PCOUNT", 0, true)? as i128;
    n *= int_card(h, "GCOUNT", 1, true)? as i128;
    let bitpix = int_card(h, "BITPIX", 8, false)? as i128;
    let bytes = bitpix.abs() / 8 * n;
    if !(0..=(1i128 << 62)).contains(&bytes) {
        return Err(format!("データの長さがおかしい ({bytes})"));
    }
    Ok(bytes as u64)
}

fn padded(n: u64) -> u64 {
    n.div_ceil(BLOCK as u64) * BLOCK as u64
}

/// ヘッダーだけで検査する(Python の `_check_layout`)。
fn check_layout(path: &Path) -> Result<(), String> {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some((scale, tile)) = parse_index_name(&name) else {
        return Err("索引ファイルの名前ではありません".into());
    };
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let (prim, used) = read_header(&mut f)?;
    let mut pos = used + padded(data_bytes(&prim)?);
    while pos < size {
        use std::io::Seek;
        f.seek(std::io::SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let (h, used) = read_header(&mut f)?;
        pos += used + padded(data_bytes(&h)?);
    }
    if pos != size {
        return Err(format!(
            "大きさが合いません (ヘッダーから {} バイト、実物 {} バイト。途中で切れたファイルです)",
            group_thousands(&pos.to_string()),
            group_thousands(&size.to_string())
        ));
    }
    let endian = prim.get("ENDIAN").map(|s| s.as_str());
    if endian != Some(ENDIAN) {
        return Err(format!("エンディアンが違います ({})", endian.unwrap_or("None")));
    }
    let indexid = int_card(&prim, "INDEXID", -1, false).map_err(|e| format!("ヘッダーの値を読めません ({e})"))?;
    if indexid != scale as i64 {
        return Err(format!("INDEXID が名前と違います ({})", prim.get("INDEXID").map(|s| s.as_str()).unwrap_or("None")));
    }
    if let Some(t) = tile {
        let hp = int_card(&prim, "HEALPIX", -1, false).map_err(|e| format!("ヘッダーの値を読めません ({e})"))?;
        if hp != t as i64 {
            return Err(format!("HEALPIX がタイル番号と違います ({})", prim.get("HEALPIX").map(|s| s.as_str()).unwrap_or("None")));
        }
    }
    Ok(())
}

/// 索引ファイルの追加の検査(Err は画面に出す理由)。
pub type Validator<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// (正規化したパス, 大きさ, 更新時刻, 追加の検査の関数の番地。無ければ 0)
type CheckKey = (String, u64, u128, usize);

fn checked() -> &'static Mutex<HashMap<CheckKey, Result<(), String>>> {
    static C: OnceLock<Mutex<HashMap<CheckKey, Result<(), String>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 索引ファイル 1 本を検査する(Python の `check_index_file`)。
///
/// 見るのは、大きさがヘッダーから計算した長さと一致するか(途中で切れたダウンロードはここで分かる)、
/// ENDIAN、INDEXID とファイル名の番号、5200 系の HEALPIX とタイル番号。中身(星や quad)までは読まない。
/// `validate`(zahyou-index の `validate_index_file` など)を渡すと、こちらの検査に通ったあとでそれも通す。
/// 同じファイルは大きさと更新時刻が変わるまで調べ直さない。
///
/// Err の文面は Python 版の `_check_layout` と同じ(追加の検査で落ちたときはその文面)。
pub fn check_index_file(path: &Path, validate: Option<Validator>) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    // 関数の番地は鍵に使うだけ(読み書きはしない)
    let vkey = validate.map(|v| v as *const dyn Fn(&Path) -> Result<(), String> as *const () as usize).unwrap_or(0);
    let key = (abs.to_string_lossy().to_lowercase(), meta.len(), mtime, vkey);
    if let Some(hit) = checked().lock().ok().and_then(|m| m.get(&key).cloned()) {
        return hit;
    }
    let mut r = check_layout(path);
    if r.is_ok()
        && let Some(v) = validate
    {
        r = v(path);
    }
    if let Ok(mut m) = checked().lock() {
        m.insert(key, r.clone());
    }
    r
}

/// zahyou-index の検査(`validate_index_file`)。解析で使う表がそろって大きさが合うか、kd-tree の形まで見る
/// (ヘッダーだけ読むので速い)。[`FetchOptions`] の既定の追加の検査。
pub fn validate_with_index(path: &Path) -> Result<(), String> {
    zahyou_index::validate_index_file(path)
}

/// 軽い検査([`check_index_file`])のあとで zahyou-index の検査も通す。解析で使えるファイルかどうか。
pub fn check_index_file_full(path: &Path) -> Result<(), String> {
    check_index_file(path, Some(&validate_with_index))
}

// ================================================================ 落とす ===

/// 落とすときの設定。既定値は Python 版と同じ(取得元・HEAD 8 本並列・5 回まで再試行)。
///
/// 検査だけは Python 版より強い: 既定で zahyou-index の検査([`validate_with_index`])も通す
/// (Python 版の fetch_index は軽い検査だけ。解析のエンジンの検査は「状態を確認」のときだけだった)。
#[derive(Clone)]
pub struct FetchOptions<'a> {
    /// 落としたファイルの追加の検査(既定は [`validate_with_index`]。None なら軽い検査だけ)
    pub validate: Option<Validator<'a>>,
    pub lite_base: String,
    pub g41_base: String,
    /// HEAD を同時に投げる数
    pub head_workers: usize,
    /// 失敗してよい回数
    pub retries: u32,
    /// 再試行の待ち時間の単位(min(2^n, 20) × これ)
    pub retry_unit: Duration,
    /// 1 回の要求で頼む大きさ(Range の幅)
    pub segment: u64,
}

impl Default for FetchOptions<'_> {
    fn default() -> Self {
        FetchOptions {
            validate: Some(&validate_with_index),
            lite_base: LITE_BASE.into(),
            g41_base: G41_BASE.into(),
            head_workers: 8,
            retries: 5,
            retry_unit: Duration::from_secs(1),
            segment: 8 << 20,
        }
    }
}

impl FetchOptions<'_> {
    fn url(&self, name: &str) -> Option<String> {
        parse_index_name(name).map(|(s, t)| url_with(s, t, &self.lite_base, &self.g41_base))
    }
}

/// [`fetch_index`] の結果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchResult {
    /// 落とすと決めたもの(既にあるものを除く)が全部そろった
    pub ok: bool,
    /// 中止した
    pub cancelled: bool,
    /// これから落とすと決めたファイルの数
    pub planned: usize,
    /// そのうち落とし終えた数
    pub done: usize,
    /// 既にそろっていて飛ばした数
    pub already: usize,
    /// 落とせなかったファイル名
    pub failed: Vec<String>,
    /// 落とす予定だった大きさ [byte]
    pub total_bytes: u64,
}

/// 落とす予定のファイルの実サイズを HEAD で調べる(8 本並列)。(名前, 大きさ or None) を入力の順で返す。
///
/// 表に持っている目安(SCALE_MB)は 6% ほどずれていて、進み具合が 100% に届かなかったり超えたりする。
/// 実物を聞いてから始める。`stop()` が真になったら、そこまでに分かったぶんだけ返す。
pub fn remote_sizes(names: &[String], stop: &dyn Fn() -> bool) -> Vec<(String, Option<u64>)> {
    let opts = FetchOptions::default();
    let urls: Vec<(String, String)> = names
        .iter()
        .map(|n| (n.clone(), opts.url(n).unwrap_or_else(|| index_url(n))))
        .collect();
    remote_sizes_urls(&urls, opts.head_workers, stop)
}

/// (名前, URL) の組で HEAD を並列に投げる。
pub fn remote_sizes_urls(jobs: &[(String, String)], workers: usize, stop: &dyn Fn() -> bool) -> Vec<(String, Option<u64>)> {
    let n = jobs.len();
    let urls: Arc<Vec<String>> = Arc::new(jobs.iter().map(|j| j.1.clone()).collect());
    let next = Arc::new(AtomicUsize::new(0));
    let abort = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<(usize, Option<u64>)>();
    for _ in 0..workers.max(1).min(n.max(1)) {
        let (urls, next, abort, tx) = (urls.clone(), next.clone(), abort.clone(), tx.clone());
        // 中止したら待たずに戻れるよう、終わりを待たないスレッドにする(残りは HEAD の上限 60 秒で終わる)
        std::thread::spawn(move || {
            let agent = http::agent(Duration::from_secs(60), true);
            loop {
                if abort.load(Ordering::Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= urls.len() {
                    break;
                }
                let size = http::head_size(&agent, &urls[i]).ok();
                if tx.send((i, size)).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut got: Vec<Option<Option<u64>>> = vec![None; n];
    let mut count = 0;
    while count < n {
        if stop() {
            abort.store(true, Ordering::Relaxed);
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok((i, s)) => {
                if got[i].is_none() {
                    count += 1;
                }
                got[i] = Some(s);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    jobs.iter()
        .zip(got)
        .filter_map(|(j, g)| g.map(|s| (j.0.clone(), s)))
        .collect()
}

/// 選んだ索引ファイルを `dest_dir` へ落とす(Python の `fetch_index`)。既定の取得元・軽い検査だけ。
///
/// `progress(done, total)` が呼ばれる(byte)。`stop()` が真を返したら止める。
pub fn fetch_index(
    names: &[String],
    dest_dir: &Path,
    progress: &dyn Fn(u64, u64),
    stop: &dyn Fn() -> bool,
    log: &dyn Fn(&str),
) -> FetchResult {
    fetch_index_with(names, dest_dir, &FetchOptions::default(), progress, stop, log)
}

/// [`fetch_index`] の設定つき版。
///
/// 数え方をひとつに揃えてある ―― 見出しの「N ファイル」、途中の [i/N]、最後の「完了 M/N」は、
/// すべて<b>これから落とすぶん</b>の数。書き込みは「名前.part」へ。最後まで落ちて大きさが合い、
/// ヘッダーの検査に通ってから本来の名前へ付け替える。
pub fn fetch_index_with(
    names: &[String],
    dest_dir: &Path,
    opts: &FetchOptions,
    progress: &dyn Fn(u64, u64),
    stop: &dyn Fn() -> bool,
    log: &dyn Fn(&str),
) -> FetchResult {
    let mut res = FetchResult::default();
    if let Err(e) = std::fs::create_dir_all(dest_dir) {
        log(&format!("  {} を作れません ({e})", dest_dir.display()));
        res.failed = names.to_vec();
        return res;
    }
    let mut jobs: Vec<(String, String, u32)> = Vec::new();
    for n in names {
        match (parse_index_name(n), opts.url(n)) {
            (Some((s, _)), Some(url)) => jobs.push((n.clone(), url, s)),
            _ => {
                log(&format!("    {n}: 索引ファイルの名前ではありません"));
                res.failed.push(n.clone());
            }
        }
    }

    log("  必要な容量を調べています...");
    let url_pairs: Vec<(String, String)> = jobs.iter().map(|j| (j.0.clone(), j.1.clone())).collect();
    let sizes: HashMap<String, Option<u64>> = remote_sizes_urls(&url_pairs, opts.head_workers, stop).into_iter().collect();
    if stop() {
        log("  中止しました。");
        res.cancelled = true;
        return res;
    }

    let mut todo: Vec<(String, String, Option<u64>)> = Vec::new();
    let mut total: u64 = 0;
    for (name, url, scale) in &jobs {
        let path = dest_dir.join(name);
        let part = part_path(&path);
        let mut got = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let done_file;
        if got == 0 && part.exists() {
            got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0); // 途中まで落ちているぶん
            done_file = false;
        } else {
            done_file = got > 0;
        }
        let want = sizes.get(name).copied().flatten();
        let Some(want) = want else {
            // 大きさを聞けなかった。あるなら完成しているとみなす
            if done_file {
                res.already += 1;
                continue;
            }
            todo.push((name.clone(), url.clone(), None));
            total += (scale_mb(*scale).unwrap_or(100) as f64 * 1024.0 * 1024.0 / index_names(*scale).len() as f64) as u64;
            continue;
        };
        if done_file && got == want {
            if check_index_file(&path, opts.validate).is_ok() {
                res.already += 1;
                continue;
            }
            got = 0; // 中身がおかしいので最初から落とし直す
        }
        todo.push((name.clone(), url.clone(), Some(want)));
        total += if 0 < got && got < want { want - got } else { want };
    }
    res.planned = todo.len();
    res.total_bytes = total;

    if todo.is_empty() {
        log(&format!("  すべて落ちています ({} ファイル)。", res.already));
        res.ok = res.failed.is_empty();
        return res;
    }
    let mut msg = format!("  {} ファイル / {} を {} へ落とします。", todo.len(), human(total as f64), dest_dir.display());
    if res.already > 0 {
        msg += &format!("  (既にある {} ファイルは飛ばします)", res.already);
    }
    log(&msg);
    if let Ok(free) = fs4::available_space(dest_dir)
        && (free as f64) < total as f64 * 1.05 + (64u64 << 20) as f64
    {
        log(&format!(
            "  空き容量が足りません (要 {} / 空き {})。置き場所を変えるか、ディスクを空けてください。",
            human(total as f64),
            human(free as f64)
        ));
        res.failed.extend(todo.iter().map(|t| t.0.clone()));
        return res;
    }

    let done = std::cell::Cell::new(0i64);
    let mut account = |n: i64| {
        done.set((done.get() + n).max(0));
        progress(done.get() as u64, total);
    };
    let head_agent = http::agent(Duration::from_secs(60), true);
    let n_todo = todo.len();
    for (i, (name, url, want)) in todo.into_iter().enumerate() {
        if stop() {
            log("  中止しました。");
            res.cancelled = true;
            return res;
        }
        let path = dest_dir.join(&name);
        match download_one(&url, &path, want, &head_agent, opts, &mut account, stop, log) {
            Ok(true) => {
                res.done += 1;
                let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                log(&format!("    [{}/{}] {name}  {}", i + 1, n_todo, human(sz as f64)));
            }
            Ok(false) => res.failed.push(name),
            Err(_) => {
                log("  中止しました。");
                res.cancelled = true;
                return res;
            }
        }
    }
    log(&format!("  完了: {}/{} ファイル", res.done, n_todo));
    res.ok = res.done == n_todo && res.failed.is_empty();
    res
}

fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

/// 1 ファイル落とす(Python の `_download`)。Ok(true) = 揃った、Ok(false) = 失敗、Err = 中止。
#[allow(clippy::too_many_arguments)]
fn download_one(
    url: &str,
    path: &Path,
    known: Option<u64>,
    head_agent: &ureq::Agent,
    opts: &FetchOptions,
    account: &mut dyn FnMut(i64),
    stop: &dyn Fn() -> bool,
    log: &dyn Fn(&str),
) -> Result<bool, NetError> {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let part = part_path(path);
    let total = match known {
        Some(t) => t,
        None => match http::head_size(head_agent, url) {
            Ok(t) => t,
            Err(e) => {
                log(&format!("    {name}: 大きさを取れません ({e})"));
                return Ok(false);
            }
        },
    };
    if path.exists() {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size == total {
            match check_index_file(path, opts.validate) {
                Ok(()) => return Ok(true),
                Err(why) => {
                    log(&format!("    {name}: 中身がおかしいので落とし直します ({why})"));
                    if let Err(e) = std::fs::remove_file(path) {
                        log(&format!("    {name}: 消せません ({e})"));
                        return Ok(false);
                    }
                }
            }
        } else {
            // v6 が残した途中のファイル → 続きから
            let r = if part.exists() { std::fs::remove_file(path) } else { std::fs::rename(path, &part) };
            if let Err(e) = r {
                log(&format!("    {name}: 続きから落とす準備ができません ({e})"));
                return Ok(false);
            }
        }
    }
    let ropts = ResumeOpts {
        segment: opts.segment,
        retries: opts.retries,
        retry_unit: opts.retry_unit,
        ..ResumeOpts::default()
    };
    match http::resume_download(url, &part, Some(total), &ropts, account, stop, log, &name) {
        Ok(()) => Ok(finish(&part, path, opts, log)),
        Err(NetError::Cancelled) => Err(NetError::Cancelled),
        Err(NetError::Failed(_)) => Ok(false),
    }
}

/// 落とし終えた .part を本来の名前へ付け替えて、ヘッダーを検査する。
fn finish(part: &Path, path: &Path, opts: &FetchOptions, log: &dyn Fn(&str)) -> bool {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Err(e) = std::fs::rename(part, path) {
        log(&format!(
            "    {name}: 名前を付け替えられません ({e})。解析で使っている最中なら、終わってからもう一度押してください。"
        ));
        return false;
    }
    match check_index_file(path, opts.validate) {
        Ok(()) => true,
        Err(why) => {
            log(&format!("    {name}: 落としたファイルの中身がおかしいので消します ({why})。"));
            let _ = std::fs::remove_file(path);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_urls() {
        assert_eq!(index_names(4107), vec!["index-4107.fits"]);
        let n = index_names(5206);
        assert_eq!(n.len(), 48);
        assert_eq!(n[7], "index-5206-07.fits");
        assert_eq!(index_url("index-5206-07.fits"), format!("{LITE_BASE}/index-5206-07.fits"));
        assert_eq!(index_url("index-4119.fits"), format!("{G41_BASE}/index-4119.fits"));
        assert_eq!(parse_index_name("index-5206-7.fits"), None);
        assert_eq!(parse_index_name("index-4107.fits.part"), None);
    }

    #[test]
    fn human_units() {
        assert_eq!(human(512.0), "512.0 B");
        assert_eq!(human(1023.96), "1,024.0 B");
        assert_eq!(human(1536.0), "1.5 KB");
        assert_eq!(human(9_802_886_400.0), "9.1 GB");
    }
}

//! 目標の座標と機材ごとの画素スケールを覚えておくファイル(cache.json)。
//!
//! 置き場所は v6 と同じ `%LOCALAPPDATA%\zahyou\cache.json`(環境変数 ZAHYOU_CACHE で上書き)。
//! 形も同じ: `{"targets": {正規化した名前: {"ra", "dec", "name"}}, "rigs": {機材の鍵: ″/px}}`。
//! 書き方も Python の `json.dump(cache, f, ensure_ascii=False, indent=1)` とバイト単位で同じにする。
//! 知らないキーは読んだまま残す。
//!
//! 以前は Temp に置いていたが、そこは「ディスク クリーンアップ」やストレージセンサーが消しにいく場所だった。
//! 覚えた座標は「一度オンラインで引いた天体名を、山の中でもそのまま使う」ための命綱なので、消えない場所へ移した。
//! 前の場所(Temp の zahyou_cache.json)にファイルが残っていれば、一度だけ引き継ぐ。

use crate::pyfmt::py_json;
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 読み・足し・書きを 1 つずつにする(同じプロセスの別スレッドどうし)。
static LOCK: Mutex<()> = Mutex::new(());

/// [`override_path`] で決めた場所。
static OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// このプロセスの cache.json を別のファイルにする(起動直後に 1 回だけ。2 回目からは何もしない)。
/// 環境変数 ZAHYOU_CACHE より優先する。自己診断が利用者の記憶を読み書きしないため(app の `--selftest`)。
pub fn override_path(p: PathBuf) {
    let _ = OVERRIDE.set(p);
}

/// cache.json の場所(Python の `_cache_path`)。必要ならフォルダーを作り、Temp の古いファイルを引き継ぐ。
/// [`override_path`] で決めてあればそのファイル(フォルダーも作らず、引き継ぎもしない)。
pub fn cache_path() -> PathBuf {
    if let Some(p) = OVERRIDE.get() {
        return p.clone();
    }
    cache_path_from(
        std::env::var_os("ZAHYOU_CACHE"),
        std::env::var_os("LOCALAPPDATA"),
        home_dir(),
        std::env::temp_dir(),
    )
}

fn home_dir() -> Option<OsString> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
}

/// [`cache_path`] の中身(環境変数を引数で受ける。試験用)。
pub fn cache_path_from(
    override_path: Option<OsString>,
    local_appdata: Option<OsString>,
    home: Option<OsString>,
    temp: PathBuf,
) -> PathBuf {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let base = local_appdata
        .filter(|p| !p.is_empty())
        .or(home)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let old_path = temp.join("zahyou_cache.json");
    let d = base.join("zahyou");
    if std::fs::create_dir_all(&d).is_err() {
        return old_path;
    }
    let new_path = d.join("cache.json");
    if !new_path.exists() && old_path.exists() && std::fs::copy(&old_path, &new_path).is_err() {
        return old_path;
    }
    new_path
}

/// 読む。無い・壊れている・オブジェクトでない ときは空のオブジェクト(Python の `_cache_load`)。
pub fn load_cache_at(path: &Path) -> Map<String, Value> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

/// Python と同じ書き方の文字列にする(`ensure_ascii=False, indent=1`)。
pub fn cache_to_string(cache: &Map<String, Value>) -> String {
    py_json(&Value::Object(cache.clone()), Some(1), false)
}

/// 書く(一時ファイルに書いてから付け替える)。
pub fn save_cache_at(path: &Path, cache: &Map<String, Value>) -> Result<(), String> {
    let text = cache_to_string(cache);
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// 天体名の鍵(Python の `" ".join(str(name).split()).upper()`)。
pub fn target_key(name: &str) -> String {
    // Python の str.split() は Unicode の空白に加え \x1c〜\x1f でも区切る
    name.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// 覚えている目標。
#[derive(Clone, Debug, PartialEq)]
pub struct CachedTarget {
    pub ra: f64,
    pub dec: f64,
    /// 引いたときに入れられた名前(そのまま)
    pub name: String,
}

/// 覚えている目標を引く(`cache` は [`load_cache_at`] の戻り値)。
pub fn lookup_target(cache: &Map<String, Value>, name: &str) -> Option<CachedTarget> {
    let hit = cache.get("targets")?.get(target_key(name))?;
    Some(CachedTarget {
        ra: hit.get("ra")?.as_f64()?,
        dec: hit.get("dec")?.as_f64()?,
        name: hit.get("name").and_then(|v| v.as_str()).unwrap_or(name).to_string(),
    })
}

/// 既定の cache.json から目標を引く。
pub fn cached_target(name: &str) -> Option<CachedTarget> {
    lookup_target(&load_cache_at(&cache_path()), name)
}

/// 目標を覚える(`path` の cache.json に足して書く)。
pub fn remember_target_at(path: &Path, name: &str, ra: f64, dec: f64) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cache = load_cache_at(path);
    let mut entry = Map::new();
    entry.insert("ra".into(), num(ra));
    entry.insert("dec".into(), num(dec));
    entry.insert("name".into(), Value::String(name.to_string()));
    if let Some(t) = section(&mut cache, "targets") {
        t.insert(target_key(name), Value::Object(entry));
    }
    save_cache_at(path, &cache)
}

/// 既定の cache.json に目標を覚える。
pub fn remember_target(name: &str, ra: f64, dec: f64) -> Result<(), String> {
    remember_target_at(&cache_path(), name, ra, dec)
}

/// 機材の鍵(Python の `rig_key`)。引数はヘッダーの値を Python の `str()` にしたもの
/// (cam は INSTRUME→CAMID→CAMERA→"?" の最初に空でないもの、bin は XBINNING→BINX→1、focal は FOCALLEN か "")。
pub fn rig_key(cam: &str, tel: &str, nx: usize, ny: usize, bin: &str, focal: &str) -> String {
    format!("{}|{}|{nx}x{ny}|bin{bin}|f{focal}", cam.trim(), tel.trim())
}

/// 前に同じ機材で解いたときの実測スケール [″/px]。
pub fn remembered_scale_at(path: &Path, rig: &str) -> Option<f64> {
    load_cache_at(path).get("rigs")?.get(rig)?.as_f64()
}

/// 既定の cache.json から機材のスケールを引く。
pub fn remembered_scale(rig: &str) -> Option<f64> {
    remembered_scale_at(&cache_path(), rig)
}

/// 解けたので、この機材の画素スケールを覚えておく(0.01〜3600 ″/px の外は覚えない)。
pub fn remember_scale_at(path: &Path, rig: &str, scale_arcsec: f64) -> Result<(), String> {
    if !(scale_arcsec > 0.01 && scale_arcsec < 3600.0) {
        return Ok(());
    }
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cache = load_cache_at(path);
    if let Some(r) = section(&mut cache, "rigs") {
        r.insert(rig.to_string(), num(scale_arcsec));
    }
    save_cache_at(path, &cache)
}

/// 既定の cache.json に機材のスケールを覚える。
pub fn remember_scale(rig: &str, scale_arcsec: f64) -> Result<(), String> {
    remember_scale_at(&cache_path(), rig, scale_arcsec)
}

/// `cache[key]` をオブジェクトとして取り出す(無い・オブジェクトでなければ作り直す。Python の setdefault に近い)。
/// 直前でオブジェクトにするので None にはならない(panic の道を作らないために Option で返す)。
fn section<'a>(cache: &'a mut Map<String, Value>, key: &str) -> Option<&'a mut Map<String, Value>> {
    let v = cache.entry(key.to_string()).or_insert_with(|| Value::Object(Map::new()));
    if !v.is_object() {
        *v = Value::Object(Map::new());
    }
    v.as_object_mut()
}

fn num(x: f64) -> Value {
    serde_json::Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
}

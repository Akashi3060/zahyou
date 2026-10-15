//! 試験の共通の道具: 星図データと基準値(JSON)の場所、SKIP の出し方、sha256。
#![allow(dead_code)]

use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// 星図データの置き場所(環境変数 ZAHYOU_INDEX_DIR で変えられる)。
pub fn index_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_INDEX_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\AstrometryData"))
}

/// 索引 1 本のパス(無ければ None)。
pub fn index_path(tag: &str) -> Option<PathBuf> {
    let p = index_dir().join(format!("index-{tag}.fits"));
    p.is_file().then_some(p)
}

/// 照合用の値の置き場所(rs/testdata。環境変数 ZAHYOU_TESTDATA で変えられる)。
pub fn testdata_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("testdata"))
}

/// 基準値の JSON を読む(無ければ None)。
pub fn load_json(name: &str) -> Option<Value> {
    let p = testdata_dir().join(name);
    let s = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str(&s).ok()
}

/// SKIP を出す(試験は通す)。
pub fn skip(name: &str, why: &str) {
    eprintln!("SKIP {name}: {why}");
}

/// u32 の並びを '<u4' のバイト列にした sha256(16 進)。Python の sha_u32 と同じ。
pub fn sha_u32(v: impl IntoIterator<Item = u32>) -> String {
    let mut h = Sha256::new();
    for x in v {
        h.update(x.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 実数の並び。入れ子の配列([[x, y, z], …])は行優先で平らにする。
pub fn f64s(v: &Value) -> Vec<f64> {
    fn walk(v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            x => out.push(x.as_f64().unwrap_or(f64::NAN)),
        }
    }
    let mut out = Vec::new();
    if v.is_array() {
        walk(v, &mut out);
    }
    out
}

pub fn u64s(v: &Value) -> Vec<u64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_u64().unwrap_or(u64::MAX)).collect()).unwrap_or_default()
}

pub fn vec3(v: &Value) -> [f64; 3] {
    let a = f64s(v);
    [a[0], a[1], a[2]]
}

/// 星図データと基準値の両方があるときだけ値を返す。どちらかが無ければ SKIP と出して None。
pub fn need(name: &str, json: &str, tags: &[&str]) -> Option<Value> {
    if !index_dir().is_dir() {
        skip(name, &format!("星図データが無い({})", index_dir().display()));
        return None;
    }
    for t in tags {
        if index_path(t).is_none() {
            skip(name, &format!("index-{t}.fits が無い"));
            return None;
        }
    }
    match load_json(json) {
        Some(v) => Some(v),
        None => {
            skip(name, &format!("基準値 {json} が無い({})", testdata_dir().display()));
            None
        }
    }
}

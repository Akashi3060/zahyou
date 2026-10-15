//! 試験の共通部品: rs/testdata の JSON を読む、gen_image.py と同じハッシュ・擬似乱数・base64。

#![allow(dead_code)]

use std::io::Read;
use std::path::PathBuf;

pub use serde_json::Value;

pub fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("testdata").join(name)
}

pub fn load_json(name: &str) -> Value {
    let p = testdata(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} を読めません: {e}", p.display()));
    serde_json::from_str(&s).expect("JSON")
}

/// Python 版の dev フォルダー(環境変数 ZAHYOU_DEV で変えられる)。
pub fn zahyou_dev() -> PathBuf {
    std::env::var_os("ZAHYOU_DEV")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\yoshi\Documents\GitHub\zahyou\dev"))
}

/// 試験用の一時フォルダー(target の下)。
pub fn tmpdir(sub: &str) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("zahyou-image-tests").join(sub);
    std::fs::create_dir_all(&d).expect("一時フォルダー");
    d
}

const K1: u64 = 0x9E3779B97F4A7C15;
const K2: u64 = 0x632BE59BD9B4E019;

/// gen_image.py の ahash(float32。NaN は 0x7fc00000 にそろえる)。
pub fn ahash_f32(a: &[f32]) -> String {
    let mut h: u64 = 0;
    for (i, &v) in a.iter().enumerate() {
        let b = if v.is_nan() { 0x7FC0_0000u32 } else { v.to_bits() } as u64;
        h = h.wrapping_add((b + 1).wrapping_mul((i as u64).wrapping_mul(K1).wrapping_add(K2)));
    }
    format!("{h:016x}")
}

/// gen_image.py の ahash(bool・uint8)。
pub fn ahash_u8(a: impl Iterator<Item = u8>) -> String {
    let mut h: u64 = 0;
    for (i, b) in a.enumerate() {
        h = h.wrapping_add((b as u64 + 1).wrapping_mul((i as u64).wrapping_mul(K1).wrapping_add(K2)));
    }
    format!("{h:016x}")
}

/// gen_image.py の synth と同じ擬似乱数の float32 列。
pub fn synth(n: usize, seed: u64, center: f64, spread: f64, outlier_every: u64) -> Vec<f32> {
    (0..n as u64)
        .map(|i| {
            let mut u = (i.wrapping_mul(0x9E3779B1).wrapping_add(seed.wrapping_mul(0x85EBCA77))) & 0xFFFF_FFFF;
            u ^= u >> 15;
            u = (u.wrapping_mul(0x2C1B3C6D)) & 0xFFFF_FFFF;
            u ^= u >> 12;
            u = (u.wrapping_mul(0x297A2D39)) & 0xFFFF_FFFF;
            u ^= u >> 15;
            let mut v = center + spread * (u as f64 / 4294967296.0 - 0.5);
            if outlier_every > 0 && u % outlier_every == 0 {
                v += (u % 5000) as f64;
            }
            v as f32
        })
        .collect()
}

pub fn b64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let Some(v) = val(c) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

pub fn b64_f32(s: &str) -> Vec<f32> {
    b64(s).chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn gunzip(b: &[u8]) -> Vec<u8> {
    let mut d = flate2::read::GzDecoder::new(b);
    let mut out = Vec::new();
    d.read_to_end(&mut out).expect("gzip");
    out
}

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

/// gen_image.py の stats と同じもの。
pub struct Stats {
    pub hash: String,
    pub sum: f64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub n_nonfinite: usize,
}

pub fn stats(a: &[f32]) -> Stats {
    let fin: Vec<f64> = a.iter().filter(|v| v.is_finite()).map(|&v| v as f64).collect();
    let sum: f64 = fin.iter().sum();
    Stats {
        hash: ahash_f32(a),
        sum,
        mean: if fin.is_empty() { 0.0 } else { sum / fin.len() as f64 },
        min: fin.iter().cloned().fold(f64::INFINITY, f64::min),
        max: fin.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        n_nonfinite: a.len() - fin.len(),
    }
}

/// Python の stats(JSON)と比べる。ハッシュが合えば完全一致。合わなければ違いを文で返す。
pub fn cmp_stats(label: &str, py: &Value, shape: (usize, usize), a: &[f32]) -> Option<String> {
    let s = stats(a);
    let ps: Vec<usize> = py["shape"]
        .as_array()
        .map(|v| v.iter().map(|x| x.as_u64().unwrap_or(0) as usize).collect())
        .unwrap_or_default();
    if ps != vec![shape.0, shape.1] {
        return Some(format!("{label}: 形 {ps:?} ≠ {shape:?}"));
    }
    if py["hash"].as_str() == Some(s.hash.as_str()) {
        return None;
    }
    Some(format!(
        "{label}: ハッシュ不一致 (sum {} vs {}, mean {} vs {}, min {} vs {}, max {} vs {}, 非有限 {} vs {})",
        f(&py["sum"]),
        s.sum,
        f(&py["mean"]),
        s.mean,
        f(&py["min"]),
        s.min,
        f(&py["max"]),
        s.max,
        py["n_nonfinite"],
        s.n_nonfinite
    ))
}

pub fn rel(a: f64, b: f64) -> f64 {
    if a == b {
        return 0.0;
    }
    (a - b).abs() / a.abs().max(b.abs()).max(1e-300)
}

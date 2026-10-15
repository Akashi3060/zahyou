//! 読み込みの照合(rs/testdata/image_formats.json、gen_image.py の gen_formats)。
//!
//! 小さな合成ファイル(FITS の型・BZERO/BSCALE/BLANK・HDU の選び方・キューブ・ベイヤー・ヘッダーの書き方・gzip・
//! タイル圧縮・PNG/JPEG/TIFF)を Python の load_image_any に読ませた結果と比べる。
//! - 形・説明(note)・上下反転・検出用の面と表示用の面のハッシュ(ビット一致)・ヘッダー(キーと値)
//! - Python が読めなかったものは Rust も Err
//! - JPEG は自前の復号(libjpeg-turbo と同じ計算)でビット一致を求める(4:4:4・4:2:2・4:2:0・4:4:0・4:1:1・
//!   プログレッシブ・RST・Adobe RGB・CMYK)。image クレートに任せる形式(算術符号など)だけ平均の差を見る
//!
//! 既知の違い(試験の中で別に確かめる):
//! - HCOMPRESS_1・PLIO_1: 未対応なので Err(Python は読める)
//! - 拡張子が .png の FITS: Python は Pillow の FITS プラグインが値を化けさせる。Rust は FITS として正しく読む

mod common;

use common::*;
use zahyou_image::{CardValue, LoadError, load_image_any};

/// 既知の違いの扱い
fn known(name: &str) -> Option<&'static str> {
    match name {
        "hcompress" | "plio" => Some("unsupported"),
        "fits_named_png" => Some("fits_as_fits"),
        _ => None,
    }
}

fn header_issues(name: &str, py: &Value, cards: &[zahyou_image::Card]) -> Vec<String> {
    let mut out = Vec::new();
    let rs: Vec<&zahyou_image::Card> =
        cards.iter().filter(|c| !matches!(c.key.as_str(), "" | "COMMENT" | "HISTORY" | "CONTINUE")).collect();
    let pyl = py.as_array().unwrap();
    if pyl.len() != rs.len() {
        out.push(format!(
            "{name}: ヘッダーのカード数 {} ≠ {} (Rust: {:?})",
            pyl.len(),
            rs.len(),
            rs.iter().map(|c| c.key.as_str()).collect::<Vec<_>>()
        ));
    }
    for e in pyl {
        let e = e.as_array().unwrap();
        let key = e[0].as_str().unwrap();
        let kind = e[1].as_str().unwrap();
        let Some(c) = rs.iter().find(|c| c.key == key) else {
            out.push(format!("{name}: キー {key} が無い"));
            continue;
        };
        let ok = match (kind, &c.value) {
            ("num", v) => match v {
                CardValue::Int(i) => *i as f64 == f(&e[2]),
                CardValue::Float(x) => *x == f(&e[2]),
                _ => false,
            },
            ("str", CardValue::Str(s)) => Some(s.as_str()) == e[2].as_str(),
            ("bool", CardValue::Bool(b)) => Some(*b) == e[2].as_bool(),
            ("other", CardValue::None) => true,
            _ => false,
        };
        if !ok {
            out.push(format!("{name}: {key} の値 {:?} ≠ {:?}", c.value, e[2]));
        }
    }
    out
}

#[test]
fn formats_match_python() {
    let j = load_json("image_formats.json");
    let dir = tmpdir("formats");
    let fixtures = j["fixtures"].as_array().unwrap();
    let mut issues: Vec<String> = Vec::new();
    let mut n_exact = 0;
    let mut n_err_ok = 0;
    let mut jpeg_notes = Vec::new();
    // base.astype(int16) の値(拡張子が .png の FITS の正しい値。header_kinds と同じ画素)
    let i16_hash =
        fixtures.iter().find(|x| x["name"] == "header_kinds").unwrap()["data"]["hash"].as_str().unwrap().to_string();
    for fx in fixtures {
        let name = fx["name"].as_str().unwrap();
        let ext = fx["ext"].as_str().unwrap();
        let bytes = gunzip(&b64(fx["bytes_gz_b64"].as_str().unwrap()));
        assert_eq!(bytes.len() as u64, fx["size"].as_u64().unwrap(), "{name} の復元");
        let p = dir.join(format!("{name}{ext}"));
        std::fs::write(&p, &bytes).unwrap();
        let r = load_image_any(&p);
        let py_ok = fx["ok"].as_bool().unwrap();
        match (known(name), py_ok, &r) {
            (Some("unsupported"), true, Err(e)) => {
                println!("  {name:22} 既知: 未対応の圧縮 → {e}");
                continue;
            }
            (Some("fits_as_fits"), true, Ok(l)) => {
                if ahash_f32(&l.data.data) != i16_hash || l.flip_y {
                    issues.push(format!("{name}: FITS として読んだ値が header_kinds と違う"));
                } else {
                    println!(
                        "  {name:22} 既知: Python は Pillow で化けた値。Rust は FITS として読み header_kinds と一致 ({})",
                        l.note
                    );
                }
                continue;
            }
            (_, false, Err(e)) => {
                let pe = fx["error"].as_str().unwrap();
                // 自前の文(小さすぎる・一様)は文まで同じ。読めない理由の詳細は読み込み器ごとに違う
                let same_text = match e {
                    LoadError::Unreadable(_) => pe.starts_with("画像として読み込めませんでした"),
                    _ => e.to_string() == pe,
                };
                if !same_text {
                    issues.push(format!("{name}: エラー文 '{e}' ≠ '{pe}'"));
                }
                println!("  {name:22} Err (Python も Err): {e}");
                n_err_ok += 1;
                continue;
            }
            (_, false, Ok(l)) => {
                issues.push(format!("{name}: Python は読めないのに Rust は読めた ({})", l.note));
                continue;
            }
            (_, true, Err(e)) => {
                issues.push(format!("{name}: Python は読めたのに Rust は Err: {e}"));
                continue;
            }
            (_, true, Ok(_)) => {}
        }
        let l = r.unwrap();
        let shape = (l.data.height, l.data.width);
        let mut mine = Vec::new();
        if l.note != fx["note"].as_str().unwrap() && known(name) != Some("jpeg_fallback") {
            mine.push(format!("{name}: note '{}' ≠ '{}'", l.note, fx["note"].as_str().unwrap()));
        }
        if l.flip_y != fx["flip_y"].as_bool().unwrap() {
            mine.push(format!("{name}: flip_y {} ≠ {}", l.flip_y, fx["flip_y"]));
        }
        let d1 = cmp_stats(&format!("{name}.data"), &fx["data"], shape, &l.data.data);
        let d2 = cmp_stats(&format!("{name}.display"), &fx["display"], shape, &l.display.data);
        if known(name) == Some("jpeg_fallback") {
            // image クレートに任せた JPEG: 平均の差だけ見る
            let s = stats(&l.data.data);
            let dm = (s.mean - f(&fx["data"]["mean"])).abs();
            jpeg_notes.push(format!(
                "{name}: {} 平均の差 {dm:.4} (Python {:.4} / Rust {:.4})",
                if d1.is_none() { "ビット一致" } else { "不一致" },
                f(&fx["data"]["mean"]),
                s.mean
            ));
            if dm > 5.0 {
                mine.push(format!("{name}: JPEG の平均の差 {dm} が大きすぎる"));
            }
        } else {
            mine.extend(d1);
            mine.extend(d2);
        }
        mine.extend(header_issues(name, &fx["header"], &l.header));
        if mine.is_empty() {
            n_exact += 1;
            println!("  {name:22} 一致  {}", l.note);
        } else {
            for m in &mine {
                println!("  ✗ {m}");
            }
            issues.extend(mine);
        }
    }
    for n in &jpeg_notes {
        println!("  JPEG {n}");
    }
    println!("一致 {n_exact} 件、Err の一致 {n_err_ok} 件、食い違い {} 件 / {} 件", issues.len(), fixtures.len());
    assert!(issues.is_empty(), "食い違い:\n{}", issues.join("\n"));
}

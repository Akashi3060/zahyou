//! 壊れたファイルで panic しないこと(Err を返すこと)の試験。
//!
//! 元にするのは image_formats.json の合成ファイル(手元の画像が無くても回る)。
//! - FITS・PNG・TIFF・JPEG・gzip を途中で切る(多数の位置)
//! - FITS のヘッダーの数字を壊す(Python 版が Err にする壊し方は Err になることも確かめる)
//! - タイル圧縮の表とヒープのバイトを 1 つずつ反転する

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use common::*;
use zahyou_image::{DetectOpts, detect_sources, detect_sources_deep, load_image_any, preprocess};

fn fixture(name: &str) -> (Vec<u8>, String) {
    let j = load_json("image_formats.json");
    let fx = j["fixtures"].as_array().unwrap().iter().find(|x| x["name"] == name).unwrap_or_else(|| panic!("{name}"));
    (gunzip(&b64(fx["bytes_gz_b64"].as_str().unwrap())), fx["ext"].as_str().unwrap().to_string())
}

/// 読ませて、panic したら Err("panic") を返す。読めたら前処理と検出も通す。
fn try_load(dir: &Path, tag: &str, bytes: &[u8], ext: &str) -> Result<String, String> {
    let p = dir.join(format!("{tag}{ext}"));
    std::fs::write(&p, bytes).unwrap();
    let r = catch_unwind(AssertUnwindSafe(|| match load_image_any(&p) {
        Ok(l) => {
            let prep = preprocess(&l.data, true);
            let _ = detect_sources(&prep, &DetectOpts::default());
            let _ = detect_sources_deep(&prep, &DetectOpts::deep());
            Ok(l.note)
        }
        Err(e) => Err(e.to_string()),
    }));
    match r {
        Ok(x) => x,
        Err(_) => Err("panic".to_string()),
    }
}

/// FITS のヘッダーの 1 枚のカードを書き換える(80 文字にそろえる)。
fn set_card(b: &[u8], key: &str, card: &str) -> Vec<u8> {
    let mut out = b.to_vec();
    let mut i = 0;
    while i + 80 <= out.len().min(2880 * 4) {
        if out[i..i + 8].starts_with(key.as_bytes()) && (key.len() == 8 || out[i + key.len()] == b' ') {
            let mut c = card.as_bytes().to_vec();
            c.resize(80, b' ');
            out[i..i + 80].copy_from_slice(&c);
            return out;
        }
        i += 80;
    }
    panic!("{key} が無い");
}

#[test]
fn truncated_files_do_not_panic() {
    let dir = tmpdir("broken-trunc");
    let mut panics = Vec::new();
    let mut n = 0;
    for name in [
        "u16",
        "f32_nan",
        "ext_after_table",
        "gz_u16",
        "rice_i32",
        "rice_f32_dither1",
        "gzip2_f32_lossless",
        "gzip1_f64",
        "nocomp_i16",
        "png_rgb8",
        "png_gray16",
        "tif_gray16_le",
        "tif_rgb16",
        "tif_gray16_deflate",
        "jpg_rgb",
        "jpg_prog",
        "jpg_prog_rst",
        "jpg_rst",
        "jpg_cmyk",
        "jpg_411",
        "jpg_gray_prog",
        "video5",
    ] {
        let (b, ext) = fixture(name);
        let step = (b.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..b.len()).step_by(step).collect();
        cuts.extend([1, 7, 79, 80, 2879, 2880, 2881, 5759, 5760, 5761, b.len().saturating_sub(1)]);
        for cut in cuts.into_iter().filter(|&c| c < b.len()) {
            n += 1;
            if try_load(&dir, &format!("{name}_cut{cut}"), &b[..cut], &ext) == Err("panic".into()) {
                panics.push(format!("{name} を {cut} バイトで切ったもの"));
            }
        }
    }
    println!("途中で切ったファイル {n} 個を読ませた。panic {} 個", panics.len());
    assert!(panics.is_empty(), "panic:\n{}", panics.join("\n"));
}

#[test]
fn broken_fits_headers() {
    let dir = tmpdir("broken-header");
    let (b, _) = fixture("u16"); // 32×40 の uint16、データ 2560 バイト
    let data_end = 2880 + 32 * 40 * 2;
    // Python 版(astropy)が Err にする壊し方は、Rust も Err
    let must_err: Vec<(&str, Vec<u8>)> = vec![
        ("データを半分で切る", b[..2880 + 1280].to_vec()),
        ("データを 1 区切りだけ", b[..2880 + 100].to_vec()),
        ("ヘッダーの途中で切る", b[..1000].to_vec()),
        ("データの最後の 100 バイトが無い", b[..data_end - 100].to_vec()),
        ("NAXIS1 が巨大", set_card(&b, "NAXIS1", "NAXIS1  =               900000")),
        ("NAXIS1 が数でない", set_card(&b, "NAXIS1", "NAXIS1  =                  abc")),
        ("BITPIX が 12", set_card(&b, "BITPIX", "BITPIX  =                   12")),
        ("END が無い", set_card(&b, "END", "XND")),
        ("SIMPLE が無い", set_card(&b, "SIMPLE", "XIMPLE  =                    T")),
        ("NAXIS が 1000", set_card(&b, "NAXIS", "NAXIS   =                 1000")),
        ("NAXIS2 が負", set_card(&b, "NAXIS2", "NAXIS2  =                  -32")),
        ("NAXIS2 が欠けている", set_card(&b, "NAXIS2", "COMMENT NAXIS2 removed")),
        ("BZERO が文字列", set_card(&b, "BZERO", "BZERO   = 'abc'")),
        ("空のファイル", Vec::new()),
    ];
    let mut bad = Vec::new();
    for (what, bytes) in &must_err {
        match try_load(&dir, &format!("hdr_{}", bad.len() + must_err.len()), bytes, ".fits") {
            Err(e) if e != "panic" => println!("  {what}: Err {e}"),
            Err(_) => bad.push(format!("{what}: panic")),
            Ok(note) => bad.push(format!("{what}: Err になるはずが読めた ({note})")),
        }
    }
    // panic しなければよいもの
    let maybe: Vec<(&str, Vec<u8>)> = vec![
        ("NAXIS1 が 0", set_card(&b, "NAXIS1", "NAXIS1  =                    0")),
        ("NAXIS1 が 1", set_card(&b, "NAXIS1", "NAXIS1  =                    1")),
        ("NAXIS1 が i64 の最大", set_card(&b, "NAXIS1", "NAXIS1  =  9223372036854775807")),
        ("BSCALE が 0", set_card(&b, "BSCALE", "BSCALE  =                  0.0")),
        ("BLANK が文字列", set_card(&b, "BSCALE", "BLANK   = 'xyz'")),
        ("ROWORDER が数", set_card(&b, "BSCALE", "ROWORDER=                    5")),
        ("BAYERPAT が数", set_card(&b, "BSCALE", "BAYERPAT=                    0")),
        ("後ろにごみ", [b.clone(), b"garbage after the last HDU".repeat(200)].concat()),
    ];
    for (k, (what, bytes)) in maybe.iter().enumerate() {
        match try_load(&dir, &format!("maybe_{k}"), bytes, ".fits") {
            Err(e) if e == "panic" => bad.push(format!("{what}: panic")),
            r => println!("  {what}: {r:?}"),
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn corrupted_compressed_tiles() {
    let dir = tmpdir("broken-tile");
    let mut panics = Vec::new();
    let mut n = 0;
    let mut n_err = 0;
    for name in ["rice_i32", "rice_i16_tiles", "rice_u8", "rice_f32_dither2", "gzip2_i16", "gzip1_i32", "nocomp_i16"] {
        let (b, ext) = fixture(name);
        // 2 つ目の HDU(BINTABLE)のデータ部 = 先頭の空の HDU とそのヘッダーの後ろ
        let start = 2880 * 2;
        let stride = ((b.len() - start) / 80).max(1);
        for pos in (start..b.len()).step_by(stride) {
            for x in [0xFFu8, 0x10] {
                let mut c = b.clone();
                c[pos] ^= x;
                n += 1;
                match try_load(&dir, &format!("{name}_flip"), &c, &ext) {
                    Err(e) if e == "panic" => panics.push(format!("{name} の {pos} バイト目を ^{x:02x}")),
                    Err(_) => n_err += 1,
                    Ok(_) => {}
                }
            }
        }
        // 圧縮のキーを壊す
        for (k, card) in [
            "ZNAXIS1 =            999999999",
            "ZTILE1  =                    0",
            "ZBITPIX =                   12",
            "ZCMPTYPE= 'NOPE    '",
            "ZNAXIS  =                   -1",
            "TFORM1  = 'Z'",
        ]
        .iter()
        .enumerate()
        {
            let key = &card[..8].trim_end().to_string();
            let found = b[2880..5760].chunks(80).any(|c| c.starts_with(key.as_bytes()));
            if !found {
                continue;
            }
            let mut c = b.clone();
            let hdr = set_card(&c[2880..], key, card);
            c[2880..].copy_from_slice(&hdr);
            n += 1;
            match try_load(&dir, &format!("{name}_key{k}"), &c, &ext) {
                Err(e) if e == "panic" => panics.push(format!("{name}: {card}")),
                Err(_) => n_err += 1,
                Ok(_) => {}
            }
        }
    }
    println!("タイル圧縮を壊したファイル {n} 個(うち Err {n_err} 個)。panic {} 個", panics.len());
    assert!(panics.is_empty(), "panic:\n{}", panics.join("\n"));
}

#[test]
fn corrupted_jpeg_does_not_panic() {
    // 自前の JPEG 復号(ハフマン・プログレッシブの精度上げ・RST)に壊れたバイト列を読ませる
    let dir = tmpdir("broken-jpeg");
    let mut panics = Vec::new();
    let mut n = 0;
    for name in ["jpg_rgb", "jpg_prog", "jpg_prog_rst", "jpg_rst", "jpg_gray_prog", "jpg_440", "jpg_cmyk"] {
        let (b, ext) = fixture(name);
        let stride = (b.len() / 120).max(1);
        for pos in (2..b.len()).step_by(stride) {
            for x in [0xFFu8, 0x08] {
                let mut c = b.clone();
                c[pos] ^= x;
                n += 1;
                if try_load(&dir, &format!("{name}_flip"), &c, &ext) == Err("panic".to_string()) {
                    panics.push(format!("{name} の {pos} バイト目を ^{x:02x}"));
                }
            }
        }
    }
    println!("壊した JPEG {n} 個を読ませた。panic {} 個", panics.len());
    assert!(
        panics.is_empty(),
        "panic:
{}",
        panics.join(
            "
"
        )
    );
}

#[test]
fn random_bytes_do_not_panic() {
    let dir = tmpdir("broken-random");
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let heads: [&[u8]; 6] = [
        b"SIMPLE  =                    T",
        b"\x89PNG\r\n\x1a\n",
        &[0xFF, 0xD8, 0xFF],
        b"II*\0",
        b"MM\0*",
        &[0x1f, 0x8b],
    ];
    for (k, h) in heads.iter().enumerate() {
        for t in 0..20 {
            let len = (next() % 6000) as usize;
            let mut b = h.to_vec();
            b.extend((0..len).map(|_| (next() & 0xFF) as u8));
            let r = try_load(&dir, &format!("rand_{k}_{t}"), &b, if k == 0 { ".fits" } else { ".bin" });
            assert_ne!(r, Err("panic".to_string()), "先頭 {k} の乱数ファイル {t}");
        }
    }
}

#[test]
fn mismatched_image_does_not_panic() {
    // 大きさと画素の数が合わない Image2D(呼ぶ側の誤り)を渡しても落ちない
    let img = zahyou_image::Image2D { width: 40, height: 30, data: vec![1.0; 100] };
    let prep = preprocess(&img, true);
    assert!(prep.sub.data.is_empty() && !prep.info.is_empty());
    let _ = detect_sources(&prep, &DetectOpts::default());
    let _ = detect_sources_deep(&prep, &DetectOpts::deep());
    let bad = zahyou_image::Prep { sub: img, mask: vec![false; 1200], sigma: 1.0, info: vec![] };
    assert!(detect_sources(&bad, &DetectOpts::default()).0.is_empty());
    assert!(detect_sources_deep(&bad, &DetectOpts::deep()).0.is_empty());
    let _ = zahyou_image::looks_inverted(&zahyou_image::Image2D::default());
}

//! ヘッダーは正しいのに表の中身が壊れた索引(Python 版に無い、Rust で安全に読むための検査)。
//!
//! ヘッダーの検査(validate_index_file・IndexFile::open)は通り、表を mmap するとき(IndexFile::data)に
//! 文つきの Err になること、そのあと探しても panic せず空を返すことを確かめる。
//! 星図データ(C:\AstrometryData。ZAHYOU_INDEX_DIR で変えられる)が無ければ SKIP と出して通す(Windows 以外でも。表を開くのは Windows だけ)。

mod common;

use std::fs::File;
use std::path::{Path, PathBuf};

use common::*;
use zahyou_index::fits::read_hdus;
use zahyou_index::{validate_index_file, IndexError, IndexFile};

/// TTYPE1 = name の表のデータの位置と大きさ(バイト)。
fn table(p: &Path, name: &str) -> (usize, usize) {
    let mut f = File::open(p).unwrap();
    let size = f.metadata().unwrap().len();
    let (hdus, _, _) = read_hdus(&mut f, size).unwrap();
    let h = hdus.iter().find(|h| h.header.str("TTYPE1") == Some(name)).expect("表が無い");
    (h.offset as usize, h.size as usize)
}

fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// tag の索引を tmp に写し、edit で中身を書き換える。
fn broken_copy(tmp: &Path, tag: &str, label: &str, edit: impl FnOnce(&Path, &mut Vec<u8>)) -> PathBuf {
    let dir = tmp.join(label);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("index-{tag}.fits"));
    let src = index_path(tag).unwrap();
    let mut b = std::fs::read(&src).unwrap();
    edit(&src, &mut b);
    std::fs::write(&p, b).unwrap();
    p
}

#[test]
fn corrupted_tables_are_rejected_without_panic() {
    if index_path("4119").is_none() || index_path("4110").is_none() {
        skip("corrupted_tables_are_rejected_without_panic", "index-4119.fits か index-4110.fits が無い");
        return;
    }
    let tmp = temp_dir();
    let mut cases: Vec<(PathBuf, String)> = Vec::new();

    // 1. コード木の lr が途中で減る(区切りの中)
    let p = broken_copy(&tmp, "4119", "lr-dec", |src, b| {
        let (o, _) = table(src, "kdtree_lr_codes");
        let v = get_u32(b, o + 4 * 4);
        assert!(v > 0);
        put_u32(b, o + 5 * 4, v - 1);
    });
    cases.push((p, "codes の木の葉の表(lr)がおかしい".into()));

    // 2. 星の木の lr がデータの数を超える
    let p = broken_copy(&tmp, "4119", "lr-big", |src, b| {
        let (o, _) = table(src, "kdtree_lr_stars");
        put_u32(b, o, u32::MAX);
    });
    cases.push((p, "stars の木の葉の表(lr)がおかしい".into()));

    // 3. 星の木の lr の最後がデータの数 − 1 でない
    let p = broken_copy(&tmp, "4119", "lr-last", |src, b| {
        let (o, n) = table(src, "kdtree_lr_stars");
        let last = o + n - 4;
        let v = get_u32(b, last);
        assert!(get_u32(b, last - 4) < v);
        put_u32(b, last, v - 1);
    });
    cases.push((p, "stars の木の葉の表(lr)がデータの数と合いません".into()));

    // 4. 星の木(3 次元)の分割次元が 3
    let p = broken_copy(&tmp, "4119", "split-dim", |src, b| {
        let (o, n) = table(src, "kdtree_split_stars");
        let at = o + (n / 4 / 2) * 4; // 真ん中あたりの節点
        let v = get_u32(b, at);
        put_u32(b, at, v | 3);
    });
    cases.push((p, "stars の木の分割の表に次元の外の値があります".into()));

    // 5. コード木の範囲の表の scale が NaN(Python 版 SvKdTree と同じ文)
    let p = broken_copy(&tmp, "4119", "range-nan", |src, b| {
        let (o, n) = table(src, "kdtree_range_codes");
        b[o + n - 8..o + n].copy_from_slice(&f64::NAN.to_le_bytes());
    });
    cases.push((p, "codes の木の範囲の表がおかしい(scale = nan)".into()));

    // 6. lr が並列の検査の区切り(16384 個)の境目で減る(4110 のコード木は葉が 32768)
    let p = broken_copy(&tmp, "4110", "lr-boundary", |src, b| {
        let (o, n) = table(src, "kdtree_lr_codes");
        assert!(n / 4 > 16384, "葉が少なすぎて境目を試せない");
        let prev = get_u32(b, o + 16383 * 4);
        assert!(prev > 0);
        put_u32(b, o + 16384 * 4, prev - 1);
    });
    cases.push((p, "codes の木の葉の表(lr)がおかしい".into()));

    for (p, want) in &cases {
        let label = p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        // ヘッダーだけの検査は通る(Python 版と同じく中身は見ない)
        assert_eq!(validate_index_file(p), Ok(()), "{label}");
        let f = IndexFile::open(p).unwrap_or_else(|e| panic!("{label}: {e}"));
        match f.data() {
            Err(IndexError::Format(m)) => assert_eq!(&m, want, "{label}"),
            other => panic!("{label}: 期待した Err にならない({other:?})"),
        }
        // 探しても panic せず空・NaN を返す
        let d = f.code_dim();
        assert!(f.code_ball(&vec![0.5; d * 40], 0.05).is_empty(), "{label}");
        assert_eq!(f.try_code_ball(&vec![0.5; d], 0.05).unwrap_err().to_string(), *want, "{label}");
        assert!(f.stars_in_cone([1.0, 0.0, 0.0], 1.0).is_empty(), "{label}");
        assert!(f.star_xyz(0)[0].is_nan(), "{label}");
        assert_eq!(f.quad_stars(0), [u32::MAX; 4], "{label}");
        assert_eq!(f.sweep(0), u8::MAX, "{label}");
        assert!(!f.is_open(), "{label}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("表の中身が壊れた索引 {} 本が、開くときに文つきで断られた(panic なし)", cases.len());
}

fn temp_dir() -> PathBuf {
    let p = std::env::temp_dir().join(format!("zahyou-index-test-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

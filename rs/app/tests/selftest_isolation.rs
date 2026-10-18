//! exe の `--selftest` が、利用者の記憶(cache.json)と設定(gui.json)の置き場所 %LOCALAPPDATA%\zahyou に触れないこと。
//!
//! 以前の `--selftest` は gui.json だけを一時へ向け、cache.json は利用者のものを読み書きしていた(覚えた座標・画素スケールで
//! 結果が変わり、自己診断の画像の機材の画素スケールや目標の座標が利用者の記憶に足されていた)。
//! ここでは ZAHYOU_CACHE・ZAHYOU_SETTINGS を外し、LOCALAPPDATA と TEMP を試験のフォルダーへ向けて本物の exe を動かす。
//! 画像はリポジトリの小さな FITS(星が 2 つで解けない)。解けなくても、記憶の読み書きは解析の初めに起きる
//! (場所を決めるだけで %LOCALAPPDATA%\zahyou を作る)ので、それが無いことで確かめられる。
//! 外へは出さない: 方法はオフラインのみ、起動時の更新の確認はつながらないプロキシ 0.0.0.0:9 で止める。

use std::path::PathBuf;
use std::process::Command;

#[test]
fn selftest_leaves_user_cache_and_settings_alone() {
    let exe = env!("CARGO_BIN_EXE_zahyou");
    let img = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("few_two.fits");
    let base = std::env::temp_dir().join(format!("zahyou-selftest-isolation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let localappdata = base.join("localappdata");
    let tmp = base.join("tmp");
    std::fs::create_dir_all(&localappdata).unwrap();
    std::fs::create_dir_all(&tmp).unwrap();
    let out = base.join("selftest.txt");
    let proxy = "http://0.0.0.0:9";
    let status = Command::new(exe)
        .arg("--selftest")
        .arg(&img)
        .arg("--offline")
        .arg("--out")
        .arg(&out)
        .env_remove("ZAHYOU_CACHE")
        .env_remove("ZAHYOU_SETTINGS")
        .env("LOCALAPPDATA", &localappdata)
        .env("TEMP", &tmp)
        .env("TMP", &tmp)
        .env("ALL_PROXY", proxy)
        .env("HTTPS_PROXY", proxy)
        .env("HTTP_PROXY", proxy)
        .status()
        .expect("exe を起動できない");
    let report = std::fs::read_to_string(&out).unwrap_or_default();
    // 最後まで走った(合否は問わない。解けない画像なので解析の項目は落ちる)
    assert!(report.contains(" passed"), "自己診断が最後まで走っていない (rc={status:?}):\n{report}");
    assert!(
        !localappdata.join("zahyou").exists(),
        "自己診断が %LOCALAPPDATA%\\zahyou を作った(利用者の記憶か設定に触れた):\n{report}"
    );
    // 一時の記憶は終わったら消す
    let left: Vec<String> = std::fs::read_dir(&tmp)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.starts_with("zahyou-selftest-") && n.ends_with("-cache.json"))
        .collect();
    assert!(left.is_empty(), "一時の記憶が残った: {left:?}");
    let _ = std::fs::remove_dir_all(&base);
}

//! 画面を出さない入口(exe は windows サブシステムなので、結果は --out のファイルへも書く)。
//!
//!   zahyou.exe --selftest <画像> [--online|--offline] [--out f]   画面を組み立てて解析を 2 回通す
//!   zahyou.exe --engine-check [--out f]                         フォント・星図データの目録・画像の読み込みを確かめる
//!   zahyou.exe --update-check [--out f]                         更新の確認だけ(試作版は問い合わせない)
//!   zahyou.exe --bench-gui [<画像>] [--out f]                    画面まわりの速さ(JSON)

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::settings::Solve;
use crate::APP_VER;

/// 引数の値(`--out f` の f)
pub fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

pub fn has(args: &[String], key: &str) -> bool {
    args.iter().any(|a| a == key)
}

/// 標準出力と --out のファイルの両方へ書く
fn emit(args: &[String], text: &str) {
    println!("{text}");
    if let Some(p) = arg_value(args, "--out")
        && let Err(e) = std::fs::write(&p, format!("{text}\n"))
    {
        eprintln!("{p} に書けません: {e}");
    }
}

/// 自己診断。pipeline 係の項目(エンジン・星図データ・解析・数字・図・ログ・目標なし)に、
/// 画面の項目(組み立て・フォント・画面からの解析・図の貼り付け・はみ出し・下のバー・配色)を足す。
pub fn selftest(args: &[String]) -> i32 {
    let Some(image) = arg_value(args, "--selftest") else {
        emit(args, "使い方: zahyou.exe --selftest <画像> [--online|--offline] [--out 結果.txt]");
        return 1;
    };
    let mut force = if has(args, "--online") {
        Some(Solve::Online)
    } else if has(args, "--offline") {
        Some(Solve::Offline)
    } else {
        None
    };
    let debug_wcs = arg_value(args, "--debug-wcs").map(PathBuf::from);
    let target = crate::selftest::SELFTEST_TARGET;
    // 利用者の設定には触れない
    isolate_settings();
    let image = PathBuf::from(image);

    // --- 解析の項目(pipeline 係) ---
    let mut log = String::new();
    let debug = match debug_wcs.as_deref().map(super::bridge::read_wcs_file) {
        Some(Ok(w)) => Some(w),
        Some(Err(e)) => {
            emit(args, &format!("--debug-wcs の WCS を読めません: {e}"));
            return 1;
        }
        None => None,
    };
    let solver = move |_r: &zahyou_solver::OfflineRequest,
                       _c: &zahyou_index::Catalog,
                       _p: &zahyou_solver::Params,
                       _t: &zahyou_solver::Control|
          -> Result<zahyou_solver::OfflineOutcome, zahyou_solver::SolveError> {
        Ok(crate::pipeline::tried(debug.clone().map(super::bridge::debug_solution)))
    };
    let mut env = crate::pipeline::RunEnv::default();
    if debug_wcs.is_some() {
        env.offline_solver = Some(&solver);
        env.online = Some(false);
        force = Some(Solve::Offline);
    }
    let mode = force.map(|f| match f {
        Solve::Auto => crate::pipeline::SolveMode::Auto,
        Solve::Online => crate::pipeline::SolveMode::Online,
        Solve::Offline => crate::pipeline::SolveMode::Offline,
    });
    let mut rows = crate::selftest::selftest_rows(&image, mode, None, &env, &mut log);

    // --- 画面の項目 ---
    for (name, ok, msg) in super::gui_selftest::run(&image, target, force, debug_wcs) {
        rows.push(crate::selftest::Check { name, ok, msg });
    }
    let text = crate::selftest::format_report(&rows, &log);
    emit(args, &text);
    if crate::selftest::all_passed(&rows) { 0 } else { 1 }
}

/// 自己診断と撮影は、設定を一時ファイルへ向ける(利用者の gui.json を書き換えない)
pub fn isolate_settings() {
    if std::env::var_os("ZAHYOU_SETTINGS").is_none() {
        let p = std::env::temp_dir().join(format!("zahyou-selftest-{}.json", std::process::id()));
        super::settings::override_path(p);
    }
}

/// エンジンの読み込みを確かめる(Python の --engine-check に当たる)
pub fn engine_check(args: &[String]) -> i32 {
    let t0 = Instant::now();
    let mut lines = vec![
        format!("version={APP_VER}"),
        format!("executable={}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()),
    ];
    let mut ok = true;
    let (_, rep) = super::fonts::build();
    lines.push(format!("fonts={}", rep.summary()));
    if rep.ui.is_none() {
        ok = false;
        lines.push("NG 日本語のフォントがありません (豆腐になります)".into());
    }
    let dir = arg_value(args, "--index-dir").unwrap_or_else(|| {
        let s = super::settings::Settings::load();
        s.indexdir
    });
    let t1 = Instant::now();
    let cat = zahyou_index::Catalog::scan(Path::new(&dir));
    let fov = cat
        .coverage_arcmin()
        .map(|(a, b)| format!("{}′〜{}′", super::survey::g((a * 10.0).round() / 10.0), super::survey::g((b * 10.0).round() / 10.0)))
        .unwrap_or_else(|| "なし".into());
    lines.push(format!(
        "catalog={} / {} ファイル / 問題 {} / 画角 {fov} / {:.3} 秒",
        dir,
        cat.files.len(),
        cat.problems.len(),
        t1.elapsed().as_secs_f64()
    ));
    let t2 = Instant::now();
    let st = super::survey::survey(Path::new(&dir));
    lines.push(format!(
        "survey=ready {} / {} ファイル / 問題 {} / {:.3} 秒",
        st.ready,
        st.index.files,
        st.check.bad.len(),
        t2.elapsed().as_secs_f64()
    ));
    lines.push(format!("{} {:.1} 秒", if ok { "OK" } else { "NG" }, t0.elapsed().as_secs_f64()));
    emit(args, &lines.join("\n"));
    if ok { 0 } else { 1 }
}

/// 更新の確認だけ(Python の --update-check)。--update-apply なら落として入れ替える(起動し直さない)
pub fn update_cli(args: &[String]) -> i32 {
    let mut out: Vec<String> = Vec::new();
    let exe = std::env::current_exe().unwrap_or_default();
    let never = || false;
    let st = zahyou_net::check_update(&exe, APP_VER, &never);
    let state = match st.state {
        zahyou_net::UpdateState::Available => "available",
        zahyou_net::UpdateState::UpToDate => "up_to_date",
        zahyou_net::UpdateState::Dev => "dev",
        zahyou_net::UpdateState::Error => "error",
        zahyou_net::UpdateState::Cancelled => "cancelled",
    };
    out.push(format!("state   : {state}"));
    out.push(format!("message : {}", st.message));
    if let Some(r) = &st.release {
        out.push(format!("tag     : {}", r.tag));
        out.push(format!("updated : {}", r.updated));
        out.push(format!("size    : {}", r.size));
        out.push(format!("sha256  : {}", r.sha256));
    }
    if let Some(h) = &st.here_sha256 {
        out.push(format!("here_sha256: {h}"));
    }
    let mut code = if matches!(st.state, zahyou_net::UpdateState::Available | zahyou_net::UpdateState::UpToDate | zahyou_net::UpdateState::Dev) { 0 } else { 1 };
    if has(args, "--update-apply")
        && st.state == zahyou_net::UpdateState::Available
        && let Some(rel) = &st.release
    {
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let prog = |_d: u64, _t: u64| {};
        match zahyou_net::download_update(rel, &exe, &prog, &never, &log) {
            Ok(new) => match zahyou_net::apply_update(&exe, &new, None, &log) {
                Ok(_) => log("入れ替えを始めました (このプロセスは終わります)"),
                Err(e) => {
                    log(&e.to_string());
                    code = 1;
                }
            },
            Err(e) => {
                log(&e.to_string());
                code = 1;
            }
        }
        out.extend(logs.into_inner());
    }
    emit(args, &out.join("\n"));
    code
}

/// 速さの物差し(JSON)。pipeline の物差しがあればそちらも含める。
pub fn bench(args: &[String]) -> i32 {
    let image = args
        .iter()
        .position(|a| a == "--bench-gui")
        .and_then(|i| args.get(i + 1))
        .filter(|s| !s.starts_with("--"))
        .cloned();
    let mut items: Vec<(String, f64)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let t = Instant::now();
    let (_, rep) = super::fonts::build();
    items.push(("fonts_s".into(), t.elapsed().as_secs_f64()));
    notes.push(rep.summary());
    let dir = arg_value(args, "--index-dir").unwrap_or_else(|| super::settings::DEFAULT_INDEX_DIR.into());
    let t = Instant::now();
    let st = super::survey::survey(Path::new(&dir));
    items.push(("survey_s".into(), t.elapsed().as_secs_f64()));
    notes.push(format!("survey {} files ready={}", st.index.files, st.ready));
    if let Some(img) = &image {
        let t = Instant::now();
        match zahyou_image::load_image_any(Path::new(img)) {
            Ok(l) => {
                items.push(("load_s".into(), t.elapsed().as_secs_f64()));
                let t = Instant::now();
                let prep = zahyou_image::preprocess(&l.data, true);
                items.push(("preprocess_s".into(), t.elapsed().as_secs_f64()));
                let t = Instant::now();
                let (src, _) = zahyou_image::detect_sources(&prep, &zahyou_image::DetectOpts::with_min_sources(60));
                items.push(("detect_s".into(), t.elapsed().as_secs_f64()));
                notes.push(format!("{} x {} / {} 個", l.data.width, l.data.height, src.len()));
                // 画面に貼る準備(灰色の画素 → 縮小の段)
                let t = Instant::now();
                let n = l.display.width * l.display.height;
                let rgba: Vec<u8> = l
                    .display
                    .data
                    .iter()
                    .flat_map(|v| {
                        let g = if v.is_finite() { (v.clamp(0.0, 255.0)) as u8 } else { 0 };
                        [g, g, g, 255]
                    })
                    .collect();
                if rgba.len() == n * 4
                    && let Some(img) = super::scene::rgba_image(l.display.width, l.display.height, &rgba)
                {
                    let p = super::scene::Pyramid::build(img);
                    notes.push(format!("pyramid {} 段 / {} 画素", p.levels.len(), p.texels()));
                }
                items.push(("pyramid_s".into(), t.elapsed().as_secs_f64()));
            }
            Err(e) => notes.push(format!("load error: {e}")),
        }
    }
    let mut json = String::from("{\n");
    json.push_str(&format!(" \"version\": \"{APP_VER}\",\n"));
    for (k, v) in &items {
        json.push_str(&format!(" \"{k}\": {v:.6},\n"));
    }
    let notes_json: Vec<String> = notes.iter().map(|n| serde_json::to_string(n).unwrap_or_default()).collect();
    json.push_str(&format!(" \"notes\": [{}]\n}}", notes_json.join(", ")));
    emit(args, &json);
    0
}

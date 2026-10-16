//! zahyou — 天体画像から目標への向きを求める(Rust 版)。
//!
//! 引数なしで画面を出す。画面を出さない入口(exe は windows サブシステムなので結果は --out へも書く):
//!
//!   zahyou.exe --selftest <画像> [--online|--offline] [--out f]   画面を組み立てて解析を通す自己診断
//!   zahyou.exe --engine-check [--out f]                         フォントと星図データの目録を確かめる
//!   zahyou.exe --update-check [--out f]                         更新の確認だけ
//!   zahyou.exe --bench [--out f]                                解析の速さの物差し(JSON。pipeline 係)
//!   zahyou.exe --bench-gui [<画像>] [--out f]                    画面まわりの速さ(フォント・星図データの検査・図の縮小の段)
//!   zahyou.exe --shot <png> [...]                               画面を自分で撮って閉じる(ui/shot.rs)
//!
//! 設計は docs/rust-port.md。解析の流れは pipeline.rs(別の係)、画面は ui/。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod pipeline;
mod scale;
mod selftest;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::Instant;

use eframe::egui;

use ui::app::{Options, TopTab, ZahyouApp};
use ui::cli::{arg_value, has};
use ui::settings::ResultTab;

/// 画面に出す版名。"-dev" で終わる試作版は更新の確認をしない(v6 の exe へ「更新」されないように)
pub const APP_VER: &str = "v7-dev";

static START: OnceLock<Instant> = OnceLock::new();

/// プロセスが始まってからの時間 [ms](最初の描画までの時間を測る)
pub fn process_elapsed_ms() -> f64 {
    START.get().map(|t| t.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0)
}

fn main() -> ExitCode {
    let _ = START.set(Instant::now());
    ui::worker::install_panic_hook();
    let args: Vec<String> = std::env::args().collect();
    let code = if has(&args, "--update-check") || has(&args, "--update-apply") {
        ui::cli::update_cli(&args)
    } else if has(&args, "--engine-check") {
        ui::cli::engine_check(&args)
    } else if has(&args, "--selftest") {
        ui::cli::selftest(&args)
    } else if has(&args, "--bench-gui") {
        ui::cli::bench(&args)
    } else if has(&args, "--bench") {
        // 解析の速さの物差し(pipeline 係)
        let out = arg_value(&args, "--out").map(PathBuf::from);
        selftest::bench(out.as_deref())
    } else {
        run_gui(&args)
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// 画面を出す
fn run_gui(args: &[String]) -> i32 {
    let glow = has(args, "--glow");
    let shot_path = arg_value(args, "--shot").map(PathBuf::from);
    let first_frame = arg_value(args, "--first-frame").map(PathBuf::from);
    let shooting = shot_path.is_some() || first_frame.is_some();
    if shooting {
        // 撮影では利用者の設定(gui.json)を読み書きしない
        ui::cli::isolate_settings();
    }
    let size = arg_value(args, "--size")
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?])
        })
        .unwrap_or([1180.0, 780.0]);
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("zahyou {APP_VER} — 天体画像から目標への向きを求める"))
        .with_inner_size(size)
        .with_min_inner_size([1000.0f32.min(size[0]), 660.0f32.min(size[1])])
        .with_app_id("zahyou")
        // アイコンはまだ無い(app/zahyou.ico も無い)。空にしておくと Windows の既定のアイコンになる(eframe の「e」を出さない)
        .with_icon(std::sync::Arc::new(egui::IconData::default()));
    let native = eframe::NativeOptions {
        renderer: if glow { eframe::Renderer::Glow } else { eframe::Renderer::Wgpu },
        viewport,
        centered: true,
        ..Default::default()
    };
    let args_owned: Vec<String> = args.to_vec();
    let ff_path = first_frame.clone();
    // 画面のスレッドの panic(窓を開く途中の OpenGL の設定探しなど)も受け止めて、理由を知らせる
    let result = ui::worker::guarded(|| {
        eframe::run_native(
            "zahyou",
            native,
            Box::new(move |cc| {
                let report = ui::fonts::setup(&cc.egui_ctx);
                ui::style::apply(&cc.egui_ctx);
                let opts = Options {
                    headless: false,
                    debug_wcs: arg_value(&args_owned, "--debug-wcs").map(PathBuf::from),
                    no_startup_checks: has(&args_owned, "--no-startup-checks"),
                    no_save: shooting,
                };
                let mut app = ZahyouApp::new(&cc.egui_ctx, report, opts);
                if shooting {
                    apply_shot_args(&mut app, &args_owned, shot_path.clone(), first_frame.clone());
                }
                Ok(Box::new(app))
            }),
        )
    });
    // 閉じるまでにかかった時間(起動の物差しの --first-frame のときだけ書き足す)
    if let Some(p) = &ff_path
        && let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(p)
    {
        use std::io::Write;
        let _ = writeln!(f, "run_native_returned_ms={:.1}", process_elapsed_ms());
    }
    let why = match result {
        Ok(Ok(())) => return 0,
        Ok(Err(e)) => e.to_string(),
        Err((msg, detail)) => {
            // 一度でも描けたあとの panic は、画面の不具合(開き直さない)
            if FRAME_SHOWN.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("画面が止まりました: {msg}");
                if !shooting {
                    tell_failure(&format!("zahyou の画面が止まりました。\n\n{msg}\n{}", detail.join("\n")));
                }
                return 1;
            }
            format!("{msg} ({})", detail.join(" / "))
        }
    };
    eprintln!("画面を開けませんでした: {why}");
    let renderer = if glow { "OpenGL" } else { "DirectX 12" };
    let advice = "グラフィックスのドライバーを更新してください。\nリモート デスクトップや仮想マシンの中では開けないことがあります。";
    // GPU(DX12)で開けなければ OpenGL で開き直す(winit のイベントループは 1 プロセスに 1 回しか作れないので、起動し直す)。
    // 開き直した先でも開けなければ、そちらが知らせる
    if !glow {
        let spawned = std::env::current_exe().and_then(|exe| {
            let mut rest: Vec<String> = args.iter().skip(1).cloned().collect();
            rest.push("--glow".into());
            std::process::Command::new(exe).args(rest).spawn()
        });
        match spawned {
            Ok(_) => return 0,
            Err(e) => {
                if !shooting {
                    tell_failure(&format!(
                        "zahyou の画面を開けませんでした ({renderer}: {why})。\nOpenGL で開き直そうとしましたが、起動できませんでした ({e})。\n\n{advice}"
                    ));
                }
                return 1;
            }
        }
    }
    if !shooting {
        tell_failure(&format!("zahyou の画面を開けませんでした ({renderer}: {why})。\n\n{advice}"));
    }
    1
}

/// 一度でも画面を描いたか(描いたあとの panic は開き直さない)
pub static FRAME_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 画面を開けなかった・止まったことを箱で知らせる(exe は windows サブシステムなので、標準エラーは見えない)
fn tell_failure(text: &str) {
    rfd::MessageDialog::new()
        .set_title("zahyou")
        .set_description(text)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// 撮影の指定を画面に反映する
fn apply_shot_args(app: &mut ZahyouApp, args: &[String], path: Option<PathBuf>, first_frame: Option<PathBuf>) {
    let mut plan = ui::shot::ShotPlan::new(path);
    plan.first_frame = first_frame;
    plan.tab = arg_value(args, "--tab").and_then(|t| match t.as_str() {
        "run" => Some(TopTab::Run),
        "env" => Some(TopTab::Env),
        "help" => Some(TopTab::Help),
        _ => None,
    });
    plan.out_tab = arg_value(args, "--out-tab").and_then(|t| ResultTab::from_key(&t));
    plan.run = has(args, "--run");
    plan.fig_view = arg_value(args, "--fig-view").and_then(|s| {
        let v: Vec<f64> = s.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
    });
    if has(args, "--dark") {
        app.set_theme(ui::theme::ThemeName::Dark);
    }
    if has(args, "--light") {
        app.set_theme(ui::theme::ThemeName::Light);
    }
    if let Some(img) = arg_value(args, "--image") {
        app.s.image = img;
    }
    if let Some(name) = arg_value(args, "--target") {
        app.s.name = name;
    }
    if has(args, "--blind") {
        app.s.mode = ui::settings::Mode::None;
    }
    if let Some(d) = arg_value(args, "--index-dir") {
        app.s.indexdir = d;
    }
    app.shot = Some(plan);
}

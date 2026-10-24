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
//!   zahyou.exe --bench-ui <画像> [...]                           描画方式の物差し(窓を出す。ui/bench_ui.rs)
//!
//! 描き方(DirectX 12 か OpenGL か・どの GPU か)は `RenderChoice`。既定と、開けなかったときに順に試す描き方は
//! `RENDER_ORDER`(決めた根拠は docs/rust-port.md の「描画方式の実測と決定」)。手で選ぶときは --dx12 / --glow / --gpu display|low|high|warp。
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

/// 画面に出す版名。パッケージの版(rs/Cargo.toml)に "v" を付け、末尾の 0 の部分を省いたもの(build.rs の `version_label`。
/// 例: 8.0.0 → "v8"、8.0.0-beta.1 → "v8-beta.1")。版名は v7・v8 のような整数の形で、GitHub のタグ名と同じ。
/// "-dev" で終わる試作版は更新の確認をしない。"-beta.N" などのプレリリースは、GitHub のプレリリースも含めて
/// 同じ大きな版(v8)の新しいものを探す(zahyou_net::update)。
pub const APP_VER: &str = env!("ZAHYOU_VERSION_LABEL");

static START: OnceLock<Instant> = OnceLock::new();

/// 版名の後ろに添える「 (β版)」(プレリリースの版のときだけ。正式版・"-dev" では空)。
/// 窓の題・ログの見出し・［使い方］の「いまの版」で使う(正式版に上げれば自然に消える)
pub fn prerelease_suffix() -> String {
    zahyou_net::update::prerelease_label(APP_VER).map(|l| format!(" ({l})")).unwrap_or_default()
}

/// 窓の題(例「zahyou v8 — 天体画像から目標への向きを求める」、β版なら「zahyou v8-beta.1 (β版) — …」)
pub fn window_title() -> String {
    format!("zahyou {APP_VER}{} — 天体画像から目標への向きを求める", prerelease_suffix())
}

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
    } else if has(&args, "--bench-ui") {
        // 描画方式の物差し(本物の窓を出して図を自動で動かす)
        ui::bench_ui::run(&args)
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
    let choice = RenderChoice::from_args(args);
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
        .with_title(window_title())
        .with_inner_size(size)
        .with_min_inner_size([1000.0f32.min(size[0]), 660.0f32.min(size[1])])
        .with_app_id("zahyou")
        // アイコンはまだ無い(app/zahyou.ico も無い)。空にしておくと Windows の既定のアイコンになる(eframe の「e」を出さない)
        .with_icon(std::sync::Arc::new(egui::IconData::default()));
    let native = choice.native_options(viewport);
    let args_owned: Vec<String> = args.to_vec();
    let ff_path = first_frame.clone();
    // 画面のスレッドの panic(窓を開く途中の OpenGL の設定探しなど)も受け止めて、理由を知らせる
    let result = ui::worker::guarded(|| {
        eframe::run_native(
            "zahyou",
            native,
            Box::new(move |cc| {
                let _ = RENDER_INFO.set(describe_renderer(cc));
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
        let _ = writeln!(f, "renderer={}", RENDER_INFO.get().map(String::as_str).unwrap_or("(開けなかった)"));
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
    eprintln!("画面を開けませんでした ({}): {why}", choice.label());
    // これまでに開けなかった描き方(開き直した先なら、前のプロセスの分も環境変数で受け取る)
    let mut tried: Vec<String> = if choice.step.is_some_and(|k| k > 0) {
        std::env::var(RENDER_FAILED_ENV).ok().map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
    } else {
        Vec::new()
    };
    tried.push(format!("{}: {}", choice.label(), one_line(&why)));
    let advice = "グラフィックスのドライバーを更新してください。\nリモート デスクトップや仮想マシンの中では開けないことがあります。";
    // 開けなければ RENDER_ORDER の次の描き方で開き直す(winit のイベントループは 1 プロセスに 1 回しか作れないので、
    // 起動し直す)。開き直した先でも開けなければ、そちらがさらに次を試すか、全部の理由を知らせる
    if let Some(k) = choice.next_step() {
        let spawned = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe).args(relaunch_args(args, k)).env(RENDER_FAILED_ENV, tried.join("\n")).spawn()
        });
        match spawned {
            Ok(_) => return 0,
            Err(e) => {
                if !shooting {
                    let (b, g) = RENDER_ORDER[k];
                    tell_failure(&format!(
                        "zahyou の画面を開けませんでした。\n{}\n{} で開き直そうとしましたが、起動できませんでした ({e})。\n\n{advice}",
                        tried.join("\n"),
                        label_of(b, g)
                    ));
                }
                return 1;
            }
        }
    }
    if !shooting {
        tell_failure(&format!("zahyou の画面を開けませんでした。\n{}\n\n{advice}", tried.join("\n")));
    }
    1
}

// ================================================================ 描き方 ===

/// 描画のしくみ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Direct3D 12(wgpu)。Windows 11 には必ずある(GPU のドライバーが無くても WARP = ソフトウェアで描ける)
    Dx12,
    /// OpenGL(glow)。GPU の会社のドライバーが要る(Microsoft Basic Display Adapter では 1.1 しか無く開けない)
    Glow,
}

/// DX12 で使う GPU の選び方。OpenGL は窓を出す画面の GPU のドライバー(と GPU の会社の設定)で決まり、ここでは選べない
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuPick {
    /// Windows が列挙する先頭の GPU(主の画面をつないでいる GPU。ノート PC ではふつう内蔵 GPU)。wgpu の PowerPreference::None
    Display,
    /// 内蔵 GPU を先に(電池を持たせる)。wgpu の LowPower
    LowPower,
    /// 外付けの GPU を先に(速さ)。wgpu の HighPerformance(egui の既定)
    HighPerformance,
    /// ソフトウェアの WARP(Microsoft Basic Render Driver)。最後の手段と試験用
    Warp,
}

impl GpuPick {
    /// --gpu の値(display / low / high / warp)
    pub fn from_key(s: &str) -> Option<GpuPick> {
        match s.trim().to_ascii_lowercase().as_str() {
            "display" | "none" | "auto" => Some(GpuPick::Display),
            "low" | "lowpower" | "igpu" => Some(GpuPick::LowPower),
            "high" | "highperformance" | "dgpu" => Some(GpuPick::HighPerformance),
            "warp" | "cpu" | "software" => Some(GpuPick::Warp),
            _ => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            GpuPick::Display => "display",
            GpuPick::LowPower => "low",
            GpuPick::HighPerformance => "high",
            GpuPick::Warp => "warp",
        }
    }
}

/// 既定の描き方と、開けなかったときに順に試す描き方(決めた根拠は docs/rust-port.md の「描画方式の実測と決定」)。
/// 0 番が既定。利用者が --dx12 / --glow で描き方を指定したときは、開けなくても次を試さない
pub const RENDER_ORDER: [(Backend, GpuPick); 3] = [
    (Backend::Dx12, GpuPick::Display),
    (Backend::Glow, GpuPick::Display),
    (Backend::Dx12, GpuPick::Warp),
];

/// DX12 の GPU メモリを小さな塊で確保するのを既定にするか(wgpu の MemoryHints::MemoryUsage。同じ節)
pub const LEAN_MEMORY_DEFAULT: bool = true;

/// 開き直すときに、それまでに開けなかった描き方と理由を子のプロセスへ渡す環境変数(1 行に 1 つ)
const RENDER_FAILED_ENV: &str = "ZAHYOU_RENDER_FAILED";

/// 実際に使っている描き方(「DirectX 12 / NVIDIA GeForce RTX 3060 (DiscreteGpu, …)」など)。画面を開いたあとに入る
pub static RENDER_INFO: OnceLock<String> = OnceLock::new();

/// 描き方の一式
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderChoice {
    pub backend: Backend,
    pub gpu: GpuPick,
    /// DX12 の GPU メモリを小さな塊で確保する(wgpu の MemoryHints::MemoryUsage)
    pub lean_memory: bool,
    /// 垂直同期(物差しで上限なしの速さを見るときだけ切る)
    pub vsync: bool,
    /// OpenGL をハードウェア支援つきで開く。--no-hw で切ると Windows の OpenGL 1.1(ソフトウェア)を選ぶので開けない。
    /// GPU のドライバーが無い環境の代わりに、OpenGL で開けなかったときの順を試すためのもの
    pub gl_hardware: bool,
    /// RENDER_ORDER の何番目か。None は利用者が描き方を指定した(開けなくても次を試さない)
    pub step: Option<usize>,
}

impl RenderChoice {
    /// 引数から決める: --dx12 / --glow(描き方の指定)、--gpu display|low|high|warp、--mem lean|perf(物差し用)、
    /// --no-vsync・--no-hw(物差しと試験用)、--render-step N(開き直した先。RENDER_ORDER の N 番目)
    pub fn from_args(args: &[String]) -> RenderChoice {
        let gpu_arg = arg_value(args, "--gpu").and_then(|s| GpuPick::from_key(&s));
        let lean_memory = match arg_value(args, "--mem").as_deref() {
            Some("perf") | Some("performance") => false,
            Some("lean") | Some("usage") => true,
            _ => LEAN_MEMORY_DEFAULT,
        };
        let vsync = !has(args, "--no-vsync");
        let gl_hardware = !has(args, "--no-hw");
        let step = arg_value(args, "--render-step").and_then(|s| s.trim().parse::<usize>().ok()).filter(|&k| k < RENDER_ORDER.len());
        let named = if has(args, "--glow") {
            Some(Backend::Glow)
        } else if has(args, "--dx12") {
            Some(Backend::Dx12)
        } else {
            None
        };
        let (backend, gpu, step) = match (step, named) {
            // 開き直した先は表の順に従う(WARP 以外の段は --gpu の指定を引き継ぐ)
            (Some(k), _) => {
                let (b, g) = RENDER_ORDER[k];
                (b, if g == GpuPick::Warp { g } else { gpu_arg.unwrap_or(g) }, Some(k))
            }
            (None, Some(b)) => (b, gpu_arg.unwrap_or(RENDER_ORDER[0].1), None),
            (None, None) => (RENDER_ORDER[0].0, gpu_arg.unwrap_or(RENDER_ORDER[0].1), Some(0)),
        };
        RenderChoice { backend, gpu, lean_memory, vsync, gl_hardware, step }
    }

    /// 開けなかったときに次に試す RENDER_ORDER の位置
    pub fn next_step(&self) -> Option<usize> {
        self.step.map(|k| k + 1).filter(|&k| k < RENDER_ORDER.len())
    }

    /// 知らせに出す名前
    pub fn label(&self) -> &'static str {
        label_of(self.backend, self.gpu)
    }

    /// eframe に渡す設定
    pub fn native_options(&self, viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
        let mut o = eframe::NativeOptions { viewport, centered: true, vsync: self.vsync, ..Default::default() };
        match self.backend {
            Backend::Glow => {
                o.renderer = eframe::Renderer::Glow;
                if !self.gl_hardware {
                    o.hardware_acceleration = eframe::HardwareAcceleration::Off;
                }
            }
            Backend::Dx12 => {
                o.renderer = eframe::Renderer::Wgpu;
                o.wgpu_options = self.wgpu_configuration();
            }
        }
        o
    }

    /// DX12 の設定。シェーダーのコンパイラーは wgpu の既定の FXC(Windows に入っている d3dcompiler_47.dll。
    /// DXC は dxcompiler.dll を同梱しないといけないので使わない)
    fn wgpu_configuration(&self) -> eframe::egui_wgpu::WgpuConfiguration {
        use eframe::egui_wgpu;
        let selector: Option<egui_wgpu::NativeAdapterSelectorMethod> = if self.gpu == GpuPick::Warp {
            Some(std::sync::Arc::new(|adapters: &[wgpu::Adapter], _surface: Option<&wgpu::Surface<'_>>| {
                adapters
                    .iter()
                    .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
                    .cloned()
                    .ok_or_else(|| "ソフトウェアの描画 (WARP) が見つかりません".to_string())
            }))
        } else {
            None
        };
        let power_preference = match self.gpu {
            GpuPick::Display | GpuPick::Warp => wgpu::PowerPreference::None,
            GpuPick::LowPower => wgpu::PowerPreference::LowPower,
            GpuPick::HighPerformance => wgpu::PowerPreference::HighPerformance,
        };
        let lean = self.lean_memory;
        let setup = egui_wgpu::WgpuSetupCreateNew {
            instance_descriptor: wgpu::InstanceDescriptor {
                // egui の既定と同じく環境変数 WGPU_BACKEND を見る(試験で WGPU_BACKEND=metal にして DX12 を失敗させる)
                backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12),
                flags: wgpu::InstanceFlags::from_build_config().with_env(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                backend_options: wgpu::BackendOptions::from_env_or_default(),
            },
            power_preference,
            native_adapter_selector: selector,
            device_descriptor: std::sync::Arc::new(move |_adapter: &wgpu::Adapter| wgpu::DeviceDescriptor {
                label: Some("zahyou"),
                // egui の既定と同じ(図は 8192 px を超えるとタイルに分けて貼る。ui/figure.rs)
                required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() },
                memory_hints: if lean { wgpu::MemoryHints::MemoryUsage } else { wgpu::MemoryHints::Performance },
                ..Default::default()
            }),
        };
        egui_wgpu::WgpuConfiguration {
            present_mode: if self.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync },
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
            ..Default::default()
        }
    }
}

/// 描き方の名前
fn label_of(b: Backend, g: GpuPick) -> &'static str {
    match (b, g) {
        (Backend::Dx12, GpuPick::Warp) => "DirectX 12 (ソフトウェア描画 WARP)",
        (Backend::Dx12, _) => "DirectX 12",
        (Backend::Glow, _) => "OpenGL",
    }
}

/// 開き直すときの引数: 描き方の指定(--dx12・--glow・前の --render-step)を外して --render-step k を足す
fn relaunch_args(args: &[String], k: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--glow" | "--dx12" => {}
            "--render-step" => {
                it.next();
            }
            _ => out.push(a.clone()),
        }
    }
    out.push("--render-step".into());
    out.push(k.to_string());
    out
}

/// 改行や続く空白を 1 つの空白にして 1 行にする
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 開いた描き方と GPU の説明(RENDER_INFO と物差しの記録に使う)
pub fn describe_renderer(cc: &eframe::CreationContext<'_>) -> String {
    if let Some(rs) = &cc.wgpu_render_state {
        let i = rs.adapter.get_info();
        let driver = format!("{} {}", i.driver, i.driver_info);
        return format!("DirectX 12 / {} ({:?}, ドライバー {})", i.name, i.device_type, driver.trim());
    }
    if let Some(gl) = &cc.gl {
        use eframe::glow::HasContext;
        let v = gl.version();
        return format!("OpenGL {}.{}{} / {}", v.major, v.minor, if v.is_embedded { " ES" } else { "" }, v.vendor_info);
    }
    "不明".into()
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
    // ［解析］の左の列をいちばん下(API キーの欄)まで送って撮る(小さい窓で下に隠れる欄を目で確かめるため)
    if has(args, "--left-end") {
        app.scroll_left_end = true;
    }
    app.shot = Some(plan);
}

#[cfg(test)]
mod render_choice_tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        std::iter::once("zahyou.exe").chain(s.split_whitespace()).map(String::from).collect()
    }

    #[test]
    fn 既定は表の先頭から始める() {
        let c = RenderChoice::from_args(&args(""));
        assert_eq!((c.backend, c.gpu, c.step), (RENDER_ORDER[0].0, RENDER_ORDER[0].1, Some(0)));
        assert_eq!(c.lean_memory, LEAN_MEMORY_DEFAULT);
        assert!(c.vsync && c.gl_hardware);
        assert_eq!(c.next_step(), Some(1));
        // GPU の選び方だけ変えても、開けなければ次を試す
        let c = RenderChoice::from_args(&args("--gpu high"));
        assert_eq!((c.gpu, c.step), (GpuPick::HighPerformance, Some(0)));
        // 知らない値は既定のまま
        let c = RenderChoice::from_args(&args("--gpu foo --mem bar"));
        assert_eq!((c.gpu, c.lean_memory), (RENDER_ORDER[0].1, LEAN_MEMORY_DEFAULT));
    }

    #[test]
    fn 描き方を指定したら次を試さない() {
        let c = RenderChoice::from_args(&args("--glow"));
        assert_eq!((c.backend, c.step, c.next_step()), (Backend::Glow, None, None));
        let c = RenderChoice::from_args(&args("--dx12 --gpu low --mem perf --no-vsync --no-hw"));
        assert_eq!((c.backend, c.gpu, c.step), (Backend::Dx12, GpuPick::LowPower, None));
        assert!(!c.lean_memory && !c.vsync && !c.gl_hardware);
        assert_eq!(c.label(), "DirectX 12");
        assert_eq!(RenderChoice::from_args(&args("--dx12 --gpu warp")).label(), "DirectX 12 (ソフトウェア描画 WARP)");
    }

    #[test]
    fn 開き直した先は表の順() {
        for (k, (b, g)) in RENDER_ORDER.iter().enumerate() {
            let c = RenderChoice::from_args(&args(&format!("--render-step {k}")));
            assert_eq!((c.backend, c.gpu, c.step), (*b, *g, Some(k)));
            assert_eq!(c.next_step(), (k + 1 < RENDER_ORDER.len()).then_some(k + 1));
        }
        // 範囲の外の番号は無視して先頭から(開き直しが終わらなくなることはない)
        assert_eq!(RenderChoice::from_args(&args("--render-step 99")).step, Some(0));
        assert_eq!(RenderChoice::from_args(&args("--render-step x")).step, Some(0));
        // 表の WARP の段は --gpu に上書きされない
        let last = RENDER_ORDER.len() - 1;
        if RENDER_ORDER[last].1 == GpuPick::Warp {
            assert_eq!(RenderChoice::from_args(&args(&format!("--gpu high --render-step {last}"))).gpu, GpuPick::Warp);
        }
    }

    #[test]
    fn 開き直すときの引数() {
        let a = args("--shot x.png --glow --render-step 1 --dx12 --gpu low --image a.fits");
        assert_eq!(relaunch_args(&a, 2), ["--shot", "x.png", "--gpu", "low", "--image", "a.fits", "--render-step", "2"]);
        assert_eq!(relaunch_args(&args(""), 1), ["--render-step", "1"]);
        // 最後の --render-step に値が無くても落ちない
        assert_eq!(relaunch_args(&args("--render-step"), 1), ["--render-step", "1"]);
    }

    #[test]
    fn 窓の題に_beta_版を添える() {
        let t = window_title();
        assert!(t.starts_with(&format!("zahyou {APP_VER}")), "{t}");
        match zahyou_net::update::prerelease_label(APP_VER) {
            Some(l) => assert!(t.contains(&format!("{APP_VER} ({l}) — ")), "{t}"),
            None => assert!(!t.contains('β') && prerelease_suffix().is_empty(), "{t}"),
        }
    }

    #[test]
    fn eframe_に渡す設定() {
        let o = RenderChoice::from_args(&args("--glow --no-hw --no-vsync")).native_options(egui::ViewportBuilder::default());
        assert_eq!(o.renderer, eframe::Renderer::Glow);
        assert_eq!(o.hardware_acceleration, eframe::HardwareAcceleration::Off);
        assert!(!o.vsync);
        let o = RenderChoice::from_args(&args("--dx12")).native_options(egui::ViewportBuilder::default());
        assert_eq!(o.renderer, eframe::Renderer::Wgpu);
        assert_eq!(o.wgpu_options.present_mode, wgpu::PresentMode::AutoVsync);
        assert!(matches!(o.wgpu_options.wgpu_setup, eframe::egui_wgpu::WgpuSetup::CreateNew(_)));
        assert_eq!(one_line("a\n  b\tc "), "a b c");
    }
}

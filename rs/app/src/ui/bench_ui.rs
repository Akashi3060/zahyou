//! 描画方式の物差し(隠しの起動 `--bench-ui`。docs/rust-port.md の「描画方式の実測と決定」)。
//!
//!   zahyou.exe --bench-ui <画像> [--glow|--dx12] [--gpu display|low|high|warp] [--mem lean|perf]
//!              [--no-vsync] [--no-hw] [--cycles 2] [--idle 3] [--hold 2] [--size 1180x780] [--out 結果.json]
//!
//! 本物の窓に、解析の結果と同じ形の図(撮影時の向き: 画像・座標格子・星の丸 300 個・中心の ×・矢印・目標)を出し、
//! 拡大(zoom: 全体 → 3 倍)・移動(pan: 3 倍のまま画像の上を回る)・全体表示との行き来(home)を決まったフレーム数だけ
//! 自動で繰り返す。段ごとに、フレームの間隔(垂直同期の待ちを含む)と 1 フレームの処理時間(eframe の cpu_usage。
//! 垂直同期の待ちを除いた、更新・三角形への分解・GPU への受け渡しの時間)の中央値・99 パーセンタイル・最大を JSON に書く。
//! 1 周目は縮小の段のテクスチャを初めて GPU に上げるので重く、2 周目は上げ終わったあとの値になる。
//!
//! メモリ(私有・working set)・CPU 時間・GPU メモリは外から測る(このプロセスは自分のメモリを読めない。unsafe を
//! 書かないため)。外の記録と突き合わせられるよう、段の始まりと終わりを UNIX 時刻 [ms] で書く。DX12 なら wgpu の
//! 割り当て器の報告(GPU メモリの塊の確保量)も段の終わりごとに書く。
//!
//! - 画像に WCS があればそれを、無ければ仮の TAN(1.5″/px)で座標格子を描く。星の丸は決まった擬似乱数の位置
//!   (検出はしない。読み込みを軽くするため)
//! - --no-hw は OpenGL をハードウェア支援なしで開く(Windows の OpenGL 1.1 = GPU のドライバーが無い環境の代わり)
//! - 利用者の設定(gui.json・cache.json)は読みも書きもしない

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;
use serde_json::{Value, json};

use super::cli::arg_value;
use super::figure::{FigureView, View};
use super::theme::{self, Theme, ThemeName};
use crate::{APP_VER, RENDER_INFO, RenderChoice, describe_renderer, process_elapsed_ms};

/// 物差しの終わりのコード(0 = 最後まで測れた)
static CODE: AtomicI32 = AtomicI32::new(0);

/// これを超えたら打ち切る [秒](WARP で 61 メガ画素を動かしても収まる長さ)
const TIMEOUT_S: u64 = 600;

/// 段の動き
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    /// 画像なしで待つ(窓を出しただけ)
    Idle,
    /// 画像を読み込んで図にし、最初の 3 フレームを描くまで
    Load,
    /// 全体表示のまま待つ
    Hold,
    /// 全体 → 3 倍の拡大まで少しずつ(縮小の段を順に替える)
    Zoom,
    /// 3 倍の拡大のまま画像の上を回る(原寸の段のタイルを渡り歩く)
    Pan,
    /// 全体表示と 1/4 の範囲を 15 フレームごとに行き来する
    Home,
}

/// 段 1 つの予定
#[derive(Clone, Debug)]
struct Step {
    name: String,
    motion: Motion,
    /// フレーム数で決まる段(Zoom・Pan・Home)
    frames: usize,
    /// 時間で決まる段(Idle・Hold)[秒]
    seconds: f64,
}

impl Step {
    fn secs(name: &str, motion: Motion, seconds: f64) -> Step {
        Step { name: name.into(), motion, frames: 0, seconds }
    }
    fn frames(name: String, motion: Motion, frames: usize) -> Step {
        Step { name, motion, frames, seconds: 0.0 }
    }
}

/// 物差しの段の並び
fn plan(idle_s: f64, cycles: usize, hold_s: f64) -> Vec<Step> {
    let mut v = vec![Step::secs("idle", Motion::Idle, idle_s), Step::secs("load", Motion::Load, 0.0), Step::secs("hold_loaded", Motion::Hold, 1.0)];
    for c in 1..=cycles {
        v.push(Step::frames(format!("zoom{c}"), Motion::Zoom, 240));
        v.push(Step::frames(format!("pan{c}"), Motion::Pan, 240));
        v.push(Step::frames(format!("home{c}"), Motion::Home, 120));
    }
    v.push(Step::secs("hold_end", Motion::Hold, hold_s));
    v
}

/// 段 1 つの記録
#[derive(Default)]
struct Rec {
    name: String,
    t0_unix_ms: f64,
    t1_unix_ms: f64,
    t0_ms: f64,
    t1_ms: f64,
    /// フレームの間隔 [ms](その段で描いたフレームから次のフレームまで)
    interval_ms: Vec<f64>,
    /// 1 フレームの処理時間 [ms](垂直同期の待ちを除く)
    work_ms: Vec<f64>,
    /// DX12 の割り当て器の報告(段の終わり)
    alloc: Option<Value>,
}

/// 読み込んで図にしたもの
struct LoadedScene {
    scene: super::scene::Scene,
    width: usize,
    height: usize,
    seconds: f64,
    wcs_from_header: bool,
    levels: usize,
    texels: usize,
}

fn unix_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

/// 仮の WCS(画像に無いとき。座標格子を描くためだけ)
fn fake_wcs(w: usize, h: usize) -> zahyou_wcs::Wcs {
    let s = 1.5 / 3600.0;
    zahyou_wcs::Wcs {
        crval: zahyou_wcs::radec_to_xyz(83.8, -5.4),
        crpix: [(w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0],
        cd: [[-s, 0.0], [0.0, s]],
        sip: None,
    }
}

/// 決まった擬似乱数の位置の星(検出の丸の代わり)
fn fake_sources(w: usize, h: usize, n: usize) -> Vec<zahyou_image::Source> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 11) as f64) / ((1u64 << 53) as f64)
    };
    (0..n)
        .map(|_| zahyou_image::Source {
            x: next() * w as f64,
            y: next() * h as f64,
            flux: 1000.0,
            peak: 100.0,
            area: 9,
            elong: 1.0,
            fwhm: 3.0,
            snr: 50.0,
        })
        .collect()
}

/// 画像を読み、解析の結果と同じ形の図(撮影時の向き)にして、画面に描く形へ直す(ワーカーで呼ぶ)
fn load_scene(path: &Path) -> Result<LoadedScene, String> {
    let t = Instant::now();
    let l = zahyou_image::load_image_any(path).map_err(|e| e.to_string())?;
    let (w, h) = (l.display.width, l.display.height);
    let (wcs, wcs_from_header) = match zahyou_wcs::Wcs::from_header_cards(&l.header) {
        Ok(w) => (w, true),
        Err(_) => (fake_wcs(w, h), false),
    };
    let sources = fake_sources(w, h, 300);
    let target = wcs.pix_to_radec(w as f64 * 0.72, h as f64 * 0.31);
    let fig = zahyou_plot::figure_original(&l.display, &wcs, &sources, Some(target), l.flip_y, true).map_err(|e| e.to_string())?;
    drop(l);
    let scene = super::bridge::figure_to_scene(&fig);
    drop(fig);
    let (levels, texels) = scene.image.as_ref().map(|p| (p.levels.len(), p.texels())).unwrap_or((0, 0));
    Ok(LoadedScene { scene, width: w, height: h, seconds: t.elapsed().as_secs_f64(), wcs_from_header, levels, texels })
}

/// 値の並びの要約(中央値・99 パーセンタイル(最も近い順位)・最大・平均)
fn stats(v: &[f64]) -> Value {
    if v.is_empty() {
        return json!(null);
    }
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return json!(null);
    }
    s.sort_by(|a, b| a.total_cmp(b));
    let n = s.len();
    let median = if !n.is_multiple_of(2) { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 };
    let p99 = s[((0.99 * n as f64).ceil() as usize).clamp(1, n) - 1];
    let mean = s.iter().sum::<f64>() / n as f64;
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    json!({"n": n, "median": r(median), "p99": r(p99), "max": r(s[n - 1]), "mean": r(mean)})
}

struct BenchApp {
    th: &'static Theme,
    image: PathBuf,
    out: Option<PathBuf>,
    choice: RenderChoice,
    steps: Vec<Step>,
    recs: Vec<Rec>,
    cur: usize,
    step_frames: usize,
    step_started: Instant,
    /// 前のフレームを描き始めた時刻と、その段の番号
    last_frame: Option<(Instant, usize)>,
    loader: Option<Receiver<Result<LoadedScene, String>>>,
    fig: Option<FigureView>,
    /// 図を出してから描いたフレーム数(読み込みの段の終わりの判定)
    shown: usize,
    image_info: Value,
    device: Option<wgpu::Device>,
    adapters: Vec<Value>,
    started: Instant,
    process_start_unix_ms: f64,
    first_frame_ms: Option<f64>,
    frames: u64,
    finished: bool,
    error: Option<String>,
}

impl BenchApp {
    fn open_rec(&mut self, k: usize) {
        if let Some(r) = self.recs.get_mut(k) {
            r.t0_unix_ms = unix_ms();
            r.t0_ms = process_elapsed_ms();
        }
        self.step_frames = 0;
        self.step_started = Instant::now();
        if self.steps.get(k).is_some_and(|s| s.motion == Motion::Load) {
            let (tx, rx) = channel();
            let path = self.image.clone();
            std::thread::Builder::new()
                .name("bench-load".into())
                .spawn(move || {
                    let r = super::worker::guarded(|| load_scene(&path)).unwrap_or_else(|(m, _)| Err(m));
                    let _ = tx.send(r);
                })
                .ok();
            self.loader = Some(rx);
        }
    }

    fn close_rec(&mut self, k: usize) {
        let alloc = self.device.as_ref().and_then(|d| d.generate_allocator_report()).map(|r| {
            let mb = |b: u64| (b as f64 / 1_048_576.0 * 10.0).round() / 10.0;
            json!({"reserved_mb": mb(r.total_reserved_bytes), "allocated_mb": mb(r.total_allocated_bytes), "blocks": r.blocks.len(), "allocations": r.allocations.len()})
        });
        if let Some(r) = self.recs.get_mut(k) {
            r.t1_unix_ms = unix_ms();
            r.t1_ms = process_elapsed_ms();
            r.alloc = alloc;
        }
    }

    /// 段の終わりを見て次へ進める(最後なら記録を書いて閉じる)
    fn advance(&mut self, ctx: &egui::Context) {
        loop {
            let Some(step) = self.steps.get(self.cur) else {
                self.finish(ctx);
                return;
            };
            let done = match step.motion {
                Motion::Idle | Motion::Hold => self.step_started.elapsed().as_secs_f64() >= step.seconds,
                Motion::Load => self.fig.is_some() && self.shown >= 3,
                Motion::Zoom | Motion::Pan | Motion::Home => self.step_frames >= step.frames,
            };
            if !done {
                return;
            }
            self.close_rec(self.cur);
            self.cur += 1;
            if self.cur >= self.steps.len() {
                self.finish(ctx);
                return;
            }
            self.open_rec(self.cur);
        }
    }

    /// 読み込みの結果を受け取る
    fn poll_loader(&mut self) {
        let Some(rx) = &self.loader else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(ls)) => {
                self.image_info = json!({
                    "path": self.image.display().to_string(),
                    "width": ls.width,
                    "height": ls.height,
                    "megapixels": (ls.width * ls.height) as f64 / 1e6,
                    "load_and_figure_s": ls.seconds,
                    "wcs_from_header": ls.wcs_from_header,
                    "pyramid_levels": ls.levels,
                    "pyramid_texels": ls.texels,
                });
                self.fig = Some(FigureView::new(Arc::new(ls.scene), "bench"));
                self.loader = None;
            }
            Ok(Err(e)) => {
                self.error = Some(format!("画像を読めません: {e}"));
                self.loader = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.error = Some("読み込みのスレッドが止まりました".into());
                self.loader = None;
            }
        }
    }

    /// 段の動きに合わせて図の表示の範囲を決める
    fn drive_view(&mut self, ctx: &egui::Context) {
        let Some(step) = self.steps.get(self.cur) else {
            return;
        };
        let Some(fig) = self.fig.as_mut() else {
            return;
        };
        let (w, h) = (fig.scene.width, fig.scene.height);
        let home = View { x0: -0.5, x1: w - 0.5, y0: -0.5, y1: h - 0.5 };
        let n = step.frames.max(2) as f64;
        let t = (self.step_frames as f64 / (n - 1.0)).clamp(0.0, 1.0);
        // 図の枠のおよその幅(画面の画素)。3 倍の拡大 = 画像の 1 画素を画面の 3 画素に
        let ax_px = (ctx.content_rect().width() * ctx.pixels_per_point()) as f64 * 0.85;
        let w_end = (ax_px.max(200.0) / 3.0).min(w);
        let aspect = h / w;
        let at = |cx: f64, cy: f64, vw: f64| View { x0: cx - vw / 2.0, x1: cx + vw / 2.0, y0: cy - vw * aspect / 2.0, y1: cy + vw * aspect / 2.0 };
        let tau = std::f64::consts::TAU;
        fig.view = match step.motion {
            Motion::Idle | Motion::Load | Motion::Hold => home,
            Motion::Zoom => {
                let vw = w * (w_end / w).powf(t);
                at(w * (0.5 - 0.13 * t), h * (0.5 - 0.09 * t), vw)
            }
            Motion::Pan => {
                let a = tau * t;
                at(w * (0.5 + 0.38 * (a - 0.35).sin()), h * (0.5 + 0.38 * (2.0 * a + 0.5).sin()), w_end)
            }
            Motion::Home => {
                if (self.step_frames / 15).is_multiple_of(2) {
                    home
                } else {
                    at(w * 0.6, h * 0.55, w / 4.0)
                }
            }
        };
    }

    fn draw(&mut self, ctx: &egui::Context) {
        let th = self.th;
        let label = self.steps.get(self.cur).map(|s| s.name.clone()).unwrap_or_default();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(th.bg).inner_margin(egui::Margin::same(8)))
            .show(ctx, |ui| {
                if let Some(fig) = self.fig.as_mut() {
                    let _ = fig.show(ui, th);
                } else {
                    ui.label(egui::RichText::new(format!("描画方式の物差し: {label}")).color(th.muted));
                }
            });
    }

    /// 記録を書いて窓を閉じる
    fn finish(&mut self, ctx: &egui::Context) {
        if self.finished {
            return;
        }
        self.finished = true;
        if self.error.is_some() {
            CODE.store(1, Ordering::Relaxed);
        }
        let ppp = ctx.pixels_per_point();
        let rect = ctx.content_rect();
        let max_tex = ctx.input(|i| i.max_texture_side);
        let phases: Vec<Value> = self
            .recs
            .iter()
            .filter(|r| r.t0_unix_ms > 0.0)
            .map(|r| {
                let over = |lim: f64| r.interval_ms.iter().filter(|&&x| x > lim).count();
                json!({
                    "name": r.name,
                    "t0_unix_ms": r.t0_unix_ms.round(),
                    "t1_unix_ms": r.t1_unix_ms.round(),
                    "t0_ms": (r.t0_ms * 10.0).round() / 10.0,
                    "t1_ms": (r.t1_ms * 10.0).round() / 10.0,
                    "interval_ms": stats(&r.interval_ms),
                    "work_ms": stats(&r.work_ms),
                    "frames_over_20ms": over(20.0),
                    "frames_over_50ms": over(50.0),
                    "frames_over_100ms": over(100.0),
                    "allocator": r.alloc,
                })
            })
            .collect();
        let report = json!({
            "bench": "ui",
            "version": APP_VER,
            "choice": {
                "backend": format!("{:?}", self.choice.backend),
                "gpu": self.choice.gpu.key(),
                "lean_memory": self.choice.lean_memory,
                "vsync": self.choice.vsync,
                "no_hw": !self.choice.gl_hardware,
            },
            "renderer": RENDER_INFO.get().cloned().unwrap_or_default(),
            "adapters": self.adapters,
            "process_start_unix_ms": self.process_start_unix_ms.round(),
            "first_frame_ms": self.first_frame_ms,
            "pixels_per_point": ppp,
            "window_px": [(rect.width() * ppp).round(), (rect.height() * ppp).round()],
            "max_texture_side": max_tex,
            "frames_total": self.frames,
            "seconds_total": self.started.elapsed().as_secs_f64(),
            "image": self.image_info,
            "phases": phases,
            "error": self.error,
        });
        write_report(self.out.as_deref(), &report);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for BenchApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let now = Instant::now();
        // 前のフレームの間隔と処理時間(cpu_usage は前のフレームの値)を、前のフレームの段に入れる
        if let Some((t, k)) = self.last_frame.take()
            && let Some(r) = self.recs.get_mut(k)
        {
            r.interval_ms.push(now.duration_since(t).as_secs_f64() * 1000.0);
            if let Some(s) = frame.info().cpu_usage {
                r.work_ms.push(s as f64 * 1000.0);
            }
        }
        self.frames += 1;
        if self.first_frame_ms.is_none() {
            self.first_frame_ms = Some(process_elapsed_ms());
            crate::FRAME_SHOWN.store(true, Ordering::Relaxed);
        }
        if self.finished {
            return;
        }
        if self.started.elapsed() > Duration::from_secs(TIMEOUT_S) {
            self.error = Some(format!("{TIMEOUT_S} 秒で打ち切りました"));
        }
        self.poll_loader();
        if self.error.is_some() {
            self.finish(ctx);
            return;
        }
        self.advance(ctx);
        if self.finished {
            return;
        }
        self.drive_view(ctx);
        self.draw(ctx);
        if self.fig.is_some() {
            self.shown += 1;
        }
        self.step_frames += 1;
        self.last_frame = Some((now, self.cur));
        ctx.request_repaint();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.th.bg.to_normalized_gamma_f32()
    }
}

/// 記録を標準出力と --out のファイルへ書く
fn write_report(out: Option<&Path>, v: &Value) {
    let text = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
    println!("{text}");
    if let Some(p) = out
        && let Err(e) = std::fs::write(p, format!("{text}\n"))
    {
        eprintln!("{} に書けません: {e}", p.display());
    }
}

/// 描画方式の物差し(`--bench-ui <画像>`)
pub fn run(args: &[String]) -> i32 {
    let image = args.iter().position(|a| a == "--bench-ui").and_then(|i| args.get(i + 1)).filter(|s| !s.starts_with("--")).cloned();
    let out = arg_value(args, "--out").map(PathBuf::from);
    let Some(image) = image else {
        write_report(out.as_deref(), &json!({"bench": "ui", "error": "使い方: zahyou.exe --bench-ui <画像> [--glow|--dx12] [--gpu display|low|high|warp] [--mem lean|perf] [--no-vsync] [--no-hw] [--cycles N] [--out f.json]"}));
        return 2;
    };
    // 利用者の設定には触れない(この物差しは設定を読まないが、念のため)
    super::cli::isolate_settings();
    let choice = RenderChoice::from_args(args);
    let num = |key: &str, default: f64| arg_value(args, key).and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(default);
    let cycles = num("--cycles", 2.0).round().clamp(1.0, 20.0) as usize;
    let steps = plan(num("--idle", 3.0), cycles, num("--hold", 2.0));
    let size = arg_value(args, "--size")
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?])
        })
        .filter(|[w, h]| w.is_finite() && h.is_finite() && *w >= 200.0 && *h >= 200.0)
        .unwrap_or([1180.0, 780.0]);
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("zahyou {APP_VER} — 描画方式の物差し"))
        .with_inner_size(size)
        .with_app_id("zahyou-bench-ui")
        .with_icon(Arc::new(egui::IconData::default()));
    let native = choice.native_options(viewport);
    let process_start_unix_ms = unix_ms() - process_elapsed_ms();
    let (choice2, out2, image2) = (choice.clone(), out.clone(), PathBuf::from(&image));
    let result = super::worker::guarded(|| {
        eframe::run_native(
            "zahyou-bench-ui",
            native,
            Box::new(move |cc| {
                let _ = RENDER_INFO.set(describe_renderer(cc));
                let _ = super::fonts::setup(&cc.egui_ctx);
                super::style::apply(&cc.egui_ctx);
                let th = theme::theme(ThemeName::Light);
                theme::apply(&cc.egui_ctx, th);
                let (device, adapters) = match &cc.wgpu_render_state {
                    Some(rs) => (
                        Some(rs.device.clone()),
                        rs.available_adapters
                            .iter()
                            .map(|a| {
                                let i = a.get_info();
                                json!({"name": i.name, "type": format!("{:?}", i.device_type), "driver": i.driver, "driver_info": i.driver_info})
                            })
                            .collect(),
                    ),
                    None => (None, Vec::new()),
                };
                let n = steps.len();
                let mut app = BenchApp {
                    th,
                    image: image2,
                    out: out2,
                    choice: choice2,
                    recs: steps.iter().map(|s| Rec { name: s.name.clone(), ..Default::default() }).collect(),
                    steps,
                    cur: 0,
                    step_frames: 0,
                    step_started: Instant::now(),
                    last_frame: None,
                    loader: None,
                    fig: None,
                    shown: 0,
                    image_info: Value::Null,
                    device,
                    adapters,
                    started: Instant::now(),
                    process_start_unix_ms,
                    first_frame_ms: None,
                    frames: 0,
                    finished: false,
                    error: None,
                };
                debug_assert!(n > 0);
                app.open_rec(0);
                Ok(Box::new(app))
            }),
        )
    });
    let why = match result {
        Ok(Ok(())) => return CODE.load(Ordering::Relaxed),
        Ok(Err(e)) => e.to_string(),
        Err((msg, detail)) => format!("{msg} ({})", detail.join(" / ")),
    };
    // 窓を開けなかった(物差しは何も描いていない)
    write_report(
        out.as_deref(),
        &json!({
            "bench": "ui",
            "version": APP_VER,
            "choice": {"backend": format!("{:?}", choice.backend), "gpu": choice.gpu.key(), "lean_memory": choice.lean_memory, "vsync": choice.vsync, "no_hw": !choice.gl_hardware},
            "renderer": RENDER_INFO.get().cloned().unwrap_or_default(),
            "process_start_unix_ms": process_start_unix_ms.round(),
            "open_failed": why,
            "frame_shown": crate::FRAME_SHOWN.load(Ordering::Relaxed),
        }),
    );
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 要約の値() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = stats(&v);
        assert_eq!(s["n"], 100);
        assert_eq!(s["median"], 50.5);
        assert_eq!(s["p99"], 99.0);
        assert_eq!(s["max"], 100.0);
        assert_eq!(stats(&[]), Value::Null);
        assert_eq!(stats(&[f64::NAN]), Value::Null);
        assert_eq!(stats(&[3.0])["p99"], 3.0);
    }

    #[test]
    fn 段の並び() {
        let p = plan(3.0, 2, 2.0);
        let names: Vec<&str> = p.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["idle", "load", "hold_loaded", "zoom1", "pan1", "home1", "zoom2", "pan2", "home2", "hold_end"]);
        assert!(p.iter().filter(|s| matches!(s.motion, Motion::Zoom | Motion::Pan | Motion::Home)).all(|s| s.frames >= 120));
    }

    #[test]
    fn 仮の星と_wcs() {
        let s = fake_sources(100, 50, 300);
        assert_eq!(s.len(), 300);
        assert!(s.iter().all(|p| (0.0..100.0).contains(&p.x) && (0.0..50.0).contains(&p.y)));
        let w = fake_wcs(100, 50);
        let (ra, dec) = w.pix_to_radec(49.5, 24.5);
        assert!((ra - 83.8).abs() < 1e-9 && (dec + 5.4).abs() < 1e-9);
    }

    #[test]
    fn 壊れた画像では読み込みが失敗を返す() {
        let dir = std::env::temp_dir().join(format!("zahyou-bench-ui-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("broken.fits");
        std::fs::write(&p, b"SIMPLE  =                    T").unwrap();
        assert!(load_scene(&p).is_err());
        assert!(load_scene(&dir.join("none.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

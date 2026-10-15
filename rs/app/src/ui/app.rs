//! 画面の本体(Python 版の class App)。状態・ワーカーとの受け渡し・ボタンの処理。
//! 各タブの部品の並べ方は run_tab.rs / env_tab.rs / help_tab.rs。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui;

use super::bridge::{self, FigureOut, ResultView};
use super::figure::{FigAction, FigureView};
use super::fonts::FontReport;
use super::settings::{Mode, ResultTab, Settings};
use super::survey::{self, Survey};
use super::theme::{self, Theme};
use super::widgets::Placed;
use super::worker::{self, Job, LogTo, Msg};
use crate::APP_VER;

pub const APP_NAME: &str = "zahyou";

/// 上のタブ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopTab {
    Run,
    Env,
    Help,
}

/// 進み具合のバーの状態
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Idle,
    /// 処理中(どこまで進んだか分からない)。始めた時刻(egui の秒)
    Busy(f64),
    /// ダウンロードなど (済み, 全体)
    Det(u64, u64),
}

/// 起動のしかた(画面の撮影・自己診断のときの指定)
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// OS のダイアログを出さない(自己診断)。確認は「いいえ」、知らせはログへ
    pub headless: bool,
    /// 解析の結果を解かずに、この WCS で出す(隠しの引数 --debug-wcs)
    pub debug_wcs: Option<PathBuf>,
    /// 起動したついでの確認(星図データ・更新)をしない
    pub no_startup_checks: bool,
    /// 設定を保存しない(画面の撮影)
    pub no_save: bool,
}

/// 結果の 3 つの数字の見出しの既定
pub const CAPS_TARGET: [&str; 3] = ["画像中心から目標までの距離", "赤経 (RA) 方向", "赤緯 (Dec) 方向"];
pub const DETAIL_START: &str = "画像を選んで「解析を実行」を押してください。";

pub struct ZahyouApp {
    pub s: Settings,
    pub th: &'static Theme,
    pub opts: Options,
    pub tab: TopTab,
    /// 結果の内側のタブ(今見ているもの)
    pub out_tab: ResultTab,

    // --- 結果 ---
    pub caps: [String; 3],
    pub vals: [String; 3],
    pub detail: String,
    pub figs: [Option<FigureView>; 2],
    pub fig_src: [Option<FigureOut>; 2],
    pub last_result: Option<ResultView>,

    // --- ログ・状態 ---
    pub log_run: String,
    pub log_env: String,
    pub status: String,
    pub progress: Progress,
    pub busy: bool,
    pub cancel: Arc<AtomicBool>,
    log_target: LogTo,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub ctx: egui::Context,

    // --- 準備 ---
    pub env_state: Option<Survey>,
    /// 表の 3 行(星図データ・検査・総合): (OK か, 文)。None は未確認
    pub env_rows: [(Option<bool>, String); 3],
    pub lbl_env: String,
    pub marks: BTreeMap<u32, bool>,

    // --- 更新 ---
    pub update_msg: String,
    pub update_state: Option<zahyou_net::UpdateCheck>,
    pub update_can_apply: bool,

    // --- そのほか ---
    pub fonts: FontReport,
    pub frames: u64,
    pub first_frame_ms: Option<f64>,
    /// 自己診断の記録: 出したダイアログの文
    pub dialogs: Vec<String>,
    /// 自己診断の記録: 部品の矩形
    pub placed: Vec<Placed>,
    pub window_rect: egui::Rect,
    quit_at: Option<Instant>,
    /// 画面の自分の撮影(--shot)
    pub shot: Option<super::shot::ShotPlan>,
}

impl ZahyouApp {
    pub fn new(ctx: &egui::Context, fonts: FontReport, opts: Options) -> ZahyouApp {
        let s = Settings::load();
        let th = theme::theme(s.theme);
        let (tx, rx) = channel();
        let mut marks = BTreeMap::new();
        for (sc, _, _) in zahyou_net::SCALE_FOV {
            marks.insert(*sc, false);
        }
        let mut app = ZahyouApp {
            out_tab: ResultTab::Orig,
            s,
            th,
            opts,
            tab: TopTab::Run,
            caps: CAPS_TARGET.map(String::from),
            vals: ["—", "—", "—"].map(String::from),
            detail: DETAIL_START.into(),
            figs: [None, None],
            fig_src: [None, None],
            last_result: None,
            log_run: String::new(),
            log_env: String::new(),
            status: String::new(),
            progress: Progress::Idle,
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            log_target: LogTo::Run,
            tx,
            rx,
            ctx: ctx.clone(),
            env_state: None,
            env_rows: [(None, "未確認".into()), (None, "未確認".into()), (None, "未確認".into())],
            lbl_env: String::new(),
            marks,
            update_msg: String::new(),
            update_state: None,
            update_can_apply: false,
            fonts,
            frames: 0,
            first_frame_ms: None,
            dialogs: Vec::new(),
            placed: Vec::new(),
            window_rect: egui::Rect::NOTHING,
            quit_at: None,
            shot: None,
        };
        theme::apply(ctx, app.th);
        app.startup();
        app
    }

    // ============================================================ 起動時 ===
    fn startup(&mut self) {
        self.log_run_line(&format!("{APP_NAME} {APP_VER} (Rust 版)   フォント: {}", self.fonts.summary()));
        if self.opts.no_startup_checks {
            return;
        }
        // 前の版の入れ替えで残った zahyou.exe.old を消す(入れ替えの直後は古いプロセスが終わるのを少し待つ)
        if let Ok(exe) = std::env::current_exe() {
            std::thread::spawn(move || {
                let _ = zahyou_net::cleanup_old_versions(&exe, Duration::from_secs(10));
            });
        }
        // 星図データ(画面はふさがない)
        let dir = self.index_dir();
        let job = self.job(LogTo::Env);
        worker::spawn_quiet(job, "survey-quiet", move |job| {
            let st = survey::survey(&dir);
            let idx = &st.index;
            if st.ready {
                let fov = idx.fov.map(|(a, b)| format!(" / 画角 {}′〜{}′", survey::g(a), survey::g(b))).unwrap_or_default();
                job.log(&format!("✅ オフライン解析できます (星図データ {} ファイル{fov})", idx.files));
            } else {
                job.log("❌ オフライン解析はまだ使えません。");
            }
            for m in &st.notes {
                job.log(&format!("  ⚠️ {m}"));
            }
            if !st.ready {
                job.log("「まとめて準備する」を押すと星図データを落とします。");
            }
            job.send(Msg::Env(Box::new(st)));
        });
        // 更新(起動時に確認する設定のときだけ。試作版は問い合わせない)
        if self.s.update_auto {
            let job = self.job(LogTo::Run);
            worker::spawn_quiet(job, "update-quiet", move |job| {
                let exe = std::env::current_exe().unwrap_or_default();
                let never = || false;
                let chk = zahyou_net::check_update(&exe, APP_VER, &never);
                job.send(Msg::Update(Box::new(chk), true));
            });
        }
    }

    // ============================================================ 仕組み ===
    pub fn job(&self, to: LogTo) -> Job {
        Job::new(self.tx.clone(), self.cancel.clone(), to, self.ctx.clone())
    }

    pub fn index_dir(&self) -> PathBuf {
        let d = self.s.indexdir.trim();
        PathBuf::from(if d.is_empty() { super::settings::DEFAULT_INDEX_DIR } else { d })
    }

    /// 重い処理を 1 本だけ走らせる(Python の _bg)。動いていれば断る。
    pub fn bg(&mut self, label: &str, to: LogTo, f: impl FnOnce(&Job) -> String + Send + 'static) -> bool {
        if self.busy {
            self.info("いま別の処理が動いています。");
            return false;
        }
        self.busy = true;
        self.cancel.store(false, Ordering::Relaxed);
        self.log_target = to;
        self.status = label.to_string();
        self.progress = Progress::Busy(self.ctx.input(|i| i.time));
        worker::spawn_busy(self.job(to), "zahyou-worker", f);
        true
    }

    /// ワーカーからの用事を片付ける(毎フレーム)
    pub fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(LogTo::Run, s) => self.log_run_line(&s),
                Msg::Log(LogTo::Env, s) => self.log_env_line(&s),
                Msg::Progress(d, t) => {
                    self.progress = Progress::Det(d, t);
                    let pct = if t > 0 { 100.0 * d as f64 / t as f64 } else { 0.0 };
                    self.status = format!(
                        "{} / {}  ({pct:.0}%)",
                        zahyou_net::human(d as f64),
                        zahyou_net::human(t as f64)
                    );
                }
                Msg::Run(r) => self.show_result(*r),
                Msg::Env(st) => self.show_env(*st),
                Msg::Update(chk, quiet) => self.show_update(*chk, quiet),
                Msg::Marks(need) => {
                    for (k, v) in self.marks.iter_mut() {
                        *v = need.contains(k);
                    }
                }
                Msg::QuitForUpdate => {
                    self.save_settings();
                    self.quit_at = Some(Instant::now() + Duration::from_millis(400));
                }
                Msg::Saved(r) => {
                    self.status = match r {
                        Ok(p) => format!("図を保存しました: {p}"),
                        Err(e) => format!("図を保存できませんでした: {e}"),
                    };
                }
                Msg::Done(final_msg) => {
                    self.busy = false;
                    self.status = final_msg;
                    self.progress = Progress::Idle;
                }
                Msg::Failed { to, msg, detail } => {
                    let mut lines = vec![String::new(), format!("❌ panic: {msg}")];
                    lines.extend(detail.iter().rev().take(12).rev().map(|l| format!("   {l}")));
                    for l in lines {
                        match to {
                            LogTo::Run => self.log_run_line(&l),
                            LogTo::Env => self.log_env_line(&l),
                        }
                    }
                    self.status = "失敗しました。ログを見てください。".into();
                }
            }
        }
        if let Some(t) = self.quit_at
            && Instant::now() >= t
        {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.quit_at = None;
        }
    }

    pub fn log_run_line(&mut self, s: &str) {
        self.log_run.push_str(s);
        self.log_run.push('\n');
    }
    pub fn log_env_line(&mut self, s: &str) {
        self.log_env.push_str(s);
        self.log_env.push('\n');
    }
    fn log_to(&mut self, to: LogTo, s: &str) {
        match to {
            LogTo::Run => self.log_run_line(s),
            LogTo::Env => self.log_env_line(s),
        }
    }

    // ------------------------------------------------------------ ダイアログ ---
    pub fn info(&mut self, text: &str) {
        self.dialogs.push(text.to_string());
        if self.opts.headless {
            return;
        }
        rfd::MessageDialog::new()
            .set_title(APP_NAME)
            .set_description(text)
            .set_level(rfd::MessageLevel::Info)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
    pub fn warn(&mut self, text: &str) {
        self.dialogs.push(text.to_string());
        if self.opts.headless {
            return;
        }
        rfd::MessageDialog::new()
            .set_title(APP_NAME)
            .set_description(text)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
    /// 確認(OK / キャンセル)。自己診断では「キャンセル」
    pub fn ask(&mut self, text: &str) -> bool {
        self.dialogs.push(text.to_string());
        if self.opts.headless {
            return false;
        }
        matches!(
            rfd::MessageDialog::new()
                .set_title(APP_NAME)
                .set_description(text)
                .set_level(rfd::MessageLevel::Info)
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show(),
            rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes
        )
    }

    // ============================================================ 設定 ===
    pub fn save_settings(&self) {
        if !self.opts.no_save {
            self.s.save();
        }
    }

    pub fn toggle_theme(&mut self) {
        self.s.theme = self.s.theme.toggled();
        self.th = theme::theme(self.s.theme);
        theme::apply(&self.ctx, self.th);
        self.save_settings();
    }

    pub fn set_theme(&mut self, name: super::theme::ThemeName) {
        self.s.theme = name;
        self.th = theme::theme(name);
        theme::apply(&self.ctx, self.th);
    }

    // ============================================================ 解析 ===
    pub fn choose_image(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("天体画像を選ぶ")
            .add_filter("天体画像", &["fits", "fit", "fts", "fz", "png", "jpg", "jpeg", "tif", "tiff"])
            .add_filter("FITS", &["fits", "fit", "fts", "fz"])
            .add_filter("画像", &["png", "jpg", "jpeg", "tif", "tiff"])
            .add_filter("すべて", &["*"])
            .pick_file();
        if let Some(p) = picked {
            self.s.image = p.display().to_string();
            if let Ok(m) = std::fs::metadata(&p) {
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.log_run_line(&format!("画像を選びました: {name}  ({:.1} MB)", m.len() as f64 / 1024.0 / 1024.0));
            }
        }
    }

    /// 窓へ落とされたファイル(画像を選んだのと同じに扱う。v6 には無かった)
    fn take_dropped(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(p) = dropped.into_iter().next() {
            self.s.image = p.display().to_string();
            self.tab = TopTab::Run;
            if let Ok(m) = std::fs::metadata(&p) {
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.log_run_line(&format!("画像を選びました: {name}  ({:.1} MB)", m.len() as f64 / 1024.0 / 1024.0));
            }
        }
    }

    /// 「解析を実行」
    pub fn start_analysis(&mut self) {
        let path = self.s.image.trim().to_string();
        if path.is_empty() || !Path::new(&path).exists() {
            self.warn("画像を選んでください。");
            return;
        }
        if self.s.mode == Mode::StarName && self.s.name.trim().is_empty() {
            self.warn(
                "目標の天体名を入れてください。\n目標を決めずに画像の向きだけ知りたいときは「指定しない (完全ブラインド)」を選んでください。",
            );
            return;
        }
        if self.busy {
            self.info("いま別の処理が動いています。");
            return;
        }
        self.log_run.clear();
        self.clear_figures();
        // v6 は見出しを戻していなかった(ブラインドのあとに目標ありで解くと見出しが違ったまま)
        self.caps = CAPS_TARGET.map(String::from);
        self.vals = ["—", "—", "—"].map(String::from);
        self.detail = "解析しています...".into();
        self.out_tab = ResultTab::Log; // こちらからの切り替え(好みとしては覚えない)
        let params = bridge::params(&self.s);
        let debug_wcs = self.opts.debug_wcs.clone();
        self.save_settings();
        let cancel = self.cancel.clone();
        self.bg("解析しています...", LogTo::Run, move |job| {
            let log = |l: &str| job.log(l);
            let r = bridge::run(&params, debug_wcs.as_deref(), &cancel, &log);
            let msg = if job.cancelled() { "中止しました。".to_string() } else { r.status.clone() };
            job.send(Msg::Run(Box::new(r)));
            msg
        });
    }

    pub fn clear_figures(&mut self) {
        self.figs = [None, None];
        self.fig_src = [None, None];
    }

    fn show_result(&mut self, r: ResultView) {
        if r.cancelled || self.cancel.load(Ordering::Relaxed) {
            self.detail = "中止しました。".into();
            self.last_result = Some(r);
            return;
        }
        if !r.solved {
            self.detail = "解けませんでした。ログを見てください。".into();
            self.last_result = Some(r);
            return;
        }
        match &r.numbers {
            Some(n) => {
                self.caps = [n[0].0.clone(), n[1].0.clone(), n[2].0.clone()];
                self.vals = [n[0].1.clone(), n[1].1.clone(), n[2].1.clone()];
            }
            None => self.vals = ["—", "—", "—"].map(String::from),
        }
        if let Some(d) = &r.detail {
            self.detail = d.clone();
        }
        for (i, f) in r.figures.iter().take(2).enumerate() {
            self.figs[i] = Some(FigureView::new(f.scene.clone(), if i == 0 { "orig" } else { "northup" }));
            self.fig_src[i] = Some(f.clone());
        }
        // 前に見ていた表示に戻す。初めては「撮影時の向き」
        let mut want = self.s.result_tab;
        let has = |t: ResultTab, figs: &[Option<FigureView>; 2]| match t {
            ResultTab::Orig => figs[0].is_some(),
            ResultTab::NorthUp => figs[1].is_some(),
            ResultTab::Log => true,
        };
        if !has(want, &self.figs) {
            // 目標を指定していないときは「北が上」の図が無い
            want = if self.figs[0].is_some() { ResultTab::Orig } else { ResultTab::Log };
        }
        self.out_tab = want;
        self.last_result = Some(r);
    }

    /// 結果の内側のタブを利用者が切り替えた(次からの既定にする)
    pub fn user_select_out(&mut self, t: ResultTab) {
        self.out_tab = t;
        if self.s.result_tab != t {
            self.s.result_tab = t;
            self.save_settings();
        }
    }

    /// 「中止」
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.status = "中止しています...".into();
        let to = self.log_target;
        self.log_to(to, "");
        self.log_to(to, "■ 中止します (数秒で止まります)...");
    }

    /// 図を保存する(「画像を保存」)
    pub fn save_figure(&mut self, which: usize) {
        let Some(src) = self.fig_src.get(which).cloned().flatten() else {
            return;
        };
        // v6 と同じく、拡大・移動しているならその範囲のまま保存する(ダイアログを開く前の表示で決める)
        let view = self.figs.get(which).and_then(|f| f.as_ref()).and_then(|f| f.zoomed_view());
        let Some(path) = rfd::FileDialog::new()
            .set_title("図を保存")
            .set_file_name("image.png")
            .add_filter("PNG 画像", &["png"])
            .save_file()
        else {
            return;
        };
        let dark = self.s.theme == super::theme::ThemeName::Dark;
        let job = self.job(LogTo::Run);
        self.status = "図を保存しています...".into();
        worker::spawn_quiet(job, "save-figure", move |job| {
            let r = bridge::save_figure(&src.source, &path, dark, view).map(|_| path.display().to_string());
            job.send(Msg::Saved(r));
        });
    }

    pub fn handle_fig_action(&mut self, which: usize, a: Option<FigAction>) {
        if let Some(FigAction::Save) = a {
            self.save_figure(which);
        }
    }

    // ============================================================ 準備 ===
    pub fn choose_indexdir(&mut self) {
        if let Some(d) = rfd::FileDialog::new().set_title("星図データの置き場所").pick_folder() {
            let d = d.display().to_string();
            self.s.indexdir = d.clone();
            self.log_env_line(&format!("星図データの置き場所: {d}"));
            self.log_env_line("  「状態を確認」を押すと、この場所で調べ直します。");
        }
    }

    /// 「状態を確認」
    pub fn survey(&mut self) {
        let dir = self.index_dir();
        self.bg("星図データを調べています...", LogTo::Env, move |job| {
            let log = |l: &str| job.log(l);
            log(&"=".repeat(56));
            log("状態を確認しています (星図データを 1 本ずつ検査します)...");
            let st = survey::survey(&dir);
            survey::report(&st, &log);
            let msg = if st.ready {
                log("✅ オフライン解析できます。");
                "オフライン解析できます。"
            } else {
                log("❌ まだ足りません。「まとめて準備する」を押してください。");
                "まだ準備が要ります。"
            };
            job.send(Msg::Env(Box::new(st)));
            msg.to_string()
        });
    }

    fn show_env(&mut self, st: Survey) {
        let idx = &st.index;
        let chk = &st.check;
        self.env_rows[0] = if !idx.ok {
            (Some(false), format!("{} にありません", idx.dir.display()))
        } else {
            let mut text = format!("{} ファイル / {}", idx.files, zahyou_net::human(idx.bytes as f64));
            if !idx.partial.is_empty() {
                let p: Vec<String> = idx.partial.keys().map(|s| s.to_string()).collect();
                text += &format!("  (途中の段 {})", p.join(", "));
            }
            (Some(idx.partial.is_empty()), text)
        };
        self.env_rows[1] = if chk.checked == 0 {
            (Some(false), "調べるものがありません".into())
        } else if !chk.bad.is_empty() {
            (Some(false), format!("{} / {} ファイルに問題", chk.bad.len(), chk.checked))
        } else {
            (Some(true), format!("{} ファイルとも正常", chk.checked))
        };
        self.env_rows[2] = (Some(st.ready), if st.ready { "オフライン解析できます" } else { "まだ足りません" }.into());
        let mut msg = format!("オフライン解析: {}", if st.ready { "使えます" } else { "まだ準備が要ります" });
        if let Some((a, b)) = idx.fov {
            msg += &format!(" / 解ける画角 {}′〜{}′", survey::g(a), survey::g(b));
        }
        self.lbl_env = msg;
        // 既にある段は外しておく
        for (s, v) in self.marks.iter_mut() {
            if idx.scales.contains(s) {
                *v = false;
            }
        }
        self.env_state = Some(st);
    }

    fn have_scales(&self) -> Vec<u32> {
        self.env_state.as_ref().map(|s| s.index.scales.clone()).unwrap_or_default()
    }

    fn marked(&self) -> Vec<u32> {
        self.marks.iter().filter(|(_, v)| **v).map(|(k, _)| *k).collect()
    }

    /// 「必要な段を選ぶ」
    pub fn recommend(&mut self) {
        let focal = self.s.focal.trim().parse::<f64>();
        let sensor = self.s.sensor.trim().parse::<f64>();
        let (Ok(focal), Ok(sensor)) = (focal, sensor) else {
            self.info(
                "焦点距離とセンサー横幅 [mm] を入れてください。\n例: 焦点距離 800、センサー横幅 7.4 (1/1.8 型)\n分からなければ、画像 1 枚を解析すると下に「逆算した焦点距離」が出ます。\n\n[解析] タブで画像を選んでおくと、切り出した縦横も見て選びます。",
            );
            return;
        };
        let have = self.have_scales();
        let path = self.s.image.trim().to_string();
        self.bg("画角を調べています...", LogTo::Env, move |job| {
            let log = |l: &str| job.log(l);
            let Some(fov_w) = zahyou_net::fov_arcmin(focal, sensor) else {
                log("焦点距離とセンサー横幅から画角を出せません (0 や数でない値)。");
                return "焦点距離とセンサー横幅を確かめてください。".into();
            };
            let mut fov_h = None;
            if !path.is_empty() && Path::new(&path).exists() {
                match bridge::image_size(Path::new(&path)) {
                    Ok((nx, ny)) => {
                        let h = fov_w * ny as f64 / nx as f64;
                        fov_h = Some(h);
                        log(&format!("画像 {nx} x {ny} 画素 → 画角 {fov_w:.1}′ x {h:.1}′"));
                        if nx.max(ny) as f64 >= 1.8 * nx.min(ny) as f64 {
                            log(&format!("  縦横比が大きいので、短辺 {:.1}′ に合う段も入れます。", fov_w.min(h)));
                        }
                    }
                    Err(_) => log("画像の大きさを読めませんでした (LoadError)。横幅だけで選びます。"),
                }
            } else {
                log(&format!(
                    "⚠️ 横幅の画角 {fov_w:.1}′ だけで選びます。必要な段は短いほうの辺で決まるので、[解析] タブで画像を選んでから押し直してください。"
                ));
            }
            let want = zahyou_net::recommend_scales(fov_w, fov_h);
            let need: Vec<u32> = want.iter().copied().filter(|s| !have.contains(s)).collect();
            let mb = zahyou_net::scales_mb(&need) as f64;
            log(&format!("  必要な段 {}", survey::list_repr(&want)));
            let need_s = if need.is_empty() { "なし".to_string() } else { survey::list_repr(&need) };
            log(&format!("  まだ無いのは {need_s} (約 {:.1} GB)", mb / 1024.0));
            job.send(Msg::Marks(need.clone()));
            if need.is_empty() {
                "必要な段はすべてそろっています。".into()
            } else {
                format!("{} 段に印を付けました (約 {:.1} GB)", need.len(), mb / 1024.0)
            }
        });
    }

    /// 「おすすめをまとめて落とす」
    pub fn download_recommended(&mut self) {
        let want = zahyou_net::recommended_scales();
        let have = self.have_scales();
        let need: Vec<u32> = want.iter().copied().filter(|s| !have.contains(s)).collect();
        for (k, v) in self.marks.iter_mut() {
            *v = need.contains(k);
        }
        let minfov = survey::g(zahyou_net::RECOMMENDED_MIN_FOV);
        if need.is_empty() {
            self.log_env_line(&format!("おすすめの段 (画角 {minfov}′ 以上) はすべてそろっています。"));
            self.status = "おすすめの段はすべてそろっています。".into();
            return;
        }
        let mb = zahyou_net::scales_mb(&need) as f64;
        self.log_env_line(&format!("おすすめの段 {}", survey::list_repr(&want)));
        self.log_env_line(&format!("  まだ無いのは {} (約 {:.1} GB)", survey::list_repr(&need), mb / 1024.0));
        let q = format!(
            "画角 {minfov}′ 以上をぜんぶそろえます。\n足りない {} 段 / 約 {:.1} GB を\n{} へ落とします。よろしいですか?\n\n回線にもよりますが、数十分かかります。\n途中で中止しても、次に押せば続きから再開します。",
            need.len(),
            mb / 1024.0,
            self.s.indexdir
        );
        if !self.ask(&q) {
            self.status = "やめました。".into();
            return;
        }
        self.start_download(need);
    }

    /// 「選んだ段を落とす」
    pub fn download_index(&mut self) {
        let scales = self.marked();
        if scales.is_empty() {
            self.info("落とす段にチェックを入れてください。");
            return;
        }
        let mb = zahyou_net::scales_mb(&scales) as f64;
        let q = format!(
            "{} 段 / 約 {:.1} GB を\n{} へ落とします。よろしいですか?\n\n途中で中止しても、次に押せば続きから再開します。",
            scales.len(),
            mb / 1024.0,
            self.s.indexdir
        );
        if !self.ask(&q) {
            return;
        }
        self.start_download(scales);
    }

    fn start_download(&mut self, scales: Vec<u32>) {
        let dir = self.index_dir();
        self.bg("星図データを落としています...", LogTo::Env, move |job| {
            bridge::release_indexes();
            let log = |l: &str| job.log(l);
            let prog = |d: u64, t: u64| job.progress(d, t);
            let stop = || job.cancelled();
            let r = zahyou_net::fetch_index(&survey::names_of(&scales), &dir, &prog, &stop, &log);
            let st = survey::survey(&dir);
            let ready = st.ready;
            job.send(Msg::Env(Box::new(st)));
            if !r.ok && (r.cancelled || job.cancelled()) {
                "中止しました。".into()
            } else if !r.ok {
                "落とせなかったファイルがあります。ログを見てください。".into()
            } else if ready {
                "星図データを入れました。".into()
            } else {
                "落とし終えましたが、まだ足りません。".into()
            }
        });
    }

    /// 「まとめて準備する」
    pub fn prepare_all(&mut self) {
        let scales = self.marked();
        if scales.is_empty() {
            let want = zahyou_net::recommended_scales();
            let have = match &self.env_state {
                Some(st) => st.index.scales.clone(),
                None => survey::index_state(&self.index_dir()).scales,
            };
            let need: Vec<u32> = want.iter().copied().filter(|s| !have.contains(s)).collect();
            if !need.is_empty() {
                let mb = zahyou_net::scales_mb(&need) as f64;
                let q = format!(
                    "段が選ばれていないので、おすすめ (画角 {}′ 以上) をそろえます。\n足りない {} 段 / 約 {:.1} GB を\n{} へ落とします。よろしいですか?\n\n(段は下のチェックか「必要な段を選ぶ」で指定できます。\n途中で中止しても、次に押せば続きから再開します)",
                    survey::g(zahyou_net::RECOMMENDED_MIN_FOV),
                    need.len(),
                    mb / 1024.0,
                    self.s.indexdir
                );
                if !self.ask(&q) {
                    self.status = "やめました。".into();
                    return;
                }
            }
        }
        let dir = self.index_dir();
        self.bg("準備しています...", LogTo::Env, move |job| {
            bridge::release_indexes();
            let log = |l: &str| job.log(l);
            let prog = |d: u64, t: u64| job.progress(d, t);
            let stop = || job.cancelled();
            let st = survey::prepare_all(&dir, &scales, &log, &prog, &stop);
            let ready = st.ready;
            job.send(Msg::Env(Box::new(st)));
            if job.cancelled() {
                "中止しました。".into()
            } else if ready {
                "準備できました。オフライン解析できます。".into()
            } else {
                "まだ足りません。ログを見てください。".into()
            }
        });
    }

    // ============================================================ 更新 ===
    pub fn check_update(&mut self) {
        self.update_msg = "確認しています...".into();
        self.bg("更新を確認しています...", LogTo::Run, move |job| {
            let exe = std::env::current_exe().unwrap_or_default();
            let stop = || job.cancelled();
            let chk = zahyou_net::check_update(&exe, APP_VER, &stop);
            let msg = chk.message.clone();
            job.send(Msg::Update(Box::new(chk), false));
            msg
        });
    }

    fn show_update(&mut self, chk: zahyou_net::UpdateCheck, quiet: bool) {
        let available = chk.state == zahyou_net::UpdateState::Available;
        self.update_msg = chk.message.clone();
        self.update_can_apply = available;
        if available {
            if let Some(why) = &chk.blocked {
                self.update_msg = format!("{}  {why}", chk.message);
                self.update_can_apply = false;
            }
            if quiet {
                self.status = format!("{}  ［使い方］タブから更新できます。", chk.message);
            }
        }
        self.update_state = Some(chk);
    }

    pub fn apply_update(&mut self) {
        let rel = match &self.update_state {
            Some(st) if st.state == zahyou_net::UpdateState::Available => st.release.clone(),
            _ => None,
        };
        let Some(rel) = rel else {
            self.info("先に「更新を確認」を押してください。");
            return;
        };
        let q = format!(
            "{}（{} 版）に入れ替えます。\n\n  1. 新しい zahyou.exe を落とします（{:.0} MB）\n  2. 中身を確かめます（SHA256）\n  3. zahyou を閉じて入れ替え、起動し直します\n\nいまの版は zahyou.exe.old に名前を変えておき、次に起動したときに消します。\n設定と、覚えた座標はそのまま引き継がれます。",
            rel.tag,
            rel.updated,
            rel.size as f64 / 1e6
        );
        if !self.ask(&q) {
            return;
        }
        self.tab = TopTab::Run;
        self.out_tab = ResultTab::Log; // こちらからの切り替え(v6 はここで好みを「ログ」に上書きしていた)
        self.bg("更新しています...", LogTo::Run, move |job| {
            let log = |l: &str| job.log(l);
            log(&"=".repeat(56));
            log(&format!("更新: {} ({} 版)", rel.tag, rel.updated));
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(e) => {
                    log(&format!("動いている exe の場所が分かりません ({e})"));
                    return "更新をやめました。".into();
                }
            };
            let prog = |d: u64, t: u64| job.progress(d, t);
            let stop = || job.cancelled();
            let new = match zahyou_net::download_update(&rel, &exe, &prog, &stop, &log) {
                Ok(p) => p,
                Err(e) => {
                    log(&format!("  {e}"));
                    return "更新をやめました。".into();
                }
            };
            let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
            match zahyou_net::apply_update(&exe, &new, Some(&args), &log) {
                Ok(_) => {
                    job.send(Msg::QuitForUpdate);
                    "入れ替えています...".into()
                }
                Err(e) => {
                    log(&format!("  {e}"));
                    "入れ替えられませんでした。".into()
                }
            }
        });
    }

    // ============================================================ 1 フレーム ===
    /// 画面 1 枚ぶん(eframe の update からも、画面を出さない自己診断からも呼ぶ)
    pub fn frame(&mut self, ctx: &egui::Context) {
        self.pump();
        self.take_dropped(ctx);
        self.placed.clear();
        self.window_rect = ctx.content_rect();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_settings();
        }
        self.bottom_bar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.th.bg).inner_margin(egui::Margin { left: 8, right: 8, top: 8, bottom: 0 }))
            .show(ctx, |ui| {
                let tabs = [(TopTab::Run, "  解析  "), (TopTab::Env, "  準備  "), (TopTab::Help, "  使い方  ")];
                if let Some(t) = super::widgets::tab_strip(ui, self.th, self.tab, &tabs, true) {
                    self.tab = t;
                }
                ui.add_space(8.0);
                match self.tab {
                    TopTab::Run => self.run_tab(ui),
                    TopTab::Env => self.env_tab(ui),
                    TopTab::Help => self.help_tab(ui),
                }
            });
        self.frames += 1;
        if self.first_frame_ms.is_none() {
            let ms = crate::process_elapsed_ms();
            self.first_frame_ms = Some(ms);
            // v6 は「準備できました」を出そうとして、直後に空の一言で上書きしていた(gui.md 7.1)
            if self.status.is_empty() {
                let font = self.fonts.ui.as_deref().map(|f| f.split(" (").next().unwrap_or(f).to_string());
                self.status = format!("準備できました ({:.1} 秒 / フォント: {})", ms / 1000.0, font.as_deref().unwrap_or("日本語フォントなし"));
            }
        }
        if self.busy || self.quit_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    /// 下のバー(状態表示・進み具合・配色ボタン)
    fn bottom_bar(&mut self, ctx: &egui::Context) {
        let th = self.th;
        let resp = egui::TopBottomPanel::bottom("bar")
            .frame(egui::Frame::new().fill(th.bg).inner_margin(egui::Margin::symmetric(10, 4)))
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let label = if self.s.theme == super::theme::ThemeName::Dark { "☀ 明るく" } else { "🌙 暗く" };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let b = ui.add(egui::Button::new(label).min_size(egui::vec2(88.0, 0.0)));
                        if b.clicked() {
                            self.toggle_theme();
                        }
                        self.placed.push(Placed { name: "配色ボタン".into(), rect: b.rect, frame: self.window_rect });
                        ui.add_space(10.0);
                        let (value, since) = match self.progress {
                            Progress::Idle => (Some(0.0), None),
                            Progress::Busy(t0) => (None, Some(t0)),
                            Progress::Det(d, t) => (Some(if t > 0 { d as f32 / t as f32 } else { 0.0 }), None),
                        };
                        super::widgets::progress_bar(ui, th, 220.0, value, since);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&self.status).color(th.muted)).truncate());
                        });
                    });
                });
            });
        self.placed.push(Placed { name: "下のバー".into(), rect: resp.response.rect, frame: self.window_rect });
    }
}

impl eframe::App for ZahyouApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
        super::shot::after_frame(self, ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.th.bg.to_normalized_gamma_f32()
    }
}

/// 図の内側のタブの番号
pub fn fig_index(t: ResultTab) -> Option<usize> {
    match t {
        ResultTab::Orig => Some(0),
        ResultTab::NorthUp => Some(1),
        ResultTab::Log => None,
    }
}

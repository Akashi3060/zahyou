//! 解析の流れ(画面を持たない)。Python 版 `dev/nb_main.py` の `run()` と、`dev/zahyou_plot.py` の
//! `wcs_summary`・`print_solution`・`report_offsets`、`dev/zahyou_core.py` の `solver_diagnostics`・
//! `index_catalog`・`_solve_offline` の前後のログ(画素スケールと段の案内・解けなかったときの案内)を移したもの。
//!
//! 流れ: 画像を読む → 星を検出する(60 個まで粘る)→ つながりを決める → 目標を決める → 画素スケールを推す →
//! (画像の WCS → オンライン → オフライン の順に)解く → 検算 → 結果のまとめ → 図 1(撮影時の向き)→
//! ずれ → 図 2(北が上)。ログの文言は Python 版と同じ(GUI の自己診断がログの「解析結果」を見る)。
//!
//! 画面(app の gui)はワーカーのスレッドで [`run`] を呼び、`cancel` を立てて中止を伝える。
//! ログは `log` に 1 行ずつ(先頭に改行を含むことがある。Python の `_log` と同じ)届く。
//!
//! 係の分担(オフライン解析): 事前のログ(画素スケール・手持ちの段・足りない段)と、解けなかったときの案内と、
//! 中止の「■ 中止しました。」はここが出す。`zahyou_solver::solve_offline_detailed` が出すのは深い検出と
//! 試行のはしごのログ(`[i/N] ラベル (最大 t 秒)`・探索・解の要約・`✅ 解析成功 (ラベル)`・星が 1〜2 個のときの
//! 「❌ 星が N 個しかない」・途中の失敗)。解けなかったときの案内は、解法が返す止まった理由(`OfflineStop`)と
//! 深い検出の星の数で選ぶ(Python の `_solve_offline` の末尾と同じ分岐)。
#![forbid(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zahyou_image::{DetectOpts, Loaded, Prep, Source};
use zahyou_index::Catalog;
use zahyou_solver::{Control, OfflineOutcome, OfflineRequest, OfflineStop, Params, SolveError};
use zahyou_wcs::{Card, CardValue, Wcs};

use crate::scale::{self, fmt_g};

/// 星図データの既定の置き場所(Python の `DEFAULT_INDEX_DIR`)。
pub const DEFAULT_INDEX_DIR: &str = r"C:\AstrometryData";
/// 既定の目標(Python の nb_settings / GUI の初期値)。
pub const DEFAULT_TARGET: &str = "UCAC4 660-021020";
pub const DEFAULT_RA: &str = "04h 07m 38.877s";
pub const DEFAULT_DEC: &str = "+41d 59m 10.512s";

fn rule() -> String {
    "=".repeat(62)
}

/// ログの見出しに出す版(パッケージの版 "0.7.0-dev" を Python 版の言い方 "v7-dev" にする)。
pub fn log_version() -> String {
    let v = env!("CARGO_PKG_VERSION");
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let mut s = match parts.as_slice() {
        ["0", minor, "0"] => format!("v{minor}"),
        ["0", minor, patch] => format!("v{minor}.{patch}"),
        _ => format!("v{core}"),
    };
    if let Some(p) = pre {
        s.push('-');
        s.push_str(p);
    }
    s
}

// ================================================================== 設定と結果 ===

/// 目標の指定方法(Python の INPUT_MODE)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputMode {
    /// 天体名で指定('STAR_NAME')
    #[default]
    StarName,
    /// 赤経・赤緯で指定('COORDS')
    Coords,
    /// 指定しない = 完全ブラインド('NONE')
    None,
}

/// 解析の方法(Python の SOLVE_MODE)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SolveMode {
    /// インターネットがあればオンライン、だめならオフライン('AUTO')
    #[default]
    Auto,
    /// オンラインのみ('ONLINE')
    Online,
    /// オフラインのみ('OFFLINE')
    Offline,
}

/// 解析 1 回の設定(Python の run() が global から読む 14 個 + 星図データの置き場所)。
#[derive(Clone, Debug)]
pub struct RunParams {
    pub mode: InputMode,
    /// 天体名(mode = StarName のとき)
    pub name: String,
    /// 赤経・赤緯の文字列(mode = Coords のとき。天体名を引けなかったときの代わりにも使う)
    pub ra_str: String,
    pub dec_str: String,
    /// 焦点距離 [mm](分からなければ None)
    pub focal_mm: Option<f64>,
    pub solve_mode: SolveMode,
    /// オンライン解析を待つ時間 [秒](既定 120)
    pub online_timeout_s: f64,
    /// オフライン解析にかけてよい合計の時間 [秒](既定 300)
    pub offline_timeout_s: f64,
    /// 目標の方向を手がかりにする
    pub use_target_as_hint: bool,
    /// 手がかりの半径 [度](既定 5)
    pub search_radius_deg: f64,
    /// 画像に入っている WCS を使わずに解き直す(既定 true)
    pub ignore_existing_wcs: bool,
    /// 検出した星を図 1 に重ねる
    pub show_sources: bool,
    /// 画像のパス
    pub image_path: PathBuf,
    /// 星図データの置き場所(空なら C:\AstrometryData)
    pub index_dir: PathBuf,
    /// nova.astrometry.net の API キー(None なら環境変数 ASTROMETRY_API_KEY)
    pub api_key: Option<String>,
}

impl Default for RunParams {
    fn default() -> Self {
        RunParams {
            mode: InputMode::StarName,
            name: DEFAULT_TARGET.to_string(),
            ra_str: DEFAULT_RA.to_string(),
            dec_str: DEFAULT_DEC.to_string(),
            focal_mm: None,
            solve_mode: SolveMode::Auto,
            online_timeout_s: 120.0,
            offline_timeout_s: 300.0,
            use_target_as_hint: true,
            search_radius_deg: 5.0,
            ignore_existing_wcs: true,
            show_sources: true,
            image_path: PathBuf::new(),
            index_dir: PathBuf::from(DEFAULT_INDEX_DIR),
            api_key: None,
        }
    }
}

/// 解けた WCS の要点(Python の `wcs_summary`。位置角とパリティは画面の見た目 = flip_y を考えたもの)。
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// 画像中心 ((幅−1)/2, (高さ−1)/2) の赤経・赤緯 [度](ICRS)
    pub center_ra: f64,
    pub center_dec: f64,
    /// 画素スケール [″/px](CD の列の長さの平均)
    pub scale: f64,
    /// 画面の上向きの位置角 [度](北から東回り、[0, 360))
    pub pa_screen_up: f64,
    /// 画面で鏡像になっているか(北が上のとき東が右なら鏡像)
    pub mirrored_on_screen: bool,
    /// 視野 [分角](幅 × スケール、高さ × スケール)
    pub fov_x: f64,
    pub fov_y: f64,
}

impl Summary {
    /// ログ・画面に出すパリティの文(Python と同じ)。
    pub fn parity_text(&self) -> &'static str {
        if self.mirrored_on_screen { "負 (鏡像)" } else { "正 (鏡像なし)" }
    }
}

/// 画像中心から目標までのずれ(Python の `report_offsets` の戻り値)。
#[derive(Clone, Debug, PartialEq)]
pub struct Offsets {
    /// 角距離 [分角]
    pub sep_arcmin: f64,
    /// 赤経方向 [分角](東が正。画面には絶対値と ra_dir を出す)
    pub ra_arcmin: f64,
    /// "東" か "西"(0 は西。Python と同じ)
    pub ra_dir: &'static str,
    /// 赤緯方向 [分角](北が正)
    pub dec_arcmin: f64,
    /// "北" か "南"(0 は南)
    pub dec_dir: &'static str,
}

/// 解析 1 回の結果(Python の GUI の `Engine.analyze` の戻り値)。
#[derive(Clone, Debug, Default)]
pub struct RunResult {
    /// 解けた WCS(解けない・中止・検算に落ちたときは None。Python の run() の戻り値)
    pub wcs: Option<Wcs>,
    pub summary: Option<Summary>,
    /// 目標があって、図 2 の段まで進んだときだけ
    pub offsets: Option<Offsets>,
    /// 図(目標があれば 撮影時の向き → 北が上 の 2 枚、無ければ 1 枚)
    pub figures: Vec<zahyou_plot::Figure>,
    /// かかった時間 [秒]
    pub seconds: f64,
    /// 終わった時点で中止の合図が立っていたか
    pub cancelled: bool,
    /// 読み込んだ画像の大きさ (幅, 高さ)
    pub image_size: Option<(usize, usize)>,
    /// 表示で上下を反転する画像か(図 1 の向き)
    pub flip_y: bool,
    /// 解いた方法(「画像に入っていた WCS」「オンライン (nova.astrometry.net)」「オフライン (zahyou 内蔵の解析エンジン)」)
    pub how: Option<String>,
    /// 目標の赤経・赤緯 [度](決まったとき)
    pub target: Option<(f64, f64)>,
    /// 検出した星の数
    pub n_sources: usize,
    /// 画像が無い・読めないなど、解析を始められなかった理由(ログにも ❌ で出している)
    pub error: Option<String>,
}

impl RunResult {
    /// 画面に大きく出す 3 つの (見出し, 値)。Python の GUI `_show_result` と同じ。
    /// 目標があればずれ、無ければ画像中心と画素スケール。どちらも無ければ None(画面は「—」)。
    pub fn numbers(&self) -> Option<[(String, String); 3]> {
        if let Some(o) = &self.offsets {
            return Some([
                ("画像中心から目標までの距離".to_string(), format!("{:.3}′", o.sep_arcmin)),
                ("赤経 (RA) 方向".to_string(), format!("{:.3}′ {}", o.ra_arcmin.abs(), o.ra_dir)),
                ("赤緯 (Dec) 方向".to_string(), format!("{:.3}′ {}", o.dec_arcmin.abs(), o.dec_dir)),
            ]);
        }
        let s = self.summary.as_ref()?;
        Some([
            ("画像中心 (赤経)".to_string(), format_ra_hms(s.center_ra, Some(1), false)),
            ("画像中心 (赤緯)".to_string(), format_dec_dms(s.center_dec, Some(0), false, true)),
            ("画素スケール".to_string(), format!("{:.3}″/px", s.scale)),
        ])
    }

    /// 数字の下に出す 1 行(Python の GUI `v_detail`)。
    pub fn detail(&self) -> Option<String> {
        let s = self.summary.as_ref()?;
        Some(format!(
            "画像中心 {}   画素スケール {:.4}″/px   視野 {:.2}′ × {:.2}′   所要 {:.1} 秒",
            format_hmsdms(s.center_ra, s.center_dec, Some(1)),
            s.scale,
            s.fov_x,
            s.fov_y,
            self.seconds
        ))
    }

    /// 状態バーに残す文(Python の GUI `_final_msg`)。
    pub fn status_text(&self) -> String {
        if self.cancelled {
            "中止しました。".to_string()
        } else if self.wcs.is_none() {
            "解けませんでした。ログを見てください。".to_string()
        } else {
            format!("解析できました ({:.1} 秒)。", self.seconds)
        }
    }
}

// ================================================================== 差し替え口 ===

/// オフライン解析の関数の形(`zahyou_solver::solve_offline_detailed` と同じ)。
pub type OfflineSolverFn =
    dyn Fn(&OfflineRequest, &Catalog, &Params, &Control) -> Result<OfflineOutcome, SolveError> + Sync;

/// 差し替えの解法が返す「試行のはしごを試した」結果(深い検出の星の数は 0 とする)。
pub fn tried(solution: Option<zahyou_solver::Solution>) -> OfflineOutcome {
    OfflineOutcome { solution, n_deep: 0, stop: OfflineStop::Tried }
}

/// 試験・自己診断・速さの物差しのための差し替え。画面からは [`RunEnv::default`](= 本物)で使う。
#[derive(Default)]
pub struct RunEnv<'a> {
    /// cache.json の場所(None なら `zahyou_net::cache_path()` = %LOCALAPPDATA%\zahyou\cache.json)
    pub cache_file: Option<PathBuf>,
    /// オフライン解析(None なら `zahyou_solver::solve_offline_detailed`)
    pub offline_solver: Option<&'a OfflineSolverFn>,
    /// AUTO のときのつながりの判定(None なら `zahyou_net::internet_available()`)
    pub online: Option<bool>,
    /// 図を描かない(速さの物差し・試験用)
    pub skip_figures: bool,
}

// ================================================================== 実行本体 ===

/// 解析を 1 回通す(Python の `run()`)。画面のワーカーから呼ぶ。panic しない(内部で起きても結果の error に入れる)。
pub fn run(params: &RunParams, cancel: &AtomicBool, log: &(dyn Fn(&str) + Sync)) -> RunResult {
    run_with(params, &RunEnv::default(), cancel, log)
}

/// [`run`] の差し替え口つきの版。
pub fn run_with(params: &RunParams, env: &RunEnv, cancel: &AtomicBool, log: &(dyn Fn(&str) + Sync)) -> RunResult {
    let t0 = Instant::now();
    let mut res = match catch_unwind(AssertUnwindSafe(|| Runner::new(params, env, cancel, log, t0).run())) {
        Ok(r) => r,
        Err(e) => {
            let what = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "不明".to_string());
            let msg = format!("内部のエラーで止まりました: {what}");
            log(&format!("\n❌ {msg}"));
            RunResult { error: Some(msg), ..Default::default() }
        }
    };
    res.seconds = t0.elapsed().as_secs_f64();
    res.cancelled = cancel.load(Ordering::Relaxed);
    res
}

struct Runner<'a> {
    p: &'a RunParams,
    env: &'a RunEnv<'a>,
    cancel: &'a AtomicBool,
    log_fn: &'a (dyn Fn(&str) + Sync),
    t0: Instant,
    cache_file: PathBuf,
    res: RunResult,
}

impl<'a> Runner<'a> {
    fn new(
        p: &'a RunParams,
        env: &'a RunEnv<'a>,
        cancel: &'a AtomicBool,
        log_fn: &'a (dyn Fn(&str) + Sync),
        t0: Instant,
    ) -> Self {
        let cache_file = env.cache_file.clone().unwrap_or_else(zahyou_net::cache_path);
        Runner { p, env, cancel, log_fn, t0, cache_file, res: RunResult::default() }
    }

    fn log(&self, s: &str) {
        (self.log_fn)(s);
    }

    /// Python の `_cancel_requested(say)`。
    fn cancelled(&self, say: bool) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            if say {
                self.log("\n■ 中止しました。");
            }
            return true;
        }
        false
    }

    fn fail(mut self, msg: String) -> RunResult {
        self.log(&format!("❌ {msg}"));
        self.res.error = Some(msg);
        self.res
    }

    fn run(mut self) -> RunResult {
        let p = self.p;
        // ------------------------------------------------------------ 画像 ---
        let path = p.image_path.clone();
        if path.as_os_str().is_empty() {
            return self.fail("画像が選ばれていません。".to_string());
        }
        if !path.exists() {
            return self.fail(format!("画像が見つかりません: {}", path.display()));
        }
        let img_name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.log(&rule());
        self.log(&format!("  zahyou {}   {img_name}", log_version()));
        self.log(&rule());

        let loaded = match zahyou_image::load_image_any(&path) {
            Ok(l) => l,
            Err(e) => return self.fail(e.to_string()),
        };
        let (nx, ny) = (loaded.data.width, loaded.data.height);
        self.res.image_size = Some((nx, ny));
        self.res.flip_y = loaded.flip_y;
        self.log(&format!("読み込み : {nx} x {ny} 画素   [{}]", loaded.note));
        if self.cancelled(true) {
            return self.res;
        }

        // -------------------------------------------------------- 星の検出 ---
        // 自前のソルバーは星が多いほど quad を選べるので、60 個見つかるまで粘る
        self.log("\n星を検出しています...");
        let prep = zahyou_image::preprocess(&loaded.data, true);
        for line in &prep.info {
            self.log(&format!("  - {line}"));
        }
        let (sources, thr) = zahyou_image::detect_sources(&prep, &DetectOpts::with_min_sources(60));
        self.res.n_sources = sources.len();
        if !sources.is_empty() {
            let fwhm = median(sources.iter().map(|s| s.fwhm).collect());
            self.log(&format!(
                "  - 星を {} 個検出 (しきい値 {}σ / 典型 FWHM {:.1} px)",
                sources.len(),
                thr.map(fmt_g).unwrap_or_else(|| "?".to_string()),
                fwhm
            ));
            if sources.len() < 8 {
                self.log("  ⚠️ 星が少なすぎます。露出を伸ばすか複数フレームを重ねてください。");
            }
        } else {
            self.log("  ⚠️ 星を検出できませんでした。露出・ピント・雲を確認してください。");
        }
        if self.cancelled(true) {
            return self.res;
        }

        // -------------------------------------------------------- つながり ---
        let online = match p.solve_mode {
            SolveMode::Online => true,
            SolveMode::Offline => false,
            SolveMode::Auto => self.env.online.unwrap_or_else(zahyou_net::internet_available),
        };
        self.log(if online {
            "\n🌍 オンライン環境で実行します。"
        } else {
            "\n🔌 オフライン環境で実行します。"
        });

        // ------------------------------------------------------------ 目標 ---
        let target = self.resolve_target(online);
        self.res.target = target;
        let hint = target.filter(|_| p.use_target_as_hint);
        let (scale_hint, scale_why) =
            scale::estimate_pixel_scale(&loaded.header, p.focal_mm, Some((nx, ny)), &self.cache_file);

        // ------------------------------------------------------------ 解析 ---
        let mut wcs: Option<Wcs> = None;
        let mut how = String::new();

        if !p.ignore_existing_wcs
            && let Some(w) = scale::existing_wcs(&loaded.header)
        {
            self.log("\n✅ 画像に入っていた WCS をそのまま使います。");
            wcs = Some(w);
            how = "画像に入っていた WCS".to_string();
        }

        if wcs.is_none() && online {
            wcs = self.solve_online(&path, &sources, (nx, ny), hint, scale_hint, &scale_why);
            if wcs.is_some() {
                self.log("✅ オンライン解析に成功しました。");
                how = "オンライン (nova.astrometry.net)".to_string();
            } else if p.solve_mode == SolveMode::Online {
                self.log("SOLVE_MODE='ONLINE' なのでここで終了します。");
            } else {
                self.log("   → オフラインの解析に切り替えます。");
            }
            if wcs.is_none() && self.cancelled(true) {
                return self.res;
            }
        }

        if wcs.is_none() && p.solve_mode != SolveMode::Online {
            if self.cancelled(true) {
                return self.res;
            }
            let off = OfflineInput {
                loaded: &loaded,
                prep: &prep,
                sources: &sources,
                hint,
                scale: scale_hint,
                scale_why: &scale_why,
            };
            wcs = self.solve_offline(&off);
            if wcs.is_some() {
                how = "オフライン (zahyou 内蔵の解析エンジン)".to_string();
            }
        }

        let Some(wcs) = wcs else {
            if self.cancelled(false) {
                // 中止はオフライン解析(か、その前の段)が知らせ済み
                return self.res;
            }
            self.log("\n❌ 座標を特定できませんでした。");
            return self.res;
        };

        // ------------------------------------------------------------ 検算 ---
        if !sanity_ok(&wcs, nx, ny) {
            self.log("❌ 解析結果がおかしいので採用しません。");
            return self.res;
        }

        // 位置角とパリティは、図 1 と同じ「画面の見た目」で出す(上下反転を考える)
        let summary = print_solution(&wcs, nx, ny, Some(loaded.flip_y), self.log_fn);
        self.res.summary = Some(summary.clone());
        self.res.how = Some(how.clone());
        self.log(&format!("  解いた方法    : {how}"));
        self.log(&format!("  所要時間      : {:.1} 秒", self.t0.elapsed().as_secs_f64()));

        // 次回この機材で撮った画像は、設定を書かなくても一発で絞り込める
        if let Some(f_mm) = scale::implied_focal_length(&loaded.header, summary.scale) {
            self.log(&format!("  逆算した焦点距離: {f_mm:.0} mm"));
        }
        if scale_hint.is_none() {
            self.log("  → この機材の画素スケールを記憶しました (次からは画角の総当たりをしません)");
        }
        // 覚えられなくても解析は続ける(Python 版も書き込みの失敗を握りつぶす)
        let _ = scale::remember_scale(&loaded.header, nx, ny, summary.scale, &self.cache_file);

        // ------------------------------------------------------------ 描画 ---
        if self.cancelled(true) {
            return self.res;
        }
        let shown: Option<&[Source]> = if p.show_sources { Some(&sources) } else { None };

        let Some(target) = target else {
            self.log("\n⚠️ 目標の座標が無いので、矢印は描きません。");
            self.log("   オフラインで天体名を使いたいときは、一度オンラインで実行して座標を覚えさせるか、INPUT_MODE='COORDS' にしてください。");
            self.figure(|| draw_original(&wcs, &loaded, shown, None));
            self.res.wcs = Some(wcs);
            return self.res;
        };

        match wcs.radec_to_pix_exact(target.0, target.1) {
            Some((tx, ty))
                if tx.is_finite() && ty.is_finite() && 0.0 <= tx && tx < nx as f64 && 0.0 <= ty && ty < ny as f64 =>
            {
                self.log(&format!("\n  目標は画面内です: ピクセル ({tx:.1}, {ty:.1})"));
                if !sources.is_empty() {
                    let d: Vec<f64> = sources.iter().map(|s| (s.x - tx).hypot(s.y - ty)).collect();
                    let i = argmin(&d);
                    let near_px = (3.0 * median(sources.iter().map(|s| s.fwhm).collect())).max(4.0);
                    if d[i] <= near_px {
                        self.log(&format!(
                            "  検出した星と {:.1} px ({:.1}″) で一致しました。",
                            d[i],
                            d[i] * summary.scale
                        ));
                    } else {
                        self.log("  目標そのものは、この検出しきい値では拾えていません (暗い星なら正常です)。");
                    }
                }
            }
            _ => self.log("\n  目標は画面の外です。矢印の向きへ動かしてください。"),
        }

        self.figure(|| draw_original(&wcs, &loaded, shown, Some(target)));
        if self.cancelled(true) {
            // 北が上の図の描き直しは重い
            return self.res;
        }
        let offsets = report_offsets((summary.center_ra, summary.center_dec), target, self.log_fn);
        self.res.offsets = Some(offsets.clone());
        self.figure(|| draw_north_up(&wcs, &loaded, target));
        self.res.wcs = Some(wcs);
        self.res
    }

    /// 図を 1 枚描いて結果に足す(描けなければログに出して先へ進む)。
    fn figure(&mut self, make: impl FnOnce() -> Result<zahyou_plot::Figure, String>) {
        if self.env.skip_figures {
            return;
        }
        match make() {
            Ok(f) => self.res.figures.push(f),
            Err(e) => self.log(&format!("  ⚠️ 図を描けませんでした: {e}")),
        }
    }

    /// Python の `_resolve_target`。決まらなければ None(ブラインド)。
    fn resolve_target(&self, online: bool) -> Option<(f64, f64)> {
        let p = self.p;
        // 完全ブラインド。天体名が空のときも同じ扱いにする(打ち忘れで止まるより親切)
        if p.mode == InputMode::None || (p.mode == InputMode::StarName && p.name.trim().is_empty()) {
            self.log("  目標は指定されていません。画像がどこを向いているかだけ求めます。");
            return None;
        }
        let (ra_s, dec_s) = (p.ra_str.trim(), p.dec_str.trim());
        if p.mode == InputMode::Coords {
            return match parse_coords(ra_s, dec_s) {
                Ok(c) => Some(c),
                Err(e) => {
                    self.log(&format!("❌ 赤経赤緯を読み取れません ({e})。書式を確認してください。"));
                    None
                }
            };
        }
        let name = p.name.trim();
        let log = |s: &str| self.log(s);
        if let Ok(r) = zahyou_net::resolve_target(name, online, &self.cache_file, &log) {
            return Some((r.ra, r.dec));
        }
        if !ra_s.is_empty()
            && !dec_s.is_empty()
            && let Ok(c) = parse_coords(ra_s, dec_s)
        {
            self.log("  → 代わりに RA_INPUT_STR / DEC_INPUT_STR の座標を使います。");
            return Some(c);
        }
        None
    }

    // ---------------------------------------------------------- オンライン ---
    fn solve_online(
        &self,
        path: &Path,
        sources: &[Source],
        (nx, ny): (usize, usize),
        hint: Option<(f64, f64)>,
        scale_hint: Option<f64>,
        scale_why: &str,
    ) -> Option<Wcs> {
        let p = self.p;
        let key = p.api_key.clone().filter(|k| !k.trim().is_empty()).or_else(zahyou_net::api_key_from_env);
        // 「自動」でキーが無いときは、問い合わせずにオフラインへ(毎回「エラー」と出さない。Python 版は既定のキーを
        // 持っていたのでこの分かれ道は無い。キーをソースに書かない Rust 版だけのもの)
        if key.is_none() && p.solve_mode != SolveMode::Online {
            self.log(
                "\n[オンライン] nova.astrometry.net の API キーが無いので、問い合わせません (環境変数 ASTROMETRY_API_KEY)。",
            );
            return None;
        }
        self.log(&format!(
            "\n[オンライン] nova.astrometry.net に問い合わせます (最大 {} 秒)",
            fmt_seconds(p.online_timeout_s)
        ));
        if let Some(s) = scale_hint {
            self.log(&format!("  - 画素スケールの手がかり: {s:.3}″/px ({scale_why})"));
        }
        if hint.is_some() {
            self.log(&format!("  - 目標の方向 (半径 {}°) を手がかりにします。", fmt_g(p.search_radius_deg)));
        }
        let Some(key) = key else {
            self.log(
                "⚠️ オンライン解析でエラー: API キーがありません (環境変数 ASTROMETRY_API_KEY に入れてください)。",
            );
            return None;
        };
        let xy: Vec<(f64, f64)> = sources.iter().map(|s| (s.x, s.y)).collect();
        let req = zahyou_net::OnlineRequest {
            api_key: key.trim(),
            image_path: Some(path),
            sources: Some(&xy),
            width: nx,
            height: ny,
            timeout_s: p.online_timeout_s,
            hint_radec: hint,
            hint_radius_deg: p.search_radius_deg,
            scale_hint,
        };
        let stop = || self.cancel.load(Ordering::Relaxed);
        let log = |s: &str| self.log(s);
        match zahyou_net::solve_online(&req, &stop, &log) {
            Ok(Some(sol)) => match Wcs::from_header_cards(&cards_from_strings(&sol.cards)) {
                Ok(w) => Some(w),
                Err(e) => {
                    self.log(&format!("⚠️ オンライン解析でエラー: 解の WCS を読めませんでした: {e}"));
                    None
                }
            },
            Ok(None) => None,
            Err(zahyou_net::NetError::Cancelled) => None,
            Err(zahyou_net::NetError::Failed(m)) => {
                self.log(&format!("⚠️ オンライン解析でエラー: {m}"));
                None
            }
        }
    }

    // ---------------------------------------------------------- オフライン ---
    fn solve_offline(&self, inp: &OfflineInput) -> Option<Wcs> {
        let p = self.p;
        let index_dir =
            if p.index_dir.as_os_str().is_empty() { PathBuf::from(DEFAULT_INDEX_DIR) } else { p.index_dir.clone() };
        self.log("\n[オフライン] 星図データを確認しています...");
        let diag = solver_diagnostics(&index_dir);
        for m in &diag.messages {
            self.log(&format!("⚠️ {m}"));
        }
        let catalog = diag.catalog.clone().filter(|_| diag.ok)?;
        self.log(&format!("  - 星図データ {} ファイル ({})", diag.indexes.len(), index_dir.display()));

        let (nx, ny) = (inp.loaded.data.width, inp.loaded.data.height);
        let pre = offline_prelog(inp.scale, inp.scale_why, nx, ny, &diag.indexes);
        for line in &pre {
            self.log(line);
        }

        let timeout = p.offline_timeout_s;
        let hint = inp.hint.map(|(ra, dec)| (ra, dec, p.search_radius_deg));
        let req = OfflineRequest {
            prep: inp.prep,
            sources: inp.sources,
            width: nx,
            height: ny,
            scale: inp.scale,
            hint,
            timeout_s: timeout,
        };
        let deadline = Duration::try_from_secs_f64(timeout.clamp(0.0, 1e7)).ok().map(|d| Instant::now() + d);
        let ctl = Control { deadline, cancel: self.cancel, log: self.log_fn };
        let params = Params::default();
        let solved = match self.env.offline_solver {
            Some(f) => f(&req, &catalog, &params, &ctl),
            None => zahyou_solver::solve_offline_detailed(&req, &catalog, &params, &ctl),
        };
        let out = match solved {
            Err(SolveError::Cancelled) => {
                self.log("\n  ■ 中止しました。");
                return None;
            }
            Ok(o) => o,
        };
        if let Some(sol) = out.solution {
            return Some(sol.wcs);
        }
        // 「写っている星が N 個」は標準の検出と深い検出の多いほう(Python の max(len(sources), len(deep_stars)))
        let n_best = inp.sources.len().max(out.n_deep);
        match out.stop {
            OfflineStop::Tried => {
                if self.cancelled(false) {
                    self.log("\n  ■ 中止しました。");
                    return None;
                }
                for line in offline_advice(n_best, inp.scale, nx, ny, &diag.indexes) {
                    self.log(&line);
                }
            }
            // 星が 1〜2 個なら解法が「❌ 星が N 個しかない」を出している。0 個の文はここで出す(Python と同じく案内は出さない)
            OfflineStop::TooFewStars if n_best == 0 => self.log("  ❌ 星が 1 つも無いので解析できません。"),
            OfflineStop::TooFewStars => {}
            OfflineStop::NoIndex => self.log("  ❌ 使える星図データがありません。［準備］タブで落としてください。"),
            // 解法が「❌ オフライン解析が途中で失敗しました」を出している(Python も案内は出さない)
            OfflineStop::Failed => {}
        }
        None
    }
}

struct OfflineInput<'a> {
    loaded: &'a Loaded,
    prep: &'a Prep,
    sources: &'a [Source],
    hint: Option<(f64, f64)>,
    scale: Option<f64>,
    scale_why: &'a str,
}

/// 秒数を Python の int のように出す(整数なら小数点なし)。
fn fmt_seconds(t: f64) -> String {
    if t.is_finite() && t.fract() == 0.0 && t.abs() < 1e15 { format!("{}", t as i64) } else { fmt_g(t) }
}

/// 解いた結果がまともか(Python の `_sanity_ok`): スケールが 0.01〜3600″/px で、画像の中央 (幅/2, 高さ/2) の座標が有限。
pub fn sanity_ok(wcs: &Wcs, nx: usize, ny: usize) -> bool {
    let s = scale::wcs_scale_arcsec(wcs);
    let (ra, dec) = wcs.pix_to_radec(nx as f64 / 2.0, ny as f64 / 2.0);
    scale::SCALE_MIN < s && s < scale::SCALE_MAX && ra.is_finite() && dec.is_finite()
}

/// numpy の `median`(偶数個なら真ん中 2 つの平均)。空なら NaN。
fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// numpy の `argmin`(最初の最小。NaN があれば最初の NaN)。
fn argmin(v: &[f64]) -> usize {
    let mut best = 0;
    for (i, x) in v.iter().enumerate() {
        if x.is_nan() {
            return i;
        }
        if *x < v[best] {
            best = i;
        }
    }
    best
}

// ================================================================== 結果のまとめ ===

/// 解けた WCS の要点(Python の `wcs_summary`)。
///
/// `flip_y` は「表示で上下を反転するか」(`Loaded::flip_y`。図 1 も同じ向きで描く)。渡すと位置角とパリティを
/// **画面の見た目**で出す: 画面の上 = flip_y なら配列の −y 方向。パリティは画面 (x 右・y 上) で
/// 「北が上なら東が左」(空を見上げたままの向き)を「正 (鏡像なし)」とする = 画面での det(CD) < 0。
/// None のときは以前の動き(配列の +y を上、det(CD) > 0 を「正」)。
pub fn wcs_summary(wcs: &Wcs, nx: usize, ny: usize, flip_y: Option<bool>) -> Summary {
    let s = scale::proj_plane_pixel_scales(wcs);
    // Python は秒角にしてから平均する(estimate_pixel_scale とは順が逆)
    let scale = (s[0] * 3600.0 + s[1] * 3600.0) / 2.0;
    let (cx, cy) = ((nx as f64 - 1.0) / 2.0, (ny as f64 - 1.0) / 2.0);
    let center = wcs.pix_to_radec(cx, cy);
    let up = wcs.pix_to_radec(cx, cy + if flip_y == Some(true) { -10.0 } else { 10.0 });
    let pa = zahyou_wcs::position_angle(center, up).to_degrees();
    let det = wcs.cd[0][0] * wcs.cd[1][1] - wcs.cd[0][1] * wcs.cd[1][0];
    let mirrored = match flip_y {
        // Python の `not det > 0`(NaN も鏡像)
        None => det.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater),
        Some(true) => -det > 0.0,
        Some(false) => det > 0.0,
    };
    Summary {
        center_ra: center.0,
        center_dec: center.1,
        scale,
        pa_screen_up: pa,
        mirrored_on_screen: mirrored,
        fov_x: nx as f64 * scale / 60.0,
        fov_y: ny as f64 * scale / 60.0,
    }
}

/// 「解析結果」の枠をログに出して要点を返す(Python の `print_solution`)。
pub fn print_solution(wcs: &Wcs, nx: usize, ny: usize, flip_y: Option<bool>, log: &(dyn Fn(&str) + Sync)) -> Summary {
    let s = wcs_summary(wcs, nx, ny, flip_y);
    for line in solution_lines(&s) {
        log(&line);
    }
    s
}

/// 「解析結果」の枠の行(先頭の行は改行で始まる)。
pub fn solution_lines(s: &Summary) -> Vec<String> {
    vec![
        format!("\n{}", rule()),
        "  解析結果".to_string(),
        rule(),
        format!("  画像中心      : {}", format_hmsdms(s.center_ra, s.center_dec, Some(2))),
        format!("                  (RA {:.6}°, Dec {:.6}°)", s.center_ra, s.center_dec),
        format!("  画素スケール  : {:.4} 秒角/px", s.scale),
        format!("  視野          : {:.2}′ x {:.2}′", s.fov_x, s.fov_y),
        format!("  画面上方向    : 位置角 {:.2}°(北から東回り)", s.pa_screen_up),
        format!("  パリティ      : {}", s.parity_text()),
    ]
}

/// 画像中心から目標までのずれ(Python の `report_offsets`)。ログにも出す。座標は度。
pub fn report_offsets(center: (f64, f64), target: (f64, f64), log: &(dyn Fn(&str) + Sync)) -> Offsets {
    let o = offsets(center, target);
    for line in offsets_lines(&o) {
        log(&line);
    }
    o
}

/// ずれの計算だけ(ログに出さない)。
pub fn offsets(center: (f64, f64), target: (f64, f64)) -> Offsets {
    let (sep, dra, ddec) = zahyou_wcs::spherical_offsets(center, target);
    let ra_min = dra.to_degrees() * 60.0;
    let dec_min = ddec.to_degrees() * 60.0;
    Offsets {
        sep_arcmin: sep.to_degrees() * 60.0,
        ra_arcmin: ra_min,
        ra_dir: if ra_min > 0.0 { "東" } else { "西" },
        dec_arcmin: dec_min,
        dec_dir: if dec_min > 0.0 { "北" } else { "南" },
    }
}

/// ずれのログの行(先頭の行は改行で始まる)。
pub fn offsets_lines(o: &Offsets) -> Vec<String> {
    vec![
        "\n  画像中心から目標までのずれ".to_string(),
        format!("    全角距離      : {:.3}′  ({:.1}″)", o.sep_arcmin, o.sep_arcmin * 60.0),
        format!("    赤経(RA) 方向 : {:.3}′ {}へ", o.ra_arcmin.abs(), o.ra_dir),
        format!("    赤緯(Dec)方向 : {:.3}′ {}へ", o.dec_arcmin.abs(), o.dec_dir),
    ]
}

/// 図 2 の題(Python の `plot_north_up`。図の題は zahyou-plot が作るので、試験で照合するときだけ使う)。
#[cfg(test)]
pub fn north_up_title(o: &Offsets) -> String {
    format!(
        "北が上・東が左\n距離 {:.2}′ | RA {:.2}′({}) Dec {:.2}′({})",
        o.sep_arcmin,
        o.ra_arcmin.abs(),
        o.ra_dir,
        o.dec_arcmin.abs(),
        o.dec_dir
    )
}

// ================================================================== 図(zahyou-plot への受け渡し) ===

/// 図 1「撮影時の向き」(Python の `plot_original`)。zahyou-plot を呼ぶのはこの関数と [`draw_north_up`] だけ。
/// 目標が None なら中心の × だけ(Python の draw_arrow=False)。`sources` が None なら検出した星を描かない。
fn draw_original(
    wcs: &Wcs,
    loaded: &Loaded,
    sources: Option<&[Source]>,
    target: Option<(f64, f64)>,
) -> Result<zahyou_plot::Figure, String> {
    zahyou_plot::figure_original(&loaded.display, wcs, sources.unwrap_or(&[]), target, loaded.flip_y, sources.is_some())
        .map_err(|e| e.to_string())
}

/// 図 2「北が上・東が左」(Python の `plot_north_up`。画像を描き直すので重い)。
fn draw_north_up(wcs: &Wcs, loaded: &Loaded, target: (f64, f64)) -> Result<zahyou_plot::Figure, String> {
    zahyou_plot::figure_north_up(&loaded.display, wcs, Some(target)).map_err(|e| e.to_string())
}

// ================================================================== 星図データ ===

/// 星図データの事前確認(Python の `solver_diagnostics`)。
#[derive(Clone, Debug)]
pub struct Diagnostics {
    /// オフライン解析できるか
    pub ok: bool,
    /// 画面とログに出す注意(Python と同じ文)
    pub messages: Vec<String>,
    /// 検査に通った索引ファイルの名前(名前の順)
    pub indexes: Vec<String>,
    /// 目録(ok のとき)。同じ置き場所・同じ中身なら前回のものを使い回す
    pub catalog: Option<Arc<Catalog>>,
}

/// 目録の「中身の目印」: 置き場所・フォルダーの更新時刻・索引ファイルの (名前, 大きさ, 更新時刻)。
type CatalogKey = (PathBuf, Option<SystemTime>, Vec<(String, u64, Option<SystemTime>)>);

static CATALOG: Mutex<Option<(CatalogKey, Arc<Catalog>)>> = Mutex::new(None);

fn catalog_key(dir: &Path) -> Option<CatalogKey> {
    let d = std::path::absolute(dir).ok()?;
    let dir_mtime = std::fs::metadata(&d).ok()?.modified().ok();
    let mut files = Vec::new();
    for e in std::fs::read_dir(&d).ok()? {
        let e = e.ok()?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("index-") && name.ends_with(".fits") {
            let m = e.metadata().ok()?;
            files.push((name, m.len(), m.modified().ok()));
        }
    }
    files.sort();
    // Windows は大文字小文字を区別しない(Python の normcase)
    let norm = PathBuf::from(d.to_string_lossy().to_lowercase());
    Some((norm, dir_mtime, files))
}

/// 星図データの目録(Python の `index_catalog`)。置き場所と中身が前回と同じなら使い回す。
pub fn index_catalog(dir: &Path) -> Arc<Catalog> {
    let key = catalog_key(dir);
    let mut g = CATALOG.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(k), Some((old, cat))) = (&key, g.as_ref())
        && k == old
    {
        return cat.clone();
    }
    if let Some((_, old)) = g.take() {
        old.close();
    }
    let cat = Arc::new(Catalog::scan(dir));
    if let Some(k) = key {
        *g = Some((k, cat.clone()));
    }
    cat
}

/// 覚えている目録を手放す(索引ファイルの mmap を閉じる)。Windows では mmap 中のファイルを消したり
/// 置き換えたりできないので、画面は星図データを落とし直す前にこれを呼ぶ(Python の `release_index_catalog`)。
pub fn release_index_catalog() {
    let mut g = CATALOG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, cat)) = g.take() {
        cat.close();
    }
}

/// 星図データが本当に使えるかを事前に確かめる(Python の `solver_diagnostics`)。
pub fn solver_diagnostics(index_dir: &Path) -> Diagnostics {
    let mut d = Diagnostics { ok: false, messages: Vec::new(), indexes: Vec::new(), catalog: None };
    if !index_dir.is_dir() {
        d.messages.push(format!(
            "星図データの置き場所 {} がありません。［準備］タブで星図データを落とすか、置き場所を指定し直してください。",
            index_dir.display()
        ));
        return d;
    }
    let catalog = index_catalog(index_dir);
    let mut names: Vec<String> =
        catalog.files.iter().filter_map(|f| f.path.file_name().map(|s| s.to_string_lossy().into_owned())).collect();
    names.sort();
    names.dedup();
    d.indexes = names;
    let problems = &catalog.problems;
    if !problems.is_empty() {
        d.messages.push(format!(
            "使えない星図データが {} 個あるので読み飛ばします。［準備］タブで落とし直してください。",
            problems.len()
        ));
        for (path, why) in problems.iter().take(5) {
            let base = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            d.messages.push(format!("  {base}: {why}"));
        }
        if problems.len() > 5 {
            d.messages.push(format!("  ほか {} 個", problems.len() - 5));
        }
    }
    if d.indexes.is_empty() {
        d.messages.push(format!(
            "使える星図データ (index-*.fits) が {} に 1 つもありません。［準備］タブで落としてください。",
            index_dir.display()
        ));
        return d;
    }
    d.ok = true;
    d.catalog = Some(catalog);
    d
}

/// オフライン解析の前に出す行(Python の `_solve_offline` の冒頭: 画素スケールと画角・手持ちの段・足りない段)。
pub fn offline_prelog<S: AsRef<str>>(
    scale: Option<f64>,
    how: &str,
    nx: usize,
    ny: usize,
    installed: &[S],
) -> Vec<String> {
    let mut out = Vec::new();
    let fov = scale.map(|s| (nx as f64 * s / 60.0, ny as f64 * s / 60.0));
    // 星の並び (quad) は「短いほうの辺」に収まる大きさまでしか作れないので、必要な索引は短辺で決まる
    match (scale, fov) {
        (Some(s), Some((w, h))) => {
            out.push(format!("  - 画素スケール {s:.3}″/px ({how}) → 画角 {w:.1}′ x {h:.1}′ (短辺 {:.1}′)", w.min(h)));
        }
        _ => out.push(format!("  - 画素スケール: {how}")),
    }
    if installed.is_empty() {
        out.push("  ❌ 使える星図データがありません。［準備］タブで落としてください。".to_string());
        return out;
    }
    if let Some((lo, hi)) = scale::index_coverage(installed) {
        out.push(format!(
            "  - 手持ちの星図データが対応する画角: {}′ 〜 {}′ (index ファイル {} 個)",
            fmt_g(lo),
            fmt_g(hi),
            installed.len()
        ));
        if let Some((w, h)) = fov {
            let short = w.min(h);
            let (ok, msgs) = scale::missing_index_advice(short, installed);
            for m in msgs {
                out.push(format!("{}{m}", if ok { "  - " } else { "  ⚠️ " }));
            }
            if !ok {
                out.push(format!("     ※ 必要な段は横幅 {w:.1}′ ではなく短辺 {short:.1}′ で決まります。"));
            }
        }
    }
    if scale.is_none() {
        let wide = scale::blind_scale_bounds(nx, installed);
        out.push(format!(
            "  - 探索するスケール: {:.3}〜{:.3}″/px (画角 {:.1}′〜{:.0}′ 相当)",
            wide.0,
            wide.1,
            wide.0 * nx as f64 / 60.0,
            wide.1 * nx as f64 / 60.0
        ));
    }
    out
}

/// すべての試行で解けなかったときの案内(Python の `_solve_offline` の末尾)。
/// `n_best` は標準の検出と深い検出の星の数の多いほう(Python の `n_best`)。
pub fn offline_advice<S: AsRef<str>>(
    n_best: usize,
    scale: Option<f64>,
    nx: usize,
    ny: usize,
    installed: &[S],
) -> Vec<String> {
    let mut out = vec!["\n  ❌ すべての条件で解けませんでした。心当たりの順に:".to_string()];
    if n_best < 15 {
        out.push(format!(
            "     ・写っている星が {n_best} 個と少なめです。露出を伸ばすか、複数フレームを重ねてください。"
        ));
    }
    if let Some((lo, hi)) = scale::index_coverage(installed) {
        out.push(format!("     ・手持ちの星図データは画角 {}′〜{}′ しか解けません。", fmt_g(lo), fmt_g(hi)));
        match scale {
            Some(s) => {
                let (w, h) = (nx as f64 * s / 60.0, ny as f64 * s / 60.0);
                let short = w.min(h);
                let (ok, msgs) = scale::missing_index_advice(short, installed);
                if !ok {
                    out.push(format!(
                        "       この画像は短辺が {short:.1}′ です(横幅 {w:.1}′)。星の並びは短いほうの辺で決まるので:"
                    ));
                    for m in msgs {
                        out.push(format!("         {m}"));
                    }
                }
            }
            None => out.push(
                "       画角が分かれば、足りない段を名指しできます。FOCAL_LENGTH_MM を書いてもう一度試してください。"
                    .to_string(),
            ),
        }
    }
    if scale.is_none() {
        out.push(
            "     ・FOCAL_LENGTH_MM に焦点距離を書くと画角が確定し、総当たりをやめるので格段に速く・確実になります。"
                .to_string(),
        );
    }
    out.push("     ・ピントや雲、極端な露出過多も疑ってください。".to_string());
    out
}

// ================================================================== nova の解(カードの文字列 → Card) ===

/// 80 文字のカードの並びを `Card` にする(値は FITS の規則で読む。`zahyou_index::fits::parse_value`)。
pub fn cards_from_strings(cards: &[String]) -> Vec<Card> {
    let mut out = Vec::with_capacity(cards.len());
    for c in cards {
        let key = c.get(..8.min(c.len())).unwrap_or("").trim_end().to_string();
        if key.is_empty() || key == "END" {
            continue;
        }
        if c.get(8..10) == Some("= ") {
            let v = match zahyou_index::fits::parse_value(c.get(10..).unwrap_or("")) {
                zahyou_index::fits::FitsValue::Null => CardValue::None,
                zahyou_index::fits::FitsValue::Bool(b) => CardValue::Bool(b),
                zahyou_index::fits::FitsValue::Int(i) => match i64::try_from(i) {
                    Ok(i) => CardValue::Int(i),
                    Err(_) => CardValue::Float(i as f64),
                },
                zahyou_index::fits::FitsValue::Float(x) => CardValue::Float(x),
                zahyou_index::fits::FitsValue::Str(s) => CardValue::Str(s),
            };
            out.push(Card::new(&key, v, ""));
        } else {
            out.push(Card::commentary(&key, c.get(8..).unwrap_or("").trim_end()));
        }
    }
    out
}

// ================================================================== 六十進の書き方(astropy の to_string) ===

/// astropy の `u.degree.to(u.hourangle)`(1/15 ではなく、その 1 ビット上)。
pub const DEG_TO_HOUR: f64 = 0.06666666666666668;
/// astropy の `u.hourangle.to(u.deg)`(15 ではなく、その 1 ビット下)。
pub const HOUR_TO_DEG: f64 = 14.999999999999998;

/// astropy の `_decimal_to_sexagesimal_string`(sep は 3 つの区切り、precision は秒の小数の桁。None は最大 8 桁で 0 を外す)。
pub fn format_sexagesimal(angle: f64, precision: Option<usize>, pad: bool, sep: [&str; 3]) -> String {
    if !angle.is_finite() {
        return if angle.is_nan() {
            "nan".to_string()
        } else if angle > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    // _decimal_to_sexagesimal
    let sign0 = 1f64.copysign(angle);
    let fa = angle.abs();
    let d = fa.trunc();
    let df = fa - d;
    let mfull = df * 60.0;
    let m = mfull.trunc();
    let mf = mfull - m;
    let s = mf * 60.0;
    let v0 = (sign0 * d).floor();
    let v1 = sign0 * m.floor();
    let v2 = sign0 * s;
    let sign = 1f64.copysign(v0);
    let mut values = [v0.abs(), v1.abs(), v2.abs()];
    let pad = if pad { if sign == -1.0 { 3 } else { 2 } } else { 0 };
    let p = precision.unwrap_or(8);
    // 秒が丸めで 60 になるなら 0 にして繰り上げる(astropy と同じしきい値)
    let thresh = 60.0 - 10f64.powf(-(p as f64));
    if values[2] >= thresh {
        values[2] = 0.0;
        values[1] += 1.0;
    }
    if values[1] >= 60.0 {
        values[1] = 0.0;
        values[0] += 1.0;
    }
    let first = values[0].copysign(sign);
    let mut out = if pad > 0 { format!("{first:0pad$.0}") } else { format!("{first:.0}") };
    out.push_str(sep[0]);
    out.push_str(&format!("{:02}", values[1] as i64));
    out.push_str(sep[1]);
    let mut last = match precision {
        None => {
            let t = format!("{:.8}", values[2]);
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Some(p) => format!("{:.p$}", values[2]),
    };
    if last.len() == 1 || last.as_bytes().get(1) == Some(&b'.') {
        last.insert(0, '0');
    }
    out.push_str(&last);
    out.push_str(sep[2]);
    out
}

/// 赤経を時分秒で(astropy の `ra.to_string(unit='hourangle', sep='hms', precision=p, pad=pad)`)。
pub fn format_ra_hms(ra_deg: f64, precision: Option<usize>, pad: bool) -> String {
    format_sexagesimal(ra_deg * DEG_TO_HOUR, precision, pad, ["h", "m", "s"])
}

/// 赤緯を度分秒で(astropy の `dec.to_string(unit='deg', sep='dms', precision=p, pad=pad, alwayssign=…)`)。
pub fn format_dec_dms(dec_deg: f64, precision: Option<usize>, pad: bool, alwayssign: bool) -> String {
    let s = format_sexagesimal(dec_deg, precision, pad, ["d", "m", "s"]);
    if alwayssign && !s.starts_with('-') { format!("+{s}") } else { s }
}

/// astropy の `SkyCoord.to_string('hmsdms', precision=p)`。
pub fn format_hmsdms(ra_deg: f64, dec_deg: f64, precision: Option<usize>) -> String {
    format!("{} {}", format_ra_hms(ra_deg, precision, true), format_dec_dms(dec_deg, precision, true, true))
}

// ================================================================== 赤経・赤緯の文字列を読む(astropy の SkyCoord(ra, dec)) ===

/// 角度の単位(読んだ値を度にする係数は astropy の `unit.to(u.deg)`)。
#[derive(Clone, Copy, Debug, PartialEq)]
enum AngUnit {
    Hour,
    Deg,
    Arcmin,
    Arcsec,
    Other(f64),
}

impl AngUnit {
    fn to_deg(self) -> f64 {
        match self {
            AngUnit::Hour => HOUR_TO_DEG,
            AngUnit::Deg => 1.0,
            AngUnit::Arcmin => 0.016666666666666666,
            AngUnit::Arcsec => 0.0002777777777777778,
            AngUnit::Other(f) => f,
        }
    }
}

/// astropy の角度の字句(astropy/coordinates/angles/formats.py の `_AngleParser`)。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Sign(f64),
    UInt(f64),
    UFloat(f64),
    Colon,
    Degree,
    Hour,
    Minute,
    Second,
    Simple(AngUnit),
    EastWest(f64),
    NorthSouth(f64),
}

/// 角度の単位の名前(astropy が数のすぐ後ろに書けるとする名前のうち、よく使うもの)。並びは astropy と同じ
/// 名前の順(正規表現の選択肢は先に書いたものが勝つ。例: "arcminute" は "arcmin" + "ute" と読まれて誤りになる)。
/// ここに無い名前(接頭辞つきの "kdeg" など)は読めない(astropy は読む)。
const SIMPLE_UNITS: [(&str, f64); 13] = [
    ("arcmin", 0.016666666666666666),
    ("arcminute", 0.016666666666666666),
    ("arcsec", 0.0002777777777777778),
    ("arcsecond", 0.0002777777777777778),
    ("cy", 360.0),
    ("cycle", 360.0),
    ("mas", 2.777777777777778e-07),
    ("mrad", 0.057295779513082325),
    ("rad", 57.29577951308232),
    ("radian", 57.29577951308232),
    ("uas", 2.7777777777777777e-10),
    ("µas", 2.7777777777777777e-10),
    ("μas", 2.7777777777777777e-10),
];

/// 全角の英数字・記号と全角の空白を半角にする(日本語入力のまま打たれた座標のため。astropy は読まない)。
fn to_halfwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

fn starts_with_at(chars: &[char], i: usize, pat: &str) -> bool {
    let mut j = i;
    for pc in pat.chars() {
        if chars.get(j) != Some(&pc) {
            return false;
        }
        j += 1;
    }
    true
}

/// 正規表現 `a(b(c)?)?` のように、長いものから順に試す(最初に一致した長さ)。
fn match_longest(chars: &[char], i: usize, alts: &[&str]) -> Option<usize> {
    alts.iter().find(|a| starts_with_at(chars, i, a)).map(|a| a.chars().count())
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let digits = |mut j: usize| {
        while j < n && chars[j].is_ascii_digit() {
            j += 1;
        }
        j
    };
    while i < n {
        let c = chars[i];
        if c == ' ' {
            i += 1;
            continue;
        }
        // UFLOAT: ((\d+\.\d*)|(\.\d+))([eE][+-−]?\d+)?
        let j = digits(i);
        let float_end = if j > i && j < n && chars[j] == '.' {
            Some(digits(j + 1))
        } else if j == i && c == '.' && digits(i + 1) > i + 1 {
            Some(digits(i + 1))
        } else {
            None
        };
        if let Some(mut k) = float_end {
            if k < n && (chars[k] == 'e' || chars[k] == 'E') {
                let mut e = k + 1;
                if e < n && matches!(chars[e], '+' | '-' | '−') {
                    e += 1;
                }
                let ee = digits(e);
                if ee > e {
                    k = ee;
                }
            }
            let text: String = chars[i..k].iter().map(|&c| if c == '−' { '-' } else { c }).collect();
            let v = text.parse::<f64>().map_err(|_| format!("数を読めません: {text}"))?;
            out.push(Tok::UFloat(v));
            i = k;
            continue;
        }
        if j > i {
            let text: String = chars[i..j].iter().collect();
            let v = text.parse::<f64>().map_err(|_| format!("数を読めません: {text}"))?;
            out.push(Tok::UInt(v));
            i = j;
            continue;
        }
        match c {
            '+' => {
                out.push(Tok::Sign(1.0));
                i += 1;
                continue;
            }
            '-' | '−' => {
                out.push(Tok::Sign(-1.0));
                i += 1;
                continue;
            }
            _ => {}
        }
        // 東西・南北は文字列の最後の 1 文字だけ([EW]$ / [NS]$)
        if i == n - 1 {
            match c {
                'E' => {
                    out.push(Tok::EastWest(1.0));
                    i += 1;
                    continue;
                }
                'W' => {
                    out.push(Tok::EastWest(-1.0));
                    i += 1;
                    continue;
                }
                'N' => {
                    out.push(Tok::NorthSouth(1.0));
                    i += 1;
                    continue;
                }
                'S' => {
                    out.push(Tok::NorthSouth(-1.0));
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        if let Some((name, f)) = SIMPLE_UNITS.iter().find(|(name, _)| starts_with_at(&chars, i, name)) {
            out.push(Tok::Simple(AngUnit::Other(*f)));
            i += name.chars().count();
            continue;
        }
        if let Some(k) = match_longest(&chars, i, &["minutes", "minute", "min", "m", "′", "'", "ᵐ"]) {
            out.push(Tok::Minute);
            i += k;
            continue;
        }
        if let Some(k) = match_longest(&chars, i, &["seconds", "second", "sec", "s", "″", "\"", "ˢ"]) {
            out.push(Tok::Second);
            i += k;
            continue;
        }
        if let Some(k) = match_longest(&chars, i, &["degrees", "degree", "deg", "d", "°"]) {
            out.push(Tok::Degree);
            i += k;
            continue;
        }
        if let Some(k) = match_longest(&chars, i, &["hours", "hour", "hr", "h", "ʰ"]) {
            out.push(Tok::Hour);
            i += k;
            continue;
        }
        if c == ':' {
            out.push(Tok::Colon);
            i += 1;
            continue;
        }
        return Err(format!("読めない文字があります ({} 文字目の '{c}')", i + 1));
    }
    Ok(out)
}

/// 読んだ値: 1 つの数か、(度または時, 分[, 秒]) の組。
#[derive(Clone, Debug, PartialEq)]
enum AngVal {
    Scalar(f64),
    Tuple(Vec<f64>),
}

fn is_uf(t: &Tok) -> Option<f64> {
    match t {
        Tok::UInt(v) | Tok::UFloat(v) => Some(*v),
        _ => None,
    }
}

/// generic: ufloat | UINT ufloat | UINT : ufloat | UINT UINT ufloat | UINT : UINT : ufloat
fn generic(t: &[Tok]) -> Option<AngVal> {
    use Tok::*;
    match t {
        [a] => Some(AngVal::Scalar(is_uf(a)?)),
        [UInt(a), b] => Some(AngVal::Tuple(vec![*a, is_uf(b)?])),
        [UInt(a), Colon, b] => Some(AngVal::Tuple(vec![*a, is_uf(b)?])),
        [UInt(a), UInt(b), c] => Some(AngVal::Tuple(vec![*a, *b, is_uf(c)?])),
        [UInt(a), Colon, UInt(b), Colon, c] => Some(AngVal::Tuple(vec![*a, *b, is_uf(c)?])),
        _ => None,
    }
}

/// hms / dms の明示の形(UINT HOUR ... / UINT DEGREE ...)と generic HOUR / generic DEGREE。
fn sexa(t: &[Tok], mark: Tok) -> Option<AngVal> {
    use Tok::*;
    if let [UInt(a), m, rest @ ..] = t
        && *m == mark
    {
        let v = match rest {
            [] => Some(AngVal::Scalar(*a)),
            [b] => Some(AngVal::Tuple(vec![*a, is_uf(b)?])),
            [UInt(b), Minute] | [UFloat(b), Minute] => Some(AngVal::Tuple(vec![*a, *b])),
            [UInt(b), Minute, c] | [UInt(b), Minute, c, Second] => Some(AngVal::Tuple(vec![*a, *b, is_uf(c)?])),
            _ => None,
        };
        if v.is_some() {
            return v;
        }
    }
    match t.split_last() {
        Some((last, head)) if *last == mark => generic(head),
        _ => None,
    }
}

/// 角度の文字列を度にする(astropy の `Angle(str)` を `SkyCoord` が度に直した値)。
fn parse_angle_deg(s: &str) -> Result<f64, String> {
    let s = to_halfwidth(s);
    let toks = lex(s.trim_start_matches(' '))?;
    let (sign, body) = match toks.split_first() {
        Some((Tok::Sign(v), rest)) => (*v, rest),
        _ => (1.0, &toks[..]),
    };
    let (dir, body, dir_kind) = match body.split_last() {
        Some((Tok::EastWest(v), rest)) => (*v, rest, 1),
        Some((Tok::NorthSouth(v), rest)) => (*v, rest, 2),
        _ => (1.0, body, 0),
    };
    if body.is_empty() {
        return Err("数がありません".to_string());
    }
    let (val, unit) = if let Some(v) = sexa(body, Tok::Hour) {
        if dir_kind == 2 {
            return Err("時分秒に N・S は付けられません".to_string());
        }
        (v, Some(AngUnit::Hour))
    } else if let Some(v) = sexa(body, Tok::Degree) {
        (v, Some(AngUnit::Deg))
    } else {
        // simple: generic | generic MINUTE | generic SECOND | generic SIMPLE_UNIT
        let (head, unit) = match body.split_last() {
            Some((Tok::Minute, h)) => (h, Some(AngUnit::Arcmin)),
            Some((Tok::Second, h)) => (h, Some(AngUnit::Arcsec)),
            Some((Tok::Simple(u), h)) => (h, Some(*u)),
            _ => (body, None),
        };
        match generic(head) {
            Some(v) => (v, unit),
            None => return Err("書式が正しくありません".to_string()),
        }
    };
    let Some(unit) = unit else {
        return Err("単位がありません(例: 04h07m38.9s・+41d59m10s・61.912d)".to_string());
    };
    let sgn = sign * dir;
    let value = match val {
        AngVal::Scalar(v) => sgn * v,
        AngVal::Tuple(t) => {
            let a0 = sgn * t[0];
            if unit == AngUnit::Hour && !(-24.0 < a0 && a0 < 24.0) && a0.abs() != 24.0 {
                return Err(format!("時が 24 を超えています ({})", fmt_g(a0)));
            }
            if !(0.0..60.0).contains(&t[1]) && t[1] != 60.0 {
                return Err(format!("分は 0〜60 で入れてください ({})", fmt_g(t[1])));
            }
            let mut a = a0.abs() + t[1] / 60.0;
            if let Some(&sec) = t.get(2) {
                if !(0.0..60.0).contains(&sec) && sec != 60.0 {
                    return Err(format!("秒は 0〜60 で入れてください ({})", fmt_g(sec)));
                }
                a += sec / 3600.0;
            }
            a.copysign(a0)
        }
    };
    let f = unit.to_deg();
    Ok(if f == 1.0 { value } else { value * f })
}

/// numpy の浮動小数の `//`(Python の float の floor division と同じ)。
fn py_floordiv(a: f64, b: f64) -> f64 {
    let m = a % b;
    let mut div = (a - m) / b;
    if m != 0.0 && ((b < 0.0) != (m < 0.0)) {
        div -= 1.0;
    }
    if div != 0.0 {
        let f = div.floor();
        if div - f > 0.5 { f + 1.0 } else { f }
    } else {
        0.0f64.copysign(a / b)
    }
}

/// 赤経を [0, 360) へ折り返す(astropy の `Longitude` の `_wrap_at`。範囲内ならそのまま)。
fn wrap_longitude(v: f64) -> f64 {
    // 範囲の中ならそのまま(NaN もここ。astropy も NaN は折り返さない)
    #[allow(clippy::manual_range_contains)]
    if !(v < 0.0 || v >= 360.0) {
        return v;
    }
    let wraps = py_floordiv(v, 360.0);
    let mut x = v - wraps * 360.0;
    if x >= 360.0 {
        x -= 360.0;
    }
    if x < 0.0 {
        x += 360.0;
    }
    x
}

/// 赤経・赤緯の文字列を読む(astropy の `SkyCoord(ra_str, dec_str, frame='icrs')`)。度で返す。
///
/// 読める形: `04h07m38.877s`・`04h 07m 38.877s`・`4h07.5m`・`4.1275h`・`61.912d`・`61.912 deg`・`61°54′43″`・
/// `+41d59m10.512s`・`41:59:10.5d` など(単位が要る。`04 07 38.9` だけでは読まない = astropy と同じ)。
/// astropy との違い: 全角の数字・記号も読む。接頭辞つきの単位(kdeg など)は読まない。
pub fn parse_coords(ra: &str, dec: &str) -> Result<(f64, f64), String> {
    let (ra, dec) = (to_halfwidth(ra), to_halfwidth(dec));
    // astropy: 末尾が N・S の文字列から経度は作らない、E・W から緯度は作らない
    if ra.ends_with(['N', 'S']) {
        return Err("赤経: 末尾の N・S は赤緯にしか付けられません".to_string());
    }
    if dec.ends_with(['E', 'W']) {
        return Err("赤緯: 末尾の E・W は赤経にしか付けられません".to_string());
    }
    let r = parse_angle_deg(&ra).map_err(|e| format!("赤経: {e}"))?;
    let d = parse_angle_deg(&dec).map_err(|e| format!("赤緯: {e}"))?;
    if d.abs() > 90.0 {
        return Err(format!("赤緯は -90°〜+90° の範囲で入れてください ({}°)", fmt_g(d)));
    }
    Ok((wrap_longitude(r), d))
}

// ================================================================== 試験 ===

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text() {
        let v = log_version();
        assert!(v.starts_with('v'), "{v}");
        if env!("CARGO_PKG_VERSION") == "0.7.0-dev" {
            assert_eq!(v, "v7-dev");
        }
    }

    #[test]
    fn sexagesimal_basic() {
        assert_eq!(format_hmsdms(61.891414, 41.964760, Some(2)), "04h07m33.94s +41d57m53.14s");
        assert_eq!(format_ra_hms(61.891414, Some(1), false), "4h07m33.9s");
        assert_eq!(format_dec_dms(41.964760, Some(0), false, true), "+41d57m53s");
        assert_eq!(format_dec_dms(-0.5, Some(1), true, true), "-00d30m00.0s");
        // 秒の繰り上げ
        assert_eq!(format_dec_dms(10.0 + 59.0 / 60.0 + 59.996 / 3600.0, Some(2), true, true), "+11d00m00.00s");
    }

    #[test]
    fn coords_basic() {
        let (r, d) = parse_coords(DEFAULT_RA, DEFAULT_DEC).unwrap();
        assert_eq!(r, 61.91198749999999);
        assert_eq!(d, 41.98625333333334);
        assert!(parse_coords("04 07 38.877", "+41 59 10.512").is_err());
        assert!(parse_coords("61.912", "41.986").is_err());
        assert_eq!(parse_coords("61.912d", "41.986d").unwrap(), (61.912, 41.986));
        assert!(parse_coords("4h07m38s", "95d").is_err());
        assert!(parse_coords("abc", "def").is_err());
        // 全角
        assert_eq!(parse_coords("６１．９１２ｄ", "４１．９８６ｄ").unwrap(), (61.912, 41.986));
    }

    #[test]
    fn diagnostics_missing_dir() {
        let d = solver_diagnostics(Path::new(r"C:\no\such\zahyou-index-dir"));
        assert!(!d.ok);
        assert!(d.messages[0].contains("がありません"), "{:?}", d.messages);
    }

    #[test]
    fn nova_cards() {
        let lines = vec![
            format!("{:<80}", "CTYPE1  = 'RA---TAN-SIP'       / TAN (gnomic) projection + SIP distortions"),
            format!("{:<80}", "CRVAL1  =        61.9119874999 / RA  of reference point"),
            format!("{:<80}", "A_ORDER =                    2 / Polynomial order, axis 1"),
            format!("{:<80}", "COMMENT hello"),
        ];
        let c = cards_from_strings(&lines);
        assert_eq!(c[0].value, CardValue::Str("RA---TAN-SIP".into()));
        assert_eq!(c[1].value, CardValue::Float(61.9119874999));
        assert_eq!(c[2].value, CardValue::Int(2));
        assert_eq!(c[3].key, "COMMENT");
    }
}

/// Python 基準実装(rs/testdata/gen_pipeline.py が作った pipeline_*.json)との突き合わせ。
/// JSON が無ければ SKIP と出して通す。手元の試験画像が要る試験も、無ければ SKIP。
#[cfg(test)]
pub(crate) mod pyref {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex as StdMutex;

    /// rs/testdata の場所(本物のワークスペースでも、本物のファイルを #[path] で読む作業用の写しでも見つける)。
    pub(crate) fn testdata() -> Option<PathBuf> {
        let here = Path::new(file!());
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut cands = vec![manifest.join("../testdata"), manifest.join("../../testdata")];
        if here.is_absolute() {
            if let Some(src) = here.parent() {
                cands.insert(0, src.join("../../testdata"));
            }
        } else if let Some(root) = manifest.parent()
            && let Some(src) = root.join(here).parent()
        {
            cands.insert(0, src.join("../../testdata"));
        }
        cands.into_iter().find(|p| p.join("pipeline_format.json").is_file())
    }

    pub(crate) fn load(name: &str) -> Option<Value> {
        let Some(dir) = testdata() else {
            eprintln!("SKIP: rs/testdata が見つかりません");
            return None;
        };
        let p = dir.join(name);
        match std::fs::read(&p) {
            Ok(b) => serde_json::from_slice(&b).ok(),
            Err(_) => {
                eprintln!("SKIP: {} がありません", p.display());
                None
            }
        }
    }

    pub(crate) fn cards(v: &Value) -> Vec<Card> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        let k = c[0].as_str()?;
                        let val = match c[1].as_str()? {
                            "b" => CardValue::Bool(c[2].as_bool()?),
                            "i" => CardValue::Int(c[2].as_i64()?),
                            "f" => CardValue::Float(c[2].as_f64()?),
                            "s" => CardValue::Str(c[2].as_str()?.to_string()),
                            _ => CardValue::None,
                        };
                        Some(Card::new(k, val, ""))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn s(v: &Value) -> String {
        v.as_str().unwrap_or("").to_string()
    }

    pub(crate) fn rel(a: f64, b: f64) -> f64 {
        if a == b { 0.0 } else { (a - b).abs() / a.abs().max(b.abs()).max(1e-300) }
    }

    #[test]
    fn sexagesimal_vs_astropy() {
        let Some(d) = load("pipeline_format.json") else { return };
        let rows = d["hmsdms"].as_array().unwrap();
        let mut n = 0;
        for r in rows {
            let (ra, dec) = (r[0].as_f64().unwrap(), r[1].as_f64().unwrap());
            assert_eq!(format_hmsdms(ra, dec, Some(1)), s(&r[2]), "{ra} {dec}");
            assert_eq!(format_hmsdms(ra, dec, Some(2)), s(&r[3]), "{ra} {dec}");
            assert_eq!(format_ra_hms(ra, Some(1), false), s(&r[4]), "{ra}");
            assert_eq!(format_dec_dms(dec, Some(0), false, true), s(&r[5]), "{dec}");
            n += 1;
        }
        eprintln!("六十進: {n} 組 × 4 通りが astropy と同じ文字列");
    }

    #[test]
    fn g_and_fixed_vs_python() {
        let Some(d) = load("pipeline_format.json") else { return };
        for r in d["g"].as_array().unwrap() {
            let x = r[0].as_f64().unwrap();
            assert_eq!(fmt_g(x), s(&r[1]), "{x:e}");
        }
        let mut n = 0;
        for r in d["fixed"].as_array().unwrap() {
            let x = r[0].as_f64().unwrap();
            for p in 0..7usize {
                assert_eq!(format!("{x:.p$}"), s(&r[1 + p]), "{x} .{p}f");
                n += 1;
            }
        }
        eprintln!("{{:g}} {} 個・{{:.Nf}} {n} 通りが Python と同じ", d["g"].as_array().unwrap().len());
    }

    #[test]
    fn coords_vs_skycoord() {
        let Some(d) = load("pipeline_format.json") else { return };
        let mut bad = Vec::new();
        let (mut ok_n, mut err_n) = (0, 0);
        for r in d["coords"].as_array().unwrap() {
            let (ra_s, dec_s) = (s(&r[0]), s(&r[1]));
            let py_ok = r[2].as_bool().unwrap();
            let got = parse_coords(ra_s.trim(), dec_s.trim());
            // 接頭辞つきの単位(kdeg など)は読まないと決めた(parse_coords の説明)
            if py_ok && got.is_err() && ra_s.contains("kdeg") {
                eprintln!("既知の違い: {ra_s:?} は Python なら読める");
                continue;
            }
            match (py_ok, &got) {
                (true, Ok((ra, dec))) => {
                    let (pra, pdec) = (r[3].as_f64().unwrap(), r[4].as_f64().unwrap());
                    if !(rel(*ra, pra) < 1e-15 && rel(*dec, pdec) < 1e-15) {
                        bad.push(format!("{ra_s:?} {dec_s:?}: Rust ({ra}, {dec}) / Python ({pra}, {pdec})"));
                    }
                    ok_n += 1;
                }
                (false, Err(_)) => err_n += 1,
                _ => bad.push(format!("{ra_s:?} {dec_s:?}: Rust {got:?} / Python {} {}", py_ok, s(&r[5]))),
            }
        }
        assert!(bad.is_empty(), "{} 件が食い違う:\n{}", bad.len(), bad.join("\n"));
        eprintln!("赤経赤緯の文字列: 読める {ok_n} 件・読めない {err_n} 件が SkyCoord と同じ");
    }

    pub(crate) fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zahyou_pipeline_test_{}_{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn scale_vs_python() {
        let Some(d) = load("pipeline_scale.json") else { return };
        let dir = temp_dir("scale");
        let cache = dir.join("cache.json");
        let body = serde_json::json!({"targets": {}, "rigs": d["rigs"]});
        std::fs::write(&cache, serde_json::to_string(&body).unwrap()).unwrap();
        let mut n = 0;
        for h in d["headers"].as_array().unwrap() {
            let name = s(&h["name"]);
            let cs = cards(&h["cards"]);
            let (ny, nx) = (h["shape"][0].as_u64().unwrap() as usize, h["shape"][1].as_u64().unwrap() as usize);
            assert_eq!(scale::rig_key(&cs, nx, ny), s(&h["rig_key"]), "{name}");
            for (key, focal) in
                [("est_None", None), ("est_818.0", Some(818.0)), ("est_0.0", Some(0.0)), ("est_400.5", Some(400.5))]
            {
                let (got, why) = scale::estimate_pixel_scale(&cs, focal, Some((nx, ny)), &cache);
                let want = h[key][0].as_f64();
                match (got, want) {
                    (Some(a), Some(b)) => assert!(rel(a, b) < 1e-15, "{name} {key}: {a} / {b}"),
                    (None, None) => {}
                    _ => panic!("{name} {key}: {got:?} / {want:?}"),
                }
                assert_eq!(why, s(&h[key][1]), "{name} {key}");
                n += 1;
            }
            let (got, why) = scale::estimate_pixel_scale(&cs, None, None, &cache);
            assert_eq!(got, h["est_noshape"][0].as_f64(), "{name}");
            assert_eq!(why, s(&h["est_noshape"][1]), "{name}");
            let imp = scale::implied_focal_length(&cs, 1.4616);
            match (imp, h["implied_1.4616"].as_f64()) {
                (Some(a), Some(b)) => assert!(rel(a, b) < 1e-15, "{name}"),
                (a, b) => assert_eq!(a, b, "{name}"),
            }
            assert_eq!(scale::implied_focal_length(&cs, 0.0), h["implied_0"].as_f64(), "{name}");
            assert_eq!(scale::existing_wcs(&cs).is_some(), h["existing"].as_bool().unwrap(), "{name}");
        }
        for r in d["index"].as_array().unwrap() {
            let names: Vec<String> = r["names"].as_array().unwrap().iter().map(s).collect();
            let cov = scale::index_coverage(&names);
            let want = (r["coverage"][0].as_f64(), r["coverage"][1].as_f64());
            assert_eq!(cov.map(|c| c.0), want.0, "{names:?}");
            assert_eq!(cov.map(|c| c.1), want.1, "{names:?}");
            for (i, w) in [968usize, 6000, 100].iter().enumerate() {
                let b = scale::blind_scale_bounds(*w, &names);
                assert_eq!(b.0, r["blind"][i][0].as_f64().unwrap(), "{names:?} {w}");
                assert_eq!(b.1, r["blind"][i][1].as_f64().unwrap(), "{names:?} {w}");
            }
            for a in r["advice"].as_array().unwrap() {
                let fov = a[0].as_f64().unwrap();
                let (ok, msgs) = scale::missing_index_advice(fov, &names);
                assert_eq!(ok, a[1].as_bool().unwrap(), "{fov}");
                let want: Vec<String> = a[2].as_array().unwrap().iter().map(s).collect();
                assert_eq!(msgs, want, "{fov} {names:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("画素スケールの推定 {n} 通り・機材の鍵・逆算した焦点距離・段の案内が Python と同じ");
    }

    #[test]
    fn summary_vs_python() {
        let Some(d) = load("pipeline_summary.json") else { return };
        let (mut worst_pos, mut worst_pa, mut worst_pix, mut worst_off) = (0f64, 0f64, 0f64, 0f64);
        let mut n = 0;
        for c in d["cases"].as_array().unwrap() {
            let name = s(&c["name"]);
            let cs = cards(&c["cards"]);
            let wcs = Wcs::from_header_cards(&cs).unwrap_or_else(|e| panic!("{name}: {e}"));
            let (ny, nx) = (c["shape"][0].as_u64().unwrap() as usize, c["shape"][1].as_u64().unwrap() as usize);
            for (key, flip) in [("None", None), ("True", Some(true)), ("False", Some(false))] {
                let p = &c["summaries"][key];
                let sm = wcs_summary(&wcs, nx, ny, flip);
                let dpos = (sm.center_ra - p["ra"].as_f64().unwrap())
                    .abs()
                    .max((sm.center_dec - p["dec"].as_f64().unwrap()).abs());
                worst_pos = worst_pos.max(dpos);
                let mut dpa = (sm.pa_screen_up - p["pa"].as_f64().unwrap()).abs();
                dpa = dpa.min(360.0 - dpa);
                worst_pa = worst_pa.max(dpa);
                assert!(dpos < 1e-9, "{name} {key}: 中心 {dpos}");
                assert!(dpa < 1e-7, "{name} {key}: 位置角 {dpa}");
                assert!(rel(sm.scale, p["scale"].as_f64().unwrap()) < 1e-14, "{name}: {} / {}", sm.scale, p["scale"]);
                assert!(rel(sm.fov_x, p["fov_x"].as_f64().unwrap()) < 1e-14, "{name}");
                assert!(rel(sm.fov_y, p["fov_y"].as_f64().unwrap()) < 1e-14, "{name}");
                assert_eq!(sm.parity_text(), s(&p["parity"]), "{name} {key}");
                let want: Vec<String> = p["lines"].as_array().unwrap().iter().map(s).collect();
                assert_eq!(solution_lines(&sm), want, "{name} {key}");
                assert_eq!(format_hmsdms(sm.center_ra, sm.center_dec, Some(1)), s(&p["hmsdms1"]), "{name}");
                assert_eq!(format_ra_hms(sm.center_ra, Some(1), false), s(&p["ra_hms1"]), "{name}");
                assert_eq!(format_dec_dms(sm.center_dec, Some(0), false, true), s(&p["dec_dms0"]), "{name}");
                n += 1;
            }
            assert_eq!(sanity_ok(&wcs, nx, ny), c["sanity"].as_bool().unwrap(), "{name}");
            let sm = wcs_summary(&wcs, nx, ny, None);
            for o in c["offsets"].as_array().unwrap() {
                let t = (o["target"][0].as_f64().unwrap(), o["target"][1].as_f64().unwrap());
                let got = offsets((sm.center_ra, sm.center_dec), t);
                // 真裏(角距離 180°)では赤経方向が ±180° のどちらにもなる(atan2(±0, −1))。向きは問わない
                let antipode = (got.sep_arcmin - 10800.0).abs() < 1e-6;
                for (a, b) in [(got.sep_arcmin, &o["sep"]), (got.ra_arcmin, &o["ra"]), (got.dec_arcmin, &o["dec"])] {
                    let b = b.as_f64().unwrap();
                    let d = if antipode { (a.abs() - b.abs()).abs() } else { (a - b).abs() };
                    worst_off = worst_off.max(d);
                    assert!(d < 1e-8, "{name} {t:?}: {a} / {b}");
                }
                if antipode {
                    continue;
                }
                // ずれがほぼ 0(目標 = 画像中心)なら東西・南北はどちらにもなる(1e-12′ の差で符号が変わる)
                if got.ra_arcmin.abs() > 1e-9 {
                    assert_eq!(got.ra_dir, s(&o["ra_dir"]), "{name} {t:?}");
                }
                if got.dec_arcmin.abs() > 1e-9 {
                    assert_eq!(got.dec_dir, s(&o["dec_dir"]), "{name} {t:?}");
                }
                if got.ra_arcmin.abs() > 1e-9 && got.dec_arcmin.abs() > 1e-9 {
                    let want: Vec<String> = o["lines"].as_array().unwrap().iter().map(s).collect();
                    assert_eq!(offsets_lines(&got), want, "{name} {t:?}");
                    assert_eq!(north_up_title(&got), format!("北が上・東が左\n{}", s(&o["title2"])));
                }
                // 目標の画素(画面の内外の判定に使う)。画像の 3 倍の範囲の外は比べない
                // (接点から何十度も離れた点は TAN 投影で差が拡大するだけで、どのみち「画面の外」)
                let (px, py) = (o["pix"][0].as_f64(), o["pix"][1].as_f64());
                if let (Some(px), Some(py)) = (px, py) {
                    let (w, h) = (nx as f64, ny as f64);
                    if (-2.0 * w..3.0 * w).contains(&px) && (-2.0 * h..3.0 * h).contains(&py) {
                        let (x, y) =
                            wcs.radec_to_pix_exact(t.0, t.1).unwrap_or_else(|| panic!("{name} {t:?}: 画素にならない"));
                        let dp = (x - px).abs().max((y - py).abs());
                        worst_pix = worst_pix.max(dp);
                        assert!(dp < 1e-7, "{name} {t:?}: 画素 ({x}, {y}) / ({px}, {py})");
                        // 画面の内外の判定も同じ
                        let inside = |x: f64, y: f64| 0.0 <= x && x < w && 0.0 <= y && y < h;
                        assert_eq!(inside(x, y), inside(px, py), "{name} {t:?}");
                    }
                }
            }
        }
        eprintln!(
            "まとめ {n} 通り: 中心の差 最大 {worst_pos:.2e}°、位置角 {worst_pa:.2e}°、ずれ {worst_off:.2e}′、目標の画素 {worst_pix:.2e} px"
        );
    }

    /// Python のログから、解法が出す行(pipeline_run.json は解法を「解なし」に差し替えて作った)と時刻の入る行をそろえる。
    pub(crate) fn normalize_log(lines: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for l in lines {
            let t = l.trim_start_matches('\n');
            if (t.starts_with("  [") && t.contains("] ") && t.contains("(最大 "))
                || t.starts_with("  - 深い検出")
                || t.starts_with("  ⏱")
                || t.starts_with("    · ")
                || t.starts_with("    索引 ")
                || t.starts_with("  ✅ 解析成功 (")
            {
                continue;
            }
            // zahyou-net はノートブック用の言い回し「INPUT_MODE='COORDS' で」を外した(net の係の報告)
            if l.starts_with("  ❌ オフラインでは '") {
                out.push(l.replace("。 INPUT_MODE='COORDS' で", "。"));
                continue;
            }
            // 赤経赤緯を読めなかった理由は astropy の英語の文なので、文の形だけ比べる
            if l.starts_with("❌ 赤経赤緯を読み取れません (") {
                out.push("❌ 赤経赤緯を読み取れません (…)。書式を確認してください。".to_string());
                continue;
            }
            if let Some(i) = l.find("所要時間      : ") {
                out.push(format!("{}所要時間      : X 秒", &l[..i]));
                continue;
            }
            out.push(l.clone());
        }
        out
    }

    /// FITS の最初のヘッダーの END の前にカードを足す(gen_pipeline.py の insert_cards と同じ)。
    pub(crate) fn insert_cards(bytes: &[u8], cards: &[String]) -> Option<Vec<u8>> {
        let mut pos = 0;
        while bytes.get(pos..pos + 8)? != b"END     " {
            pos += 80;
        }
        let mut head = bytes[..pos].to_vec();
        for c in cards {
            head.extend_from_slice(c.as_bytes());
        }
        head.extend_from_slice(format!("{:<80}", "END").as_bytes());
        let end_old = (pos / 2880 + 1) * 2880;
        while !head.len().is_multiple_of(2880) {
            head.push(b' ');
        }
        head.extend_from_slice(bytes.get(end_old..)?);
        Some(head)
    }

    /// pipeline_run.json の試験画像をそろえる(手元の画像の sha256 が同じときだけ)。(WCS を足した版, そのままの版, 作業フォルダー)
    pub(crate) fn run_images(d: &Value, tag: &str) -> Option<(PathBuf, PathBuf, PathBuf)> {
        if d.get("skipped").is_some() {
            eprintln!("SKIP: 基準値を作ったときに試験画像が無かった");
            return None;
        }
        let img = PathBuf::from(s(&d["image"]));
        let Ok(bytes) = std::fs::read(&img) else {
            eprintln!("SKIP: 試験画像がありません: {}", img.display());
            return None;
        };
        if sha2_like::sha256_hex(&bytes) != s(&d["image_sha256"]) {
            eprintln!("SKIP: 試験画像の中身が基準値を作ったときと違います");
            return None;
        }
        let cards: Vec<String> = d["wcs_cards"].as_array()?.iter().map(s).collect();
        let dir = temp_dir(tag);
        let with_wcs = dir.join("zahyou_wcs_test.fits");
        let plain = dir.join("zahyou_plain_test.fits");
        std::fs::write(&with_wcs, insert_cards(&bytes, &cards)?).ok()?;
        std::fs::write(&plain, &bytes).ok()?;
        Some((with_wcs, plain, dir))
    }

    pub(crate) fn params_from(p: &Value, image: PathBuf) -> RunParams {
        RunParams {
            mode: match p["INPUT_MODE"].as_str() {
                Some("COORDS") => InputMode::Coords,
                Some("NONE") => InputMode::None,
                _ => InputMode::StarName,
            },
            name: s(&p["TARGET_STAR_NAME"]),
            ra_str: s(&p["RA_INPUT_STR"]),
            dec_str: s(&p["DEC_INPUT_STR"]),
            focal_mm: p["FOCAL_LENGTH_MM"].as_f64(),
            solve_mode: match p["SOLVE_MODE"].as_str() {
                Some("ONLINE") => SolveMode::Online,
                Some("OFFLINE") => SolveMode::Offline,
                _ => SolveMode::Auto,
            },
            online_timeout_s: p["ONLINE_TIMEOUT"].as_f64().unwrap_or(120.0),
            offline_timeout_s: p["OFFLINE_TIMEOUT"].as_f64().unwrap_or(300.0),
            use_target_as_hint: p["USE_TARGET_AS_HINT"].as_bool().unwrap_or(true),
            search_radius_deg: p["SEARCH_RADIUS_DEG"].as_f64().unwrap_or(5.0),
            ignore_existing_wcs: p["IGNORE_EXISTING_WCS"].as_bool().unwrap_or(true),
            show_sources: p["SHOW_DETECTED_SOURCES"].as_bool().unwrap_or(true),
            image_path: image,
            index_dir: PathBuf::from(s(&p["INDEX_DIR"])),
            api_key: None,
        }
    }

    /// 解法の代わり(いつも「解なし」。Python の基準値も sv_solve を「解なし」に差し替えて作った)。
    pub(crate) fn no_solution(_: &OfflineRequest, _: &Catalog, _: &Params, _: &Control) -> Result<OfflineOutcome, SolveError> {
        Ok(tried(None))
    }

    /// 試験用の cache.json(目標 1 件、機材の記憶は with_rig のときだけ)。
    pub(crate) fn write_cache(path: &Path, with_rig: bool) {
        let mut c = serde_json::json!({"targets": {DEFAULT_TARGET: {"ra": 61.91199157844, "dec": 41.98626242835, "name": DEFAULT_TARGET}}, "rigs": {}});
        if with_rig {
            c["rigs"]["ZWO ASI290MM||968x548|bin2|f"] = serde_json::json!(1.461614244726863);
        }
        if let Some(o) = c.as_object() {
            let _ = std::fs::write(path, zahyou_net::cache::cache_to_string(o));
        }
    }

    #[test]
    fn run_log_vs_python() {
        let Some(d) = load("pipeline_run.json") else { return };
        let Some((with_wcs, plain, dir)) = run_images(&d, "run") else { return };
        let mut n_lines = 0;
        let mut n_runs = 0;
        for r in d["runs"].as_array().unwrap() {
            let name = s(&r["name"]);
            let image = match s(&r["which"]).as_str() {
                "wcs" => with_wcs.clone(),
                "plain" => plain.clone(),
                // 手元のほかの実写(dev/realcase の real_flip.fits・real.png)。無いか中身が違えば SKIP
                _ => {
                    let p = PathBuf::from(s(&r["image_path"]));
                    match std::fs::read(&p) {
                        Ok(b) if sha2_like::sha256_hex(&b) == s(&r["image_path_sha256"]) => p,
                        _ => {
                            eprintln!("SKIP {name}: {} が無いか、基準値を作ったときと中身が違う", p.display());
                            continue;
                        }
                    }
                }
            };
            let params = params_from(&r["params"], image);
            if s(&r["which"]) != "wcs" && !params.index_dir.is_dir() && name != "off_noindex" {
                eprintln!("SKIP {name}: 星図データがありません");
                continue;
            }
            let cache = dir.join(format!("cache_{name}.json"));
            write_cache(&cache, r["with_rig"].as_bool().unwrap());
            // 解法の代わり: 正解 WCS の場面ならそれを返す、ほかは「解なし」(Python の基準値も同じ差し替え)
            let oracle = r.get("oracle_cards").and_then(Value::as_array).map(|a| {
                let cards: Vec<String> = a.iter().map(s).collect();
                oracle_solver(Wcs::from_header_cards(&cards_from_strings(&cards)).expect("正解 WCS"))
            });
            let solver: &OfflineSolverFn = match &oracle {
                Some(f) => f,
                None => &no_solution,
            };
            let env = RunEnv {
                cache_file: Some(cache.clone()),
                offline_solver: Some(solver),
                online: Some(false),
                skip_figures: false,
            };
            let lines = StdMutex::new(Vec::<String>::new());
            let log = |m: &str| lines.lock().unwrap().push(m.to_string());
            let res = run_with(&params, &env, &AtomicBool::new(false), &log);
            let got = normalize_log(&lines.into_inner().unwrap());
            let want = normalize_log(&r["log"].as_array().unwrap().iter().map(s).collect::<Vec<_>>());
            if got != want {
                let mut diff = String::new();
                for i in 0..got.len().max(want.len()) {
                    let (g, w) = (
                        got.get(i).map(String::as_str).unwrap_or("<無>"),
                        want.get(i).map(String::as_str).unwrap_or("<無>"),
                    );
                    if g != w {
                        diff.push_str(&format!("  {i}: Rust {g:?}\n     Python {w:?}\n"));
                    }
                }
                panic!("{name}: ログが違う\n{diff}");
            }
            n_lines += got.len();
            n_runs += 1;
            assert_eq!(res.wcs.is_some(), r["solved"].as_bool().unwrap(), "{name}");
            assert_eq!(res.figures.len(), r["figures"].as_u64().unwrap() as usize, "{name}: 図の枚数");
            if let Some(ps) = r.get("summary") {
                let sm = res.summary.as_ref().unwrap();
                assert!((sm.center_ra - ps["ra"].as_f64().unwrap()).abs() < 1e-9, "{name}");
                assert!((sm.center_dec - ps["dec"].as_f64().unwrap()).abs() < 1e-9, "{name}");
                assert!(rel(sm.scale, ps["scale"].as_f64().unwrap()) < 1e-12, "{name}");
                assert!((sm.pa_screen_up - ps["pa"].as_f64().unwrap()).abs() < 1e-7, "{name}");
                assert_eq!(sm.parity_text(), s(&ps["parity"]), "{name}");
            } else {
                assert!(res.summary.is_none(), "{name}");
            }
            match (r.get("offsets"), &res.offsets) {
                (Some(po), Some(o)) => {
                    assert!((o.sep_arcmin - po[0].as_f64().unwrap()).abs() < 1e-8, "{name}");
                    assert!((o.ra_arcmin - po[1].as_f64().unwrap()).abs() < 1e-8, "{name}");
                    assert!((o.dec_arcmin - po[2].as_f64().unwrap()).abs() < 1e-8, "{name}");
                    assert_eq!(o.ra_dir, s(&po[3]));
                    assert_eq!(o.dec_dir, s(&po[4]));
                }
                (None, None) => {}
                (a, b) => panic!("{name}: ずれ {a:?} / {b:?}"),
            }
            // 記憶した機材の画素スケール
            let after = zahyou_net::cache::load_cache_at(&cache);
            let want_rigs = &r["cache_after"]["rigs"];
            let got_rigs = after.get("rigs").cloned().unwrap_or(Value::Null);
            for (k, v) in want_rigs.as_object().unwrap() {
                let g = got_rigs.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
                assert!(rel(g, v.as_f64().unwrap()) < 1e-12, "{name}: 記憶 {k} {g} / {v}");
            }
            assert_eq!(got_rigs.as_object().map(|o| o.len()), want_rigs.as_object().map(|o| o.len()), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("run() のログ {n_runs} 通り・{n_lines} 行が Python と同じ(解法のはしごの行と所要時間を除く)");
    }

    /// WSL 版(astrometry.net)の正解 WCS(pipeline_run.json の wcs_cards = oracle/real_orig/solution.wcs)。
    pub(crate) fn oracle_wcs(d: &Value) -> Option<Wcs> {
        let cards: Vec<String> = d["wcs_cards"].as_array()?.iter().map(s).collect();
        Wcs::from_header_cards(&cards_from_strings(&cards)).ok()
    }

    /// 解法の代わりに正解 WCS を返すもの(解法と切り離して、オフラインの経路を通しで確かめる)。
    pub(crate) fn oracle_solver(
        wcs: Wcs,
    ) -> impl Fn(&OfflineRequest, &Catalog, &Params, &Control) -> Result<OfflineOutcome, SolveError> + Sync {
        move |req: &OfflineRequest, _: &Catalog, _: &Params, ctl: &Control| {
            (ctl.log)("  ✅ 解析成功 (正解 WCS で代用)");
            Ok(tried(Some(zahyou_solver::Solution {
                wcs: wcs.clone(),
                logodds: 87.77,
                nmatch: 13,
                ndistract: 0,
                nfield: req.sources.len(),
                nref: 0,
                index_name: "index-5205-02 (WSL の解)".to_string(),
                quads_tried: 0,
                hypotheses: 0,
                seconds: 0.0,
            })))
        }
    }

    /// 依頼の基準値(Python 版の --selftest と同じ画像・同じ目標): 1.583′・0.918′ 東・1.290′ 北・1.4616″/px・
    /// 視野 23.58′ × 13.35′・中心 04h07m33.9s +41d57m53s。解法の代わりに WSL 版の正解 WCS を読ませる。
    #[test]
    fn oracle_offline_path_gives_selftest_numbers() {
        let Some(d) = load("pipeline_run.json") else { return };
        let Some((_, plain, dir)) = run_images(&d, "oracle") else { return };
        let index_dir = PathBuf::from(DEFAULT_INDEX_DIR);
        if !index_dir.is_dir() {
            eprintln!("SKIP: 星図データがありません({DEFAULT_INDEX_DIR})");
            return;
        }
        let wcs = oracle_wcs(&d).expect("正解 WCS を読めない");
        let solver = oracle_solver(wcs);
        let cache = dir.join("cache_oracle.json");
        write_cache(&cache, false);
        let env = RunEnv {
            cache_file: Some(cache.clone()),
            offline_solver: Some(&solver),
            online: Some(false),
            skip_figures: false,
        };
        let params = RunParams { solve_mode: SolveMode::Offline, image_path: plain.clone(), ..Default::default() };
        let lines = StdMutex::new(String::new());
        let log = |m: &str| {
            let mut g = lines.lock().unwrap();
            g.push_str(m);
            g.push('\n');
        };
        let res = run_with(&params, &env, &AtomicBool::new(false), &log);
        let text = lines.into_inner().unwrap();
        assert!(res.wcs.is_some(), "{text}");
        let nums = res.numbers().unwrap();
        assert_eq!(nums[0], ("画像中心から目標までの距離".to_string(), "1.583′".to_string()));
        assert_eq!(nums[1], ("赤経 (RA) 方向".to_string(), "0.918′ 東".to_string()));
        assert_eq!(nums[2], ("赤緯 (Dec) 方向".to_string(), "1.290′ 北".to_string()));
        let detail = res.detail().unwrap();
        assert!(
            detail.starts_with(
                "画像中心 04h07m33.9s +41d57m53.1s   画素スケール 1.4616″/px   視野 23.58′ × 13.35′   所要 "
            ),
            "{detail}"
        );
        assert_eq!(res.figures.len(), 2);
        assert!(text.contains("  解析結果\n"), "{text}");
        assert!(text.contains("  解いた方法    : オフライン (zahyou 内蔵の解析エンジン)"), "{text}");
        assert!(text.contains("  画面上方向    : 位置角 332.87°(北から東回り)"), "{text}");
        assert!(text.contains("  パリティ      : 正 (鏡像なし)"), "{text}");
        assert!(text.contains("  → この機材の画素スケールを記憶しました"), "{text}");
        assert!(res.status_text().starts_with("解析できました ("));
        // 同じ正解 WCS を画像に入れて Python の run() が出したずれ(pipeline_run.json の wcs_star)と同じ
        let o = res.offsets.as_ref().unwrap();
        let py = d["runs"].as_array().unwrap().iter().find(|r| r["name"] == "wcs_star").unwrap();
        for (a, b) in
            [(o.sep_arcmin, &py["offsets"][0]), (o.ra_arcmin, &py["offsets"][1]), (o.dec_arcmin, &py["offsets"][2])]
        {
            assert!((a - b.as_f64().unwrap()).abs() < 1e-9, "{a} / {b}");
        }
        // 解けた画素スケールを覚えた(次は「前回この機材で解いたときの実測値」になる)
        let after = zahyou_net::cache::load_cache_at(&cache);
        let rig = after["rigs"]["ZWO ASI290MM||968x548|bin2|f"].as_f64().unwrap();
        assert!((rig - 1.4616).abs() < 1e-4, "{rig}");

        // 目標なし: 画像中心と画素スケール、図は 1 枚
        let params2 = RunParams { mode: InputMode::None, ..params };
        let res2 = run_with(&params2, &env, &AtomicBool::new(false), &|_: &str| {});
        let n2 = res2.numbers().unwrap();
        assert_eq!(n2[0], ("画像中心 (赤経)".to_string(), "4h07m33.9s".to_string()));
        assert_eq!(n2[1], ("画像中心 (赤緯)".to_string(), "+41d57m53s".to_string()));
        assert_eq!(n2[2], ("画素スケール".to_string(), "1.462″/px".to_string()));
        assert_eq!(res2.figures.len(), 1);
        assert!(res2.offsets.is_none());
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("正解 WCS を通したオフラインの経路: {} / {} / {} / {detail}", nums[0].1, nums[1].1, nums[2].1);
    }

    /// 自己診断(画面の項目なし)を正解 WCS で通す。
    #[test]
    fn selftest_rows_with_oracle() {
        let Some(d) = load("pipeline_run.json") else { return };
        let Some((_, plain, dir)) = run_images(&d, "selftest") else { return };
        if !Path::new(DEFAULT_INDEX_DIR).is_dir() {
            eprintln!("SKIP: 星図データがありません");
            return;
        }
        let solver = oracle_solver(oracle_wcs(&d).unwrap());
        let cache = dir.join("cache_selftest.json");
        write_cache(&cache, false);
        let env =
            RunEnv { cache_file: Some(cache), offline_solver: Some(&solver), online: Some(false), skip_figures: false };
        let mut log = String::new();
        let rows = crate::selftest::selftest_rows(&plain, None, None, &env, &mut log);
        let text = crate::selftest::format_report(&rows, &log);
        assert!(crate::selftest::all_passed(&rows), "{text}");
        assert!(text.contains("  PASS  距離が出る"), "{text}");
        assert!(text.contains("1.583′"), "{text}");
        assert!(log.contains("目標は指定されていません"), "最後の解析(目標なし)のログ: {log}");
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("{}", text.lines().take(rows.len() + 2).collect::<Vec<_>>().join("\n"));
    }

    /// 中止: 始める前に立っていれば、画像を読んだところで止まる。解法の途中なら解法の中止がそのまま返る。
    #[test]
    fn cancel_paths() {
        let Some(d) = load("pipeline_run.json") else { return };
        let Some((_, plain, dir)) = run_images(&d, "cancel") else { return };
        let cache = dir.join("cache_cancel.json");
        write_cache(&cache, false);
        let params = RunParams { solve_mode: SolveMode::Offline, image_path: plain.clone(), ..Default::default() };
        let env = RunEnv { cache_file: Some(cache.clone()), online: Some(false), ..Default::default() };
        let cancel = AtomicBool::new(true);
        let lines = StdMutex::new(Vec::<String>::new());
        let log = |m: &str| lines.lock().unwrap().push(m.to_string());
        let res = run_with(&params, &env, &cancel, &log);
        assert!(res.cancelled && res.wcs.is_none());
        let l = lines.into_inner().unwrap();
        assert_eq!(l.last().map(String::as_str), Some("\n■ 中止しました。"), "{l:?}");
        assert!(!l.iter().any(|x| x.contains("星を検出しています")), "{l:?}");

        if Path::new(DEFAULT_INDEX_DIR).is_dir() {
            let cancel2 = AtomicBool::new(false);
            let solver = |_: &OfflineRequest, _: &Catalog, _: &Params, ctl: &Control| {
                ctl.cancel.store(true, Ordering::Relaxed);
                Err(SolveError::Cancelled)
            };
            let env2 = RunEnv {
                cache_file: Some(cache),
                offline_solver: Some(&solver),
                online: Some(false),
                ..Default::default()
            };
            let lines = StdMutex::new(Vec::<String>::new());
            let log = |m: &str| lines.lock().unwrap().push(m.to_string());
            let res = run_with(&params, &env2, &cancel2, &log);
            assert!(res.cancelled && res.wcs.is_none() && res.summary.is_none());
            let l = lines.into_inner().unwrap();
            assert_eq!(l.last().map(String::as_str), Some("\n  ■ 中止しました。"), "{l:?}");
            assert!(!l.iter().any(|x| x.contains("座標を特定できませんでした")), "{l:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 壊れた入力で panic せず、理由を返す(画像が無い・空のパス・画像でないファイル・解法の panic)。
    #[test]
    fn broken_inputs_do_not_panic() {
        let dir = temp_dir("broken");
        let quiet = |_: &str| {};
        let env = RunEnv { cache_file: Some(dir.join("c.json")), online: Some(false), ..Default::default() };
        let cases: Vec<(PathBuf, &str)> = vec![
            (PathBuf::new(), "画像が選ばれていません"),
            (dir.join("no_such.fits"), "画像が見つかりません"),
            (dir.join("garbage.fits"), "画像として読み込めませんでした"),
            (dir.join("tiny.png"), "画像として読み込めませんでした"),
        ];
        std::fs::write(dir.join("garbage.fits"), b"SIMPLE  = garbage").unwrap();
        std::fs::write(dir.join("tiny.png"), [0x89u8, b'P', b'N', b'G']).unwrap();
        for (p, want) in &cases {
            let params = RunParams { image_path: p.clone(), solve_mode: SolveMode::Offline, ..Default::default() };
            let res = run_with(&params, &env, &AtomicBool::new(false), &quiet);
            let e = res.error.clone().unwrap_or_default();
            assert!(e.contains(want), "{p:?}: {e}");
            assert!(res.wcs.is_none() && res.numbers().is_none() && res.detail().is_none());
            assert_eq!(res.status_text(), "解けませんでした。ログを見てください。");
        }
        // 解法が panic しても、結果の error に入れて返す
        // (標準エラーに「試験のための panic」と出るのは panic の既定の表示。試験は通っている)
        if let Some(d) = load("pipeline_run.json")
            && let Some((_, plain, dir2)) = run_images(&d, "panic")
        {
            if Path::new(DEFAULT_INDEX_DIR).is_dir() {
                let boom =
                    |_: &OfflineRequest,
                     _: &Catalog,
                     _: &Params,
                     _: &Control|
                     -> Result<OfflineOutcome, SolveError> { panic!("試験のための panic") };
                let env2 = RunEnv {
                    cache_file: Some(dir2.join("c.json")),
                    offline_solver: Some(&boom),
                    online: Some(false),
                    ..Default::default()
                };
                let params = RunParams { image_path: plain, solve_mode: SolveMode::Offline, ..Default::default() };
                let res = run_with(&params, &env2, &AtomicBool::new(false), &quiet);
                assert!(res.error.as_deref().unwrap_or("").contains("試験のための panic"), "{:?}", res.error);
            }
            let _ = std::fs::remove_dir_all(&dir2);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// オンラインの流れ(通信はしない): API キーが無ければ、オンラインのみなら「オンライン解析でエラー」で
    /// そこで終わり、自動なら「エラー」とは書かずに問い合わせを飛ばしてオフラインへ切り替える。
    #[test]
    fn online_without_key() {
        if zahyou_net::api_key_from_env().is_some() {
            eprintln!("SKIP: 環境変数 ASTROMETRY_API_KEY があるので、本物の nova に送ってしまう");
            return;
        }
        let Some(d) = load("pipeline_run.json") else { return };
        let Some((with_wcs, plain, dir)) = run_images(&d, "online") else { return };
        let cache = dir.join("cache_online.json");
        write_cache(&cache, false);
        let solver: &OfflineSolverFn = &no_solution;
        let env =
            RunEnv { cache_file: Some(cache), offline_solver: Some(solver), online: Some(true), skip_figures: true };
        for (mode, image) in [(SolveMode::Online, &plain), (SolveMode::Auto, &plain), (SolveMode::Auto, &with_wcs)] {
            // 目標なし(天体名をオンラインで引きにいかない = 通信しない)
            let params = RunParams {
                mode: InputMode::None,
                solve_mode: mode,
                image_path: image.clone(),
                ignore_existing_wcs: image == &plain,
                api_key: Some(" ".into()),
                ..Default::default()
            };
            let lines = StdMutex::new(Vec::<String>::new());
            let log = |m: &str| lines.lock().unwrap().push(m.to_string());
            let res = run_with(&params, &env, &AtomicBool::new(false), &log);
            let l = lines.into_inner().unwrap();
            let has = |s: &str| l.iter().any(|x| x.contains(s));
            assert!(has("🌍 オンライン環境で実行します。"), "{l:?}");
            if image == &with_wcs {
                // 画像の WCS があればオンラインへは行かない
                assert!(!has("[オンライン]") && res.wcs.is_some(), "{l:?}");
                continue;
            }
            match mode {
                SolveMode::Online => {
                    assert!(has("[オンライン] nova.astrometry.net に問い合わせます (最大 120 秒)"), "{l:?}");
                    assert!(has("⚠️ オンライン解析でエラー: API キーがありません"), "{l:?}");
                    assert!(has("SOLVE_MODE='ONLINE' なのでここで終了します。") && !has("[オフライン]"), "{l:?}");
                }
                _ => {
                    assert!(has("[オンライン] nova.astrometry.net の API キーが無いので、問い合わせません"), "{l:?}");
                    assert!(!has("問い合わせます (最大") && !has("⚠️ オンライン解析でエラー"), "{l:?}");
                    assert!(has("   → オフラインの解析に切り替えます。") && has("[オフライン]"), "{l:?}");
                }
            }
            assert!(res.wcs.is_none() && has("❌ 座標を特定できませんでした。"), "{l:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------ 本物の解法つきの run() ---

    /// app/testdata の場所。
    fn app_testdata() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
    }

    /// run_solver.json の画像のパス("$ZAHYOU_TEST_IMAGES/…" か、リポジトリからの相対)を実際のパスに。
    /// リポジトリに無いもの(dev/tcase・dev/realcase は .gitignore)は ZAHYOU_PY_REPO(既定は Python 版の
    /// リポジトリ)の同じ相対パスも見る(rs/testdata/gen_solver.py と同じ決まり)。
    pub(crate) fn resolve_image(p: &str) -> PathBuf {
        if let Some(rest) = p.strip_prefix("$ZAHYOU_TEST_IMAGES/") {
            let base = std::env::var_os("ZAHYOU_TEST_IMAGES")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_default();
            return rest.split('/').fold(base, |b, s| b.join(s));
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let here = p.split('/').fold(repo, |b, s| b.join(s));
        if here.exists() {
            return here;
        }
        let py_repo = std::env::var_os("ZAHYOU_PY_REPO")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Users\yoshi\Documents\GitHub\zahyou"));
        p.split('/').fold(py_repo, |b, s| b.join(s))
    }

    /// 時刻で変わる数を伏せる: 「(最大 N 秒)」の N と、「1.23 秒」のような小数の秒。
    pub(crate) fn mask_seconds(line: &str) -> String {
        let mut s = line.to_string();
        if let Some(i) = s.find("(最大 ")
            && let Some(j) = s[i..].find(" 秒)")
        {
            s.replace_range(i + "(最大 ".len()..i + j, "X");
        }
        // 「<数字>.<数字> 秒」を後ろから探して X にする
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(k) = rest.find(" 秒") {
            let head = &rest[..k];
            let start = head
                .char_indices()
                .rev()
                .find(|&(_, c)| !(c.is_ascii_digit() || c == '.'))
                .map_or(0, |(i, c)| i + c.len_utf8());
            let num = &head[start..];
            if num.contains('.') && num.chars().any(|c| c.is_ascii_digit()) {
                out.push_str(&head[..start]);
                out.push('X');
            } else {
                out.push_str(head);
            }
            out.push_str(" 秒");
            rest = &rest[k + " 秒".len()..];
        }
        out.push_str(rest);
        out
    }

    /// 試行のはしごの行か(時刻しだいで変わるので、"advice" の場面では比べない)。
    fn ladder_line(l: &str) -> bool {
        let t = l.trim_start_matches('\n');
        (t.starts_with("  [") && t.contains("(最大 "))
            || t.starts_with("    · ")
            || t.starts_with("  ⏱")
            || t.starts_with("    索引 ")
            || t.starts_with("  ✅ 解析成功 (")
    }

    pub(crate) fn normalize_solver_log(lines: &[String], advice_only: bool) -> Vec<String> {
        lines
            .iter()
            .filter(|l| !(advice_only && ladder_line(l)))
            .map(|l| match l.find("所要時間      : ") {
                Some(i) => format!("{}所要時間      : X 秒", &l[..i]),
                None => mask_seconds(l),
            })
            .collect()
    }

    #[test]
    fn mask_seconds_examples() {
        assert_eq!(mask_seconds("\n  [1/4] 深い検出の星 15 個 + 全天  (最大 150 秒)"), "\n  [1/4] 深い検出の星 15 個 + 全天  (最大 X 秒)");
        assert_eq!(
            mask_seconds("    · 解けました: index-5205-02、対数オッズ 87.8、一致 13 個、quad 1,234 個・仮説 56 個、0.12 秒"),
            "    · 解けました: index-5205-02、対数オッズ 87.8、一致 13 個、quad 1,234 個・仮説 56 個、X 秒"
        );
        assert_eq!(mask_seconds("    索引 a / 一致 3 個 / 対数オッズ 50.0 / quad 7 個 / 0.0 秒"), "    索引 a / 一致 3 個 / 対数オッズ 50.0 / quad 7 個 / X 秒");
        assert_eq!(mask_seconds("最大 300 秒の 2 秒"), "最大 300 秒の 2 秒");
    }

    /// 本物の解法を通した run() のログ・結果を、Python の run()(sv_solve も本物)と比べる
    /// (app/testdata/run_solver.json。gen_run_solver.py が作る)。画像・星図データが手元に無い場面は SKIP。
    /// 時刻で変わる数(所要時間・「(最大 N 秒)」・小数の秒)だけを伏せる。締め切りで打ち切る場面("advice")は
    /// 試行のはしごの行を除いて、深い検出・事前のログ・解けなかったときの案内を比べる。
    /// 既定では、締め切りまで探す 2 場面(焦点距離の書き違い・ピンぼけ。Rust でも 20〜30 秒)を飛ばす。
    /// 全部は `cargo test -p zahyou run_solver_vs_python_all -- --ignored --nocapture`。
    #[test]
    fn run_solver_vs_python() {
        run_solver_cases(false);
    }

    #[test]
    #[ignore]
    fn run_solver_vs_python_all() {
        run_solver_cases(true);
    }

    /// 同じ目録(index_catalog が覚えているもの)をいくつかの解析が同時に使っても止まらない。
    /// 以前は、索引を開くときの木の検査(rayon で並列)を錠を持ったまま走らせていたので、rayon の作業スレッドが
    /// 待つ間に横取りした仕事が同じ索引を開こうとして止まった(zahyou-index の IndexFile::data と解法の Store::data)。
    /// 解析は 1 回ごとに索引の mmap を放すので、毎回開き直す。止まったら 180 秒で失敗にする(スレッドは残るが試験は終わる)。
    #[test]
    fn concurrent_runs_do_not_deadlock() {
        const ROUNDS: usize = 5;
        // 同じ画像を同時に解く(同じ索引を同時に開くときに止まっていた)。ほかの画像も混ぜる
        let images = [
            "$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_10_33.fits",
            "$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_10_33.fits",
            "$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_10_33.fits",
            "$ZAHYOU_TEST_IMAGES/Downloads/C2025A6/C2025A6_00001 04_23_42.fits",
        ];
        let paths: Vec<PathBuf> = images.iter().map(|p| resolve_image(p)).filter(|p| p.is_file()).collect();
        if paths.len() < 3 || !Path::new(DEFAULT_INDEX_DIR).is_dir() {
            eprintln!("SKIP: 画像か星図データが無い");
            return;
        }
        let dir = temp_dir("concurrent");
        let (tx, rx) = std::sync::mpsc::channel();
        for (k, img) in paths.iter().enumerate() {
            let (tx, img, cache) = (tx.clone(), img.clone(), dir.join(format!("cache_{k}.json")));
            std::thread::spawn(move || {
                let mut solved = 0;
                for _ in 0..ROUNDS {
                    let _ = std::fs::remove_file(&cache);
                    let params = RunParams {
                        mode: InputMode::None,
                        solve_mode: SolveMode::Offline,
                        image_path: img.clone(),
                        offline_timeout_s: 60.0,
                        ..Default::default()
                    };
                    let env = RunEnv { cache_file: Some(cache.clone()), online: Some(false), skip_figures: true, ..Default::default() };
                    if run_with(&params, &env, &AtomicBool::new(false), &|_: &str| {}).wcs.is_some() {
                        solved += 1;
                    }
                }
                let _ = tx.send((k, solved));
            });
        }
        drop(tx);
        for _ in 0..paths.len() {
            match rx.recv_timeout(Duration::from_secs(180)) {
                Ok((k, solved)) => assert_eq!(solved, ROUNDS, "{}: 毎回解けるはず", paths[k].display()),
                Err(_) => panic!("同時に走らせた解析が 180 秒で終わらない(止まっている)"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 締め切りまで探す場面(Rust でも 20 秒以上かかる)。
    const SLOW_SCENES: [&str; 2] = ["orig_focal_short", "defocus_rig"];

    fn run_solver_cases(include_slow: bool) {
        let path = app_testdata().join("run_solver.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("SKIP: {} がありません", path.display());
            return;
        };
        let d: Value = serde_json::from_str(&text).expect("run_solver.json を読めない");
        let dir = temp_dir(if include_slow { "run_solver_all" } else { "run_solver" });
        let (mut n_runs, mut n_lines, mut bad) = (0, 0, Vec::<String>::new());
        for r in d["runs"].as_array().unwrap() {
            let name = s(&r["name"]);
            if let Some(why) = r.get("skipped") {
                eprintln!("SKIP {name}: 基準値を作ったとき {}", s(why));
                continue;
            }
            if !include_slow && SLOW_SCENES.contains(&name.as_str()) {
                eprintln!("---- {name}: 締め切りまで探すので既定では飛ばす(run_solver_vs_python_all で回す)");
                continue;
            }
            let img = resolve_image(&s(&r["path"]));
            match std::fs::read(&img) {
                Ok(b) if sha2_like::sha256_hex(&b) == s(&r["sha256"]) => {}
                Ok(_) => {
                    eprintln!("SKIP {name}: {} の中身が基準値を作ったときと違う", img.display());
                    continue;
                }
                Err(_) => {
                    eprintln!("SKIP {name}: {} がありません", img.display());
                    continue;
                }
            }
            let params = params_from(&r["params"], img.clone());
            if !params.index_dir.is_dir() && name != "noindex" {
                eprintln!("SKIP {name}: 星図データがありません");
                continue;
            }
            let cache = dir.join(format!("cache_{name}.json"));
            write_cache(&cache, r["with_rig"].as_bool().unwrap_or(false));
            let env = RunEnv { cache_file: Some(cache.clone()), offline_solver: None, online: Some(false), skip_figures: false };
            let lines = StdMutex::new(Vec::<String>::new());
            let log = |m: &str| lines.lock().unwrap().push(m.to_string());
            let t0 = Instant::now();
            let res = run_with(&params, &env, &AtomicBool::new(false), &log);
            let secs = t0.elapsed().as_secs_f64();
            let advice_only = s(&r["compare"]) == "advice";
            let got = normalize_solver_log(&lines.into_inner().unwrap(), advice_only);
            let want = normalize_solver_log(&r["log"].as_array().unwrap().iter().map(s).collect::<Vec<_>>(), advice_only);
            let mut why = Vec::new();
            if got != want {
                let mut diff = String::new();
                for i in 0..got.len().max(want.len()) {
                    let (g, w) = (got.get(i).map(String::as_str).unwrap_or("<無>"), want.get(i).map(String::as_str).unwrap_or("<無>"));
                    if g != w {
                        diff.push_str(&format!("  {i}: Rust {g:?}\n     Python {w:?}\n"));
                    }
                }
                why.push(format!("ログが違う\n{diff}"));
            }
            if res.wcs.is_some() != r["solved"].as_bool().unwrap() {
                why.push(format!("解けたか: Rust {} / Python {}", res.wcs.is_some(), r["solved"]));
            }
            if res.figures.len() != r["figures"].as_u64().unwrap() as usize {
                why.push(format!("図の枚数: Rust {} / Python {}", res.figures.len(), r["figures"]));
            }
            match (r.get("summary"), res.summary.as_ref()) {
                (Some(ps), Some(sm)) => {
                    // 解の WCS は Python と 1e-6 px で同じ(解法の係の試験)。まとめはそれより粗く比べる
                    let dra = (sm.center_ra - ps["ra"].as_f64().unwrap()).abs() * sm.center_dec.to_radians().cos();
                    let ddec = (sm.center_dec - ps["dec"].as_f64().unwrap()).abs();
                    if dra.max(ddec) * 3600.0 > 1e-3
                        || rel(sm.scale, ps["scale"].as_f64().unwrap()) > 1e-8
                        || (sm.pa_screen_up - ps["pa"].as_f64().unwrap()).abs() > 1e-6
                        || sm.parity_text() != s(&ps["parity"])
                    {
                        why.push(format!("まとめが違う: Rust {sm:?} / Python {ps}"));
                    }
                }
                (None, None) => {}
                (a, b) => why.push(format!("まとめ: Python {a:?} / Rust {b:?}")),
            }
            match (r.get("offsets"), &res.offsets) {
                (Some(po), Some(o)) => {
                    let e = [(o.sep_arcmin, &po[0]), (o.ra_arcmin, &po[1]), (o.dec_arcmin, &po[2])]
                        .iter()
                        .map(|(a, b)| (a - b.as_f64().unwrap()).abs())
                        .fold(0.0, f64::max);
                    if e > 1e-6 || o.ra_dir != s(&po[3]) || o.dec_dir != s(&po[4]) {
                        why.push(format!("ずれが違う: Rust {o:?} / Python {po}"));
                    }
                }
                (None, None) => {}
                (a, b) => why.push(format!("ずれ: Python {a:?} / Rust {b:?}")),
            }
            // 覚えた機材の画素スケール
            let after = zahyou_net::cache::load_cache_at(&cache);
            let got_rigs = after.get("rigs").cloned().unwrap_or(Value::Null);
            let want_rigs = &r["rigs_after"];
            for (k, v) in want_rigs.as_object().unwrap() {
                let g = got_rigs.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
                let e = rel(g, v.as_f64().unwrap());
                if e.is_nan() || e >= 1e-8 {
                    why.push(format!("機材の記憶 {k}: Rust {g} / Python {v}"));
                }
            }
            if got_rigs.as_object().map(|o| o.len()) != want_rigs.as_object().map(|o| o.len()) {
                why.push(format!("機材の記憶の数: Rust {got_rigs} / Python {want_rigs}"));
            }
            eprintln!(
                "{} {name}: {} 行 / 解けた {} / Rust {secs:.2} 秒・Python {} 秒",
                if why.is_empty() { "OK  " } else { "FAIL" },
                got.len(),
                res.wcs.is_some(),
                r["python_seconds"]
            );
            if !why.is_empty() {
                bad.push(format!("{name}: {}", why.join("\n")));
            }
            n_runs += 1;
            n_lines += got.len();
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(bad.is_empty(), "{}", bad.join("\n\n"));
        eprintln!("本物の解法つきの run(): {n_runs} 場面・{n_lines} 行が Python と同じ(時刻の数を除く)");
    }

    /// sha256(試験画像が基準値を作ったときと同じかを見るだけ。依存を足さないよう自前で書く)。
    pub(crate) mod sha2_like {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
            0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
            0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
            0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
            0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
            0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
            0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        pub fn sha256_hex(data: &[u8]) -> String {
            let mut h: [u32; 8] =
                [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
            let mut msg = data.to_vec();
            let bits = (data.len() as u64).wrapping_mul(8);
            msg.push(0x80);
            while msg.len() % 64 != 56 {
                msg.push(0);
            }
            msg.extend_from_slice(&bits.to_be_bytes());
            for chunk in msg.chunks(64) {
                let mut w = [0u32; 64];
                for i in 0..16 {
                    w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
                }
                for i in 16..64 {
                    let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                    let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                    w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
                }
                let mut v = h;
                for i in 0..64 {
                    let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
                    let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
                    let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
                    let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
                    let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
                    let t2 = s0.wrapping_add(maj);
                    v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
                }
                for i in 0..8 {
                    h[i] = h[i].wrapping_add(v[i]);
                }
            }
            h.iter().map(|x| format!("{x:08x}")).collect()
        }

        #[test]
        fn known() {
            assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        }
    }
}

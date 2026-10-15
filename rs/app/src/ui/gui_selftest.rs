//! 画面の自己診断(Python 版 selftest の画面の項目)。窓を出さずに egui を回して確かめる。
//!
//!   zahyou.exe --selftest <画像> [--online|--offline] [--out f]
//!
//! 画面を組み立て、解析を 2 回(目標あり・完全ブラインド)通し、3 つの数字・図・ログ・状態バー・
//! 部品のはみ出し・下のバー・配色の切り替えを見る。利用者の設定(gui.json)には書かない。

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, RawInput, Rect, vec2};

use super::app::{Options, TopTab, ZahyouApp};
use super::fonts;
use super::settings::{Mode, Solve};
use super::theme::ThemeName;

/// 1 項目 (名前, 通ったか, 補足)
pub type Row = (String, bool, String);

struct Rig {
    ctx: egui::Context,
    app: ZahyouApp,
    size: egui::Vec2,
}

impl Rig {
    fn frame(&mut self) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            max_texture_side: Some(8192),
            ..Default::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| app.frame(ctx));
    }
    /// until が真になるか seconds が過ぎるまで回す
    fn pump(&mut self, seconds: f64, until: impl Fn(&ZahyouApp) -> bool) -> bool {
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f64() < seconds {
            self.frame();
            if until(&self.app) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        false
    }
}

/// 画面の自己診断(画面の項目だけ。解析そのものの項目は pipeline 係の selftest_rows)。
/// 画面からも解析を 1 回通して、数字・図・状態バーが画面に届くことを見る。
pub fn run(image: &Path, target: &str, force: Option<Solve>, debug_wcs: Option<std::path::PathBuf>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let mut check = |name: &str, ok: bool, msg: String| out.push((name.to_string(), ok, msg));
    let ctx = egui::Context::default();
    let (defs, report) = fonts::build();
    ctx.set_fonts(defs);
    super::style::apply(&ctx);
    let opts = Options { headless: true, debug_wcs, no_startup_checks: false, no_save: true };
    let app = ZahyouApp::new(&ctx, report, opts);
    let mut rig = Rig { ctx, app, size: vec2(1180.0, 780.0) };
    rig.frame();
    check("画面が組み立てられる", true, String::new());
    check("日本語のフォントを読める", rig.app.fonts.ui.is_some(), rig.app.fonts.summary());

    let surveyed = rig.pump(120.0, |a| a.env_state.is_some());
    let ready = rig.app.env_state.as_ref().is_some_and(|s| s.ready);
    check("準備タブの表が埋まる", surveyed && rig.app.env_rows[2].0.is_some(), rig.app.lbl_env.clone());

    if image.exists() {
        rig.app.s.image = image.display().to_string();
        rig.app.s.mode = Mode::StarName;
        rig.app.s.name = target.to_string();
        rig.app.s.solve = force.unwrap_or(if ready { Solve::Offline } else { Solve::Auto });
        rig.frame();
        let t0 = Instant::now();
        rig.app.start_analysis();
        let done = rig.pump(420.0, |a| !a.busy);
        rig.pump(0.3, |_| false);
        let a = &rig.app;
        let shown = a.vals.iter().all(|v| v != "—" && !v.is_empty());
        check(
            "画面から解析を通せる",
            done && shown,
            format!("{:.1} 秒 / {} / {} / {}", t0.elapsed().as_secs_f64(), a.vals[0], a.vals[1], a.vals[2]),
        );
        let n = a.figs.iter().filter(|f| f.is_some()).count();
        check("図が 2 枚貼られる", n == 2, format!("{n} 枚"));
        check("結果が状態バーに残る", a.status.contains("解析できました"), format!("{:?}", a.status));
        // 図を描けること(両方のタブを表示してみる。テクスチャの段づくり・タイル分け)
        let r = super::worker::guarded(|| {
            for t in [super::settings::ResultTab::Orig, super::settings::ResultTab::NorthUp] {
                rig.app.out_tab = t;
                rig.frame();
                rig.frame();
            }
        });
        check("図を表示できる", r.is_ok(), r.err().map(|e| e.0).unwrap_or_default());
    } else {
        check("画像がある", false, image.display().to_string());
    }

    // --- 部品が窓の中・枠の中にあること(いちばん小さい窓でも) ---
    let mut outside = Vec::new();
    let mut over = Vec::new();
    for size in [vec2(1180.0, 780.0), vec2(1000.0, 660.0)] {
        rig.size = size;
        for tab in [TopTab::Run, TopTab::Env, TopTab::Help] {
            rig.app.tab = tab;
            rig.frame();
            rig.frame();
            let win = Rect::from_min_size(Pos2::ZERO, size);
            for p in &rig.app.placed {
                if !win.expand(1.0).contains_rect(p.rect) {
                    outside.push(format!("{} ({:.0}x{:.0})", p.name, size.x, size.y));
                }
                if p.frame.is_positive() && !p.frame.expand(1.0).contains_rect(p.rect) {
                    over.push(format!("{} (右端 {:.0} > 枠 {:.0})", p.name, p.rect.max.x, p.frame.max.x));
                }
            }
        }
    }
    outside.dedup();
    over.dedup();
    check("部品が窓の中にある", outside.is_empty(), outside.join(" / ").chars().take(90).collect());
    check("枠からはみ出た部品が無い", over.is_empty(), over.join(" / ").chars().take(90).collect());
    rig.app.tab = TopTab::Run;
    rig.frame();
    let bar = rig.app.placed.iter().find(|p| p.name == "配色ボタン").map(|p| p.rect);
    let win = Rect::from_min_size(Pos2::ZERO, rig.size);
    check(
        "下のバーが窓の中にある",
        bar.is_some_and(|b| win.contains_rect(b)),
        bar.map(|b| format!("ボタン下端 {:.0} / 窓 {:.0}", b.max.y, win.max.y)).unwrap_or_else(|| "置かれていない".into()),
    );

    // --- 明→暗→明 ---
    let r = super::worker::guarded(|| {
        let first = rig.app.s.theme;
        for name in [ThemeName::Dark, ThemeName::Light, first] {
            rig.app.set_theme(name);
            for tab in [TopTab::Run, TopTab::Env, TopTab::Help] {
                rig.app.tab = tab;
                rig.frame();
            }
        }
        rig.app.s.theme
    });
    match r {
        Ok(t) => check("配色を切り替えられる", true, format!("最後は {}", t.key())),
        Err((m, _)) => check("配色を切り替えられる", false, m),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 中止すると中止しましたと出る() {
        let img = Path::new(r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits");
        if !img.exists() {
            eprintln!("SKIP: {} が無い", img.display());
            return;
        }
        super::super::settings::override_path(std::env::temp_dir().join("zahyou-test-cancel.json"));
        let ctx = egui::Context::default();
        let (defs, report) = fonts::build();
        ctx.set_fonts(defs);
        let opts = Options { headless: true, debug_wcs: None, no_startup_checks: true, no_save: true };
        let app = ZahyouApp::new(&ctx, report, opts);
        let mut rig = Rig { ctx, app, size: vec2(1180.0, 780.0) };
        rig.frame();
        rig.app.s.image = img.display().to_string();
        rig.app.s.solve = Solve::Offline;
        rig.app.start_analysis();
        assert!(rig.app.busy);
        rig.app.cancel();
        assert!(rig.app.log_run.contains("■ 中止します"));
        assert!(rig.pump(60.0, |a| !a.busy), "止まらない");
        rig.pump(0.2, |_| false);
        assert_eq!(rig.app.detail, "中止しました。");
        assert_eq!(rig.app.status, "中止しました。");
        // 二重には走らない
        rig.app.start_analysis();
        rig.app.start_analysis();
        assert!(rig.app.dialogs.iter().any(|d| d.contains("いま別の処理が動いています")));
        rig.pump(60.0, |a| !a.busy);
    }

    #[test]
    fn 画像が無いと知らせる() {
        super::super::settings::override_path(std::env::temp_dir().join("zahyou-test-cancel.json"));
        let ctx = egui::Context::default();
        let opts = Options { headless: true, debug_wcs: None, no_startup_checks: true, no_save: true };
        let mut app = ZahyouApp::new(&ctx, Default::default(), opts);
        app.s.image = r"C:\no\such\file.fits".into();
        app.start_analysis();
        assert!(!app.busy);
        assert_eq!(app.dialogs.last().map(String::as_str), Some("画像を選んでください。"));
        app.s.image = std::env::current_exe().unwrap().display().to_string();
        app.s.mode = Mode::StarName;
        app.s.name = "  ".into();
        app.start_analysis();
        assert!(app.dialogs.last().unwrap().starts_with("目標の天体名を入れてください。"));
        // 準備タブ: 段を選ばずに「選んだ段を落とす」
        app.download_index();
        assert_eq!(app.dialogs.last().map(String::as_str), Some("落とす段にチェックを入れてください。"));
        // 焦点距離が空で「必要な段を選ぶ」
        app.recommend();
        assert!(app.dialogs.last().unwrap().starts_with("焦点距離とセンサー横幅 [mm] を入れてください。"));
        // 更新を確認する前に「更新する」
        app.apply_update();
        assert_eq!(app.dialogs.last().map(String::as_str), Some("先に「更新を確認」を押してください。"));
    }
}

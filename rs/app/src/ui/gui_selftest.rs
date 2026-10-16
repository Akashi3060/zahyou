//! 画面の自己診断(Python 版 selftest の画面の項目)。窓を出さずに egui を回して確かめる。
//!
//!   zahyou.exe --selftest <画像> [--online|--offline] [--out f]
//!
//! 画面を組み立て、解析を 2 回(目標あり・完全ブラインド)通し、API キーの欄(値は出さない)・3 つの数字・図・ログ・
//! 状態バー・部品のはみ出し・下のバー・配色の切り替えを見る。利用者の設定(gui.json)には書かない。

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
    // ［解析］の枠の API キーの欄(値は出さない。長さと既定のキーとの一致だけ)
    let (ok, msg) = super::run_tab::api_key_selftest(&rig.app);
    check("API キーの欄がある", ok, msg);

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
        rig.app.start_analysis(None);
        // 始める前に断られた(「オンラインのみ」なのに API キーが無いなど)ときは、その知らせを補足に出す
        let refused = (!rig.app.busy).then(|| rig.app.dialogs.last().cloned().unwrap_or_default());
        let done = rig.pump(420.0, |a| !a.busy);
        rig.pump(0.3, |_| false);
        let a = &rig.app;
        let shown = a.vals.iter().all(|v| v != "—" && !v.is_empty());
        check(
            "画面から解析を通せる",
            refused.is_none() && done && shown,
            match &refused {
                Some(m) => format!("始められませんでした: {}", m.replace('\n', " ")),
                None => format!("{:.1} 秒 / {} / {} / {}", t0.elapsed().as_secs_f64(), a.vals[0], a.vals[1], a.vals[2]),
            },
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
            // ［解析］の左の列はスクロールする。小さい窓で下に隠れる欄(API キー)も、いちばん下まで送れば窓に入るかを見る
            rig.app.scroll_left_end = tab == TopTab::Run;
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
    rig.app.scroll_left_end = false;
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
        rig.app.start_analysis(None);
        assert!(rig.app.busy);
        rig.app.cancel();
        assert!(rig.app.log_run.contains("■ 中止します"));
        assert!(rig.pump(60.0, |a| !a.busy), "止まらない");
        rig.pump(0.2, |_| false);
        assert_eq!(rig.app.detail, "中止しました。");
        assert_eq!(rig.app.status, "中止しました。");
        // 二重には走らない
        rig.app.start_analysis(None);
        rig.app.start_analysis(None);
        assert!(rig.app.dialogs.iter().any(|d| d.contains("いま別の処理が動いています")));
        rig.pump(60.0, |a| !a.busy);
    }

    /// 画面から「中止」を押してから、解析のワーカーが止まる(busy が下りる)までの時間を測る。手で回す:
    ///   cargo test -p zahyou --release 中止の遅れ -- --ignored --nocapture
    /// 解けない画像(ピンぼけの実写。$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_55_14.fits)を目標なし・
    /// オフラインで解かせ、始めてから 0.5〜8 秒後に押す。画面は 30 ms ごとに回すので、その分の粒の粗さがある。
    /// 利用者の設定には書かない(gui.json は一時ファイル。目標なしなので cache.json も読み書きしない)。
    #[test]
    #[ignore]
    fn 中止の遅れを測る() {
        let base = std::env::var_os("ZAHYOU_TEST_IMAGES")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let img = base.join("Downloads").join("Capture_00001 00_55_14.fits");
        if !img.exists() || !Path::new(crate::pipeline::DEFAULT_INDEX_DIR).is_dir() {
            eprintln!("SKIP: {} か星図データが無い", img.display());
            return;
        }
        super::super::settings::override_path(std::env::temp_dir().join("zahyou-test-cancel-lag.json"));
        let ctx = egui::Context::default();
        let (defs, report) = fonts::build();
        ctx.set_fonts(defs);
        let opts = Options { headless: true, debug_wcs: None, no_startup_checks: true, no_save: true };
        let app = ZahyouApp::new(&ctx, report, opts);
        let mut rig = Rig { ctx, app, size: vec2(1180.0, 780.0) };
        let mut worst: f64 = 0.0;
        for after in [0.5, 1.0, 2.0, 3.0, 5.0, 8.0] {
            rig.app.s.image = img.display().to_string();
            rig.app.s.mode = Mode::None;
            rig.app.s.solve = Solve::Offline;
            rig.frame();
            rig.app.start_analysis(None);
            assert!(rig.app.busy);
            let t0 = Instant::now();
            assert!(!rig.pump(after, |a| !a.busy), "{after} 秒より前に終わった(解けない画像のはず)");
            let t_cancel = Instant::now();
            rig.app.cancel();
            assert!(rig.pump(30.0, |a| !a.busy), "止まらない");
            let lag = t_cancel.elapsed().as_secs_f64();
            worst = worst.max(lag);
            rig.pump(0.1, |_| false);
            eprintln!(
                "始めて {:.2} 秒で中止 → {lag:.3} 秒で止まった / 状態バー {:?}",
                (t_cancel - t0).as_secs_f64(),
                rig.app.status
            );
            assert_eq!(rig.app.status, "中止しました。");
        }
        eprintln!("画面からの中止: 最大の遅れ {worst:.3} 秒(画面を回す間隔 30 ms 込み)");
        assert!(worst < 1.0, "中止の遅れが 1 秒を超えた: {worst:.3} 秒");
    }

    #[test]
    fn 画像が無いと知らせる() {
        super::super::settings::override_path(std::env::temp_dir().join("zahyou-test-cancel.json"));
        let ctx = egui::Context::default();
        let opts = Options { headless: true, debug_wcs: None, no_startup_checks: true, no_save: true };
        let mut app = ZahyouApp::new(&ctx, Default::default(), opts);
        app.s.image = r"C:\no\such\file.fits".into();
        app.start_analysis(None);
        assert!(!app.busy);
        assert_eq!(app.dialogs.last().map(String::as_str), Some("画像を選んでください。"));
        app.s.image = std::env::current_exe().unwrap().display().to_string();
        app.s.mode = Mode::StarName;
        app.s.name = "  ".into();
        app.start_analysis(None);
        assert!(app.dialogs.last().unwrap().starts_with("目標の天体名を入れてください。"));
        // 準備タブ: 段を選ばずに「選んだ段を落とす」
        app.download_index(None);
        assert_eq!(app.dialogs.last().map(String::as_str), Some("落とす段にチェックを入れてください。"));
        // 焦点距離が空で「必要な段を選ぶ」
        app.recommend(None);
        assert!(app.dialogs.last().unwrap().starts_with("焦点距離とセンサー横幅 [mm] を入れてください。"));
        // 更新を確認する前に「更新する」
        app.apply_update(None);
        assert_eq!(app.dialogs.last().map(String::as_str), Some("先に「更新を確認」を押してください。"));
    }

    /// 窓を出さない画面(確認はいつも「キャンセル」)。設定は一時ファイル、星図データの置き場所は空のフォルダー
    fn headless_app(sub: &str) -> (ZahyouApp, std::path::PathBuf) {
        super::super::settings::override_path(std::env::temp_dir().join("zahyou-test-cancel.json"));
        let ctx = egui::Context::default();
        let opts = Options { headless: true, debug_wcs: None, no_startup_checks: true, no_save: true };
        let mut app = ZahyouApp::new(&ctx, Default::default(), opts);
        let dir = std::env::temp_dir().join(format!("zahyou-gui-empty-{sub}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.s.indexdir = dir.display().to_string();
        (app, dir)
    }

    #[test]
    fn 探索半径がおかしいと始めない() {
        let (mut app, dir) = headless_app("radius");
        app.s.image = std::env::current_exe().unwrap().display().to_string();
        app.s.mode = Mode::None;
        for r in ["-3", "0", "abc"] {
            app.s.radius = r.into();
            app.start_analysis(None);
            assert!(!app.busy, "{r} で始めた");
            assert_eq!(
                app.dialogs.last().map(String::as_str),
                Some(format!("探索半径の「{r}」は使えません。\n0 より大きい数 (度) を入れてください。空欄なら 5° で探します。").as_str())
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn まとめて準備は落とす前に必ず聞く() {
        let (mut app, dir) = headless_app("prepare");
        // (a) 印があっても聞く(確認は「キャンセル」なので落とさない)
        for s in [5202u32, 5203] {
            app.marks.insert(s, true);
        }
        let n = app.dialogs.len();
        app.prepare_all(None);
        assert!(!app.busy, "確認なしで落とし始めた");
        assert_eq!(app.dialogs.len(), n + 1);
        assert!(app.dialogs.last().unwrap().starts_with("印を付けた段をそろえます。\n足りない 2 段"), "{:?}", app.dialogs.last());
        // (b) 「おすすめをまとめて落とす」をキャンセルしたら印を元に戻す
        let before = app.marks.clone();
        app.download_recommended(None);
        assert!(!app.busy);
        assert!(app.dialogs.last().unwrap().starts_with("画角 "), "{:?}", app.dialogs.last());
        assert_eq!(app.marks, before, "キャンセルしたのに印が残った");
        for v in app.marks.values_mut() {
            *v = false;
        }
        // 印が無ければおすすめの段について聞く
        app.prepare_all(None);
        assert!(!app.busy);
        assert!(app.dialogs.last().unwrap().starts_with("段が選ばれていないので、おすすめ"), "{:?}", app.dialogs.last());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn 置き場所を変えたら前の結果で判断しない() {
        let (mut app, dir) = headless_app("stale");
        // 前の置き場所でおすすめの段が全部そろっていた、という古い結果
        let mut old = super::super::survey::Survey::default();
        old.index.dir = std::path::PathBuf::from(r"C:\somewhere\else");
        old.index.scales = zahyou_net::recommended_scales();
        old.ready = true;
        app.env_state = Some(old.clone());
        // 新しい(空の)置き場所で押す: 数え直すので「そろっています」とは言わず、容量を聞く
        app.download_recommended(None);
        assert!(!app.busy);
        assert_ne!(app.status, "おすすめの段はすべてそろっています。");
        assert!(app.dialogs.last().is_some_and(|d| d.starts_with("画角 ")), "{:?}", app.dialogs.last());
        for v in app.marks.values_mut() {
            *v = false;
        }
        let n = app.dialogs.len();
        app.prepare_all(None);
        assert!(!app.busy, "確認なしで落とし始めた");
        assert_eq!(app.dialogs.len(), n + 1);
        // 置き場所を変えたら表は「未確認」に戻り、前の置き場所の結果が(調べ終わって)届いても使わない
        app.forget_env();
        assert!(app.env_state.is_none());
        assert!(app.env_rows.iter().all(|r| r.0.is_none()));
        app.job(super::super::worker::LogTo::Env).send(super::super::worker::Msg::Env(Box::new(old)));
        app.pump();
        assert!(app.env_state.is_none(), "前の置き場所の結果を使った");
        // 今の置き場所の結果は使う
        let now = super::super::survey::survey(&app.index_dir());
        app.job(super::super::worker::LogTo::Env).send(super::super::worker::Msg::Env(Box::new(now)));
        app.pump();
        assert!(app.env_state.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! [解析] タブ: 上段(画像・参照・実行・中止)、左の列(目標・望遠鏡・解析)、右の結果欄。

use eframe::egui::{self, RichText, TextEdit};
use zahyou_net::nova::ApiKey;

use super::app::{Action, ZahyouApp, fig_index};
use super::fonts::{self, pt};
use super::settings::{Mode, ResultTab, SOLVE_CHOICES};
use super::widgets::{self, Placed};

/// ［解析］タブの API キーの欄の見出し
pub const API_KEY_LABEL: &str = "API キー (nova.astrometry.net)";
/// 自己診断が部品の記録(placed)で欄を探すときの名前
pub const API_KEY_FIELD: &str = "API キー";
/// 欄の egui の id(並びの順から決まる自動の id にせず固定する。上に部品が増えても入力中の状態が保たれる)
const API_KEY_FIELD_ID: &str = "api_key_field";

/// 自己診断の「API キーの欄がある」(［解析］タブを描いたあとに呼ぶ)。
///
/// 欄・「表示」・「元に戻す」が部品の記録にあり、伏せ字で、初めの値(環境変数 ASTROMETRY_API_KEY、無ければ既定のキー)が
/// 入っていれば通る。補足にキーの値は書かず、長さと既定のキーとの一致だけを書く。
pub fn api_key_selftest(app: &ZahyouApp) -> (bool, String) {
    let has = |n: &str| app.placed.iter().any(|p| p.name == n);
    let field = has(API_KEY_FIELD) && has("表示") && has("元に戻す");
    let k = &app.s.api_key;
    let n = k.expose().chars().count();
    let what = if k.usable().is_none() {
        "空".to_string()
    } else if k.is_default() {
        format!("既定のキーと一致 ({n} 文字)")
    } else if zahyou_net::api_key_from_env().is_some_and(|e| k.usable() == Some(e.as_str())) {
        format!("環境変数 ASTROMETRY_API_KEY のキー ({n} 文字)")
    } else {
        format!("既定のキーと違う ({n} 文字)")
    };
    let ok = field && !app.show_api_key && *k == ApiKey::initial();
    let msg = format!(
        "{} / {} / {what}",
        if field { "欄あり" } else { "欄が見つからない" },
        if app.show_api_key { "伏せ字でない" } else { "伏せ字" }
    );
    (ok, msg)
}

/// 欄の下の短い説明(キーの値は書かない。既定か・自分のキーか・空か と、どの方法で使うか)
pub fn api_key_note(k: &ApiKey) -> String {
    const USE: &str = "方法が「自動」「オンラインのみ」のときに使います。";
    if k.usable().is_none() {
        "空です。「自動」は問い合わせずにオフラインで解き、「オンラインのみ」は始められません。".to_string()
    } else if k.is_default() {
        format!("既定のキー (作者のもの) です。{USE}")
    } else {
        format!("自分のキーです。{USE}")
    }
}

impl ZahyouApp {
    pub fn run_tab(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        // --- 上段 -------------------------------------------------------------
        let top = ui.horizontal(|ui| {
            widgets::head(ui, th, "画像");
            ui.add_space(4.0);
            // 右から: 中止・解析を実行・参照...、残りの幅をパスの欄に
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = ui.add_enabled(self.busy, egui::Button::new("中止"));
                if b.clicked() {
                    self.cancel();
                }
                self.placed.push(Placed { name: "中止".into(), rect: b.rect, frame: self.window_rect });
                let b = ui.add_enabled(!self.busy, widgets::run_button("解析を実行"));
                if b.clicked() {
                    self.request(Action::StartAnalysis);
                }
                self.placed.push(Placed { name: "解析を実行".into(), rect: b.rect, frame: self.window_rect });
                ui.add_space(8.0);
                let b = ui.button("参照...");
                if b.clicked() {
                    self.request(Action::ChooseImage);
                }
                self.placed.push(Placed { name: "参照...".into(), rect: b.rect, frame: self.window_rect });
                ui.add(TextEdit::singleline(&mut self.s.image).font(fonts::path(pt(10.0))).desired_width(ui.available_width()));
            });
        });
        let _ = top;
        ui.add_space(8.0);

        // --- 左の列 -------------------------------------------------------------
        egui::SidePanel::left("run_left")
            .resizable(true)
            .default_width(350.0)
            .width_range(280.0..=560.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 8, top: 0, bottom: 0 }))
            .show_inside(ui, |ui| {
                let mut area = egui::ScrollArea::vertical().id_salt("run_left_scroll").auto_shrink([false, false]);
                // 自己診断: いちばん下(API キーの欄)まで送って描く(小さい窓では下に隠れる。送れば窓に入るかを見る)。
                // 送る量は前の 1 枚で測ったもの。scroll_to_cursor は 2 枚あとにしか効かないので使わない
                if self.scroll_left_end {
                    area = area.vertical_scroll_offset(self.left_scroll_max);
                }
                widgets::scroll(ui, th, area, |ui| {
                    self.run_left(ui);
                    // 送れる量 = 中身の高さ − 見えている高さ(見えている枠は上下に clip_rect_margin ずつ広い)
                    let view_h = ui.clip_rect().height() - 2.0 * ui.visuals().clip_rect_margin;
                    self.left_scroll_max = (ui.min_rect().height() - view_h).max(0.0);
                });
            });

        // --- 右: 結果 -------------------------------------------------------------
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 0, top: 0, bottom: 4 })).show_inside(ui, |ui| {
            self.result_head(ui);
            ui.add_space(6.0);
            let tabs = [(ResultTab::Orig, " 撮影時の向き "), (ResultTab::NorthUp, " 北が上 "), (ResultTab::Log, " ログ ")];
            if let Some(t) = widgets::tab_strip(ui, th, self.out_tab, &tabs, false) {
                self.user_select_out(t);
            }
            ui.add_space(2.0);
            match fig_index(self.out_tab) {
                Some(i) => {
                    let a = match self.figs[i].as_mut() {
                        Some(f) => f.show(ui, th),
                        None => {
                            // 図がまだ無い(Tk でも空の枠だった)
                            let r = ui.available_rect_before_wrap();
                            ui.painter().rect_filled(r, 0.0, th.bg);
                            None
                        }
                    };
                    self.handle_fig_action(i, a);
                }
                None => self.log_view(ui, true),
            }
        });
    }

    fn run_left(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        let wrap_note = |ui: &mut egui::Ui, text: &str| {
            widgets::muted(ui, th, text);
        };
        // --- 目標 ---
        let (_, r1) = widgets::group(ui, th, "目標", |ui| {
            ui.radio_value(&mut self.s.mode, Mode::StarName, "天体名で指定");
            ui.add_enabled(self.s.mode == Mode::StarName, TextEdit::singleline(&mut self.s.name).desired_width(f32::INFINITY));
            ui.add_space(6.0);
            ui.radio_value(&mut self.s.mode, Mode::Coords, "赤経・赤緯で指定");
            egui::Grid::new("radec").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                ui.label("RA");
                ui.add_enabled(self.s.mode == Mode::Coords, TextEdit::singleline(&mut self.s.ra).desired_width(f32::INFINITY));
                ui.end_row();
                ui.label("Dec");
                ui.add_enabled(self.s.mode == Mode::Coords, TextEdit::singleline(&mut self.s.dec).desired_width(f32::INFINITY));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.radio_value(&mut self.s.mode, Mode::None, "指定しない (完全ブラインド)");
            wrap_note(ui, "画像がどこを向いているかだけ求めます。矢印は出ません。");
        });
        ui.add_space(8.0);
        // --- 望遠鏡 ---
        let (_, r2) = widgets::group(ui, th, "望遠鏡", |ui| {
            ui.horizontal(|ui| {
                ui.label("焦点距離 [mm]");
                ui.add(TextEdit::singleline(&mut self.s.focal).desired_width(80.0));
            });
            ui.add_space(4.0);
            wrap_note(ui, "空欄でも解けます。入れるとオフラインが速く確実に。");
        });
        ui.add_space(8.0);
        // --- 解析 ---
        let (rects, r3) = widgets::group(ui, th, "解析", |ui| {
            let mut rects = Vec::new();
            egui::Grid::new("solve").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("方法");
                let cb = egui::ComboBox::from_id_salt("solve_mode")
                    .selected_text(self.s.solve.label())
                    .width(ui.available_width().max(120.0))
                    .show_ui(ui, |ui| {
                        for (label, v) in SOLVE_CHOICES {
                            ui.selectable_value(&mut self.s.solve, v, label);
                        }
                    });
                rects.push(("方法".to_string(), cb.response.rect));
                ui.end_row();
                ui.label("探索半径 [°]");
                let e = ui.add(TextEdit::singleline(&mut self.s.radius).desired_width(64.0));
                rects.push(("探索半径".to_string(), e.rect));
                ui.end_row();
            });
            ui.add_space(6.0);
            for (text, v) in [
                ("目標の方向を手がかりにする (速い)", &mut self.s.hint),
                ("検出した星を緑の丸で囲む", &mut self.s.marks),
                ("画像に入っている座標情報を無視する", &mut self.s.ignore),
            ] {
                let c = ui.checkbox(v, text);
                rects.push((text.to_string(), c.rect));
            }
            // 「打ち切り [秒]」と 2 つの欄を 1 行に並べると、DPI の高い環境で右の欄が枠からはみ出す(v6 で踏んだ)
            ui.add_space(8.0);
            ui.label("打ち切り [秒]");
            ui.horizontal(|ui| {
                ui.label("オンライン");
                let e = ui.add(TextEdit::singleline(&mut self.s.t_online).desired_width(52.0));
                rects.push(("オンライン".to_string(), e.rect));
                ui.add_space(12.0);
                ui.label("オフライン");
                let e = ui.add(TextEdit::singleline(&mut self.s.t_offline).desired_width(52.0));
                rects.push(("オフライン".to_string(), e.rect));
            });
            // --- API キー(オンライン解析)。既定は伏せ字、「表示」で見える。「元に戻す」で作者の既定のキーへ ---
            // 欄・「表示」・「元に戻す」を 1 行に並べ、欄は残りの幅(枠の幅が 280 でも、はみ出さない)
            ui.add_space(8.0);
            ui.label(API_KEY_LABEL);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = ui
                        .add_enabled(!self.s.api_key.is_default(), egui::Button::new("元に戻す"))
                        .on_hover_text("作者の既定のキー (v6 と同じもの) に戻します");
                    if b.clicked() {
                        self.s.api_key = ApiKey::default_key();
                        self.save_settings();
                    }
                    rects.push(("元に戻す".to_string(), b.rect));
                    let c = ui.checkbox(&mut self.show_api_key, "表示");
                    rects.push(("表示".to_string(), c.rect));
                    let e = ui.add(
                        TextEdit::singleline(self.s.api_key.edit())
                            .id_salt(API_KEY_FIELD_ID)
                            .password(!self.show_api_key)
                            .hint_text("空 (問い合わせない)")
                            .font(fonts::path(pt(10.0)))
                            .desired_width(ui.available_width()),
                    );
                    // 入れ終えたら覚える(閉じる前に落ちても消えないように)
                    if e.lost_focus() {
                        self.save_settings();
                    }
                    rects.push((API_KEY_FIELD.to_string(), e.rect));
                });
            });
            wrap_note(ui, &api_key_note(&self.s.api_key));
            ui.add_space(10.0);
            if !self.lbl_env.is_empty() {
                widgets::muted(ui, th, &self.lbl_env);
            }
            rects
        });
        let _ = (r1, r2);
        for (name, rect) in rects {
            self.placed.push(Placed { name, rect, frame: r3 });
        }
    }

    /// 3 つの大きな数字と見出し、その下の 1 行
    fn result_head(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        ui.columns(3, |cols| {
            for (i, col) in cols.iter_mut().enumerate() {
                col.label(RichText::new(&self.caps[i]).size(pt(9.0)).color(th.muted));
                col.add(egui::Label::new(RichText::new(&self.vals[i]).font(fonts::bold(pt(20.0))).color(th.ink)).truncate());
            }
        });
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(&self.detail).color(th.muted)).truncate());
    }

    /// ログの欄(読むだけ。選んでコピーはできる)
    pub fn log_view(&mut self, ui: &mut egui::Ui, run: bool) {
        let th = self.th;
        let font = egui::FontId::monospace(pt(9.0));
        let color = th.ink;
        let wrap = !run; // 解析のログは折り返さない(v6 と同じ)、準備のログは折り返す
        egui::Frame::new().fill(th.panel).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            let text: &str = if run { &self.log_run } else { &self.log_env };
            let mut view: &str = text;
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = egui::text::LayoutJob::simple(buf.as_str().to_owned(), font.clone(), color, if wrap { wrap_width } else { f32::INFINITY });
                job.wrap.max_width = if wrap { wrap_width } else { f32::INFINITY };
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let id = if run { "log_run" } else { "log_env" };
            let area = if wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
            let area = area.id_salt(id).auto_shrink([false, false]).stick_to_bottom(true);
            // 折り返さないときの幅は「見えている幅」と「いちばん長い行」の大きいほう
            let view_w = (ui.available_width() - 16.0).max(50.0);
            widgets::scroll(ui, th, area, |ui| {
                ui.add(
                    TextEdit::multiline(&mut view)
                        .id_salt(id)
                        .frame(false)
                        .desired_width(view_w)
                        .desired_rows(8)
                        .layouter(&mut layouter),
                );
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::app::{ONLINE_NEEDS_KEY, Options, TopTab};
    use crate::ui::settings::Solve;
    use eframe::egui::{Pos2, RawInput, Rect, vec2};
    use zahyou_net::nova::DEFAULT_API_KEY;

    /// 窓を出さない画面(確認はいつも「キャンセル」、設定は一時ファイルで保存しない)
    fn headless(fonts_on: bool) -> (egui::Context, ZahyouApp) {
        crate::ui::settings::override_path(std::env::temp_dir().join("zahyou-test-cancel.json"));
        let ctx = egui::Context::default();
        // 本物の画面と同じ書体・文字の大きさ・間隔(自己診断と同じ)
        let report = if fonts_on {
            let (defs, report) = fonts::build();
            ctx.set_fonts(defs);
            report
        } else {
            Default::default()
        };
        crate::ui::style::apply(&ctx);
        let opts = Options { headless: true, debug_wcs: None, no_startup_checks: true, no_save: true };
        let app = ZahyouApp::new(&ctx, report, opts);
        (ctx, app)
    }

    /// 1 枚描いて、描いた文字を全部返す
    /// `ppp` は画面の拡大率(Windows の表示スケール 100%・150%・200% = 1.0・1.5・2.0)。窓の大きさは論理 pt
    fn frame_texts(ctx: &egui::Context, app: &mut ZahyouApp, size: egui::Vec2, ppp: f32) -> Vec<String> {
        let mut input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        let out = ctx.run(input, |ctx| app.frame(ctx));
        fn walk(s: &egui::Shape, acc: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => acc.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, acc)),
                _ => {}
            }
        }
        let mut acc = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, &mut acc);
        }
        acc
    }

    #[test]
    fn api_キーの欄は伏せ字で_最小の窓でも枠に収まる() {
        let (ctx, mut app) = headless(true);
        app.tab = TopTab::Run;
        app.s.api_key = ApiKey::default_key();
        // 自己診断と同じく、準備タブを調べ終えたあと(枠の下に 1 行増えている)
        app.lbl_env = "オフライン解析: 使えます / 解ける画角 4′〜2000′".into();
        for (size, ppp) in [(vec2(1180.0, 780.0), 1.0), (vec2(1000.0, 660.0), 1.0), (vec2(1000.0, 660.0), 1.5), (vec2(1000.0, 660.0), 2.0)] {
            // 左の列をいちばん下まで送って 2 枚描く(1 枚目で送れる量を測り、2 枚目でそこまで送る)
            app.scroll_left_end = true;
            frame_texts(&ctx, &mut app, size, ppp);
            let texts = frame_texts(&ctx, &mut app, size, ppp);
            assert_eq!(ctx.pixels_per_point(), ppp);
            // 画面に描いた文字にキーの値が無い(伏せ字)。値は失敗の文にも入れない
            assert!(texts.iter().all(|t| !t.contains(DEFAULT_API_KEY)), "伏せ字のはずの欄にキーの値が描かれた");
            assert!(texts.iter().any(|t| t.contains(&"•".repeat(16))), "伏せ字 (16 文字) が描かれていない");
            assert!(texts.iter().any(|t| t == API_KEY_LABEL), "欄の見出しが無い");
            let win = Rect::from_min_size(Pos2::ZERO, size);
            for name in [API_KEY_FIELD, "表示", "元に戻す"] {
                let p = app.placed.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("{name} が置かれていない"));
                assert!(win.contains_rect(p.rect), "{name} が窓の外 ({size:?} 拡大率 {ppp}): {:?}", p.rect);
                assert!(p.frame.expand(1.0).contains_rect(p.rect), "{name} が［解析］の枠からはみ出た ({size:?} 拡大率 {ppp})");
            }
            let (ok, msg) = api_key_selftest(&app);
            assert_eq!(msg, "欄あり / 伏せ字 / 既定のキーと一致 (16 文字)");
            // 環境変数 ASTROMETRY_API_KEY があれば、初めの値はそちら(既定のキーの欄は「初めの値と違う」になる)
            if zahyou_net::api_key_from_env().is_none() {
                assert!(ok, "{msg}");
            }
        }
        // 「表示」にすると見える(描いた文字を拾えていることの確かめ。値は書かない)
        app.show_api_key = true;
        let texts = frame_texts(&ctx, &mut app, vec2(1180.0, 780.0), 1.0);
        assert!(texts.iter().any(|t| t.contains(DEFAULT_API_KEY)), "「表示」でキーが見えない");
        let (ok, msg) = api_key_selftest(&app);
        assert!(!ok && msg.contains("伏せ字でない"), "{msg}");
    }

    #[test]
    fn api_キーが空でオンラインのみなら始めない() {
        let (_ctx, mut app) = headless(false);
        app.s.image = std::env::current_exe().unwrap().display().to_string();
        app.s.mode = crate::ui::settings::Mode::None;
        app.s.solve = Solve::Online;
        for blank in ["", "  "] {
            app.s.api_key = ApiKey::new(blank);
            app.start_analysis(None);
            assert!(!app.busy, "空のキーで始めた");
            assert_eq!(app.dialogs.last().map(String::as_str), Some(ONLINE_NEEDS_KEY));
        }
        assert!(ONLINE_NEEDS_KEY.starts_with("API キーが空です。"));
    }

    #[test]
    fn 欄の説明にキーの値を書かない() {
        let def = api_key_note(&ApiKey::default_key());
        assert!(def.starts_with("既定のキー (作者のもの) です。"), "{def}");
        let own = api_key_note(&ApiKey::new("my-own-key-0123"));
        assert!(own.starts_with("自分のキーです。") && !own.contains("my-own-key-0123"), "{own}");
        let empty = api_key_note(&ApiKey::new(" "));
        assert!(empty.starts_with("空です。"), "{empty}");
        for t in [def, own, empty] {
            assert!(!t.contains(DEFAULT_API_KEY));
        }
    }
}

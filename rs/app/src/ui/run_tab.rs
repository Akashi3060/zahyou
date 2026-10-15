//! [解析] タブ: 上段(画像・参照・実行・中止)、左の列(目標・望遠鏡・解析)、右の結果欄。

use eframe::egui::{self, RichText, TextEdit};

use super::app::{ZahyouApp, fig_index};
use super::fonts::{self, pt};
use super::settings::{Mode, ResultTab, SOLVE_CHOICES};
use super::widgets::{self, Placed};

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
                    self.start_analysis();
                }
                self.placed.push(Placed { name: "解析を実行".into(), rect: b.rect, frame: self.window_rect });
                ui.add_space(8.0);
                let b = ui.button("参照...");
                if b.clicked() {
                    self.choose_image();
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
                let area = egui::ScrollArea::vertical().id_salt("run_left_scroll").auto_shrink([false, false]);
                widgets::scroll(ui, th, area, |ui| self.run_left(ui));
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

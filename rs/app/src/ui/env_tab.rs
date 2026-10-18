//! [準備] タブ: 星図データの表・まとめて準備・置き場所・おすすめ・必要な段・段の一覧・ログ。

use eframe::egui::{self, RichText, TextEdit};

use super::app::{Action, ZahyouApp};
use super::survey;
use super::widgets::{self, Placed};

impl ZahyouApp {
    pub fn env_tab(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        egui::SidePanel::left("env_left")
            .resizable(true)
            .default_width(430.0)
            .width_range(360.0..=640.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 8, top: 0, bottom: 4 }))
            .show_inside(ui, |ui| {
                let area = egui::ScrollArea::vertical().id_salt("env_left_scroll").auto_shrink([false, false]);
                widgets::scroll(ui, th, area, |ui| self.env_left(ui));
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 0, top: 0, bottom: 4 })).show_inside(ui, |ui| {
            widgets::head(ui, th, "ログ");
            ui.add_space(2.0);
            self.log_view(ui, false);
        });
    }

    fn env_left(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        widgets::head(ui, th, "オフライン解析に必要なもの");
        ui.add_space(4.0);
        let rows = [
            ("星図データ", self.env_rows[0].0, self.env_rows[0].1.clone()),
            ("検査", self.env_rows[1].0, self.env_rows[1].1.clone()),
            ("総合", self.env_rows[2].0, self.env_rows[2].1.clone()),
        ];
        let rows: Vec<(&str, Option<bool>, String)> = rows
            .into_iter()
            .map(|(n, ok, t)| {
                let shown = match ok {
                    Some(true) => format!("OK  {t}"),
                    Some(false) => format!("—  {t}"),
                    None => t,
                };
                (n, ok, shown)
            })
            .collect();
        widgets::table(ui, th, &rows);
        ui.add_space(6.0);
        let frame = ui.max_rect();
        ui.horizontal(|ui| {
            let b = ui.add_enabled(!self.busy, widgets::run_button("まとめて準備する"));
            if b.clicked() {
                self.request(Action::PrepareAll);
            }
            self.placed.push(Placed { name: "まとめて準備する".into(), rect: b.rect, frame });
            let b = ui.button("状態を確認");
            if b.clicked() {
                self.request(Action::Survey);
            }
            self.placed.push(Placed { name: "状態を確認".into(), rect: b.rect, frame });
            let b = ui.add_enabled(self.busy, egui::Button::new("中止"));
            if b.clicked() {
                self.cancel();
            }
            self.placed.push(Placed { name: "中止 (準備)".into(), rect: b.rect, frame });
        });
        ui.add_space(4.0);
        widgets::muted(ui, th, "解析は zahyou の中で行うので、要るのは星図データだけです (WSL は要りません)。");
        ui.add_space(8.0);

        let rec_gb = zahyou_net::scales_mb(&zahyou_net::recommended_scales()) as f64 / 1024.0;
        let minfov = survey::g(zahyou_net::RECOMMENDED_MIN_FOV);
        let (placed, grect) = widgets::group(ui, th, "星図データ", |ui| {
            let mut placed: Vec<(String, egui::Rect)> = Vec::new();
            ui.horizontal(|ui| {
                ui.label("置き場所");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = ui.add(egui::Button::new("...").min_size(egui::vec2(30.0, 0.0)));
                    if b.clicked() {
                        self.request(Action::ChooseIndexDir);
                    }
                    placed.push(("...".into(), b.rect));
                    let e = ui.add(
                        TextEdit::singleline(&mut self.s.indexdir)
                            .font(super::fonts::path(super::fonts::pt(10.0)))
                            .desired_width(ui.available_width()),
                    );
                    // 置き場所を書き換えたら、前の置き場所で調べた結果は使わない
                    if e.changed() {
                        self.forget_env();
                    }
                    placed.push(("置き場所".into(), e.rect));
                });
            });
            ui.add_space(6.0);
            let b = ui.button(format!("おすすめをまとめて落とす (約 {rec_gb:.1} GB)"));
            if b.clicked() {
                self.request(Action::DownloadRecommended);
            }
            placed.push(("おすすめをまとめて落とす".into(), b.rect));
            widgets::muted(
                ui,
                th,
                &format!("焦点距離が分からなければ、こちら。画角 {minfov}′ 以上をぜんぶ入れます（たいていの望遠鏡はこれで足ります）"),
            );
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
            widgets::muted(ui, th, "焦点距離が分かるなら、要る段だけに絞れます");
            ui.horizontal_wrapped(|ui| {
                ui.label("焦点距離 [mm]");
                let e = ui.add(TextEdit::singleline(&mut self.s.focal).desired_width(64.0));
                placed.push(("焦点距離".into(), e.rect));
                ui.add_space(12.0);
                ui.label("センサー横幅 [mm]");
                let e = ui.add(TextEdit::singleline(&mut self.s.sensor).desired_width(64.0));
                placed.push(("センサー横幅".into(), e.rect));
            });
            ui.add_space(4.0);
            let b = ui.button("必要な段を選ぶ");
            if b.clicked() {
                self.request(Action::Recommend);
            }
            placed.push(("必要な段を選ぶ".into(), b.rect));
            widgets::muted(ui, th, "この 2 つから画角を出して、要る段だけに印を付けます");
            ui.add_space(4.0);

            // 段の一覧。番号順ではなく画角の順(4100 系のあとに 5200 系が来ると、はしごが途中で切れて見える)
            let rec = zahyou_net::recommended_scales();
            let mut order: Vec<(u32, f64, f64)> = zahyou_net::SCALE_FOV.to_vec();
            order.sort_by(|a, b| (a.1, a.2).partial_cmp(&(b.1, b.2)).unwrap_or(std::cmp::Ordering::Equal));
            let have: Vec<u32> = self.env_state.as_ref().map(|s| s.index.scales.clone()).unwrap_or_default();
            egui::Frame::new().fill(th.panel).stroke(egui::Stroke::new(1.0, th.border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                let area = egui::ScrollArea::vertical().id_salt("scales").max_height(150.0).auto_shrink([false, true]);
                widgets::scroll(ui, th, area, |ui| {
                    ui.set_width(ui.available_width());
                    // ttk のチェックの並びと同じくらいに詰める
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.spacing_mut().interact_size.y = 20.0;
                    for (s, lo, hi) in order {
                        let mb = zahyou_net::scale_mb(s).unwrap_or(0);
                        let size = if mb >= 1024 { format!("{:.1} GB", mb as f64 / 1024.0) } else { format!("{mb} MB") };
                        let mark = if rec.contains(&s) { "" } else { " (おすすめ外)" };
                        let text = format!("index-{s}  画角 {}′〜{}′  {size}{mark}", survey::g(lo), survey::g(hi));
                        let v = self.marks.entry(s).or_insert(false);
                        ui.horizontal(|ui| {
                            ui.checkbox(v, text);
                            // v6 には入っている段の印が無かった(gui.md 7.9)。入っていれば「入っています」と出す
                            if have.contains(&s) {
                                ui.label(RichText::new("入っています").color(th.ok).size(super::fonts::pt(9.0)));
                            }
                        });
                    }
                });
            });
            ui.add_space(6.0);
            let b = ui.button("選んだ段を落とす");
            if b.clicked() {
                self.request(Action::DownloadIndex);
            }
            placed.push(("選んだ段を落とす".into(), b.rect));
            placed
        });
        for (name, rect) in placed {
            self.placed.push(Placed { name, rect, frame: grect });
        }
    }
}

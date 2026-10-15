//! [使い方] タブ: 更新の枠と短い手引き。

use eframe::egui::{self, RichText};

use super::app::{APP_NAME, ZahyouApp};
use super::fonts::{self, pt};
use super::settings::settings_dir;
use super::widgets::{self, Placed};
use crate::APP_VER;

pub const REPO_URL: &str = "https://github.com/Akashi3060/zahyou";

/// 手引きの 1 行の種類(Tk の Text のタグ h / b / m に当たる)
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Plain,
    Head,
    Bold,
    Muted,
}

/// 手引きの文(Python 版 v7 の文。WSL の手順は無い)
fn help_lines() -> Vec<(Tag, String)> {
    use Tag::*;
    let mut v: Vec<(Tag, String)> = Vec::new();
    let mut add = |t: Tag, s: &str| v.push((t, s.to_string()));
    add(Head, "使い方");
    add(Plain, "1. [解析] タブで「参照...」を押して、天体画像を選びます (窓へドラッグしても選べます)。");
    add(Plain, "   FITS / PNG / JPEG / TIFF が読めます。カラーでも動画キューブでも構いません。");
    add(Plain, "2. 目標を指定します。天体名 (UCAC4 660-021020 など) か、赤経・赤緯のどちらかです。");
    add(Plain, "3. 「解析を実行」を押します。");
    add(Plain, "");
    add(Head, "出た数字の読み方");
    add(Plain, "画面上部の 3 つが答えです。いま向いている点 (画像の中心) から目標まで、");
    add(Plain, "赤経方向と赤緯方向にどれだけ動かせばよいかを表しています。");
    add(Plain, "図は 2 枚出ます。「撮影時の向き」は撮ったままの向き、「北が上」は");
    add(Plain, "北を上・東を左にそろえたものです。赤道儀の軸操作と向きが揃います。");
    add(Plain, "水色の × が画像の中心、黄色の ○ が目標、赤い矢印が動かす向きです。");
    add(Plain, "図はホイールで拡大・縮小、ドラッグで移動できます。「全体」で元に戻ります。");
    add(Plain, "");
    add(Head, "ネットが無いところで使う");
    add(Plain, "オフラインで解くのに要るのは、星図データ (画角ごとに分かれた index ファイル) だけです。");
    add(Plain, "星の並びから向きを割り出す計算は zahyou の中で行うので、");
    add(Bold, "ほかのソフトを入れる必要はありません。");
    add(Plain, "[準備] タブで「まとめて準備する」を押すと、足りない星図データを落として、");
    add(Plain, "1 本ずつ中身 (途中で切れていないか) を確かめます。");
    add(Plain, "星図データは、望遠鏡の画角に合った段だけあれば足ります。");
    add(Plain, "  ・機材の数値が分からない  → 「おすすめをまとめて落とす」");
    add(Plain, "      画角 4' 以上をぜんぶ (約 9.5 GB)。たいていはこれで足ります。");
    add(Plain, "  ・焦点距離が分かる        → 入れてから「必要な段を選ぶ」");
    add(Plain, "      要る段だけに絞れるので、ずっと小さく済みます。");
    add(Muted, "途中で止めても、次に押せば続きから落とし直します。");
    add(Plain, "");
    add(Head, "天体名はオフラインでも使えます (2 回目から)");
    add(Plain, "天体名から座標を調べるにはネットが要りますが、一度調べた名前は");
    add(Plain, "この PC に記憶されます。自宅で一度実行しておけば、山の中でも");
    add(Plain, "同じ目標なら名前のまま使えます。");
    add(Plain, "初めての目標をオフラインで扱うときは、赤経・赤緯を直接入れてください。");
    add(Plain, "");
    add(Head, "困ったとき");
    add(Plain, "・星が少ないと解けません。露出を伸ばすか、複数枚を重ねてください。");
    add(Plain, "・オフラインで解けないときは、[準備] タブの「解ける画角」を見て、");
    add(Plain, "  その画角の段が入っているか確かめてください。");
    add(Plain, "・焦点距離を入れると、探す範囲が狭まって速く確実になります。");
    add(Plain, "・解析が長引いたら「中止」を押してください。数秒で止まります。");
    add(Plain, "");
    add(Head, "設定と記憶の置き場所");
    add(Plain, "設定 (目標・焦点距離・配色) と、覚えた座標・画素スケールは");
    add(Plain, "この PC の次の場所にあります。");
    add(Plain, "");
    add(Bold, &format!("    {}", settings_dir().display()));
    add(Plain, "");
    add(Plain, "エクスプローラーのアドレス欄にそのまま貼り付けると開きます。");
    add(Plain, "(AppData は隠しフォルダーなので、たどっても出てきません)");
    add(Plain, "消しても解析には影響しません。設定が初期状態に戻るだけです。");
    add(Plain, "");
    add(Head, "新しい版に入れ替える / やめる");
    add(Plain, "新しい zahyou.exe を、古いものに上書きするだけです。");
    add(Plain, "設定も覚えた座標も引き継がれ、星図データはそのままで構いません。");
    add(Plain, "v6 までに WSL へ入れた astrometry.net は、もう使いません。");
    add(Plain, "要らなければマニュアル 9 章の手順で消せます。");
    add(Plain, "やめるときは、zahyou.exe と上の置き場所を消してください。");
    add(Plain, "星図データ (C:\\AstrometryData) は、使わないならフォルダーごと消せます。");
    add(Plain, "");
    add(Head, "ライセンス");
    add(Plain, "zahyou 自身は MIT ライセンスです。自由に使って、書き換えて、");
    add(Plain, "配ってかまいません。著作権表示だけ残してください。");
    add(Muted, "星の並びから向きを割り出す解き方は、Lang et al. (2010, AJ 139, 1782) の");
    add(Muted, "論文をもとに zahyou 用に自前で書き直したものです。");
    add(Muted, "astrometry.net のプログラム (コード) は含んでいません。");
    add(Muted, "星図データ (index ファイル) は Tycho-2 と Gaia DR2 から作られたもので、");
    add(Muted, "この exe には同梱していません ([準備] タブが配布元から落とします)。");
    add(Muted, "この exe には第三者のライブラリが入っていますが、GPL・LGPL は含みません。");
    add(Muted, "詳しくは配布ページの THIRD-PARTY-NOTICES.md にあります。");
    add(Plain, "");
    add(Muted, &format!("{APP_NAME} {APP_VER}   Rust で書き直した版です (画面は egui)。"));
    v
}

impl ZahyouApp {
    pub fn help_tab(&mut self, ui: &mut egui::Ui) {
        let th = self.th;
        // --- 更新 ---------------------------------------------------------------
        let frame_rect = ui.max_rect();
        let (placed, grect) = widgets::group(ui, th, "更新", |ui| {
            let mut placed = Vec::new();
            ui.horizontal(|ui| {
                widgets::head(ui, th, &format!("いまの版: {APP_NAME} {APP_VER}"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = ui.add_enabled(self.update_can_apply && !self.busy, egui::Button::new("更新する"));
                    if b.clicked() {
                        self.apply_update();
                    }
                    placed.push(("更新する".to_string(), b.rect));
                    let b = ui.button("更新を確認");
                    if b.clicked() {
                        self.check_update();
                    }
                    placed.push(("更新を確認".to_string(), b.rect));
                });
            });
            if !self.update_msg.is_empty() {
                widgets::muted(ui, th, &self.update_msg);
            }
            let c = ui.checkbox(&mut self.s.update_auto, "起動したときに確認する（GitHub に問い合わせます）");
            if c.changed() {
                self.save_settings();
            }
            placed.push(("起動したときに確認する".to_string(), c.rect));
            placed
        });
        let _ = frame_rect;
        for (name, rect) in placed {
            self.placed.push(Placed { name, rect, frame: grect });
        }
        ui.add_space(8.0);

        // --- 手引き ---------------------------------------------------------------
        egui::Frame::new().fill(th.panel).inner_margin(egui::Margin { left: 14, right: 14, top: 12, bottom: 12 }).show(ui, |ui| {
            let area = egui::ScrollArea::vertical().id_salt("help").auto_shrink([false, false]);
            widgets::scroll(ui, th, area, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                for (tag, text) in help_lines() {
                    match tag {
                        Tag::Head => {
                            ui.add_space(10.0);
                            ui.label(RichText::new(text).font(fonts::bold(pt(12.0))).color(th.accent));
                            ui.add_space(4.0);
                        }
                        Tag::Bold => {
                            // 太字の行は置き場所のパスだけ(「\」が「¥」に見えないよう、英字は Segoe UI)
                            let f = if text.contains('\\') { fonts::path(pt(10.0)) } else { fonts::bold(pt(10.0)) };
                            ui.label(RichText::new(text).font(f).color(th.ink));
                        }
                        Tag::Muted => {
                            ui.label(RichText::new(text).color(th.muted));
                        }
                        Tag::Plain => {
                            if text.is_empty() {
                                ui.add_space(pt(10.0) * 0.8);
                            } else {
                                ui.label(RichText::new(text).color(th.ink));
                            }
                        }
                    }
                }
                ui.hyperlink_to(RichText::new(REPO_URL).color(th.accent), REPO_URL);
            });
        });
    }
}

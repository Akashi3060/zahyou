//! [使い方] タブ: 更新の枠と短い手引き。

use eframe::egui::{self, RichText};

use super::app::{APP_NAME, Action, ZahyouApp};
use super::fonts::{self, pt};
use super::settings::settings_dir;
use super::theme::Theme;
use super::widgets::{self, Placed};
use crate::APP_VER;

pub const REPO_URL: &str = "https://github.com/Akashi3060/zahyou";

/// 同梱しているソフトウェアの表示(rs/THIRD-PARTY-NOTICES.md。rs/tools/gen_notices.py が作る)。
///
/// exe だけを配っても(更新も exe の入れ替えだけ)表示が一緒に届くよう、exe に埋め込んで
/// ［使い方］タブで読めるようにする。約 200 KB(13 MB の exe の 1.5% ほど)。
pub const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.md");

/// 文字の幅(等幅の桁数): 英数字は 1、それ以外(日本語・記号)は 2 と数える
fn char_cols(c: char) -> usize {
    if c.is_ascii() { 1 } else { 2 }
}

/// 文を幅 cols 桁で折り返して行に分ける(なるべく空白のところで切る)。どの行も元の文の一部を指す
fn wrap_lines(text: &str, cols: usize) -> Vec<&str> {
    let cols = cols.max(20);
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        loop {
            // rest の先頭から cols 桁に収まるところまで
            let mut w = 0;
            let mut cut = rest.len();
            let mut space = None;
            for (i, c) in rest.char_indices() {
                let cw = char_cols(c);
                if w + cw > cols && i > 0 {
                    cut = i;
                    break;
                }
                w += cw;
                if c == ' ' {
                    space = Some(i + c.len_utf8());
                }
            }
            if cut >= rest.len() {
                out.push(rest);
                break;
            }
            // 後ろ半分に空白があれば、そこで切る(語の途中で切らない)
            if let Some(sp) = space
                && sp > cut / 2
                && sp < cut
            {
                cut = sp;
            }
            out.push(&rest[..cut]);
            rest = &rest[cut..];
        }
    }
    out
}

/// 表示を幅 cols 桁で折り返した行。幅が変わらない間は前の結果を使う(画面のスレッドだけが呼ぶ)
fn notices_wrapped(cols: usize) -> std::rc::Rc<Vec<&'static str>> {
    type Cache = std::cell::RefCell<Option<(usize, std::rc::Rc<Vec<&'static str>>)>>;
    thread_local! {
        static CACHE: Cache = const { std::cell::RefCell::new(None) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((k, v)) = c.as_ref()
            && *k == cols
        {
            return v.clone();
        }
        let v = std::rc::Rc::new(wrap_lines(THIRD_PARTY_NOTICES, cols));
        *c = Some((cols, v.clone()));
        v
    })
}

/// 同梱しているソフトウェアの表示の欄。既定は閉じておき、開いたときは見えている行だけを描く
/// (全部で約 3,600 行あるので、毎フレーム全行を並べると重い)。
///
/// 文は等幅で、欄の幅に合わせて自分で折り返す。横のスクロールにしないのは、入れ子のスクロールの欄の
/// 横のつまみを、egui が外側の欄の下端へ寄せて描くため(この欄が外側の欄の見えている範囲より下にあっても、
/// 外側の欄の下端に横のつまみが浮いて見える)。
fn notices_view(ui: &mut egui::Ui, th: &Theme) {
    let title = RichText::new("同梱しているソフトウェアの表示 (THIRD-PARTY-NOTICES.md)").color(th.ink);
    egui::CollapsingHeader::new(title).id_salt("third_party_notices").default_open(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::muted(ui, th, "配布物の THIRD-PARTY-NOTICES.md と同じ文です。");
            if ui.button("全文をコピー").clicked() {
                ui.ctx().copy_text(THIRD_PARTY_NOTICES.to_string());
            }
        });
        let font = egui::FontId::monospace(pt(9.0));
        // 1 行の高さ: 日本語の混じる行も英字だけの行も同じ高さで並べる(show_rows は行の高さが揃っている前提)
        let row_h = ui.painter().layout_no_wrap("Ag漢字".to_string(), font.clone(), th.ink).size().y.ceil().max(1.0);
        // 1 行の桁数: 枠の内側の幅(枠の余白とつまみの分を引く)を英数字 1 字の幅で割る
        let char_w = ui.painter().layout_no_wrap("0000000000".to_string(), font.clone(), th.ink).size().x / 10.0;
        let text_w = ui.available_width() - 8.0 - 2.0 - ui.spacing().scroll.allocated_width();
        let cols = if char_w > 0.5 { (text_w / char_w).floor().max(1.0) as usize } else { 100 };
        let lines = notices_wrapped(cols);
        let area = egui::ScrollArea::vertical().id_salt("third_party_notices_text").max_height(360.0).auto_shrink([false, true]);
        egui::Frame::new().fill(th.entry).stroke(egui::Stroke::new(1.0, th.border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.scope(|ui| {
                // つまみの色は widgets::scroll と同じにする(既定の色はチェックの箱の地と同じで見えにくい)
                {
                    let w = &mut ui.visuals_mut().widgets;
                    w.inactive.bg_fill = th.border;
                    w.hovered.bg_fill = th.muted;
                    w.active.bg_fill = th.accent;
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                area.show_rows(ui, row_h, lines.len(), |ui, range| {
                    for i in range {
                        let Some(line) = lines.get(i) else { break };
                        // 見出し(# で始まる行)だけ色を変える
                        let color = if line.starts_with('#') { th.accent } else { th.ink };
                        let g = ui.painter().layout_no_wrap((*line).to_string(), font.clone(), color);
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::hover());
                        ui.painter().galley(rect.min, g, color);
                    }
                });
            });
        });
    });
}

/// 手引きの 1 行の種類(Tk の Text のタグ h / b / m に当たる)
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Plain,
    Head,
    Bold,
    Muted,
    /// 同梱しているソフトウェアの表示(開け閉めできる欄。文は THIRD_PARTY_NOTICES)
    Notices,
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
    add(Head, "ネットにつながっているとき (オンライン解析)");
    add(Plain, "[解析] の「方法」が「自動」なら、ネットがあれば先に nova.astrometry.net に問い合わせ、");
    add(Plain, "解けなければ zahyou の中で解きます (オフライン)。問い合わせには API キーが要ります。");
    add(Plain, "キーは環境変数 ASTROMETRY_API_KEY に入れてから zahyou を起動してください");
    add(Plain, "(キーは nova.astrometry.net に登録すると、自分のページで見られます)。");
    add(Plain, "キーが無ければ問い合わせを飛ばして、はじめからオフラインで解きます。");
    add(Plain, "「オンラインのみ」はキーが無いと始められません。");
    add(Muted, "送るのは検出した星の位置です。それで解けなければ画像そのものを送ります。");
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
    add(Muted, "Gaia のデータは CC BY-NC 3.0 IGO です (商用利用には ESA の許可が要ります)。");
    add(Muted, "この exe には Rust のライブラリと、画面の既定の書体 (Hack・Ubuntu・Noto Emoji ほか) が");
    add(Muted, "入っています。どれも MIT・Apache-2.0・BSD・SIL OFL などの許容型で、GPL・LGPL・MPL のものは含みません。");
    add(Muted, "(リンクのときに入る GCC の実行時ライブラリは、GPL に「実行時ライブラリ例外」が付いたもので、");
    add(Muted, " exe の配り方には制限がかかりません。)");
    add(Muted, "一覧と各ライセンスの本文は、下の「同梱しているソフトウェアの表示」を開くと読めます。");
    add(Notices, "");
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
                        self.request(Action::ApplyUpdate);
                    }
                    placed.push(("更新する".to_string(), b.rect));
                    let b = ui.button("更新を確認");
                    if b.clicked() {
                        self.request(Action::CheckUpdate);
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
                        Tag::Notices => {
                            ui.add_space(4.0);
                            notices_view(ui, th);
                        }
                    }
                }
                ui.hyperlink_to(RichText::new(REPO_URL).color(th.accent), REPO_URL);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width(s: &str) -> usize {
        s.chars().map(char_cols).sum()
    }

    #[test]
    fn 折り返しても文は欠けず幅に収まる() {
        for cols in [0, 1, 20, 37, 80, 120] {
            let lines = wrap_lines(THIRD_PARTY_NOTICES, cols);
            // 元の行ごとにつなぐと元に戻る(どこも捨てていない)
            let joined: String = lines.concat();
            let orig: String = THIRD_PARTY_NOTICES.lines().collect();
            assert_eq!(joined, orig, "cols = {cols}");
            for l in &lines {
                assert!(width(l) <= cols.max(20), "cols = {cols}: {l:?}");
            }
        }
    }

    #[test]
    fn 変な入力でも止まる() {
        for t in ["", "\n\n", "あいうえおかきくけこさしすせそたちつてとなにぬねの", &"x".repeat(1000), "a b c d e f g h i j k l m n o p q r s t u v w x y z"] {
            let lines = wrap_lines(t, 20);
            assert_eq!(lines.concat(), t.lines().collect::<String>());
        }
        // 空白で切れるときは空白の後ろで切る
        let lines = wrap_lines("aaaa bbbb cccc dddd eeee ffff", 20);
        assert_eq!(lines, vec!["aaaa bbbb cccc dddd ", "eeee ffff"]);
    }

    #[test]
    fn 表示が埋め込まれている() {
        // 生成物の要るところが入っていること(rs/tools/gen_notices.py が作る)
        for key in ["Tycho-2", "ESA/Gaia/DPAC", "Lang, D., Hogg", "SIL OPEN FONT LICENSE Version 1.1", "UBUNTU FONT LICENCE Version 1.0", "`ring`", "GCC"] {
            assert!(THIRD_PARTY_NOTICES.contains(key), "{key} が無い");
        }
    }
}

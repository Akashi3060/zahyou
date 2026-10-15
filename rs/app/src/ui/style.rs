//! 文字の大きさと部品の間隔(ttk の既定に近づける)。色は theme.rs。

use eframe::egui::{self, FontFamily, FontId, TextStyle, vec2};

use super::fonts::pt;

/// 文字の大きさ(Tk の 10pt = 画面 13.3 px が本文)と間隔を当てる
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(pt(11.0), FontFamily::Name(super::fonts::BOLD.into()))),
            (TextStyle::Body, FontId::new(pt(10.0), FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(pt(10.0), FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(pt(9.0), FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(pt(9.0), FontFamily::Monospace)),
        ]
        .into();
        let sp = &mut style.spacing;
        sp.button_padding = vec2(10.0, 4.0);
        sp.item_spacing = vec2(6.0, 4.0);
        sp.interact_size = vec2(24.0, 24.0);
        sp.icon_width = 15.0;
        sp.icon_width_inner = 9.0;
        sp.icon_spacing = 6.0;
        sp.combo_height = 300.0;
        // 常に場所を取る太いスクロールバー(ttk と同じ)。つまみは地の色で塗る(widgets::scroll)
        sp.scroll = egui::style::ScrollStyle::solid();
        sp.scroll.bar_width = 12.0;
        sp.scroll.foreground_color = false;
        style.interaction.tooltip_delay = 0.4;
        // 文字を選べる(ログのコピーのため)。ボタンの文字は選ばない
        style.interaction.selectable_labels = false;
    });
}

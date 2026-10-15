//! 配色。Python 版(gui/zahyou_gui.py の THEMES)と同じ 21 個の色トークンで、明るい / 暗いの 2 つ。
//!
//! egui の Visuals をトークンから組み立てる。タイトルバーは winit の set_theme(中で
//! DwmSetWindowAttribute を呼ぶ)で合わせるので、自分で unsafe を書かなくて済む。

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

/// 配色の名前(gui.json の "theme")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeName {
    Light,
    Dark,
}

impl ThemeName {
    pub fn key(self) -> &'static str {
        match self {
            ThemeName::Light => "light",
            ThemeName::Dark => "dark",
        }
    }
    pub fn from_key(s: &str) -> Option<ThemeName> {
        match s {
            "light" => Some(ThemeName::Light),
            "dark" => Some(ThemeName::Dark),
            _ => None,
        }
    }
    pub fn toggled(self) -> ThemeName {
        match self {
            ThemeName::Light => ThemeName::Dark,
            ThemeName::Dark => ThemeName::Light,
        }
    }
}

/// 色トークン(Python 版の THEMES と同じ名前・同じ値)
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: ThemeName,
    pub bg: Color32,
    pub panel: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub entry: Color32,
    pub border: Color32,
    pub trough: Color32,
    pub btn: Color32,
    pub btn_hi: Color32,
    pub tab_bg: Color32,
    pub tab_fg: Color32,
    pub tab_sel_bg: Color32,
    pub tab_sel_fg: Color32,
    pub tab_line: Color32,
    pub ok: Color32,
    pub ng: Color32,
    pub fig_bg: Color32,
    pub ax_bg: Color32,
    pub fig_fg: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const LIGHT: Theme = Theme {
    name: ThemeName::Light,
    bg: hex(0xF4F6F8),
    panel: hex(0xFFFFFF),
    ink: hex(0x14181F),
    muted: hex(0x5A6472),
    accent: hex(0x1F5C8B),
    entry: hex(0xFFFFFF),
    border: hex(0xC9D4DE),
    trough: hex(0xE4E9EE),
    btn: hex(0xE8ECF1),
    btn_hi: hex(0xD6DEE6),
    tab_bg: hex(0xDCE3EA),
    tab_fg: hex(0x4A5563),
    tab_sel_bg: hex(0xFFFFFF),
    tab_sel_fg: hex(0x12507D),
    tab_line: hex(0x1F5C8B),
    ok: hex(0x2E7D32),
    ng: hex(0xC0392B),
    fig_bg: hex(0xFFFFFF),
    ax_bg: hex(0xFFFFFF),
    fig_fg: hex(0x14181F),
};

pub const DARK: Theme = Theme {
    name: ThemeName::Dark,
    bg: hex(0x15181D),
    panel: hex(0x1B1F26),
    ink: hex(0xDCE1E8),
    muted: hex(0x8A94A3),
    accent: hex(0x6FB4E6),
    entry: hex(0x0D0F13),
    border: hex(0x39414D),
    trough: hex(0x0D0F13),
    btn: hex(0x262C36),
    btn_hi: hex(0x333B47),
    tab_bg: hex(0x0F1216),
    tab_fg: hex(0x8A94A3),
    tab_sel_bg: hex(0x2B3543),
    tab_sel_fg: hex(0x9CD1F5),
    tab_line: hex(0x6FB4E6),
    ok: hex(0x5FC96F),
    ng: hex(0xE5736B),
    fig_bg: hex(0x15181D),
    ax_bg: hex(0x05070A),
    fig_fg: hex(0xDCE1E8),
};

pub fn theme(name: ThemeName) -> &'static Theme {
    match name {
        ThemeName::Light => &LIGHT,
        ThemeName::Dark => &DARK,
    }
}

/// トークンから egui の見た目を組み立てる。
pub fn visuals(th: &Theme) -> Visuals {
    let mut v = match th.name {
        ThemeName::Light => Visuals::light(),
        ThemeName::Dark => Visuals::dark(),
    };
    let r = CornerRadius::same(2);
    v.panel_fill = th.bg;
    v.window_fill = th.bg;
    v.window_stroke = Stroke::new(1.0, th.border);
    v.extreme_bg_color = th.trough; // スクロールバーの溝など
    // 入力欄の地
    v.text_edit_bg_color = Some(th.entry);
    v.faint_bg_color = th.panel;
    v.code_bg_color = th.panel;
    v.hyperlink_color = th.accent;
    v.warn_fg_color = th.ng;
    v.error_fg_color = th.ng;
    v.override_text_color = None;
    v.selection.bg_fill = th.accent;
    v.selection.stroke = Stroke::new(1.0, th.panel);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(3);
    v.striped = false;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    // 文字だけの部品(ラベル)
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = th.bg;
    w.noninteractive.weak_bg_fill = th.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, th.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, th.ink);
    w.noninteractive.corner_radius = r;
    // 押せる部品。ボタンの地は weak_bg_fill、チェックの箱の地は bg_fill
    w.inactive.weak_bg_fill = th.btn;
    w.inactive.bg_fill = th.entry;
    w.inactive.bg_stroke = Stroke::new(1.0, th.border);
    w.inactive.fg_stroke = Stroke::new(1.0, th.ink);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;
    w.hovered.weak_bg_fill = th.btn_hi;
    w.hovered.bg_fill = th.entry;
    w.hovered.bg_stroke = Stroke::new(1.0, th.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, th.ink);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.weak_bg_fill = th.btn_hi;
    w.active.bg_fill = th.entry;
    w.active.bg_stroke = Stroke::new(1.0, th.accent);
    w.active.fg_stroke = Stroke::new(2.0, th.accent);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open.weak_bg_fill = th.btn_hi;
    w.open.bg_fill = th.entry;
    w.open.bg_stroke = Stroke::new(1.0, th.accent);
    w.open.fg_stroke = Stroke::new(1.0, th.ink);
    w.open.corner_radius = r;
    v
}

/// 今の配色を画面へ当てる(egui の見た目とタイトルバー)。
pub fn apply(ctx: &egui::Context, th: &Theme) {
    let vis = visuals(th);
    let pref = match th.name {
        ThemeName::Light => egui::Theme::Light,
        ThemeName::Dark => egui::Theme::Dark,
    };
    // 両方の見た目を同じにしておく(OS の配色が変わっても、選んだ配色のまま)
    ctx.set_visuals_of(egui::Theme::Light, vis.clone());
    ctx.set_visuals_of(egui::Theme::Dark, vis);
    ctx.set_theme(pref);
    // タイトルバー(Windows では winit が DwmSetWindowAttribute を呼ぶ)
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match th.name {
        ThemeName::Light => egui::SystemTheme::Light,
        ThemeName::Dark => egui::SystemTheme::Dark,
    }));
}

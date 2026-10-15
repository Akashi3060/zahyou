//! 画面の小さな部品(ttk のタブ・枠・見出しの見た目を egui で作る)。

use eframe::egui::{self, Align2, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::fonts::{self, pt};
use super::theme::Theme;

/// 部品の位置の記録(自己診断で「はみ出し」「下のバー」を調べるため)
#[derive(Clone, Debug)]
pub struct Placed {
    pub name: String,
    /// 部品の矩形
    pub rect: Rect,
    /// 入っているべき枠(ウィンドウや LabelFrame)
    pub frame: Rect,
}

/// タブ(ttk.Notebook のタブ)。選んでいるものは太字・挿し色の縁・少し大きく。
/// 押されたら Some(そのタブ)。
pub fn tab_strip<T: Copy + PartialEq>(ui: &mut Ui, th: &Theme, current: T, tabs: &[(T, &str)], big: bool) -> Option<T> {
    let (pad_x, pad_y) = if big { (22.0, 8.0) } else { (12.0, 5.0) };
    let size = pt(10.0);
    let mut clicked = None;
    let start = ui.cursor().min;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(3.0);
        for (t, label) in tabs {
            let sel = *t == current;
            let font = if sel { fonts::bold(size) } else { FontId::proportional(size) };
            let color = if sel { th.tab_sel_fg } else { th.tab_fg };
            let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
            let want = vec2(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
            let (rect, resp) = ui.allocate_exact_size(want, Sense::click());
            // 選んでいるタブは上へ 2 px 広げて、下の線を消す
            let r = if sel { Rect::from_min_max(rect.min - vec2(2.0, 2.0), rect.max + vec2(2.0, 1.0)) } else { rect };
            let fill = if sel {
                th.tab_sel_bg
            } else if resp.hovered() {
                th.btn_hi
            } else {
                th.tab_bg
            };
            let stroke = if sel { Stroke::new(1.0, th.tab_line) } else { Stroke::new(1.0, th.border) };
            ui.painter().rect(r, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, fill, stroke, StrokeKind::Inside);
            ui.painter().galley(pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0), galley, color);
            if resp.clicked() {
                clicked = Some(*t);
            }
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    });
    // タブの下の線(選んでいるタブの下は切る)
    let y = row.response.rect.max.y;
    let x0 = start.x;
    let x1 = ui.max_rect().max.x;
    ui.painter().line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.0, th.border));
    clicked
}

/// 見出し(Head.TLabel: 11pt 太字・挿し色)
pub fn head(ui: &mut Ui, th: &Theme, text: &str) -> egui::Response {
    ui.label(RichText::new(text).font(fonts::bold(pt(11.0))).color(th.accent))
}

/// 控えめな説明(Muted.TLabel)
pub fn muted(ui: &mut Ui, th: &Theme, text: &str) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).color(th.muted)).wrap())
}

/// 目立つボタン(Run.TButton: 11pt 太字)
pub fn run_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).font(fonts::bold(pt(11.0))))
}

/// 枠(ttk.LabelFrame)。題は枠線の上に重ねて描く。中身の Ui を返す関数を受け取る。
pub fn group<R>(ui: &mut Ui, th: &Theme, title: &str, add: impl FnOnce(&mut Ui) -> R) -> (R, Rect) {
    let font = fonts::bold(pt(9.0));
    let galley = ui.painter().layout_no_wrap(format!(" {title} "), font, th.accent);
    let half = galley.size().y / 2.0;
    ui.add_space(half);
    let frame = egui::Frame::new()
        .stroke(Stroke::new(1.0, th.border))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin { left: 8, right: 8, top: (half + 4.0) as i8, bottom: 8 });
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let rect = inner.response.rect;
    // 題の下の線を背景色で消してから文字を置く
    let tpos = pos2(rect.min.x + 8.0, rect.min.y - half);
    let bg = Rect::from_min_size(tpos, galley.size());
    ui.painter().rect_filled(bg, 0.0, th.bg);
    ui.painter().galley(tpos, galley, th.accent);
    (inner.inner, rect)
}

/// 進み具合のバー。total が Some なら決まった割合、None なら行き来する帯(処理中)
pub fn progress_bar(ui: &mut Ui, th: &Theme, width: f32, value: Option<f32>, busy_since: Option<f64>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 14.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(2), th.trough, Stroke::new(1.0, th.border), StrokeKind::Inside);
    let inner = rect.shrink(2.0);
    match (value, busy_since) {
        (Some(v), _) => {
            let w = inner.width() * v.clamp(0.0, 1.0);
            if w > 0.5 {
                p.rect_filled(Rect::from_min_size(inner.min, vec2(w, inner.height())), CornerRadius::same(1), th.accent);
            }
        }
        (None, Some(t0)) => {
            // ttk の indeterminate と同じく、帯が左右に行き来する
            let t = ui.input(|i| i.time) - t0;
            let seg = inner.width() * 0.25;
            let span = inner.width() - seg;
            let phase = (t * 0.8).rem_euclid(2.0) as f32;
            let x = if phase < 1.0 { phase * span } else { (2.0 - phase) * span };
            p.rect_filled(Rect::from_min_size(pos2(inner.min.x + x, inner.min.y), vec2(seg, inner.height())), CornerRadius::same(1), th.accent);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
        }
        _ => {}
    }
    resp
}

/// 表の 1 行(項目・状態)。ok なら緑、ng なら赤、None(未確認)は普通の色
pub fn table(ui: &mut Ui, th: &Theme, rows: &[(&str, Option<bool>, String)]) {
    let width = ui.available_width();
    let row_h = pt(10.0) * 1.7;
    let col0 = 150.0f32.min(width * 0.4);
    let total_h = row_h * (rows.len() as f32 + 1.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_h), Sense::hover());
    let p = ui.painter();
    p.rect(rect, 0.0, th.panel, Stroke::new(1.0, th.border), StrokeKind::Inside);
    let head = Rect::from_min_size(rect.min, vec2(width, row_h));
    p.rect_filled(head.shrink(1.0), 0.0, th.tab_bg);
    let hf = fonts::bold(pt(9.0));
    p.text(pos2(rect.min.x + 6.0, head.center().y), Align2::LEFT_CENTER, "項目", hf.clone(), th.ink);
    p.text(pos2(rect.min.x + col0 + 6.0, head.center().y), Align2::LEFT_CENTER, "状態", hf, th.ink);
    p.line_segment([pos2(rect.min.x + col0, rect.min.y + 2.0), pos2(rect.min.x + col0, head.max.y - 2.0)], Stroke::new(1.0, th.border));
    let f = FontId::proportional(pt(10.0));
    for (i, (name, ok, text)) in rows.iter().enumerate() {
        let y = rect.min.y + row_h * (i as f32 + 1.5);
        let color = match ok {
            Some(true) => th.ok,
            Some(false) => th.ng,
            None => th.ink,
        };
        p.text(pos2(rect.min.x + 6.0, y), Align2::LEFT_CENTER, *name, f.clone(), color);
        let clip = Rect::from_min_max(pos2(rect.min.x + col0 + 4.0, y - row_h / 2.0), pos2(rect.max.x - 2.0, y + row_h / 2.0));
        ui.painter_at(clip).text(pos2(rect.min.x + col0 + 6.0, y), Align2::LEFT_CENTER, text, f.clone(), color);
    }
}

/// スクロールできる欄。つまみは ttk と同じく枠線の色で、溝は trough の色で描く。
///
/// egui のつまみは部品の地の色(bg_fill)で塗るが、その色はチェックの箱の地と同じなので、
/// スクロールの欄の外側でだけ色を差し替え、中身では元に戻す。
pub fn scroll<R>(ui: &mut Ui, th: &Theme, area: egui::ScrollArea, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        {
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.bg_fill = th.border;
            w.hovered.bg_fill = th.muted;
            w.active.bg_fill = th.accent;
        }
        area.show(ui, |ui| {
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.bg_fill = th.entry;
            w.hovered.bg_fill = th.entry;
            w.active.bg_fill = th.entry;
            add(ui)
        })
        .inner
    })
    .inner
}

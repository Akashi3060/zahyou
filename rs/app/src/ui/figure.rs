//! 図を画面に描く(v6 の FigureCanvasTkAgg + ツールバーに当たる)。
//!
//! - 画像はテクスチャ。縮小の段(scene::Pyramid)から表示の倍率に合うものを選び、
//!   GPU の上限(既定 8192 px)を超える段はタイルに分ける
//! - 座標格子・目盛り・印・矢印・文字は毎回 Painter で描く(拡大しても線が太らず、文字もにじまない)
//! - 拡大・移動したら、格子と目盛りを今の表示の範囲で作り直す(WCSAxes と同じ。作り直す関数は zahyou-plot)
//! - 操作: 全体・戻る・進む・移動・拡大(matplotlib のツールバーと同じ 5 つ)。ホイールでも拡大縮小できる
//! - 大きさ(線・印・文字)は pt。v6 は図を dpi 100 で貼っていたので、1 pt = 100/72 px で描く

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, TextureHandle,
    TextureOptions, Ui, Vec2, pos2, vec2,
};

use super::fonts;
use super::scene::{GridSet, HAlign, Prim, Scene, Side, VAlign};
use super::theme::Theme;

/// 1 pt の画面の大きさ(v6 の図は dpi 100)
pub const PT: f32 = 100.0 / 72.0;
/// 題・目盛りの文字・軸名の大きさ(pt。matplotlib の既定と Python 版の設定)
const TITLE_PT: f32 = 14.0;
const TICK_LABEL_PT: f32 = 10.0;
const AXIS_LABEL_PT: f32 = 10.0;
const TICK_LEN_PT: f32 = 3.5;
const TICK_PAD_PT: f32 = 3.5;
const TICK_WIDTH_PT: f32 = 0.8;
const FRAME_WIDTH_PT: f32 = 0.8;
const TITLE_PAD_PT: f32 = 6.0;
const TOOLBAR_H: f32 = 32.0;

/// 表示の範囲(画素。x0 < x1、y0 < y1)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
}

impl View {
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    fn arr(&self) -> [f64; 4] {
        [self.x0, self.x1, self.y0, self.y1]
    }
}

/// 移動・拡大のどちらの操作か(matplotlib のツールバーの「移動」「拡大」)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    None,
    Pan,
    Zoom,
}

/// 図の欄から親へ返す用事
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FigAction {
    Save,
}

/// 1 段ぶんのテクスチャ(タイルに分けたもの)
struct LevelTex {
    /// (テクスチャ, 段の画素での範囲 [x0, y0, x1, y1])
    tiles: Vec<(TextureHandle, [usize; 4])>,
}

/// 図 1 枚の表示の状態
pub struct FigureView {
    pub scene: Arc<Scene>,
    levels: Vec<Option<LevelTex>>,
    pub view: View,
    home: View,
    history: Vec<View>,
    pos: usize,
    pub tool: Tool,
    zoom_start: Option<Pos2>,
    hover: Option<[f64; 2]>,
    dragging: bool,
    /// 拡大した表示のために作り直した格子(その表示の範囲と一緒に)
    grid_cache: Option<(View, Arc<GridSet>)>,
    /// 識別子(テクスチャの名前と egui の Id)
    key: String,
}

/// 画面の点 = 画素の点の写し
#[derive(Clone, Copy)]
struct Map {
    v: View,
    ax: Rect,
    flip: bool,
}

impl Map {
    fn to_screen(self, p: [f64; 2]) -> Pos2 {
        to_screen(p, self.v, self.ax, self.flip)
    }
}

impl FigureView {
    pub fn new(scene: Arc<Scene>, key: &str) -> FigureView {
        let home = View { x0: -0.5, x1: scene.width - 0.5, y0: -0.5, y1: scene.height - 0.5 };
        let n = scene.image.as_ref().map(|p| p.levels.len()).unwrap_or(0);
        FigureView {
            scene,
            levels: (0..n).map(|_| None).collect(),
            view: home,
            home,
            history: vec![home],
            pos: 0,
            tool: Tool::None,
            zoom_start: None,
            hover: None,
            dragging: false,
            grid_cache: None,
            key: key.to_string(),
        }
    }

    // ------------------------------------------------------------ 履歴 ---
    fn push(&mut self, v: View) {
        self.history.truncate(self.pos + 1);
        if self.history.last() != Some(&v) {
            self.history.push(v);
        }
        self.pos = self.history.len() - 1;
        self.view = v;
    }
    /// 表示の範囲を決める(画面の撮影で拡大した表示を確かめるため)
    pub fn set_view(&mut self, x0: f64, x1: f64, y0: f64, y1: f64) {
        if x0 < x1 && y0 < y1 {
            self.push(View { x0, x1, y0, y1 });
        }
    }
    pub fn home(&mut self) {
        let h = self.home;
        if self.view != h {
            self.push(h);
        }
    }
    /// 拡大・移動しているときだけ、その範囲 [x0, x1, y0, y1](画像の座標)を返す(「画像を保存」に渡す)
    pub fn zoomed_view(&self) -> Option<[f64; 4]> {
        (self.view != self.home).then(|| self.view.arr())
    }
    pub fn back(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            self.view = self.history[self.pos];
        }
    }
    pub fn forward(&mut self) {
        if self.pos + 1 < self.history.len() {
            self.pos += 1;
            self.view = self.history[self.pos];
        }
    }

    /// 今の表示の範囲の格子(全体なら図のもの、拡大していれば作り直したもの)
    fn grid(&mut self) -> Option<Arc<GridSet>> {
        if self.view == self.home {
            return None; // 全体表示は scene.home をそのまま使う
        }
        if let Some((v, g)) = &self.grid_cache
            && *v == self.view
        {
            return Some(g.clone());
        }
        if self.dragging {
            // ドラッグの最中は作り直さない(前の格子のまま。文字は出さない)
            return self.grid_cache.as_ref().map(|(_, g)| Arc::new(GridSet { lines: g.lines.clone(), ..Default::default() }));
        }
        let g = self.scene.regrid.as_ref().and_then(|f| f(self.view.arr()));
        let g = Arc::new(g.unwrap_or_else(|| GridSet { lines: self.scene.home.lines.clone(), ..Default::default() }));
        self.grid_cache = Some((self.view, g.clone()));
        Some(g)
    }

    // ------------------------------------------------------------ 描く ---
    /// 図とツールバーを描く。ツールバーで「画像を保存」が押されたら Some(Save)。
    pub fn show(&mut self, ui: &mut Ui, th: &Theme) -> Option<FigAction> {
        let full = ui.available_rect_before_wrap();
        let fig_rect = Rect::from_min_max(full.min, pos2(full.max.x, (full.max.y - TOOLBAR_H).max(full.min.y)));
        let bar_rect = Rect::from_min_max(pos2(full.min.x, fig_rect.max.y), full.max);
        ui.allocate_rect(full, Sense::hover());
        self.paint(ui, fig_rect, th);
        let mut action = None;
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar_rect.shrink2(vec2(2.0, 3.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if bar.button("全体").clicked() {
            self.home();
        }
        if bar.add_enabled(self.pos > 0, egui::Button::new("戻る")).clicked() {
            self.back();
        }
        if bar.add_enabled(self.pos + 1 < self.history.len(), egui::Button::new("進む")).clicked() {
            self.forward();
        }
        bar.add_space(8.0);
        if bar.add(egui::Button::new("移動").selected(self.tool == Tool::Pan)).clicked() {
            self.tool = if self.tool == Tool::Pan { Tool::None } else { Tool::Pan };
        }
        if bar.add(egui::Button::new("拡大").selected(self.tool == Tool::Zoom)).clicked() {
            self.tool = if self.tool == Tool::Zoom { Tool::None } else { Tool::Zoom };
        }
        bar.add_space(8.0);
        if bar.button("画像を保存").clicked() {
            action = Some(FigAction::Save);
        }
        if let Some([x, y]) = self.hover {
            bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("x={x:.1}  y={y:.1}")).color(th.muted).size(fonts::pt(9.0)));
            });
        }
        action
    }

    /// 図の外枠の中に、題・軸・画像・格子・印を描く(操作もここで受ける)。
    pub fn paint(&mut self, ui: &mut Ui, rect: Rect, th: &Theme) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, th.fig_bg);
        let fg = th.fig_fg;
        let scene = self.scene.clone();

        // --- 余白(題・目盛り・軸名)。全体表示の目盛りの文字で決める(拡大で枠が揺れないように) ---
        let tick_font = FontId::proportional(TICK_LABEL_PT * PT);
        let lab_font = FontId::proportional(AXIS_LABEL_PT * PT);
        let mut ext = [0.0f32; 4]; // 下・右・上・左
        for l in &scene.home.labels {
            let g = painter.layout_no_wrap(l.plain(), tick_font.clone(), fg);
            let i = side_index(l.side);
            let e = if matches!(l.side, Side::Bottom | Side::Top) { g.size().y } else { g.size().x };
            ext[i] = ext[i].max(e + (TICK_LEN_PT + TICK_PAD_PT) * PT);
        }
        let lab_h = AXIS_LABEL_PT * PT * 1.3;
        for a in &scene.axis_labels {
            ext[side_index(a.side)] += lab_h + 4.0 * PT;
        }
        let title_lines = scene.title.lines().count().max(1) as f32;
        let title_h = title_lines * TITLE_PT * PT * 1.25 + TITLE_PAD_PT * PT;
        let pad = 10.0;
        let avail = Rect::from_min_max(
            pos2(rect.min.x + pad + ext[3], rect.min.y + pad + title_h + ext[2]),
            pos2(rect.max.x - pad - ext[1].max(12.0), rect.max.y - pad - ext[0]),
        );
        if avail.width() < 20.0 || avail.height() < 20.0 {
            return;
        }
        // 画素は正方形のまま(aspect = equal)。入るように縮めて真ん中へ
        let v = self.view;
        let aspect = (v.h() / v.w()) as f32;
        let (mut aw, mut ah) = (avail.width(), avail.width() * aspect);
        if ah > avail.height() {
            ah = avail.height();
            aw = ah / aspect;
        }
        let ax = Rect::from_center_size(avail.center(), vec2(aw, ah));

        // --- 操作 -----------------------------------------------------------
        let id = ui.id().with(("fig", &self.key));
        let resp = ui.interact(ax, id, Sense::click_and_drag());
        self.handle_input(ui, &resp, ax);
        let v = self.view;
        let map = Map { v, ax, flip: scene.flip_y };

        // --- 画像 -----------------------------------------------------------
        let ap = ui.painter_at(ax);
        ap.rect_filled(ax, 0.0, th.ax_bg);
        self.paint_image(ui, &ap, ax, v);

        // --- 座標格子・印 -----------------------------------------------------
        let regrid = self.grid();
        let grid: &GridSet = regrid.as_deref().unwrap_or(&scene.home);
        for p in &grid.lines {
            paint_prim(&ap, p, &map);
        }
        for p in &scene.prims {
            paint_prim(&ap, p, &map);
        }

        // --- 拡大の四角(ドラッグ中) -------------------------------------------
        if let (Tool::Zoom, Some(start), Some(cur)) = (self.tool, self.zoom_start, resp.interact_pointer_pos()) {
            ap.rect_stroke(Rect::from_two_pos(start, cur), 0.0, Stroke::new(1.0, fg), egui::StrokeKind::Inside);
        }

        // --- 枠・目盛り・目盛りの文字 -------------------------------------------
        painter.rect_stroke(ax, 0.0, Stroke::new(FRAME_WIDTH_PT * PT, fg), egui::StrokeKind::Middle);
        let tick_stroke = Stroke::new(TICK_WIDTH_PT * PT, fg);
        for t in &grid.ticks {
            let q = map.to_screen(t.pos);
            let a = (t.angle_deg as f32).to_radians();
            // 等値線が内側へ向かう向き(画面の y 上)の逆 = 外向き。egui の y は下向き
            let out = vec2(-a.cos(), a.sin());
            painter.line_segment([q, q + out * TICK_LEN_PT * PT], tick_stroke);
        }
        let gap = (TICK_LEN_PT + TICK_PAD_PT) * PT;
        for l in &grid.labels {
            let q = map.to_screen(l.pos);
            let (anchor, at) = match l.side {
                Side::Bottom => (Align2::CENTER_TOP, q + vec2(0.0, gap)),
                Side::Top => (Align2::CENTER_BOTTOM, q - vec2(0.0, gap)),
                Side::Left => (Align2::RIGHT_CENTER, q - vec2(gap, 0.0)),
                Side::Right => (Align2::LEFT_CENTER, q + vec2(gap, 0.0)),
            };
            if !rect.contains(at) {
                continue;
            }
            let galley = spans_galley(ui, &l.spans, TICK_LABEL_PT * PT, fg);
            let r = anchor.anchor_size(at, galley.size());
            painter.galley(r.min, galley, fg);
        }

        // --- 軸名・題 -------------------------------------------------------------
        for a in &scene.axis_labels {
            let galley = painter.layout_no_wrap(a.text.clone(), lab_font.clone(), fg);
            let sz = galley.size();
            match a.side {
                Side::Bottom => {
                    let y = ax.max.y + ext[0] - lab_h;
                    painter.galley(pos2(ax.center().x - sz.x / 2.0, y), galley, fg);
                }
                Side::Top => {
                    let y = ax.min.y - ext[2];
                    painter.galley(pos2(ax.center().x - sz.x / 2.0, y), galley, fg);
                }
                Side::Left => {
                    let x = ax.min.x - ext[3];
                    let shape = egui::epaint::TextShape::new(pos2(x, ax.center().y + sz.x / 2.0), galley, fg)
                        .with_angle(-std::f32::consts::FRAC_PI_2);
                    painter.add(shape);
                }
                Side::Right => {
                    let x = ax.max.x + ext[1];
                    let shape = egui::epaint::TextShape::new(pos2(x, ax.center().y - sz.x / 2.0), galley, fg)
                        .with_angle(std::f32::consts::FRAC_PI_2);
                    painter.add(shape);
                }
            }
        }
        let tf = FontId::proportional(TITLE_PT * PT);
        let lines: Vec<&str> = scene.title.lines().collect();
        let mut ty = ax.min.y - ext[2] - TITLE_PAD_PT * PT;
        for line in lines.iter().rev() {
            painter.text(pos2(ax.center().x, ty), Align2::CENTER_BOTTOM, *line, tf.clone(), fg);
            ty -= TITLE_PT * PT * 1.25;
        }
    }

    fn handle_input(&mut self, ui: &Ui, resp: &egui::Response, ax: Rect) {
        let v = self.view;
        let flip = self.scene.flip_y;
        // 画面の位置 → 画素
        let inv = |p: Pos2| -> [f64; 2] {
            let fx = ((p.x - ax.min.x) / ax.width()) as f64;
            let fy = ((p.y - ax.min.y) / ax.height()) as f64;
            let x = v.x0 + fx * v.w();
            let y = if flip { v.y0 + fy * v.h() } else { v.y1 - fy * v.h() };
            [x, y]
        };
        self.hover = resp.hover_pos().map(inv);

        // ホイールで拡大縮小(ポインターの位置を中心に)
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let mut factor = 1.0f64;
            if zoom != 1.0 {
                factor /= zoom as f64;
            } else if scroll != 0.0 {
                factor = (-(scroll as f64) * 0.002).exp();
            }
            if factor != 1.0
                && let Some(p) = resp.hover_pos()
            {
                let c = inv(p);
                let nv = View {
                    x0: c[0] - (c[0] - v.x0) * factor,
                    x1: c[0] + (v.x1 - c[0]) * factor,
                    y0: c[1] - (c[1] - v.y0) * factor,
                    y1: c[1] + (v.y1 - c[1]) * factor,
                };
                if nv.w() > 2.0 && nv.w() < self.home.w() * 50.0 {
                    self.view = nv;
                    self.history.truncate(self.pos + 1);
                    ui.ctx().request_repaint();
                }
            }
        }
        // ホイールは区切りが無いので、ポインターが図から出たら履歴に積む
        if !resp.hovered() && !resp.dragged() && self.history.get(self.pos) != Some(&self.view) {
            let cur = self.view;
            self.push(cur);
        }

        self.dragging = resp.dragged();
        match self.tool {
            Tool::Zoom => {
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                }
                self.dragging = false; // 四角を描いている間も格子はそのまま
                if resp.drag_started() {
                    self.zoom_start = resp.interact_pointer_pos();
                }
                if resp.drag_stopped() {
                    if let (Some(a), Some(b)) = (self.zoom_start, resp.interact_pointer_pos()) {
                        let r = Rect::from_two_pos(a, b);
                        if r.width() > 5.0 && r.height() > 5.0 {
                            let p0 = inv(r.min);
                            let p1 = inv(r.max);
                            let (x0, x1) = (p0[0].min(p1[0]), p0[0].max(p1[0]));
                            let (y0, y1) = (p0[1].min(p1[1]), p0[1].max(p1[1]));
                            // 枠の縦横比に合わせて広げる(画素は正方形のまま)
                            let want = v.h() / v.w();
                            let (mut w, mut h) = (x1 - x0, y1 - y0);
                            if h / w < want {
                                h = w * want;
                            } else {
                                w = h / want;
                            }
                            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                            self.push(View { x0: cx - w / 2.0, x1: cx + w / 2.0, y0: cy - h / 2.0, y1: cy + h / 2.0 });
                        }
                    }
                    self.zoom_start = None;
                }
            }
            Tool::Pan | Tool::None => {
                if resp.hovered() && self.tool == Tool::Pan {
                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                }
                if resp.dragged() {
                    let d = resp.drag_delta();
                    let dx = -(d.x / ax.width()) as f64 * v.w();
                    let dy = (d.y / ax.height()) as f64 * v.h() * if flip { -1.0 } else { 1.0 };
                    self.view = View { x0: v.x0 + dx, x1: v.x1 + dx, y0: v.y0 + dy, y1: v.y1 + dy };
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                if resp.drag_stopped() {
                    let cur = self.view;
                    self.view = self.history[self.pos];
                    self.push(cur);
                    ui.ctx().request_repaint();
                }
            }
        }
        if resp.double_clicked() {
            self.home();
        }
    }

    /// 画像を描く(表示の倍率に合う段を選び、見えているタイルだけ)。
    fn paint_image(&mut self, ui: &Ui, p: &egui::Painter, ax: Rect, v: View) {
        let Some(pyr) = self.scene.image.as_ref() else {
            return;
        };
        let img_per_px = (v.w() / ax.width() as f64 / ui.ctx().pixels_per_point() as f64) as f32;
        let k = pyr.level_for(img_per_px);
        if self.levels.get(k).is_some_and(|l| l.is_none()) {
            let tex = upload_level(ui.ctx(), &pyr.levels[k], &format!("{}-{k}", self.key));
            self.levels[k] = Some(tex);
        }
        let Some(Some(level)) = self.levels.get(k) else {
            return;
        };
        let f = (1usize << k) as f64;
        let flip = self.scene.flip_y;
        for (tex, [tx0, ty0, tx1, ty1]) in &level.tiles {
            // 段の画素 (i, j) は原寸の [i*f - 0.5, (i+1)*f - 0.5] を覆う
            let x_lo = *tx0 as f64 * f - 0.5;
            let x_hi = (*tx1 as f64 * f - 0.5).min(pyr.width as f64 - 0.5);
            let y_lo = *ty0 as f64 * f - 0.5;
            let y_hi = (*ty1 as f64 * f - 0.5).min(pyr.height as f64 - 0.5);
            if x_hi < v.x0 || x_lo > v.x1 || y_hi < v.y0 || y_lo > v.y1 {
                continue;
            }
            // 原寸の端で切ったぶん、テクスチャの uv も詰める
            let u1 = ((x_hi + 0.5) / f - *tx0 as f64) / (*tx1 - *tx0) as f64;
            let v1 = ((y_hi + 0.5) / f - *ty0 as f64) / (*ty1 - *ty0) as f64;
            let c00 = to_screen([x_lo, y_lo], v, ax, flip);
            let c10 = to_screen([x_hi, y_lo], v, ax, flip);
            let c01 = to_screen([x_lo, y_hi], v, ax, flip);
            let c11 = to_screen([x_hi, y_hi], v, ax, flip);
            let mut mesh = Mesh::with_texture(tex.id());
            let w = Color32::WHITE;
            mesh.vertices.push(egui::epaint::Vertex { pos: c00, uv: pos2(0.0, 0.0), color: w });
            mesh.vertices.push(egui::epaint::Vertex { pos: c10, uv: pos2(u1 as f32, 0.0), color: w });
            mesh.vertices.push(egui::epaint::Vertex { pos: c01, uv: pos2(0.0, v1 as f32), color: w });
            mesh.vertices.push(egui::epaint::Vertex { pos: c11, uv: pos2(u1 as f32, v1 as f32), color: w });
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(1, 3, 2);
            p.add(Shape::mesh(mesh));
        }
    }
}

fn side_index(s: Side) -> usize {
    match s {
        Side::Bottom => 0,
        Side::Right => 1,
        Side::Top => 2,
        Side::Left => 3,
    }
}

/// 上付きを含む文字(赤経の h m s は小さく上に)
fn spans_galley(ui: &Ui, spans: &[(String, bool)], size: f32, color: Color32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    for (t, sup) in spans {
        let fmt = if *sup {
            egui::TextFormat { font_id: FontId::proportional(size * 0.7), color, valign: Align::TOP, ..Default::default() }
        } else {
            egui::TextFormat { font_id: FontId::proportional(size), color, ..Default::default() }
        };
        job.append(t, 0.0, fmt);
    }
    ui.fonts_mut(|f| f.layout_job(job))
}

/// 描く物 1 つ
fn paint_prim(p: &egui::Painter, prim: &Prim, m: &Map) {
    match prim {
        Prim::Polyline { pts, color, width, dash } => {
            let s: Vec<Pos2> = pts.iter().map(|q| m.to_screen(*q)).collect();
            if s.len() < 2 {
                return;
            }
            let st = Stroke::new(width * PT, *color);
            match dash {
                Some([on, off]) if *on > 0.0 && *off > 0.0 => {
                    p.extend(Shape::dashed_line(&s, st, on * PT, off * PT));
                }
                _ => {
                    p.add(Shape::line(s, st));
                }
            }
        }
        Prim::Circle { c, diameter, color, width } => {
            p.circle_stroke(m.to_screen(*c), diameter * PT / 2.0, Stroke::new(width * PT, *color));
        }
        Prim::Cross { c, size, color, width } => {
            let q = m.to_screen(*c);
            let h = size * PT / 2.0;
            let st = Stroke::new(width * PT, *color);
            p.line_segment([q + vec2(-h, -h), q + vec2(h, h)], st);
            p.line_segment([q + vec2(-h, h), q + vec2(h, -h)], st);
        }
        Prim::Arrow { from, to, color, tail, head_w, head_l, edge } => {
            paint_arrow(p, m.to_screen(*from), m.to_screen(*to), *color, tail * PT, head_w * PT, head_l * PT, edge * PT);
        }
        Prim::Text { at, offset, text, size, color, bold, halign, valign } => {
            let q = m.to_screen(*at) + vec2(offset[0] * PT, -offset[1] * PT);
            let font = if *bold { fonts::bold(size * PT) } else { FontId::proportional(size * PT) };
            let h = match halign {
                HAlign::Left => Align::LEFT,
                HAlign::Center => Align::Center,
                HAlign::Right => Align::RIGHT,
            };
            let v = match valign {
                VAlign::Top => Align::TOP,
                VAlign::Center => Align::Center,
                // 基準線は下端で近似する(英字なら差は数 px)
                VAlign::Baseline | VAlign::Bottom => Align::BOTTOM,
            };
            p.text(q, Align2([h, v]), text, font, *color);
        }
    }
}

/// 画素 → 画面
fn to_screen(p: [f64; 2], v: View, ax: Rect, flip: bool) -> Pos2 {
    let fx = (p[0] - v.x0) / v.w();
    let fy = (p[1] - v.y0) / v.h();
    let x = ax.min.x + (fx as f32) * ax.width();
    let y = if flip { ax.min.y + (fy as f32) * ax.height() } else { ax.max.y - (fy as f32) * ax.height() };
    pos2(x, y)
}

/// 矢印(matplotlib の annotate の "simple": 軸は細い帯、矢じりは三角。縁も同じ色)
#[allow(clippy::too_many_arguments)]
fn paint_arrow(p: &egui::Painter, from: Pos2, to: Pos2, color: Color32, tail: f32, head_w: f32, head_l: f32, edge: f32) {
    let d = to - from;
    let len = d.length();
    if len.is_nan() || len <= 1.0 {
        return;
    }
    let u = d / len;
    let n = Vec2::new(-u.y, u.x);
    let hl = head_l.min(len);
    let base = to - u * hl;
    let st = if edge > 0.0 { Stroke::new(edge, color) } else { Stroke::NONE };
    if len > hl {
        let t = tail / 2.0;
        p.add(Shape::convex_polygon(vec![from + n * t, base + n * t, base - n * t, from - n * t], color, st));
    }
    p.add(Shape::convex_polygon(vec![to, base + n * (head_w / 2.0), base - n * (head_w / 2.0)], color, st));
}

/// 段 1 つをテクスチャに上げる(GPU の上限を超えればタイルに分ける)。
fn upload_level(ctx: &egui::Context, img: &egui::ColorImage, name: &str) -> LevelTex {
    let max_side = ctx.input(|i| i.max_texture_side).clamp(512, 16384);
    let tile = max_side.min(4096);
    let [w, h] = img.size;
    let opts = TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    };
    let mut tiles = Vec::new();
    if w <= max_side && h <= max_side {
        let t = ctx.load_texture(name, img.clone(), opts);
        tiles.push((t, [0, 0, w, h]));
        return LevelTex { tiles };
    }
    let mut y = 0;
    while y < h {
        let y1 = (y + tile).min(h);
        let mut x = 0;
        while x < w {
            let x1 = (x + tile).min(w);
            let mut px = Vec::with_capacity((x1 - x) * (y1 - y));
            for row in y..y1 {
                px.extend_from_slice(&img.pixels[row * w + x..row * w + x1]);
            }
            let part = egui::ColorImage::new([x1 - x, y1 - y], px);
            let t = ctx.load_texture(format!("{name}-{x}-{y}"), part, opts);
            tiles.push((t, [x, y, x1, y1]));
            x = x1;
        }
        y = y1;
    }
    LevelTex { tiles }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 画面への写し方() {
        let v = View { x0: -0.5, x1: 99.5, y0: -0.5, y1: 49.5 };
        let ax = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0));
        // 上下を反転しないとき、y=-0.5 は画面の下
        assert_eq!(to_screen([-0.5, -0.5], v, ax, false), pos2(0.0, 100.0));
        // 反転するとき、y=-0.5 は画面の上
        assert_eq!(to_screen([-0.5, -0.5], v, ax, true), pos2(0.0, 0.0));
    }

    #[test]
    fn 履歴() {
        let mut s = Scene::empty();
        s.width = 100.0;
        s.height = 50.0;
        let mut f = FigureView::new(Arc::new(s), "t");
        let z = View { x0: 0.0, x1: 10.0, y0: 0.0, y1: 5.0 };
        f.push(z);
        assert_eq!(f.view, z);
        f.back();
        assert_eq!(f.view, f.home);
        f.forward();
        assert_eq!(f.view, z);
        f.home();
        assert_eq!(f.view, f.home);
        assert_eq!(f.history.len(), 3);
    }

    #[test]
    fn 大きな画像はタイルに分ける() {
        let ctx = egui::Context::default();
        // 既定の上限(2048 などの小さな値)より大きい画像
        let img = egui::ColorImage::new([5000, 300], vec![Color32::GRAY; 5000 * 300]);
        let _ = ctx.run(egui::RawInput { max_texture_side: Some(2048), ..Default::default() }, |_| {});
        let t = upload_level(&ctx, &img, "t");
        assert_eq!(t.tiles.len(), 3);
        assert_eq!(t.tiles[2].1, [4096, 0, 5000, 300]);
    }
}

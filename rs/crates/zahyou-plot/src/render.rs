//! 図を PNG にする(tiny-skia で表示リストを描き、文字は ab_glyph で游ゴシックを塗る)。
//!
//! 大きさと配置は Python 版の図(matplotlib + WCSAxes、`plt.tight_layout()` 1 回)に合わせる
//! (`rs/testdata/plot_layout.json` と比べて、軸の矩形の差は 1 px 未満):
//! - 既定の大きさは Python 版と同じ 12 × (12·ny/nx + 1.2) インチ、100 dpi(幅 1200 px)。1 pt = dpi/72 px
//! - 余白は tight_layout と同じく 1.08 × 10 pt + 枠からはみ出す文字(`plan`)。画像は縦横比を保って
//!   残りの領域の中央に置く(imshow の aspect="equal")
//! - 文字の置き方は WCSAxes の RectangularFrame・AxisLabels と matplotlib の題と同じ(`decorations`)。
//!   文字の箱の大きさは matplotlib(Agg)と同じ決め方(font.rs の `Line`)
//! - 画像は表示の大きさへ面積平均で縮める(拡大のときも同じ式で、端だけなめらかになる)
//! - 意図して変えた所: 上の辺に目盛りの文字がある図(極のそば)では、題をその上へ逃がす
//!   (Python 版は題が目盛りの文字・軸名に重なって読めない)

use rayon::prelude::*;
use tiny_skia::{FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

use crate::PlotError;
use crate::axes::{AxisLabel, Grid, Side, TextSpan, Tick, TickLabel, View, grid_for_view};
use crate::figure::{Color, Figure, HAlign, Prim, Theme, VAlign, style};
use crate::font::{Coverage, Fonts, Line, SUP_SCALE, layout, rasterize};

/// PNG の書き出しの設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// 幅(px)。0 なら 12 インチ分(既定の dpi で 1200)
    pub width: u32,
    /// 高さ(px)。0 なら Python 版と同じく 幅 × ny/nx + 1.2 インチ
    pub height: u32,
    /// 1 インチの px 数(pt → px の換算。既定 100)
    pub dpi: f32,
    pub theme: Theme,
    /// 表示の範囲 [x の最小, x の最大, y の最小, y の最大](画像の座標。画素の中心が整数で、画像全体は
    /// [−0.5, 幅 − 0.5, −0.5, 高さ − 0.5])。None なら画像全体。拡大・移動した表示を保存するときに使う
    /// (matplotlib のツールバーの「保存」と同じく、格子・目盛り・文字はこの範囲で選び直す。上下の向きは図の flip_y)
    pub view: Option<[f64; 4]>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { width: 0, height: 0, dpi: 100.0, theme: Theme::light(), view: None }
    }
}

/// 描いた図(straight alpha の RGBA、行優先、行 0 が上)。
#[derive(Clone, Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// 図の中の軸の矩形と、画像の座標 → 画面(px、左上が原点・y 下向き)の写し。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxesBox {
    /// 軸の左上(px)
    pub x0: f64,
    pub y0: f64,
    /// 画像 1 画素あたりの px
    pub scale: f64,
    /// 画像の大きさ(画素)
    pub width: usize,
    pub height: usize,
    pub flip_y: bool,
    /// 表示している範囲の x(軸の左端, 右端)(画像の座標)
    pub vx: (f64, f64),
    /// 表示している範囲の y(軸の下端, 上端)(画像の座標。上下反転なら vy.0 > vy.1)
    pub vy: (f64, f64),
}

impl AxesBox {
    /// 画像全体を表示する軸(x は −0.5〜幅 − 0.5、y は flip_y なら上が −0.5)。
    pub fn full(x0: f64, y0: f64, scale: f64, width: usize, height: usize, flip_y: bool) -> AxesBox {
        let (vx, vy) = full_view(width, height, flip_y);
        AxesBox { x0, y0, scale, width, height, flip_y, vx, vy }
    }

    /// 軸の幅・高さ(px)。
    pub fn size(&self) -> (f64, f64) {
        ((self.vx.1 - self.vx.0).abs() * self.scale, (self.vy.1 - self.vy.0).abs() * self.scale)
    }

    fn signs(&self) -> (f64, f64) {
        (if self.vx.1 >= self.vx.0 { 1.0 } else { -1.0 }, if self.vy.1 >= self.vy.0 { 1.0 } else { -1.0 })
    }

    /// 画像の座標 → 画面の px。
    pub fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let (gx, gy) = self.signs();
        (self.x0 + (x - self.vx.0) * self.scale * gx, self.y0 + (self.vy.1 - y) * self.scale * gy)
    }

    /// 画面の px → 画像の座標。
    pub fn to_image(&self, sx: f64, sy: f64) -> (f64, f64) {
        let (gx, gy) = self.signs();
        (self.vx.0 + (sx - self.x0) / (self.scale * gx), self.vy.1 - (sy - self.y0) / (self.scale * gy))
    }
}

/// 画像全体の表示範囲 (x の左端, 右端)・(y の下端, 上端)。
fn full_view(width: usize, height: usize, flip_y: bool) -> ((f64, f64), (f64, f64)) {
    let (w, h) = (width as f64, height as f64);
    ((-0.5, w - 0.5), if flip_y { (h - 0.5, -0.5) } else { (-0.5, h - 0.5) })
}

/// 描く中身: 図の表示リストと、表示範囲(拡大・移動)に合わせて選び直した格子・目盛り・文字。
struct Content<'a> {
    fig: &'a Figure,
    /// 表示範囲で選び直した格子(None なら図の格子をそのまま使う)
    grid: Option<Grid>,
    /// 選び直した格子の線(表示リストの形)
    grid_prims: Vec<Prim>,
    vx: (f64, f64),
    vy: (f64, f64),
}

impl<'a> Content<'a> {
    fn new(fig: &'a Figure, opts: &RenderOptions) -> Content<'a> {
        let full = full_view(fig.width, fig.height, fig.flip_y);
        let Some(v) = opts.view.filter(|v| v.iter().all(|a| a.is_finite()) && v[0] < v[1] && v[2] < v[3]) else {
            return Content { fig, grid: None, grid_prims: Vec::new(), vx: full.0, vy: full.1 };
        };
        let (vx, vy) = ((v[0], v[1]), if fig.flip_y { (v[3], v[2]) } else { (v[2], v[3]) });
        if (vx, vy) == full {
            return Content { fig, grid: None, grid_prims: Vec::new(), vx, vy };
        }
        let grid = grid_for_view(&fig.wcs, fig.width, fig.height, &View::new(vx, vy));
        let gc = style::grid_color();
        let grid_prims = grid
            .lines
            .iter()
            .map(|l| Prim::Polyline {
                points: l.points.clone(),
                color: gc,
                width_pt: style::GRID_WIDTH,
                dash_pt: Some(style::GRID_DASH),
            })
            .collect();
        Content { fig, grid: Some(grid), grid_prims, vx, vy }
    }

    /// 表示する範囲の幅・高さ(画像の画素)
    fn extent(&self) -> (f64, f64) {
        ((self.vx.1 - self.vx.0).abs().max(1e-9), (self.vy.1 - self.vy.0).abs().max(1e-9))
    }

    fn ticks(&self) -> &[Tick] {
        self.grid.as_ref().map_or(&self.fig.ticks, |g| &g.ticks)
    }

    fn tick_labels(&self) -> &[TickLabel] {
        self.grid.as_ref().map_or(&self.fig.tick_labels, |g| &g.labels)
    }

    fn axis_labels(&self) -> &[AxisLabel] {
        self.grid.as_ref().map_or(&self.fig.axis_labels, |g| &g.axis_labels)
    }

    /// 描く表示リスト(格子を選び直したときは、図の格子の線を除いて選び直した線を先に)
    fn prims(&self) -> Vec<&Prim> {
        match self.grid {
            None => self.fig.prims.iter().collect(),
            Some(_) => self.grid_prims.iter().chain(self.fig.prims.iter().filter(|p| !is_grid_line(p))).collect(),
        }
    }
}

/// 図の格子の線か(figure.rs は格子を deepskyblue の破線で表示リストの先頭に入れる)
fn is_grid_line(p: &Prim) -> bool {
    matches!(p, Prim::Polyline { color, dash_pt: Some(_), .. } if *color == style::grid_color())
}

/// 図の大きさの上限(px)
const MAX_SIDE: u32 = 16384;

fn to_ts(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(to_ts(c));
    p.anti_alias = true;
    p
}

/// 文字の向き(軸名は左で 90°、右で 270° 回す)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rot {
    R0,
    /// 反時計回りに 90°(下から上へ読む)
    R90,
    /// 反時計回りに 270°(上から下へ読む)
    R270,
}

/// 文字 1 行の箱の大きさ(px): 幅と、箱の左端の原点からの位置、基準線より上・下(matplotlib の h − d と d)、
/// "lp" の字の基準線より上の高さ。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    w: f64,
    x0: f64,
    asc: f64,
    desc: f64,
    lp_asc: f64,
}

/// 組んだ 1 行。フォントが無いときは大きさの見積もりだけを持ち、描かない。
struct Text1 {
    line: Option<Line>,
    m: Metrics,
}

/// 文字を組んで大きさを測る。フォントが無いときは、配置を決められるように大きさだけ見積もる
/// (ASCII は 0.6 字幅、ほかは 1 字幅、基準線より上 0.8・下 0.25 字)。
fn measure(fonts: Option<&Fonts>, spans: &[TextSpan], size_px: f64, bold: bool) -> Text1 {
    match fonts {
        Some(f) => {
            let line = layout(f, spans, size_px as f32, bold);
            let m = Metrics {
                w: line.width as f64,
                x0: line.left as f64,
                asc: line.ascent as f64,
                desc: line.descent as f64,
                lp_asc: line.lp_ascent as f64,
            };
            Text1 { line: Some(line), m }
        }
        None => {
            let mut w = 0.0;
            for sp in spans {
                let k = if sp.superscript { SUP_SCALE as f64 } else { 1.0 };
                for c in sp.text.chars() {
                    w += size_px * k * if c.is_ascii() { 0.6 } else { 1.0 };
                }
            }
            let m = Metrics { w, x0: 0.0, asc: 0.8 * size_px, desc: 0.25 * size_px, lp_asc: 0.8 * size_px };
            Text1 { line: None, m }
        }
    }
}

/// 上付きのない文字の並び。
fn plain(text: &str) -> Vec<TextSpan> {
    vec![TextSpan { text: text.to_string(), superscript: false }]
}

/// 文字を寄せと向きに合わせて置いたときの、基準線の原点と外接矩形 [左, 上, 右, 下](画面の px、y 下向き)。
/// 寄せは回した後の外接矩形に対して(matplotlib の rotation_mode="default")。回した文字の Baseline は Center と同じ。
fn place(m: &Metrics, x: f64, y: f64, ha: HAlign, va: VAlign, rot: Rot) -> ((f64, f64), [f64; 4]) {
    let (w, x0, asc, desc) = (m.w, m.x0, m.asc, m.desc);
    match rot {
        Rot::R0 => {
            // 回す前の文字の箱: x は x0〜x0 + w、y は −asc〜desc(基準線の原点から、下が正)
            let left = match ha {
                HAlign::Left => x,
                HAlign::Center => x - w / 2.0,
                HAlign::Right => x - w,
            };
            let ox = left - x0;
            let oy = match va {
                VAlign::Baseline => y,
                VAlign::Top => y + asc,
                VAlign::Bottom => y - desc,
                VAlign::Center => y + (asc - desc) / 2.0,
            };
            ((ox, oy), [left, oy - asc, left + w, oy + desc])
        }
        Rot::R90 | Rot::R270 => {
            // 回した箱の幅は asc + desc、高さは w
            let (bw, bh) = (asc + desc, w);
            let left = match ha {
                HAlign::Left => x,
                HAlign::Center => x - bw / 2.0,
                HAlign::Right => x - bw,
            };
            let top = match va {
                VAlign::Top => y,
                VAlign::Center | VAlign::Baseline => y - bh / 2.0,
                VAlign::Bottom => y - bh,
            };
            // R90: 文字の (u, v) → 画面 (ox + v, oy − u)。R270: (u, v) → (ox − v, oy + u)(u は x0〜x0 + w)
            let origin = if rot == Rot::R90 { (left + asc, top + bh + x0) } else { (left + desc, top - x0) };
            (origin, [left, top, left + bw, top + bh])
        }
    }
}

/// 組んだ文字を、基準線の原点 `origin` から塗る。
fn draw_text(pm: &mut Pixmap, fonts: &Fonts, line: &Line, color: Color, origin: (f64, f64), rot: Rot) {
    let (ox, oy) = origin;
    let (fx, fy) = (ox.floor(), oy.floor());
    let cov = match rot {
        Rot::R0 => rasterize(fonts, line, (ox - fx) as f32, (oy - fy) as f32),
        _ => rasterize(fonts, line, 0.0, 0.0),
    };
    blit(pm, &cov, color, fx, fy, rot);
}

/// 図の文字(目盛りの文字・軸名・題)を組んだもの。位置に関係しないので 1 度だけ組む。
struct Texts {
    ticks: Vec<Text1>,
    axis: Vec<Text1>,
    title: Vec<Text1>,
}

/// 図の文字を全部組む(目盛りの文字・軸名 10 pt、題 14 pt)。
fn measure_all(c: &Content, fonts: Option<&Fonts>, ppt: f64) -> Texts {
    let fig = c.fig;
    let tick_px = style::TICK_LABEL_SIZE as f64 * ppt;
    let axis_px = style::AXIS_LABEL_SIZE as f64 * ppt;
    let title_px = style::TITLE_SIZE as f64 * ppt;
    Texts {
        ticks: c.tick_labels().iter().map(|l| measure(fonts, &l.spans(), tick_px, false)).collect(),
        axis: c.axis_labels().iter().map(|a| measure(fonts, &plain(&a.text), axis_px, false)).collect(),
        // 題が空なら行も無い(matplotlib は空の題を置かない)
        title: if fig.title.is_empty() {
            Vec::new()
        } else {
            fig.title.split('\n').map(|t| measure(fonts, &plain(t), title_px, false)).collect()
        },
    }
}

/// どの文字か(`Texts` の添字)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Tick(usize),
    Axis(usize),
    Title(usize),
}

/// 枠の外に置く文字 1 つ(基準線の原点・向き・外接矩形)。
#[derive(Clone, Copy, Debug)]
struct Deco {
    which: Which,
    origin: (f64, f64),
    rot: Rot,
    bbox: [f64; 4],
}

impl Texts {
    fn get(&self, w: Which) -> &Text1 {
        match w {
            Which::Tick(i) => &self.ticks[i],
            Which::Axis(i) => &self.axis[i],
            Which::Title(i) => &self.title[i],
        }
    }
}

fn union_box(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    }
}

/// 目盛りの文字・軸名・題を置く(WCSAxes の RectangularFrame と matplotlib の題と同じ置き方)。
///
/// - 目盛りの文字: 目盛りの点から外へ pad = 目盛りの長さ + すき間(3.5 + 3.5 pt)。WCSAxes と同じく文字の**下端**で合わせる:
///   下の辺は中央寄せで下端を点の pad + 10 pt 下に、上の辺は下端を pad 上に、左の辺は右寄せ・右の辺は左寄せで、
///   下端を点の 5 pt(文字の大きさの半分)下に
/// - 軸名: **全部の**目盛りの文字の外接矩形(astropy は座標と辺をまとめた和を使う)から 10 pt(軸名の大きさ × minpad 1)
///   外に、中心を置く。左は 90°、右は 270° 回す。目盛りの文字が無ければ枠から 10 pt
/// - 題: 最後の行の基準線を枠の上端から 6 pt 上に置き(matplotlib の va="baseline" は最後の行の基準線)、
///   前の行は「前の行の深さ + max(1.2 × "lp" の高さ, 1.2 × その行の高さ)」ずつ上へ。外接矩形の下端が枠より下になるときは
///   上へずらす(`_update_title_position`)。上の辺に目盛りの文字・軸名があるときはその上に置く
///   (Python 版は WCSAxes の上の文字を避けずに重ねてしまう。ここだけ変えてある)
fn decorations(c: &Content, texts: &Texts, bx: &AxesBox, ppt: f64) -> Vec<Deco> {
    let mut out = Vec::new();
    let ts = style::TICK_LABEL_SIZE as f64 * ppt;
    let pad = (style::TICK_LENGTH + style::TICK_PAD) as f64 * ppt;
    let (bw, bh) = bx.size();
    let (ax0, ay0, ax1, ay1) = (bx.x0, bx.y0, bx.x0 + bw, bx.y0 + bh);
    let mut all: Option<[f64; 4]> = None;
    let mut top_ref = ay0;
    for (i, l) in c.tick_labels().iter().enumerate() {
        let (x, y) = bx.to_screen(l.pos[0], l.pos[1]);
        let (tx, ty, ha) = match l.side {
            Side::Bottom => (x, y + pad + ts, HAlign::Center),
            Side::Top => (x, y - pad, HAlign::Center),
            Side::Left => (x - pad, y + ts / 2.0, HAlign::Right),
            Side::Right => (x + pad, y + ts / 2.0, HAlign::Left),
        };
        let (origin, bbox) = place(&texts.ticks[i].m, tx, ty, ha, VAlign::Bottom, Rot::R0);
        all = Some(union_box(all, bbox));
        if l.side == Side::Top {
            top_ref = top_ref.min(bbox[1]);
        }
        out.push(Deco { which: Which::Tick(i), origin, rot: Rot::R0, bbox });
    }
    let axis_px = style::AXIS_LABEL_SIZE as f64 * ppt;
    let (cx, cy) = ((ax0 + ax1) / 2.0, (ay0 + ay1) / 2.0);
    for (i, a) in c.axis_labels().iter().enumerate() {
        let (x, y, rot) = match a.side {
            Side::Bottom => (cx, all.map_or(ay1, |u| u[3]) + axis_px, Rot::R0),
            Side::Top => (cx, all.map_or(ay0, |u| u[1]) - axis_px, Rot::R0),
            Side::Left => (all.map_or(ax0, |u| u[0]) - axis_px, cy, Rot::R90),
            Side::Right => (all.map_or(ax1, |u| u[2]) + axis_px, cy, Rot::R270),
        };
        let (origin, bbox) = place(&texts.axis[i].m, x, y, HAlign::Center, VAlign::Center, rot);
        if a.side == Side::Top {
            top_ref = top_ref.min(bbox[1]);
        }
        out.push(Deco { which: Which::Axis(i), origin, rot, bbox });
    }
    let n = texts.title.len();
    if n > 0 {
        let lp_asc = texts.title[0].m.lp_asc;
        let mut base = vec![0.0f64; n];
        base[n - 1] = top_ref - style::TITLE_PAD as f64 * ppt;
        for i in (1..n).rev() {
            let (prev, cur) = (&texts.title[i - 1].m, &texts.title[i].m);
            base[i - 1] = base[i] - prev.desc - (1.2 * lp_asc).max(1.2 * cur.asc);
        }
        let low = base[n - 1] + texts.title[n - 1].m.desc;
        let shift = (low - top_ref).max(0.0);
        for (i, b) in base.iter().enumerate() {
            let (origin, bbox) = place(&texts.title[i].m, cx, b - shift, HAlign::Center, VAlign::Baseline, Rot::R0);
            out.push(Deco { which: Which::Title(i), origin, rot: Rot::R0, bbox });
        }
    }
    out
}

/// 塗りの濃さを色で重ねる(premultiplied の source-over)。
fn blit(pm: &mut Pixmap, cov: &Coverage, color: Color, fx: f64, fy: f64, rot: Rot) {
    let (pw, ph) = (pm.width() as i64, pm.height() as i64);
    let data = pm.data_mut();
    let a = color.a as f32 / 255.0;
    for j in 0..cov.h {
        for i in 0..cov.w {
            let c = cov.data[j * cov.w + i];
            if c <= 0.0 {
                continue;
            }
            // buffer の (i, j) は文字の座標 (i − ox, j − oy)
            let u = i as f64 - cov.ox as f64;
            let v = j as f64 - cov.oy as f64;
            let (sx, sy) = match rot {
                Rot::R0 => (fx + u, fy + v),
                Rot::R90 => (fx + v, fy - u - 1.0),
                Rot::R270 => (fx - v - 1.0, fy + u),
            };
            let (px, py) = (sx.round() as i64, sy.round() as i64);
            if px < 0 || py < 0 || px >= pw || py >= ph {
                continue;
            }
            blend(data, (py * pw + px) as usize, color, a * c);
        }
    }
}

#[inline]
fn blend(data: &mut [u8], idx: usize, color: Color, sa: f32) {
    let k = idx * 4;
    let inv = 1.0 - sa;
    let src = [color.r as f32 * sa, color.g as f32 * sa, color.b as f32 * sa, 255.0 * sa];
    for ch in 0..4 {
        let d = data[k + ch] as f32;
        data[k + ch] = (src[ch] + d * inv).round().clamp(0.0, 255.0) as u8;
    }
}

/// 図を RGBA に描く(文字はシステムの游ゴシック。無ければ文字なし)。
pub fn render_rgba(fig: &Figure, opts: &RenderOptions) -> Result<RgbaImage, PlotError> {
    render_rgba_with(fig, opts, Fonts::system())
}

/// フォントを指定して RGBA に描く(None なら文字を描かない)。
pub fn render_rgba_with(fig: &Figure, opts: &RenderOptions, fonts: Option<&Fonts>) -> Result<RgbaImage, PlotError> {
    let pm = render_pixmap(fig, opts, fonts)?;
    let (w, h) = (pm.width(), pm.height());
    let mut data = pm.take();
    // premultiplied → straight(地の色が不透明なので実際は変わらない)
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for v in px.iter_mut().take(3) {
                *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(RgbaImage { width: w, height: h, data })
}

/// 図を PNG のバイト列に(文字はシステムの游ゴシック)。
pub fn render_png(fig: &Figure, opts: &RenderOptions) -> Result<Vec<u8>, PlotError> {
    render_png_with(fig, opts, Fonts::system())
}

/// フォントを指定して PNG に(None なら文字を描かない)。
pub fn render_png_with(fig: &Figure, opts: &RenderOptions, fonts: Option<&Fonts>) -> Result<Vec<u8>, PlotError> {
    let pm = render_pixmap(fig, opts, fonts)?;
    pm.encode_png().map_err(|e| PlotError::Render(format!("PNG にできません: {e}")))
}

/// 図を PNG のファイルに書く。
pub fn save_png(fig: &Figure, opts: &RenderOptions, path: &std::path::Path) -> Result<(), PlotError> {
    let png = render_png(fig, opts)?;
    std::fs::write(path, png).map_err(|e| PlotError::Render(format!("{}: {e}", path.display())))
}

/// 図の大きさ(px)を決める。
pub fn figure_size(fig: &Figure, opts: &RenderOptions) -> (u32, u32) {
    let dpi = if opts.dpi.is_finite() && opts.dpi > 0.0 { opts.dpi as f64 } else { 100.0 };
    let w = if opts.width > 0 { opts.width as f64 } else { 12.0 * dpi };
    let h = if opts.height > 0 {
        opts.height as f64
    } else {
        let r = fig.height.max(1) as f64 / fig.width.max(1) as f64;
        w * r + 1.2 * dpi
    };
    (w.round().clamp(1.0, MAX_SIDE as f64) as u32, h.round().clamp(1.0, MAX_SIDE as f64) as u32)
}

/// 軸の矩形を決める(描かずに配置だけ。画面に図を出す側が使える)。
pub fn axes_box(fig: &Figure, opts: &RenderOptions) -> AxesBox {
    axes_box_with(fig, opts, Fonts::system())
}

/// フォントを指定して軸の矩形を決める(None なら文字の大きさを見積もって決める。描くときと同じ結果)。
pub fn axes_box_with(fig: &Figure, opts: &RenderOptions, fonts: Option<&Fonts>) -> AxesBox {
    layout_with(fig, opts, fonts).axes
}

/// 描いたときの配置(画面の px、左上が原点・y 下向き)。外接矩形は [左, 上, 右, 下]。
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// 図の大きさ(px)
    pub size: (u32, u32),
    /// 軸の矩形
    pub axes: AxesBox,
    /// 題の外接矩形(題が空なら None)
    pub title: Option<[f64; 4]>,
    /// 軸名の (座標 0 = 赤経・1 = 赤緯, 辺, 外接矩形)
    pub axis_labels: Vec<(usize, Side, [f64; 4])>,
    /// 目盛りの文字の外接矩形(`Figure::tick_labels` と同じ順。表示範囲を指定したときは選び直した目盛りの文字の順)
    pub tick_labels: Vec<[f64; 4]>,
}

/// 描かずに配置だけを求める(fonts が None なら文字の大きさを見積もる。描くときと同じ結果)。
pub fn layout_with(fig: &Figure, opts: &RenderOptions, fonts: Option<&Fonts>) -> Layout {
    let (w, h) = figure_size(fig, opts);
    let ppt = dpi(opts) / 72.0;
    let c = Content::new(fig, opts);
    let texts = measure_all(&c, fonts, ppt);
    let (axes, decos) = plan(&c, &texts, w, h, ppt);
    let mut out = Layout { size: (w, h), axes, title: None, axis_labels: Vec::new(), tick_labels: Vec::new() };
    for d in &decos {
        match d.which {
            Which::Tick(_) => out.tick_labels.push(d.bbox),
            Which::Axis(i) => {
                let a = &c.axis_labels()[i];
                out.axis_labels.push((a.coord, a.side, d.bbox));
            }
            Which::Title(_) => out.title = Some(union_box(out.title, d.bbox)),
        }
    }
    out
}

fn dpi(opts: &RenderOptions) -> f64 {
    if opts.dpi.is_finite() && opts.dpi > 0.0 { opts.dpi as f64 } else { 100.0 }
}

/// 余白と軸の矩形、文字の置き場所。Python 版の図(`plt.tight_layout()` を 1 回かけたもの)と同じ決め方:
///
/// - 左・右・下の余白は、目盛りの文字と軸名が枠からはみ出す長さ + pad(1.08 × 10 pt)。枠の外向きの目盛り
///   (3.5 pt)は数えない(matplotlib の tight_layout も WCSAxes の目盛りを数えない)
/// - 上の余白は、上の辺の目盛りの文字・軸名のはみ出し + pad。題が図の上端から pad の内側に収まらないときだけ広げる
///   (Python 版の tight_layout は、題を縦横比で縮める前の枠に対して測るので、ふつうは題の分を足さない)
/// - 軸の矩形は縦横比を保って、余白を除いた領域の中央に置く(imshow の aspect="equal"、anchor "C")。
///   この図はふつう横幅で大きさが決まり縦が余るので、下の余白の分だけ画像が上に寄る(Python 版と同じ)
fn plan(c: &Content, texts: &Texts, w: u32, h: u32, ppt: f64) -> (AxesBox, Vec<Deco>) {
    let fig = c.fig;
    let pad = 1.08 * 10.0 * ppt;
    let (wf, hf) = (w as f64, h as f64);
    let (nx, ny) = c.extent();
    // m は [左, 上, 右, 下] の余白(px)
    let fit = |m: [f64; 4]| {
        let aw = (wf - m[0] - m[2]).max(1.0);
        let ah = (hf - m[1] - m[3]).max(1.0);
        let s = (aw / nx).min(ah / ny);
        let (bw, bh) = (nx * s, ny * s);
        AxesBox {
            x0: m[0] + (aw - bw) / 2.0,
            y0: m[1] + (ah - bh) / 2.0,
            scale: s,
            width: fig.width,
            height: fig.height,
            flip_y: fig.flip_y,
            vx: c.vx,
            vy: c.vy,
        }
    };
    let mut m = [pad; 4];
    let mut bx = fit(m);
    let mut decos = decorations(c, texts, &bx, ppt);
    // はみ出しの長さは枠の大きさにほとんどよらないので数回で決まる。題の分の上の余白は、縦が余るときは
    // 足した長さの半分しか題が下がらないので、少しずつ足す(壊れた入力でも回数で打ち切る)
    for _ in 0..16 {
        let (bw, bh) = bx.size();
        let mut ext = [0.0f64; 4];
        let mut top = f64::INFINITY;
        for d in &decos {
            let b = d.bbox;
            top = top.min(b[1]);
            if matches!(d.which, Which::Title(_)) {
                continue;
            }
            ext[0] = ext[0].max(bx.x0 - b[0]);
            ext[1] = ext[1].max(bx.y0 - b[1]);
            ext[2] = ext[2].max(b[2] - (bx.x0 + bw));
            ext[3] = ext[3].max(b[3] - (bx.y0 + bh));
        }
        let mut m2 = [pad + ext[0], (pad + ext[1]).max(m[1]), pad + ext[2], pad + ext[3]];
        if top.is_finite() && top < pad {
            m2[1] = m[1] + (pad - top);
        }
        let changed = m2.iter().zip(&m).any(|(a, b)| (a - b).abs() > 0.01);
        if !changed || m2.iter().any(|v| !v.is_finite()) {
            break;
        }
        m = m2;
        bx = fit(m);
        decos = decorations(c, texts, &bx, ppt);
    }
    (bx, decos)
}

/// 画像を軸の矩形へ面積平均で描く(premultiplied で足し、地の上へ source-over)。表示範囲の外の画素は描かない。
fn draw_image(pm: &mut Pixmap, fig: &Figure, bx: &AxesBox) {
    let (nx, ny) = (fig.width, fig.height);
    if fig.rgba.len() != nx * ny * 4 || nx == 0 || ny == 0 {
        return;
    }
    let (pw, ph) = (pm.width() as usize, pm.height() as usize);
    let s = bx.scale;
    let (bw, bh) = bx.size();
    let xs = (bx.x0.floor().max(0.0) as usize).min(pw);
    let xe = ((bx.x0 + bw).ceil().max(0.0) as usize).min(pw);
    let ys = (bx.y0.floor().max(0.0) as usize).min(ph);
    let ye = ((bx.y0 + bh).ceil().max(0.0) as usize).min(ph);
    if xs >= xe || ys >= ye {
        return;
    }
    // 出力の px が覆う元の画素の範囲と重み(重なりの長さ × s。全部覆えば合計 1)
    let weights = |a: f64, b: f64, n: usize| -> (usize, Vec<f32>) {
        let a = a.clamp(0.0, n as f64);
        let b = b.clamp(0.0, n as f64);
        if b <= a {
            return (0, Vec::new());
        }
        let i0 = a.floor() as usize;
        let i1 = (b.ceil() as usize).min(n);
        let ws =
            (i0..i1).map(|i| (((i + 1) as f64).min(b) - (i as f64).max(a)).max(0.0) * s).map(|v| v as f32).collect();
        (i0, ws)
    };
    // 画面の px の両端を画像の添字の座標(画素 i が [i, i + 1])に写す
    let span = |a: f64, b: f64| if a <= b { (a, b) } else { (b, a) };
    let cols: Vec<(usize, Vec<f32>)> = (xs..xe)
        .map(|px| {
            let (a, b) = span(bx.to_image(px as f64, 0.0).0 + 0.5, bx.to_image(px as f64 + 1.0, 0.0).0 + 0.5);
            weights(a, b, nx)
        })
        .collect();
    let rgba = &fig.rgba;
    let width = pw;
    let data = pm.data_mut();
    data.par_chunks_mut(width * 4).enumerate().skip(ys).take(ye - ys).for_each(|(py, row)| {
        let (r0, r1) = span(bx.to_image(0.0, py as f64).1 + 0.5, bx.to_image(0.0, py as f64 + 1.0).1 + 0.5);
        let (j0, wy) = weights(r0, r1, ny);
        if wy.is_empty() {
            return;
        }
        let mut acc = vec![[0.0f32; 4]; xe - xs];
        for (dj, &wyj) in wy.iter().enumerate() {
            let src = &rgba[(j0 + dj) * nx * 4..(j0 + dj + 1) * nx * 4];
            for (k, (i0, wx)) in cols.iter().enumerate() {
                let a = &mut acc[k];
                for (di, &wxi) in wx.iter().enumerate() {
                    let p = &src[(i0 + di) * 4..(i0 + di) * 4 + 4];
                    let w = wyj * wxi * (p[3] as f32 / 255.0);
                    if w != 0.0 {
                        a[0] += w * p[0] as f32;
                        a[1] += w * p[1] as f32;
                        a[2] += w * p[2] as f32;
                        a[3] += w;
                    }
                }
            }
        }
        for (k, a) in acc.iter().enumerate() {
            let sa = a[3].clamp(0.0, 1.0);
            if sa <= 0.0 {
                continue;
            }
            let o = (xs + k) * 4;
            let inv = 1.0 - sa;
            for ch in 0..3 {
                let d = row[o + ch] as f32;
                row[o + ch] = (a[ch] + d * inv).round().clamp(0.0, 255.0) as u8;
            }
            row[o + 3] = (255.0 * sa + row[o + 3] as f32 * inv).round().clamp(0.0, 255.0) as u8;
        }
    });
}

/// 矢印の形(matplotlib の ArrowStyle "simple")。点は画面の px。
fn arrow_path(from: (f64, f64), to: (f64, f64), tail: f64, head_w: f64, head_l: f64) -> Option<tiny_skia::Path> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy);
    if len.is_nan() || len <= 1e-9 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let base = (to.0 - ux * head_l, to.1 - uy * head_l);
    let mut pb = PathBuilder::new();
    let p = |x: f64, y: f64| (x as f32, y as f32);
    if len > head_l {
        let (a, b) = p(from.0 + nx * tail / 2.0, from.1 + ny * tail / 2.0);
        pb.move_to(a, b);
        let (a, b) = p(base.0 + nx * tail / 2.0, base.1 + ny * tail / 2.0);
        pb.line_to(a, b);
    } else {
        // 矢じりより短いときは矢じりだけ(matplotlib と同じく矢じりの長さは変えない)
        let (a, b) = p(base.0, base.1);
        pb.move_to(a, b);
    }
    let (a, b) = p(base.0 + nx * head_w / 2.0, base.1 + ny * head_w / 2.0);
    pb.line_to(a, b);
    let (a, b) = p(to.0, to.1);
    pb.line_to(a, b);
    let (a, b) = p(base.0 - nx * head_w / 2.0, base.1 - ny * head_w / 2.0);
    pb.line_to(a, b);
    if len > head_l {
        let (a, b) = p(base.0 - nx * tail / 2.0, base.1 - ny * tail / 2.0);
        pb.line_to(a, b);
        let (a, b) = p(from.0 - nx * tail / 2.0, from.1 - ny * tail / 2.0);
        pb.line_to(a, b);
    }
    pb.close();
    pb.finish()
}

fn stroke(width: f64, cap: LineCap, join: LineJoin, dash: Option<[f32; 2]>, ppt: f64) -> Stroke {
    let mut s = Stroke { width: width as f32, line_cap: cap, line_join: join, ..Stroke::default() };
    if let Some(d) = dash {
        let (on, off) = ((d[0] as f64 * ppt) as f32, (d[1] as f64 * ppt) as f32);
        if on > 0.0 && off > 0.0 {
            s.dash = StrokeDash::new(vec![on, off], 0.0);
        }
    }
    s
}

fn render_pixmap(fig: &Figure, opts: &RenderOptions, fonts: Option<&Fonts>) -> Result<Pixmap, PlotError> {
    let (w, h) = figure_size(fig, opts);
    let mut pm = Pixmap::new(w, h).ok_or_else(|| PlotError::Render(format!("{w}×{h} の画像を作れません")))?;
    let ppt = dpi(opts) / 72.0;
    let theme = opts.theme;
    let c = Content::new(fig, opts);
    let texts = measure_all(&c, fonts, ppt);
    let (bx, decos) = plan(&c, &texts, w, h, ppt);
    pm.fill(to_ts(theme.fig_bg));
    let (bw, bh) = bx.size();
    let rect = Rect::from_xywh(bx.x0 as f32, bx.y0 as f32, bw.max(1e-3) as f32, bh.max(1e-3) as f32);
    if let Some(r) = rect {
        pm.fill_rect(r, &paint(theme.ax_bg), Transform::identity(), None);
    }
    draw_image(&mut pm, fig, &bx);
    // 軸の中だけに描く(matplotlib の clip_on)
    let mut mask = Mask::new(w, h);
    if let (Some(m), Some(r)) = (mask.as_mut(), rect) {
        m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, true, Transform::identity());
    }
    let clip = mask.as_ref();
    let tr = |p: [f64; 2]| bx.to_screen(p[0], p[1]);
    let mut texts_after: Vec<&Prim> = Vec::new();
    for prim in c.prims() {
        match prim {
            Prim::Polyline { points, color, width_pt, dash_pt } => {
                if points.len() < 2 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                let (x, y) = tr(points[0]);
                pb.move_to(x as f32, y as f32);
                for p in &points[1..] {
                    let (x, y) = tr(*p);
                    pb.line_to(x as f32, y as f32);
                }
                if let Some(path) = pb.finish() {
                    let st = stroke(*width_pt as f64 * ppt, LineCap::Butt, LineJoin::Miter, *dash_pt, ppt);
                    pm.stroke_path(&path, &paint(*color), &st, Transform::identity(), clip);
                }
            }
            Prim::Circle { center, diameter_pt, color, width_pt } => {
                let (x, y) = tr(*center);
                let r = (*diameter_pt as f64 * ppt / 2.0) as f32;
                if let Some(path) = PathBuilder::from_circle(x as f32, y as f32, r) {
                    let st = stroke(*width_pt as f64 * ppt, LineCap::Butt, LineJoin::Round, None, ppt);
                    pm.stroke_path(&path, &paint(*color), &st, Transform::identity(), clip);
                }
            }
            Prim::Cross { center, size_pt, color, width_pt } => {
                let (x, y) = tr(*center);
                let hs = *size_pt as f64 * ppt / 2.0;
                let mut pb = PathBuilder::new();
                pb.move_to((x - hs) as f32, (y - hs) as f32);
                pb.line_to((x + hs) as f32, (y + hs) as f32);
                pb.move_to((x - hs) as f32, (y + hs) as f32);
                pb.line_to((x + hs) as f32, (y - hs) as f32);
                if let Some(path) = pb.finish() {
                    let st = stroke(*width_pt as f64 * ppt, LineCap::Butt, LineJoin::Miter, None, ppt);
                    pm.stroke_path(&path, &paint(*color), &st, Transform::identity(), clip);
                }
            }
            Prim::Arrow { from, to, color, tail_width_pt, head_width_pt, head_length_pt, edge_width_pt } => {
                let f = tr(*from);
                let t = tr(*to);
                let path = arrow_path(
                    f,
                    t,
                    *tail_width_pt as f64 * ppt,
                    *head_width_pt as f64 * ppt,
                    *head_length_pt as f64 * ppt,
                );
                if let Some(path) = path {
                    pm.fill_path(&path, &paint(*color), FillRule::Winding, Transform::identity(), clip);
                    if *edge_width_pt > 0.0 {
                        let st = stroke(*edge_width_pt as f64 * ppt, LineCap::Round, LineJoin::Round, None, ppt);
                        pm.stroke_path(&path, &paint(*color), &st, Transform::identity(), clip);
                    }
                }
            }
            Prim::Text { .. } => texts_after.push(prim),
        }
    }
    // 目盛り(外向き)と枠
    let fg = theme.fig_fg;
    {
        let mut pb = PathBuilder::new();
        let len = style::TICK_LENGTH as f64 * ppt;
        for t in c.ticks() {
            let (x, y) = tr(t.pos);
            let a = (t.angle_deg + 180.0).to_radians();
            // 角度は画面の上向きが正(y 上)なので、画面の座標(y 下)では符号を反転
            let (ex, ey) = (x + a.cos() * len, y - a.sin() * len);
            pb.move_to(x as f32, y as f32);
            pb.line_to(ex as f32, ey as f32);
        }
        if let Some(path) = pb.finish() {
            let st = stroke(style::TICK_WIDTH as f64 * ppt, LineCap::Butt, LineJoin::Miter, None, ppt);
            pm.stroke_path(&path, &paint(fg), &st, Transform::identity(), None);
        }
        if let Some(r) = rect {
            let path = PathBuilder::from_rect(r);
            let st = stroke(style::FRAME_WIDTH as f64 * ppt, LineCap::Butt, LineJoin::Miter, None, ppt);
            pm.stroke_path(&path, &paint(fg), &st, Transform::identity(), None);
        }
    }
    let Some(fonts) = fonts else { return Ok(pm) };
    // 表示リストの文字(TARGET。matplotlib の annotate の既定は左寄せ・基準線)。指す点が軸の外なら描かない
    // (annotation_clip。拡大した表示で目標が範囲の外に出たとき)
    let (ax0, ay0) = (bx.x0, bx.y0);
    let (ax1, ay1) = (bx.x0 + bw, bx.y0 + bh);
    for prim in texts_after {
        if let Prim::Text { anchor, offset_pt, text, size_pt, color, bold, halign, valign } = prim {
            let (x, y) = tr(*anchor);
            if !(x >= ax0 - 1e-6 && x <= ax1 + 1e-6 && y >= ay0 - 1e-6 && y <= ay1 + 1e-6) {
                continue;
            }
            let x = x + offset_pt[0] as f64 * ppt;
            let y = y - offset_pt[1] as f64 * ppt;
            let t = measure(Some(fonts), &plain(text), *size_pt as f64 * ppt, *bold);
            let (origin, _) = place(&t.m, x, y, *halign, *valign, Rot::R0);
            if let Some(line) = &t.line {
                draw_text(&mut pm, fonts, line, *color, origin, Rot::R0);
            }
        }
    }
    // 目盛りの文字・軸名・題(置き場所は plan で決めたもの)
    for d in &decos {
        if let Some(line) = &texts.get(d.which).line {
            draw_text(&mut pm, fonts, line, fg, d.origin, d.rot);
        }
    }
    Ok(pm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_box_roundtrip() {
        for flip in [false, true] {
            let b = AxesBox::full(50.0, 30.0, 0.25, 400, 300, flip);
            let (sx, sy) = b.to_screen(12.0, 34.0);
            let (x, y) = b.to_image(sx, sy);
            assert!((x - 12.0).abs() < 1e-9 && (y - 34.0).abs() < 1e-9);
            // 画像の y = −0.5 は flip_y でなければ軸の下端
            let (_, s) = b.to_screen(0.0, -0.5);
            assert!((s - if flip { 30.0 } else { 30.0 + 75.0 }).abs() < 1e-9);
        }
    }

    #[test]
    fn arrow_shapes() {
        assert!(arrow_path((0.0, 0.0), (0.0, 0.0), 2.0, 10.0, 12.0).is_none());
        assert!(arrow_path((0.0, 0.0), (100.0, 0.0), 2.0, 10.0, 12.0).is_some());
        assert!(arrow_path((0.0, 0.0), (5.0, 0.0), 2.0, 10.0, 12.0).is_some());
    }
}

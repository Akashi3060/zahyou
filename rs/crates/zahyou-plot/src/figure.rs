//! 図 2 枚(「撮影時の向き」と「北が上・東が左」)の中身。Python 版 `dev/zahyou_plot.py` の
//! `plot_original`・`plot_north_up`・`_draw_markers`・`clip_arrow_to_boundary`・`report_offsets`・`wcs_summary` を移したもの。
//!
//! 図は「画像の RGBA」と「画像の座標で書いた表示リスト」で持つ。描く側(PNG の書き出しは render.rs、画面は app)は
//! 画像の座標 → 画面の座標の写しだけを決めればよい。線の太さ・印の大きさ・文字の大きさは pt(1/72 インチ。画面の上で一定)。
//!
//! 色と大きさは Python 版と同じ:
//! - 格子: deepskyblue の破線、0.9 pt、不透明度 0.65(matplotlib の破線 [3.7, 1.6] × 線幅)
//! - 検出した星: lime の円(直径 √90 pt)、0.8 pt、不透明度 0.55
//! - 画像中心: cyan の ×(√180 pt)、2 pt
//! - 目標: 赤い矢印(軸 2.4 pt・矢じりの幅 13 pt・長さ 16 pt、不透明度 0.9)。画面内なら yellow の円(√520 pt、2 pt)と
//!   「TARGET」(12 pt 太字、目標から右上へ 14 pt ずらす)。画面外なら矢印を枠との交点の 0.97 倍で止める

use zahyou_image::{Image2D, Source};
use zahyou_wcs::{Wcs, offset_by, position_angle, spherical_offsets};

use crate::PlotError;
use crate::axes::{AxisLabel, Tick, TickLabel, wcs_grid};
use crate::norm::{Norm, compute_norm, compute_norm_f64, gray_rgba};
use crate::reproject::{check_image, mean_pixel_scale_deg, reproject_north_up};
use crate::sexa::hmsdms;

/// 色(sRGB、不透明度つき)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// `#RRGGBB` から(読めなければ None)。
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.strip_prefix('#')?;
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// 不透明度(0〜1)を掛けたもの(matplotlib の alpha)。
    pub fn alpha(self, alpha: f32) -> Color {
        Color { a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, ..self }
    }
}

/// matplotlib の色の名前(CSS4)。
pub mod named {
    use super::Color;
    pub const DEEPSKYBLUE: Color = Color::rgb(0x00, 0xBF, 0xFF);
    pub const LIME: Color = Color::rgb(0x00, 0xFF, 0x00);
    pub const CYAN: Color = Color::rgb(0x00, 0xFF, 0xFF);
    pub const YELLOW: Color = Color::rgb(0xFF, 0xFF, 0x00);
    pub const RED: Color = Color::rgb(0xFF, 0x00, 0x00);
}

/// 図の配色(Python 版の GUI の fig_bg・ax_bg・fig_fg)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// 図の地の色
    pub fig_bg: Color,
    /// 軸の中の地の色(画像の NaN の所に見える)
    pub ax_bg: Color,
    /// 文字・枠・目盛りの色
    pub fig_fg: Color,
}

impl Theme {
    /// 明るい配色(#FFFFFF / #FFFFFF / #14181F)。
    pub const fn light() -> Theme {
        Theme {
            fig_bg: Color::rgb(0xFF, 0xFF, 0xFF),
            ax_bg: Color::rgb(0xFF, 0xFF, 0xFF),
            fig_fg: Color::rgb(0x14, 0x18, 0x1F),
        }
    }

    /// 暗い配色(#15181D / #05070A / #DCE1E8)。
    pub const fn dark() -> Theme {
        Theme {
            fig_bg: Color::rgb(0x15, 0x18, 0x1D),
            ax_bg: Color::rgb(0x05, 0x07, 0x0A),
            fig_fg: Color::rgb(0xDC, 0xE1, 0xE8),
        }
    }

    /// 名前("light" / "dark")から。ほかは light。
    pub fn by_name(name: &str) -> Theme {
        if name == "dark" { Theme::dark() } else { Theme::light() }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}

/// 文字の横の寄せ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// 文字の縦の寄せ(Baseline は文字の基準線)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Center,
    Baseline,
    Bottom,
}

/// 表示リストの 1 要素。位置は画像の座標(画素、0 始まり、画素の中心が整数)、大きさは pt。
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    /// 折れ線。`dash_pt` があれば破線(線の長さ・すき間)
    Polyline { points: Vec<[f64; 2]>, color: Color, width_pt: f32, dash_pt: Option<[f32; 2]> },
    /// 中を塗らない円(直径 pt)
    Circle { center: [f64; 2], diameter_pt: f32, color: Color, width_pt: f32 },
    /// ×(大きさ pt。斜めの 2 本の線が収まる正方形の 1 辺)
    Cross { center: [f64; 2], size_pt: f32, color: Color, width_pt: f32 },
    /// 矢印(matplotlib の annotate の "simple" の形。先端が `to`)。縁も同じ色で `edge_width_pt` の線を引く
    Arrow {
        from: [f64; 2],
        to: [f64; 2],
        color: Color,
        tail_width_pt: f32,
        head_width_pt: f32,
        head_length_pt: f32,
        edge_width_pt: f32,
    },
    /// 文字。`anchor` から画面の向きで `offset_pt`(右・上が正)だけずらした点に、寄せを合わせて置く
    Text {
        anchor: [f64; 2],
        offset_pt: [f32; 2],
        text: String,
        size_pt: f32,
        color: Color,
        bold: bool,
        halign: HAlign,
        valign: VAlign,
    },
}

/// 線と印の大きさ(pt。Python 版の値)。
pub mod style {
    use super::{Color, named};

    pub const GRID_WIDTH: f32 = 0.9;
    pub const GRID_ALPHA: f32 = 0.65;
    /// matplotlib の破線 [3.7, 1.6] に線幅を掛けたもの
    pub const GRID_DASH: [f32; 2] = [3.7 * 0.9, 1.6 * 0.9];
    pub fn grid_color() -> Color {
        named::DEEPSKYBLUE.alpha(GRID_ALPHA)
    }
    /// scatter の s=90 → 直径 √90 pt
    pub const SOURCE_DIAMETER: f32 = 9.486_833;
    pub const SOURCE_WIDTH: f32 = 0.8;
    pub const SOURCE_ALPHA: f32 = 0.55;
    /// s=180 → √180 pt
    pub const CENTER_SIZE: f32 = 13.416_408;
    pub const CENTER_WIDTH: f32 = 2.0;
    /// s=520 → √520 pt
    pub const TARGET_DIAMETER: f32 = 22.803_509;
    pub const TARGET_WIDTH: f32 = 2.0;
    pub const ARROW_TAIL: f32 = 2.4;
    pub const ARROW_HEAD_WIDTH: f32 = 13.0;
    pub const ARROW_HEAD_LENGTH: f32 = 16.0;
    pub const ARROW_ALPHA: f32 = 0.9;
    /// FancyArrowPatch の縁の線(patch.linewidth)
    pub const ARROW_EDGE: f32 = 1.0;
    pub const TARGET_TEXT_SIZE: f32 = 12.0;
    pub const TARGET_TEXT_OFFSET: [f32; 2] = [14.0, 14.0];
    /// 題(size=14)
    pub const TITLE_SIZE: f32 = 14.0;
    /// 目盛りの文字・軸名(rcParams の medium = 10 pt)
    pub const TICK_LABEL_SIZE: f32 = 10.0;
    pub const AXIS_LABEL_SIZE: f32 = 10.0;
    /// 目盛り(xtick.major.size・width、外向き)
    pub const TICK_LENGTH: f32 = 3.5;
    pub const TICK_WIDTH: f32 = 0.8;
    /// 目盛りと文字のすき間(xtick.major.pad)
    pub const TICK_PAD: f32 = 3.5;
    /// 枠の線(axes.linewidth)
    pub const FRAME_WIDTH: f32 = 0.8;
    /// 題と枠のすき間(axes.titlepad)
    pub const TITLE_PAD: f32 = 6.0;
}

/// どちらの図か。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FigureKind {
    /// 撮影時の向き(flip_y なら上下を反転して表示)
    Original,
    /// 北が上・東が左
    NorthUp,
}

/// 図 1 枚。
#[derive(Clone, Debug)]
pub struct Figure {
    pub kind: FigureKind,
    /// 画像の大きさ(画素)。表示の範囲は x が −0.5〜width−0.5、y が −0.5〜height−0.5
    pub width: usize,
    pub height: usize,
    /// 伸張した画像(1 画素 4 バイトの RGBA、straight alpha)。並びは画像と同じ(行 0 = 画像の y = 0)。
    /// 有限でない画素は透明(軸の背景色が見える)
    pub rgba: Vec<u8>,
    /// true なら行 0 を画面の上に(Python の invert_yaxis)。false なら行 0 が画面の下(origin="lower")
    pub flip_y: bool,
    /// 題(2 行。"\n" で区切る)
    pub title: String,
    /// 表示リスト(描く順。格子 → 星 → 中心 → 矢印 → 目標の円 → TARGET)
    pub prims: Vec<Prim>,
    /// 枠の 4 辺の目盛り(赤経・赤緯の両方。外向きに 3.5 pt)
    pub ticks: Vec<Tick>,
    /// 目盛りの文字(座標ごとに 1 つの辺だけ)
    pub tick_labels: Vec<TickLabel>,
    /// 軸名(「赤経 (RA)」「赤緯 (Dec)」)
    pub axis_labels: Vec<AxisLabel>,
    /// この図の画素の座標の WCS(北が上の図では描き直した先の WCS)
    pub wcs: Wcs,
    /// 伸張の範囲(有限な画素が無ければ None)
    pub norm: Option<Norm>,
    /// 目標の画素(投影できれば。画面外でも入る)
    pub target_pixel: Option<[f64; 2]>,
    /// 目標が画面内か(目標が無ければ None)
    pub target_inside: Option<bool>,
}

impl Figure {
    /// 画像の座標の (x, y) の画素の RGBA(範囲外は None)。
    pub fn pixel(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y * self.width + x) * 4;
        self.rgba.get(i..i + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }
}

/// 画面外の目標へ向かう矢印を、画面の縁で止める(Python の `clip_arrow_to_boundary`)。
///
/// 戻り値 (止める x, 止める y, 目標が画面内か)。画面内ならそのまま、画面外なら縁との交点の 0.97 倍の所。
/// 交点が無ければ目標そのものを返す(矢印は描けない)。
pub fn clip_arrow_to_boundary(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    xlim: (f64, f64),
    ylim: (f64, f64),
) -> (f64, f64, bool) {
    let (xmin, xmax) = (xlim.0.min(xlim.1), xlim.0.max(xlim.1));
    let (ymin, ymax) = (ylim.0.min(ylim.1), ylim.0.max(ylim.1));
    if xmin <= x1 && x1 <= xmax && ymin <= y1 && y1 <= ymax {
        return (x1, y1, true);
    }
    let (dx, dy) = (x1 - x0, y1 - y0);
    if dx.abs() < 1e-12 && dy.abs() < 1e-12 {
        return (x1, y1, true);
    }
    let mut ts = Vec::with_capacity(4);
    if dx.abs() > 1e-12 {
        ts.push((xmin - x0) / dx);
        ts.push((xmax - x0) / dx);
    }
    if dy.abs() > 1e-12 {
        ts.push((ymin - y0) / dy);
        ts.push((ymax - y0) / dy);
    }
    let mut best: Option<f64> = None;
    for t in ts {
        if t <= 1e-9 {
            continue;
        }
        let (ix, iy) = (x0 + t * dx, y0 + t * dy);
        if xmin - 1e-6 <= ix && ix <= xmax + 1e-6 && ymin - 1e-6 <= iy && iy <= ymax + 1e-6 {
            best = Some(match best {
                None => t,
                Some(b) => b.min(t),
            });
        }
    }
    match best {
        None => (x1, y1, false),
        Some(b) => (x0 + b * 0.97 * dx, y0 + b * 0.97 * dy, false),
    }
}

/// 目標までのずれ(Python の `report_offsets`)。分角。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offsets {
    /// 角距離(′)
    pub sep_arcmin: f64,
    /// 角距離(″)
    pub sep_arcsec: f64,
    /// 赤経方向(′、東が正)
    pub ra_arcmin: f64,
    /// 赤緯方向(′、北が正)
    pub dec_arcmin: f64,
}

impl Offsets {
    /// 画像中心 (赤経, 赤緯) → 目標 (赤経, 赤緯)(度)。
    pub fn new(center: (f64, f64), target: (f64, f64)) -> Offsets {
        let (sep, dra, ddec) = spherical_offsets(center, target);
        // astropy の Angle(度)→ arcmin(× 60)
        let am = |r: f64| r.to_degrees() * 60.0;
        Offsets { sep_arcmin: am(sep), sep_arcsec: sep.to_degrees() * 3600.0, ra_arcmin: am(dra), dec_arcmin: am(ddec) }
    }

    /// 「東」/「西」(Python: ra_min > 0 なら東)
    pub fn ra_dir(&self) -> &'static str {
        if self.ra_arcmin > 0.0 { "東" } else { "西" }
    }

    /// 「北」/「南」
    pub fn dec_dir(&self) -> &'static str {
        if self.dec_arcmin > 0.0 { "北" } else { "南" }
    }

    /// 解析ログに出す 4 行(Python の `report_offsets` の `_log` と同じ文。1 行目は改行で始まる)。
    pub fn report_lines(&self) -> Vec<String> {
        vec![
            "\n  画像中心から目標までのずれ".to_string(),
            format!("    全角距離      : {:.3}′  ({:.1}″)", self.sep_arcmin, self.sep_arcsec),
            format!("    赤経(RA) 方向 : {:.3}′ {}へ", self.ra_arcmin.abs(), self.ra_dir()),
            format!("    赤緯(Dec)方向 : {:.3}′ {}へ", self.dec_arcmin.abs(), self.dec_dir()),
        ]
    }
}

/// 「撮影時の向き」の題。目標があれば `目標: 04h07m38.88s +41d59m10.54s`、無ければ「目標未指定 (中心のみ表示)」。
pub fn title_original(target: Option<(f64, f64)>) -> String {
    let sub = match target {
        Some((ra, dec)) => format!("目標: {}", hmsdms(ra, dec, 2)),
        None => "目標未指定 (中心のみ表示)".to_string(),
    };
    format!("撮影時の向き\n{sub}")
}

/// 「北が上・東が左」の題。`距離 1.58′ | RA 0.92′(東) Dec 1.29′(北)`。目標が無ければ 2 行目は「目標未指定 (中心のみ表示)」。
pub fn title_north_up(center: (f64, f64), target: Option<(f64, f64)>) -> String {
    match target {
        Some(t) => {
            let o = Offsets::new(center, t);
            format!(
                "北が上・東が左\n距離 {:.2}′ | RA {:.2}′({}) Dec {:.2}′({})",
                o.sep_arcmin,
                o.ra_arcmin.abs(),
                o.ra_dir(),
                o.dec_arcmin.abs(),
                o.dec_dir()
            )
        }
        None => "北が上・東が左\n目標未指定 (中心のみ表示)".to_string(),
    }
}

/// 解いた WCS の要点(Python の `wcs_summary`。位置角とパリティは画面の見た目で)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    /// 画像中心 (赤経, 赤緯)(度)
    pub center: (f64, f64),
    /// 画素スケール(″/px。CD の列の長さの平均)
    pub scale_arcsec: f64,
    /// 画面の上向きの位置角(度、北から東回り、[0, 360))
    pub pa_deg: f64,
    /// 鏡像か(画面で「北が上なら東が左」= 空を見上げたままの向きが鏡像なし)
    pub mirrored: bool,
    /// 視野 (横, 縦)(′)
    pub fov_arcmin: (f64, f64),
}

impl Summary {
    /// パリティの文言(「正 (鏡像なし)」/「負 (鏡像)」)。
    pub fn parity(&self) -> &'static str {
        if self.mirrored { "負 (鏡像)" } else { "正 (鏡像なし)" }
    }

    /// 解析ログの結果の行(Python の `print_solution` と同じ文。1 行目は改行で始まる)。
    pub fn lines(&self) -> Vec<String> {
        let bar = "=".repeat(62);
        vec![
            format!("\n{bar}"),
            "  解析結果".to_string(),
            bar.clone(),
            format!("  画像中心      : {}", hmsdms(self.center.0, self.center.1, 2)),
            format!("                  (RA {:.6}°, Dec {:.6}°)", self.center.0, self.center.1),
            format!("  画素スケール  : {:.4} 秒角/px", self.scale_arcsec),
            format!("  視野          : {:.2}′ x {:.2}′", self.fov_arcmin.0, self.fov_arcmin.1),
            format!("  画面上方向    : 位置角 {:.2}°(北から東回り)", self.pa_deg),
            format!("  パリティ      : {}", self.parity()),
        ]
    }
}

/// `wcs_summary(wcs, shape, flip_y)`。
pub fn wcs_summary(wcs: &Wcs, width: usize, height: usize, flip_y: bool) -> Summary {
    let (nx, ny) = (width as f64, height as f64);
    let scale = mean_pixel_scale_deg(wcs) * 3600.0;
    let (cx, cy) = ((nx - 1.0) / 2.0, (ny - 1.0) / 2.0);
    let center = wcs.pix_to_radec(cx, cy);
    let up = wcs.pix_to_radec(cx, cy + if flip_y { -10.0 } else { 10.0 });
    let pa = position_angle(center, up).to_degrees();
    let det = wcs.cd[0][0] * wcs.cd[1][1] - wcs.cd[0][1] * wcs.cd[1][0];
    let mirrored = (if flip_y { -det } else { det }) > 0.0;
    Summary { center, scale_arcsec: scale, pa_deg: pa, mirrored, fov_arcmin: (nx * scale / 60.0, ny * scale / 60.0) }
}

/// 目標の画素(SIP はニュートン法で正確に解く)。投影できなければ None(反対の空など)。
///
/// SIP の多項式は画像の外では意味を持たず、画像から遠い目標ではニュートン法が別の根へ行くことがある
/// (Python の astropy も all_world2pix が発散して当てにならない値を返す)。そこで、SIP の解が TAN だけの解から
/// 画像の対角線より離れたとき・解けなかったときは TAN だけの解を使う(矢印の向きだけが要る場面なので)。
pub fn target_pixel(wcs: &Wcs, width: usize, height: usize, target: (f64, f64)) -> Option<(f64, f64)> {
    let ok = |p: &(f64, f64)| p.0.is_finite() && p.1.is_finite();
    let exact = wcs.radec_to_pix_exact(target.0, target.1).filter(ok);
    if !wcs.has_sip() {
        return exact;
    }
    let tan = wcs.tan_only().radec_to_pix(target.0, target.1).filter(ok);
    let diag = (width as f64).hypot(height as f64).max(1.0);
    match (exact, tan) {
        (Some(e), Some(t)) if (e.0 - t.0).hypot(e.1 - t.1) > diag => Some(t),
        (None, t) => t,
        (e, _) => e,
    }
}

/// 中心の ×、目標の矢印・円・TARGET を表示リストへ(Python の `_draw_markers`)。戻り値 (目標の画素, 画面内か)。
fn draw_markers(
    prims: &mut Vec<Prim>,
    wcs: &Wcs,
    width: usize,
    height: usize,
    target: Option<(f64, f64)>,
    label_target: bool,
) -> (Option<[f64; 2]>, Option<bool>) {
    let (nx, ny) = (width as f64, height as f64);
    let c = ((nx - 1.0) / 2.0, (ny - 1.0) / 2.0);
    let center = wcs.pix_to_radec(c.0, c.1);
    // 中心の天球座標から画素へ戻す(Python と同じ。往復の差は 1e-9 画素以下)
    let (sx, sy) =
        wcs.radec_to_pix_exact(center.0, center.1).filter(|p| p.0.is_finite() && p.1.is_finite()).unwrap_or(c);
    prims.push(Prim::Cross {
        center: [sx, sy],
        size_pt: style::CENTER_SIZE,
        color: named::CYAN,
        width_pt: style::CENTER_WIDTH,
    });
    let Some(t) = target else { return (None, None) };
    if !(t.0.is_finite() && t.1.is_finite()) {
        return (None, None);
    }
    let direct = target_pixel(wcs, width, height, t);
    let (tx, ty) = match direct {
        Some(p) => p,
        None => {
            // 反対の空を指しているなど、投影が破綻するとき: 方向だけ求めて遠くを指す
            let pa = position_angle(center, t);
            let near = offset_by(center, pa, (1.0f64 / 60.0).to_radians());
            match wcs.radec_to_pix_exact(near.0, near.1) {
                Some((a, b)) if a.is_finite() && b.is_finite() => (sx + (a - sx) * 1e5, sy + (b - sy) * 1e5),
                _ => return (None, Some(false)),
            }
        }
    };
    let (cx, cy, inside) = clip_arrow_to_boundary(sx, sy, tx, ty, (-0.5, nx - 0.5), (-0.5, ny - 0.5));
    if cx.is_finite() && cy.is_finite() && (cx != sx || cy != sy) {
        prims.push(Prim::Arrow {
            from: [sx, sy],
            to: [cx, cy],
            color: named::RED.alpha(style::ARROW_ALPHA),
            tail_width_pt: style::ARROW_TAIL,
            head_width_pt: style::ARROW_HEAD_WIDTH,
            head_length_pt: style::ARROW_HEAD_LENGTH,
            edge_width_pt: style::ARROW_EDGE,
        });
    }
    if inside {
        prims.push(Prim::Circle {
            center: [tx, ty],
            diameter_pt: style::TARGET_DIAMETER,
            color: named::YELLOW,
            width_pt: style::TARGET_WIDTH,
        });
        if label_target {
            prims.push(Prim::Text {
                anchor: [tx, ty],
                offset_pt: style::TARGET_TEXT_OFFSET,
                text: "TARGET".to_string(),
                size_pt: style::TARGET_TEXT_SIZE,
                color: named::YELLOW,
                bold: true,
                halign: HAlign::Left,
                valign: VAlign::Baseline,
            });
        }
    }
    let tp = if direct.is_some() { Some([tx, ty]) } else { None };
    (tp, Some(inside))
}

/// 伸張した画像・格子・目盛りを作る(2 つの図で共通の部分)。
fn base_figure(kind: FigureKind, img: &Image2D, wcs: &Wcs, flip_y: bool, title: String) -> Figure {
    // 北が上の図は Python では float64 の配列(reproject の結果)なので、パーセント点を float64 の式で
    let norm = if kind == FigureKind::NorthUp { compute_norm_f64(&img.data) } else { compute_norm(&img.data) };
    let rgba = gray_rgba(img, norm.as_ref());
    let grid = wcs_grid(wcs, img.width, img.height, flip_y);
    let gc = style::grid_color();
    let prims = grid
        .lines
        .into_iter()
        .map(|l| Prim::Polyline {
            points: l.points,
            color: gc,
            width_pt: style::GRID_WIDTH,
            dash_pt: Some(style::GRID_DASH),
        })
        .collect();
    Figure {
        kind,
        width: img.width,
        height: img.height,
        rgba,
        flip_y,
        title,
        prims,
        ticks: grid.ticks,
        tick_labels: grid.labels,
        axis_labels: grid.axis_labels,
        wcs: wcs.clone(),
        norm,
        target_pixel: None,
        target_inside: None,
    }
}

/// 図 1「撮影時の向き」(Python の `plot_original`)。
///
/// - `img`: 表示用の画像(`Loaded::display`。NaN を含んでよい)
/// - `sources`: 検出した星(`mark_sources` が true なら lime の円で囲む)
/// - `target`: 目標 (赤経, 赤緯)(度)。None なら中心の × だけ(Python の draw_arrow=False)
/// - `flip_y`: 上下を反転して表示するか(`Loaded::flip_y`)
pub fn figure_original(
    img: &Image2D,
    wcs: &Wcs,
    sources: &[Source],
    target: Option<(f64, f64)>,
    flip_y: bool,
    mark_sources: bool,
) -> Result<Figure, PlotError> {
    check_image(img)?;
    let mut fig = base_figure(FigureKind::Original, img, wcs, flip_y, title_original(target));
    if mark_sources {
        let color = named::LIME.alpha(style::SOURCE_ALPHA);
        for s in sources.iter().filter(|s| s.x.is_finite() && s.y.is_finite()) {
            fig.prims.push(Prim::Circle {
                center: [s.x, s.y],
                diameter_pt: style::SOURCE_DIAMETER,
                color,
                width_pt: style::SOURCE_WIDTH,
            });
        }
    }
    let (tp, inside) = draw_markers(&mut fig.prims, wcs, img.width, img.height, target, true);
    fig.target_pixel = tp;
    fig.target_inside = inside;
    Ok(fig)
}

/// 図 2「北が上・東が左」(Python の `plot_north_up`)。画像を描き直すので重い(24 メガ画素で 1 秒弱)。
///
/// 題のずれは、元の画像の中心から目標まで(Python と同じ)。中心の × は描き直した画像の中心に置く。
pub fn figure_north_up(img: &Image2D, wcs: &Wcs, target: Option<(f64, f64)>) -> Result<Figure, PlotError> {
    check_image(img)?;
    let nu = reproject_north_up(img, wcs)?;
    let center = wcs.pix_to_radec((img.width as f64 - 1.0) / 2.0, (img.height as f64 - 1.0) / 2.0);
    let title = title_north_up(center, target.filter(|t| t.0.is_finite() && t.1.is_finite()));
    let mut fig = base_figure(FigureKind::NorthUp, &nu.image, &nu.wcs, false, title);
    let (tp, inside) = draw_markers(&mut fig.prims, &nu.wcs, nu.image.width, nu.image.height, target, true);
    fig.target_pixel = tp;
    fig.target_inside = inside;
    Ok(fig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_clip() {
        // 画面内
        assert_eq!(clip_arrow_to_boundary(5.0, 5.0, 8.0, 2.0, (-0.5, 9.5), (-0.5, 9.5)), (8.0, 2.0, true));
        // 右へはみ出す: x = 9.5 との交点の 0.97 倍
        let (x, y, inside) = clip_arrow_to_boundary(5.0, 5.0, 25.0, 5.0, (-0.5, 9.5), (9.5, -0.5));
        assert!(!inside);
        assert!((x - (5.0 + 0.225 * 0.97 * 20.0)).abs() < 1e-12 && y == 5.0);
        // 同じ点
        assert_eq!(clip_arrow_to_boundary(1.0, 1.0, 1.0, 1.0, (0.0, 0.5), (0.0, 0.5)), (1.0, 1.0, true));
        // NaN は交点なし
        let (_, _, i) = clip_arrow_to_boundary(1.0, 1.0, f64::NAN, 2.0, (0.0, 0.5), (0.0, 0.5));
        assert!(!i);
    }

    #[test]
    fn titles() {
        assert_eq!(title_original(None), "撮影時の向き\n目標未指定 (中心のみ表示)");
        let t = title_north_up((61.891414, 41.964760), Some((61.91199157844, 41.98626242835)));
        assert_eq!(t, "北が上・東が左\n距離 1.58′ | RA 0.92′(東) Dec 1.29′(北)");
        let o = Offsets::new((61.891414, 41.964760), (61.91199157844, 41.98626242835));
        assert_eq!(o.report_lines()[1], "    全角距離      : 1.583′  (95.0″)");
    }

    #[test]
    fn colors() {
        assert_eq!(Color::from_hex("#15181D"), Some(Color::rgb(0x15, 0x18, 0x1D)));
        assert_eq!(Color::from_hex("15181D"), None);
        assert_eq!(named::RED.alpha(0.9).a, 230);
        assert_eq!(Theme::by_name("dark"), Theme::dark());
    }
}

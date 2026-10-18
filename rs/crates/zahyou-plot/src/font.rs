//! 文字の組み版と塗り。ab_glyph で C:\Windows\Fonts の游ゴシックを読む(フォントは同梱しない)。
//!
//! Python 版の図は matplotlib の font.family = "Yu Gothic"(YuGothR.ttc の 0 番 = Yu Gothic Regular、
//! 太字は YuGothB.ttc の 0 番 = Yu Gothic Bold)で描いている。同じ面を選ぶ。無ければメイリオ、MS ゴシックの順。
//! TTC の番号に注意: YuGothR.ttc の 1 番は「Yu Gothic UI Semilight」、YuGothM.ttc の 0 番は「Yu Gothic Medium」。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ab_glyph::{Font, FontVec, Glyph, GlyphId, PxScale, ScaleFont, point};

use crate::PlotError;
use crate::axes::TextSpan;

/// 上付きの文字の大きさ(本文に対する比。matplotlib の mathtext の SHRINK_FACTOR)
pub(crate) const SUP_SCALE: f32 = 0.7;
/// 上付きの基準線の持ち上げ(x 高さに対する比。mathtext の FontConstantsBase.sup1)
const SUP1: f32 = 0.7;
/// 上付きの前の空き(x 高さに対する比。mathtext の delta。上付きと一緒に 0.7 倍に縮む)
const SUP_DELTA: f32 = 0.025;
/// 上付きの後ろの空き(x 高さに対する比。mathtext の script_space)
const SCRIPT_SPACE: f32 = 0.05;

/// 普通の字と太字のフォント。
pub struct Fonts {
    regular: FontVec,
    bold: Option<FontVec>,
    /// 読んだファイルと番号(ログ用)
    pub description: String,
}

/// (ファイル名, TTC の番号)の候補。先にあるものほど優先。
const REGULAR: [(&str, u32); 4] = [("YuGothR.ttc", 0), ("YuGothM.ttc", 0), ("meiryo.ttc", 0), ("msgothic.ttc", 0)];
const BOLD: [(&str, u32); 3] = [("YuGothB.ttc", 0), ("meiryob.ttc", 0), ("YuGothM.ttc", 0)];

static SYSTEM: OnceLock<Option<Fonts>> = OnceLock::new();

/// Windows のフォントのフォルダー(%WINDIR%\Fonts。無ければ C:\Windows\Fonts)。
pub fn system_font_dir() -> PathBuf {
    let win = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"));
    match win {
        Some(w) => PathBuf::from(w).join("Fonts"),
        None => PathBuf::from(r"C:\Windows\Fonts"),
    }
}

fn load_face(path: &Path, index: u32) -> Result<FontVec, PlotError> {
    let data = std::fs::read(path).map_err(|e| PlotError::Font(format!("{}: {e}", path.display())))?;
    FontVec::try_from_vec_and_index(data, index)
        .map_err(|_| PlotError::Font(format!("{} の {index} 番を読めません", path.display())))
}

impl Fonts {
    /// システムのフォント(1 度だけ読み、以後は同じものを返す)。見つからなければ None(文字を描かない)。
    pub fn system() -> Option<&'static Fonts> {
        SYSTEM.get_or_init(|| Fonts::from_dir(&system_font_dir()).ok()).as_ref()
    }

    /// フォルダーから游ゴシック(無ければメイリオ・MS ゴシック)を探す。
    pub fn from_dir(dir: &Path) -> Result<Fonts, PlotError> {
        let mut regular = None;
        let mut desc = String::new();
        for (name, idx) in REGULAR {
            let p = dir.join(name);
            if p.is_file()
                && let Ok(f) = load_face(&p, idx)
            {
                desc = format!("{name}#{idx}");
                regular = Some(f);
                break;
            }
        }
        let regular =
            regular.ok_or_else(|| PlotError::Font(format!("{} に日本語のフォントがありません", dir.display())))?;
        let mut bold = None;
        for (name, idx) in BOLD {
            let p = dir.join(name);
            if p.is_file()
                && let Ok(f) = load_face(&p, idx)
            {
                desc.push_str(&format!(" / 太字 {name}#{idx}"));
                bold = Some(f);
                break;
            }
        }
        Ok(Fonts { regular, bold, description: desc })
    }

    /// ファイルを直接指定する(試験・ほかの環境用)。
    pub fn from_files(regular: (&Path, u32), bold: Option<(&Path, u32)>) -> Result<Fonts, PlotError> {
        let r = load_face(regular.0, regular.1)?;
        let b = match bold {
            Some((p, i)) => Some(load_face(p, i)?),
            None => None,
        };
        Ok(Fonts { regular: r, bold: b, description: format!("{}#{}", regular.0.display(), regular.1) })
    }

    fn face(&self, bold: bool) -> &FontVec {
        if bold { self.bold.as_ref().unwrap_or(&self.regular) } else { &self.regular }
    }
}

/// フォントの em の大きさ(px)→ ab_glyph の PxScale(= 行の高さ ascent − descent の px)。
fn px_scale(font: &FontVec, size_px: f32) -> PxScale {
    let upem = font.units_per_em().unwrap_or(1000.0).max(1.0);
    PxScale::from(size_px * font.height_unscaled() / upem)
}

/// 置いた字形 1 つ(基準線の原点からの位置)。
struct Placed {
    bold: bool,
    glyph: Glyph,
}

/// 1 行の文字の組み版の結果。座標は基準線の左端を原点に、x 右・y 下(px)。
///
/// 文字の箱は matplotlib(Agg)と同じ決め方:
/// - 普通の文字: 幅は送り幅の和(FT2Font の get_width_height。実測で ±0.3 px)、高さは字形の外接矩形(インク)。
///   箱の左端は原点
/// - 上付きを含む文字(WCSAxes の赤経の `5$^h$40$^m$` は mathtext): インクの外接矩形に原点を含め、上下左右に 1 px 足す
///   (`_mathtext.Output.to_raster`)
/// - 高さ h と基準線より下の深さ d は、"lp" の字の値より小さくしない(`Text._get_layout`)。`ascent` = h − d、`descent` = d
pub(crate) struct Line {
    glyphs: Vec<Placed>,
    /// 箱の幅(px)
    pub width: f32,
    /// 箱の左端の、原点からの位置(px)
    pub left: f32,
    /// 基準線より上の高さ(px)
    pub ascent: f32,
    /// 基準線より下の深さ(px、正の値)
    pub descent: f32,
    /// "lp" の字の基準線より上の高さ(px。複数行の行送りの最小に使う)
    pub lp_ascent: f32,
}

/// 字形の輪郭の外接矩形 (左, 上, 右, 下)(基準線の原点から、下が正、px)。px に丸めない
/// (`OutlinedGlyph::px_bounds` は外側へ切り上げるので、1 行の大きさが最大 2 px 大きく出る)。
fn ink_box(font: &FontVec, g: &Glyph) -> Option<[f32; 4]> {
    let o = font.outline(g.id)?;
    let f = font.as_scaled(g.scale).scale_factor();
    // ab_glyph の Outline::bounds は min.y が字の上端(フォント単位、上が正)、max.y が下端
    Some([
        g.position.x + o.bounds.min.x * f.horizontal,
        g.position.y - o.bounds.min.y * f.vertical,
        g.position.x + o.bounds.max.x * f.horizontal,
        g.position.y - o.bounds.max.y * f.vertical,
    ])
}

/// x 高さ(px。"x" の字の上端。mathtext の get_xheight は PCLT の表が無ければこの値)。
fn x_height(font: &FontVec, size_px: f32) -> f32 {
    let sf = font.as_scaled(px_scale(font, size_px));
    match ink_box(font, &sf.scaled_glyph('x')) {
        Some(b) if b[1] < 0.0 => -b[1],
        _ => 0.5 * size_px,
    }
}

/// "lp" の字の上端・下端(matplotlib は行の高さをこの字で決める)。
fn lp_metrics(font: &FontVec, size_px: f32) -> (f32, f32) {
    let scale = px_scale(font, size_px);
    let sf = font.as_scaled(scale);
    let mut asc = 0.0f32;
    let mut desc = 0.0f32;
    for ch in ['l', 'p'] {
        if let Some(b) = ink_box(font, &sf.scaled_glyph(ch)) {
            asc = asc.max(-b[1]);
            desc = desc.max(b[3]);
        }
    }
    if asc <= 0.0 {
        asc = sf.ascent();
        desc = -sf.descent();
    }
    (asc, desc)
}

/// 文字を 1 行に組む。`spans` の上付きは mathtext と同じく 0.7 倍にして x 高さ × 0.7 持ち上げ、
/// 前に x 高さ × 0.025 × 0.7、後ろに x 高さ × 0.05 の空きを入れる。
pub(crate) fn layout(fonts: &Fonts, spans: &[TextSpan], size_px: f32, bold: bool) -> Line {
    let font = fonts.face(bold);
    let (lp_asc, lp_desc) = lp_metrics(font, size_px);
    let math = spans.iter().any(|s| s.superscript);
    let xh = if math { x_height(font, size_px) } else { 0.0 };
    let mut glyphs = Vec::new();
    let mut x = 0.0f32;
    let mut prev: Option<(GlyphId, f32)> = None;
    for sp in spans {
        let sz = if sp.superscript { size_px * SUP_SCALE } else { size_px };
        let rise = if sp.superscript { SUP1 * xh } else { 0.0 };
        if sp.superscript {
            x += SUP_SCALE * SUP_DELTA * xh;
            prev = None;
        }
        let scale = px_scale(font, sz);
        let sf = font.as_scaled(scale);
        for ch in sp.text.chars() {
            let id = font.glyph_id(ch);
            if let Some((pid, psz)) = prev
                && psz == sz
            {
                x += sf.kern(pid, id);
            }
            let g = Glyph { id, scale, position: point(x, -rise) };
            x += sf.h_advance(id);
            glyphs.push(Placed { bold, glyph: g });
            prev = Some((id, sz));
        }
        if sp.superscript {
            x += SCRIPT_SPACE * xh;
            prev = None;
        }
    }
    // 字形の外接矩形(基準線が 0、下が正)
    let mut ink: Option<[f32; 4]> = None;
    for p in &glyphs {
        if let Some(b) = ink_box(font, &p.glyph) {
            ink = Some(match ink {
                None => b,
                Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
            });
        }
    }
    let (left, width, h, d) = match (ink, math) {
        // 空白だけ・空の文字: 高さは "lp"
        (None, _) => (0.0, x, 0.0, 0.0),
        (Some(b), false) => (0.0, x, b[3] - b[1], b[3]),
        (Some(b), true) => {
            // to_raster: 原点を含めて 1 px ずつ広げる。h = (ymax − ymin) − 箱の深さ、d = (ymax − ymin) − 箱の高さ、
            // 返す高さは h + d、深さは d(箱の高さ・深さは mathtext の Hlist の値 ≒ インクの上・下、0 以上)
            let (x0, x1) = (b[0].min(0.0) - 1.0, b[2].max(0.0) + 1.0);
            let (y0, y1) = (b[1].min(0.0) - 1.0, b[3].max(0.0) + 1.0);
            let (bh, bd) = ((-b[1]).max(0.0), b[3].max(0.0));
            let (hh, dd) = ((y1 - y0) - bd, (y1 - y0) - bh);
            (x0, x1 - x0, hh + dd, dd)
        }
    };
    let h = h.max(lp_asc + lp_desc);
    let d = d.max(lp_desc);
    Line { glyphs, width, left, ascent: h - d, descent: d, lp_ascent: lp_asc }
}

/// 塗りの濃さの画像(0〜1)。`(ox, oy)` は基準線の原点が buffer のどこにあるか。
pub(crate) struct Coverage {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
    /// buffer の左上から見た基準線の原点(px)
    pub ox: f32,
    pub oy: f32,
}

/// 組んだ行を塗りの濃さに(字形の輪郭を ab_glyph で塗る)。
pub(crate) fn rasterize(fonts: &Fonts, line: &Line, sub_x: f32, sub_y: f32) -> Coverage {
    // 小数の位置(sub_x, sub_y)をずらしてから塗ると、置く位置が px の端数まで合う
    let mut outlined = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in &line.glyphs {
        let mut g = p.glyph.clone();
        g.position = point(g.position.x + sub_x, g.position.y + sub_y);
        if let Some(o) = fonts.face(p.bold).outline_glyph(g) {
            let b = o.px_bounds();
            x0 = x0.min(b.min.x);
            y0 = y0.min(b.min.y);
            x1 = x1.max(b.max.x);
            y1 = y1.max(b.max.y);
            outlined.push(o);
        }
    }
    if outlined.is_empty() {
        return Coverage { w: 0, h: 0, data: Vec::new(), ox: 0.0, oy: 0.0 };
    }
    let (bx, by) = (x0.floor(), y0.floor());
    let w = ((x1.ceil() - bx) as usize).clamp(1, 1 << 14);
    let h = ((y1.ceil() - by) as usize).clamp(1, 1 << 14);
    let mut data = vec![0.0f32; w * h];
    for o in outlined {
        let b = o.px_bounds();
        let (gx, gy) = ((b.min.x - bx) as i64, (b.min.y - by) as i64);
        o.draw(|x, y, c| {
            let (px, py) = (gx + x as i64, gy + y as i64);
            if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                let v = &mut data[py as usize * w + px as usize];
                *v = (*v + c).min(1.0);
            }
        });
    }
    Coverage { w, h, data, ox: -bx, oy: -by }
}

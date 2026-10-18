//! 図 1 枚の表示用の中身(画面の Painter で描く形)。zahyou-plot の Figure と同じ考え方で持つ。
//!
//! - 位置はすべて**画像の画素**(0 始まり、x = 列、y = 行。画素の中心が整数)
//! - 線の太さ・印・文字の大きさは **pt**(画面の上で一定。v6 の図と同じく 1 pt = 100/72 px で描く)
//! - 画面への写し方(上下の向き・拡大・移動)は figure.rs が決める
//!
//! 画像は縮小の段(1/1, 1/2, 1/4, …)を前もって作っておく。wgpu はミップマップを作らないので、
//! 全体表示(24 MP を 800 px に縮める)で星が消えたりちらついたりしないように、表示の倍率に
//! 合った段を選んで描く(matplotlib の antialiased 表示に当たる)。

use std::sync::Arc;

use eframe::egui::{Color32, ColorImage};

/// 図の枠の辺(画面で見た向き)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Bottom,
    Right,
    Top,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Center,
    Baseline,
    Bottom,
}

/// 描く物(位置は画素、大きさは pt)
#[derive(Clone, Debug)]
pub enum Prim {
    /// 折れ線(dash があれば破線: 線の長さ・すき間)
    Polyline { pts: Vec<[f64; 2]>, color: Color32, width: f32, dash: Option<[f32; 2]> },
    /// 円(直径)
    Circle { c: [f64; 2], diameter: f32, color: Color32, width: f32 },
    /// ×(斜めの 2 本が収まる正方形の 1 辺)
    Cross { c: [f64; 2], size: f32, color: Color32, width: f32 },
    /// 矢印(先端が to)。軸の太さ・矢じりの幅と長さ・縁の線
    Arrow { from: [f64; 2], to: [f64; 2], color: Color32, tail: f32, head_w: f32, head_l: f32, edge: f32 },
    /// 文字。at から画面の向きで offset(右・上が正)ずらした点に、寄せを合わせて置く
    Text { at: [f64; 2], offset: [f32; 2], text: String, size: f32, color: Color32, bold: bool, halign: HAlign, valign: VAlign },
}

/// 目盛り 1 本(枠の上の印)
#[derive(Clone, Debug)]
pub struct Tick {
    /// 枠の上の位置(画素)
    pub pos: [f64; 2],
    /// 等値線が枠の内側へ向かう向き(画面の x 右・y 上、度)。目盛りは逆向き(外向き)に描く
    pub angle_deg: f64,
}

/// 目盛りの文字(上付きの部分を分けて持つ: 赤経の h m s)
#[derive(Clone, Debug)]
pub struct TickLabel {
    pub side: Side,
    pub pos: [f64; 2],
    /// (文字, 上付きか)
    pub spans: Vec<(String, bool)>,
}

impl TickLabel {
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.0.as_str()).collect()
    }
}

/// 軸名(「赤経 (RA)」など)と出す辺
#[derive(Clone, Debug)]
pub struct AxisLabel {
    pub side: Side,
    pub text: String,
}

/// 座標格子の線・目盛り・目盛りの文字(表示の範囲ごとに作り直す)
#[derive(Clone, Debug, Default)]
pub struct GridSet {
    pub lines: Vec<Prim>,
    pub ticks: Vec<Tick>,
    pub labels: Vec<TickLabel>,
}

/// 表示の範囲 [x0, x1, y0, y1](画素、x0 < x1・y0 < y1)から格子を作り直す関数
pub type Regrid = Arc<dyn Fn([f64; 4]) -> Option<GridSet> + Send + Sync>;

/// 縮小の段を持つ画像(段 0 が原寸)
#[derive(Clone)]
pub struct Pyramid {
    pub width: usize,
    pub height: usize,
    /// 段 k は 2^k 分の 1。行 0 = 画像の y=0 の行(画面の上下は figure.rs が決める)
    pub levels: Vec<ColorImage>,
}

impl Pyramid {
    /// 原寸の画像から縮小の段を作る(長辺が 256 px を切るまで)。透明(NaN の画素)は平均から外す。
    pub fn build(base: ColorImage) -> Pyramid {
        let [w, h] = base.size;
        let mut levels = vec![base];
        loop {
            let last = levels.last().map(|l| l.size).unwrap_or([0, 0]);
            if last[0].max(last[1]) <= 256 || last[0] < 2 || last[1] < 2 {
                break;
            }
            let next = half(levels.last().expect("段は 1 つ以上ある"));
            levels.push(next);
        }
        Pyramid { width: w, height: h, levels }
    }

    /// 画面 1 px あたりの画像の画素数(> 1 なら縮小表示)に合う段
    pub fn level_for(&self, img_px_per_screen_px: f32) -> usize {
        if img_px_per_screen_px.is_nan() || img_px_per_screen_px <= 1.0 {
            return 0;
        }
        let k = img_px_per_screen_px.log2().floor().max(0.0) as usize;
        k.min(self.levels.len().saturating_sub(1))
    }

    /// 持っている画素の合計(メモリの見積もり用)
    pub fn texels(&self) -> usize {
        self.levels.iter().map(|l| l.size[0] * l.size[1]).sum()
    }
}

/// 2×2 の平均で半分にする(端の 1 列・1 行は残りだけで平均)。
fn half(src: &ColorImage) -> ColorImage {
    let [w, h] = src.size;
    let nw = w.div_ceil(2);
    let nh = h.div_ceil(2);
    let mut out = vec![Color32::TRANSPARENT; nw * nh];
    use rayon::prelude::*;
    out.par_chunks_mut(nw).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let mut acc = [0u32; 3];
            let mut n = 0u32;
            let mut seen = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx >= w || sy >= h {
                        continue;
                    }
                    seen += 1;
                    let c = src.pixels[sy * w + sx];
                    if c.a() == 0 {
                        continue;
                    }
                    acc[0] += c.r() as u32;
                    acc[1] += c.g() as u32;
                    acc[2] += c.b() as u32;
                    n += 1;
                }
            }
            // 半分以上が透明なら透明(縁のぎざぎざを抑える)
            *px = if n == 0 || n * 2 < seen {
                Color32::TRANSPARENT
            } else {
                Color32::from_rgb(((acc[0] + n / 2) / n) as u8, ((acc[1] + n / 2) / n) as u8, ((acc[2] + n / 2) / n) as u8)
            };
        }
    });
    ColorImage::new([nw, nh], out)
}

/// 図 1 枚(題・画像・格子・目盛り・印)
#[derive(Clone)]
pub struct Scene {
    /// 題(改行で 2 行)
    pub title: String,
    /// 画像の大きさ(画素)。表示の範囲の基準
    pub width: f64,
    pub height: f64,
    pub image: Option<Pyramid>,
    /// 表示で上下を反転する(行 0 を画面の上に出す)
    pub flip_y: bool,
    pub axis_labels: Vec<AxisLabel>,
    /// 画像全体を表示したときの格子・目盛り
    pub home: GridSet,
    /// 格子より上に描く物(星の丸・中心の ×・矢印・目標の円・TARGET)
    pub prims: Vec<Prim>,
    /// 拡大・移動したときに格子を作り直す関数(無ければ全体表示の格子の線だけを出す)
    pub regrid: Option<Regrid>,
}

impl Scene {
    pub fn empty() -> Scene {
        Scene {
            title: String::new(),
            width: 1.0,
            height: 1.0,
            image: None,
            flip_y: false,
            axis_labels: Vec::new(),
            home: GridSet::default(),
            prims: Vec::new(),
            regrid: None,
        }
    }
}

/// RGBA(straight alpha、行 0 = 画像の y=0)から ColorImage を作る。大きさが合わなければ None。
pub fn rgba_image(width: usize, height: usize, rgba: &[u8]) -> Option<ColorImage> {
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return None;
    }
    let px: Vec<Color32> = rgba.chunks_exact(4).map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
    Some(ColorImage::new([width, height], px))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: usize, h: usize, v: u8) -> Vec<u8> {
        (0..w * h).flat_map(|_| [v, v, v, 255]).collect()
    }

    #[test]
    fn 縮小の段() {
        let img = rgba_image(1000, 600, &gray(1000, 600, 100)).unwrap();
        let p = Pyramid::build(img);
        let sizes: Vec<[usize; 2]> = p.levels.iter().map(|l| l.size).collect();
        assert_eq!(sizes[0], [1000, 600]);
        assert_eq!(sizes[1], [500, 300]);
        assert_eq!(*sizes.last().unwrap(), [250, 150]);
        assert_eq!(p.level_for(0.5), 0);
        assert_eq!(p.level_for(1.9), 0);
        assert_eq!(p.level_for(2.0), 1);
        assert_eq!(p.level_for(100.0), sizes.len() - 1);
        assert_eq!(p.level_for(f32::NAN), 0);
        assert_eq!(p.levels[1].pixels[0], Color32::from_gray(100));
    }

    #[test]
    fn 透明の画素は平均から外す() {
        let rgba = vec![200, 200, 200, 255, 0, 0, 0, 0, 200, 200, 200, 255, 0, 0, 0, 0];
        let img = rgba_image(2, 2, &rgba).unwrap();
        let h = half(&img);
        assert_eq!(h.pixels[0], Color32::from_gray(200));
        assert!(rgba_image(3, 2, &rgba).is_none());
        assert!(rgba_image(0, 2, &[]).is_none());
    }
}

//! zahyou の図: 伸張、赤経赤緯の格子・目盛り・印・矢印の表示リスト、北が上の再投影、PNG の書き出し。
//!
//! Python 版 `dev/zahyou_plot.py`(matplotlib + astropy の WCSAxes + reproject)と同じ図を作る。
//! 値は Python 版と同じになるように、計算の順序と途中の型まで写してある(`rs/testdata/gen_plot.py` の値で確かめる)。
//!
//! - norm.rs       伸張(`_norm`: 3σ クリップの中央値・σ、99.9 パーセント点、AsinhStretch(1.0))と灰色の色表
//! - sexa.rs       60 進の文字列(astropy の `Angle.to_string`・`SkyCoord.to_string("hmsdms")`)
//! - axes.rs       赤経赤緯の格子・目盛り・目盛りの文字(WCSAxes の選び方)
//! - figure.rs     図 2 枚の表示リスト・題・ずれ・解析結果の文(`plot_original`・`plot_north_up`・`report_offsets`・`wcs_summary`)
//! - reproject.rs  北が上・東が左への描き直し(`reproject_interp` と同じ双一次補間)
//! - render.rs     tiny-skia で PNG に(余白・目盛りの文字・題の置き方は matplotlib に合わせる)。
//!   拡大・移動した範囲の書き出し(`RenderOptions::view`)と、描かずに配置だけを求める `layout_with` も
//! - font.rs       游ゴシックの読み込みと文字の塗り(ab_glyph。フォントは同梱しない)
//!
//! 画素は 0 始まり(x = 列、y = 行)、画素の中心が整数。`Figure::flip_y` が true なら行 0 を画面の上に描く
//! (Python の `invert_yaxis`)。大きさ(線の太さ・印・文字)は pt で、表示の拡大に関係なく一定。

#![forbid(unsafe_code)]

pub mod axes;
pub mod figure;
pub mod font;
pub mod norm;
pub mod render;
pub mod reproject;
pub mod sexa;

pub use axes::{AxisLabel, Grid, GridLine, Side, TextSpan, Tick, TickLabel, View, grid_for_view, wcs_grid};
pub use figure::{
    Color, Figure, FigureKind, HAlign, Offsets, Prim, Summary, Theme, VAlign, clip_arrow_to_boundary, figure_north_up,
    figure_original, named, style, title_north_up, title_original, wcs_summary,
};
pub use font::Fonts;
pub use norm::{GRAY_LUT, Norm, compute_norm, compute_norm_f64, gray_rgba, percentile_f32, percentile_f64};
pub use render::{
    AxesBox, Layout, RenderOptions, RgbaImage, axes_box, axes_box_with, figure_size, layout_with, render_png,
    render_png_with, render_rgba, render_rgba_with, save_png,
};
pub use reproject::{NorthUp, mean_pixel_scale_deg, north_up_frame, reproject_interp, reproject_north_up};
pub use sexa::{hmsdms, sexagesimal_string};

use std::fmt;

/// 図を作る・書き出すときの失敗。
#[derive(Clone, Debug, PartialEq)]
pub enum PlotError {
    /// 画像の大きさが 0
    EmptyImage,
    /// 画像の大きさと画素の数が合わない
    BadImage(String),
    /// WCS から図の座標が決められない
    BadWcs(String),
    /// 北が上の図が大きくなりすぎる(壊れた WCS)
    TooLarge { width: f64, height: f64 },
    /// フォントが読めない
    Font(String),
    /// PNG を作れない・書けない
    Render(String),
}

impl fmt::Display for PlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlotError::EmptyImage => write!(f, "画像が空です"),
            PlotError::BadImage(s) => write!(f, "画像が壊れています: {s}"),
            PlotError::BadWcs(s) => write!(f, "WCS から図を作れません: {s}"),
            PlotError::TooLarge { width, height } => {
                write!(f, "北が上の図が大きくなりすぎます({width:.0}×{height:.0} 画素)。WCS がおかしいようです")
            }
            PlotError::Font(s) => write!(f, "フォントを読めません: {s}"),
            PlotError::Render(s) => write!(f, "図を書き出せません: {s}"),
        }
    }
}

impl std::error::Error for PlotError {}

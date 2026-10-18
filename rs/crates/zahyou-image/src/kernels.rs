//! numpy・scipy・astropy と同じ値を返す部品(照合の試験と、ほかの係が使うためのもの)。
//!
//! どれも float32 の画像(行優先)を受け取り、Python 版と同じ順序・同じ途中の型で計算する。
//! 速さのため rayon で並列にしてあるが、足す順序は変えていないので結果は Python と同じ。

pub use crate::filters::{binomial3, gaussian1, median3, uniform3, zoom_linear, zoom_out_len};
pub use crate::prep::{background, kill_hot_pixels, mask_dead_edges, mask_overlay_bands};
pub use crate::stats::{
    ClipStats, mean_f32, median_f32, median_finite_f32, sigma_clipped_stats, sigma_clipped_std, std_f32, sum_f32,
    sum_f64,
};

/// `ndimage.label(binary)`(4 連結)。戻り値は (各画素の番号(0 は背景、1 から), 成分の数)。
pub fn label4(binary: &[bool], ny: usize, nx: usize) -> (Vec<u32>, usize) {
    crate::detect::label4(binary, ny, nx)
}

/// `ndimage.zoom(c, (ny/by, nx/bx), order=1)`。戻り値は (画素, 高さ, 幅)。
pub fn zoom(c: &[f32], by: usize, bx: usize, ny: usize, nx: usize) -> (Vec<f32>, usize, usize) {
    let oy = zoom_out_len(by, ny);
    let ox = zoom_out_len(bx, nx);
    (zoom_linear(c, by, bx, oy, ox), oy, ox)
}

/// 輝度の重み `_COLOR_WEIGHTS / _COLOR_WEIGHTS.sum()`(float32)。
pub fn lum_weights() -> [f32; 3] {
    crate::array::lum_weights()
}

//! zahyou の画像の読み込み・前処理・星の検出。
//!
//! Python 版 `dev/zahyou_core.py` の `load_image_any`・`preprocess`・`detect_sources`・`detect_sources_deep`・
//! `looks_inverted` を移したもの。値は Python 版(numpy・scipy・astropy・Pillow)と同じになるように、
//! 計算の順序と途中の型(float32 / float64)まで写してある(`rs/testdata/gen_image.py` の値で確かめる)。
//!
//! - fits.rs     FITS を自前で読む(2880 バイト単位のヘッダー・HDU の選び方・BZERO/BSCALE/BLANK・.gz)
//! - tile.rs     タイル圧縮(ZIMAGE の BINTABLE。RICE_1・GZIP_1・GZIP_2・NOCOMPRESS と浮動小数の量子化)
//! - raster.rs   PNG・JPEG・TIFF(Pillow の `convert` と同じ値)
//! - jpeg.rs     JPEG の復号(libjpeg-turbo と同じ整数 IDCT・色差の補間・色変換)
//! - array.rs    多次元配列を Python の型の値として取り出す・`_reduce_to_2d`
//! - load.rs     `load_image_any`・`looks_inverted`(ベイヤー・非有限値の補完・明暗の反転)
//! - stats.rs    numpy / astropy と同じ和・中央値・σ クリップ
//! - filters.rs  scipy.ndimage と同じフィルター(中央値・一様・ガウス・二項・zoom)
//! - prep.rs     前処理(背景・ホットピクセル・縁・焼き込み文字)
//! - detect.rs   星の検出(4 連結のラベル・重心・深い検出)
//!
//! 画素は 0 始まり(x = 列、y = 行)。`Image2D` の行 0 は Python 版の `bundle.data` の行 0 と同じ
//! (FITS は先頭の行 = 画像の下、PNG/JPEG/TIFF は画面の上)。表示の上下は `Loaded::flip_y` で持つ。

#![forbid(unsafe_code)]

mod array;
mod detect;
mod filters;
mod fits;
mod jpeg;
pub mod kernels;
mod load;
mod prep;
mod raster;
mod stats;
mod tile;

pub use detect::{detect_raw, detect_sources, detect_sources_deep};
pub use load::{LoadError, load_image_any, looks_inverted};
pub use prep::preprocess;
pub use zahyou_wcs::{Card, CardValue};

/// 2 次元の画像(float32、行優先)。`data[y * width + x]`。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Image2D {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Image2D {
    /// 大きさと画素から作る。画素の数が合わなければ None。
    pub fn new(width: usize, height: usize, data: Vec<f32>) -> Option<Image2D> {
        if width.checked_mul(height)? != data.len() {
            return None;
        }
        Some(Image2D { width, height, data })
    }

    /// (x, y) の画素。範囲外なら None。
    pub fn get(&self, x: usize, y: usize) -> Option<f32> {
        if x < self.width && y < self.height { self.data.get(y * self.width + x).copied() } else { None }
    }

    /// 行 y の画素(範囲外なら空)。
    pub fn row(&self, y: usize) -> &[f32] {
        if y < self.height { &self.data[y * self.width..(y + 1) * self.width] } else { &[] }
    }

    /// numpy の形の順 (高さ, 幅)。
    pub fn shape(&self) -> (usize, usize) {
        (self.height, self.width)
    }
}

/// 読み込んだ画像(Python の `SolveInput`)。
#[derive(Clone, Debug)]
pub struct Loaded {
    /// 星の検出に使う面(ベイヤーの平滑化・非有限値の補完・明暗の反転を済ませたもの)
    pub data: Image2D,
    /// 表示用の面(読み込んだままの値。非有限値もそのまま。`data` と同じ幾何)
    pub display: Image2D,
    /// FITS のヘッダー(画像ファイルなら空)。タイル圧縮なら astropy と同じく画像のヘッダーに戻したもの
    pub header: Vec<Card>,
    /// 読み込み方法の説明(Python 版と同じ文。" / " でつなぐ)
    pub note: String,
    /// 表示するとき上下を反転するか(行 0 が画面の上に来る画像なら true)
    pub flip_y: bool,
}

/// 前処理の結果(Python の `preprocess` の戻り値 (sub, mask, sigma, info))。
#[derive(Clone, Debug)]
pub struct Prep {
    /// 背景を引き、ホットピクセルを消した画像
    pub sub: Image2D,
    /// 使わない画素(縁・焼き込み文字)。`sub.data` と同じ並び
    pub mask: Vec<bool>,
    /// 背景のノイズ σ(マスク外の σ クリップの標準偏差)
    pub sigma: f64,
    /// 何をしたかの説明(Python 版と同じ文)
    pub info: Vec<String>,
}

/// 検出した星 1 つ(Python の `_measure` の dict と `snr`)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    /// 重心(画素、0 始まり)
    pub x: f64,
    pub y: f64,
    /// 領域内の背景差引の値の和
    pub flux: f64,
    /// 領域内の最大値
    pub peak: f64,
    /// 領域の画素数(深い検出では元の画素に直して 4 倍)
    pub area: usize,
    /// 伸び(長軸 / 短軸)
    pub elong: f64,
    /// 2.3548 × √((vx + vy) / 2)(深い検出では 2 倍)
    pub fwhm: f64,
    /// 開口 SNR = flux / (σ √area)
    pub snr: f64,
}

/// 検出の条件(Python の `detect_sources` の引数)。既定値は Python と同じ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectOpts {
    /// これだけ見つかるまでしきい値を下げる(既定 12。Python の run() は 60 で呼ぶ)
    pub min_sources: usize,
    /// 明るい順にこれだけ返す(既定 400)
    pub max_sources: usize,
    /// これより伸びた領域は捨てる(既定 4.0)
    pub max_elongation: f64,
    /// これより小さい領域は捨てる(既定 3 画素)
    pub min_area: usize,
    /// これより SNR が低い領域は捨てる(既定 5.0)
    pub min_snr: f64,
}

impl Default for DetectOpts {
    fn default() -> Self {
        DetectOpts { min_sources: 12, max_sources: 400, max_elongation: 4.0, min_area: 3, min_snr: 5.0 }
    }
}

impl DetectOpts {
    /// min_sources だけ変えたもの(ほかは既定値)。Python の `detect_sources(..., min_sources=n)`。
    pub fn with_min_sources(n: usize) -> DetectOpts {
        DetectOpts { min_sources: n, ..Default::default() }
    }

    /// 深い検出で Python 版が使う条件(まとめた画像に min_sources = 60、ほかは既定値)。
    /// `detect_sources_deep(prep, &DetectOpts::deep())` が Python の `detect_sources_deep(sub, mask, sigma)` と同じ。
    pub fn deep() -> DetectOpts {
        DetectOpts::with_min_sources(60)
    }
}

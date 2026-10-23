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
//! - detect.rs   星の検出(4 連結のラベル・重心)
//! - deep.rs     深い検出(星の幅に合わせてぼかし、局所の雑音の u 倍を超える極大を拾う。2026-10-16 版)
//!
//! 画素は 0 始まり(x = 列、y = 行)。`Image2D` の行 0 は Python 版の `bundle.data` の行 0 と同じ
//! (FITS は先頭の行 = 画像の下、PNG/JPEG/TIFF は画面の上)。表示の上下は `Loaded::flip_y` で持つ。

#![forbid(unsafe_code)]

use std::sync::Arc;

mod array;
mod deep;
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

pub use deep::{detect_deep, detect_deep_raw, detect_sources_deep};
pub use detect::{detect_raw, detect_sources};
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
///
/// `data` と `display` は `Arc` で持つ。ベイヤー・非有限値の補完・明暗の反転のどれも無い画像では、2 つは
/// 同じ配列を指す(写しを作らない。24 MP で 96 MB の確保と直列の memcpy が無くなる)。`Arc` 越しには `&mut` が
/// 取れないので、片方を書き換えて他方まで変わる、ということは型で起きない(書き換えるなら `Arc::make_mut` で写す)。
/// 呼ぶ側は `&loaded.data` を `&Image2D` として渡せる(参照外しの型強制)。
#[derive(Clone, Debug)]
pub struct Loaded {
    /// 星の検出に使う面(ベイヤーの平滑化・非有限値の補完・明暗の反転を済ませたもの)
    pub data: Arc<Image2D>,
    /// 表示用の面(読み込んだままの値。非有限値もそのまま。`data` と同じ幾何)
    pub display: Arc<Image2D>,
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
/// 深い検出(`detect_deep`)の星は、Python の `detect_sources_deep` と同じく意味が少し違う(括弧の中)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    /// 重心(画素、0 始まり)(深い検出: ぼかした像の 3×3 に対数の放物線を当てた頂の位置)
    pub x: f64,
    pub y: f64,
    /// 領域内の背景差引の値の和(深い検出: ぼかした像の頂の値から出したガウスの全光量。並べ替えの目安)
    pub flux: f64,
    /// 領域内の最大値(深い検出: 頂の画素の前処理後の値)
    pub peak: f64,
    /// 領域の画素数(深い検出: FWHM の円の面積。目安)
    pub area: usize,
    /// 伸び(長軸 / 短軸)(深い検出: 1)
    pub elong: f64,
    /// 2.3548 × √((vx + vy) / 2)(深い検出: 2.3548 × 星像の σ。全部の星で同じ値)
    pub fwhm: f64,
    /// 開口 SNR = flux / (σ √area)(深い検出: ぼかした像の頂の値 / その場所の雑音)
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

    /// 以前の深い検出(2×2 にまとめて min_sources = 60 で検出)の条件。今の `detect_sources_deep` が使うのは
    /// `max_sources`(既定 400)だけなので、`detect_sources_deep(prep, &DetectOpts::deep())` は
    /// Python の `detect_sources_deep(sub, mask, sigma)` と同じ。新しく書くなら `detect_deep(prep, &DeepOpts::default())`。
    pub fn deep() -> DetectOpts {
        DetectOpts::with_min_sources(60)
    }
}

/// 深い検出の条件(Python の `detect_sources_deep` の引数)。既定値は Python と同じ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeepOpts {
    /// 明るい順にこれだけ返す(既定 400)
    pub max_sources: usize,
    /// 雑音だけの画像で画像全体に出てよい偽の極大の数の見込み(既定 2.0)。しきい値 u はこれから決める
    pub target_false: f64,
    /// しきい値 u(局所の雑音の倍数)の下限(既定 4.0)。上限は 8 付近
    pub min_u: f64,
}

impl Default for DeepOpts {
    fn default() -> Self {
        DeepOpts { max_sources: 400, target_false: 2.0, min_u: 4.0 }
    }
}

/// 深い検出で捨てた候補の数(理由ごと。Python の info["rejected"])。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DeepRejected {
    /// もっと明るい星との間の谷が浅い(同じ星の肩・光条のこぶ)
    pub saddle: usize,
    /// ぼかした像の曲率で見た伸びが 4 倍を超える(光跡・線)
    pub elong: usize,
    /// 鋭さが明るい星の典型の 2 倍を超える(ベイヤーで十字に広がったホットピクセル)
    pub sharp: usize,
    /// まわりの輪の中央値より十分高くない(大きなかぶりの上の背景の当てはめの残り)
    pub ring: usize,
}

/// 深い検出で測った値(Python の `detect_sources_deep(..., info=dict)` が入れるもの)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeepInfo {
    /// 星像の σ [画素](明るい星から測れなければ None)
    pub s_psf: Option<f64>,
    /// 星像の σ を測るのに使った星の数
    pub n_psf: usize,
    /// ぼかしに使ったガウスの σ [画素](1〜8)
    pub s: f64,
    /// しきい値(局所の雑音の倍数)
    pub u: f64,
    /// ぼかした画像の雑音(σ クリップの標準偏差)
    pub noise: f64,
    /// 候補(しきい値を超える局所の極大。明るい順に max(4000, 10 × max_sources) 個まで)の数
    pub candidates: usize,
    /// 残った星の数(max_sources で切る前)
    pub kept: usize,
    /// 捨てた理由ごとの数
    pub rejected: DeepRejected,
    /// 鋭さ(中心 / 上下左右の平均)の上限(明るい星の典型の 2 倍。測れなければ inf)
    pub sharp_max: f64,
    /// かかった秒数
    pub seconds: f64,
}

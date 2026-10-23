//! zahyou の索引ファイル(astrometry.net の index-41xx / index-52xx-NN)を読む。
//!
//! Python 版の基準実装は `dev/solver_index.py`(形式は docs/index-format.md、実物を読んで全件で確かめたもの)。
//! 同じ入力に同じ結果を返す(試験 `tests/` が Python の出力 `rs/testdata/index_*.json` と突き合わせる)。
//!
//! 読み方の要点
//! - FITS は 2880 バイトのブロックと 80 文字のカード。ヘッダーだけを自前で読み、表は TTYPE1 の名前で探す
//!   (列の型 '16A' などは当てにならない。中身は生のバイト列)。
//! - 表の数値はリトルエンディアン(ENDIAN = '04:03:02:01')。ほかの値のファイルは読まない。
//!   4100 系の等級の表だけは標準の FITS どおりビッグエンディアン。
//! - データはファイル全体の mmap(読み取り専用)から型付きのビュー(bytemuck の検査つきキャスト)で取り、
//!   整数のまま探す。実数へ戻すのは要る分だけ(実数 = minval + 整数 / scale)。mmap するのは Windows だけ
//!   (ほかの OS では表を開かない。理由は `file.rs` の `map_readonly`)。
//! - kd-tree はファイルに入っている木をそのまま使う(ヒープ順の完全二分木。作り直さない)。
//! - quad i のコード = コード木のデータ i、quad の星番号 = 星の木のデータの行番号。
//! - 5200 系のタイル番号は xy 方式の HEALPix(NN = 基本番号·nside² + x·nside + y)。星はタイルの中だけにある。
//!
//! unsafe は mmap の 1 か所だけ(`file.rs` の `map_readonly`。SAFETY の説明はそこ)。Windows だけで使うので、
//! ほかの OS ではこの crate に unsafe は無い(forbid)。

#![cfg_attr(windows, deny(unsafe_code))]
#![cfg_attr(not(windows), forbid(unsafe_code))]

mod catalog;
mod code;
mod file;
pub mod fits;
mod healpix;
mod kdtree;
mod layout;
mod pyfmt;

use std::path::Path;

pub use catalog::{Catalog, QUAD_MIN_FRAC};
pub use code::quad_code_from_xyz;
pub use file::{IndexData, IndexError, IndexFile};
pub use healpix::{
    healpix_xy_center, healpix_xy_from_xyz, healpix_xy_from_xyz_u64, healpix_xy_point, healpix_xy_radius,
    healpix_xy_radius_samples,
};
pub use layout::{match_index_name, ALLSKY_HEALPIX, INDEX_ENDIAN};
pub use zahyou_wcs::Vec3;

/// 索引ファイル 1 本をヘッダーだけで検査する(Python 版 `sv_validate_index_file` の ok / ng)。
///
/// 見ること: ファイル名の形、大きさ = ヘッダーから計算した長さ(途中で切れたダウンロード)、ENDIAN、
/// INDEXID と名前の系列、HEALPIX と名前のタイル番号、必要な表がそろって大きさが合うこと。
/// Err は画面に出す文(Python 版と同じ文面)。
pub fn validate_index_file(path: impl AsRef<Path>) -> Result<(), String> {
    check_index_file(path, None).map(|_| ())
}

/// [`validate_index_file`] と同じ検査をして、通れば中身の要約を返す
/// (例「タイル 2、4 星の quad 193,600 個・星 121,000 個、quad の大きさ 16.0′〜22.0′」)。
///
/// name を渡すと名前の検査にはそちらを使う(ダウンロード中の .part を、名前を変える前に検査するとき)。
pub fn check_index_file(path: impl AsRef<Path>, name: Option<&str>) -> Result<String, String> {
    match layout::read_layout(path.as_ref(), name) {
        Ok(lay) => Ok(layout::summary(&lay)),
        Err(e) => Err(e.message()),
    }
}

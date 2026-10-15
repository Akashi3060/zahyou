//! zahyou の座標系: 天球の単位ベクトル・TAN 投影・WCS(TAN/SIP)・当てはめ・FITS ヘッダーのカード・目標までのずれ。
//! Python 版の基準実装は dev/solver_wcs.py(WCS と当てはめ)と dev/zahyou_plot.py の report_offsets(ずれ)。
//!
//! - geom.rs     単位ベクトルと TAN 投影
//! - sip.rs      SIP の多項式とニュートン法
//! - wcs.rs      `Wcs`(画素 ↔ 天)
//! - header.rs   `Wcs::header_cards`(書く)・`Wcs::from_header_cards`(画像の WCS を読む)
//! - fit.rs      `fit_similarity`・`fit_wcs_lsq`・`fit_sip_inverse`
//! - offsets.rs  `spherical_offsets`・`position_angle`・`offset_by`
//! - linalg.rs   2×2 の SVD・極分解、QR の最小二乗(自前。行列ライブラリは使わない)
#![forbid(unsafe_code)]

pub mod card;
pub mod fit;
pub mod geom;
pub mod header;
pub mod linalg;
pub mod offsets;
pub mod sip;
pub mod wcs;

pub use card::*;
pub use fit::{FitError, fit_similarity, fit_sip_inverse, fit_sip_inverse_bounds, fit_wcs_lsq, try_fit_similarity, try_fit_wcs_lsq};
pub use geom::*;
pub use header::text_wrap;
pub use linalg::Mat2;
pub use offsets::{offset_by, position_angle, spherical_offsets};
pub use sip::{MAX_SIP_ORDER, Sip};
pub use wcs::Wcs;

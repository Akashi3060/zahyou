//! zahyou のネットワークまわり(画面に依存しない)。
//!
//! - 更新: GitHub Releases の最新版と動いている exe を SHA256 で突き合わせ、改名方式で入れ替える([`update`])
//! - 星図データ: astrometry.net 形式の索引ファイル(index-*.fits)を落とす([`index_dl`])
//! - 天体名: Sesame で引いて VizieR の Gaia DR3 で精密化する([`target`])。結果は cache.json に覚える([`cache`])
//! - オンライン解析: nova.astrometry.net の API([`nova`])
//!
//! Python 版の基準実装は gui/zahyou_update.py・gui/zahyou_env.py・dev/zahyou_core.py
//! (internet_available / solve_online / resolve_target_name / _cache_*)。
//! どの関数もブロッキング。呼ぶ側(app)がワーカーのスレッドで動かし、`stop` で中止を伝える。
#![forbid(unsafe_code)]

pub mod cache;
mod http;
pub mod index_dl;
pub mod nova;
pub mod pyfmt;
pub mod target;
pub mod update;

pub use cache::{
    CachedTarget, cache_path, cached_target, remember_scale, remember_target, remembered_scale,
    rig_key, target_key,
};
pub use index_dl::{
    FetchOptions, FetchResult, G41_BASE, LITE_BASE, RECOMMENDED_MIN_FOV, SCALE_FOV, SCALE_MB,
    TILES, check_index_file, check_index_file_full, fetch_index, fetch_index_with, fov_arcmin,
    human, index_names, index_url, index_url_for, parse_index_name, plan_size, recommend_scales,
    recommended_scales, remote_sizes, remote_sizes_urls, scale_fov, scale_mb, scales_mb,
    Validator, validate_with_index,
};
pub use nova::{NovaSolution, OnlineRequest, api_key_from_env, solve_online};
pub use target::{Resolved, resolve_target, resolve_target_name};
pub use update::{
    ApplyOutcome, ReleaseInfo, UpdateCheck, UpdateState, apply_update, check_update,
    cleanup_old_versions, download_update, update_blocked_reason,
};

use std::fmt;
use std::time::Duration;

/// 中止と失敗を分けて返すための誤り。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// `stop()` が真を返したので途中でやめた
    Cancelled,
    /// 失敗。画面とログにそのまま出せる 1 行の説明
    Failed(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Cancelled => write!(f, "中止しました。"),
            NetError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for NetError {}

/// nova.astrometry.net に本当に届くかを見る(google では意味がない)。
///
/// Python 版と同じく `https://nova.astrometry.net/api/` と `/` を 4 秒ずつ試し、
/// どちらかが 2xx/3xx を返せば真。
pub fn internet_available() -> bool {
    internet_available_at(nova::NOVA_URL, Duration::from_secs(4))
}

/// [`internet_available`] の宛先と待ち時間を指定する版(試験用)。
pub fn internet_available_at(base: &str, timeout: Duration) -> bool {
    let base = base.trim_end_matches('/');
    let agent = http::agent(timeout, true);
    [format!("{base}/api/"), format!("{base}/")]
        .iter()
        .any(|url| agent.get(url).call().is_ok())
}

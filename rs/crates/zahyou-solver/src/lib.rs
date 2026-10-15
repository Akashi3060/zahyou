//! 解法(Lang et al. 2010 の quad 法)。Python 版の基準実装 dev/solver_core.py を移す。
//!
//! **いまは API の約束だけを置いた仮の実装**(常に「解なし」を返す)。中身は第 3 段で解法の係が書く。
//! 名前・引数・戻り値は docs/rust-port.md 4.5 節の約束で、画面(app)はこれに向けて書く。
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use zahyou_index::Catalog;
use zahyou_wcs::Wcs;

/// 画像の星 1 つ(0 始まりの画素、明るい順に並べて渡す)。
#[derive(Clone, Copy, Debug)]
pub struct FieldStar {
    pub x: f64,
    pub y: f64,
    pub flux: f64,
}

/// 1 回の探索の条件(Python の sv_solve の引数)。
#[derive(Clone, Debug, Default)]
pub struct SolveRequest {
    pub stars: Vec<FieldStar>,
    pub width: usize,
    pub height: usize,
    /// 画素スケールの範囲 [″/px]。無ければ手持ちの索引から逆算する
    pub scale_lo: Option<f64>,
    pub scale_hi: Option<f64>,
    /// ヒント (赤経 度, 赤緯 度, 半径 度)
    pub hint: Option<(f64, f64, f64)>,
}

/// 解法の調整値(Python の SvParams と同じ既定値)。
#[derive(Clone, Debug)]
pub struct Params {
    pub codetol: f64,
    pub distractor: f64,
    pub verify_pix: f64,
    pub logodds_solve: f64,
    pub logodds_tune: f64,
    pub logodds_bail: f64,
    pub max_quad_stars: usize,
    pub max_verify_stars: usize,
    pub quad_min_frac: f64,
    pub uniformize_boxes: usize,
    pub tweak_order: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            codetol: 0.01,
            distractor: 0.25,
            verify_pix: 1.0,
            logodds_solve: 1e9f64.ln(),
            logodds_tune: 1e6f64.ln(),
            logodds_bail: 1e-100f64.ln(),
            max_quad_stars: 60,
            max_verify_stars: 300,
            quad_min_frac: 0.1,
            uniformize_boxes: 10,
            tweak_order: 2,
        }
    }
}

/// 締め切り・中止・ログの受け渡し。
pub struct Control<'a> {
    pub deadline: Option<Instant>,
    pub cancel: &'a AtomicBool,
    pub log: &'a (dyn Fn(&str) + Sync),
}

impl Control<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

/// 解(Python の SvSolution)。
#[derive(Clone, Debug)]
pub struct Solution {
    pub wcs: Wcs,
    pub logodds: f64,
    pub nmatch: usize,
    pub ndistract: usize,
    pub nfield: usize,
    pub nref: usize,
    pub index_name: String,
    pub quads_tried: u64,
    pub hypotheses: u64,
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveError {
    /// 中止された
    Cancelled,
}

/// 1 回の探索(Python の sv_solve)。解が無ければ Ok(None)。
pub fn solve(
    _req: &SolveRequest,
    _catalog: &Catalog,
    _params: &Params,
    ctl: &Control,
) -> Result<Option<Solution>, SolveError> {
    if ctl.cancelled() {
        return Err(SolveError::Cancelled);
    }
    (ctl.log)("(仮の実装: 解法はまだ移していません)");
    Ok(None)
}

/// オフライン解析の試行のはしご(Python の solve_offline。設計書 4 節)。
pub struct OfflineRequest<'a> {
    pub prep: &'a zahyou_image::Prep,
    /// 標準の検出(明るい順)
    pub sources: &'a [zahyou_image::Source],
    pub width: usize,
    pub height: usize,
    /// 推定した画素スケール [″/px](分かっていれば)
    pub scale: Option<f64>,
    /// 目標の方向をヒントにするとき (赤経, 赤緯, 半径) 度
    pub hint: Option<(f64, f64, f64)>,
    /// 全体の締め切り [秒]
    pub timeout_s: f64,
}

/// オフライン解析の本体。ログは Python 版と同じ形(`[i/N] ラベル  (最大 t 秒)` → `✅ 解析成功 (ラベル)`)。
pub fn solve_offline(
    _req: &OfflineRequest,
    _catalog: &Catalog,
    params: &Params,
    ctl: &Control,
) -> Result<Option<Solution>, SolveError> {
    solve(&SolveRequest::default(), _catalog, params, ctl)
}

//! 解法(Lang et al. 2010 の quad 法)。Python 版の基準実装 dev/solver_core.py の `sv_solve`・`sv_verify_wcs` と、
//! dev/zahyou_core.py の `solve_offline`(試行のはしご)を移したもの(設計書 docs/design-native-solver.md 3 節、
//! docs/rust-port.md 4.5 節)。
//!
//! 流れ(`solve`)
//!   1. 星を明るい順 → 一様化。quad には先頭 max_quad_stars 個、検証には先頭 max_verify_stars 個。
//!      ヒントがあれば、タイルの索引は「その段の quad が照合で残りうるタイル」に絞る。
//!   2. 星を 1 つずつ足し、足した星を含む quad だけを作る(newpoint)。quad が束にたまったら照合へ。
//!   3. 索引ごとに AB の長さで quad を選び、コード(通常と鏡像、正準化の境界の近くは両方の並び)を
//!      code_ball に投げる。当たりはスケールとヒントで安く捨ててから、4 点で回転+等方スケールを当てる。
//!   4. 仮説を系列ごとに newpoint の順で 8192 個ずつ検証する(安い足切り → 対数オッズ)。
//!      tune 以上 solve 未満なら 1 次で改良して検証し直す。
//!   5. solve 以上になった最初の仮説を改良して返す。ただし、改良した後の対数オッズが「試した仮説の数で上げた値」
//!      (`parts::accept_logodds`)に届かないもの、試した仮説が 1e6 個を超えてから tune-up で上がったのに改良で一致が
//!      増えないもの(`parts::tuneup_accepted`)は解にしない(astrometry.net には無い条件。設計書 3.7 節の 6)。
//!
//! 探す順と判定は Python と同じ。並列(rayon)にするのは索引のファイルごとの照合・当たりの安い棄却と当てはめ・
//! 検証の束の中の仮説ごとの検証だけで、結果は Python の順に並べてから選ぶので、スレッドの数によらず同じ解になる。
//! 締め切りと中止は重い処理の区切りごと(code_ball の問い合わせ 512 個(search.rs の BALL_CHUNK。Python は 2048)・仮説 1 つ・
//! タイル 1 枚)に見る。
//! 中身の壊れた索引は、そのファイル(か系列)を外して探索を続ける。
//!
//! 座標の約束は zahyou-wcs と同じ(画素は 0 始まり、x = 列、y = 行。CD は度/px)。
#![forbid(unsafe_code)]
// 比較は「!(a >= b)」の形で NaN を「範囲の外」として扱う(Python の `not (a >= b)` と同じ意味)ので許す
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use zahyou_index::Catalog;
use zahyou_wcs::{Vec3, Wcs};

mod clock;
mod field;
mod lsq;
mod offline;
pub mod parts;
mod pyfmt;
mod search;
mod trace;
mod verify;

use clock::{Abort, Clock};
use pyfmt::float_repr;

pub use offline::{blind_scale_bounds, index_coverage};

/// 画像の星 1 つ(0 始まりの画素、明るい順に並べて渡す)。
#[derive(Clone, Copy, Debug)]
pub struct FieldStar {
    pub x: f64,
    pub y: f64,
    /// 明るさ(並べ替えに使う。NaN・無限大は最後へ。全部 NaN なら渡した順)
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

/// 探す鏡像の向き(Python の SvParams.parity)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Parity {
    /// 両方("both")
    #[default]
    Both,
    /// 配列座標で det CD < 0 だけ("normal")
    Normal,
    /// det CD > 0 だけ("mirror")
    Mirror,
}

impl Parity {
    /// Python の名前("both" / "normal" / "mirror")。
    pub fn name(self) -> &'static str {
        match self {
            Parity::Both => "both",
            Parity::Normal => "normal",
            Parity::Mirror => "mirror",
        }
    }
}

/// 解法の調整値(Python の SvParams と同じ既定値)。
#[derive(Clone, Debug)]
pub struct Params {
    /// コードの許容誤差(4 次元の L2)
    pub codetol: f64,
    /// 画像の星が偽物である割合
    pub distractor: f64,
    /// 星の位置誤差 [px]
    pub verify_pix: f64,
    /// これ以上で解
    pub logodds_solve: f64,
    /// これ以上・solve 未満なら 1 次で改良して検証し直す
    pub logodds_tune: f64,
    /// 検証の打ち切り
    pub logodds_bail: f64,
    /// tune-up で上がった仮説を解にするのに要る、改良での一致の増え(改良後の一致数 ≥ 4 点の検証の一致数 + これ)。
    /// 0 なら確かめない
    pub tune_min_gain: usize,
    /// tune-up の一致の増えを求めるのは、試した仮説の数がこれを超えてから(それまでは今までどおり)。無限大なら求めない
    pub tune_gain_hyps0: f64,
    /// 試した仮説の数 N がこれを超えたら、解とみなす値を logodds_solve + ln(N / これ) に上げる。無限大なら上げない
    pub accept_hyps0: f64,
    /// quad 作りに使う星(並べ替えた順)の上限
    pub max_quad_stars: usize,
    /// 検証に使う星の上限
    pub max_verify_stars: usize,
    /// quad の AB の最小 = quad_min_frac × 短辺
    pub quad_min_frac: f64,
    /// 探索の前の並べ替え(画面全体に散らす)の箱の数
    pub uniformize_boxes: usize,
    /// 改良の SIP の次数(星が足りなければ下げる)
    pub tweak_order: usize,
    /// 探す鏡像の向き
    pub parity: Parity,
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
            tune_min_gain: 1,
            tune_gain_hyps0: 1e6,
            accept_hyps0: 1e5,
            max_quad_stars: 60,
            max_verify_stars: 300,
            quad_min_frac: 0.1,
            uniformize_boxes: 10,
            tweak_order: 2,
            parity: Parity::Both,
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

/// 解(Python の SvSolution)。wcs の crpix は 0 始まり(ヘッダーに書くときに 1 を足す)。
#[derive(Clone, Debug)]
pub struct Solution {
    /// TAN か TAN+SIP
    pub wcs: Wcs,
    /// 改良した後の検証の対数オッズ
    pub logodds: f64,
    /// 一致した画像の星の数(quad の星は含まない)
    pub nmatch: usize,
    /// 偽物とした画像の星の数
    pub ndistract: usize,
    /// 検証に使った画像の星の数(quad の星は含まない)
    pub nfield: usize,
    /// 検証に使った参照星の数(同)
    pub nref: usize,
    /// 解いた索引(例 "index-5205-02")
    pub index_name: String,
    /// 照合に投げた画像の quad の数
    pub quads_tried: u64,
    /// 検証した仮説の数
    pub hypotheses: u64,
    /// かかった時間 [秒]
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveError {
    /// 中止された
    Cancelled,
}

/// 一致した星 1 つ(画像の星の番号は SolveRequest.stars の添字)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarMatch {
    pub field: usize,
    /// 参照星の単位ベクトル
    pub star: Vec3,
    /// ずれ [px]
    pub dist_px: f64,
}

/// [`solve_detailed`] の解: 解と、その素性(Python の SvSolution の quad_field・quad_index・matches)。
#[derive(Clone, Debug)]
pub struct SolveDetail {
    pub solution: Solution,
    /// 画像の星の番号(SolveRequest.stars の添字。索引の quad の A, B, C, D の順)
    pub quad_field: Vec<usize>,
    /// 索引の星の番号
    pub quad_index: Vec<u32>,
    /// 最後の検証で一致した星
    pub matches: Vec<StarMatch>,
}

/// [`solve_detailed`] の追加の指定。
#[derive(Clone, Debug, Default)]
pub struct SolveOptions {
    /// 途中の値(入力の星・試した quad の数・仮説の 4 点の WCS・足切り・対数オッズ・最終の WCS)を
    /// Python の sv_solve の trace と同じ形の JSON に書くパス。書けなくても探索には影響しない
    pub trace: Option<PathBuf>,
}

/// 1 回の探索(Python の sv_solve)。解が無ければ Ok(None)。中止なら Err(Cancelled)。
///
/// ヒントの値がおかしい(数でない・赤緯が ±90° の外・半径が負)ときと、スケール・画像の大きさがおかしいときは、
/// 理由をログに書いて Ok(None)(Python は ValueError)。
pub fn solve(req: &SolveRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<Option<Solution>, SolveError> {
    solve_detailed(req, catalog, params, ctl, &SolveOptions::default()).map(|o| o.map(|d| d.solution))
}

/// [`solve`] の、解の素性(quad の星・一致)も返し、trace も書ける版。
pub fn solve_detailed(
    req: &SolveRequest,
    catalog: &Catalog,
    params: &Params,
    ctl: &Control,
    opts: &SolveOptions,
) -> Result<Option<SolveDetail>, SolveError> {
    let store = verify::Store::new(catalog);
    let log = |s: &str| (ctl.log)(s);
    let clock = Clock::new(ctl.deadline, ctl.cancel);
    match solve_in(req, &store, params, clock, &log, opts.trace.as_deref()) {
        Ok(found) => Ok(found),
        Err(Abort::Cancelled) => Err(SolveError::Cancelled),
        Err(Abort::Invalid(msg)) => {
            log(&msg);
            Ok(None)
        }
        Err(_) => Ok(None),
    }
}

/// ヒントの値を確かめる(Python の sv_solve の冒頭と同じ文)。
fn check_hint(hint: Option<(f64, f64, f64)>) -> Result<(), String> {
    let Some((ra, dec, rad)) = hint else { return Ok(()) };
    if !(ra.is_finite() && dec.is_finite() && rad.is_finite()) {
        return Err(format!(
            "ヒントに数でない値があります(赤経 {}、赤緯 {}、半径 {})",
            float_repr(ra),
            float_repr(dec),
            float_repr(rad)
        ));
    }
    if !(-90.0..=90.0).contains(&dec) {
        return Err(format!("ヒントの赤緯 {}° が −90°〜+90° の外です", float_repr(dec)));
    }
    if rad < 0.0 {
        return Err(format!("ヒントの半径 {}° が負です", float_repr(rad)));
    }
    Ok(())
}

/// 探索 1 回(索引の持ち方を外から渡す。試行のはしごは壊れた索引の記録を持ち越す)。
/// Invalid はヒント・スケールなどの入力がおかしいとき(Python の ValueError)。
pub(crate) fn solve_in(
    req: &SolveRequest,
    store: &verify::Store,
    params: &Params,
    clock: Clock,
    log: &dyn Fn(&str),
    trace: Option<&Path>,
) -> Result<Option<SolveDetail>, Abort> {
    check_hint(req.hint).map_err(Abort::Invalid)?;
    let t0 = Instant::now();
    let mut st = search::SearchState::new(t0, trace.is_some());
    let xy: Vec<[f64; 2]> = req.stars.iter().map(|s| [s.x, s.y]).collect();
    let flux: Vec<f64> = req.stars.iter().map(|s| s.flux).collect();
    let inp = search::SearchInput {
        xy: &xy,
        flux: Some(&flux),
        width: req.width as f64,
        height: req.height as f64,
        scale_lo: req.scale_lo,
        scale_hi: req.scale_hi,
        hint: req.hint,
    };
    let r = search::search(&inp, store, params, clock, log, &mut st);
    let (status, out) = match r {
        Ok(Some(f)) => ("solved", Ok(Some(f))),
        Ok(None) => ("unsolved", Ok(None)),
        Err(Abort::TimeUp) => {
            log(&format!(
                "締め切り: quad {} 個・仮説 {} 個を試して解けず(対数オッズの最大 {:.1}、{:.1} 秒){}",
                pyfmt::fmt_int(st.quads),
                pyfmt::fmt_int(st.hyps),
                st.logodds_max,
                t0.elapsed().as_secs_f64(),
                st.passed.note()
            ));
            ("deadline", Ok(None))
        }
        Err(Abort::Cancelled) => ("cancelled", Err(Abort::Cancelled)),
        Err(e) => ("unsolved", Err(e)),
    };
    if let Some(path) = trace {
        let found = match &out {
            Ok(Some(f)) => Some(f),
            _ => None,
        };
        trace::write_trace(path, &st, params, found, status, req, &xy, &flux);
    }
    out.map(|o| {
        o.map(|f| SolveDetail {
            solution: Solution {
                wcs: f.wcs,
                logodds: f.logodds,
                nmatch: f.nmatch,
                ndistract: f.ndistract,
                nfield: f.nfield,
                nref: f.nref,
                index_name: f.index_name,
                quads_tried: f.quads_tried,
                hypotheses: f.hypotheses,
                seconds: f.seconds,
            },
            quad_field: f.quad_field,
            quad_index: f.quad_index,
            matches: f.matches.into_iter().map(|(i, v, d)| StarMatch { field: i, star: v, dist_px: d }).collect(),
        })
    })
}

// ------------------------------------------------------------------ 与えた WCS の検証 ---

/// [`verify_wcs`] の結果(Python の sv_verify_wcs の戻り値)。
#[derive(Clone, Debug)]
pub struct VerifyReport {
    pub logodds: f64,
    pub nmatch: usize,
    pub ndistract: usize,
    /// 検証に使った画像の星の数
    pub nfield: usize,
    /// 検証に使った参照星の数
    pub nref: usize,
    pub matches: Vec<StarMatch>,
}

/// 与えた WCS をそのまま検証して対数オッズを返す(Python の sv_verify_wcs。試験と、Python と同じ値が出るかの
/// 確かめ用。探索では使わない)。
///
/// 参照星は系列 indexid の全タイルから集める(探索の検証と同じ)。quad = (画像の星の番号, 索引の名前,
/// 索引の星の番号) を渡すと、探索と同じく σ の広がりと RoR を quad から決め、quad の星を両側から外す
/// ([`SolveDetail`] の quad_field・index_name・quad_index をそのまま渡せば、解の logodds と同じ値になる)。
/// quad が無ければ σ = pix(広がりなし)、RoR は画像全体。系列・索引が無い、quad の形がおかしいときは Err。
#[allow(clippy::too_many_arguments)]
pub fn verify_wcs(
    stars: &[FieldStar],
    width: usize,
    height: usize,
    catalog: &Catalog,
    indexid: u32,
    wcs: &Wcs,
    quad: Option<(&[usize], &str, &[u32])>,
    params: &Params,
) -> Result<VerifyReport, String> {
    let store = verify::Store::new(catalog);
    let files = catalog.series.get(&indexid).filter(|f| !f.is_empty()).ok_or_else(|| format!("系列 {indexid} の索引がありません"))?;
    let xy: Vec<[f64; 2]> = stars.iter().map(|s| [s.x, s.y]).collect();
    let flux: Vec<f64> = stars.iter().map(|s| s.flux).collect();
    let field = field::Field::new(&xy, Some(&flux), width as f64, height as f64, params);
    let sc = verify::SeriesCtx::new(&store, indexid, files[0]);
    let (east, north) = zahyou_wcs::tan_basis(wcs.crval);
    let mut h = verify::Hyp {
        crval: wcs.crval,
        east,
        north,
        cd: wcs.cd,
        cdinv: zahyou_wcs::linalg::inv2(&wcs.cd),
        crpix: wcs.crpix,
        scale: wcs.pixel_scale_arcsec(),
        dq: 0,
        qfield: [usize::MAX; 4],
        qindex: [u32::MAX; 4],
        fid: usize::MAX,
        qc: field.center,
        q2: f64::INFINITY,
        row: 0,
        mirror: false,
    };
    let mut gamma = 0.0;
    if let Some((qf, name, qi)) = quad {
        if qf.len() != qi.len() || !(3..=4).contains(&qf.len()) {
            return Err(format!("quad の星の数が合いません(画像 {}、索引 {})", qf.len(), qi.len()));
        }
        if let Some(&bad) = qf.iter().find(|&&i| i >= xy.len()) {
            return Err(format!("quad の画像の星の番号 {bad} が星の数 {} の外です", xy.len()));
        }
        let fid = files
            .iter()
            .position(|&i| catalog.files[i].name == name)
            .ok_or_else(|| format!("系列 {indexid} に索引 {name} がありません"))?;
        let (a, b) = (xy[qf[0]], xy[qf[1]]);
        h.dq = qf.len();
        h.qfield[..h.dq].copy_from_slice(qf);
        h.qindex[..h.dq].copy_from_slice(qi);
        h.fid = fid;
        h.qc = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        h.q2 = ((b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1])) / 4.0;
        gamma = 1.0;
    }
    let res = verify::verify_one(&field, &sc, params, &h, verify::Proj::Wcs(wcs), None, gamma, true).map_err(|e| match e {
        Abort::Corrupt { msg, .. } => format!("星図データが壊れています: {msg}"),
        Abort::Unavailable { msg, .. } => format!("星図データを今は開けません: {msg}"),
        other => format!("{other:?}"),
    })?;
    Ok(VerifyReport {
        logodds: res.best,
        nmatch: res.nmatch,
        ndistract: res.ndistract,
        nfield: res.nt,
        nref: res.nr,
        matches: res.matches.iter().map(|m| StarMatch { field: m.0, star: m.1, dist_px: m.2 }).collect(),
    })
}

// ------------------------------------------------------------------ オフライン解析 ---

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
///
/// 条件(ヒント+スケール既知 → ヒント+スケール全域 → 全天+スケール既知 → 全天+スケール全域)ごとに、
/// 深い検出の星 → 標準の検出の星の順に [`solve`] を試す。各試行には残り時間の一部を割り当てる。
/// 事前のログ(画素スケール・手持ちの段)と、解けなかったときの案内・「■ 中止しました。」は呼ぶ側(app)が出す。
/// 終わったら索引の mmap を放す(Python の finally の catalog.close())。
/// 案内に要る「深い検出の星の数」「どこで止まったか」も欲しいときは [`solve_offline_detailed`]。
pub fn solve_offline(
    req: &OfflineRequest,
    catalog: &Catalog,
    params: &Params,
    ctl: &Control,
) -> Result<Option<Solution>, SolveError> {
    solve_offline_detailed(req, catalog, params, ctl).map(|o| o.solution)
}

/// オフライン解析([`solve_offline_detailed`])がどこで止まったか。呼ぶ側が解けなかったときの案内を選ぶのに使う
/// (Python の `_solve_offline` は、星が足りない・途中で失敗したときは案内を出さずに戻る)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OfflineStop {
    /// 試行のはしごを試した(解けた・全部試した・時間切れ)
    #[default]
    Tried,
    /// 3 個以上の星の組が無かった。星が 1 個以上なら「星が N 個しかない」はログに出してある。
    /// 1 つも無いとき(「星が 1 つも無い」)の文は呼ぶ側が出す
    TooFewStars,
    /// 使える索引が 1 つも無かった(文は呼ぶ側が出す)
    NoIndex,
    /// 途中で失敗した(panic。「オフライン解析が途中で失敗しました」はログに出してある)
    Failed,
}

/// [`solve_offline_detailed`] の結果。
#[derive(Clone, Debug, Default)]
pub struct OfflineOutcome {
    /// 解(解けなければ None)
    pub solution: Option<Solution>,
    /// 深い検出の星の数(検出できなかった・失敗したときは 0)。案内の「写っている星が N 個」は
    /// Python と同じく max(標準の検出の数, これ)
    pub n_deep: usize,
    pub stop: OfflineStop,
}

/// [`solve_offline`] の、止まった理由と深い検出の星の数も返す版。中止なら Err(Cancelled)。
pub fn solve_offline_detailed(
    req: &OfflineRequest,
    catalog: &Catalog,
    params: &Params,
    ctl: &Control,
) -> Result<OfflineOutcome, SolveError> {
    // Python と同じく、途中で失敗しても(panic でも)呼ぶ側へは「解なし」で返す
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| offline::run(req, catalog, params, ctl)))
        .unwrap_or_else(|p| {
            (ctl.log)(&format!("\n  ❌ オフライン解析が途中で失敗しました: panic: {}", offline::panic_text(&*p)));
            Ok(OfflineOutcome { stop: OfflineStop::Failed, ..Default::default() })
        });
    release_in_background(catalog);
    r
}

/// 索引の mmap を放す(Catalog::close)。mmap を外す仕事(触ったページの多い大きな索引では 0.1 秒ほど)は
/// 別のスレッドで行い、呼ぶ側(中止の 0.2 秒以内の約束)を待たせない。開いている表のハンドルを先に集めてから
/// 目録の側を放し、集めたハンドルを別のスレッドで捨てる(最後のハンドルが消えたときに mmap が外れる)。
fn release_in_background(catalog: &Catalog) {
    let held: Vec<std::sync::Arc<zahyou_index::IndexData>> =
        catalog.files.iter().filter(|f| f.is_open()).filter_map(|f| f.data().ok()).collect();
    catalog.close();
    if !held.is_empty() {
        // スレッドを作れなければ、ここで捨てる(spawn が失敗するとクロージャごと捨てられる)
        let _ = std::thread::Builder::new().name("zahyou-index-unmap".to_string()).spawn(move || drop(held));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_messages_like_python() {
        assert!(check_hint(None).is_ok());
        assert!(check_hint(Some((75.0, 35.0, 0.0))).is_ok());
        assert_eq!(
            check_hint(Some((f64::NAN, 35.0, 2.0))).unwrap_err(),
            "ヒントに数でない値があります(赤経 nan、赤緯 35.0、半径 2.0)"
        );
        assert_eq!(check_hint(Some((255.0, 145.0, 2.0))).unwrap_err(), "ヒントの赤緯 145.0° が −90°〜+90° の外です");
        assert_eq!(check_hint(Some((75.0, 35.0, -1.0))).unwrap_err(), "ヒントの半径 -1.0° が負です");
    }

    #[test]
    fn empty_catalog_gives_none_without_panic() {
        let cat = Catalog::scan(std::env::temp_dir().join("zahyou-solver-no-such-dir"));
        let cancel = AtomicBool::new(false);
        let lines = std::sync::Mutex::new(Vec::<String>::new());
        let log = |s: &str| lines.lock().unwrap().push(s.to_string());
        let ctl = Control { deadline: None, cancel: &cancel, log: &log };
        let stars: Vec<FieldStar> =
            (0..20).map(|i| FieldStar { x: 10.0 + 37.0 * i as f64, y: 5.0 + 23.0 * i as f64, flux: 100.0 - i as f64 }).collect();
        let req = SolveRequest { stars, width: 968, height: 548, ..Default::default() };
        assert!(matches!(solve(&req, &cat, &Params::default(), &ctl), Ok(None)));
        // 中止が立っていれば Err
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(solve(&req, &cat, &Params::default(), &ctl).unwrap_err(), SolveError::Cancelled);
    }

    /// ほかのプロセスが書き込み用に開いている索引(Windows では共有違反で開けない)は「壊れている」とせず、
    /// その探索の中でだけ外す。ほかの索引で解け、閉じられたあとの次の試行(同じ Store)では開き直して使う。
    /// 星図データ(index-4113〜4117)と rs/testdata/solver_synth.json の corrupt の星野が無ければ SKIP。
    #[test]
    fn index_open_for_writing_is_not_called_broken() {
        if !cfg!(windows) {
            // 共有モードで書き込み中の索引を断るのも、索引の表を開くのも Windows だけ(zahyou-index の map_readonly)
            eprintln!("SKIP: index_open_for_writing_is_not_called_broken: Windows 以外では索引の表を開かない");
            return;
        }
        let idx = std::env::var_os("ZAHYOU_INDEX_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\AstrometryData"));
        let tags = ["4113", "4114", "4115", "4116", "4117"];
        if tags.iter().any(|t| !idx.join(format!("index-{t}.fits")).is_file()) {
            eprintln!("SKIP: index_open_for_writing_is_not_called_broken: index-4113〜4117 が無い");
            return;
        }
        let td = std::env::var_os("ZAHYOU_TESTDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata"));
        let Ok(text) = std::fs::read_to_string(td.join("solver_synth.json")) else {
            eprintln!("SKIP: index_open_for_writing_is_not_called_broken: solver_synth.json が無い");
            return;
        };
        let doc: serde_json::Value = serde_json::from_str(&text).expect("solver_synth.json");
        let c = &doc["corrupt"];
        if c.is_null() {
            eprintln!("SKIP: index_open_for_writing_is_not_called_broken: corrupt の星野が無い");
            return;
        }
        let f = |v: &serde_json::Value| v.as_f64().expect("数");
        let stars: Vec<FieldStar> = c["xy"]
            .as_array()
            .expect("xy")
            .iter()
            .zip(c["flux"].as_array().expect("flux"))
            .map(|(p, fl)| FieldStar { x: f(&p[0]), y: f(&p[1]), flux: f(fl) })
            .collect();
        let req = SolveRequest {
            stars,
            width: c["width"].as_u64().expect("width") as usize,
            height: c["height"].as_u64().expect("height") as usize,
            scale_lo: c["scale_lo"].as_f64(),
            scale_hi: c["scale_hi"].as_f64(),
            hint: None,
        };
        let base = std::env::temp_dir().join(format!("zahyou-solver-busy-{}", std::process::id()));
        for (sub, use_tags) in [("five", &tags[..]), ("only4116", &tags[3..4])] {
            let dir = base.join(sub);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("一時フォルダー");
            for t in use_tags {
                std::fs::copy(idx.join(format!("index-{t}.fits")), dir.join(format!("index-{t}.fits"))).expect("写す");
            }
            let cat = Catalog::scan(&dir);
            // std の OpenOptions は共有を全部許して開く(ほかのプロセスが書き込み用に開いているのと同じ)
            let writer = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(dir.join("index-4116.fits"))
                .expect("書き込み用に開く");
            let lines = std::sync::Mutex::new(Vec::<String>::new());
            let log = |s: &str| lines.lock().unwrap().push(s.to_string());
            let cancel = AtomicBool::new(false);
            let store = verify::Store::new(&cat);
            let deadline = Some(Instant::now() + std::time::Duration::from_secs(120));
            let r = solve_in(&req, &store, &Params::default(), Clock::new(deadline, &cancel), &log, None);
            let got = r.expect("探索").map(|d| d.solution.index_name);
            {
                let ls = lines.lock().unwrap();
                assert!(!ls.iter().any(|l| l.contains("壊れ")), "{sub}: 「壊れている」と言った: {ls:?}");
                assert!(ls.iter().any(|l| l.contains("今は開けない") && l.contains("index-4116")), "{sub}: 知らせていない: {ls:?}");
            }
            assert!(!store.is_broken(cat.files.iter().position(|x| x.name == "index-4116").expect("4116")));
            if sub == "five" {
                let name = got.expect("ほかの索引で解けるはず");
                assert_ne!(name, "index-4116");
            } else {
                assert_eq!(got, None, "開けない索引しか無いので解けないはず");
            }
            // 閉じたあとの次の試行(同じ Store)では開き直して使う
            drop(writer);
            if sub == "only4116" {
                lines.lock().unwrap().clear();
                let r = solve_in(&req, &store, &Params::default(), Clock::new(deadline, &cancel), &log, None);
                let got = r.expect("探索").map(|d| d.solution.index_name);
                assert_eq!(got.as_deref(), Some("index-4116"), "{:?}", lines.lock().unwrap());
            }
            cat.close();
            drop(store);
            drop(cat);
            let _ = std::fs::remove_dir_all(&dir);
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}

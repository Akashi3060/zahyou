//! オフライン解析の段階的な試行(Python の dev/zahyou_core.py の `_solve_offline` の、試行の部分)。
//!
//! 星はディープ検出(`detect_sources_deep`)の星を先に、標準の検出の星を予備に使う。条件は探す範囲の狭い(速い)
//! ものから順に、条件ごとに ディープ → 標準 の星で試す:
//!   座標ヒント + スケール既知(残り時間の 25 %)→ 座標ヒント + スケール全域(25 %)→ 全天 + スケール既知(25 %)
//!   → 全天 + スケール全域(50 %。最後の試行は残り全部)
//! 全体のタイムアウトは timeout_s 秒の壁時計で、試行ごとに残り時間の一部を割り当てる。残りが 2 秒を切ったらやめる。
//!
//! 試行の前に、前処理の結果に 3×3 の中央値フィルターをかけて(`zahyou_image::median_prep`)星を検出し直す。
//! 元の画像の明るい星の多くがフィルター後の明るい星から外れるとき(ダーク補正をしていない画像のホットピクセルなど、
//! 星より小さい点が明るい星として検出されているとき。フィルター後の星が少なすぎるときは見ない)は、条件ごとに、
//! フィルター後の星(ディープ → 標準)を元の星より先に試す。フィルター後の星の試行 1 回に使う時間は残りの 1 割まで
//! (`MEDIAN_SHARE`。判定は元の星ですぐ解ける画像でも通ることがあるので、そうした画像を長く待たせない)。
//! 判定にかかるのは中央値フィルター 1 回と検出 1 回(2 MP で 20 ms ほど、24 MP で 0.2 秒ほど)。
//!
//! モジュールの分担(app/src/pipeline.rs): 事前のログ(画素スケール・手持ちのスケール・足りないスケール・探索するスケール)と、
//! 解けなかったときの案内(「❌ すべての条件で解けませんでした」以下)と、「■ 中止しました。」は app が出す。ここが出すのは、
//! ディープ検出・中央値フィルターの判定(「元の画像の明るい星 N 個のうち M 個が…」の 2 行)とフィルター後のディープ検出・
//! `[i/N] ラベル  (最大 t 秒)`・探索のログ(`    · ` を付けて)・解の要約・`✅ 解析成功 (ラベル)`・時間切れ・エンジンのエラーと、
//! 判定が通って解けなかったときの「中央値フィルターをかけた画像でも解けませんでした。ダーク補正をした画像で試してください。」
//! (app の案内の前に出る)。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::{Duration, Instant};

use zahyou_image::{DeepInfo, DeepOpts, DetectOpts, Source};
use zahyou_index::Catalog;

use crate::clock::{Abort, Clock};
use crate::parts::py_max;
use crate::pyfmt::{fmt_g, fmt_int};
use crate::verify::Store;
use crate::{Control, FieldStar, OfflineOutcome, OfflineRequest, OfflineStop, Params, Solution, SolveError, SolveRequest};

/// インデックスがカバーする画角(分角)。astrometry.net の付番規則(番号の下 2 桁 → スケール。Python の `_INDEX_SCALES`)。
const INDEX_SCALES: [(f64, f64); 20] = [
    (2.0, 2.8),
    (2.8, 4.0),
    (4.0, 5.6),
    (5.6, 8.0),
    (8.0, 11.0),
    (11.0, 16.0),
    (16.0, 22.0),
    (22.0, 30.0),
    (30.0, 42.0),
    (42.0, 60.0),
    (60.0, 85.0),
    (85.0, 120.0),
    (120.0, 170.0),
    (170.0, 240.0),
    (240.0, 340.0),
    (340.0, 480.0),
    (480.0, 680.0),
    (680.0, 1000.0),
    (1000.0, 1400.0),
    (1400.0, 2000.0),
];

/// index-XXXX[-YY].fits の名前の並びから、扱える画角(分角)の範囲(Python の `index_coverage`)。
pub fn index_coverage<S: AsRef<str>>(names: &[S]) -> Option<(f64, f64)> {
    let mut out: Option<(f64, f64)> = None;
    for name in names {
        let base = Path::new(name.as_ref()).file_name().and_then(|s| s.to_str()).unwrap_or("");
        let Some(rest) = base.strip_prefix("index-") else { continue };
        let num = rest.split('-').next().unwrap_or("").split('.').next().unwrap_or("");
        if num.len() < 2 || !num.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(k) = num[num.len() - 2..].parse::<usize>() else { continue };
        let Some(&(lo, hi)) = INDEX_SCALES.get(k) else { continue };
        out = Some(match out {
            None => (lo, hi),
            Some((a, b)) => (a.min(lo), b.max(hi)),
        });
    }
    out
}

/// 画角が全く分からないときに探す画素スケールの範囲 [″/px](Python の zahyou_core の `blind_scale_bounds`)。
/// 手持ちのインデックスが扱える画角から画像の幅で逆算する(下限 × 0.9、上限 × 1.1)。インデックスが無ければ 2′〜2000′ で。
pub fn blind_scale_bounds<S: AsRef<str>>(width_px: usize, names: &[S]) -> (f64, f64) {
    let (lo_fov, hi_fov) = index_coverage(names).unwrap_or((2.0, 2000.0));
    let low = lo_fov * 60.0 / width_px as f64 * 0.9;
    let high = hi_fov * 60.0 / width_px as f64 * 1.1;
    (if 0.01 > low { 0.01 } else { low }, if 3600.0 < high { 3600.0 } else { high })
}

/// 目録に入っている(検査に通った)インデックスファイルの名前(拡張子つき、名前の順。Python の `_catalog_names`)。
fn catalog_names(cat: &Catalog) -> Vec<String> {
    let mut v: Vec<String> =
        cat.files.iter().filter_map(|f| f.path.file_name().map(|s| s.to_string_lossy().into_owned())).collect();
    v.sort();
    v.dedup();
    v
}

/// ディープ検出のぼかし方の説明(Python の `星の幅 (FWHM … px) に合わせてぼかし` / `星の幅を測れなかったので σ … px でぼかし`)。
///
/// ディープ検出が測った値(DeepInfo の s_psf・s)から Python と同じ式で書く(Python の `if s_psf` と同じく 0 は「測れなかった」)。
fn deep_how(info: Option<&DeepInfo>) -> String {
    match info.and_then(|i| i.s_psf).filter(|&s| s != 0.0) {
        Some(s_psf) => format!("星の幅 (FWHM {:.1} px) に合わせてぼかし", 2.3548 * s_psf),
        None => format!("星の幅を測れなかったので σ {:.1} px でぼかし", info.map_or(1.5, |i| i.s)),
    }
}

/// panic の中身の文字列。
pub(crate) fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "(理由不明)".to_string()
    }
}

/// 解の中身を 1 行に(Python の `_solution_note`)。
fn solution_note(s: &Solution) -> String {
    let mut parts = Vec::new();
    if !s.index_name.is_empty() {
        parts.push(format!("インデックス {}", s.index_name));
    }
    parts.push(format!("一致 {} 個", s.nmatch));
    parts.push(format!("対数オッズ {:.1}", s.logodds));
    parts.push(format!("quad {} 個", fmt_int(s.quads_tried)));
    parts.push(format!("{:.1} 秒", s.seconds));
    parts.join(" / ")
}

/// 解の WCS がヘッダーに書ける値か(Python は astropy の fits.Header と WCS で確かめる)。
fn wcs_problem(s: &Solution) -> Option<String> {
    let w = &s.wcs;
    let finite = w.crval.iter().chain(w.crpix.iter()).chain(w.cd.iter().flatten()).all(|v| v.is_finite());
    if !finite {
        return Some("ValueError: WCS の値に有限でないものがあります".to_string());
    }
    let det = w.cd[0][0] * w.cd[1][1] - w.cd[0][1] * w.cd[1][0];
    if !(det != 0.0 && det.is_finite()) {
        return Some("ValueError: 天球座標の WCS になっていません".to_string());
    }
    None
}

/// 秒数を Duration に(負・NaN は 0、大きすぎる値は抑える)。
fn secs(t: f64) -> Duration {
    Duration::try_from_secs_f64(if t > 0.0 { t.min(1e9) } else { 0.0 }).unwrap_or(Duration::ZERO)
}

/// 試行 1 つ。
struct Attempt<'s> {
    label: String,
    stars: &'s [FieldStar],
    hint: Option<(f64, f64, f64)>,
    range: (f64, f64),
    share: f64,
}

/// 条件 1 つ: (名前, ヒント, スケールの範囲, 残り時間の何割まで使うか)。
type Cond<'c> = (&'c str, Option<(f64, f64, f64)>, (f64, f64), f64);
/// 星の組 1 つ: (名前, 星)。
type StarSet<'n, 's> = (&'n str, &'s [FieldStar]);

/// 試行の並びを作る(Python の `_build_attempts`): 条件ごとに、フィルター後の星の組(判定が通ったときだけある)→
/// 元の星の組の順。フィルター後の星の試行が使う時間は残りの MEDIAN_SHARE までに抑える(元の星の試行は条件どおり)。
fn build_attempts<'s>(
    conds: &[Cond<'_>],
    f_sets: &[StarSet<'_, 's>],
    star_sets: &[StarSet<'_, 's>],
) -> Vec<Attempt<'s>> {
    let sets: Vec<_> = f_sets.iter().map(|&s| (s, MEDIAN_SHARE)).chain(star_sets.iter().map(|&s| (s, 1.0))).collect();
    conds
        .iter()
        .flat_map(|&(cond, hint, range, share)| {
            sets.iter().map(move |&((name, stars), cap)| Attempt {
                label: format!("{name}の星 {} 個 + {cond}", stars.len()),
                stars,
                hint,
                range,
                share: share.min(cap),
            })
        })
        .collect()
}

/// 段階的な試行の本体(Python の `_solve_offline` の試行の部分)。
pub(crate) fn run(req: &OfflineRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<OfflineOutcome, SolveError> {
    let t_start = Instant::now();
    let t_end = t_start + secs(req.timeout_s);
    let log = |s: &str| (ctl.log)(s);
    let nx = req.width;
    let installed = catalog_names(catalog);
    if installed.is_empty() {
        // 「使えるインデックスファイルがありません」は app が出す
        return Ok(OfflineOutcome { stop: OfflineStop::NoIndex, ..Default::default() });
    }
    // Python の `if scale` と同じ(0 と None は不明、NaN はそのまま渡してエンジンのエラーになる)
    let scale = req.scale.filter(|&s| s != 0.0);
    let narrow = scale.map(|s| (py_max(s * 0.75, 0.01), s * 1.25));
    let wide = blind_scale_bounds(nx, &installed);

    if ctl.cancelled() {
        return Err(SolveError::Cancelled);
    }
    // ディープ検出(解析エンジンのために暗い星まで検出し直した星)。失敗しても標準の検出の星で続ける。
    // Python の detect_sources_deep(*prep, info=dinfo) と同じ既定(400 個・偽の極大 2 個・u ≥ 4)
    let deep = catch_unwind(AssertUnwindSafe(|| zahyou_image::detect_deep(req.prep, &DeepOpts::default())));
    let (deep_src, u, dinfo) = match deep {
        Ok(r) => r,
        Err(p) => {
            log(&format!("\n  ⚠️ ディープ検出に失敗したので、標準の検出の星だけで解きます: panic: {}", panic_text(&*p)));
            (Vec::new(), None, None)
        }
    };
    if !deep_src.is_empty() {
        log(&format!(
            "\n  - ディープ検出 (解析エンジンに渡す星): {}、星を {} 個検出 (しきい値 {}σ)",
            deep_how(dinfo.as_ref()),
            deep_src.len(),
            u.map_or("None".to_string(), |u| fmt_g(u, 6))
        ));
    } else if dinfo.is_some() {
        // Python の `elif dinfo:`(検出を最後まで進めて、星が 0 個だったとき)
        log("\n  - ディープ検出: 星が見つかりませんでした。");
    }
    let to_stars = |s: &[Source]| -> Vec<FieldStar> { s.iter().map(|c| FieldStar { x: c.x, y: c.y, flux: c.flux }).collect() };
    let deep_stars = to_stars(&deep_src);
    let std_stars = to_stars(req.sources);
    let star_sets: Vec<StarSet> = [("ディープ検出", &deep_stars[..]), ("標準の検出", &std_stars[..])]
        .into_iter()
        .filter(|(_, s)| s.len() >= 3)
        .collect();
    let n_deep = deep_src.len();
    if star_sets.is_empty() {
        let n_max = req.sources.len().max(n_deep);
        if n_max > 0 {
            // 0 個のとき(「星が 1 つも無い」)は app の案内が出す
            log(&format!("  ❌ 星が {n_max} 個しかないので解析できません (3 個以上要ります)。"));
        }
        return Ok(OfflineOutcome { solution: None, n_deep, stop: OfflineStop::TooFewStars });
    }
    let tried = |solution: Option<Solution>| OfflineOutcome { solution, n_deep, stop: OfflineStop::Tried };

    // 段階的な試行: (条件, ヒント, スケールの範囲, 残り時間の何割まで使うか)
    let hint = req.hint;
    let mut conds: Vec<Cond> = Vec::new();
    if let (Some(h), Some(n)) = (hint, narrow) {
        conds.push(("座標ヒント + スケール既知", Some(h), n, 0.25));
    }
    if let Some(h) = hint {
        conds.push(("座標ヒント + スケール全域", Some(h), wide, 0.25));
    }
    if let Some(n) = narrow {
        conds.push(("全天 + スケール既知", None, n, 0.25));
    }
    conds.push(("全天 + スケール全域", None, wide, 0.5));
    // ホットピクセルの多い画像の手当て(Python の `_median_sets`): 前処理の結果に 3×3 の中央値フィルターをかけて
    // 星を検出し直し、元の画像の明るい星の多くがフィルター後の明るい星から外れる(= 星より小さい点が明るい星として
    // 検出されている)ときは、フィルター後の星の試行を条件ごとに先に行う
    let fprep = zahyou_image::median_prep(req.prep);
    let (f_std_src, _) = zahyou_image::detect_sources(&fprep, &DetectOpts::with_min_sources(60));
    if ctl.cancelled() {
        return Err(SolveError::Cancelled);
    }
    let hot = hot_point_count(req.sources, &f_std_src);
    let (f_deep_stars, f_std_stars) = match hot {
        Some((top, lost)) => {
            log(&format!(
                "\n  - 元の画像の明るい星 {top} 個のうち {lost} 個が、3×3 の中央値フィルターをかけると明るい星から外れます。"
            ));
            log(
                "    ホットピクセルなど星より小さい点が星として検出されているとみて、中央値フィルターをかけた画像の星を先に試します。",
            );
            let f_deep = catch_unwind(AssertUnwindSafe(|| zahyou_image::detect_deep(&fprep, &DeepOpts::default())));
            let (f_deep_src, fu, fdinfo) = match f_deep {
                Ok(r) => r,
                Err(p) => {
                    log(&format!("  ⚠️ 中央値フィルター後のディープ検出に失敗しました: panic: {}", panic_text(&*p)));
                    (Vec::new(), None, None)
                }
            };
            if !f_deep_src.is_empty() {
                log(&format!(
                    "  - 中央値フィルター後のディープ検出: {}、星を {} 個検出 (しきい値 {}σ)",
                    deep_how(fdinfo.as_ref()),
                    f_deep_src.len(),
                    fu.map_or("None".to_string(), |u| fmt_g(u, 6))
                ));
            }
            (to_stars(&f_deep_src), to_stars(&f_std_src))
        }
        None => (Vec::new(), Vec::new()),
    };
    drop(fprep);
    let f_sets: Vec<StarSet> =
        [("中央値フィルター後のディープ検出", &f_deep_stars[..]), ("中央値フィルター後の標準の検出", &f_std_stars[..])]
            .into_iter()
            .filter(|(_, s)| s.len() >= 3)
            .collect();
    let attempts = build_attempts(&conds, &f_sets, &star_sets);

    let store = Store::new(catalog);
    if let Some(sol) = run_attempts(&attempts, req, &store, params, ctl, t_end)? {
        return Ok(tried(Some(sol)));
    }
    if ctl.cancelled() {
        return Err(SolveError::Cancelled);
    }
    if hot.is_some() {
        log("  - 中央値フィルターをかけた画像でも解けませんでした。ダーク補正をした画像で試してください。");
    }
    Ok(tried(None))
}

/// 壁時計の残り時間 [秒](過ぎていれば負)。
fn seconds_left(t_end: Instant) -> f64 {
    let now = Instant::now();
    if t_end > now { (t_end - now).as_secs_f64() } else { -(now - t_end).as_secs_f64() }
}

/// 試行の並びを順に解く(Python の `_run_attempts`)。解けたら Some。中止なら Err(Cancelled)。残りが 2 秒を切ったらやめる。
fn run_attempts(
    attempts: &[Attempt],
    req: &OfflineRequest,
    store: &Store,
    params: &Params,
    ctl: &Control,
    t_end: Instant,
) -> Result<Option<Solution>, SolveError> {
    let log = |s: &str| (ctl.log)(s);
    let total = attempts.len();
    for (i, a) in attempts.iter().enumerate() {
        let i = i + 1;
        if ctl.cancelled() {
            return Err(SolveError::Cancelled);
        }
        let left = seconds_left(t_end);
        if left < 2.0 {
            log("  ⏱ 制限時間に達したので、これ以上の再試行はしません。");
            break;
        }
        let t = if i == total { left } else { left * a.share };
        log(&format!("\n  [{i}/{total}] {}  (最大 {t:.0} 秒)", a.label));
        let mut deadline = Instant::now() + secs(t);
        if let Some(d) = ctl.deadline {
            deadline = deadline.min(d);
        }
        let sreq = SolveRequest {
            stars: a.stars.to_vec(),
            width: req.width,
            height: req.height,
            scale_lo: Some(a.range.0),
            scale_hi: Some(a.range.1),
            hint: a.hint,
        };
        let solver_log = |m: &str| log(&format!("    · {m}"));
        let r = crate::solve_in(&sreq, store, params, Clock::new(Some(deadline), ctl.cancel), &solver_log, None);
        let sol = match r {
            Ok(Some(d)) => d.solution,
            Ok(None) => continue,
            Err(Abort::Cancelled) => return Err(SolveError::Cancelled),
            Err(Abort::Invalid(m)) => {
                log(&format!("  ⚠️ 解析エンジンでエラー: ValueError: {m}"));
                continue;
            }
            Err(e) => {
                log(&format!("  ⚠️ 解析エンジンでエラー: {e:?}"));
                continue;
            }
        };
        if let Some(why) = wcs_problem(&sol) {
            log(&format!("  ⚠️ 解の WCS を読めませんでした: {why}"));
            continue;
        }
        log(&format!("    {}", solution_note(&sol)));
        log(&format!("  ✅ 解析成功 ({})", a.label));
        return Ok(Some(sol));
    }
    Ok(None)
}

/// フィルター後の星を先に試すかの判定に見る、標準の検出の明るいほうの星の数(元の画像・フィルター後とも)
const MEDIAN_TOP: usize = 30;
/// 判定に要る、元の画像の標準の検出の星の数の下限(少なければフィルター後の星は試さない)
const MEDIAN_MIN_TOP: usize = 8;
/// 同じ星とみなす距離 [px]
const MEDIAN_NEAR_PX: f64 = 2.0;
/// 元の明るい星のうち、これ以上の割合がフィルター後の明るい星から外れたら、フィルター後の星を先に試す
/// (実写では、フィルター後の星で解けたダーク補正なしのコマが 4〜8 割。元の星で解ける画像でも 4 割を超えることがある)
const MEDIAN_MIN_LOST: f64 = 0.4;
/// フィルター後の星の試行 1 回に使う、残り時間の割合の上限(元の星の試行は条件どおり)。判定が元の星ですぐ解ける
/// 画像で通ったとき、フィルター後の星で解けない探索が条件どおりの時間(全天・スケール全域なら残りの半分)を
/// 使い切ると、そうした画像を長く待たせる。実写では、フィルター後の星で解けた試行はどれも始めてから 6 秒以内に
/// 解けた(既定の制限 300 秒なら、この上限は 30 秒)。
const MEDIAN_SHARE: f64 = 0.1;

/// 元の画像の標準の検出の明るいほうから MEDIAN_TOP 個のうち、中央値フィルター後の標準の検出の明るいほうから
/// MEDIAN_TOP 個の中に MEDIAN_NEAR_PX 以内の星が無いもの(= 明るい星から外れたもの)を数える(Python の `_hot_point_count`)。
/// ホットピクセルは消えずに 2.5〜5 割の明るさで残ることが多いので、「どこかに残っているか」ではなく
/// 「明るいほうに残っているか」で見る。フィルター後の星を先に試すべき(外れたものが MEDIAN_MIN_LOST 以上)なら
/// Some((見た数, 外れた数))。フィルター後の星が見た数より少ないときは None: フィルターが星まで消している
/// (8 bit で量子化した暗い星・星像の小さい画像など)ので、ホットピクセルとはみない。
fn hot_point_count(orig: &[Source], filtered: &[Source]) -> Option<(usize, usize)> {
    let top = orig.len().min(MEDIAN_TOP);
    if top < MEDIAN_MIN_TOP || filtered.len() < top {
        return None;
    }
    let near2 = MEDIAN_NEAR_PX * MEDIAN_NEAR_PX;
    let cands = &filtered[..filtered.len().min(MEDIAN_TOP)];
    let lost = orig[..top]
        .iter()
        .filter(|a| !cands.iter().any(|b| (b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y) <= near2))
        .count();
    if (lost as f64) >= MEDIAN_MIN_LOST * top as f64 { Some((top, lost)) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_and_blind_bounds_like_python() {
        let names = ["index-4107.fits", "index-5205-03.fits", "index-4110.fits", "foo.fits", "index-x1.fits"];
        assert_eq!(index_coverage(&names), Some((11.0, 85.0)));
        let none: [&str; 0] = [];
        assert_eq!(index_coverage(&none), None);
        // 4107 = 22′〜30′、5205 = 11′〜16′、4110 = 60′〜85′ → 11′〜85′。968 px なら (11·60/968·0.9, 85·60/968·1.1)
        let (lo, hi) = blind_scale_bounds(968, &names);
        assert!((lo - 11.0 * 60.0 / 968.0 * 0.9).abs() < 1e-15);
        assert!((hi - 85.0 * 60.0 / 968.0 * 1.1).abs() < 1e-12);
        let (lo, hi) = blind_scale_bounds(1000, &none);
        assert_eq!((lo, hi), (2.0 * 60.0 / 1000.0 * 0.9, 2000.0 * 60.0 / 1000.0 * 1.1));
        // 上限 3600″/px・下限 0.01″/px で抑える
        assert_eq!(blind_scale_bounds(10, &none).1, 3600.0);
        assert_eq!(blind_scale_bounds(100_000, &none).0, 0.01);
    }

    #[test]
    fn rounding_of_seconds_like_python() {
        // f"{t:.0f}" は偶数への丸め
        assert_eq!(format!("{:.0}", 2.5f64), "2");
        assert_eq!(format!("{:.0}", 3.5f64), "4");
        assert_eq!(format!("{:.0}", 74.6f64), "75");
    }

    #[test]
    fn deep_how_texts() {
        let info = |s_psf: Option<f64>, s: f64| DeepInfo {
            s_psf,
            n_psf: 0,
            s,
            u: 4.0,
            noise: 1.0,
            candidates: 0,
            kept: 0,
            rejected: Default::default(),
            sharp_max: f64::INFINITY,
            seconds: 0.0,
        };
        // 2.3548 × 1.7156 = 4.0399…
        assert_eq!(deep_how(Some(&info(Some(1.7156), 2.1445))), "星の幅 (FWHM 4.0 px) に合わせてぼかし");
        // σ_psf が 1.2 でも測れた値として書く(星の fwhm から逆算していた頃は「測れなかった」と区別できなかった)
        assert_eq!(deep_how(Some(&info(Some(1.2), 1.5))), "星の幅 (FWHM 2.8 px) に合わせてぼかし");
        // 測れなかった(None・0)ときは、ぼかしに使った σ(Python の dinfo['s'])
        assert_eq!(deep_how(Some(&info(None, 1.5))), "星の幅を測れなかったので σ 1.5 px でぼかし");
        assert_eq!(deep_how(Some(&info(Some(0.0), 1.0))), "星の幅を測れなかったので σ 1.0 px でぼかし");
        assert_eq!(deep_how(None), "星の幅を測れなかったので σ 1.5 px でぼかし");
    }

    #[test]
    fn filtered_sets_come_first_with_a_short_share() {
        let s: Vec<FieldStar> = (0..5).map(|k| FieldStar { x: 10.0 * k as f64, y: 0.0, flux: 1.0 }).collect();
        let conds: Vec<Cond> =
            vec![("全天 + スケール既知", None, (1.0, 2.0), 0.25), ("全天 + スケール全域", None, (0.5, 50.0), 0.5)];
        let f: Vec<StarSet> = vec![("F1", &s[..]), ("F2", &s[..3])];
        let o: Vec<StarSet> = vec![("O1", &s[..4]), ("O2", &s[..])];
        let got = |f: &[StarSet]| -> Vec<(String, f64)> {
            build_attempts(&conds, f, &o).iter().map(|a| (a.label.clone(), a.share)).collect()
        };
        let want = |v: &[(&str, f64)]| -> Vec<(String, f64)> { v.iter().map(|&(l, s)| (l.to_string(), s)).collect() };
        // 条件ごとにフィルター後の星 → 元の星。フィルター後の星は残りの 1 割まで
        assert_eq!(
            got(&f),
            want(&[
                ("F1の星 5 個 + 全天 + スケール既知", 0.1),
                ("F2の星 3 個 + 全天 + スケール既知", 0.1),
                ("O1の星 4 個 + 全天 + スケール既知", 0.25),
                ("O2の星 5 個 + 全天 + スケール既知", 0.25),
                ("F1の星 5 個 + 全天 + スケール全域", 0.1),
                ("F2の星 3 個 + 全天 + スケール全域", 0.1),
                ("O1の星 4 個 + 全天 + スケール全域", 0.5),
                ("O2の星 5 個 + 全天 + スケール全域", 0.5),
            ])
        );
        // 判定が通らなければ今までどおり
        assert_eq!(
            got(&[]),
            want(&[
                ("O1の星 4 個 + 全天 + スケール既知", 0.25),
                ("O2の星 5 個 + 全天 + スケール既知", 0.25),
                ("O1の星 4 個 + 全天 + スケール全域", 0.5),
                ("O2の星 5 個 + 全天 + スケール全域", 0.5),
            ])
        );
    }

    #[test]
    fn hot_point_count_looks_at_the_bright_end() {
        let src = |x: f64, y: f64| Source { x, y, flux: 1.0, peak: 1.0, area: 1, elong: 1.0, fwhm: 2.0, snr: 10.0 };
        let grid = |n: usize, y0: f64| -> Vec<Source> { (0..n).map(|k| src(10.0 * k as f64, y0)).collect() };
        // 同じ星が同じ順なら外れたものは 0(フィルター後の星は試さない)
        let a = grid(40, 0.0);
        assert_eq!(hot_point_count(&a, &a), None);
        // 1.5 px ずれても同じ星(2 px 以内)
        let shifted: Vec<Source> = a.iter().map(|s| src(s.x + 1.2, s.y + 0.9)).collect();
        assert_eq!(hot_point_count(&a, &shifted), None);
        // 明るいほうの 30 個のうち 12 個(4 割)がフィルター後の明るい 30 個から外れる(後ろへ下がる)→ フィルター後の星を先に
        let mut f: Vec<Source> = a[12..].to_vec();
        f.extend(grid(20, 500.0));
        f.extend_from_slice(&a[..12]);
        assert_eq!(hot_point_count(&a, &f), Some((30, 12)));
        // 11 個ならそのまま
        let mut f: Vec<Source> = a[11..].to_vec();
        f.extend(grid(20, 500.0));
        f.extend_from_slice(&a[..11]);
        assert_eq!(hot_point_count(&a, &f), None);
        // フィルター後の星が全部別の場所なら全部外れる。ただしフィルター後の星が見た数より少なければ判定しない
        // (フィルターが星まで消した)。元の星が 8 個未満でも判定しない
        assert_eq!(hot_point_count(&a, &grid(30, 500.0)), Some((30, 30)));
        assert_eq!(hot_point_count(&a, &grid(29, 500.0)), None);
        assert_eq!(hot_point_count(&a, &[]), None);
        assert_eq!(hot_point_count(&grid(8, 0.0), &grid(8, 500.0)), Some((8, 8)));
        assert_eq!(hot_point_count(&grid(7, 0.0), &grid(7, 500.0)), None);
    }
}

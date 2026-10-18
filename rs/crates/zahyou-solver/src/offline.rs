//! オフライン解析の試行のはしご(Python の dev/zahyou_core.py の `_solve_offline` の、試行の部分)。
//!
//! 星は深い検出(`detect_sources_deep`)の星を先に、標準の検出の星を予備に使う。条件は探す範囲の狭い(速い)
//! ものから順に、条件ごとに 深い → 標準 の星で試す:
//!   座標ヒント + スケール既知(残り時間の 25 %)→ 座標ヒント + スケール全域(25 %)→ 全天 + スケール既知(25 %)
//!   → 全天 + スケール全域(50 %。最後の試行は残り全部)
//! 全体の締め切りは timeout_s 秒の壁時計で、試行ごとに残り時間の一部を割り当てる。残りが 2 秒を切ったらやめる。
//!
//! 係の分担(app/src/pipeline.rs): 事前のログ(画素スケール・手持ちの段・足りない段・探索するスケール)と、
//! 解けなかったときの案内と、「■ 中止しました。」は app が出す。ここが出すのは、深い検出・`[i/N] ラベル  (最大 t 秒)`・
//! 探索のログ(`    · ` を付けて)・解の要約・`✅ 解析成功 (ラベル)`・時間切れ・エンジンのエラー。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::{Duration, Instant};

use zahyou_image::{DeepInfo, DeepOpts, Source};
use zahyou_index::Catalog;

use crate::clock::{Abort, Clock};
use crate::parts::py_max;
use crate::pyfmt::{fmt_g, fmt_int};
use crate::verify::Store;
use crate::{Control, FieldStar, OfflineOutcome, OfflineRequest, OfflineStop, Params, Solution, SolveError, SolveRequest};

/// 索引がカバーする画角(分角)。astrometry.net の付番規則(番号の下 2 桁 → 段。Python の `_INDEX_SCALES`)。
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
/// 手持ちの索引が扱える画角から画像の幅で逆算する(下限 × 0.9、上限 × 1.1)。索引が無ければ 2′〜2000′ で。
pub fn blind_scale_bounds<S: AsRef<str>>(width_px: usize, names: &[S]) -> (f64, f64) {
    let (lo_fov, hi_fov) = index_coverage(names).unwrap_or((2.0, 2000.0));
    let low = lo_fov * 60.0 / width_px as f64 * 0.9;
    let high = hi_fov * 60.0 / width_px as f64 * 1.1;
    (if 0.01 > low { 0.01 } else { low }, if 3600.0 < high { 3600.0 } else { high })
}

/// 目録に入っている(検査に通った)索引ファイルの名前(拡張子つき、名前の順。Python の `_catalog_names`)。
fn catalog_names(cat: &Catalog) -> Vec<String> {
    let mut v: Vec<String> =
        cat.files.iter().filter_map(|f| f.path.file_name().map(|s| s.to_string_lossy().into_owned())).collect();
    v.sort();
    v.dedup();
    v
}

/// 深い検出のぼかし方の説明(Python の `星の幅 (FWHM … px) に合わせてぼかし` / `星の幅を測れなかったので σ … px でぼかし`)。
///
/// 深い検出が測った値(DeepInfo の s_psf・s)から Python と同じ式で書く(Python の `if s_psf` と同じく 0 は「測れなかった」)。
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
        parts.push(format!("索引 {}", s.index_name));
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

/// 試行のはしごの本体(Python の `_solve_offline` の試行の部分)。
pub(crate) fn run(req: &OfflineRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<OfflineOutcome, SolveError> {
    let t_start = Instant::now();
    let t_end = t_start + secs(req.timeout_s);
    let log = |s: &str| (ctl.log)(s);
    let nx = req.width;
    let installed = catalog_names(catalog);
    if installed.is_empty() {
        // 「使える星図データがありません」は app が出す
        return Ok(OfflineOutcome { stop: OfflineStop::NoIndex, ..Default::default() });
    }
    // Python の `if scale` と同じ(0 と None は不明、NaN はそのまま渡してエンジンのエラーになる)
    let scale = req.scale.filter(|&s| s != 0.0);
    let narrow = scale.map(|s| (py_max(s * 0.75, 0.01), s * 1.25));
    let wide = blind_scale_bounds(nx, &installed);

    if ctl.cancelled() {
        return Err(SolveError::Cancelled);
    }
    // 深い検出(解析エンジンのために暗い星まで拾い直した星)。失敗しても標準の検出の星で続ける。
    // Python の detect_sources_deep(*prep, info=dinfo) と同じ既定(400 個・偽の極大 2 個・u ≥ 4)
    let deep = catch_unwind(AssertUnwindSafe(|| zahyou_image::detect_deep(req.prep, &DeepOpts::default())));
    let (deep_src, u, dinfo) = match deep {
        Ok(r) => r,
        Err(p) => {
            log(&format!("\n  ⚠️ 深い検出に失敗したので、標準の検出の星だけで解きます: panic: {}", panic_text(&*p)));
            (Vec::new(), None, None)
        }
    };
    if !deep_src.is_empty() {
        log(&format!(
            "\n  - 深い検出 (解析エンジンに渡す星): {}、星を {} 個検出 (しきい値 {}σ)",
            deep_how(dinfo.as_ref()),
            deep_src.len(),
            u.map_or("None".to_string(), |u| fmt_g(u, 6))
        ));
    } else if dinfo.is_some() {
        // Python の `elif dinfo:`(検出を最後まで進めて、星が 0 個だったとき)
        log("\n  - 深い検出: 星が見つかりませんでした。");
    }
    let to_stars = |s: &[Source]| -> Vec<FieldStar> { s.iter().map(|c| FieldStar { x: c.x, y: c.y, flux: c.flux }).collect() };
    let deep_stars = to_stars(&deep_src);
    let std_stars = to_stars(req.sources);
    let star_sets: Vec<(&str, &[FieldStar])> = [("深い検出", &deep_stars[..]), ("標準の検出", &std_stars[..])]
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

    // 試行のはしご: (条件, ヒント, スケールの範囲, 残り時間の何割まで使うか)
    let hint = req.hint;
    type Cond<'c> = (&'c str, Option<(f64, f64, f64)>, (f64, f64), f64);
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
    let attempts: Vec<Attempt> = conds
        .iter()
        .flat_map(|&(cond, h, range, share)| {
            star_sets.iter().map(move |&(name, stars)| Attempt {
                label: format!("{name}の星 {} 個 + {cond}", stars.len()),
                stars,
                hint: h,
                range,
                share,
            })
        })
        .collect();

    let store = Store::new(catalog);
    let total = attempts.len();
    for (i, a) in attempts.iter().enumerate() {
        let i = i + 1;
        if ctl.cancelled() {
            return Err(SolveError::Cancelled);
        }
        let now = Instant::now();
        let left = if t_end > now { (t_end - now).as_secs_f64() } else { -(now - t_end).as_secs_f64() };
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
        let r = crate::solve_in(&sreq, &store, params, Clock::new(Some(deadline), ctl.cancel), &solver_log, None);
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
        return Ok(tried(Some(sol)));
    }
    if ctl.cancelled() {
        return Err(SolveError::Cancelled);
    }
    Ok(tried(None))
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
}

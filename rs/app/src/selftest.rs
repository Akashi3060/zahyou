//! 画面を出さない入口: 自己診断(`zahyou.exe --selftest 画像 [--out 結果.txt] [--online|--offline]`)と
//! 速さの物差し(`zahyou.exe --bench [--out 結果.json]`)。
//!
//! 自己診断は Python 版 `gui/zahyou_gui.py` の `selftest` のうち、画面に依らない項目(エンジン・星図データ・
//! 解析・3 つの数字・図・ログ・目標なし)を受け持つ。画面の項目(ボタンの見切れ・配色など)は gui の係が
//! [`selftest_rows`] の結果に足して [`format_report`] で書く。
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::pipeline::{self, InputMode, RunEnv, RunParams, RunResult, SolveMode};

/// 自己診断の 1 項目。
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub msg: String,
}

impl Check {
    fn new(name: &str, ok: bool, msg: impl Into<String>) -> Check {
        Check { name: name.to_string(), ok, msg: msg.into() }
    }
}

/// 自己診断の解析の目標(Python の selftest と同じ)。
pub const SELFTEST_TARGET: &str = "UCAC4 660-021020";

/// 解析の方法の画面の言い方(Python の SOLVE_CHOICES)。
pub fn solve_mode_label(m: SolveMode) -> &'static str {
    match m {
        SolveMode::Auto => "自動 (インターネットがあれば使う)",
        SolveMode::Online => "オンラインのみ",
        SolveMode::Offline => "オフラインのみ",
    }
}

/// 解析 1 回を流し、ログを文字列にためて返す。
fn run_logged(params: &RunParams, env: &RunEnv) -> (RunResult, String) {
    let buf = Mutex::new(String::new());
    let log = |s: &str| {
        let mut b = buf.lock().unwrap_or_else(|e| e.into_inner());
        b.push_str(s);
        b.push('\n');
    };
    let cancel = AtomicBool::new(false);
    let res = pipeline::run_with(params, env, &cancel, &log);
    let text = buf.into_inner().unwrap_or_else(|e| e.into_inner());
    (res, text)
}

/// 画面に依らない自己診断の項目(Python の selftest の順)。
///
/// - `force`: Some なら解析の方法をそれにする(--online / --offline)。None なら、星図データがそろっていれば
///   オフラインのみ、無ければ自動(Python と同じ)
/// - `index_dir`: 星図データの置き場所(None なら C:\AstrometryData)
/// - `log_out`: 最後の解析のログ(Python と同じく 2 回目 = 目標なしの解析のログ)
pub fn selftest_rows(
    image: &Path,
    force: Option<SolveMode>,
    index_dir: Option<&Path>,
    env: &RunEnv,
    log_out: &mut String,
) -> Vec<Check> {
    let mut out = Vec::new();
    if !image.exists() {
        out.push(Check::new("画像がある", false, image.display().to_string()));
        return out;
    }
    // 解析エンジンは exe に組み込み(Python 版は zahyou_engine.py を探して読み込んでいた)
    out.push(Check::new("エンジンを読み込める", true, format!("組み込み (zahyou {})", pipeline::log_version())));

    let index_dir = index_dir.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(pipeline::DEFAULT_INDEX_DIR));
    let diag = pipeline::solver_diagnostics(&index_dir);
    out.push(Check::new(
        "星図データを調べられる",
        true,
        format!(
            "{} ファイル / {}",
            diag.indexes.len(),
            if diag.ok { "オフライン解析できます" } else { "まだ足りません" }
        ),
    ));
    let mode = force.unwrap_or(if diag.ok { SolveMode::Offline } else { SolveMode::Auto });

    let mut params = RunParams {
        mode: InputMode::StarName,
        name: SELFTEST_TARGET.to_string(),
        solve_mode: mode,
        image_path: image.to_path_buf(),
        index_dir: index_dir.clone(),
        ..Default::default()
    };
    let t0 = Instant::now();
    let (res, log1) = run_logged(&params, env);
    out.push(Check::new(
        "解析が終わる",
        true,
        format!("{:.1} 秒 / {}", t0.elapsed().as_secs_f64(), solve_mode_label(mode)),
    ));
    let nums = res.numbers();
    let num = |i: usize| nums.as_ref().map(|n| n[i].1.clone()).unwrap_or_else(|| "—".to_string());
    out.push(Check::new("距離が出る", res.offsets.is_some(), num(0)));
    out.push(Check::new("赤経方向が出る", res.offsets.is_some(), num(1)));
    out.push(Check::new("赤緯方向が出る", res.offsets.is_some(), num(2)));
    out.push(Check::new("図が 2 枚できる", res.figures.len() == 2, format!("{} 枚", res.figures.len())));
    let detail = res.detail().unwrap_or_default();
    out.push(Check::new(
        "画像中心などが出る",
        detail.contains("画素スケール"),
        detail.chars().take(70).collect::<String>(),
    ));
    out.push(Check::new("ログが流れている", log1.contains("解析結果"), ""));
    out.push(Check::new(
        "結果の文が出る",
        res.status_text().contains("解析できました"),
        format!("{:?}", res.status_text()),
    ));

    // --- 完全ブラインド: 目標を指定せずに向きだけ求める -------------
    params.mode = InputMode::None;
    let (res2, log2) = run_logged(&params, env);
    let nums2 = res2.numbers();
    let cap = |i: usize| nums2.as_ref().map(|n| n[i].0.clone()).unwrap_or_default();
    let val = |i: usize| nums2.as_ref().map(|n| n[i].1.clone()).unwrap_or_else(|| "—".to_string());
    out.push(Check::new(
        "目標なしでも解ける",
        res2.summary.is_some() && val(0) != "—",
        format!("{} = {}", cap(0), val(0)),
    ));
    out.push(Check::new(
        "向きが数字で出る",
        cap(0).contains("画像中心"),
        format!("{} = {} / {} = {}", cap(1), val(1), cap(2), val(2)),
    ));
    out.push(Check::new("目標なしの図は 1 枚", res2.figures.len() == 1, format!("{} 枚", res2.figures.len())));
    *log_out = log2;
    out
}

/// Python の `str.ljust`(文字の数で数える)。
fn ljust(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width { s.to_string() } else { format!("{s}{}", " ".repeat(width - n)) }
}

/// 自己診断の結果の文(Python の `_run_selftest` と同じ形)。
pub fn format_report(rows: &[Check], log: &str) -> String {
    let width = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(1);
    let mut lines: Vec<String> = rows
        .iter()
        .map(|r| format!("  {}  {}  {}", if r.ok { "PASS" } else { "FAIL" }, ljust(&r.name, width), r.msg))
        .collect();
    let n_ok = rows.iter().filter(|r| r.ok).count();
    lines.push(String::new());
    lines.push(format!("  {n_ok}/{} passed", rows.len()));
    if !log.is_empty() {
        lines.push(String::new());
        lines.push(format!("--- 解析ログ {}", "-".repeat(46)));
        lines.push(log.trim_end().to_string());
    }
    lines.join("\n")
}

/// 全部通ったか(項目が 1 つも無ければ失敗)。
pub fn all_passed(rows: &[Check]) -> bool {
    !rows.is_empty() && rows.iter().all(|r| r.ok)
}

/// `--selftest`(画面の項目なし)。結果を表示し、`out` があれば UTF-8 で書く。戻り値は終了コード(全部通れば 0)。
// main.rs の --selftest は画面の項目も足す gui の入口(ui/cli.rs)を使うので、今はどこからも呼ばれない
#[allow(dead_code)]
pub fn selftest(image: &Path, mode: Option<SolveMode>, out: Option<&Path>) -> i32 {
    let mut log = String::new();
    let rows = selftest_rows(image, mode, None, &RunEnv::default(), &mut log);
    let text = format_report(&rows, &log);
    if let Some(p) = out
        && let Err(e) = std::fs::write(p, format!("{text}\n"))
    {
        eprintln!("結果を書けませんでした: {}: {e}", p.display());
    }
    println!("{text}");
    if all_passed(&rows) { 0 } else { 1 }
}

// ================================================================== 速さの物差し ===

/// 速さの物差しの 1 段。
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub name: &'static str,
    /// 最も速かった回の秒数(測れなかった段は None)
    pub seconds: Option<f64>,
    pub note: String,
}

/// 1 枚の画像の段ごとの時間。
#[derive(Clone, Debug, PartialEq)]
pub struct BenchImage {
    pub label: String,
    pub path: PathBuf,
    pub width: usize,
    pub height: usize,
    pub stages: Vec<Stage>,
}

/// 速さを測る画像の既定: 手元の実写(あれば)と、6000×4000 の合成画像(一時フォルダーに作って消す)。
pub const BENCH_REAL: &str = r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits";

/// `--bench`。段ごとの時間(読み込み・前処理・検出・解法・北が上の再投影と描画・PNG)を測り、表示して
/// `out_json` があれば JSON で書く。戻り値は終了コード。
///
/// 環境変数 ZAHYOU_BENCH_DIR があれば、合成画像をそこに作って残す(あれば作り直さない)。Python 版の物差し
/// (rs/testdata/bench_pipeline.py)に同じ画像を読ませて比べるため。
/// 環境変数 ZAHYOU_BENCH_IMAGES(`;` 区切りのパス)があれば、既定の画像の代わりにそれを測る。
pub fn bench(out_json: Option<&Path>) -> i32 {
    if let Some(list) = std::env::var_os("ZAHYOU_BENCH_IMAGES").filter(|s| !s.is_empty()) {
        let images: Vec<(String, PathBuf)> = list
            .to_string_lossy()
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                let p = PathBuf::from(s.trim());
                (p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p)
            })
            .collect();
        return bench_with(&images, 3, out_json);
    }
    let mut images: Vec<(String, PathBuf)> = Vec::new();
    if Path::new(BENCH_REAL).exists() {
        images.push(("実写 968×548".to_string(), PathBuf::from(BENCH_REAL)));
    }
    let keep = std::env::var_os("ZAHYOU_BENCH_DIR").filter(|s| !s.is_empty()).map(PathBuf::from);
    let tmp = keep.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("zahyou_bench_{}", std::process::id())));
    let synth = tmp.join("synth_6000x4000.fits");
    let made = std::fs::create_dir_all(&tmp).is_ok()
        && (keep.is_some() && synth.is_file() || write_synthetic_fits(&synth, 6000, 4000, 2000, 1).is_ok());
    if made {
        images.push(("合成 6000×4000".to_string(), synth.clone()));
    }
    let code = bench_with(&images, 3, out_json);
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    code
}

/// 画像を指定して速さを測る(`repeat` 回のうち最速)。
pub fn bench_with(images: &[(String, PathBuf)], repeat: usize, out_json: Option<&Path>) -> i32 {
    let mut results = Vec::new();
    for (label, path) in images {
        match bench_image(label, path, repeat.max(1)) {
            Ok(r) => results.push(r),
            Err(e) => eprintln!("{label}: {e}"),
        }
    }
    let text = bench_text(&results);
    println!("{text}");
    if let Some(p) = out_json
        && let Err(e) = std::fs::write(p, bench_json(&results))
    {
        eprintln!("結果を書けませんでした: {}: {e}", p.display());
        return 1;
    }
    if results.is_empty() { 1 } else { 0 }
}

fn best_of<T>(repeat: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut last = None;
    for _ in 0..repeat {
        let t = Instant::now();
        let v = f();
        best = best.min(t.elapsed().as_secs_f64());
        last = Some(v);
    }
    // repeat >= 1 なので last は必ず入っている
    match last {
        Some(v) => (best, v),
        None => (best, f()),
    }
}

/// 1 枚の画像を段ごとに測る。解法(第 3 段で中身が入る)は解けたときだけ北が上の段を測る。
/// 解けなければ、画像に入っている WCS があればそれで北が上の段を測る(合成画像は WCS を持っている)。
pub fn bench_image(label: &str, path: &Path, repeat: usize) -> Result<BenchImage, String> {
    use zahyou_image::DetectOpts;
    let mut stages = Vec::new();
    let (t, loaded) = best_of(repeat, || zahyou_image::load_image_any(path));
    let loaded = loaded.map_err(|e| e.to_string())?;
    stages.push(Stage { name: "読み込み", seconds: Some(t), note: loaded.note.clone() });
    let (t, prep) = best_of(repeat, || zahyou_image::preprocess(&loaded.data, true));
    stages.push(Stage { name: "前処理", seconds: Some(t), note: format!("σ = {:.3}", prep.sigma) });
    let (t, (sources, thr)) =
        best_of(repeat, || zahyou_image::detect_sources(&prep, &DetectOpts::with_min_sources(60)));
    stages.push(Stage {
        name: "検出",
        seconds: Some(t),
        note: format!("{} 個 (しきい値 {})", sources.len(), thr.map(crate::scale::fmt_g).unwrap_or_else(|| "-".into())),
    });

    // 解法(星図データがあれば)
    let (nx, ny) = (loaded.data.width, loaded.data.height);
    let diag = pipeline::solver_diagnostics(Path::new(pipeline::DEFAULT_INDEX_DIR));
    let mut wcs = None;
    if let Some(cat) = diag.catalog.as_ref().filter(|_| diag.ok) {
        let cache = zahyou_net::cache_path();
        let (scale, _) = crate::scale::estimate_pixel_scale(&loaded.header, None, Some((nx, ny)), &cache);
        let req = zahyou_solver::OfflineRequest {
            prep: &prep,
            sources: &sources,
            width: nx,
            height: ny,
            scale,
            hint: None,
            timeout_s: 60.0,
        };
        let cancel = AtomicBool::new(false);
        let quiet = |_: &str| {};
        let ctl = zahyou_solver::Control {
            deadline: Some(Instant::now() + std::time::Duration::from_secs(60)),
            cancel: &cancel,
            log: &quiet,
        };
        let (t, sol) = best_of(1, || zahyou_solver::solve_offline(&req, cat, &zahyou_solver::Params::default(), &ctl));
        let note = match &sol {
            Ok(Some(s)) => format!("解けた ({}、一致 {} 個)", s.index_name, s.nmatch),
            Ok(None) => "解けなかった".to_string(),
            Err(_) => "中止".to_string(),
        };
        stages.push(Stage { name: "解法", seconds: Some(t), note });
        wcs = sol.ok().flatten().map(|s| s.wcs);
    } else {
        stages.push(Stage { name: "解法", seconds: None, note: "星図データが無い".to_string() });
    }
    let wcs_note = if wcs.is_some() { "解いた WCS" } else { "画像の WCS" };
    let wcs = wcs.or_else(|| crate::scale::existing_wcs(&loaded.header));

    match wcs {
        Some(w) => {
            // 目標は画像中心から少し北東(図 2 の題と矢印まで作る)
            let c = w.pix_to_radec((nx as f64 - 1.0) / 2.0, (ny as f64 - 1.0) / 2.0);
            let target = Some((c.0 + 0.01, (c.1 + 0.01).min(90.0)));
            let (t, nu) = best_of(repeat, || zahyou_plot::reproject_north_up(&loaded.display, &w));
            let note = match &nu {
                Ok(n) => format!("{wcs_note}、{} x {} 画素", n.image.width, n.image.height),
                Err(e) => format!("{wcs_note}、失敗: {e}"),
            };
            stages.push(Stage { name: "北が上の再投影", seconds: Some(t), note });
            let (t1, fig1) = best_of(repeat, || {
                zahyou_plot::figure_original(&loaded.display, &w, &sources, target, loaded.flip_y, true)
            });
            let (t2, fig2) = best_of(repeat, || zahyou_plot::figure_north_up(&loaded.display, &w, target));
            stages.push(Stage {
                name: "図 2 枚 (再投影込み)",
                seconds: Some(t1 + t2),
                note: format!("図 1 {t1:.3} 秒 + 図 2 {t2:.3} 秒"),
            });
            match (&fig1, &fig2) {
                (Ok(f1), Ok(f2)) => {
                    let opts = zahyou_plot::RenderOptions::default();
                    let (t, bytes) =
                        best_of(repeat, || (zahyou_plot::render_png(f1, &opts), zahyou_plot::render_png(f2, &opts)));
                    let note = match bytes {
                        (Ok(a), Ok(b)) => format!("{} + {} バイト", a.len(), b.len()),
                        (Err(e), _) | (_, Err(e)) => format!("失敗: {e}"),
                    };
                    stages.push(Stage { name: "PNG (図 2 枚)", seconds: Some(t), note });
                }
                (Err(e), _) | (_, Err(e)) => {
                    stages.push(Stage {
                        name: "PNG (図 2 枚)", seconds: None, note: format!("図を作れなかった: {e}")
                    })
                }
            }
        }
        None => {
            for name in ["北が上の再投影", "図 2 枚 (再投影込み)", "PNG (図 2 枚)"] {
                stages.push(Stage { name, seconds: None, note: "WCS が無い".to_string() });
            }
        }
    }

    // 解析全体(run): 画像の WCS があれば使う・オフライン・目標は既定の天体名(引けなければ既定の赤経赤緯)。
    // cache.json は一時ファイルにする(使う人の記憶を書き換えない)
    let tmp = std::env::temp_dir().join(format!("zahyou_bench_cache_{}.json", std::process::id()));
    let env = RunEnv { cache_file: Some(tmp.clone()), online: Some(false), ..Default::default() };
    let params = RunParams {
        solve_mode: SolveMode::Offline,
        ignore_existing_wcs: false,
        image_path: path.to_path_buf(),
        offline_timeout_s: 60.0,
        ..Default::default()
    };
    let quiet = |_: &str| {};
    let (t, res) = best_of(repeat, || {
        let _ = std::fs::remove_file(&tmp);
        pipeline::run_with(&params, &env, &AtomicBool::new(false), &quiet)
    });
    let _ = std::fs::remove_file(&tmp);
    stages.push(Stage {
        name: "解析全体 (run)",
        seconds: Some(t),
        note: format!(
            "{} / 図 {} 枚",
            res.how.clone().unwrap_or_else(|| "解けなかった".to_string()),
            res.figures.len()
        ),
    });
    Ok(BenchImage { label: label.to_string(), path: path.to_path_buf(), width: nx, height: ny, stages })
}

fn bench_text(results: &[BenchImage]) -> String {
    let mut lines = Vec::new();
    for r in results {
        lines.push(format!("{}  ({} x {})  {}", r.label, r.width, r.height, r.path.display()));
        for s in &r.stages {
            let t = s.seconds.map(|t| format!("{t:8.3} 秒")).unwrap_or_else(|| "       — ".to_string());
            lines.push(format!("  {t}  {}  {}", ljust(s.name, 14), s.note));
        }
    }
    lines.join("\n")
}

fn bench_json(results: &[BenchImage]) -> String {
    use serde_json::{Value, json};
    let arr: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "label": r.label,
                "path": r.path.display().to_string(),
                "width": r.width,
                "height": r.height,
                "stages": r.stages.iter().map(|s| json!({"name": s.name, "seconds": s.seconds, "note": s.note})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let v = json!({"zahyou": pipeline::log_version(), "images": arr});
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
}

/// 速さの物差し用の合成画像(float32 の FITS、TAN の WCS つき)。星 `nstars` 個・雑音・ゆるい勾配。
/// 擬似乱数は `seed` から決まる(同じ引数なら同じファイル)。
pub fn write_synthetic_fits(path: &Path, width: usize, height: usize, nstars: usize, seed: u64) -> Result<(), String> {
    let mut rng = SplitMix64(seed);
    let mut data = vec![0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let g = 1000.0 + 0.02 * x as f64 + 0.01 * y as f64;
            data[y * width + x] = (g + 10.0 * rng.normal()) as f32;
        }
    }
    for _ in 0..nstars {
        let cx = rng.uniform() * width as f64;
        let cy = rng.uniform() * height as f64;
        let amp = 200.0 * (1.0 + 30.0 * rng.uniform().powi(3));
        let sig = 1.3;
        let (x0, x1) = ((cx - 6.0).max(0.0) as usize, ((cx + 7.0) as usize).min(width));
        let (y0, y1) = ((cy - 6.0).max(0.0) as usize, ((cy + 7.0) as usize).min(height));
        for y in y0..y1 {
            for x in x0..x1 {
                let r2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                data[y * width + x] += (amp * (-r2 / (2.0 * sig * sig)).exp()) as f32;
            }
        }
    }
    let scale_deg = 1.0 / 3600.0;
    let cards: Vec<String> = vec![
        "SIMPLE  =                    T".into(),
        "BITPIX  =                  -32".into(),
        "NAXIS   =                    2".into(),
        format!("NAXIS1  = {width:>20}"),
        format!("NAXIS2  = {height:>20}"),
        "CTYPE1  = 'RA---TAN'".into(),
        "CTYPE2  = 'DEC--TAN'".into(),
        format!("CRPIX1  = {:>20.1}", width as f64 / 2.0 + 0.5),
        format!("CRPIX2  = {:>20.1}", height as f64 / 2.0 + 0.5),
        "CRVAL1  =                 83.8".into(),
        "CRVAL2  =                 -5.4".into(),
        format!("CD1_1   = {:>20.12E}", -scale_deg),
        "CD1_2   =                  0.0".into(),
        "CD2_1   =                  0.0".into(),
        format!("CD2_2   = {:>20.12E}", scale_deg),
        "END".into(),
    ];
    let mut bytes: Vec<u8> = Vec::new();
    for c in &cards {
        bytes.extend_from_slice(format!("{c:<80}").as_bytes());
    }
    while !bytes.len().is_multiple_of(2880) {
        bytes.push(b' ');
    }
    for v in &data {
        bytes.extend_from_slice(&v.to_be_bytes());
    }
    while !bytes.len().is_multiple_of(2880) {
        bytes.push(0);
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// 小さな擬似乱数(SplitMix64)。
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// [0, 1)
    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// 標準正規(Box–Muller)
    fn normal(&mut self) -> f64 {
        let u1 = self.uniform().max(1e-300);
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_format_like_python() {
        let rows = vec![Check::new("画像中心などが出る", true, "x"), Check::new("ログ", false, "")];
        let t = format_report(&rows, "abc\n");
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "  PASS  画像中心などが出る  x");
        assert_eq!(lines[1], format!("  FAIL  ログ{}  ", " ".repeat(7)));
        assert_eq!(lines[3], "  1/2 passed");
        assert_eq!(lines[5], format!("--- 解析ログ {}", "-".repeat(46)));
        assert_eq!(lines[6], "abc");
        assert!(!all_passed(&rows));
        assert!(!all_passed(&[]));
    }

    #[test]
    fn missing_image() {
        let mut log = String::new();
        let rows = selftest_rows(Path::new(r"C:\no\such\image.fits"), None, None, &RunEnv::default(), &mut log);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].ok);
    }
}

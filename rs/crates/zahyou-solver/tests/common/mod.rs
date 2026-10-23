//! zahyou-solver の試験の共通部品: 基準値(rs/testdata/solver_*.json)と正解(dev/solver_truth.json)を読む、
//! 星図データの目録(系列を絞った写しも)、物差し(Python の sv_synth_measure・sv_synth_compare)、
//! WCS の差(画素)、画像の置き場所、SKIP の出し方、重い試験を 1 本ずつ流す鍵。
//!
//! 置き場所は環境変数で変えられる:
//!   ZAHYOU_INDEX_DIR     星図データ(既定 C:\AstrometryData)
//!   ZAHYOU_TESTDATA      基準値の JSON(既定 rs/testdata)
//!   ZAHYOU_TEST_IMAGES   $ZAHYOU_TEST_IMAGES/ で始まる画像のパスの基点(既定はホームフォルダー)
//!   ZAHYOU_PY_REPO       リポジトリからの相対の画像が zahyou-rs に無いとき(dev/tcase・dev/realcase は
//!                        .gitignore)に見る Python 版のリポジトリ(既定 ~/Documents/GitHub/zahyou)
//!   ZAHYOU_SOLVER_DEADLINE  解ける場合の 1 回の締め切り [秒](既定 120)
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub use serde_json::Value;
use zahyou_index::Catalog;
use zahyou_solver::{Control, FieldStar, SolveRequest};
use zahyou_wcs::{ARCSEC_PER_RAD, Sip, Vec3, Wcs, angle_between, normalize, radec_to_xyz, tan_basis, xyz_to_radec};

// ------------------------------------------------------------------ 置き場所 ---

pub fn repo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
}

pub fn testdata_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("testdata"))
}

pub fn index_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_INDEX_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\AstrometryData"))
}

/// ホームフォルダー(USERPROFILE、無ければ HOME)。作者のホームの絶対パスをソースに書かないため。
pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

pub fn py_repo_dir() -> PathBuf {
    std::env::var_os("ZAHYOU_PY_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("Documents").join("GitHub").join("zahyou"))
}

/// 解ける場合の締め切り [秒]。
pub fn solve_deadline_s() -> f64 {
    std::env::var("ZAHYOU_SOLVER_DEADLINE").ok().and_then(|s| s.parse().ok()).unwrap_or(120.0)
}

/// SKIP を出す(試験は通す)。
pub fn skip(name: &str, why: &str) {
    eprintln!("SKIP {name}: {why}");
}

/// 基準値の JSON(rs/testdata)。無ければ None。
pub fn load_testdata(name: &str) -> Option<Value> {
    let p = testdata_dir().join(name);
    let s = std::fs::read_to_string(&p).ok()?;
    Some(serde_json::from_str(&s).unwrap_or_else(|e| panic!("{} を読めない: {e}", p.display())))
}

/// 正解ファイル dev/solver_truth.json。
pub fn load_truth() -> Option<Value> {
    let p = repo_dir().join("dev").join("solver_truth.json");
    let s = std::fs::read_to_string(&p).ok()?;
    Some(serde_json::from_str(&s).expect("solver_truth.json"))
}

/// solver_truth.json の path をこの PC のパスにする($ZAHYOU_TEST_IMAGES/ か、リポジトリからの相対)。
pub fn image_path(rel: &str) -> PathBuf {
    const VAR: &str = "$ZAHYOU_TEST_IMAGES/";
    if let Some(rest) = rel.strip_prefix(VAR) {
        let base = std::env::var_os("ZAHYOU_TEST_IMAGES").map(PathBuf::from).unwrap_or_else(home_dir);
        return rest.split('/').fold(base, |p, s| p.join(s));
    }
    let q = rel.split('/').fold(repo_dir(), |p, s| p.join(s));
    if q.is_file() {
        return q;
    }
    rel.split('/').fold(py_repo_dir(), |p, s| p.join(s))
}

pub fn sha256_file(p: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let b = std::fs::read(p).ok()?;
    Some(Sha256::digest(&b).iter().map(|x| format!("{x:02x}")).collect())
}

// ------------------------------------------------------------------ 星図データ ---

static CATALOG: OnceLock<Option<Arc<Catalog>>> = OnceLock::new();

/// 星図データの目録(1 つの試験の実行ファイルの中で使い回す)。無ければ None。
/// Windows 以外でも None(zahyou-index が索引の表を開くのは Windows だけなので、星図データを使う試験をしない)。
pub fn catalog() -> Option<Arc<Catalog>> {
    CATALOG
        .get_or_init(|| {
            if !cfg!(windows) {
                eprintln!("SKIP: Windows 以外では索引の表を開かないので、星図データを使う試験はしない");
                return None;
            }
            let d = index_dir();
            if !d.is_dir() {
                return None;
            }
            let c = Catalog::scan(&d);
            if c.files.is_empty() { None } else { Some(Arc::new(c)) }
        })
        .clone()
}

/// 系列 ids だけを見せる目録(Python の test_solver_synth.restrict)。フォルダーを走査し直して絞る。
pub fn restricted_catalog(ids: &[u32]) -> Catalog {
    let mut c = Catalog::scan(index_dir());
    let files = std::mem::take(&mut c.files);
    c.series.clear();
    for f in files {
        if ids.contains(&f.indexid) {
            let k = c.files.len();
            c.series.entry(f.indexid).or_default().push(k);
            c.files.push(f);
        }
    }
    c
}

/// 索引のファイル 1 本のパス(無ければ None。Windows 以外でも None。[`catalog`] と同じ理由)。
pub fn index_path(tag: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let p = index_dir().join(format!("index-{tag}.fits"));
    p.is_file().then_some(p)
}

// ------------------------------------------------------------------ 重い試験を 1 本ずつ ---

static HEAVY: Mutex<()> = Mutex::new(());

/// 時間を測る試験・重い試験はこれを取ってから走らせる(同じ実行ファイルの中で並列に走ると、締め切りの超過や
/// 速さの値が他の試験の負荷で狂うため)。前の試験が panic して毒になっていても取る。
pub fn heavy() -> MutexGuard<'static, ()> {
    HEAVY.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ JSON の値 ---

pub fn f64s(v: &Value) -> Vec<f64> {
    fn walk(v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Null => out.push(f64::NAN),
            x => out.push(x.as_f64().unwrap_or(f64::NAN)),
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

pub fn usizes(v: &Value) -> Vec<usize> {
    v.as_array().map(|a| a.iter().map(|x| x.as_u64().unwrap_or(u64::MAX) as usize).collect()).unwrap_or_default()
}

pub fn i64s(v: &Value) -> Vec<i64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_i64().unwrap_or(i64::MIN)).collect()).unwrap_or_default()
}

pub fn opt_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

pub fn vec3(v: &Value) -> Vec3 {
    let a = f64s(v);
    [a[0], a[1], a[2]]
}

/// [[x, y], …] と flux の並びから星(flux が null の星は NaN)。
pub fn stars_xy_flux(xy: &Value, flux: &Value) -> Vec<FieldStar> {
    let p = f64s(xy);
    let f = if flux.is_null() { vec![f64::NAN; p.len() / 2] } else { f64s(flux) };
    p.chunks(2).zip(f).map(|(c, fl)| FieldStar { x: c[0], y: c[1], flux: fl }).collect()
}

/// [[x, y, flux], …] から星。
pub fn stars_xyf(v: &Value) -> Vec<FieldStar> {
    f64s(v).chunks(3).map(|c| FieldStar { x: c[0], y: c[1], flux: c[2] }).collect()
}

/// 星のリストを '<f8' の [x, y, flux] の並びにした sha256(gen_solver.py の sha_stars と同じ)。
pub fn sha_stars(st: &[(f64, f64, f64)]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for &(x, y, f) in st {
        h.update(x.to_le_bytes());
        h.update(y.to_le_bytes());
        h.update(f.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Python の SvWcs.to_dict(crval_xyz・crpix(0 始まり)・cd・a・b・ap・bp)から Wcs。
pub fn wcs_from_json(d: &Value) -> Wcs {
    let crval = vec3(&d["crval_xyz"]);
    let p = f64s(&d["crpix"]);
    let c = f64s(&d["cd"]);
    let cd = [[c[0], c[1]], [c[2], c[3]]];
    let sip = if d.get("a").is_some_and(|a| !a.is_null()) {
        let a = f64s(&d["a"]);
        let b = f64s(&d["b"]);
        let order = (a.len() as f64).sqrt().round() as usize - 1;
        let mut s = Sip::new(order, a, b).expect("SIP の係数");
        if d.get("ap").is_some_and(|a| !a.is_null()) {
            let ap = f64s(&d["ap"]);
            let bp = f64s(&d["bp"]);
            let ao = (ap.len() as f64).sqrt().round() as usize - 1;
            s = s.with_inverse(ao, ap, bp).expect("SIP の逆変換の係数");
        }
        Some(s)
    } else {
        None
    };
    Wcs::new(normalize(crval), [p[0], p[1]], cd, sip)
}

/// 仮説の記録(crval = [赤経, 赤緯]、crpix、cd)から TAN の Wcs。
pub fn hyp_wcs(h: &Value) -> Wcs {
    let cv = f64s(&h["crval"]);
    let p = f64s(&h["crpix"]);
    let c = f64s(&h["cd"]);
    Wcs::tan(radec_to_xyz(cv[0], cv[1]), [p[0], p[1]], [[c[0], c[1]], [c[2], c[3]]])
}

/// 画像の 5 × 5 の格子(縁を含む)で、a の画素を空へ写し b で画素に戻したときのずれの最大 [px]。
/// 戻せない点があれば ∞。
pub fn wcs_diff_px(a: &Wcs, b: &Wcs, w: usize, h: usize) -> f64 {
    let mut worst = 0.0f64;
    for i in 0..5 {
        for j in 0..5 {
            let x = -0.5 + (w as f64) * (i as f64) / 4.0;
            let y = -0.5 + (h as f64) * (j as f64) / 4.0;
            let v = a.pix_to_xyz(x, y);
            match b.xyz_to_pix_exact(v) {
                Some((x2, y2)) => worst = worst.max((x2 - x).hypot(y2 - y)),
                None => return f64::INFINITY,
            }
        }
    }
    worst
}

// ------------------------------------------------------------------ 物差し(Python の solver_synth) ---

/// sv_synth_measure の値: 中心・画素スケール・+y の位置角・パリティ(と、正解の中心の北で測った位置角)。
#[derive(Clone, Copy, Debug)]
pub struct Measure {
    pub center_xyz: Vec3,
    pub scale_arcsec: f64,
    pub pa_up_deg: f64,
    pub parity: i32,
    pub pa_ref_deg: Option<f64>,
    pub ref_xyz: Option<Vec3>,
}

fn jacobian(pts: &[Vec3; 5], basis: Vec3, h: f64) -> Option<([f64; 2], [f64; 2])> {
    let (e, n) = tan_basis(basis);
    let mut xi = [0.0; 4];
    let mut eta = [0.0; 4];
    for k in 0..4 {
        let p = pts[k + 1];
        let w = zahyou_wcs::dot(p, basis);
        if w <= 0.0 {
            return None;
        }
        xi[k] = zahyou_wcs::dot(p, e) / w;
        eta[k] = zahyou_wcs::dot(p, n) / w;
    }
    Some(([(xi[0] - xi[1]) / (2.0 * h), (eta[0] - eta[1]) / (2.0 * h)], [(xi[2] - xi[3]) / (2.0 * h), (eta[2] - eta[3]) / (2.0 * h)]))
}

/// WCS(画素 → 単位ベクトル)を測る。ref_xyz を渡すとその点の北で測った位置角も出す。
pub fn measure(f: &dyn Fn(f64, f64) -> Vec3, w: usize, h: usize, ref_xyz: Option<Vec3>) -> Option<Measure> {
    let cx = (w as f64 - 1.0) / 2.0;
    let cy = (h as f64 - 1.0) / 2.0;
    let st = 1.0;
    let pts = [f(cx, cy), f(cx + st, cy), f(cx - st, cy), f(cx, cy + st), f(cx, cy - st)].map(normalize);
    let c = pts[0];
    let (jx, jy) = jacobian(&pts, c, st)?;
    let det = jx[0] * jy[1] - jy[0] * jx[1];
    let mut m = Measure {
        center_xyz: c,
        scale_arcsec: det.abs().sqrt() * ARCSEC_PER_RAD,
        pa_up_deg: jy[0].atan2(jy[1]).to_degrees().rem_euclid(360.0),
        parity: if det >= 0.0 { 1 } else { -1 },
        pa_ref_deg: None,
        ref_xyz: None,
    };
    if let Some(r) = ref_xyz {
        let r = normalize(r);
        let (_, ry) = jacobian(&pts, r, st)?;
        m.pa_ref_deg = Some(ry[0].atan2(ry[1]).to_degrees().rem_euclid(360.0));
        m.ref_xyz = Some(r);
    }
    Some(m)
}

pub fn measure_wcs(wcs: &Wcs, w: usize, h: usize, ref_xyz: Option<Vec3>) -> Option<Measure> {
    measure(&|x, y| wcs.pix_to_xyz(x, y), w, h, ref_xyz)
}

/// 正解の測った値(solver_truth.json の truth / solver_synth.json の truth_summary)。
pub fn truth_measure(t: &Value) -> Measure {
    let xyz = if t.get("center_xyz").is_some_and(|v| !v.is_null()) {
        vec3(&t["center_xyz"])
    } else {
        radec_to_xyz(t["center_ra"].as_f64().unwrap(), t["center_dec"].as_f64().unwrap())
    };
    Measure {
        center_xyz: xyz,
        scale_arcsec: t["scale_arcsec"].as_f64().unwrap(),
        pa_up_deg: t["pa_up_deg"].as_f64().unwrap(),
        parity: t["parity"].as_i64().unwrap() as i32,
        pa_ref_deg: None,
        ref_xyz: None,
    }
}

/// 基準(設計書 8 節。solver_truth.json の criteria と同じ形)。
#[derive(Clone, Copy, Debug)]
pub struct Tol {
    pub center_arcsec: f64,
    pub center_px: f64,
    pub scale_rel: f64,
    pub rot_deg: f64,
}

pub const TOL_DEFAULT: Tol = Tol { center_arcsec: 2.0, center_px: 0.5, scale_rel: 0.002, rot_deg: 0.1 };

impl Tol {
    /// criteria に criteria_override(あれば)を重ねる(Python の effective_criteria)。
    pub fn from_criteria(crit: &Value, over: Option<&Value>) -> Tol {
        let get = |k: &str, d: f64| -> f64 {
            over.and_then(|o| o.get(k)).and_then(|v| v.as_f64()).or_else(|| crit.get(k).and_then(|v| v.as_f64())).unwrap_or(d)
        };
        Tol {
            center_arcsec: get("center_arcsec", 2.0),
            center_px: get("center_px", 0.5),
            scale_rel: get("scale_rel", 0.002),
            rot_deg: get("rot_deg", 0.1),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cmp {
    pub ok: bool,
    pub failed: Vec<&'static str>,
    pub center_arcsec: f64,
    pub center_tol_arcsec: f64,
    pub scale_rel: f64,
    pub rot_deg: f64,
    pub parity: (i32, i32),
}

pub fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// sv_synth_compare と同じ判定。
pub fn compare(got: &Measure, want: &Measure, tol: Tol) -> Cmp {
    let sep = angle_between(got.center_xyz, want.center_xyz) * ARCSEC_PER_RAD;
    let ctol = tol.center_arcsec.max(tol.center_px * want.scale_arcsec);
    let srel = got.scale_arcsec / want.scale_arcsec - 1.0;
    let mut pa = got.pa_up_deg;
    if let (Some(p), Some(r)) = (got.pa_ref_deg, got.ref_xyz)
        && angle_between(r, want.center_xyz) < 1e-9
    {
        pa = p;
    }
    let drot = angle_diff(pa, want.pa_up_deg);
    let mut failed = Vec::new();
    if sep > ctol {
        failed.push("center");
    }
    if srel.abs() > tol.scale_rel {
        failed.push("scale");
    }
    if drot.abs() > tol.rot_deg {
        failed.push("rot");
    }
    if got.parity != want.parity {
        failed.push("parity");
    }
    Cmp {
        ok: failed.is_empty(),
        failed,
        center_arcsec: sep,
        center_tol_arcsec: ctol,
        scale_rel: srel,
        rot_deg: drot,
        parity: (got.parity, want.parity),
    }
}

/// 解の WCS を正解(Measure)と比べる。
pub fn compare_wcs(wcs: &Wcs, w: usize, h: usize, want: &Measure, tol: Tol) -> Option<Cmp> {
    let got = measure_wcs(wcs, w, h, Some(want.center_xyz))?;
    Some(compare(&got, want, tol))
}

pub fn describe(c: &Cmp) -> String {
    format!(
        "中心 {:.3}″(許容 {:.2}″)・スケール {:+.4}%・回転 {:+.4}°・パリティ {:+}/{:+}",
        c.center_arcsec,
        c.center_tol_arcsec,
        c.scale_rel * 100.0,
        c.rot_deg,
        c.parity.0,
        c.parity.1
    )
}

// ------------------------------------------------------------------ 解く ---

/// ログを集める入れ物(Control.log に渡す)。
#[derive(Default)]
pub struct LogBuf(pub Mutex<Vec<String>>);

impl LogBuf {
    pub fn push(&self, s: &str) {
        if let Ok(mut v) = self.0.lock() {
            v.push(s.to_string());
        }
    }
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }
    pub fn tail(&self, n: usize) -> String {
        let v = self.lines();
        v[v.len().saturating_sub(n)..].join(" / ")
    }
}

/// 締め切り(秒)から Control を作る道具。
pub fn deadline_in(sec: f64) -> Instant {
    Instant::now() + Duration::from_secs_f64(sec.max(0.0))
}

/// 1 回解く(締め切り sec 秒・中止なし・ログは buf へ)。戻り値 (結果, 秒)。
pub fn solve_once(
    req: &SolveRequest,
    cat: &Catalog,
    params: &zahyou_solver::Params,
    sec: f64,
    buf: &LogBuf,
) -> (Result<Option<zahyou_solver::Solution>, zahyou_solver::SolveError>, f64) {
    let cancel = AtomicBool::new(false);
    let log = |s: &str| buf.push(s);
    let ctl = Control { deadline: Some(deadline_in(sec)), cancel: &cancel, log: &log };
    let t0 = Instant::now();
    let r = zahyou_solver::solve(req, cat, params, &ctl);
    (r, t0.elapsed().as_secs_f64())
}

/// Python の解(trace の要点)と Rust の解を突き合わせる。食い違いの説明の並びを返す(空なら一致)。
pub fn solution_diff(sol: &zahyou_solver::Solution, py: &Value, w: usize, h: usize) -> Vec<String> {
    let s = &py["solution"];
    let mut bad = Vec::new();
    let pidx = s["index"].as_str().unwrap_or("");
    if sol.index_name != pidx {
        bad.push(format!("索引 {} ≠ Python {pidx}", sol.index_name));
    }
    let plo = s["logodds"].as_f64().unwrap_or(f64::NAN);
    if (sol.logodds - plo).abs() > 1e-6 || !sol.logodds.is_finite() {
        bad.push(format!("対数オッズ {:.9} ≠ Python {plo:.9}", sol.logodds));
    }
    for (k, v) in [("nmatch", sol.nmatch), ("ndistract", sol.ndistract), ("nfield", sol.nfield), ("nref", sol.nref)] {
        let p = s[k].as_u64().unwrap_or(u64::MAX) as usize;
        if v != p {
            bad.push(format!("{k} {v} ≠ Python {p}"));
        }
    }
    let pw = wcs_from_json(&s["wcs"]);
    let d = wcs_diff_px(&sol.wcs, &pw, w, h).max(wcs_diff_px(&pw, &sol.wcs, w, h));
    if d.is_nan() || d > 1e-6 {
        bad.push(format!("WCS が {d:.2e} px 違う(SIP {} / Python {})", sol.wcs.has_sip(), pw.has_sip()));
    }
    for (k, v) in [("quads_tried", sol.quads_tried), ("hypotheses", sol.hypotheses)] {
        let p = py[k].as_u64().unwrap_or(u64::MAX);
        if v != p {
            bad.push(format!("{k} {v} ≠ Python {p}"));
        }
    }
    bad
}

// ------------------------------------------------------------------ trace(途中の値)の突き合わせ ---

/// trace つきで 1 回解く(zahyou_solver::solve_detailed。trace は一時ファイルに書かせて読む)。戻り値 (結果, 秒, trace)。
/// trace は Python の `_sv_write_trace` と同じ名前の項目の JSON(status・files・scale_used・quads_tried・hits・
/// hypotheses・quick_rejected・logodds_max・flushes・tuneups・hypotheses_log・solution)。読めなければ None。
/// 秒には trace を書く時間(仮説 2 万個まで)も入る。
pub fn solve_traced(
    req: &SolveRequest,
    cat: &Catalog,
    params: &zahyou_solver::Params,
    sec: f64,
    buf: &LogBuf,
) -> (Result<Option<zahyou_solver::SolveDetail>, zahyou_solver::SolveError>, f64, Option<Value>) {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = std::env::var_os("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("zahyou-solver-tests").join("trace");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("trace-{}-{k}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let cancel = AtomicBool::new(false);
    let log = |s: &str| buf.push(s);
    let ctl = Control { deadline: Some(deadline_in(sec)), cancel: &cancel, log: &log };
    let opts = zahyou_solver::SolveOptions { trace: Some(path.clone()) };
    let t0 = Instant::now();
    let r = zahyou_solver::solve_detailed(req, cat, params, &ctl, &opts);
    let dt = t0.elapsed().as_secs_f64();
    let t = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let _ = std::fs::remove_file(&path);
    (r, dt, t)
}

fn close(a: f64, b: f64, abs: f64, rel: f64) -> bool {
    (a - b).abs() <= abs.max(rel * a.abs().max(b.abs())) || (a.is_nan() && b.is_nan())
}

/// v が小数 nd 桁に丸めた値に見えるか(Rust の trace は Python と同じく仮説のスケールを 6 桁・対数オッズを 4 桁に
/// 丸めて書く。gen_solver.py の Python 側の値は丸めていない)。
fn looks_rounded(v: f64, nd: i32) -> bool {
    let m = v * 10f64.powi(nd);
    (m - m.round()).abs() < 1e-6 * m.abs().max(1.0)
}

/// 丸めを見込んだ比較: a が nd 桁に丸めた値に見えれば、丸めの半分 + tol まで許す。
fn close_rounded(a: f64, b: f64, nd: i32, tol: f64) -> bool {
    let slack = if looks_rounded(a, nd) { 0.5 * 10f64.powi(-nd) } else { 0.0 };
    (a - b).abs() <= slack + tol
}

/// 仮説 1 つ(Python の hypotheses_log の 1 行)の食い違い。無ければ None。
pub fn hyp_diff(r: &Value, p: &Value) -> Option<String> {
    for k in ["index", "row", "mirror", "quad_field", "quad_index", "quick_rejected", "nmatch"] {
        if r[k] != p[k] {
            return Some(format!("{k} {} ≠ Python {}", r[k], p[k]));
        }
    }
    let (rc, pc) = (f64s(&r["crval"]), f64s(&p["crval"]));
    let (rv, pv) = (radec_to_xyz(rc[0], rc[1]), radec_to_xyz(pc[0], pc[1]));
    let sep = angle_between(rv, pv).to_degrees() * 3600.0;
    if sep.is_nan() || sep > 1e-6 {
        return Some(format!("CRVAL が {sep:.2e}″ 違う"));
    }
    let (rp, pp) = (f64s(&r["crpix"]), f64s(&p["crpix"]));
    if !(close(rp[0], pp[0], 1e-6, 0.0) && close(rp[1], pp[1], 1e-6, 0.0)) {
        return Some(format!("CRPIX ({:.9}, {:.9}) ≠ Python ({:.9}, {:.9})", rp[0], rp[1], pp[0], pp[1]));
    }
    let (rd, pd) = (f64s(&r["cd"]), f64s(&p["cd"]));
    let cmax = pd.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if rd.len() != 4 || (0..4).any(|k| !close(rd[k], pd[k], 1e-9 * cmax, 0.0)) {
        return Some(format!("CD {rd:?} ≠ Python {pd:?}"));
    }
    let (rs, ps) = (r["scale"].as_f64().unwrap_or(f64::NAN), p["scale"].as_f64().unwrap_or(f64::NAN));
    if !close_rounded(rs, ps, 6, 1e-9 * ps.abs()) {
        return Some(format!("スケール {rs} ≠ Python {ps}"));
    }
    match (r["logodds"].as_f64(), p["logodds"].as_f64()) {
        (None, None) => {}
        (Some(a), Some(b)) if close_rounded(a, b, 4, 1e-6) => {}
        (a, b) => return Some(format!("対数オッズ {a:?} ≠ Python {b:?}")),
    }
    None
}

/// Rust の trace(Python と同じ名前の項目)と、gen_solver.py が Python の trace から抜き出した要点を突き合わせる。
/// 見るもの: 解いた quad(画像側・索引側)、使った索引の一覧(数と sha256)、スケールの範囲、試した quad・当たり・
/// 仮説・足切りの数、対数オッズの最大、束ごとの経過、tune-up、最初の仮説(gen_solver の nhyp 個)、解いた仮説の順番。
/// 食い違いの説明の並び(空なら一致)。
pub fn trace_diff(rust: &Value, py: &Value) -> Vec<String> {
    use sha2::{Digest, Sha256};
    let mut bad = Vec::new();
    if rust["status"] != py["status"] {
        bad.push(format!("status {} ≠ Python {}", rust["status"], py["status"]));
    }
    // 解いた quad
    if py["solution"].is_object() {
        for k in ["quad_field", "quad_index", "index"] {
            if rust["solution"][k] != py["solution"][k] {
                bad.push(format!("解いた quad の {k} {} ≠ Python {}", rust["solution"][k], py["solution"][k]));
            }
        }
    }
    // 索引の一覧
    let files: Vec<String> = rust["files"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let sha: String = Sha256::digest(files.join("\n").as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    if Some(files.len() as u64) != py["files_count"].as_u64() || py["files_sha256"].as_str() != Some(sha.as_str()) {
        bad.push(format!("使った索引 {} 本 ≠ Python {} 本(一覧の sha256 も比べた)", files.len(), py["files_count"]));
    }
    let (rs, ps) = (f64s(&rust["scale_used"]), f64s(&py["scale_used"]));
    if rs.len() != ps.len() || rs.iter().zip(&ps).any(|(a, b)| !close(*a, *b, 0.0, 1e-12)) {
        bad.push(format!("スケールの範囲 {rs:?} ≠ Python {ps:?}"));
    }
    for k in ["quads_tried", "hits", "hypotheses", "quick_rejected"] {
        if rust[k].as_u64() != py[k].as_u64() {
            bad.push(format!("{k} {} ≠ Python {}", rust[k], py[k]));
        }
    }
    match (rust["logodds_max"].as_f64(), py["logodds_max"].as_f64()) {
        (None, None) => {}
        (Some(a), Some(b)) if (a - b).abs() <= 1e-6 => {}
        (a, b) => bad.push(format!("対数オッズの最大 {a:?} ≠ Python {b:?}")),
    }
    // 束ごとの経過
    let rf = rust["flushes"].as_array().cloned().unwrap_or_default();
    let pf = py["flushes"].as_array().cloned().unwrap_or_default();
    let nf = rf.len().min(pf.len());
    if let Some(k) = (0..nf).find(|&k| ["newest", "quads", "hits", "hyps"].iter().any(|f| rf[k][f].as_u64() != pf[k][f].as_u64())) {
        bad.push(format!("束 {k} の経過 {} ≠ Python {}", rf[k], pf[k]));
    } else if rf.len() != pf.len() {
        bad.push(format!("束の数 {} ≠ Python {}", rf.len(), pf.len()));
    }
    // tune-up
    let rt = rust["tuneups"].as_array().cloned().unwrap_or_default();
    let pt = py["tuneups"].as_array().cloned().unwrap_or_default();
    if rt.len() != pt.len()
        || rt.iter().zip(&pt).any(|(a, b)| {
            a["index"] != b["index"]
                || !close_rounded(a["before"].as_f64().unwrap_or(f64::NAN), b["before"].as_f64().unwrap_or(f64::NAN), 4, 1e-6)
                || !close_rounded(a["after"].as_f64().unwrap_or(f64::NAN), b["after"].as_f64().unwrap_or(f64::NAN), 4, 1e-6)
        })
    {
        bad.push(format!("tune-up {} 回 ≠ Python {} 回(か値が違う)", rt.len(), pt.len()));
    }
    // 最初の仮説
    let rh = rust["hypotheses_log"].as_array().cloned().unwrap_or_default();
    let ph = py["hyp_first"].as_array().cloned().unwrap_or_default();
    for (k, p) in ph.iter().enumerate() {
        match rh.get(k) {
            None => {
                bad.push(format!("仮説 {k} が Rust の記録に無い(記録 {} 個)", rh.len()));
                break;
            }
            Some(r) => {
                if let Some(m) = hyp_diff(r, p) {
                    bad.push(format!("仮説 {k}: {m}"));
                    break;
                }
            }
        }
    }
    // 解いた仮説の順番と中身
    if let Some(o) = py["solved_hyp"]["ordinal"].as_u64() {
        match rh.get(o as usize) {
            Some(r) => {
                if let Some(m) = hyp_diff(r, &py["solved_hyp"]) {
                    bad.push(format!("解いた仮説(Python では {o} 番目): {m}"));
                }
            }
            None => bad.push(format!("解いた仮説(Python では {o} 番目)が Rust の記録に無い")),
        }
    }
    bad
}

/// solve_detailed の解を Python と突き合わせる(solution_diff に、解いた quad の 4 点と最後の一致の並びを足したもの)。
pub fn detail_diff(d: &zahyou_solver::SolveDetail, py: &Value, w: usize, h: usize) -> Vec<String> {
    let mut bad = solution_diff(&d.solution, py, w, h);
    let s = &py["solution"];
    let qf: Vec<u64> = d.quad_field.iter().map(|&v| v as u64).collect();
    let qi: Vec<u64> = d.quad_index.iter().map(|&v| v as u64).collect();
    let pqf: Vec<u64> = s["quad_field"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64()).collect()).unwrap_or_default();
    let pqi: Vec<u64> = s["quad_index"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64()).collect()).unwrap_or_default();
    if qf != pqf || qi != pqi {
        bad.push(format!("解いた quad 画像 {qf:?}・索引 {qi:?} ≠ Python {pqf:?}・{pqi:?}"));
    }
    let pm: Vec<(u64, f64)> = s["matches"]
        .as_array()
        .map(|a| a.iter().map(|m| (m[0].as_u64().unwrap_or(u64::MAX), m[1].as_f64().unwrap_or(f64::NAN))).collect())
        .unwrap_or_default();
    let same = pm.len() == d.matches.len()
        && pm.iter().zip(&d.matches).all(|(p, r)| p.0 == r.field as u64 && (p.1 - r.dist_px).abs() <= 1e-6);
    if !same {
        bad.push(format!("最後の一致 {} 個が Python {} 個と違う(星の番号かずれ 1e-6 px)", d.matches.len(), pm.len()));
    }
    bad
}

/// JSON の hint([赤経, 赤緯, 半径] か null)。
pub fn hint_of(v: &Value) -> Option<(f64, f64, f64)> {
    let a = f64s(v);
    (a.len() == 3).then(|| (a[0], a[1], a[2]))
}

/// 赤経赤緯(度)の文字列(ログ用)。
pub fn radec_str(v: Vec3) -> String {
    let (ra, dec) = xyz_to_radec(v);
    format!("({ra:.5}, {dec:+.5})")
}

/// 一時フォルダー(target の下。試験ごとに名前を分ける)。
pub fn temp_dir(sub: &str) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("zahyou-solver-tests").join(sub);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("一時フォルダー");
    d
}

/// 結果の表を 1 行ずつ出す(cargo test -- --nocapture で見える)。
pub fn row(tag: &str, name: &str, msg: &str) {
    eprintln!("  {tag:<4} {name:<22} {msg}");
}

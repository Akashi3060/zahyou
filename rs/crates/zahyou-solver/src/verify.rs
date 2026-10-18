//! 仮説の検証(Python の `_sv_verify_hyps`・`_sv_quick_pass`・`_sv_series_cone`・`_sv_test_lists`・`_sv_sigma2`・
//! `_sv_verify_boxes`。設計書 3.7 節)。
//!
//! Python は仮説の束をまとめて numpy で検証するが、仮説どうしは互いに影響しない(参照星・σ・並びはどれも
//! 仮説ごと)ので、ここでは 1 仮説ずつ同じ計算をする(束の分け方は時間とメモリのためだけ)。並列にするのは呼ぶ側。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use zahyou_index::{Catalog, IndexData, IndexError, IndexFile};
use zahyou_wcs::linalg::inv2;
use zahyou_wcs::{ARCSEC_PER_RAD, Mat2, Vec3, Wcs};

use crate::Params;
use crate::clock::Abort;
use crate::field::Field;
use crate::lsq::deproject;
use crate::parts::{CoreResult, accumulate, healpix_side_rad, nearest_refs};

/// 検証の前の安い足切り: 並びの先頭何個の星に参照星が 1 つも無ければ捨てるか(Python の SV_QUICK_STARS)。
pub(crate) const QUICK_STARS: usize = 10;

// ------------------------------------------------------------------ 索引の持ち方 ---

/// 1 回の探索(か試行のはしご全体)で使う索引の mmap と、壊れていると分かったファイル。
pub(crate) struct Store<'a> {
    pub cat: &'a Catalog,
    /// 開いた表か、中身の問題(形式・この機械で読めない)。OS のエラーで開けなかったときは入れない(次に開き直す)
    data: Vec<OnceLock<Result<Arc<IndexData>, String>>>,
    /// 中身が壊れていると分かったファイル(files の番号 → 理由)。試行のはしごの間は持ち続ける
    pub broken: Mutex<BTreeMap<usize, String>>,
    /// 今は開けないファイル(files の番号 → 理由)。その探索の中でだけ外す([`Store::begin_search`] で忘れる)
    unavailable: Mutex<BTreeMap<usize, String>>,
}

impl<'a> Store<'a> {
    pub fn new(cat: &'a Catalog) -> Store<'a> {
        Store {
            cat,
            data: (0..cat.files.len()).map(|_| OnceLock::new()).collect(),
            broken: Mutex::new(BTreeMap::new()),
            unavailable: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn file(&self, i: usize) -> &'a IndexFile {
        &self.cat.files[i]
    }

    /// 探索 1 回の始まり。前の探索で開けなかったファイルを忘れ、この探索でもう一度開いてみる
    /// (ほかのプロセスが閉じていれば使える。中身の壊れたファイルは覚えたまま)。
    pub fn begin_search(&self) {
        self.unavailable.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// 索引の表(mmap)。中身が壊れていれば Corrupt、OS のエラーで開けなければ Unavailable。
    pub fn data(&self, i: usize) -> Result<&Arc<IndexData>, Abort> {
        // get_or_init の中で開かない: 開くときの木の検査は rayon で並列に走り、待つ間に横取りした仕事(並列の検証)が
        // 同じ索引を開こうとすると、同じスレッドで初期化中のセルを待って止まる。外で開いて先に入ったほうを使う
        let cell = &self.data[i];
        let name = &self.cat.files[i].name;
        if cell.get().is_none() {
            match self.cat.files[i].data() {
                // OS のエラー(共有違反・権限など)は一時的なことがあるので覚えない
                Err(e @ IndexError::Io(_)) => return Err(Abort::Unavailable { file: i, msg: format!("{name}: {e}") }),
                r => {
                    let _ = cell.set(r.map_err(|e| e.to_string()));
                }
            }
        }
        let Some(r) = cell.get() else {
            return Err(Abort::Corrupt { file: i, msg: format!("{name}: 索引を開けません") });
        };
        match r {
            Ok(d) => Ok(d),
            Err(e) => Err(Abort::Corrupt { file: i, msg: format!("{name}: {e}") }),
        }
    }

    pub fn is_broken(&self, i: usize) -> bool {
        self.broken.lock().map(|b| b.contains_key(&i)).unwrap_or_else(|e| e.into_inner().contains_key(&i))
    }

    /// この探索で使わないファイルか(中身が壊れている・今は開けない)。
    pub fn is_skipped(&self, i: usize) -> bool {
        self.is_broken(i) || self.is_unavailable(i)
    }

    pub fn is_unavailable(&self, i: usize) -> bool {
        self.unavailable.lock().map(|b| b.contains_key(&i)).unwrap_or_else(|e| e.into_inner().contains_key(&i))
    }

    pub fn mark_broken(&self, i: usize, why: &str) {
        let mut b = self.broken.lock().unwrap_or_else(|e| e.into_inner());
        b.entry(i).or_insert_with(|| why.to_string());
    }

    pub fn mark_unavailable(&self, i: usize, why: &str) {
        let mut b = self.unavailable.lock().unwrap_or_else(|e| e.into_inner());
        b.entry(i).or_insert_with(|| why.to_string());
    }
}

/// 系列 1 つぶんの情報(検証で同じ系列の全タイルから参照星を集めるため)。
pub(crate) struct SeriesCtx<'s, 'a> {
    pub store: &'s Store<'a>,
    /// 系列のファイル(files の番号)。並びの位置 = Python の fid
    pub files: Vec<usize>,
    /// この探索で使わないファイル(中身が壊れている・今は開けない。fid ごと。このときの写し)
    pub broken: Vec<bool>,
    /// タイルの中心と外接半径と、その cos(fid ごと。全天は None)
    pub tiles: Vec<Option<(Vec3, f64, f64)>>,
    pub allsky: bool,
    /// 系列の代表(JITTER・CUTNSIDE・HPNSIDE は系列の中で同じ)
    pub rep: &'a IndexFile,
}

impl<'s, 'a> SeriesCtx<'s, 'a> {
    pub fn new(store: &'s Store<'a>, iid: u32, rep_file: usize) -> SeriesCtx<'s, 'a> {
        let files = store.cat.series.get(&iid).cloned().unwrap_or_default();
        let broken = files.iter().map(|&i| store.is_skipped(i)).collect();
        let tiles = files
            .iter()
            .map(|&i| {
                let f = store.file(i);
                match (f.tile_center, f.tile_radius_rad) {
                    (Some(c), Some(r)) => Some((c, r, r.cos())),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        let allsky = tiles.first().is_none_or(|t| t.is_none());
        SeriesCtx { store, files, broken, tiles, allsky, rep: store.file(rep_file) }
    }
}

// ------------------------------------------------------------------ 参照星 ---

/// 円錐検索で集めた参照星(系列の全タイルから。タイルの順、タイルの中は星の番号の順)。
#[derive(Default, Clone)]
pub(crate) struct Refs {
    pub fid: Vec<u32>,
    pub sid: Vec<u32>,
    pub xyz: Vec<Vec3>,
    pub sweep: Vec<u8>,
}

/// 系列の全タイルから、中心 c(単位ベクトル)の半径 r(ラジアン)の円の中の星を集める(Python の `_sv_series_cone`)。
///
/// タイルは「中心どうしの内積 ≥ cos(min(外接半径 + r + 1e-9, π))」で選ぶ(Python と同じ)。判定は stars_in_cone と同じ。
pub(crate) fn series_cone(sc: &SeriesCtx, c: Vec3, r: f64) -> Result<Refs, Abort> {
    let mut out = Refs::default();
    for_tiles(sc, c, r, |fid, data| {
        for id in data.stars_in_cone(c, r) {
            let (Some(v), Some(sw)) = (data.star_xyz(id), data.sweep(id)) else { continue };
            out.fid.push(fid as u32);
            out.sid.push(id);
            out.xyz.push(v);
            out.sweep.push(sw);
        }
        Ok(false)
    })?;
    Ok(out)
}

/// タイル(中心 tc、外接半径 tr、cos_tr = cos(tr))が円 (c, r) に届くか(Python の `_sv_series_cone` と同じ判定:
/// 中心どうしの内積 ≥ cos(min(tr + r + 1e-9, π)))。
///
/// 判定は同じまま、cos を計算するのは境目のタイルだけにする。x = min(tr + r + 1e-9, π) は tr 以上なので
/// cos x ≤ cos tr、また cos は傾きが 1 以下なので cos x ≥ cos tr − (r + 1e-9)。cos と足し算の丸め(合わせて 1e-15 未満)の分を
/// 4e-15 だけ広げて、内積がその外なら確実に「届かない」「届く」と決める。
#[inline]
fn tile_reaches(dot: f64, tr: f64, cos_tr: f64, r: f64) -> bool {
    if r.is_finite() && r >= 0.0 && dot.is_finite() {
        if dot < cos_tr - (r + 1e-9) - 4e-15 {
            return false;
        }
        if dot >= cos_tr + 4e-15 {
            return true;
        }
    }
    // np.minimum と同じく NaN はそのまま(cos も NaN で、届かない)
    let x = tr + r + 1e-9;
    dot >= (if x > std::f64::consts::PI { std::f64::consts::PI } else { x }).cos()
}

/// 円 (c, r) に届くタイルごとに f(fid, 表) を呼ぶ。f が true を返したらそこでやめて true。
pub(crate) fn for_tiles(
    sc: &SeriesCtx,
    c: Vec3,
    r: f64,
    mut f: impl FnMut(usize, &IndexData) -> Result<bool, Abort>,
) -> Result<bool, Abort> {
    if sc.files.is_empty() {
        return Ok(false);
    }
    if sc.allsky {
        if sc.broken[0] {
            return Ok(false);
        }
        let d = sc.store.data(sc.files[0])?;
        return f(0, d);
    }
    for (fid, &file) in sc.files.iter().enumerate() {
        if sc.broken[fid] {
            continue;
        }
        let Some((tc, tr, cos_tr)) = sc.tiles[fid] else { continue };
        let dot = tc[0] * c[0] + tc[1] * c[1] + tc[2] * c[2];
        if !tile_reaches(dot, tr, cos_tr, r) {
            continue;
        }
        let d = sc.store.data(file)?;
        if f(fid, d)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// ------------------------------------------------------------------ 仮説 ---

/// 照合と 4 点の当てはめで作った仮説 1 つ(Python の仮説の束の 1 行)。
#[derive(Clone, Debug)]
pub(crate) struct Hyp {
    pub crval: Vec3,
    pub east: Vec3,
    pub north: Vec3,
    pub cd: Mat2,
    pub cdinv: Option<Mat2>,
    pub crpix: [f64; 2],
    /// ″/px
    pub scale: f64,
    pub dq: usize,
    /// 画像の星(入力の添字)。索引の quad の A, B, C(, D) の順
    pub qfield: [usize; 4],
    /// 索引の星の番号
    pub qindex: [u32; 4],
    /// quad の索引の、系列の中の位置
    pub fid: usize,
    /// 画像側の quad の中心 (A+B)/2 と |AB|²/4
    pub qc: [f64; 2],
    pub q2: f64,
    /// 照合した画像の quad の行(newpoint の順)・鏡像の形か
    pub row: usize,
    pub mirror: bool,
}

impl Hyp {
    /// この仮説の TAN を WCS に置き換えたもの(検証の参照星の範囲・σ を決めるのに使う。Python の `_sv_hyp_from_wcs`)。
    pub fn with_wcs(&self, wcs: &Wcs) -> Hyp {
        let (east, north) = zahyou_wcs::tan_basis(wcs.crval);
        Hyp {
            crval: wcs.crval,
            east,
            north,
            cd: wcs.cd,
            cdinv: inv2(&wcs.cd),
            crpix: wcs.crpix,
            scale: wcs.pixel_scale_arcsec(),
            ..self.clone()
        }
    }

    /// 画素 → 単位ベクトル(この仮説の TAN。Python の `_sv_pix_to_xyz_many`)。
    #[inline]
    pub fn pix_to_xyz(&self, x: f64, y: f64) -> Vec3 {
        let rad = std::f64::consts::PI / 180.0;
        let u = x - self.crpix[0];
        let v = y - self.crpix[1];
        let xi = (self.cd[0][0] * u + self.cd[0][1] * v) * rad;
        let eta = (self.cd[1][0] * u + self.cd[1][1] * v) * rad;
        deproject(xi, eta, self.crval, self.east, self.north)
    }

    /// 単位ベクトル → 画素(この仮説の TAN。Python の `_sv_project_many`)。接点の反対側なら None。
    #[inline]
    pub fn project(&self, s: Vec3) -> Option<(f64, f64)> {
        let deg = 180.0 / std::f64::consts::PI;
        let w = s[0] * self.crval[0] + s[1] * self.crval[1] + s[2] * self.crval[2];
        if !(w > 0.0) {
            return None;
        }
        let ci = self.cdinv?;
        let xi = (s[0] * self.east[0] + s[1] * self.east[1] + s[2] * self.east[2]) / w * deg;
        let eta = (s[0] * self.north[0] + s[1] * self.north[1] + s[2] * self.north[2]) / w * deg;
        Some((ci[0][0] * xi + ci[0][1] * eta + self.crpix[0], ci[1][0] * xi + ci[1][1] * eta + self.crpix[1]))
    }

    #[inline]
    pub fn is_quad_star(&self, fid: usize, sid: u32) -> bool {
        fid == self.fid && self.qindex[..self.dq].contains(&sid)
    }
}

/// 検証の一様化の箱の数 (nw, nh)。箱 = 索引の CUTNSIDE の HEALPix の 1 辺(Python の `_sv_verify_boxes`)。
pub(crate) fn verify_boxes(field: &Field, rep: &IndexFile, scale: f64) -> (usize, usize) {
    match rep.cutnside {
        Some(cut) if cut > 0 => {
            let cutpix = healpix_side_rad(cut) * ARCSEC_PER_RAD / scale;
            let conv = |v: f64| {
                let f = (v / cutpix + 0.5).floor();
                // 大きすぎる値は抑える(箱の数がおかしくなるだけで、どのみち 1 箱 1 星以下になる)
                if f >= 1.0 { f.min(1e6) as usize } else { 1 }
            };
            (conv(field.w), conv(field.h))
        }
        _ => (1, 1),
    }
}

/// 仮説の画像の星の並び(Python の `_sv_test_lists` の 1 行)。
///
/// 検証の一様化の並び(箱 nw × nh)の先頭 ncand 個から、quad の星と RoR の外(qc から距離² > ror2)の星を除いて
/// 先頭 ncols 個(vxy の行)。
pub(crate) fn test_list(field: &Field, h: &Hyp, nw: usize, nh: usize, ror2: f64, ncols: usize, ncand: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(ncols.min(field.vxy.len()));
    test_list_each(field, h, nw, nh, ror2, ncols, ncand, |c| {
        out.push(c);
        Ok(false)
    })
    .ok();
    out
}

/// [`test_list`] の並びを順に f に渡す(割り当てなし)。f が true を返したらそこでやめて true。
#[allow(clippy::too_many_arguments)]
fn test_list_each(
    field: &Field,
    h: &Hyp,
    nw: usize,
    nh: usize,
    ror2: f64,
    ncols: usize,
    ncand: usize,
    mut f: impl FnMut(usize) -> Result<bool, Abort>,
) -> Result<bool, Abort> {
    let nv = field.vxy.len();
    if nv == 0 || ncols == 0 {
        return Ok(false);
    }
    let m = nv.min(ncand);
    let vo = field.verify_order(nw, nh);
    let mut skip = [usize::MAX; 4];
    for (k, &qf) in h.qfield[..h.dq].iter().enumerate() {
        let vp = field.vpos.get(qf).copied().unwrap_or(-1);
        if vp >= 0 {
            skip[k] = vo.pos[vp as usize];
        }
    }
    let mut n = 0usize;
    for (j, &c) in vo.order[..m].iter().enumerate() {
        if skip.contains(&j) {
            continue;
        }
        let p = field.vxy[c];
        let (dx, dy) = (p[0] - h.qc[0], p[1] - h.qc[1]);
        if dx * dx + dy * dy > ror2 {
            continue;
        }
        if f(c)? {
            return Ok(true);
        }
        n += 1;
        if n >= ncols {
            break;
        }
    }
    Ok(false)
}

/// 位置の分散 σᵢ² = pix² × (1 + γ·Rᵢ²/Q²)(Rᵢ = 星 i と quad の中心の距離。Python の `_sv_sigma2`)。
#[inline]
pub(crate) fn sigma2(field: &Field, h: &Hyp, t: usize, pix2: f64, gamma: f64) -> f64 {
    let p = field.vxy[t];
    let (dx, dy) = (p[0] - h.qc[0], p[1] - h.qc[1]);
    let r2 = dx * dx + dy * dy;
    pix2 * (1.0 + gamma * r2 / h.q2)
}

/// σ の「画素の誤差 + 索引の JITTER」の分(pix² = verify_pix² + (jitter / scale)²)。
#[inline]
pub(crate) fn pix2_of(params: &Params, rep: &IndexFile, scale: f64) -> f64 {
    let j = rep.jitter_arcsec / scale;
    params.verify_pix * params.verify_pix + j * j
}

// ------------------------------------------------------------------ 検証 ---

/// 検証の結果(Python の `_sv_verify_hyps` の 1 行)。
#[derive(Clone, Debug)]
pub(crate) struct VerifyOut {
    pub best: f64,
    pub nmatch: usize,
    pub ndistract: usize,
    /// 参照星の数(RoR の中)と、使った画像の星の数
    pub nr: usize,
    pub nt: usize,
    pub quick_rejected: bool,
    /// record のときだけ: (画像の星の番号(入力の添字), 参照星の単位ベクトル, ずれ [px], 1 つの一致の対数オッズ)
    pub matches: Vec<(usize, Vec3, f64, f64)>,
}

impl VerifyOut {
    pub fn rejected(quick: bool) -> VerifyOut {
        VerifyOut {
            best: f64::NEG_INFINITY,
            nmatch: 0,
            ndistract: 0,
            nr: 0,
            nt: 0,
            quick_rejected: quick,
            matches: Vec::new(),
        }
    }
}

/// 参照星の投影のしかた(仮説の TAN か、改良した WCS)。
#[derive(Clone, Copy)]
pub(crate) enum Proj<'w> {
    Tan,
    Wcs(&'w Wcs),
}

/// 仮説 1 つを検証する(3.7 節の 1〜5。安い足切りはかけない)。
///
/// refs を渡さなければ、画像中心から半径 hypot(W, H)/2 × スケール × 1.05 の円の中の星を系列の全タイルから集める。
/// gamma は σ の「quad からの距離による広がり」の掛け率(改良のとき 1 未満)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_one(
    field: &Field,
    sc: &SeriesCtx,
    params: &Params,
    h: &Hyp,
    proj: Proj,
    refs: Option<&Refs>,
    gamma: f64,
    record: bool,
) -> Result<VerifyOut, Abort> {
    let owned;
    let refs = match refs {
        Some(r) => r,
        None => {
            let cen = match proj {
                Proj::Wcs(w) => w.pix_to_xyz(field.center[0], field.center[1]),
                Proj::Tan => h.pix_to_xyz(field.center[0], field.center[1]),
            };
            let radius = field.half_diag * h.scale * 1.05 / ARCSEC_PER_RAD;
            owned = series_cone(sc, cen, radius)?;
            &owned
        }
    };
    let (w, hh) = (field.w, field.h);
    let d = params.distractor;
    // 1. 参照星: 画像の中に写るもの(quad の星は外す)を sweep・タイル・番号の順に
    let mut sel: Vec<(u8, u32, u32, usize, f64, f64)> = Vec::with_capacity(refs.sid.len());
    for k in 0..refs.sid.len() {
        let p = match proj {
            Proj::Tan => h.project(refs.xyz[k]),
            Proj::Wcs(wc) => wc.xyz_to_pix(refs.xyz[k]),
        };
        let Some((rx, ry)) = p else { continue };
        if !(rx >= -0.5 && rx <= w - 0.5 && ry >= -0.5 && ry <= hh - 0.5) {
            continue;
        }
        if h.is_quad_star(refs.fid[k] as usize, refs.sid[k]) {
            continue;
        }
        sel.push((refs.sweep[k], refs.fid[k], refs.sid[k], k, rx, ry));
    }
    sel.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    let nr0 = sel.len() as f64;
    // 2・3. 位置の分散と関連半径(RoR)
    let pix2 = pix2_of(params, sc.rep, h.scale);
    let fac = w * hh * (1.0 - d) / (4.0 * std::f64::consts::PI * nr0.max(1.0) * pix2) - 1.0;
    let ror2 = h.q2 * crate::parts::py_max(1.0, fac);
    sel.retain(|e| {
        let (dx, dy) = (e.4 - h.qc[0], e.5 - h.qc[1]);
        dx * dx + dy * dy <= ror2
    });
    let nr = sel.len();
    let inside = field
        .grid
        .iter()
        .filter(|g| {
            let (dx, dy) = (g[0] - h.qc[0], g[1] - h.qc[1]);
            dx * dx + dy * dy <= ror2
        })
        .count();
    let frac = inside as f64 / field.grid.len() as f64;
    let area = w * hh * crate::parts::py_max(frac, 1.0 / field.grid.len() as f64);
    let logbg = -area.ln();
    if nr == 0 {
        return Ok(VerifyOut::rejected(false));
    }
    // 4. 画像の星の並び(検証の一様化。箱 = 索引の CUTNSIDE の HEALPix の 1 辺)
    let (nw, nh) = verify_boxes(field, sc.rep, h.scale);
    let nv = field.vxy.len();
    let test = test_list(field, h, nw, nh, ror2, nv, nv);
    let nt = test.len();
    let test_idx: Vec<i64> = test.iter().map(|&t| t as i64).collect();
    let sig2: Vec<f64> = test.iter().map(|&t| sigma2(field, h, t, pix2, gamma)).collect();
    let ref_xy: Vec<[f64; 2]> = sel.iter().map(|e| [e.4, e.5]).collect();
    // 5. 積み上げ
    let (nn, nd2) = nearest_refs(&field.vxy, &test_idx, &sig2, &ref_xy);
    let core: CoreResult = accumulate(&test_idx, &sig2, &nn, &nd2, nr, logbg, params, record);
    let mut matches = Vec::new();
    if record {
        for m in &core.matches {
            let t = m.test;
            let e = &sel[m.reference];
            let dist = (field.vxy[t][0] - e.4).hypot(field.vxy[t][1] - e.5);
            matches.push((field.vidx[t], refs.xyz[e.3], dist, m.logodds));
        }
    }
    Ok(VerifyOut {
        best: core.best,
        nmatch: core.nmatch,
        ndistract: core.ndistract,
        nr,
        nt,
        quick_rejected: false,
        matches,
    })
}

/// 検証の前の安い足切り(Python の `_sv_quick_pass`)。検証の並びの先頭 QUICK_STARS 個の画像の星のどれかから
/// 5σ 以内に系列の星(quad の星を除く)があれば true。
pub(crate) fn quick_pass(field: &Field, sc: &SeriesCtx, params: &Params, h: &Hyp, gamma: f64) -> Result<bool, Abort> {
    let k = QUICK_STARS;
    let (nw, nh) = verify_boxes(field, sc.rep, h.scale);
    let nv = field.vxy.len();
    let pix2 = pix2_of(params, sc.rep, h.scale);
    test_list_each(field, h, nw, nh, f64::INFINITY, k, nv.min(k + h.dq), |t| {
        let s2 = sigma2(field, h, t, pix2, gamma);
        let p = field.vxy[t];
        let c = h.pix_to_xyz(p[0], p[1]);
        let rad = 5.0 * s2.sqrt() * h.scale / ARCSEC_PER_RAD;
        for_tiles(sc, c, rad, |fid, data| Ok(data.stars_in_cone(c, rad).into_iter().any(|sid| !h.is_quad_star(fid, sid))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_reaches_matches_exact_formula() {
        // 境目の近くを含めて、cos を毎回計算する形(Python と同じ式)と同じ答えになる
        let mut s = 0x2545F4914F6CDD1Du64;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut near = 0;
        for k in 0..2_000_000 {
            let tr = rnd() * 0.6;
            let r = if k % 3 == 0 { rnd() * 1e-4 } else { rnd() * 0.05 };
            let exact_cos = (tr + r + 1e-9).min(std::f64::consts::PI).cos();
            // 内積は境目のまわり(± 1e-12)か、全体から
            let dot = if k % 2 == 0 { exact_cos + (rnd() - 0.5) * 2e-12 } else { rnd() * 2.0 - 1.0 };
            let want = dot >= exact_cos;
            assert_eq!(tile_reaches(dot, tr, tr.cos(), r), want, "tr={tr} r={r} dot={dot}");
            if (dot - exact_cos).abs() < 1e-12 {
                near += 1;
            }
        }
        assert!(near > 100_000);
        // r が NaN・負なら式どおり(NaN は届かない)
        assert!(!tile_reaches(0.9, 0.1, 0.1f64.cos(), f64::NAN));
        assert_eq!(tile_reaches(0.995, 0.1, 0.1f64.cos(), -0.05), 0.995 >= (0.1f64 - 0.05 + 1e-9).cos());
    }
}

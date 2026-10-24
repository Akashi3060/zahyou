//! 探索の本体(Python の `sv_solve`・`_sv_search`・`_sv_flush`・`_sv_flush_series`・`_sv_match_file`・
//! `_sv_verify_and_pick`・`_sv_refine`・`_sv_hint_tiles`。設計書 3 節)。
//!
//! 探す順と判定は Python と同じ。並列にするのは、索引のファイルごとの照合、当たりごとの安い棄却と 4 点の当てはめ、
//! 検証の束の中の仮説ごとの検証(足切りと本検証)だけで、どれも結果を Python の順に並べてから選ぶ
//! (区切りごとに newpoint の順で最初に solve を超えた仮説を採る)ので、スレッドの数によらず同じ解になる。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Instant;

use rayon::prelude::*;
use zahyou_index::{IndexFile, healpix_xy_from_xyz, healpix_xy_point};
use zahyou_wcs::{ARCSEC_PER_RAD, Vec3, Wcs, angle_between, linalg::inv2, radec_to_xyz, xyz_to_radec};

use crate::clock::{Abort, Clock};
use crate::field::Field;
use crate::lsq::{LSQ_MAX_SHIFTS, fit_wcs_lsq, normalize3, wrms};
use crate::parts::{
    CodeVariants, SIP_F_Z, accept_logodds, code_variants, f_upper, field_codes, fit_similarity_quad, logodds_weight,
    quad_ab_range, quad_geometry, quads_newest_into, tuneup_accepted,
};
use crate::pyfmt::{fmt_g, fmt_int};
use crate::verify::{Hyp, Proj, Refs, SeriesCtx, Store, VerifyOut, quick_pass, series_cone, verify_one};
use crate::Params;

/// 最初の束の quad の数(このあと束ごとに倍、MAX_BATCH まで)。
const FIRST_BATCH: usize = 24;
const MAX_BATCH: usize = 768;
/// 系列の仮説をこの数ずつ検証し、そのたびに解を選ぶ(Python の SV_VERIFY_STEP。仮説の数の数え方もこれに合わせる)。
const VERIFY_STEP: usize = 8192;
/// 改良の反復の回数と、反復ごとに quad からの距離による σ の広がりを弱める割合。
const TWEAK_STEPS: usize = 10;
const TWEAK_GAMMA: f64 = 0.7;
/// 改良の最後の当て直しを一致だけで行う一致の数。
const TWEAK_MIN_MATCHES: usize = 6;
/// SIP を付けるには、当てる星が画像を 3 × 3 に分けた箱の 7 つ以上にあること。
const SIP_GRID: usize = 3;
const SIP_MIN_BOXES: usize = 7;
/// trace に書く仮説の数の上限。
pub(crate) const TRACE_MAX_HYP: usize = 20000;
/// code_ball で一度にたどる問い合わせの数(区切りごとに締め切りと中止を見る。Python の SV_BALL_CHUNK は 2048 だが、
/// 答えは区切り方によらないので、中止の遅れを短くするため小さくした。1 区切りは大きな索引でも 10 ms ほど)。
const BALL_CHUNK: usize = 512;
/// 当たりの安い棄却と 4 点の当てはめを並列にする最小の数。
const PAR_MIN_HITS: usize = 256;

/// 探索の入力。
pub(crate) struct SearchInput<'a> {
    pub xy: &'a [[f64; 2]],
    pub flux: Option<&'a [f64]>,
    pub width: f64,
    pub height: f64,
    pub scale_lo: Option<f64>,
    pub scale_hi: Option<f64>,
    /// (赤経, 赤緯, 半径)(度)。値は確かめ済み
    pub hint: Option<(f64, f64, f64)>,
}

/// 探索で見つけた解(Python の SvSolution)。
#[derive(Clone, Debug)]
pub(crate) struct Found {
    pub wcs: Wcs,
    pub logodds: f64,
    pub nmatch: usize,
    pub ndistract: usize,
    pub nfield: usize,
    pub nref: usize,
    pub index_name: String,
    pub quad_field: Vec<usize>,
    pub quad_index: Vec<u32>,
    pub quads_tried: u64,
    pub hypotheses: u64,
    pub seconds: f64,
    pub matches: Vec<(usize, Vec3, f64)>,
}

/// 束 1 つの経過(trace の flushes)。
#[derive(Clone, Debug, Default)]
pub(crate) struct FlushRec {
    pub newest: usize,
    pub quads: u64,
    pub hits: u64,
    pub hyps: u64,
    pub t: f64,
}

/// 検証した仮説 1 つ(trace の hypotheses_log)。
#[derive(Clone, Debug)]
pub(crate) struct HypLog {
    pub index: String,
    pub row: usize,
    pub mirror: bool,
    pub quad_field: Vec<usize>,
    pub quad_index: Vec<u32>,
    pub crval: (f64, f64),
    pub crpix: [f64; 2],
    pub cd: [[f64; 2]; 2],
    pub scale: f64,
    pub quick_rejected: bool,
    pub logodds: Option<f64>,
    pub nmatch: usize,
}

/// 探索の途中の数(Python の st)。
pub(crate) struct SearchState {
    pub t0: Instant,
    pub trace: bool,
    pub quads: u64,
    pub hits: u64,
    pub hyps: u64,
    pub quick_rejected: u64,
    pub logodds_max: f64,
    pub flushes: Vec<FlushRec>,
    pub hyp_log: Vec<HypLog>,
    pub tuneups: Vec<(String, f64, f64)>,
    pub broken: Vec<String>,
    pub bad_series: BTreeSet<u32>,
    pub files: Vec<String>,
    pub scale: Option<(f64, f64)>,
}

impl SearchState {
    pub fn new(t0: Instant, trace: bool) -> SearchState {
        SearchState {
            t0,
            trace,
            quads: 0,
            hits: 0,
            hyps: 0,
            quick_rejected: 0,
            logodds_max: f64::NEG_INFINITY,
            flushes: Vec::new(),
            hyp_log: Vec::new(),
            tuneups: Vec::new(),
            broken: Vec::new(),
            bad_series: BTreeSet::new(),
            files: Vec::new(),
            scale: None,
        }
    }
}

/// スケール不明のときの範囲(″/px。Python の `sv_blind_scale_bounds`)。手持ちの段の quad の大きさから画像の幅で
/// 逆算する(下限 × 0.9、上限 × 1.1)。
pub(crate) fn blind_scale_bounds(cat: &zahyou_index::Catalog, width: f64) -> (f64, f64) {
    let (lo_fov, hi_fov) = cat.coverage_arcmin().unwrap_or((2.0, 2000.0));
    let low = lo_fov * 60.0 / width * 0.9;
    let high = hi_fov * 60.0 / width * 1.1;
    (crate::parts::py_max(low, 0.01), if 3600.0 < high { 3600.0 } else { high })
}

/// 照合に使う索引 1 本(Python の info の 1 行)。
#[derive(Clone, Copy)]
struct Member {
    file: usize,
    fid: usize,
}

/// 足した星の束の、quad の大きさ(3 星・4 星)ごとの照合の材料。
struct Batch {
    dq: usize,
    quads: Vec<[usize; 4]>,
    ab: Vec<f64>,
    var: CodeVariants,
}

// ------------------------------------------------------------------ 本体 ---

/// 探索の本体(Python の `_sv_search`)。
pub(crate) fn search(
    inp: &SearchInput,
    store: &Store,
    params: &Params,
    clock: Clock,
    log: &dyn Fn(&str),
    st: &mut SearchState,
) -> Result<Option<Found>, Abort> {
    let (w, h) = (inp.width, inp.height);
    // 前の試行で開けなかった索引も、この探索でもう一度開いてみる
    store.begin_search();
    let field = Field::new(inp.xy, inp.flux, w, h, params);
    clock.check()?;
    if field.n < 3 {
        log(&format!("星が {} 個しかないので解けません", field.n));
        return Ok(None);
    }
    let cat = store.cat;
    let (b_lo, b_hi) = blind_scale_bounds(cat, w);
    let mut s_lo = inp.scale_lo.unwrap_or(b_lo);
    let mut s_hi = inp.scale_hi.unwrap_or(b_hi);
    if s_hi < s_lo {
        std::mem::swap(&mut s_lo, &mut s_hi);
    }
    let hint_t: Option<(Vec3, f64)> = inp.hint.map(|(ra, dec, r)| (radec_to_xyz(ra, dec), r.to_radians()));
    let mut files: Vec<usize> = cat.try_select(s_lo, s_hi, w, h, hint_t).map_err(Abort::Invalid)?;
    files.retain(|&i| !store.is_skipped(i));
    if let Some(ht) = hint_t {
        files = hint_tiles(store, &files, &field, s_hi, ht, params);
    }
    st.files = files.iter().map(|&i| store.file(i).name.clone()).collect();
    st.scale = Some((s_lo, s_hi));
    if files.is_empty() {
        log("使える星図データ(索引)がありません");
        return Ok(None);
    }
    // 索引ごとの AB の範囲と、系列の中の位置
    let tol = params.codetol;
    let mut info: Vec<(Member, f64, f64)> = Vec::new();
    for &i in &files {
        let f = store.file(i);
        let (lo, hi) = quad_ab_range(f.scale_lo_rad, f.scale_hi_rad, s_lo, s_hi, w, h, tol, params.quad_min_frac);
        let Some(fid) = cat.series.get(&f.indexid).and_then(|s| s.iter().position(|&k| k == i)) else { continue };
        if hi >= lo && f.dimquads <= field.nq.max(3) {
            info.push((Member { file: i, fid }, lo, hi));
        }
    }
    if info.is_empty() {
        return Ok(None);
    }
    let mut ab: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
    for (m, lo, hi) in &info {
        let dq = store.file(m.file).dimquads;
        let e = ab.entry(dq).or_insert((*lo, *hi));
        e.0 = e.0.min(*lo);
        e.1 = e.1.max(*hi);
    }
    let dqs: Vec<usize> = ab.keys().copied().collect();
    {
        let series: BTreeSet<u32> = info.iter().map(|(m, _, _)| store.file(m.file).indexid).collect();
        let names: Vec<String> = series.iter().map(|s| s.to_string()).collect();
        let mut line = format!(
            "星 {} 個(quad に {} 個)、スケール {}〜{}″/px、索引 {} 本(系列 {})",
            field.n,
            field.nq,
            fmt_g(s_lo, 3),
            fmt_g(s_hi, 3),
            info.len(),
            names.join(", ")
        );
        if let Some((ra, dec, r)) = inp.hint {
            line.push_str(&format!("、ヒント ({ra:.3}, {dec:+.3}) 半径 {}°", fmt_g(r, 6)));
        }
        log(&line);
    }
    let members: Vec<Member> = info.iter().map(|(m, _, _)| *m).collect();
    let geom = quad_geometry(&field.p, tol);
    let mut pending: BTreeMap<usize, Vec<[usize; 4]>> = dqs.iter().map(|&d| (d, Vec::new())).collect();
    let mut npend = 0usize;
    let mut target = FIRST_BATCH;
    for n in 0..field.nq {
        for &dq in &dqs {
            let (lo, hi) = ab[&dq];
            let v = pending.get_mut(&dq).expect("dq");
            let before = v.len();
            quads_newest_into(n, dq, &geom, lo * lo, hi * hi, v);
            npend += v.len() - before;
        }
        if npend < target && n + 1 < field.nq {
            continue;
        }
        if npend == 0 {
            continue;
        }
        clock.check()?;
        let batch = std::mem::replace(&mut pending, dqs.iter().map(|&d| (d, Vec::new())).collect());
        npend = 0;
        let sol = flush(&field, store, &members, batch, s_lo, s_hi, hint_t, params, clock, log, st, n)?;
        target = (target * 2).min(MAX_BATCH);
        if let Some(mut sol) = sol {
            sol.seconds = st.t0.elapsed().as_secs_f64();
            log(&format!(
                "解けました: {}、対数オッズ {:.1}、一致 {} 個、quad {} 個・仮説 {} 個、{:.2} 秒",
                sol.index_name,
                sol.logodds,
                sol.nmatch,
                fmt_int(sol.quads_tried),
                fmt_int(sol.hypotheses),
                sol.seconds
            ));
            return Ok(Some(sol));
        }
    }
    log(&format!(
        "quad を全部試しても解けませんでした(quad {} 個・仮説 {} 個、対数オッズの最大 {:.1}、{:.1} 秒)",
        fmt_int(st.quads),
        fmt_int(st.hyps),
        st.logodds_max,
        st.t0.elapsed().as_secs_f64()
    ));
    Ok(None)
}

/// たまった quad の束を、選んだ索引に順に照合して検証する(Python の `_sv_flush`)。
#[allow(clippy::too_many_arguments)]
fn flush(
    field: &Field,
    store: &Store,
    members: &[Member],
    pending: BTreeMap<usize, Vec<[usize; 4]>>,
    s_lo: f64,
    s_hi: f64,
    hint_t: Option<(Vec3, f64)>,
    params: &Params,
    clock: Clock,
    log: &dyn Fn(&str),
    st: &mut SearchState,
    newest: usize,
) -> Result<Option<Found>, Abort> {
    let mut batches: BTreeMap<usize, Batch> = BTreeMap::new();
    let mut nq = 0u64;
    for (dq, quads) in pending {
        if quads.is_empty() {
            continue;
        }
        nq += quads.len() as u64;
        let codes = field_codes(&field.p, &quads, dq);
        let var = code_variants(&codes, dq, params.codetol, params.parity);
        let ab = quads
            .iter()
            .map(|q| {
                let (a, b) = (field.p[q[0]], field.p[q[1]]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                (dx * dx + dy * dy).sqrt()
            })
            .collect();
        batches.insert(dq, Batch { dq, quads, ab, var });
    }
    st.quads += nq;
    st.flushes.push(FlushRec { newest, quads: nq, ..Default::default() });
    let fl = st.flushes.len() - 1;
    // 系列ごとの組(info の順に、同じ系列が続くものをまとめる。同じ系列は 1 か所に並ぶ)
    let mut groups: Vec<(u32, Vec<Member>)> = Vec::new();
    for m in members {
        let iid = store.file(m.file).indexid;
        match groups.last_mut() {
            Some((g, v)) if *g == iid => v.push(*m),
            _ => groups.push((iid, vec![*m])),
        }
    }
    // 照合は系列の順によらず同じ答えになるので、索引の数がスレッドの数に届くまで後ろの系列もまとめて並列に
    // 照合しておく(全天の索引は 1 本ずつが別の系列なので、1 つずつだと並列にならない)。検証・当たりの数・壊れた
    // 索引の扱いは Python と同じく系列の順に 1 つずつ行い、解けたらそこでやめる(先に照合した後ろの系列は捨てる)
    let lookahead = rayon::current_num_threads().max(1);
    let mut gi = 0;
    while gi < groups.len() {
        let mut gj = gi;
        let mut nfiles = 0usize;
        while gj < groups.len() && (gj == gi || nfiles < lookahead) {
            if !st.bad_series.contains(&groups[gj].0) {
                nfiles += groups[gj].1.len();
            }
            gj += 1;
        }
        let window: Vec<(usize, Vec<Member>)> = (gi..gj)
            .filter(|&g| !st.bad_series.contains(&groups[g].0))
            .map(|g| (g, groups[g].1.iter().copied().filter(|m| !store.is_skipped(m.file)).collect()))
            .collect();
        let flat: Vec<Member> = window.iter().flat_map(|(_, ms)| ms.iter().copied()).collect();
        let mut matched =
            match_members(field, store, &flat, &batches, s_lo, s_hi, hint_t, params, clock).into_iter();
        let mut pre: BTreeMap<usize, (Vec<Member>, Vec<MatchResult>)> = BTreeMap::new();
        for (g, ms) in window {
            let rs: Vec<MatchResult> = matched.by_ref().take(ms.len()).collect();
            pre.insert(g, (ms, rs));
        }
        for (g, (iid, ms)) in groups.iter().enumerate().take(gj).skip(gi) {
            if st.bad_series.contains(iid) {
                continue;
            }
            let mut first = pre.remove(&g);
            let sol = loop {
                // 中身の壊れた索引は、そのファイルを外して系列の照合と検証をやり直す(ほかの索引で解ける画像まで
                // 解けなくならないように)。外したはずのファイルがまた出たら、系列ごとこの探索から外す
                let alive: Vec<Member> = ms.iter().copied().filter(|m| !store.is_skipped(m.file)).collect();
                if alive.is_empty() {
                    break None;
                }
                let results = match first.take() {
                    Some((used, rs)) if used.iter().map(|m| m.file).eq(alive.iter().map(|m| m.file)) => rs,
                    _ => match_members(field, store, &alive, &batches, s_lo, s_hi, hint_t, params, clock),
                };
                match finish_series(field, store, *iid, &alive, results, params, clock, log, st, fl) {
                    Ok(s) => break s,
                    Err(Abort::Corrupt { file, msg }) => {
                        if store.is_skipped(file) {
                            st.bad_series.insert(*iid);
                            break None;
                        }
                        store.mark_broken(file, &msg);
                        log(&format!("星図データが壊れているので、この先は使いません: {msg}"));
                        st.broken.push(msg);
                    }
                    // OS のエラーで開けない(ほかのプロセスが書き込み用に開いているなど)。中身が壊れているとは
                    // 限らないので「壊れている」とは言わず、この探索の中でだけ外す(次の試行では開き直す)
                    Err(Abort::Unavailable { file, msg }) => {
                        if store.is_skipped(file) {
                            st.bad_series.insert(*iid);
                            break None;
                        }
                        store.mark_unavailable(file, &msg);
                        log(&format!("星図データを今は開けないので、この探索では使いません: {msg}"));
                    }
                    Err(e) => {
                        st.flushes[fl].t = round4(st.t0.elapsed().as_secs_f64());
                        return Err(e);
                    }
                }
            };
            if sol.is_some() {
                st.flushes[fl].t = round4(st.t0.elapsed().as_secs_f64());
                return Ok(sol);
            }
        }
        gi = gj;
    }
    st.flushes[fl].t = round4(st.t0.elapsed().as_secs_f64());
    Ok(None)
}

fn round4(v: f64) -> f64 {
    crate::pyfmt::py_round(v, 4)
}

/// 索引 1 本の照合の結果(仮説(newpoint の順)と当たりの数)。
type MatchResult = Result<(Vec<Hyp>, u64), Abort>;

/// 索引を照合する(索引ごとに並列。結果は members の順)。その quad の大きさ(3 星・4 星)の束が無い索引は当たりなし。
#[allow(clippy::too_many_arguments)]
fn match_members(
    field: &Field,
    store: &Store,
    members: &[Member],
    batches: &BTreeMap<usize, Batch>,
    s_lo: f64,
    s_hi: f64,
    hint_t: Option<(Vec3, f64)>,
    params: &Params,
    clock: Clock,
) -> Vec<MatchResult> {
    members
        .par_iter()
        .map(|m| match batches.get(&store.file(m.file).dimquads) {
            None => Ok((Vec::new(), 0)),
            Some(b) => {
                clock.check()?;
                match_file(field, store, m.file, m.fid, b, s_lo, s_hi, hint_t, params, clock)
            }
        })
        .collect()
}

/// 系列 1 つぶんの残り: 照合の結果を索引の順に数え、仮説をまとめて検証する(Python の `_sv_flush_series`)。
#[allow(clippy::too_many_arguments)]
fn finish_series(
    field: &Field,
    store: &Store,
    iid: u32,
    members: &[Member],
    results: Vec<MatchResult>,
    params: &Params,
    clock: Clock,
    log: &dyn Fn(&str),
    st: &mut SearchState,
    fl: usize,
) -> Result<Option<Found>, Abort> {
    let sc = SeriesCtx::new(store, iid, members[0].file);
    // 当たりの数は索引の順に足す。Python は 1 本ずつ照合するので、問題が起きた索引より前の分だけが数えられる
    let mut per_file: Vec<Vec<Hyp>> = Vec::with_capacity(results.len());
    for r in results {
        match r {
            Ok((hy, nhit)) => {
                st.hits += nhit;
                st.flushes[fl].hits += nhit;
                per_file.push(hy);
            }
            Err(e) => return Err(clock.prefer_cancel(e)),
        }
    }
    // 同じ系列のタイルの仮説をつなぎ、newpoint の順(quad の行の順。同じ行は索引の順)に並べる。
    // 仮説(約 270 バイト)を 1 本につないで並べ替えると、写しと並べ替えの作業領域が大きい(しかも並列の外)ので、
    // 参照(8 バイト)を並べる。参照の列は今までの連結と同じ順(索引の順、その中は索引の中の順)に作り、同じ鍵 row で
    // 安定に並べるので、並びは仮説そのものを並べたときと完全に同じ
    let mut all: Vec<&Hyp> = per_file.iter().flatten().collect();
    if all.is_empty() {
        return Ok(None);
    }
    all.sort_by_key(|h| h.row);
    verify_and_pick(field, &sc, params, &all, clock, log, st, fl)
}

// ------------------------------------------------------------------ 照合 ---

/// 当たり 1 つを安く捨てるか、4 点で当てはめた仮説にする。
struct Cand {
    row: usize,
    qid: u32,
    local: [usize; 4],
    stars: [u32; 4],
    fit: crate::parts::SimilarityFit,
    mirror: bool,
}

/// quad の束を索引 1 本に照合し、安く捨ててから 4 点で当てはめた仮説を返す(Python の `_sv_match_file`)。
/// 戻り値は (仮説(newpoint の順), 当たりの数)。
#[allow(clippy::too_many_arguments)]
fn match_file(
    field: &Field,
    store: &Store,
    file: usize,
    fid: usize,
    b: &Batch,
    s_lo: f64,
    s_hi: f64,
    hint_t: Option<(Vec3, f64)>,
    params: &Params,
    clock: Clock,
) -> Result<(Vec<Hyp>, u64), Abort> {
    let f = store.file(file);
    let tol = params.codetol;
    let (lo, hi) = quad_ab_range(f.scale_lo_rad, f.scale_hi_rad, s_lo, s_hi, field.w, field.h, tol, params.quad_min_frac);
    let var = &b.var;
    let vsel: Vec<usize> = (0..var.q.len())
        .filter(|&v| {
            let a = b.ab[var.row[v]];
            a >= lo && a <= hi
        })
        .collect();
    if vsel.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let data = store.data(file)?;
    let d = f.code_dim();
    if data.code_dim() != d {
        return Err(Abort::Corrupt { file, msg: format!("{}: コードの次元がヘッダーと合いません", f.name) });
    }
    // 三角形(2 次元のコード)は、同じ位置のずれでコードの L2 が √(D/4) 倍になるので許容誤差もそう縮める
    let radius = tol * (d as f64 / 4.0).sqrt();
    let mut hits: Vec<(u32, u32)> = Vec::new();
    let mut qbuf: Vec<f64> = Vec::with_capacity(BALL_CHUNK * d);
    for (ci, chunk) in vsel.chunks(BALL_CHUNK).enumerate() {
        if ci > 0 {
            clock.check()?;
        }
        qbuf.clear();
        for &v in chunk {
            qbuf.extend_from_slice(&var.q[v][..d]);
        }
        let got = data
            .code_ball(&qbuf, radius)
            .map_err(|e| Abort::Corrupt { file, msg: format!("{}: {e}", f.name) })?;
        let base = (ci * BALL_CHUNK) as u32;
        hits.extend(got.into_iter().map(|(q, c)| (q + base, c)));
    }
    if hits.is_empty() {
        return Ok((Vec::new(), 0));
    }
    clock.check()?;
    let nhit = hits.len() as u64;
    let dq = b.dq;
    let hint_reach = hint_t;
    let one = |&(qi, qid): &(u32, u32)| -> Result<Option<Cand>, Abort> {
        let v = vsel[qi as usize];
        let row = var.row[v];
        let perm = var.perm[v];
        let corrupt = || Abort::Corrupt {
            file,
            msg: format!(
                "{}: quad の星番号が星の数 {} の外です(中身が壊れています。落とし直してください)",
                f.name,
                crate::pyfmt::fmt_int(f.nstars as u64)
            ),
        };
        // 星番号は quad_stars が 4 つとも確かめる。安い棄却は A・B だけで決まるので、C・D は通ったものだけ読む
        let stars = data.quad_stars(qid).ok_or_else(corrupt)?;
        let mut s = [[0.0f64; 3]; 4];
        s[0] = data.star_xyz(stars[0]).ok_or_else(corrupt)?;
        s[1] = data.star_xyz(stars[1]).ok_or_else(corrupt)?;
        // 安い棄却 1: 索引の AB の角距離 ÷ 画像の |AB| が [s_lo(1 − tol), s_hi(1 + tol)] の外
        let th = angle_between(s[0], s[1]) * ARCSEC_PER_RAD;
        let s_impl = th / b.ab[row];
        if !(s_impl >= s_lo * (1.0 - tol) && s_impl <= s_hi * (1.0 + tol)) {
            return Ok(None);
        }
        // 安い棄却 2: 索引の quad の中心がヒントの円(半径 + 画像の対角の半分)の外
        if let Some((hc, hr)) = hint_reach {
            let qc = normalize3([s[0][0] + s[1][0], s[0][1] + s[1][1], s[0][2] + s[1][2]]);
            let reach = hr + field.half_diag * s_impl * 1.05 / ARCSEC_PER_RAD;
            if !(angle_between(qc, hc) <= reach) {
                return Ok(None);
            }
        }
        for j in 2..dq {
            s[j] = data.star_xyz(stars[j]).ok_or_else(corrupt)?;
        }
        let quad = b.quads[row];
        let mut local = [usize::MAX; 4];
        let mut fxy = [[0.0f64; 2]; 4];
        for j in 0..dq {
            local[j] = quad[perm[j]];
            fxy[j] = field.p[local[j]];
        }
        let fit = fit_similarity_quad(&fxy[..dq], &s[..dq]);
        // スケールの判定は安い棄却 1 と同じ許容で
        if !(fit.ok && fit.scale >= s_lo * (1.0 - tol) && fit.scale <= s_hi * (1.0 + tol)) {
            return Ok(None);
        }
        Ok(Some(Cand { row, qid, local, stars, fit, mirror: var.mirror[v] }))
    };
    let cands: Vec<Option<Cand>> = if hits.len() >= PAR_MIN_HITS {
        hits.par_chunks(4096)
            .map(|ch| -> Result<Vec<Option<Cand>>, Abort> {
                clock.check()?;
                ch.iter().map(one).collect()
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect()
    } else {
        hits.iter().map(one).collect::<Result<Vec<_>, _>>()?
    };
    // 同じ組(画像の quad の行・索引の quad・星の対応)は 1 つにまとめ、newpoint の順(quad の行の順)に並べる
    let mut seen: HashSet<(usize, u32, [usize; 4])> = HashSet::new();
    let mut keep: Vec<Cand> = Vec::new();
    for c in cands.into_iter().flatten() {
        if seen.insert((c.row, c.qid, c.local)) {
            keep.push(c);
        }
    }
    keep.sort_by_key(|c| c.row);
    let hyps = keep
        .into_iter()
        .map(|c| {
            let a = field.p[c.local[0]];
            let bb = field.p[c.local[1]];
            let mut qfield = [usize::MAX; 4];
            for (q, &l) in qfield.iter_mut().zip(&c.local[..dq]) {
                *q = field.order[l];
            }
            let abl = b.ab[c.row];
            Hyp {
                crval: c.fit.crval,
                east: c.fit.east,
                north: c.fit.north,
                cd: c.fit.cd,
                cdinv: inv2(&c.fit.cd),
                crpix: c.fit.crpix,
                scale: c.fit.scale,
                dq,
                qfield,
                qindex: c.stars,
                fid,
                qc: [(a[0] + bb[0]) / 2.0, (a[1] + bb[1]) / 2.0],
                q2: abl * abl / 4.0,
                row: c.row,
                mirror: c.mirror,
            }
        })
        .collect();
    Ok((hyps, nhit))
}

// ------------------------------------------------------------------ 検証して選ぶ ---

/// 仮説を newpoint の順に VERIFY_STEP 個ずつ検証し、solve 以上の最初の仮説(なければ tune-up で上がった最初のもの)を
/// 改良して返す(Python の `_sv_verify_and_pick`)。区切りの中は並列に検証し、選ぶのは順に。
/// 改良した後の対数オッズが accept_logodds(試した数で上げた値)に届かないもの、試した仮説が多くなってから tune-up で
/// 上がったのに改良で一致が増えないもの(tuneup_accepted)は解にしない(設計書 3.7 節の 6)。
#[allow(clippy::too_many_arguments)]
fn verify_and_pick(
    field: &Field,
    sc: &SeriesCtx,
    params: &Params,
    hyps: &[&Hyp],
    clock: Clock,
    log: &dyn Fn(&str),
    st: &mut SearchState,
    fl: usize,
) -> Result<Option<Found>, Abort> {
    let solve = params.logodds_solve;
    let tune = params.logodds_tune;
    for part in hyps.chunks(VERIFY_STEP) {
        let m = part.len();
        // 区切りごとに全部の検証の終わりを待つので、終わりぎわに残る仕事を小さくする: 仕事は 16〜31 仮説にそろえる
        // (with_max_len が無いと、rayon は分ける回数をスレッドの数から半分ずつ減らすので、盗まれない部分の仕事は
        // 20 スレッドで 8192 / 32 = 256 仮説、8 スレッドで 512 仮説になる)。collect は並びを保つので、
        // 結果の順も選ぶ仮説も分け方によらない。Err は今までと同じく、並列で先に出たものを返す(種類は締め切り・中止・
        // 壊れた索引・開けない索引で、同時に起きたときにどれになるかが決まっていないのは分け方によらず前から同じ)
        let res: Vec<VerifyOut> = part
            .par_iter()
            .with_min_len(16)
            .with_max_len(16)
            .map(|h| -> Result<VerifyOut, Abort> {
                clock.check()?;
                if !quick_pass(field, sc, params, h, 1.0)? {
                    return Ok(VerifyOut::rejected(true));
                }
                verify_one(field, sc, params, h, Proj::Tan, None, 1.0, false)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| clock.prefer_cancel(e))?;
        st.hyps += m as u64;
        st.flushes[fl].hyps += m as u64;
        st.quick_rejected += res.iter().filter(|r| r.quick_rejected).count() as u64;
        let mx = res.iter().map(|r| r.best).fold(f64::NEG_INFINITY, f64::max);
        if m > 0 && mx > st.logodds_max {
            st.logodds_max = mx; // 改良する前の、検証した仮説の対数オッズの最大
        }
        if st.trace {
            let room = TRACE_MAX_HYP.saturating_sub(st.hyp_log.len());
            for (h, r) in part.iter().zip(&res).take(room) {
                let (ra, dec) = xyz_to_radec(h.crval);
                st.hyp_log.push(HypLog {
                    index: sc.store.file(sc.files[h.fid]).name.clone(),
                    row: h.row,
                    mirror: h.mirror,
                    quad_field: h.qfield[..h.dq].to_vec(),
                    quad_index: h.qindex[..h.dq].to_vec(),
                    crval: (ra, dec),
                    crpix: h.crpix,
                    cd: h.cd,
                    scale: h.scale,
                    quick_rejected: r.quick_rejected,
                    logodds: r.best.is_finite().then_some(r.best),
                    nmatch: r.nmatch,
                });
            }
        }
        // 解とみなす値は、この区切りの終わりまでに試した仮説の数で上げる(区切りの中は同じ値。Python も同じ数え方)
        let accept = accept_logodds(params, st.hyps);
        for (i, r) in res.iter().enumerate() {
            if !(r.best >= tune) {
                continue;
            }
            let h = part[i];
            let name = sc.store.file(sc.files[h.fid]).name.clone();
            let tuned = r.best < solve;
            if tuned {
                // tune-up: 1 次で改良して検証し直す
                let (_, r1) = refine(field, sc, params, h, 1, false, clock)?;
                st.tuneups.push((name.clone(), r.best, r1.best));
                if !(r1.best >= solve) {
                    continue;
                }
            }
            let (wcs, res1) = match refine(field, sc, params, h, params.tweak_order, true, clock) {
                Err(Abort::TimeUp) => {
                    // solve を超えた仮説がある(解は見つかっている)ので、締め切りを過ぎても改良は最後まで済ませる
                    // (中止の合図はそのまま見る)。ここで止めると、解けている画像が「解けず」になる
                    log("締め切りを過ぎましたが、解が見つかっているので改良を済ませます");
                    refine(field, sc, params, h, params.tweak_order, true, clock.without_deadline())?
                }
                other => other?,
            };
            if !(res1.best >= accept) {
                continue;
            }
            // 試した仮説が多くなってから tune-up で上がった仮説は、改良で一致が増えたときだけ解にする
            // (同じ星への当て直しだけで上がったものは偶然)
            if tuned && !tuneup_accepted(params, st.hyps, r.nmatch, res1.nmatch) {
                continue;
            }
            return Ok(Some(Found {
                wcs,
                logodds: res1.best,
                nmatch: res1.nmatch,
                ndistract: res1.ndistract,
                nfield: res1.nt,
                nref: res1.nr,
                index_name: name,
                quad_field: h.qfield[..h.dq].to_vec(),
                quad_index: h.qindex[..h.dq].to_vec(),
                quads_tried: st.quads,
                hypotheses: st.hyps,
                seconds: 0.0,
                matches: res1.matches.iter().map(|m| (m.0, m.1, m.2)).collect(),
            }));
        }
        clock.check()?;
    }
    Ok(None)
}

// ------------------------------------------------------------------ 改良 ---

/// 点が画像を g × g に分けた箱のいくつに入っているか(Python の `_sv_spread_boxes`)。
fn spread_boxes(xy: &[[f64; 2]], w: f64, h: f64) -> usize {
    let g = SIP_GRID;
    let cell = |v: f64, size: f64| {
        let f = ((v + 0.5) / size * g as f64).floor();
        if !(f >= 0.0) {
            0
        } else if f >= (g - 1) as f64 {
            g - 1
        } else {
            f as usize
        }
    };
    let set: BTreeSet<usize> = xy.iter().map(|p| cell(p[1], h) * g + cell(p[0], w)).collect();
    set.len()
}

/// 仮説 1 個を改良する(Python の `_sv_refine`。設計書 3.8 節)。戻り値 (WCS, 最後の検証)。
///
/// 一致した組を今の WCS で投影し直した参照星との最近傍で取り直し、重みつき最小二乗で TAN を当て直す。これを
/// TWEAK_STEPS 回、σ の広がりを γ = 0.7^k で弱めながら繰り返す(CRPIX は画像の中心、接点の初めは今の WCS で
/// 画像の中心が写る向き)。final なら最後の当て直しは quad の星を除いた一致(6 個以上あれば)で行う。
/// order ≥ 2 で SIP を付けるのは、組が 3 × 未知数以上・画像の 3 × 3 の箱の 7 つ以上・F 検定(上側 1 %)で有意なときだけ。
/// 最後に γ = 1 で検証し直す。
pub(crate) fn refine(
    field: &Field,
    sc: &SeriesCtx,
    params: &Params,
    h: &Hyp,
    order: usize,
    final_: bool,
    clock: Clock,
) -> Result<(Wcs, VerifyOut), Abort> {
    let mut wcs = Wcs::tan(h.crval, h.crpix, h.cd);
    let qfile = sc.files[h.fid];
    let data = sc.store.data(qfile)?;
    let mut quad_xyz = [[0.0f64; 3]; 4];
    for (v, &id) in quad_xyz.iter_mut().zip(&h.qindex[..h.dq]) {
        *v = data.star_xyz(id).ok_or_else(|| Abort::Corrupt {
            file: qfile,
            msg: format!("{}: quad の星番号が星の数の外です", sc.store.file(qfile).name),
        })?;
    }
    let (cx, cy) = (field.center[0], field.center[1]);
    // 参照星は 1 回だけ集める(少し広めに。改良で中心とスケールが少し動いても足りるように)
    let cen = wcs.pix_to_xyz(cx, cy);
    let rad = field.half_diag * h.scale * 1.25 / ARCSEC_PER_RAD;
    let refs: Refs = series_cone(sc, cen, rad)?;
    let mut pairs: Option<FitPairs> = None;
    let mut nmatch = 0usize;
    for k in 0..TWEAK_STEPS {
        clock.check()?;
        let gamma = TWEAK_GAMMA.powf(k as f64);
        let hw = h.with_wcs(&wcs);
        let res = verify_one(field, sc, params, &hw, Proj::Wcs(&wcs), Some(&refs), gamma, true)?;
        let (fxy, sxyz, w) = fit_pairs(field, &res, h, &quad_xyz);
        nmatch = res.matches.len();
        let usable = w.iter().filter(|&&x| x > 0.0).count();
        let crval0 = wcs.pix_to_xyz(cx, cy);
        let next = if usable < 3 {
            None
        } else {
            fit_wcs_lsq(&fxy, &sxyz, Some(&w), Some(field.center), 0, Some(crval0), LSQ_MAX_SHIFTS, None).ok()
        };
        pairs = Some((fxy, sxyz, w));
        match next {
            Some(nw) => wcs = nw,
            None => break,
        }
    }
    if let Some((fxy, sxyz, w)) = pairs.as_mut()
        && final_
        && w[..nmatch.min(w.len())].iter().filter(|&&x| x > 0.0).count() >= TWEAK_MIN_MATCHES
    {
        // quad の星(後ろに付いている)を外す
        fxy.truncate(nmatch);
        sxyz.truncate(nmatch);
        w.truncate(nmatch);
    }
    let nm = pairs.as_ref().map_or(0, |(_, _, w)| w.iter().filter(|&&x| x > 0.0).count());
    let cv = wcs.pix_to_xyz(cx, cy);
    if let Some((fxy, sxyz, w)) = pairs.as_ref() {
        if order >= 2 {
            let terms = (order + 1) * (order + 2) / 2;
            let used: Vec<[f64; 2]> = fxy.iter().zip(w).filter(|(_, x)| **x > 0.0).map(|(p, _)| *p).collect();
            if nm >= 3 * 2 * terms && spread_boxes(&used, field.w, field.h) >= SIP_MIN_BOXES {
                let tan_c = fit_wcs_lsq(fxy, sxyz, Some(w), Some(field.center), 0, Some(cv), LSQ_MAX_SHIFTS, None);
                let sip_c = fit_wcs_lsq(
                    fxy,
                    sxyz,
                    Some(w),
                    Some(field.center),
                    order,
                    Some(cv),
                    LSQ_MAX_SHIFTS,
                    Some((field.w, field.h)),
                );
                if let (Ok(tan_c), Ok(sip_c)) = (tan_c, sip_c) {
                    // 残差の減り方の F 検定(上側 1 %)
                    let rss_t = wrms(&tan_c, fxy, sxyz, w).powi(2);
                    let rss_s = wrms(&sip_c, fxy, sxyz, w).powi(2);
                    let d1 = (2 * terms - 2 * 3) as f64;
                    let d2 = 2.0 * nm as f64 - 2.0 * terms as f64;
                    let take = if rss_s > 0.0 {
                        (rss_t - rss_s) / d1 > f_upper(d1, d2, SIP_F_Z) * rss_s / d2
                    } else {
                        rss_t > 0.0
                    };
                    if take {
                        wcs = sip_c;
                    }
                }
            }
        }
        if !wcs.has_sip()
            && nm >= 3
            && let Ok(w2) = fit_wcs_lsq(fxy, sxyz, Some(w), Some(field.center), 0, Some(cv), LSQ_MAX_SHIFTS, None)
        {
            wcs = w2;
        }
    }
    clock.check()?;
    let hw = h.with_wcs(&wcs);
    let res = verify_one(field, sc, params, &hw, Proj::Wcs(&wcs), Some(&refs), 1.0, true)?;
    Ok((wcs, res))
}

/// 改良の当てはめに使う組(画像の点・星・重み)。
type FitPairs = (Vec<[f64; 2]>, Vec<Vec3>, Vec<f64>);

/// 改良の当てはめに使う組: 一致した星(重み = 1 つの一致の対数オッズから)と quad の星(重み 1)。
fn fit_pairs(field: &Field, res: &VerifyOut, h: &Hyp, quad_xyz: &[Vec3; 4]) -> FitPairs {
    let mut fxy = Vec::with_capacity(res.matches.len() + 4);
    let mut sxyz = Vec::with_capacity(res.matches.len() + 4);
    let mut w = Vec::with_capacity(res.matches.len() + 4);
    for m in &res.matches {
        fxy.push(field.xy[m.0]);
        sxyz.push(m.1);
        w.push(logodds_weight(m.3));
    }
    for (&qf, &v) in h.qfield[..h.dq].iter().zip(quad_xyz) {
        fxy.push(field.xy[qf]);
        sxyz.push(v);
        w.push(1.0);
    }
    (fxy, sxyz, w)
}

// ------------------------------------------------------------------ ヒントのタイル ---

/// ヒントがあるとき、タイルの索引を「その段の quad が照合で残りうるタイル」に絞る(Python の `_sv_hint_tiles`)。
///
/// 照合で残る quad の画素スケールは s_impl ≤ min(s_hi·(1 + codetol), SCALE_U ÷ (quad_min_frac × 短辺))で、
/// 残る quad の星 A はヒントから R = ヒントの半径 + 対角の半分 × s_max × 1.05 + SCALE_U/2 以内にある。タイルの中に
/// R 以内の点が無ければ(ヒントがタイルの外で、境界の標本点がどれも R + 標本の間隔の半分より遠ければ)外す。
fn hint_tiles(store: &Store, files: &[usize], field: &Field, s_hi: f64, hint_t: (Vec3, f64), p: &Params) -> Vec<usize> {
    let (hc, hr) = hint_t;
    let short = field.w.min(field.h);
    let mut home: HashMap<i64, i64> = HashMap::new();
    let mut out = Vec::new();
    for &i in files {
        let f = store.file(i);
        let (Some(tc), Some(tr)) = (f.tile_center, f.tile_radius_rad) else {
            out.push(i);
            continue;
        };
        let u = f.scale_hi_rad * 1.001;
        let mut s_max = s_hi * (1.0 + p.codetol);
        if p.quad_min_frac > 0.0 {
            s_max = s_max.min(u * ARCSEC_PER_RAD / (p.quad_min_frac * short));
        }
        let r = hr + field.half_diag * s_max * 1.05 / ARCSEC_PER_RAD + u / 2.0;
        if angle_between(tc, hc) > tr + r + 1e-9 {
            continue;
        }
        let Ok(nside) = u32::try_from(f.hpnside) else {
            out.push(i);
            continue;
        };
        let hp = *home.entry(f.hpnside).or_insert_with(|| healpix_xy_from_xyz(hc, nside) as i64);
        if hp != f.healpix as i64 {
            let (pts, half_gap) = tile_edge(f, nside);
            let dmin = pts.iter().map(|q| angle_between(*q, hc)).fold(f64::INFINITY, f64::min);
            if dmin > r + half_gap + 1e-6 {
                continue;
            }
        }
        out.push(i);
    }
    out
}

/// タイルの境界を辺ごとに 64 点で区切った点と、境界上のどの点からも最寄りの点までの角距離の上限。
fn tile_edge(f: &IndexFile, nside: u32) -> (Vec<Vec3>, f64) {
    let hp = f.healpix.max(0) as u64;
    let n = 64usize;
    let mut pts = Vec::with_capacity(4 * n);
    let t: Vec<f64> = (0..n).map(|k| k as f64 / n as f64).collect();
    for &v in &t {
        pts.push(healpix_xy_point(hp, nside, v, 0.0));
    }
    for &v in &t {
        pts.push(healpix_xy_point(hp, nside, 1.0, v));
    }
    for &v in &t {
        pts.push(healpix_xy_point(hp, nside, 1.0 - v, 1.0));
    }
    for &v in &t {
        pts.push(healpix_xy_point(hp, nside, 0.0, 1.0 - v));
    }
    let m = pts.len();
    let gap = (0..m).map(|k| angle_between(pts[k], pts[(k + 1) % m])).fold(0.0f64, f64::max);
    (pts, gap / 2.0)
}

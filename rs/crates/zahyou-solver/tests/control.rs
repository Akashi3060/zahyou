//! 締め切り・中止・壊れた入力・壊れた索引・並列(設計書 3.1・7 節、Python の test_solver_core case_08・09・17・18)。
//!
//! - 中止: cancel が立ってから 0.2 秒以内に Err(Cancelled)(探索の途中のいろいろな時点で立てる。Python の case_09 と同じ基準)。
//!   始める前に立っていれば即座に。solve_offline も 0.2 秒以内に止まり、解を返さない(索引の mmap を外す仕事が
//!   0.1〜0.4 秒かかるので、それを待たずに返すこと。遅れたときは中止のあとのログを時刻つきで出す)
//! - 過ぎた締め切り: 0.5 秒以内に Ok(None)
//! - 壊れた入力(NaN・無限大の座標や明るさ、星 0〜2 個、幅 0、おかしいヒント・スケール、星 1 万個): panic しない、
//!   解を返さない(解けない入力)、締め切りを 1 秒以上超えない
//! - 中身の壊れた索引(大きさとヘッダーは正しい。index-4116 の星の木の葉の範囲を入れ替えたもの・quad の表の星番号を
//!   化けさせたもの): panic せず、ほかの索引で解き(正解の基準を満たす)、壊れていることをログで知らせる
//! - 同じ目録を 4 本のスレッドから同時に使っても、1 本で解いたときと同じ答え
//!
//! ```text
//! cargo test -p zahyou-solver --test control -- --nocapture
//! ```
//! 星図データ(index-5205-* と index-4113〜4117)が無ければ SKIP。

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::*;
use zahyou_image::{Image2D, Prep, Source};
use zahyou_solver::{Control, FieldStar, OfflineRequest, Params, SolveError, SolveRequest};

fn random_field(n: usize, w: usize, h: usize, seed: u64) -> Vec<FieldStar> {
    let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut next = move || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        (s.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut v: Vec<FieldStar> =
        (0..n).map(|_| FieldStar { x: next() * (w as f64 - 1.0), y: next() * (h as f64 - 1.0), flux: 1.0 + 99.0 * next() }).collect();
    v.sort_by(|a, b| b.flux.total_cmp(&a.flux));
    v
}

/// 探索を始めて at 秒後に中止を立て、止まるまでの遅れ(秒)を測る。戻り値 (結果, 遅れ)。
fn cancel_after(req: &SolveRequest, at: f64) -> (Result<Option<zahyou_solver::Solution>, SolveError>, f64) {
    let cat = catalog().unwrap();
    let cancel = AtomicBool::new(false);
    let set_at: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let log = |_: &str| {};
    std::thread::scope(|sc| {
        let t0 = Instant::now();
        let h = sc.spawn(|| {
            std::thread::sleep(Duration::from_secs_f64(at));
            *set_at.lock().unwrap() = Some(Instant::now());
            cancel.store(true, Ordering::SeqCst);
        });
        let ctl = Control { deadline: Some(t0 + Duration::from_secs(60)), cancel: &cancel, log: &log };
        let r = zahyou_solver::solve(req, &cat, &Params::default(), &ctl);
        let end = Instant::now();
        // 中止より先に終わった(解けた・quad を試し尽くした)ときは遅れ 0 とし、結果で見分ける
        let lag = set_at.lock().unwrap().map(|t| end.saturating_duration_since(t).as_secs_f64()).unwrap_or(0.0);
        h.join().unwrap();
        (r, lag)
    })
}

#[test]
fn cancel_stops_within_0_2_s() {
    if catalog().is_none() {
        skip("cancel_stops_within_0_2_s", "星図データが無い");
        return;
    }
    let _g = heavy();
    let mut bad = Vec::new();
    // 乱数の星 150 個のブラインド(解は無い。探索は締め切りまで続く)
    let req = SolveRequest { stars: random_field(150, 968, 548, 9), width: 968, height: 548, ..Default::default() };
    for at in [0.05, 0.3, 1.0, 2.5] {
        let (r, lag) = cancel_after(&req, at);
        let ok = matches!(r, Err(SolveError::Cancelled)) && lag <= 0.2;
        row(if ok { "PASS" } else { "FAIL" }, &format!("中止 {at} 秒後"), &format!("{r:?}、立ててから {lag:.3} 秒で止まった"));
        if !ok {
            bad.push(format!("{at} 秒後の中止: {:?}、遅れ {lag:.3} 秒", r.map(|o| o.map(|s| s.index_name))));
        }
    }
    // ヒントあり・スケール既知(照合の束が大きくなる形。Python の case_19)
    let req = SolveRequest {
        stars: random_field(150, 1200, 800, 19),
        width: 1200,
        height: 800,
        scale_lo: Some(1.5),
        scale_hi: Some(2.5),
        hint: Some((200.0, -10.0, 5.0)),
    };
    let (r, lag) = cancel_after(&req, 0.7);
    let ok = matches!(r, Err(SolveError::Cancelled) | Ok(None)) && lag <= 0.2;
    row(if ok { "PASS" } else { "FAIL" }, "中止(ヒント・スケール既知)", &format!("{r:?}、遅れ {lag:.3} 秒"));
    if !ok {
        bad.push(format!("ヒントありの中止: 遅れ {lag:.3} 秒"));
    }
    // 始める前に立っている
    let cat = catalog().unwrap();
    let cancel = AtomicBool::new(true);
    let log = |_: &str| {};
    let ctl = Control { deadline: None, cancel: &cancel, log: &log };
    let t0 = Instant::now();
    let r = zahyou_solver::solve(&req, &cat, &Params::default(), &ctl);
    let dt = t0.elapsed().as_secs_f64();
    if !matches!(r, Err(SolveError::Cancelled)) || dt > 0.2 {
        bad.push(format!("始める前の中止: {:?}、{dt:.3} 秒", r.map(|o| o.is_some())));
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn past_deadline_returns_at_once() {
    let Some(cat) = catalog() else {
        skip("past_deadline_returns_at_once", "星図データが無い");
        return;
    };
    let _g = heavy();
    let req = SolveRequest { stars: random_field(150, 968, 548, 8), width: 968, height: 548, ..Default::default() };
    let cancel = AtomicBool::new(false);
    let log = |_: &str| {};
    let ctl = Control { deadline: Some(Instant::now() - Duration::from_secs(1)), cancel: &cancel, log: &log };
    let t0 = Instant::now();
    let r = zahyou_solver::solve(&req, &cat, &Params::default(), &ctl);
    let dt = t0.elapsed().as_secs_f64();
    assert!(matches!(r, Ok(None)) && dt < 0.5, "過ぎた締め切り: {:?}、{dt:.3} 秒", r.map(|o| o.is_some()));
}

/// 前処理の結果の代わり(何も写っていない画像。深い検出は星を見つけない)。
fn blank_prep(w: usize, h: usize) -> Prep {
    Prep { sub: Image2D::new(w, h, vec![0.0; w * h]).unwrap(), mask: vec![false; w * h], sigma: 1.0, info: Vec::new() }
}

fn sources_of(st: &[FieldStar]) -> Vec<Source> {
    st.iter()
        .map(|s| Source { x: s.x, y: s.y, flux: s.flux, peak: s.flux, area: 9, elong: 1.0, fwhm: 3.0, snr: 50.0 })
        .collect()
}

#[test]
fn solve_offline_cancel_and_timeout() {
    let Some(cat) = catalog() else {
        skip("solve_offline_cancel_and_timeout", "星図データが無い");
        return;
    };
    let _g = heavy();
    let (w, h) = (968usize, 548usize);
    let prep = blank_prep(w, h);
    let src = sources_of(&random_field(120, w, h, 31));
    let mut bad = Vec::new();
    // 中止(1 秒後)
    let req = OfflineRequest { prep: &prep, sources: &src, width: w, height: h, scale: None, hint: None, timeout_s: 60.0 };
    let cancel = AtomicBool::new(false);
    let set_at = std::sync::Mutex::new(None::<Instant>);
    // ログは始めてからの時刻つきで残す(遅れたとき、どの段で止まらなかったかを出す)
    let t_start = Instant::now();
    let lines = std::sync::Mutex::new(Vec::<(f64, String)>::new());
    let log = |s: &str| {
        if let Ok(mut v) = lines.lock() {
            v.push((t_start.elapsed().as_secs_f64(), s.trim().to_string()));
        }
    };
    let (r, lag, t_set) = std::thread::scope(|sc| {
        let th = sc.spawn(|| {
            std::thread::sleep(Duration::from_secs(1));
            *set_at.lock().unwrap() = Some(Instant::now());
            cancel.store(true, Ordering::SeqCst);
        });
        let ctl = Control { deadline: None, cancel: &cancel, log: &log };
        let r = zahyou_solver::solve_offline(&req, &cat, &Params::default(), &ctl);
        let end = Instant::now();
        th.join().unwrap();
        let t = *set_at.lock().unwrap();
        let lag = t.map(|t| end.saturating_duration_since(t).as_secs_f64()).unwrap_or(-1.0);
        (r, lag, t.map(|t| t.saturating_duration_since(t_start).as_secs_f64()))
    });
    let ok = matches!(r, Err(SolveError::Cancelled) | Ok(None)) && (0.0..=0.2).contains(&lag);
    let mut msg = format!("{:?}、遅れ {lag:.3} 秒", r.as_ref().map(|o| o.is_some()));
    if lag > 0.2 {
        // 中止を立てたあとのログ(と直前の 1 行)
        let v = lines.lock().map(|v| v.clone()).unwrap_or_default();
        let ts = t_set.unwrap_or(0.0);
        let k = v.iter().position(|(t, _)| *t >= ts).unwrap_or(v.len()).saturating_sub(1);
        let tail: Vec<String> = v[k..].iter().map(|(t, l)| format!("[{t:.3} 秒] {l}")).collect();
        msg += &format!("(中止を立てたのは {ts:.3} 秒。そのあとのログ: {})", tail.join(" / "));
    }
    row(if ok { "PASS" } else { "FAIL" }, "solve_offline の中止", &msg);
    if !ok {
        bad.push(format!("solve_offline の中止: {:?}、遅れ {lag:.3} 秒", r.map(|o| o.is_some())));
    }
    // 全体の締め切り(5 秒、ヒントとスケールありで試行 4 段)
    let req = OfflineRequest {
        prep: &prep,
        sources: &src,
        width: w,
        height: h,
        scale: Some(1.4616),
        hint: Some((61.9, 41.98, 5.0)),
        timeout_s: 5.0,
    };
    let cancel = AtomicBool::new(false);
    let ctl = Control { deadline: None, cancel: &cancel, log: &log };
    let t0 = Instant::now();
    let r = zahyou_solver::solve_offline(&req, &cat, &Params::default(), &ctl);
    let over = t0.elapsed().as_secs_f64() - 5.0;
    let ok = matches!(r, Ok(None)) && over <= 1.0;
    row(if ok { "PASS" } else { "FAIL" }, "solve_offline の締め切り", &format!("超過 {over:+.3} 秒"));
    if !ok {
        bad.push(format!("solve_offline の締め切り: {:?}、超過 {over:.3} 秒", r.map(|o| o.is_some())));
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn broken_inputs_do_not_panic() {
    let Some(cat) = catalog() else {
        skip("broken_inputs_do_not_panic", "星図データが無い");
        return;
    };
    let _g = heavy();
    let good = random_field(40, 968, 548, 5);
    let nan = f64::NAN;
    let inf = f64::INFINITY;
    let mut cases: Vec<(&str, SolveRequest)> = Vec::new();
    let base = SolveRequest { stars: good.clone(), width: 968, height: 548, ..Default::default() };
    cases.push(("星 0 個", SolveRequest { stars: vec![], ..base.clone() }));
    cases.push(("星 2 個", SolveRequest { stars: good[..2].to_vec(), ..base.clone() }));
    cases.push(("星 3 個", SolveRequest { stars: good[..3].to_vec(), ..base.clone() }));
    let mut s = good.clone();
    s[0].x = nan;
    s[3].y = inf;
    s[5].x = -1e300;
    s[7].y = 1e300;
    cases.push(("座標に NaN・無限大・巨大な値", SolveRequest { stars: s, ..base.clone() }));
    let mut s = good.clone();
    s.iter_mut().step_by(3).for_each(|p| p.flux = nan);
    s[1].flux = inf;
    s[2].flux = -inf;
    cases.push(("明るさに NaN・±無限大", SolveRequest { stars: s, ..base.clone() }));
    cases.push(("全部同じ位置", SolveRequest { stars: vec![FieldStar { x: 10.0, y: 10.0, flux: 1.0 }; 30], ..base.clone() }));
    cases.push(("幅 0", SolveRequest { width: 0, ..base.clone() }));
    cases.push(("高さ 0", SolveRequest { height: 0, ..base.clone() }));
    cases.push(("幅 1・高さ 1", SolveRequest { width: 1, height: 1, ..base.clone() }));
    cases.push(("幅が巨大", SolveRequest { width: usize::MAX / 4, height: 548, ..base.clone() }));
    cases.push(("ヒントの半径が負", SolveRequest { hint: Some((75.0, 35.0, -1.0)), ..base.clone() }));
    cases.push(("ヒントの半径が NaN", SolveRequest { hint: Some((75.0, 35.0, nan)), ..base.clone() }));
    cases.push(("ヒントの赤経が NaN", SolveRequest { hint: Some((nan, 35.0, 2.0)), ..base.clone() }));
    cases.push(("ヒントの赤緯が 145°", SolveRequest { hint: Some((255.0, 145.0, 2.0)), ..base.clone() }));
    cases.push(("ヒントの半径が 1000°", SolveRequest { hint: Some((255.0, 45.0, 1000.0)), ..base.clone() }));
    cases.push(("スケールが 0", SolveRequest { scale_lo: Some(0.0), scale_hi: Some(0.0), ..base.clone() }));
    cases.push(("スケールが負", SolveRequest { scale_lo: Some(-2.0), scale_hi: Some(-1.0), ..base.clone() }));
    cases.push(("スケールが NaN", SolveRequest { scale_lo: Some(nan), scale_hi: Some(2.0), ..base.clone() }));
    cases.push(("スケールが無限大", SolveRequest { scale_lo: Some(1.0), scale_hi: Some(inf), ..base.clone() }));
    cases.push(("スケールの上下が逆", SolveRequest { scale_lo: Some(2.0), scale_hi: Some(1.0), ..base.clone() }));
    cases.push(("星 1 万個", SolveRequest { stars: random_field(10_000, 6000, 4000, 77), width: 6000, height: 4000, ..base.clone() }));
    let deadline = 2.0;
    let mut bad = Vec::new();
    for (label, req) in &cases {
        let buf = LogBuf::default();
        let t0 = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| solve_once(req, &cat, &Params::default(), deadline, &buf)));
        let dt = t0.elapsed().as_secs_f64();
        let m = match r {
            Err(_) => Err("panic した".to_string()),
            Ok((Ok(Some(s)), _)) => Err(format!("解けない入力なのに解を返した({})", s.index_name)),
            Ok((_, _)) if dt > deadline + 1.0 => Err(format!("締め切りを {:.2} 秒超えた", dt - deadline)),
            Ok((r, _)) => Ok(format!("{:?}、{dt:.3} 秒{}", r.map(|_| ()), if buf.lines().is_empty() { String::new() } else { format!("、ログ: {}", buf.tail(1)) })),
        };
        match m {
            Ok(m) => row("PASS", label, &m),
            Err(m) => {
                row("FAIL", label, &m);
                bad.push(format!("{label}: {m}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// TTYPE1 = name の表のデータの位置と大きさ(バイト)。
fn table(p: &Path, name: &str) -> (usize, usize) {
    let mut f = std::fs::File::open(p).unwrap();
    let size = f.metadata().unwrap().len();
    let (hdus, _, _) = zahyou_index::fits::read_hdus(&mut f, size).unwrap();
    let h = hdus.iter().find(|h| h.header.str("TTYPE1") == Some(name)).expect("表が無い");
    (h.offset as usize, h.size as usize)
}

/// 大きさとヘッダーはそのままで中身を壊す(Python の test_solver_index._corrupt と同じ考え方)。
/// lr: 星の木の葉の表の前半と後半を入れ替える。quad: quad の表の星番号の上位バイトを count か所化けさせる。
fn corrupt(p: &Path, mode: &str, count: usize) {
    let mut b = std::fs::read(p).unwrap();
    if mode == "lr" {
        let (o, n) = table(p, "kdtree_lr_stars");
        let words: Vec<[u8; 4]> = b[o..o + n].chunks(4).map(|c| c.try_into().unwrap()).collect();
        let m = words.len() / 2;
        let swapped: Vec<u8> = words[m..].iter().chain(words[..m].iter()).flatten().copied().collect();
        b[o..o + n].copy_from_slice(&swapped);
    } else {
        let (o, n) = table(p, "quads");
        let mut s: u64 = 7;
        for _ in 0..count {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let k = ((s >> 33) as usize) % (n / 4);
            b[o + 4 * k + 3] ^= 0x10;
        }
    }
    std::fs::write(p, b).unwrap();
}

#[test]
fn corrupt_index_is_skipped_and_others_solve() {
    let label = "corrupt_index_is_skipped_and_others_solve";
    let need = ["4113", "4114", "4115", "4116", "4117"];
    if need.iter().any(|t| index_path(t).is_none()) {
        skip(label, "index-4113〜4117 が無い");
        return;
    }
    let Some(doc) = load_testdata("solver_synth.json") else {
        skip(label, "solver_synth.json が無い");
        return;
    };
    let c = &doc["corrupt"];
    if c.is_null() {
        skip(label, "solver_synth.json に corrupt の星野が無い");
        return;
    }
    let _g = heavy();
    let w = c["width"].as_u64().unwrap() as usize;
    let h = c["height"].as_u64().unwrap() as usize;
    let req = SolveRequest {
        stars: stars_xy_flux(&c["xy"], &c["flux"]),
        width: w,
        height: h,
        scale_lo: opt_f64(&c["scale_lo"]),
        scale_hi: opt_f64(&c["scale_hi"]),
        hint: None,
    };
    let want = truth_measure(&c["truth_summary"]);
    let mut bad = Vec::new();
    for mode in ["none", "lr", "quad"] {
        let dir = temp_dir(&format!("corrupt-{mode}"));
        for t in need {
            std::fs::copy(index_path(t).unwrap(), dir.join(format!("index-{t}.fits"))).unwrap();
        }
        if mode != "none" {
            corrupt(&dir.join("index-4116.fits"), mode, 10_000);
        }
        let cat = zahyou_index::Catalog::scan(&dir);
        let buf = LogBuf::default();
        let r = catch_unwind(AssertUnwindSafe(|| solve_once(&req, &cat, &Params::default(), solve_deadline_s(), &buf)));
        cat.close();
        let m = match r {
            Err(_) => Err("panic した".to_string()),
            Ok((Ok(Some(s)), dt)) => match compare_wcs(&s.wcs, w, h, &want, TOL_DEFAULT) {
                Some(cmp) if cmp.ok => {
                    let told = buf.lines().iter().any(|l| l.contains("壊れ"));
                    if mode != "none" && !told {
                        Err(format!("{} で解けたが、壊れていることをログで知らせていない({})", s.index_name, buf.tail(2)))
                    } else {
                        Ok(format!("{} で解けた({dt:.2} 秒、{})", s.index_name, describe(&cmp)))
                    }
                }
                Some(cmp) => Err(format!("{} の解が外れ {:?}", s.index_name, cmp.failed)),
                None => Err("解の WCS を測れない".into()),
            },
            Ok((Ok(None), dt)) => Err(format!("解けなかった({dt:.2} 秒、ログ: {})", buf.tail(3))),
            Ok((Err(e), _)) => Err(format!("エラー {e:?}")),
        };
        let name = format!("壊れた索引 {mode}");
        match m {
            Ok(m) => row("PASS", &name, &m),
            Err(m) => {
                row("FAIL", &name, &m);
                bad.push(format!("{name}: {m}"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn same_catalog_from_four_threads_gives_same_answer() {
    let label = "same_catalog_from_four_threads_gives_same_answer";
    let (Some(cat), Some(doc)) = (catalog(), load_testdata("solver_synth.json")) else {
        skip(label, "星図データか solver_synth.json が無い");
        return;
    };
    let Some(c) = doc["cases"].as_array().unwrap().iter().find(|c| c["name"] == "bs" && c.get("skipped").is_none()) else {
        skip(label, "場合 bs が無い");
        return;
    };
    let _g = heavy();
    let req = SolveRequest {
        stars: stars_xy_flux(&c["xy"], &c["flux"]),
        width: c["width"].as_u64().unwrap() as usize,
        height: c["height"].as_u64().unwrap() as usize,
        scale_lo: opt_f64(&c["scale_lo"]),
        scale_hi: opt_f64(&c["scale_hi"]),
        hint: hint_of(&c["hint"]),
    };
    let one = |_: usize| {
        let buf = LogBuf::default();
        let (r, _) = solve_once(&req, &cat, &Params::default(), solve_deadline_s(), &buf);
        r.ok().flatten().map(|s| (s.index_name, s.logodds.to_bits(), s.nmatch, s.wcs.crval, s.wcs.cd))
    };
    let first = one(0);
    let others: Vec<_> = std::thread::scope(|sc| {
        let hs: Vec<_> = (1..=4).map(|k| sc.spawn(move || one(k))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(first.is_some(), "1 本で解けない");
    for (k, o) in others.iter().enumerate() {
        assert_eq!(o, &first, "スレッド {} の答えが 1 本のときと違う", k + 1);
    }
}

//! 解けてはいけない星野・画像(設計書 7・9 節): 締め切りまで探して解なし、締め切りの超過 1 秒以内
//! (dev/solver_truth.json の criteria.negative_overrun_s)。
//!
//! - 乱数の星野: rs/testdata/solver_synth.json の rand60・rand300・rand_hint(Python の solver_synth が種で作ったもの)と、
//!   ここで作る一様乱数の星 150 個(Python の test_solver_core case_08 と同じ形)
//! - 実写の陰性(ピンぼけ・光条)と dev/tcase の乱数の星野の画像: solver_real.json の検出の星(深い・標準)で
//!   Python の試行と同じ条件(全天・スケールの範囲)、画像が手元にあれば利用者の経路(zahyou-image → solve_offline)も
//! - 合成の乱数の星野と実写の陰性で Python が quad を試し尽くしたもの(rand_hint・tcase_few)は、試した quad と仮説の数も
//!   Python と同じか。合成の乱数の星野は最初の 24 個の仮説も比べる(探す順が同じか)
//!
//! 締め切り 30 秒の --ignored では、Python が 15 秒の締め切りで止まった画像でも Rust は quad を試し尽くすことが多い
//! (そのときは「締め切りとの差」が負になる)。
//! - 解の判定の条件(設計書 3.7 節の 6): tcase_dense の深い検出の星は、tune-up の一致の増えと試した数の値上げのどちらかが
//!   あれば最後まで探して解なし、両方を外すと前と同じ誤った解(仮説 16,187,319 個目)に届くか(negative_dense_accept_rule)
//!
//! ```text
//! cargo test -p zahyou-solver --test negative -- --nocapture             短い締め切り(8 秒)で代表だけ
//! cargo test -p zahyou-solver --test negative -- --ignored --nocapture   全部(締め切り 30 秒。14 分ほど)
//! ZAHYOU_NEG_DEADLINE=60 …                                                締め切りを変える
//! ```

mod common;

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use common::*;
use zahyou_image::{DetectOpts, detect_sources, load_image_any, preprocess};
use zahyou_solver::{Control, FieldStar, OfflineRequest, Params, SolveRequest};

const OVERRUN_S: f64 = 1.0;

fn neg_deadline(default: f64) -> f64 {
    std::env::var("ZAHYOU_NEG_DEADLINE").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// 擬似乱数(xorshift64*。試験の星野を作るだけ)。
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 一様乱数の星 n 個(明るい順)。
pub fn random_field(n: usize, w: usize, h: usize, seed: u64) -> Vec<FieldStar> {
    let mut r = Rng::new(seed);
    let mut v: Vec<FieldStar> = (0..n)
        .map(|_| FieldStar { x: r.next_f64() * (w as f64 - 1.0), y: r.next_f64() * (h as f64 - 1.0), flux: 1.0 + 99.0 * r.next_f64() })
        .collect();
    v.sort_by(|a, b| b.flux.total_cmp(&a.flux));
    v
}

/// 解なし・超過 1 秒以内か。log は解法のログの最後の行(どこまで探したかの参考に出す)。
fn judge(
    label: &str,
    r: Result<Option<zahyou_solver::Solution>, zahyou_solver::SolveError>,
    dt: f64,
    deadline: f64,
    log: &str,
    bad: &mut Vec<String>,
) {
    let over = dt - deadline;
    let m = match &r {
        Ok(None) if over <= OVERRUN_S => {
            let tail = if log.is_empty() { String::new() } else { format!("  ログ: {log}") };
            row("PASS", label, &format!("解なし(締め切りとの差 {over:+.3} 秒){tail}"));
            return;
        }
        Ok(None) => format!("締め切りを {over:.3} 秒超えた"),
        Ok(Some(s)) => format!("解けてはいけないのに解が出た({} 対数オッズ {:.1}・一致 {})", s.index_name, s.logodds, s.nmatch),
        Err(e) => format!("エラー {e:?}"),
    };
    row("FAIL", label, &m);
    bad.push(format!("{label}: {m}"));
}

fn synth_random(names: &[&str], deadline: f64, label: &str) {
    let Some(cat) = catalog() else {
        skip(label, "星図データが無い");
        return;
    };
    let Some(doc) = load_testdata("solver_synth.json") else {
        skip(label, "solver_synth.json が無い");
        return;
    };
    let _g = heavy();
    let mut bad = Vec::new();
    for c in doc["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap_or("");
        if !names.contains(&name) || c.get("skipped").is_some() {
            continue;
        }
        let req = SolveRequest {
            stars: stars_xy_flux(&c["xy"], &c["flux"]),
            width: c["width"].as_u64().unwrap() as usize,
            height: c["height"].as_u64().unwrap() as usize,
            scale_lo: opt_f64(&c["scale_lo"]),
            scale_hi: opt_f64(&c["scale_hi"]),
            hint: hint_of(&c["hint"]),
        };
        let buf = LogBuf::default();
        let (r, dt, trace) = solve_traced(&req, &cat, &Params::default(), deadline, &buf);
        let r = r.map(|o| o.map(|d| d.solution));
        judge(&format!("{name}(締め切り {deadline} 秒)"), r, dt, deadline, &buf.tail(1), &mut bad);
        // 探す順も Python と同じか: Python が quad を試し尽くした(unsolved)なら trace の全部、締め切りで
        // 止まった(deadline)なら最初の仮説だけを比べる(Rust が同じ数まで記録していれば)
        let py = &c["python"];
        if let Some(t) = &trace {
            let d = match py["status"].as_str() {
                Some("unsolved") => trace_diff(t, py),
                _ => {
                    let rh = t["hypotheses_log"].as_array().cloned().unwrap_or_default();
                    let ph = py["hyp_first"].as_array().cloned().unwrap_or_default();
                    ph.iter()
                        .zip(&rh)
                        .enumerate()
                        .find_map(|(k, (p, r))| hyp_diff(r, p).map(|m| format!("仮説 {k}: {m}")))
                        .into_iter()
                        .collect()
                }
            };
            let what = format!("{name} trace(Python は {})", py["status"]);
            if d.is_empty() {
                row("PASS", &what, "探す順も Python と同じ");
            } else {
                row("FAIL", &what, &d.join("、"));
                bad.push(format!("{what}: {}", d.join("、")));
            }
        }
    }
    // ここで作る一様乱数の星 150 個(ブラインド)
    let req = SolveRequest { stars: random_field(150, 968, 548, 8), width: 968, height: 548, ..Default::default() };
    let buf = LogBuf::default();
    let (r, dt) = solve_once(&req, &cat, &Params::default(), deadline.min(10.0), &buf);
    judge(&format!("uniform150(締め切り {} 秒)", deadline.min(10.0)), r, dt, deadline.min(10.0), &buf.tail(1), &mut bad);
    assert!(bad.is_empty(), "{} 件:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn negative_synth_quick() {
    synth_random(&["rand60", "rand_hint"], neg_deadline(8.0), "negative_synth_quick");
}

#[test]
#[ignore]
fn negative_synth_all() {
    synth_random(&["rand60", "rand300", "rand_hint"], neg_deadline(30.0), "negative_synth_all");
}

/// tune-up と試した数の条件(設計書 3.7 節の 6)を外すと出る誤った解。
///
/// tcase_dense(dev/tcase/dense.fits。乱数の密な星野)の深い検出の星 400 個は、全天ブラインドで仮説 16,187,319 個目に
/// index-4112(三角形の索引)の誤った解に届いていた。4 点の検証は対数オッズ 18.7・一致 5 で、tune-up(同じ星への当て直し)で
/// 22.773709888689385 に上がり、ln(1e9) = 20.72 を超えた。一致は 5 のままだった(2026-10-24 に見つけた。それまでは
/// 締め切り 30 秒に届かない機械の速さに隠れていた)。
const DENSE_FALSE_INDEX: &str = "index-4112";
const DENSE_FALSE_LOGODDS_BITS: u64 = 4627103447142662977; // 22.773709888689385
const DENSE_FALSE_NMATCH: usize = 5;
const DENSE_FALSE_HYPS: u64 = 16_187_319;

/// tcase_dense の深い検出の星を、Python の試行と同じ条件(全天ブラインド・試行のスケールの範囲)で。
fn dense_deep_request() -> Option<SolveRequest> {
    let doc = load_testdata("solver_real.json")?;
    let img = doc["images"].as_array()?.iter().find(|i| i["name"].as_str() == Some("tcase_dense"))?.clone();
    if img.get("skipped").is_some_and(|v| !v.is_null()) {
        return None;
    }
    let att = img["attempts"].as_array().and_then(|a| a.iter().find(|a| a["stars"].as_str() == Some("deep")).cloned());
    Some(SolveRequest {
        stars: stars_xyf(img.get("stars_deep").filter(|v| !v.is_null())?),
        width: img["width"].as_u64()? as usize,
        height: img["height"].as_u64()? as usize,
        scale_lo: att.as_ref().and_then(|a| a["scale_lo"].as_f64()),
        scale_hi: att.as_ref().and_then(|a| a["scale_hi"].as_f64()),
        hint: att.as_ref().and_then(|a| hint_of(&a["hint"])),
    })
}

/// 解の判定の 2 つの条件が、それぞれ単独でも tcase_dense の誤った解を止め、両方を外すと前と同じ誤った解に届くか。
///
/// 条件ごとに締め切りまで(既定 180 秒。この機械では約 28 秒で quad を試し尽くす)探し、仮説 16,187,319 個目を
/// 越えたところまで試したことも確かめる(越える前に締め切りが来たら、確かめられなかったとして失敗にする)。
/// 止めたときは、探索の終わりの行に、見送ったその候補のまとめ(条件ごとの文・最大 22.8)が付くことも確かめる。
/// 両方を外したときは、系列・対数オッズのビット・一致の数・仮説の番号まで前と同じ誤った解になる
/// (探す順が変わっていないことと、止めているのがこの 2 つの条件であることの確かめ)。
#[test]
#[ignore]
fn negative_dense_accept_rule() {
    let label = "negative_dense_accept_rule";
    let Some(cat) = catalog() else {
        skip(label, "星図データが無い");
        return;
    };
    let Some(req) = dense_deep_request() else {
        skip(label, "solver_real.json に tcase_dense の深い星が無い");
        return;
    };
    let deadline = neg_deadline(180.0);
    let _g = heavy();
    let mut bad = Vec::new();
    let d = Params::default();
    // 止めたときの最後の行の末尾(見送った候補のまとめ)。止めたのはどれもこの誤った解 1 件だけ
    let short = "。改良の後の対数オッズが足りず見送った候補 1 件(最大 22.8、要った値 25.8)";
    let nogain = "。tune-up で一致が増えず見送った候補 1 件(改良の後の対数オッズの最大 22.8)";
    let cases = [
        ("既定(両方の条件)", d.clone(), false, short),
        ("tune-up の一致の増えだけ", Params { accept_hyps0: f64::INFINITY, ..d.clone() }, false, nogain),
        ("試した数の値上げだけ", Params { tune_min_gain: 0, ..d.clone() }, false, short),
        ("両方を外す", Params { tune_min_gain: 0, accept_hyps0: f64::INFINITY, ..d.clone() }, true, ""),
    ];
    for (what, p, expect_false, note) in cases {
        let buf = LogBuf::default();
        let (r, dt, trace) = solve_traced(&req, &cat, &p, deadline, &buf);
        let tried = trace.as_ref().and_then(|t| t["hypotheses"].as_u64()).unwrap_or(0);
        let label = format!("tcase_dense stars_deep {what}(締め切り {deadline} 秒)");
        let r = r.map(|o| o.map(|x| x.solution));
        if expect_false {
            let m = match &r {
                Ok(Some(s)) => {
                    let same = s.index_name == DENSE_FALSE_INDEX
                        && s.logodds.to_bits() == DENSE_FALSE_LOGODDS_BITS
                        && s.nmatch == DENSE_FALSE_NMATCH
                        && s.hypotheses == DENSE_FALSE_HYPS;
                    let m = format!("{} 対数オッズ {}・一致 {}・仮説 {} 個目、{dt:.2} 秒", s.index_name, s.logodds, s.nmatch, s.hypotheses);
                    if same {
                        row("PASS", &label, &format!("前と同じ誤った解: {m}"));
                        continue;
                    }
                    format!("前と違う解 {m}")
                }
                other => format!("前の誤った解に届かない({other:?}、仮説 {tried} 個、{dt:.2} 秒)"),
            };
            row("FAIL", &label, &m);
            bad.push(format!("{label}: {m}"));
            continue;
        }
        match r {
            Ok(None) if tried > DENSE_FALSE_HYPS && buf.tail(1).ends_with(note) => {
                row("PASS", &label, &format!("解なし(仮説 {tried} 個を試した、{dt:.2} 秒)  ログ: {}", buf.tail(1)));
            }
            Ok(None) if tried > DENSE_FALSE_HYPS => {
                let m = format!("解なしだが、最後の行が「…{note}」で終わらない: {}", buf.tail(1));
                row("FAIL", &label, &m);
                bad.push(format!("{label}: {m}"));
            }
            Ok(None) => {
                let m = format!("仮説 {tried} 個で締め切り({dt:.2} 秒)。{DENSE_FALSE_HYPS} 個目まで届かず確かめられない");
                row("FAIL", &label, &m);
                bad.push(format!("{label}: {m}"));
            }
            other => judge(&label, other, dt, deadline, &buf.tail(1), &mut bad),
        }
    }
    assert!(bad.is_empty(), "{} 件:\n{}", bad.len(), bad.join("\n"));
}

/// 実写の陰性: 検出の星(solver_real.json)でブラインド。
fn real_stars(tags: Option<&[&str]>, deadline: f64, label: &str) {
    let Some(cat) = catalog() else {
        skip(label, "星図データが無い");
        return;
    };
    let Some(doc) = load_testdata("solver_real.json") else {
        skip(label, "solver_real.json が無い");
        return;
    };
    let _g = heavy();
    let mut bad = Vec::new();
    let mut n = 0;
    for img in doc["images"].as_array().unwrap() {
        if img["expect_solve"].as_bool() != Some(false) {
            continue;
        }
        let name = img["name"].as_str().unwrap_or("?");
        if let Some(t) = tags
            && !t.contains(&name)
        {
            continue;
        }
        if let Some(why) = img.get("skipped").and_then(|v| v.as_str()) {
            skip(name, why);
            continue;
        }
        for key in ["stars_deep", "stars_standard"] {
            let Some(v) = img.get(key).filter(|v| !v.is_null()) else { continue };
            let stars = stars_xyf(v);
            if stars.len() < 3 {
                row("----", &format!("{name} {key}"), &format!("星が {} 個(解析しない)", stars.len()));
                continue;
            }
            n += 1;
            // Python の試行(solve_offline と同じ全天ブラインドの条件。スケールの範囲は試行のもの)
            let which = if key == "stars_deep" { "deep" } else { "standard" };
            let att = img["attempts"].as_array().and_then(|a| a.iter().find(|a| a["stars"].as_str() == Some(which)));
            let req = SolveRequest {
                stars,
                width: img["width"].as_u64().unwrap() as usize,
                height: img["height"].as_u64().unwrap() as usize,
                scale_lo: att.and_then(|a| a["scale_lo"].as_f64()),
                scale_hi: att.and_then(|a| a["scale_hi"].as_f64()),
                hint: att.and_then(|a| hint_of(&a["hint"])),
            };
            let buf = LogBuf::default();
            let (r, dt, trace) = solve_traced(&req, &cat, &Params::default(), deadline, &buf);
            let r = r.map(|o| o.map(|d| d.solution));
            let label = format!("{name} {key}(締め切り {deadline} 秒)");
            judge(&label, r, dt, deadline, &buf.tail(1), &mut bad);
            // Python が quad を試し尽くした試行は、試した quad・仮説の数も同じはず
            if let (Some(a), Some(t)) = (att, &trace)
                && a["status"].as_str() == Some("unsolved")
            {
                let same = (t["quads_tried"].as_u64(), t["hypotheses"].as_u64()) == (a["quads_tried"].as_u64(), a["hypotheses"].as_u64());
                let m = format!("quad {}・仮説 {} / Python {}・{}", t["quads_tried"], t["hypotheses"], a["quads_tried"], a["hypotheses"]);
                row(if same { "PASS" } else { "FAIL" }, &format!("{name} {key} 試した数"), &m);
                if !same {
                    bad.push(format!("{label} 試した数: {m}"));
                }
            }
        }
    }
    eprintln!("  {label}: {n} 件");
    assert!(bad.is_empty(), "{} 件:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn negative_real_stars_quick() {
    // capture_005514(締め切りまで探す)と tcase_few(星 6〜7 個。quad を試し尽くすので数も Python と比べる)
    real_stars(Some(&["capture_005514", "tcase_few"]), neg_deadline(8.0), "negative_real_stars_quick");
}

#[test]
#[ignore]
fn negative_real_stars_all() {
    real_stars(None, neg_deadline(30.0), "negative_real_stars_all");
}

/// 実写の陰性を利用者の経路で(画像が手元にあるときだけ)。solve_offline の全体の締め切りを守るか。
#[test]
#[ignore]
fn negative_real_offline_all() {
    let label = "negative_real_offline_all";
    let (Some(cat), Some(truth)) = (catalog(), load_truth()) else {
        skip(label, "星図データか solver_truth.json が無い");
        return;
    };
    let real = load_testdata("solver_real.json");
    let timeout = neg_deadline(30.0);
    let _g = heavy();
    let mut bad = Vec::new();
    for e in truth["images"].as_array().unwrap() {
        if e["expect_solve"].as_bool() != Some(false) {
            continue;
        }
        let tag = e["tag"].as_str().unwrap();
        let path = image_path(e["path"].as_str().unwrap());
        if !path.is_file() || sha256_file(&path).as_deref() != e["sha256"].as_str() {
            skip(tag, &format!("画像が無いか中身が違う({})", path.display()));
            continue;
        }
        let loaded = match load_image_any(&path) {
            Ok(l) => l,
            Err(err) => {
                bad.push(format!("{tag}: 読めない {err}"));
                continue;
            }
        };
        let prep = preprocess(&loaded.data, true);
        let (src, _) = detect_sources(&prep, &DetectOpts::with_min_sources(60));
        let scale = real
            .as_ref()
            .and_then(|d| d["images"].as_array()?.iter().find(|i| i["name"].as_str() == Some(tag)).cloned())
            .and_then(|i| i["pixel_scale"].as_f64());
        let req = OfflineRequest {
            prep: &prep,
            sources: &src,
            width: loaded.data.width,
            height: loaded.data.height,
            scale,
            hint: None,
            timeout_s: timeout,
        };
        let cancel = AtomicBool::new(false);
        let buf = LogBuf::default();
        let log = |s: &str| buf.push(s);
        let ctl = Control { deadline: None, cancel: &cancel, log: &log };
        let t0 = Instant::now();
        let r = zahyou_solver::solve_offline(&req, &cat, &Params::default(), &ctl);
        let dt = t0.elapsed().as_secs_f64();
        judge(&format!("{tag} 利用者の経路(締め切り {timeout} 秒、星 {} 個)", src.len()), r, dt, timeout, &buf.tail(1), &mut bad);
    }
    assert!(bad.is_empty(), "{} 件:\n{}", bad.len(), bad.join("\n"));
}

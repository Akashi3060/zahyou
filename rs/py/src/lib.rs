//! ノートブック版(Python)から Rust の解法(zahyou-solver の `solve_detailed`)を呼ぶ拡張モジュール `zahyou_native`。
//!
//! Python の `sv_solve` の代わりに dev/zahyou_core.py の `_solve_stars` が呼ぶ。入れていなければ Python の解法のまま。
//! 探す順と判定は Python と同じなので、同じ星・同じ条件なら同じ解になる(速さだけが違う)。
//!
//! 解法は別のスレッドで走らせ、呼んだスレッドは GIL を放して待つ。0.1 秒ごとに GIL を取って、
//! ログを Python の関数へ渡し、中止の関数を呼び、Ctrl+C(KeyboardInterrupt)を見る。
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use zahyou_index::Catalog;
use zahyou_solver::{
    Control, FieldStar, Params, SolveDetail, SolveOptions, SolveRequest, solve_detailed,
};

/// Python 側と取り決めた呼び方の版。引数や戻り値の形を変えたら上げる(Python 側は合わなければ使わない)。
const API: u32 = 1;

/// 星のリストから WCS を求める。解けなければ None。
///
/// xy: [(x, y)](0 始まりの画素)、flux: 明るさ(xy と同じ長さ)、index_dir: 星図データのフォルダー、
/// scale_lo / scale_hi: 画素スケールの範囲 [″/px](None なら手持ちの段から逆算)、hint: (赤経, 赤緯, 半径) 度、
/// timeout: 締め切りまでの秒、cancel: 呼ぶと真偽を返す関数(真なら中止して InterruptedError)、log: 1 行ずつ受ける関数。
#[pyfunction]
#[pyo3(signature = (xy, flux, width, height, index_dir, scale_lo=None, scale_hi=None, hint=None,
                    timeout=None, cancel=None, log=None))]
#[allow(clippy::too_many_arguments)]
fn solve(
    py: Python<'_>,
    xy: Vec<(f64, f64)>,
    flux: Vec<f64>,
    width: usize,
    height: usize,
    index_dir: String,
    scale_lo: Option<f64>,
    scale_hi: Option<f64>,
    hint: Option<(f64, f64, f64)>,
    timeout: Option<f64>,
    cancel: Option<PyObject>,
    log: Option<PyObject>,
) -> PyResult<Option<PyObject>> {
    if xy.len() != flux.len() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "xy({} 個)と flux({} 個)の数が合いません",
            xy.len(),
            flux.len()
        )));
    }
    let req = SolveRequest {
        stars: xy
            .iter()
            .zip(&flux)
            .map(|(&(x, y), &f)| FieldStar { x, y, flux: f })
            .collect(),
        width,
        height,
        scale_lo,
        scale_hi,
        hint,
    };
    let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<String>();
    // allow_threads の中で待つには Sync が要る(Receiver は Sync でない)ので Mutex に入れる。使うのはこのスレッドだけ
    let rx = Mutex::new(rx);

    let outcome = std::thread::scope(|s| -> PyResult<Result<Option<SolveDetail>, String>> {
        let worker = s.spawn(|| {
            let tx = tx;
            let say = |m: &str| {
                let _ = tx.send(m.to_string());
            };
            // 目録は呼ぶたびに作り、終わったら閉じる(Windows で索引ファイルを開いたままにしない)
            let catalog = Catalog::scan(&index_dir);
            if catalog.files.is_empty() {
                say("使えるインデックスファイルがありません");
                return Ok(None);
            }
            let ctl = Control {
                deadline,
                cancel: &stop,
                log: &say,
            };
            let r = solve_detailed(
                &req,
                &catalog,
                &Params::default(),
                &ctl,
                &SolveOptions::default(),
            );
            catalog.close();
            r.map_err(|_| "cancelled".to_string())
        });
        loop {
            let lines: Vec<String> = py.allow_threads(|| {
                let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
                let mut v: Vec<String> = rx
                    .recv_timeout(Duration::from_millis(100))
                    .into_iter()
                    .collect();
                v.extend(rx.try_iter());
                v
            });
            if let Some(f) = &log {
                for m in &lines {
                    f.bind(py).call1((m.as_str(),))?;
                }
            }
            if worker.is_finished() {
                // 終わる直前のログも渡す
                if let Some(f) = &log {
                    let rest: Vec<String> = rx
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .try_iter()
                        .collect();
                    for m in rest {
                        f.bind(py).call1((m.as_str(),))?;
                    }
                }
                return worker
                    .join()
                    .map_err(|_| PyRuntimeError::new_err("Rust の解法が異常終了しました"));
            }
            let mut want_stop = false;
            if let Some(c) = &cancel {
                want_stop = c.bind(py).call0()?.is_truthy()?;
            }
            if let Err(e) = py.check_signals() {
                stop.store(true, Ordering::Relaxed);
                let _ = py.allow_threads(|| worker.join());
                return Err(e);
            }
            if want_stop {
                stop.store(true, Ordering::Relaxed);
            }
        }
    })?;

    match outcome {
        Err(_) => Err(pyo3::exceptions::PyInterruptedError::new_err(
            "中止しました",
        )),
        Ok(None) => Ok(None),
        Ok(Some(d)) => Ok(Some(detail_to_dict(py, &d)?.into_any().unbind())),
    }
}

fn detail_to_dict<'py>(py: Python<'py>, d: &SolveDetail) -> PyResult<Bound<'py, PyDict>> {
    let s = &d.solution;
    let w = &s.wcs;
    let wcs = PyDict::new(py);
    wcs.set_item("crval_xyz", w.crval.to_vec())?;
    wcs.set_item("crpix", w.crpix.to_vec())?;
    wcs.set_item("cd", w.cd.iter().map(|r| r.to_vec()).collect::<Vec<_>>())?;
    if let Some(sip) = &w.sip {
        // (次数+1)×(次数+1) の行列(添字 [p][q] が u^p v^q の係数。Python の SvWcs と同じ)
        let mat = |c: &[f64], n: usize| -> Vec<Vec<f64>> {
            let m = n + 1;
            (0..m)
                .map(|p| {
                    (0..m)
                        .map(|q| c.get(p * m + q).copied().unwrap_or(0.0))
                        .collect()
                })
                .collect()
        };
        wcs.set_item("a", mat(&sip.a, sip.order))?;
        wcs.set_item("b", mat(&sip.b, sip.order))?;
        if sip.ap_order > 0 && !sip.ap.is_empty() {
            wcs.set_item("ap", mat(&sip.ap, sip.ap_order))?;
            wcs.set_item("bp", mat(&sip.bp, sip.ap_order))?;
        }
    }
    let out = PyDict::new(py);
    out.set_item("wcs", wcs)?;
    out.set_item("logodds", s.logodds)?;
    out.set_item("nmatch", s.nmatch)?;
    out.set_item("ndistract", s.ndistract)?;
    out.set_item("nfield", s.nfield)?;
    out.set_item("nref", s.nref)?;
    out.set_item("index_name", s.index_name.as_str())?;
    out.set_item("quads_tried", s.quads_tried)?;
    out.set_item("hypotheses", s.hypotheses)?;
    out.set_item("seconds", s.seconds)?;
    out.set_item("quad_field", d.quad_field.clone())?;
    out.set_item("quad_index", d.quad_index.clone())?;
    out.set_item(
        "matches",
        d.matches
            .iter()
            .map(|m| (m.field, m.star.to_vec(), m.dist_px))
            .collect::<Vec<_>>(),
    )?;
    Ok(out)
}

#[pymodule]
fn zahyou_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solve, m)?)?;
    m.add("API", API)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

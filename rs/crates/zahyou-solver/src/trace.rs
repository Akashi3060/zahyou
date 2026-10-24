//! 途中の値を JSON に書く(Python の `_sv_write_trace`。移植先で同じ値が出るかを確かめるため。設計書 12 節)。
//!
//! 形は Python と同じ: 入力の星(xy・flux)、照合した quad の数・当たりの数、束ごとの経過、検証した仮説
//! (先頭 20000 個: quad の星・4 点で当てはめた WCS・足切りで落ちたか・対数オッズ・一致数)、tune-up、解(WCS・一致)。
//! 実数の書き方は serde_json(最短で戻る桁)。有限でない値は null(Python は NaN・Infinity と書く)。
//! 書けなくても探索には影響しない(Python と同じく黙って捨てる)。

use std::path::Path;

use serde_json::{Map, Value, json};
use zahyou_wcs::Wcs;

use crate::pyfmt::py_round;
use crate::search::{Found, SearchState};
use crate::{Params, SolveRequest};

/// 実数(有限でなければ null)。
fn num(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

fn nums(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|&x| num(x)).collect())
}

/// (n+1)×(n+1) の係数(行優先)を入れ子の並びに。
fn matrix(c: &[f64], n: usize) -> Value {
    let m = n + 1;
    Value::Array((0..m).map(|p| nums(c.get(p * m..p * m + m).unwrap_or(&[]))).collect())
}

/// Python の SvWcs.to_dict。
pub(crate) fn wcs_dict(w: &Wcs) -> Value {
    let (ra, dec) = w.crval_radec();
    let mut d = Map::new();
    d.insert("crval_radec".into(), nums(&[ra, dec]));
    d.insert("crval_xyz".into(), nums(&w.crval));
    d.insert("crpix".into(), nums(&w.crpix));
    d.insert("cd".into(), Value::Array(w.cd.iter().map(|r| nums(r)).collect()));
    if let Some(s) = &w.sip {
        d.insert("a".into(), matrix(&s.a, s.order));
        d.insert("b".into(), matrix(&s.b, s.order));
        if s.has_inverse() {
            d.insert("ap".into(), matrix(&s.ap, s.ap_order));
            d.insert("bp".into(), matrix(&s.bp, s.ap_order));
        }
    }
    Value::Object(d)
}

/// Python の SvParams.as_dict。
fn params_dict(p: &Params) -> Value {
    json!({
        "codetol": num(p.codetol), "distractor": num(p.distractor), "verify_pix": num(p.verify_pix),
        "logodds_solve": num(p.logodds_solve), "logodds_tune": num(p.logodds_tune), "logodds_bail": num(p.logodds_bail),
        "parity": p.parity.name(), "max_quad_stars": p.max_quad_stars, "max_verify_stars": p.max_verify_stars,
        "quad_min_frac": num(p.quad_min_frac), "uniformize_boxes": p.uniformize_boxes, "tweak_order": p.tweak_order,
        "tune_min_gain": p.tune_min_gain, "tune_gain_hyps0": num(p.tune_gain_hyps0), "accept_hyps0": num(p.accept_hyps0),
    })
}

/// trace を書く。
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_trace(
    path: &Path,
    st: &SearchState,
    p: &Params,
    sol: Option<&Found>,
    status: &str,
    req: &SolveRequest,
    xy: &[[f64; 2]],
    flux: &[f64],
) {
    let opt = |v: Option<f64>| v.map_or(Value::Null, num);
    let mut out = Map::new();
    out.insert("status".into(), json!(status));
    out.insert("width".into(), json!(req.width));
    out.insert("height".into(), json!(req.height));
    out.insert("nstars".into(), json!(xy.len()));
    out.insert("xy".into(), Value::Array(xy.iter().map(|q| nums(q)).collect()));
    out.insert("flux".into(), nums(flux));
    out.insert("scale_given".into(), Value::Array(vec![opt(req.scale_lo), opt(req.scale_hi)]));
    out.insert("scale_used".into(), st.scale.map_or(Value::Array(Vec::new()), |(a, b)| nums(&[a, b])));
    out.insert("hint".into(), req.hint.map_or(Value::Null, |(a, b, c)| nums(&[a, b, c])));
    out.insert("params".into(), params_dict(p));
    out.insert("files".into(), json!(st.files));
    out.insert("quads_tried".into(), json!(st.quads));
    out.insert("hits".into(), json!(st.hits));
    out.insert("hypotheses".into(), json!(st.hyps));
    out.insert("quick_rejected".into(), json!(st.quick_rejected));
    out.insert("logodds_max".into(), if st.logodds_max.is_finite() { num(st.logodds_max) } else { Value::Null });
    out.insert("seconds".into(), num(py_round(st.t0.elapsed().as_secs_f64(), 4)));
    out.insert(
        "flushes".into(),
        Value::Array(
            st.flushes
                .iter()
                .map(|f| json!({"newest": f.newest, "quads": f.quads, "hits": f.hits, "hyps": f.hyps, "t": num(f.t)}))
                .collect(),
        ),
    );
    out.insert(
        "tuneups".into(),
        Value::Array(
            st.tuneups
                .iter()
                .map(|(i, b, a)| json!({"index": i, "before": num(py_round(*b, 4)), "after": num(py_round(*a, 4))}))
                .collect(),
        ),
    );
    out.insert("hypotheses_logged".into(), json!(st.hyp_log.len()));
    out.insert(
        "hypotheses_log".into(),
        Value::Array(
            st.hyp_log
                .iter()
                .map(|h| {
                    json!({
                        "index": h.index, "row": h.row, "mirror": h.mirror,
                        "quad_field": h.quad_field, "quad_index": h.quad_index,
                        "crval": nums(&[h.crval.0, h.crval.1]), "crpix": nums(&h.crpix),
                        "cd": Value::Array(h.cd.iter().map(|r| nums(r)).collect()),
                        "scale": num(py_round(h.scale, 6)), "quick_rejected": h.quick_rejected,
                        "logodds": h.logodds.map_or(Value::Null, |v| num(py_round(v, 4))), "nmatch": h.nmatch,
                    })
                })
                .collect(),
        ),
    );
    if let Some(s) = sol {
        out.insert(
            "solution".into(),
            json!({
                "index": s.index_name, "logodds": num(s.logodds), "nmatch": s.nmatch, "ndistract": s.ndistract,
                "nfield": s.nfield, "nref": s.nref, "quad_field": s.quad_field, "quad_index": s.quad_index,
                "wcs": wcs_dict(&s.wcs),
                "matches": Value::Array(s.matches.iter().map(|(i, v, d)| json!([i, nums(v), num(*d)])).collect()),
            }),
        );
    }
    // 字下げなし(大きいので速さ優先)。書けなければ黙って捨てる
    if let Ok(text) = serde_json::to_string(&Value::Object(out)) {
        let _ = std::fs::write(path, text);
    }
}

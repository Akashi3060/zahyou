# -*- coding: utf-8 -*-
"""
解法の速さの物差し(Python 側)。examples/bench_solver.rs と同じ星・同じ条件を Python 基準実装の sv_solve で測る。

  python -B bench_solver.py [--repeat 3] [--cases real_orig:blind,ra0,...] [--out py.json] [--index-dir C:\\AstrometryData]
  cargo run --release -p zahyou-solver --example bench_solver -- --python py.json      (Rust 側。この JSON の秒と並べる)

星と条件は rs/testdata の solver_real.json・solver_synth.json(gen_solver.py が作ったもの)から読むので、
Rust と Python はビットまで同じ入力を解く。項目の名前は bench_solver.rs と同じ:
  <tag>:blind / <tag>:hint / <tag>:scale   基準の実写(real_orig、深い検出の星)をブラインド・ヒント 5°・スケール既知(正解 ×0.75〜1.25)
  <tag>:solve_offline                     利用者の経路(試行のはしご。画像が手元にあるときだけ。読み込み・前処理・検出は含めない)
  ra0・tile_corner・polein・few20・bb・w25deg・many300・rot090   合成の難しい場合(only の系列に絞った目録で)
  rand60:deadline                         解けない星野(ブラインド、締め切り 5 秒)の超過

秒は repeat 回の最短(1 回で 20 秒を超えるものは繰り返さない)。trace は書かない(書くと遅くなる)。
利用者のファイル(%LOCALAPPDATA%\\zahyou)は使わない(gen_solver.py を読み込むと一時フォルダーに向く)。
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import math
import os
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen_solver as gs                                        # noqa: E402  (環境変数を一時フォルダーに向ける)

import numpy as np                                             # noqa: E402

SYNTH = ["ra0", "tile_corner", "polein", "few20", "bb", "w25deg", "many300", "rot090"]
LONG_S = 20.0


def load(name):
    with open(os.path.join(HERE, name), encoding="utf-8") as f:
        return json.load(f)


def solve_best(sc, xy, flux, W, H, cat, lo, hi, hint, repeat, deadline_s=600.0):
    best, last = math.inf, None
    for k in range(max(1, repeat)):
        t0 = time.perf_counter()
        sol = sc.sv_solve(np.asarray(xy, dtype=np.float64).reshape(-1, 2),
                          None if flux is None else np.asarray(flux, dtype=np.float64),
                          int(W), int(H), cat, scale_lo=lo, scale_hi=hi, hint=hint,
                          deadline=time.monotonic() + deadline_s)
        dt = time.perf_counter() - t0
        best = min(best, dt)
        last = sol
        if k == 0 and dt > LONG_S:
            break
    return best, last


def row(name, what, seconds, sol, extra=None):
    r = dict(name=name, what=what, python_seconds=round(seconds, 4),
             status="solved" if sol is not None else "unsolved",
             index=None if sol is None else sol.index_name, logodds=None if sol is None else sol.logodds,
             nmatch=None if sol is None else sol.nmatch, quads_tried=None if sol is None else sol.quads_tried,
             hypotheses=None if sol is None else sol.hypotheses)
    if extra:
        r.update(extra)
    note = "解なし" if sol is None else f"{sol.index_name} 対数オッズ {sol.logodds:.2f}・quad {sol.quads_tried}"
    print(f"  {name:<26s} {seconds:9.4f} s  {note}", flush=True)
    return r


def main():
    try:
        sys.stdout.reconfigure(errors="replace", encoding="utf-8")
    except Exception:
        pass
    ap = argparse.ArgumentParser(description="解法の速さ(Python 基準実装)")
    ap.add_argument("--dev", default=os.environ.get("ZAHYOU_DEV_RS", gs.DEFAULT_DEV))
    ap.add_argument("--index-dir", default=os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData"))
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--cases", help="項目の名前(カンマ区切り。real_orig:blind のように。既定は全部)")
    ap.add_argument("--out", help="結果の JSON")
    args = ap.parse_args()
    sw, si, sc, ss = gs.setup(args.dev)
    cat = si.SvIndexCatalog(args.index_dir) if os.path.isdir(args.index_dir) else None
    if cat is None or not cat.files:
        print(f"星図データが無い({args.index_dir})")
        return 1
    pick = set(args.cases.split(",")) if args.cases else None
    want = lambda n: pick is None or n in pick or n.split(":")[-1] in pick   # noqa: E731
    rows = []
    print(f"星図データ {len(cat.files)} 本、repeat {args.repeat}", flush=True)

    real = load("solver_real.json") if os.path.isfile(os.path.join(HERE, "solver_real.json")) else None
    img = None
    if real:
        img = next((i for i in real["images"] if isinstance(i.get("bench"), list)), None)
    if img is not None:
        W, H = img["width"], img["height"]
        st = np.asarray(img["stars_deep"], dtype=np.float64).reshape(-1, 3)
        for b in img["bench"]:
            name = f"{img['tag']}:{b['label']}"
            if not want(name):
                continue
            hint = None if b["hint"] is None else tuple(b["hint"])
            t, sol = solve_best(sc, st[:, :2], st[:, 2], W, H, cat, b["scale_lo"], b["scale_hi"], hint, args.repeat)
            rows.append(row(name, f"{W}×{H} 実写・深い検出の星 {len(st)} 個・{b['label']}", t, sol))
        # 利用者の経路(画像があれば。検出までは時間に入れない)
        name = f"{img['tag']}:solve_offline"
        path = gs.image_path(img, os.environ.get("ZAHYOU_PY_REPO", gs.DEFAULT_PY_REPO))
        if want(name) and os.path.isfile(path):
            import zahyou_core as zc
            b = zc.load_image_any(path)
            sub, mask, sigma, _ = zc.preprocess(b.data)
            src, _ = zc.detect_sources(sub, mask, sigma, min_sources=60)
            best, ok = math.inf, None
            for _ in range(max(1, args.repeat)):
                t0 = time.perf_counter()
                with contextlib.redirect_stdout(io.StringIO()):
                    hdr = zc.solve_offline(b, src, gs._TMP, timeout=300.0, focal_length_mm=None, hint_radec=None,
                                           hint_radius_deg=5.0, verbose=False, diagnostics=None,
                                           index_dir=args.index_dir, prep=(sub, mask, sigma))
                best = min(best, time.perf_counter() - t0)
                ok = hdr is not None
            rows.append(dict(name=name, what=f"{W}×{H} 実写・利用者の経路(ヒントなし・焦点距離なし。検出は含めない)",
                             python_seconds=round(best, 4), status="solved" if ok else "unsolved"))
            print(f"  {name:<26s} {best:9.4f} s  {'解けた' if ok else '解なし'}", flush=True)
    else:
        print("solver_real.json に bench が無い(実写の 3 条件は飛ばす)", flush=True)

    syn = load("solver_synth.json")
    for c in syn["cases"]:
        name = c["name"]
        if name not in SYNTH or not want(name) or c.get("skipped"):
            continue
        cc = gs.restrict(cat, c["only"]) if c["only"] else cat
        hint = None if c["hint"] is None else tuple(c["hint"])
        t, sol = solve_best(sc, c["xy"], c["flux"], c["width"], c["height"], cc, c["scale_lo"], c["scale_hi"], hint,
                            args.repeat)
        rows.append(row(name, f"{c['width']}×{c['height']} 合成・{c['what']}", t, sol))
    c = next((c for c in syn["cases"] if c["name"] == "rand60"), None)
    if c is not None and want("rand60:deadline"):
        t0 = time.perf_counter()
        sol = sc.sv_solve(np.asarray(c["xy"], dtype=np.float64), np.asarray(c["flux"], dtype=np.float64),
                          c["width"], c["height"], cat, deadline=time.monotonic() + 5.0)
        dt = time.perf_counter() - t0
        rows.append(row("rand60:deadline", "乱数の星 60 個・ブラインド・締め切り 5 秒", dt, sol,
                        dict(overrun_s=round(dt - 5.0, 4))))
    doc = dict(index_dir=args.index_dir, index_files=len(cat.files), repeat=args.repeat, rows=rows)
    if args.out:
        with io.open(args.out, "w", encoding="utf-8", newline="\n") as f:
            json.dump(gs.finite_only(doc), f, ensure_ascii=False, indent=1, allow_nan=False)
        print(f"書いた: {args.out}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

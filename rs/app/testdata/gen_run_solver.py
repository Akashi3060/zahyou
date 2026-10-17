# -*- coding: utf-8 -*-
"""
app の解析の流れ(pipeline.rs の run)を、本物の解法つきで Python 基準実装の run() と突き合わせる値を作る。

  python -B gen_run_solver.py [--only 名前,...] [--out run_solver.json]

Python 版は dev/ の solver_wcs.py・solver_index.py・solver_core.py・zahyou_core.py・zahyou_plot.py・nb_main.py を
dev/build_nb.py と同じ順にメモリ上でつないで使う(ファイルは書かない。sv_solve は本物)。GUI と同じ文言にするため
CANCEL_EVENT を置く(立てない)。図は描くが表示しない。

場面ごとに run() のログ・解けたか・結果のまとめ・ずれ・覚えた機材の画素スケール・かかった秒を書く。
Rust の試験(app/src/pipeline.rs の run_solver_vs_python)は同じ画像・同じ設定で本物の解法を通し、
時刻の入る数だけを伏せてログを 1 行ずつ比べる。

画像のパスは dev/solver_truth.json と同じ決まり: "$ZAHYOU_TEST_IMAGES/…"(既定はホームフォルダー)か、
リポジトリからの相対("dev/…"・"rs/app/testdata/…"。無ければ ZAHYOU_PY_REPO の同じ相対パス)。
手元に無ければその場面は飛ばす(JSON に skipped)。
星が 0〜2 個の小さな画像(few_two.fits・few_none.fits)は、このスクリプトが作ってこのフォルダーに置く。
利用者のファイル(%LOCALAPPDATA%\\zahyou)には触れない(cache.json は場面ごとの一時ファイル)。
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import os
import re
import shutil
import sys
import tempfile
import threading
import time
import warnings

sys.dont_write_bytecode = True
warnings.simplefilter("ignore")

import numpy as np                                            # noqa: E402
import matplotlib                                             # noqa: E402
matplotlib.use("Agg")

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
DEV = os.path.join(REPO, "dev")
DEFAULT_PY_REPO = os.path.join(os.path.expanduser("~"), "Documents", "GitHub", "zahyou")
INDEX_DIR = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
TARGET = "UCAC4 660-021020"
TARGET_RADEC = (61.91199157844, 41.98626242835)   # 手元の cache.json の値(gen_pipeline.py と同じ)
RIG_KEY = "ZWO ASI290MM||968x548|bin2|f"
RIG_SCALE = 1.461614244726863

# (名前, 画像, 設定の上書き, 機材の記憶を入れておくか, 比べ方)
#   比べ方 "all" = ログを全部(時刻の数だけ伏せる)、"advice" = 試行のはしごは時刻で変わるので、はしごの外と案内だけ
BASE = dict(INPUT_MODE="STAR_NAME", TARGET_STAR_NAME=TARGET, RA_INPUT_STR="04h 07m 38.877s",
            DEC_INPUT_STR="+41d 59m 10.512s", FOCAL_LENGTH_MM=None, SOLVE_MODE="OFFLINE",
            ONLINE_TIMEOUT=120, OFFLINE_TIMEOUT=300, USE_TARGET_AS_HINT=True, SEARCH_RADIUS_DEG=5.0,
            IGNORE_EXISTING_WCS=True, SHOW_DETECTED_SOURCES=True, API_KEY="", INDEX_DIR=INDEX_DIR)
ORIG = "$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_10_33.fits"
SCENES = [
    ("orig_star", ORIG, {}, False, "all"),
    ("orig_blind", ORIG, {"INPUT_MODE": "NONE"}, False, "all"),
    ("orig_coords_nohint", ORIG, {"INPUT_MODE": "COORDS", "USE_TARGET_AS_HINT": False}, False, "all"),
    ("orig_focal", ORIG, {"FOCAL_LENGTH_MM": 818.0}, False, "all"),
    # 焦点距離の書き違い(狭いスケールで全天を探して外れ、スケール全域で解ける。外れる試行は時刻しだいで締め切りに当たる)
    ("orig_focal_short", ORIG, {"FOCAL_LENGTH_MM": 2500.0, "INPUT_MODE": "NONE"}, False, "advice"),
    ("orig_rig", ORIG, {"INPUT_MODE": "NONE"}, True, "all"),
    ("flip_star", "dev/realcase/real_flip.fits", {}, False, "all"),
    ("png_star", "dev/realcase/real.png", {}, False, "all"),
    ("jpg_blind", "dev/realcase/real.jpg", {"INPUT_MODE": "NONE"}, False, "all"),
    ("3i_blind", "$ZAHYOU_TEST_IMAGES/Downloads/02013I_00010 20_47_25.fits", {"INPUT_MODE": "NONE"}, False, "all"),
    ("phaethon_blind", "$ZAHYOU_TEST_IMAGES/Downloads/phaethon_00001.fits", {"INPUT_MODE": "NONE"}, False, "all"),
    ("imx432_blind", "$ZAHYOU_TEST_IMAGES/Documents/claude/nextcloud-fitsviewer/samples/sharpcap-16bit.fits",
     {"INPUT_MODE": "NONE"}, False, "all"),
    ("siril_blind", "$ZAHYOU_TEST_IMAGES/Documents/claude/nextcloud-fitsviewer/samples/siril-rgb-float.fit",
     {"INPUT_MODE": "NONE"}, False, "all"),
    ("c2025a6_blind", "$ZAHYOU_TEST_IMAGES/Downloads/C2025A6/C2025A6_00001 04_23_42.fits",
     {"INPUT_MODE": "NONE"}, False, "all"),
    ("starchart_blind", "$ZAHYOU_TEST_IMAGES/Downloads/20261031 3200 Phaethon StarChart 40arcmin.png",
     {"INPUT_MODE": "NONE"}, False, "all"),
    ("few_two", "rs/app/testdata/few_two.fits", {"INPUT_MODE": "NONE"}, False, "all"),
    ("few_none", "rs/app/testdata/few_none.fits", {"INPUT_MODE": "NONE"}, False, "all"),
    ("tcase_few", "dev/tcase/few.fits", {"INPUT_MODE": "NONE", "OFFLINE_TIMEOUT": 60}, False, "all"),
    ("defocus_rig", "$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_55_14.fits",
     {"INPUT_MODE": "NONE", "OFFLINE_TIMEOUT": 20}, True, "advice"),
    ("noindex", ORIG, {"INDEX_DIR": r"C:\no\such\zahyou-index-dir"}, False, "all"),
]


def resolve(path):
    """$ZAHYOU_TEST_IMAGES/… とリポジトリからの相対を実際のパスに。リポジトリに無いもの(dev/tcase・dev/realcase は
    .gitignore)は、ZAHYOU_PY_REPO(既定は Python 版のリポジトリ)の同じ相対パスも見る(rs/testdata/gen_solver.py と同じ)。"""
    if path.startswith("$ZAHYOU_TEST_IMAGES/"):
        base = os.environ.get("ZAHYOU_TEST_IMAGES") or os.path.expanduser("~")
        return os.path.join(base, *path.split("/")[1:])
    here = os.path.join(REPO, *path.split("/"))
    if os.path.exists(here):
        return here
    return os.path.join(os.environ.get("ZAHYOU_PY_REPO", DEFAULT_PY_REPO), *path.split("/"))


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def finite_only(v):
    """JSON に NaN・無限大を書かない(Rust の serde_json は読めない)。"""
    if isinstance(v, float):
        return v if math.isfinite(v) else None
    if isinstance(v, dict):
        return {k: finite_only(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [finite_only(x) for x in v]
    return v


def write_fits_int16(path, a):
    """2 次元の int16 の FITS(BZERO なし)。"""
    h, w = a.shape
    cards = ["SIMPLE  =                    T", "BITPIX  =                   16", "NAXIS   =                    2",
             f"NAXIS1  = {w:20d}", f"NAXIS2  = {h:20d}", "END"]
    head = b"".join(c.ljust(80).encode("ascii") for c in cards)
    head += b" " * ((-len(head)) % 2880)
    body = a.astype(">i2").tobytes()
    body += b"\0" * ((-len(body)) % 2880)
    with open(path, "wb") as f:
        f.write(head + body)


def make_few_images():
    """星が 2 個の画像と、星の無い画像(雑音だけ)。160×120、背景 1000・雑音 σ 5。"""
    rng = np.random.default_rng(5)
    yy, xx = np.mgrid[0:120, 0:160]
    for name, stars in (("few_two.fits", [(50, 40, 3000), (110, 80, 2000)]), ("few_none.fits", [])):
        a = 1000 + rng.normal(0, 5.0, (120, 160))
        for x, y, amp in stars:
            a += amp * np.exp(-((xx - x) ** 2 + (yy - y) ** 2) / (2 * 1.5 ** 2))
        write_fits_int16(os.path.join(HERE, name), np.round(a))


def build_engine():
    sys.path.insert(0, DEV)
    import build_nb                                            # noqa: E402  (main は呼ばない)
    sys.path.pop(0)
    parts, imports = [], []
    for name in tuple(build_nb.SOLVER_MODULES) + ("zahyou_core.py", "zahyou_plot.py"):
        imp, body = build_nb.split_module_head(build_nb.read(name))
        imports += imp
        parts.append(body)
    main = build_nb.read("nb_main.py").replace("\r\n", "\n")
    run_call = "\n_result_wcs = run()\n"
    assert main.endswith(run_call)
    head = build_nb.ENGINE_HEAD
    extra = build_nb.engine_imports(imports, head)
    if extra:
        head += "\n" + "\n".join(extra) + "\n"
    engine = "\n\n".join([head] + parts + [main[: -len(run_call)].rstrip("\n")]) + "\n"
    ns = {"__name__": "zahyou_engine"}
    exec(compile(engine, "zahyou_engine.py", "exec"), ns)
    ns["plt"].show = lambda *a, **k: None
    srcs = {n: sha256(os.path.join(DEV, n)) for n in
            tuple(build_nb.SOLVER_MODULES) + ("zahyou_core.py", "zahyou_plot.py", "nb_main.py")}
    return ns, srcs


def run_scene(ns, work, scene):
    name, path, over, with_rig, compare = scene
    img = resolve(path)
    rec = {"name": name, "path": path, "compare": compare, "with_rig": with_rig}
    if not os.path.isfile(img):
        rec["skipped"] = f"画像がありません: {img}"
        return rec
    params = dict(BASE)
    params.update(over)
    if not os.path.isdir(INDEX_DIR) and "INDEX_DIR" not in over:
        rec["skipped"] = f"星図データがありません: {INDEX_DIR}"
        return rec
    cache = os.path.join(work, f"cache_{name}.json")
    c = {"targets": {TARGET: {"ra": TARGET_RADEC[0], "dec": TARGET_RADEC[1], "name": TARGET}}, "rigs": {}}
    if with_rig:
        c["rigs"][RIG_KEY] = RIG_SCALE
    with io.open(cache, "w", encoding="utf-8") as f:
        json.dump(c, f, ensure_ascii=False, indent=1)
    os.environ["ZAHYOU_CACHE"] = cache
    ns.update(params)
    ns["IMAGE_PATH"] = img
    ns["CANCEL_EVENT"] = threading.Event()          # GUI と同じ文言(［準備］タブ…)。立てない
    lines = []
    ns["_log"] = lambda m="": lines.append(str(m))
    got = {}
    orig_ps, orig_ro = ns["print_solution"], ns["report_offsets"]

    def ps(w, shape, *a, **k):
        s = orig_ps(w, shape, *a, **k)
        got["summary"] = {"ra": float(s["center"].ra.deg), "dec": float(s["center"].dec.deg),
                          "scale": float(s["scale"]), "pa": float(s["pa"]), "parity": s["parity"],
                          "fov_x": float(s["fov_x"]), "fov_y": float(s["fov_y"])}
        return s

    def ro(center, target):
        r = orig_ro(center, target)
        got["offsets"] = [float(r[0].arcmin), float(r[1]), float(r[2]), r[3], r[4]]
        return r

    ns["print_solution"], ns["report_offsets"] = ps, ro
    import matplotlib.pyplot as plt
    plt.close("all")
    t0 = time.perf_counter()
    try:
        w = ns["run"]()
    finally:
        ns["print_solution"], ns["report_offsets"] = orig_ps, orig_ro
    seconds = time.perf_counter() - t0
    nfig = len(plt.get_fignums())
    plt.close("all")
    cache_after = json.load(io.open(cache, encoding="utf-8"))
    rec.update({"sha256": sha256(img), "params": {k: params[k] for k in BASE}, "log": lines,
                "solved": w is not None, "figures": nfig, "rigs_after": cache_after.get("rigs", {}),
                "python_seconds": round(seconds, 3), **got})
    print(f"  {name}: {len(lines)} 行 / 解けた {w is not None} / 図 {nfig} 枚 / {seconds:.1f} 秒", flush=True)
    return rec


def main():
    try:
        sys.stdout.reconfigure(errors="replace", encoding="utf-8")
    except Exception:
        pass
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="")
    ap.add_argument("--out", default=os.path.join(HERE, "run_solver.json"))
    a = ap.parse_args()
    make_few_images()
    ns, srcs = build_engine()
    only = set(filter(None, a.only.split(",")))
    work = tempfile.mkdtemp(prefix="zahyou_gen_run_solver_")
    old_cache = os.environ.get("ZAHYOU_CACHE")
    try:
        runs = [run_scene(ns, work, s) for s in SCENES if not only or s[0] in only]
    finally:
        shutil.rmtree(work, ignore_errors=True)
        if old_cache is None:
            os.environ.pop("ZAHYOU_CACHE", None)
        else:
            os.environ["ZAHYOU_CACHE"] = old_cache
    meta = {"python": sys.version.split()[0], "numpy": np.__version__, "sources_sha256": srcs,
            "index_dir": INDEX_DIR, "made": time.strftime("%Y-%m-%dT%H:%M:%S")}
    doc = {"meta": meta, "target": TARGET, "target_radec": list(TARGET_RADEC), "rig_key": RIG_KEY,
           "rig_scale": RIG_SCALE, "runs": runs}
    if only and os.path.isfile(a.out):
        # 一部だけ作り直すときは、ほかの場面を残す
        old = json.load(io.open(a.out, encoding="utf-8"))
        keep = [r for r in old.get("runs", []) if r["name"] not in only]
        order = [s[0] for s in SCENES]
        doc["runs"] = sorted(keep + runs, key=lambda r: order.index(r["name"]) if r["name"] in order else 999)
    with io.open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(finite_only(doc), f, ensure_ascii=False, indent=1, allow_nan=False)
        f.write("\n")
    print(f"書いた: {a.out} ({os.path.getsize(a.out):,} バイト)")


if __name__ == "__main__":
    main()

"""
app の pipeline.rs / scale.rs の照合用の値を Python 版(基準実装)で作る。

  python -B gen_pipeline.py [--zahyou <Python 版のリポジトリ>] [--oracle <WSL 版の正解 WCS のフォルダー>]

既定の場所は環境変数とホームフォルダーから組み立てる(作者のホームの絶対パスをソースに書かない):
  --zahyou   $ZAHYOU_PY_REPO、無ければ ~/Documents/GitHub/zahyou
  --oracle   $ZAHYOU_ORACLE、無ければ $ZAHYOU_TEST_IMAGES(無ければ ~)/Documents/claude/zahyou/testdata/oracle
             (WSL 版 astrometry.net の正解。リポジトリには入れない作者のデータ。docs/rust-port.md 11 節)
  試験画像   $ZAHYOU_TEST_IMAGES(無ければ ~)/Downloads/Capture_00001 00_10_33.fits(試験の画像と同じ決まり)
pipeline_run.json の画像のパスも同じ決まりの形で書く(Rust の試験 pipeline.rs の resolve_image がこの PC のパスにする):
  image は "$ZAHYOU_TEST_IMAGES/Downloads/…"、dev/realcase の画像(runs の image_path)はリポジトリからの相対
  "dev/realcase/…"(zahyou-rs に無ければ $ZAHYOU_PY_REPO の同じ相対パス)。

書くもの(このスクリプトと同じフォルダー):
  pipeline_format.json   六十進の書き方(astropy の to_string)・{:g}・{:.Nf}・赤経赤緯の文字列の読み方(SkyCoord)
  pipeline_scale.json    estimate_pixel_scale・rig_key・implied_focal_length・index_coverage・blind_scale_bounds・
                         missing_index_advice・_existing_wcs_header
  pipeline_summary.json  wcs_summary・print_solution・_sanity_ok・report_offsets・目標の画素(world_to_pixel)
  pipeline_run.json      run() のログ(手元の試験画像があるときだけ。無ければ "skipped")

Python 版は dev/ の solver_wcs.py・solver_index.py・zahyou_core.py・zahyou_plot.py・nb_main.py を
dev/build_nb.py と同じ手順でメモリ上に組み立てて使う(ファイルは書かない)。solver_core.py は使わない
(別の係が書き換え中。オフライン解析の本体 sv_solve は「解なし」を返す関数に差し替える)。
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import os
import random
import re
import sys
import shutil
import tempfile
import warnings

sys.dont_write_bytecode = True
warnings.simplefilter("ignore")

import numpy as np                                            # noqa: E402
import matplotlib                                             # noqa: E402
matplotlib.use("Agg")
import astropy.units as u                                     # noqa: E402
from astropy.coordinates import SkyCoord, Angle               # noqa: E402
from astropy.io import fits                                   # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.expanduser("~")
IMAGES = os.environ.get("ZAHYOU_TEST_IMAGES") or HOME
DEFAULT_ZAHYOU = os.environ.get("ZAHYOU_PY_REPO") or os.path.join(HOME, "Documents", "GitHub", "zahyou")
DEFAULT_ORACLE = (os.environ.get("ZAHYOU_ORACLE")
                  or os.path.join(IMAGES, "Documents", "claude", "zahyou", "testdata", "oracle"))
TEST_IMAGE_REL = "$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_10_33.fits"   # JSON に書く形
TEST_IMAGE = os.path.join(IMAGES, *TEST_IMAGE_REL.split("/")[1:])
INDEX_DIR = r"C:\AstrometryData"
TARGET = "UCAC4 660-021020"
TARGET_RADEC = (61.91199157844, 41.98626242835)   # 手元の cache.json の値(Gaia DR3 で精密化したもの)


def finite_only(v):
    """JSON に NaN・無限大を書かない(Rust の serde_json は読めない)。null にする。"""
    if isinstance(v, float):
        return v if math.isfinite(v) else None
    if isinstance(v, dict):
        return {k: finite_only(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [finite_only(x) for x in v]
    return v


def write_json(name, obj):
    path = os.path.join(HERE, name)
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(finite_only(obj), f, ensure_ascii=False, indent=None, separators=(",", ":"), allow_nan=False)
        f.write("\n")
    print(f"wrote {name} ({os.path.getsize(path):,} バイト)")


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


# ------------------------------------------------------------------ エンジンを組み立てる ---

def build_engine(zahyou):
    dev = os.path.join(zahyou, "dev")
    sys.path.insert(0, dev)
    import build_nb                                            # noqa: E402  (main は呼ばない)
    sys.path.pop(0)
    parts, imports = [], []
    for name in ("solver_wcs.py", "solver_index.py"):
        imp, body = build_nb.split_module_head(build_nb.read(name))
        imports += imp
        parts.append(body)
    for name in ("zahyou_core.py", "zahyou_plot.py"):
        imp, body = build_nb.split_module_head(build_nb.read(name))
        imports += imp
        parts.append(body)
    main = build_nb.read("nb_main.py").replace("\r\n", "\n")
    run_call = "\n_result_wcs = run()\n"
    assert main.endswith(run_call)
    main_defs = main[: -len(run_call)]
    head = build_nb.ENGINE_HEAD
    extra = build_nb.engine_imports(imports, head)
    if extra:
        head += "\n" + "\n".join(extra) + "\n"
    engine = "\n\n".join([head] + parts + [main_defs.rstrip("\n")]) + "\n"
    ns = {"__name__": "zahyou_engine"}
    exec(compile(engine, "zahyou_engine.py", "exec"), ns)

    # オフライン解析の本体は使わない(いつも「解なし」)
    class SvCancelled(Exception):
        pass
    ns["SvCancelled"] = SvCancelled
    ns["sv_solve"] = lambda *a, **k: None
    import matplotlib.pyplot as plt
    ns["plt"].show = lambda *a, **k: None
    srcs = {n: sha256(os.path.join(dev, n)) for n in
            ("solver_wcs.py", "solver_index.py", "zahyou_core.py", "zahyou_plot.py", "nb_main.py")}
    return ns, srcs


# ------------------------------------------------------------------ 書式 ---

def gen_format():
    rng = random.Random(20261015)
    out = {"hmsdms": [], "g": [], "fixed": [], "coords": []}

    vals = []
    for _ in range(600):
        vals.append((rng.uniform(0, 360), rng.uniform(-90, 90)))
    # 繰り上がりと符号の境目
    for s in (59.994, 59.995, 59.9949999, 59.9950001, 59.9999, 59.5, 59.49, 0.0, 0.004, 0.005, 29.999, 30.0):
        for m in (0, 59):
            for dd in (0, 1, 41, 89):
                v = dd + m / 60.0 + s / 3600.0
                vals.append((v * 15.0 % 360.0, v))
                vals.append((v, -v))
    vals += [(0.0, 0.0), (0.0, -0.0), (359.99999999999, 90.0), (0.0, -90.0), (61.891414, 41.964760),
             (61.91199157844, 41.98626242835), (180.0, -0.5), (0.0, -1e-9), (359.9999999, -89.9999999)]
    for ra, dec in vals:
        c = SkyCoord(ra * u.deg, dec * u.deg, frame="icrs")
        row = [ra, dec,
               c.to_string("hmsdms", precision=1),
               c.to_string("hmsdms", precision=2),
               c.ra.to_string(unit="hourangle", sep="hms", precision=1),
               c.dec.to_string(unit="deg", sep="dms", precision=0, alwayssign=True),
               c.ra.deg, c.dec.deg]
        out["hmsdms"].append(row)

    gvals = [0.0, -0.0, 1.0, 2.0, 2.5, 8.0, 12.0, 818.0, 2.8, 5.6, 1400.0, 2000.0, 0.1, 1e-5, 1e-4, 1.5e-4,
             123456.0, 1234567.0, 999999.4, 999999.5, 0.00001234, 3.14159265, -2.5, 1e20, 1e-20, 5.0, 30.0]
    gvals += [rng.uniform(-1e4, 1e4) for _ in range(60)] + [10 ** rng.uniform(-8, 8) for _ in range(60)]
    out["g"] = [[x, format(x, "g")] for x in gvals]

    fvals = [0.5, 1.5, 2.5, 0.125, 0.375, 2.675, 1.005, 0.045, 1.25, 61.8914145, 41.9647605, 1.4616,
             23.575, 13.345, 0.05, 0.15, 0.25, 0.35, 152.865, 332.875, 1.58335, 0.91775, 1.29025]
    fvals += [rng.uniform(0, 400) for _ in range(150)]
    fvals += [round(rng.uniform(0, 100), 3) + 0.0005 for _ in range(50)]
    for x in fvals:
        out["fixed"].append([x] + [format(x, f".{p}f") for p in range(7)])

    ra_strs = ["04h 07m 38.877s", "04h07m38.877s", "4h07m38s", "4h07.5m", "4.1275h", "4h", "4h07", "4h07m",
               "04 07 38.877", "04:07:38.877", "61.912", "61.912d", "61.912 deg", "61.912 degrees", "61°54′43″",
               "4ʰ07ᵐ38.877ˢ", "04:07:38.877h", "4 7 38.877 h", "-4h07m38s", "25h", "24h", "23h59m59.999s",
               "4h60m", "4h61m", "4h07m60s", "4h07m61s", "1e1d", "1.5e1d", "abc", "", "  4h07m38.877s",
               "4H07M38S", "4 h 7 m 38 s", "+4h07m38s", "−4h07m38s", "4h07m38.877sE", "4h07m38.877sW",
               "61.9 arcmin", "3600arcsec", "1.08 rad", "0.17cycle", "61.912°", "61d54'43\"", "61d54.5'",
               "4hr07m38s", "4hours", "61deg54m43s", ".5d", "5.d", "61.912dE", "61.912dW", "61.912dN",
               "4h07m38.877sN", "4h 07m 38.877s E", "61:54:43d", "61 54 43 d", "61 54d", "61d 54", "4.5h30m",
               "4h7.5m30s", "4h07m38.8.7s", "360d", "-0.5d", "-0d30m", "720.5d", "-360d", "1e400d",
               "61.912 radian", "61.912 arcminute", "61.912 kdeg"]
    dec_strs = ["+41d 59m 10.512s", "41d59m10.512s", "+41 59 10.512", "41.986", "41.986d", "-41d59.2m",
                "41°59′10″", "+41°59′10.512″", "90d", "-90d", "90.0000001d", "95d", "-0d30m", "41d59m60s",
                "41d60m", "41.986dN", "41.986dS", "41d59m10sS", "abc", "", "+41d59m", "1.5 rad", "-1.6rad",
                "41.986dE", "41.986dW", "+41d59m10s W"]
    for i, r in enumerate(ra_strs):
        d = dec_strs[i % len(dec_strs)]
        out["coords"].append(coord_case(r, d))
    for i, d in enumerate(dec_strs):
        out["coords"].append(coord_case("4h07m38.877s", d))
    out["coords"].append(coord_case("04h 07m 38.877s", "+41d 59m 10.512s"))
    return out


def coord_case(r, d):
    try:
        c = SkyCoord(r, d, frame="icrs")
        return [r, d, True, float(c.ra.deg), float(c.dec.deg), ""]
    except Exception as e:                       # noqa: BLE001
        return [r, d, False, None, None, f"{type(e).__name__}: {e}"[:160]]


# ------------------------------------------------------------------ スケール ---

def card_list(h):
    """fits.Header → [[key, 型, 値]](Rust の Card にする)。"""
    out = []
    for c in h.cards:
        k = c.keyword
        if k in ("COMMENT", "HISTORY", ""):
            continue
        # WCS(header) はカードの文字列(実数は 20 文字に丸めたもの)から読むので、値もその文字列から読み直す
        v = fits.Card.fromstring(c.image).value
        if isinstance(v, bool):
            out.append([k, "b", v])
        elif isinstance(v, (int, np.integer)):
            out.append([k, "i", int(v)])
        elif isinstance(v, (float, np.floating)):
            out.append([k, "f", float(v)])
        elif isinstance(v, str):
            out.append([k, "s", v])
        else:
            out.append([k, "n", None])
    return out


def gen_scale(ns, oracle):
    cache = os.path.join(tempfile.mkdtemp(prefix="zahyou_gen_"), "cache.json")
    rigs = {"ZWO ASI290MM||968x548|bin2|f": 1.461614244726863, "?||2015x1001|bin1|f": 1.1820162907221232,
            "cam|C11|100x80|bin2.0|f2800.0": 0.25}
    with io.open(cache, "w", encoding="utf-8") as f:
        json.dump({"targets": {}, "rigs": rigs}, f, ensure_ascii=False, indent=1)
    os.environ["ZAHYOU_CACHE"] = cache

    headers = []

    def add(name, cards, shape=(548, 968)):
        h = fits.Header()
        for k, v in cards:
            h[k] = v
        headers.append((name, h, shape))

    add("empty", [])
    add("asi290", [("INSTRUME", "ZWO ASI290MM"), ("XBINNING", 2), ("XPIXSZ", 5.8)])
    add("asi290_focal", [("INSTRUME", "ZWO ASI290MM"), ("XBINNING", 2), ("XPIXSZ", 5.8), ("FOCALLEN", 818.0)])
    add("asi290_focal_int", [("INSTRUME", "ZWO ASI290MM"), ("XBINNING", 2), ("XPIXSZ", 6), ("FOCALLEN", 818)])
    add("pixstr", [("XPIXSZ", " 3.76 "), ("FOCAL", "2800")])
    add("secpix", [("SECPIX", 1.25), ("XPIXSZ", 3.0)])
    add("cdelt", [("CDELT1", -0.0002777)])
    add("cdelt_big", [("CDELT1", 2.0)])
    add("pltscale_zero", [("SECPIX", 0.0), ("PLTSCALE", 0.5)])
    add("nan_pix", [("XPIXSZ", "nan"), ("PIXSIZE1", 4.63), ("FOCALLEN", 1000.0)])
    add("bool_pix", [("XPIXSZ", True), ("FOCALLEN", 100.0)])
    add("str_bad", [("XPIXSZ", "abc"), ("PIXSZ", "2.4"), ("FOCALLENGTH", 600.0)])
    add("camid", [("INSTRUME", ""), ("CAMID", "  cam "), ("XBINNING", 0), ("BINX", 2.0), ("TELESCOP", " C11 "),
                  ("FOCALLEN", 2800.0)], (80, 100))
    add("camera", [("CAMERA", "QHY"), ("TELESCOP", "FSQ106"), ("XBINNING", 1)], (1001, 2015))
    add("huge_scale", [("XPIXSZ", 5.0), ("FOCALLEN", 0.0001)])
    add("tiny_scale", [("XPIXSZ", 0.001), ("FOCALLEN", 1e6)])
    add("tan_wcs", [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRVAL1", 10.0), ("CRVAL2", 20.0),
                    ("CRPIX1", 50.0), ("CRPIX2", 40.0), ("CD1_1", -0.0004), ("CD1_2", 0.00001),
                    ("CD2_1", 0.00002), ("CD2_2", 0.0004), ("XPIXSZ", 5.8)])
    add("tan_wcs_cdelt", [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRVAL1", 10.0), ("CRVAL2", 20.0),
                          ("CRPIX1", 50.0), ("CRPIX2", 40.0), ("CDELT1", -0.001), ("CDELT2", 0.001)])
    add("tan_wcs_tooscale", [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRVAL1", 10.0), ("CRVAL2", 20.0),
                             ("CRPIX1", 50.0), ("CRPIX2", 40.0), ("CD1_1", -2.0), ("CD1_2", 0.0),
                             ("CD2_1", 0.0), ("CD2_2", 2.0), ("SECPIX", 3.0)])
    # 正解 WCS(SIP つき)
    sol = os.path.join(oracle, "real_orig", "solution.wcs")
    if os.path.exists(sol):
        h = fits.getheader(sol)
        h2 = fits.Header()
        for c in h.cards:
            if c.keyword in ("SIMPLE", "BITPIX", "NAXIS", "EXTEND", "COMMENT", "HISTORY", "DATE", ""):
                continue
            h2[c.keyword] = c.value
        headers.append(("oracle_sip", h2, (548, 968)))

    rows = []
    for name, h, shape in headers:
        row = {"name": name, "cards": card_list(h), "shape": list(shape), "rig_key": ns["rig_key"](h, shape)}
        for focal in (None, 818.0, 0.0, 400.5):
            s, why = ns["estimate_pixel_scale"](h, focal, shape)
            row[f"est_{focal}"] = [s, why]
        s, why = ns["estimate_pixel_scale"](h, None, None)
        row["est_noshape"] = [s, why]
        row["implied_1.4616"] = ns["implied_focal_length"](h, 1.4616)
        row["implied_0"] = ns["implied_focal_length"](h, 0.0)
        ex = ns["_existing_wcs_header"](h, shape)
        row["existing"] = ex is not None
        rows.append(row)

    names_sets = [
        [],
        ["index-4107.fits", "index-5205-02.fits", "index-4119.fits", "readme.txt"],
        ["index-5206-00.fits", "index-5206-47.fits"],
        ["index-4110.fits", "index-4111.fits"],
        sorted(os.listdir(INDEX_DIR)) if os.path.isdir(INDEX_DIR) else ["index-4107.fits"],
        ["index-5200-00.fits", "C:\\x\\index-4115.fits", "index-41.fits", "index-x9.fits", "index-.fits"],
    ]
    idx = []
    for names in names_sets:
        cov = ns["index_coverage"](names)
        r = {"names": names, "coverage": list(cov),
             "blind": [list(ns["blind_scale_bounds"](w, names)) for w in (968, 6000, 100)],
             "advice": []}
        for fov in (1.0, 2.0, 3.0, 9.0, 13.35, 15.99, 16.0, 23.58, 100.0, 1999.0, 2000.0, 5000.0):
            ok, msgs = ns["missing_index_advice"](fov, names)
            r["advice"].append([fov, ok, msgs])
        idx.append(r)
    shutil.rmtree(os.path.dirname(cache), ignore_errors=True)
    return {"rigs": rigs, "headers": rows, "index": idx}


# ------------------------------------------------------------------ 結果のまとめ ---

def capture(ns):
    lines = []
    ns["_log"] = lambda m="": lines.append(str(m))
    return lines


def gen_summary(ns, oracle):
    from astropy.wcs import WCS
    rng = random.Random(7)
    cases = []
    for sub in ("real_orig", "real_flip_fits", "real_png"):
        p = os.path.join(oracle, sub, "solution.wcs")
        if os.path.exists(p):
            res = json.load(io.open(os.path.join(oracle, sub, "result.json"), encoding="utf-8"))
            cases.append((sub, fits.getheader(p), (res["height"], res["width"])))
    # 乱数の WCS(TAN と SIP、鏡像、極の近く、赤経 0° のまたぎ)
    for i in range(24):
        h = fits.Header()
        sip = i % 3 == 2
        h["CTYPE1"] = "RA---TAN-SIP" if sip else "RA---TAN"
        h["CTYPE2"] = "DEC--TAN-SIP" if sip else "DEC--TAN"
        ra0 = [0.001, 359.99, 180.0, rng.uniform(0, 360)][i % 4]
        dec0 = [89.9, -89.5, 0.0, rng.uniform(-80, 80), 45.0][i % 5]
        h["CRVAL1"], h["CRVAL2"] = ra0, dec0
        nx, ny = rng.choice([(968, 548), (6000, 4000), (100, 1000), (17, 16)])
        h["CRPIX1"], h["CRPIX2"] = rng.uniform(0, nx), rng.uniform(0, ny)
        s = 10 ** rng.uniform(-4.5, -1.5)
        th = rng.uniform(0, 2 * math.pi)
        par = -1 if i % 2 else 1
        h["CD1_1"] = -s * math.cos(th)
        h["CD1_2"] = s * math.sin(th) * par
        h["CD2_1"] = s * math.sin(th)
        h["CD2_2"] = s * math.cos(th) * par
        if sip:
            h["A_ORDER"] = 2
            h["B_ORDER"] = 2
            for p_, q in ((2, 0), (1, 1), (0, 2)):
                h[f"A_{p_}_{q}"] = rng.uniform(-1e-6, 1e-6)
                h[f"B_{p_}_{q}"] = rng.uniform(-1e-6, 1e-6)
        cases.append((f"rand{i}", h, (ny, nx)))

    out = []
    for name, h, shape in cases:
        ny, nx = shape
        hn = ns["normalize_wcs_header"](h, shape)
        w = WCS(hn)
        row = {"name": name, "cards": card_list(h), "shape": [ny, nx], "summaries": {}}
        for flip in (None, True, False):
            lines = capture(ns)
            s = ns["print_solution"](w, shape, flip_y=flip)
            row["summaries"][str(flip)] = {
                "ra": float(s["center"].ra.deg), "dec": float(s["center"].dec.deg), "scale": float(s["scale"]),
                "pa": float(s["pa"]), "parity": s["parity"], "fov_x": float(s["fov_x"]), "fov_y": float(s["fov_y"]),
                "lines": lines,
                "hmsdms1": s["center"].to_string("hmsdms", precision=1),
                "ra_hms1": s["center"].ra.to_string(unit="hourangle", sep="hms", precision=1),
                "dec_dms0": s["center"].dec.to_string(unit="deg", sep="dms", precision=0, alwayssign=True),
            }
        lines = capture(ns)
        row["sanity"] = bool(ns["_sanity_ok"](w, shape))
        center = w.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0)
        targets = [TARGET_RADEC, (float(center.ra.deg), float(center.dec.deg)),
                   ((float(center.ra.deg) + 0.01) % 360, float(center.dec.deg) - 0.01),
                   ((float(center.ra.deg) + 180) % 360, -float(center.dec.deg)),
                   ((float(center.ra.deg) + 0.3) % 360, max(-90.0, min(90.0, float(center.dec.deg) + 0.2)))]
        offs = []
        for t in targets:
            tc = SkyCoord(t[0] * u.deg, t[1] * u.deg, frame="icrs")
            lines = capture(ns)
            sep, ra_min, dec_min, ra_dir, dec_dir = ns["report_offsets"](center, tc)
            tx, ty = w.world_to_pixel(tc)
            offs.append({"target": list(t), "sep": float(sep.arcmin), "ra": float(ra_min), "dec": float(dec_min),
                         "ra_dir": ra_dir, "dec_dir": dec_dir, "lines": lines,
                         "pix": [float(tx), float(ty)],
                         "title2": f"距離 {sep.arcmin:.2f}′ | RA {abs(ra_min):.2f}′({ra_dir}) "
                                   f"Dec {abs(dec_min):.2f}′({dec_dir})"})
        row["offsets"] = offs
        out.append(row)
    return out


# ------------------------------------------------------------------ run() のログ ---

def wcs_card_images(oracle, sub="real_orig"):
    """正解 WCS のうち画像へ足すカード(80 文字の文字列のまま)。"""
    p = os.path.join(oracle, sub, "solution.wcs")
    raw = open(p, "rb").read()
    out = []
    for i in range(0, len(raw), 80):
        c = raw[i:i + 80].decode("ascii")
        k = c[:8].strip()
        if k == "END":
            break
        if k in ("SIMPLE", "BITPIX", "NAXIS", "EXTEND", "COMMENT", "HISTORY", "DATE", ""):
            continue
        out.append(c)
    return out


def insert_cards(fits_bytes, cards):
    """FITS の最初のヘッダーの END の前にカードを足す(データはそのまま。ヘッダーは 2880 の倍数に詰め直す)。"""
    pos = 0
    while True:
        c = fits_bytes[pos:pos + 80]
        if c[:8] == b"END     ":
            break
        pos += 80
    head = fits_bytes[:pos] + b"".join(x.encode("ascii") for x in cards) + b"END".ljust(80)
    end_old = (pos // 2880 + 1) * 2880
    head += b" " * ((-len(head)) % 2880)
    return head + fits_bytes[end_old:]


class _OracleWcs:
    """sv_solve の解の wcs の代わり(to_header_cards だけ。正解 WCS のカードをそのまま返す)。"""

    def __init__(self, cards):
        self.cards = cards

    def to_header_cards(self, width, height, comments=()):
        out = []
        for c in self.cards:
            card = fits.Card.fromstring(c)
            out.append((card.keyword, card.value, card.comment))
        return out + [("COMMENT", x, "") for x in comments]


class _OracleSolution:
    """sv_solve の解の代わり(WSL 版の正解 WCS を返す)。"""

    def __init__(self, cards):
        self.wcs = _OracleWcs(cards)
        self.index_name = "index-5205-02"
        self.logodds = 87.77
        self.nmatch = 13
        self.quads_tried = 0
        self.seconds = 0.0


REALCASE_REL = "dev/realcase"      # JSON にはリポジトリからの相対で書く(dev/realcase は .gitignore)
REALCASE = os.path.join(DEFAULT_ZAHYOU, *REALCASE_REL.split("/"))


def gen_run(ns, oracle):
    if not os.path.exists(TEST_IMAGE):
        return {"skipped": f"試験画像がありません: {TEST_IMAGE_REL}"}
    work = tempfile.mkdtemp(prefix="zahyou_gen_run_")
    cards = wcs_card_images(oracle)
    with_wcs = os.path.join(work, "zahyou_wcs_test.fits")
    with open(with_wcs, "wb") as f:
        f.write(insert_cards(open(TEST_IMAGE, "rb").read(), cards))
    plain = os.path.join(work, "zahyou_plain_test.fits")
    with open(plain, "wb") as f:
        f.write(open(TEST_IMAGE, "rb").read())

    base = dict(INPUT_MODE="STAR_NAME", TARGET_STAR_NAME=TARGET, RA_INPUT_STR="04h 07m 38.877s",
                DEC_INPUT_STR="+41d 59m 10.512s", FOCAL_LENGTH_MM=None, SOLVE_MODE="OFFLINE",
                ONLINE_TIMEOUT=120, OFFLINE_TIMEOUT=300, USE_TARGET_AS_HINT=True, SEARCH_RADIUS_DEG=5.0,
                IGNORE_EXISTING_WCS=False, SHOW_DETECTED_SOURCES=True, API_KEY="", INDEX_DIR=INDEX_DIR,
                CANCEL_EVENT=None)
    scen = [
        ("wcs_star", "wcs", {}, False),
        ("wcs_none", "wcs", {"INPUT_MODE": "NONE"}, False),
        ("wcs_coords", "wcs", {"INPUT_MODE": "COORDS", "SHOW_DETECTED_SOURCES": False}, False),
        ("wcs_coords_bad", "wcs", {"INPUT_MODE": "COORDS", "RA_INPUT_STR": "04 07 38.9"}, False),
        ("wcs_star_unknown", "wcs", {"TARGET_STAR_NAME": "NO SUCH STAR 123"}, False),
        ("wcs_star_unknown_nocoords", "wcs", {"TARGET_STAR_NAME": "NO SUCH STAR 123", "RA_INPUT_STR": ""}, False),
        ("wcs_star_focal", "wcs", {"FOCAL_LENGTH_MM": 818.0}, False),
        ("wcs_rig", "wcs", {}, True),
        ("off_blind", "plain", {"IGNORE_EXISTING_WCS": True}, False),
        ("off_focal", "plain", {"IGNORE_EXISTING_WCS": True, "FOCAL_LENGTH_MM": 818.0}, False),
        ("off_focal_short", "plain", {"IGNORE_EXISTING_WCS": True, "FOCAL_LENGTH_MM": 2500.0}, False),
        ("off_rig", "plain", {"IGNORE_EXISTING_WCS": True, "INPUT_MODE": "NONE"}, True),
        ("off_noindex", "plain", {"IGNORE_EXISTING_WCS": True, "INDEX_DIR": r"C:\no\such\zahyou-index-dir"}, False),
        # 解法の代わりに WSL 版の正解 WCS を返す(オフラインの経路を最後まで通す。上下反転の FITS と PNG も)
        ("oracle_orig", "plain", {"IGNORE_EXISTING_WCS": True}, False, "real_orig"),
        ("oracle_orig_none", "plain", {"IGNORE_EXISTING_WCS": True, "INPUT_MODE": "NONE"}, False, "real_orig"),
        ("oracle_flip", os.path.join(REALCASE, "real_flip.fits"), {"IGNORE_EXISTING_WCS": True}, False, "real_flip_fits"),
        ("oracle_png", os.path.join(REALCASE, "real.png"), {"IGNORE_EXISTING_WCS": True, "FOCAL_LENGTH_MM": 818.0},
         False, "real_png"),
    ]
    out = {"wcs_cards": cards, "image": TEST_IMAGE_REL, "image_sha256": sha256(TEST_IMAGE), "runs": []}
    for row in scen:
        name, which, over, with_rig = row[:4]
        sub = row[4] if len(row) > 4 else None
        if which not in ("wcs", "plain") and not os.path.exists(which):
            print(f"  run {name}: 画像がないので飛ばす ({which})")
            continue
        if which != "wcs" and not os.path.isdir(INDEX_DIR) and "INDEX_DIR" not in over:
            continue
        cache = os.path.join(work, f"cache_{name}.json")
        c = {"targets": {TARGET: {"ra": TARGET_RADEC[0], "dec": TARGET_RADEC[1], "name": TARGET}}, "rigs": {}}
        if with_rig:
            c["rigs"]["ZWO ASI290MM||968x548|bin2|f"] = 1.461614244726863
        with io.open(cache, "w", encoding="utf-8") as f:
            json.dump(c, f, ensure_ascii=False, indent=1)
        os.environ["ZAHYOU_CACHE"] = cache
        ns.update(base)
        ns.update(over)
        ns["IMAGE_PATH"] = {"wcs": with_wcs, "plain": plain}.get(which, which)
        oracle_cards = wcs_card_images(oracle, sub) if sub else None
        ns["sv_solve"] = ((lambda *a, **k: _OracleSolution(oracle_cards)) if sub
                          else (lambda *a, **k: None))
        lines = capture(ns)
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
        try:
            w = ns["run"]()
        finally:
            ns["print_solution"], ns["report_offsets"] = orig_ps, orig_ro
        nfig = len(plt.get_fignums())
        plt.close("all")
        # 実行のたびに変わる「所要時間」は伏せる(作り直しても同じ JSON になるように)
        lines = [re.sub(r"(所要時間      : )[0-9.]+( 秒)", r"\1X\2", ln) for ln in lines]
        cache_after = json.load(io.open(cache, encoding="utf-8"))
        extra = {}
        if sub:
            extra["oracle"] = sub
            extra["oracle_cards"] = oracle_cards
        if which not in ("wcs", "plain"):
            extra["image_path"] = REALCASE_REL + "/" + os.path.basename(which)
            extra["image_path_sha256"] = sha256(which)
        out["runs"].append({"name": name, "which": which if which in ("wcs", "plain") else "other",
                            "params": {k: ns[k] for k in base if k != "CANCEL_EVENT"},
                            "with_rig": with_rig, "log": lines, "solved": w is not None, "figures": nfig,
                            "cache_after": cache_after, **extra, **got})
        print(f"  run {name}: {len(lines)} 行 / 解けた {w is not None} / 図 {nfig} 枚")
    shutil.rmtree(work, ignore_errors=True)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zahyou", default=DEFAULT_ZAHYOU)
    ap.add_argument("--oracle", default=DEFAULT_ORACLE)
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    ns, srcs = build_engine(a.zahyou)
    meta = {"python": sys.version.split()[0], "numpy": np.__version__,
            "astropy": __import__("astropy").__version__, "sources_sha256": srcs}
    only = set(filter(None, a.only.split(",")))
    if not only or "format" in only:
        write_json("pipeline_format.json", {"meta": meta, **gen_format()})
    if not only or "scale" in only:
        write_json("pipeline_scale.json", {"meta": meta, **gen_scale(ns, a.oracle)})
    if not only or "summary" in only:
        write_json("pipeline_summary.json", {"meta": meta, "cases": gen_summary(ns, a.oracle)})
    if not only or "run" in only:
        write_json("pipeline_run.json", {"meta": meta, **gen_run(ns, a.oracle)})


if __name__ == "__main__":
    main()

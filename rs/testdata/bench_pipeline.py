"""
zahyou.exe --bench と同じ段を Python 版(基準実装)で測る。同じ機械・同じ画像で比べるための物差し。

  set ZAHYOU_BENCH_DIR=<フォルダー>
  zahyou.exe --bench --out rust.json          (合成画像 synth_6000x4000.fits をそのフォルダーに作って残す)
  python -B bench_pipeline.py <フォルダー>\\synth_6000x4000.fits [実写.fits ...] [--out py.json] [--repeat 3]

段: 読み込み(load_image_any)・前処理(preprocess)・検出(detect_sources, min_sources=60)・
北が上の再投影(plot_north_up の reproject_interp まで、block_size=(512, 512))・
図 2 枚(plot_original + plot_north_up。matplotlib は描くのを savefig まで遅らせるので、ここは図の組み立て)・
PNG(2 枚を savefig。dpi 100)。WCS は画像のヘッダーのもの(合成画像は TAN の WCS を持つ)。
Python 版は gen_pipeline.py と同じくメモリ上に組み立てたエンジン(dev/zahyou_core.py ほか)を使う。
"""

from __future__ import annotations

import argparse
import io
import json
import os
import sys
import time
import warnings

sys.dont_write_bytecode = True
warnings.simplefilter("ignore")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen_pipeline                                            # noqa: E402

import numpy as np                                             # noqa: E402


def best_of(n, fn):
    best, out = float("inf"), None
    for _ in range(n):
        t = time.perf_counter()
        out = fn()
        best = min(best, time.perf_counter() - t)
    return best, out


def bench_one(ns, path, repeat):
    from astropy.wcs import WCS
    from astropy.coordinates import SkyCoord
    import astropy.units as u
    plt = ns["plt"]
    stages = []
    t, b = best_of(repeat, lambda: ns["load_image_any"](path))
    stages.append(("読み込み", t, b.note))
    t, pre = best_of(repeat, lambda: ns["preprocess"](b.data))
    sub, mask, sigma, _ = pre
    stages.append(("前処理", t, f"σ = {sigma:.3f}"))
    t, (src, thr) = best_of(repeat, lambda: ns["detect_sources"](sub, mask, sigma, min_sources=60))
    stages.append(("検出", t, f"{len(src)} 個 (しきい値 {thr:g})"))
    h = ns["_existing_wcs_header"](b.header, b.shape)
    if h is None:
        return {"path": path, "width": b.shape[1], "height": b.shape[0],
                "stages": [{"name": n, "seconds": s, "note": x} for n, s, x in stages]}
    w = WCS(h)
    ny, nx = b.shape
    c = w.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0)
    target = SkyCoord((c.ra.deg + 0.01) * u.deg, min(c.dec.deg + 0.01, 90.0) * u.deg, frame="icrs")

    def reproject_only():
        from reproject import reproject_interp
        from astropy.wcs.utils import proj_plane_pixel_scales
        scale = float(np.mean(proj_plane_pixel_scales(w.celestial)))
        out = WCS(naxis=2)
        out.wcs.ctype = ["RA---TAN", "DEC--TAN"]
        out.wcs.crval = [c.ra.deg, c.dec.deg]
        out.wcs.cdelt = np.array([-scale, scale])
        out.wcs.crpix = [1.0, 1.0]
        sky = w.pixel_to_world(np.array([0, nx - 1, 0, nx - 1.0]), np.array([0, 0, ny - 1, ny - 1.0]))
        px, py = out.world_to_pixel(sky)
        out.wcs.crpix = [1.0 - float(np.min(px)), 1.0 - float(np.min(py))]
        shape_out = (int(np.ceil(np.ptp(py))) + 1, int(np.ceil(np.ptp(px))) + 1)
        rep, foot = reproject_interp((np.nan_to_num(b.display), w), out, shape_out=shape_out, block_size=(512, 512))
        return shape_out

    t, shp = best_of(repeat, reproject_only)
    stages.append(("北が上の再投影", t, f"{shp[1]} x {shp[0]} 画素"))
    ns["_log"] = lambda m="": None

    def figs():
        plt.close("all")
        ns["plot_original"](w, b.display, target, b.flip_y, sources=src)
        ns["plot_north_up"](w, b.display, target)
        return [plt.figure(n) for n in sorted(plt.get_fignums())]

    t, fs = best_of(repeat, figs)
    stages.append(("図 2 枚 (再投影込み)", t, "matplotlib の図の組み立て"))

    def png():
        sizes = []
        for f in fs:
            buf = io.BytesIO()
            f.savefig(buf, format="png", dpi=100)
            sizes.append(len(buf.getvalue()))
        return sizes

    t, sizes = best_of(repeat, png)
    stages.append(("PNG (図 2 枚)", t, " + ".join(f"{s} バイト" for s in sizes)))
    plt.close("all")

    # 解析全体 (run): Rust の --bench と同じ設定(画像の WCS を使う・オフライン・既定の目標、cache.json は一時ファイル)
    import tempfile
    cache = os.path.join(tempfile.mkdtemp(prefix="zahyou_bench_"), "cache.json")
    os.environ["ZAHYOU_CACHE"] = cache
    ns.update(INPUT_MODE="STAR_NAME", TARGET_STAR_NAME="UCAC4 660-021020", RA_INPUT_STR="04h 07m 38.877s",
              DEC_INPUT_STR="+41d 59m 10.512s", FOCAL_LENGTH_MM=None, SOLVE_MODE="OFFLINE", ONLINE_TIMEOUT=120,
              OFFLINE_TIMEOUT=60, USE_TARGET_AS_HINT=True, SEARCH_RADIUS_DEG=5.0, IGNORE_EXISTING_WCS=False,
              SHOW_DETECTED_SOURCES=True, API_KEY="", INDEX_DIR=r"C:\AstrometryData", CANCEL_EVENT=None,
              IMAGE_PATH=path)

    def full():
        if os.path.exists(cache):
            os.remove(cache)
        plt.close("all")
        w2 = ns["run"]()
        n = len(plt.get_fignums())
        plt.close("all")
        return w2, n

    t, (w2, n) = best_of(repeat, full)
    stages.append(("解析全体 (run)", t, f"{'解けた' if w2 is not None else '解けなかった'} / 図 {n} 枚"))
    import shutil
    shutil.rmtree(os.path.dirname(cache), ignore_errors=True)
    return {"path": path, "width": nx, "height": ny,
            "stages": [{"name": n, "seconds": s, "note": x} for n, s, x in stages]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("images", nargs="+")
    ap.add_argument("--out", default="")
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--zahyou", default=gen_pipeline.DEFAULT_ZAHYOU)
    a = ap.parse_args()
    ns, _ = gen_pipeline.build_engine(a.zahyou)
    res = []
    for p in a.images:
        r = bench_one(ns, p, a.repeat)
        res.append(r)
        print(f"{p}  ({r['width']} x {r['height']})")
        for s in r["stages"]:
            print(f"  {s['seconds']:8.3f} 秒  {s['name']}  {s['note']}")
    if a.out:
        with io.open(a.out, "w", encoding="utf-8") as f:
            json.dump({"python": sys.version.split()[0], "images": res}, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()

"""
本物の WSL + solve-field を相手に、オフライン解析を端から端まで通すテスト。

同じ実写画像をいろいろな形式に変換し、どれでも同じ空の場所に解けることを確かめる。
WSL が入っていない PC では、はっきり «WSL なし» と出して終わる。

  python test_wsl_e2e.py
"""
from __future__ import annotations

import os
import sys
import time
import warnings

warnings.filterwarnings("ignore")
import numpy as np
from astropy.coordinates import SkyCoord
from astropy.io import fits
from astropy.wcs import WCS
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("ZAHYOU_CACHE", os.path.join(HERE, "tcase", "e2e_cache.json"))
os.makedirs(os.path.join(HERE, "tcase"), exist_ok=True)
sys.path.insert(0, HERE)

import zahyou_core as zc                                    # noqa: E402
from zahyou_plot import normalize_wcs_header                # noqa: E402

SRC = r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits"
CASES_DIR = os.path.join(HERE, "realcase")

# nova.astrometry.net が返した基準解 (§4.1 参照)
TRUE_CENTER = SkyCoord(ra=61.891414, dec=41.964760, unit="deg")
TRUE_SCALE = 1.4616          # arcsec/px
TOL_ARCSEC = 20.0            # 中心位置の許容ずれ
TOL_SCALE = 0.02             # スケールの許容相対差


def make_cases():
    """実写画像から、いろいろな形式の派生を作る。"""
    os.makedirs(CASES_DIR, exist_ok=True)
    d = fits.getdata(SRC).astype(np.float32)
    hdr = fits.getheader(SRC)
    lo, hi = np.percentile(d, [10, 99.95])
    n8 = (np.clip((d - lo) / max(hi - lo, 1e-6), 0, 1) * 255).astype(np.uint8)

    def path(name):
        return os.path.join(CASES_DIR, name)

    Image.fromarray(n8).save(path("real.png"))
    Image.fromarray(n8).convert("RGB").save(path("real.jpg"), quality=85)
    fits.PrimaryHDU((d * 257).astype(np.uint16),
                    header=hdr).writeto(path("real16.fits"), overwrite=True)
    fits.PrimaryHDU(np.stack([d * 0.9, d, d * 0.8]).astype(np.float32)) \
        .writeto(path("real_color.fits"), overwrite=True)
    rng = np.random.default_rng(1)
    cube = np.stack([d + rng.normal(0, 18, d.shape) for _ in range(20)])
    fits.PrimaryHDU(cube.astype(np.float32)).writeto(path("real_cube.fits"),
                                                     overwrite=True)
    fits.PrimaryHDU(d[::-1].astype(np.float32)).writeto(path("real_flip.fits"),
                                                        overwrite=True)


CASES = [
    ("実写 FITS 8bit (原本)",              SRC,                     True),
    ("16bit FITS (BZERO 付き)",            "real16.fits",           True),
    ("8bit PNG に変換したもの",             "real.png",              True),
    ("JPEG (非可逆圧縮)",                   "real.jpg",              True),
    ("カラー FITS (3, ny, nx)",            "real_color.fits",       True),
    ("動画キューブ 20 枚",                  "real_cube.fits",        True),
    ("上下反転した FITS",                   "real_flip.fits",        False),
]


def solve_one(path, timeout=180):
    """オフライン経路だけで解く。戻り値 (WCS or None, 秒)"""
    import tempfile
    import shutil
    t0 = time.time()
    bundle = zc.load_image_any(path)
    sub, mask, sigma, _info = zc.preprocess(bundle.data)
    sources, _thr = zc.detect_sources(sub, mask, sigma)
    diag = zc.solver_diagnostics()
    if not diag["ok"]:
        return None, 0.0, len(sources), diag
    work = tempfile.mkdtemp(prefix="zahyou_e2e_")
    try:
        hdr = zc.solve_offline(bundle, sources, work, timeout=timeout,
                               hint_radec=None, verbose=False, diagnostics=diag)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    if hdr is None:
        return None, time.time() - t0, len(sources), diag
    return (WCS(normalize_wcs_header(hdr, bundle.shape)), bundle.shape), \
        time.time() - t0, len(sources), diag


def main():
    if not zc.wsl_available():
        print("WSL (または solve-field) がありません。このテストは実行できません。")
        return 2
    if not os.path.exists(SRC):
        print(f"テスト画像がありません: {SRC}")
        return 2
    make_cases()

    diag = zc.solver_diagnostics()
    lo, hi = zc.index_coverage(diag["indexes"])
    print(f"solve-field OK / index {len(diag['indexes'])} 個 / 対応画角 {lo}′〜{hi}′")
    print(f"参照先: {[p for p in diag['paths'] if 'Astrometry' in p or 'astrometry' in p]}")
    print()

    rows, ok_all = [], True
    for label, name, same_parity in CASES:
        path = name if os.path.isabs(name) else os.path.join(CASES_DIR, name)
        if not os.path.exists(path):
            rows.append((label, False, "ファイルなし"))
            ok_all = False
            continue
        try:
            got, el, nsrc, _d = solve_one(path)
        except Exception as e:
            rows.append((label, False, f"{type(e).__name__}: {e}"))
            ok_all = False
            continue
        if got is None:
            rows.append((label, False, f"解けなかった ({el:.0f} 秒 / 星 {nsrc} 個)"))
            ok_all = False
            continue
        wcs, shape = got
        ny, nx = shape
        from astropy.wcs.utils import proj_plane_pixel_scales
        scale = float(np.mean(proj_plane_pixel_scales(wcs.celestial)) * 3600.0)
        center = wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0).icrs
        sep = TRUE_CENTER.separation(center).arcsec
        dscale = abs(scale - TRUE_SCALE) / TRUE_SCALE
        good = sep < TOL_ARCSEC and dscale < TOL_SCALE
        ok_all &= good
        rows.append((label, good,
                     f"{el:5.1f} 秒 / 星 {nsrc:3d} 個 / 中心ずれ {sep:5.1f}″ / "
                     f"{scale:.4f}″/px"))

    w = max(len(r[0]) for r in rows)
    n_ok = 0
    for label, good, msg in rows:
        n_ok += bool(good)
        print(f"  {'PASS' if good else 'FAIL'}  {label.ljust(w)}  {msg}")
    print(f"\n  {n_ok}/{len(rows)} passed")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())

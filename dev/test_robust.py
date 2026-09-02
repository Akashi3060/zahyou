"""どんな画像でも「星のリストが作れる」ことを確認するテスト。"""
import os
import sys
import warnings

warnings.filterwarnings("ignore")
import numpy as np
from astropy.io import fits
from PIL import Image

import zahyou_core as zc

rng = np.random.default_rng(20260831)
TMP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tcase")
os.makedirs(TMP, exist_ok=True)

RESULTS = []


def make_field(ny=600, nx=900, n=45, bg=400.0, noise=12.0, peak=(300, 9000),
               fwhm=3.0, seed=None):
    """星像を植えた素の画像と、植えた位置を返す。"""
    r = np.random.default_rng(seed) if seed is not None else rng
    img = np.full((ny, nx), bg, dtype=np.float64)
    img += r.normal(0, noise, img.shape)
    sig = fwhm / 2.3548
    xs = r.uniform(30, nx - 30, n)
    ys = r.uniform(30, ny - 30, n)
    amps = r.uniform(*peak, n)
    yy, xx = np.mgrid[0:ny, 0:nx]
    for x, y, a in zip(xs, ys, amps):
        sl = (slice(max(0, int(y) - 12), min(ny, int(y) + 13)),
              slice(max(0, int(x) - 12), min(nx, int(x) + 13)))
        img[sl] += a * np.exp(-(((xx[sl] - x) ** 2 + (yy[sl] - y) ** 2) /
                                (2 * sig ** 2)))
    return img, np.column_stack([xs, ys])


def burn_timestamp(img, where="top"):
    """SharpCap 風の焼き込み文字を模す (飽和した細い文字の列)。"""
    ny, nx = img.shape
    val = float(np.max(img)) * 1.05
    rows = range(4, 16) if where == "top" else range(ny - 16, ny - 4)
    x = 6
    for _ in range(26):                       # 26 文字ぶん
        for dx in (0, 1, 4, 5):
            for y in rows:
                if rng.random() < 0.65 and x + dx < nx:
                    img[y, x + dx] = val
        x += 9
    return img


def check(name, path, expect_sources=8, planted=None, tol=2.0, expect_fail=False):
    try:
        b = zc.load_image_any(path)
        sub, mask, sigma, info = zc.preprocess(b.data)
        src, thr = zc.detect_sources(sub, mask, sigma)
    except Exception as e:
        ok = expect_fail
        RESULTS.append((name, ok, f"{type(e).__name__}: {e}"))
        return None
    if expect_fail:
        RESULTS.append((name, False, "例外になるはずが通ってしまった"))
        return None

    msg = f"{len(src):4d} sources thr={thr}  [{b.note[:58]}]"
    ok = len(src) >= expect_sources
    if planted is not None and src:
        det = np.array([[s["x"], s["y"]] for s in src])
        d = np.linalg.norm(det[:, None, :] - planted[None, :, :], axis=2)
        matched = int((d.min(axis=1) < tol).sum())
        frac = matched / len(src)
        msg += f"  一致 {matched}/{len(src)} ({frac:.0%})"
        ok = ok and frac >= 0.80
    RESULTS.append((name, ok, msg))
    return b


# --------------------------------------------------------------- ケース群 ---
def case_real():
    p = r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits"
    if os.path.exists(p):
        check("01 実データ SharpCap 8bit + 焼き込み時刻", p, expect_sources=10)


def case_basic16():
    img, pl = make_field()
    p = os.path.join(TMP, "basic16.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("02 16bit 素直な星野", p, 30, pl)


def case_int16_bzero():
    img, pl = make_field(bg=33000, peak=(400, 9000))
    p = os.path.join(TMP, "int16_bzero.fits")
    h = fits.PrimaryHDU((img - 32768).astype(np.int16))
    h.header["BZERO"] = 32768
    h.header["BSCALE"] = 1
    h.writeto(p, overwrite=True)
    check("03 int16 + BZERO=32768 (符号付き格納)", p, 30, pl)


def case_float_nan():
    img, pl = make_field()
    img = img.astype(np.float32)
    img[100:140, 200:260] = np.nan
    img[0, :] = np.inf
    p = os.path.join(TMP, "float_nan.fits")
    fits.PrimaryHDU(img).writeto(p, overwrite=True)
    check("04 float32 + NaN/Inf", p, 25, pl)


def case_ext1():
    img, pl = make_field()
    p = os.path.join(TMP, "ext1.fits")
    fits.HDUList([fits.PrimaryHDU(),
                  fits.ImageHDU(img.astype(np.float32))]).writeto(p, overwrite=True)
    check("05 データが拡張 HDU[1] にある", p, 30, pl)


def case_compressed():
    img, pl = make_field()
    p = os.path.join(TMP, "comp.fits")
    fits.HDUList([fits.PrimaryHDU(),
                  fits.CompImageHDU(img.astype(np.int32))]).writeto(p, overwrite=True)
    check("06 タイル圧縮 FITS (CompImageHDU)", p, 30, pl)


def case_color_cube():
    img, pl = make_field()
    cube = np.stack([img * 0.9, img, img * 0.8]).astype(np.float32)
    p = os.path.join(TMP, "color.fits")
    fits.PrimaryHDU(cube).writeto(p, overwrite=True)
    check("07 カラー FITS (3, ny, nx)", p, 30, pl)


def case_video_cube():
    img, pl = make_field(noise=40.0, peak=(200, 3000))
    frames = np.stack([img + rng.normal(0, 40, img.shape) for _ in range(30)])
    p = os.path.join(TMP, "cube.fits")
    fits.PrimaryHDU(frames.astype(np.float32)).writeto(p, overwrite=True)
    check("08 動画キューブ (30, ny, nx) 平均で SNR 回復", p, 20, pl)


def case_4d():
    img, pl = make_field()
    p = os.path.join(TMP, "cube4d.fits")
    fits.PrimaryHDU(img.astype(np.float32)[None, None, :, :]).writeto(p, overwrite=True)
    check("09 4 次元 (1,1,ny,nx)", p, 30, pl)


def case_bayer():
    img, pl = make_field(peak=(800, 9000))
    mosaic = img.copy()
    mosaic[0::2, 0::2] *= 1.35      # R
    mosaic[1::2, 1::2] *= 0.65      # B
    p = os.path.join(TMP, "bayer.fits")
    h = fits.PrimaryHDU(mosaic.astype(np.uint16))
    h.header["BAYERPAT"] = "RGGB"
    h.writeto(p, overwrite=True)
    check("10 ベイヤー配列 (BAYERPAT=RGGB)", p, 25, pl, tol=2.5)


def case_overlay_top():
    img, pl = make_field()
    img = burn_timestamp(img, "top")
    p = os.path.join(TMP, "ov_top.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("11 上端に焼き込み時刻", p, 25, pl)


def case_overlay_bottom():
    img, pl = make_field()
    img = burn_timestamp(img, "bottom")
    p = os.path.join(TMP, "ov_bottom.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("12 下端に焼き込み時刻", p, 25, pl)


def case_gradient():
    img, pl = make_field()
    ny, nx = img.shape
    yy, xx = np.mgrid[0:ny, 0:nx]
    img = img + 2500 * (xx / nx) + 1800 * (yy / ny)          # 強いかぶり
    img *= (1 - 0.55 * (((xx - nx / 2) / nx) ** 2 + ((yy - ny / 2) / ny) ** 2) * 4)
    p = os.path.join(TMP, "grad.fits")
    fits.PrimaryHDU(img.astype(np.float32)).writeto(p, overwrite=True)
    check("13 強いかぶり + 周辺減光", p, 25, pl)


def case_trail_hotpix():
    img, pl = make_field()
    ny, nx = img.shape
    for t in range(700):                                    # 人工衛星の光跡
        y = int(80 + t * 0.55)
        x = int(60 + t * 1.1)
        if 0 <= y < ny and 0 <= x < nx:
            img[y - 1:y + 2, x - 1:x + 2] += 4000
    ys = rng.integers(0, ny, 250)
    xs = rng.integers(0, nx, 250)
    img[ys, xs] += 12000                                     # ホットピクセル
    p = os.path.join(TMP, "trail.fits")
    fits.PrimaryHDU(img.astype(np.float32)).writeto(p, overwrite=True)
    check("14 衛星の光跡 + ホットピクセル 250 個", p, 25, pl, tol=2.5)


def case_few_stars():
    img, pl = make_field(n=6)
    p = os.path.join(TMP, "few.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("15 星が 6 個しかない", p, 5, pl)


def case_dense():
    img, pl = make_field(n=1500, peak=(200, 6000))
    p = os.path.join(TMP, "dense.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    b = check("16 天の川級の密集 (1500 星)", p, 100)


def case_defocus():
    img, pl = make_field(fwhm=9.0, peak=(3000, 30000))
    p = os.path.join(TMP, "defocus.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("17 ピンボケ (FWHM 9px)", p, 25, pl, tol=3.0)


def case_undersampled():
    img, pl = make_field(fwhm=1.2, peak=(2000, 30000))
    p = os.path.join(TMP, "under.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("18 アンダーサンプル (FWHM 1.2px)", p, 25, pl)


def case_saturated():
    img, pl = make_field(peak=(60000, 90000))
    img = np.clip(img, 0, 65535)
    p = os.path.join(TMP, "sat.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("19 全部飽和した星", p, 25, pl, tol=2.5)


def case_vignette_black():
    img, pl = make_field()
    img[:12, :] = 0
    img[-12:, :] = 0
    img[:, :12] = 0
    img[:, -12:] = 0
    p = os.path.join(TMP, "vig.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("20 縁が真っ黒 (ケラレ)", p, 25, pl)


def case_png8():
    img, pl = make_field(bg=25, noise=3, peak=(60, 220))
    p = os.path.join(TMP, "gray8.png")
    Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).save(p)
    check("21 8bit グレースケール PNG", p, 25, pl)


def case_png16():
    img, pl = make_field()
    p = os.path.join(TMP, "gray16.png")
    Image.fromarray(np.clip(img, 0, 65535).astype(np.uint16)).save(p)
    check("22 16bit PNG", p, 25, pl)


def case_jpeg_rgb():
    img, pl = make_field(bg=22, noise=2.0, peak=(80, 230))
    rgb = np.dstack([np.clip(img, 0, 255).astype(np.uint8)] * 3)
    p = os.path.join(TMP, "color.jpg")
    Image.fromarray(rgb).save(p, quality=92)
    check("23 カラー JPEG", p, 25, pl, tol=2.5)


def case_tiff16():
    img, pl = make_field()
    p = os.path.join(TMP, "img16.tif")
    Image.fromarray(np.clip(img, 0, 65535).astype(np.uint16)).save(p)
    check("24 16bit TIFF", p, 25, pl)


def case_flat():
    p = os.path.join(TMP, "flat.fits")
    fits.PrimaryHDU(np.full((300, 300), 1000, dtype=np.uint16)).writeto(p, overwrite=True)
    check("25 一様な画像 (星なし) → エラーで知らせる", p, expect_fail=True)


def case_starless_noise():
    p = os.path.join(TMP, "noise.fits")
    fits.PrimaryHDU(rng.normal(500, 20, (400, 600)).astype(np.float32)).writeto(p, overwrite=True)
    b = zc.load_image_any(p)
    sub, mask, sigma, _ = zc.preprocess(b.data)
    src, thr = zc.detect_sources(sub, mask, sigma)
    ok = len(src) < 40      # ノイズだけの画像で大量に「星」を作らないこと
    RESULTS.append(("26 ノイズだけ (曇天) → 誤検出が暴走しない", ok,
                    f"{len(src)} sources thr={thr}"))


def case_tiny_but_valid():
    img, pl = make_field(ny=120, nx=160, n=14, seed=7)
    p = os.path.join(TMP, "tiny.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("27 小さい画像 (160x120)", p, 8, pl)


def case_portrait():
    img, pl = make_field(ny=1200, nx=400, n=60)
    p = os.path.join(TMP, "tall.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("28 縦長 (400x1200)", p, 35, pl)


def case_moon_glow():
    img, pl = make_field(bg=8000, noise=90, peak=(2000, 40000))
    ny, nx = img.shape
    yy, xx = np.mgrid[0:ny, 0:nx]
    img = img + 30000 * np.exp(-(((xx - 60) ** 2 + (yy - 60) ** 2) / (2 * 260 ** 2)))
    p = os.path.join(TMP, "moon.fits")
    fits.PrimaryHDU(np.clip(img, 0, 65535).astype(np.uint16)).writeto(p, overwrite=True)
    check("29 月明かりの大きなカブリ", p, 20, pl, tol=2.5)


def case_no_simple_card():
    """SIMPLE カードが壊れた FITS。FITS でも画像でもないので、
    黙って誤読せず、はっきりエラーにするのが正しい振る舞い。"""
    img, pl = make_field()
    p = os.path.join(TMP, "nosimple.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    with open(p, "r+b") as f:
        f.seek(0)
        f.write(b"XIMPLE  ")
    check("30 SIMPLE カードが壊れた FITS → 誤読せずエラー", p, expect_fail=True)


def case_spaces_in_name():
    img, pl = make_field()
    p = os.path.join(TMP, "名前に 空白 と日本語.fits")
    fits.PrimaryHDU(img.astype(np.uint16)).writeto(p, overwrite=True)
    check("31 空白・日本語入りファイル名", p, 25, pl)


def case_star_chart():
    """星図 (白地に黒い星)。明暗が逆なので、そのままでは 1 つも拾えない。"""
    img, pl = make_field()
    v = (img - img.min()) / max(float(np.ptp(img)), 1e-6)
    inv = ((1.0 - v) * 255.0).astype(np.uint8)          # 白地に黒い星
    p2 = os.path.join(TMP, "starchart.png")
    Image.fromarray(inv).save(p2)
    check("32 星図 (白地に黒い星)", p2, 25, pl)


def case_real_star_chart():
    """実物の星図 (Occult などが描くもの)。手元にあるときだけ。"""
    p2 = (r"C:\Users\yoshi\Downloads"
          r"\20261031 3200 Phaethon StarChart 40arcmin.png")
    if os.path.exists(p2):
        check("33 実物の星図 (40 分角)", p2, expect_sources=50)


def main():
    for fn in sorted(k for k in globals() if k.startswith("case_")):
        globals()[fn]()
    width = max(len(r[0]) for r in RESULTS)
    n_ok = 0
    print()
    for name, ok, msg in RESULTS:
        n_ok += bool(ok)
        print(f"  {'PASS' if ok else 'FAIL'}  {name.ljust(width)}  {msg}")
    print(f"\n  {n_ok}/{len(RESULTS)} passed")
    return 0 if n_ok == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())

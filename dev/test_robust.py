"""どんな画像でも「星のリストが作れる」ことを確認するテスト。

星のリストは 2 つ。detect_sources (図に緑の丸で出す星) と detect_sources_deep
(解析エンジンに渡す、暗い星まで拾った星)。各ケースで両方を確かめる。
34 番以降は深い検出だけのケース (暗い星・雑音だけ・十字のホットピクセル・位置の精度・実写)。

ここで書き出す tcase/*.fits の一部は test_solver_real.py の陰性の画像 (sha256 で照合) なので、
共通の乱数 rng を使う順番を変えないこと。新しいケースは自分の seed を使う。
"""
import os
import sys
import warnings

warnings.filterwarnings("ignore")
import numpy as np
from astropy.io import fits
from PIL import Image
from scipy import ndimage

import zahyou_core as zc

rng = np.random.default_rng(20260831)
TMP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tcase")
# 手元の実写 (あるときだけ走る)。test_solver_real.py と同じく ZAHYOU_TEST_IMAGES (無ければホームフォルダー) から
IMAGES = os.environ.get("ZAHYOU_TEST_IMAGES") or os.path.expanduser("~")
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


def match_planted(src, planted, tol):
    """(植えた星から tol 画素以内にある検出の数, 検出の数)。"""
    if not src:
        return 0, 0
    det = np.array([[s["x"], s["y"]] for s in src])
    d = np.linalg.norm(det[:, None, :] - planted[None, :, :], axis=2)
    return int((d.min(axis=1) < tol).sum()), len(src)


def check(name, path, expect_sources=8, planted=None, tol=2.0, expect_fail=False):
    """
    標準の検出で expect_sources 個以上・純度 80% 以上 (planted があるとき)。
    深い検出も同じ数・同じ純度を求める (解析エンジンに渡す星が図の星より劣らないこと)。
    """
    try:
        b = zc.load_image_any(path)
        sub, mask, sigma, info = zc.preprocess(b.data)
        src, thr = zc.detect_sources(sub, mask, sigma)
        deep, u = zc.detect_sources_deep(sub, mask, sigma)
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
        matched, n = match_planted(src, planted, tol)
        msg += f"  一致 {matched}/{n} ({matched / n:.0%})"
        ok = ok and matched / n >= 0.80
    msg += f"  | 深い {len(deep)} 個 u={u}"
    ok_deep = len(deep) >= expect_sources
    if planted is not None and deep:
        matched, n = match_planted(deep, planted, tol)
        msg += f" 一致 {matched}/{n} ({matched / n:.0%})"
        ok_deep = ok_deep and matched / n >= 0.80
    if not ok_deep:
        msg += "  ← 深い検出が基準に届かない"
    RESULTS.append((name, ok and ok_deep, msg))
    return b


# --------------------------------------------------------------- ケース群 ---
def case_real():
    p = os.path.join(IMAGES, "Downloads", "Capture_00001 00_10_33.fits")
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
    deep, u = zc.detect_sources_deep(sub, mask, sigma)
    # ノイズだけの画像で大量に「星」を作らないこと。深い検出は偽の極大が 2 個ほどに
    # なるしきい値を選ぶので、数個まで (ポアソン分布で 9 個以上になる見込みは 0.03%)
    ok = len(src) < 40 and len(deep) <= 8
    RESULTS.append(("26 ノイズだけ (曇天) → 誤検出が暴走しない", ok,
                    f"{len(src)} sources thr={thr}  | 深い {len(deep)} 個 u={u}"))


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
    p2 = os.path.join(IMAGES, "Downloads", "20261031 3200 Phaethon StarChart 40arcmin.png")
    if os.path.exists(p2):
        check("33 実物の星図 (40 分角)", p2, expect_sources=50)


# ---------------------------------------------------- 深い検出だけのケース ---
# 共通の rng は使わない (上の tcase/*.fits が変わらないように)。名前は既存のケースの後に並べる。

def _detect_both(img):
    sub, mask, sigma, _ = zc.preprocess(np.asarray(img, dtype=np.float64))
    std, _ = zc.detect_sources(sub, mask, sigma)
    info = {}
    deep, u = zc.detect_sources_deep(sub, mask, sigma, info=info)
    return std, deep, info


def case_zz34_deep_faint():
    """暗い星 (頂が雑音の 2〜6 倍) だけの星野。深い検出は標準より多く、純度を保って拾う。"""
    img, pl = make_field(n=60, noise=12.0, peak=(25, 70), fwhm=3.5, seed=34)
    std, deep, info = _detect_both(img)
    ms, ns = match_planted(std, pl, 2.0)
    md, nd = match_planted(deep, pl, 2.0)
    ok = md >= 50 and md >= ms + 15 and nd and md / nd >= 0.90
    RESULTS.append(("34 暗い星だけ (60 個) → 深い検出が標準より多く拾う", ok,
                    f"標準 {ms}/{ns}  深い {md}/{nd} (植えた星と一致/検出、u={info['u']}、"
                    f"星の σ {info['s_psf'] and round(info['s_psf'], 2)} px)"))


def case_zz35_deep_noise():
    """雑音だけの画像 4 種。隣どうしがそろった雑音 (ベイヤーの補間・縮小の後) でも偽の星は数個まで。"""
    r = np.random.default_rng(35)
    imgs = (("白色 900x600", r.normal(500, 20, (600, 900))),
            ("白色 1608x1104", r.normal(500, 20, (1104, 1608))),
            ("3x3 平均", ndimage.uniform_filter(r.normal(500, 20, (600, 900)), 3)),
            ("σ=1 ぼかし", ndimage.gaussian_filter(r.normal(500, 20, (600, 900)), 1.0)))
    ok, counts = True, []
    for label, img in imgs:
        std, deep, info = _detect_both(img)
        ok = ok and len(deep) <= 8
        counts.append(f"{label}: 深い {len(deep)} (参考: 標準 {len(std)})")
    RESULTS.append(("35 雑音だけ 4 種 → 深い検出の偽の星は 8 個以下", ok, " / ".join(counts)))


def case_zz36_deep_hot_cross():
    """
    ベイヤーの補間で十字に広がったホットピクセル (中心と上下左右)。前処理の 1 画素だけの
    ホットピクセル消しにはかからないので、深い検出は「鋭さ」で捨てる。
    (標準の検出はこれを星として拾う。図の星はこのケースの対象外)
    """
    img, pl = make_field(seed=36)
    r = np.random.default_rng(1036)
    ny, nx = img.shape
    for y, x in zip(r.integers(3, ny - 3, 300), r.integers(3, nx - 3, 300)):
        img[y, x] += 12000
        img[y - 1, x] += 3000
        img[y + 1, x] += 3000
        img[y, x - 1] += 3000
        img[y, x + 1] += 3000
    std, deep, info = _detect_both(img)
    ms, ns = match_planted(std, pl, 2.0)
    md, nd = match_planted(deep, pl, 2.0)
    ok = md >= 35 and nd and md / nd >= 0.80
    RESULTS.append(("36 十字のホットピクセル 300 個 → 深い検出は拾わない", ok,
                    f"深い {md}/{nd} (一致/検出、鋭さで捨てた {info['rejected']['sharp']} 個)  "
                    f"参考: 標準 {ms}/{ns}"))


def case_zz37_deep_position():
    """深い検出の位置の精度。植えた星 (FWHM 3 px) との差の中央値が 0.05 px 以下。"""
    errs = []
    for seed in (37, 137):
        img, pl = make_field(seed=seed)
        _, deep, _ = _detect_both(img)
        xy = np.array([[s["x"], s["y"]] for s in deep])
        d = np.linalg.norm(xy[:, None, :] - pl[None, :, :], axis=2)
        errs += list(d.min(axis=1)[d.min(axis=1) < 2.0])
    med = float(np.median(errs)) if errs else float("inf")
    ok = len(errs) >= 70 and med <= 0.05
    RESULTS.append(("37 深い検出の位置の精度 (FWHM 3 px)", ok,
                    f"一致 {len(errs)} 個、植えた位置との差 中央値 {med:.3f} px・"
                    f"90% 点 {float(np.percentile(errs, 90)) if errs else float('inf'):.3f} px"))


def case_zz38_deep_real_few_stars():
    """実写の 3I (星の少ない画像)。標準の検出は 8 個。深い検出で 12 個以上。手元にあるときだけ。"""
    p = os.path.join(IMAGES, "Downloads", "02013I_00010 20_47_25.fits")
    if not os.path.exists(p):
        return
    b = zc.load_image_any(p)
    std, deep, info = _detect_both(b.data)
    ok = len(deep) >= 12 and len(deep) >= len(std) + 4
    RESULTS.append(("38 実写 3I (星が少ない) → 深い検出で 12 個以上", ok,
                    f"標準 {len(std)} 個  深い {len(deep)} 個 (u={info['u']}、星の σ "
                    f"{info['s_psf'] and round(info['s_psf'], 2)} px、{info['seconds']:.2f} 秒)"))


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

# -*- coding: utf-8 -*-
"""
zahyou-image(Rust)の照合用の値を Python 基準実装(dev/zahyou_core.py)で作る。

    python gen_image.py            # image_kernels.json / image_formats.json / image_pipeline.json を書く
    python gen_image.py --big DIR  # 6000x4000 の合成画像(critic の prof_big.py と同じ作り方)を DIR に作り、
                                   # Python の結果と時間を DIR\\big_expected.json に書く(リポジトリには入れない)

Python 基準実装は既定でこのリポジトリの dev/zahyou_core.py(環境変数 ZAHYOU_DEV で別の dev フォルダーにできる)。

書くもの(このスクリプトと同じフォルダー):
  image_kernels.json   numpy / scipy / astropy の部品(和・中央値・σ クリップ・フィルター・zoom・label)の値
  image_formats.json   小さな合成ファイル(FITS の型・圧縮・PNG/TIFF/JPEG)を gzip+base64 で持ち、load_image_any の結果を書く
  image_pipeline.json  dev/tcase と手元の実写について、読み込み・前処理・検出の結果を書く(画像そのものは入れない)。
                       画像の path は dev/solver_truth.json と同じ決まり: リポジトリからの相対(dev/tcase は
                       test_robust.py が作る)か、$ZAHYOU_TEST_IMAGES/(環境変数。無ければホームフォルダー)からの相対。
                       dev/tcase がリポジトリに無ければ $ZAHYOU_DEV/tcase、隣の Python 版のリポジトリ ../zahyou/dev/tcase の順に探す

照合の約束(Rust の試験 crates/zahyou-image/tests/{kernels,formats,pipeline}.rs と同じ):
  - ハッシュ ahash(a): float32 は NaN を 0x7fc00000 にそろえた 32 bit の値 b_i(bool・uint8 は 0/1・値)について
    Σ (b_i + 1) * (i * 0x9E3779B97F4A7C15 + 0x632BE59BD9B4E019)  (mod 2^64)。16 進 16 桁の文字列
  - 星は明るい順の上位 50 個の (x, y, flux, peak, area, elong, fwhm, snr)。深い検出は全部の星の (x, y) も持つ(xy)。
    深い検出は info(s_psf・s・u・雑音・候補の数・捨てた理由ごとの数・鋭さの上限。秒は除く)も持つ
"""
import base64
import gzip
import io
import json
import os
import sys
import time
import warnings

import numpy as np

warnings.simplefilter("ignore")

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
ZAHYOU_DEV = os.environ.get("ZAHYOU_DEV") or os.path.join(REPO, "dev")
sys.path.insert(0, ZAHYOU_DEV)
IMAGES_VAR = "$ZAHYOU_TEST_IMAGES/"
import zahyou_core as zc                                  # noqa: E402

from astropy.io import fits                               # noqa: E402
from astropy.stats import sigma_clipped_stats             # noqa: E402
from scipy import ndimage                                 # noqa: E402

M64 = (1 << 64) - 1
K1 = np.uint64(0x9E3779B97F4A7C15)
K2 = np.uint64(0x632BE59BD9B4E019)


def ahash(a):
    a = np.asarray(a)
    if a.dtype == np.bool_ or a.dtype == np.uint8:
        b = a.ravel().astype(np.uint64)
    else:
        f = np.ascontiguousarray(a, dtype=np.float32).ravel().copy()
        bits = f.view(np.uint32).copy()
        bits[np.isnan(f)] = 0x7FC00000
        b = bits.astype(np.uint64)
    with np.errstate(over="ignore"):
        i = np.arange(b.size, dtype=np.uint64)
        h = ((b + np.uint64(1)) * (i * K1 + K2)).sum(dtype=np.uint64)
    return f"{int(h) & M64:016x}"


def bytes_hash(raw):
    return ahash(np.frombuffer(raw, dtype=np.uint8))


def synth(n, seed, center=500.0, spread=40.0, outlier_every=0):
    """Rust と同じ手順で作る擬似乱数の float32 列(大きな配列を JSON に入れないため)。
    u = mix32(i * 0x9E3779B1 + seed * 0x85EBCA77)、値 = center + spread * (u / 2^32 - 0.5)。
    outlier_every > 0 なら u % outlier_every == 0 の要素に (u % 5000) を足す。"""
    i = np.arange(n, dtype=np.uint64)
    u = (i * np.uint64(0x9E3779B1) + np.uint64(seed) * np.uint64(0x85EBCA77)) & np.uint64(0xFFFFFFFF)
    u ^= u >> np.uint64(15)
    u = (u * np.uint64(0x2C1B3C6D)) & np.uint64(0xFFFFFFFF)
    u ^= u >> np.uint64(12)
    u = (u * np.uint64(0x297A2D39)) & np.uint64(0xFFFFFFFF)
    u ^= u >> np.uint64(15)
    v = center + spread * (u.astype(np.float64) / 4294967296.0 - 0.5)
    if outlier_every:
        hit = (u % np.uint64(outlier_every)) == 0
        v = v + np.where(hit, (u % np.uint64(5000)).astype(np.float64), 0.0)
    return v.astype(np.float32)


def f32_b64(a):
    return base64.b64encode(np.ascontiguousarray(a, dtype="<f4").tobytes()).decode()


def stats(a):
    a = np.asarray(a, dtype=np.float32)
    fin = np.isfinite(a)
    v = a[fin].astype(np.float64)
    return {
        "shape": list(a.shape),
        "hash": ahash(a),
        "sum": float(v.sum()) if v.size else 0.0,
        "mean": float(v.mean()) if v.size else 0.0,
        "min": float(v.min()) if v.size else 0.0,
        "max": float(v.max()) if v.size else 0.0,
        "n_nonfinite": int((~fin).sum()),
    }


def header_list(h):
    out = []
    for c in h.cards:
        k = c.keyword
        if k in ("", "COMMENT", "HISTORY"):
            continue
        v = c.value
        if isinstance(v, bool):
            out.append([k, "bool", bool(v)])
        elif isinstance(v, (int, np.integer)):
            out.append([k, "num", float(v)])
        elif isinstance(v, (float, np.floating)):
            out.append([k, "num", float(v)])
        elif isinstance(v, str):
            out.append([k, "str", v])
        else:
            out.append([k, "other", str(v)])
    return out


# =========================================================== 1. 部品 (kernels) ===

def gen_kernels():
    rng = np.random.default_rng(20261015)
    out = {}

    # numpy の float32 の和(ペアワイズ)と平均・std
    sums = []
    for n in (1, 2, 7, 8, 9, 15, 16, 17, 127, 128, 129, 130, 255, 256, 1000, 4097, 100003, 1234567):
        a = synth(n, n, 1000.0, 120.0)
        sums.append({"n": n, "seed": n, "center": 1000.0, "spread": 120.0, "sum": float(np.sum(a)),
                     "mean": float(np.mean(a)), "std": float(np.std(a)), "median": float(np.median(a))})
    out["sum_f32"] = sums

    # σ クリップ(sigma_clipped_stats sigma=3)
    clips = []
    for n, kind in ((5, "n"), (8, "n"), (599, "n"), (600, "n"), (601, "o"), (1000, "o"),
                    (777, "nan"), (2000, "o"), (3, "const")):
        a = rng.normal(500.0, 12.0, n).astype(np.float32)
        if kind == "o":
            idx = rng.choice(n, max(1, n // 20), replace=False)
            a[idx] += rng.uniform(100, 5000, idx.size).astype(np.float32)
        if kind == "nan":
            a[::17] = np.nan
            a[5] = np.inf
        if kind == "const":
            a[:] = 7.0
        mean, med, std = sigma_clipped_stats(a, sigma=3.0)
        clips.append({"x": f32_b64(a), "mean": float(mean), "median": float(med), "std": float(std)})
    for n, oe in ((200001, 23), (1000000, 7), (999999, 0)):
        a = synth(n, 77 + n, 500.0, 40.0, oe)
        mean, med, std = sigma_clipped_stats(a, sigma=3.0)
        clips.append({"n": n, "seed": 77 + n, "center": 500.0, "spread": 40.0, "outlier_every": oe,
                      "mean": float(mean), "median": float(med), "std": float(std)})
    out["sigma_clip"] = clips

    # 2 次元のフィルター
    def arr2(ny, nx, scale=30.0):
        return rng.normal(100.0, scale, (ny, nx)).astype(np.float32)

    filt = []
    for (ny, nx) in ((13, 17), (1, 9), (2, 3), (4, 5), (30, 1), (9, 40)):
        a = arr2(ny, nx)
        e = {"ny": ny, "nx": nx, "x": f32_b64(a),
             "median3": f32_b64(ndimage.median_filter(a, size=3, mode="nearest")),
             "uniform3": f32_b64(ndimage.uniform_filter(a, size=3, mode="nearest")),
             "gauss1": f32_b64(ndimage.gaussian_filter(a, sigma=1.0)),
             "binom3": None}
        k = np.array([0.25, 0.5, 0.25], dtype=np.float32)
        b = ndimage.correlate1d(a, k, axis=0, mode="nearest")
        b = ndimage.correlate1d(b, k, axis=1, mode="nearest")
        e["binom3"] = f32_b64(b)
        filt.append(e)
    out["filters"] = filt

    # zoom(order=1)
    zooms = []
    for (by, bx, ny, nx) in ((3, 4, 200, 300), (9, 14, 600, 900), (18, 6, 1200, 400), (3, 3, 191, 250),
                             (7, 5, 450, 333)):
        c = arr2(by, bx)
        z = ndimage.zoom(c, (ny / by, nx / bx), order=1)
        zooms.append({"by": by, "bx": bx, "ny": ny, "nx": nx, "x": f32_b64(c),
                      "out_shape": list(z.shape), "hash": ahash(z), "first": [float(v) for v in z.ravel()[:50]]})
    out["zoom"] = zooms

    # 背景(_background)
    backs = []
    for (ny, nx) in ((200, 300), (191, 250), (130, 700)):
        a = synth(ny * nx, ny + nx, 400.0, 60.0, 31).reshape(ny, nx)
        bk = zc._background(a)
        backs.append({"ny": ny, "nx": nx, "seed": ny + nx, "center": 400.0, "spread": 60.0, "outlier_every": 31,
                      "hash": ahash(bk), "sum": float(bk.astype(np.float64).sum())})
    out["background"] = backs

    # label(4 連結)
    labs = []
    for p in (0.5, 0.8):
        b = rng.random((23, 31)) > p
        lab, n = ndimage.label(b)
        labs.append({"ny": 23, "nx": 31, "bits": "".join("1" if v else "0" for v in b.ravel()),
                     "n": int(n), "labels": [int(v) for v in lab.ravel()]})
    out["label"] = labs

    # 輝度の重み
    w = zc._COLOR_WEIGHTS / zc._COLOR_WEIGHTS.sum()
    out["lum_weights"] = [float(v) for v in w]
    return out


# ====================================================== 2. 形式 (fixtures) ===

def make_field(ny, nx, seed, bg=400.0, noise=8.0, nstar=6, peak=(300, 4000)):
    r = np.random.default_rng(seed)
    img = r.normal(bg, noise, (ny, nx))
    yy, xx = np.mgrid[0:ny, 0:nx]
    for _ in range(nstar):
        x0, y0 = r.uniform(3, nx - 4), r.uniform(3, ny - 4)
        f = r.uniform(*peak)
        img += f * np.exp(-((xx - x0) ** 2 + (yy - y0) ** 2) / (2 * 1.2 ** 2))
    return img


def fits_bytes(hdus):
    b = io.BytesIO()
    fits.HDUList(hdus).writeto(b, output_verify="silentfix")
    return b.getvalue()


def raw_header(cards):
    """手で書くヘッダー(80 文字 × n、END、2880 の倍数まで空白)"""
    s = "".join(c.ljust(80)[:80] for c in cards) + "END".ljust(80)
    s += " " * ((-len(s)) % 2880)
    return s.encode("ascii")


def pad(b):
    return b + b"\0" * ((-len(b)) % 2880)


def gen_fixtures():
    fx = []

    def add(name, data_bytes, ext):
        fx.append((name, data_bytes, ext))

    ny, nx = 32, 40
    base = make_field(ny, nx, 1)

    # ---- 整数・実数の型と BZERO/BSCALE/BLANK
    add("u8", fits_bytes([fits.PrimaryHDU(np.clip(base / 20, 0, 255).astype(np.uint8))]), ".fits")
    add("i16", fits_bytes([fits.PrimaryHDU((base - 600).astype(np.int16))]), ".fits")
    add("u16", fits_bytes([fits.PrimaryHDU((base * 8).astype(np.uint16))]), ".fits")
    add("i32", fits_bytes([fits.PrimaryHDU((base * 1000 - 400000).astype(np.int32))]), ".fits")
    add("u32", fits_bytes([fits.PrimaryHDU((base * 1000).astype(np.uint32))]), ".fits")
    add("i64", fits_bytes([fits.PrimaryHDU((base * 100000).astype(np.int64))]), ".fits")
    f32 = base.astype(np.float32)
    f32[3, 4] = np.nan
    f32[10, 11:14] = np.inf
    f32[20, 5] = -np.inf
    add("f32_nan", fits_bytes([fits.PrimaryHDU(f32)]), ".fits")
    add("f64", fits_bytes([fits.PrimaryHDU(base.astype(np.float64) * 1.000001)]), ".fits")
    add("i8", fits_bytes([fits.PrimaryHDU(np.clip(base / 20 - 60, -128, 127).astype(np.int8))]), ".fits")
    # 手で書く: BITPIX 16 + BSCALE 0.5 + BZERO 100
    raw = (base * 2).astype(">i2")
    add("i16_scaled", raw_header([
        "SIMPLE  =                    T", "BITPIX  =                   16", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "BSCALE  =                  0.5", "BZERO   =                100.0",
    ]) + pad(raw.tobytes()), ".fits")
    raw = (base * 3).astype(">i4")
    add("i32_scaled", raw_header([
        "SIMPLE  =                    T", "BITPIX  =                   32", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "BSCALE  =                  2.5", "BZERO   =                 -7.0",
    ]) + pad(raw.tobytes()), ".fits")
    raw = (base - 600).astype(">i2")
    raw[5, 6] = -32768
    raw[7, 0:3] = -32768
    add("i16_blank", raw_header([
        "SIMPLE  =                    T", "BITPIX  =                   16", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "BLANK   =               -32768",
    ]) + pad(raw.tobytes()), ".fits")
    raw = (base * 8 - 32768).astype(">i2")
    raw[2, 2] = 100
    add("u16_blank", raw_header([
        "SIMPLE  =                    T", "BITPIX  =                   16", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "BZERO   =                32768", "BSCALE  =                    1",
        "BLANK   =                  100",
    ]) + pad(raw.tobytes()), ".fits")
    raw = (base / 7).astype(">f4")
    add("f32_scaled", raw_header([
        "SIMPLE  =                    T", "BITPIX  =                  -32", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "BSCALE  =                  1.7", "BZERO   =                 0.25",
    ]) + pad(raw.tobytes()), ".fits")
    raw = base.astype(">f4")
    raw[4, 4] = 77.0
    add("f32_blank_ignored", raw_header([
        "SIMPLE  =                    T", "BITPIX  =                  -32", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "BLANK   =                   77",
    ]) + pad(raw.tobytes()), ".fits")

    # ---- HDU の選び方
    add("ext1", fits_bytes([fits.PrimaryHDU(), fits.ImageHDU(base.astype(np.float32))]), ".fits")
    add("ext_largest", fits_bytes([fits.PrimaryHDU(np.ones((20, 20), np.int16) * np.arange(20, dtype=np.int16)),
                                   fits.ImageHDU(base[:20, :30].astype(np.float32)),
                                   fits.ImageHDU(base.astype(np.int16)),
                                   fits.ImageHDU(base[:16, :16].astype(np.float32))]), ".fits")
    tbl = fits.BinTableHDU.from_columns([fits.Column(name="a", format="J", array=np.arange(5))])
    add("ext_after_table", fits_bytes([fits.PrimaryHDU(), tbl, fits.ImageHDU(base.astype(np.uint16))]), ".fits")

    # ---- キューブ・カラー
    add("color_first", fits_bytes([fits.PrimaryHDU(np.stack([base * 0.9, base, base * 0.8]).astype(np.float32))]),
        ".fits")
    add("color_last", fits_bytes([fits.PrimaryHDU(np.stack([base, base * 1.1, base * 0.7], axis=-1).astype(np.int16))]),
        ".fits")
    add("rgba_first", fits_bytes([fits.PrimaryHDU(np.stack([base, base * 0.5, base * 0.3, base * 0 + 9]).astype(np.uint8))]),
        ".fits")
    r = np.random.default_rng(5)
    small = make_field(20, 24, 2)
    add("video5", fits_bytes([fits.PrimaryHDU(np.stack([small + r.normal(0, 30, small.shape) for _ in range(5)])
                                              .astype(np.int16))]), ".fits")
    v = (np.arange(16 * 16).reshape(16, 16) % 31).astype(np.uint8)
    add("video205", fits_bytes([fits.PrimaryHDU(np.stack([v * (1 + i % 3) + (i % 7) for i in range(205)])
                                                .astype(np.uint8))]),
        ".fits")
    add("cube4d_1", fits_bytes([fits.PrimaryHDU(base.astype(np.float32)[None, None])]), ".fits")
    add("cube4d_2x3", fits_bytes([fits.PrimaryHDU(np.stack([np.stack([base, base * 2, base * 3])] * 2)
                                                  .astype(np.float32)[:, :, :20, :24])]), ".fits")
    add("cube4d_2x1", fits_bytes([fits.PrimaryHDU(np.stack([base, base * 2])[:, None].astype(np.float32))]), ".fits")

    # ---- 上下・ベイヤー・ヘッダーの書き方
    h = fits.PrimaryHDU(base.astype(np.uint16))
    h.header["ROWORDER"] = "TOP-DOWN"
    add("roworder_top", fits_bytes([h]), ".fits")
    mos = base.copy()
    mos[0::2, 0::2] *= 1.35
    mos[1::2, 1::2] *= 0.65
    for nm, key, val in (("bayer_rggb", "BAYERPAT", "RGGB"), ("bayer_colortyp", "COLORTYP", "grbg "),
                         ("bayer_xbayroff", "XBAYROFF", 0), ("bayer_bad", "BAYERPAT", "XYZW")):
        h = fits.PrimaryHDU(mos.astype(np.uint16))
        h.header[key] = val
        add(nm, fits_bytes([h]), ".fits")
    h = fits.PrimaryHDU(mos.astype(np.float32))
    h.header["BAYERPAT"] = "BGGR"
    d = h.data.copy()
    d[6, 7] = np.nan
    h.data = d
    add("bayer_nan", fits_bytes([h]), ".fits")
    h = fits.PrimaryHDU(base.astype(np.int16))
    h.header["HIERARCH ESO DET CHIP NAME"] = "chip-1"
    h.header["LONGSTR"] = "x" * 100 + " end"
    h.header["QUOTE"] = "it's"
    h.header["FLOATD"] = (1.5e-3, "comment / with slash")
    h.header["NEGINT"] = -42
    h.header["BOOLF"] = False
    h.header["EMPTYS"] = ""
    h.header["COMMENT"] = "a comment"
    h.header["HISTORY"] = "a history"
    add("header_kinds", fits_bytes([h]), ".fits")
    hb = raw_header([
        "SIMPLE  =                    T", "BITPIX  =                   16", "NAXIS   =                    2",
        f"NAXIS1  = {nx:20d}", f"NAXIS2  = {ny:20d}", "EXPTIME =               1.5D+1 / D exponent",
        "LOWER   = 'abc     '           / string with trailing blanks", "UNDEF   =",
    ])
    add("header_dexp", hb + pad((base - 600).astype(">i2").tobytes()), ".fits")

    # ---- gzip・特殊ケース
    add("gz_u16", gzip.compress(fits_bytes([fits.PrimaryHDU((base * 8).astype(np.uint16))]), mtime=0), ".fits.gz")
    add("uniform", fits_bytes([fits.PrimaryHDU(np.full((32, 40), 1000, np.uint16))]), ".fits")
    add("too_small", fits_bytes([fits.PrimaryHDU(base[:10].astype(np.uint16))]), ".fits")
    b = bytearray(fits_bytes([fits.PrimaryHDU(base.astype(np.uint16))]))
    b[0:8] = b"XIMPLE  "
    add("nosimple", bytes(b), ".fits")
    inv = (base.max() + 10 - base).astype(np.float32)
    add("inverted", fits_bytes([fits.PrimaryHDU(inv)]), ".fits")
    nanimg = base.astype(np.float32)
    nanimg[10:14, 10:20] = np.nan
    add("nan_block", fits_bytes([fits.PrimaryHDU(nanimg)]), ".fits")
    add("fits_named_png", fits_bytes([fits.PrimaryHDU(base.astype(np.int16))]), ".png")

    # ---- タイル圧縮
    def comp(nm, data, **kw):
        add(nm, fits_bytes([fits.PrimaryHDU(), fits.CompImageHDU(data, **kw)]), ".fits")

    comp("rice_i32", (base * 10).astype(np.int32))
    comp("rice_i16_tiles", (base - 600).astype(np.int16), compression_type="RICE_1", tile_shape=(7, 11))
    comp("rice_u8", np.clip(base / 20, 0, 255).astype(np.uint8))
    comp("rice_u16", (base * 8).astype(np.uint16), tile_shape=(16, 16))
    comp("gzip1_i32", (base * 10).astype(np.int32), compression_type="GZIP_1", tile_shape=(8, 40))
    comp("gzip2_i16", (base - 600).astype(np.int16), compression_type="GZIP_2", tile_shape=(32, 13))
    comp("nocomp_i16", (base - 600).astype(np.int16), compression_type="NOCOMPRESS")
    fl = (base / 3).astype(np.float32)
    fl[2, 3] = np.nan
    comp("rice_f32_dither1", fl, compression_type="RICE_1", quantize_level=16.0, quantize_method=1, dither_seed=17)
    comp("rice_f32_nodither", fl, compression_type="RICE_1", quantize_level=8.0, quantize_method=-1)
    fz = fl.copy()
    fz[5:7, 5:9] = 0.0
    comp("rice_f32_dither2", fz, compression_type="RICE_1", quantize_level=16.0, quantize_method=2, dither_seed=9999)
    comp("gzip2_f32_lossless", fl, compression_type="GZIP_2", quantize_level=0.0)
    comp("gzip1_f64", (base / 3).astype(np.float64), compression_type="GZIP_1", quantize_level=4.0, dither_seed=3)
    comp("rice_cube3", np.stack([base, base * 1.2, base * 0.6]).astype(np.int16), tile_shape=(1, 32, 40))
    comp("hcompress", (base * 10).astype(np.int32), compression_type="HCOMPRESS_1")
    comp("plio", np.clip(base, 0, 2 ** 20).astype(np.int32), compression_type="PLIO_1")

    # ---- PNG / TIFF / JPEG
    from PIL import Image
    import tifffile

    def pil_bytes(im, fmt, **kw):
        b = io.BytesIO()
        im.save(b, format=fmt, **kw)
        return b.getvalue()

    g8 = np.clip(base / 20, 0, 255).astype(np.uint8)
    g16 = np.clip(base * 8, 0, 65535).astype(np.uint16)
    rgb = np.stack([g8, np.clip(g8 * 1.2, 0, 255), np.clip(g8 * 0.5 + 20, 0, 255)], axis=-1).astype(np.uint8)
    add("png_gray8", pil_bytes(Image.fromarray(g8), "PNG"), ".png")
    add("png_gray16", pil_bytes(Image.fromarray(g16), "PNG"), ".png")
    add("png_rgb8", pil_bytes(Image.fromarray(rgb), "PNG"), ".png")
    add("png_rgba8", pil_bytes(Image.fromarray(np.dstack([rgb, np.full(g8.shape, 128, np.uint8)])), "PNG"), ".png")
    add("png_palette", pil_bytes(Image.fromarray(rgb).convert("P", palette=Image.Palette.ADAPTIVE, colors=37), "PNG"),
        ".png")
    add("png_la", pil_bytes(Image.fromarray(np.dstack([g8, g8 // 2])).convert("LA"), "PNG"), ".png")
    add("png_1bit", pil_bytes(Image.fromarray(g8 > 25).convert("1"), "PNG"), ".png")
    ex = Image.Exif()
    ex[0x0112] = 6
    add("png_gray8_exif6", pil_bytes(Image.fromarray(g8), "PNG", exif=ex.tobytes()), ".png")
    # 16 bit RGB の PNG は Pillow では書けないので、numpy から手で作る(PIL は 8 bit に落として読む)
    import zlib
    import struct

    def png16rgb(a):
        h_, w_ = a.shape[:2]
        rows = b"".join(b"\0" + a[y].astype(">u2").tobytes() for y in range(h_))

        def chunk(t, d):
            return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w_, h_, 16, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
    add("png_rgb16", png16rgb(np.stack([g16, g16 // 2 + 1000, 65535 - g16], axis=-1)), ".png")
    add("jpg_gray", pil_bytes(Image.fromarray(g8), "JPEG", quality=95), ".jpg")
    add("jpg_rgb", pil_bytes(Image.fromarray(rgb), "JPEG", quality=92), ".jpg")
    ex = Image.Exif()
    ex[0x0112] = 8
    add("jpg_rgb_exif8", pil_bytes(Image.fromarray(rgb), "JPEG", quality=92, exif=ex.tobytes()), ".jpg")
    # JPEG の変種(Rust は libjpeg-turbo と同じ計算で自前に復号する。間引き・プログレッシブ・RST・端数の大きさ)
    fodd = make_field(45, 61, 7, bg=60.0, noise=6.0, nstar=9, peak=(80, 190))
    yy, xx = np.mgrid[0:45, 0:61]
    codd = np.stack([fodd + xx * 1.5, fodd * 0.8 + yy * 2.0, fodd * 0.6 + 40 + (xx - yy)], axis=-1)
    codd = np.clip(codd, 0, 255).astype(np.uint8)
    imo = Image.fromarray(codd)
    add("jpg_444", pil_bytes(imo, "JPEG", quality=90, subsampling=0), ".jpg")
    add("jpg_422", pil_bytes(imo, "JPEG", quality=90, subsampling=1), ".jpg")
    add("jpg_420_odd", pil_bytes(imo, "JPEG", quality=90, subsampling=2), ".jpg")
    add("jpg_prog", pil_bytes(imo, "JPEG", quality=88, progressive=True), ".jpg")
    add("jpg_prog_444", pil_bytes(imo, "JPEG", quality=95, progressive=True, subsampling=0), ".jpg")
    add("jpg_rst", pil_bytes(imo, "JPEG", quality=90, restart_marker_blocks=2), ".jpg")
    add("jpg_prog_rst", pil_bytes(imo, "JPEG", quality=90, progressive=True, restart_marker_rows=1), ".jpg")
    add("jpg_gray_prog", pil_bytes(Image.fromarray(codd[..., 0]), "JPEG", quality=85, progressive=True), ".jpg")
    add("jpg_keep_rgb", pil_bytes(imo, "JPEG", quality=90, keep_rgb=True), ".jpg")
    add("jpg_q100", pil_bytes(imo, "JPEG", quality=100, subsampling=0), ".jpg")
    add("jpg_q10", pil_bytes(imo, "JPEG", quality=10), ".jpg")
    add("jpg_cmyk", pil_bytes(imo.convert("CMYK"), "JPEG", quality=90), ".jpg")
    try:
        import cv2
        bgr = np.ascontiguousarray(codd[..., ::-1])
        for nm, sf in (("jpg_440", 0x121111), ("jpg_411", 0x411111)):
            ok, enc = cv2.imencode(".jpg", bgr, [cv2.IMWRITE_JPEG_QUALITY, 90, cv2.IMWRITE_JPEG_SAMPLING_FACTOR, sf])
            if ok:
                add(nm, enc.tobytes(), ".jpg")
    except ImportError:
        print("  (cv2 が無いので 4:4:0・4:1:1 の JPEG は作らない)")

    def tif(a, **kw):
        b = io.BytesIO()
        tifffile.imwrite(b, a, **kw)
        return b.getvalue()
    add("tif_gray8", tif(g8), ".tif")
    add("tif_gray16_le", tif(g16), ".tif")
    add("tif_gray16_be", tif(g16, byteorder=">"), ".tif")
    add("tif_f32", tif((base / 3).astype(np.float32)), ".tif")
    add("tif_i32", tif((base * 100 - 30000).astype(np.int32)), ".tif")
    add("tif_rgb8", tif(rgb, photometric="rgb"), ".tif")
    add("tif_rgb16", tif(np.stack([g16, g16 // 2, g16 // 3], axis=-1), photometric="rgb"), ".tif")
    add("tif_gray16_deflate", tif(g16, compression="zlib"), ".tif")
    add("tif_gray8_orient3", tif(g8, extratags=[(274, "H", 1, 3, True)]), ".tif")
    pngb = pil_bytes(Image.fromarray(g8), "PNG")
    add("png_truncated", pngb[:len(pngb) // 2], ".png")
    add("not_an_image", b"hello, this is not an image" * 10, ".png")
    return fx


def run_load(path):
    try:
        b = zc.load_image_any(path)
    except Exception as e:
        return {"ok": False, "error": str(e)}
    return {"ok": True, "note": b.note, "flip_y": bool(b.flip_y), "data": stats(b.data),
            "display": stats(b.display), "header": header_list(b.header)}


def gen_formats(tmpdir):
    out = []
    for name, data, ext in gen_fixtures():
        p = os.path.join(tmpdir, name + ext)
        with open(p, "wb") as f:
            f.write(data)
        r = run_load(p)
        r["name"] = name
        r["ext"] = ext
        r["size"] = len(data)
        r["bytes_gz_b64"] = base64.b64encode(gzip.compress(data, mtime=0)).decode()
        out.append(r)
        print(f"  {name:22s} {'OK ' if r['ok'] else 'ERR'} {r.get('note', r.get('error', ''))[:70]}")
    return out


# ================================================ 3. 実画像の通し (pipeline) ===

def sources_list(found, n=50):
    return [[float(c["x"]), float(c["y"]), float(c["flux"]), float(c["peak"]), int(c["area"]),
             float(c["elong"]), float(c["fwhm"]), float(c["snr"])] for c in found[:n]]


def image_path(p):
    """image_pipeline.json の path をこの PC のパスにする(dev/test_solver_real.py の image_path と同じ決まり)。
    dev/ で始まるものは、リポジトリに無ければ $ZAHYOU_DEV、隣の ../zahyou/dev の順に探す。"""
    if p.startswith(IMAGES_VAR):
        base = os.environ.get("ZAHYOU_TEST_IMAGES") or os.path.expanduser("~")
        return os.path.join(base, *p[len(IMAGES_VAR):].split("/"))
    cands = [os.path.join(REPO, *p.split("/"))]
    if p.startswith("dev/"):
        rest = p[len("dev/"):].split("/")
        cands.append(os.path.join(ZAHYOU_DEV, *rest))
        cands.append(os.path.join(REPO, "..", "zahyou", "dev", *rest))
    for c in cands:
        if os.path.exists(c):
            return os.path.normpath(c)
    return cands[0]


def tcase_dir():
    for d in (os.path.join(REPO, "dev", "tcase"), os.path.join(ZAHYOU_DEV, "tcase"),
              os.path.join(REPO, "..", "zahyou", "dev", "tcase")):
        if os.path.isdir(d):
            return os.path.normpath(d)
    return None


def deep_record(sub, mask, sigma):
    """深い検出の結果(上位 50 個の全部の値・全部の星の (x, y)・info)と、かかった秒数。"""
    info = {}
    t0 = time.perf_counter()
    found, thr = zc.detect_sources_deep(sub, mask, sigma, info=info)
    dt = time.perf_counter() - t0
    info.pop("seconds", None)
    for k, v in list(info.items()):
        if isinstance(v, (np.floating, np.integer)):
            v = info[k] = v.item()
        if isinstance(v, float) and not np.isfinite(v):
            info[k] = str(v)                              # JSON に無い inf は文字列 "inf" にする
    return {"n": len(found), "thr": thr, "stars": sources_list(found),
            "xy": [[float(c["x"]), float(c["y"])] for c in found], "info": info}, dt


def run_pipeline(path):
    t0 = time.perf_counter()
    r = run_load(path)
    with open(path, "rb") as f:
        raw = f.read()
    r["file_size"] = len(raw)
    r["file_hash"] = bytes_hash(raw)
    if not r["ok"]:
        return r
    b = zc.load_image_any(path)
    t1 = time.perf_counter()
    sub, mask, sigma, info = zc.preprocess(b.data)
    t2 = time.perf_counter()
    r["prep"] = {"sigma": float(sigma), "mask_count": int(mask.sum()), "mask_hash": ahash(mask),
                 "info": list(info), "sub": stats(sub)}
    nomask = zc.preprocess(b.data, mask_overlay=False)
    r["prep_no_overlay"] = {"sigma": float(nomask[2]), "mask_count": int(nomask[1].sum()), "info": list(nomask[3])}
    t3 = time.perf_counter()
    for key, ms in (("det12", 12), ("det60", 60)):
        found, thr = zc.detect_sources(sub, mask, sigma, min_sources=ms)
        r[key] = {"n": len(found), "thr": thr, "stars": sources_list(found)}
    t4 = time.perf_counter()
    r["deep"], t_deep = deep_record(sub, mask, sigma)
    r["py_seconds"] = {"load": t1 - t0, "preprocess": t2 - t1, "detect_x2": t4 - t3, "deep": t_deep}
    return r


def gen_pipeline():
    tdir = tcase_dir()
    items = []
    if tdir:
        for nm in sorted(os.listdir(tdir)):
            if nm.lower().endswith((".fits", ".fit", ".png", ".jpg", ".tif")):
                items.append("dev/tcase/" + nm)
    real = [IMAGES_VAR + "Downloads/Capture_00001 00_10_33.fits",
            IMAGES_VAR + "Downloads/02013I_00010 20_47_25.fits",
            IMAGES_VAR + "Downloads/phaethon_00001.fits",
            IMAGES_VAR + "Downloads/20261031 3200 Phaethon StarChart 40arcmin.png",
            IMAGES_VAR + "Documents/claude/nextcloud-fitsviewer/samples/sharpcap-16bit.fits",
            IMAGES_VAR + "Documents/claude/nextcloud-fitsviewer/samples/siril-rgb-float.fit"]
    for p in real:
        if os.path.exists(image_path(p)):
            items.append(p)
    out = []
    for rel in items:
        path = image_path(rel)
        r = run_pipeline(path)
        r["path"] = rel
        print(f"  {os.path.basename(path)[:40]:40s} {'OK ' if r['ok'] else 'ERR'} "
              f"{r.get('prep', {}).get('sigma', '')} det60={r.get('det60', {}).get('n', '')} "
              f"deep={r.get('deep', {}).get('n', '')}")
        out.append(r)
    return out


# ================================================== 4. 速さ (prof_big と同じ) ===

def gen_big(outdir):
    nx, ny = 6000, 4000
    rng = np.random.default_rng(1)
    img = rng.normal(1000, 20, (ny, nx)).astype(np.float32)
    n = 3000
    xs = rng.uniform(5, nx - 5, n)
    ys = rng.uniform(5, ny - 5, n)
    fl = rng.lognormal(7, 1, n)
    for x, y, f in zip(xs, ys, fl):
        xi, yi = int(x), int(y)
        yy, xx = np.mgrid[yi - 3:yi + 4, xi - 3:xi + 4]
        img[yi - 3:yi + 4, xi - 3:xi + 4] += (f * np.exp(-((xx - x) ** 2 + (yy - y) ** 2) / (2 * 1.3 ** 2))
                                              ).astype(np.float32)
    p = os.path.join(outdir, "big6000x4000.fits")
    fits.PrimaryHDU(img).writeto(p, overwrite=True)
    res = {"path": p}
    t0 = time.perf_counter()
    sub, mask, sigma, info = zc.preprocess(img)
    t1 = time.perf_counter()
    found, thr = zc.detect_sources(sub, mask, sigma)
    t2 = time.perf_counter()
    found60, thr60 = zc.detect_sources(sub, mask, sigma, min_sources=60)
    t3 = time.perf_counter()
    deep_rec, t_deep = deep_record(sub, mask, sigma)
    t4 = t3 + t_deep
    t4b = time.perf_counter()
    b = zc.load_image_any(p)
    t5 = time.perf_counter()
    res.update({"sigma": float(sigma), "mask_count": int(mask.sum()), "info": info, "sub": stats(sub),
                "det12": {"n": len(found), "thr": thr, "stars": sources_list(found)},
                "det60": {"n": len(found60), "thr": thr60, "stars": sources_list(found60)},
                "deep": deep_rec,
                "load_data": stats(b.data),
                "py_seconds": {"preprocess": t1 - t0, "detect12": t2 - t1, "detect60": t3 - t2, "deep": t4 - t3,
                               "load": t5 - t4b}})
    with open(os.path.join(outdir, "big_expected.json"), "w", encoding="utf-8") as f:
        json.dump(res, f, ensure_ascii=False, indent=1)
    print(json.dumps(res["py_seconds"]), res["sigma"], len(found), deep_rec["n"])


def dump(name, obj):
    p = os.path.join(HERE, name)
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, ensure_ascii=False, separators=(",", ":"))
    print(f"wrote {p} ({os.path.getsize(p) // 1024} KB)")


def main():
    if len(sys.argv) >= 3 and sys.argv[1] == "--big":
        os.makedirs(sys.argv[2], exist_ok=True)
        gen_big(sys.argv[2])
        return
    import tempfile
    print("kernels")
    dump("image_kernels.json", gen_kernels())
    print("formats")
    with tempfile.TemporaryDirectory() as td:
        dump("image_formats.json", {"generator": "gen_image.py", "fixtures": gen_formats(td)})
    print("pipeline")
    dump("image_pipeline.json", {"generator": "gen_image.py", "items": gen_pipeline()})


if __name__ == "__main__":
    main()

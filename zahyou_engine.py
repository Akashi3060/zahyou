# ==============================================================================
#  zahyou 解析エンジン
#
#  zahyou_v6.ipynb のセル A から読み込まれます。単体では実行しません。
#  中身を読む必要はありません (直したいときだけ触ってください)。
# ==============================================================================

import concurrent.futures
import json
import os
import shlex
import shutil
import subprocess
import tempfile
import time
import warnings

import numpy as np
import matplotlib
import matplotlib.pyplot as plt
from astropy.coordinates import SkyCoord
from astropy.io import fits
from astropy.stats import sigma_clipped_stats
from astropy.visualization import AsinhStretch, ImageNormalize
from astropy.wcs import WCS
from astropy.wcs.utils import proj_plane_pixel_scales
import astropy.units as u

try:
    from scipy import ndimage
except ImportError:
    ndimage = None
    raise SystemExit("scipy が必要です。1 つ目のセルの pip 行を実行してください。")

# SharpCap などの FITS には DATE-OBS はあっても MJD-OBS が無い。astropy はそれを
# 補ったうえで警告を出すが、直す必要のあるものではないので画面には出さない。
# (抑えるのはこの 1 種類だけ。他の警告は今までどおり表示する)
from astropy.wcs import FITSFixedWarning
warnings.simplefilter("ignore", FITSFixedWarning)


def _log(msg=""):
    print(msg, flush=True)


class SolveInput:
    """解析エンジンに渡す準備が整った画像。"""

    def __init__(self, data, header, note, display_data=None, flip_y=False):
        self.data = data                 # 2次元 float32 (星検出用)
        self.header = header if header is not None else fits.Header()
        self.note = note                 # 読み込み方法の説明
        # 表示用。検出用と同じ幾何であること (同じ WCS を当てるため)
        self.display = display_data if display_data is not None else data
        # 表示するとき上下を反転するか (行 0 が画面の上に来る画像なら True)
        self.flip_y = flip_y

    @property
    def shape(self):
        return self.data.shape


# ============================================================== 画像読み込み ===

_COLOR_WEIGHTS = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
_FITS_EXT = (".fits", ".fit", ".fts", ".fz", ".fits.gz", ".fit.gz")


def _planes_to_mono(cube, axis):
    """RGB(A) の面を輝度 1 面にまとめる。"""
    cube = np.moveaxis(np.asarray(cube), axis, 0)[:3]
    n = cube.shape[0]
    w = _COLOR_WEIGHTS[:n] / _COLOR_WEIGHTS[:n].sum()
    return np.tensordot(w, cube.astype(np.float32), axes=(0, 0))


def _reduce_to_2d(data, max_frames=200):
    """3 次元以上を 2 次元へ。戻り値 (2次元 float32, 説明)"""
    data = np.asarray(data)
    while data.ndim > 3 and 1 in data.shape[:-2]:
        data = np.squeeze(data, axis=int(np.argmax([s == 1 for s in data.shape[:-2]])))
    if data.ndim > 3:
        data = data[(0,) * (data.ndim - 3)]
    if data.ndim == 2:
        return np.array(data, dtype=np.float32), ""
    if data.ndim == 3:
        nz, ny, nx = data.shape
        if nz in (3, 4) and ny > 8 and nx > 8:
            return _planes_to_mono(data, 0), "カラー 3 面を輝度に合成"
        if nx in (3, 4) and ny > 8 and nz > 8:
            return _planes_to_mono(data, 2), "カラー 3 面を輝度に合成"
        use = int(min(nz, max_frames))
        acc = np.zeros((ny, nx), dtype=np.float64)
        for i in range(use):                  # 巨大キューブでも一度に載せない
            acc += np.asarray(data[i], dtype=np.float64)
        return (acc / use).astype(np.float32), \
               f"{nz} 枚のキューブのうち先頭 {use} 枚を平均 (SNR 稼ぎ)"
    raise ValueError(f"2 次元画像に変換できない形状です: {data.shape}")


def _image_hdus(hdul):
    for hdu in hdul:
        if isinstance(hdu, (fits.PrimaryHDU, fits.ImageHDU, fits.CompImageHDU)):
            naxis = int(hdu.header.get("NAXIS", 0) or 0)
            if naxis >= 2:
                yield hdu


def _pick_fits_hdu(hdul):
    """一番大きい画像 HDU を選ぶ (拡張 HDU・圧縮 HDU も対象)。"""
    best, best_size = None, -1
    for hdu in _image_hdus(hdul):
        h = hdu.header
        size = 1
        for i in range(1, int(h.get("NAXIS", 0)) + 1):
            size *= int(h.get(f"NAXIS{i}", 1))
        if size > best_size:
            best, best_size = hdu, size
    if best is not None:
        return best
    # ヘッダが壊れていて HDU の種別を判別できないファイル向けの保険
    for hdu in hdul:
        try:
            arr = hdu.data
        except Exception:
            continue
        if arr is not None and getattr(arr, "dtype", None) is not None \
                and arr.dtype.fields is None and np.ndim(arr) >= 2:
            return hdu
    return None


def _bayer_pattern(header):
    pat = header.get("BAYERPAT") or header.get("COLORTYP") or header.get("CFAIMAGE")
    if pat is None and ("XBAYROFF" in header or "BAYOFFX" in header):
        pat = "RGGB"
    if pat is None:
        return None
    pat = str(pat).strip().upper()
    return pat if pat in ("RGGB", "BGGR", "GRBG", "GBRG") else None


def _load_fits(path):
    """戻り値 (2次元配列, header, notes, flip_y)"""
    notes = []
    # memmap は巨大キューブで効くが、BZERO/BSCALE 付き (= 一般的な 16bit FITS)
    # では astropy が拒否するので、そのときだけ読み込みに切り替える。
    for use_memmap in (True, False):
        notes = []
        try:
            with fits.open(path, memmap=use_memmap,
                           ignore_missing_simple=True) as hdul:
                hdu = _pick_fits_hdu(hdul)
                if hdu is None:
                    raise ValueError("画像データを含む HDU が FITS の中にありません。")
                header = hdu.header.copy()
                idx = list(hdul).index(hdu)
                raw = hdu.data
                notes.append(f"FITS HDU[{idx}] {tuple(np.shape(raw))} "
                             f"{np.asarray(raw).dtype}")
                data, note = _reduce_to_2d(raw)      # ここで必ずコピーが作られる
                if note:
                    notes.append(note)
                blank = header.get("BLANK")
                if blank is not None:
                    data[data == float(blank)] = np.nan
            # SharpCap / FireCapture は 1 行目が画面の上
            flip = str(header.get("ROWORDER", "")).upper().startswith("TOP")
            return data, header, notes, flip
        except ValueError as e:
            if use_memmap and "memory-mapped" in str(e):
                continue
            raise
    raise ValueError("FITS を読めませんでした。")


def _load_pil(path):
    from PIL import Image, ImageOps
    notes = []
    with Image.open(path) as im:
        try:
            im = ImageOps.exif_transpose(im)
        except Exception:
            pass
        notes.append(f"{im.format} {im.size} mode={im.mode}")
        if im.mode in ("I;16", "I;16B", "I;16L", "I", "F"):
            arr = np.asarray(im)
        elif im.mode in ("RGB", "RGBA", "P", "YCbCr", "CMYK", "LA"):
            arr = np.asarray(im.convert("RGB"))
            notes.append("カラー 3 面を輝度に合成")
        else:
            arr = np.asarray(im.convert("F"))
    data, note = _reduce_to_2d(arr)
    if note and note not in notes:
        notes.append(note)
    return data, fits.Header(), notes, True     # 画像ファイルは 1 行目が上


def looks_inverted(data):
    """
    白地に黒い星 (星図・ネガ) かどうか。

    見るのは「背景 (中央値) が、明るさの幅のどのあたりに居るか」。
      ふつうの天体写真 … 背景は下のほう。上へ大きく伸びる (星・月)
      星図            … 背景がいちばん明るい。下へ大きく伸びる (星・文字)

    裾の「太さ」(パーセンタイル) では測れない。星図に描かれた星は
    画素数でいえばごく一部なので、0.5 パーセンタイルでも背景のままになる
    (合成した星図で実際に見落とした)。端の値との距離で測る。

    逆に、周辺減光やケラレで隅が真っ黒な写真は「下へ長い裾」を持つが、
    背景の上にはもっと長い裾 (星) があるので取り違えない
    (これも実際に取り違えたので、比を 5% と厳しくしてある)。
    """
    v = data[np.isfinite(data)]
    if v.size == 0:
        return False
    med = float(np.median(v))
    up = float(v.max()) - med
    down = med - float(v.min())
    return down > 0 and up <= down * 0.05


def load_image_any(path):
    """FITS / PNG / JPEG / TIFF / キューブ を読み、2 次元 float32 にして返す。"""
    lower = str(path).lower()
    order = (_load_fits, _load_pil) if lower.endswith(_FITS_EXT) \
        else (_load_pil, _load_fits)

    first_error = None
    for loader in order:
        try:
            data, header, notes, flip_y = loader(path)
            break
        except Exception as e:                # 拡張子と中身が食い違うファイル対策
            if first_error is None:
                first_error = e
    else:
        raise ValueError(f"画像として読み込めませんでした: {first_error}")

    data = np.array(data, dtype=np.float32, copy=True)
    if data.ndim != 2:
        raise ValueError(f"2 次元画像になりませんでした: shape={data.shape}")
    if min(data.shape) < 16:
        raise ValueError(f"画像が小さすぎて解析できません: {data.shape}")

    display = data.copy()

    pat = _bayer_pattern(header)
    if pat and ndimage is not None:
        data = ndimage.uniform_filter(data, size=2, mode="nearest")
        notes.append(f"ベイヤー配列 ({pat}) を 2x2 平均で平滑化")

    bad = ~np.isfinite(data)
    if bad.any():
        good = data[~bad]
        data[bad] = float(np.median(good)) if good.size else 0.0
        notes.append(f"非有限値 {int(bad.sum())} 画素を中央値で補完")

    if float(np.ptp(data)) <= 0:
        raise ValueError("画像が一様です (全画素が同じ値)。露出やファイルを確認してください。")

    # 星図 (白地に黒い星) は明暗が逆。検出も solve-field も「明るい点」を
    # 探すので、そのままでは 1 つも拾えない。検出に使う面だけ反転する。
    # 表示は元のまま (白地の星図は白地で見せたほうが自然)。
    if looks_inverted(data):
        data = float(np.nanmax(data)) - data
        notes.append("白地に黒い星 (星図) とみて明暗を反転")

    return SolveInput(data, header, " / ".join(notes), display, flip_y)


# ================================================================== 前処理 ===

def _mask_overlay_bands(data, mask, max_band=0.08):
    """
    SharpCap / FireCapture などが焼き込む日時オーバーレイを検出してマスクする。
    文字は「画面の縁に張り付き」「1 行の中で横に広く散らばり」「飽和に近い」。
    """
    ny, nx = data.shape
    med = float(np.median(data))
    sigma = float(sigma_clipped_stats(data, sigma=3.0)[2]) or 1.0
    hot = data > med + 8.0 * sigma
    if not hot.any():
        return 0

    masked = 0
    band_rows = max(3, int(ny * max_band))
    band_cols = max(3, int(nx * max_band))

    def scan(count_fn, length, span, band, apply_fn):
        nonlocal masked
        for seq in (range(0, band), range(length - 1, length - band - 1, -1)):
            streak = []
            for i in seq:
                idxs = count_fn(i)
                if idxs.size >= 12 and (idxs.max() - idxs.min()) > 0.10 * span:
                    streak.append(i)
                elif streak:
                    break
            if streak:
                lo = max(0, min(streak) - 3)
                hi = min(length - 1, max(streak) + 3)
                apply_fn(lo, hi)
                masked += (hi - lo + 1) * span

    scan(lambda y: np.flatnonzero(hot[y]), ny, nx, band_rows,
         lambda lo, hi: mask.__setitem__((slice(lo, hi + 1), slice(None)), True))
    scan(lambda x: np.flatnonzero(hot[:, x]), nx, ny, band_cols,
         lambda lo, hi: mask.__setitem__((slice(None), slice(lo, hi + 1)), True))
    return int(masked)


def _mask_dead_edges(data, mask, frac=0.02):
    """周辺減光やデベイヤー端で潰れた縁をマスクする。"""
    ny, nx = data.shape
    med = float(np.median(data))
    for y in range(max(1, int(ny * frac))):
        for row in (y, ny - 1 - y):
            line = data[row]
            if float(np.ptp(line)) < 1e-6 or float(np.median(line)) < med * 0.05:
                mask[row, :] = True
    for x in range(max(1, int(nx * frac))):
        for col in (x, nx - 1 - x):
            line = data[:, col]
            if float(np.ptp(line)) < 1e-6 or float(np.median(line)) < med * 0.05:
                mask[:, col] = True


def _background(data, box=64):
    """粗いグリッドの中央値を伸ばして背景面を作る (かぶり・周辺減光対策)。"""
    ny, nx = data.shape
    if ndimage is None:
        return np.full_like(data, float(np.median(data)))
    by, bx = max(1, ny // box), max(1, nx // box)
    if by < 3 or bx < 3:
        return np.full_like(data, float(np.median(data)))
    ys = np.linspace(0, ny, by + 1).astype(int)
    xs = np.linspace(0, nx, bx + 1).astype(int)
    coarse = np.empty((by, bx), dtype=np.float32)
    for i in range(by):
        for j in range(bx):
            tile = data[ys[i]:ys[i + 1], xs[j]:xs[j + 1]]
            coarse[i, j] = np.median(tile) if tile.size else 0.0
    coarse = ndimage.median_filter(coarse, size=3, mode="nearest")
    back = ndimage.zoom(coarse, (ny / by, nx / bx), order=1)
    if back.shape != data.shape:                     # zoom の丸め対策
        out = np.full(data.shape, float(np.median(coarse)), dtype=np.float32)
        h = min(ny, back.shape[0])
        w = min(nx, back.shape[1])
        out[:h, :w] = back[:h, :w]
        back = out
    return back


def _kill_hot_pixels(sub, sigma):
    """
    まわりに何の広がりも持たない 1 画素だけの突出 (ホットピクセル / 宇宙線) を消す。

    「隣 8 画素の合計が中心の 30% 未満」を条件にする。星は FWHM が 1 画素を
    下回らないので、アンダーサンプルの星でも隣接画素が必ずついてくる。
    個数でしきい値を切ると、光跡などで数が増えたときに一切効かなくなるので
    条件そのもので判定する。
    """
    if ndimage is None:
        return sub, 0
    neigh_sum = ndimage.uniform_filter(sub, size=3, mode="nearest") * 9.0 - sub
    hot = (sub > 8.0 * sigma) & (neigh_sum < 0.30 * sub)
    n = int(hot.sum())
    if n == 0 or n > 0.05 * sub.size:
        return sub, 0
    med3 = ndimage.median_filter(sub, size=3, mode="nearest")
    return np.where(hot, med3, sub), n


def preprocess(data, mask_overlay=True):
    """背景を引き、マスクを作る。戻り値 (背景差引画像, mask, ノイズσ, 説明)"""
    ny, nx = data.shape
    mask = np.zeros(data.shape, dtype=bool)
    info = []

    sub = data - _background(data)
    sigma = float(sigma_clipped_stats(sub, sigma=3.0)[2])
    if not np.isfinite(sigma) or sigma <= 0:
        sigma = float(np.std(sub)) or 1.0

    # 焼き込み文字の判定より先に外す。ホットピクセルが行に散っていると
    # 「文字列がある」と誤判定してしまうため。
    sub, n_hot = _kill_hot_pixels(sub, sigma)
    if n_hot:
        info.append(f"ホットピクセル {n_hot} 画素を平滑化")

    _mask_dead_edges(data, mask)
    if mask_overlay:
        n = _mask_overlay_bands(sub + float(np.median(data)), mask)
        if n:
            info.append(f"焼き込みオーバーレイとみられる {n} 画素をマスク")

    ref = sub[~mask] if (~mask).any() else sub
    s2 = float(sigma_clipped_stats(ref, sigma=3.0)[2])
    if np.isfinite(s2) and s2 > 0:
        sigma = s2

    if mask.any():
        info.append(f"マスク合計 {int(mask.sum())} / {ny * nx} 画素")
    return sub, mask, sigma, info


# ================================================================ 星の検出 ===

def _measure(sub, labels, keep):
    """ラベル領域ごとに重心・フラックス・伸びを測る。"""
    out = []
    slices = ndimage.find_objects(labels)
    for lab in keep:
        sl = slices[lab - 1]
        if sl is None:
            continue
        tile = np.asarray(sub[sl], dtype=np.float64)
        m = labels[sl] == lab
        w = np.where(m, tile, 0.0)
        tot = float(w.sum())
        if tot <= 0:
            continue
        yy, xx = np.mgrid[sl[0].start:sl[0].stop, sl[1].start:sl[1].stop]
        cx = float((w * xx).sum() / tot)
        cy = float((w * yy).sum() / tot)
        vx = float((w * (xx - cx) ** 2).sum() / tot)
        vy = float((w * (yy - cy) ** 2).sum() / tot)
        vxy = float((w * (xx - cx) * (yy - cy)).sum() / tot)
        tr, det = vx + vy, vx * vy - vxy ** 2
        root = np.sqrt(max(tr * tr / 4.0 - det, 0.0))
        a2 = max(tr / 2.0 + root, 1e-6)
        b2 = max(tr / 2.0 - root, 1e-6)
        out.append(dict(x=cx, y=cy, flux=tot, peak=float(tile[m].max()),
                        area=int(m.sum()), elong=float(np.sqrt(a2 / b2)),
                        fwhm=float(2.3548 * np.sqrt(max((vx + vy) / 2.0, 1e-6)))))
    return out


def detect_sources(sub, mask, sigma, min_sources=12, max_sources=400,
                   max_elongation=4.0, min_area=3, min_snr=5.0):
    """
    しきい値を下げながら星を探す。少なくとも min_sources 個見つかるまで粘る。
    astrometry.net は 4 個で 1 quad を作るが、実用上は 10 個以上ほしい。

    min_snr を 3.0 に下げると暗い画像で星は増えるが (実測 8 → 15 個)、
    疎な星野では雑音のこぶも同じだけ増える (合成テストで純度 100% → 30%)。
    どの値でも「本物だけ増やす」ことはできなかったので 5.0 のままにしてある。
    星が少ない画像は、こちらの検出に頼らず solve-field 自身の抽出に任せる
    (solve_offline が画像そのものを渡す経路を先に試す)。
    """
    if ndimage is None:
        raise RuntimeError("scipy が必要です:  pip install scipy")

    work = np.where(mask, 0.0, sub)
    ny, nx = work.shape
    smooth = ndimage.gaussian_filter(work, sigma=1.0)
    smooth_sigma = sigma / 2.0          # 平滑化でノイズが下がる分

    best, used = [], None
    for thr in (12.0, 8.0, 6.0, 5.0, 4.0, 3.0, 2.5, 2.0):
        binary = smooth > thr * smooth_sigma
        n_on = int(binary.sum())
        if n_on == 0:
            continue
        if n_on > 0.25 * binary.size:   # 何かおかしい (雲・月明かり) ので下げない
            break
        labels, n = ndimage.label(binary)
        if n == 0:
            continue
        if n > 20000:                   # ノイズが噴き出している
            break
        sizes = ndimage.sum(binary, labels, range(1, n + 1))
        keep = np.flatnonzero(sizes >= min_area) + 1
        if keep.size == 0:
            continue
        cand = []
        for c in _measure(work, labels, keep):
            # 開口 SNR。ノイズのこぶは面積のわりに光量が無いのでここで落ちる。
            c["snr"] = c["flux"] / (sigma * np.sqrt(max(c["area"], 1)))
            if c["snr"] < min_snr or c["elong"] > max_elongation:
                continue
            if not (2.0 <= c["x"] <= nx - 3 and 2.0 <= c["y"] <= ny - 3):
                continue
            cand.append(c)
        if len(cand) > len(best):
            best, used = cand, thr
        if len(cand) >= min_sources:
            best, used = cand, thr
            break

    best.sort(key=lambda c: -c["flux"])
    return best[:max_sources], used


def write_xylist(sources, width, height, path):
    """solve-field に渡す xylist。FITS 規約なので座標は 1 始まりで書く。"""
    x = np.array([s["x"] for s in sources], dtype=np.float32) + 1.0
    y = np.array([s["y"] for s in sources], dtype=np.float32) + 1.0
    flux = np.array([s["flux"] for s in sources], dtype=np.float32)
    hdu = fits.BinTableHDU.from_columns(fits.ColDefs([
        fits.Column(name="X", format="E", array=x),
        fits.Column(name="Y", format="E", array=y),
        fits.Column(name="FLUX", format="E", array=flux),
    ]))
    hdu.header["IMAGEW"] = int(width)
    hdu.header["IMAGEH"] = int(height)
    fits.HDUList([fits.PrimaryHDU(), hdu]).writeto(path, overwrite=True)
    return path


def write_solver_fits(data, path):
    """solve-field にそのまま渡せる素直な 2 次元 float32 FITS。"""
    arr = np.asarray(data, dtype=np.float32)
    if not np.isfinite(arr).all():
        arr = np.nan_to_num(arr, nan=float(np.nanmedian(arr)), posinf=0.0, neginf=0.0)
    fits.PrimaryHDU(arr).writeto(path, overwrite=True)
    return path


# ============================================================ スケール推定 ===

def _first_float(header, keys):
    for k in keys:
        if k in header:
            try:
                v = float(header[k])
            except (TypeError, ValueError):
                continue
            if np.isfinite(v) and v != 0:
                return v, k
    return None, None


def rig_key(header, shape):
    """
    「同じ機材・同じ画像サイズ」を表す鍵。
    画素スケールは焦点距離とセンサーで決まるので、これが同じなら前回の実測値が使える。
    """
    ny, nx = shape
    cam = str(header.get("INSTRUME") or header.get("CAMID")
              or header.get("CAMERA") or "?").strip()
    bx = header.get("XBINNING") or header.get("BINX") or 1
    tel = str(header.get("TELESCOP") or "").strip()
    focal = header.get("FOCALLEN") or ""
    return f"{cam}|{tel}|{nx}x{ny}|bin{bx}|f{focal}"


def remembered_scale(header, shape):
    """前に同じ機材で解いたときの実測スケール [arcsec/px]。無ければ None。"""
    return _cache_load().get("rigs", {}).get(rig_key(header, shape))


def remember_scale(header, shape, scale_arcsec):
    """解けたので、この機材の画素スケールを覚えておく。"""
    if not (scale_arcsec and 0.01 < scale_arcsec < 3600):
        return
    cache = _cache_load()
    cache.setdefault("rigs", {})[rig_key(header, shape)] = float(scale_arcsec)
    _cache_save(cache)


def estimate_pixel_scale(header, focal_length_mm=None, shape=None):
    """
    1 画素の秒角を推定する。戻り値 (arcsec/px or None, 説明)

    上ほど信用できる順:
      1. ユーザーが焦点距離を書いた
      2. この画像自身が持っている WCS
      3. ヘッダの画素スケールキーワード
      4. ヘッダの焦点距離 x 画素サイズ
      5. 前に同じ機材で解いたときの実測値  ← 2 回目以降は設定不要で速くなる

    5 を最後に置いているのは、鏡筒を替えてもカメラが同じなら鍵が一致してしまうため。
    ヘッダが焦点距離を書いてくれているなら、そちらを信じた方が安全。
    """
    if focal_length_mm:
        pix, pk = _first_float(header, ["XPIXSZ", "PIXSIZE1", "XPIXELSZ",
                                        "PIXSZ", "PIXSIZEX"])
        if pix:
            s = 206.264806 * pix / float(focal_length_mm)
            if 0.01 < s < 3600:
                return s, f"{pk}={pix}um と FOCAL_LENGTH_MM={focal_length_mm:g}mm から"

    try:
        w = WCS(header)
        if w.is_celestial:
            from astropy.wcs.utils import proj_plane_pixel_scales
            s = float(np.mean(proj_plane_pixel_scales(w.celestial)) * 3600.0)
            if 0.01 < s < 3600:
                return s, "画像に既にあった WCS から"
    except Exception:
        pass

    v, k = _first_float(header, ["SECPIX", "SECPIX1", "PIXSCALE", "SCALE",
                                 "PLTSCALE", "CDELT1"])
    if v is not None:
        s = abs(v) * (3600.0 if k == "CDELT1" else 1.0)
        if 0.01 < s < 3600:
            return s, f"ヘッダ {k} から"

    pix, pk = _first_float(header, ["XPIXSZ", "PIXSIZE1", "XPIXELSZ",
                                    "PIXSZ", "PIXSIZEX"])
    focal, fk = _first_float(header, ["FOCALLEN", "FOCAL", "FOCALLENGTH"])
    if pix and focal:
        s = 206.264806 * pix / focal
        if 0.01 < s < 3600:
            return s, f"{pk}={pix}um, {fk}={focal:g}mm から"

    if shape is not None:
        s = remembered_scale(header, shape)
        if s:
            return float(s), "前回この機材で解いたときの実測値"

    return None, "手がかりなし (星図データが対応する画角を総当たりします)"


def implied_focal_length(header, scale_arcsec):
    """解けたスケールから焦点距離を逆算する (ヘッダに画素サイズがあるときだけ)。"""
    pix, _ = _first_float(header, ["XPIXSZ", "PIXSIZE1", "XPIXELSZ",
                                   "PIXSZ", "PIXSIZEX"])
    if not pix or not scale_arcsec:
        return None
    return 206.264806 * pix / scale_arcsec


# ================================================================ WSL 実行 ===

# Windows で wsl.exe をそのまま呼ぶと、1 回ごとに黒いコンソール窓が開いて消える。
# オフライン解析は solve-field を何度も呼ぶので画面がちかちか点滅して驚かせる。
# CREATE_NO_WINDOW を渡すと窓を作らない (出力は今までどおり受け取れる)。
_NO_WINDOW = 0x08000000 if os.name == "nt" else 0


def _run(cmd, timeout):
    return subprocess.run(cmd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=timeout,
                          creationflags=_NO_WINDOW)


def _bash(command, timeout):
    """Windows なら WSL 経由、Linux/macOS ならそのまま bash に渡す。"""
    if os.name == "nt":
        return _run(["wsl.exe", "-e", "bash", "-lc", command], timeout=timeout)
    return _run(["bash", "-lc", command], timeout=timeout)


def wsl_available():
    if os.name != "nt":
        return shutil.which("solve-field") is not None
    if not (shutil.which("wsl") or shutil.which("wsl.exe")):
        return False
    try:
        p = _bash("echo ok", timeout=45)
        return p.returncode == 0 and "ok" in (p.stdout or "")
    except Exception:
        return False


def to_solver_path(win_path):
    """Windows パス → WSL パス。wslpath に任せるので空白も日本語も安全。"""
    if os.name != "nt":
        return win_path
    try:
        p = _run(["wsl.exe", "-e", "wslpath", "-a", "-u", win_path], timeout=60)
        if p.returncode == 0 and p.stdout.strip():
            return p.stdout.strip()
    except Exception:
        pass
    drive, rest = os.path.splitdrive(os.path.abspath(win_path))
    return "/mnt/" + drive[0].lower() + rest.replace("\\", "/")


# --- インデックスがカバーする画角 (arcmin)。astrometry.net の付番規則 ---------
_INDEX_SCALES = {
    0: (2, 2.8), 1: (2.8, 4), 2: (4, 5.6), 3: (5.6, 8), 4: (8, 11),
    5: (11, 16), 6: (16, 22), 7: (22, 30), 8: (30, 42), 9: (42, 60),
    10: (60, 85), 11: (85, 120), 12: (120, 170), 13: (170, 240),
    14: (240, 340), 15: (340, 480), 16: (480, 680), 17: (680, 1000),
    18: (1000, 1400), 19: (1400, 2000),
}


def index_coverage(index_names):
    """index-XXXX[-YY].fits の一覧から、扱える画角 (arcmin) の範囲を返す。"""
    lo = hi = None
    for name in index_names:
        core = os.path.basename(str(name))
        if not core.startswith("index-"):
            continue
        num = core[len("index-"):].split("-")[0].split(".")[0]
        if not num.isdigit() or len(num) < 2:
            continue
        s = _INDEX_SCALES.get(int(num[-2:]))
        if not s:
            continue
        lo = s[0] if lo is None else min(lo, s[0])
        hi = s[1] if hi is None else max(hi, s[1])
    return lo, hi


def blind_scale_bounds(width_px, index_names=None):
    """
    画角が全く分からないときに使う --scale-low/--scale-high。

    手持ちの index が扱える画角から逆算するので、
    「絶対に当たらない範囲」を探索しない = 総当たりでも無駄がない。
    """
    lo_fov, hi_fov = index_coverage(index_names or [])
    if lo_fov is None:
        lo_fov, hi_fov = 2.0, 2000.0        # astrometry.net が配布する全範囲
    # index の quad は画角の 10%〜100% 程度まで使えるので下側に少し余裕を持たせる
    low = lo_fov * 60.0 / float(width_px) * 0.9
    high = hi_fov * 60.0 / float(width_px) * 1.1
    return max(low, 0.01), min(high, 3600.0)


def missing_index_advice(fov_arcmin, index_names):
    """
    この画角を解くにはどの index を足せばよいかを教える。
    戻り値 (足りているか, 案内文のリスト)
    """
    have = set()
    for name in index_names or []:
        core = os.path.basename(str(name))
        if core.startswith("index-"):
            num = core[len("index-"):].split("-")[0].split(".")[0]
            if num.isdigit() and len(num) >= 2:
                have.add(int(num[-2:]))
    need = [k for k, (lo, hi) in _INDEX_SCALES.items() if lo <= fov_arcmin < hi]
    if not need:
        return False, [f"画角 {fov_arcmin:.1f}′ は astrometry.net が配布する星図データの範囲 "
                       "(2′〜2000′) の外です。ビニングやモザイクで画角を稼ぐか、"
                       "build-astrometry-index で自作する必要があります。"]
    if need[0] in have:
        return True, []
    lo, hi = _INDEX_SCALES[need[0]]
    files = f"index-52{need[0]:02d}-00〜47.fits" if need[0] <= 6 else f"index-41{need[0]:02d}.fits"
    msgs = [
        f"画角 {fov_arcmin:.1f}′ に対応する星図データ ({lo:g}′〜{hi:g}′ 段) がありません。",
        f"必要なファイル: {files}",
    ]
    if need[0] <= 6:
        msgs.append(f"取得: python dev/fetch_index.py 52{need[0]:02d}")
    return False, msgs


def solver_diagnostics():
    """solve-field と星図データが本当に使えるかを事前に確認する。"""
    rep = {"ok": False, "messages": [], "indexes": [], "cfg": "", "paths": []}
    if not wsl_available():
        rep["messages"].append(
            "WSL (または solve-field) が使えません。マニュアル 2.1〜2.2 のとおり "
            "`wsl --install` と `sudo apt install astrometry.net -y` を実行してください。")
        return rep

    p = _bash("command -v solve-field", timeout=60)
    if p.returncode != 0 or not p.stdout.strip():
        rep["messages"].append(
            "WSL の中に solve-field がありません。WSL のターミナルで "
            "`sudo apt update && sudo apt install astrometry.net -y` を実行してください。")
        return rep

    cfg = _bash("cat /etc/astrometry.cfg 2>/dev/null", timeout=60)
    rep["cfg"] = cfg.stdout or ""
    paths = [ln.split(None, 1)[1].strip()
             for ln in rep["cfg"].splitlines()
             if ln.strip().startswith("add_path") and len(ln.split(None, 1)) > 1]
    paths += ["/usr/share/astrometry", "/usr/local/astrometry/data"]
    rep["paths"] = paths

    listing = _bash(
        "for d in %s; do ls -1 \"$d\"/index-*.fits 2>/dev/null; done"
        % " ".join(shlex.quote(x) for x in paths), timeout=240)
    rep["indexes"] = sorted({os.path.basename(x)
                             for x in listing.stdout.split() if x.strip()})
    if not rep["indexes"]:
        rep["messages"].append(
            "星図データ (index-*.fits) が 1 つも見つかりません。C:\\AstrometryData に "
            "展開されているか、/etc/astrometry.cfg の add_path が正しいかを確認してください。")
        return rep

    rep["ok"] = True
    return rep


# ============================================================== solve-field ===

def _wcs_from_dir(out_dir_win):
    for name in sorted(os.listdir(out_dir_win)):
        if not name.endswith(".wcs"):
            continue
        try:
            hdr = fits.getheader(os.path.join(out_dir_win, name))
            if WCS(hdr).is_celestial:
                return hdr
        except Exception:
            continue
    return None


def _clean_outputs(out_dir_win):
    for name in os.listdir(out_dir_win):
        if name.endswith((".wcs", ".corr", ".axy", ".solved", ".rdls", ".match")):
            try:
                os.remove(os.path.join(out_dir_win, name))
            except OSError:
                pass


def run_solve_field(file_win, kind, out_dir_win, width, height, opts,
                    timeout, verbose=True):
    """solve-field を 1 回だけ動かす。成功したら WCS ヘッダ、失敗したら None。"""
    _clean_outputs(out_dir_win)
    args = ["solve-field", to_solver_path(file_win),
            "--overwrite", "--no-plots",
            "--dir", to_solver_path(out_dir_win),
            "--new-fits", "none", "--rdls", "none",
            "--match", "none", "--solved", "none", "--index-xyls", "none"]
    if kind == "xylist":
        args += ["--width", str(int(width)), "--height", str(int(height)),
                 "--x-column", "X", "--y-column", "Y", "--sort-column", "FLUX"]
    args += list(opts)
    command = " ".join(shlex.quote(a) for a in args)
    if verbose:
        _log(f"    $ {command}")
    try:
        p = _bash(command, timeout=timeout)
    except subprocess.TimeoutExpired:
        if verbose:
            _log("      ! 制限時間内に終わりませんでした")
        return None

    hdr = _wcs_from_dir(out_dir_win)
    if hdr is None and verbose:
        for line in (p.stdout or "").splitlines():
            if "simplexy: found" in line or "Field center" in line:
                _log(f"      · {line.strip()}")
        for line in (p.stderr or "").strip().splitlines()[-6:]:
            _log(f"      ! {line}")
    return hdr


def solve_offline(bundle, sources, work_dir, timeout=300, focal_length_mm=None,
                  hint_radec=None, hint_radius_deg=5.0, verbose=True,
                  diagnostics=None):
    """
    オフライン (WSL の astrometry.net) で解く。条件を変えながら粘り強く挑む。
    戻り値: WCS ヘッダ or None
    """
    ny, nx = bundle.shape
    if not sources:
        _log("  ❌ 星が 1 つも無いので解析できません。")
        return None

    scale, how = estimate_pixel_scale(bundle.header, focal_length_mm, bundle.shape)
    fov_w = nx * scale / 60.0 if scale else None
    fov_h = ny * scale / 60.0 if scale else None
    fov_short = min(fov_w, fov_h) if scale else None
    # 星の並び (quad) は「短いほうの辺」に収まる大きさまでしか作れないので、
    # 必要な index は短辺で決まる。横幅だけ見ていると足りない段を見落とす。
    #   実測: 23.6′ x 13.4′ の画像 (ごく普通の 16:9) で、横幅に合う 22′〜30′ の
    #   段だけでは 120 秒かけても解けず、短辺に合う 11′〜16′ を足したら 10 秒で
    #   解けた。掩蔽観測のように縦を切り詰めるとこの差はさらに開く。
    if scale:
        _log(f"  - 画素スケール {scale:.3f}″/px ({how}) "
             f"→ 画角 {fov_w:.1f}′ x {fov_h:.1f}′ (短辺 {fov_short:.1f}′)")
    else:
        _log(f"  - 画素スケール: {how}")

    installed = (diagnostics or {}).get("indexes") or []
    if installed:
        lo, hi = index_coverage(installed)
        if lo is not None:
            _log(f"  - 手持ちの星図データが対応する画角: {lo:g}′ 〜 {hi:g}′"
                 f" (index ファイル {len(installed)} 個)")
            if scale:
                # 判定は短辺で行う (上のコメント参照)
                ok, msgs = missing_index_advice(fov_short, installed)
                for m in msgs:
                    _log(("  ⚠️ " if not ok else "  - ") + m)
                if not ok:
                    _log(f"     ※ 必要な段は横幅 {fov_w:.1f}′ ではなく"
                         f"短辺 {fov_short:.1f}′ で決まります。")

    xy_path = write_xylist(sources, nx, ny,
                           os.path.join(work_dir, "zahyou_sources.xyls"))
    img_path = write_solver_fits(bundle.data,
                                 os.path.join(work_dir, "zahyou_image.fits"))

    t_short = max(30, int(timeout * 0.25))
    t_long = max(60, int(timeout * 0.5))
    # 星が少ないときは、こちらの星リストで粘っても当たりにくい。長く回しても
    # 無駄なので短く切り上げ、時間は solve-field 自身に画像を探させる試行へ回す。
    # (星 8 個の画像で、1 つ目の試行が 150 秒使い切ってから 2 つ目が 4 秒で
    #  解けていた = 待ち時間のほとんどが無駄だった)
    few = len(sources) < 12
    t_xy = t_short if few else t_long

    # --objs は「使う星を上位いくつに絞るか」。
    # xylist はこちらが渡した星がすべてなので実数でよいが、画像を渡すときは
    # solve-field 自身が星を探す。こちらの検出数で絞ってはいけない ――
    # simplexy が 76 個見つけた画像で --objs 8 を渡していたために解けなかった
    # (外したら 3.5 秒で解けた)。
    OBJS_IMAGE = 200

    def base(cpu, kind="xylist"):
        n = min(len(sources), 200) if kind == "xylist" else OBJS_IMAGE
        return ["--cpulimit", str(int(cpu)), "--objs", str(n)]

    hint = []
    if hint_radec is not None:
        hint = ["--ra", f"{hint_radec[0]:.6f}", "--dec", f"{hint_radec[1]:.6f}",
                "--radius", f"{hint_radius_deg:g}"]

    narrow = []
    if scale:
        narrow = ["--scale-units", "arcsecperpix",
                  "--scale-low", f"{max(scale * 0.75, 0.01):.4f}",
                  "--scale-high", f"{scale * 1.25:.4f}"]
    # 画角が分からないときは「手持ちの index で解ける範囲」を総当たりする。
    # 固定値 (v6.0 は 0.2〜120) だと、長焦点の画像がそもそも範囲外になってしまう。
    wlo, whi = blind_scale_bounds(nx, installed)
    wide = ["--scale-units", "arcsecperpix",
            "--scale-low", f"{wlo:.4f}", "--scale-high", f"{whi:.4f}"]
    if not scale:
        _log(f"  - 探索するスケール: {wlo:.3f}〜{whi:.3f}″/px "
             f"(画角 {wlo * nx / 60:.1f}′〜{whi * nx / 60:.0f}′ 相当)")

    attempts = []
    # 星が少ないときは、こちらの星リストより solve-field 自身の抽出のほうが
    # 当たりやすい (simplexy は淡い星も拾う)。いちばん先に試す。
    # 実測: 星 8 個の画像で、星リストの試行が 150 秒粘ってから画像の試行が
    # 4 秒で解けていた。順番を入れ替えるだけで 154 秒 → 20 秒になった。
    if few:
        attempts.append(("image", img_path, "画像を直接渡す (2x 縮小)",
                         base(t_short, "image") + hint + (narrow or wide)
                         + ["--downsample", "2"], t_short))
    if hint and narrow:
        attempts.append(("xylist", xy_path, "座標ヒント + スケール既知",
                         base(t_short) + hint + narrow, t_short))
    if hint:
        attempts.append(("xylist", xy_path, "座標ヒント + スケール全域",
                         base(t_short) + hint + wide, t_short))
    if narrow:
        attempts.append(("xylist", xy_path, "全天 + スケール既知",
                         base(t_xy) + narrow, t_xy))
    attempts += [
        ("xylist", xy_path, "全天 + スケール全域", base(t_xy) + wide, t_xy),
    ]
    attempts += [
        ("xylist", xy_path, "全天 + 歪み補正なし (星が少ない画像向け)",
         base(t_xy) + wide + ["--no-tweak"], t_xy),
        ("image", img_path, "画像を直接渡す (2x 縮小)",
         base(t_long, "image") + wide + ["--downsample", "2"], t_long),
        ("image", img_path, "画像を直接渡す (4x 縮小)",
         base(t_long, "image") + wide + ["--downsample", "4"], t_long),
    ]
    seen = set()                       # 同じ条件を 2 度走らせない
    uniq = []
    for a in attempts:
        key = (a[0], tuple(a[3]))
        if key not in seen:
            seen.add(key)
            uniq.append(a)
    attempts = uniq

    deadline = time.time() + timeout
    for i, (kind, path, label, opts, t) in enumerate(attempts, 1):
        left = deadline - time.time()
        if left < 20:
            _log("  ⏱ 制限時間に達したので、これ以上の再試行はしません。")
            break
        t = int(min(t, left))
        _log(f"\n  [{i}/{len(attempts)}] {label}  (最大 {t} 秒)")
        hdr = run_solve_field(path, kind, work_dir, nx, ny, opts,
                              timeout=t + 45, verbose=verbose)
        if hdr is not None:
            _log(f"  ✅ 解析成功 ({label})")
            return hdr

    _log("\n  ❌ すべての条件で解けませんでした。心当たりの順に:")
    if len(sources) < 15:
        _log(f"     ・写っている星が {len(sources)} 個と少なめです。"
             "露出を伸ばすか、複数フレームを重ねてください。")
    lo_fov, hi_fov = index_coverage(installed)
    if lo_fov is not None:
        _log(f"     ・手持ちの星図データは画角 {lo_fov:g}′〜{hi_fov:g}′ しか解けません。")
        if fov_short:
            ok2, msgs2 = missing_index_advice(fov_short, installed)
            if not ok2:
                _log(f"       この画像は短辺が {fov_short:.1f}′ です"
                     f"(横幅 {fov_w:.1f}′)。星の並びは短いほうの辺で決まるので:")
                for m in msgs2:
                    _log("         " + m)
        else:
            _log("       画角が分かれば、足りない段を名指しできます。"
                 "FOCAL_LENGTH_MM を書いてもう一度試してください。")
    if not scale:
        _log("     ・FOCAL_LENGTH_MM に焦点距離を書くと画角が確定し、"
             "総当たりをやめるので格段に速く・確実になります。")
    _log("     ・ピントや雲、極端な露出過多も疑ってください。")
    return None


# ============================================================ オンライン解析 ===

def internet_available(timeout=4):
    """astrometry.net に本当に届くかを見る (google では意味がない)。"""
    import urllib.request
    for url in ("https://nova.astrometry.net/api/", "https://nova.astrometry.net/"):
        try:
            urllib.request.urlopen(url, timeout=timeout)
            return True
        except Exception:
            continue
    return False


def _with_deadline(fn, timeout, label):
    """
    指定時間で必ず戻る呼び出し。
    v5 は with ThreadPoolExecutor(...) を使っていたため、タイムアウト後も
    ブロックの出口でワーカーの終了を待ってしまい、実際には打ち切れなかった。
    """
    executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)
    try:
        future = executor.submit(fn)
        try:
            return future.result(timeout=timeout)
        except concurrent.futures.TimeoutError:
            future.cancel()
            _log(f"⚠️ {label}が {timeout:.0f} 秒で終わりませんでした。")
            return None
    finally:
        # wait=False にしないと、ここで居残りスレッドの終了を待ってしまう
        executor.shutdown(wait=False, cancel_futures=True)


def solve_online(image_path, api_key, sources=None, shape=None, timeout=120,
                 hint_radec=None, hint_radius_deg=5.0, scale_hint=None):
    """
    nova.astrometry.net に投げる。指定時間で必ず戻る。

    星のリストがあるならそれを送る。画像そのものより桁違いに軽く、
    オフラインとまったく同じ検出結果で解けるので結果もぶれない。
    """
    from astroquery.astrometry_net import AstrometryNet

    ast = AstrometryNet()
    ast.api_key = api_key
    settings = {}
    if hint_radec is not None:
        settings.update(center_ra=float(hint_radec[0]),
                        center_dec=float(hint_radec[1]),
                        radius=float(hint_radius_deg))
    if scale_hint:
        settings.update(scale_units="arcsecperpix", scale_type="ul",
                        scale_lower=float(scale_hint) * 0.75,
                        scale_upper=float(scale_hint) * 1.25)

    if sources and shape is not None:
        ny, nx = shape
        xs = [s["x"] + 1.0 for s in sources]      # xylist は FITS 規約で 1 始まり
        ys = [s["y"] + 1.0 for s in sources]
        _log(f"  - 検出した {len(xs)} 個の星の位置だけを送ります。")
        hdr = _with_deadline(
            lambda: ast.solve_from_source_list(
                xs, ys, int(nx), int(ny),
                solve_timeout=int(timeout), verbose=False, **settings),
            timeout + 15, "オンライン解析")
        if hdr:
            return hdr
        _log("  - 星の位置だけでは解けなかったので、画像そのものを送ります。")

    return _with_deadline(
        lambda: ast.solve_from_image(image_path, solve_timeout=int(timeout),
                                     verbose=False, **settings),
        timeout + 30, "オンライン解析")


# ============================================================== 目標の座標 ===

def _cache_path():
    """
    目標の座標と機材ごとの画素スケールを覚えておくファイル。

    以前は Temp に置いていたが、そこは Windows の「ディスクの クリーンアップ」や
    ストレージセンサーが消しにいく場所だった。覚えた座標は
    「一度オンラインで引いた天体名を、山の中でもそのまま使う」ための命綱なので、
    消えない場所 (%LOCALAPPDATA%\\zahyou) へ移した。
    前の場所にファイルが残っていれば、一度だけ引き継ぐ。
    """
    override = os.environ.get("ZAHYOU_CACHE")
    if override:
        return override
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
    old_path = os.path.join(tempfile.gettempdir(), "zahyou_cache.json")
    try:
        d = os.path.join(base, "zahyou")
        os.makedirs(d, exist_ok=True)
    except OSError:
        return old_path
    new_path = os.path.join(d, "cache.json")
    if not os.path.exists(new_path) and os.path.exists(old_path):
        try:
            shutil.copyfile(old_path, new_path)
        except OSError:
            return old_path
    return new_path


def _cache_load():
    try:
        with open(_cache_path(), encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def _cache_save(cache):
    try:
        with open(_cache_path(), "w", encoding="utf-8") as f:
            json.dump(cache, f, ensure_ascii=False, indent=1)
    except Exception:
        pass


def resolve_target_name(name, online=True):
    """
    天体名 / カタログ名 → (RA, Dec) [度]。
    一度引いた名前はキャッシュするので、次からはオフラインでも使える。
    """
    from astropy.coordinates import SkyCoord
    import astropy.units as u

    key = " ".join(str(name).split()).upper()
    cache = _cache_load()
    targets = cache.setdefault("targets", {})

    if not online:
        hit = targets.get(key)
        if hit:
            _log(f"  (オフライン) 以前調べた座標を再利用します: {name}")
            return hit["ra"], hit["dec"]
        _log(f"  ❌ オフラインでは '{name}' を座標に変換できません。"
             " INPUT_MODE='COORDS' で赤経赤緯を直接入力してください。")
        return None, None

    _log(f"\n'{name}' の座標を調べています...")
    try:
        base = SkyCoord.from_name(name)
    except Exception as e:
        _log(f"  ❌ 名前を解決できませんでした: {e}")
        hit = targets.get(key)
        if hit:
            _log("  → 以前調べた座標を使います。")
            return hit["ra"], hit["dec"]
        return None, None
    _log(f"  - 名前解決 (SIMBAD/Sesame): {base.to_string('hmsdms', precision=3)}")

    ra, dec = base.ra.deg, base.dec.deg
    try:
        from astroquery.vizier import Vizier
        v = Vizier(catalog="I/355/gaiadr3",
                   columns=["RA_ICRS", "DE_ICRS", "Gmag"])
        v.ROW_LIMIT = 50
        res = v.query_region(base, radius=5 * u.arcsec)
        if res and len(res[0]):
            t = res[0]
            cand = SkyCoord(np.asarray(t["RA_ICRS"], dtype=float),
                            np.asarray(t["DE_ICRS"], dtype=float), unit="deg")
            # v5 は先頭行を無条件に採用していた。5″以内に複数あると別の星を掴む。
            i = int(np.argmin(base.separation(cand).arcsec))
            ra, dec = float(t["RA_ICRS"][i]), float(t["DE_ICRS"][i])
            extra = ""
            if "Gmag" in t.colnames:
                try:
                    extra = f" (G={float(t['Gmag'][i]):.2f}, 候補 {len(t)} 個中で最も近い星)"
                except Exception:
                    pass
            _log(f"  - Gaia DR3 で精密化: RA={ra:.6f}° Dec={dec:.6f}°{extra}")
    except Exception as e:
        _log(f"  - Gaia DR3 の照合はできませんでした ({e})。SIMBAD の座標を使います。")

    targets[key] = {"ra": ra, "dec": dec, "name": str(name)}
    _cache_save(cache)
    return ra, dec

def normalize_wcs_header(header, shape):
    """
    solve-field が返す .wcs は画像データを持たないので、そのまま WCS() に渡すと
    「軸数が合わない」と警告が出る。画像に合わせて整えたコピーを返す。
    """
    h = header.copy()
    h["SIMPLE"] = True
    h["NAXIS"] = 2
    h["NAXIS1"] = int(shape[1])
    h["NAXIS2"] = int(shape[0])
    # 観測日時由来のキーワードが中途半端に残っていると FITSFixedWarning が出る
    for key in ("MJD-OBS", "DATE-OBS", "MJD-AVG", "MJD-END"):
        h.pop(key, None)
    # astrometry.net は EQUINOX しか書かないので astropy は FK5 と解釈するが、
    # インデックスの元カタログ (Tycho-2 / Gaia) は ICRS。明示しておく。
    h.pop("EQUINOX", None)
    h.pop("RADECSYS", None)
    h["RADESYS"] = "ICRS"
    return h


def as_icrs(coord):
    """フレームが違う SkyCoord 同士を安全に比べられるよう ICRS に揃える。"""
    try:
        return coord.icrs
    except Exception:
        return coord


def wcs_summary(wcs, shape):
    """解けた WCS の要点をまとめる。"""
    ny, nx = shape
    scales = proj_plane_pixel_scales(wcs.celestial) * 3600.0
    scale = float(np.mean(scales))
    center = as_icrs(wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0))
    # 画面上向き (+y) が空のどちらを向いているか = 位置角
    up = as_icrs(wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0 + 10))
    pa = center.position_angle(up).to(u.deg).value
    cd = wcs.pixel_scale_matrix
    parity = "正 (鏡像なし)" if np.linalg.det(cd) > 0 else "負 (鏡像)"
    return dict(center=center, scale=scale, pa=pa, parity=parity,
                fov_x=nx * scale / 60.0, fov_y=ny * scale / 60.0)


def print_solution(wcs, shape):
    s = wcs_summary(wcs, shape)
    _log("\n" + "=" * 62)
    _log("  解析結果")
    _log("=" * 62)
    _log(f"  画像中心      : {s['center'].to_string('hmsdms', precision=2)}")
    _log(f"                  (RA {s['center'].ra.deg:.6f}°, Dec {s['center'].dec.deg:.6f}°)")
    _log(f"  画素スケール  : {s['scale']:.4f} 秒角/px")
    _log(f"  視野          : {s['fov_x']:.2f}′ x {s['fov_y']:.2f}′")
    _log(f"  画面上方向    : 位置角 {s['pa']:.2f}°(北から東回り)")
    _log(f"  パリティ      : {s['parity']}")
    return s


def report_offsets(center, target):
    """画像中心 → 目標 のずれを、赤経赤緯の軸ごとに出す。"""
    center, target = as_icrs(center), as_icrs(target)
    sep = center.separation(target)
    d_ra, d_dec = center.spherical_offsets_to(target)
    ra_min = d_ra.to(u.arcmin).value
    dec_min = d_dec.to(u.arcmin).value
    ra_dir = "東" if ra_min > 0 else "西"
    dec_dir = "北" if dec_min > 0 else "南"
    _log("\n  画像中心から目標までのずれ")
    _log(f"    全角距離      : {sep.arcmin:.3f}′  ({sep.arcsec:.1f}″)")
    _log(f"    赤経(RA) 方向 : {abs(ra_min):.3f}′ {ra_dir}へ")
    _log(f"    赤緯(Dec)方向 : {abs(dec_min):.3f}′ {dec_dir}へ")
    return sep, ra_min, dec_min, ra_dir, dec_dir


def _norm(data):
    """
    星が見えるように整える。

    v5 は 1〜99 パーセンタイルを使っていたが、星野では明るい画素が全体の
    1% に満たないので上限がノイズの中に落ち、星が軒並み真っ白になっていた。
    背景の中央値とノイズ σ を基準にする。
    """
    from astropy.stats import sigma_clipped_stats

    finite = np.isfinite(data)
    if not finite.any():
        return None
    vals = data[finite]
    _, med, sigma = sigma_clipped_stats(vals, sigma=3.0)
    if not np.isfinite(sigma) or sigma <= 0:
        sigma = float(np.std(vals)) or 1.0
    lo = float(med) - 1.0 * float(sigma)
    hi = max(float(np.percentile(vals, 99.9)), float(med) + 12.0 * float(sigma))
    if not (hi > lo):
        hi = lo + 1e-6
    return ImageNormalize(vmin=lo, vmax=hi, stretch=AsinhStretch(1.0))


def clip_arrow_to_boundary(x0, y0, x1, y1, xlim, ylim):
    """画面外の目標へ向かう矢印を、画面の縁で止める。"""
    xmin, xmax = min(xlim), max(xlim)
    ymin, ymax = min(ylim), max(ylim)
    if xmin <= x1 <= xmax and ymin <= y1 <= ymax:
        return x1, y1, True
    dx, dy = x1 - x0, y1 - y0
    if abs(dx) < 1e-12 and abs(dy) < 1e-12:
        return x1, y1, True
    ts = []
    if abs(dx) > 1e-12:
        ts += [(xmin - x0) / dx, (xmax - x0) / dx]
    if abs(dy) > 1e-12:
        ts += [(ymin - y0) / dy, (ymax - y0) / dy]
    best = None
    for t in ts:
        if t <= 1e-9:
            continue
        ix, iy = x0 + t * dx, y0 + t * dy
        if xmin - 1e-6 <= ix <= xmax + 1e-6 and ymin - 1e-6 <= iy <= ymax + 1e-6:
            best = t if best is None else min(best, t)
    if best is None:
        return x1, y1, False
    return x0 + best * 0.97 * dx, y0 + best * 0.97 * dy, False


def _draw_markers(ax, wcs, shape, target, label_target=True, draw_arrow=True):
    ny, nx = shape
    center = as_icrs(wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0))
    sx, sy = wcs.world_to_pixel(center)
    ax.scatter(sx, sy, color="cyan", marker="x", s=180, linewidths=2.0, zorder=5)
    if target is None or not draw_arrow:
        return None
    target = as_icrs(target)

    tx, ty = wcs.world_to_pixel(target)
    if not np.all(np.isfinite([tx, ty])):
        # 反対の空を指しているなど、投影が破綻するとき: 方向だけ求めて遠くを指す
        pa = center.position_angle(target)
        near = center.directional_offset_by(pa, 1 * u.arcmin)
        nxp, nyp = wcs.world_to_pixel(near)
        tx, ty = sx + (nxp - sx) * 1e5, sy + (nyp - sy) * 1e5

    cx, cy, inside = clip_arrow_to_boundary(sx, sy, tx, ty,
                                            ax.get_xlim(), ax.get_ylim())
    ax.annotate("", xy=(cx, cy), xytext=(sx, sy), zorder=6,
                arrowprops=dict(facecolor="red", edgecolor="red",
                                width=2.4, headwidth=13, headlength=16,
                                alpha=0.9))
    if inside:
        ax.scatter(tx, ty, s=520, facecolors="none", edgecolors="yellow",
                   linewidths=2.0, zorder=6)
        if label_target:
            ax.annotate("TARGET", xy=(tx, ty), xytext=(14, 14),
                        textcoords="offset points", color="yellow",
                        fontsize=12, fontweight="bold", zorder=7)
    return inside


def plot_original(wcs, data, target, flip_y, title_extra="", sources=None,
                  draw_arrow=True):
    """撮影したときと同じ向きの画像に、目標への矢印を描く。"""
    ny, nx = data.shape
    fig = plt.figure(figsize=(12, 12 * ny / nx + 1.2))
    ax = fig.add_subplot(111, projection=wcs)
    ax.imshow(data, origin="lower", cmap="gray", norm=_norm(data))
    ax.set_xlim(-0.5, nx - 0.5)
    ax.set_ylim(-0.5, ny - 0.5)
    if flip_y:
        ax.invert_yaxis()      # 表示だけ上下反転 (座標系はそのまま)
    ax.coords.grid(color="deepskyblue", linestyle="--", linewidth=0.9, alpha=0.65)
    ax.coords[0].set_axislabel("赤経 (RA)")
    ax.coords[1].set_axislabel("赤緯 (Dec)")

    if sources:
        ax.scatter([s["x"] for s in sources], [s["y"] for s in sources],
                   s=90, facecolors="none", edgecolors="lime",
                   linewidths=0.8, alpha=0.55, zorder=4)

    _draw_markers(ax, wcs, data.shape, target, draw_arrow=draw_arrow)
    subtitle = (f"目標: {target.to_string('hmsdms', precision=2)}"
                if (target is not None and draw_arrow) else "目標未指定 (中心のみ表示)")
    ax.set_title(f"撮影時の向き{title_extra}\n{subtitle}", size=14)
    plt.tight_layout()
    plt.show()


def plot_north_up(wcs, data, target):
    """北を上・東を左にそろえた画像を作る。赤道儀の軸操作と向きが揃う。"""
    from reproject import reproject_interp

    ny, nx = data.shape
    scale = float(np.mean(proj_plane_pixel_scales(wcs.celestial)))
    center = as_icrs(wcs.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0))

    out = WCS(naxis=2)
    out.wcs.ctype = ["RA---TAN", "DEC--TAN"]
    out.wcs.crval = [center.ra.deg, center.dec.deg]
    out.wcs.cdelt = np.array([-scale, scale])
    out.wcs.crpix = [1.0, 1.0]

    corners_x = np.array([0, nx - 1, 0, nx - 1], dtype=float)
    corners_y = np.array([0, 0, ny - 1, ny - 1], dtype=float)
    sky = wcs.pixel_to_world(corners_x, corners_y)
    px, py = out.world_to_pixel(sky)
    # FITS の CRPIX は 1 始まりなので +1 する (v5 はここが 1px ずれていた)
    out.wcs.crpix = [1.0 - float(np.min(px)), 1.0 - float(np.min(py))]
    shape_out = (int(np.ceil(np.ptp(py))) + 1, int(np.ceil(np.ptp(px))) + 1)

    # 大きな画像では、この再投影が全体でいちばんメモリを食う。
    # block_size を渡すと小分けに処理する。実測 (19.1 メガ画素):
    #   そのまま 2,581 MB / 7.1 秒  →  小分け 557 MB / 8.8 秒
    # 結果は同一 (平均も有効画素数も一致)。時間は 2 割増だが、
    # 一眼レフの画像でメモリが足りずに落ちるほうが困る。
    try:
        rep, foot = reproject_interp((np.nan_to_num(data), wcs), out,
                                     shape_out=shape_out,
                                     block_size=(512, 512))
    except TypeError:            # 古い reproject には block_size が無い
        rep, foot = reproject_interp((np.nan_to_num(data), wcs), out,
                                     shape_out=shape_out)
    rep = np.where(foot > 0, rep, np.nan)

    fig = plt.figure(figsize=(12, 12 * shape_out[0] / shape_out[1] + 1.2))
    ax = fig.add_subplot(111, projection=out)
    ax.imshow(rep, origin="lower", cmap="gray", norm=_norm(rep))
    ax.coords.grid(color="deepskyblue", linestyle="--", linewidth=0.9, alpha=0.65)
    ax.coords[0].set_axislabel("赤経 (RA)")
    ax.coords[1].set_axislabel("赤緯 (Dec)")

    _draw_markers(ax, out, shape_out, target)

    sep, ra_min, dec_min, ra_dir, dec_dir = report_offsets(center, target)
    ax.set_title(f"北が上・東が左\n"
                 f"距離 {sep.arcmin:.2f}′ | "
                 f"RA {abs(ra_min):.2f}′({ra_dir}) "
                 f"Dec {abs(dec_min):.2f}′({dec_dir})", size=14)
    plt.tight_layout()
    plt.show()
    return sep, ra_min, dec_min

def setup_japanese_font():
    """図の日本語が豆腐 (□□□) にならないようフォントを選ぶ。"""
    import matplotlib.font_manager as fm
    have = {f.name for f in fm.fontManager.ttflist}
    for name in ("Yu Gothic", "Meiryo", "MS Gothic", "Noto Sans CJK JP",
                 "IPAexGothic", "Hiragino Sans"):
        if name in have:
            matplotlib.rcParams["font.family"] = name
            matplotlib.rcParams["axes.unicode_minus"] = False
            return name
    return None


ZAHYOU_FONT = setup_japanese_font()

# 選ばれた画像はここに入る。セル C から参照される。
ZAHYOU_PICKED = {"path": None, "name": None}


def _widget_files(w):
    """ipywidgets 7 系 (dict) と 8 系 (tuple) の両方に対応する。"""
    v = w.value
    if not v:
        return []
    if isinstance(v, dict):                      # v7: {filename: {...}}
        return [{"name": k, "content": d["content"]} for k, d in v.items()]
    return [{"name": f["name"], "content": f["content"]} for f in v]


def pick_image():
    """
    画像を選ぶボタンを出す。選んだファイルは ZAHYOU_PICKED に入る。

    戻り値は None にしておくこと。ウィジェットを返すと、display() で出した分と
    「セルの最後の式の値」として自動表示される分の 2 つが並んでしまう。
    """
    import ipywidgets as widgets
    from IPython.display import display

    status = widgets.Output()
    uploader = widgets.FileUpload(
        accept=".fits,.fit,.fts,.jpg,.jpeg,.png,.tif,.tiff",
        multiple=False,
        description="画像ファイルを選択",
        button_style="primary",
        layout=widgets.Layout(width="260px"),
    )

    def on_upload(_change):
        files = _widget_files(uploader)
        if not files:
            return
        f = files[0]
        suffix = os.path.splitext(f["name"])[1] or ".fits"
        tmp = tempfile.NamedTemporaryFile(delete=False, suffix=suffix,
                                          prefix="zahyou_")
        tmp.write(bytes(f["content"]))
        tmp.close()
        ZAHYOU_PICKED["path"] = tmp.name
        ZAHYOU_PICKED["name"] = f["name"]
        uploader.button_style = "success"
        with status:
            status.clear_output(wait=True)
            print(f"✅ 選択しました: {f['name']}  ({len(f['content']) / 1024:.0f} KB)")

    uploader.observe(on_upload, names="value")
    display(widgets.VBox([uploader, status]))
    print("画像を選んだら、次のセル C を実行してください。")
    print("※ ボタンを使わず、セル C の IMAGE_PATH にパスを直接書いても構いません。")
    return None

# ============================================================== 実行本体 ===

def _pick_image_path():
    if IMAGE_PATH:
        if not os.path.exists(IMAGE_PATH):
            raise FileNotFoundError(f"IMAGE_PATH が見つかりません: {IMAGE_PATH}")
        return IMAGE_PATH, os.path.basename(IMAGE_PATH)
    picked = globals().get("ZAHYOU_PICKED") or {}
    if picked.get("path"):
        return picked["path"], picked.get("name") or os.path.basename(picked["path"])
    raise RuntimeError(
        "画像が選ばれていません。1 つ目のセルのボタンで選ぶか、"
        "IMAGE_PATH にファイルのパスを書いてください。")


def _resolve_target(online):
    """設定から目標の SkyCoord を作る。作れなければ None。"""
    # 完全ブラインド: 目標を決めずに、画像がどこを向いているかだけ知りたいとき。
    # 天体名が空のときも同じ扱いにする (打ち忘れで止まるより親切)。
    if INPUT_MODE == 'NONE' or (INPUT_MODE == 'STAR_NAME'
                                and not str(TARGET_STAR_NAME or '').strip()):
        _log("  目標は指定されていません。画像がどこを向いているかだけ求めます。")
        return None
    if INPUT_MODE == 'COORDS':
        try:
            return SkyCoord(RA_INPUT_STR, DEC_INPUT_STR, frame='icrs')
        except Exception as e:
            _log(f"❌ 赤経赤緯を読み取れません ({e})。書式を確認してください。")
            return None
    if INPUT_MODE == 'STAR_NAME':
        ra, dec = resolve_target_name(TARGET_STAR_NAME, online=online)
        if ra is not None:
            return SkyCoord(ra=ra * u.deg, dec=dec * u.deg, frame='icrs')
        if RA_INPUT_STR and DEC_INPUT_STR:
            try:
                c = SkyCoord(RA_INPUT_STR, DEC_INPUT_STR, frame='icrs')
                _log("  → 代わりに RA_INPUT_STR / DEC_INPUT_STR の座標を使います。")
                return c
            except Exception:
                pass
        return None
    _log(f"❌ INPUT_MODE が不正です: {INPUT_MODE!r} ('STAR_NAME' か 'COORDS')")
    return None


def _existing_wcs_header(header, shape):
    """画像に元から入っている WCS が使えるなら、その FITS ヘッダを返す。"""
    try:
        h = normalize_wcs_header(header, shape)
        w = WCS(h)
        if not w.is_celestial:
            return None
        s = float(np.mean(proj_plane_pixel_scales(w.celestial)) * 3600.0)
        return h if 0.01 < s < 3600 else None
    except Exception:
        return None


def _sanity_ok(wcs, shape):
    """解いた結果がまともかどうか、描画の前に確かめる。"""
    try:
        s = float(np.mean(proj_plane_pixel_scales(wcs.celestial)) * 3600.0)
        c = wcs.pixel_to_world(shape[1] / 2.0, shape[0] / 2.0)
        return (0.01 < s < 3600) and np.isfinite(c.ra.deg) and np.isfinite(c.dec.deg)
    except Exception as e:
        _log(f"  ⚠️ WCS の検証に失敗しました: {e}")
        return False


def run():
    t0 = time.time()
    img_path, img_name = _pick_image_path()
    _log("=" * 62)
    _log(f"  zahyou v6   {img_name}")
    _log("=" * 62)

    # ---------------------------------------------------------------- 画像 ---
    bundle = load_image_any(img_path)
    ny, nx = bundle.shape
    _log(f"読み込み : {nx} x {ny} 画素   [{bundle.note}]")

    # ------------------------------------------------------------ 星の検出 ---
    # オンラインでもオフラインでも同じ星のリストを使う。
    # v5 は 1〜99 パーセンタイルで 8bit PNG に変換していたため、星が軒並み
    # 真っ白に飽和し、ノイズと区別がつかなくなって解けなくなっていた。
    _log("\n星を検出しています...")
    sub, mask, sigma, info = preprocess(bundle.data)
    for line in info:
        _log(f"  - {line}")
    sources, thr = detect_sources(sub, mask, sigma)
    if sources:
        fwhm = float(np.median([s["fwhm"] for s in sources]))
        _log(f"  - 星を {len(sources)} 個検出 "
             f"(しきい値 {thr:g}σ / 典型 FWHM {fwhm:.1f} px)")
        if len(sources) < 8:
            _log("  ⚠️ 星が少なすぎます。露出を伸ばすか複数フレームを重ねてください。")
    else:
        _log("  ⚠️ 星を検出できませんでした。露出・ピント・雲を確認してください。")

    # ------------------------------------------------------------- つながり ---
    if SOLVE_MODE == 'ONLINE':
        online = True
    elif SOLVE_MODE == 'OFFLINE':
        online = False
    else:
        online = internet_available()
    _log("\n🌍 オンライン環境で実行します。" if online else "\n🔌 オフライン環境で実行します。")

    # ---------------------------------------------------------------- 目標 ---
    target = _resolve_target(online)
    hint = (target.ra.deg, target.dec.deg) if (target is not None and USE_TARGET_AS_HINT) else None
    scale_hint, scale_why = estimate_pixel_scale(bundle.header, FOCAL_LENGTH_MM,
                                                 bundle.shape)

    # ---------------------------------------------------------------- 解析 ---
    wcs_header, how = None, ""

    if not IGNORE_EXISTING_WCS:
        h = _existing_wcs_header(bundle.header, bundle.shape)
        if h is not None:
            _log("\n✅ 画像に入っていた WCS をそのまま使います。")
            wcs_header, how = h, "画像に入っていた WCS"

    work_dir = tempfile.mkdtemp(prefix="zahyou_solve_")
    try:
        if wcs_header is None and online:
            _log(f"\n[オンライン] nova.astrometry.net に問い合わせます "
                 f"(最大 {ONLINE_TIMEOUT} 秒)")
            if scale_hint:
                _log(f"  - 画素スケールの手がかり: {scale_hint:.3f}″/px ({scale_why})")
            if hint:
                _log(f"  - 目標の方向 (半径 {SEARCH_RADIUS_DEG:g}°) を手がかりにします。")
            try:
                wcs_header = solve_online(
                    img_path, API_KEY, sources=sources, shape=bundle.shape,
                    timeout=ONLINE_TIMEOUT, hint_radec=hint,
                    hint_radius_deg=SEARCH_RADIUS_DEG, scale_hint=scale_hint)
            except Exception as e:
                _log(f"⚠️ オンライン解析でエラー: {type(e).__name__}: {e}")
                wcs_header = None
            if wcs_header is not None:
                _log("✅ オンライン解析に成功しました。")
                how = "オンライン (nova.astrometry.net)"
            elif SOLVE_MODE == 'ONLINE':
                _log("SOLVE_MODE='ONLINE' なのでここで終了します。")
            else:
                _log("   → ローカル (WSL) の解析に切り替えます。")

        if wcs_header is None and SOLVE_MODE != 'ONLINE':
            _log("\n[オフライン] WSL の astrometry.net を確認しています...")
            diag = solver_diagnostics()
            for m in diag["messages"]:
                _log(f"⚠️ {m}")
            if diag["ok"]:
                _log(f"  - solve-field OK / index ファイル {len(diag['indexes'])} 個")
                wcs_header = solve_offline(
                    bundle, sources, work_dir, timeout=OFFLINE_TIMEOUT,
                    focal_length_mm=FOCAL_LENGTH_MM, hint_radec=hint,
                    hint_radius_deg=SEARCH_RADIUS_DEG, diagnostics=diag)
                if wcs_header is not None:
                    how = "オフライン (WSL の astrometry.net)"

        if wcs_header is None:
            _log("\n❌ 座標を特定できませんでした。")
            return None

        # ------------------------------------------------------------ 検算 ---
        wcs = WCS(normalize_wcs_header(wcs_header, bundle.shape))
        if not _sanity_ok(wcs, bundle.shape):
            _log("❌ 解析結果がおかしいので採用しません。")
            return None

        summary = print_solution(wcs, bundle.shape)
        _log(f"  解いた方法    : {how}")
        _log(f"  所要時間      : {time.time() - t0:.1f} 秒")

        # 次回この機材で撮った画像は、設定を書かなくても一発で絞り込める
        f_mm = implied_focal_length(bundle.header, summary["scale"])
        if f_mm:
            _log(f"  逆算した焦点距離: {f_mm:.0f} mm")
        if scale_hint is None:
            _log("  → この機材の画素スケールを記憶しました "
                 "(次からは画角の総当たりをしません)")
        remember_scale(bundle.header, bundle.shape, summary["scale"])

        # ------------------------------------------------------------ 描画 ---
        shown = sources if SHOW_DETECTED_SOURCES else None

        if target is None:
            _log("\n⚠️ 目標の座標が無いので、矢印は描きません。")
            _log("   オフラインで天体名を使いたいときは、一度オンラインで実行して"
                 "座標を覚えさせるか、INPUT_MODE='COORDS' にしてください。")
            plot_original(wcs, bundle.display, None, bundle.flip_y,
                          sources=shown, draw_arrow=False)
            return wcs

        tx, ty = wcs.world_to_pixel(target)
        if np.all(np.isfinite([tx, ty])) and 0 <= tx < nx and 0 <= ty < ny:
            _log(f"\n  目標は画面内です: ピクセル ({float(tx):.1f}, {float(ty):.1f})")
            if sources:
                d = [float(np.hypot(s["x"] - tx, s["y"] - ty)) for s in sources]
                i = int(np.argmin(d))
                near_px = max(4.0, 3.0 * float(np.median([s["fwhm"] for s in sources])))
                if d[i] <= near_px:
                    _log(f"  検出した星と {d[i]:.1f} px "
                         f"({d[i] * summary['scale']:.1f}″) で一致しました。")
                else:
                    _log("  目標そのものは、この検出しきい値では拾えていません "
                         "(暗い星なら正常です)。")
        else:
            _log("\n  目標は画面の外です。矢印の向きへ動かしてください。")

        plot_original(wcs, bundle.display, target, bundle.flip_y, sources=shown)
        plot_north_up(wcs, bundle.display, target)
        return wcs

    finally:
        shutil.rmtree(work_dir, ignore_errors=True)

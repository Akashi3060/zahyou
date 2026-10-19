# ==============================================================================
#  zahyou  解析エンジン
#
#  役割 : どんな天体画像 (FITS / PNG / JPEG / TIFF / 動画キューブ) が来ても
#         「星の座標リスト」を作り、自前の解析エンジン (solver_*.py) で確実に解く。
#         WSL も astrometry.net の実行ファイルも使わない。星図データ (index-*.fits)
#         だけを読む。
# ==============================================================================

from __future__ import annotations

import concurrent.futures
import json
import math
import os
import shutil
import tempfile
import time
import warnings

import numpy as np
from astropy.io import fits
from astropy.stats import sigma_clipped_stats
from astropy.wcs import WCS

try:
    from scipy import ndimage
except ImportError:                                   # pragma: no cover
    ndimage = None

# 自前の解析エンジン (sv_solve / SvIndexCatalog / SvCancelled)。
# zahyou_engine.py に組み立てるときはこの import ごと消え、同じ名前空間に入った
# solver_*.py の中身をそのまま使う。ここが効くのは、このファイルを単体で import
# したとき (dev/test_robust.py など) だけ。読み込み・検出だけの試験は solver_*.py が
# 無くても動くよう、solver そのものが無いときだけは黙って先へ進む。
try:
    from solver_core import *                         # noqa: F401,F403
except ModuleNotFoundError as _e:                     # pragma: no cover
    if _e.name not in ("solver_core", "solver_index", "solver_wcs"):
        raise


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
        # 2x2 の平均 (v6 まで) は窓が片側に寄るので、星の位置が x・y とも +0.5 px
        # ずれていた。対称な 3x3 の二項カーネル [1,2,1]⊗[1,2,1]/16 ならずれない。
        # どの画素を中心にしても R:G:B = 1:2:1 の重みになるので、2x2 の平均と
        # 同じく市松模様も消える。
        k = np.array([0.25, 0.5, 0.25], dtype=np.float32)
        data = ndimage.correlate1d(data, k, axis=0, mode="nearest")
        data = ndimage.correlate1d(data, k, axis=1, mode="nearest")
        notes.append(f"ベイヤー配列 ({pat}) を 3x3 の二項フィルターで平滑化")

    bad = ~np.isfinite(data)
    if bad.any():
        good = data[~bad]
        data[bad] = float(np.median(good)) if good.size else 0.0
        notes.append(f"非有限値 {int(bad.sum())} 画素を中央値で補完")

    if float(np.ptp(data)) <= 0:
        raise ValueError("画像が一様です (全画素が同じ値)。露出やファイルを確認してください。")

    # 星図 (白地に黒い星) は明暗が逆。星の検出は「明るい点」を
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
    解析エンジンは 4 個で 1 quad を作るが、実用上は 10 個以上ほしい。
    run() は min_sources=60 で呼ぶ (自前のソルバーは星が多いほど quad を選べる)。

    min_snr を 3.0 に下げると暗い画像で星は増えるが (実測 8 → 15 個)、
    疎な星野では雑音のこぶも同じだけ増える (合成テストで純度 100% → 30%)。
    どの値でも「本物だけ増やす」ことはできなかったので 5.0 のままにしてある。
    これは図に緑の丸で出す星で、確かなものだけを残す。解析エンジンには、暗い星まで
    拾う detect_sources_deep の星を先に渡す (solve_offline)。

    しきい値の段の「σ」は smooth_sigma = sigma / 2 を単位にしているが、σ=1 の
    ガウスでぼかした白色雑音の実際の標準偏差は sigma / (2√π) ≈ 0.28 sigma
    (3I の画像の実測 0.31 sigma)。つまり最後の段 2.0 は実際には約 3.2σ で、
    書いてある値より 1.6〜1.8 倍高い。図の星を変えないために、このままにしてある。
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


# ------------------------------------------------ 解析エンジンに渡す星 (深い検出) ---
#
# detect_sources は図に出す星なので、領域の面積と開口 SNR で確かなものだけを残す。
# 星の少ない画像ではそれが数個にとどまり、解析エンジンの検証 (対数オッズ) の
# 証拠が足りなくなる (3I: 星 8 個で対数オッズ 21.6〜21.9、合格は 20.7。この検出の星 15 個で 47.7)。
# 解析エンジンに渡す星は次の考え方で別に拾う (solve-field の image2xy と同じ方針。コードは自前):
#   背景を引く → 星の幅に合わせてガウスでぼかす (整合フィルター) →
#   局所の雑音の u 倍を超える局所の極大を星とする。
# u は「雑音だけの画像で偽の極大が画像全体で 2 個ほど」になる値にする (下の式)。
# 偽の星が多いと検証で不利になるので、数は増やしすぎない。

_DEEP_FALSE_RATE = 0.045        # 偽の極大の数の係数 (白色雑音で数えて合わせた値)


def _block_map(img, valid, box, fn):
    """
    box 画素の升ごとに fn (升の値を最後の軸に並べた配列 → 升ごとの値) を計算し、
    3x3 の中央値でならしてから升の中心を節点に双一次で引き伸ばす (外側は端の値)。
    マスクの画素は NaN にして渡す。升が縦横 3 つ未満なら None。
    """
    ny, nx = img.shape
    by, bx = ny // box, nx // box
    if by < 3 or bx < 3:
        return None
    h, w = by * box, bx * box
    oy, ox = (ny - h) // 2, (nx - w) // 2
    t = np.where(valid, img, np.nan)[oy:oy + h, ox:ox + w].astype(np.float32)
    t = t.reshape(by, box, bx, box).transpose(0, 2, 1, 3).reshape(by, bx, box * box)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")              # 全部マスクの升 (All-NaN slice)
        coarse = fn(t)
    good = np.isfinite(coarse)
    if not good.any():
        return None
    coarse = np.where(good, coarse, float(np.median(coarse[good])))
    coarse = ndimage.median_filter(coarse, size=3, mode="nearest")
    fy = np.clip((np.arange(ny) - (oy + 0.5 * box - 0.5)) / box, 0, by - 1)
    fx = np.clip((np.arange(nx) - (ox + 0.5 * box - 0.5)) / box, 0, bx - 1)
    iy = np.minimum(fy.astype(int), by - 2)
    wy = (fy - iy)[:, None]
    rows = coarse[iy] * (1 - wy) + coarse[iy + 1] * wy
    ix = np.minimum(fx.astype(int), bx - 2)
    wx = (fx - ix)[None, :]
    return (rows[:, ix] * (1 - wx) + rows[:, ix + 1] * wx).astype(np.float32)


def _median_last(t):
    """最後の軸の中央値 (NaN を除く)。NaN が無ければ速い np.median で済ませる。"""
    if np.isnan(t).any():
        return np.nanmedian(t, axis=-1)
    return np.median(t, axis=-1)


def _mad_sigma_last(t):
    """最後の軸の MAD から出した標準偏差。有効な画素が 1/4 未満の升は NaN。"""
    med = _median_last(t)
    mad = _median_last(np.abs(t - med[..., None]))
    n = np.sum(np.isfinite(t), axis=-1)
    return np.where(n >= t.shape[-1] // 4, 1.4826 * mad, np.nan)


def _clipped_stats(values, limit=500_000):
    """sigma_clipped_stats (3σ) の (中央値, 標準偏差)。多いときは等間隔に間引いて速くする。"""
    values = np.ravel(values)
    if values.size > limit:
        values = values[::int(math.ceil(values.size / limit))]
    _, med, std = sigma_clipped_stats(values, sigma=3.0, maxiters=5)
    return float(med), float(std)


def _sharpness(work, y, x):
    """中心の値 / 上下左右の平均。ベイヤーの補間で十字に広がったホットピクセルは大きい。"""
    n4 = (float(work[y - 1, x]) + float(work[y + 1, x])
          + float(work[y, x - 1]) + float(work[y, x + 1])) / 4.0
    return float(work[y, x]) / n4 if n4 > 0 else math.inf


def _psf_sigma(sm1, work, valid, ns1, sigma, max_stars=40, min_snr=10.0):
    """
    明るい星 (σ=1 でぼかした画像で雑音の min_snr 倍以上) の半値の面積から、
    星像の σ を測る。ぼかした分 (σ=1) は二乗で引く。
    戻り値 (σ [画素] か None, 使った星の数, 鋭さの上限)。
    鋭さ (_sharpness) が明るい星の典型 (下から 1/4) の 2 倍を超える点は
    ホットピクセルとみなして幅の計算に使わない。頂が平ら (飽和) な星も使わない。
    """
    ny, nx = sm1.shape
    mx = ndimage.maximum_filter(sm1, size=7)
    pk = (sm1 == mx) & (sm1 > min_snr * ns1) & valid
    pk[:2] = pk[-2:] = False
    pk[:, :2] = pk[:, -2:] = False
    ys, xs = np.nonzero(pk)
    if len(ys) < 3:
        return None, 0, math.inf
    found = []                                      # (σ, 鋭さ か None)
    # 同じ値の候補は元の順に(安定な並べ替え。同点の順が numpy の実装や CPU で変わらないように)
    for i in np.argsort(-sm1[ys, xs], kind="stable")[:max_stars]:
        y, x = int(ys[i]), int(xs[i])
        v = float(sm1[y, x])
        sharp = _sharpness(work, y, x) if work[y, x] > 10.0 * sigma else None
        for r in (8, 16, 32):                       # 半値の領域が窓に収まるまで広げる
            y0, y1 = max(0, y - r), min(ny, y + r + 1)
            x0, x1 = max(0, x - r), min(nx, x + r + 1)
            t = sm1[y0:y1, x0:x1]
            lab, _ = ndimage.label(t > 0.5 * v)
            k = lab[y - y0, x - x0]
            if k == 0:
                break
            reg = lab == k
            rr, cc = np.nonzero(reg)
            touches = (rr.min() == 0 or cc.min() == 0 or rr.max() == reg.shape[0] - 1
                       or cc.max() == reg.shape[1] - 1)
            if touches:
                if r < 32:
                    continue
                break                               # 65 画素の窓より大きい = 星ではない
            if int((t[reg] > 0.98 * v).sum()) > 4:  # 頂が平ら = 飽和
                break
            fwhm = 2.0 * math.sqrt(float(reg.sum()) / math.pi)
            found.append((math.sqrt(max((fwhm / 2.3548) ** 2 - 1.0, 0.0)), sharp))
            break
    sharps = [s for _, s in found if s is not None]
    limit = max(2.5, 2.0 * float(np.percentile(sharps, 25))) if len(sharps) >= 3 else math.inf
    widths = [w for w, s in found if (s is None or s <= limit) and w > 0.3]
    if len(widths) < 3:
        return None, len(widths), limit
    return float(np.percentile(widths, 40)), len(widths), limit


def _false_peaks(u, area, s):
    """
    雑音だけの画像を σ=s でぼかしたとき、雑音の u 倍を超える局所の極大の数の見込み。
    ガウス場の極大の数の式 (面積 / s² · u · exp(-u²/2) に比例) の係数を、
    白色雑音で数えて合わせた (s = 1〜3、968x548 と 1608x1104 で係数 0.04〜0.045)。
    """
    return _DEEP_FALSE_RATE * area / (s * s) * u * math.exp(-0.5 * u * u)


def _peak_offset(a, b, c):
    """
    3 点 (左, 中央, 右) の極大の位置を -0.5〜0.5 で返す。値が全部正ならガウスに
    ちょうど合う対数の放物線、そうでなければ普通の放物線を当てる。
    """
    if a > 0.0 and b > 0.0 and c > 0.0:
        la, lb, lc = math.log(a), math.log(b), math.log(c)
        den = la - 2.0 * lb + lc
        if den < 0.0:
            return min(max(0.5 * (la - lc) / den, -0.5), 0.5)
    den = a - 2.0 * b + c
    if den < 0.0:
        return min(max(0.5 * (a - c) / den, -0.5), 0.5)
    return 0.0


def detect_sources_deep(sub, mask, sigma, max_sources=400, target_false=2.0, min_u=4.0,
                        info=None):
    """
    深い検出 (解析エンジンに渡す星)。detect_sources より暗い星まで拾う。

      1. σ=1 でぼかした画像の明るい星から、星像の幅 (σ_psf) を測る
      2. 画像を σ = 1.25 σ_psf (測れなければ 1.5、1〜8 に収める) のガウスでぼかし、
         升ごとの中央値で背景を引く。雑音は升ごとの MAD で場所ごとに測る
      3. 局所の極大のうち、その場所の雑音の u 倍を超えるものを候補にする。
         u は _false_peaks が target_false 個 (画像全体) になる値 (min_u 以上)
      4. 明るい順に見て、次のものは捨てる:
         ・もっと明るい星との間の谷が雑音の 3 倍より浅い (同じ星の肩・光条のこぶ)
         ・ぼかした像の曲率で見た伸びが 4 倍を超える (光跡・線)
         ・鋭さが明るい星の典型の 2 倍を超える (ベイヤーで十字に広がったホットピクセル)
         ・まわりの輪 (ぼかした星像の σ の 3 倍の半径) の中央値より雑音の 3 倍以上高くない
           (大きなかぶりの上で、背景の当てはめの残りに雑音が乗っただけの極大。月明かりの
           合成画像で偽の星が 42 → 5 個。前処理の背景は 64 画素の升を双一次で引き伸ばすので、
           急なかぶりでは升の節点に折れ目のこぶが残る)
      5. 位置はぼかした像の 3x3 に (対数の) 放物線を当てて小数まで出す

    戻り値は detect_sources と同じ形 (星のリスト, しきい値 u)。リストは明るい順。
    flux はぼかした像の頂の値から出したガウスの全光量 (並べ替えの目安)、
    snr は頂の値 / その場所の雑音、area は FWHM の円の面積 (目安)。
    info に dict を渡すと、測った値 (s_psf, s, u, 候補の数, 捨てた理由ごとの数, 秒) を入れる。

    以前の版は画像を 2x2 にまとめて detect_sources をかけていたが、効くのは FWHM が
    7 画素以上の星だけだった。ここでは星の幅に合わせてぼかすので、細い星でも太い星でも
    同じように効く。
    """
    if ndimage is None:
        raise RuntimeError("scipy が必要です:  pip install scipy")
    from scipy.spatial import cKDTree

    t_start = time.perf_counter()
    ny, nx = sub.shape
    valid = ~np.asarray(mask, dtype=bool)
    if ny < 16 or nx < 16 or int(valid.sum()) < 256:
        return [], None
    work = np.where(valid, sub, 0.0).astype(np.float32)
    sigma = float(sigma)

    # 1. 星像の幅
    sm1 = ndimage.gaussian_filter(work, 1.0)
    back = _block_map(sm1, valid, 32, _median_last)
    sm1 = sm1 - (back if back is not None else float(np.median(sm1[valid])))
    ns1 = _clipped_stats(sm1[valid])[1]
    if not np.isfinite(ns1) or ns1 <= 0:
        ns1 = 0.28 * sigma
    s_psf, n_psf, sharp_max = _psf_sigma(sm1, work, valid, ns1, sigma)
    del sm1, back
    s = min(max(1.25 * s_psf if s_psf else 1.5, 1.0), 8.0)

    # 2. ぼかす・背景を引く・雑音を測る
    sm = ndimage.gaussian_filter(work, s)
    box = int(max(16, round(8 * s)))
    back = _block_map(sm, valid, box, _median_last)
    sm = sm - (back if back is not None else float(np.median(sm[valid])))
    del back
    # マスクの縁はぼかしでマスクの 0 が混ざって沈むので、ぼかしの幅だけ広げて除く
    bad = (ndimage.binary_dilation(~valid, iterations=int(math.ceil(2 * s)) + 1)
           if not valid.all() else np.zeros(valid.shape, dtype=bool))
    if int((~bad).sum()) < 256:
        return [], None
    med, ns = _clipped_stats(sm[~bad])
    if not np.isfinite(ns) or ns <= 0:
        ns = max(ns1, 1e-12)
    sm -= med
    nmap = _block_map(sm, ~bad, int(max(32, round(16 * s))), _mad_sigma_last)
    if nmap is None:
        nmap = np.full(sm.shape, ns, dtype=np.float32)
    nmap = np.maximum(nmap, 0.7 * ns)              # 星で埋まった升で雑音を低く見積もらない

    # 3. しきい値と候補
    area = float((~bad).sum())
    u = float(min_u)
    while u < 8.0 and _false_peaks(u, area, s) > target_false:
        u += 0.05
    u = round(u, 2)
    mx = ndimage.maximum_filter(sm, size=2 * int(math.ceil(s)) + 1)
    pk = (sm == mx) & (sm > u * nmap) & ~bad
    edge = max(2, int(math.ceil(s)))
    pk[:edge] = False
    pk[-edge:] = False
    pk[:, :edge] = False
    pk[:, -edge:] = False
    ys, xs = np.nonzero(pk)
    del mx, pk
    vals = sm[ys, xs]
    order = np.argsort(-vals, kind="stable")[:max(4000, 10 * max_sources)]
    ys, xs, vals = ys[order], xs[order], vals[order].astype(np.float64)
    n = len(ys)

    # 4. 明るい順に選ぶ
    keep = np.zeros(n, dtype=bool)
    rejected = {"saddle": 0, "elong": 0, "sharp": 0, "ring": 0}
    # 局所の背景を測る輪: ぼかした星像の σ (√(σ_psf² + s²)) の 3 倍の半径。星の光はそこで頂の 1% まで落ちる
    sp = s_psf if s_psf else s / 1.25
    r_ring = 3.0 * math.sqrt(sp * sp + s * s)
    ang = np.linspace(0.0, 2.0 * math.pi, max(16, int(2.0 * math.pi * r_ring)), endpoint=False)
    ring_dx = np.round(r_ring * np.cos(ang)).astype(int)
    ring_dy = np.round(r_ring * np.sin(ang)).astype(int)

    def reach(v):
        # 明るい星の像が雑音に沈むまでの半径の目安 (肩のこぶを探す範囲)
        return min(max(4.0 * s, 2.0 * s * math.sqrt(max(v / (u * ns), 1.0))), 40.0 * s)

    if n:
        pts = np.column_stack([xs, ys]).astype(np.float64)
        tree = cKDTree(pts)
        r_max = reach(vals[0])
        for i in range(n):
            y, x, v = int(ys[i]), int(xs[i]), float(vals[i])
            if work[y, x] > 10.0 * sigma and _sharpness(work, y, x) > sharp_max:
                rejected["sharp"] += 1
                continue
            shoulder = False
            for j in tree.query_ball_point(pts[i], r_max):
                if j >= i or not keep[j]:
                    continue
                d = math.hypot(x - xs[j], y - ys[j])
                if d > reach(vals[j]):
                    continue
                m = int(math.ceil(2 * d)) + 2
                line = ndimage.map_coordinates(
                    sm, [np.linspace(y, ys[j], m), np.linspace(x, xs[j], m)], order=1)
                if v - float(line.min()) < 3.0 * float(nmap[y, x]):
                    shoulder = True
                    break
            if shoulder:
                rejected["saddle"] += 1
                continue
            c = sm[y - 1:y + 2, x - 1:x + 2].astype(np.float64)
            dxx = c[1, 2] - 2.0 * c[1, 1] + c[1, 0]
            dyy = c[2, 1] - 2.0 * c[1, 1] + c[0, 1]
            dxy = (c[2, 2] - c[2, 0] - c[0, 2] + c[0, 0]) / 4.0
            tr, det = dxx + dyy, dxx * dyy - dxy * dxy
            elong = math.inf
            if det > 0 and tr < 0:
                root = math.sqrt(max(tr * tr / 4.0 - det, 0.0))
                l1, l2 = tr / 2.0 - root, tr / 2.0 + root   # どちらも負。l1 が急なほう
                if l2 < 0:
                    elong = math.sqrt(l1 / l2)
            if elong > 4.0:
                rejected["elong"] += 1
                continue
            ry = np.clip(y + ring_dy, 0, ny - 1)
            rx = np.clip(x + ring_dx, 0, nx - 1)
            ring = sm[ry, rx][~bad[ry, rx]]               # マスクの側は沈んでいるので使わない
            if len(ring) >= 8 and v - float(np.median(ring)) < 3.0 * float(nmap[y, x]):
                rejected["ring"] += 1
                continue
            keep[i] = True

    # 5. 位置と明るさ
    fwhm = 2.3548 * sp
    out = []
    for i in np.flatnonzero(keep):
        y, x, v = int(ys[i]), int(xs[i]), float(vals[i])
        c = sm[y - 1:y + 2, x - 1:x + 2].astype(np.float64)
        ox = _peak_offset(c[1, 0], c[1, 1], c[1, 2])
        oy = _peak_offset(c[0, 1], c[1, 1], c[2, 1])
        out.append(dict(x=x + ox, y=y + oy, flux=v * 2.0 * math.pi * (sp * sp + s * s),
                        peak=float(sub[y, x]), area=int(round(math.pi * fwhm * fwhm / 4.0)),
                        elong=1.0, fwhm=fwhm, snr=v / float(nmap[y, x])))
    out.sort(key=lambda c: -c["flux"])
    if info is not None:
        info.update(s_psf=s_psf, n_psf=n_psf, s=s, u=u, noise=ns, candidates=n,
                    kept=len(out), rejected=rejected, sharp_max=sharp_max,
                    seconds=time.perf_counter() - t_start)
    return out[:max_sources], u


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


# ============================================================== 星図データ ===

# 星図データ (astrometry.net 形式の index-*.fits) の既定の置き場所。
# GUI は［準備］タブの欄、ノートブックはセル C の INDEX_DIR で変えられる。
DEFAULT_INDEX_DIR = (r"C:\AstrometryData" if os.name == "nt"
                     else "/usr/share/astrometry")

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
    画角が全く分からないときに探す画素スケールの範囲 [arcsec/px]。

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
        return False, [f"画角 {fov_arcmin:.1f}′ は星図データの範囲 (2′〜2000′) の外なので、"
                       "この画角のままでは解けません。"]
    if need[0] in have:
        return True, []
    lo, hi = _INDEX_SCALES[need[0]]
    series = (5200 if need[0] <= 6 else 4100) + need[0]
    files = f"index-{series}-00〜47.fits" if need[0] <= 6 else f"index-{series}.fits"
    return False, [
        f"画角 {fov_arcmin:.1f}′ に対応する星図データ ({lo:g}′〜{hi:g}′ 段) がありません。",
        f"必要なファイル: {files}",
        _index_get_advice(series),
    ]


def _in_gui():
    """GUI から動いているか。GUI はエンジンの名前空間に CANCEL_EVENT を置く (ノートブックには無い)。"""
    return globals().get("CANCEL_EVENT") is not None


def _index_get_advice(series):
    """足りない段の取り方。GUI は［準備］タブ、ノートブックは配布ページの zip とセル C の INDEX_DIR。"""
    if _in_gui():
        return (f"取得: ［準備］タブの段の一覧で index-{series} に印を付け、"
                "「選んだ段を落とす」を押してください。")
    if series in (5202, 5203, 5204):
        z = f"AstrometryData-index-{series}.zip"
    elif series >= 5205 or 4100 <= series < 5200:
        z = "AstrometryData.zip (基本セット)"
    else:
        return (f"取得: この段は配布ページに zip がありません。原典 "
                f"https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/ から "
                f"index-{series}-00〜47.fits を落として、セル C の INDEX_DIR のフォルダーに"
                "入れてください。")
    return (f"取得: 配布ページ (README の 5 章) から {z} を落として展開し、"
            "セル C の INDEX_DIR のフォルダーに index-*.fits が並ぶようにしてください。")


def _index_place_advice():
    """星図データの置き場所の直し方 (GUI とノートブックで違う)。"""
    if _in_gui():
        return "［準備］タブで星図データを落とすか、置き場所を指定し直してください。"
    return ("配布ページ (README の 5 章) の zip を展開するか、"
            "セル C の INDEX_DIR を星図データのある場所に直してください。")


# 星図データの目録 (SvIndexCatalog) を覚えておく。作るときにヘッダーを 1 本ずつ
# 検査するので、置き場所と中身が前回と同じなら解析のたびに作り直さない。
_INDEX_CATALOG = {"key": None, "catalog": None}


def _index_dir_key(index_dir):
    """置き場所の「中身の目印」: フォルダーの更新時刻と、索引ファイルの名前・大きさ・更新時刻。"""
    d = os.path.abspath(index_dir)
    try:
        dir_mtime = os.stat(d).st_mtime_ns
        files = []
        with os.scandir(d) as it:
            for e in it:
                if e.name.startswith("index-") and e.name.endswith(".fits"):
                    s = e.stat()
                    files.append((e.name, s.st_size, s.st_mtime_ns))
    except OSError:
        return None
    return os.path.normcase(d), dir_mtime, tuple(sorted(files))


def index_catalog(index_dir=None):
    """星図データの目録 (SvIndexCatalog) を返す。置き場所と中身が前回と同じなら使い回す。"""
    index_dir = index_dir or DEFAULT_INDEX_DIR
    key = _index_dir_key(index_dir)
    if (key is not None and key == _INDEX_CATALOG["key"]
            and _INDEX_CATALOG["catalog"] is not None):
        return _INDEX_CATALOG["catalog"]
    release_index_catalog()
    catalog = SvIndexCatalog(index_dir)
    _INDEX_CATALOG.update(key=key, catalog=catalog)
    return catalog


def release_index_catalog():
    """
    覚えている目録を手放す (索引ファイルの mmap を閉じる)。

    Windows では mmap 中のファイルを消したり置き換えたりできない。
    GUI は星図データを落とし直す前にこれを呼ぶ。
    """
    catalog = _INDEX_CATALOG["catalog"]
    _INDEX_CATALOG.update(key=None, catalog=None)
    close = getattr(catalog, "close", None)
    if callable(close):
        try:
            close()
        except Exception:
            pass


def _catalog_names(catalog):
    """目録に入っている (検査に通った) 索引ファイルの名前の一覧。"""
    names = set()
    for f in getattr(catalog, "files", None) or []:
        names.add(os.path.basename(str(getattr(f, "path", f))))
    return sorted(names)


def _problem_text(p):
    """目録が読み飛ばしたファイル 1 つぶんの説明。(パス, 理由) でも文字列でも受ける。"""
    if isinstance(p, (tuple, list)) and len(p) >= 2:
        return f"{os.path.basename(str(p[0]))}: {p[1]}"
    return str(p)


def solver_diagnostics(index_dir=None):
    """
    星図データが本当に使えるかを事前に確かめる。置き場所を走査し、ヘッダーを検査する。

    戻り値の形は v6 (WSL 版) と同じ {"ok", "messages", "indexes", "cfg", "paths"}。
    indexes は検査に通ったファイルの名前、cfg は空、paths は置き場所だけ。
    """
    index_dir = index_dir or DEFAULT_INDEX_DIR
    rep = {"ok": False, "messages": [], "indexes": [], "cfg": "", "paths": [index_dir]}
    if not os.path.isdir(index_dir):
        rep["messages"].append(
            f"星図データの置き場所 {index_dir} がありません。" + _index_place_advice())
        return rep
    try:
        catalog = index_catalog(index_dir)
    except Exception as e:
        rep["messages"].append(
            f"星図データを読めませんでした ({type(e).__name__}: {e})。")
        return rep

    rep["indexes"] = _catalog_names(catalog)
    problems = list(getattr(catalog, "problems", None) or [])
    if problems:
        rep["messages"].append(
            f"使えない星図データが {len(problems)} 個あるので読み飛ばします。"
            + ("［準備］タブで落とし直してください。" if _in_gui() else
               "配布ページの zip から入れ直してください。"))
        for p in problems[:5]:
            rep["messages"].append("  " + _problem_text(p))
        if len(problems) > 5:
            rep["messages"].append(f"  ほか {len(problems) - 5} 個")
    if not rep["indexes"]:
        rep["messages"].append(
            f"使える星図データ (index-*.fits) が {index_dir} に 1 つもありません。"
            + _index_place_advice())
        return rep

    rep["ok"] = True
    return rep


# ================================================================ オフライン解析 ===

def _cancel_checker(cancel):
    """中止の合図を「呼ぶと True/False を返す関数」にそろえる (threading.Event でも関数でも受ける)。"""
    if cancel is None:
        return lambda: False
    if callable(getattr(cancel, "is_set", None)):
        return cancel.is_set
    return lambda: bool(cancel())


def _solution_comments(sol):
    """ヘッダーの COMMENT に残す解の素性 (FITS のカードは ASCII だけなので英語で書く)。"""
    def num(attr, fmt):
        v = getattr(sol, attr, None)
        try:
            return fmt.format(v) if v is not None else "?"
        except (TypeError, ValueError):
            return "?"
    return ["Solved by the zahyou native solver (after Lang et al. 2010, AJ 139, 1782).",
            f"index {getattr(sol, 'index_name', '?')}, "
            f"log-odds {num('logodds', '{:.2f}')}, matched {num('nmatch', '{:d}')} stars, "
            f"{num('quads_tried', '{:d}')} quads tried"]


def _solution_header(sol, width, height):
    """
    sv_solve の解 (SvSolution) を astropy で読める FITS ヘッダーにする。

    カードは SvWcs.to_header_cards が作る ([(キー, 値, 注記)]。CRPIX はそこで 1 始まりになる)。
    ソルバー側は astropy を使わない約束なので、fits.Header にするのはこちらの仕事。
    """
    cards = sol.wcs.to_header_cards(int(width), int(height),
                                    comments=_solution_comments(sol))
    h = fits.Header()
    for key, value, comment in cards:
        if key == "COMMENT":
            h.add_comment(str(value))
        elif key == "HISTORY":
            h.add_history(str(value))
        else:
            h[key] = (value, comment or "")
    if not WCS(h).is_celestial:
        raise ValueError("天球座標の WCS になっていません")
    return h


def _solution_note(sol):
    """解の中身を 1 行に (索引・一致した星・対数オッズ・試した quad・時間)。"""
    parts = []
    name = getattr(sol, "index_name", None)
    if name:
        parts.append(f"索引 {name}")
    for attr, fmt in (("nmatch", "一致 {:d} 個"), ("logodds", "対数オッズ {:.1f}"),
                      ("quads_tried", "quad {:,d} 個"), ("seconds", "{:.1f} 秒")):
        v = getattr(sol, attr, None)
        if v is None:
            continue
        try:
            parts.append(fmt.format(int(v) if "d}" in fmt else float(v)))
        except (TypeError, ValueError):
            continue
    return " / ".join(parts)


def solve_offline(bundle, sources, work_dir, timeout=300, focal_length_mm=None,
                  hint_radec=None, hint_radius_deg=5.0, verbose=True,
                  diagnostics=None, index_dir=None, prep=None, cancel=None):
    """
    オフライン (zahyou 自前の解析エンジン sv_solve) で解く。条件を変えながら粘り強く挑む。
    星は深い検出 (detect_sources_deep) の星を先に、sources (図に出す標準の検出の星) を
    予備に使う。戻り値: WCS ヘッダ (CRPIX は 1 始まり) or None。

    例外は投げない (中止したときも None)。全体の締め切りは timeout 秒の壁時計で、
    試行ごとに残り時間の一部を割り当て、その締め切りを sv_solve に渡す。
    v6 (WSL) は試行の打ち切りが --cpulimit 任せで、合計が 45 秒ほどはみ出していた。

      prep   : run() が作った前処理の結果 (sub, mask, sigma)。深い検出に使う
      cancel : 中止の合図 (threading.Event か、呼ぶと True を返す関数)
      work_dir は使わない (v6 の引数の並びを保つために残してある)
    """
    try:
        return _solve_offline(bundle, sources, timeout, focal_length_mm, hint_radec,
                              hint_radius_deg, verbose, index_dir, prep,
                              _cancel_checker(cancel))
    except Exception as e:
        _log(f"\n  ❌ オフライン解析が途中で失敗しました: {type(e).__name__}: {e}")
        return None
    finally:
        # 索引ファイルの mmap を放す (目録とヘッダーの検査結果は覚えたまま。次の解析で開き直す)。
        # 開いたままだと、GUI を開いている間は Explorer やほかのプロセスから星図データを
        # 消したり置き換えたりできない (Windows)
        cat = _INDEX_CATALOG["catalog"]
        if cat is not None:
            try:
                cat.close()
            except Exception:
                pass


def _solve_offline(bundle, sources, timeout, focal_length_mm, hint_radec,
                   hint_radius_deg, verbose, index_dir, prep, stopped):
    t_end = time.monotonic() + float(timeout)
    ny, nx = bundle.shape
    sources = list(sources or [])

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

    catalog = index_catalog(index_dir)
    installed = _catalog_names(catalog)
    if not installed:
        _log("  ❌ 使える星図データがありません。" + _index_place_advice())
        return None
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

    hint = None
    if hint_radec is not None:
        hint = (float(hint_radec[0]), float(hint_radec[1]), float(hint_radius_deg))
    narrow = (max(scale * 0.75, 0.01), scale * 1.25) if scale else None
    # 画角が分からないときは「手持ちの index で解ける範囲」を総当たりする。
    # 固定値 (v6.0 は 0.2〜120) だと、長焦点の画像がそもそも範囲外になってしまう。
    wide = blind_scale_bounds(nx, installed)
    if not scale:
        _log(f"  - 探索するスケール: {wide[0]:.3f}〜{wide[1]:.3f}″/px "
             f"(画角 {wide[0] * nx / 60:.1f}′〜{wide[1] * nx / 60:.0f}′ 相当)")

    # 星の組は 2 つ。
    #   深い = 解析エンジンのために暗い星まで拾い直した星 (detect_sources_deep)
    #   標準 = run() が検出して図に緑の丸で出す星 (detect_sources)
    # 深い検出の星を先に渡す。解析エンジンは quad に使わなかった星で検証するので、
    # 星が多いほど対数オッズの余裕が増える。実測 (sv_solve にブラインドで直接):
    #   3I は標準 8 個で 21.6 (合格 20.7 のぎりぎり) → 深い 15 個で 47.7、
    #   c2025a6 は 159 → 317、ほかの実写は同等。標準の星は予備に回す。
    if stopped():
        _log("\n  ■ 中止しました。")
        return None
    deep_stars, dinfo = [], {}
    try:
        if prep is None:
            prep = preprocess(bundle.data)[:3]
        deep_stars, u = detect_sources_deep(*prep, info=dinfo)
    except Exception as e:
        _log(f"\n  ⚠️ 深い検出に失敗したので、標準の検出の星だけで解きます: {type(e).__name__}: {e}")
    if deep_stars:
        s_psf = dinfo.get("s_psf")
        how = (f"星の幅 (FWHM {2.3548 * s_psf:.1f} px) に合わせてぼかし" if s_psf else
               f"星の幅を測れなかったので σ {dinfo.get('s', 1.5):.1f} px でぼかし")
        _log(f"\n  - 深い検出 (解析エンジンに渡す星): {how}、星を {len(deep_stars)} 個検出"
             f" (しきい値 {u:g}σ)")
    elif dinfo:
        _log("\n  - 深い検出: 星が見つかりませんでした。")
    star_sets = [(name, stars) for name, stars in (("深い検出", deep_stars), ("標準の検出", sources))
                 if len(stars) >= 3]
    if not star_sets:
        n_max = max(len(sources), len(deep_stars))
        _log("  ❌ 星が 1 つも無いので解析できません。" if n_max == 0 else
             f"  ❌ 星が {n_max} 個しかないので解析できません (3 個以上要ります)。")
        return None

    # 試行のはしご: 探す範囲の狭い (速い) 条件から順に、条件ごとに 深い → 標準 の星で。
    # (ラベル, 星, ヒント, スケールの範囲, 残り時間の何割まで使うか)
    # sv_solve は quad を試し尽くせば締め切り前に戻るので、狭い条件は両方の星で試しても安い。
    conds = []
    if hint and narrow:
        conds.append(("座標ヒント + スケール既知", hint, narrow, 0.25))
    if hint:
        conds.append(("座標ヒント + スケール全域", hint, wide, 0.25))
    if narrow:
        conds.append(("全天 + スケール既知", None, narrow, 0.25))
    conds.append(("全天 + スケール全域", None, wide, 0.5))
    attempts = [(f"{name}の星 {len(stars)} 個 + {cond}", stars, hint_i, rng, share)
                for cond, hint_i, rng, share in conds for name, stars in star_sets]

    solver_log = (lambda m: _log(f"    · {m}")) if verbose else None
    for i, (label, stars, hint_i, (s_lo, s_hi), share) in enumerate(attempts, 1):
        if stopped():
            _log("\n  ■ 中止しました。")
            return None
        left = t_end - time.monotonic()
        if left < 2.0:
            _log("  ⏱ 制限時間に達したので、これ以上の再試行はしません。")
            break
        t = left if i == len(attempts) else left * share
        _log(f"\n  [{i}/{len(attempts)}] {label}  (最大 {t:.0f} 秒)")
        xy = np.array([[s["x"], s["y"]] for s in stars], dtype=np.float64)
        flux = np.array([s["flux"] for s in stars], dtype=np.float64)
        try:
            sol = sv_solve(xy, flux, nx, ny, catalog, scale_lo=s_lo, scale_hi=s_hi,
                           hint=hint_i, deadline=time.monotonic() + t,
                           cancel=stopped, log=solver_log)
        except SvCancelled:
            _log("  ■ 中止しました。")
            return None
        except Exception as e:
            _log(f"  ⚠️ 解析エンジンでエラー: {type(e).__name__}: {e}")
            continue
        if sol is None:
            continue
        try:
            hdr = _solution_header(sol, nx, ny)
        except Exception as e:
            _log(f"  ⚠️ 解の WCS を読めませんでした: {type(e).__name__}: {e}")
            continue
        note = _solution_note(sol)
        if note:
            _log(f"    {note}")
        _log(f"  ✅ 解析成功 ({label})")
        return hdr

    if stopped():
        _log("\n  ■ 中止しました。")
        return None
    _log("\n  ❌ すべての条件で解けませんでした。心当たりの順に:")
    n_best = max(len(sources), len(deep_stars))
    if n_best < 15:
        _log(f"     ・写っている星が {n_best} 個と少なめです。"
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


def _with_deadline(fn, timeout, label, cancel=None):
    """
    指定時間で必ず戻る呼び出し。中止の合図 (cancel) が立てば 0.5 秒以内に None で戻る。
    v5 は with ThreadPoolExecutor(...) を使っていたため、タイムアウト後も
    ブロックの出口でワーカーの終了を待ってしまい、実際には打ち切れなかった。

    fn 自身が TimeoutError を投げたとき (astroquery の solve_timeout 切れ) は、こちらの締め切りとは
    区別して知らせる。Python 3.11 以降は concurrent.futures.TimeoutError と組み込みの TimeoutError が
    同じクラスなので、区別しないと「〜秒で終わりませんでした」と誤って出る。
    """
    stopped = _cancel_checker(cancel)
    executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)
    try:
        future = executor.submit(fn)
        t_end = time.monotonic() + float(timeout)
        while True:
            if stopped():
                future.cancel()
                _log(f"  ■ {label}を中止しました。")
                return None
            left = t_end - time.monotonic()
            if left <= 0:
                future.cancel()
                _log(f"⚠️ {label}が {timeout:.0f} 秒で終わりませんでした。")
                return None
            try:
                return future.result(timeout=min(0.5, left))
            except concurrent.futures.TimeoutError as e:
                if future.done():               # fn が投げた TimeoutError
                    why = e.args[0] if e.args else "時間切れ"
                    _log(f"⚠️ {label}: 時間内に解けませんでした ({why})。")
                    return None
    finally:
        # wait=False にしないと、ここで居残りスレッドの終了を待ってしまう
        executor.shutdown(wait=False, cancel_futures=True)


def solve_online(image_path, api_key, sources=None, shape=None, timeout=120,
                 hint_radec=None, hint_radius_deg=5.0, scale_hint=None, cancel=None):
    """
    nova.astrometry.net に投げる。指定時間で必ず戻る。解けなければ None。

    星のリストがあるならそれを送る。画像そのものより桁違いに軽く、
    オフラインとまったく同じ検出結果で解けるので結果もぶれない。
    cancel (threading.Event か、呼ぶと True を返す関数) が立てば 0.5 秒以内に None で戻り、
    画像そのものは送らない (中止を押した人の画像を、押したあとに外へ送らない)。
    nova が「解けない」と空の結果を返したときも None (成功扱いにしてオフラインを飛ばさない)。

    星の位置で解けなかったとき (星が 0 個ならはじめから) は、画像ファイルそのもの (ファイル名も) を
    api/upload で送る。photutils が入っていても同じ (exe と同じ動き。下の _no_source_detector)。

    送るときは必ず「公開しない」(publicly_visible='n') を付ける。付けないと nova は公開 ('y') として
    受け付け、画像のページや「最近送られた画像」の一覧などに、API キーの持ち主のアカウント名で載る
    (astroquery も既定値を自分では付けない)。'n' にしても解き方は変わらない。
    ただし 'n' でも一覧や画像ファイル・API の結果から完全には隠れない可能性がある (サーバーのコードの読み。
    実サイトでは未確認) ので、利用者には「nova に送ったものは第三者に見られることがある」と説明している。
    """
    from astroquery.astrometry_net import AstrometryNet
    stopped = _cancel_checker(cancel)

    ast = AstrometryNet()
    ast.api_key = api_key
    # astroquery 0.4.10 の solve_from_image は、photutils を import できると画像を上げず、
    # CCDData.read (FITS しか読めない) と DAOStarFinder で星を数え直して位置を送る。そのままだと
    # JPEG・PNG・TIFF はそこで例外になり、FITS でも「画像そのものを送ります」が事実と違う (v6 の exe がそうだった)。
    # 内部の旗をこのインスタンスにだけ立てて、いつも画像を上げさせる。force_image_upload=True は 0.4.8 から非推奨で、
    # 引数が消えると「Setting force_image_upload is not allowed」で画像の段が必ず失敗するので使わない。
    # 予告どおり photutils の経路が消えれば、この旗は読まれなくなるだけで画像は送られる。
    ast._no_source_detector = True
    settings = {"publicly_visible": "n"}      # 星の位置を送るときも画像を送るときも
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
            timeout + 15, "オンライン解析", cancel)
        if hdr:
            return hdr
        if stopped():
            return None
        _log("  - 星の位置だけでは解けなかったので、画像そのものを送ります。")

    if stopped():
        return None
    hdr = _with_deadline(
        lambda: ast.solve_from_image(image_path, solve_timeout=int(timeout),
                                     verbose=False, **settings),
        timeout + 30, "オンライン解析", cancel)
    return hdr if hdr else None


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

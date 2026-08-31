# ==============================================================================
#  zahyou  描画・レポート部
# ==============================================================================

from __future__ import annotations

import numpy as np
import matplotlib.pyplot as plt
from astropy.visualization import AsinhStretch, ImageNormalize
from astropy.wcs import WCS
from astropy.wcs.utils import proj_plane_pixel_scales
import astropy.units as u


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

    rep, foot = reproject_interp((np.nan_to_num(data), wcs), out, shape_out=shape_out)
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

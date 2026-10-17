"""zahyou-plot(Rust)の照合用の値を、Python 版の基準実装から作る。

基準実装は dev/zahyou_plot.py(_norm・clip_arrow_to_boundary・_draw_markers・plot_north_up・report_offsets・
print_solution)と、その下の matplotlib 3.10・astropy 7.1(WCSAxes・SkyCoord)・reproject 0.15。

使い方(numpy・astropy・matplotlib・reproject が要る):
    python gen_plot.py [norm text axes reproject layout]   (引数を省くと全部)
同じフォルダーに次の 5 つを書く。Rust の試験 crates/zahyou-plot/tests/python_ref.rs がこれと比べる。
    plot_norm.json       _norm の vmin・vmax と、matplotlib の灰色の色表で 8 bit にした値(cmap(norm(x), bytes=True))
    plot_text.json       60 進の文字列(SkyCoord.to_string("hmsdms")・WCSAxes の目盛りの書式)、目盛りの間隔、
                         題・ずれの文(report_offsets)・解析結果の文(print_solution)
    plot_axes.json       WCSAxes の座標の範囲・目盛りの値と間隔・各辺の目盛り・文字・文字を出す辺・格子の線(枠で切った区間)。
                         画像全体の表示 11 例と、拡大・移動した表示 3 例(ツールバーの拡大と同じく範囲だけを変える)
    plot_reproject.json  plot_north_up の出力の WCS・大きさ・reproject_interp の値・_norm、_draw_markers の位置(2 つの図)
    plot_layout.json     plot_original・plot_north_up をそのまま呼んで描いた図の配置(軸の矩形・題・軸名の外接矩形、px)。
                         游ゴシック(GUI と同じ font.family)で描いた値なので、Rust も游ゴシックがあるときだけ比べる

Python 版の dev フォルダーは環境変数 ZAHYOU_PY_DEV(既定は $ZAHYOU_PY_REPO、無ければ ~/Documents/GitHub/zahyou の下の dev)。
Python 版のフォルダーには何も書かない(__pycache__ も作らない)。乱数の種は固定。
"""
import json
import math
import os
import sys
import warnings

sys.dont_write_bytecode = True
os.environ.setdefault("MPLBACKEND", "Agg")
PY_REPO = os.environ.get("ZAHYOU_PY_REPO") or os.path.join(os.path.expanduser("~"), "Documents", "GitHub", "zahyou")
DEV = os.environ.get("ZAHYOU_PY_DEV") or os.path.join(PY_REPO, "dev")
sys.path.insert(0, DEV)

import numpy as np                                                    # noqa: E402
import matplotlib                                                     # noqa: E402
import matplotlib.pyplot as plt                                       # noqa: E402
from matplotlib.path import Path                                      # noqa: E402
import astropy                                                        # noqa: E402
import astropy.units as u                                             # noqa: E402
from astropy.coordinates import SkyCoord, Angle                       # noqa: E402
from astropy.io import fits                                           # noqa: E402
from astropy.wcs import WCS                                           # noqa: E402
from astropy.wcs.utils import proj_plane_pixel_scales                 # noqa: E402
from astropy.visualization.wcsaxes.formatter_locator import AngleFormatterLocator  # noqa: E402
from astropy.visualization.wcsaxes.utils import select_step_degree, select_step_hour  # noqa: E402
import reproject                                                      # noqa: E402
from reproject import reproject_interp                                # noqa: E402

import solver_wcs as sw                                               # noqa: E402
import zahyou_plot as zp                                              # noqa: E402

OUT = os.path.dirname(os.path.abspath(__file__))
LOG = []
zp._log = LOG.append          # dev/zahyou_plot.py は _log をエンジンから借りている

# GUI と同じ設定(図の日本語フォントと、マイナス記号を ASCII にする)
for name in ("Yu Gothic", "Meiryo", "MS Gothic"):
    if name in {f.name for f in matplotlib.font_manager.fontManager.ttflist}:
        matplotlib.rcParams["font.family"] = name
        break
matplotlib.rcParams["axes.unicode_minus"] = False
warnings.simplefilter("ignore")

RA_SEPS = {r"$\mathregular{^h}$": "ʰ", r"$\mathregular{^m}$": "ᵐ", r"$\mathregular{^s}$": "ˢ"}


def compact(label):
    """WCSAxes の数式の上付き($\\mathregular{^h}$)を Rust と同じ 1 文字の印にする。"""
    for k, v in RA_SEPS.items():
        label = label.replace(k, v)
    return label


def dump(name, obj):
    path = os.path.join(OUT, name)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(obj, fh, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    print(f"{name}: {os.path.getsize(path) / 1024:.0f} KB")


def meta():
    return {"generator": "rs/testdata/gen_plot.py", "python": sys.version.split()[0], "numpy": np.__version__,
            "astropy": astropy.__version__, "matplotlib": matplotlib.__version__,
            "reproject": reproject.__version__, "reference": "dev/zahyou_plot.py (branch native-solver)"}


def enc_f32(a):
    """float32 の配列を JSON に(有限でない値は null にして、位置を別に持つ)。"""
    a = np.asarray(a, dtype=np.float32).ravel()
    vals = [float(x) if np.isfinite(x) else None for x in a]
    return {"v": vals,
            "nan": [int(i) for i in np.nonzero(np.isnan(a))[0]],
            "posinf": [int(i) for i in np.nonzero(np.isposinf(a))[0]],
            "neginf": [int(i) for i in np.nonzero(np.isneginf(a))[0]]}


def num(x):
    """有限でない値は文字列 "inf" / "-inf" / "nan" にする(JSON に入らないため)。"""
    x = float(x)
    if np.isfinite(x):
        return x
    return "nan" if np.isnan(x) else ("inf" if x > 0 else "-inf")


def enc_f64_9(a):
    """float64 の配列を 9 桁で(NaN は null)。"""
    a = np.asarray(a, dtype=np.float64).ravel()
    return [float(f"{x:.9g}") if np.isfinite(x) else None for x in a]


def cards_json(cards):
    out = []
    for k, v, c in cards:
        if isinstance(v, (bool, np.bool_)):
            v = bool(v)
        elif isinstance(v, (int, np.integer)):
            v = int(v)
        elif isinstance(v, (float, np.floating)):
            v = float(v)
        out.append({"k": k, "v": v, "c": c})
    return out


def astropy_wcs(cards, shape):
    h = fits.Header([(k, v, c) for k, v, c in cards])
    return WCS(zp.normalize_wcs_header(h, shape))


def make_wcs(ra, dec, scale_arcsec, rot_deg, parity, w, h, crpix_frac=(0.5, 0.5), sip=None, skew=0.0, inverse=True):
    """SvWcs(Python 版の解析が返すもの)を作り、そのヘッダーのカード(AP・BP 付き)を返す。"""
    t = math.radians(rot_deg)
    m = np.array([[math.cos(t), -math.sin(t)], [math.sin(t), math.cos(t)]]) @ np.diag([1.0, float(parity)])
    m = m @ np.array([[1.0, skew], [0.0, 1.0]])
    cd = np.diag([-1.0, 1.0]) @ m * (scale_arcsec / 3600.0)
    crpix = (crpix_frac[0] * w, crpix_frac[1] * h)
    a = b = None
    if sip is not None:
        order, amp = sip
        rng = np.random.default_rng(order * 1000 + w)
        hd = 0.5 * math.hypot(w, h)
        a = np.zeros((order + 1, order + 1))
        b = np.zeros((order + 1, order + 1))
        for d in range(2, order + 1):
            for p in range(d + 1):
                a[p, d - p] = rng.normal() * amp / hd ** d
                b[p, d - p] = rng.normal() * amp / hd ** d
    sv = sw.SvWcs(sw.sv_radec_to_xyz(ra, dec), crpix, cd, a, b)
    if sip is not None and inverse:
        sv = sw.sv_fit_sip_inverse(sv, w, h)
    return sv.to_header_cards(w, h)


# ---------------------------------------------------------------- 伸張 ---

def gray8(data):
    """Python 版の図と同じ 8 bit の灰色(_norm → cmap gray、有限でない値は透明)。"""
    norm = zp._norm(data)
    if norm is None:
        return None, None
    rgba = plt.get_cmap("gray")(norm(np.ma.masked_invalid(data)), bytes=True)
    return norm, rgba


def synth_image(rng, w, h, nstars=12, bg=1000.0, noise=20.0):
    y, x = np.mgrid[0:h, 0:w]
    img = bg + 0.3 * x + 0.2 * y + rng.normal(0, noise, (h, w))
    for _ in range(nstars):
        cx, cy = rng.uniform(0, w), rng.uniform(0, h)
        amp = rng.uniform(50, 30000)
        img += amp * np.exp(-((x - cx) ** 2 + (y - cy) ** 2) / (2 * 1.3 ** 2))
    return img.astype(np.float32)


def big_series(n):
    """Rust でも同じ値を作れる大きな配列(整数のハッシュ → float64 の 4 乗 → float32)。"""
    i = np.arange(n, dtype=np.uint64)
    hsh = (i * np.uint64(2654435761)) % np.uint64(2 ** 32)
    x = hsh.astype(np.float64) / 2.0 ** 32
    return (x * x * x * x * 5000.0 + 100.0).astype(np.float32)


def gen_norm():
    rng = np.random.default_rng(20261015)
    cases = []
    lut = plt.get_cmap("gray")(np.linspace(0, 1, 256), bytes=True)[:, 0]

    def add(note, data):
        data = np.asarray(data, dtype=np.float32)
        norm, rgba = gray8(data)
        c = {"note": note, "shape": list(data.shape), "data": enc_f32(data)}
        if norm is None:
            c["norm"] = None
        else:
            c["norm"] = [num(norm.vmin), num(norm.vmax)]
            g = rgba[..., 0].ravel()
            a = rgba[..., 3].ravel()
            assert np.all((a == 0) | (a == 255))
            c["gray"] = "".join(f"{v:02x}" if al else "--" for v, al in zip(g, a))
        cases.append(c)

    add("星野(64×48)", synth_image(rng, 64, 48))
    im = synth_image(rng, 60, 40, nstars=30)
    im.ravel()[rng.choice(im.size, 40, replace=False)] = np.nan
    im.ravel()[rng.choice(im.size, 5, replace=False)] = np.inf
    im.ravel()[rng.choice(im.size, 5, replace=False)] = -np.inf
    add("NaN・±∞ を含む", im)
    add("一様(σ = 0 → std も 0 → σ = 1)", np.full((10, 12), 7.5))
    add("2 値", np.where(rng.random((16, 16)) < 0.97, 10.0, 1000.0))
    add("1 画素", np.array([[3.25]]))
    add("2 画素", np.array([[1.0, 2.0]]))
    add("全部 NaN", np.full((3, 3), np.nan))
    add("16 bit のカメラ風(整数値・飽和あり)", np.clip(np.round(synth_image(rng, 50, 40, nstars=20, bg=800, noise=12)),
                                                    0, 65535))
    add("負の値と大きな範囲", synth_image(rng, 40, 30, nstars=8, bg=-500, noise=3) * 1e12)
    add("float32 の 2 乗の和があふれる(σ = ∞)", synth_image(rng, 40, 30, nstars=8, bg=-500, noise=3) * 1e15)
    add("背景が一様で星だけ(σ クリップで σ = 0 にならない程度)", np.where(rng.random((32, 32)) < 0.9, 100.0,
                                                                        rng.uniform(100, 200, (32, 32))))

    big = []
    for n in (100_003, 20_000_001):
        v = big_series(n)
        norm = zp._norm(v)
        big.append({"n": n, "norm": [float(norm.vmin), float(norm.vmax)],
                    "p999": float(np.percentile(v, 99.9))})
        print(f"  大きな配列 n={n}: vmin={norm.vmin!r} vmax={norm.vmax!r}")
    dump("plot_norm.json", {"meta": meta(), "lut": [int(x) for x in lut], "cases": cases, "big": big})


# ---------------------------------------------------------------- 文字 ---

def gen_text():
    rng = np.random.default_rng(7)
    out = {"meta": meta()}
    # SkyCoord.to_string("hmsdms", precision=…)
    pts = [(0.0, 0.0), (0.0, -0.5), (359.9999999999, 89.99999999), (359.99999, -89.9999999), (180.0, -0.0),
           (61.91199157844, 41.98626242835), (61.891414, 41.964760), (15.0, 1.0), (14.999999999999998, -1.0),
           (0.000000001, -0.000000001), (83.82, -5.39), (359.99999999, 0.0)]
    # 秒の四捨五入の境目(xx.xx5 秒と 59.99x 秒)
    for hh, mm, ss in [(4, 7, 38.875), (4, 7, 59.995), (4, 7, 59.9949999), (23, 59, 59.996), (0, 0, 0.005),
                       (12, 30, 29.995), (1, 2, 3.0049999999), (5, 59, 59.99)]:
        pts.append(((hh + mm / 60 + ss / 3600) * 15.0, -(hh + mm / 60 + ss / 3600)))
    for _ in range(300):
        pts.append((float(rng.uniform(0, 360)), float(np.degrees(np.arcsin(rng.uniform(-1, 1))))))
    hms = []
    for ra, dec in pts:
        for prec in (0, 1, 2, 3):
            if prec != 2 and len(hms) % 7:
                continue
            c = SkyCoord(ra * u.deg, dec * u.deg)
            hms.append({"ra": ra, "dec": dec, "p": prec, "s": c.to_string("hmsdms", precision=prec)})
    out["hmsdms"] = hms

    # WCSAxes の目盛りの書式(間隔 → 項目数・桁、値 → 文字)
    fmt = []
    spacings_ra = [12 * 15.0, 15.0, 2 * 15.0, 0.25, 0.5, 1 / 60 * 15, 10 / 3600 * 15, 1 / 3600 * 15,
                   0.5 / 3600 * 15, 0.2 / 3600 * 15, 0.05 / 3600 * 15]
    spacings_dec = [30.0, 5.0, 1.0, 0.5, 10 / 60, 1 / 60, 30 / 3600, 5 / 3600, 1 / 3600, 0.5 / 3600, 0.1 / 3600,
                    0.02 / 3600]
    for coord, sps in ((0, spacings_ra), (1, spacings_dec)):
        fl = AngleFormatterLocator(unit=u.deg, format_unit=u.hourangle if coord == 0 else u.deg)
        for sp in sps:
            k0 = rng.integers(-40, 40) if coord == 1 else rng.integers(0, 300)
            vals = (np.arange(k0, k0 + 6) * sp) % 360.0 if coord == 0 else np.clip(np.arange(k0, k0 + 6) * sp, -90, 90)
            if coord == 1:
                vals = np.concatenate([vals, [-sp / 2, -0.0, 0.0, 89.99999999999]])
            txt = fl.formatter(vals * u.deg, spacing=sp * u.deg)
            fmt.append({"coord": coord, "spacing": sp, "values": [float(v) for v in vals],
                        "text": [compact(t) for t in txt]})
    out["format"] = fmt

    # 目盛りの間隔の選び方
    dvs = sorted(set([float(x) for x in 10 ** rng.uniform(-5, 2, 200)] +
                     [v / 3600 for v in (1.5, 2.5, 3.5, 8, 11, 18, 25, 45)] +
                     [v / 60 for v in (1.5, 2.5, 3.5, 8, 11, 18, 25, 45)] +
                     [1.5, 3, 7, 13, 20, 40, 70] + [15 / 3600, 1 / 3600, 0.3, 0.31, 72.0, 36.0]))
    out["steps"] = [{"dv": dv, "deg": float(select_step_degree(dv * u.deg).to_value(u.deg)),
                     "hour": float(select_step_hour(dv * u.deg).to_value(u.deg))} for dv in dvs]

    # 題とずれ(report_offsets)・解析結果(print_solution)
    offs = []
    centers = [(61.891414, 41.964760), (83.82, -5.39), (0.001, 0.0), (359.999, 89.9), (180.0, -60.0)]
    for cra, cdec in centers:
        for _ in range(12):
            dist = 10 ** rng.uniform(-3, 1.3)
            pa = rng.uniform(0, 360)
            c = SkyCoord(cra * u.deg, cdec * u.deg)
            t = c.directional_offset_by(pa * u.deg, dist * u.deg)
            LOG.clear()
            sep, ra_min, dec_min, ra_dir, dec_dir = zp.report_offsets(c, t)
            title = (f"北が上・東が左\n"
                     f"距離 {sep.arcmin:.2f}′ | "
                     f"RA {abs(ra_min):.2f}′({ra_dir}) "
                     f"Dec {abs(dec_min):.2f}′({dec_dir})")
            offs.append({"center": [cra, cdec], "target": [float(t.ra.deg), float(t.dec.deg)],
                         "lines": list(LOG), "title": title,
                         "subtitle": f"目標: {t.to_string('hmsdms', precision=2)}"})
    out["offsets"] = offs

    summ = []
    for i, (ra, dec, sc, rot, par, w, h, sip) in enumerate([
            (61.9, 41.97, 1.7, 12.0, 1, 968, 548, (3, 2.0)), (83.8, -5.4, 0.7, 200.0, -1, 3000, 2000, None),
            (10.0, 89.9, 20.0, 95.0, 1, 600, 400, (2, 5.0)), (359.99, -30.0, 3.0, -45.0, -1, 500, 800, None)]):
        cards = make_wcs(ra, dec, sc, rot, par, w, h, sip=sip)
        aw = astropy_wcs(cards, (h, w))
        for flip in (False, True):
            LOG.clear()
            zp.print_solution(aw, (h, w), flip_y=flip)
            summ.append({"cards": cards_json(cards), "width": w, "height": h, "flip_y": flip, "lines": list(LOG)})
    out["summary"] = summ
    dump("plot_text.json", out)


# ---------------------------------------------------------------- 軸 ---

def clip_polyline(pts, rect):
    """Rust の axes::clip_polyline と同じ Liang–Barsky(枠で切った区間)。"""
    xmin, xmax, ymin, ymax = rect
    out, cur = [], []
    for p, q in zip(pts[:-1], pts[1:]):
        dx, dy = q[0] - p[0], q[1] - p[1]
        t0, t1, ok = 0.0, 1.0, True
        for pp, qq in ((-dx, p[0] - xmin), (dx, xmax - p[0]), (-dy, p[1] - ymin), (dy, ymax - p[1])):
            if pp == 0:
                if qq < 0:
                    ok = False
                    break
            else:
                r = qq / pp
                if pp < 0:
                    if r > t1:
                        ok = False
                        break
                    t0 = max(t0, r)
                else:
                    if r < t0:
                        ok = False
                        break
                    t1 = min(t1, r)
        if not ok or not (t0 <= t1):
            if len(cur) >= 2:
                out.append(cur)
            cur = []
            continue
        a = (p[0] + t0 * dx, p[1] + t0 * dy)
        b = (p[0] + t1 * dx, p[1] + t1 * dy)
        if not (t0 == 0.0 and cur and cur[-1] == tuple(p)):
            if len(cur) >= 2:
                out.append(cur)
            cur = [a]
        cur.append(b)
        if t1 < 1.0:
            if len(cur) >= 2:
                out.append(cur)
            cur = []
    if len(cur) >= 2:
        out.append(cur)
    return out


def path_pieces(path, rect):
    """matplotlib の Path(MOVETO で区切る)を折れ線に分け、枠で切る。"""
    pieces, cur = [], []
    for (x, y), code in zip(path.vertices, path.codes):
        if code == Path.MOVETO:
            if len(cur) >= 2:
                pieces.append(cur)
            cur = []
        if np.isfinite(x) and np.isfinite(y):
            cur.append((float(x), float(y)))
        else:
            cur = []
    if len(cur) >= 2:
        pieces.append(cur)
    out = []
    for pc in pieces:
        for c in clip_polyline(pc, rect):
            out.append({"n": len(c), "first": list(c[0]), "last": list(c[-1])})
    return out


def draw_like_plot_original(aw, shape, flip_y, data=None, view=None):
    """plot_original と同じ軸を作る。view = (x0, x1, y0, y1) を渡すと、その範囲に拡大した表示(ツールバーの拡大と同じ)。"""
    ny, nx = shape
    fig = plt.figure(figsize=(12, 12 * ny / nx + 1.2))
    ax = fig.add_subplot(111, projection=aw)
    ax.imshow(np.zeros(shape, dtype=np.float32) if data is None else data, origin="lower", cmap="gray")
    x0, x1, y0, y1 = view if view is not None else (-0.5, nx - 0.5, -0.5, ny - 0.5)
    ax.set_xlim(x0, x1)
    ax.set_ylim(y0, y1)
    if flip_y:
        ax.invert_yaxis()
    ax.coords.grid(color="deepskyblue", linestyle="--", linewidth=0.9, alpha=0.65)
    ax.coords[0].set_axislabel("赤経 (RA)")
    ax.coords[1].set_axislabel("赤緯 (Dec)")
    ax.set_title("撮影時の向き\n目標", size=14)
    plt.tight_layout()
    fig.canvas.draw()
    return fig, ax


def axes_state(ax, shape):
    ny, nx = shape
    xl, yl = ax.get_xlim(), ax.get_ylim()
    rect = (min(xl), max(xl), min(yl), max(yl))
    rng = ax.coords.get_coord_range()
    coords = []
    for i in (0, 1):
        c = ax.coords[i]
        vals, sp = c.locator(*rng[i])
        ticks = {}
        for axis in "brtl":
            ticks[axis] = {"world": [float(w) for w in c._ticks.world.get(axis, [])],
                           "pixel": [[float(p[0]), float(p[1])] for p in c._ticks.pixel.get(axis, [])],
                           "angle": [float(a) for a in c._ticks.angle.get(axis, [])]}
        pos = [p for p in c.get_ticklabel_position() if p != "#"]
        labels = {}
        for axis in pos:
            labels[axis] = {"text": [compact(t) for t in c._ticklabels.text.get(axis, [])],
                            "data": [[float(d[0]), float(d[1])] for d in c._ticklabels.data.get(axis, [])],
                            "world": [float(w) for w in c._ticklabels.world.get(axis, [])]}
        grid = []
        for w, path in zip(vals.to_value(u.deg), c._grid_lines):
            grid.append({"world": float(w), "pieces": path_pieces(path, rect)})
        coords.append({"range": [float(rng[i][0]), float(rng[i][1])],
                       "values": [float(v) for v in vals.to_value(u.deg)], "spacing": float(sp.to_value(u.deg)),
                       "ticks": ticks, "label_sides": pos, "labels": labels, "grid": grid})
    return {"xlim": [float(x) for x in xl], "ylim": [float(y) for y in yl], "coords": coords}


AXES_CASES = [
    # (メモ, ra, dec, ″/px, 回転°, パリティ, 幅, 高さ, flip_y, SIP)
    ("ふつう(20° 回転・SIP 2 次・上下反転)", 61.9, 41.97, 2.0, 20.0, 1, 600, 400, True, (2, 3.0)),
    ("同じ・反転なし", 61.9, 41.97, 2.0, 20.0, 1, 600, 400, False, (2, 3.0)),
    ("極を含む(赤経が全周)", 30.0, 89.99, 6.0, 0.0, 1, 500, 400, False, None),
    ("極のそば(極は画面外)", 200.0, 89.5, 4.0, 35.0, -1, 500, 400, True, None),
    ("赤経 0h のまたぎ", 0.01, 20.0, 9.0, 10.0, 1, 500, 400, False, None),
    ("90° 回転(赤経が縦)", 120.0, -30.0, 3.0, 90.0, 1, 400, 600, False, None),
    ("鏡像・140° 回転", 250.0, -75.0, 5.0, 140.0, -1, 640, 480, True, (3, 2.0)),
    ("狭い視野(0.1″/px)", 83.8, -5.4, 0.1, 5.0, 1, 300, 200, False, None),
    ("広い視野(60″/px、30° 角)", 280.0, 38.0, 60.0, 15.0, 1, 2000, 1500, False, None),
    ("南天・45° 回転", 10.0, -60.0, 1.2, 45.0, 1, 968, 548, True, (3, 1.0)),
    ("南極を含む", 77.0, -89.995, 3.0, 60.0, -1, 400, 300, False, None),
]


ZOOM_CASES = [
    # (メモ, AXES_CASES の番号, 拡大した範囲 (x0, x1, y0, y1))。目盛りは WCSAxes が見えている範囲で選び直す
    ("拡大: ふつう(上下反転)の左上", 0, (40.0, 210.0, 250.0, 380.0)),
    ("拡大: 狭い視野をさらに 1/6", 7, (120.0, 170.0, 80.0, 115.0)),
    ("移動: 赤経 0h のまたぎを画像の外まで", 4, (300.0, 700.0, -100.0, 200.0)),
]


def gen_axes():
    cases = []
    for note, k, view in [(c[0], i, None) for i, c in enumerate(AXES_CASES)] + ZOOM_CASES:
        _, ra, dec, sc, rot, par, w, h, flip, sip = AXES_CASES[k]
        cards = make_wcs(ra, dec, sc, rot, par, w, h, sip=sip)
        aw = astropy_wcs(cards, (h, w))
        fig, ax = draw_like_plot_original(aw, (h, w), flip, view=view)
        st = axes_state(ax, (h, w))
        plt.close(fig)
        st.update({"note": note, "cards": cards_json(cards), "width": w, "height": h, "flip_y": flip,
                   "view": None if view is None else list(view)})
        cases.append(st)
        print(f"  {note}: 目盛り {[len(c['values']) for c in st['coords']]}、文字の辺 "
              f"{[c['label_sides'] for c in st['coords']]}")
    dump("plot_axes.json", {"meta": meta(), "cases": cases})


# ---------------------------------------------------------------- 北が上 ---

def north_up_like_python(aw, data):
    """plot_north_up の前半(描かずに値だけ)。"""
    ny, nx = data.shape
    scale = float(np.mean(proj_plane_pixel_scales(aw.celestial)))
    center = zp.as_icrs(aw.pixel_to_world((nx - 1) / 2.0, (ny - 1) / 2.0))
    out = WCS(naxis=2)
    out.wcs.ctype = ["RA---TAN", "DEC--TAN"]
    out.wcs.crval = [center.ra.deg, center.dec.deg]
    out.wcs.cdelt = np.array([-scale, scale])
    out.wcs.crpix = [1.0, 1.0]
    corners_x = np.array([0, nx - 1, 0, nx - 1], dtype=float)
    corners_y = np.array([0, 0, ny - 1, ny - 1], dtype=float)
    sky = aw.pixel_to_world(corners_x, corners_y)
    px, py = out.world_to_pixel(sky)
    out.wcs.crpix = [1.0 - float(np.min(px)), 1.0 - float(np.min(py))]
    shape_out = (int(np.ceil(np.ptp(py))) + 1, int(np.ceil(np.ptp(px))) + 1)
    rep, foot = reproject_interp((np.nan_to_num(data), aw), out, shape_out=shape_out, block_size=(512, 512))
    rep = np.where(foot > 0, rep, np.nan)
    return center, scale, out, shape_out, rep


def markers(aw, shape, target, flip_y=None, data=None):
    """_draw_markers を本物の WCSAxes で呼び、置いたものの位置を読む。flip_y が None なら北が上の図の作り方。"""
    ny, nx = shape
    fig = plt.figure(figsize=(6, 6 * ny / nx + 0.6))
    ax = fig.add_subplot(111, projection=aw)
    ax.imshow(np.zeros(shape) if data is None else data, origin="lower", cmap="gray")
    if flip_y is not None:
        ax.set_xlim(-0.5, nx - 0.5)
        ax.set_ylim(-0.5, ny - 0.5)
        if flip_y:
            ax.invert_yaxis()
    n0 = len(ax.collections)
    # astropy の all_world2pix が発散したか(SIP の外挿が破綻する画像の外の目標。そのときの値は当てにならない)
    diverged = False
    if target is not None:
        with warnings.catch_warnings(record=True) as wl:
            warnings.simplefilter("always")
            aw.world_to_pixel(target)
        diverged = any("converge" in str(x.message) for x in wl)
    inside = zp._draw_markers(ax, aw, shape, target, draw_arrow=target is not None)
    cols = ax.collections[n0:]
    res = {"center": [float(v) for v in cols[0].get_offsets()[0]], "inside": inside, "diverged": diverged}
    arrows = [t for t in ax.texts if t.get_text() == ""]
    if arrows:
        a = arrows[0]
        res["arrow"] = {"from": [float(v) for v in a.xyann], "to": [float(v) for v in a.xy]}
    if len(cols) > 1:
        res["circle"] = [float(v) for v in cols[1].get_offsets()[0]]
    labels = [t for t in ax.texts if t.get_text() == "TARGET"]
    if labels:
        res["label"] = [float(v) for v in labels[0].xy]
    plt.close(fig)
    return res


REPRO_CASES = [
    # (メモ, ra, dec, ″/px, 回転°, パリティ, 幅, 高さ, SIP, 中心の位置の割合, skew)
    ("TAN・30° 回転", 83.8, -5.4, 5.0, 30.0, 1, 40, 30, None, (0.5, 0.5), 0.0),
    ("SIP 3 次・NaN と ±∞ を含む", 61.9, 41.97, 3.0, -20.0, 1, 36, 28, (3, 1.5), (0.4, 0.6), 0.0),
    ("極のそば", 10.0, 89.98, 20.0, 75.0, -1, 34, 26, None, (0.5, 0.5), 0.0),
    ("赤経 0° のまたぎ・SIP 2 次", 359.999, 30.0, 8.0, 160.0, 1, 38, 30, (2, 1.0), (0.3, 0.7), 0.0),
    ("鏡像・120° 回転・ゆがみ(skew)", 250.0, -75.0, 6.0, 120.0, -1, 32, 32, None, (0.6, 0.4), 0.05),
    ("45° 回転(出力が大きくなる)", 180.0, 0.0, 4.0, 45.0, 1, 40, 24, (2, 0.5), (0.5, 0.5), 0.0),
]


def gen_reproject():
    rng = np.random.default_rng(99)
    cases = []
    for note, ra, dec, sc, rot, par, w, h, sip, cf, skew in REPRO_CASES:
        cards = make_wcs(ra, dec, sc, rot, par, w, h, crpix_frac=cf, sip=sip, skew=skew)
        aw = astropy_wcs(cards, (h, w))
        data = synth_image(rng, w, h, nstars=6)
        if "NaN" in note:
            data.ravel()[rng.choice(data.size, 12, replace=False)] = np.nan
            data.ravel()[rng.choice(data.size, 2, replace=False)] = np.inf
            data.ravel()[rng.choice(data.size, 2, replace=False)] = -np.inf
        center, scale, out, shape_out, rep = north_up_like_python(aw, data)
        norm = zp._norm(rep)
        # 目標: 画面内・画面外(近い)・画面外(遠い、TAN だけ)・反対の空・目標なし
        c = center
        fov = sc * max(w, h) / 3600.0
        targets = [c.directional_offset_by(37 * u.deg, 0.2 * fov * u.deg),
                   c.directional_offset_by(250 * u.deg, 1.7 * fov * u.deg)]
        if sip is None:
            targets.append(c.directional_offset_by(100 * u.deg, 40 * u.deg))
            # 反対の空(TAN で写せない。方向だけ求めて遠くを指す)。真裏ちょうどは方向が決まらないので 150°
            targets.append(c.directional_offset_by(300 * u.deg, 150 * u.deg))
        mk = []
        for t in targets + [None]:
            m1 = markers(aw, (h, w), t, flip_y=True)
            m2 = markers(out, shape_out, t, data=rep)
            tt = None if t is None else [float(t.ra.deg), float(t.dec.deg)]
            title2 = None
            if t is not None:
                LOG.clear()
                sep, ra_min, dec_min, ra_dir, dec_dir = zp.report_offsets(center, t)
                title2 = (f"北が上・東が左\n距離 {sep.arcmin:.2f}′ | RA {abs(ra_min):.2f}′({ra_dir}) "
                          f"Dec {abs(dec_min):.2f}′({dec_dir})")
            mk.append({"target": tt, "original": m1, "north_up": m2, "title_north_up": title2})
        cases.append({"note": note, "cards": cards_json(cards), "width": w, "height": h, "data": enc_f32(data),
                      "center": [float(center.ra.deg), float(center.dec.deg)], "scale": scale,
                      "out_crval": [float(v) for v in out.wcs.crval], "out_crpix": [float(v) for v in out.wcs.crpix],
                      "out_cdelt": [float(v) for v in out.wcs.cdelt], "shape_out": list(shape_out),
                      "rep": enc_f64_9(rep), "norm": None if norm is None else [num(norm.vmin), num(norm.vmax)],
                      "markers": mk})
        print(f"  {note}: {w}×{h} → {shape_out[1]}×{shape_out[0]}、有効 {int(np.isfinite(rep).sum())} 画素")
    dump("plot_reproject.json", {"meta": meta(), "cases": cases})


# ---------------------------------------------------------------- 配置 ---

def capture_figures():
    """plt.show を差し替えて、Python 版の plot_* が作った図を描いて(savefig と同じ draw)取っておく。"""
    figs = []

    def show():
        fig = plt.gcf()
        fig.canvas.draw()
        figs.append(fig)
    plt.show = show
    return figs


def boxes_px(fig, ax):
    """描いた図の軸・題・軸名の外接矩形(px、左上が原点)。"""
    r = fig.canvas.get_renderer()
    H = fig.bbox.height

    def bb(b):
        return [float(b.x0), float(H - b.y1), float(b.x1), float(H - b.y0)]
    out = {"fig": [float(fig.bbox.width), float(H)], "axes": bb(ax.get_window_extent(r)),
           "title": bb(ax.title.get_window_extent(r)), "axislabels": []}
    for i in (0, 1):
        al = ax.coords[i]._axislabels
        if al.get_visible() and al.get_text():
            vis = [a for a in al.get_visible_axes() if a != "#"]
            out["axislabels"].append({"coord": i, "box": bb(al.get_window_extent(r)), "axes": vis})
    return out


LAYOUT_CASES = [
    # (メモ, AXES_CASES の番号, 目標の方向°(北から東回り), 目標までの距離(視野の何倍)。None なら目標なし)
    ("ふつう(上下反転)", 0, 37.0, 0.2),
    ("同じ・反転なし", 1, 200.0, 0.8),
    ("極を含む", 2, 10.0, 0.2),
    ("極のそば", 3, 120.0, 0.3),
    ("赤経 0h のまたぎ", 4, 300.0, 0.25),
    ("90° 回転(縦長)", 5, 80.0, 0.2),
    ("鏡像・140° 回転", 6, 15.0, 2.0),
    ("南天・45° 回転・目標なし", 9, None, None),
]


def gen_layout():
    """Python 版の plot_original・plot_north_up をそのまま呼び(GUI と同じフォント設定)、描いた後の配置を取る。

    matplotlib の配置は描く順序で変わる(tight_layout の前に描くと題の位置が変わる)ので、Python 版と同じ手順
    (図を作る → tight_layout 1 回 → savefig / 画面に描く)で測る。画像の中身は配置に関係しないので一様な値。
    """
    cases = []
    for note, k, pa, dist in LAYOUT_CASES:
        _, ra, dec, sc, rot, par, w, h, flip, sip = AXES_CASES[k]
        cards = make_wcs(ra, dec, sc, rot, par, w, h, sip=sip)
        aw = astropy_wcs(cards, (h, w))
        data = np.zeros((h, w), dtype=np.float32)
        c = zp.as_icrs(aw.pixel_to_world((w - 1) / 2.0, (h - 1) / 2.0))
        fov = sc * max(w, h) / 3600.0
        target = None if pa is None else c.directional_offset_by(pa * u.deg, dist * fov * u.deg)
        figs = capture_figures()
        zp.plot_original(aw, data, target, flip, sources=None, draw_arrow=target is not None)
        if target is not None:
            zp.plot_north_up(aw, data, target)
        res = {"note": note, "cards": cards_json(cards), "width": w, "height": h, "flip_y": flip,
               "target": None if target is None else [float(target.ra.deg), float(target.dec.deg)],
               "figures": []}
        for fig in figs:
            res["figures"].append(boxes_px(fig, fig.axes[0]))
            plt.close(fig)
        cases.append(res)
        print(f"  {note}: 図 {len(figs)} 枚、軸 {[[round(v, 1) for v in f['axes']] for f in res['figures']]}")
    fonts = sorted({f.name for f in matplotlib.font_manager.fontManager.ttflist})
    dump("plot_layout.json", {"meta": meta(), "font_family": matplotlib.rcParams["font.family"],
                              "yu_gothic": "Yu Gothic" in fonts, "cases": cases})


def main():
    only = sys.argv[1:] or ["norm", "text", "axes", "reproject", "layout"]
    if "norm" in only:
        print("伸張 …")
        gen_norm()
    if "text" in only:
        print("文字 …")
        gen_text()
    if "axes" in only:
        print("軸 …")
        gen_axes()
    if "reproject" in only:
        print("北が上 …")
        gen_reproject()
    if "layout" in only:
        print("配置 …")
        gen_layout()


if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except (AttributeError, ValueError):
        pass
    main()

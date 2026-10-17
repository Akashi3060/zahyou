"""zahyou-wcs(Rust)の照合用の値を、Python 版の基準実装から作る。

使い方(numpy・astropy が要る):
    python gen_wcs.py
同じフォルダーに次の 4 つを書く。Rust の試験 crates/zahyou-wcs/tests/python_ref.rs がこれと比べる。
    wcs_proj.json     乱数の WCS(TAN・SIP 2 次・3 次、逆変換あり・なし)で 画素 ↔ 天、スケール、鏡像、ヘッダーのカード、COMMENT の折り返し
    wcs_fit.json      4 点の相似変換、重みつき最小二乗(TAN・SIP 2 次・3 次)、SIP の逆変換、縮退した入力
    wcs_header.json   いろいろな形のヘッダー(CD・PC+CDELT・CROTA2・LONPOLE・極・solve-field の実物)を astropy が読んだ結果
    wcs_offsets.json  目標までのずれ・位置角・その向きへ進んだ点(astropy)

Python 版の dev フォルダーは環境変数 ZAHYOU_PY_DEV(既定は $ZAHYOU_PY_REPO、無ければ ~/Documents/GitHub/zahyou の下の dev)。
Python 版のフォルダーには何も書かない(__pycache__ も作らない)。乱数の種は固定。
"""
import json
import math
import os
import sys
import textwrap
import warnings

sys.dont_write_bytecode = True
PY_REPO = os.environ.get("ZAHYOU_PY_REPO") or os.path.join(os.path.expanduser("~"), "Documents", "GitHub", "zahyou")
DEV = os.environ.get("ZAHYOU_PY_DEV") or os.path.join(PY_REPO, "dev")
sys.path.insert(0, DEV)

import numpy as np                                                    # noqa: E402
import astropy                                                        # noqa: E402
import astropy.units as u                                             # noqa: E402
from astropy.coordinates import SkyCoord                              # noqa: E402
from astropy.io import fits                                           # noqa: E402
from astropy.wcs import WCS                                           # noqa: E402

import solver_wcs as sw                                               # noqa: E402

OUT = os.path.dirname(os.path.abspath(__file__))

# 接点の特別な場所(dev/test_solver_wcs.py と同じ): 北極・南極ちょうど、極のすぐそば、赤経 0° のまたぎ、南天
SPECIAL_CRVAL = [(0.0, 90.0), (0.0, -90.0), (137.0, 89.9999), (300.0, -89.99),
                 (359.9995, 0.0), (0.0004, -45.0), (359.99, 61.0), (250.0, -75.0), (83.8, -5.4)]


def rot(theta_deg):
    t = math.radians(theta_deg)
    return np.array([[math.cos(t), -math.sin(t)], [math.sin(t), math.cos(t)]])


def random_cd(rng, scale_arcsec, parity, skew=0.0):
    m = rot(rng.uniform(0, 360)) @ np.diag([1.0, float(parity)]) @ np.array([[1.0, skew], [0.0, 1.0]])
    return m * (scale_arcsec / 3600.0)


def random_sip(rng, order, half_diag, amp=(3.0, 1.0, 0.5)):
    a = np.zeros((order + 1, order + 1))
    b = np.zeros((order + 1, order + 1))
    for d in range(2, order + 1):
        for p in range(d + 1):
            a[p, d - p] = rng.normal() * amp[d - 2] / half_diag ** d
            b[p, d - p] = rng.normal() * amp[d - 2] / half_diag ** d
    return a, b


def random_config(rng, i, sip_order=0, scale=None):
    """dev/test_solver_wcs.py の random_config と同じ作り方。"""
    if i < len(SPECIAL_CRVAL):
        crval = sw.sv_radec_to_xyz(*SPECIAL_CRVAL[i])
    else:
        crval = sw.sv_normalize(rng.normal(size=3))
    if scale is None:
        scale = 10 ** rng.uniform(math.log10(0.3), math.log10(300.0))
    w = int(rng.uniform(300, 4000))
    h = int(rng.uniform(200, 3000))
    lim = 100.0 / (scale / 3600.0)
    f = min(1.0, lim / math.hypot(w, h))
    w, h = max(50, int(w * f)), max(40, int(h * f))
    parity = 1 if i % 2 == 0 else -1
    skew = 0.02 * rng.normal() if i % 3 == 0 else 0.0
    cd = random_cd(rng, scale, parity, skew)
    crpix = (rng.uniform(0.2, 0.8) * w, rng.uniform(0.2, 0.8) * h)
    a = b = None
    if sip_order >= 2:
        a, b = random_sip(rng, sip_order, 0.5 * math.hypot(w, h))
    return sw.SvWcs(crval, crpix, cd, a, b), w, h


def fl(x):
    """numpy の値を JSON の数へ(NaN は None)。"""
    x = float(x)
    return None if not math.isfinite(x) else x


def wcs_json(w):
    """Rust の Wcs のフィールドそのまま(crval は正規化済みの値をそのまま渡す)。"""
    d = {"crval": [fl(t) for t in w.crval_xyz], "crpix": [fl(t) for t in w.crpix],
         "cd": [[fl(t) for t in row] for row in w.cd], "sip": None}
    if w.a is not None:
        d["sip"] = {"order": int(w.a_order), "a": [fl(t) for t in w.a.ravel()], "b": [fl(t) for t in w.b.ravel()],
                    "ap_order": int(w.ap_order) if w.ap is not None else 0,
                    "ap": [fl(t) for t in w.ap.ravel()] if w.ap is not None else [],
                    "bp": [fl(t) for t in w.bp.ravel()] if w.bp is not None else []}
    return d


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


def dump(name, obj):
    path = os.path.join(OUT, name)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(obj, fh, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    print(f"{name}: {os.path.getsize(path) / 1024:.0f} KB")


def meta():
    return {"generator": "rs/testdata/gen_wcs.py", "python": sys.version.split()[0], "numpy": np.__version__,
            "astropy": astropy.__version__, "reference": "dev/solver_wcs.py (branch native-solver)"}


# ---------------------------------------------------------------- 投影 ---

def random_text(rng):
    """COMMENT の折り返しを試す文(英字・数字・ハイフン・em ダッシュ・記号・長い語・ASCII 以外・連続した空白)。"""
    pieces = ["index-5205-02", "logodds", "116.7", "nmatch", "well-known", "goof-ball", "a-b-c", "--", "x--y",
              "foo--bar", "e-mail", "re-", "-b", "-", "---", "_a-_b", "ab-cd-ef", "Hello,", "there!", "\"quote\"",
              "it's", "&co.", "q?", "日本語", "é", "\t", "\n", "  ", "   ", "a" * 75, "b-" * 40, "c" * 140,
              "1-2-3", "x1-y2", "ab-12", "12-ab", "a_b-c_d", "--start", "end--", "mid--", "(paren)", "[x]-[y]"]
    n = int(rng.integers(0, 30))
    words = []
    for _ in range(n):
        if rng.random() < 0.5:
            words.append(pieces[int(rng.integers(0, len(pieces)))])
        else:
            k = int(rng.integers(1, 12))
            alphabet = "abcdefghijklmnopqrstuvwxyz0123456789-_.,"
            words.append("".join(alphabet[int(rng.integers(0, len(alphabet)))] for _ in range(k)))
    sep = [" ", " ", " ", "  ", "", "-"]
    s = ""
    for w in words:
        s += w + sep[int(rng.integers(0, len(sep)))]
    if rng.random() < 0.2:
        s = " " * int(rng.integers(1, 4)) + s
    return s


def gen_proj():
    rng = np.random.default_rng(20261015)
    cases = []
    for sip_order, ncase in ((0, 14), (2, 10), (3, 10)):
        for i in range(ncase):
            wcs, w, h = random_config(rng, i, sip_order)
            if sip_order >= 2 and i % 2 == 0:
                wcs = sw.sv_fit_sip_inverse(wcs, w, h)
            npts = 12
            x = rng.uniform(-0.1 * w, 1.1 * w, npts)
            y = rng.uniform(-0.1 * h, 1.1 * h, npts)
            xi, eta = wcs.pix_to_iwc(x, y)
            xyz = wcs.pix_to_xyz(x, y)
            ra, dec = wcs.pix_to_radec(x, y)
            px, py, ok = wcs.xyz_to_pix(xyz)
            ex, ey, eok = wcs.xyz_to_pix(xyz, exact=True)
            rx, ry, rok = wcs.radec_to_pix(ra, dec)
            bx, by, bok = wcs.xyz_to_pix(np.array([-wcs.crval_xyz]))
            comments = [random_text(rng)] if i % 3 == 0 else []
            cases.append({
                "sip_order": sip_order, "width": w, "height": h, "wcs": wcs_json(wcs),
                "x": [fl(t) for t in x], "y": [fl(t) for t in y],
                "iwc": [[fl(a), fl(b)] for a, b in zip(xi, eta)],
                "xyz": [[fl(t) for t in v] for v in xyz],
                "radec": [[fl(a), fl(b)] for a, b in zip(ra, dec)],
                "pix": [[fl(a), fl(b)] if o else None for a, b, o in zip(px, py, ok)],
                "pix_exact": [[fl(a), fl(b)] if o else None for a, b, o in zip(ex, ey, eok)],
                "pix_from_radec": [[fl(a), fl(b)] if o else None for a, b, o in zip(rx, ry, rok)],
                "behind_ok": bool(bok[0]),
                "scale_arcsec": wcs.pixel_scale_arcsec(), "parity": wcs.parity(),
                "crval_radec": list(wcs.crval_radec()),
                "comments": comments,
                "cards": cards_json(wcs.to_header_cards(w, h, comments)),
            })
    # COMMENT の折り返しだけを多数(header_cards の comments に 1 文ずつ渡したときの COMMENT のカード)
    tan = sw.SvWcs(sw.sv_radec_to_xyz(10.0, 20.0), (5.0, 5.0), np.eye(2) * 1e-4)
    wraps = []
    for _ in range(220):
        s = random_text(rng)
        lines = [v for k, v, _ in tan.to_header_cards(10, 10, [s]) if k == "COMMENT"]
        assert lines == (textwrap.wrap("".join(c if " " <= c <= "~" else "?" for c in s), 70) or [""])
        wraps.append({"text": s, "lines": lines})
    dump("wcs_proj.json", {"meta": meta(), "cases": cases, "wraps": wraps})


# -------------------------------------------------------------- 当てはめ ---

def gen_fit():
    rng = np.random.default_rng(1015)
    sim = []
    # 4 点(dev/test_solver_wcs.py の 08・09 と同じ作り方)と、3 点・6 点
    for i in range(40):
        typical = i % 2 == 1
        npt = 4 if i < 34 else (3 if i < 37 else 6)
        if typical:
            scale = 10 ** rng.uniform(math.log10(0.3), math.log10(3.0))
            w, h = int(rng.uniform(600, 4000)), int(rng.uniform(400, 3000))
            crval = (sw.sv_radec_to_xyz(*SPECIAL_CRVAL[i % len(SPECIAL_CRVAL)]) if i < 18
                     else sw.sv_normalize(rng.normal(size=3)))
            truth = sw.SvWcs(crval, ((w - 1) / 2, (h - 1) / 2), random_cd(rng, scale, 1 if i % 4 < 2 else -1))
            side = rng.uniform(0.1, 0.5) * math.hypot(w, h)
            c0 = rng.uniform([0, 0], [max(1.0, w - side), max(1.0, h - side)])
            f = c0 + rng.uniform(0, side, (npt, 2))
        else:
            scale = 10 ** rng.uniform(math.log10(0.3), math.log10(300.0))
            side = rng.uniform(50, 1500) if scale < 30 else rng.uniform(20, 300)
            f = rng.uniform(0, side, (npt, 2)) + rng.uniform(0, 3000, 2)
            crval = (sw.sv_radec_to_xyz(*SPECIAL_CRVAL[i % len(SPECIAL_CRVAL)]) if i < 18
                     else sw.sv_normalize(rng.normal(size=3)))
            truth = sw.SvWcs(crval, f.mean(axis=0), random_cd(rng, scale, 1 if i % 4 < 2 else -1))
        stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
        fit = sw.sv_fit_similarity(f, stars)
        sim.append({"field": f.tolist(), "stars": stars.tolist(), "expect": wcs_json(fit)})

    lsq = []

    def add_lsq(f, s, wts, crpix, order, crval, iters, size, note):
        try:
            fit = sw.sv_fit_wcs_lsq(f, s, weights=wts, crpix=crpix, sip_order=order, crval_xyz=crval,
                                    iterations=iters, width=None if size is None else size[0],
                                    height=None if size is None else size[1])
            exp = wcs_json(fit)
            err = None
        except ValueError as e:
            exp, err = None, str(e)
        lsq.append({"note": note, "field": [[fl(a), fl(b)] for a, b in f], "stars": [[fl(t) for t in v] for v in s],
                    "weights": None if wts is None else [fl(t) for t in wts],
                    "crpix": None if crpix is None else [fl(t) for t in crpix],
                    "sip_order": order, "crval": None if crval is None else [fl(t) for t in crval],
                    "iterations": iters, "size": size, "expect": exp, "error": err})

    for sip_order, ncase, npts in ((0, 7, 50), (1, 1, 40), (2, 7, 60), (3, 5, 80)):
        for i in range(ncase):
            truth, w, h = random_config(rng, i + 3 * sip_order, max(sip_order, 0))
            field_deg = math.hypot(w, h) * truth.pixel_scale_arcsec() / 3600.0
            f = np.column_stack([rng.uniform(-0.5, w - 0.5, npts), rng.uniform(-0.5, h - 0.5, npts)])
            s = truth.pix_to_xyz(f[:, 0], f[:, 1])
            fn = f + rng.normal(0, 0.3, f.shape)
            wts = None
            if i % 3 == 1:
                wts = rng.uniform(0.5, 2.0, npts)
                wts[rng.random(npts) < 0.15] = 0.0
                fn[wts == 0.0] += rng.uniform(5, 50, (int((wts == 0.0).sum()), 2))
            crpix = truth.crpix if i % 2 == 0 else None
            iters = 3 if field_deg <= 10.0 else 30
            size = (w, h) if i % 4 != 3 else None
            add_lsq(fn, s, wts, crpix, sip_order, None, iters, size,
                    f"SIP {sip_order} 次、{npts} 点、視野 {field_deg:.2f}°")
    # 接点を渡す・反復 0 回
    truth, w, h = random_config(rng, 30, 2, scale=1.46)
    f = np.column_stack([rng.uniform(0, w, 40), rng.uniform(0, h, 40)])
    s = truth.pix_to_xyz(f[:, 0], f[:, 1])
    add_lsq(f, s, None, None, 2, truth.crval_xyz, 0, (w, h), "接点を渡す・反復 0 回")
    # 縮退した入力(dev/test_solver_wcs.py の 16 と同じ)と、最少の点数
    wcs, w, h = random_config(rng, 12, 0, scale=1.0)
    f = rng.uniform(0, 500, (10, 2))
    s = wcs.pix_to_xyz(f[:, 0], f[:, 1])
    line = np.column_stack([np.linspace(0, 500, 10), np.linspace(0, 300, 10)])
    sl = wcs.pix_to_xyz(line[:, 0], line[:, 1])
    add_lsq(f[:2], s[:2], None, None, 0, None, 3, None, "TAN 2 点(縮退)")
    add_lsq(f[:5], s[:5], None, None, 2, None, 3, None, "SIP 2 次 5 点(縮退)")
    add_lsq(line, sl, None, None, 0, None, 3, None, "一直線(縮退)")
    add_lsq(f, s, np.zeros(10), None, 0, None, 3, None, "重みが全部 0")
    add_lsq(f[:3], s[:3], None, None, 0, None, 3, None, "TAN 3 点(最少)")
    add_lsq(f[:6], s[:6], None, None, 2, None, 3, (500, 500), "SIP 2 次 6 点(最少)")

    inv = []
    for i in range(6):
        order = 2 if i % 2 == 0 else 3
        wcs, w, h = random_config(rng, 40 + i, order)
        if i < 3:
            got = sw.sv_fit_sip_inverse(wcs, w, h)
            inv.append({"wcs": wcs_json(wcs), "size": [w, h], "bounds": None, "order": None, "expect": wcs_json(got)})
        else:
            bounds = (rng.uniform(0, 0.3 * w), rng.uniform(0.7 * w, w), rng.uniform(0, 0.3 * h), rng.uniform(0.7 * h, h))
            got = sw.sv_fit_sip_inverse(wcs, order=order + 2, bounds=bounds)
            inv.append({"wcs": wcs_json(wcs), "size": None, "bounds": list(bounds), "order": order + 2,
                        "expect": wcs_json(got)})
    dump("wcs_fit.json", {"meta": meta(), "similarity": sim, "lsq": lsq, "sip_inverse": inv})


# ------------------------------------------------------------ ヘッダー ---

# solve-field が書いた実物(scratch の oracle/truth.json の real16_fits と real_flip_fits、
# 116.56/18.51 は extra_02013I_00010)
SOLVE_FIELD = [
    ({"CTYPE1": "RA---TAN-SIP", "CTYPE2": "DEC--TAN-SIP", "CRVAL1": 61.9054382756, "CRVAL2": 41.9670267704,
      "CRPIX1": 459.098098755, "CRPIX2": 281.228652954, "CD1_1": -0.000361486860273, "CD1_2": 0.00018512743951,
      "CD2_1": -0.000184892385759, "CD2_2": -0.000361314701789, "IMAGEW": 968, "IMAGEH": 548, "A_ORDER": 2,
      "B_ORDER": 2, "AP_ORDER": 2, "BP_ORDER": 2, "EQUINOX": 2000.0, "LONPOLE": 180.0, "LATPOLE": 0.0},
     {"A_0_0": 0.0, "A_0_1": 0.0, "A_0_2": 2.98507130294e-06, "A_1_0": 0.0, "A_1_1": 5.37118129539e-06,
      "A_2_0": 1.36586248574e-06, "B_0_0": 0.0, "B_0_1": 0.0, "B_0_2": 1.51990499133e-05, "B_1_0": 0.0,
      "B_1_1": -1.00888286491e-06, "B_2_0": -5.84940310419e-07, "AP_0_0": -3.48139593983e-05,
      "AP_0_1": 6.2412188987e-06, "AP_0_2": -2.984571256e-06, "AP_1_0": 3.09907558116e-06,
      "AP_1_1": -5.37209843686e-06, "AP_2_0": -1.36590231872e-06, "BP_0_0": 0.000176574456665,
      "BP_0_1": 1.99727114191e-05, "BP_0_2": -1.52101612782e-05, "BP_1_0": -1.62416822558e-06,
      "BP_1_1": 1.0084463586e-06, "BP_2_0": 5.85038820244e-07}, 968, 548, "solve-field real16"),
    ({"CTYPE1": "RA---TAN-SIP", "CTYPE2": "DEC--TAN-SIP", "CRVAL1": 61.9054382725, "CRVAL2": 41.9670267739,
      "CRPIX1": 459.098098755, "CRPIX2": 267.771348953, "CD1_1": -0.000361486863787, "CD1_2": -0.000185127444355,
      "CD2_1": -0.000184892362121, "CD2_2": 0.000361314736178, "IMAGEW": 968, "IMAGEH": 548, "A_ORDER": 2,
      "B_ORDER": 2, "AP_ORDER": 2, "BP_ORDER": 2, "EQUINOX": 2000.0, "LONPOLE": 180.0, "LATPOLE": 0.0},
     {"A_0_0": 0.0, "A_0_1": 0.0, "A_0_2": 2.98494620983e-06, "A_1_0": 0.0, "A_1_1": -5.37115184544e-06,
      "A_2_0": 1.36586976501e-06, "B_0_0": 0.0, "B_0_1": 0.0, "B_0_2": -1.51986861229e-05, "B_1_0": 0.0,
      "B_1_1": -1.00895311043e-06, "B_2_0": 5.8484162407e-07, "AP_0_0": -4.10561933346e-05,
      "AP_0_1": -6.23910417394e-06, "AP_0_2": -2.98412577527e-06, "AP_1_0": 3.09980737061e-06,
      "AP_1_1": 5.37184361323e-06, "AP_2_0": -1.36589645512e-06, "BP_0_0": -0.000156578736458,
      "BP_0_1": 1.99910519416e-05, "BP_0_2": 1.52084188341e-05, "BP_1_0": 1.62356637531e-06,
      "BP_1_1": 1.00840704135e-06, "BP_2_0": -5.84915686985e-07}, 968, 548, "solve-field real_flip(鏡像)"),
    ({"CTYPE1": "RA---TAN-SIP", "CTYPE2": "DEC--TAN-SIP", "CRVAL1": 116.561870621, "CRVAL2": 18.5055496257,
      "CRPIX1": 345.732587814, "CRPIX2": 233.672239304, "CD1_1": -0.000301881190154, "CD1_2": 0.000273188385618,
      "CD2_1": -0.000273601414483, "CD2_2": -0.000300758432034, "IMAGEW": 968, "IMAGEH": 548, "A_ORDER": 2,
      "B_ORDER": 2, "AP_ORDER": 2, "BP_ORDER": 2, "EQUINOX": 2000.0, "LONPOLE": 180.0, "LATPOLE": 0.0},
     {"A_0_0": 0.0, "A_0_1": 0.0, "A_0_2": 3.54526704848e-05, "A_1_0": 0.0, "A_1_1": -2.43767290339e-05,
      "A_2_0": 1.86637899651e-05, "B_0_0": 0.0, "B_0_1": 0.0, "B_0_2": -1.11985185123e-05, "B_1_0": 0.0,
      "B_1_1": 5.62056290124e-06, "B_2_0": -5.15504299719e-06, "AP_0_0": -0.0165459840936,
      "AP_0_1": -0.000218336504772, "AP_0_2": -3.52903453659e-05, "AP_1_0": 0.000158587672987,
      "AP_1_1": 2.40655676403e-05, "AP_2_0": -1.83958249757e-05, "BP_0_0": 0.00442641480414,
      "BP_0_1": 5.8908944468e-05, "BP_0_2": 1.11560443858e-05, "BP_1_0": -4.25593456757e-05,
      "BP_1_1": -5.53618852402e-06, "BP_2_0": 5.08292104574e-06}, 968, 548, "solve-field 02013I"),
]


def astropy_read(items, w, h):
    """[(キー, 値)] を astropy に読ませる。戻り値 (WCS, 警告)。"""
    hdr = fits.Header()
    hdr["NAXIS"] = 2
    hdr["NAXIS1"] = w
    hdr["NAXIS2"] = h
    for k, v in items:
        if k == "COMMENT":
            hdr.add_comment(v)
        else:
            hdr[k] = v
    with warnings.catch_warnings(record=True) as ws:
        warnings.simplefilter("always")
        aw = WCS(hdr)
    return aw, [str(x.message)[:200] for x in ws]


def gen_header():
    rng = np.random.default_rng(2610)
    cases = []

    def add(note, items, w, h, expect_ok=True):
        entry = {"note": note, "cards": [{"k": k, "v": v, "c": ""} for k, v in items], "ok": expect_ok}
        if expect_ok:
            aw, warns = astropy_read(items, w, h)
            x = np.concatenate([[0.0, w - 1.0, 0.0, w - 1.0, (w - 1) / 2], rng.uniform(-0.1 * w, 1.1 * w, 10)])
            y = np.concatenate([[0.0, 0.0, h - 1.0, h - 1.0, (h - 1) / 2], rng.uniform(-0.1 * h, 1.1 * h, 10)])
            ra, dec = aw.all_pix2world(x, y, 0)
            bx, by = aw.all_world2pix(ra, dec, 0, tolerance=1e-12, maxiter=200, quiet=True)
            entry.update({"width": w, "height": h, "x": x.tolist(), "y": y.tolist(),
                          "radec": [[fl(a), fl(b)] for a, b in zip(ra, dec)],
                          "back": [[fl(a), fl(b)] for a, b in zip(bx, by)],
                          "scale_arcsec": float(math.sqrt(abs(np.linalg.det(aw.pixel_scale_matrix))) * 3600.0),
                          "det_positive": bool(np.linalg.det(aw.pixel_scale_matrix) > 0),
                          "astropy_warnings": warns})
        cases.append(entry)

    # Python 版の to_header_cards の出力(SIP 2 次 + 逆変換、TAN、極ちょうど)
    a, b = random_sip(rng, 2, 1000.0)
    wsip = sw.sv_fit_sip_inverse(sw.SvWcs(sw.sv_radec_to_xyz(61.9, 41.97), (459.1, 281.2),
                                          random_cd(rng, 1.46, -1), a, b), 968, 548)
    add("to_header_cards SIP 2 次 + 逆変換", [(k, v) for k, v, _ in wsip.to_header_cards(968, 548, ["x y"])], 968, 548)
    a, b = random_sip(rng, 3, 1500.0)
    wsip3 = sw.SvWcs(sw.sv_radec_to_xyz(250.0, -75.0), (1000.3, 700.6), random_cd(rng, 2.2, 1), a, b)
    add("to_header_cards SIP 3 次・逆変換なし", [(k, v) for k, v, _ in wsip3.to_header_cards(2000, 1400)], 2000, 1400)
    for ra0, de0 in [(0.0, 90.0), (0.0, -90.0), (359.9995, 0.0)]:
        wt = sw.SvWcs(sw.sv_radec_to_xyz(ra0, de0), (300.0, 200.0), random_cd(rng, 20.0, 1, 0.01))
        add(f"to_header_cards TAN ({ra0}, {de0})", [(k, v) for k, v, _ in wt.to_header_cards(600, 400)], 600, 400)

    base = [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRVAL1", 83.8221), ("CRVAL2", -5.3911),
            ("CRPIX1", 512.5), ("CRPIX2", 384.25)]
    cd = random_cd(rng, 1.7, -1, 0.01)
    cdk = [("CD1_1", cd[0, 0]), ("CD1_2", cd[0, 1]), ("CD2_1", cd[1, 0]), ("CD2_2", cd[1, 1])]
    add("CD だけ", base + cdk, 1024, 768)
    add("CD の一部だけ(無いものは 0)", base + [("CD1_1", -4.7e-4), ("CD2_2", 4.7e-4)], 1024, 768)
    add("PC + CDELT", base + [("CDELT1", -4.2e-4), ("CDELT2", 4.3e-4), ("PC1_1", 0.94), ("PC1_2", -0.33),
                              ("PC2_1", 0.34), ("PC2_2", 0.93)], 1024, 768)
    add("PC の一部 + CDELT", base + [("CDELT1", -4.2e-4), ("CDELT2", 4.3e-4), ("PC1_2", 0.05)], 1024, 768)
    add("CDELT + CROTA2", base + [("CDELT1", -4.2e-4), ("CDELT2", 4.2e-4), ("CROTA2", 23.5)], 1024, 768)
    add("CDELT + CROTA2(鏡像)", base + [("CDELT1", 4.2e-4), ("CDELT2", 4.2e-4), ("CROTA2", -151.0)], 1024, 768)
    add("CDELT だけ", base + [("CDELT1", -4.2e-4), ("CDELT2", 4.2e-4)], 1024, 768)
    add("CDELT + CROTA1 だけ(CROTA1 は使わない)", base + [("CDELT1", -4.2e-4), ("CDELT2", 4.2e-4), ("CROTA1", 30.0)],
        1024, 768)
    add("CD + CDELT + CROTA2(CD が勝つ)", base + cdk + [("CDELT1", -1e-3), ("CDELT2", 1e-3), ("CROTA2", 30.0)],
        1024, 768)
    add("PC + CROTA2(PC が勝つ)", base + [("CDELT1", -4.2e-4), ("CDELT2", 4.2e-4), ("PC1_1", 0.9), ("PC1_2", 0.1),
                                        ("CROTA2", 30.0)], 1024, 768)
    add("CUNIT deg・RADESYS FK5・EQUINOX", base + cdk + [("CUNIT1", "deg"), ("CUNIT2", "deg"), ("RADESYS", "FK5"),
                                                         ("EQUINOX", 2000.0)], 1024, 768)
    add("CTYPE の後ろに空白", [("CTYPE1", "RA---TAN    "), ("CTYPE2", "DEC--TAN    ")] + base[2:] + cdk, 1024, 768)
    add("CRPIX なし(既定 0)", base[:4] + cdk, 1024, 768)
    add("LONPOLE 150(極でない)", base + cdk + [("LONPOLE", 150.0)], 1024, 768)
    add("LONPOLE 180 + LATPOLE", base + cdk + [("LONPOLE", 180.0), ("LATPOLE", 0.0)], 1024, 768)
    cdp = random_cd(rng, 30.0, 1)
    cdpk = [("CD1_1", cdp[0, 0]), ("CD1_2", cdp[0, 1]), ("CD2_1", cdp[1, 0]), ("CD2_2", cdp[1, 1])]
    pole = [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRPIX1", 300.5), ("CRPIX2", 200.5)]
    add("北極ちょうど CRVAL1=137・LONPOLE なし(既定 0)", pole + [("CRVAL1", 137.0), ("CRVAL2", 90.0)] + cdpk, 600, 400)
    add("北極ちょうど CRVAL1=137・LONPOLE 180", pole + [("CRVAL1", 137.0), ("CRVAL2", 90.0), ("LONPOLE", 180.0)] + cdpk,
        600, 400)
    add("南極ちょうど CRVAL1=40・LONPOLE なし", pole + [("CRVAL1", 40.0), ("CRVAL2", -90.0)] + cdpk, 600, 400)
    add("南極ちょうど CRVAL1=40・LONPOLE 100", pole + [("CRVAL1", 40.0), ("CRVAL2", -90.0), ("LONPOLE", 100.0)] + cdpk,
        600, 400)
    add("極のそば 89.9999", pole + [("CRVAL1", 137.0), ("CRVAL2", 89.9999)] + cdpk, 600, 400)
    a = {"A_ORDER": 2, "B_ORDER": 2, "A_2_0": 3e-6, "A_1_1": -1e-6, "A_0_2": 2e-6, "B_2_0": 1e-6, "B_0_2": -4e-6}
    add("TAN-SIP 逆変換なし", [("CTYPE1", "RA---TAN-SIP"), ("CTYPE2", "DEC--TAN-SIP")] + base[2:] + cdk
        + list(a.items()), 1024, 768)
    add("CTYPE は TAN だが A_ORDER あり(astropy は SIP を掛ける)", base + cdk + list(a.items()), 1024, 768)
    add("CTYPE は TAN-SIP だが A_ORDER なし", [("CTYPE1", "RA---TAN-SIP"), ("CTYPE2", "DEC--TAN-SIP")] + base[2:] + cdk,
        1024, 768)
    add("A_ORDER 1(SIP なし)", base + cdk + [("A_ORDER", 1), ("B_ORDER", 1)], 1024, 768)
    for hdr, sip, w, h, note in SOLVE_FIELD:
        add(note, list(hdr.items()) + list(sip.items()), w, h)

    # 読めないもの(Rust は Err を返す。astropy は読めるものもあるが、この WCS の形では扱わない)
    bad = [
        ("投影 SIN", [("CTYPE1", "RA---SIN"), ("CTYPE2", "DEC--SIN")] + base[2:] + cdk),
        ("投影 TPV", [("CTYPE1", "RA---TPV"), ("CTYPE2", "DEC--TPV")] + base[2:] + cdk),
        ("銀河座標", [("CTYPE1", "GLON-TAN"), ("CTYPE2", "GLAT-TAN")] + base[2:] + cdk),
        ("軸の入れ替え", [("CTYPE1", "DEC--TAN"), ("CTYPE2", "RA---TAN")] + base[2:] + cdk),
        ("CTYPE なし", base[2:] + cdk),
        ("CRVAL1 なし", base[:2] + base[3:] + cdk),
        ("CD も CDELT も無い", base),
        ("CD が特異", base + [("CD1_1", 1e-4), ("CD1_2", 2e-4), ("CD2_1", 2e-4), ("CD2_2", 4e-4)]),
        ("A_ORDER だけ", [("CTYPE1", "RA---TAN-SIP"), ("CTYPE2", "DEC--TAN-SIP")] + base[2:] + cdk + [("A_ORDER", 2)]),
        ("SIP の 1 次の項", [("CTYPE1", "RA---TAN-SIP"), ("CTYPE2", "DEC--TAN-SIP")] + base[2:] + cdk
         + [("A_ORDER", 2), ("B_ORDER", 2), ("A_1_0", 1e-2)]),
        ("CUNIT arcsec", base + cdk + [("CUNIT1", "arcsec"), ("CUNIT2", "arcsec")]),
        ("CRVAL2 が 90 を超える", base[:3] + [("CRVAL2", 91.0)] + base[4:] + cdk),
    ]
    for note, items in bad:
        add(note, items, 1024, 768, expect_ok=False)
    dump("wcs_header.json", {"meta": meta(), "cases": cases})


# ---------------------------------------------------------------- ずれ ---

def gen_offsets():
    rng = np.random.default_rng(26101501)
    pairs = [((61.891414, 41.964760), (61.91199157844, 41.98626242835))]
    specials = [((359.99, 0.0), (0.01, 0.0)), ((0.01, 10.0), (359.98, 10.5)), ((10.0, 89.9), (190.0, 89.9)),
                ((0.0, 90.0), (123.0, 80.0)), ((45.0, -90.0), (10.0, -85.0)), ((10.0, 20.0), (190.0, -20.0)),
                ((10.0, 20.0), (10.0, 20.0)), ((10.0, 0.0), (10.0, 1e-9)), ((200.0, -30.0), (200.0 + 1e-7, -30.0))]
    pairs += specials
    for _ in range(150):
        v = rng.normal(size=3)
        c = sw.sv_xyz_to_radec(v / np.linalg.norm(v))
        dist = 10 ** rng.uniform(-6, math.log10(179.0))
        pa = rng.uniform(0, 360)
        t = SkyCoord(c[0] * u.deg, c[1] * u.deg).directional_offset_by(pa * u.deg, dist * u.deg)
        pairs.append(((float(c[0]), float(c[1])), (float(t.ra.deg), float(t.dec.deg))))
    out = []
    for (ra0, de0), (ra1, de1) in pairs:
        c = SkyCoord(ra0 * u.deg, de0 * u.deg, frame="icrs")
        t = SkyCoord(ra1 * u.deg, de1 * u.deg, frame="icrs")
        sep = c.separation(t)
        dra, ddec = c.spherical_offsets_to(t)
        pa = c.position_angle(t)
        back = c.directional_offset_by(pa, sep)
        out.append({"center": [ra0, de0], "target": [ra1, de1], "sep": float(sep.rad), "dra": float(dra.rad),
                    "ddec": float(ddec.rad), "pa": float(pa.rad), "offset_by": [float(back.ra.deg), float(back.dec.deg)]})
    dump("wcs_offsets.json", {"meta": meta(), "pairs": out})


if __name__ == "__main__":
    gen_proj()
    gen_fit()
    gen_header()
    gen_offsets()

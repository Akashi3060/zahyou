# ==============================================================================
#  solver_synth.py — 試験用: 索引の星から「正解つきの合成星野」を作る
#
#  docs/design-native-solver.md の 8 節・12 節。エンジンには入れない(build_nb.py はつながない)。
#  dev/test_solver_synth.py と dev/test_solver_real.py が使い、第 2 段(別の言語への移植)の試験も
#  ここで作った星野を JSON にして同じ入力に使う。乱数の種と索引ファイルが同じなら、出力は完全に同じ。
#  numpy と標準ライブラリ、solver_wcs / solver_index だけを使う。最上位の名前は sv_ / _sv_ / SV_ で始める。
#
#  約束(dev/solver_truth.json と同じ定義。試験の物差しはどれもこの定義で測る)
#    - 画素は 0 始まり(x = 列、y = 行)。配列の向きのまま(表示の上下反転 flip_y は考えない)。
#    - 中心 = 配列の中央 ((W−1)/2, (H−1)/2) が写る天球の位置。正解の WCS は CRPIX をここに置く。
#    - 回転 = 配列の +y(行が増える向き)の位置角。北から東回り、度、[0, 360)。
#    - パリティ = 配列座標での det CD の符号(+1 / −1)。mirror=True は det CD > 0
#      (y を上にして見ると空の鏡像。行 0 が画面の上の FITS(SharpCap など)を素直に写すとこちら)。
#      mirror=False は det CD < 0(y を上にすると北が上なら東が左 = 普通の空)。
#    - 画素スケール = 中心での画素 → 接平面の写像の √|det|(″/px)。SIP があれば SIP 込み。
#    - 星の明るさは等級ではなく「各系列の sweep の順位」から作る(5200 系には等級が無いため)。
#
#  星の集め方(実写と同じく、どの 1 つの索引よりも星が多い状態にする)
#    1. 視野に入る星を、手持ちの複数の系列(細かい段と粗い段)から集める。系列ごとに
#       sweep の小さい順(同じ値は乱数で)に並べた順位をつける。
#    2. 1″ 以内の重複(同じ星が複数の系列に入っている)は 1 つにまとめ、順位は最小のものを採る。
#       位置は Gaia(5200 系)を優先する(Tycho-2 の 4100 系とは元期の差で 1″ 弱ずれることがある)。
#    3. まとめた順位 → フラックス ∝ 1/(順位+1)、さらに 0.3 等ほどの揺らぎを掛ける(画像の帯域・飽和で
#       明るさの順がカタログと少し入れ替わるのを真似る)。
#    4. drop_frac の星を消し、位置に pos_noise_px の雑音を足し、全体の distractor_frac が偽物になるよう
#       一様乱数の偽の星を混ぜ、明るい順に n_stars 個を返す。
# ==============================================================================

import json
import math
import os

import numpy as np

from solver_wcs import *
from solver_index import *


SV_SYNTH_FORMAT = "zahyou-synth-field-1"
SV_SYNTH_DEDUP_ARCSEC = 1.0          # これより近い星は同じ星とみなす
SV_SYNTH_MAX_SERIES_STARS = 4000     # 視野に入る星の見込みがこれより多い系列は使わない(細かすぎる段)
SV_SYNTH_FLUX_TOP = 1.0e5            # いちばん明るい星のフラックス
SV_SYNTH_FLUX_JITTER_MAG = 0.3       # 明るさの揺らぎ(等級の標準偏差)
SV_SYNTH_TOL = {"center_arcsec": 2.0,   # 中心のずれ(″)。0.5 px のほうが大きければそちら
                "center_px": 0.5,
                "scale_rel": 0.002,     # 画素スケールの相対差
                "rot_deg": 0.1}         # 回転(+y の位置角)の差(度)


# ------------------------------------------------------------------ 正解の WCS ---

def sv_synth_cd(scale_arcsec, rotation_deg, mirror):
    """画素スケール・+y の位置角・鏡像から CD(度/px)を作る。

    +y の列 = s·(sin θ, cos θ)(東, 北)。+x の列は鏡像でなければ s·(−cos θ, sin θ)(det < 0)、
    鏡像なら s·(cos θ, −sin θ)(det > 0)。θ = 0・鏡像なしなら +y が北、+x が西(北が上・東が左)。
    """
    s = float(scale_arcsec) / 3600.0
    th = math.radians(float(rotation_deg))
    m = 1.0 if mirror else -1.0
    c, n = math.cos(th), math.sin(th)
    return s * np.array([[m * c, n], [-m * n, c]])


def sv_synth_sip_radial(width, height, corner_px, order=3):
    """隅で corner_px 画素だけ外へずれる放射状の歪み(糸巻き型。負なら樽型)の SIP 係数 (A, B)。

    u' = u (1 + k r²)、v' = v (1 + k r²)、k = corner_px / r_c³(r_c = 中心から隅までの画素数)。
    """
    rc = math.hypot((float(width) - 1.0) / 2.0, (float(height) - 1.0) / 2.0)
    k = float(corner_px) / rc ** 3
    n = max(3, int(order))
    a = np.zeros((n + 1, n + 1))
    b = np.zeros((n + 1, n + 1))
    a[3, 0] = a[1, 2] = k                    # k u (u² + v²)
    b[2, 1] = b[0, 3] = k                    # k v (u² + v²)
    return a, b


def sv_synth_truth_wcs(center_ra, center_dec, scale_arcsec, width, height,
                       rotation_deg=0.0, mirror=False, sip=None):
    """正解の WCS。CRVAL = 中心、CRPIX = 配列の中央、CD は sv_synth_cd。

    sip: None(TAN だけ)/ 数(隅でのずれ [px]。sv_synth_sip_radial)/ (A, B) の係数。
    SIP があれば逆変換 AP, BP も画像全体の格子で当てておく。
    """
    cd = sv_synth_cd(scale_arcsec, rotation_deg, mirror)
    crpix = ((float(width) - 1.0) / 2.0, (float(height) - 1.0) / 2.0)
    a = b = None
    if sip is not None:
        if isinstance(sip, (int, float)) and not isinstance(sip, bool):
            if float(sip) != 0.0:
                a, b = sv_synth_sip_radial(width, height, float(sip))
        else:
            a, b = (np.asarray(sip[0], dtype=np.float64), np.asarray(sip[1], dtype=np.float64))
    w = SvWcs(sv_radec_to_xyz(center_ra, center_dec), crpix, cd, a, b)
    if w.has_sip:
        w = sv_fit_sip_inverse(w, width, height)
    return w


# ------------------------------------------------------------------ 物差し ---

def _sv_synth_jacobian(pts, basis_xyz, h):
    """中心と ±h の 4 点(pts[0] が中心、続いて +x, −x, +y, −y)を basis_xyz の接平面(東, 北)に投影し、
    画素 → 接平面の写像 J の 2 列 (∂/∂x, ∂/∂y) [rad/px] を返す。"""
    xi, eta, ok = sv_tan_project(pts[1:], basis_xyz)
    if not np.all(ok):
        raise ValueError("中心のまわりの点を接平面に投影できない")
    jx = np.array([xi[0] - xi[1], eta[0] - eta[1]]) / (2.0 * h)
    jy = np.array([xi[2] - xi[3], eta[2] - eta[3]]) / (2.0 * h)
    return jx, jy


def sv_synth_measure(wcs, width, height, step=1.0, ref_xyz=None):
    """WCS の中心・画素スケール・+y の位置角・パリティを測る(試験の物差し。約束は冒頭)。

    wcs は SvWcs か、画素 (x, y)(0 始まり、配列の向き。numpy の配列)→ 単位ベクトル (…, 3) を返す関数
    (astropy の WCS を測るときは呼ぶ側で包む。原点 0 で all_pix2world を呼ぶこと)。
    中心の点で、画素の ±step の差分から接平面(東, 北)への写像 J を出す。
    ref_xyz(正解の中心)を渡すと、同じ差分をその点の接平面でも測った位置角 "pa_ref_deg" を足す。
    位置角は「その点の北」が基準なので、極のそばでは中心が 1″ ずれただけで大きく変わる
    (赤緯 89.95° なら 1″ で 0.3°)。回転を比べるときは正解の中心の北で測ったこの値を使う。
    """
    fn = wcs.pix_to_xyz if hasattr(wcs, "pix_to_xyz") else wcs
    cx, cy = (float(width) - 1.0) / 2.0, (float(height) - 1.0) / 2.0
    h = float(step)
    xs = np.array([cx, cx + h, cx - h, cx, cx])
    ys = np.array([cy, cy, cy, cy + h, cy - h])
    pts = sv_normalize(np.asarray(fn(xs, ys), dtype=np.float64).reshape(5, 3))
    c = pts[0]
    jx, jy = _sv_synth_jacobian(pts, c, h)
    det = float(jx[0] * jy[1] - jy[0] * jx[1])
    ra, dec = sv_xyz_to_radec(c)
    out = {"center_ra": float(ra), "center_dec": float(dec), "center_xyz": c.tolist(),
           "scale_arcsec": math.sqrt(abs(det)) * SV_ARCSEC_PER_RAD,
           "pa_up_deg": math.degrees(math.atan2(jy[0], jy[1])) % 360.0,
           "parity": 1 if det >= 0.0 else -1}
    if ref_xyz is not None:
        ref = sv_normalize(np.asarray(ref_xyz, dtype=np.float64).reshape(3))
        _, ry = _sv_synth_jacobian(pts, ref, h)
        out["ref_xyz"] = ref.tolist()
        out["pa_ref_deg"] = math.degrees(math.atan2(ry[0], ry[1])) % 360.0
    return out


def sv_synth_angle_diff(a_deg, b_deg):
    """角度の差 a − b を (−180, 180] に。"""
    d = (float(a_deg) - float(b_deg)) % 360.0
    return d - 360.0 if d > 180.0 else d


def sv_synth_compare(got, want, tol=None):
    """測った値 got を正解 want(どちらも sv_synth_measure の形)と比べる(設計書 8 節の基準)。

    中心は max(center_arcsec, center_px × 正解のスケール) 以内、スケールの相対差・回転の差は tol 以内、
    パリティは一致。回転は、got を正解の中心の北で測った値(sv_synth_measure の ref_xyz)があればそれで比べる。
    戻り値 {"ok", "failed": [項目], "center_arcsec", "center_tol_arcsec", "scale_rel", "rot_deg",
    "parity": [got, want]}。
    """
    t = dict(SV_SYNTH_TOL)
    if tol:
        t.update(tol)
    if "center_xyz" in got and "center_xyz" in want:
        sep = float(sv_angle_between(np.asarray(got["center_xyz"]), np.asarray(want["center_xyz"])))
    else:
        sep = float(sv_angle_between(sv_radec_to_xyz(got["center_ra"], got["center_dec"]),
                                     sv_radec_to_xyz(want["center_ra"], want["center_dec"])))
    sep *= SV_ARCSEC_PER_RAD
    ctol = max(t["center_arcsec"], t["center_px"] * float(want["scale_arcsec"]))
    srel = float(got["scale_arcsec"]) / float(want["scale_arcsec"]) - 1.0
    pa = got["pa_up_deg"]
    if "pa_ref_deg" in got and "center_xyz" in want:
        if float(sv_angle_between(np.asarray(got["ref_xyz"]), np.asarray(want["center_xyz"]))) < 1e-9:
            pa = got["pa_ref_deg"]
    drot = sv_synth_angle_diff(pa, want["pa_up_deg"])
    checks = [("center", sep <= ctol), ("scale", abs(srel) <= t["scale_rel"]),
              ("rot", abs(drot) <= t["rot_deg"]), ("parity", int(got["parity"]) == int(want["parity"]))]
    failed = [k for k, ok in checks if not ok]
    return {"ok": not failed, "failed": failed, "center_arcsec": sep, "center_tol_arcsec": ctol,
            "scale_rel": srel, "rot_deg": drot, "parity": [int(got["parity"]), int(want["parity"])]}


# ------------------------------------------------------------------ 星を集める ---

def sv_synth_expected_stars(catalog, indexid, scale_arcsec, width, height):
    """系列 indexid の星が視野(width × height 画素、scale_arcsec)に入る数の見込み(面積比)。"""
    fs = catalog.series.get(indexid) or []
    if not fs:
        return 0.0
    if fs[0].allsky:
        dens = fs[0].nstars / (4.0 * math.pi)
    else:
        tile_sr = 4.0 * math.pi / (12.0 * fs[0].hpnside ** 2)
        dens = float(np.mean([f.nstars for f in fs])) / tile_sr
    area = float(width) * float(height) * (float(scale_arcsec) / SV_ARCSEC_PER_RAD) ** 2
    return dens * area


def sv_synth_series_choice(catalog, scale_arcsec, width, height, max_stars=SV_SYNTH_MAX_SERIES_STARS):
    """星を集める系列: 視野に入る星の見込みが max_stars 以下の系列を全部(番号の順)。

    どれも多すぎるときは、いちばん少ないもの 1 つ。
    """
    est = {iid: sv_synth_expected_stars(catalog, iid, scale_arcsec, width, height)
           for iid in sorted(catalog.series)}
    out = [iid for iid, n in est.items() if n <= max_stars]
    if not out and est:
        out = [min(est, key=lambda iid: (est[iid], iid))]
    return out


def _sv_synth_field_radius(truth, width, height):
    """視野の中心から、画像の隅・辺の中点までの最大の角距離(ラジアン)に余裕を足したもの。"""
    W, H = float(width), float(height)
    xs = np.array([-0.5, W / 2 - 0.5, W - 0.5, -0.5, W - 0.5, -0.5, W / 2 - 0.5, W - 0.5])
    ys = np.array([-0.5, -0.5, -0.5, H / 2 - 0.5, H / 2 - 0.5, H - 0.5, H - 0.5, H - 0.5])
    edge = truth.pix_to_xyz(xs, ys)
    c = truth.pix_to_xyz((W - 1.0) / 2.0, (H - 1.0) / 2.0)
    r = float(np.max(sv_angle_between(edge, c[None, :])))
    return r * 1.02 + 1e-5


def _sv_synth_gather(catalog, series, truth, width, height, rng):
    """系列ごとに視野の星を集め、sweep の順位をつけて 1″ 以内の重複をまとめる。

    戻り値 (xyz (M,3), xy (M,2) 雑音なしの画素, rank (M,) まとめた順位, source [str], per_series {id: 数})。
    """
    W, H = float(width), float(height)
    center = truth.pix_to_xyz((W - 1.0) / 2.0, (H - 1.0) / 2.0)
    radius = _sv_synth_field_radius(truth, width, height)
    rows = []                                      # (rank, priority, iid, xyz, x, y, source)
    per_series = {}
    for iid in sorted(series):
        xyz_l, sw_l, src_l = [], [], []
        for f in catalog.tiles_near(iid, center, radius):
            sid = f.stars_in_cone(center, radius)
            if len(sid) == 0:
                continue
            xyz_l.append(f.star_xyz(sid))
            sw_l.append(f.sweep(sid).astype(np.int64))
            src_l.extend(f"{f.name}#{int(s)}" for s in sid)
        if not xyz_l:
            per_series[iid] = 0
            continue
        xyz = np.concatenate(xyz_l)
        sw = np.concatenate(sw_l)
        x, y, ok = truth.xyz_to_pix(xyz, exact=True)
        inside = ok & (x >= -0.5) & (x <= W - 0.5) & (y >= -0.5) & (y <= H - 0.5)
        idx = np.nonzero(inside)[0]
        tie = rng.random(len(idx))                 # 同じ sweep の値は乱数で並べる
        order = idx[np.lexsort((tie, sw[idx]))]
        per_series[iid] = int(len(order))
        prio = 0 if iid >= 5000 else 1             # 位置は Gaia(5200 系)を優先
        for rank, i in enumerate(order):
            rows.append((rank, prio, iid, xyz[i], float(x[i]), float(y[i]), src_l[i]))
    if not rows:
        return (np.zeros((0, 3)), np.zeros((0, 2)), np.zeros(0, np.int64), [], per_series)

    rows.sort(key=lambda r: (r[0], r[1], r[2]))
    tol = SV_SYNTH_DEDUP_ARCSEC / SV_ARCSEC_PER_RAD   # 弦 ≒ 角(ラジアン)
    cells = {}
    kept = []                                      # [rank, prio, xyz, x, y, source]
    for rank, prio, iid, v, x, y, src in rows:
        key = tuple(int(math.floor(t / tol)) for t in v)
        hit = None
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    for k in cells.get((key[0] + dx, key[1] + dy, key[2] + dz), ()):
                        if float(np.sum((kept[k][2] - v) ** 2)) <= tol * tol:
                            hit = k
                            break
                    if hit is not None:
                        break
                if hit is not None:
                    break
            if hit is not None:
                break
        if hit is None:
            cells.setdefault(key, []).append(len(kept))
            kept.append([rank, prio, v, x, y, src])
        elif prio < kept[hit][1]:                  # 順位はそのまま(先に来たほうが小さい)、位置だけ替える
            kept[hit][1:] = [prio, v, x, y, src]
    xyz = np.array([k[2] for k in kept])
    xy = np.array([[k[3], k[4]] for k in kept])
    rank = np.array([k[0] for k in kept], dtype=np.int64)
    source = [k[5] for k in kept]
    return xyz, xy, rank, source, per_series


def _sv_synth_jitter(rng, n):
    return 10.0 ** (-0.4 * rng.normal(0.0, SV_SYNTH_FLUX_JITTER_MAG, n))


# ------------------------------------------------------------------ 合成星野 ---

def sv_synth_field(catalog, center_ra, center_dec, scale_arcsec, width, height,
                   rotation_deg=0.0, mirror=False, sip=None, seed=0, n_stars=60,
                   drop_frac=0.3, distractor_frac=0.3, pos_noise_px=0.3, series=None):
    """索引の星から正解つきの合成星野を作る(作り方は冒頭)。

    catalog: SvIndexCatalog。center_ra, center_dec: 配列の中央の位置(度)。scale_arcsec: ″/px。
    rotation_deg: 配列の +y の位置角。mirror: det CD > 0 にするか。sip: sv_synth_truth_wcs。
    n_stars: 返す星の数(明るい順の上位。None なら全部)。series: 星を集める系列(None なら
    sv_synth_series_choice)。
    戻り値 {"xy" (N,2) 明るい順, "flux" (N,), "truth" SvWcs, "width", "height", "meta" {…}}。
    meta は JSON にそのまま書ける: "kind" = "sky"、"params"(この関数の引数。series は実際に使ったもの)、
    "series"(系列ごとの視野内の星の数)、"counts"、"truth_summary"(sv_synth_measure)、
    星ごとの "is_real"・"star_xyz"(偽の星は None)・"source"(「索引名#星番号」。偽の星は None)。
    """
    W, H = int(width), int(height)
    rng = np.random.default_rng(int(seed))
    truth = sv_synth_truth_wcs(center_ra, center_dec, scale_arcsec, W, H, rotation_deg, mirror, sip)
    if series is None:
        series = sv_synth_series_choice(catalog, scale_arcsec, W, H)
    series = sorted(int(s) for s in series if int(s) in catalog.series)
    xyz, xy0, rank, source, per_series = _sv_synth_gather(catalog, series, truth, W, H, rng)
    M = len(xyz)

    # まとめた順位 → 明るさ(同じ順位は乱数で並べる)
    tie = rng.random(M)
    order = np.lexsort((tie, rank))
    pos = np.empty(M, dtype=np.int64)
    pos[order] = np.arange(M)
    flux = SV_SYNTH_FLUX_TOP / (pos + 1.0) * _sv_synth_jitter(rng, M)

    # 欠け・位置の雑音
    n_drop = int(round(float(drop_frac) * M))
    keep = np.ones(M, dtype=bool)
    if n_drop > 0:
        keep[rng.choice(M, n_drop, replace=False)] = False
    kidx = np.nonzero(keep)[0]
    kxy = xy0[kidx] + rng.normal(0.0, float(pos_noise_px), (len(kidx), 2))
    inside = (kxy[:, 0] >= -0.5) & (kxy[:, 0] <= W - 0.5) & (kxy[:, 1] >= -0.5) & (kxy[:, 1] <= H - 0.5)
    kidx, kxy = kidx[inside], kxy[inside]
    kflux = flux[kidx]
    n_real = len(kidx)

    # 偽の星: 全体の distractor_frac が偽物になる数。明るさは本物の分布から取る
    d = float(distractor_frac)
    n_fake = int(round(n_real * d / (1.0 - d))) if 0.0 < d < 1.0 else 0
    fxy = np.column_stack([rng.uniform(-0.5, W - 0.5, n_fake), rng.uniform(-0.5, H - 0.5, n_fake)])
    if n_fake and n_real:
        fflux = kflux[rng.integers(0, n_real, n_fake)] * _sv_synth_jitter(rng, n_fake)
    else:
        fflux = SV_SYNTH_FLUX_TOP / (np.arange(n_fake) + 1.0) * _sv_synth_jitter(rng, n_fake)

    all_xy = np.concatenate([kxy, fxy]) if n_fake else kxy
    all_flux = np.concatenate([kflux, fflux]) if n_fake else kflux
    real = np.concatenate([np.ones(n_real, bool), np.zeros(n_fake, bool)])
    srt = np.argsort(-all_flux, kind="stable")
    if n_stars is not None:
        srt = srt[:int(n_stars)]
    out_xy = all_xy[srt].reshape(-1, 2)
    out_flux = all_flux[srt]
    out_real = real[srt]
    star_xyz, star_src = [], []
    for j in srt:
        if j < n_real:
            star_xyz.append(xyz[kidx[j]].tolist())
            star_src.append(source[kidx[j]])
        else:
            star_xyz.append(None)
            star_src.append(None)

    sip_param = sip
    if sip is not None and not (isinstance(sip, (int, float)) and not isinstance(sip, bool)):
        sip_param = [np.asarray(sip[0], dtype=np.float64).tolist(), np.asarray(sip[1], dtype=np.float64).tolist()]
    params = {"center_ra": float(center_ra), "center_dec": float(center_dec),
              "scale_arcsec": float(scale_arcsec), "width": W, "height": H,
              "rotation_deg": float(rotation_deg), "mirror": bool(mirror), "sip": sip_param,
              "seed": int(seed), "n_stars": None if n_stars is None else int(n_stars),
              "drop_frac": float(drop_frac), "distractor_frac": d,
              "pos_noise_px": float(pos_noise_px), "series": series}
    meta = {"kind": "sky", "params": params,
            "series": {str(k): int(v) for k, v in sorted(per_series.items())},
            "counts": {"catalog_stars": int(M), "dropped": int(n_drop), "real": int(n_real),
                       "distractors": int(n_fake), "out": int(len(srt)),
                       "out_real": int(out_real.sum()), "out_distractors": int((~out_real).sum())},
            "truth_summary": sv_synth_measure(truth, W, H),
            "is_real": out_real.tolist(), "star_xyz": star_xyz, "source": star_src}
    return {"xy": out_xy, "flux": out_flux, "truth": truth, "width": W, "height": H, "meta": meta}


def sv_synth_random_field(width, height, n_stars=60, seed=0):
    """一様乱数の星野(実在の空ではない。解けてはいけない)。形は sv_synth_field と同じで truth は None。"""
    W, H = int(width), int(height)
    rng = np.random.default_rng(int(seed))
    n = int(n_stars)
    xy = np.column_stack([rng.uniform(-0.5, W - 0.5, n), rng.uniform(-0.5, H - 0.5, n)])
    flux = SV_SYNTH_FLUX_TOP / (np.arange(n) + 1.0) * _sv_synth_jitter(rng, n)
    srt = np.argsort(-flux, kind="stable")
    meta = {"kind": "random",
            "params": {"width": W, "height": H, "n_stars": n, "seed": int(seed)},
            "counts": {"out": n, "out_real": 0, "out_distractors": n},
            "is_real": [False] * n, "star_xyz": [None] * n, "source": [None] * n}
    return {"xy": xy[srt], "flux": flux[srt], "truth": None, "width": W, "height": H, "meta": meta}


# ------------------------------------------------------------------ JSON ---

def sv_synth_to_json(field):
    """合成星野を JSON に書ける dict に(浮動小数はそのまま。Python の json は往復で値が変わらない)。"""
    t = field.get("truth")
    return {"format": SV_SYNTH_FORMAT, "width": int(field["width"]), "height": int(field["height"]),
            "xy": np.asarray(field["xy"], dtype=np.float64).tolist(),
            "flux": np.asarray(field["flux"], dtype=np.float64).tolist(),
            "truth": None if t is None else t.to_dict(), "meta": field["meta"]}


def sv_synth_from_json(d):
    """sv_synth_to_json の逆。"""
    if d.get("format") != SV_SYNTH_FORMAT:
        raise ValueError(f"合成星野の形式が違います: {d.get('format')!r}")
    t = d.get("truth")
    truth = None
    if t is not None:
        truth = SvWcs(t["crval_xyz"], t["crpix"], t["cd"], t.get("a"), t.get("b"), t.get("ap"), t.get("bp"))
    return {"xy": np.asarray(d["xy"], dtype=np.float64).reshape(-1, 2),
            "flux": np.asarray(d["flux"], dtype=np.float64).reshape(-1),
            "truth": truth, "width": int(d["width"]), "height": int(d["height"]), "meta": d["meta"]}


def sv_synth_save(field, path):
    with open(os.fspath(path), "w", encoding="utf-8") as f:
        json.dump(sv_synth_to_json(field), f, ensure_ascii=False)


def sv_synth_load(path):
    with open(os.fspath(path), "r", encoding="utf-8") as f:
        return sv_synth_from_json(json.load(f))

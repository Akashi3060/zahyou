"""自前ソルバーの幾何(solver_wcs.py)の試験。投影の往復、astropy の WCS との一致、当てはめ。"""
import json
import math
import os
import sys
import warnings

import numpy as np
from astropy.io import fits
from astropy.wcs import WCS
from numpy.polynomial.polynomial import polyval2d

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from solver_wcs import *                                   # noqa: E402,F403

ARCSEC = SV_ARCSEC_PER_RAD
RESULTS = []

# 接点の特別な場所: 北極・南極ちょうど、極のすぐそば、赤経 0° のまたぎ、南天
SPECIAL_CRVAL = [(0.0, 90.0), (0.0, -90.0), (137.0, 89.9999), (300.0, -89.99),
                 (359.9995, 0.0), (0.0004, -45.0), (359.99, 61.0), (250.0, -75.0), (83.8, -5.4)]


def record(name, ok, msg):
    RESULTS.append((name, bool(ok), msg))


def rot(theta_deg):
    t = math.radians(theta_deg)
    return np.array([[math.cos(t), -math.sin(t)], [math.sin(t), math.cos(t)]])


def random_cd(rng, scale_arcsec, parity, skew=0.0):
    """回転は乱数、det の符号 = parity、√|det| = scale。skew は |det| を変えないずれ。"""
    m = rot(rng.uniform(0, 360)) @ np.diag([1.0, float(parity)]) @ np.array([[1.0, skew], [0.0, 1.0]])
    return m * (scale_arcsec / 3600.0)


def random_sip(rng, order, half_diag, amp=(3.0, 1.0, 0.5)):
    """画像の隅で amp[d−2] px ほど動く SIP の A, B。"""
    a = np.zeros((order + 1, order + 1))
    b = np.zeros((order + 1, order + 1))
    for d in range(2, order + 1):
        for p in range(d + 1):
            a[p, d - p] = rng.normal() * amp[d - 2] / half_diag ** d
            b[p, d - p] = rng.normal() * amp[d - 2] / half_diag ** d
    return a, b


def random_config(rng, i, sip_order=0, scale=None):
    """乱数の WCS と画像の大きさ。最初の数件は SPECIAL_CRVAL を接点にする。"""
    if i < len(SPECIAL_CRVAL):
        crval = sv_radec_to_xyz(*SPECIAL_CRVAL[i])
    else:
        crval = sv_normalize(rng.normal(size=3))
    if scale is None:
        scale = 10 ** rng.uniform(math.log10(0.3), math.log10(300.0))
    w = int(rng.uniform(300, 4000))
    h = int(rng.uniform(200, 3000))
    lim = 100.0 / (scale / 3600.0)                 # 対角が接平面で 100° 分を超えないように
    f = min(1.0, lim / math.hypot(w, h))
    w, h = max(50, int(w * f)), max(40, int(h * f))
    parity = 1 if i % 2 == 0 else -1
    skew = 0.02 * rng.normal() if i % 3 == 0 else 0.0
    cd = random_cd(rng, scale, parity, skew)
    crpix = (rng.uniform(0.2, 0.8) * w, rng.uniform(0.2, 0.8) * h)
    a = b = None
    if sip_order >= 2:
        a, b = random_sip(rng, sip_order, 0.5 * math.hypot(w, h))
    return SvWcs(crval, crpix, cd, a, b), w, h


def astropy_wcs(wcs, w, h, comments=()):
    """to_header_cards → fits.Header → WCS。警告が出たらその文を返す。"""
    hdr = fits.Header()
    hdr["NAXIS"] = 2
    hdr["NAXIS1"] = w
    hdr["NAXIS2"] = h
    hdr.extend(wcs.to_header_cards(w, h, comments))
    with warnings.catch_warnings(record=True) as ws:
        warnings.simplefilter("always")
        aw = WCS(hdr)
    return aw, hdr, [str(x.message) for x in ws]


def sep_arcsec(ra1, de1, ra2, de2):
    return sv_angle_between(sv_radec_to_xyz(ra1, de1), sv_radec_to_xyz(ra2, de2)) * ARCSEC


# ------------------------------------------------------------- 投影の往復 ---

def case_01_projection_roundtrip():
    rng = np.random.default_rng(101)
    centers = [np.array([0.0, 0.0, 1.0]), np.array([0.0, 0.0, -1.0])]
    centers += [sv_radec_to_xyz(ra, de) for ra, de in SPECIAL_CRVAL[2:]]
    centers += [sv_normalize(rng.normal(size=3)) for _ in range(20)]
    worst_xyz = worst_tan = worst_xi = 0.0
    for r in centers:
        # 接点のまわり 60° 以内の星を、投影を通さずに作る
        n = 400
        th = np.radians(60.0) * np.sqrt(rng.uniform(0, 1, n))
        e, nn = sv_tan_basis(r)
        phi = rng.uniform(0, 2 * np.pi, n)
        d = np.cos(phi)[:, None] * e + np.sin(phi)[:, None] * nn
        s = np.cos(th)[:, None] * r + np.sin(th)[:, None] * d
        xi, eta, ok = sv_tan_project(s, r)
        back = sv_tan_deproject(xi, eta, r)
        worst_xyz = max(worst_xyz, float(np.max(sv_angle_between(back, s))) * ARCSEC)
        # 心射: 接平面の距離 = tan(接点からの角)
        worst_tan = max(worst_tan, float(np.max(np.abs(np.hypot(xi, eta) - np.tan(th)) / (1 + np.tan(th)))))
        # 接平面 → 球 → 接平面
        x2, e2, ok2 = sv_tan_project(back, r)
        worst_xi = max(worst_xi, float(np.max(np.hypot(x2 - xi, e2 - eta))) * ARCSEC)
        if not (ok.all() and ok2.all()):
            record("01 投影の往復", False, "投影できない星が出た")
            return
    record("01 投影の往復 (極・赤経 0°・南天、60° まで)",
           worst_xyz < 1e-6 and worst_tan < 1e-12 and worst_xi < 1e-6,
           f"球→平面→球 {worst_xyz:.1e}″  平面→球→平面 {worst_xi:.1e}″  |ξη|=tanθ の相対誤差 {worst_tan:.1e}")


def case_02_projection_orientation():
    """東 = 赤経が増える向き、北 = 赤緯が増える向き。赤経 0° をまたいでも連続。反対の半球は投影しない。"""
    msgs = []
    ok_all = True
    for ra0, de0 in [(359.995, 0.0), (0.002, -40.0), (180.0, 60.0), (250.0, -75.0)]:
        r = sv_radec_to_xyz(ra0, de0)
        d = 0.01
        ce = math.cos(math.radians(de0))
        xi_e, eta_e, _ = sv_tan_project(sv_radec_to_xyz(ra0 + d / ce, de0), r)
        xi_n, eta_n, _ = sv_tan_project(sv_radec_to_xyz(ra0, de0 + d), r)
        good = (abs(math.degrees(xi_e) - d) < 1e-6 and abs(math.degrees(eta_e)) < 1e-5
                and abs(math.degrees(eta_n) - d) < 1e-6 and abs(math.degrees(xi_n)) < 1e-9)
        ok_all &= good
        msgs.append(f"({ra0},{de0}) 東 ξ={math.degrees(xi_e):.6f}° 北 η={math.degrees(eta_n):.6f}°")
    # 赤経 0° のまたぎ: 359.99° と 0.01° は ξ で 0.02° 離れているだけ
    r = sv_radec_to_xyz(0.0, 0.0)
    xa, _, _ = sv_tan_project(sv_radec_to_xyz(359.99, 0.0), r)
    xb, _, _ = sv_tan_project(sv_radec_to_xyz(0.01, 0.0), r)
    wrap = abs(math.degrees(xb - xa) - 0.02) < 1e-9
    # 反対の半球(90° を少しでも越えたら投影しない)
    xi, eta, ok = sv_tan_project(np.array([-r, r, sv_radec_to_xyz(90.001, 0.0), sv_radec_to_xyz(89.999, 0.0)]), r)
    back = (not ok[0]) and np.isnan(xi[0]) and ok[1] and not ok[2] and ok[3]
    record("02 投影の向き (東・北・赤経 0° のまたぎ・反対の半球)", ok_all and wrap and back,
           "; ".join(msgs[:2]) + f"; またぎ {math.degrees(xb - xa):.6f}°; 反対側 ok={list(ok)}")


# ------------------------------------------------------ astropy の WCS と比べる ---

def compare_with_astropy(rng, wcs, w, h, npts=400):
    """乱数の画素で、SvWcs と astropy の WCS の行き来を比べる。戻り値は誤差(″)の辞書と警告。"""
    aw, hdr, warns = astropy_wcs(wcs, w, h)
    x = rng.uniform(-0.5, w - 0.5, npts)
    y = rng.uniform(-0.5, h - 0.5, npts)
    scale = wcs.pixel_scale_arcsec()
    out = {}
    ra_a, de_a = aw.all_pix2world(x, y, 0)
    ra_s, de_s = wcs.pix_to_radec(x, y)
    out["pix→天"] = float(np.max(sep_arcsec(ra_a, de_a, ra_s, de_s)))
    # 天 → 画素(SIP があればニュートン法で正確に)
    px, py, ok = wcs.radec_to_pix(ra_a, de_a, exact=True)
    out["天→pix"] = float(np.max(np.hypot(px - x, py - y))) * scale if ok.all() else math.inf
    if wcs.has_sip:
        ax, ay = aw.all_world2pix(ra_a, de_a, 0, tolerance=1e-12, maxiter=100, quiet=True)
        out["天→pix(astropy)"] = float(np.max(np.hypot(px - ax, py - ay))) * scale
        if wcs.ap is not None:
            # AP, BP の近似: astropy が読んだ係数で U + AP(U, V) を計算したものと比べる
            fx, fy = aw.wcs_world2pix(ra_a, de_a, 0)          # SIP を掛けない = CRPIX + CD⁻¹(ξ, η)
            uu = fx - (aw.wcs.crpix[0] - 1.0)
            vv = fy - (aw.wcs.crpix[1] - 1.0)
            ex = aw.wcs.crpix[0] - 1.0 + uu + polyval2d(uu, vv, aw.sip.ap)
            ey = aw.wcs.crpix[1] - 1.0 + vv + polyval2d(uu, vv, aw.sip.bp)
            qx, qy, _ = wcs.radec_to_pix(ra_a, de_a)
            out["AP/BP"] = float(np.max(np.hypot(qx - ex, qy - ey))) * scale
        # astropy は文字のヘッダーを読むので、係数は 14〜17 桁に丸まって届く
        pairs = [(aw.sip.a, wcs.a), (aw.sip.b, wcs.b)]
        if wcs.ap is not None:
            pairs += [(aw.sip.ap, wcs.ap), (aw.sip.bp, wcs.bp)]
        out["係数(相対)"] = max(float(np.max(np.abs(x1 - x2))) / float(np.max(np.abs(x2))) for x1, x2 in pairs)
    else:
        ax, ay = aw.wcs_world2pix(ra_a, de_a, 0)
        out["天→pix(astropy)"] = float(np.max(np.hypot(px - ax, py - ay))) * scale
    # スケールと鏡像
    sc_a = math.sqrt(abs(np.linalg.det(aw.pixel_scale_matrix))) * 3600.0
    out["スケール(相対)"] = abs(sc_a - scale) / scale
    out["鏡像"] = 0.0 if (np.linalg.det(aw.pixel_scale_matrix) > 0) == (wcs.parity() > 0) else math.inf
    return out, warns


def run_astropy_cases(name, sip_order, ncases, seed):
    rng = np.random.default_rng(seed)
    worst = {}
    warns_all = []
    scales = []
    for i in range(ncases):
        wcs, w, h = random_config(rng, i, sip_order)
        if sip_order >= 2 and i % 2 == 0:
            wcs = sv_fit_sip_inverse(wcs, w, h)            # 半分は逆変換つき、半分はニュートン法だけ
        scales.append(wcs.pixel_scale_arcsec())
        out, warns = compare_with_astropy(rng, wcs, w, h)
        warns_all += warns
        for k, v in out.items():
            worst[k] = max(worst.get(k, 0.0), v)
    tol = {"スケール(相対)": 1e-12, "係数(相対)": 1e-13}
    ok = all(v < tol.get(k, 1e-6) for k, v in worst.items()) and not warns_all
    msg = "  ".join(f"{k} {v:.1e}" for k, v in worst.items())
    msg += f"  (″、{ncases} 件、{min(scales):.2f}〜{max(scales):.0f}″/px)"
    if warns_all:
        msg += f"  警告: {warns_all[0][:60]}"
    record(name, ok, msg)


def case_03_astropy_tan():
    run_astropy_cases("03 astropy と一致 TAN (回転・鏡像・0.3〜300″/px)", 0, 60, 301)


def case_04_astropy_sip2():
    run_astropy_cases("04 astropy と一致 TAN-SIP 2 次", 2, 40, 401)


def case_05_astropy_sip3():
    run_astropy_cases("05 astropy と一致 TAN-SIP 3 次", 3, 30, 501)


def case_06_header_cards():
    rng = np.random.default_rng(601)
    a, b = random_sip(rng, 2, 1000.0)
    wcs = sv_fit_sip_inverse(SvWcs(sv_radec_to_xyz(61.9, 41.97), (459.1, 281.2),
                                   random_cd(rng, 1.46, -1), a, b), 968, 548)
    long_comment = "index-5205-02 logodds 116.7 nmatch 17 " * 4
    aw, hdr, warns = astropy_wcs(wcs, 968, 548, comments=[long_comment, "日本語の注記"])
    tan = SvWcs(wcs.crval_xyz, wcs.crpix, wcs.cd)
    taw, thdr, twarns = astropy_wcs(tan, 968, 548)
    checks = {
        "CRPIX は 1 始まり": hdr["CRPIX1"] == wcs.crpix[0] + 1 and hdr["CRPIX2"] == wcs.crpix[1] + 1,
        "CTYPE (SIP)": hdr["CTYPE1"] == "RA---TAN-SIP" and hdr["CTYPE2"] == "DEC--TAN-SIP",
        "CTYPE (TAN)": thdr["CTYPE1"] == "RA---TAN" and thdr["CTYPE2"] == "DEC--TAN"
                       and "A_ORDER" not in thdr and "AP_ORDER" not in thdr,
        "SIP の次数": hdr["A_ORDER"] == 2 and hdr["B_ORDER"] == 2 and hdr["AP_ORDER"] == 3 and hdr["BP_ORDER"] == 3,
        "SIP のキー": "A_1_1" in hdr and "A_1_0" not in hdr and "AP_0_0" in hdr and "BP_3_0" in hdr,
        "CUNIT・RADESYS・LONPOLE": hdr["CUNIT1"] == "deg" and hdr["RADESYS"] == "ICRS" and hdr["LONPOLE"] == 180.0,
        "IMAGEW・IMAGEH": hdr["IMAGEW"] == 968 and hdr["IMAGEH"] == 548,
        "COMMENT(長い文を語の切れ目で分ける・ASCII 以外は ?)":
                       len(hdr["COMMENT"]) >= 4 and all(len(c) <= 70 for c in hdr["COMMENT"])
                       and " ".join(hdr["COMMENT"][:-1]) == long_comment.strip()
                       and hdr["COMMENT"][-1] == "?" * 6,
        "astropy が celestial として読む": aw.has_celestial and aw.wcs.radesys == "ICRS" and taw.has_celestial,
        "警告なし": not warns and not twarns,
    }
    # 文字にして読み直す(ファイルに書くのと同じ)。値は 15〜16 桁で丸まる
    h2 = fits.Header.fromstring(hdr.tostring())
    aw2 = WCS(h2)
    x = rng.uniform(0, 968, 200)
    y = rng.uniform(0, 548, 200)
    d = float(np.max(sep_arcsec(*aw2.all_pix2world(x, y, 0), *wcs.pix_to_radec(x, y))))
    checks["文字にして読み直しても 1e-6″ 以内"] = d < 1e-6
    bad = [k for k, v in checks.items() if not v]
    record("06 to_header_cards の中身", not bad, ("NG: " + ", ".join(bad)) if bad else f"読み直し {d:.1e}″")


def case_07_scale_parity_copy():
    cd = np.array([[-1.2e-4, 3.0e-5], [2.9e-5, 1.21e-4]])
    w = SvWcs(sv_radec_to_xyz(10.0, 20.0), (100.0, 50.0), cd)
    sc = math.sqrt(abs(np.linalg.det(cd))) * 3600.0
    c = w.copy()
    c.cd[0, 0] = 1.0
    c.crpix[0] = -5.0
    m = SvWcs(w.crval_xyz, w.crpix, cd @ np.diag([1.0, -1.0]))
    ra, dec = SvWcs(np.array([0.0, 0.0, 2.0]), (0, 0), cd).crval_radec()
    d = w.to_dict()
    checks = {
        "スケール": abs(w.pixel_scale_arcsec() - sc) < 1e-12 * sc,
        "鏡像の符号": w.parity() == -1 and m.parity() == 1,
        "copy は別物": w.cd[0, 0] == -1.2e-4 and w.crpix[0] == 100.0,
        "crval_radec": np.allclose(w.crval_radec(), (10.0, 20.0), atol=1e-12) and (ra, dec) == (0.0, 90.0),
        "to_dict を JSON に": bool(json.dumps(d)) and d["crpix"] == [100.0, 50.0],
        "SIP なし": not w.has_sip and w.a_order == 0 and w.ap_order == 0,
    }
    bad = [k for k, v in checks.items() if not v]
    record("07 スケール・鏡像・copy・crval_radec", not bad, ("NG: " + ", ".join(bad)) if bad else repr(w))


# -------------------------------------------------------------- 4 点の当てはめ ---

def quad_box_error(fit, truth, f, n=7):
    """4 点を囲む箱の格子で、当てた WCS と真の WCS が指す空のずれの最大 [px]。"""
    lo, hi = f.min(axis=0), f.max(axis=0)
    gx, gy = np.meshgrid(np.linspace(lo[0], hi[0], n), np.linspace(lo[1], hi[1], n))
    d = sv_angle_between(fit.pix_to_xyz(gx, gy), truth.pix_to_xyz(gx, gy))
    return float(np.max(d)) * ARCSEC / truth.pixel_scale_arcsec()


def pa_deg(wcs, x, y, ref):
    """画素 (x, y) での画像の +y の向き(北から東回り、度)。東・北は ref の点で測る(2 つの WCS を同じ物差しで比べる)。"""
    s0 = wcs.pix_to_xyz(x, y)
    s1 = wcs.pix_to_xyz(x, y + 1.0)
    e, n = sv_tan_basis(ref)
    d = s1 - s0
    return math.degrees(math.atan2(float(d @ e), float(d @ n)))


def angle_diff(a, b):
    return abs((a - b + 180.0) % 360.0 - 180.0)


def case_08_similarity_exact():
    """雑音なしの 4 点。真の WCS の CRPIX を 4 点の重心に置けば、接点はほぼ同じ所になり、真の値が出る。

    残るのは、接点(星の和の方向)と CRPIX の像の差による、4 点の広がり(ラジアン)の 2 乗くらいの誤差だけ。
    CD の値そのものは比べない(極のそばでは、接点がわずかに違うだけで東・北の向きが大きく変わるため)。
    """
    rng = np.random.default_rng(801)
    worst = {"CRPIX px": 0.0, "CRVAL=星の和 ″": 0.0, "箱のずれ/許容": 0.0, "スケール/許容": 0.0, "向き/許容": 0.0}
    nfail = 0
    for i in range(300):
        scale = 10 ** rng.uniform(math.log10(0.3), math.log10(300.0))
        side = rng.uniform(50, 1500) if scale < 30 else rng.uniform(20, 300)   # quad の大きさ [px]
        f = rng.uniform(0, side, (4, 2)) + rng.uniform(0, 3000, 2)
        crval = sv_radec_to_xyz(*SPECIAL_CRVAL[i]) if i < len(SPECIAL_CRVAL) else sv_normalize(rng.normal(size=3))
        parity = 1 if i % 2 == 0 else -1
        truth = SvWcs(crval, f.mean(axis=0), random_cd(rng, scale, parity))
        stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
        fit = sv_fit_similarity(f, stars)
        size = float(np.max(sv_angle_between(stars[:, None], stars[None]))) / 2   # 4 点の広がりの半分 [rad]
        tol = size ** 2
        c = f.mean(axis=0)
        e = {
            "CRPIX px": float(np.max(np.abs(fit.crpix - c))),
            "CRVAL=星の和 ″": float(sv_angle_between(fit.crval_xyz, sv_normalize(stars.sum(axis=0)))) * ARCSEC,
            "箱のずれ/許容": quad_box_error(fit, truth, f) / (tol * side),
            "スケール/許容": abs(fit.pixel_scale_arcsec() / scale - 1) / tol,
            "向き/許容": math.radians(angle_diff(pa_deg(fit, *c, truth.crval_xyz),
                                               pa_deg(truth, *c, truth.crval_xyz))) / tol,
        }
        if not (fit.parity() == parity and e["CRPIX px"] < 1e-9 and e["CRVAL=星の和 ″"] < 1e-6
                and e["箱のずれ/許容"] < 1 and e["スケール/許容"] < 1 and e["向き/許容"] < 1):
            nfail += 1
        for k, v in e.items():
            worst[k] = max(worst[k], v)
    record("08 4 点の相似変換 (雑音なし、両方の向き、0.3〜300″/px)", nfail == 0,
           f"失敗 {nfail}/300  " + "  ".join(f"{k} {v:.2g}" for k, v in worst.items())
           + "  (許容 = 4 点の広がり[rad] の 2 乗)")


def case_09_similarity_typical():
    """普段の画像(0.3〜3″/px、CRPIX は画像の中心)で、画面のどこかの 4 点から当てる。

    真の接点(画像の中心)と 4 点の接点が δ 離れ、4 点の広がりの半分が θ のとき、投影の違いで
    スケールと向きに δ² + δθ、位置に δθ × quad の大きさ くらいの差が出る(TAN は接点が違うと相似にならない)。
    許容はその数倍。
    """
    rng = np.random.default_rng(901)
    worst = {"スケール": 0.0, "向き°": 0.0, "4 点の残差 px": 0.0, "箱のずれ px": 0.0, "画像全体 px": 0.0,
             "理論との比": 0.0}
    nfail = 0
    for i in range(300):
        scale = 10 ** rng.uniform(math.log10(0.3), math.log10(3.0))
        w, h = int(rng.uniform(600, 4000)), int(rng.uniform(400, 3000))
        parity = 1 if i % 2 == 0 else -1
        crval = sv_radec_to_xyz(*SPECIAL_CRVAL[i]) if i < len(SPECIAL_CRVAL) else sv_normalize(rng.normal(size=3))
        truth = SvWcs(crval, ((w - 1) / 2, (h - 1) / 2), random_cd(rng, scale, parity))
        side = rng.uniform(0.1, 0.5) * math.hypot(w, h)
        c0 = rng.uniform([0, 0], [max(1.0, w - side), max(1.0, h - side)])
        f = c0 + rng.uniform(0, side, (4, 2))
        stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
        fit = sv_fit_similarity(f, stars)
        c = f.mean(axis=0)
        res = sv_wcs_residuals(fit, f, stars)
        e = {
            "スケール": abs(fit.pixel_scale_arcsec() / scale - 1),
            "向き°": angle_diff(pa_deg(fit, *c, fit.crval_xyz), pa_deg(truth, *c, fit.crval_xyz)),
            "4 点の残差 px": float(np.max(np.hypot(res[:, 0], res[:, 1]))),
            "箱のずれ px": quad_box_error(fit, truth, f),
            "画像全体 px": grid_error(fit, truth, w, h),
        }
        delta = float(sv_angle_between(fit.crval_xyz, truth.crval_xyz))
        theta = float(np.max(sv_angle_between(stars[:, None], stars[None]))) / 2
        lin = delta ** 2 + delta * theta                    # スケール・向き(ラジアン)の理論の大きさ
        pos = delta * theta * side                          # 位置 [px] の理論の大きさ
        e["理論との比"] = max(e["スケール"] / lin, math.radians(e["向き°"]) / lin,
                           e["4 点の残差 px"] / pos, e["箱のずれ px"] / pos)
        if not (fit.parity() == parity and e["理論との比"] < 4.0):
            nfail += 1
        for k, v in e.items():
            worst[k] = max(worst[k], v)
    record("09 4 点の相似変換 (普段の画像、quad は画面のどこか)", nfail == 0,
           f"失敗 {nfail}/300  " + "  ".join(f"{k} {v:.2g}" for k, v in worst.items())
           + "  (画像全体は外挿で、参考)")


# ---------------------------------------------------------- 最小二乗の当てはめ ---

def grid_error(fit, truth, w, h, n=21):
    """画像全体の格子で、当てた WCS と真の WCS が同じ空を指すかの最大のずれ [px]。"""
    gx, gy = np.meshgrid(np.linspace(-0.5, w - 0.5, n), np.linspace(-0.5, h - 0.5, n))
    d = sv_angle_between(fit.pix_to_xyz(gx, gy), truth.pix_to_xyz(gx, gy))
    return float(np.max(d)) * ARCSEC / truth.pixel_scale_arcsec()


def lsq_cases(sip_order, seed, ncases):
    """乱数の WCS(0.3〜300″/px、視野は 100° まで)から作った 400 点で当てる。戻り値は最大の誤差の辞書。"""
    rng = np.random.default_rng(seed)
    worst = {}

    def put(k, v):
        worst[k] = max(worst.get(k, 0.0), v)

    for i in range(ncases):
        truth, w, h = random_config(rng, i, sip_order)
        field = math.hypot(w, h) * truth.pixel_scale_arcsec() / 3600.0
        n = 400
        f = np.column_stack([rng.uniform(-0.5, w - 0.5, n), rng.uniform(-0.5, h - 0.5, n)])
        stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
        # 雑音なし・CRPIX = 真の CRPIX。接点を十分に動かせば(30 回)、視野が広くても同じ WCS に戻る
        fit = sv_fit_wcs_lsq(f, stars, crpix=truth.crpix, sip_order=sip_order, iterations=30, width=w, height=h)
        put("雑音なし 格子 px", grid_error(fit, truth, w, h))
        put("雑音なし CRVAL″", float(sv_angle_between(fit.crval_xyz, truth.crval_xyz)) * ARCSEC)
        put("雑音なし スケール相対", abs(fit.pixel_scale_arcsec() / truth.pixel_scale_arcsec() - 1))
        if abs(truth.crval_radec()[1]) < 89.0:          # 極のそばでは東・北の向きが定まらないので CD は比べない
            put("雑音なし CD 相対", float(np.max(np.abs(fit.cd - truth.cd))) / np.max(np.abs(truth.cd)))
        put("鏡像", 0.0 if fit.parity() == truth.parity() else math.inf)
        if sip_order >= 2:
            hd = 0.5 * math.hypot(w, h)
            p, q = np.indices(truth.a.shape)
            sc = hd ** (p + q)                          # 係数 × 画像の半対角^次数 = 隅での px
            put("雑音なし A,B px", float(max(np.max(np.abs((fit.a - truth.a) * sc)),
                                           np.max(np.abs((fit.b - truth.b) * sc)))))
        # 既定の反復(接点を 3 回動かす)でも、視野 10° までなら同じ WCS に戻る
        if field <= 10.0:
            fit = sv_fit_wcs_lsq(f, stars, crpix=truth.crpix, sip_order=sip_order, width=w, height=h)
            put("既定の反復 格子 px(視野10°まで)", grid_error(fit, truth, w, h))
        # 雑音あり(σ = 0.3 px)
        sig = 0.3
        fn = f + rng.normal(0, sig, f.shape)
        fit = sv_fit_wcs_lsq(fn, stars, crpix=truth.crpix, sip_order=sip_order,
                             iterations=3 if field <= 10.0 else 30, width=w, height=h)
        put("雑音 格子/σ", grid_error(fit, truth, w, h) / sig)
        put("雑音 スケール相対", abs(fit.pixel_scale_arcsec() / truth.pixel_scale_arcsec() - 1))
        put("鏡像", 0.0 if fit.parity() == truth.parity() else math.inf)
    return worst


def lsq_ok(worst, noisy_tol):
    return (worst["雑音なし 格子 px"] < 1e-6 and worst["雑音なし CRVAL″"] < 1e-6
            and worst["雑音なし スケール相対"] < 1e-9 and worst["雑音なし CD 相対"] < 1e-9
            and worst.get("雑音なし A,B px", 0.0) < 1e-6 and worst["既定の反復 格子 px(視野10°まで)"] < 1e-6
            and worst["雑音 格子/σ"] < noisy_tol and worst["雑音 スケール相対"] < 1e-3 and worst["鏡像"] == 0.0)


def case_10_lsq_tan():
    # 雑音の許容: 画像の隅での予測の標準誤差(400 点・3 項でおよそ 0.25σ)の 3 倍
    worst = lsq_cases(0, 1001, 40)
    record("10 最小二乗 TAN (雑音なしは元どおり、σ=0.3px・400 点)", lsq_ok(worst, 0.8),
           "  ".join(f"{k} {v:.1e}" for k, v in worst.items()))


def case_11_lsq_sip2():
    # 雑音の許容: 画像の隅での予測の標準誤差(400 点・6 項でおよそ 0.4σ)の 3 倍
    worst = lsq_cases(2, 1101, 40)
    record("11 最小二乗 SIP 2 次 (雑音なしは元どおり、σ=0.3px・400 点)", lsq_ok(worst, 1.2),
           "  ".join(f"{k} {v:.1e}" for k, v in worst.items()))


def case_12_lsq_default_crpix():
    """CRPIX を渡さない(= 点の重心)。普段の画像(0.3〜3″/px、真の接点は画像の中心)。

    接点が真の WCS と δ ずれると、TAN では画像の隅で δθ²(θ = 半対角)の差が出る(SIP は大半を吸収する)。
    許容は雑音の分(10・11 と同じ)に、その 3 倍を足したもの。
    """
    rng = np.random.default_rng(1201)
    worst_tan = worst_sip = worst_inv = worst_ratio = 0.0
    for i in range(30):
        scale = 10 ** rng.uniform(math.log10(0.3), math.log10(3.0))
        for order in (0, 2):
            truth, w, h = random_config(rng, i, order, scale=scale)
            truth = SvWcs(truth.crval_xyz, ((w - 1) / 2, (h - 1) / 2), truth.cd, truth.a, truth.b)
            n = 300
            f = np.column_stack([rng.uniform(-0.5, w - 0.5, n), rng.uniform(-0.5, h - 0.5, n)])
            stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
            fn = f + rng.normal(0, 0.3, f.shape)
            fit = sv_fit_wcs_lsq(fn, stars, sip_order=order)          # width, height も渡さない
            e = grid_error(fit, truth, w, h)
            theta = 0.5 * math.hypot(w, h) * scale / ARCSEC
            delta = float(sv_angle_between(fit.crval_xyz, truth.crval_xyz))      # 接点のずれ
            proj = delta * theta ** 2 * ARCSEC / scale
            worst_ratio = max(worst_ratio, e / ((0.8 if order == 0 else 1.2) * 0.3 + 3 * proj))
            if order == 0:
                worst_tan = max(worst_tan, e)
            else:
                worst_sip = max(worst_sip, e)
                # 逆変換 AP, BP は点の範囲で当ててある。その中でニュートン法の答えと比べる
                gx, gy = np.meshgrid(np.linspace(f[:, 0].min(), f[:, 0].max(), 15),
                                     np.linspace(f[:, 1].min(), f[:, 1].max(), 15))
                s = fit.pix_to_xyz(gx, gy)
                x1, y1, _ = fit.xyz_to_pix(s)
                worst_inv = max(worst_inv, float(np.max(np.hypot(x1 - gx, y1 - gy))))
    ok = worst_ratio < 1.0 and worst_inv < 0.01
    record("12 最小二乗 CRPIX = 重心 (σ=0.3px・300 点、範囲は点から)", ok,
           f"TAN の格子 {worst_tan:.3f} px  SIP 2 次の格子 {worst_sip:.3f} px  許容との比 {worst_ratio:.2f}  "
           f"AP/BP の戻り {worst_inv:.1e} px")


def case_13_lsq_weights():
    """重み 0 の外れ値は無視される。重みを付けないと外れ値に引っ張られる。"""
    rng = np.random.default_rng(1301)
    truth, w, h = random_config(rng, 20, 2, scale=1.46)
    n = 300
    f = np.column_stack([rng.uniform(0, w, n), rng.uniform(0, h, n)])
    stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
    bad = rng.random(n) < 0.15
    f2 = f.copy()
    f2[bad] += rng.uniform(5, 50, (int(bad.sum()), 2))
    wt = np.where(bad, 0.0, rng.uniform(0.5, 2.0, n))
    fit_w = sv_fit_wcs_lsq(f2, stars, weights=wt, sip_order=2, crpix=truth.crpix, width=w, height=h)
    fit_u = sv_fit_wcs_lsq(f2, stars, sip_order=2, crpix=truth.crpix, width=w, height=h)
    ew = grid_error(fit_w, truth, w, h)
    eu = grid_error(fit_u, truth, w, h)
    # 重み 0 の点を消して当てたものと同じになる
    fit_d = sv_fit_wcs_lsq(f2[~bad], stars[~bad], weights=wt[~bad], sip_order=2, crpix=truth.crpix,
                           width=w, height=h)
    ed = grid_error(fit_w, fit_d, w, h)
    record("13 最小二乗の重み (外れ値 15% を重み 0)", ew < 1e-6 and eu > 1.0 and ed < 1e-9,
           f"重みあり {ew:.1e} px  重みなし {eu:.2f} px  重み 0 の点を消したものとの差 {ed:.1e} px")


def case_14_sip_roundtrip():
    """SIP ありの xyz_to_pix と pix_to_xyz の往復。逆変換あり・なし、画像の外側 10% まで。"""
    rng = np.random.default_rng(1401)
    worst = {"ニュートン px": 0.0, "AP/BP px": 0.0, "天→pix→天 ″": 0.0}
    nbad = 0
    for i in range(40):
        order = 2 if i % 2 == 0 else 3
        wcs, w, h = random_config(rng, i, order)
        winv = sv_fit_sip_inverse(wcs, w, h)
        x = rng.uniform(-0.1 * w, 1.1 * w, 500)
        y = rng.uniform(-0.1 * h, 1.1 * h, 500)
        s = wcs.pix_to_xyz(x, y)
        x1, y1, ok1 = wcs.xyz_to_pix(s)                       # 逆変換なし → ニュートン法
        x2, y2, ok2 = winv.xyz_to_pix(s, exact=True)
        x3, y3, ok3 = winv.xyz_to_pix(s)                      # AP, BP
        nbad += int((~ok1).sum() + (~ok2).sum() + (~ok3).sum())
        inside = (x >= -0.5) & (x <= w - 0.5) & (y >= -0.5) & (y <= h - 0.5)
        worst["ニュートン px"] = max(worst["ニュートン px"], float(np.max(np.hypot(x1 - x, y1 - y))),
                                   float(np.max(np.hypot(x2 - x, y2 - y))))
        worst["AP/BP px"] = max(worst["AP/BP px"], float(np.max(np.hypot(x3 - x, y3 - y)[inside])))
        # 天 → 画素 → 天
        stars = wcs.pix_to_xyz(rng.uniform(0, w, 300), rng.uniform(0, h, 300))
        px, py, _ = wcs.xyz_to_pix(stars)
        back = wcs.pix_to_xyz(px, py)
        worst["天→pix→天 ″"] = max(worst["天→pix→天 ″"], float(np.max(sv_angle_between(back, stars))) * ARCSEC)
    # 接点の反対側は ok = False
    wcs, w, h = random_config(rng, 0, 2)
    xb, yb, okb = wcs.xyz_to_pix(np.array([-wcs.crval_xyz]))
    behind = (not okb[0]) and np.isnan(xb[0]) and np.isnan(yb[0])
    ok = (worst["ニュートン px"] < 1e-8 and worst["AP/BP px"] < 0.01 and worst["天→pix→天 ″"] < 1e-6
          and nbad == 0 and behind)
    record("14 SIP の往復 (ニュートン法・AP/BP、2・3 次)", ok,
           "  ".join(f"{k} {v:.1e}" for k, v in worst.items()) + f"  解けなかった点 {nbad}  反対側 ok={bool(okb[0])}")


def case_15_residuals():
    rng = np.random.default_rng(1501)
    truth, w, h = random_config(rng, 30, 2, scale=2.0)
    truth = sv_fit_sip_inverse(truth, w, h)
    f = np.column_stack([rng.uniform(0, w, 50), rng.uniform(0, h, 50)])
    stars = truth.pix_to_xyz(f[:, 0], f[:, 1])
    r0 = sv_wcs_residuals(truth, f, stars, exact=True)
    r1 = sv_wcs_residuals(truth, f + [0.5, -0.25], stars, exact=True)
    stars2 = np.vstack([stars, -truth.crval_xyz])
    r2 = sv_wcs_residuals(truth, np.vstack([f, [0.0, 0.0]]), stars2)
    ok = (r0.shape == (50, 2) and np.max(np.abs(r0)) < 1e-8
          and np.allclose(r1, [0.5, -0.25], atol=1e-8) and np.all(np.isnan(r2[-1])) and np.all(np.isfinite(r2[:-1])))
    record("15 sv_wcs_residuals (画像の点 − 投影した星)", ok,
           f"雑音なし {np.max(np.abs(r0)):.1e} px  ずらした分 {r1.mean(axis=0).round(9).tolist()}  反対側の星は nan")


def case_16_errors():
    """縮退した入力は ValueError。"""
    rng = np.random.default_rng(1601)
    wcs, w, h = random_config(rng, 12, 0, scale=1.0)
    f = rng.uniform(0, 500, (10, 2))
    s = wcs.pix_to_xyz(f[:, 0], f[:, 1])
    line = np.column_stack([np.linspace(0, 500, 10), np.linspace(0, 300, 10)])
    sl = wcs.pix_to_xyz(line[:, 0], line[:, 1])
    tries = {
        "相似 1 点": lambda: sv_fit_similarity(f[:1], s[:1]),
        "相似 同じ位置": lambda: sv_fit_similarity(np.zeros((4, 2)), s[:4]),
        "相似 数が違う": lambda: sv_fit_similarity(f[:4], s[:3]),
        "最小二乗 TAN 2 点": lambda: sv_fit_wcs_lsq(f[:2], s[:2]),
        "最小二乗 SIP2 5 点": lambda: sv_fit_wcs_lsq(f[:5], s[:5], sip_order=2),
        "最小二乗 一直線": lambda: sv_fit_wcs_lsq(line, sl),
        "最小二乗 重みが全部 0": lambda: sv_fit_wcs_lsq(f, s, weights=np.zeros(10)),
        "逆変換 範囲なし": lambda: sv_fit_sip_inverse(SvWcs(wcs.crval_xyz, wcs.crpix, wcs.cd, *random_sip(rng, 2, 500.0))),
    }
    bad = []
    for k, fn in tries.items():
        try:
            fn()
            bad.append(k + "(通った)")
        except ValueError:
            pass
        except Exception as e:                                    # noqa: BLE001
            bad.append(f"{k}({type(e).__name__})")
    # 最小限の点数ならちょうど解ける
    try:
        sv_fit_wcs_lsq(f[:3], s[:3])
        sv_fit_wcs_lsq(f[:6], s[:6], sip_order=2, width=500, height=500)
    except Exception as e:                                        # noqa: BLE001
        bad.append(f"最少の点数で失敗({e})")
    record("16 縮退した入力は ValueError", not bad, ("NG: " + ", ".join(bad)) if bad else f"{len(tries)} 通り")


def main():
    for fn in sorted(k for k in globals() if k.startswith("case_")):
        try:
            globals()[fn]()
        except Exception as e:                                    # noqa: BLE001
            RESULTS.append((fn, False, f"例外 {type(e).__name__}: {e}"))
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

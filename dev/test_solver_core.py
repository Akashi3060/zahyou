"""solver_core.py(自前ソルバーの解法)の試験。docs/design-native-solver.md の 3 節と 8 節。

部品: 星の並べ替え、newpoint の quad 作り(= 総当たり)、画像側のコード = 索引のコード(索引の星を既知の WCS で
画像へ投影して quad を作り、正準化したコードの code_ball で元の quad が引けること。両方の向き・三角形の索引も)、
4 点の当てはめ、小さい円の検索、検証の対数オッズ(真の WCS は ≥ ln 1e9、乱数の WCS は低い)、合成星野を解く、
締め切り・中止・cancel() の間隔、trace の中身。
実写の回帰: dev/solver_truth.json の正解(WSL の astrometry.net)と比べる。解けるべき画像はブラインド・ヒントあり
(正解の中心から 0.5° ずらした点、半径 5°)・スケール既知(±25%)で、中心 2″(0.5 px のほうが大きければそれ)、
スケール 0.2%、回転 0.1°、鏡像の一致。解けてはいけない画像は締め切り(60 秒)まで探して解なし、超過 1 秒以内。

  python dev/test_solver_core.py            全部(解けてはいけない画像はブラインドだけ)
  python dev/test_solver_core.py --unit     部品だけ
  python dev/test_solver_core.py --real     実写の回帰だけ
  --full(解けてはいけない画像もヒントあり・スケール既知で)  --deadline 60  --case 名前の一部,…
  --json 結果.json  --detect main|dev(星の検出に使う zahyou_core。既定は git の main の版を写して使う。
  dev の zahyou_core は書き換え中のことがあるため。正解もその検出の星で作られている)

C:\\AstrometryData(環境変数 ZAHYOU_INDEX_DIR)が無ければ索引を使う項目は SKIP、画像が無ければ SKIP。
"""
import argparse
import copy
import importlib.util
import json
import math
import os
import subprocess
import sys
import tempfile
import time
import traceback
import warnings

warnings.filterwarnings("ignore")
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import solver_core as sc       # noqa: E402

DATA = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
TRUTH = os.path.join(HERE, "solver_truth.json")
LN1E9 = math.log(1e9)
RESULTS = []                   # (名前, True / False / None(SKIP), 説明, 記録)
ARGS = None
_CAT = []

POSITIVE = ("real_orig", "phaethon_00001", "imx432_16bit", "siril_rgb_stack", "c2025a6_00001",
            "starchart_phaethon", "3I_00010")
NEGATIVE = ("capture_005514", "cap_20260201", "tcase_basic16", "tcase_dense")
# 解けてはいけない画像の「ヒントあり」「スケール既知」(--full)に使う値(同じ機材の画像の向き・スケール)
NEG_HINT = {"capture_005514": (61.89, 41.96, 1.4616), "cap_20260201": (61.89, 41.96, 1.4616),
            "tcase_basic16": (10.0, 20.0, 1.5), "tcase_dense": (10.0, 20.0, 1.5)}


def record(name, ok, msg, rec=None):
    ok = None if ok is None else bool(ok)          # numpy の bool も JSON に書けるように
    RESULTS.append((name, ok, msg, rec or {}))
    tag = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
    print(f"  {tag}  {name}  {msg}", flush=True)


def catalog():
    """テスト全体で使い回す SvIndexCatalog(索引が無ければ None)。"""
    if not _CAT:
        cat = sc.SvIndexCatalog(DATA) if os.path.isdir(DATA) else None
        _CAT.append(cat if cat is not None and cat.files else None)
    return _CAT[0]


def have(*ids):
    cat = catalog()
    return cat is not None and all(i in cat.series for i in ids)


def run_case(name, fn, *ids):
    if ids and not have(*ids):
        record(name, None, f"索引が無い({DATA} の {', '.join(map(str, ids))})")
        return
    t0 = time.perf_counter()
    try:
        ok, msg = fn()
    except Exception as e:
        ok, msg = False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}"
    record(name, ok, f"{msg}  [{time.perf_counter() - t0:.1f} s]")


# ------------------------------------------------------------------ 物差し ---

def cd_of(scale_arcsec, rot_deg, mirror):
    """CD(度/px)。mirror = False は配列座標で det CD < 0(FITS の向きの普通の空)、True は det CD > 0。"""
    t = math.radians(rot_deg)
    R = np.array([[math.cos(t), -math.sin(t)], [math.sin(t), math.cos(t)]])
    M = np.eye(2) if mirror else np.diag([-1.0, 1.0])
    return scale_arcsec / 3600.0 * (R @ M)


def measure(wcs, W, H):
    """中心((W−1)/2, (H−1)/2)の位置・中心での画素スケール(SIP 込み)・配列の +y の位置角・det の符号。"""
    cx, cy = (W - 1) / 2.0, (H - 1) / 2.0
    c = wcs.pix_to_xyz(cx, cy).reshape(3)
    e, n = sc.sv_tan_basis(c)
    dx = (wcs.pix_to_xyz(cx + 0.5, cy) - wcs.pix_to_xyz(cx - 0.5, cy)).reshape(3)
    dy = (wcs.pix_to_xyz(cx, cy + 0.5) - wcs.pix_to_xyz(cx, cy - 0.5)).reshape(3)
    J = np.array([[dx @ e, dy @ e], [dx @ n, dy @ n]])
    det = float(np.linalg.det(J))
    return dict(xyz=c, scale=math.sqrt(abs(det)) * sc.SV_ARCSEC_PER_RAD,
                pa=math.degrees(math.atan2(dy @ e, dy @ n)) % 360.0, parity=1 if det > 0 else -1)


def compare(got, want, crit):
    """got・want は measure の形(want の xyz は単位ベクトル)。戻り値 (合格か, 差の dict)。"""
    dcen = float(sc.sv_angle_between(got["xyz"], want["xyz"])) * sc.SV_ARCSEC_PER_RAD
    ds = got["scale"] / want["scale"] - 1.0
    dr = (got["pa"] - want["pa"] + 180.0) % 360.0 - 180.0
    lim = max(crit["center_arcsec"], crit["center_px"] * want["scale"])
    ok = (dcen <= lim and abs(ds) <= crit["scale_rel"] and abs(dr) <= crit["rot_deg"]
          and got["parity"] == want["parity"])
    return ok, dict(center_arcsec=dcen, scale_pct=100.0 * ds, rot_deg=dr,
                    parity_same=got["parity"] == want["parity"], center_lim=lim)


CRIT = dict(center_arcsec=2.0, center_px=0.5, scale_rel=0.002, rot_deg=0.1)


def synth(cat, indexid, ra, dec, scale, W, H, rot, mirror, n=60, noise=0.25, drop=0.0, fake=0, seed=0):
    """索引の星から正解つきの星野を作る(sweep の小さい順 = 明るい順に n 個、位置に雑音、欠けと偽の星)。

    戻り値 (xy, flux, 正解の SvWcs)。flux は明るい順の順位から作る(偽の星はばらばらの順位に混ぜる)。
    """
    r = np.random.default_rng(seed)
    crval = sc.sv_radec_to_xyz(ra, dec)
    wcs = sc.SvWcs(crval, [(W - 1) / 2.0, (H - 1) / 2.0], cd_of(scale, rot, mirror))
    rad = 0.5 * math.hypot(W, H) * scale / sc.SV_ARCSEC_PER_RAD * 1.1
    xyz, sw = [], []
    for f in cat.tiles_near(indexid, crval, rad):
        ids = f.stars_in_cone(crval, rad)
        xyz.append(f.star_xyz(ids))
        sw.append(f.sweep(ids))
    xyz = np.concatenate(xyz)
    sw = np.concatenate(sw)
    x, y, ok = wcs.xyz_to_pix(xyz)
    ok &= (x >= 0) & (x <= W - 1) & (y >= 0) & (y <= H - 1)
    o = np.argsort(sw[ok], kind="stable")
    pts = np.column_stack([x[ok], y[ok]])[o][:n]
    if drop > 0:
        pts = pts[r.random(len(pts)) >= drop]
    pts = pts + r.normal(0.0, noise, pts.shape)
    rank = np.arange(len(pts), dtype=np.float64)
    if fake:
        fk = np.column_stack([r.uniform(0, W - 1, fake), r.uniform(0, H - 1, fake)])
        pts = np.concatenate([pts, fk])
        rank = np.concatenate([rank, r.uniform(0, len(rank), fake)])
    flux = 1000.0 / (1.0 + rank)
    return pts, flux, wcs


# ------------------------------------------------------------------ 部品 ---

def case_01_order():
    """sv_order_stars = 素直な書き方(明るい順 → 箱ごとのリストを 1 番目から順に)。箱の数は C の round(半分は上へ)。"""
    r = np.random.default_rng(1)
    bad = 0
    for trial in range(200):
        W, H = [(968, 548), (536, 368), (2704, 1536), (400, 1200), (900, 600)][trial % 5]
        n = int(r.integers(1, 300))
        xy = np.column_stack([r.uniform(-0.5, W - 0.5, n), r.uniform(-0.5, H - 0.5, n)])
        flux = r.integers(0, 50, n).astype(float) if trial % 3 else None   # 同じ値を混ぜる
        nb = int(r.integers(1, 20))
        got = sc.sv_order_stars(xy, flux, W, H, nb)
        bright = np.arange(n) if flux is None else np.argsort(-flux, kind="stable")
        NX = max(1, int(math.floor(math.sqrt(nb * W / H) + 0.5)))
        NY = max(1, int(math.floor(nb / NX + 0.5)))
        boxes = {}
        for i in bright:
            bx = min(NX - 1, max(0, int((xy[i, 0] + 0.5) / W * NX)))
            by = min(NY - 1, max(0, int((xy[i, 1] + 0.5) / H * NY)))
            boxes.setdefault(by * NX + bx, []).append(i)
        want = []
        k = 0
        while len(want) < n:
            for i in bright:          # 「元の順」= 明るい順で、各箱の k 番目
                bx = min(NX - 1, max(0, int((xy[i, 0] + 0.5) / W * NX)))
                by = min(NY - 1, max(0, int((xy[i, 1] + 0.5) / H * NY)))
                lst = boxes[by * NX + bx]
                if len(lst) > k and lst[k] == i:
                    want.append(i)
            k += 1
        bad += not np.array_equal(got, np.array(want, dtype=np.int64))
    # 箱の数の例: 968×548・10 箱 → NX = round(4.20) = 4、NY = round(2.5) = 3(C の round)
    xy1 = np.array([[10.0, 10.0], [20.0, 10.0], [900.0, 500.0]])
    o1 = sc.sv_order_stars(xy1, np.array([3.0, 2.0, 1.0]), 968, 548, 10)
    o1 = [int(v) for v in o1]
    ok = bad == 0 and o1 == [0, 2, 1]
    return ok, f"乱数 200 通りで一致しないもの {bad}、隅に固まった明るい 2 個と暗い 1 個 → {o1}(期待 [0, 2, 1])"


def case_02_newpoint():
    """newpoint の quad = 総当たり(AB の長さが範囲内、内側の星が AB を直径とする円の中。重複なし・漏れなし)。"""
    r = np.random.default_rng(2)
    msgs = []
    ok = True
    for dq in (4, 3):
        tot = 0
        for trial in range(6):
            N = int(r.integers(8, 26))
            P = np.column_stack([r.uniform(0, 900, N), r.uniform(0, 600, N)])
            lo, hi = 60.0, float(r.uniform(300, 1100))
            L2, inside = sc.sv_quad_geometry(P, 0.01)
            got = []
            for n in range(N):
                q = sc.sv_quads_newest(n, dq, L2, inside, lo * lo, hi * hi)
                assert np.all(q.max(axis=1) == n) if len(q) else True
                got.extend(tuple(int(v) for v in row[:2]) + tuple(sorted(int(v) for v in row[2:])) for row in q)
            want = []
            for a in range(N):
                for b in range(a + 1, N):
                    if not (lo * lo <= L2[a, b] <= hi * hi):
                        continue
                    ins = [k for k in range(N) if k not in (a, b) and inside[a, b, k]]
                    if dq == 3:
                        want.extend((a, b, c) for c in ins)
                    else:
                        want.extend((a, b, c, d) for i, c in enumerate(ins) for d in ins[i + 1:])
            same = len(got) == len(set(got)) and set(got) == set(want)
            ok &= same
            tot += len(got)
            if not same:
                msgs.append(f"{dq} 星 試行 {trial}: newpoint {len(got)}(重複 {len(got) - len(set(got))})"
                            f" ≠ 総当たり {len(want)}")
        msgs.append(f"{dq} 星 6 通りで計 {tot} 個が総当たりと一致")
    return ok, "、".join(msgs)


def _quads_of(f, r, k):
    """索引 f の quad を k 個(乱数)。"""
    ids = r.integers(0, f.nquads, k)
    return ids, f.quad_stars(ids), f.star_xyz(f.quad_stars(ids).reshape(-1)).reshape(k, f.dimquads, 3)


def case_03_codes():
    """画像側のコード = 索引のコード。索引の quad の星を既知の WCS で画像へ投影し、A, B と C, D の並びを乱数で
    入れ替えた画像の quad から sv_field_codes → sv_code_variants → code_ball で、元の quad が正しい星の対応で
    引けること。WCS の接点は quad の中心から |AB| の 0〜0.5 倍ずらす。鏡像の印は det CD > 0 と一致すること。"""
    cat = catalog()
    r = np.random.default_rng(3)
    files = [cat.series[5206][int(r.integers(0, len(cat.series[5206])))], cat.series[5202][7],
             cat.series[4107][0], cat.series[4111][0], cat.series[4113][0]]
    msgs = []
    ok = True
    for noise in (0.0, 0.3):
        for f in files:
            K = 150
            qids, qst, S = _quads_of(f, r, K)
            dq = f.dimquads
            pts, quads, truth_cols, mir = [], [], [], []
            for j in range(K):
                ab = float(r.uniform(150, 600))                       # |AB| [px]
                th = float(sc.sv_angle_between(S[j, 0], S[j, 1]))
                scale = th * sc.SV_ARCSEC_PER_RAD / ab
                mid = sc.sv_normalize(S[j, 0] + S[j, 1])
                e, nn = sc.sv_tan_basis(mid)
                off = float(r.uniform(0, 0.5)) * th
                phi = float(r.uniform(0, 2 * math.pi))
                cen = sc.sv_normalize(mid + off * (math.cos(phi) * e + math.sin(phi) * nn))
                m = bool(r.random() < 0.5)
                w = sc.SvWcs(cen, [500.0, 400.0], cd_of(scale, float(r.uniform(0, 360)), m))
                x, y, okp = w.xyz_to_pix(S[j])
                assert okp.all()
                P = np.column_stack([x, y]) + r.normal(0, noise, (dq, 2))
                # 画像の quad の列: {A, B} と {C, D} のそれぞれを乱数で入れ替える(newpoint は番号順に並べるため)
                cols = [0, 1] if r.random() < 0.5 else [1, 0]
                inner = list(range(2, dq))
                if dq == 4 and r.random() < 0.5:
                    inner = inner[::-1]
                cols += inner
                base = len(pts) * 0 + j * dq
                quads.append([base + c for c in cols])
                truth_cols.append(cols)
                pts.append(P)
                mir.append(m)
            P = np.concatenate(pts)
            quads = np.array(quads)
            codes = sc.sv_field_codes(P, quads)
            q, row, perm, mirror = sc.sv_code_variants(codes, dq, 0.01, "both")
            qi, qid = f.code_ball(q, 0.01 * math.sqrt(f.code_dim / 4.0))
            found = np.zeros(K, dtype=bool)
            other = np.zeros(K, dtype=bool)
            for a, b in zip(qi, qid):
                j = row[a]
                if b != qids[j]:
                    continue
                # perm[a] = 索引の A, B, C, D に対応する画像の quad の列 → 画像の星の本当の番号(列 cols の逆)
                local = np.array(truth_cols[j])[perm[a]]
                if np.array_equal(local, np.arange(dq)):
                    if bool(mirror[a]) == mir[j]:
                        found[j] = True
                    else:
                        other[j] = True     # 内側の星が AB の線の近くにあると、鏡像の形でも同じ対応で当たる
            # 雑音なしは全部。雑音 0.3 px(|AB| 150〜600 px)はコードのずれが codetol に近づくので 95 % 以上
            this = found.all() if noise == 0.0 else found.mean() >= 0.95
            ok &= this
            n_other = int((other & ~found).sum())
            n_both = int((other & found).sum())
            msgs.append(f"{f.name}{'(三角形)' if dq == 3 else ''} 雑音 {noise} px: {found.sum()}/{K}"
                        + (f"(逆の向きでも当たる {n_both})" if n_both else "")
                        + (f"、逆の向きでだけ当たる {n_other}" if n_other else ""))
    return ok, "; ".join(msgs)


def case_04_similarity():
    """sv_fit_similarity_many(仮説の束でまとめて)= solver_wcs.sv_fit_similarity(1 組ずつ)。"""
    r = np.random.default_rng(4)
    H = 300
    F = np.zeros((H, 4, 2))
    S = np.zeros((H, 4, 3))
    for h in range(H):
        c = sc.sv_normalize(r.normal(size=3))
        sc_ = float(10 ** r.uniform(-0.5, 1.5))
        w = sc.SvWcs(c, [r.uniform(0, 2000), r.uniform(0, 1500)], cd_of(sc_, r.uniform(0, 360), r.random() < 0.5))
        F[h] = np.column_stack([r.uniform(0, 2000, 4), r.uniform(0, 1500, 4)])
        S[h] = w.pix_to_xyz(F[h, :, 0], F[h, :, 1])
        F[h] += r.normal(0, 0.5, (4, 2))
    got = sc.sv_fit_similarity_many(F, S)
    dmax = dcv = dcp = 0.0
    for h in range(H):
        w = sc.sv_fit_similarity(F[h], S[h])
        dmax = max(dmax, float(np.abs(got["cd"][h] - w.cd).max() / np.abs(w.cd).max()))
        dcv = max(dcv, float(sc.sv_angle_between(got["crval"][h], w.crval_xyz)) * sc.SV_ARCSEC_PER_RAD)
        dcp = max(dcp, float(np.abs(got["crpix"][h] - w.crpix).max()))
    ok = bool(got["ok"].all()) and dmax < 1e-9 and dcv < 1e-6 and dcp < 1e-9
    return ok, f"{H} 組: CD の相対差 {dmax:.1e}、CRVAL {dcv:.1e}″、CRPIX {dcp:.1e} px"


def case_05_small_cone():
    """小さい円の検索(_sv_kd_small_ball)= 箱を持つ検索(ball_candidates)= stars_in_cone(総当たりの代わり)。"""
    cat = catalog()
    r = np.random.default_rng(5)
    msgs = []
    ok = True
    for iid in (5205, 5202, 4107, 4111):
        fs = cat.series[iid]
        f = fs[int(r.integers(0, len(fs)))]
        st = f.star_xyz(r.integers(0, f.nstars, 400))
        e = sc.sv_normalize(np.cross(st, r.normal(size=(400, 3))))
        cen = sc.sv_normalize(st + e * r.uniform(0, 20, (400, 1)) / sc.SV_ARCSEC_PER_RAD)
        rad = r.uniform(0.5, 60.0, 400) / sc.SV_ARCSEC_PER_RAD
        a = sc._sv_series_cone(cat, iid, cen, rad, small=True)
        b = sc._sv_series_cone(cat, iid, cen, rad, small=False)
        sa = set(zip(a["h"].tolist(), a["fid"].tolist(), a["sid"].tolist()))
        sb = set(zip(b["h"].tolist(), b["fid"].tolist(), b["sid"].tolist()))
        sw = set()
        for h in range(400):
            for k, g in enumerate(fs):
                if g in cat.tiles_near(iid, cen[h], rad[h]):
                    sw.update((h, k, int(s)) for s in g.stars_in_cone(cen[h], rad[h]))
        same = sa == sb == sw and len(a["h"]) == len(sa)
        ok &= same
        msgs.append(f"{iid}: {len(sa)} 組" + ("" if same else f" ≠ 箱 {len(sb)} / stars_in_cone {len(sw)}"))
    return ok, "400 個の円(半径 0.5〜60″)で " + "、".join(msgs)


def case_06_verify():
    """検証の対数オッズ: 真の WCS は ≥ ln 1e9、乱数の WCS・回したもの・鏡像は tune(ln 1e6)未満。"""
    cat = catalog()
    r = np.random.default_rng(6)
    msgs = []
    ok = True
    for iid, (W, H, s, mirror) in ((5205, (968, 548, 1.4616, False)), (4108, (1608, 1104, 2.3188, True)),
                                   (5203, (2704, 1536, 0.7473, False))):
        ra, dec = float(r.uniform(0, 360)), float(np.degrees(np.arcsin(r.uniform(-0.9, 0.9))))
        xy, fl, w = synth(cat, iid, ra, dec, s, W, H, float(r.uniform(0, 360)), mirror, n=80, noise=0.3,
                          drop=0.2, fake=20, seed=int(r.integers(1 << 30)))
        t = sc.sv_verify_wcs(xy, fl, W, H, cat, iid, w)
        bad = []
        for k in range(20):
            sep = math.radians(float(r.uniform(0.2, 5.0)))
            d = sc.sv_normalize(np.cross(w.crval_xyz, r.normal(size=3)))
            c2 = sc.sv_normalize(math.cos(sep) * w.crval_xyz + math.sin(sep) * d)
            w2 = sc.SvWcs(c2, w.crpix, cd_of(s, float(r.uniform(0, 360)), bool(r.random() < 0.5)))
            bad.append(sc.sv_verify_wcs(xy, fl, W, H, cat, iid, w2)["logodds"])
        rot = sc.SvWcs(w.crval_xyz, w.crpix, cd_of(s, 10.0, False) @ np.linalg.inv(cd_of(s, 0.0, False)) @ w.cd)
        mir = sc.SvWcs(w.crval_xyz, w.crpix, w.cd @ np.array([[0.0, 1.0], [1.0, 0.0]]))
        lo_rot = sc.sv_verify_wcs(xy, fl, W, H, cat, iid, rot)["logodds"]
        lo_mir = sc.sv_verify_wcs(xy, fl, W, H, cat, iid, mir)["logodds"]
        worst = max(bad + [lo_rot, lo_mir])
        this = t["logodds"] >= LN1E9 and worst < math.log(1e6)
        ok &= this
        msgs.append(f"{iid} {W}×{H}: 真 {t['logodds']:.1f}(一致 {t['nmatch']}/{t['nfield']}、参照星 {t['nref']})、"
                    f"乱数 20 個の最大 {max(bad):.1f}、10° 回転 {lo_rot:.1f}、鏡像 {lo_mir:.1f}")
    return ok, "; ".join(msgs)


def _solve_synth(cat, iid, W, H, s, mirror, seed, **kw):
    r = np.random.default_rng(seed)
    ra, dec = float(r.uniform(0, 360)), float(np.degrees(np.arcsin(r.uniform(-0.9, 0.9))))
    xy, fl, w = synth(cat, iid, ra, dec, s, W, H, float(r.uniform(0, 360)), mirror, n=70, noise=0.3,
                      drop=0.15, fake=15, seed=seed)
    if kw.pop("hint", False):
        kw["hint"] = (ra + 1.0 / max(0.1, math.cos(math.radians(dec))), dec - 0.5, 5.0)
    t0 = time.monotonic()
    sol = sc.sv_solve(xy, fl, W, H, cat, deadline=t0 + 120, **kw)
    return xy, fl, w, sol, time.monotonic() - t0


def case_07_synth_solve():
    """合成星野を解く: 4 星(普通・鏡像、ブラインド・ヒント)と三角形の索引だけの広い視野。解の対数オッズは
    sv_verify_wcs に解の quad を渡した値と一致すること。"""
    cat = catalog()
    msgs = []
    ok = True
    runs = [("5205 968×548 ブラインド", cat, 5205, 968, 548, 1.4616, False, 71, {}),
            ("5206 鏡像 536×368 ヒント", cat, 5206, 536, 368, 3.15, True, 72, {"hint": True}),
            ("4108 鏡像 1608×1104 ブラインド", cat, 4108, 1608, 1104, 2.3188, True, 73, {})]
    tri = copy.copy(cat)
    tri.series = {k: v for k, v in cat.series.items() if k in (4111, 4112)}
    tri.files = [f for f in cat.files if f.indexid in (4111, 4112)]
    runs.append(("三角形 4111・4112 だけ 1200×800 30″/px", tri, 4111, 1200, 800, 30.0, False, 74,
                 {"scale_lo": 22.5, "scale_hi": 37.5}))
    for label, c, iid, W, H, s, mirror, seed, kw in runs:
        xy, fl, w, sol, dt = _solve_synth(c, iid, W, H, s, mirror, seed, **kw)
        if sol is None:
            ok = False
            msgs.append(f"{label}: 解けず({dt:.1f} 秒)")
            continue
        good, d = compare(measure(sol.wcs, W, H), measure(w, W, H), CRIT)
        v = sc.sv_verify_wcs(xy, fl, W, H, c, int(sol.index_name.split("-")[1]), sol.wcs,
                             quad=(sol.quad_field, sol.index_name, sol.quad_index))
        same = abs(v["logodds"] - sol.logodds) < 1e-6 and v["nmatch"] == sol.nmatch
        ok &= good and same
        msgs.append(f"{label}: {sol.index_name} 対数オッズ {sol.logodds:.1f}、中心 {d['center_arcsec']:.2f}″・"
                    f"スケール {d['scale_pct']:+.3f}%・回転 {d['rot_deg']:+.3f}°・鏡像 {'一致' if d['parity_same'] else '違う'}"
                    f"、{dt:.2f} 秒" + ("" if same else f"、検証し直すと {v['logodds']:.3f}(一致 {v['nmatch']})"))
    return ok, "; ".join(msgs)


def case_08_deadline():
    """締め切り: 乱数の星野(解は無い)をブラインドで 3 秒 → None、超過 1 秒以内、cancel() の間隔 0.2 秒以内。
    締め切りが過ぎていれば即座に None。"""
    cat = catalog()
    r = np.random.default_rng(8)
    W, H = 968, 548
    xy = np.column_stack([r.uniform(0, W - 1, 150), r.uniform(0, H - 1, 150)])
    fl = r.uniform(1, 100, 150)
    calls = []

    def cancel():
        calls.append(time.monotonic())
        return False

    t0 = time.monotonic()
    sol = sc.sv_solve(xy, fl, W, H, cat, deadline=t0 + 3.0, cancel=cancel)
    t1 = time.monotonic()
    gaps = np.diff(np.array([t0] + calls + [t1]))
    t2 = time.monotonic()
    sol2 = sc.sv_solve(xy, fl, W, H, cat, deadline=t2 - 1.0)
    t3 = time.monotonic()
    over = t1 - (t0 + 3.0)
    ok = sol is None and sol2 is None and over <= 1.0 and gaps.max() <= 0.2 and t3 - t2 < 0.5
    return ok, (f"解なし、超過 {over:.3f} 秒、cancel() {len(calls)} 回・間隔の最大 {gaps.max():.3f} 秒;"
                f" 過ぎた締め切り → {t3 - t2:.3f} 秒で None")


def case_09_cancel():
    """中止: cancel() が 1 秒後から True → SvCancelled(True になってから 0.2 秒以内)。"""
    cat = catalog()
    r = np.random.default_rng(9)
    W, H = 968, 548
    xy = np.column_stack([r.uniform(0, W - 1, 150), r.uniform(0, H - 1, 150)])
    t0 = time.monotonic()
    first = []

    def cancel():
        now = time.monotonic()
        if now >= t0 + 1.0:
            first.append(now)
            return True
        return False

    raised = False
    try:
        sc.sv_solve(xy, None, W, H, cat, deadline=t0 + 30.0, cancel=cancel)
    except sc.SvCancelled:
        raised = True
    t1 = time.monotonic()
    late = t1 - (t0 + 1.0)
    ok = raised and late <= 0.2
    return ok, f"SvCancelled {'あり' if raised else 'なし'}、中止の合図から {late:.3f} 秒で止まった"


def case_10_trace():
    """trace: 入力の星・quad の数・当たり・仮説(4 点の WCS と対数オッズ)・解の WCS が JSON に入る。"""
    cat = catalog()
    xy, fl, w = synth(cat, 5205, 61.89, 41.96, 1.4616, 968, 548, 152.9, True, n=70, noise=0.3, seed=10)
    path = os.path.join(tempfile.mkdtemp(prefix="zahyou_trace_"), "trace.json")
    sol = sc.sv_solve(xy, fl, 968, 548, cat, deadline=time.monotonic() + 60, trace=path)
    with open(path, encoding="utf-8") as fh:
        t = json.load(fh)
    hl = t.get("hypotheses_log", [])
    keys = ("status", "xy", "flux", "params", "files", "quads_tried", "hits", "hypotheses", "quick_rejected",
            "flushes", "hypotheses_log", "solution")
    miss = [k for k in keys if k not in t]
    solved_hyp = [h for h in hl if h["quad_field"] == np.asarray(sol.quad_field).tolist()
                  and h["quad_index"] == np.asarray(sol.quad_index).tolist()] if sol else []
    ok = (sol is not None and not miss and t["status"] == "solved" and t["quads_tried"] == sol.quads_tried
          and t["hypotheses"] == sol.hypotheses and len(hl) > 0
          and all(k in hl[0] for k in ("crval", "cd", "crpix", "logodds", "quick_rejected"))
          and len(solved_hyp) >= 1 and solved_hyp[0]["logodds"] is not None
          and "cd" in t["solution"]["wcs"] and len(t["xy"]) == len(xy))
    return ok, (f"{os.path.getsize(path) / 1024:.0f} KiB、欠けたキー {miss or 'なし'}、quad {t.get('quads_tried')}・"
                f"当たり {t.get('hits')}・仮説 {t.get('hypotheses')}(記録 {len(hl)}、足切り {t.get('quick_rejected')})、"
                f"解いた仮説の 4 点の対数オッズ {solved_hyp[0]['logodds'] if solved_hyp else None}")


def _ref_accumulate(row, s2row, NRh, ref_xy, logbg, d, bail):
    """検証の積み上げを 1 つずつ素直に(調べ物ノート 4.4 と solver_core の _sv_accumulate の説明どおり)。

    row: 画像の星の位置 (NT, 2)(詰め物は nan)、s2row: σ² (NT,)、ref_xy: その仮説の参照星 (NR, 2)。
    戻り値 (best, nmatch, ndistract, nconflict)(best のときの数)。
    """
    lnfg0 = math.log((1.0 - d) / (2.0 * math.pi * NRh))
    NT = len(row)
    owner, fg_at, mu_at = {}, [0.0] * NT, [0.0] * NT
    S = [0.0] * (NT + 1)
    lo, best, mu = 0.0, -math.inf, 0
    nm = nd = nc = 0
    got = (0, 0, 0)

    def logd(m):
        return math.log(d + (1.0 - d) * m / NRh) + logbg

    for i in range(NT):
        if not np.isfinite(row[i, 0]):
            S[i + 1] = S[i]
            continue
        d2 = ((ref_xy - row[i]) ** 2).sum(axis=1)
        k = int(np.argmin(d2)) if len(d2) else -1
        if k >= 0 and d2[k] > 25.0 * s2row[i]:
            k = -1
        logfg = lnfg0 - math.log(s2row[i]) - d2[k] / (2.0 * s2row[i]) if k >= 0 else -math.inf
        ld, ld_m1 = logd(mu), logd(max(mu - 1, 0))
        S[i + 1] = S[i]
        if logfg < ld:
            nd += 1
            term = ld
            S[i + 1] += ld_m1 - ld
        elif k in owner:
            nc += 1
            j = owner[k]
            sw = logfg + (logd(mu_at[j]) - fg_at[j]) + (S[i] - S[j + 1])
            if sw > ld:
                owner[k], fg_at[i], mu_at[i] = i, logfg, mu
                term = sw
            else:
                term = ld
                S[i + 1] += ld_m1 - ld
        else:
            owner[k], fg_at[i], mu_at[i] = i, logfg, mu
            mu += 1
            nm += 1
            term = logfg
        lo += term - logbg
        if lo > best:
            best, got = lo, (nm, nd, nc)
        if lo < bail:
            break
    return best, got[0], got[1], got[2]


def case_11_accumulate():
    """検証の積み上げ(1 対 1 の付け替えを含む)の、仮説をまとめた numpy の書き方(sv_verify_core)=
    1 つずつの素直な書き方。同じ参照星の近くに画像の星を 2 つ置いて付け替えを起こす。"""
    r = np.random.default_rng(11)
    p = sc.SvParams()
    H, NTmax = 400, 40
    test_pos, rows, s2, refs, rstart, rcount, logbg = [], [], [], [], [], [], []
    for h in range(H):
        NR = int(r.integers(1, 26))
        rx = r.uniform(0, 300, (NR, 2))
        sig = float(r.uniform(0.3, 2.0))
        NT = int(r.integers(1, NTmax + 1))
        pts = []
        for _ in range(NT):
            u = r.random()
            if u < 0.55:
                pts.append(rx[int(r.integers(0, NR))] + r.normal(0, sig * float(r.uniform(0.2, 3.0)), 2))
            elif u < 0.7 and pts:
                pts.append(pts[int(r.integers(0, len(pts)))] + r.normal(0, sig, 2))    # 同じ参照星の取り合い
            else:
                pts.append(r.uniform(0, 300, 2))
        base = len(test_pos)
        test_pos.extend(pts)
        rows.append(list(range(base, base + NT)) + [-1] * (NTmax - NT))
        s2.append(list(sig * sig * (1.0 + r.uniform(0, 3, NT))) + [1.0] * (NTmax - NT))
        rstart.append(sum(rcount))
        rcount.append(NR)
        refs.append(rx)
        logbg.append(-math.log(300.0 * 300.0) + float(r.uniform(-1, 1)))
    test_xy = np.array(test_pos)
    test_idx = np.array(rows, dtype=np.int64)
    sig2 = np.array(s2)
    ref_xy = np.concatenate(refs)
    res = sc.sv_verify_core(test_xy, test_idx, sig2, ref_xy, np.array(rstart), np.array(rcount), np.array(logbg),
                            p, record=True)
    bad = 0
    dmax = 0.0
    for h in range(H):
        row = np.where(test_idx[h][:, None] >= 0, test_xy[np.maximum(test_idx[h], 0)], np.nan)
        b, nm, nd, nc = _ref_accumulate(row, sig2[h], rcount[h], refs[h], logbg[h], p.distractor, p.logodds_bail)
        dmax = max(dmax, abs(b - res["best"][h]))
        bad += (abs(b - res["best"][h]) > 1e-4 or nm != res["nmatch"][h] or nd != res["ndistract"][h]
                or nc != res["nconflict"][h] or len(res["matches"][h]) != nm)
    ok = bad == 0 and int(res["nconflict"].sum()) > 50
    return ok, (f"{H} 仮説で食い違い {bad}、対数オッズの差の最大 {dmax:.1e}、一致の計 {int(res['nmatch'].sum())}・"
                f"偽物 {int(res['ndistract'].sum())}・取り合い {int(res['nconflict'].sum())}")


UNIT = [
    ("01 星の並べ替え(明るい順 → 一様化)", case_01_order, ()),
    ("02 newpoint の quad 作り = 総当たり(4 星・3 星)", case_02_newpoint, ()),
    ("03 画像側のコード = 索引のコード(code_ball で元の quad、両方の向き、三角形)", case_03_codes,
     (5206, 5202, 4107, 4111, 4113)),
    ("04 4 点の当てはめ(束でまとめて = 1 組ずつ)", case_04_similarity, ()),
    ("05 小さい円の検索 = 箱を持つ検索 = stars_in_cone", case_05_small_cone, (5205, 5202, 4107, 4111)),
    ("06 検証の対数オッズ(真 ≥ ln 1e9、乱数・回転・鏡像は低い)", case_06_verify, (5205, 4108, 5203)),
    ("07 合成星野を解く(4 星・鏡像・三角形の索引)", case_07_synth_solve, (5205, 5206, 4108, 4111, 4112)),
    ("08 締め切り(解なし・超過 1 秒以内・cancel() の間隔 0.2 秒以内)", case_08_deadline, (5205,)),
    ("09 中止(SvCancelled が 0.2 秒以内)", case_09_cancel, (5205,)),
    ("10 trace の中身", case_10_trace, (5205,)),
    ("11 検証の積み上げ(1 対 1 の付け替え)= 1 つずつの素直な書き方", case_11_accumulate, ()),
]


# ------------------------------------------------------------------ 実写 ---

def load_detector():
    """星の検出に使う zahyou_core(既定は git の main の版を一時フォルダーに写して読む)。"""
    if ARGS.detect == "main":
        try:
            src = subprocess.run(["git", "-C", REPO, "show", "main:dev/zahyou_core.py"], capture_output=True,
                                 check=True, timeout=60).stdout
            d = tempfile.mkdtemp(prefix="zahyou_core_main_")
            p = os.path.join(d, "zahyou_core_main.py")
            with open(p, "wb") as fh:
                fh.write(src)
            spec = importlib.util.spec_from_file_location("zahyou_core_main", p)
            m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(m)
            return m, "main"
        except Exception as e:
            print(f"  (main の zahyou_core を読めないので dev の版を使う: {type(e).__name__}: {e})")
    import zahyou_core as m
    return m, "dev"


def detect(zc, path):
    """標準の検出(detect_sources(min_sources=60))。12 個未満なら 2×2 にまとめた深い検出((2x+0.5, 2y+0.5) に戻す)。

    戻り値 (xy, flux, W, H, どちらを使ったか, 標準の数, 深い数)。
    """
    from astropy.stats import sigma_clipped_stats
    b = zc.load_image_any(path)
    sub, mask, sigma, _ = zc.preprocess(b.data)
    src, _ = zc.detect_sources(sub, mask, sigma, min_sources=60)
    H, W = b.shape[:2]
    n_std = len(src)
    n_deep = None
    used = "標準"
    if n_std < 12:
        ny, nx = sub.shape
        h2, w2 = ny // 2 * 2, nx // 2 * 2
        s2 = np.asarray(sub[:h2, :w2], dtype=np.float32).reshape(h2 // 2, 2, w2 // 2, 2).sum(axis=(1, 3))
        m2 = np.asarray(mask[:h2, :w2]).reshape(h2 // 2, 2, w2 // 2, 2).any(axis=(1, 3))
        ref = s2[~m2] if (~m2).any() else s2
        sig2 = float(sigma_clipped_stats(ref, sigma=3.0)[2])
        if not np.isfinite(sig2) or sig2 <= 0:
            sig2 = 2.0 * float(sigma)
        dsrc, _ = zc.detect_sources(s2, m2, sig2, min_sources=60)
        n_deep = len(dsrc)
        if n_deep >= n_std:
            src = [dict(s, x=2.0 * s["x"] + 0.5, y=2.0 * s["y"] + 0.5) for s in dsrc]
            used = "深い"
    xy = np.array([[s["x"], s["y"]] for s in src], dtype=np.float64).reshape(-1, 2)
    fl = np.array([s["flux"] for s in src], dtype=np.float64)
    return xy, fl, W, H, used, n_std, n_deep


def truth_measure(t):
    xyz = np.array(t["center_xyz"]) if "center_xyz" in t else sc.sv_radec_to_xyz(t["center_ra"], t["center_dec"])
    return dict(xyz=xyz, scale=t["scale_arcsec"], pa=t["pa_up_deg"], parity=t["parity"])


def real_cases(doc):
    """実写の回帰。1 枚ごとに検出し、3 条件(陰性はブラインドだけ、--full なら 3 条件)で解く。"""
    cat = catalog()
    imgs = {e["tag"]: e for e in doc["images"]}
    crit = doc["criteria"]
    zc, which = load_detector()
    print(f"  (星の検出は {which} の zahyou_core)", flush=True)
    sel = [s.strip() for s in ARGS.case.split(",")] if ARGS.case else None
    for tag in POSITIVE + NEGATIVE:
        if sel and not any(s in tag for s in sel):
            continue
        e = imgs.get(tag)
        if e is None or not os.path.exists(e["path"]):
            record(f"R {tag}", None, "画像が無い" if e else "正解ファイルに無い")
            continue
        if cat is None:
            record(f"R {tag}", None, f"索引が無い({DATA})")
            continue
        try:
            xy, fl, W, H, used, n_std, n_deep = detect(zc, e["path"])
        except Exception as ex:
            record(f"R {tag}", False, f"検出で例外: {type(ex).__name__}: {ex}")
            continue
        det = f"星 {len(xy)} 個({used}検出。標準 {n_std}" + ("" if n_deep is None else f"・深い {n_deep}") + ")"
        positive = tag in POSITIVE
        if positive:
            t = e["truth"]
            want = [truth_measure(t)]
            if "truth_tan" in e.get("accept", []):
                want.append(truth_measure(e["truth_tan"]))
            ra, dec = t["center_ra"], t["center_dec"]
            # 正解の中心から 0.5° ずらしたヒント(北へ 0.3°、東へ 0.4°)
            modes = [("ブラインド", {}),
                     ("ヒント", {"hint": (ra + 0.4 / math.cos(math.radians(dec)), dec + 0.3, 5.0)}),
                     ("スケール", {"scale_lo": 0.75 * t["scale_arcsec"], "scale_hi": 1.25 * t["scale_arcsec"]})]
        else:
            hr, hd, hs = NEG_HINT[tag]
            modes = [("ブラインド", {})]
            if ARGS.full:
                modes += [("ヒント", {"hint": (hr, hd, 5.0)}), ("スケール", {"scale_lo": 0.75 * hs, "scale_hi": 1.25 * hs})]
        for label, kw in modes:
            name = f"R {tag} {label}"
            t0 = time.monotonic()
            try:
                sol = sc.sv_solve(xy, fl, W, H, cat, deadline=t0 + ARGS.deadline, **kw)
            except Exception as ex:
                record(name, False, f"{det}: 例外 {type(ex).__name__}: {ex}\n{traceback.format_exc()}")
                continue
            dt = time.monotonic() - t0
            rec = dict(tag=tag, mode=label, positive=positive, nstars=len(xy), detect=used, seconds=round(dt, 3),
                       solved=sol is not None)
            if sol is not None:
                rec.update(index=sol.index_name, logodds=round(sol.logodds, 2), nmatch=sol.nmatch,
                           nfield=sol.nfield, nref=sol.nref, quads=sol.quads_tried, hyps=sol.hypotheses,
                           sip=sol.wcs.has_sip)
            if positive:
                if sol is None:
                    record(name, False, f"{det}: 解けず({dt:.2f} 秒)", rec)
                    continue
                got = measure(sol.wcs, W, H)
                res = [compare(got, w_, crit) for w_ in want]
                good = any(r_[0] for r_ in res)
                d = res[0][1] if (res[0][0] or not good) else next(r_[1] for r_ in res if r_[0])
                rec.update(center_arcsec=round(d["center_arcsec"], 3), scale_pct=round(d["scale_pct"], 4),
                           rot_deg=round(d["rot_deg"], 4), parity_same=d["parity_same"])
                record(name, good,
                       f"{det}: {sol.index_name} 対数オッズ {sol.logodds:.1f}、一致 {sol.nmatch}、quad {sol.quads_tried}、"
                       f"仮説 {sol.hypotheses}、{dt:.2f} 秒 / 中心 {d['center_arcsec']:.2f}″(≤ {d['center_lim']:.2f})・"
                       f"スケール {d['scale_pct']:+.3f}%・回転 {d['rot_deg']:+.3f}°・鏡像 {'一致' if d['parity_same'] else '違う'}"
                       + ("・SIP" if sol.wcs.has_sip else "") + ("" if res[0][0] or not good else "(TAN の正解と比べて)"),
                       rec)
            else:
                over = dt - ARGS.deadline
                rec["overrun"] = round(over, 3)
                good = sol is None and over <= crit["negative_overrun_s"]
                record(name, good, f"{det}: " + (f"解なし、締め切りの超過 {over:+.3f} 秒" if sol is None else
                                                 f"解けてしまった {sol.index_name} 対数オッズ {sol.logodds:.1f}"), rec)


# ------------------------------------------------------------------ 本体 ---

def main():
    global ARGS
    ap = argparse.ArgumentParser(description="solver_core.py の試験")
    ap.add_argument("--unit", action="store_true", help="部品だけ")
    ap.add_argument("--real", action="store_true", help="実写の回帰だけ")
    ap.add_argument("--full", action="store_true", help="解けてはいけない画像も 3 条件で")
    ap.add_argument("--deadline", type=float, default=60.0, help="実写 1 回の締め切り [秒]")
    ap.add_argument("--case", default="", help="名前の一部(カンマ区切り)で絞る")
    ap.add_argument("--json", default=None, help="結果を JSON に書く")
    ap.add_argument("--detect", choices=("main", "dev"), default="main")
    ARGS = ap.parse_args()
    try:
        sys.stdout.reconfigure(errors="replace")    # cp932 の画面でも止まらないように
    except Exception:
        pass
    t_all = time.perf_counter()
    do_unit = not ARGS.real or ARGS.unit
    do_real = not ARGS.unit or ARGS.real
    sel = [s.strip() for s in ARGS.case.split(",")] if ARGS.case else None
    if do_unit:
        for name, fn, ids in UNIT:
            if sel and not any(s in name for s in sel) and not any(s in fn.__name__ for s in sel):
                continue
            run_case(name, fn, *ids)
    if do_real:
        if not os.path.exists(TRUTH):
            record("R 正解ファイル", None, f"{TRUTH} が無い")
        else:
            with open(TRUTH, encoding="utf-8") as fh:
                real_cases(json.load(fh))
    if catalog() is not None:
        catalog().close()
    n_ok = sum(1 for r in RESULTS if r[1])
    n_skip = sum(1 for r in RESULTS if r[1] is None)
    n_run = len(RESULTS) - n_skip
    print()
    width = max([len(r[0]) for r in RESULTS] + [10])
    for name, ok, msg, _ in RESULTS:
        tag = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
        print(f"  {tag}  {name.ljust(width)}  {msg.splitlines()[0] if msg else ''}")
    print(f"\n  {n_ok}/{n_run} passed, {n_skip} skipped  ({time.perf_counter() - t_all:.0f} s)")
    if ARGS.json:
        with open(ARGS.json, "w", encoding="utf-8") as fh:
            json.dump([dict(name=n, ok=o, msg=m, **r) for n, o, m, r in RESULTS], fh, ensure_ascii=False, indent=1)
    return 0 if n_ok == n_run else 1


if __name__ == "__main__":
    sys.exit(main())

# -*- coding: utf-8 -*-
"""
zahyou-solver(Rust)の照合用の値を Python 基準実装で作る。

  python -B gen_solver.py [--dev <dev フォルダー>] [--index-dir C:\\AstrometryData] [--only parts,synth,real]
                          [--nhyp 24] [--deadline 600] [--cases s5205,ra0,...] [--tags real_orig,...]

基準実装は zahyou-rs の dev/(solver_wcs.py・solver_index.py・solver_core.py・solver_synth.py・zahyou_core.py)。
dev/ のファイルは読むだけで書き換えない(trace は一時フォルダーに書く)。

書くもの(このスクリプトと同じフォルダー):
  solver_parts.json   部品の値: 星の並べ替え(明るい順 → 一様化、検証の星の並び)、newpoint の quad 作り
                      (足した星ごとの quad の並び)、画像側のコードと正準化・鏡像の形(sv_code_variants)、
                      AB の範囲、検証の積み上げ(1 対 1 の付け替えを含む sv_verify_core)、一致の重み、F 分布の点、
                      HEALPix の 1 辺、解の判定(sv_accept_logodds・sv_tuneup_accepted)と見送った候補のまとめ(sv_passed_note)、
                      4 点の当てはめ(束)、検証の対数オッズ(sv_verify_wcs。真の WCS・乱数の WCS・
                      回転・鏡像、quad あり・なし。索引が要る)
  solver_synth.json   合成星野(dev/test_solver_synth.py の場合から代表。星は solver_synth.py が種で作る)と、
                      Python の sv_solve の結果(解いた索引・quad の 4 点・対数オッズ・一致数・最終の WCS・試した quad の数・
                      束ごとの経過・最初の nhyp 個の仮説の 4 点の WCS と対数オッズ)。乱数の星野(解けてはいけない)も入れる。
                      選んだ索引の一覧は数と sha256(改行でつないだ名前)だけ(24 本以下なら名前も)
  solver_real.json    実写(dev/solver_truth.json の画像。手元にあるものだけ)の星のリスト(load_image_any → preprocess →
                      detect_sources(min_sources=60) / detect_sources_deep)と、solve_offline と同じ試行のはしごを
                      sv_solve で順に解いた結果(解けた試行は trace の要点つき)、solve_offline そのものの結果

決まり:
  - 締め切りは十分に長く(既定 600 秒)とり、Python が締め切りで止まらないようにする(止まると結果が時間しだいになる)。
    締め切りに当たった試行は "status": "deadline" と書く(Rust の試験はその試行を突き合わせない)。
  - 仮説の対数オッズ・スケールは trace では丸めて書かれるので、solver_core の名前空間の round を差し替えて
    丸めずに書かせる(探索の中身には round を使っていない)。
  - JSON に NaN・無限大は書かない(null にする)。乱数の入力は小数 2〜3 桁に丸めてから Python に渡す
    (丸めた値がそのまま入力なので、Rust 側も同じ値で計算できる)。
  - 利用者のファイル(%LOCALAPPDATA%\\zahyou の cache.json・gui.json)は使わない: ZAHYOU_CACHE・ZAHYOU_SETTINGS・
    LOCALAPPDATA を一時フォルダーに向けてから zahyou_core を読む。
  - 画像の path は solver_truth.json の決まり($ZAHYOU_TEST_IMAGES/ で始まるか、リポジトリからの相対)。
    リポジトリからの相対で zahyou-rs に無いもの(dev/tcase・dev/realcase は .gitignore)は、環境変数
    ZAHYOU_PY_REPO(既定 ~/Documents/GitHub/zahyou)の同じ相対パスも見る。
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import io
import json
import math
import os
import sys
import tempfile
import time
import warnings

sys.dont_write_bytecode = True
warnings.simplefilter("ignore")

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
DEFAULT_DEV = os.path.join(REPO, "dev")
DEFAULT_PY_REPO = os.path.join(os.path.expanduser("~"), "Documents", "GitHub", "zahyou")
IMAGES_VAR = "$ZAHYOU_TEST_IMAGES/"

# 利用者の記憶ファイル・設定を読まない・書かない(zahyou_core を読む前に決める)
_TMP = tempfile.mkdtemp(prefix="zahyou_gen_solver_")
os.environ["ZAHYOU_CACHE"] = os.path.join(_TMP, "cache.json")
os.environ["ZAHYOU_SETTINGS"] = os.path.join(_TMP, "gui.json")
os.environ["LOCALAPPDATA"] = os.path.join(_TMP, "localappdata")
os.makedirs(os.environ["LOCALAPPDATA"], exist_ok=True)

import numpy as np                                            # noqa: E402

# 合成星野の代表(dev/test_solver_synth.py の CASES の名前)。種は一覧の順で決まるので名前で選ぶ
SYNTH_CASES = ["s5202", "s5205", "s5206", "s4107", "s4110", "t4111", "t4113", "t4117", "t4119",
               "w10deg", "w25deg", "rot090", "rot211", "mir000", "mir152", "tile_edge", "tile_corner", "ra0",
               "polein", "pole89_h", "south60_h", "ra0_h", "hs", "hb", "bs", "bb", "sip_pin", "sip_bar", "few20",
               "many300", "wide30_s12", "rand60", "rand300", "rand_hint"]
NEG_DEADLINE = 15.0                     # 乱数の星野を Python で確かめる締め切り [秒](解なしの確認だけ)


# ------------------------------------------------------------------ 道具 ---

def finite_only(v):
    """JSON に NaN・無限大を書かない(Rust の serde_json は読めない)。null にする。"""
    if isinstance(v, float):
        return v if math.isfinite(v) else None
    if isinstance(v, dict):
        return {k: finite_only(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [finite_only(x) for x in v]
    if isinstance(v, np.ndarray):
        return finite_only(v.tolist())
    if isinstance(v, (np.floating,)):
        return finite_only(float(v))
    if isinstance(v, (np.integer,)):
        return int(v)
    if isinstance(v, (np.bool_,)):
        return bool(v)
    return v


def write_json(name, obj):
    path = os.path.join(HERE, name)
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(finite_only(obj), f, ensure_ascii=False, indent=None, separators=(",", ":"), allow_nan=False)
        f.write("\n")
    print(f"wrote {name} ({os.path.getsize(path):,} バイト)", flush=True)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def sha_stars(stars):
    """星のリスト [(x, y, flux)] を '<f8' のバイト列にした sha256(Rust の試験で同じ値を作れる)。"""
    a = np.asarray(stars, dtype="<f8").reshape(-1, 3)
    return hashlib.sha256(a.tobytes()).hexdigest()


def ints(a):
    return [int(v) for v in np.asarray(a).reshape(-1)]


def setup(dev):
    sys.path.insert(0, dev)
    import solver_wcs as sw
    import solver_index as si
    import solver_core as sc
    import solver_synth as ss
    # trace の仮説の対数オッズ・スケールを丸めずに書かせる(solver_core の名前空間の round だけを差し替える。
    # 探索の中身の四捨五入は _sv_round(math.floor)で、round は使っていない)
    sc.round = lambda v, n=None: v
    return sw, si, sc, ss


def restrict(catalog, ids):
    """目録のうち系列 ids だけを見せる写し(ファイルは共有)。test_solver_synth.restrict と同じ。"""
    import copy
    c = copy.copy(catalog)
    c.series = {i: catalog.series[i] for i in ids if i in catalog.series}
    c.files = [f for f in catalog.files if f.indexid in c.series]
    return c


# ------------------------------------------------------------------ 部品 ---

def gen_parts(sw, si, sc, ss, catalog):
    p = sc.SvParams()
    out = {"format": "zahyou-solver-parts-1",
           "about": "gen_solver.py が dev/solver_core.py の部品で計算した値。入力は小数 1〜3 桁に丸めてから渡した",
           "params": p.as_dict()}

    # (a) 星の並べ替え: sv_order_stars と、_SvField の検証の星の並び(vidx)
    r = np.random.default_rng(101)
    sizes = [(968, 548), (536, 368), (2704, 1536), (400, 1200), (900, 600)]
    order = []
    for t in range(34):
        W, H = sizes[t % 5]
        n = int(r.integers(1, 61)) if t < 30 else int(r.integers(300, 361))
        nd = 2 if t < 30 else 1
        xy = np.round(np.column_stack([r.uniform(-0.5, W - 0.5, n), r.uniform(-0.5, H - 0.5, n)]), nd)
        if t % 7 == 3:
            xy[: max(1, n // 4), 0] = -0.5               # 箱の境界(画像の縁)ちょうどの星
            xy[: max(1, n // 6), 1] = H - 0.5
        kind = t % 4
        if kind == 0:
            flux = None
        elif kind == 1:
            flux = r.integers(0, 50, n).astype(float)    # 同じ値が多い
        elif kind == 2:
            flux = np.round(r.uniform(0, 1000, n), 3)
        else:
            flux = np.round(r.uniform(0, 1000, n), 3)
            k = r.choice(n, max(1, n // 5), replace=False)
            flux[k] = np.nan                            # 非有限は −∞ として最後に(JSON では null)
        nb = int(r.integers(1, 21)) if t % 6 else 10
        o = sc.sv_order_stars(xy, flux, W, H, nb)
        nbx = max(1, sc._sv_round(math.sqrt(nb * W / H)))
        nby = max(1, sc._sv_round(nb / nbx))
        fp = sc.SvParams(uniformize_boxes=nb)
        field = sc._SvField(xy, flux, W, H, fp)
        assert np.array_equal(field.order, o)
        order.append(dict(width=W, height=H, nboxes=nb, nx=nbx, ny=nby, xy=xy.tolist(),
                          flux=None if flux is None else [None if not np.isfinite(v) else float(v) for v in flux],
                          order=ints(o), nq=int(field.nq), verify=ints(field.vidx)))
    # 箱の数の例(test_solver_core の case_01): 968×548・10 箱 → 4 × 3、隅の明るい 2 個と暗い 1 個 → [0, 2, 1]
    xy1 = np.array([[10.0, 10.0], [20.0, 10.0], [900.0, 500.0]])
    o1 = sc.sv_order_stars(xy1, np.array([3.0, 2.0, 1.0]), 968, 548, 10)
    order.append(dict(width=968, height=548, nboxes=10, nx=4, ny=3, xy=xy1.tolist(), flux=[3.0, 2.0, 1.0],
                      order=ints(o1), nq=3, verify=ints(sc._SvField(xy1, np.array([3.0, 2.0, 1.0]), 968, 548,
                                                                      p).vidx)))
    out["order"] = order

    # (b) newpoint の quad 作り: 足した星 n ごとの sv_quads_newest の並び(列は A, B, 内側の星(番号の小さい順))
    r = np.random.default_rng(102)
    newpoint = []
    for dq in (4, 3):
        for trial in range(5):
            N = int(r.integers(8, 19))
            P = np.round(np.column_stack([r.uniform(0, 900, N), r.uniform(0, 600, N)]), 1)
            lo = 60.0
            hi = float(np.round(r.uniform(300, 1100), 1))
            if trial == 4:
                P[3] = P[1] + np.array([0.0, 0.0])      # 同じ位置の星(AB の長さ 0 の組)
            L2, inside = sc.sv_quad_geometry(P, p.codetol)
            per = []
            tot = 0
            for n in range(N):
                q = sc.sv_quads_newest(n, dq, L2, inside, lo * lo, hi * hi)
                per.append([ints(row) for row in q])
                tot += len(q)
            newpoint.append(dict(dimquads=dq, points=P.tolist(), ab_lo=lo, ab_hi=hi, codetol=p.codetol,
                                 quads=per, total=tot))
    out["newpoint"] = newpoint

    # (c) 画像側のコードと照合に投げる形(正準化・境界の近くは両方・鏡像)
    r = np.random.default_rng(103)
    codes = []

    def place(code_xy, dq, rng):
        """コード (cx, cy[, dx, dy]) になる 4(3)点を作り、乱数の相似変換(回転・拡大・平行移動)をかける。
        A = (0, 0)、B = (1, 1)(コードの座標)= 画像では A + d·(x, y)。鏡像はかけない(コードが変わるため)。"""
        d = 1.0
        pts = [(0.0, 0.0), (1.0, 1.0)] + [(code_xy[2 * k], code_xy[2 * k + 1]) for k in range(dq - 2)]
        pts = np.array(pts) * d
        th = float(rng.uniform(0, 2 * math.pi))
        s = float(rng.uniform(80, 400))
        R = np.array([[math.cos(th), -math.sin(th)], [math.sin(th), math.cos(th)]]) * s
        t = rng.uniform(0, 800, 2)
        return pts @ R.T + t

    for dq in (4, 3):
        pts_all, quads = [], []
        # 乱数の quad(内側の星は円の中とは限らない)
        for _ in range(40):
            P = np.round(np.column_stack([r.uniform(0, 1000, dq), r.uniform(0, 700, dq)]), 2)
            base = len(pts_all)
            pts_all.extend(P.tolist())
            quads.append(list(range(base, base + dq)))
        # 正準化の境界の近く(cx + dx ≈ 1、cx ≈ dx、3 星は cx ≈ 1/2)
        offs = (-0.03, -0.016, -0.014, -0.005, 0.0, 0.005, 0.014, 0.016, 0.03)
        for e in offs:
            if dq == 4:
                cy, dy = float(r.uniform(0.1, 0.9)), float(r.uniform(0.1, 0.9))
                cx = float(r.uniform(0.2, 0.4))
                for c in ((cx, cy, 1.0 + e - cx, dy), (0.3 + e / 2, cy, 0.3 - e / 2, dy)):
                    P = place(c, dq, r)
                    base = len(pts_all)
                    pts_all.extend(P.tolist())
                    quads.append(list(range(base, base + dq)))
            else:
                c = (0.5 + e, float(r.uniform(0.1, 0.9)))
                P = place(c, dq, r)
                base = len(pts_all)
                pts_all.extend(P.tolist())
                quads.append(list(range(base, base + dq)))
        P = np.array(pts_all)
        Q = np.array(quads, dtype=np.int64)
        cd = sc.sv_field_codes(P, Q)
        var = {}
        for parity in ("both", "normal", "mirror"):
            q, row, perm, mir = sc.sv_code_variants(cd, dq, p.codetol, parity)
            var[parity] = dict(q=q.tolist(), row=ints(row), perm=[ints(x) for x in perm], mirror=[bool(v) for v in mir])
        codes.append(dict(dimquads=dq, codetol=p.codetol, points=P.tolist(), quads=Q.tolist(), codes=cd.tolist(),
                          variants=var))
    out["codes"] = codes

    # (d) quad の AB の範囲(sv_quad_ab_range)。段の値は手元の索引のヘッダー(無ければ設計の値)
    r = np.random.default_rng(104)
    ab = []
    for iid, lo_am, hi_am in ((5202, 4.0, 5.6), (5206, 16.0, 22.0), (4108, 30.0, 42.0), (4113, 170.0, 240.0)):
        lo_rad, hi_rad = math.radians(lo_am / 60.0), math.radians(hi_am / 60.0)
        if catalog is not None and iid in catalog.series:
            f0 = catalog.series[iid][0]
            lo_rad, hi_rad = f0.scale_lo_rad, f0.scale_hi_rad
        for W, H in ((968, 548), (1608, 1104)):
            s_lo = float(np.round(r.uniform(0.5, 30.0), 3))
            s_hi = float(np.round(s_lo * r.uniform(1.0, 3.0), 3))
            a, b = sc.sv_quad_ab_range(lo_rad, hi_rad, s_lo, s_hi, W, H, p.codetol, p.quad_min_frac)
            ab.append(dict(indexid=iid, scale_lo_rad=lo_rad, scale_hi_rad=hi_rad, s_lo=s_lo, s_hi=s_hi, width=W,
                           height=H, codetol=p.codetol, quad_min_frac=p.quad_min_frac, lo=a, hi=b))
    out["ab_range"] = ab

    # (e) 検証の積み上げ(sv_verify_core。test_solver_core の case_11 と同じ作り方で小さく)
    r = np.random.default_rng(105)
    H_, NTmax = 60, 24
    test_pos, rows, s2, refs, rstart, rcount, logbg = [], [], [], [], [], [], []
    for h in range(H_):
        NR = int(r.integers(1, 21))
        rx = np.round(r.uniform(0, 300, (NR, 2)), 2)
        sig = float(r.uniform(0.3, 2.0))
        NT = int(r.integers(1, NTmax + 1))
        pts = []
        for _ in range(NT):
            u = r.random()
            if u < 0.55:
                pts.append(rx[int(r.integers(0, NR))] + r.normal(0, sig * float(r.uniform(0.2, 3.0)), 2))
            elif u < 0.7 and pts:
                pts.append(pts[int(r.integers(0, len(pts)))] + r.normal(0, sig, 2))     # 同じ参照星の取り合い
            else:
                pts.append(r.uniform(0, 300, 2))
        base = len(test_pos)
        test_pos.extend(np.round(np.array(pts), 3).tolist())
        rows.append(list(range(base, base + NT)) + [-1] * (NTmax - NT))
        s2.append(list(np.round(sig * sig * (1.0 + r.uniform(0, 3, NT)), 4)) + [1.0] * (NTmax - NT))
        rstart.append(sum(rcount))
        rcount.append(NR)
        refs.append(rx)
        logbg.append(round(-math.log(300.0 * 300.0) + float(r.uniform(-1, 1)), 4))
    test_xy = np.array(test_pos)
    test_idx = np.array(rows, dtype=np.int64)
    sig2 = np.array(s2)
    ref_xy = np.concatenate(refs)
    res = sc.sv_verify_core(test_xy, test_idx, sig2, ref_xy, np.array(rstart), np.array(rcount),
                            np.array(logbg), p, record=True)
    out["accumulate"] = dict(
        distractor=p.distractor, logodds_bail=p.logodds_bail,
        test_xy=test_xy.tolist(), test_idx=test_idx.tolist(), sig2=sig2.tolist(), ref_xy=ref_xy.tolist(),
        ref_start=ints(rstart), ref_count=ints(rcount), logbg=logbg,
        best=res["best"].tolist(), nmatch=ints(res["nmatch"]), ndistract=ints(res["ndistract"]),
        nconflict=ints(res["nconflict"]), nused=ints(res["nused"]), best_col=ints(res["best_col"]),
        matches=[[[int(c), int(t), int(rr), float(lo)] for c, t, rr, lo in m] for m in res["matches"]])

    # (f) 一致の重み・F 分布の点・HEALPix の 1 辺
    lo_vals = [-1000.0, -700.0, -699.9, -50.0, -3.0, -0.5, 0.0, 0.25, 3.0, 20.0, 39.99, 40.0, 40.01, 300.0]
    out["logodds_weight"] = dict(lo=lo_vals, w=sc.sv_logodds_weight(lo_vals).tolist())
    out["f_upper"] = [dict(d1=d1, d2=d2, z=sc.SV_SIP_F_Z, f=sc.sv_f_upper(d1, d2))
                      for d1, d2 in ((6, 12), (6, 30), (14, 40), (24, 100), (14, 2), (1, 1))]
    out["healpix_side_rad"] = [dict(nside=n, side=sc.sv_healpix_side_rad(n)) for n in (1, 2, 8, 10, 16, 64)]

    # (f2) 解の判定: 試した仮説の数で上げた受け入れの値と、tune-up の一致の増え(既定の値・切ったとき・起点を変えたとき)
    nhyps = [0, 1, 8192, 99999, 100000, 100001, 123457, 2671113, 16187319, 20229488, 10**12]
    out["accept"] = [dict(accept_hyps0=h0, nhyp=n, logodds=sc.sv_accept_logodds(sc.SvParams(accept_hyps0=h0), n))
                     for h0 in (1e5, 1e6, 1.0, 0.0, -1.0) for n in nhyps]
    out["tuneup_accepted"] = [
        dict(tune_min_gain=g, tune_gain_hyps0=h0, nhyp=n, before=b, after=a,
             ok=sc.sv_tuneup_accepted(sc.SvParams(tune_min_gain=g, tune_gain_hyps0=h0), n, b, a))
        for g, h0 in ((1, 1e5), (2, 1e5), (0, 1e5), (1, 0.0), (1, 1e6))
        for n in (1, 4553, 100000, 100001, 244413, 1000000, 1000001, 2671113, 16187319)
        for b, a in ((5, 5), (4, 4), (5, 6), (4, 9), (24, 24), (20, 40), (7, 6), (0, 0))]

    # (f3) 改良の後の判定で見送った候補のまとめ(sv_passed_add の並び → 探索の終わりの行の末尾に足す文)。
    # 最大の候補の要った値を覚えること・同じ値なら先のもの・千の位の区切り・丸め(x.x5 ちょうど・負・-0.0)を見る
    acc = lambda n: sc.sv_accept_logodds(sc.SvParams(), n)  # noqa: E731
    seqs = [
        [],
        [("short", 18.734, acc(100000))],
        [("short", 21.0, acc(200001)), ("short", 24.95, acc(3000000)), ("short", 22.3, acc(16187319))],
        [("short", 22.5, 24.0), ("short", 22.5, 26.0)],
        [("nogain", 30.9), ("nogain", 22.773709888689385)],
        [("nogain", 21.4), ("short", 22.773709888689385, acc(16187319)), ("nogain", 22.25), ("short", -3.75, acc(1))],
        [("short", 22.25, 22.35), ("nogain", 20.75)],
        [("short", -3.75, acc(1))],
        [("short", -0.04, -0.06)],
        [("short", 15.0 + (i % 97) * 0.07, acc(100000 + 1000 * i)) for i in range(1001)]
        + [("nogain", 21.0 + 0.01 * i) for i in range(12)],
    ]
    out["passed_note"] = []
    for seq in seqs:
        st = {}
        for e in seq:
            sc.sv_passed_add(st, *e)
        out["passed_note"].append(dict(events=[list(e) for e in seq], note=sc.sv_passed_note(st)))
    # (f4) 見送りが出る探索の最後の行: 合成星野(5205 だけ・ヒント・スケール既知)を、受け入れの値を極端に上げて
    # (accept_hyps0 = 1e-300)解かせる。正しい候補も見送り、quad を試し尽くして最後の行にまとめが出る(時刻によらない)
    out["passed_solve"] = gen_passed_solve(sw, si, sc, ss, catalog) if catalog is not None else None

    # (g) 4 点の当てはめを束で(sv_fit_similarity_many)
    r = np.random.default_rng(106)
    F_, S_ = [], []
    for h in range(16):
        c = sw.sv_normalize(r.normal(size=3))
        s = float(10 ** r.uniform(-0.5, 1.5))
        th = math.radians(float(r.uniform(0, 360)))
        m = 1.0 if r.random() < 0.5 else -1.0
        cd = s / 3600.0 * np.array([[m * math.cos(th), math.sin(th)], [-m * math.sin(th), math.cos(th)]])
        w = sw.SvWcs(c, [float(r.uniform(0, 2000)), float(r.uniform(0, 1500))], cd)
        fxy = np.round(np.column_stack([r.uniform(0, 2000, 4), r.uniform(0, 1500, 4)]), 2)
        S_.append(w.pix_to_xyz(fxy[:, 0], fxy[:, 1]))
        F_.append(fxy + np.round(r.normal(0, 0.5, (4, 2)), 2))
    F_ = np.array(F_)
    S_ = np.array(S_)
    fit = sc.sv_fit_similarity_many(F_, S_)
    out["similarity"] = dict(field=F_.tolist(), stars=S_.tolist(), crval=fit["crval"].tolist(),
                             crpix=fit["crpix"].tolist(), cd=fit["cd"].tolist(), scale=fit["scale"].tolist(),
                             ok=[bool(v) for v in fit["ok"]])

    # (h) 検証の対数オッズ(sv_verify_wcs。索引が要る)
    out["verify"] = gen_verify(sw, si, sc, ss, catalog) if catalog is not None else []
    return out


def gen_passed_solve(sw, si, sc, ss, catalog):
    """改良の後の判定で候補を見送らせて試し尽くした探索の、最後のログの行(秒の所は試験の側で伏せる)。"""
    iid = 5205
    if iid not in catalog.series:
        return None
    W, H, s, (ra, dec) = 968, 548, 1.4616, (61.89, 41.96)
    fld = ss.sv_synth_field(catalog, ra, dec, s, W, H, rotation_deg=152.87, mirror=True, seed=1105, n_stars=60,
                            drop_frac=0.2, distractor_frac=0.2, pos_noise_px=0.3, series=[iid])
    xy = np.round(np.asarray(fld["xy"], dtype=np.float64), 3)
    fl = np.round(np.asarray(fld["flux"], dtype=np.float64), 3)
    hint, s_lo, s_hi, h0 = (ra + 0.5, dec - 0.3, 3.0), s * 0.9, s * 1.1, 1e-300
    lines = []
    t0 = time.monotonic()
    sol = sc.sv_solve(xy, fl, W, H, restrict(catalog, [iid]), scale_lo=s_lo, scale_hi=s_hi, hint=hint,
                      deadline=t0 + 600.0, log=lines.append, params=sc.SvParams(accept_hyps0=h0))
    dt = time.monotonic() - t0
    print(f"  見送りの探索: {'解けた' if sol is not None else '解なし'} {dt:.2f} s / {lines[-1] if lines else ''}", flush=True)
    return dict(series=[iid], width=W, height=H, xy=xy.tolist(), flux=fl.tolist(), hint=list(hint), scale_lo=s_lo,
                scale_hi=s_hi, accept_hyps0=h0, solved=sol is not None, log=lines)


def gen_verify(sw, si, sc, ss, catalog):
    """合成星野で、真の WCS・乱数の WCS・10° 回転・鏡像の対数オッズ(quad なし)と、quad ありの値。"""
    r = np.random.default_rng(107)
    rows = []
    specs = [(5205, 968, 548, 1.4616, False, (61.89, 41.96), 152.87), (4108, 1608, 1104, 2.3188, True, (180.0, 0.0), 40.0),
             (5203, 2704, 1536, 0.7473, False, (10.68, 41.27), 300.0)]
    for iid, W, H, s, mirror, (ra, dec), rot in specs:
        if iid not in catalog.series:
            continue
        fld = ss.sv_synth_field(catalog, ra, dec, s, W, H, rotation_deg=rot, mirror=mirror, seed=1000 + iid,
                                n_stars=80, drop_frac=0.2, distractor_frac=0.2, pos_noise_px=0.3, series=[iid])
        xy, fl, w = fld["xy"], fld["flux"], fld["truth"]
        wcss = [("truth", w)]
        for k in range(3):
            sep = math.radians(float(r.uniform(0.2, 5.0)))
            d = sw.sv_normalize(np.cross(w.crval_xyz, r.normal(size=3)))
            c2 = sw.sv_normalize(math.cos(sep) * w.crval_xyz + math.sin(sep) * d)
            th = math.radians(float(r.uniform(0, 360)))
            m = 1.0 if r.random() < 0.5 else -1.0
            cd = s / 3600.0 * np.array([[m * math.cos(th), math.sin(th)], [-m * math.sin(th), math.cos(th)]])
            wcss.append((f"random{k}", sw.SvWcs(c2, w.crpix, cd)))
        e = math.radians(10.0)
        wcss.append(("rot10", sw.SvWcs(w.crval_xyz, w.crpix,
                                       np.array([[math.cos(e), math.sin(e)], [-math.sin(e), math.cos(e)]]) @ w.cd)))
        wcss.append(("mirror", sw.SvWcs(w.crval_xyz, w.crpix, w.cd @ np.array([[0.0, 1.0], [1.0, 0.0]]))))
        # 真の WCS を少しずらしたもの(一致はするが対数オッズが下がる。偽物の判定と付け替えが働く)
        wcss.append(("shift1.5", sw.SvWcs(w.crval_xyz, w.crpix + np.array([1.5, 0.0]), w.cd)))
        wcss.append(("shift4", sw.SvWcs(w.crval_xyz, w.crpix + np.array([2.5, -3.0]), w.cd)))
        e = math.radians(0.3)
        wcss.append(("rot0.3", sw.SvWcs(w.crval_xyz, w.crpix,
                                        np.array([[math.cos(e), math.sin(e)], [-math.sin(e), math.cos(e)]]) @ w.cd)))
        wcss.append(("scale1.004", sw.SvWcs(w.crval_xyz, w.crpix, w.cd * 1.004)))
        cases = []
        for label, ww in wcss:
            v = sc.sv_verify_wcs(xy, fl, W, H, catalog, iid, ww)
            cases.append(dict(label=label, wcs=ww.to_dict(), logodds=v["logodds"], nmatch=v["nmatch"],
                              ndistract=v["ndistract"], nfield=v["nfield"], nref=v["nref"],
                              matches=[[int(a), float(c)] for a, _, c in v["matches"]]))
        rows.append(dict(indexid=iid, width=W, height=H, xy=np.asarray(xy).tolist(), flux=np.asarray(fl).tolist(),
                         cases=cases))
    return rows


# ------------------------------------------------------------------ sv_solve を trace つきで ---

def summarize_trace(t, nhyp, sc, sw):
    """sv_solve の trace(JSON)から、突き合わせに使う部分を抜き出す。"""
    hl = t.get("hypotheses_log", [])
    files = t.get("files", [])
    out = dict(status=t["status"], files_count=len(files),
               files_sha256=hashlib.sha256(chr(10).join(files).encode("utf-8")).hexdigest(),
               files=files if len(files) <= 24 else None, scale_used=t.get("scale_used"),
               quads_tried=t["quads_tried"], hits=t["hits"], hypotheses=t["hypotheses"],
               quick_rejected=t.get("quick_rejected", 0), logodds_max=t.get("logodds_max"),
               flushes=[dict(newest=fl["newest"], quads=fl["quads"], hits=fl["hits"], hyps=fl["hyps"])
                        for fl in t.get("flushes", [])],
               tuneups=t.get("tuneups", []), hypotheses_logged=len(hl),
               hyp_first=[hyp_entry(h) for h in hl[:nhyp]])
    s = t.get("solution")
    if s is not None:
        out["solution"] = dict(index=s["index"], logodds=s["logodds"], nmatch=s["nmatch"], ndistract=s["ndistract"],
                               nfield=s["nfield"], nref=s["nref"], quad_field=s["quad_field"],
                               quad_index=s["quad_index"], wcs=s["wcs"],
                               matches=[[int(m[0]), float(m[2])] for m in s["matches"]])
        # 解いた quad の仮説(4 点の当てはめ)が仮説の記録の何番目か
        for k, h in enumerate(hl):
            if (h["quad_field"] == s["quad_field"] and h["quad_index"] == s["quad_index"]
                    and h["index"] == s["index"]):
                out["solved_hyp"] = dict(ordinal=k, **hyp_entry(h))
                break
    return out


def hyp_entry(h):
    return dict(index=h["index"], row=h["row"], mirror=h["mirror"], quad_field=h["quad_field"],
                quad_index=h["quad_index"], crval=h["crval"], crpix=h["crpix"], cd=h["cd"], scale=h["scale"],
                quick_rejected=h["quick_rejected"], logodds=h["logodds"], nmatch=h["nmatch"])


def run_traced(sc, xy, flux, W, H, catalog, scale_lo, scale_hi, hint, deadline_s):
    """sv_solve を trace つきで回す。戻り値 (解, trace の dict, 秒)。"""
    path = os.path.join(_TMP, f"trace_{time.monotonic_ns()}.json")
    t0 = time.monotonic()
    sol = sc.sv_solve(np.asarray(xy, dtype=np.float64).reshape(-1, 2),
                      None if flux is None else np.asarray(flux, dtype=np.float64),
                      int(W), int(H), catalog, scale_lo=scale_lo, scale_hi=scale_hi, hint=hint,
                      deadline=t0 + deadline_s, trace=path)
    dt = time.monotonic() - t0
    with open(path, encoding="utf-8") as fh:
        t = json.load(fh)
    os.remove(path)
    return sol, t, dt


def measure_vs(ss, sw, wcs, W, H, truth_summary):
    want = {k: truth_summary[k] for k in ("center_ra", "center_dec", "center_xyz", "scale_arcsec", "pa_up_deg",
                                          "parity")}
    got = ss.sv_synth_measure(wcs, W, H, ref_xyz=want["center_xyz"])
    cmp = ss.sv_synth_compare(got, want)
    return dict(ok=cmp["ok"], failed=cmp["failed"], center_arcsec=cmp["center_arcsec"], scale_rel=cmp["scale_rel"],
                rot_deg=cmp["rot_deg"], parity=cmp["parity"])


# ------------------------------------------------------------------ 合成星野 ---

def gen_synth(sw, si, sc, ss, catalog, dev, args):
    import test_solver_synth as tss                 # 場合の一覧(種は一覧の順で決まる)
    names = args.cases.split(",") if args.cases else SYNTH_CASES
    by_name = {c["name"]: c for c in tss.CASES}
    out = {"format": "zahyou-solver-synth-1",
           "about": "dev/test_solver_synth.py の場合から代表を選び、solver_synth.py の星野を Python の sv_solve で解いた結果。"
                    "only は目録をその系列だけにする。hint は (赤経, 赤緯, 半径) 度。scale_lo/hi は null ならスケール不明",
           "tolerance": ss.SV_SYNTH_TOL, "nhyp": args.nhyp, "deadline_s": args.deadline, "cases": []}
    for name in names:
        c = by_name.get(name)
        if c is None:
            print(f"  ? 場合 {name} が無い", flush=True)
            continue
        ok, why = tss.needed(c, catalog)
        if not ok:
            print(f"  SKIP {name}: {why}", flush=True)
            out["cases"].append(dict(name=name, skipped=why))
            continue
        f = tss.make_field(c, catalog)
        W, H = c["size"]
        s_lo, s_hi = tss.scale_range(c)
        hint = tss.hint_of(c)
        rec = dict(name=name, what=c["what"], width=W, height=H, expect_solve=not c["random"], only=c["only"],
                   scale_lo=s_lo, scale_hi=s_hi, hint=None if hint is None else list(hint),
                   xy=np.asarray(f["xy"]).tolist(), flux=np.asarray(f["flux"]).tolist(),
                   truth=None if f["truth"] is None else f["truth"].to_dict(),
                   truth_summary=f["meta"].get("truth_summary"))
        cat = restrict(catalog, c["only"]) if c["only"] else catalog
        deadline = NEG_DEADLINE if c["random"] else args.deadline
        sol, t, dt = run_traced(sc, f["xy"], f["flux"], W, H, cat, s_lo, s_hi, hint, deadline)
        py = summarize_trace(t, args.nhyp, sc, sw)
        py["seconds"] = round(dt, 3)
        if sol is not None and f["truth"] is not None:
            py["vs_truth"] = measure_vs(ss, sw, sol.wcs, W, H, f["meta"]["truth_summary"])
        rec["python"] = py
        out["cases"].append(rec)
        tag = py["status"]
        extra = ""
        if sol is not None:
            extra = (f" {sol.index_name} lo={sol.logodds:.2f} nm={sol.nmatch} quads={sol.quads_tried} "
                     f"hyps={sol.hypotheses} ok={py.get('vs_truth', {}).get('ok')}")
        print(f"  {name:<14s} {tag:<9s} {dt:7.2f} s{extra}", flush=True)
    out["corrupt"] = gen_corrupt_field(sw, si, sc, ss, catalog, args)
    return out


def gen_corrupt_field(sw, si, sc, ss, catalog, args):
    """壊れた索引の試験の星野(test_solver_core の case_17 と同じ: 横 40°・2000×1300・赤経 150°・赤緯 20°・種 2)。
    Rust の試験は index-4113〜4117 を一時フォルダーに写し、4116 の中身を壊して解く。壊さないときの Python の解も書く。"""
    need = (4113, 4114, 4115, 4116, 4117)
    if any(i not in catalog.series for i in need):
        return None
    W, H, fov = 2000, 1300, 40.0
    scale = fov * 3600.0 / W
    fld = ss.sv_synth_field(catalog, 150.0, 20.0, scale, W, H, rotation_deg=20.0, mirror=False, seed=2, n_stars=80,
                            sip=None, drop_frac=0.3, distractor_frac=0.3)
    cat = restrict(catalog, need)
    sol, t, dt = run_traced(sc, fld["xy"], fld["flux"], W, H, cat, scale * 0.8, scale * 1.25, None, args.deadline)
    rec = dict(width=W, height=H, series=list(need), scale_lo=scale * 0.8, scale_hi=scale * 1.25,
               xy=np.asarray(fld["xy"]).tolist(), flux=np.asarray(fld["flux"]).tolist(),
               truth=fld["truth"].to_dict(), truth_summary=fld["meta"]["truth_summary"],
               python=dict(status=t["status"], seconds=round(dt, 3),
                           index=None if sol is None else sol.index_name,
                           logodds=None if sol is None else sol.logodds))
    print(f"  corrupt の星野(壊さない): {t['status']} {dt:.2f} s {rec['python']['index']}", flush=True)
    return rec


# ------------------------------------------------------------------ 実写 ---

def image_path(e, py_repo):
    p = e["path"]
    if p.startswith(IMAGES_VAR):
        base = os.environ.get("ZAHYOU_TEST_IMAGES") or os.path.expanduser("~")
        return os.path.join(base, *p[len(IMAGES_VAR):].split("/"))
    q = os.path.join(REPO, *p.split("/"))
    if os.path.isfile(q):
        return q
    return os.path.join(py_repo, *p.split("/"))


def attempts_of(zc, bundle, deep, std, hint_radec, hint_radius_deg, installed):
    """solve_offline と同じ試行のはしご(ラベル, 星の組の名前, ヒント, (s_lo, s_hi), 残り時間の割合)。"""
    ny, nx = bundle.shape
    scale, how = zc.estimate_pixel_scale(bundle.header, None, bundle.shape)
    hint = None
    if hint_radec is not None:
        hint = (float(hint_radec[0]), float(hint_radec[1]), float(hint_radius_deg))
    narrow = (max(scale * 0.75, 0.01), scale * 1.25) if scale else None
    wide = zc.blind_scale_bounds(nx, installed)
    star_sets = [(nm, st) for nm, st in (("深い検出", deep), ("標準の検出", std)) if len(st) >= 3]
    conds = []
    if hint and narrow:
        conds.append(("座標ヒント + スケール既知", hint, narrow, 0.25))
    if hint:
        conds.append(("座標ヒント + スケール全域", hint, wide, 0.25))
    if narrow:
        conds.append(("全天 + スケール既知", None, narrow, 0.25))
    conds.append(("全天 + スケール全域", None, wide, 0.5))
    out = []
    for cond, hint_i, rng, share in conds:
        for nm, st in star_sets:
            out.append(dict(label=f"{nm}の星 {len(st)} 個 + {cond}", stars="deep" if nm == "深い検出" else "standard",
                            hint=None if hint_i is None else list(hint_i), scale_lo=float(rng[0]),
                            scale_hi=float(rng[1]), share=share))
    return out, scale, how


def gen_real(sw, si, sc, ss, catalog, dev, args):
    import zahyou_core as zc
    truth_path = os.path.join(dev, "solver_truth.json")
    with open(truth_path, encoding="utf-8") as f:
        doc = json.load(f)
    base = doc.get("target_offset_baseline") or {}
    py_repo = os.environ.get("ZAHYOU_PY_REPO", DEFAULT_PY_REPO)
    installed = zc._catalog_names(catalog)
    out = {"format": "zahyou-solver-real-1",
           "about": "dev/solver_truth.json の画像を Python 基準実装で検出し、solve_offline と同じ試行のはしごを sv_solve で "
                    "順に解いた結果。stars は [x, y, flux](明るい順。detect_sources_deep / detect_sources(min_sources=60))。"
                    "正解・基準(criteria・criteria_override)は dev/solver_truth.json を読むこと",
           "nhyp": args.nhyp, "deadline_s": args.deadline, "images": []}
    tags = args.tags.split(",") if args.tags else None
    jobs = []
    for e in doc["images"]:
        if tags and e["tag"] not in tags:
            continue
        jobs.append((e, None))
        if base and e["tag"] == base["image"]:
            jobs.append((e, (base["ra_deg"], base["dec_deg"], 5.0)))
    cache = {}
    for e, hint in jobs:
        name = e["tag"] + ("+hint" if hint else "")
        path = image_path(e, py_repo)
        rec = dict(name=name, tag=e["tag"], path=e["path"], expect_solve=bool(e["expect_solve"]), width=e["width"],
                   height=e["height"], hint=None if hint is None else list(hint))
        if not os.path.isfile(path):
            rec["skipped"] = f"画像が無い({path})"
            out["images"].append(rec)
            print(f"  SKIP {name}: 画像が無い", flush=True)
            continue
        if sha256_file(path) != e["sha256"]:
            rec["skipped"] = "画像の中身が正解を作ったときと違う(sha256)"
            out["images"].append(rec)
            print(f"  SKIP {name}: sha256 が違う", flush=True)
            continue
        if e["tag"] not in cache:
            t0 = time.perf_counter()
            b = zc.load_image_any(path)
            sub, mask, sigma, info = zc.preprocess(b.data)
            src, thr = zc.detect_sources(sub, mask, sigma, min_sources=60)
            dinfo = {}
            deep, u = zc.detect_sources_deep(sub, mask, sigma, info=dinfo)
            cache[e["tag"]] = (b, sub, mask, sigma, src, deep, u, round(time.perf_counter() - t0, 3))
        b, sub, mask, sigma, src, deep, u, t_det = cache[e["tag"]]
        std_l = [[float(s["x"]), float(s["y"]), float(s["flux"])] for s in src]
        deep_l = [[float(s["x"]), float(s["y"]), float(s["flux"])] for s in deep]
        rec.update(flip_y=bool(getattr(b, "flip_y", False)), sigma=float(sigma), n_standard=len(std_l),
                   n_deep=len(deep_l), sha_standard=sha_stars(std_l) if std_l else None,
                   sha_deep=sha_stars(deep_l) if deep_l else None, deep_threshold=u, detect_s=t_det,
                   stars_deep=deep_l)
        # 標準の検出の星は、標準の検出で解いた試行があるときと、解けてはいけない画像のときだけ書く(大きいので)
        if not e["expect_solve"]:
            rec["stars_standard"] = std_l
        atts, scale, how = attempts_of(zc, b, deep, src, None if hint is None else hint[:2],
                                       5.0 if hint is None else hint[2], installed)
        rec["pixel_scale"] = scale
        rec["pixel_scale_how"] = how
        sets = {"deep": deep_l, "standard": std_l}
        runs = []
        solved = None
        for i, a in enumerate(atts, 1):
            st = np.asarray(sets[a["stars"]], dtype=np.float64).reshape(-1, 3)
            dl = args.deadline if e["expect_solve"] else NEG_DEADLINE
            sol, t, dt = run_traced(sc, st[:, :2], st[:, 2], e["width"], e["height"], catalog, a["scale_lo"],
                                    a["scale_hi"], None if a["hint"] is None else tuple(a["hint"]), dl)
            r1 = dict(a, attempt=i, status=t["status"], seconds=round(dt, 3), quads_tried=t["quads_tried"],
                      hypotheses=t["hypotheses"], logodds_max=t.get("logodds_max"))
            if sol is not None:
                r1["trace"] = summarize_trace(t, args.nhyp, sc, sw)
                tr = e.get("truth")
                if tr:
                    r1["vs_truth"] = measure_vs(ss, sw, sol.wcs, e["width"], e["height"], {
                        "center_ra": tr["center_ra"], "center_dec": tr["center_dec"],
                        "center_xyz": tr.get("center_xyz") or sw.sv_radec_to_xyz(tr["center_ra"],
                                                                                   tr["center_dec"]).tolist(),
                        "scale_arcsec": tr["scale_arcsec"], "pa_up_deg": tr["pa_up_deg"], "parity": tr["parity"]})
                if a["stars"] == "standard":
                    rec["stars_standard"] = std_l
            runs.append(r1)
            print(f"  {name:<20s} [{i}/{len(atts)}] {a['label']}: {t['status']} {dt:.2f} s"
                  + (f" {sol.index_name} lo={sol.logodds:.1f} nm={sol.nmatch}" if sol is not None else ""), flush=True)
            if sol is not None:
                solved = i
                break
            if not e["expect_solve"]:
                continue
        rec["attempts"] = runs
        rec["solved_attempt"] = solved
        # 速さの物差し(examples/bench_solver.rs)の条件: 基準の画像の深い検出の星で、ブラインド・ヒント・スケール既知。
        # 秒は参考値(この PC で sv_solve を 3 回回した最短。Rust 側は同じ星・同じ条件で測る)
        if base and e["tag"] == base["image"] and hint is None and e.get("truth"):
            s_true = float(e["truth"]["scale_arcsec"])
            conds = [("blind", None, None, None),
                     ("hint", None, None, (base["ra_deg"], base["dec_deg"], 5.0)),
                     ("scale", s_true * 0.75, s_true * 1.25, None)]
            bench = []
            st = np.asarray(deep_l, dtype=np.float64).reshape(-1, 3)
            for label, lo_, hi_, h_ in conds:
                best = math.inf
                last = None
                for _ in range(3):
                    sol, t, dt = run_traced(sc, st[:, :2], st[:, 2], e["width"], e["height"], catalog, lo_, hi_, h_,
                                            args.deadline)
                    best = min(best, dt)
                    last = (sol, t)
                sol, t = last
                bench.append(dict(label=label, stars="deep", scale_lo=lo_, scale_hi=hi_,
                                  hint=None if h_ is None else list(h_), python_seconds=round(best, 4),
                                  status=t["status"], index=None if sol is None else sol.index_name,
                                  logodds=None if sol is None else sol.logodds, quads_tried=t["quads_tried"],
                                  hypotheses=t["hypotheses"]))
                print(f"  bench {label}: {best:.3f} s {t['status']}", flush=True)
            rec["bench"] = bench
        # solve_offline そのもの(利用者の経路。ログの試行の行と、返したヘッダーの WCS)
        logbuf = io.StringIO()
        t1 = time.monotonic()
        with contextlib.redirect_stdout(logbuf):
            hdr = zc.solve_offline(b, src, _TMP, timeout=args.deadline if e["expect_solve"] else 20.0,
                                   focal_length_mm=None, hint_radec=None if hint is None else hint[:2],
                                   hint_radius_deg=5.0 if hint is None else hint[2], verbose=False,
                                   diagnostics=None, index_dir=args.index_dir, prep=(sub, mask, sigma))
        dt = time.monotonic() - t1
        lines = [ln.strip() for ln in logbuf.getvalue().splitlines() if ln.strip()]
        so = dict(seconds=round(dt, 3), solved=hdr is not None,
                  attempts=[ln for ln in lines if ln.startswith("[")],
                  success=[ln for ln in lines if ln.startswith("✅")])
        if hdr is not None:
            so["wcs_cards"] = {k: hdr[k] for k in hdr.keys() if k not in ("COMMENT", "HISTORY", "")}
            so["comment"] = [str(c) for c in hdr.get("COMMENT", [])]
        rec["solve_offline"] = so
        out["images"].append(rec)
    return out


# ------------------------------------------------------------------ 本体 ---

def main():
    try:
        sys.stdout.reconfigure(errors="replace", encoding="utf-8")
    except Exception:
        pass
    ap = argparse.ArgumentParser(description="zahyou-solver の照合用の値を作る")
    ap.add_argument("--dev", default=os.environ.get("ZAHYOU_DEV_RS", DEFAULT_DEV), help="基準実装の dev フォルダー")
    ap.add_argument("--index-dir", default=os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData"))
    ap.add_argument("--only", default="parts,synth,real", help="parts / synth / real(カンマ区切り)")
    ap.add_argument("--nhyp", type=int, default=24, help="書く仮説の数(最初から)")
    ap.add_argument("--deadline", type=float, default=600.0, help="sv_solve の締め切り [秒](止まらないよう長く)")
    ap.add_argument("--cases", help="合成星野の場合の名前(カンマ区切り。既定は代表)")
    ap.add_argument("--tags", help="実写の tag(カンマ区切り。既定は全部)")
    args = ap.parse_args()
    sw, si, sc, ss = setup(args.dev)
    catalog = si.SvIndexCatalog(args.index_dir) if os.path.isdir(args.index_dir) else None
    if catalog is not None and not catalog.files:
        catalog = None
    print(f"基準実装 {args.dev}、索引 {args.index_dir}({'無し' if catalog is None else len(catalog.files)} 本)",
          flush=True)
    only = set(args.only.split(","))
    t_all = time.perf_counter()
    if "parts" in only:
        write_json("solver_parts.json", gen_parts(sw, si, sc, ss, catalog))
    if catalog is None:
        print("索引が無いので solver_synth.json・solver_real.json は作りません", flush=True)
        return 0
    if "synth" in only:
        write_json("solver_synth.json", gen_synth(sw, si, sc, ss, catalog, args.dev, args))
    if "real" in only:
        write_json("solver_real.json", gen_real(sw, si, sc, ss, catalog, args.dev, args))
    print(f"終わり({time.perf_counter() - t_all:.0f} 秒)", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

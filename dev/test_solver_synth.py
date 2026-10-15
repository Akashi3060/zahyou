"""合成星野(索引の星から作った正解つきの星野)で、自前の解析エンジン sv_solve を確かめるテスト。

docs/design-native-solver.md の 8 節。星野は dev/solver_synth.py が作る(星の欠け 30 %・偽の星 30 %・
位置の雑音 0.3 px。どの 1 つの索引よりも星が多い)。合格の基準:
  中心のずれ 2″ 以内(0.5 px のほうが大きければそれ)、画素スケール 0.2 %、回転(配列の +y の位置角)0.1°、
  鏡像(配列座標の det CD の符号)の一致。一様乱数の星野は締め切りまで探して解なし(超過 1 秒以内)。

  python dev/test_solver_synth.py                  全部
  python dev/test_solver_synth.py --quick          代表 13 件(全部は 51 件)
  python dev/test_solver_synth.py --case s52*,ra0  名前で選ぶ(ワイルドカード可、カンマ区切り)
  --json 結果.json   --deadline 60(1 件の締め切り [秒])   --deadline-random 30(乱数の星野)
  --index-dir C:\\AstrometryData(環境変数 ZAHYOU_INDEX_DIR でも)   --dump-fields フォルダー(星野を JSON に)
  --trace フォルダー(sv_solve の trace)   --verbose(解法のログ)   --fake(sv_solve の代わりに正解を返す偽物)

最初の 5 項目は物差しと星野の作り方の自己検査で、sv_solve が無くても走る(00c は、どの星野にも
「画像の星だけで組めてコードが許容 0.01 以内の索引の quad」がある = 設計どおりの解法なら解ける形であることを
確かめる)。C:\\AstrometryData に無い段の場合は SKIP、dev/solver_core.py(sv_solve)がまだ無ければ解く項目は SKIP。
"""
import argparse
import copy
import fnmatch
import json
import math
import os
import sys
import time
import traceback
import warnings

warnings.filterwarnings("ignore")
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import solver_synth as ss      # noqa: E402
import solver_index as si      # noqa: E402
import solver_wcs as sw        # noqa: E402

DATA = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
RESULTS = []                   # (名前, True / False / None(SKIP), 説明, 記録)
ARGS = None

# 各段の quad の大きさ(AB の角距離、分角)。docs/index-format.md の 1 節
SERIES_ARCMIN = {
    5202: (4.0, 5.6), 5203: (5.6, 8.0), 5204: (8.0, 11.0), 5205: (11.0, 16.0), 5206: (16.0, 22.0),
    4107: (22.0, 30.0), 4108: (30.0, 42.0), 4109: (42.0, 60.0), 4110: (60.0, 85.0),
    4111: (85.0, 120.0), 4112: (120.0, 170.0), 4113: (170.0, 240.0), 4114: (240.0, 340.0),
    4115: (340.0, 480.0), 4116: (480.0, 680.0), 4117: (680.0, 1000.0), 4118: (1000.0, 1400.0),
    4119: (1400.0, 2000.0)}


# ------------------------------------------------------------------ 場合の一覧 ---

CASES = []


def case(name, what, center, scale, size=(968, 548), rot=0.0, mirror=False, sip=None, seed=None,
         n_stars=60, drop=0.3, distract=0.3, noise=0.3, hint=None, scale_known=False, only=None,
         random=False, quick=False):
    """1 件の場合。hint = (東へ [度], 北へ [度], 半径 [度]): 正解の中心からずらした点をヒントにする。"""
    CASES.append(dict(name=name, what=what, center=center, scale=float(scale), size=tuple(size),
                      rot=float(rot), mirror=bool(mirror), sip=sip,
                      seed=seed if seed is not None else 1000 + len(CASES),
                      n_stars=n_stars, drop=drop, distract=distract, noise=noise, hint=hint,
                      scale_known=bool(scale_known), only=only, random=bool(random), quick=bool(quick)))


def _gm(iid):
    lo, hi = SERIES_ARCMIN[iid]
    return math.sqrt(lo * hi)


# 段ごと: その段だけの目録で、スケール既知(±25 %)で解く。画像の短辺 = 2 × 段の quad の大きさ(幾何平均)。
# スケール不明にしないのは、sv_solve のスケール不明の範囲が v6 の blind_scale_bounds と同じく「手持ちの段の
# quad の大きさ ≒ 画像の横幅」から逆算するため(段 1 つだけの目録では、横幅がその段の大きさから外れる
# この星野は範囲の外になる)。スケール不明の全天ブラインドは、全部の目録の bb・w*deg で確かめる
_SPOTS = {5202: (83.82, -5.39), 5203: (10.68, 41.27), 5204: (201.30, -43.02), 5205: (61.89, 41.96),
          5206: (148.89, 69.07), 4107: (266.42, -29.01), 4108: (180.0, 0.0), 4109: (30.0, -75.0),
          4110: (330.0, 15.0), 4111: (56.75, 24.12), 4112: (210.0, 30.0), 4113: (100.0, -30.0),
          4114: (279.23, 38.78), 4115: (150.0, 60.0), 4116: (20.0, -20.0), 4117: (260.0, 10.0),
          4118: (80.0, 40.0), 4119: (225.0, 15.0)}
# 4119 は全天で 1,080 星・1,728 quad しかなく、短辺 24〜30° の視野で組める quad は位置しだいで 0〜5 個、
# 許容内(三角形 0.0071)は 0〜2 個(8 か所 × 3 通りで実測)。最初の (190, −50) は許容内が 0 個だったので、
# 許容内の quad の距離に余裕がある (225, 15) にした(30° で 14 番目の星まで、距離 0.0012)
for _iid in (5202, 5203, 5204, 5205, 5206, 4107, 4108, 4109, 4110):
    case(f"s{_iid}", f"段 {_iid} だけ(短辺 {2 * _gm(_iid):.1f}′)", _SPOTS[_iid],
         2.0 * _gm(_iid) * 60.0 / 548.0, rot=(37.0 * _iid) % 360.0, mirror=(_iid % 2 == 1),
         only=[_iid], scale_known=True, quick=_iid in (5205, 4108))
# 短辺は 30° で頭打ち(4118・4119)。普通のレンズ(TAN)の視野がそれより広いと、視野の端の quad が投影で
# ゆがみ、画素で測ったコードが索引のコード(quad の中心で投影)から許容より離れる。実測(位置 (190, −50)、
# 許容 0.01): 段 4119 は短辺 28° で組める quad 1 個のうち許容内 1 個、33° で 6 個のうち 0 個、40° で 1 個のうち
# 0 個(33° と 40° は解けない)。段 4118 は 25〜30° で許容内 9〜10 個、39.4° では 5 個で、種しだいで解けたり
# 解けなかったりした
for _iid in range(4111, 4120):
    _short = min(2.0 * _gm(_iid), 30.0 * 60.0)            # 分角
    case(f"t{_iid}", f"三角形の段 {_iid} だけ(短辺 {_short / 60:.1f}°)", _SPOTS[_iid],
         _short * 60.0 / 800.0, size=(1200, 800), rot=(53.0 * _iid) % 360.0, mirror=(_iid % 2 == 0),
         only=[_iid], scale_known=True, quick=_iid == 4113)

# 広い視野(全部の目録で、スケール不明・ヒントなし)
for _deg, _spot, _q in ((2, (300.0, 35.0), False), (5, (120.0, -10.0), False),
                        (10, (45.0, 60.0), True), (25, (200.0, -35.0), False)):
    case(f"w{_deg}deg", f"横 {_deg}° の広い視野", _spot, _deg * 3600.0 / 1200.0, size=(1200, 800),
         rot=17.0 * _deg, quick=_q)

# 回転・鏡像(実写と同じカメラ 968×548・1.4616″/px、スケール既知)
for _rot, _mir, _q in ((0.0, False, False), (90.0, False, False), (211.3, False, True),
                       (0.0, True, False), (152.87, True, True), (300.0, True, False)):
    case(f"{'mir' if _mir else 'rot'}{int(_rot):03d}", f"+y の位置角 {_rot}°・{'鏡像' if _mir else '鏡像なし'}",
         (95.0 + _rot, 20.0 - _rot / 10.0), 1.4616, rot=_rot, mirror=_mir, scale_known=True, quick=_q)

# 位置: タイルの境界・赤経 0°・極・南天
case("tile_edge", "5200 系のタイルの辺の上(2 枚にまたがる)",
     tuple(float(v) for v in sw.sv_xyz_to_radec(si.sv_healpix_xy_point(17, 2, 0.5, 0.0))), 2.0, rot=33.0)
case("tile_corner", "5200 系のタイルの角の上(3〜4 枚にまたがる)",
     tuple(float(v) for v in sw.sv_xyz_to_radec(si.sv_healpix_xy_point(5, 2, 0.0, 0.0))), 2.0,
     rot=128.0, mirror=True)
case("ra0", "赤経 0° をまたぐ", (0.0, 25.0), 1.4616, rot=250.0)
case("pole89", "極付近(赤緯 89°)", (123.0, 89.0), 1.4616, rot=45.0, quick=True)
case("polein", "視野に天の北極が入る(赤緯 89.95°)", (77.0, 89.95), 1.4616, rot=300.0, mirror=True)
case("south60", "南天(赤緯 −60°)", (250.0, -60.0), 1.4616, rot=190.0, quick=True)

# ヒントの有無 × スケールの既知・不明(同じ星野)
_F = dict(center=(300.0, 35.0), scale=1.4616, rot=70.0)
case("hs", "ヒントあり(0.36° ずれ・半径 1°)+ スケール既知", hint=(0.3, -0.2, 1.0), scale_known=True,
     quick=True, seed=2001, **_F)
case("hb", "ヒントあり + スケール不明", hint=(0.3, -0.2, 1.0), seed=2001, **_F)
case("bs", "ヒントなし + スケール既知", scale_known=True, seed=2001, **_F)
case("bb", "ヒントなし + スケール不明(全天ブラインド)", quick=True, seed=2001, **_F)

# 歪み(SIP)・星の数
case("sip_pin", "糸巻き型の歪み(隅で +3 px)", (4.28, 38.70), 2.3188, size=(1608, 1104), rot=49.14,
     sip=3.0, scale_known=True)
case("sip_bar", "樽型の歪み(隅で −4 px)+ 鏡像", (156.64, 41.93), 0.7474, size=(2704, 1536), rot=87.45,
     mirror=True, sip=-4.0, scale_known=True)
case("few20", "星 20 個だけ", (61.89, 41.96), 1.4616, rot=152.87, mirror=True, n_stars=20)
case("many300", "星 300 個", (300.0, 35.0), 2.0, size=(1608, 1104), rot=10.0, n_stars=300)

# 一様乱数の星野(解けてはいけない)
case("rand60", "一様乱数の星 60 個(ブラインド)", None, 1.4616, random=True, quick=True)
case("rand300", "一様乱数の星 300 個(ブラインド)", None, 2.3188, size=(1608, 1104), n_stars=300, random=True)
case("rand_hint", "一様乱数の星 60 個 + ヒント + スケール既知", (61.89, 41.96), 1.4616, n_stars=60,
     hint=(0.0, 0.0, 5.0), scale_known=True, random=True)

# 位置の場合と同じ星野(種も同じ)にヒントを与えた版。位置の場合のブラインドは全天探索の速さにも左右される
# (Python 版では tile_corner・ra0・polein が 60 秒に収まらないことがある)ので、位置の扱い(タイルの選び方・
# 赤経 0°・極)はこちらで速さと切り離して確かめる。種は一覧の順で決まるので、前の場合を変えないよう最後に置く
for _n, _q in (("tile_edge", False), ("tile_corner", True), ("ra0", True), ("pole89", False),
               ("polein", False), ("south60", False)):
    _b = next(c for c in CASES if c["name"] == _n)
    case(f"{_n}_h", _b["what"] + " + ヒント(0.36° ずれ・半径 1°)", _b["center"], _b["scale"], size=_b["size"],
         rot=_b["rot"], mirror=_b["mirror"], seed=_b["seed"], hint=(0.3, -0.2, 1.0), quick=_q)


# ------------------------------------------------------------------ 道具 ---

def restrict(catalog, ids):
    """目録のうち系列 ids だけを見せる写し(ファイルは共有)。"""
    c = copy.copy(catalog)
    c.series = {i: catalog.series[i] for i in ids if i in catalog.series}
    c.files = [f for f in catalog.files if f.indexid in c.series]
    return c


def needed(c, catalog):
    """この場合に要る段がそろっているか。(ok, 足りない説明)。"""
    have = set(catalog.series) if catalog is not None else set()
    if c["only"]:
        miss = [i for i in c["only"] if i not in have]
        return (not miss), f"段 {', '.join(map(str, miss))} が {DATA} に無い"
    if c["random"]:
        return bool(have), f"{DATA} に索引が無い"
    W, H = c["size"]
    qlo = 0.1 * min(W, H) * c["scale"] / 60.0
    qhi = math.hypot(W, H) * c["scale"] / 60.0
    fit = [i for i, (lo, hi) in SERIES_ARCMIN.items() if lo <= qhi and hi >= qlo]
    return any(i in have for i in fit), f"画角に合う段({min(fit)}〜{max(fit)})が {DATA} に無い"


def match_series(c):
    """この場合を解くのに使うはずの段: only の先頭か、quad の大きさが短辺の半分に最も近い段。"""
    if c["only"]:
        return c["only"][0]
    W, H = c["size"]
    target = 0.5 * min(W, H) * c["scale"] / 60.0
    return min(SERIES_ARCMIN, key=lambda i: abs(math.log(_gm(i) / target)))


def make_field(c, catalog):
    W, H = c["size"]
    if c["random"]:
        return ss.sv_synth_random_field(W, H, n_stars=c["n_stars"], seed=c["seed"])
    return ss.sv_synth_field(catalog, c["center"][0], c["center"][1], c["scale"], W, H,
                             rotation_deg=c["rot"], mirror=c["mirror"], sip=c["sip"], seed=c["seed"],
                             n_stars=c["n_stars"], drop_frac=c["drop"], distractor_frac=c["distract"],
                             pos_noise_px=c["noise"])


def hint_of(c):
    """sv_solve に渡すヒント (赤経, 赤緯, 半径)(度)か None。"""
    if c["hint"] is None:
        return None
    de, dn, rad = c["hint"]
    ctr = sw.sv_radec_to_xyz(*c["center"])
    p = sw.sv_tan_deproject(math.radians(de), math.radians(dn), ctr)
    ra, dec = sw.sv_xyz_to_radec(p)
    return (float(ra), float(dec), float(rad))


def scale_range(c):
    return (0.75 * c["scale"], 1.25 * c["scale"]) if c["scale_known"] else (None, None)


def evaluate(field, wcs):
    """解の WCS を正解と比べる。sv_synth_compare の結果に「測った値」を足して返す。"""
    W, H = field["width"], field["height"]
    want = ss.sv_synth_measure(field["truth"], W, H)
    got = ss.sv_synth_measure(wcs, W, H, ref_xyz=want["center_xyz"])
    cmp = ss.sv_synth_compare(got, want)
    cmp["got"] = got
    cmp["want"] = want
    return cmp


def describe(cmp):
    return (f"中心 {cmp['center_arcsec']:.3f}″(許容 {cmp['center_tol_arcsec']:.2f}″)・"
            f"スケール {cmp['scale_rel'] * 100:+.4f}%・回転 {cmp['rot_deg']:+.4f}°・"
            f"パリティ {cmp['parity'][0]:+d}/{cmp['parity'][1]:+d}")


def record(name, ok, msg, rec=None):
    RESULTS.append((name, ok, msg, rec or {}))
    tag = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
    print(f"  {tag}  {name}  {msg}", flush=True)


def astropy_fn(wcs_astropy):
    """astropy の WCS を「画素(0 始まり)→ 単位ベクトル」の関数に包む。"""
    def fn(x, y):
        ra, dec = wcs_astropy.all_pix2world(np.asarray(x, float), np.asarray(y, float), 0)
        return sw.sv_radec_to_xyz(ra, dec)
    return fn


def header_of(wcs, W, H):
    from astropy.io import fits
    h = fits.Header()
    for k, v, cm in wcs.to_header_cards(W, H):
        if k == "COMMENT":
            h.add_comment(str(v))
        else:
            h[k] = (v, cm)
    return h


# ------------------------------------------------------------------ 自己検査 ---

def selfcheck_generator(fields, catalog):
    """00: 星野の作り方の約束(正解の WCS が引数どおり・星の位置・種で決まる・JSON の往復)。"""
    bad = []
    n_sky = 0
    worst = {"center": 0.0, "scale": 0.0, "rot": 0.0, "star": 0.0}
    for c, f in fields:
        if c["random"]:
            continue
        n_sky += 1
        t = f["meta"]["truth_summary"]
        sep = float(sw.sv_angle_between(np.asarray(t["center_xyz"]),
                                        sw.sv_radec_to_xyz(*c["center"]))) * sw.SV_ARCSEC_PER_RAD
        srel = abs(t["scale_arcsec"] / c["scale"] - 1.0)
        drot = abs(ss.sv_synth_angle_diff(t["pa_up_deg"], c["rot"]))
        worst["center"] = max(worst["center"], sep)
        worst["scale"] = max(worst["scale"], srel)
        worst["rot"] = max(worst["rot"], drot)
        # 中心は CRPIX そのもの。スケールと回転は ±1 px の差分なので、SIP の 3 次の項の分(k·1²)だけずれる
        if sep > 1e-6 or srel > 1e-7 or drot > 1e-6 or t["parity"] != (1 if c["mirror"] else -1):
            bad.append(f"{c['name']}: 正解 {t} が引数と違う")
        # 本物の星は正解の WCS で投影した位置から雑音の 6σ 以内
        real = np.array(f["meta"]["is_real"], dtype=bool)
        if real.any():
            xyz = np.array([v for v in f["meta"]["star_xyz"] if v is not None])
            x, y, ok = f["truth"].xyz_to_pix(xyz, exact=True)
            d = np.hypot(f["xy"][real, 0] - x, f["xy"][real, 1] - y)
            worst["star"] = max(worst["star"], float(d.max()) / max(c["noise"], 1e-9))
            if not ok.all() or d.max() > 6.0 * c["noise"] + 1e-6:
                bad.append(f"{c['name']}: 本物の星が正解の位置から {d.max():.2f} px")
        if len(f["xy"]) and not np.all(np.diff(f["flux"]) <= 0):
            bad.append(f"{c['name']}: 明るい順に並んでいない")
    # 同じ種で同じ星野、違う種で違う星野、JSON の往復で同じ値
    c0 = next(c for c, _ in fields if c["name"] == "sip_pin") if any(
        c["name"] == "sip_pin" for c, _ in fields) else fields[0][0]
    f1, f2 = make_field(c0, catalog), make_field(c0, catalog)
    c3 = dict(c0, seed=c0["seed"] + 1)
    f3 = make_field(c3, catalog)
    if not (np.array_equal(f1["xy"], f2["xy"]) and np.array_equal(f1["flux"], f2["flux"])
            and json.dumps(ss.sv_synth_to_json(f1)) == json.dumps(ss.sv_synth_to_json(f2))):
        bad.append(f"{c0['name']}: 同じ種で違う星野になった")
    if np.array_equal(f1["xy"], f3["xy"]):
        bad.append(f"{c0['name']}: 種を変えても同じ星野")
    for c, f in fields[:: max(1, len(fields) // 8)] + [(c0, f1)]:
        g = ss.sv_synth_from_json(json.loads(json.dumps(ss.sv_synth_to_json(f))))
        if not (np.array_equal(g["xy"], f["xy"]) and np.array_equal(g["flux"], f["flux"])):
            bad.append(f"{c['name']}: JSON の往復で星が変わった")
        if f["truth"] is not None:
            px = np.array([[0.0, 0.0], [f["width"] - 1.0, f["height"] - 1.0], [123.4, 56.7]])
            a = f["truth"].pix_to_xyz(px[:, 0], px[:, 1])
            b = g["truth"].pix_to_xyz(px[:, 0], px[:, 1])
            if not np.array_equal(a, b):
                bad.append(f"{c['name']}: JSON の往復で正解の WCS が変わった")
    msg = (f"空の星野 {n_sky} 件: 正解の中心 ≤ {worst['center']:.1e}″・スケール ≤ {worst['scale']:.1e}・"
           f"回転 ≤ {worst['rot']:.1e}°、本物の星は ≤ {worst['star']:.1f}σ、種で決まり JSON で往復する")
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def selfcheck_contents(fields):
    """00b: 星の数・偽の星の割合・どの 1 つの索引よりも多い・境界をまたぐ場合が本当にまたいでいる。"""
    bad, notes = [], []
    fr, ratio, same = [], [], []
    for c, f in fields:
        if c["random"]:
            continue
        m = f["meta"]
        cnt = m["counts"]
        if cnt["out"] < min(c["n_stars"], 12):
            bad.append(f"{c['name']}: 星が {cnt['out']} 個しかない")
        if cnt["out"]:
            fr.append(cnt["out_distractors"] / cnt["out"])
        # 解くのに使うはずの段(only か、quad の大きさが短辺の半分に近い段)より星が多いか。
        # 段どうしはほぼ入れ子(細かい段が粗い段の星をほぼ全部含む)なので、集めた星の数は
        # いちばん細かい段とほぼ同じになる。いちばん細かい段で解く場合(s5202)は同じ数にしかならない
        mid = match_series(c)
        n_mid = m["series"].get(str(mid), 0)
        finest = max(m["series"].values()) if m["series"] else 0
        if n_mid < finest:
            if not cnt["catalog_stars"] > n_mid:
                bad.append(f"{c['name']}: 集めた星 {cnt['catalog_stars']} が段 {mid}({n_mid})より多くない")
            ratio.append(cnt["catalog_stars"] / max(n_mid, 1))
        else:
            same.append(c["name"])
        src = [s for s in m["source"] if s]
        if c["name"].startswith("tile_"):
            # 5200 系の各系列で、視野の星が何枚のタイルから来たか(いちばん多い系列の枚数)
            tiles = {}
            for s in gathered_sources(c, f):
                fname = s.split("#")[0]                    # 例 index-5205-02
                tiles.setdefault(fname[:10], set()).add(fname)
            n = max((len(v) for k, v in tiles.items() if k.startswith("index-52")), default=0)
            notes.append(f"{c['name']} は {n} 枚にまたがる")
            if n < 2:
                bad.append(f"{c['name']}: タイル 1 枚の星しか無い")
        if c["name"] == "ra0":
            ra = [sw.sv_xyz_to_radec(np.array(v))[0] for v in m["star_xyz"] if v is not None]
            if not (any(r < 1.0 for r in ra) and any(r > 359.0 for r in ra)):
                bad.append("ra0: 星が赤経 0° の両側に無い")
        if c["name"] == "polein":
            x, y, ok = f["truth"].radec_to_pix(0.0, 90.0, exact=True)
            W, H = c["size"]
            if not (bool(ok) and -0.5 <= float(x) <= W - 0.5 and -0.5 <= float(y) <= H - 0.5):
                bad.append("polein: 極が視野に入っていない")
            else:
                notes.append(f"polein 極は ({float(x):.0f}, {float(y):.0f}) px")
        if not src:
            bad.append(f"{c['name']}: 本物の星が 1 つも無い")
    if not fr:                                         # 乱数の星野だけを選んだとき
        return True, "空の星野が無い(乱数の星野だけ)"
    msg = (f"偽の星の割合 {min(fr):.2f}〜{max(fr):.2f}(平均 {np.mean(fr):.2f})、"
           + (f"集めた星は解く段の {min(ratio):.1f}〜{max(ratio):.0f} 倍" if ratio else "")
           + (f"(いちばん細かい段で解く {', '.join(same)} は同じ数)" if same else "")
           + (f"。{'、'.join(notes)}" if notes else ""))
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


_GATHERED = {}


def gathered_sources(c, f):
    """星野を作るときに視野で見つかった星の出どころ(落とす前の全部)。meta の source は出力した星だけなので
    タイルの数え方には足りない。同じ引数で集め直す。"""
    if c["name"] not in _GATHERED:
        cat = _CATALOG[0]
        W, H = c["size"]
        truth = f["truth"]
        series = ss.sv_synth_series_choice(cat, c["scale"], W, H)
        _, _, _, source, _ = ss._sv_synth_gather(cat, series, truth, W, H, np.random.default_rng(0))
        _GATHERED[c["name"]] = source
    return _GATHERED[c["name"]]


_CATALOG = [None]
CODETOL = 0.01                 # 設計書 3.1 の SvParams.codetol の既定(三角形は下の codetol_for で縮める)
QUAD_STARS = 60                # 設計書 3.1 の SvParams.max_quad_stars の既定
WELLPOSED = {}                 # 名前 → wellposed() の結果(解けなかったときの説明に使う)


def codetol_for(dimquads):
    """照合の許容。4 星は CODETOL。三角形は CODETOL × √(2/4)(solver_core の _sv_match_file と同じ。同じ位置の
    ずれでコードの L2 が √(D/4) 倍になるため。設計書 3.4 は三角形も codetol のままで、縮めるとは書いていない)。
    小さいほうで数えれば、どちらの許容でも当たる。"""
    return CODETOL * (math.sqrt(0.5) if int(dimquads) == 3 else 1.0)


def wellposed(c, f, catalog):
    """この星野が「設計どおりの解法なら解ける形」か。

    画角に合う段(quad の大きさが [0.1 × 短辺, 対角] に重なる段。only があればそれだけ)の索引の quad のうち、
    4 星(三角形は 3 星)が全部「出力した本物の星」にあり(1″ 以内)、画素で計算したコードと索引のコードの
    距離が codetol_for 以内のもの(= 照合で当たるはずの quad)を数える。コードは索引の星の順 A, B, C, D のまま
    計算するので正準化は要らない。鏡像(det CD > 0)の星野は x と y を入れ替えて計算する(設計書 3.4)。
    戻り値 {"quads": 組める数, "good": 許容内の数, "good_top": 許容内で星が全部明るい順 QUAD_STARS 番以内の数,
    "first": 許容内で最も早く組める quad の (最後の星の順位, 距離, 索引名), "dist_max": 組める quad の距離の最大}。
    """
    W, H = f["width"], f["height"]
    truth = f["truth"]
    real = np.array(f["meta"]["is_real"], dtype=bool)
    ridx = np.nonzero(real)[0]
    rxyz = np.array([f["meta"]["star_xyz"][k] for k in ridx], dtype=np.float64).reshape(-1, 3)
    center = truth.pix_to_xyz((W - 1) / 2.0, (H - 1) / 2.0)
    rad = ss._sv_synth_field_radius(truth, W, H)
    s = truth.pixel_scale_arcsec()
    qlo, qhi = 0.1 * min(W, H) * s, math.hypot(W, H) * s          # ″
    series = c["only"] or [i for i, fs in sorted(catalog.series.items())
                           if fs[0].scale_lo_arcsec <= qhi and fs[0].scale_hi_arcsec >= qlo]
    swap = truth.parity() > 0
    tol = math.cos(1.0 / sw.SV_ARCSEC_PER_RAD)
    out = {"quads": 0, "good": 0, "good_top": 0, "first": None, "dist_max": 0.0, "series": series}
    for iid in series:
        for fi in catalog.tiles_near(iid, center, rad):
            sid = fi.stars_in_cone(center, rad)
            if len(sid) == 0 or len(ridx) == 0:
                continue
            dots = rxyz @ fi.star_xyz(sid).T                  # (本物の星, 索引の星)
            j = np.argmax(dots, axis=1)
            hit = dots[np.arange(len(ridx)), j] >= tol
            lookup = np.full(fi.nstars, -1, dtype=np.int64)
            lookup[sid[j[hit]]] = ridx[hit]
            q = fi.quad_stars(np.arange(fi.nquads))
            qi = np.nonzero((lookup[q] >= 0).all(axis=1))[0]
            if len(qi) == 0:
                continue
            ranks = lookup[q[qi]]                              # (n, dimquads) 出力の星の番号
            p = f["xy"][ranks]
            if swap:
                p = p[:, :, ::-1]
            z = p[:, :, 0] + 1j * p[:, :, 1]
            w = (z[:, 2:] - z[:, :1]) / (z[:, 1:2] - z[:, :1]) * (1 + 1j)
            mine = np.stack([w.real, w.imag], axis=-1).reshape(len(qi), -1)
            dist = np.linalg.norm(mine - fi.codes(qi), axis=1)
            last = ranks.max(axis=1)
            good = dist <= codetol_for(fi.dimquads)
            out["quads"] += int(len(qi))
            out["good"] += int(good.sum())
            out["good_top"] += int((good & (last < QUAD_STARS)).sum())
            out["dist_max"] = max(out["dist_max"], float(dist.max()))
            for k in np.nonzero(good)[0]:
                cand = (int(last[k]), round(float(dist[k]), 4), fi.name)
                if out["first"] is None or cand < out["first"]:
                    out["first"] = cand
    return out


def selfcheck_wellposed(fields, catalog):
    """00c: どの空の星野にも「照合で当たるはずの quad」が 1 つ以上ある(解けないのは解法のせいだと言える)。"""
    bad, rows = [], []
    for c, f in fields:
        if c["random"]:
            continue
        r = wellposed(c, f, catalog)
        WELLPOSED[c["name"]] = r
        rows.append((r["good_top"], c["name"], r))
        if r["good_top"] == 0:
            bad.append(f"{c['name']}: 照合で当たるはずの quad が無い(組める {r['quads']} 個、許容内 {r['good']} 個、"
                       f"距離の最大 {r['dist_max']:.4f})")
    if not rows:
        return True, "空の星野が無い(乱数の星野だけ)"
    rows.sort(key=lambda t: (t[0], t[1]))
    few = "、".join(f"{n} {g} 個(最初は {r['first'][0] + 1} 番目の星まで)" for g, n, r in rows[:4] if r["first"])
    msg = (f"空の星野 {len(rows)} 件とも、明るい順 {QUAD_STARS} 個で組めてコードが許容({CODETOL}、三角形は "
           f"{codetol_for(3):.4f})以内の索引の quad がある。少ないもの: {few}")
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def selfcheck_astropy(fields):
    """01: 物差しを astropy で確かめる(ヘッダーにした正解を astropy で読み、同じ物差しで測る。
    位置角は zahyou の wcs_summary と同じ「中心 → +y へ 10 px の点」の position_angle とも比べる)。"""
    from astropy.wcs import WCS
    from astropy.coordinates import SkyCoord
    import astropy.units as u
    bad = []
    worst = {"center": 0.0, "scale": 0.0, "rot": 0.0, "pa10": 0.0}
    for c, f in fields:
        if c["random"]:
            continue
        W, H = f["width"], f["height"]
        w = WCS(header_of(f["truth"], W, H))
        mine = ss.sv_synth_measure(f["truth"], W, H)
        theirs = ss.sv_synth_measure(astropy_fn(w), W, H)
        cmp = ss.sv_synth_compare(theirs, mine, tol={"center_arcsec": 1e-3, "center_px": 0.0,
                                                      "scale_rel": 1e-7, "rot_deg": 1e-5})
        worst["center"] = max(worst["center"], cmp["center_arcsec"])
        worst["scale"] = max(worst["scale"], abs(cmp["scale_rel"]))
        worst["rot"] = max(worst["rot"], abs(cmp["rot_deg"]))
        if not cmp["ok"]:
            bad.append(f"{c['name']}: astropy と違う {cmp['failed']} {describe(cmp)}")
        cx, cy = (W - 1) / 2.0, (H - 1) / 2.0
        r0, d0 = w.all_pix2world(cx, cy, 0)
        r1, d1 = w.all_pix2world(cx, cy + 10.0, 0)
        pa10 = SkyCoord(r0 * u.deg, d0 * u.deg).position_angle(SkyCoord(r1 * u.deg, d1 * u.deg)).deg
        dpa = abs(ss.sv_synth_angle_diff(pa10, mine["pa_up_deg"]))
        # TAN は画素の直線を大円に写すので、10 px 先の点への方位は中心での向きと同じ(極のそばでも)。
        # SIP があると直線が曲がる分だけ違う
        allow = 0.01 if c["sip"] else 2e-4
        worst["pa10"] = max(worst["pa10"], dpa)
        if dpa > allow:
            bad.append(f"{c['name']}: 10 px 先の位置角 {pa10:.5f}° と {mine['pa_up_deg']:.5f}° が違う")
        det = float(np.linalg.det(w.pixel_scale_matrix))
        if (1 if det > 0 else -1) != mine["parity"]:
            bad.append(f"{c['name']}: astropy の det CD の符号と違う")
    msg = (f"astropy との差: 中心 ≤ {worst['center']:.1e}″・スケール ≤ {worst['scale']:.1e}・"
           f"回転 ≤ {worst['rot']:.1e}°、10 px 先の位置角との差 ≤ {worst['pa10']:.4f}°、det CD の符号も一致")
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def perturbed(truth, W, H, kind, k):
    """正解をわざとずらした WCS。kind = center / scale / rot / parity、k = 許容の何倍ずらすか。"""
    t = ss.SV_SYNTH_TOL
    s = truth.pixel_scale_arcsec()
    crpix, cd = truth.crpix.copy(), truth.cd.copy()
    if kind == "center":
        d = k * max(t["center_arcsec"], t["center_px"] * s) / s       # 画素
        crpix = crpix - np.array([d, d]) / math.sqrt(2.0)
    elif kind == "scale":
        cd = cd * (1.0 + k * t["scale_rel"])
    elif kind == "rot":
        e = math.radians(k * t["rot_deg"])
        cd = np.array([[math.cos(e), math.sin(e)], [-math.sin(e), math.cos(e)]]) @ cd
    elif kind == "parity":
        cd[:, 0] = -cd[:, 0]
    return sw.SvWcs(truth.crval_xyz, crpix, cd, truth.a, truth.b, truth.ap, truth.bp)


def selfcheck_fake(fields):
    """02: 偽のソルバーで物差しを確かめる。正解そのまま → 全部 PASS、許容の半分ずらす → PASS、
    1.5〜2 倍ずらす → その項目だけ FAIL。乱数の星野に解を返したら FAIL になること。"""
    bad = []
    n = 0
    for c, f in fields:
        if c["random"]:
            continue
        W, H = f["width"], f["height"]
        tr = f["truth"]
        cmp = evaluate(f, tr)
        if not cmp["ok"] or cmp["center_arcsec"] > 1e-6 or abs(cmp["rot_deg"]) > 1e-6:
            bad.append(f"{c['name']}: 正解そのままが通らない {describe(cmp)}")
        for kind, k, expect in (("center", 0.5, []), ("scale", 0.5, []), ("rot", 0.5, []),
                                ("center", 1.5, ["center"]), ("scale", 2.0, ["scale"]),
                                ("scale", -2.0, ["scale"]), ("rot", 2.0, ["rot"]),
                                ("rot", -2.0, ["rot"]), ("parity", 1, ["parity"])):
            cmp = evaluate(f, perturbed(tr, W, H, kind, k))
            n += 1
            if cmp["failed"] != expect:
                bad.append(f"{c['name']}: {kind}×{k} で {cmp['failed']}(期待 {expect}) {describe(cmp)}")
    # 乱数の星野: 解なしなら PASS、解を返したら・締め切りを 1 秒超えたら FAIL(判定の関数そのもの)
    ok_none, _ = judge_random(None, 1.0, 10.0)
    ok_some, _ = judge_random(object(), 1.0, 10.0)
    late, _ = judge_random(None, 11.5, 10.0)
    if not ok_none or ok_some or late:
        bad.append("乱数の星野の判定がおかしい(解なし・解あり・締め切り超過)")
    msg = f"正解そのまま → PASS、ずらした {n} 通り → 期待どおりの項目だけ FAIL(極のそばも含む)"
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def judge_random(sol, elapsed, deadline_s):
    over = elapsed - deadline_s
    if sol is not None:
        return False, "解けてはいけないのに解が出た"
    if over > 1.0:
        return False, f"締め切りを {over:.1f} 秒超えた"
    return True, f"解なし(締め切りとの差 {over:+.2f} 秒)"


# ------------------------------------------------------------------ 解く ---

def load_solver():
    """(sv_solve, 説明)。無ければ (None, 理由)。"""
    if ARGS.fake:
        return fake_solve, "偽のソルバー(正解をそのまま返す)"
    try:
        import solver_core as sc
    except ModuleNotFoundError as e:
        if e.name in ("solver_core",):
            return None, "dev/solver_core.py がまだ無い"
        return None, f"solver_core を読めない: {type(e).__name__}: {e}"
    except Exception as e:
        return None, f"solver_core を読めない: {type(e).__name__}: {e}"
    if not hasattr(sc, "sv_solve"):
        return None, "solver_core に sv_solve がまだ無い"
    return sc.sv_solve, "solver_core.sv_solve"


_FAKE_TRUTH = {}


class _FakeSolution:
    def __init__(self, wcs):
        self.wcs = wcs
        self.index_name = "(偽物)"
        self.logodds = float("inf")
        self.nmatch = 0
        self.quads_tried = 0


def fake_solve(xy, flux, width, height, catalog, **kw):
    t = _FAKE_TRUTH.get("truth")
    return None if t is None else _FakeSolution(t)


def run_solve(c, f, catalog, solver):
    W, H = c["size"]
    cat = restrict(catalog, c["only"]) if c["only"] else catalog
    s_lo, s_hi = scale_range(c)
    deadline_s = ARGS.deadline_random if c["random"] else ARGS.deadline
    kw = dict(scale_lo=s_lo, scale_hi=s_hi, hint=hint_of(c))
    if ARGS.verbose:
        kw["log"] = lambda m: print(f"      · {m}", flush=True)
    if ARGS.trace:
        os.makedirs(ARGS.trace, exist_ok=True)
        kw["trace"] = os.path.join(ARGS.trace, f"{c['name']}.json")
    _FAKE_TRUTH["truth"] = f["truth"]
    t0 = time.monotonic()
    kw["deadline"] = t0 + deadline_s
    try:
        sol = solver(np.array(f["xy"]), np.array(f["flux"]), W, H, cat, **kw)
        err = None
    except Exception as e:
        sol, err = None, f"{type(e).__name__}: {e}\n{traceback.format_exc()}"
    dt = time.monotonic() - t0
    rec = {"name": c["name"], "what": c["what"], "seconds": round(dt, 3), "deadline_s": deadline_s,
           "n_stars": int(len(f["xy"])), "random": c["random"], "only": c["only"],
           "hint": kw["hint"], "scale_lo": s_lo, "scale_hi": s_hi}
    wp = WELLPOSED.get(c["name"])
    if wp:
        rec["wellposed"] = {k: wp[k] for k in ("quads", "good", "good_top", "first")}
    info = ""
    if sol is not None:
        for a in ("index_name", "logodds", "nmatch", "ndistract", "nfield", "nref", "quads_tried",
                  "hypotheses", "seconds"):
            v = getattr(sol, a, None)
            if isinstance(v, (int, float, str, np.integer, np.floating)):
                rec[f"sol_{a}"] = v.item() if hasattr(v, "item") else v
        lo = rec.get("sol_logodds")
        info = (f"  [{rec.get('sol_index_name', '?')}・一致 {rec.get('sol_nmatch', '?')}・"
                f"対数オッズ {lo:.1f}・quad {rec.get('sol_quads_tried', '?')}]"
                if isinstance(lo, (int, float)) and math.isfinite(lo) else
                f"  [{rec.get('sol_index_name', '?')}]")
    if err:
        return False, f"例外 {err}", rec
    if c["random"]:
        ok, msg = judge_random(sol, dt, deadline_s)
        rec["over_s"] = round(dt - deadline_s, 3)
        return ok, f"{msg}  {dt:.1f} 秒{info}", rec
    if sol is None:
        why = (f"。当たるはずの quad {wp['good_top']} 個、最初は {wp['first'][0] + 1} 番目の星まで"
               f"({wp['first'][2]}、距離 {wp['first'][1]})" if wp and wp["first"] else "")
        return False, f"解けなかった({dt:.1f} 秒、締め切り {deadline_s:g} 秒){why}", rec
    try:
        cmp = evaluate(f, sol.wcs)
    except Exception as e:
        return False, f"解の WCS を測れない: {type(e).__name__}: {e}", rec
    rec.update({k: cmp[k] for k in ("ok", "failed", "center_arcsec", "center_tol_arcsec", "scale_rel",
                                      "rot_deg", "parity")})
    msg = describe(cmp) + f"  {dt:.1f} 秒{info}"
    if cmp["failed"]:
        msg = f"外れ {cmp['failed']}: " + msg
    return cmp["ok"], msg, rec


# ------------------------------------------------------------------ 本体 ---

def pick_cases():
    sel = CASES
    if ARGS.quick:
        sel = [c for c in sel if c["quick"]]
    if ARGS.case:
        pats = [p.strip() for p in ARGS.case.split(",") if p.strip()]
        sel = [c for c in CASES if any(fnmatch.fnmatch(c["name"], p) for p in pats)]
        if not sel:
            raise SystemExit(f"--case {ARGS.case} に合う場合がありません。名前: "
                             + ", ".join(c["name"] for c in CASES))
    return sel


def main():
    global ARGS, DATA
    try:
        sys.stdout.reconfigure(errors="replace")    # cp932 の画面でも止まらないように
    except Exception:
        pass
    ap = argparse.ArgumentParser(description="合成星野で sv_solve を確かめる")
    ap.add_argument("--quick", action="store_true", help="代表の場合だけ")
    ap.add_argument("--case", help="場合の名前(ワイルドカード可、カンマ区切り)")
    ap.add_argument("--json", help="結果を JSON に書く")
    ap.add_argument("--deadline", type=float, default=60.0, help="1 件の締め切り [秒](既定 60)")
    ap.add_argument("--deadline-random", type=float, default=30.0, help="乱数の星野の締め切り [秒](既定 30)")
    ap.add_argument("--index-dir", default=None, help="索引の置き場所(既定 C:\\AstrometryData)")
    ap.add_argument("--dump-fields", help="星野を JSON でこのフォルダーに書く(移植の試験の入力)")
    ap.add_argument("--trace", help="sv_solve の trace をこのフォルダーに書く")
    ap.add_argument("--verbose", action="store_true", help="解法のログを出す")
    ap.add_argument("--fake", action="store_true", help="sv_solve の代わりに正解を返す偽物で通す")
    ARGS = ap.parse_args()
    if ARGS.index_dir:
        DATA = ARGS.index_dir
    t_all = time.perf_counter()
    cases = pick_cases()
    print(f"  索引 {DATA}、場合 {len(cases)} 件", flush=True)

    catalog = si.SvIndexCatalog(DATA) if os.path.isdir(DATA) else None
    _CATALOG[0] = catalog
    if catalog is None or not catalog.files:
        for c in cases:
            record(c["name"], None, f"索引が無い({DATA})")
        return finish(t_all)

    # 星野を作る(要る段が無い場合は SKIP)
    fields, runnable = [], []
    for c in cases:
        ok, why = needed(c, catalog)
        if not ok:
            runnable.append((c, None, why))
            continue
        try:
            f = make_field(c, catalog)
        except Exception as e:
            runnable.append((c, None, f"星野を作れない: {type(e).__name__}: {e}"))
            continue
        fields.append((c, f))
        runnable.append((c, f, None))
        if ARGS.dump_fields:
            os.makedirs(ARGS.dump_fields, exist_ok=True)
            d = ss.sv_synth_to_json(f)
            s_lo, s_hi = scale_range(c)
            d["case"] = {"name": c["name"], "what": c["what"], "scale_lo": s_lo, "scale_hi": s_hi,
                         "hint": hint_of(c), "only": c["only"], "expect_solve": not c["random"]}
            with open(os.path.join(ARGS.dump_fields, f"{c['name']}.json"), "w", encoding="utf-8") as fp:
                json.dump(d, fp, ensure_ascii=False)

    # 自己検査(sv_solve が無くても走る)
    if fields:
        for name, fn, args in (("00a 星野: 正解・星の位置・種・JSON", selfcheck_generator, (fields, catalog)),
                               ("00b 星野: 星の数・偽の星・境界", selfcheck_contents, (fields,)),
                               ("00c 星野: 解ける形か(当たるはずの quad)", selfcheck_wellposed, (fields, catalog)),
                               ("01  物差し: astropy と同じ値", selfcheck_astropy, (fields,)),
                               ("02  物差し: 偽のソルバー", selfcheck_fake, (fields,))):
            t0 = time.perf_counter()
            try:
                ok, msg = fn(*args)
            except Exception as e:
                ok, msg = False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}"
            record(name, ok, f"{msg}  [{time.perf_counter() - t0:.1f} s]")

    solver, how = load_solver()
    print(f"  解法: {how}", flush=True)
    for c, f, why in runnable:
        name = f"{c['name']:<13s} {c['what']}"
        if f is None:
            record(name, None, why)
            continue
        if solver is None:
            record(name, None, how)
            continue
        ok, msg, rec = run_solve(c, f, catalog, solver)
        record(name, ok, msg, rec)
    catalog.close()
    return finish(t_all)


def finish(t_all):
    n_ok = sum(1 for r in RESULTS if r[1] is True)
    n_skip = sum(1 for r in RESULTS if r[1] is None)
    n_run = len(RESULTS) - n_skip
    width = max((len(r[0]) for r in RESULTS), default=0)
    print()
    for name, ok, msg, _ in RESULTS:
        tag = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
        print(f"  {tag}  {name.ljust(width)}  {msg.splitlines()[0] if msg else ''}")
    total = time.perf_counter() - t_all
    print(f"\n  {n_ok}/{n_run} passed, {n_skip} skipped  ({total:.0f} s)")
    if ARGS is not None and ARGS.json:
        out = [{"name": n, "status": "SKIP" if ok is None else ("PASS" if ok else "FAIL"), "message": m,
                **rec} for n, ok, m, rec in RESULTS]
        with open(ARGS.json, "w", encoding="utf-8") as fp:
            json.dump({"index_dir": DATA, "seconds": round(total, 1), "results": out}, fp,
                      ensure_ascii=False, indent=1, default=str)
    return 0 if n_ok == n_run else 1


if __name__ == "__main__":
    sys.exit(main())

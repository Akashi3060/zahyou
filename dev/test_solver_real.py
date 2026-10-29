"""実写の画像を自前の解析エンジンで解き、WSL 版(astrometry.net)の正解 dev/solver_truth.json と比べるテスト。

docs/design-native-solver.md の 7 節・8 節。合格の基準(solver_truth.json の criteria):
  中心のずれ 2″ 以内(0.5 px のほうが大きければそれ)、画素スケール 0.2 %、回転(配列の +y の位置角)0.1°、
  鏡像(配列座標の det CD の符号)の一致。陰性の画像(ピンぼけ・光条・乱数の星野)は締め切りまで探して解なし、
  締め切りの超過 1 秒以内。画像はリポジトリに入れていない(手元に無ければ SKIP)。
  正解ファイルの path は、リポジトリからの相対パス(dev/tcase は test_robust.py が作る)か、
  $ZAHYOU_TEST_IMAGES/ で始まるパス(環境変数 ZAHYOU_TEST_IMAGES。無ければホームフォルダー)。
  陽性の画像が 1 枚も解かれなかった(全部 SKIP)ときは、最後にそう出して終了コード 2 で終わる
  (自己検査だけが PASS して「合格」に見えないように)。

  python dev/test_solver_real.py             利用者の経路: load_image_any → preprocess →
                                             detect_sources(min_sources=60) → solve_offline(ヒントなし・焦点距離なし)
  python dev/test_solver_real.py --direct    sv_solve を直接(同じ検出の星。12 個未満なら深い検出の星)
  python dev/test_solver_real.py --both      両方
  python dev/test_solver_real.py --engine    利用者の経路を、組み立てたエンジン(直下の zahyou_engine.py)で。
                                             GUI と同じく 1 つの名前空間へ exec する(--engine パス で別のファイル)
  --deadline 60(1 枚の締め切り [秒])  --case real_orig,tcase_*  --quick(代表 8 枚)  --json 結果.json
  --index-dir C:\\AstrometryData(環境変数 ZAHYOU_INDEX_DIR でも)  --truth 正解.json  --verbose(解析のログ)

基準の画像 real_orig は、目標 UCAC4 660-021020 をヒント(半径 5°)にした解き方(real_orig+hint)も試し、
目標までのずれ(1.583′・0.918′ 東・1.290′ 北)も比べる。最初の 4 項目は正解ファイルと物差しの自己検査で、
エンジンが無くても走る。記憶ファイル(cache.json)は使わない(ZAHYOU_CACHE を空の一時ファイルに向ける)。
"""
import argparse
import contextlib
import fnmatch
import hashlib
import io
import json
import math
import os
import sys
import tempfile
import time
import traceback
import types
import warnings

warnings.filterwarnings("ignore")
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
# 利用者の記憶ファイル(前回の実測スケール)を読まない・書かない。zahyou_core を読む前に決める
_TMP = tempfile.mkdtemp(prefix="zahyou_test_real_")
os.environ["ZAHYOU_CACHE"] = os.path.join(_TMP, "cache.json")
import solver_synth as ss      # noqa: E402
import solver_wcs as sw        # noqa: E402

DATA = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
TRUTH = os.path.join(HERE, "solver_truth.json")
QUICK = ("real_orig", "real_flip_fits", "3I_00010", "phaethon_00001", "imx432_16bit", "siril_rgb_stack",
         "capture_005514", "tcase_basic16")
RESULTS = []                   # (名前, True / False / None(SKIP), 説明, 記録)
ARGS = None
REPO = os.path.dirname(HERE)
IMAGES_VAR = "$ZAHYOU_TEST_IMAGES/"


# Downloads の試験画像の控え(作者の PC では Downloads を整理するので、試験が読む元の画像を同じ並びで写してある。2026-10-29)
TEST_IMAGES_COPY = ("Documents", "claude", "zahyou", "testdata", "images")


def images_path(rest):
    """"$ZAHYOU_TEST_IMAGES/" の後ろ(rest)を実際のパスに。ZAHYOU_TEST_IMAGES(無ければホームフォルダー)からの相対で、
    Downloads/… がそこに無ければ、同じ並びの控え(TEST_IMAGES_COPY の下)も見る。"""
    base = os.environ.get("ZAHYOU_TEST_IMAGES") or os.path.expanduser("~")
    parts = rest.split("/")
    p = os.path.join(base, *parts)
    if not os.path.exists(p) and parts[0] == "Downloads":
        q = os.path.join(base, *TEST_IMAGES_COPY, *parts)
        if os.path.exists(q):
            return q
    return p


def image_path(e):
    """正解ファイルの path を、この PC のパスにする(リポジトリからの相対か $ZAHYOU_TEST_IMAGES/ から)。"""
    p = e["path"]
    if p.startswith(IMAGES_VAR):
        return images_path(p[len(IMAGES_VAR):])
    return os.path.join(REPO, *p.split("/"))


def record(name, ok, msg, rec=None):
    RESULTS.append((name, ok, msg, rec or {}))
    tag = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
    print(f"  {tag}  {name}  {msg}", flush=True)


# ------------------------------------------------------------------ 物差し ---

def header_from_cards(cards):
    """solver_truth.json の wcs(カードの dict)か fits.Header から astropy の WCS。"""
    from astropy.io import fits
    from astropy.wcs import WCS
    h = fits.Header()
    for k, v in (cards.items() if isinstance(cards, dict) else cards.items()):
        if k in ("COMMENT", "HISTORY", ""):
            continue
        h[k] = v
    return WCS(h)


def wcs_of_header(h, shape):
    """解析が返したヘッダー(CRPIX は 1 始まり)→ astropy の WCS(画像の大きさに合わせて整える)。"""
    from astropy.wcs import WCS
    h = h.copy()
    h["NAXIS"] = 2
    h["NAXIS1"] = int(shape[1])
    h["NAXIS2"] = int(shape[0])
    for k in ("EQUINOX", "RADECSYS", "MJD-OBS", "DATE-OBS"):
        h.pop(k, None)
    h["RADESYS"] = "ICRS"
    return WCS(h)


def astropy_fn(w):
    """astropy の WCS を「画素(0 始まり)→ 単位ベクトル」の関数に包む。フレームの変換はしない(どれも ICRS)。"""
    def fn(x, y):
        ra, dec = w.all_pix2world(np.asarray(x, float), np.asarray(y, float), 0)
        return sw.sv_radec_to_xyz(ra, dec)
    return fn


def truth_measure(t):
    """solver_truth.json の truth / truth_tan を sv_synth_compare の形に。"""
    xyz = t.get("center_xyz") or sw.sv_radec_to_xyz(t["center_ra"], t["center_dec"]).tolist()
    return {"center_ra": t["center_ra"], "center_dec": t["center_dec"], "center_xyz": xyz,
            "scale_arcsec": t["scale_arcsec"], "pa_up_deg": t["pa_up_deg"], "parity": int(t["parity"])}


def effective_criteria(e, crit):
    """正解そのものの不確かさが基準と同じ大きさの画像は、solver_truth.json の criteria_override で基準を広げる(理由もそこに書く)。"""
    return {**crit, **{k: v for k, v in e.get("criteria_override", {}).items() if k != "reason"}}


def evaluate(e, fn_or_wcs, crit):
    """解(SvWcs か画素 → 単位ベクトルの関数)を正解と比べる。accept に truth_tan があればそれも。

    戻り値 (ok, 比べた結果, どの正解に合ったか)。"""
    W, H = e["width"], e["height"]
    crit = effective_criteria(e, crit)
    tol = {k: crit[k] for k in ("center_arcsec", "center_px", "scale_rel", "rot_deg")}
    want = truth_measure(e["truth"])
    got = ss.sv_synth_measure(fn_or_wcs, W, H, ref_xyz=want["center_xyz"])
    cmp = ss.sv_synth_compare(got, want, tol)
    cmp["got"] = got
    if cmp["ok"]:
        return True, cmp, "truth"
    if "truth_tan" in e.get("accept", []) and e.get("truth_tan"):
        w2 = truth_measure(e["truth_tan"])
        got2 = ss.sv_synth_measure(fn_or_wcs, W, H, ref_xyz=w2["center_xyz"])
        cmp2 = ss.sv_synth_compare(got2, w2, tol)
        cmp2["got"] = got2
        if cmp2["ok"]:
            cmp2["vs_truth"] = cmp
            return True, cmp2, "truth_tan"
    return False, cmp, "truth"


def describe(cmp):
    return (f"中心 {cmp['center_arcsec']:.3f}″(許容 {cmp['center_tol_arcsec']:.2f}″)・"
            f"スケール {cmp['scale_rel'] * 100:+.4f}%・回転 {cmp['rot_deg']:+.4f}°・"
            f"パリティ {cmp['parity'][0]:+d}/{cmp['parity'][1]:+d}")


def offsets_arcmin(c_ra, c_dec, ra, dec):
    """中心から目標までの (全角距離, 東, 北)(分角)。astropy の spherical_offsets_to と同じ式。"""
    a0, d0, a, d = map(math.radians, (c_ra, c_dec, ra, dec))
    da = a - a0
    x = math.cos(d) * math.cos(da) * math.cos(d0) + math.sin(d) * math.sin(d0)
    y = math.cos(d) * math.sin(da)
    z = math.sin(d) * math.cos(d0) - math.cos(d) * math.cos(da) * math.sin(d0)
    return (math.degrees(math.atan2(math.hypot(y, z), x)) * 60.0, math.degrees(math.atan2(y, x)) * 60.0,
            math.degrees(math.asin(max(-1.0, min(1.0, z)))) * 60.0)


def check_offsets(base, center_ra, center_dec):
    """目標までのずれを基準値(小数 3 桁の表示)と比べる。許容は中心の基準 2″ と同じ(0.0334′)。"""
    sep, de, dn = offsets_arcmin(center_ra, center_dec, base["ra_deg"], base["dec_deg"])
    d = max(abs(sep - base["sep_arcmin"]), abs(de - base["east_arcmin"]), abs(dn - base["north_arcmin"]))
    same = (f"{sep:.3f}", f"{de:.3f}", f"{dn:.3f}") == (f"{base['sep_arcmin']:.3f}", f"{base['east_arcmin']:.3f}",
                                                       f"{base['north_arcmin']:.3f}")
    ok = d <= 2.0 / 60.0 + 5e-4                # 表示の丸め(0.0005′)の分を足す
    msg = (f"{sep:.3f}′・{de:.3f}′ 東・{dn:.3f}′ 北(基準 {base['sep_arcmin']:.3f}′・{base['east_arcmin']:.3f}′・"
           f"{base['north_arcmin']:.3f}′、差 ≤ {d * 60:.2f}″、表示{'も一致' if same else 'は違う'})")
    return ok, msg


# ------------------------------------------------------------------ 自己検査 ---

def selfcheck_file(doc):
    """R00: 正解ファイルの形。"""
    bad = []
    imgs = doc.get("images") or []
    crit = doc.get("criteria") or {}
    for k in ("center_arcsec", "center_px", "scale_rel", "rot_deg", "negative_overrun_s"):
        if k not in crit:
            bad.append(f"criteria に {k} が無い")
    tags = [e.get("tag") for e in imgs]
    if len(set(tags)) != len(tags):
        bad.append("tag が重複している")
    for e in imgs:
        for k in ("tag", "path", "expect_solve", "width", "height", "sha256"):
            if k not in e:
                bad.append(f"{e.get('tag')}: {k} が無い")
        if e.get("expect_solve"):
            t = e.get("truth") or {}
            for k in ("center_ra", "center_dec", "center_xyz", "scale_arcsec", "pa_up_deg", "parity"):
                if k not in t:
                    bad.append(f"{e['tag']}: truth.{k} が無い")
            if not e.get("wcs"):
                bad.append(f"{e['tag']}: wcs が無い")
        p = e.get("path", "")
        if os.path.isabs(p) or "\\" in p or ":" in p:
            bad.append(f"{e.get('tag')}: パスが PC に依存する形({p})")
    npos = sum(1 for e in imgs if e.get("expect_solve"))
    nneg = len(imgs) - npos
    b = doc.get("target_offset_baseline") or {}
    if not b or b.get("image") not in tags:
        bad.append("target_offset_baseline が無いか、画像が一覧に無い")
    msg = f"画像 {len(imgs)} 件(陽性 {npos}、陰性 {nneg})、基準 {crit}"
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def selfcheck_measure(doc):
    """R01: 正解の WCS(カード)を astropy で読み、同じ物差しで測ると truth と同じ値になること。
    記録されていた値(oracle の center・pa・scale)との差も出す(scale は記録が CD の値、こちらは中心の値)。"""
    bad = []
    worst = {"center": 0.0, "scale": 0.0, "rot": 0.0}
    rep = {"center": 0.0, "scale": 0.0, "rot": 0.0}
    n = 0
    for e in doc["images"]:
        if not e.get("expect_solve"):
            continue
        n += 1
        w = header_from_cards(e["wcs"])
        want = truth_measure(e["truth"])
        got = ss.sv_synth_measure(astropy_fn(w), e["width"], e["height"], ref_xyz=want["center_xyz"])
        cmp = ss.sv_synth_compare(got, want, {"center_arcsec": 1e-4, "center_px": 0.0, "scale_rel": 1e-8,
                                              "rot_deg": 1e-6})
        worst["center"] = max(worst["center"], cmp["center_arcsec"])
        worst["scale"] = max(worst["scale"], abs(cmp["scale_rel"]))
        worst["rot"] = max(worst["rot"], abs(cmp["rot_deg"]))
        if not cmp["ok"]:
            bad.append(f"{e['tag']}: {cmp['failed']} {describe(cmp)}")
        if (1 if float(np.linalg.det(w.pixel_scale_matrix)) > 0 else -1) != want["parity"]:
            bad.append(f"{e['tag']}: det CD の符号と parity が違う")
        rv = e.get("reported_vs_truth") or {}
        rep["center"] = max(rep["center"], rv.get("center_arcsec", 0.0))
        rep["scale"] = max(rep["scale"], abs(rv.get("scale_rel", 0.0)))
        rep["rot"] = max(rep["rot"], abs(rv.get("rot_deg", 0.0)))
    msg = (f"陽性 {n} 件の WCS を測り直して truth と一致(中心 ≤ {worst['center']:.1e}″・スケール ≤ "
           f"{worst['scale']:.1e}・回転 ≤ {worst['rot']:.1e}°)。記録の値との差は中心 ≤ {rep['center']:.3f}″・"
           f"スケール ≤ {rep['scale'] * 100:.3f}%・回転 ≤ {rep['rot']:.4f}°")
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def tan_header(t, W, H):
    """正解の値(中心・スケール・+y の位置角・パリティ)から作った TAN のヘッダー(偽のソルバーの答え)。"""
    from astropy.io import fits
    wcs = ss.sv_synth_truth_wcs(t["center_ra"], t["center_dec"], t["scale_arcsec"], W, H,
                                rotation_deg=t["pa_up_deg"], mirror=int(t["parity"]) > 0)
    h = fits.Header()
    for k, v, c in wcs.to_header_cards(W, H):
        if k != "COMMENT":
            h[k] = (v, c)
    return h, wcs


def selfcheck_fake(doc):
    """R02: 偽のソルバーで物差しを確かめる。正解から作ったヘッダー・正解の WCS そのもの → PASS、
    許容を超えてずらしたもの → その項目だけ FAIL。陰性は「解なし → PASS、解あり・締め切り超過 → FAIL」。"""
    crit = doc["criteria"]
    bad = []
    n = 0
    for e in doc["images"]:
        if not e.get("expect_solve"):
            continue
        W, H = e["width"], e["height"]
        h, base = tan_header(e["truth"], W, H)
        ok, cmp, _ = evaluate(e, astropy_fn(wcs_of_header(h, (H, W))), crit)
        if not ok or cmp["center_arcsec"] > 1e-4 or abs(cmp["rot_deg"]) > 1e-5:
            bad.append(f"{e['tag']}: 正解から作ったヘッダーが通らない {describe(cmp)}")
        ok, cmp, _ = evaluate(e, astropy_fn(header_from_cards(e["wcs"])), crit)
        if not ok:
            bad.append(f"{e['tag']}: 正解の WCS そのものが通らない {describe(cmp)}")
        if e.get("truth_tan"):
            h2, _ = tan_header(e["truth_tan"], W, H)
            ok, cmp, which = evaluate(e, astropy_fn(wcs_of_header(h2, (H, W))), crit)
            expect = "truth_tan" in e.get("accept", []) or ok
            if ok != expect:
                bad.append(f"{e['tag']}: TAN の参考値の扱いが違う")
        s = base.pixel_scale_arcsec()
        ec = effective_criteria(e, crit)
        ctol = max(ec["center_arcsec"], ec["center_px"] * s)
        for kind, k, expect in (("center", 1.5, ["center"]), ("scale", 2.0, ["scale"]), ("rot", 2.0, ["rot"]),
                                ("parity", 1, ["parity"]), ("center", 0.4, []), ("rot", 0.4, [])):
            crpix, cd = base.crpix.copy(), base.cd.copy()
            if kind == "center":
                crpix -= np.array([1.0, 1.0]) / math.sqrt(2.0) * k * ctol / s
            elif kind == "scale":
                cd *= 1.0 + k * ec["scale_rel"]
            elif kind == "rot":
                r = math.radians(k * ec["rot_deg"])
                cd = np.array([[math.cos(r), math.sin(r)], [-math.sin(r), math.cos(r)]]) @ cd
            else:
                cd[:, 0] = -cd[:, 0]
            pw = sw.SvWcs(base.crval_xyz, crpix, cd)
            # 正解があいまいな画像は truth_tan でも合格になりうるので、truth だけで見る
            e1 = dict(e, accept=["truth"])
            ok, cmp, _ = evaluate(e1, pw, crit)
            n += 1
            if cmp["failed"] != expect:
                bad.append(f"{e['tag']}: {kind}×{k} で {cmp['failed']}(期待 {expect})")
    neg_ok = (judge_negative(None, 10.0, 10.0, crit)[0] and not judge_negative(object(), 1.0, 10.0, crit)[0]
              and not judge_negative(None, 11.5, 10.0, crit)[0])
    if not neg_ok:
        bad.append("陰性の判定がおかしい")
    msg = f"正解から作った解・正解の WCS → PASS、ずらした {n} 通り → 期待どおりの項目だけ FAIL、陰性の判定も正しい"
    return (not bad), (msg if not bad else "; ".join(bad[:6]))


def selfcheck_offsets(doc):
    """R03: 正解の中心から目標までのずれが、基準値(1.583′・0.918′ 東・1.290′ 北)になること(式の検算)。"""
    b = doc["target_offset_baseline"]
    e = next(x for x in doc["images"] if x["tag"] == b["image"])
    ok, msg = check_offsets(b, e["truth"]["center_ra"], e["truth"]["center_dec"])
    return ok, f"{b['image']} の正解から {b['target']}: {msg}"


def judge_negative(sol, elapsed, deadline_s, crit):
    over = elapsed - deadline_s
    if sol is not None:
        return False, "解けてはいけないのに解が出た"
    if over > crit["negative_overrun_s"]:
        return False, f"締め切りを {over:.1f} 秒超えた"
    return True, f"解なし(締め切りとの差 {over:+.2f} 秒)"


# ------------------------------------------------------------------ 解く ---

def load_engine():
    """(zahyou_core, sv_solve, 説明)。読めなければ (None, None, 理由)。"""
    if ARGS is not None and ARGS.engine:
        # 組み立てたエンジンを GUI の Engine.load と同じ形で読む(sys.modules に載らない
        # 名前空間へ exec。dataclass がここで落ちたことがある)
        path = ARGS.engine
        ns = {"__name__": "zahyou_engine", "__file__": path}
        try:
            with io.open(path, encoding="utf-8") as f:
                exec(compile(f.read(), path, "exec"), ns)
        except BaseException as e:             # scipy が無いときの SystemExit も
            return None, None, f"{path} を読めない: {type(e).__name__}: {e}"
        missing = [n for n in ("sv_solve", "load_image_any", "preprocess", "detect_sources",
                               "solve_offline") if n not in ns]
        if missing:
            return None, None, f"{path} に {', '.join(missing)} が無い"
        return types.SimpleNamespace(**ns), ns["sv_solve"], f"組み立てたエンジン {path}"
    try:
        import zahyou_core as zc
    except Exception as e:
        return None, None, f"zahyou_core を読めない: {type(e).__name__}: {e}"
    solve = getattr(zc, "sv_solve", None)
    if solve is None:
        try:
            import solver_core as sc
            solve = getattr(sc, "sv_solve", None)
        except Exception as e:
            return zc, None, f"solver_core を読めない: {type(e).__name__}: {e}"
    if solve is None:
        return zc, None, "sv_solve がまだ無い(dev/solver_core.py の作業中)"
    for name in ("load_image_any", "preprocess", "detect_sources", "solve_offline"):
        if not hasattr(zc, name):
            return zc, None, f"zahyou_core に {name} が無い"
    return zc, solve, "zahyou_core + sv_solve"


_SHA = {}


def sha_ok(e):
    p = image_path(e)
    if p not in _SHA:
        h = hashlib.sha256()
        with open(p, "rb") as f:
            for b in iter(lambda: f.read(1 << 20), b""):
                h.update(b)
        _SHA[p] = h.hexdigest()
    return _SHA[p] == e["sha256"]


def run_image(e, mode, hint, zc, solve, catalog, crit, base):
    """1 枚を解く。mode = user(solve_offline)/ direct(sv_solve)。戻り値 (ok, 説明, 記録)。"""
    W, H = e["width"], e["height"]
    rec = {"tag": e["tag"], "mode": mode, "hint": hint, "expect_solve": e["expect_solve"]}
    t0 = time.perf_counter()
    b = zc.load_image_any(image_path(e))
    if tuple(b.shape) != (H, W):
        return False, f"画像の大きさ {b.shape} が正解 ({H}, {W}) と違う", rec
    sub, mask, sigma, info = zc.preprocess(b.data)
    src, thr = zc.detect_sources(sub, mask, sigma, min_sources=60)
    rec["n_sources"] = len(src)
    rec["detect_s"] = round(time.perf_counter() - t0, 3)
    deadline_s = ARGS.deadline
    log = io.StringIO()
    sol = hdr = None
    if mode == "user":
        t1 = time.monotonic()
        out = sys.stdout if ARGS.verbose else log
        with contextlib.redirect_stdout(out):
            hdr = zc.solve_offline(b, src, _TMP, timeout=deadline_s, focal_length_mm=None,
                                   hint_radec=None if hint is None else hint[:2],
                                   hint_radius_deg=5.0 if hint is None else hint[2],
                                   verbose=True, diagnostics=None, index_dir=DATA,
                                   prep=(sub, mask, sigma))
        dt = time.monotonic() - t1
        got = hdr
    else:
        stars = src
        how = "標準の検出"
        if len(src) < 12 and hasattr(zc, "detect_sources_deep"):
            stars, _ = zc.detect_sources_deep(sub, mask, sigma)
            how = "深い検出(detect_sources_deep)"
        rec["direct_stars"] = f"{how} {len(stars)} 個"
        xy = np.array([[s["x"], s["y"]] for s in stars], dtype=np.float64).reshape(-1, 2)
        flux = np.array([s["flux"] for s in stars], dtype=np.float64)
        kw = {}
        if ARGS.verbose:
            kw["log"] = lambda m: print(f"      · {m}", flush=True)
        t1 = time.monotonic()
        try:
            sol = solve(xy, flux, W, H, catalog, scale_lo=None, scale_hi=None, hint=hint,
                        deadline=t1 + deadline_s, **kw)
        except Exception as ex:
            return False, f"例外 {type(ex).__name__}: {ex}\n{traceback.format_exc()}", rec
        dt = time.monotonic() - t1
        got = sol
    rec["solve_s"] = round(dt, 3)
    rec["log_tail"] = log.getvalue().splitlines()[-8:]
    info = f"  {dt:.1f} 秒(星 {len(src)} 個"
    if sol is not None:
        for a in ("index_name", "nmatch", "logodds", "quads_tried"):
            v = getattr(sol, a, None)
            if isinstance(v, (int, float, str, np.integer, np.floating)):
                rec[f"sol_{a}"] = v.item() if hasattr(v, "item") else v
        lo = rec.get("sol_logodds")
        info += (f"・{rec.get('sol_index_name', '?')}・一致 {rec.get('sol_nmatch', '?')}"
                 + (f"・対数オッズ {lo:.1f}" if isinstance(lo, (int, float)) else ""))
    elif hdr is not None:
        com = " ".join(str(c) for c in hdr.get("COMMENT", []))
        rec["comment"] = com[-200:]
    info += ")"
    if mode == "direct" and "direct_stars" in rec:
        info = info[:-1] + f"、解いたのは{rec['direct_stars']})"

    if not e["expect_solve"]:
        ok, msg = judge_negative(got, dt, deadline_s, crit)
        rec["over_s"] = round(dt - deadline_s, 3)
        return ok, msg + info, rec
    if got is None:
        return False, f"解けなかった(締め切り {deadline_s:g} 秒){info}", rec
    try:
        target = sol.wcs if sol is not None else astropy_fn(wcs_of_header(hdr, (H, W)))
        ok, cmp, which = evaluate(e, target, crit)
    except Exception as ex:
        return False, f"解の WCS を測れない: {type(ex).__name__}: {ex}{info}", rec
    rec.update({k: cmp[k] for k in ("failed", "center_arcsec", "center_tol_arcsec", "scale_rel", "rot_deg",
                                      "parity")})
    rec["matched_truth"] = which
    msg = describe(cmp) + info
    if which == "truth_tan":
        msg = "TAN の参考値に合格(正解の SIP とは " + describe(cmp["vs_truth"]) + "): " + msg
    if not ok:
        msg = f"外れ {cmp['failed']}: " + msg
    # 基準の画像は、目標までのずれも比べる
    if base and e["tag"] == base["image"]:
        g = cmp["got"]
        ok2, m2 = check_offsets(base, g["center_ra"], g["center_dec"])
        rec["offsets_ok"] = ok2
        msg += f"  目標まで {m2}"
        ok = ok and ok2
    return ok, msg, rec


def pick(doc):
    imgs = doc["images"]
    if ARGS.quick:
        imgs = [e for e in imgs if e["tag"] in QUICK]
    if ARGS.case:
        pats = [p.strip() for p in ARGS.case.split(",") if p.strip()]
        imgs = [e for e in doc["images"] if any(fnmatch.fnmatch(e["tag"], p) for p in pats)]
        if not imgs:
            raise SystemExit(f"--case {ARGS.case} に合う画像がありません。tag: "
                             + ", ".join(e["tag"] for e in doc["images"]))
    return imgs


def main():
    global ARGS, DATA
    try:
        sys.stdout.reconfigure(errors="replace")    # cp932 の画面でも止まらないように
    except Exception:
        pass
    ap = argparse.ArgumentParser(description="実写の画像で自前の解析エンジンを確かめる")
    ap.add_argument("--direct", action="store_true", help="sv_solve を直接呼ぶ(既定は利用者の経路)")
    ap.add_argument("--both", action="store_true", help="利用者の経路と sv_solve の直接の両方")
    ap.add_argument("--deadline", type=float, default=60.0, help="1 枚の締め切り [秒](既定 60)")
    ap.add_argument("--case", help="画像の tag(ワイルドカード可、カンマ区切り)")
    ap.add_argument("--quick", action="store_true", help="代表 8 枚だけ")
    ap.add_argument("--json", help="結果を JSON に書く")
    ap.add_argument("--index-dir", default=None, help="索引の置き場所(既定 C:\\AstrometryData)")
    ap.add_argument("--truth", default=TRUTH, help="正解のファイル(既定 dev/solver_truth.json)")
    ap.add_argument("--verbose", action="store_true", help="解析のログを出す")
    ap.add_argument("--engine", nargs="?", default=None,
                    const=os.path.join(os.path.dirname(HERE), "zahyou_engine.py"),
                    help="利用者の経路を組み立てたエンジンで(既定は直下の zahyou_engine.py)")
    ARGS = ap.parse_args()
    if ARGS.index_dir:
        DATA = ARGS.index_dir
    t_all = time.perf_counter()
    with open(ARGS.truth, encoding="utf-8") as f:
        doc = json.load(f)
    crit = doc["criteria"]
    base = doc.get("target_offset_baseline")

    for name, fn in (("R00 正解ファイルの形", selfcheck_file), ("R01 物差し: 正解の WCS を測り直す", selfcheck_measure),
                     ("R02 物差し: 偽のソルバー", selfcheck_fake), ("R03 目標までのずれの式", selfcheck_offsets)):
        t0 = time.perf_counter()
        try:
            ok, msg = fn(doc)
        except Exception as ex:
            ok, msg = False, f"{type(ex).__name__}: {ex}\n{traceback.format_exc()}"
        record(name, ok, f"{msg}  [{time.perf_counter() - t0:.1f} s]")

    modes = ["user", "direct"] if ARGS.both else (["direct"] if ARGS.direct else ["user"])
    imgs = pick(doc)
    zc, solve, how = load_engine()
    catalog = None
    if solve is not None and "direct" in modes:
        try:
            catalog = zc.index_catalog(DATA) if hasattr(zc, "index_catalog") else None
        except Exception:
            catalog = None
        if catalog is None:
            import solver_index as si
            catalog = si.SvIndexCatalog(DATA)
    print(f"  エンジン: {how}、索引 {DATA}、締め切り {ARGS.deadline:g} 秒、経路 {', '.join(modes)}", flush=True)
    have_index = os.path.isdir(DATA) and any(n.startswith("index-") for n in os.listdir(DATA))

    jobs = []
    for e in imgs:
        jobs.append((e, None))
        if base and e["tag"] == base["image"]:
            jobs.append((e, (base["ra_deg"], base["dec_deg"], 5.0)))
    for mode in modes:
        for e, hint in jobs:
            label = {"user": "利用者の経路", "direct": "sv_solve 直接"}[mode]
            name = f"{e['tag'] + ('+hint' if hint else ''):<20s} {label}  {e.get('kind', '')}"
            if not os.path.isfile(image_path(e)):
                record(name, None, f"画像が無い({image_path(e)})", {"positive": e["expect_solve"]})
                continue
            if not sha_ok(e):
                record(name, None, "画像の中身が正解を作ったときと違う(sha256)")
                continue
            if not have_index:
                record(name, None, f"索引が無い({DATA})")
                continue
            if solve is None:
                record(name, None, how)
                continue
            try:
                ok, msg, rec = run_image(e, mode, hint, zc, solve, catalog, crit, base)
            except Exception as ex:
                ok, msg, rec = False, f"{type(ex).__name__}: {ex}\n{traceback.format_exc()}", {}
            rec = dict(rec or {})
            rec["positive"] = e["expect_solve"]
            record(name, ok, msg, rec)
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
    n_pos = sum(1 for r in RESULTS if r[1] is not None and r[3].get("positive"))
    if n_pos == 0:
        print("  ⚠️ 実写の陽性の画像を 1 枚も解いていません(画像が無いか索引が無い)。"
              "自己検査だけの結果なので、終了コード 2 で終わります。")
    if ARGS is not None and ARGS.json:
        out = [{"name": n, "status": "SKIP" if ok is None else ("PASS" if ok else "FAIL"), "message": m, **rec}
               for n, ok, m, rec in RESULTS]
        with open(ARGS.json, "w", encoding="utf-8") as fp:
            json.dump({"index_dir": DATA, "deadline_s": ARGS.deadline, "seconds": round(total, 1),
                       "results": out}, fp, ensure_ascii=False, indent=1, default=str)
    if n_ok != n_run:
        return 1
    return 0 if n_pos else 2


if __name__ == "__main__":
    sys.exit(main())

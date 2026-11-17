"""ノートブック版の拡張モジュール zahyou-native(rs/py)が、Python の sv_solve と同じ解を返すかを確かめるテスト。

zahyou_core._solve_stars(エンジンが使う入口)で解き、同じ星野を sv_solve で直接解いた解と比べる。
比べるのは 索引・対数オッズ・一致の数・試した仮説の数・WCS(中心 1e-6″、CD 1e-12)。合成星野は test_solver_synth.py の場合
(ヒントつきの速いものと、解けない乱数の星野)。拡張が入っていなければ(pip install zahyou-native)全部 SKIP。

  python dev/test_native.py        (拡張を入れた Python で)
"""
import math
import os
import sys
import time
import types
import warnings

warnings.filterwarnings("ignore")
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import test_solver_synth as T   # noqa: E402
import solver_index as si       # noqa: E402
import solver_core as sc        # noqa: E402
import solver_wcs as sw         # noqa: E402
import zahyou_core as zc        # noqa: E402

CASES = ["hs", "hb", "ra0_h", "tile_corner_h", "polein_h", "few20", "sip_pin", "rand_hint"]
RESULTS = []


def main():
    try:
        sys.stdout.reconfigure(errors="replace")
    except Exception:
        pass
    mod = zc.native_solver()
    if mod is None:
        print(f"  SKIP 全部: {zc._NATIVE['why']}")
        return 0
    print(f"  zahyou_native {mod.__version__}(API {mod.API})")
    T.ARGS = types.SimpleNamespace(deadline=60.0, deadline_random=30.0, verbose=False, trace=None, fake=False)
    cat = si.SvIndexCatalog(T.DATA)
    T._CATALOG[0] = cat
    if not cat.files:
        print(f"  SKIP 全部: 星図データが無い({T.DATA})")
        return 0
    for name in CASES:
        c = next(x for x in T.CASES if x["name"] == name)
        ok_need, why = T.needed(c, cat)
        if not ok_need or c["only"]:
            print(f"  SKIP {name}: {why if not ok_need else 'only は目録を絞るので比べない'}")
            continue
        f = T.make_field(c, cat)
        xy, flux = np.array(f["xy"]), np.array(f["flux"])
        W, H = c["size"]
        s_lo, s_hi = T.scale_range(c)
        hint = T.hint_of(c)
        dl = 10.0 if c["random"] else 60.0
        t0 = time.monotonic()
        nat = zc._solve_stars(xy, flux, W, H, cat, scale_lo=s_lo, scale_hi=s_hi, hint=hint,
                              deadline=time.monotonic() + dl, cancel=lambda: False, log=None)
        t1 = time.monotonic()
        py = sc.sv_solve(xy, flux, W, H, cat, scale_lo=s_lo, scale_hi=s_hi, hint=hint, deadline=time.monotonic() + dl)
        t2 = time.monotonic()
        bad = []
        if (nat is None) != (py is None):
            bad.append(f"片方だけ解けた(Rust {nat is not None}・Python {py is not None})")
        elif nat is not None:
            if nat.index_name != py.index_name:
                bad.append(f"索引 {nat.index_name} / {py.index_name}")
            if abs(nat.logodds - py.logodds) > 1e-6:
                bad.append(f"対数オッズ {nat.logodds} / {py.logodds}")
            for a in ("nmatch", "ndistract", "nfield", "nref", "hypotheses"):
                if getattr(nat, a) != getattr(py, a):
                    bad.append(f"{a} {getattr(nat, a)} / {getattr(py, a)}")
            d = math.degrees(math.acos(min(1.0, float(np.dot(nat.wcs.crval_xyz, py.wcs.crval_xyz))))) * 3600.0
            if d > 1e-6 or np.abs(nat.wcs.cd - py.wcs.cd).max() > 1e-12 or np.abs(nat.wcs.crpix - py.wcs.crpix).max() > 1e-6:
                bad.append(f"WCS が違う(中心 {d:.2e}″)")
            if nat.wcs.has_sip != py.wcs.has_sip:
                bad.append("SIP の有無が違う")
            if not nat.wcs.to_header_cards(int(W), int(H)):
                bad.append("ヘッダーにできない")
        what = (f"{nat.index_name}・対数オッズ {nat.logodds:.2f}・仮説 {nat.hypotheses:,}" if nat else "解なし")
        msg = f"{what}  Rust {t1 - t0:.2f} 秒 / Python {t2 - t1:.2f} 秒"
        RESULTS.append(not bad)
        print(f"  {'PASS' if not bad else 'FAIL'}  {name:<14s} {msg}" + ("" if not bad else "  " + "; ".join(bad)))
    # 中止: すぐ止まって SvCancelled
    c = next(x for x in T.CASES if x["name"] == "rand60")
    f = T.make_field(c, cat)
    t0 = time.monotonic()
    try:
        zc._solve_stars(np.array(f["xy"]), np.array(f["flux"]), *c["size"], cat, scale_lo=None, scale_hi=None,
                        hint=None, deadline=time.monotonic() + 30.0, cancel=lambda: time.monotonic() - t0 > 1.0, log=None)
        ok, msg = False, "中止しても戻らなかった"
    except sc.SvCancelled:
        dt = time.monotonic() - t0
        ok, msg = dt < 2.0, f"中止の合図から {dt - 1.0:.2f} 秒で SvCancelled"
    RESULTS.append(ok)
    print(f"  {'PASS' if ok else 'FAIL'}  {'cancel':<14s} {msg}")
    cat.close()
    n = sum(RESULTS)
    print(f"\n  {n}/{len(RESULTS)} passed")
    return 0 if n == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())

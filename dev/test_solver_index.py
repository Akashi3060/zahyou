"""solver_index.py(索引ファイルの読み込み・kd-tree の探索・タイルの選び方)を実物の索引で確かめるテスト。

C:\\AstrometryData(環境変数 ZAHYOU_INDEX_DIR で変えられる)が無ければ全部 SKIP。
"""
import math
import os
import shutil
import sys
import tempfile
import time
import traceback
import warnings

warnings.filterwarnings("ignore")
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import solver_index as si      # noqa: E402

DATA = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
rng = np.random.default_rng(20261015)

RESULTS = []                   # (名前, True / False / None(SKIP), 説明)
_FILES = {}


def path_of(tag):
    return os.path.join(DATA, f"index-{tag}.fits")


def have(*tags):
    return os.path.isdir(DATA) and all(os.path.isfile(path_of(t)) for t in tags)


def index(tag):
    """テスト全体で使い回す SvIndexFile。"""
    if tag not in _FILES:
        _FILES[tag] = si.SvIndexFile(path_of(tag))
    return _FILES[tag]


def skip(name, why):
    RESULTS.append((name, None, why))


def run_case(name, fn, *tags):
    if not have(*tags):
        skip(name, f"索引が無い({DATA} の {', '.join(tags) or '*'})")
        return
    t0 = time.perf_counter()
    try:
        ok, msg = fn()
    except Exception as e:
        ok, msg = False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}"
    RESULTS.append((name, ok, f"{msg}  [{time.perf_counter() - t0:.1f} s]"))


def random_codes(r, n, D):
    """C, D を AB を直径とする円の中に一様に置き、索引と同じ正準形にしたコード(画像側の問い合わせの代わり)。"""
    k = D // 2
    ang = r.uniform(0, 2 * np.pi, (n, k))
    rad = np.sqrt(r.uniform(0, 1, (n, k)) * 0.5)
    c = np.stack([0.5 + rad * np.cos(ang), 0.5 + rad * np.sin(ang)], -1).reshape(n, D)
    if D == 4:
        flip = c[:, 0] + c[:, 2] > 1
        c[flip] = 1 - c[flip]
        sw = c[:, 0] > c[:, 2]
        c[sw] = c[sw][:, [2, 3, 0, 1]]
    else:
        flip = c[:, 0] > 0.5
        c[flip] = 1 - c[flip]
    return c


def random_sphere(r, n):
    v = r.normal(size=(n, 3))
    return v / np.linalg.norm(v, axis=1, keepdims=True)


# ------------------------------------------------------------------ 01 走査 ---
def case_01_scan():
    names = [n for n in os.listdir(DATA) if si.SV_INDEX_NAME_RE.fullmatch(n)]
    times = []
    for _ in range(2):
        t0 = time.perf_counter()
        cat = si.SvIndexCatalog(DATA)
        times.append(time.perf_counter() - t0)
    ok = len(cat.problems) == 0 and len(cat.files) == len(names)
    expect = {**{i: 1 for i in range(4107, 4120)}, **{i: 48 for i in range(5202, 5207)}}
    got = {k: len(v) for k, v in cat.series.items()}
    for k, n in expect.items():
        if k in got and got[k] != n:
            ok = False
    if len(names) == 253:
        ok = ok and got == expect
    lo, hi = cat.coverage_arcmin()
    msg = (f"{len(cat.files)} 本 / 名前の合うファイル {len(names)} 本、problems {len(cat.problems)}、"
           f"走査 {times[0] * 1000:.0f} ms(2 回目 {times[1] * 1000:.0f} ms)、段 {lo:.2f}′〜{hi:.1f}′")
    if cat.problems:
        msg += f"  例: {cat.problems[0]}"
    return ok, msg


# ----------------------------------------------------- 02 壊れたファイルの検査 ---
def _patch(path, old, new):
    with open(path, "r+b") as f:
        i = f.read().find(old)
        assert i >= 0, (path, old)
        f.seek(i)
        f.write(new)


def case_02_validate():
    with tempfile.TemporaryDirectory() as tmp:
        def cp(tag, name):
            p = os.path.join(tmp, name)
            shutil.copyfile(path_of(tag), p)
            return p
        cp("4119", "index-4119.fits")                                  # 正常
        cp("4119", "index-4118.fits")                                  # 名前と INDEXID が違う
        cp("5206-00", "index-5206-01.fits")                            # 名前とタイル番号が違う
        cp("5206-00", "index-5206.fits")                               # 全天の名前なのにタイル
        p = cp("4117", "index-4117.fits")                              # 途中で切れた
        with open(p, "r+b") as f:
            f.truncate(os.path.getsize(p) - 1000)
        p = cp("4116", "index-4116.fits")                              # ENDIAN が違う
        _patch(p, b"'04:03:02:01'", b"'01:02:03:04'")
        p = cp("4115", "index-4115.fits")                              # 後ろに余計なバイト
        with open(p, "ab") as f:
            f.write(b"\0" * 2880)
        with open(os.path.join(tmp, "index-4114.fits"), "wb") as f:   # FITS ではない
            f.write(rng.integers(0, 256, 8000, dtype=np.uint8).tobytes())
        p = cp("4113", "index-4113.fits")                              # sweep の表が無い
        _patch(p, b"TTYPE1  = 'sweep", b"TTYPE1  = 'sweeq")
        cp("4119", "index-4119.fits.part")                             # 名前の形が違う(無視される)
        cp("4119", "index-4119-copy.fits")

        expect = {"index-4118.fits": "系列", "index-5206-01.fits": "タイル番号",
                  "index-5206.fits": "全天", "index-4117.fits": "途中で切れ",
                  "index-4116.fits": "ENDIAN", "index-4115.fits": "合いません",
                  "index-4114.fits": "FITS ではありません", "index-4113.fits": "sweep"}
        bad, sample = [], ""
        for nm, word in expect.items():
            ok, msg = si.sv_validate_index_file(os.path.join(tmp, nm))
            if ok or word not in msg:
                bad.append(f"{nm}: {ok} {msg}")
            try:
                si.SvIndexFile(os.path.join(tmp, nm))
                bad.append(f"{nm}: SvIndexFile が例外にならない")
            except ValueError:
                pass
        ok0, msg0 = si.sv_validate_index_file(os.path.join(tmp, "index-4119.fits"))
        if not ok0:
            bad.append(f"正常なファイルが不合格: {msg0}")
        part = os.path.join(tmp, "index-4119.fits.part")             # 名前を変える前の検査
        if not si.sv_validate_index_file(part, name="index-4119.fits")[0]:
            bad.append(".part を name= で検査できない")
        if si.sv_validate_index_file(part, name="index-4118.fits")[0] or si.sv_validate_index_file(part)[0]:
            bad.append(".part の名前の検査が通ってしまう")
        cat = si.SvIndexCatalog(tmp)
        names = [f.name for f in cat.files]
        pnames = sorted(os.path.basename(p) for p, _ in cat.problems)
        if names != ["index-4119"]:
            bad.append(f"files = {names}")
        if pnames != sorted(expect):
            bad.append(f"problems = {pnames}")
        cat.close()
        for p in cat.problems[:1]:
            sample = f"{os.path.basename(p[0])}: {p[1]}"
        missing = si.SvIndexCatalog(os.path.join(tmp, "無いフォルダー"))
        if missing.files or len(missing.problems) != 1:
            bad.append("無いフォルダーで problems が 1 件にならない")
    if bad:
        return False, " / ".join(bad)
    return True, (f"壊れた 8 種を全部見分けた(例 {sample})、正常 1 本は「{msg0}」、名前の違う 2 本は無視、"
                  f".part は name= で検査できる")


# --------------------------------------------- 03 close でファイルを放す ---
def case_03_close_releases():
    with tempfile.TemporaryDirectory() as tmp:
        p = os.path.join(tmp, "index-4119.fits")
        shutil.copyfile(path_of("4119"), p)
        f = si.SvIndexFile(p)
        a = f.code_ball(f.codes([0, 1, 2]), 0.01)
        s = f.stars_in_cone(f.star_xyz(5), math.radians(30))
        locked = None
        if os.name == "nt":                      # 開いている間は消せないこと(テストに意味があるか)
            try:
                os.remove(p)
                locked = False
            except PermissionError:
                locked = True
        if locked is False:
            return False, "開いている間にファイルが消せた(mmap が働いていない?)"
        f.close()
        x = f.star_xyz([0, 1])                   # 閉じた後に使えばまた開く
        f.close()
        try:
            os.remove(p)
            removed = True
        except PermissionError as e:
            removed = False
            err = str(e)
        with f:
            pass
    if not removed:
        return False, f"close() の後もファイルが消せない: {err}"
    return True, (f"開いている間の削除は {'拒否' if locked else '(Windows 以外で未確認)'}、close() 後は削除できた"
                  f"(問い合わせ {len(a[0])} 件・円錐 {len(s)} 星・再オープン {x.shape})")


# --------------------------------------- 04 コードを星から計算し直す ---
def case_04_codes_recomputed():
    plan = [("4119", None), ("4115", None), ("4111", None), ("4110", None), ("5206-00", None),
            ("4107", 300_000), ("5202-00", 300_000)]
    rows, ok = [], True
    for tag, nsamp in plan:
        if not have(tag):
            rows.append(f"{tag} 無し")
            continue
        f = index(tag)
        t = f.codetree
        ids = (np.arange(f.nquads) if nsamp is None or nsamp >= f.nquads
               else np.unique(np.concatenate([[0, f.nquads - 1], rng.integers(0, f.nquads, nsamp)])))
        same = 0
        maxerr = 0.0
        for s in range(0, len(ids), 200_000):
            q = ids[s:s + 200_000]
            c = si.sv_index_codes_from_xyz(f.star_xyz(f.quad_stars(q)))
            enc = np.rint(t.to_tree(c)).astype(np.int64)
            st = t.data[q].astype(np.int64)
            same += int((enc == st).all(axis=1).sum())
            maxerr = max(maxerr, float(np.abs(c - t.to_real(st)).max()))
        ok = ok and same == len(ids)
        rows.append(f"{tag}{'(三角形)' if f.dimquads == 3 else ''} {same:,}/{len(ids):,}"
                    f"(最大差 {maxerr:.2e})")
    return ok, "整数まで一致: " + "、".join(rows)


# --------------------------------------------- 05 code_ball = 総当たり ---
def _brute_ball(C, Cs, order, qq, radius, pure):
    """総当たりの答え {(問い合わせ, 点)}。判定の式は code_ball の最終判定と同じ。

    pure = True は全点 C と比べる。False は第 1 成分で並べ替えた Cs(= C[order])から
    |c₀ − q₀| ≤ r の帯(必要条件。帯の外は必ず距離 > r)を切り出し、同じ式で比べる
    (木は使わない。速くして問い合わせの数を増やすため)。
    """
    want = set()
    r2 = radius * radius
    c0 = Cs[:, 0]
    for j, x in enumerate(qq):
        if pure:
            hit = np.nonzero(((C - x) ** 2).sum(axis=1) <= r2)[0]
        else:
            a = np.searchsorted(c0, x[0] - radius - 1e-9, "left")
            b = np.searchsorted(c0, x[0] + radius + 1e-9, "right")
            hit = order[a:b][((Cs[a:b] - x) ** 2).sum(axis=1) <= r2]
        want.update((j, int(h)) for h in hit)
    return want


def case_05_code_ball_bruteforce():
    plan = [("4107", 1000), ("5202-00", 1000), ("4111", 1000), ("4119", 400), ("5206-00", 1000), ("4110", 1000)]
    rows, ok = [], True
    for tag, n in plan:
        if not have(tag):
            rows.append(f"{tag} 無し")
            continue
        f = index(tag)
        t = f.codetree
        D = f.code_dim
        C = t.to_real(t.data)                     # 総当たり用に全部を実数へ(写し)
        order = np.argsort(C[:, 0], kind="stable")
        Cs = C[order]
        k = n // 4
        q = np.concatenate([
            C[rng.integers(0, len(C), k)] + rng.normal(0, 0.004, (k, D)),     # 保存値の近く
            random_codes(rng, k, D),                                          # 画像の quad らしいもの
            C[rng.integers(0, len(C), k)],                                    # 保存値ちょうど(距離 0)
            rng.uniform(-0.25, 1.25, (n - 3 * k, D)),                         # 範囲いっぱい(外も含む)
        ])
        q = q[rng.permutation(n)]
        npure = 0
        for radius in (0.01, 0.003, 0.03):
            qq = q if radius == 0.01 else q[: n // 4]
            qi, quad = f.code_ball(qq, radius)
            got = set(zip(qi.tolist(), quad.tolist()))
            want = _brute_ball(C, Cs, order, qq, radius, pure=False)
            m = 100 if len(C) < 1_000_000 else (20 if radius == 0.01 else 5)   # 先頭の何問かは全点と比べる
            pure = _brute_ball(C, Cs, order, qq[:m], radius, pure=True)
            npure += min(m, len(qq))
            same = (got == want and len(got) == len(qi) and np.all(np.diff(qi) >= 0)
                    and pure == {p for p in want if p[0] < m})
            ok = ok and same
            if not same:
                rows.append(f"{tag} r={radius}: 違う(木 {len(got)}、総当たり {len(want)}、"
                            f"木だけ {len(got - want)}、総当たりだけ {len(want - got)})")
            elif radius == 0.01:
                rows.append(f"{tag}(D={D}){len(qq)} 問 {len(want):,} 件")
        rows[-1] += f"(r = 0.003・0.03 も、うち {npure} 問は全点と比較)"
        del C, Cs
    # 1 問ずつ投げても一緒にまとめても同じ
    if have("4111"):
        f = index("4111")
        q = random_codes(rng, 50, 2)
        qi, quad = f.code_ball(q, 0.01)
        one = [f.code_ball(x, 0.01)[1] for x in q]
        same = all(np.array_equal(quad[qi == j], one[j]) for j in range(len(q)))
        ok = ok and same
        rows.append(f"まとめて投げる = 1 問ずつ: {same}")
    return ok, "総当たりと一致: " + "、".join(rows)


# ------------------------------------------- 06 stars_in_cone = 総当たり ---
def case_06_cone_bruteforce():
    plan = [("4107", 60, (0.05, 0.25, 1.0, 5.0)), ("5202-00", 60, (0.03, 0.2, 1.0, 4.0)),
            ("4119", 200, (1.0, 10.0, 40.0, 120.0)), ("5206-00", 200, (0.3, 2.0, 10.0, 30.0)),
            ("4111", 100, (0.5, 3.0, 20.0))]
    rows, ok = [], True
    for tag, n, radii in plan:
        if not have(tag):
            rows.append(f"{tag} 無し")
            continue
        f = index(tag)
        S = f.star_xyz(np.arange(f.nstars))
        cen = np.concatenate([S[rng.integers(0, len(S), n // 2)], random_sphere(rng, n - n // 2)])
        nfound, bad = 0, 0
        for j, c in enumerate(cen):
            c = si.sv_normalize(c)               # 関数の中と同じ正規化(境目の 1 ulp の違いを避ける)
            r = math.radians(radii[j % len(radii)])
            got = f.stars_in_cone(c, r)
            ch = 2 * math.sin(r / 2)
            want = np.nonzero(((S - c) ** 2).sum(axis=1) <= ch * ch)[0]
            if not np.array_equal(got, want):
                bad += 1
            nfound += len(want)
        # 端の場合: 半径 0・負・π 以上
        c7 = si.sv_normalize(S[7])
        edge = (np.array_equal(f.stars_in_cone(S[7], 0.0), np.nonzero(((S - c7) ** 2).sum(axis=1) <= 0.0)[0])
                and len(f.stars_in_cone(S[7], -1.0)) == 0
                and len(f.stars_in_cone(S[7], math.pi)) == f.nstars)
        ok = ok and bad == 0 and edge
        rows.append(f"{tag} {n} 回 {nfound:,} 星{'' if bad == 0 else f' 不一致 {bad}'}{'' if edge else ' 端の場合 NG'}")
        del S
    return ok, "総当たりと一致: " + "、".join(rows)


# --------------------------------------------------- 07 HEALPix(xy 方式) ---
def _spread_bits(v):
    out = np.zeros_like(v)
    for b in range(31):
        out |= ((v >> b) & 1) << (2 * b)
    return out


def _compact_bits(v):
    out = np.zeros_like(v)
    for b in range(31):
        out |= ((v >> (2 * b)) & 1) << b
    return out


def case_07_healpix_vs_astropy():
    try:
        from astropy_healpix import healpix_to_lonlat, lonlat_to_healpix
        import astropy.units as u
    except ImportError:
        return None, "astropy_healpix が無い"
    rows, ok = [], True
    for nside in (1, 2, 4, 8, 64, 1024):
        v = random_sphere(rng, 200_000)
        lon = np.arctan2(v[:, 1], v[:, 0])
        lat = np.arcsin(v[:, 2])
        nest = np.asarray(lonlat_to_healpix(lon * u.rad, lat * u.rad, nside, order="nested")).astype(np.int64)
        n2 = nside * nside
        face, sub = nest // n2, nest % n2
        want = face * n2 + _compact_bits(sub) * nside + _compact_bits(sub >> 1)
        got = si.sv_healpix_xy_from_xyz(v, nside)
        same = int((got == want).sum())
        # 中心: xy 番号 → NESTED に直して astropy の中心と比べる
        hp = np.arange(12 * n2) if n2 <= 64 else rng.integers(0, 12 * n2, 5000)
        f_, x_, y_ = hp // n2, (hp % n2) // nside, hp % nside
        lo_, la_ = healpix_to_lonlat(f_ * n2 + _spread_bits(x_) + 2 * _spread_bits(y_), nside, order="nested")
        ref = np.stack([np.cos(la_.rad) * np.cos(lo_.rad), np.cos(la_.rad) * np.sin(lo_.rad), np.sin(la_.rad)], -1)
        dc = float(np.max(si.sv_angle_between(si.sv_healpix_xy_center(hp, nside), ref)))
        ok = ok and same == len(v) and dc < 1e-9
        rows.append(f"nside {nside}: {same}/{len(v)}・中心の差 {dc:.1e} rad")
    # 極・赤道帯の境目・経度 0 の付近でも範囲内の番号が出る(境界ちょうどの点の割り振りは比べない)
    sp = np.array([[0, 0, 1], [0, 0, -1], [1, 0, 0], [1, -1e-17, 0], [0.6, 0, 0.8], [0.6, -1e-15, -0.8]])
    for nside in (1, 2, 3, 1024):
        h = si.sv_healpix_xy_from_xyz(sp, nside)
        ok = ok and bool(np.all((h >= 0) & (h < 12 * nside * nside)))
    return ok, "astropy_healpix(NESTED を xy に並べ替え)と一致: " + "、".join(rows)


def case_08_healpix_self():
    rows, ok = [], True
    # 1) 中心 → 番号の往復と、ピクセルの中の点(nside が 2 の累乗でないものも)
    for nside in (1, 2, 3, 5, 7, 16):
        hp = np.arange(12 * nside * nside)
        back = si.sv_healpix_xy_from_xyz(si.sv_healpix_xy_center(hp, nside), nside)
        dx = rng.uniform(0.01, 0.99, (len(hp), 8))
        dy = rng.uniform(0.01, 0.99, (len(hp), 8))
        pts = si.sv_healpix_xy_point(hp[:, None], nside, dx, dy)
        back2 = si.sv_healpix_xy_from_xyz(pts, nside)
        good = np.array_equal(back, hp) and np.array_equal(back2, np.repeat(hp[:, None], 8, 1))
        ok = ok and good
        if not good:
            rows.append(f"nside {nside} の往復 NG")
    # 2) index-format.md 12 節の表(nside 2 の中心)
    table = {0: (45.0, 19.47), 1: (22.5, 41.81), 3: (45.0, 66.44), 17: (337.5, 0.0), 25: (157.5, 0.0),
             36: (135.0, -66.44), 40: (225.0, -66.44), 47: (315.0, -19.47)}
    worst = 0.0
    for hp, (ra, dec) in table.items():
        r, d = si.sv_xyz_to_radec(si.sv_healpix_xy_center(hp, 2))
        worst = max(worst, abs((r - ra + 180) % 360 - 180), abs(d - dec))
    ok = ok and worst < 0.01
    rows.append(f"表の中心と最大 {worst:.3f}°")
    # 3) 外接半径は境界を細かく調べた最大以上、かつ 1 % 以内
    ratios = []
    for nside in (2, 3):
        for hp in range(12 * nside * nside):
            t = np.linspace(0, 1, 2001)
            z, o = np.zeros_like(t), np.ones_like(t)
            e = si.sv_healpix_xy_point(hp, nside, np.concatenate([t, o, t, z]), np.concatenate([z, t, o, t]))
            dense = float(np.max(si.sv_angle_between(e, si.sv_healpix_xy_center(hp, nside)[None, :])))
            ratios.append(si.sv_healpix_xy_radius(hp, nside) / dense)
    ok = ok and min(ratios) >= 1.0 and max(ratios) < 1.01
    rows.append(f"外接半径 / 細かく調べた最大 = {min(ratios):.5f}〜{max(ratios):.5f}")
    r2 = [math.degrees(si.sv_healpix_xy_radius(h, 2)) for h in range(48)]
    rows.append(f"nside 2 の外接半径 {min(r2):.2f}°〜{max(r2):.2f}°")
    return ok, "、".join(rows)


def case_09_tiles_all_stars():
    rows, ok = [], True
    n_tiles = n_stars = 0
    worst_ratio = 0.0
    bad = []
    for s in ("5204", "5205", "5206"):
        for hp in range(48):
            tag = f"{s}-{hp:02d}"
            if not have(tag):
                bad.append(f"{tag} 無し")
                ok = False
                continue
            with si.SvIndexFile(path_of(tag)) as f:
                S = f.star_xyz(np.arange(f.nstars))
                h = si.sv_healpix_xy_from_xyz(S, f.hpnside)
                ang = si.sv_angle_between(S, f.tile_center_xyz[None, :])
                ratio = float(ang.max() / f.tile_radius_rad)
                worst_ratio = max(worst_ratio, ratio)
                if f.healpix != hp or not np.all(h == f.healpix) or ratio >= 1.0:
                    bad.append(f"{tag}: 番号の一致 {np.mean(h == f.healpix):.4f}・半径比 {ratio:.4f}")
                    ok = False
                n_tiles += 1
                n_stars += f.nstars
                del S
    rows.append(f"{n_tiles} タイル・{n_stars:,} 星が全部自分のタイル(xy 方式)に入り、"
                f"中心からの最大距離 / 外接半径 = {worst_ratio:.4f}")
    if bad:
        rows.append("NG: " + "; ".join(bad[:5]))
    return ok and n_tiles == 144, "、".join(rows)


# --------------------------------------------------------- 10 select ---
def _tiles_hit_by_cap(center, radius, nside, n=4000):
    """円(center, radius)の中の点を乱数で取り、入るタイルの番号の集合を返す。"""
    e = np.cross(center, [0.0, 0.0, 1.0] if abs(center[2]) < 0.9 else [1.0, 0.0, 0.0])
    e /= np.linalg.norm(e)
    w = np.cross(center, e)
    th = np.arccos(1 - rng.uniform(0, 1, n) * (1 - math.cos(radius)))
    ph = rng.uniform(0, 2 * np.pi, n)
    p = (np.cos(th)[:, None] * center + np.sin(th)[:, None] * (np.cos(ph)[:, None] * e + np.sin(ph)[:, None] * w))
    return set(si.sv_healpix_xy_from_xyz(p, nside).tolist())


def case_10_select():
    cat = si.SvIndexCatalog(DATA)
    rows, ok = [], True

    def series_of(sel):
        out = []
        for f in sel:
            if not out or out[-1] != f.indexid:
                out.append(f.indexid)
        return out

    # 968×548・1.4616″/px(実写の正解)。ちょうど / ±25 %
    sel = cat.select(1.4616, 1.4616, 968, 548)
    s1 = series_of(sel)
    sel25 = cat.select(1.4616 * 0.75, 1.4616 * 1.25, 968, 548)
    s25 = series_of(sel25)
    want_in = {5204, 5205, 5206, 4107}
    good = (want_in <= set(s1) and not ({4108, 4109, 4110} & set(s1)) and (want_in | {4108}) <= set(s25)
            and not ({4109, 4110, 4111} & set(s25)) and s1[0] in (5203, 5204)
            and len([f for f in sel if f.indexid == 5205]) == 48)
    ok = ok and good
    rows.append(f"968×548・1.4616″/px → {s1}(先頭 {s1[0]})、±25 % → {s25}")

    # ヒントあり: 選んだタイルが、ヒントの円(+ 対角の半分)に入る点のタイルを全部含む
    half_diag = 0.5 * math.hypot(968, 548) * 1.4616 * 1.25 / si.SV_ARCSEC_PER_RAD
    worst = []
    for ra, dec in ((45.0, 19.47), (45.0, 0.0), (0.0, 41.81), (0.3, 89.9), (200.0, -60.0), (359.9, -0.1)):
        hint = si.sv_radec_to_xyz(ra, dec)
        for hr in (0.0, math.radians(1.0), math.radians(5.0)):
            sel = cat.select(1.4616 * 0.75, 1.4616 * 1.25, 968, 548, hint_xyz=hint, hint_radius_rad=hr)
            need = _tiles_hit_by_cap(hint, hr + half_diag, 2)
            for iid in (5204, 5205, 5206):
                got = {f.healpix for f in sel if f.indexid == iid}
                if not need <= got:
                    ok = False
                    worst.append(f"({ra},{dec},{hr:.3f}) {iid}: 要 {sorted(need)} 選 {sorted(got)}")
            if not any(f.indexid == 4107 for f in sel):
                ok = False
                worst.append("全天の 4107 が落ちた")
    n5 = len([f for f in cat.select(1.4616, 1.4616, 968, 548, hint_xyz=si.sv_radec_to_xyz(45.0, 19.47),
                                    hint_radius_rad=math.radians(1.0)) if f.indexid == 5205])
    rows.append(f"ヒント 6 か所 × 半径 3 通りで必要なタイルを全部含む{'' if not worst else ' NG ' + '; '.join(worst[:3])}"
                f"(タイルの中心・半径 1° なら 5205 は {n5} タイル)")

    # 重なる段が無いときは最も近い段 1 つ
    tiny = series_of(cat.select(0.05, 0.05, 968, 548))
    huge = series_of(cat.select(5000.0, 5000.0, 968, 548))
    wide = series_of(cat.select(300.0, 300.0, 968, 548))
    good = tiny == [5202] and huge == [4119] and set(wide) <= set(range(4110, 4120)) and 4119 in wide
    ok = ok and good
    rows.append(f"0.05″/px → {tiny}、5000″/px → {huge}、300″/px → {wide}")
    cov = cat.coverage_arcmin()
    ok = ok and abs(cov[0] - 4.0) < 1e-6 and abs(cov[1] - 2000.0) < 1e-6
    try:
        cat.select(2.0, 1.0, 968, 548)
        ok = False
        rows.append("scale_lo > scale_hi が通った")
    except ValueError:
        pass
    cat.close()
    return ok, "、".join(rows)


# --------------------------------------------------------- 11 tiles_near ---
def case_11_tiles_near():
    cat = si.SvIndexCatalog(DATA)
    tiles = cat.series[5206]
    rows, ok = [], True
    ncone = nmiss = 0
    for c in np.concatenate([random_sphere(rng, 40), [si.sv_healpix_xy_point(0, 2, 0.0, 0.0),
                                                       si.sv_healpix_xy_point(5, 2, 1.0, 0.5)]]):
        for rdeg in (0.3, 2.0, 8.0):
            r = math.radians(rdeg)
            near = {f.healpix for f in cat.tiles_near(5206, c, r)}
            have_stars = {f.healpix for f in tiles if len(f.stars_in_cone(c, r))}
            ncone += 1
            if not have_stars <= near:
                nmiss += 1
    allsky = cat.tiles_near(4107, si.sv_radec_to_xyz(10, 10), 0.01)
    ok = nmiss == 0 and len(allsky) == 1 and allsky[0].indexid == 4107 and cat.tiles_near(9999, [1, 0, 0], 1) == []
    rows.append(f"{ncone} 個の円で、星のあるタイルは全部 tiles_near に入る{'' if nmiss == 0 else f'(NG {nmiss})'}")
    cat.close()
    return ok, "、".join(rows)


# ----------------------------------------------------------- 12 速さ ---
def _bench_queries(f, n=1000, sigma=0.003):
    t = f.codetree
    k = n // 2
    q = np.concatenate([t.to_real(t.data[rng.integers(0, f.nquads, k)]) + rng.normal(0, sigma, (k, f.code_dim)),
                        random_codes(rng, n - k, f.code_dim)])
    t0 = time.perf_counter()
    res = f.code_ball(q, 0.01)
    first = time.perf_counter() - t0
    best = min(_timeit(lambda: f.code_ball(q, 0.01)) for _ in range(3))
    return first, best, len(res[0])


def _timeit(fn):
    t0 = time.perf_counter()
    fn()
    return time.perf_counter() - t0


def case_12_speed():
    try:
        import psutil
        proc = psutil.Process()
    except ImportError:
        proc = None

    def mem():
        if proc is None:
            return (0.0, 0.0)
        m = proc.memory_info()
        return m.rss / 2 ** 20, getattr(m, "private", m.rss) / 2 ** 20

    rows, ok = [], True
    m0 = mem()
    t0 = time.perf_counter()
    cat = si.SvIndexCatalog(DATA)
    scan = time.perf_counter() - t0
    m1 = mem()
    rows.append(f"走査 {scan * 1000:.0f} ms(RSS {m1[0] - m0[0]:+.1f} MiB)")
    by = {f.name: f for f in cat.files}
    radius = math.hypot(968, 548) / 2 * 1.4616 * 1.05 / si.SV_ARCSEC_PER_RAD
    for name in ("index-4107", "index-5205-02", "index-4111"):
        if name not in by:
            rows.append(f"{name} 無し")
            continue
        f = by[name]
        ma = mem()
        first, best, nhit = _bench_queries(f)
        mb = mem()
        if name == "index-4107":
            ok = ok and best <= 0.5
        S = f.star_xyz(rng.integers(0, f.nstars, 100))
        cones = [_timeit(lambda c=c: f.stars_in_cone(c, radius)) for c in S]
        nst = np.mean([len(f.stars_in_cone(c, radius)) for c in S[:20]])
        rows.append(f"{name}: 1000 問 {best * 1000:.1f} ms(初回 {first * 1000:.1f} ms、{nhit} 件、"
                    f"RSS {mb[0] - ma[0]:+.1f} / private {mb[1] - ma[1]:+.1f} MiB)、"
                    f"円錐 {math.degrees(radius) * 60:.1f}′ 1 回 {np.median(cones) * 1000:.2f} ms(平均 {nst:.0f} 星)")
    # ブラインドの重い場面の目安: 5205 の 48 タイルに 1000 問ずつ
    fs = cat.series.get(5205, [])
    if len(fs) == 48:
        ma = mem()
        q = random_codes(rng, 1000, 4)
        tt = _timeit(lambda: [f.code_ball(q, 0.01) for f in fs])
        mb = mem()
        rows.append(f"5205 の 48 タイルに 1000 問ずつ {tt:.2f} s(RSS {mb[0] - ma[0]:+.0f} / private {mb[1] - ma[1]:+.1f} MiB)")
    cat.close()
    m2 = mem()
    rows.append(f"全部閉じた後 RSS {m2[0]:.0f} MiB / private {m2[1]:.0f} MiB(開始時 {m0[0]:.0f} / {m0[1]:.0f})")
    return ok, "、".join(rows)


CASES = [
    ("01 253 本の走査(problems 0)", case_01_scan, ()),
    ("02 壊れた・名前の違うファイルを検査で見分ける", case_02_validate,
     ("4119", "4117", "4116", "4115", "4113", "5206-00")),
    ("03 close() でファイルを放す(Windows のロック)", case_03_close_releases, ("4119",)),
    ("04 コードを星から計算し直すと保存値と整数まで一致", case_04_codes_recomputed, ("4119",)),
    ("05 code_ball = 総当たり(D=4 と三角形)", case_05_code_ball_bruteforce, ("4119",)),
    ("06 stars_in_cone = 総当たり", case_06_cone_bruteforce, ("4119",)),
    ("07 xy 方式の HEALPix = astropy_healpix", case_07_healpix_vs_astropy, ()),
    ("08 HEALPix の往復・中心の表・外接半径", case_08_healpix_self, ()),
    ("09 5204〜5206 の 144 タイル: 星は自分のタイル・外接半径の内", case_09_tiles_all_stars, ("5206-00",)),
    ("10 select(段とタイルの選び方)", case_10_select, ("4107", "5205-00")),
    ("11 tiles_near(境界をまたぐ視野)", case_11_tiles_near, ("5206-00", "4107")),
    ("12 速さとメモリ(4107 に 1000 問 0.5 秒以内)", case_12_speed, ("4107",)),
]


def main():
    try:
        sys.stdout.reconfigure(errors="replace")    # cp932 の画面でも止まらないように
    except Exception:
        pass
    t0 = time.perf_counter()
    for name, fn, tags in CASES:
        if fn is case_07_healpix_vs_astropy or fn is case_08_healpix_self:
            # 索引が無くても走る(HEALPix の式だけ)
            t1 = time.perf_counter()
            try:
                ok, msg = fn()
            except Exception as e:
                ok, msg = False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}"
            RESULTS.append((name, ok, f"{msg}  [{time.perf_counter() - t1:.1f} s]"))
        else:
            run_case(name, fn, *tags)
        print(f"  ... {name}", flush=True)
    for f in _FILES.values():
        f.close()
    width = max(len(r[0]) for r in RESULTS)
    n_ok = n_skip = 0
    print()
    for name, ok, msg in RESULTS:
        if ok is None:
            n_skip += 1
            tag = "SKIP"
        else:
            n_ok += bool(ok)
            tag = "PASS" if ok else "FAIL"
        print(f"  {tag}  {name.ljust(width)}  {msg}")
    n_run = len(RESULTS) - n_skip
    print(f"\n  {n_ok}/{n_run} passed, {n_skip} skipped  ({time.perf_counter() - t0:.0f} s)")
    return 0 if n_ok == n_run else 1


if __name__ == "__main__":
    sys.exit(main())

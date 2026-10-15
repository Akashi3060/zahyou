# -*- coding: utf-8 -*-
"""
zahyou-index(Rust)の照合用の値を Python 基準実装(dev/solver_index.py)で作る。

    python gen_index.py        # index_*.json を書く(星図データ C:\\AstrometryData が要る。ZAHYOU_INDEX_DIR で変えられる)

書くもの(このスクリプトと同じフォルダー):
  index_scan.json         253 本の走査: 名前の順の files(ヘッダーの値・タイルの中心と外接半径・検査の要約)、
                          problems、series、coverage_arcmin
  index_broken.json       壊れたファイル 8 種(dev/test_solver_index.py の case_02 と同じ作り方)の検査の文と、
                          そのフォルダーの走査結果
  index_healpix.json      xy 方式の HEALPix: 乱数の点 → 番号(nside 8 通り)、nside 2・3 の全ピクセルの中心と外接半径、
                          ピクセルの中の点
  index_ball_<tag>.json   索引 1 本ごと: code_ball の問い合わせと答え(問い合わせごとの件数と sha256)、
                          stars_in_cone の中心・半径と答え、星と quad の値の見本
  index_select.json       select / tiles_near の入力と答え(index_scan.json の files の番号の並び)

照合の約束(Rust の試験 crates/zahyou-index/tests/python_match.rs と同じ):
  - code_ball の答えは (問い合わせの番号, quad の番号) を '<u4' で交互に並べたバイト列の sha256(16 進)
  - stars_in_cone の答えは星の番号の並び('<u4')の sha256
  - 実数は repr(最短で元に戻る桁)。Rust は serde_json の float_roundtrip で同じビットに戻す
Python 版のリポジトリは環境変数 ZAHYOU_DEV で変えられる(既定 C:\\Users\\yoshi\\Documents\\GitHub\\zahyou\\dev)。
"""
import hashlib
import json
import math
import os
import shutil
import sys
import tempfile
import time
import warnings

import numpy as np

warnings.simplefilter("ignore")

HERE = os.path.dirname(os.path.abspath(__file__))
ZAHYOU_DEV = os.environ.get("ZAHYOU_DEV", r"C:\Users\yoshi\Documents\GitHub\zahyou\dev")
sys.path.insert(0, ZAHYOU_DEV)
import solver_index as si                                  # noqa: E402

DATA = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
rng = np.random.default_rng(20261015)

BALL_TAGS = [("4119", 400), ("4115", 400), ("4111", 1000), ("4107", 1000),
             ("5206-00", 1000), ("5205-02", 1000), ("5202-00", 1000)]
CONE_PLAN = {"4119": (200, (1.0, 10.0, 40.0, 120.0)), "4115": (100, (0.5, 3.0, 20.0, 60.0)),
             "4111": (100, (0.5, 3.0, 20.0)), "4107": (60, (0.05, 0.25, 1.0, 5.0)),
             "5206-00": (200, (0.3, 2.0, 10.0, 30.0)), "5205-02": (60, (0.1, 1.0, 5.0, 30.0)),
             "5202-00": (60, (0.03, 0.2, 1.0, 4.0))}
# 速さの物差しの円錐(test_solver_index の case_12 と同じ半径: 968×548・1.4616″/px の対角の半分 × 1.05)
BENCH_CONE_RAD = math.hypot(968, 548) / 2 * 1.4616 * 1.05 / si.SV_ARCSEC_PER_RAD


def path_of(tag):
    return os.path.join(DATA, f"index-{tag}.fits")


def sha_u32(a):
    return hashlib.sha256(np.ascontiguousarray(np.asarray(a, dtype="<u4")).tobytes()).hexdigest()


def fl(a):
    """numpy の値を JSON の素の値に(実数は float の repr で書かれる)。"""
    return np.asarray(a, dtype=np.float64).tolist()


def dump(name, obj):
    p = os.path.join(HERE, name)
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    print(f"  {name}: {os.path.getsize(p) / 1024:.0f} KiB")


def random_codes(r, n, D):
    """dev/test_solver_index.py の random_codes と同じ(画像側の quad らしい正準形のコード)。"""
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


def file_record(f, msg):
    return dict(name=f.name, indexid=f.indexid, healpix=f.healpix, hpnside=f.hpnside, dimquads=f.dimquads,
                nquads=f.nquads, nstars=f.nstars, scale_lo_rad=f.scale_lo_rad, scale_hi_rad=f.scale_hi_rad,
                jitter_arcsec=f.jitter_arcsec, cutnside=f.cutnside,
                tile_center=None if f.tile_center_xyz is None else fl(f.tile_center_xyz),
                tile_radius_rad=f.tile_radius_rad, validate=msg)


# ------------------------------------------------------------------ 走査 ---
def gen_scan():
    t0 = time.perf_counter()
    cat = si.SvIndexCatalog(DATA)
    t_scan = time.perf_counter() - t0
    names = sorted(n for n in os.listdir(DATA) if si.SV_INDEX_NAME_RE.fullmatch(n))
    files = []
    for f in cat.files:
        ok, msg = si.sv_validate_index_file(f.path)
        assert ok, msg
        files.append(file_record(f, msg))
    lo, hi = cat.coverage_arcmin()
    out = dict(n_names=len(names), files=files,
               problems=[[os.path.basename(p), m] for p, m in cat.problems],
               series={str(k): [f.healpix for f in v] for k, v in cat.series.items()},
               coverage_arcmin=[lo, hi], python_scan_seconds=t_scan)
    dump("index_scan.json", out)
    return cat


# ------------------------------------------------------- 壊れたファイル ---
def _patch(path, old, new):
    with open(path, "r+b") as f:
        i = f.read().find(old)
        assert i >= 0, (path, old)
        f.seek(i)
        f.write(new)


def gen_broken():
    junk = rng.integers(0, 256, 8000, dtype=np.uint8).tobytes()
    # 先頭に引用符と制御文字と ASCII 以外を入れて、repr の書き方も比べる
    junk = b"\x00'AB\x7f\x80\t" + junk[8:]
    with tempfile.TemporaryDirectory() as tmp:
        def cp(tag, name):
            p = os.path.join(tmp, name)
            shutil.copyfile(path_of(tag), p)
            return p
        cp("4119", "index-4119.fits")
        cp("4119", "index-4118.fits")
        cp("5206-00", "index-5206-01.fits")
        cp("5206-00", "index-5206.fits")
        p = cp("4117", "index-4117.fits")
        with open(p, "r+b") as f:
            f.truncate(os.path.getsize(p) - 1000)
        p = cp("4116", "index-4116.fits")
        _patch(p, b"'04:03:02:01'", b"'01:02:03:04'")
        p = cp("4115", "index-4115.fits")
        with open(p, "ab") as f:
            f.write(b"\0" * 2880)
        with open(os.path.join(tmp, "index-4114.fits"), "wb") as f:
            f.write(junk)
        p = cp("4113", "index-4113.fits")
        _patch(p, b"TTYPE1  = 'sweep", b"TTYPE1  = 'sweeq")
        cp("4119", "index-4119.fits.part")
        cp("4119", "index-4119-copy.fits")
        # 追加: ヘッダーの値の型の違い(整数の欄に実数・文字列)、SCALE の範囲、名前の 2 桁が範囲の外
        p = cp("4112", "index-4112.fits")
        _patch(p, b"DIMQUADS=                    3", b"DIMQUADS=                  3.0")
        p = cp("4110", "index-4110.fits")
        _patch(p, b"KDT_NNOD=", b"KDT_NNOX=")
        p = cp("4109", "index-4109.fits")
        _patch(p, b"KDT_EXT = 'double", b"KDT_EXT = 'single")
        files = sorted(n for n in os.listdir(tmp))
        checks = {}
        for nm in files:
            ok, msg = si.sv_validate_index_file(os.path.join(tmp, nm))
            try:
                si.SvIndexFile(os.path.join(tmp, nm))
                opened = True
            except ValueError:
                opened = False
            checks[nm] = dict(ok=ok, message=msg, opens=opened)
        part = os.path.join(tmp, "index-4119.fits.part")
        named = {n: list(si.sv_validate_index_file(part, name=n)) for n in ("index-4119.fits", "index-4118.fits")}
        cat = si.SvIndexCatalog(tmp)
        out = dict(junk_hex=junk.hex(), checks=checks, part_named=named,
                   catalog_files=[f.name for f in cat.files],
                   catalog_problems=[[os.path.basename(p), m] for p, m in cat.problems])
        cat.close()
        missing = si.SvIndexCatalog(os.path.join(tmp, "無いフォルダー"))
        out["missing_dir_problems"] = [m for _, m in missing.problems]
    dump("index_broken.json", out)


# ------------------------------------------------------------- HEALPix ---
def gen_healpix():
    pts = random_sphere(rng, 1000)
    special = np.array([[0, 0, 1], [0, 0, -1], [1, 0, 0], [0, 1, 0], [-1, 0, 0], [0.6, 0, 0.8], [0.6, 0, -0.8],
                        [1, 1, 1], [-1, -1, -1], [3.0, -4.0, 12.0]], dtype=np.float64)
    pts = np.concatenate([pts, special])
    nsides = [1, 2, 3, 4, 5, 8, 64, 1024]
    hps = {str(n): si.sv_healpix_xy_from_xyz(pts, n).tolist() for n in nsides}
    cen = {}
    for n in (2, 3):
        hp = np.arange(12 * n * n)
        cen[str(n)] = dict(center=fl(si.sv_healpix_xy_center(hp, n)),
                           radius=[si.sv_healpix_xy_radius(int(h), n) for h in hp])
    hp = rng.integers(0, 12 * 9, 200)
    dx = rng.uniform(0, 1, 200)
    dy = rng.uniform(0, 1, 200)
    inner = dict(nside=3, hp=hp.tolist(), dx=fl(dx), dy=fl(dy), xyz=fl(si.sv_healpix_xy_point(hp, 3, dx, dy)))
    dump("index_healpix.json", dict(points=fl(pts), nsides=nsides, hp=hps, centers=cen, inner=inner))


# --------------------------------------------- code_ball・円錐・値の見本 ---
def gen_ball(tag, n):
    f = si.SvIndexFile(path_of(tag))
    t = f.codetree
    D = f.code_dim
    C = t.to_real(t.data)
    k = n // 4
    q = np.concatenate([
        C[rng.integers(0, len(C), k)] + rng.normal(0, 0.004, (k, D)),     # 保存値の近く
        random_codes(rng, k, D),                                          # 画像の quad らしいもの
        C[rng.integers(0, len(C), k)],                                    # 保存値ちょうど(距離 0)
        rng.uniform(-0.25, 1.25, (n - 3 * k, D)),                         # 範囲いっぱい(外も含む)
    ])
    q = q[rng.permutation(n)]
    del C
    balls = []
    for radius in (0.01, 0.003, 0.03):
        qq = q if radius == 0.01 else q[: n // 4]
        t0 = time.perf_counter()
        qi, quad = f.code_ball(qq, radius)
        dt = time.perf_counter() - t0
        counts = np.bincount(qi, minlength=len(qq))
        balls.append(dict(radius=radius, nq=len(qq), total=int(len(qi)), counts=counts.tolist(),
                          sha256=sha_u32(np.stack([qi, quad], 1).ravel()),
                          first=[quad[qi == j].tolist() for j in range(3)], python_seconds=dt))
    # 円錐
    S = f.star_xyz(np.arange(f.nstars))
    ncone, radii = CONE_PLAN[tag]
    cen = np.concatenate([S[rng.integers(0, len(S), ncone // 2)], random_sphere(rng, ncone - ncone // 2)])
    cones = []
    for j, c in enumerate(cen):
        r = math.radians(radii[j % len(radii)])
        got = f.stars_in_cone(c, r)
        cones.append(dict(center=fl(c), radius=r, count=int(len(got)), sha256=sha_u32(got),
                          ids=got.tolist() if len(got) <= 40 else None))
    bc = S[rng.integers(0, len(S))]
    got = f.stars_in_cone(bc, BENCH_CONE_RAD)
    bench_cone = dict(center=fl(bc), radius=BENCH_CONE_RAD, count=int(len(got)), sha256=sha_u32(got))
    del S
    # 値の見本(星・quad)
    sid = np.concatenate([[0, f.nstars - 1], rng.integers(0, f.nstars, 30)])
    qid = np.concatenate([[0, f.nquads - 1], rng.integers(0, f.nquads, 30)])
    samples = dict(star_ids=sid.tolist(), star_xyz=fl(f.star_xyz(sid)), sweep=f.sweep(sid).tolist(),
                   quad_ids=qid.tolist(), quad_stars=f.quad_stars(qid).tolist(), codes=fl(f.codes(qid)),
                   codes_from_xyz=fl(si.sv_index_codes_from_xyz(f.star_xyz(f.quad_stars(qid)))))
    out = dict(tag=tag, name=f.name, code_dim=D, queries=fl(q.ravel()), balls=balls, cones=cones,
               bench_cone=bench_cone, samples=samples)
    f.close()
    dump(f"index_ball_{tag}.json", out)


# ------------------------------------------------------ select・tiles_near ---
def gen_select(cat):
    pos = {id(f): i for i, f in enumerate(cat.files)}

    def ids(sel):
        return [pos[id(f)] for f in sel]

    cases = []

    def add(lo, hi, w, h, hint=None, hr=None):
        try:
            sel = ids(cat.select(lo, hi, w, h, hint_xyz=hint, hint_radius_rad=hr))
            err = None
        except ValueError as e:
            sel, err = None, str(e)
        cases.append(dict(scale_lo=lo, scale_hi=hi, width=w, height=h,
                          hint=None if hint is None else fl(hint), hint_radius=hr, result=sel, error=err))

    s = 1.4616
    add(s, s, 968, 548)
    add(s * 0.75, s * 1.25, 968, 548)
    for ra, dec in ((45.0, 19.47), (45.0, 0.0), (0.0, 41.81), (0.3, 89.9), (200.0, -60.0), (359.9, -0.1)):
        hint = si.sv_radec_to_xyz(ra, dec)
        for hr in (0.0, math.radians(1.0), math.radians(5.0)):
            add(s * 0.75, s * 1.25, 968, 548, hint, hr)
    add(s, s, 968, 548, si.sv_radec_to_xyz(45.0, 19.47), math.radians(1.0))
    add(s, s, 968, 548, si.sv_radec_to_xyz(45.0, 19.47), None)
    add(0.05, 0.05, 968, 548)
    add(5000.0, 5000.0, 968, 548)
    add(300.0, 300.0, 968, 548)
    add(2.0, 1.0, 968, 548)                     # scale_lo > scale_hi
    add(1.0, 2.0, 0, 548)                       # 幅 0
    for _ in range(40):
        lo = float(np.exp(rng.uniform(np.log(0.1), np.log(400.0))))
        hi = lo * float(rng.uniform(1.0, 3.0))
        w, h = int(rng.integers(100, 8000)), int(rng.integers(100, 6000))
        if rng.uniform() < 0.6:
            add(lo, hi, w, h, random_sphere(rng, 1)[0], float(rng.uniform(0, 0.2)))
        else:
            add(lo, hi, w, h)
    near = []
    centers = np.concatenate([random_sphere(rng, 40), [si.sv_healpix_xy_point(0, 2, 0.0, 0.0),
                                                       si.sv_healpix_xy_point(5, 2, 1.0, 0.5)]])
    for c in centers:
        for rdeg in (0.3, 2.0, 8.0):
            near.append(dict(indexid=5206, center=fl(c), radius=math.radians(rdeg),
                             result=ids(cat.tiles_near(5206, c, math.radians(rdeg)))))
    near.append(dict(indexid=4107, center=fl(si.sv_radec_to_xyz(10, 10)), radius=0.01,
                     result=ids(cat.tiles_near(4107, si.sv_radec_to_xyz(10, 10), 0.01))))
    near.append(dict(indexid=9999, center=[1.0, 0.0, 0.0], radius=1.0, result=ids(cat.tiles_near(9999, [1, 0, 0], 1))))
    dump("index_select.json", dict(select=cases, tiles_near=near))


def main():
    if not os.path.isdir(DATA):
        print(f"{DATA} がありません")
        return 1
    t0 = time.perf_counter()
    cat = gen_scan()
    gen_select(cat)
    cat.close()
    gen_broken()
    gen_healpix()
    for tag, n in BALL_TAGS:
        gen_ball(tag, n)
    print(f"  ({time.perf_counter() - t0:.0f} s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

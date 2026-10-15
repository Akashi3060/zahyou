# -*- coding: utf-8 -*-
"""
zahyou-index(Rust)の速さの物差しの Python 側。crates/zahyou-index/examples/bench_index.rs と同じ項目を、
Python 基準実装(dev/solver_index.py)で同じ問い合わせについて測る。

    python bench_index.py [結果.json]

問い合わせは index_ball_<tag>.json(gen_index.py が作ったもの。半径 0.01 の 1000 問と bench_cone)。
答えの件数と sha256 も基準値と比べる。星図データは C:\\AstrometryData(ZAHYOU_INDEX_DIR)。
プライベートメモリは Windows の PrivateUsage(GetProcessMemoryInfo)。
"""
import ctypes
import hashlib
import json
import os
import statistics
import sys
import time
import warnings
from ctypes import wintypes

import numpy as np

warnings.simplefilter("ignore")

HERE = os.path.dirname(os.path.abspath(__file__))
ZAHYOU_DEV = os.environ.get("ZAHYOU_DEV", r"C:\Users\yoshi\Documents\GitHub\zahyou\dev")
sys.path.insert(0, ZAHYOU_DEV)
import solver_index as si                                  # noqa: E402

DATA = os.environ.get("ZAHYOU_INDEX_DIR", r"C:\AstrometryData")
TAGS = ["4107", "5205-02", "5202-00"]


class _PMC(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
                ("PrivateUsage", ctypes.c_size_t)]


def memory():
    """(プライベート, ワーキングセット)(MiB)。Windows 以外は (None, None)。"""
    try:
        k = ctypes.windll.kernel32
        k.GetCurrentProcess.restype = wintypes.HANDLE
        k.K32GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(_PMC), wintypes.DWORD]
        m = _PMC()
        m.cb = ctypes.sizeof(m)
        if not k.K32GetProcessMemoryInfo(k.GetCurrentProcess(), ctypes.byref(m), m.cb):
            return None, None
        return m.PrivateUsage / 1048576, m.WorkingSetSize / 1048576
    except (AttributeError, OSError):
        return None, None


def sha_u32(a):
    return hashlib.sha256(np.ascontiguousarray(np.asarray(a, dtype="<u4")).tobytes()).hexdigest()


def median_of(n, fn):
    ts, r = [], None
    for _ in range(max(1, n)):
        t0 = time.perf_counter()
        r = fn()
        ts.append(time.perf_counter() - t0)
    return statistics.median(ts), r


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else None
    if not os.path.isdir(DATA):
        print(f"SKIP: 星図データが無い({DATA})")
        return 0
    ok = True
    res = {"numpy": np.__version__, "python": sys.version.split()[0], "ball": {}}
    for tag in TAGS:
        with open(os.path.join(HERE, f"index_ball_{tag}.json"), encoding="utf-8") as fp:
            want = json.load(fp)
        b = want["balls"][0]
        D = want["code_dim"]
        q = np.asarray(want["queries"], dtype=np.float64).reshape(-1, D)[: b["nq"]]
        r = b["radius"]
        t0 = time.perf_counter()
        f = si.SvIndexFile(os.path.join(DATA, f"index-{tag}.fits"))
        qi, quad = f.code_ball(q, r)
        t_first = time.perf_counter() - t0
        same = len(qi) == b["total"] and sha_u32(np.stack([qi, quad], 1).ravel()) == b["sha256"]
        ok &= same
        t_warm, _ = median_of(5, lambda: f.code_ball(q, r))

        def each():
            n = 0
            for x in q:
                n += len(f.code_ball(x[None, :], r)[0])
            return n
        t_each, n_each = median_of(3, each)
        ok &= n_each == len(qi)
        c = want["bench_cone"]
        t_cone, got = median_of(30, lambda: f.stars_in_cone(c["center"], c["radius"]))
        cone_same = len(got) == c["count"] and sha_u32(got) == c["sha256"]
        ok &= cone_same
        print(f"code_ball {tag:8} {len(q)} 問 → {len(qi)} 件{'(一致)' if same else '(違う!)'}: 初回 {t_first * 1e3:.1f} ms、"
              f"2 回目から {t_warm * 1e3:.1f} ms、1 問ずつ {len(q)} 回 {t_each * 1e3:.0f} ms")
        print(f"円錐     {tag:8} → {len(got)} 星{'(一致)' if cone_same else '(違う!)'}: {t_cone * 1e6:.0f} μs")
        res["ball"][tag] = dict(nq=len(q), radius=r, total=len(qi), match=same, first_s=t_first, warm_s=t_warm,
                                each_s=t_each, cone=dict(count=len(got), match=cone_same, s=t_cone))
        f.close()

    t_scan, cat = median_of(3, lambda: si.SvIndexCatalog(DATA))
    print(f"走査 {len(cat.files) + len(cat.problems)} 本: {t_scan * 1e3:.0f} ms")
    res["scan"] = dict(files=len(cat.files), problems=len(cat.problems), s=t_scan)

    tiles = cat.series.get(5202, [])
    if tiles:
        p0, w0 = memory()
        t0 = time.perf_counter()
        hits = stars = 0
        for f in tiles:
            f.open()
            hits += len(f.code_ball(np.array([[0.4, 0.45, 0.6, 0.5]]), 0.01)[0])
            if f.tile_center_xyz is not None:
                stars += len(f.stars_in_cone(f.tile_center_xyz, np.radians(0.5)))
        t_open = time.perf_counter() - t0
        p1, w1 = memory()
        cat.close()
        p2, _ = memory()
        print(f"5202 の {len(tiles)} タイルを開いて探す({hits} 件・{stars} 星): {t_open * 1e3:.0f} ms、"
              f"プライベートメモリ {p0:.1f} → {p1:.1f} MiB(増加 {p1 - p0:.1f} MiB)、ワーキングセット {w0:.1f} → {w1:.1f} MiB、"
              f"閉じた後 {p2:.1f} MiB")
        res["open_5202"] = dict(tiles=len(tiles), s=t_open, private_before_mib=p0, private_after_mib=p1,
                                private_closed_mib=p2, working_set_before_mib=w0, working_set_after_mib=w1,
                                hits=hits, stars=stars)
    res["all_match"] = bool(ok)
    if out_path:
        with open(out_path, "w", encoding="utf-8") as fp:
            json.dump(res, fp, ensure_ascii=False, indent=1)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

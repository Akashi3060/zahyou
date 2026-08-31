"""
astrometry.net の index ファイル (5200 LITE シリーズ) を取ってくる。

  python fetch_index.py 5202 5203 5204            # C:\\AstrometryData へ
  python fetch_index.py --dest D:\\idx 5204        # 置き場所を変える
  python fetch_index.py --check 5202 5203 5204    # 検証だけ (ダウンロードしない)

・途中で止めても Range ヘッダで続きから再開する
・落とし終えたら「サイズ一致」と「FITS として開けて INDEXID が一致」まで確かめる
・壊れていたファイルは消して次の実行で取り直す
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
import shutil
import sys
import threading
import time
import urllib.error
import urllib.request

BASE = "https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE"
DEFAULT_DEST = r"C:\AstrometryData" if os.name == "nt" else "/usr/share/astrometry"
TILES = 48
RETRIES = 6

# 末尾 2 桁が画角の段 (arcmin)
SCALE_FOV = {
    5200: (2, 2.8), 5201: (2.8, 4), 5202: (4, 5.6), 5203: (5.6, 8),
    5204: (8, 11), 5205: (11, 16), 5206: (16, 22),
}

_print_lock = threading.Lock()
_done_bytes = [0]
_t0 = [time.time()]


def log(msg):
    with _print_lock:
        print(msg, flush=True)


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(n) < 1024:
            return f"{n:,.1f} {unit}"
        n /= 1024
    return f"{n:.1f} PB"


def remote_size(url, timeout=60):
    req = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return int(r.headers["Content-Length"])


def verify(path, index_id, expect_size=None):
    """サイズと FITS の中身を確かめる。戻り値 (ok, 理由)"""
    if not os.path.exists(path):
        return False, "ファイルが無い"
    size = os.path.getsize(path)
    if expect_size is not None and size != expect_size:
        return False, f"サイズ不一致 {size} != {expect_size}"
    try:
        from astropy.io import fits
        with fits.open(path, memmap=False) as h:
            hdr = h[0].header
            if int(hdr.get("INDEXID", -1)) != index_id:
                return False, f"INDEXID が違う: {hdr.get('INDEXID')}"
            if int(hdr.get("HPNSIDE", 0)) != 2:
                return False, f"HPNSIDE が違う: {hdr.get('HPNSIDE')}"
            if int(hdr.get("NQUADS", 0)) <= 0:
                return False, "quad が 0 個"
    except ImportError:
        pass                      # astropy が無ければサイズだけで妥協する
    except Exception as e:
        return False, f"FITS として読めない: {type(e).__name__}: {e}"
    return True, "ok"


def fetch_one(index_id, tile, dest, total_bytes):
    name = f"index-{index_id}-{tile:02d}.fits"
    url = f"{BASE}/{name}"
    final = os.path.join(dest, name)
    part = final + ".part"

    try:
        size = remote_size(url)
    except Exception as e:
        return name, False, f"サイズを取得できない: {e}"

    ok, _ = verify(final, index_id, size)
    if ok:
        with _print_lock:
            _done_bytes[0] += size
        return name, True, "既にある"

    if os.path.exists(final):
        os.remove(final)          # 壊れていたので取り直す

    for attempt in range(1, RETRIES + 1):
        have = os.path.getsize(part) if os.path.exists(part) else 0
        if have > size:
            os.remove(part)
            have = 0
        if have == size:
            break
        req = urllib.request.Request(url)
        mode = "wb"
        if have:
            req.add_header("Range", f"bytes={have}-")
            mode = "ab"
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                if have and r.status != 206:      # 続きから貰えなかった
                    have, mode = 0, "wb"
                with open(part, mode) as f:
                    while True:
                        chunk = r.read(1 << 20)
                        if not chunk:
                            break
                        f.write(chunk)
                        with _print_lock:
                            _done_bytes[0] += len(chunk)
            break
        except Exception as e:
            wait = min(30, 2 ** attempt)
            log(f"    ! {name} 失敗 ({attempt}/{RETRIES}): {type(e).__name__} {e} -> {wait}s 後に再試行")
            time.sleep(wait)
    else:
        return name, False, "リトライ上限"

    if os.path.getsize(part) != size:
        return name, False, f"サイズ不一致 {os.path.getsize(part)} != {size}"
    os.replace(part, final)
    ok, why = verify(final, index_id, size)
    if not ok:
        os.remove(final)
        return name, False, why

    done = _done_bytes[0]
    el = max(time.time() - _t0[0], 1e-6)
    rate = done / el
    left = max(total_bytes - done, 0)
    eta = left / rate if rate > 0 else 0
    log(f"  ✓ {name}  {human(size)}   "
        f"[{human(done)} / {human(total_bytes)}  {rate/1e6:.1f} MB/s  "
        f"残り {eta/60:.0f} 分]")
    return name, True, "ok"


def plan(scales, dest):
    """必要なサイズを見積もる。"""
    total = 0
    per_scale = {}
    for s in scales:
        sizes = []
        for t in (0, 12, 24, 36, 47):
            sizes.append(remote_size(f"{BASE}/index-{s}-{t:02d}.fits"))
        avg = sum(sizes) / len(sizes)
        per_scale[s] = avg * TILES
        total += per_scale[s]
    return per_scale, total


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scales", nargs="+", type=int)
    ap.add_argument("--dest", default=DEFAULT_DEST)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--check", action="store_true", help="検証だけ行う")
    args = ap.parse_args()

    for s in args.scales:
        if s not in SCALE_FOV:
            sys.exit(f"未知の scale: {s} (5200〜5206)")

    os.makedirs(args.dest, exist_ok=True)

    if args.check:
        bad = 0
        for s in args.scales:
            for t in range(TILES):
                name = f"index-{s}-{t:02d}.fits"
                p = os.path.join(args.dest, name)
                try:
                    size = remote_size(f"{BASE}/{name}")
                except Exception:
                    size = None
                ok, why = verify(p, s, size)
                if not ok:
                    bad += 1
                    print(f"  NG {name}: {why}")
            print(f"index-{s}: 検証完了")
        print("すべて正常" if bad == 0 else f"{bad} 個に問題あり")
        return 0 if bad == 0 else 1

    print(f"置き場所: {args.dest}")
    per_scale, total = plan(args.scales, args.dest)
    for s in args.scales:
        lo, hi = SCALE_FOV[s]
        print(f"  index-{s}-00〜47  画角 {lo}′〜{hi}′  約 {human(per_scale[s])}")
    free = shutil.disk_usage(args.dest).free
    print(f"合計 約 {human(total)} / 空き {human(free)}")
    if free < total * 1.15:
        sys.exit("空き容量が足りません。")

    jobs = [(s, t) for s in args.scales for t in range(TILES)]
    _t0[0] = time.time()
    failures = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(fetch_one, s, t, args.dest, total): (s, t) for s, t in jobs}
        for fut in concurrent.futures.as_completed(futs):
            name, ok, why = fut.result()
            if not ok:
                failures.append((name, why))
                log(f"  ✗ {name}: {why}")

    el = time.time() - _t0[0]
    print(f"\n所要 {el/60:.1f} 分")
    if failures:
        print(f"失敗 {len(failures)} 件:")
        for n, w in failures:
            print(f"  {n}: {w}")
        return 1
    print("すべて取得・検証できました。")
    return 0


if __name__ == "__main__":
    sys.exit(main())

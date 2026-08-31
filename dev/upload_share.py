"""
Nextcloud の公開共有へファイルを上げる。

  python upload_share.py --token M2sNsnA2K4oJ4Co C:\\AstrometryData\\dist\\*.zip
  python upload_share.py --token ... --list          # 中身を見るだけ

まるごと 1 回の PUT で送る。

公開共有では分割アップロード (dav/uploads) の結合 (MOVE .file) が
Sabre\\DAV\\Exception\\Forbidden で弾かれるので使えない。
生の PUT なら PHP の upload_max_filesize は効かない (multipart ではないため)
ので、GB 級でもそのまま通る。実測 40 MB/s / 868 MB が 23 秒。
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import os
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

HOST = "https://cloud.akashi-kosaku.uk"


def human(n):
    for u in ("B", "KB", "MB", "GB"):
        if abs(n) < 1024:
            return f"{n:,.1f} {u}"
        n /= 1024
    return f"{n:.1f} TB"


def curl(args, expect=(200, 201, 204, 207), timeout=1800):
    p = subprocess.run(["curl.exe", "-sS", "--max-time", str(timeout),
                        "-w", "\n%{http_code}"] + args,
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace")
    if p.returncode != 0:
        raise RuntimeError(f"curl 失敗: {p.stderr.strip()}")
    body, _, code = p.stdout.rpartition("\n")
    code = int(code.strip() or 0)
    if code not in expect:
        raise RuntimeError(f"HTTP {code}: {body[:400]}")
    return code, body


def listing(token):
    _, body = curl(["-u", f"{token}:", "-X", "PROPFIND", "-H", "Depth: 1",
                    f"{HOST}/public.php/webdav/"], expect=(207,))
    ns = {"d": "DAV:"}
    out = []
    for resp in ET.fromstring(body).findall("d:response", ns):
        href = resp.findtext("d:href", "", ns)
        size = resp.find(".//d:getcontentlength", ns)
        name = os.path.basename(href.rstrip("/"))
        if name:
            from urllib.parse import unquote
            out.append((unquote(name), int(size.text) if size is not None else None))
    return out


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 22), b""):
            h.update(chunk)
    return h.hexdigest()


def upload(token, path, name):
    """まるごと PUT する。GB 級でも通る (上のコメント参照)。"""
    from urllib.parse import quote
    curl(["-u", f"{token}:", "-T", path,
          f"{HOST}/public.php/webdav/{quote(name)}"],
         expect=(200, 201, 204))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="*")
    ap.add_argument("--token", required=True)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    if args.list:
        for name, size in listing(args.token):
            print(f"  {human(size) if size is not None else '<dir>':>12}  {name}")
        return 0

    paths = []
    for pat in args.files:
        paths.extend(sorted(glob.glob(pat)))
    if not paths:
        sys.exit("ファイルが見つかりません。")

    before = {n: s for n, s in listing(args.token)}
    total = sum(os.path.getsize(p) for p in paths)
    print(f"アップロード {len(paths)} ファイル / {human(total)}")
    for p in paths:
        print(f"  {os.path.basename(p):40s} {human(os.path.getsize(p))}")
    if args.dry_run:
        print("(--dry-run なので送信しません)")
        return 0

    for p in paths:
        name = os.path.basename(p)
        size = os.path.getsize(p)
        if before.get(name) == size:
            print(f"\n{name}: 同じサイズで既にあるので飛ばします")
            continue
        print(f"\n{name} ({human(size)}) を送っています...", flush=True)
        t0 = time.time()
        upload(args.token, p, name)
        el = max(time.time() - t0, 1e-6)
        print(f"  完了 {el:.0f} 秒 ({size/el/1e6:.0f} MB/s)", flush=True)

    print("\n--- 共有の中身 ---")
    after = dict(listing(args.token))
    ok = True
    for p in paths:
        name = os.path.basename(p)
        want = os.path.getsize(p)
        got = after.get(name)
        mark = "OK " if got == want else "NG "
        ok &= got == want
        print(f"  {mark} {name}  {got} (期待 {want})")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

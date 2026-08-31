"""
C:\\AstrometryData の index ファイルを、画角の段ごとに zip へまとめる。

  python make_index_zips.py 5202 5203 5204
  python make_index_zips.py --src C:\\AstrometryData --out D:\\dist 5204

利用者が「自分の画角に必要な段だけ」落とせるように、scale ごとに分ける。
zip の中身は AstrometryData/index-....fits なので、
C:\\ 直下に展開すればマニュアル 2.3 の配置になる。
"""
from __future__ import annotations

import argparse
import hashlib
import os
import sys
import time
import zipfile

SCALE_FOV = {
    5200: (2, 2.8), 5201: (2.8, 4), 5202: (4, 5.6), 5203: (5.6, 8),
    5204: (8, 11), 5205: (11, 16), 5206: (16, 22),
}
TILES = 48

README = """\
astrometry.net 星図データ (index files)
=======================================

このアーカイブは index-{scale}-00 〜 47 の 48 ファイルです。
対応する画角: {lo}′ 〜 {hi}′
  (画角 = 画像の横幅[px] x 画素スケール["/px] / 60)

出典: https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/
      5200 シリーズ LITE (Tycho-2 + Gaia DR2)

展開のしかた
------------
C:\\ 直下に展開してください。次のようになれば正しい配置です。

    C:\\AstrometryData\\index-{scale}-00.fits
    ...
    C:\\AstrometryData\\index-{scale}-47.fits

WSL 側の /etc/astrometry.cfg に
    add_path /mnt/c/AstrometryData
    autoindex
が書かれていれば、追加のファイルは自動で認識されます。

なお /mnt/c からの読み出しは遅いので、WSL の中へ移すと数倍速くなります。
    cp -r /mnt/c/AstrometryData ~/AstrometryData
    sudo sed -i "s#/mnt/c/AstrometryData#$HOME/AstrometryData#" /etc/astrometry.cfg

どの段が必要か
--------------
    index-5200 :   2′ 〜 2.8′
    index-5201 : 2.8′ 〜 4′
    index-5202 :   4′ 〜 5.6′
    index-5203 : 5.6′ 〜 8′
    index-5204 :   8′ 〜 11′
    index-5205 :  11′ 〜 16′
    index-5206 :  16′ 〜 22′
    index-4107 〜 4119 : 22′ 〜 2000′
"""


def human(n):
    for u in ("B", "KB", "MB", "GB"):
        if abs(n) < 1024:
            return f"{n:,.1f} {u}"
        n /= 1024
    return f"{n:.1f} TB"


def build(scale, src, out_dir, level=1):
    lo, hi = SCALE_FOV[scale]
    names = [f"index-{scale}-{i:02d}.fits" for i in range(TILES)]
    paths = [os.path.join(src, n) for n in names]
    missing = [n for n, p in zip(names, paths) if not os.path.exists(p)]
    if missing:
        raise SystemExit(f"index-{scale}: {len(missing)} ファイル足りません "
                         f"(例: {missing[0]})")

    raw = sum(os.path.getsize(p) for p in paths)
    zip_path = os.path.join(out_dir, f"AstrometryData-index-{scale}.zip")
    print(f"index-{scale} ({lo}′〜{hi}′) : {len(names)} ファイル {human(raw)} "
          f"-> {os.path.basename(zip_path)}")

    t0 = time.time()
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED,
                         compresslevel=level, allowZip64=True) as z:
        z.writestr(f"AstrometryData/README-index-{scale}.txt",
                   README.format(scale=scale, lo=lo, hi=hi))
        for i, (n, p) in enumerate(zip(names, paths), 1):
            z.write(p, f"AstrometryData/{n}")
            if i % 12 == 0 or i == len(names):
                el = time.time() - t0
                print(f"    {i}/{len(names)}  {human(os.path.getsize(zip_path))}  "
                      f"{el:.0f} 秒", flush=True)

    size = os.path.getsize(zip_path)
    h = hashlib.sha256()
    with open(zip_path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 22), b""):
            h.update(chunk)
    print(f"    完了 {human(size)} ({size/raw*100:.0f}%)  "
          f"sha256 {h.hexdigest()[:16]}…  {time.time()-t0:.0f} 秒\n")
    return zip_path, size, h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scales", nargs="+", type=int)
    ap.add_argument("--src", default=r"C:\AstrometryData")
    ap.add_argument("--out", default=r"C:\AstrometryData\dist")
    ap.add_argument("--level", type=int, default=1,
                    help="zip の圧縮レベル (1 が速くて十分)")
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)
    results = []
    for s in args.scales:
        if s not in SCALE_FOV:
            sys.exit(f"未知の scale: {s}")
        results.append(build(s, args.src, args.out, args.level))

    lines = ["# AstrometryData 追加分  sha256  サイズ"]
    for path, size, digest in results:
        lines.append(f"{digest}  {os.path.basename(path)}  {size}")
    manifest = os.path.join(args.out, "SHA256SUMS.txt")
    with open(manifest, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")

    total = sum(s for _, s, _ in results)
    print(f"合計 {human(total)}  ->  {args.out}")
    print(f"チェックサム: {manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

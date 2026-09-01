"""
GUI を実際に組み立てて、解析を 1 回通すテスト。

画面は出るが、人が触らなくても最後まで進む (mainloop の代わりに update を回す)。
中身は zahyou_gui.selftest() ―― exe に固めたあとも同じ確認ができるよう、
本体側に置いてある。

  python test_gui.py [画像パス]

exe を確かめるときは:
  dist\\zahyou\\zahyou.exe --selftest "画像パス" --out result.txt
"""
from __future__ import annotations

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

IMAGE = (sys.argv[1] if len(sys.argv) > 1
         else r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits")


def main():
    if not os.path.exists(IMAGE):
        print(f"テスト画像がありません: {IMAGE}")
        return 2

    import zahyou_gui as g

    rows = g.selftest(IMAGE)
    width = max([len(r[0]) for r in rows] or [1])
    n_ok = 0
    print()
    for name, ok, msg in rows:
        n_ok += ok
        print(f"  {'PASS' if ok else 'FAIL'}  {name.ljust(width)}  {msg}")
    print(f"\n  {n_ok}/{len(rows)} passed")
    return 0 if n_ok == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())

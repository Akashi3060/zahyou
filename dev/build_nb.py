"""dev/*.py を組み立てて zahyou.ipynb (markdown + 3 セル) を作る。"""
import io
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def read(name):
    with io.open(os.path.join(HERE, name), encoding="utf-8") as f:
        return f.read()


def strip_module_head(src):
    """モジュール冒頭の import 群と from __future__ を取り除く。"""
    lines = src.splitlines()
    out, i = [], 0
    # 先頭のコメントブロック / import / 空行を読み飛ばす
    while i < len(lines):
        s = lines[i]
        t = s.strip()
        if (t.startswith("#") or t == ""
                or t.startswith("import ") or t.startswith("from ")
                or t.startswith("try:") or t.startswith("except ImportError")
                or t.startswith("ndimage = None")):
            i += 1
            continue
        break
    out = lines[i:]
    return "\n".join(out).strip("\n")


ENGINE_HEAD = '''# ==============================================================================
#  zahyou 解析エンジン
#
#  zahyou_v6.ipynb のセル A から読み込まれます。単体では実行しません。
#  中身を読む必要はありません (直したいときだけ触ってください)。
# ==============================================================================

import concurrent.futures
import json
import os
import shlex
import shutil
import subprocess
import tempfile
import time
import warnings

import numpy as np
import matplotlib
import matplotlib.pyplot as plt
from astropy.coordinates import SkyCoord
from astropy.io import fits
from astropy.stats import sigma_clipped_stats
from astropy.visualization import AsinhStretch, ImageNormalize
from astropy.wcs import WCS
from astropy.wcs.utils import proj_plane_pixel_scales
import astropy.units as u

try:
    from scipy import ndimage
except ImportError:
    ndimage = None
    raise SystemExit("scipy が必要です。1 つ目のセルの pip 行を実行してください。")

# SharpCap などの FITS には DATE-OBS はあっても MJD-OBS が無い。astropy はそれを
# 補ったうえで警告を出すが、直す必要のあるものではないので画面には出さない。
# (抑えるのはこの 1 種類だけ。他の警告は今までどおり表示する)
from astropy.wcs import FITSFixedWarning
warnings.simplefilter("ignore", FITSFixedWarning)
'''


HEADER_MD = """\
# zahyou — 天体画像から目標への向きを求める

**使い方は 3 ステップだけです。**

1. **セル A** を実行 → 準備（1 回だけでよい）
2. **セル B** を実行 → ボタンで画像を選ぶ
3. **セル C** に目標を書いて実行 → すぐ下に結果と矢印が出ます

2 回目以降は **セル C だけ**を実行し直せば十分です。
別の画像に変えるときは、セル B で選び直してからセル C を実行します。

> このノートブックは `zahyou_engine.py` と**同じフォルダー**に置いてください。
"""

CELL_C_HEAD = """\
# ==============================================================================
#  セル C : ユーザー設定と実行
#
#  ★ 書き換えるのはこのセルだけです。実行すると、すぐ下に結果が出ます。
# ==============================================================================
"""


def build():
    core = strip_module_head(read("zahyou_core.py"))
    plot = strip_module_head(read("zahyou_plot.py"))
    # 描画側の遅延 import はそのまま残す (reproject は重いので必要なときだけ)
    settings = read("nb_settings.py")
    main = read("nb_main.py")

    # --- 解析エンジンは .py に切り出す -------------------------------------
    #  ノートブックに 1500 行を貼ると、画像を選ぶ場所と結果が出る場所が
    #  遠く離れてしまう。VS Code は cell metadata の jupyter.source_hidden を
    #  無視するので「既定で折りたたむ」ことはできない (実機で確認済み)。
    run_call = "\n_result_wcs = run()\n"
    assert main.endswith(run_call), "nb_main.py の末尾が run() 呼び出しではない"
    main_defs = main[: -len(run_call)]

    engine = "\n\n".join([
        ENGINE_HEAD,
        core,
        plot,
        strip_module_head(read("nb_ui.py")),
        main_defs.rstrip("\n"),
    ]) + "\n"

    # --- セル A = エンジン読み込み / セル B = 画像選択 ----------------------
    cell_a = read("nb_loader.py")
    cell_b = read("nb_pick.py")

    # --- セル C : 設定と実行 (短い。ここだけ触ればよい) ---------------------
    cell_c = CELL_C_HEAD + "\n" + settings.strip("\n") + "\n\n_result_wcs = run()\n"

    nb = {
        "cells": [
            {"cell_type": "markdown", "id": "zahyou-intro",
             "metadata": {}, "source": HEADER_MD.splitlines(keepends=True)},
            {"cell_type": "code", "id": "zahyou-a-setup", "execution_count": None,
             "metadata": {}, "outputs": [],
             "source": cell_a.splitlines(keepends=True)},
            {"cell_type": "code", "id": "zahyou-b-pick", "execution_count": None,
             "metadata": {}, "outputs": [],
             "source": cell_b.splitlines(keepends=True)},
            {"cell_type": "code", "id": "zahyou-c-run", "execution_count": None,
             "metadata": {}, "outputs": [],
             "source": cell_c.splitlines(keepends=True)},
        ],
        "metadata": {
            "kernelspec": {"display_name": "Python 3", "language": "python",
                           "name": "python3"},
            "language_info": {"name": "python", "version": "3.11"},
        },
        "nbformat": 4, "nbformat_minor": 5,
    }
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "zahyou.ipynb")
    with io.open(out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(nb, f, ensure_ascii=False, indent=1)
        f.write("\n")

    # エンジン本体を .ipynb の隣に置く (セル B がこれを読み込む)
    engine_path = os.path.join(os.path.dirname(os.path.abspath(out)),
                               "zahyou_engine.py")
    with io.open(engine_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(engine)

    # テスト用: エンジン + セル C をつないだ .py (exec すれば 1 回分の実行になる)
    with io.open(os.path.join(HERE, "_cell2_check.py"), "w", encoding="utf-8") as f:
        f.write(engine + "\n" + cell_c)

    print(f"wrote {out}")
    total = 0
    for label, text in (("A 準備 (エンジン読み込み)", cell_a),
                        ("B 画像を選ぶ", cell_b),
                        ("C 設定と実行", cell_c)):
        n = len(text.splitlines())
        total += n
        print(f"  セル {label:18s} {n:5d} 行")
    print(f"  ノートブック合計 {total:20d} 行")
    print(f"wrote {engine_path}  ({len(engine.splitlines())} 行)")


if __name__ == "__main__":
    build()

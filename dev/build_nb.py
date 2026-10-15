"""dev/*.py を組み立てて zahyou.ipynb (markdown + 3 セル) と zahyou_engine.py を作る。"""
import ast
import io
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# 自前の解析エンジン。この順につなぐ (後ろのものが前のものの名前を使う)。
# solver_synth.py は試験用なので入れない。
SOLVER_MODULES = ("solver_wcs.py", "solver_index.py", "solver_core.py")
# 1 つの名前空間に入るので、solver_*.py の最上位の名前はこの接頭辞で始める
# (sv_ / Sv / SV_ と、その頭に _ を付けた内部用。_SV_ のような定数も通す)
SOLVER_PREFIXES = ("sv_", "Sv", "SV_", "_sv_", "_Sv", "_SV_")


def read(name):
    with io.open(os.path.join(HERE, name), encoding="utf-8") as f:
        return f.read()


def _skip_block(lines, j):
    """字下げされた行 (と空行) を読み飛ばした先の行番号。"""
    while j < len(lines) and (lines[j][:1] in (" ", "\t") or not lines[j].strip()):
        j += 1
    return j


def split_module_head(src):
    """
    モジュール冒頭のコメント・import 群・try/except ImportError を本体から切り離す。
    戻り値 (外した import 文のリスト, 本体)。

    import 文として返すのは行頭 (字下げなし) のものだけで、括弧で複数行に
    分けたものは 1 文にまとめる。try の中の import は返さない (エンジンの
    冒頭に自前で書く。scipy を必須にする ENGINE_HEAD のように)。
    """
    lines = src.splitlines()
    imports, i = [], 0
    while i < len(lines):
        s = lines[i]
        t = s.strip()
        if t.startswith("#") or t == "":
            i += 1
            continue
        if s.startswith(("import ", "from ")):          # 行頭の import
            stmt = [s]
            if "(" in s and ")" not in s:              # from x import (a,\n b)
                while i + 1 < len(lines) and ")" not in lines[i]:
                    i += 1
                    stmt.append(lines[i])
            imports.append(" ".join(" ".join(stmt).split()))
            i += 1
            continue
        if s.startswith("try:"):                        # try: import … / except ImportError:
            j = _skip_block(lines, i + 1)
            handlers = 0
            while j < len(lines) and lines[j].startswith("except"):
                if "ImportError" not in lines[j] and "ModuleNotFoundError" not in lines[j]:
                    handlers = 0
                    break
                handlers += 1
                j = _skip_block(lines, j + 1)
            if j > i + 1 and handlers:
                i = j
                continue
        break
    return imports, "\n".join(lines[i:]).strip("\n")


def strip_module_head(src):
    """モジュール冒頭の import 群と from __future__ を取り除く。"""
    return split_module_head(src)[1]


def _top_level_names(tree):
    """モジュールの最上位で定義される名前 (関数・クラス・代入。if / try の中も見る)。"""
    names = []

    def visit(body):
        for node in body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names.append(node.name)
            elif isinstance(node, (ast.Assign, ast.AnnAssign, ast.AugAssign)):
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                for t in targets:
                    for n in ast.walk(t):
                        if isinstance(n, ast.Name):
                            names.append(n.id)
            elif isinstance(node, (ast.If, ast.Try, ast.With, ast.For, ast.While)):
                for field in ("body", "orelse", "finalbody"):
                    visit(getattr(node, field, []) or [])
                for h in getattr(node, "handlers", []) or []:
                    visit(h.body)
    visit(tree.body)
    return names


def check_solver_module(name, src):
    """
    solver_*.py が組み立ての約束を守っているか。守っていなければ止める。

      - 冒頭に docstring を置かない (strip_module_head は docstring を外せない)
      - 最上位の名前は sv_ / Sv / _sv_ / SV_ で始める (zahyou_core などとぶつけない)
      - numpy と標準ライブラリだけを import する
    """
    problems = []
    tree = ast.parse(src, filename=name)
    if ast.get_docstring(tree) is not None:
        problems.append("冒頭に docstring があります (# のコメントにしてください)")
    bad = sorted({n for n in _top_level_names(tree)
                  if not n.startswith(SOLVER_PREFIXES)
                  and not (n.startswith("__") and n.endswith("__"))})
    if bad:
        problems.append("接頭辞 (sv_ / Sv / _sv_ / SV_) の無い最上位の名前: " + ", ".join(bad))
    allowed = set(sys.stdlib_module_names) | {"numpy"}
    for node in ast.walk(tree):
        mods = []
        if isinstance(node, ast.Import):
            mods = [a.name for a in node.names]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            mods = [node.module]
        for m in mods:
            top = m.split(".")[0]
            if top not in allowed and not top.startswith("solver_"):
                problems.append(f"numpy と標準ライブラリ以外を import しています: {m}")
    if problems:
        raise SystemExit(f"{name} が組み立ての約束を守っていません:\n  "
                         + "\n  ".join(problems)
                         + "\n  (docs/design-native-solver.md の 1 節)")


def engine_imports(module_imports, head):
    """
    各モジュールの冒頭にあった import のうち、ENGINE_HEAD に無いものを集める。

    import は組み立てのときに外すので、エンジンの冒頭にまとめて書き直す必要がある。
    ここで集めないと、solver_*.py が使う標準ライブラリ (math・mmap など) が
    エンジンでは未定義になる。from __future__ と、solver どうしの
    from solver_xxx import * (同じ名前空間に入るので要らない) は除く。
    """
    have = {ln.strip() for ln in head.splitlines()}
    out = []
    for t in module_imports:
        if t.startswith("from __future__") or re.match(r"from solver_\w+ import", t):
            continue
        if t not in have and t not in out:
            out.append(t)
    return out


ENGINE_HEAD = '''# ==============================================================================
#  zahyou 解析エンジン
#
#  zahyou_v6.ipynb のセル A と GUI 版 (zahyou.exe) から読み込まれます。単体では実行しません。
#  中身を読む必要はありません (直したいときだけ dev/ の元のファイルを触ってください)。
#  オフライン解析は自前のエンジン (dev/solver_*.py) で行い、WSL は使いません。
# ==============================================================================

from __future__ import annotations

import concurrent.futures
import json
import math
import os
import shutil
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
    # --- 自前の解析エンジン (solver_*.py) がそろっているか -----------------
    # 別の人が書いている途中だと、組み立てても import エラーで動かない
    # エンジンができてしまう。先に止めて、何が無いかをはっきり言う。
    missing = [m for m in SOLVER_MODULES if not os.path.exists(os.path.join(HERE, m))]
    if missing:
        raise SystemExit(
            "組み立てに要るファイルがありません: "
            + ", ".join("dev/" + m for m in missing)
            + "\n  自前の解析エンジン (solver_*.py) が出来てから、もう一度実行してください。"
            + "\n  つなぐ順: " + " → ".join(SOLVER_MODULES)
            + " → zahyou_core → zahyou_plot → nb_ui → nb_main")

    parts, imports = [], []
    for name in SOLVER_MODULES:
        src = read(name)
        check_solver_module(name, src)
        imp, body = split_module_head(src)
        imports += imp
        parts.append(body)
    for name in ("zahyou_core.py", "zahyou_plot.py", "nb_ui.py"):
        imp, body = split_module_head(read(name))
        imports += imp
        # 描画側の遅延 import はそのまま残す (reproject は重いので必要なときだけ)
        parts.append(body)
    settings = read("nb_settings.py")
    main = read("nb_main.py")

    # --- 解析エンジンは .py に切り出す -------------------------------------
    #  ノートブックに 1500 行を貼ると、画像を選ぶ場所と結果が出る場所が
    #  遠く離れてしまう。VS Code は cell metadata の jupyter.source_hidden を
    #  無視するので「既定で折りたたむ」ことはできない (実機で確認済み)。
    run_call = "\n_result_wcs = run()\n"
    main = main.replace("\r\n", "\n")
    assert main.endswith(run_call), "nb_main.py の末尾が run() 呼び出しではない"
    main_defs = main[: -len(run_call)]

    extra = engine_imports(imports, ENGINE_HEAD)
    head = ENGINE_HEAD
    if extra:
        head += ("\n# 各モジュールの冒頭にあった import (組み立てのときに集めたもの)\n"
                 + "\n".join(extra) + "\n")
    engine = "\n\n".join([head] + parts + [main_defs.rstrip("\n")]) + "\n"
    compile(engine, "zahyou_engine.py", "exec")         # 構文の壊れたエンジンを書き出さない

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

    # エンジン本体を .ipynb の隣に置く (セル A がこれを読み込む)
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

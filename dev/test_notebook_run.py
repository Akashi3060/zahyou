"""
本物の Jupyter カーネルでノートブックを実行し、画面に出るものを数えるテスト。

「セルの最後の式の値も自動表示される」ことを忘れると、display() したウィジェットと
戻り値のウィジェットが 2 つ並ぶ。目で見ないと気づけないので、ここで機械的に数える。

  python test_notebook_run.py

nbclient が要る:  pip install nbclient
"""
from __future__ import annotations

import os
import sys
import warnings

warnings.filterwarnings("ignore")

HERE = os.path.dirname(os.path.abspath(__file__))
NB = os.path.join(HERE, "zahyou.ipynb")
WIDGET_MIME = "application/vnd.jupyter.widget-view+json"

RESULTS = []


def check(name, ok, msg=""):
    RESULTS.append((name, bool(ok), msg))


def main():
    try:
        import nbformat
        from nbclient import NotebookClient
    except ImportError as e:
        print(f"nbclient / nbformat がありません ({e.name})。")
        print("  pip install nbclient nbformat")
        return 2
    if not os.path.exists(NB):
        print(f"{NB} がありません。先に build_nb.py を実行してください。")
        return 2

    nb = nbformat.read(NB, as_version=4)
    # セル C は解析が走って重いので、ここでは A と B だけ動かす
    #（解析そのものは test_wsl_e2e.py / test_robust.py で確認している）
    nb.cells = [c for c in nb.cells if c.get("id") != "zahyou-c-run"]

    NotebookClient(nb, timeout=300, kernel_name="python3",
                   resources={"metadata": {"path": HERE}}).execute()

    by_id = {c.get("id"): c for c in nb.cells if c.cell_type == "code"}

    for cid, cell in by_id.items():
        errs = [o for o in cell.outputs if o.output_type == "error"]
        check(f"{cid}: 例外が出ない", not errs,
              errs[0].ename + ": " + errs[0].evalue if errs else "")

    a = by_id.get("zahyou-a-setup")
    if a:
        text = "".join(o.get("text", "") for o in a.outputs
                       if o.output_type == "stream")
        check("セル A: 準備できたと表示される", "準備できました" in text,
              text.strip()[:80])

    b = by_id.get("zahyou-b-pick")
    if b:
        shown = [o for o in b.outputs
                 if o.output_type in ("display_data", "execute_result")]
        widgets = [o for o in shown if WIDGET_MIME in o.get("data", {})]
        check("セル B: 画像選択ウィジェットがちょうど 1 つ", len(widgets) == 1,
              f"{len(widgets)} 個 / 出力の種類 "
              f"{[o.output_type for o in shown]}")
        check("セル B: 戻り値が自動表示されていない",
              not any(o.output_type == "execute_result" for o in shown),
              "pick_image() がウィジェットを返していないか確認する")

    width = max(len(r[0]) for r in RESULTS)
    n_ok = 0
    print()
    for name, ok, msg in RESULTS:
        n_ok += ok
        print(f"  {'PASS' if ok else 'FAIL'}  {name.ljust(width)}  {msg}")
    print(f"\n  {n_ok}/{len(RESULTS)} passed")
    return 0 if n_ok == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())

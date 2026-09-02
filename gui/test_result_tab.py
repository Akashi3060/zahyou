"""
初回は「撮影時の向き」、次からは最後に見ていた表示になるか。

  python test_result_tab.py

設定ファイルは退避してから消し、終わったら戻す。
「組み立ての途中でもタブ変更の合図は飛んでくる」ので、そのまま覚えると
記憶が毎回いちばん左のタブで上書きされる ―― それを見張るテスト。
"""
import io
import json
import os
import shutil
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import zahyou_gui as g                                        # noqa: E402

IMAGE = r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits"
CFG = g.settings_path()
BAK = CFG + ".tabtest-bak"

rows = []


def check(name, ok, msg=""):
    rows.append((name, bool(ok), str(msg)))


def pump(app, sec, until=None):
    t0 = time.time()
    while time.time() - t0 < sec:
        app.update()
        if until and until():
            return True
        time.sleep(0.05)
    return until is None


def run_once(app):
    app.image_path.set(IMAGE)
    app.v_mode.set("STAR_NAME")
    app.v_name.set("UCAC4 660-021020")
    app.v_solve.set("オフライン (WSL) のみ")
    app._sync_mode()
    app.update()
    app._start_analysis()
    pump(app, 420, lambda: not app.busy)
    pump(app, 2)
    return {str(app.f_fig1): "撮影時の向き", str(app.f_fig2): "北が上",
            str(app.f_log): "ログ"}.get(app.out.select(), "?")


if os.path.exists(CFG):
    shutil.copyfile(CFG, BAK)
    os.remove(CFG)
try:
    # --- 1 回目 (設定なし = 初めて使う人) ---------------------------
    app = g.App()
    app.update()
    pump(app, 180, lambda: app.engine_ready)
    check("初回の既定は「撮影時の向き」", app.result_tab == "orig", app.result_tab)
    shown = run_once(app)
    check("初回の結果は「撮影時の向き」で出る", shown == "撮影時の向き", shown)

    # --- ユーザーが「北が上」に切り替える ---------------------------
    app.out.select(app.f_fig2)
    pump(app, 1)
    check("切り替えを覚える", app.result_tab == "northup", app.result_tab)
    saved = json.load(io.open(CFG, encoding="utf-8")).get("result_tab")
    check("設定ファイルにも残る", saved == "northup", saved)
    app.destroy()

    # --- 2 回目 (次に開いたとき) ------------------------------------
    app = g.App()
    app.update()
    pump(app, 180, lambda: app.engine_ready)
    check("開き直しても覚えている", app.result_tab == "northup", app.result_tab)
    shown = run_once(app)
    check("2 回目は「北が上」で出る", shown == "北が上", shown)
    app.destroy()
finally:
    if os.path.exists(BAK):
        shutil.copyfile(BAK, CFG)
        os.remove(BAK)
        print("設定ファイルを戻しました")

w = max(len(r[0]) for r in rows)
n = 0
print()
for name, ok, msg in rows:
    n += ok
    print(f"  {'PASS' if ok else 'FAIL'}  {name.ljust(w)}  {msg}")
print(f"\n  {n}/{len(rows)} passed")
sys.exit(0 if n == len(rows) else 1)

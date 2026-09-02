"""
押せるボタンを全部押して、「何か起きたと分かるか」を確かめる。

    ログに行が増える / 状態バーに一言が残る / ダイアログが出る
    ―― このどれかが無いボタンは、押しても反応が分からない。

実際に「状態を確認」がこれで、押しても表が黙って更新されるだけだった。
時間のかかるもの・お金や時間を使うもの (WSL の導入・9 GB のダウンロード) は
差し替えるか、確認ダイアログで「いいえ」を返して止める。

  python test_buttons.py
"""
from __future__ import annotations

import os
import sys
import time

from tkinter import ttk

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

IMAGE = r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits"

# 押しても安全なように差し替えるもの
DIALOGS = []


def main():
    import zahyou_env as env
    import zahyou_gui as g

    # --- 危ないもの・止まるものを差し替える --------------------------
    g.filedialog.askopenfilename = lambda **k: IMAGE if os.path.exists(IMAGE) else ""
    g.filedialog.askdirectory = lambda **k: env.DEFAULT_INDEX_DIR
    for name in ("showinfo", "showwarning", "showerror"):
        setattr(g.messagebox, name,
                lambda *a, name=name, **k: DIALOGS.append(name))
    # 確認は「いいえ」= 実行しない (9 GB を落とし始めないため)
    g.messagebox.askokcancel = lambda *a, **k: DIALOGS.append("ask") or False
    # WSL の導入は UAC が出て止まるので、押せたことだけ確かめる
    env.install_wsl = lambda log: (log("  (テストなので実際には入れません)"), True)[1]

    app = g.App()
    app.update()

    def pump(sec, until=None):
        t0 = time.time()
        while time.time() - t0 < sec:
            app.update()
            if until and until():
                return True
            time.sleep(0.05)
        return until is None

    pump(180, lambda: app.engine_ready)
    pump(120, lambda: app.env_state is not None)

    # --- ボタンを集める ----------------------------------------------
    buttons = []
    for tab, tab_name in ((app.tab_run, "解析"), (app.tab_env, "準備")):
        app.nb.select(tab)
        app.update()

        def walk(w):
            for c in w.winfo_children():
                if isinstance(c, ttk.Button):
                    try:
                        buttons.append((tab_name, c.cget("text"), c, tab))
                    except Exception:
                        pass
                walk(c)
        walk(tab)
    buttons.append(("下のバー", app.btn_theme.cget("text"), app.btn_theme, None))

    # 「解析を実行」は別のテスト (test_gui.py) で見ているので、ここでは飛ばす
    skip = {"解析を実行"}

    rows = []
    for tab_name, text, btn, tab in buttons:
        if text in skip:
            continue
        if tab is not None:
            app.nb.select(tab)
        app.txt_env.delete("1.0", "end")
        app.txt_log.delete("1.0", "end")
        app.v_status.set("")
        DIALOGS.clear()
        before_theme = app.theme_name
        app.update()

        # 「中止」は処理中しか押せない。押せる状態にしてから確かめる
        # (無効のまま invoke しても何も起きず、テストにならない)
        disabled = str(btn.cget("state")) == "disabled"
        if disabled:
            btn.configure(state="normal")
        try:
            btn.invoke()
        except Exception as e:
            rows.append((tab_name, text, False, f"例外 {type(e).__name__}: {e}"))
            continue
        pump(300, lambda: not app.busy)
        pump(1.5)
        if disabled:
            btn.configure(state="disabled")
            app.cancel_flag = False

        env_log = app.txt_env.get("1.0", "end").strip()
        run_log = app.txt_log.get("1.0", "end").strip()
        status = app.v_status.get().strip()
        theme_changed = app.theme_name != before_theme

        signals = []
        if env_log:
            signals.append(f"ログ{len(env_log.splitlines())}行")
        if run_log:
            signals.append(f"解析ログ{len(run_log.splitlines())}行")
        if status:
            signals.append(f"状態「{status}」")
        if DIALOGS:
            signals.append("ダイアログ")
        if theme_changed:
            signals.append("配色が変わった")
        rows.append((tab_name, text, bool(signals),
                     " / ".join(signals) or "何も起きない"))

    # 配色は元に戻す
    if app.theme_name != "light":
        app._apply_theme("light")
    app._save_settings()
    app.destroy()

    width = max(len(r[1]) for r in rows)
    n_ok = 0
    print()
    for tab_name, text, ok, msg in rows:
        n_ok += ok
        print(f"  {'PASS' if ok else 'FAIL'}  [{tab_name:4s}] "
              f"{text.ljust(width)}  {msg}")
    print(f"\n  {n_ok}/{len(rows)} passed")
    return 0 if n_ok == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())

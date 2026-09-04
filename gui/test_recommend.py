"""
「おすすめをまとめて落とす」を確かめる。

焦点距離もセンサー横幅も入れずに押せるボタンなので、確かめたいのは 3 つ。

  ・選ぶ段が正しいか (画角 4' 以上だけ。25 GB の 5200/5201 を巻き込まない)
  ・すでに持っている段を二度落とさないか
  ・「たいていの望遠鏡はこれで足ります」が本当か
    ―― 焦点距離を入れて選んだ段が、おすすめの中に収まるかで確かめる

  python test_recommend.py
"""
from __future__ import annotations

import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

PASS, FAIL = [], []


def check(name, ok, note=""):
    (PASS if ok else FAIL).append(name)
    print(f"  {'PASS' if ok else 'FAIL'}  {name:36s}  {note}")


def main():
    import zahyou_env as env
    import zahyou_gui as g

    # ============================================================ 段の中身 ===
    rec = env.recommended_scales()
    gb = env.scales_mb(rec) / 1024.0
    check("画角 4' 未満の段は入っていない",
          all(env.SCALE_FOV[s][0] >= 4.0 for s in rec),
          f"{len(rec)} 段 / {gb:.1f} GB")

    check("25 GB の 5200 / 5201 を巻き込まない",
          5200 not in rec and 5201 not in rec,
          f"外したぶん {env.scales_mb([5200, 5201])/1024:.1f} GB")

    check("4' 以上の段は 1 つも漏れていない",
          set(rec) == {s for s, (lo, _) in env.SCALE_FOV.items() if lo >= 4.0})

    check("容量が分かる段だけ",
          all(env.SCALE_MB.get(s) for s in rec),
          "SCALE_MB に穴が無い")

    # 段の「隣」は画角の順。番号順だと 4119 (1400'-2000') の隣が
    # 5200 (2'-2.8') になり、広角の人に 15.9 GB を勧めてしまう。
    ladder = sorted(env.SCALE_FOV, key=lambda s: env.SCALE_FOV[s])
    check("段が画角の順につながっている",
          all(env.SCALE_FOV[a][1] == env.SCALE_FOV[b][0]
              for a, b in zip(ladder, ladder[1:])),
          f"{env.SCALE_FOV[ladder[0]][0]:g}' 〜 {env.SCALE_FOV[ladder[-1]][1]:g}' が切れ目なし")

    # --------------------------------------------------- 望遠鏡の当てはまり ---
    # 焦点距離とセンサー横幅から選んだ段が、おすすめに収まるか。
    # enough=False は「収まらないのが正しい」= おすすめだけでは足りない機材。
    # 画面に「たいていの望遠鏡はこれで足ります」と書いてあるので、
    # 足りない側の境目もここに残しておく。
    rigs = [
        ("35mm レンズ / フルサイズ", 35, 36.0, True),
        ("200mm 望遠 / APS-C", 200, 23.5, True),
        ("800mm F5 / ASI290MM (1/2.8型)", 800, 5.6, True),
        ("2000mm C8 / ASI2600MC", 2000, 23.5, True),
        ("4000mm 直焦 / 1/1.8型", 4000, 7.4, True),
        ("8000mm 拡大撮影 / 1/1.8型", 8000, 7.4, False),
    ]
    for label, focal, sensor, enough in rigs:
        fov = env.fov_arcmin(focal, sensor)
        want = env.recommend_scales(fov)
        short = sorted(set(want) - set(rec))
        note = f"画角 {fov:.1f}'" + (f" / 別に {short}" if short else "")
        check(f"{'足りる' if enough else '足りない'}: {label}",
              (not short) == enough, note)

    # 切り出した動画の短辺。掩蔽で 968x548 に切ると、短辺はさらに狭い
    fov_w = env.fov_arcmin(800, 5.6)
    fov_h = fov_w * 548 / 968.0
    want = env.recommend_scales(fov_w, fov_h)
    check("おすすめで足りる: 800mm を 968x548 に切り出し",
          set(want) <= set(rec), f"短辺 {fov_h:.1f}'")

    # =========================================================== ボタン本体 ===
    asked = []
    started = []
    g.messagebox.askokcancel = lambda *a, **k: asked.append(a) or False
    g.messagebox.showinfo = lambda *a, **k: asked.append(a)

    app = g.App()
    app.update()
    t0 = time.time()
    while time.time() - t0 < 180 and not app.engine_ready:
        app.update()
        time.sleep(0.05)
    app._bg = lambda *a, **k: started.append(a)

    def press(have):
        asked.clear()
        started.clear()
        app.env_state = {"index": {"scales": list(have)}}
        app._download_recommended()
        app.update()
        return [s for s, v in app.scale_vars.items() if v.get()]

    # --- まっさら ---
    got = press([])
    check("何も無いときは、おすすめを全部選ぶ", sorted(got) == rec,
          f"{len(got)} 段に印")
    check("まっさらなら確認ダイアログが出る", len(asked) == 1)
    check("「いいえ」なら落とし始めない", not started)

    # --- 途中まで持っている ---
    have = [4107, 4108, 5205, 5206]
    got = press(have)
    check("持っている段は選び直さない",
          sorted(got) == [s for s in rec if s not in have],
          f"{len(got)} 段だけ")
    check("5200 / 5201 に印は付かない",
          not app.scale_vars[5200].get() and not app.scale_vars[5201].get())

    # --- 全部そろっている ---
    got = press(rec)
    check("そろっていれば何も選ばない", got == [])
    check("そろっていれば聞きもしない", not asked and not started)
    check("そろっていても状態バーに一言残る",
          "そろって" in app.v_status.get(), app.v_status.get())

    app.destroy()

    print(f"\n  {len(PASS)}/{len(PASS) + len(FAIL)} passed")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())

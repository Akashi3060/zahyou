"""
zahyou — 天体画像から目標への向きを求める (デスクトップ版)

1 つのウィンドウで完結する。タブは 3 つだけ:

    [解析]   画像を選ぶ → 目標を打ち込む → 実行 → 結果と図
    [準備]   WSL / astrometry.net / 星図データ をボタンひとつで用意する
    [使い方] 短い手引き

解析そのものは zahyou_engine.py (ノートブック版と同じもの) をそのまま読み込んで
使う。ここには解析のロジックを書かない ―― 二重管理になると必ずずれるため。
"""
from __future__ import annotations

import io
import json
import os
import queue
import sys
import threading
import time
import traceback
import webbrowser

import tkinter as tk
from tkinter import filedialog, messagebox, ttk

import warnings

import matplotlib
matplotlib.use("Agg")                    # 図はこちらで Tk へ貼るので Agg でよい
# エンジンは plt.show() を呼ぶ (ノートブック用)。Agg では何も起きないだけなので、
# 「非対話なので表示できません」という警告は出さない。
warnings.filterwarnings("ignore", message=".*non-interactive.*")
import matplotlib.pyplot as plt          # noqa: E402
from matplotlib.backends.backend_tkagg import (        # noqa: E402
    FigureCanvasTkAgg, NavigationToolbar2Tk)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import zahyou_env as env                 # noqa: E402

APP_NAME = "zahyou"
APP_VER = "v6"
ENGINE_FILE = "zahyou_engine.py"

BG = "#F4F6F8"
INK = "#14181F"
MUTED = "#5A6472"
ACCENT = "#1F5C8B"
OK_C = "#2E7D32"
NG_C = "#C0392B"


# ============================================================== 小道具 ===

_FAMILIES = None


def _pick_family(candidates, fallback):
    """入っているフォントから最初の 1 つ。Tk の root ができてから呼ぶこと。"""
    global _FAMILIES
    if _FAMILIES is None:
        try:
            import tkinter.font as tkfont
            _FAMILIES = set(tkfont.families())
        except Exception:
            _FAMILIES = set()
    for name in candidates:
        if name in _FAMILIES:
            return name
    return fallback


def ui_font(size=10, bold=False):
    name = _pick_family(("Yu Gothic UI", "Meiryo UI", "Yu Gothic", "MS UI Gothic"),
                        "TkDefaultFont")
    return (name, size, "bold") if bold else (name, size)


def mono_font(size=9):
    return (_pick_family(("HackGen Console", "HackGen", "Consolas", "MS Gothic"),
                         "TkFixedFont"), size)


def settings_path():
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
    d = os.path.join(base, "zahyou")
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, "gui.json")


def find_engine():
    """zahyou_engine.py を探す。exe に同梱したものを最優先。"""
    cands = []
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        cands.append(os.path.join(meipass, ENGINE_FILE))
    if getattr(sys, "frozen", False):
        cands.append(os.path.join(os.path.dirname(sys.executable), ENGINE_FILE))
    here = os.path.dirname(os.path.abspath(__file__))
    cands += [os.path.join(here, ENGINE_FILE),
              os.path.join(os.path.dirname(here), ENGINE_FILE),
              os.path.join(os.getcwd(), ENGINE_FILE)]
    for p in cands:
        if os.path.exists(p):
            return p
    raise FileNotFoundError(
        ENGINE_FILE + " が見つかりません。探した場所:\n  " + "\n  ".join(cands))


# ============================================================== エンジン ===

class Engine:
    """
    zahyou_engine.py を 1 つの名前空間に読み込んで持っておく係。

    ノートブックのセル A と同じことをしている (exec して、設定を同じ名前空間へ
    書き込んでから run() を呼ぶ)。エンジン側には手を入れない。
    """

    def __init__(self):
        self.ns = None
        self.path = None
        self.font = None

    def load(self):
        self.path = find_engine()
        ns = {"__name__": "zahyou_engine", "__file__": self.path}
        with io.open(self.path, encoding="utf-8") as f:
            code = compile(f.read(), self.path, "exec")
        exec(code, ns)
        self.ns = ns
        self.font = ns.get("ZAHYOU_FONT")
        return ns

    # --- 解析 1 回 ------------------------------------------------------
    def analyze(self, params, log, cancelled=lambda: False):
        """
        params は GUI の設定そのまま。戻り値は結果の辞書。

        エンジンの print_solution / report_offsets を包んで、画面に大きく出す
        数値を横取りする (ログを文字列で解析するより確実)。
        """
        ns = self.ns
        if ns is None:
            raise RuntimeError("エンジンが読み込まれていません。")

        ns.update(params)
        ns["_log"] = log                       # エンジンの出力を GUI へ

        got = {}
        orig_ps = ns["print_solution"]
        orig_ro = ns["report_offsets"]

        def print_solution(wcs, shape):
            s = orig_ps(wcs, shape)
            got["summary"], got["shape"] = s, shape
            return s

        def report_offsets(center, target):
            r = orig_ro(center, target)
            got["offsets"] = r
            return r

        ns["print_solution"] = print_solution
        ns["report_offsets"] = report_offsets

        plt.close("all")
        t0 = time.time()
        try:
            import contextlib
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):      # 迷子の print も拾う
                wcs = ns["run"]()
            for line in buf.getvalue().splitlines():
                if line.strip():
                    log(line)
        finally:
            ns["print_solution"] = orig_ps
            ns["report_offsets"] = orig_ro

        figs = [plt.figure(n) for n in sorted(plt.get_fignums())]
        return {"wcs": wcs, "figures": figs, "seconds": time.time() - t0,
                "cancelled": cancelled(), **got}


# ============================================================== 本体 ===

class App(tk.Tk):

    def __init__(self):
        super().__init__()
        self.title(f"{APP_NAME} {APP_VER} — 天体画像から目標への向きを求める")
        self.geometry("1180x780")
        self.minsize(1000, 660)
        self.configure(bg=BG)

        self.engine = Engine()
        self.engine_ready = False
        self.busy = False
        self.cancel_flag = False
        self.jobs = queue.Queue()          # ワーカー → 画面 の受け渡し
        self.canvases = []
        self.fig_slots = {}                # 図を貼る枠 -> 図。窓の伸縮で貼り直す
        self.env_state = None
        self.image_path = tk.StringVar(value="")
        # Tk の変数はメインスレッドからしか触れない。ワーカーはこちらを見る。
        self.index_dir = env.DEFAULT_INDEX_DIR

        self._build_style()
        self._build_vars()
        self._build_ui()
        self._load_settings()

        self.after(60, self._pump)
        self.protocol("WM_DELETE_WINDOW", self._on_close)
        self._bg(self._load_engine, label="エンジンを読み込んでいます...")

    # ---------------------------------------------------------- 見た目 ---
    def _build_style(self):
        st = ttk.Style(self)
        try:
            st.theme_use("vista")
        except tk.TclError:
            pass
        f = ui_font(10)
        st.configure(".", font=f)
        st.configure("TNotebook.Tab", font=ui_font(10), padding=(16, 7))
        st.configure("Head.TLabel", font=ui_font(11, True), foreground=ACCENT)
        st.configure("Muted.TLabel", foreground=MUTED)
        st.configure("Big.TLabel", font=ui_font(20, True), foreground=INK)
        st.configure("BigCap.TLabel", font=ui_font(9), foreground=MUTED)
        st.configure("Run.TButton", font=ui_font(11, True))
        st.configure("OK.TLabel", foreground=OK_C)
        st.configure("NG.TLabel", foreground=NG_C)

    def _build_vars(self):
        self.v_mode = tk.StringVar(value="STAR_NAME")
        self.v_name = tk.StringVar(value="UCAC4 660-021020")
        self.v_ra = tk.StringVar(value="04h 07m 38.877s")
        self.v_dec = tk.StringVar(value="+41d 59m 10.512s")
        self.v_focal = tk.StringVar(value="")
        self.v_solve = tk.StringVar(value="自動 (ネットがあれば使う)")
        self.v_radius = tk.StringVar(value="5.0")
        self.v_hint = tk.BooleanVar(value=True)
        self.v_marks = tk.BooleanVar(value=True)
        self.v_ignore = tk.BooleanVar(value=True)
        self.v_t_online = tk.StringVar(value="120")
        self.v_t_offline = tk.StringVar(value="300")
        self.v_indexdir = tk.StringVar(value=env.DEFAULT_INDEX_DIR)
        self.v_indexdir.trace_add(
            "write", lambda *_: setattr(self, "index_dir", self.v_indexdir.get()))
        self.v_sensor = tk.StringVar(value="")
        self.v_status = tk.StringVar(value="")
        self.scale_vars = {}

    def _build_ui(self):
        self.nb = ttk.Notebook(self)
        self.nb.pack(fill="both", expand=True, padx=8, pady=(8, 0))
        self.tab_run = ttk.Frame(self.nb, padding=8)
        self.tab_env = ttk.Frame(self.nb, padding=8)
        self.tab_help = ttk.Frame(self.nb, padding=8)
        self.nb.add(self.tab_run, text="  解析  ")
        self.nb.add(self.tab_env, text="  準備  ")
        self.nb.add(self.tab_help, text="  使い方  ")

        bar = ttk.Frame(self, padding=(10, 4))
        bar.pack(fill="x")
        ttk.Label(bar, textvariable=self.v_status, style="Muted.TLabel").pack(side="left")
        self.prog = ttk.Progressbar(bar, mode="determinate", length=220)
        self.prog.pack(side="right")

        self._build_run_tab()
        self._build_env_tab()
        self._build_help_tab()

    # ============================================================ 解析タブ ===
    def _build_run_tab(self):
        top = ttk.Frame(self.tab_run)
        top.pack(fill="x", pady=(0, 8))
        ttk.Label(top, text="画像", style="Head.TLabel").pack(side="left")
        e = ttk.Entry(top, textvariable=self.image_path)
        e.pack(side="left", fill="x", expand=True, padx=8)
        ttk.Button(top, text="参照...", command=self._choose_image).pack(side="left")
        self.btn_run = ttk.Button(top, text="解析を実行", style="Run.TButton",
                                  command=self._start_analysis)
        self.btn_run.pack(side="left", padx=(12, 4))
        self.btn_stop = ttk.Button(top, text="中止", command=self._cancel,
                                   state="disabled")
        self.btn_stop.pack(side="left")

        pane = ttk.Panedwindow(self.tab_run, orient="horizontal")
        pane.pack(fill="both", expand=True)
        left = ttk.Frame(pane, padding=(0, 0, 8, 0))
        right = ttk.Frame(pane)
        pane.add(left, weight=0)
        pane.add(right, weight=1)
        left.configure(width=350)
        left.pack_propagate(False)

        # --- 目標 -------------------------------------------------------
        g = ttk.LabelFrame(left, text=" 目標 ", padding=8)
        g.pack(fill="x")
        ttk.Radiobutton(g, text="天体名で指定", value="STAR_NAME",
                        variable=self.v_mode, command=self._sync_mode
                        ).grid(row=0, column=0, columnspan=2, sticky="w")
        self.e_name = ttk.Entry(g, textvariable=self.v_name)
        self.e_name.grid(row=1, column=0, columnspan=2, sticky="ew", pady=(2, 8))
        ttk.Radiobutton(g, text="赤経・赤緯で指定", value="COORDS",
                        variable=self.v_mode, command=self._sync_mode
                        ).grid(row=2, column=0, columnspan=2, sticky="w")
        ttk.Label(g, text="RA").grid(row=3, column=0, sticky="w")
        self.e_ra = ttk.Entry(g, textvariable=self.v_ra)
        self.e_ra.grid(row=3, column=1, sticky="ew", pady=1)
        ttk.Label(g, text="Dec").grid(row=4, column=0, sticky="w")
        self.e_dec = ttk.Entry(g, textvariable=self.v_dec)
        self.e_dec.grid(row=4, column=1, sticky="ew", pady=1)
        g.columnconfigure(1, weight=1)

        # --- 望遠鏡 -----------------------------------------------------
        g2 = ttk.LabelFrame(left, text=" 望遠鏡 ", padding=8)
        g2.pack(fill="x", pady=8)
        ttk.Label(g2, text="焦点距離 [mm]").grid(row=0, column=0, sticky="w")
        ttk.Entry(g2, textvariable=self.v_focal, width=10).grid(row=0, column=1,
                                                                sticky="w")
        ttk.Label(g2, text="空欄でも解けます。入れるとオフラインが速く確実に。",
                  style="Muted.TLabel", wraplength=310).grid(
            row=1, column=0, columnspan=2, sticky="w", pady=(4, 0))

        # --- 解析 -------------------------------------------------------
        g3 = ttk.LabelFrame(left, text=" 解析 ", padding=8)
        g3.pack(fill="both", expand=True)
        ttk.Label(g3, text="方法").grid(row=0, column=0, sticky="w")
        cb = ttk.Combobox(g3, textvariable=self.v_solve, state="readonly",
                          values=["自動 (ネットがあれば使う)",
                                  "オンラインのみ",
                                  "オフライン (WSL) のみ"])
        cb.grid(row=0, column=1, sticky="ew", pady=2)
        ttk.Label(g3, text="探索半径 [°]").grid(row=1, column=0, sticky="w")
        ttk.Entry(g3, textvariable=self.v_radius, width=8).grid(row=1, column=1,
                                                                sticky="w")
        ttk.Checkbutton(g3, text="目標の方向を手がかりにする (速い)",
                        variable=self.v_hint).grid(row=2, column=0, columnspan=2,
                                                   sticky="w", pady=(6, 0))
        ttk.Checkbutton(g3, text="検出した星を緑の丸で囲む",
                        variable=self.v_marks).grid(row=3, column=0, columnspan=2,
                                                    sticky="w")
        ttk.Checkbutton(g3, text="画像に入っている座標情報を無視する",
                        variable=self.v_ignore).grid(row=4, column=0, columnspan=2,
                                                     sticky="w")
        ttk.Label(g3, text="打ち切り [秒]  オンライン").grid(row=5, column=0,
                                                            sticky="w", pady=(8, 0))
        ttk.Entry(g3, textvariable=self.v_t_online, width=8).grid(row=5, column=1,
                                                                  sticky="w",
                                                                  pady=(8, 0))
        ttk.Label(g3, text="                オフライン").grid(row=6, column=0,
                                                              sticky="w")
        ttk.Entry(g3, textvariable=self.v_t_offline, width=8).grid(row=6, column=1,
                                                                   sticky="w")
        g3.columnconfigure(1, weight=1)
        self.lbl_env = ttk.Label(g3, text="", style="Muted.TLabel", wraplength=310)
        self.lbl_env.grid(row=7, column=0, columnspan=2, sticky="sw", pady=(10, 0))

        # --- 右: 結果 ---------------------------------------------------
        self.res = ttk.Frame(right, padding=(0, 0, 0, 6))
        self.res.pack(fill="x")
        self._build_result_head(self.res)

        self.out = ttk.Notebook(right)
        self.out.pack(fill="both", expand=True)
        self.f_fig1 = ttk.Frame(self.out)
        self.f_fig2 = ttk.Frame(self.out)
        self.f_log = ttk.Frame(self.out)
        self.out.add(self.f_fig1, text=" 撮影時の向き ")
        self.out.add(self.f_fig2, text=" 北が上 ")
        self.out.add(self.f_log, text=" ログ ")

        self.txt_log = tk.Text(self.f_log, font=mono_font(9), wrap="none",
                               bg="white", fg=INK, relief="flat")
        sb = ttk.Scrollbar(self.f_log, command=self.txt_log.yview)
        self.txt_log.configure(yscrollcommand=sb.set)
        sb.pack(side="right", fill="y")
        self.txt_log.pack(fill="both", expand=True)

    def _build_result_head(self, parent):
        box = ttk.Frame(parent)
        box.pack(fill="x")
        self.v_r1 = tk.StringVar(value="—")
        self.v_r2 = tk.StringVar(value="—")
        self.v_r3 = tk.StringVar(value="—")
        for i, (cap, var) in enumerate([("画像中心から目標までの距離", self.v_r1),
                                        ("赤経 (RA) 方向", self.v_r2),
                                        ("赤緯 (Dec) 方向", self.v_r3)]):
            cell = ttk.Frame(box, padding=(2, 2))
            cell.grid(row=0, column=i, sticky="ew", padx=(0, 18))
            ttk.Label(cell, text=cap, style="BigCap.TLabel").pack(anchor="w")
            ttk.Label(cell, textvariable=var, style="Big.TLabel").pack(anchor="w")
        box.columnconfigure((0, 1, 2), weight=1)
        self.v_detail = tk.StringVar(value="画像を選んで「解析を実行」を押してください。")
        ttk.Label(parent, textvariable=self.v_detail, style="Muted.TLabel"
                  ).pack(anchor="w", pady=(4, 0))

    # ============================================================ 準備タブ ===
    def _build_env_tab(self):
        pane = ttk.Panedwindow(self.tab_env, orient="horizontal")
        pane.pack(fill="both", expand=True)
        left = ttk.Frame(pane, padding=(0, 0, 8, 0))
        right = ttk.Frame(pane)
        pane.add(left, weight=0)
        pane.add(right, weight=1)
        left.configure(width=430)
        left.pack_propagate(False)

        ttk.Label(left, text="オフライン解析に必要なもの", style="Head.TLabel"
                  ).pack(anchor="w")
        self.tv = ttk.Treeview(left, columns=("state",), show="tree headings",
                               height=5, selectmode="none")
        self.tv.heading("#0", text="項目")
        self.tv.heading("state", text="状態")
        self.tv.column("#0", width=170, anchor="w")
        self.tv.column("state", width=230, anchor="w")
        self.tv.pack(fill="x", pady=6)
        self.tv.tag_configure("ok", foreground=OK_C)
        self.tv.tag_configure("ng", foreground=NG_C)
        for key, name in [("wsl", "WSL"), ("solver", "astrometry.net"),
                          ("index", "星図データ"), ("cfg", "設定ファイル"),
                          ("ready", "総合")]:
            self.tv.insert("", "end", iid=key, text=name, values=("未確認",))

        row = ttk.Frame(left)
        row.pack(fill="x", pady=(2, 8))
        self.btn_all = ttk.Button(row, text="まとめて準備する", style="Run.TButton",
                                  command=self._prepare_all)
        self.btn_all.pack(side="left")
        ttk.Button(row, text="状態を確認", command=self._survey).pack(side="left",
                                                                     padx=6)
        self.btn_env_stop = ttk.Button(row, text="中止", command=self._cancel,
                                       state="disabled")
        self.btn_env_stop.pack(side="left")

        row2 = ttk.Frame(left)
        row2.pack(fill="x")
        ttk.Label(row2, text="個別に:", style="Muted.TLabel").pack(side="left")
        ttk.Button(row2, text="WSL", width=7,
                   command=lambda: self._step(env.install_wsl)).pack(side="left", padx=2)
        ttk.Button(row2, text="astrometry.net", width=15,
                   command=lambda: self._step(env.install_astrometry)).pack(side="left",
                                                                           padx=2)
        ttk.Button(row2, text="設定ファイル", width=13,
                   command=lambda: self._step(
                       lambda log: env.write_cfg(self.index_dir, log))
                   ).pack(side="left", padx=2)

        # --- 星図データ -------------------------------------------------
        g = ttk.LabelFrame(left, text=" 星図データ ", padding=8)
        g.pack(fill="both", expand=True, pady=8)
        r = ttk.Frame(g)
        r.pack(fill="x")
        ttk.Label(r, text="置き場所").pack(side="left")
        ttk.Entry(r, textvariable=self.v_indexdir).pack(side="left", fill="x",
                                                        expand=True, padx=4)
        ttk.Button(r, text="...", width=3, command=self._choose_indexdir).pack(side="left")

        r2 = ttk.Frame(g)
        r2.pack(fill="x", pady=(6, 0))
        ttk.Label(r2, text="焦点距離 [mm]").pack(side="left")
        ttk.Entry(r2, textvariable=self.v_focal, width=8).pack(side="left", padx=(4, 12))
        ttk.Label(r2, text="センサー横幅 [mm]").pack(side="left")
        ttk.Entry(r2, textvariable=self.v_sensor, width=8).pack(side="left", padx=4)
        r3 = ttk.Frame(g)
        r3.pack(fill="x", pady=(4, 2))
        ttk.Button(r3, text="必要な段を選ぶ", command=self._recommend).pack(side="left")
        ttk.Label(r3, text="この 2 つから画角を出して、要る段だけに印を付けます",
                  style="Muted.TLabel").pack(side="left", padx=8)

        wrap = ttk.Frame(g)
        wrap.pack(fill="both", expand=True, pady=(4, 0))
        cv = tk.Canvas(wrap, height=150, bg="white", highlightthickness=1,
                       highlightbackground="#C9D4DE")
        sb = ttk.Scrollbar(wrap, orient="vertical", command=cv.yview)
        inner = ttk.Frame(cv)
        inner.bind("<Configure>",
                   lambda e: cv.configure(scrollregion=cv.bbox("all")))
        win = cv.create_window((0, 0), window=inner, anchor="nw")
        # 中身を横幅いっぱいに広げる (これが無いと右側が白く余る)
        cv.bind("<Configure>", lambda e: cv.itemconfigure(win, width=e.width))
        cv.configure(yscrollcommand=sb.set)
        cv.pack(side="left", fill="both", expand=True)
        sb.pack(side="right", fill="y")
        for s in sorted(env.SCALE_FOV):
            lo, hi = env.SCALE_FOV[s]
            mb = env.SCALE_MB.get(s, 0)
            size = f"{mb/1024:.1f} GB" if mb >= 1024 else f"{mb} MB"
            var = tk.BooleanVar(value=False)
            self.scale_vars[s] = var
            ttk.Checkbutton(inner, variable=var,
                            text=f"index-{s}   画角 {lo:g}′〜{hi:g}′   {size}"
                            ).pack(anchor="w")
        ttk.Button(g, text="選んだ段を落とす",
                   command=self._download_index).pack(anchor="w", pady=(6, 0))

        # --- 右: ログ ---------------------------------------------------
        ttk.Label(right, text="ログ", style="Head.TLabel").pack(anchor="w")
        self.txt_env = tk.Text(right, font=mono_font(9), wrap="word", bg="white",
                               fg=INK, relief="flat")
        sb2 = ttk.Scrollbar(right, command=self.txt_env.yview)
        self.txt_env.configure(yscrollcommand=sb2.set)
        sb2.pack(side="right", fill="y")
        self.txt_env.pack(fill="both", expand=True)

    # ============================================================ 使い方 ===
    def _build_help_tab(self):
        t = tk.Text(self.tab_help, wrap="word", font=ui_font(10), bg="white",
                    fg=INK, relief="flat", padx=14, pady=12)
        sb = ttk.Scrollbar(self.tab_help, command=t.yview)
        t.configure(yscrollcommand=sb.set)
        sb.pack(side="right", fill="y")
        t.pack(fill="both", expand=True)
        t.tag_configure("h", font=ui_font(12, True), foreground=ACCENT,
                        spacing1=10, spacing3=4)
        t.tag_configure("b", font=ui_font(10, True))
        t.tag_configure("m", foreground=MUTED)

        def add(text, tag=None):
            t.insert("end", text + "\n", tag or ())

        add("使い方", "h")
        add("1. [解析] タブで「参照...」を押して、天体画像を選びます。")
        add("   FITS / PNG / JPEG / TIFF が読めます。カラーでも動画キューブでも構いません。")
        add("2. 目標を指定します。天体名 (UCAC4 660-021020 など) か、赤経・赤緯のどちらかです。")
        add("3. 「解析を実行」を押します。")
        add("")
        add("出た数字の読み方", "h")
        add("画面上部の 3 つが答えです。いま向いている点 (画像の中心) から目標まで、")
        add("赤経方向と赤緯方向にどれだけ動かせばよいかを表しています。")
        add("図は 2 枚出ます。「撮影時の向き」は撮ったままの向き、「北が上」は")
        add("北を上・東を左にそろえたものです。赤道儀の軸操作と向きが揃います。")
        add("水色の × が画像の中心、黄色の ○ が目標、赤い矢印が動かす向きです。")
        add("")
        add("ネットが無いところで使う", "h")
        add("[準備] タブで「まとめて準備する」を押すと、次を順に用意します。")
        add("  ・WSL (Windows の中で動く Linux)")
        add("  ・astrometry.net (解析エンジン本体)")
        add("  ・星図データ (画角ごとに分かれた index ファイル)")
        add("  ・/etc/astrometry.cfg (星図データの場所を教える設定)")
        add("WSL を入れた直後だけ Windows の再起動が要ります。", "b")
        add("星図データは大きいので、焦点距離とセンサー横幅を入れて")
        add("「必要な段を選ぶ」を押し、必要な段だけ落とすのがおすすめです。")
        add("途中で止めても、次に押せば続きから落とし直します。", "m")
        add("")
        add("天体名はオフラインでも使えます (2 回目から)", "h")
        add("天体名から座標を調べるにはネットが要りますが、一度調べた名前は")
        add("この PC に記憶されます。自宅で一度実行しておけば、山の中でも")
        add("同じ目標なら名前のまま使えます。")
        add("初めての目標をオフラインで扱うときは、赤経・赤緯を直接入れてください。")
        add("")
        add("困ったとき", "h")
        add("・星が少ないと解けません。露出を伸ばすか、複数枚を重ねてください。")
        add("・オフラインで解けないときは、[準備] タブの「解ける画角」を見て、")
        add("  その画角の段が入っているか確かめてください。")
        add("・焦点距離を入れると、探す範囲が狭まって速く確実になります。")
        add("")
        add(f"{APP_NAME} {APP_VER}   ノートブック版と同じ解析エンジンを使っています。", "m")
        t.insert("end", "https://github.com/Akashi3060/zahyou\n", "link")
        t.tag_configure("link", foreground=ACCENT, underline=True)
        t.tag_bind("link", "<Button-1>",
                   lambda e: webbrowser.open("https://github.com/Akashi3060/zahyou"))
        t.configure(state="disabled")

    # ============================================================ 仕組み ===
    def _pump(self):
        """ワーカーからの用事を画面側で片付ける。"""
        try:
            while True:
                fn = self.jobs.get_nowait()
                try:
                    fn()
                except Exception:
                    # exe (--windowed) には print の出し先が無いので画面のログへ
                    try:
                        self._append(self.txt_log, traceback.format_exc())
                    except Exception:
                        pass
        except queue.Empty:
            pass
        self.after(60, self._pump)

    def _post(self, fn):
        self.jobs.put(fn)

    def _bg(self, fn, label=""):
        """重い処理をワーカーへ。二重に走らせない。"""
        if self.busy:
            messagebox.showinfo(APP_NAME, "いま別の処理が動いています。")
            return
        self.busy = True
        self.cancel_flag = False
        self._set_busy(True, label)

        def wrap():
            try:
                fn()
            except Exception as exc:
                # except 節を抜けると exc は消える。lambda の中に残すと後で
                # NameError になり、失敗の中身が誰にも見えなくなる (実際に踏んだ)。
                msg = f"{type(exc).__name__}: {exc}"
                tb = traceback.format_exc()
                self._post(lambda m=msg, t=tb: self._fail(m, t))
            finally:
                self._post(lambda: self._set_busy(False, ""))
        threading.Thread(target=wrap, daemon=True).start()

    def _set_busy(self, busy, label):
        self.busy = busy
        self.v_status.set(label)
        state = "disabled" if busy else "normal"
        self.btn_run.configure(state=state)
        self.btn_all.configure(state=state)
        self.btn_stop.configure(state="normal" if busy else "disabled")
        self.btn_env_stop.configure(state="normal" if busy else "disabled")
        if busy:
            self.prog.configure(mode="indeterminate")
            self.prog.start(12)
        else:
            self.prog.stop()
            self.prog.configure(mode="determinate", value=0)

    def _fail(self, msg, tb):
        self.log_run(f"\n❌ {msg}")
        for line in tb.splitlines()[-12:]:
            self.log_run("   " + line)
        self.v_status.set("失敗しました。ログを見てください。")

    def _cancel(self):
        self.cancel_flag = True
        self.v_status.set("中止しています...")
        # WSL 側で走っている solve-field を止める (これをしないと粘り続ける)
        threading.Thread(target=self._kill_solver, daemon=True).start()

    def _kill_solver(self):
        for _ in range(30):
            if not self.busy:
                return
            try:
                env.wsl("pkill -f solve-field; pkill -f backend", timeout=30)
            except Exception:
                pass
            time.sleep(2)

    # --- ログ -----------------------------------------------------------
    def _append(self, widget, text):
        widget.configure(state="normal")
        widget.insert("end", text + "\n")
        widget.see("end")

    def log_run(self, text=""):
        self._post(lambda: self._append(self.txt_log, str(text)))

    def log_env(self, text=""):
        self._post(lambda: self._append(self.txt_env, str(text)))

    # ============================================================ 起動時 ===
    def _load_engine(self):
        t0 = time.time()
        self.engine.load()
        el = time.time() - t0
        self.engine_ready = True
        self._post(lambda: self.v_status.set(
            f"準備できました ({el:.1f} 秒 / 図のフォント: "
            f"{self.engine.font or '日本語フォントなし'})"))
        self.log_run(f"エンジン: {self.engine.path}")
        # 起動ついでに環境も見ておく (画面はふさがない)
        threading.Thread(target=self._survey_quiet, daemon=True).start()

    def _survey_quiet(self):
        try:
            st = env.survey(self.index_dir)
        except Exception:
            self.log_env("環境の確認に失敗しました:\n" + traceback.format_exc())
            return
        self.env_state = st
        self._post(lambda: self._show_env(st))

    # ============================================================ 解析 ===
    def _choose_image(self):
        p = filedialog.askopenfilename(
            title="天体画像を選ぶ",
            filetypes=[("天体画像", "*.fits *.fit *.fts *.fz *.png *.jpg *.jpeg "
                                    "*.tif *.tiff"),
                       ("FITS", "*.fits *.fit *.fts *.fz"),
                       ("画像", "*.png *.jpg *.jpeg *.tif *.tiff"),
                       ("すべて", "*.*")])
        if p:
            self.image_path.set(p)

    def _sync_mode(self):
        star = self.v_mode.get() == "STAR_NAME"
        self.e_name.configure(state="normal" if star else "disabled")
        for w in (self.e_ra, self.e_dec):
            w.configure(state="disabled" if star else "normal")

    def _params(self):
        def num(s, default=None):
            s = (s or "").strip()
            if not s:
                return default
            try:
                return float(s)
            except ValueError:
                return default

        mode = {"自動 (ネットがあれば使う)": "AUTO",
                "オンラインのみ": "ONLINE",
                "オフライン (WSL) のみ": "OFFLINE"}[self.v_solve.get()]
        return {
            "INPUT_MODE": self.v_mode.get(),
            "TARGET_STAR_NAME": self.v_name.get().strip(),
            "RA_INPUT_STR": self.v_ra.get().strip(),
            "DEC_INPUT_STR": self.v_dec.get().strip(),
            "FOCAL_LENGTH_MM": num(self.v_focal.get()),
            "SOLVE_MODE": mode,
            "ONLINE_TIMEOUT": int(num(self.v_t_online.get(), 120)),
            "OFFLINE_TIMEOUT": int(num(self.v_t_offline.get(), 300)),
            "USE_TARGET_AS_HINT": bool(self.v_hint.get()),
            "SEARCH_RADIUS_DEG": num(self.v_radius.get(), 5.0),
            "IGNORE_EXISTING_WCS": bool(self.v_ignore.get()),
            "SHOW_DETECTED_SOURCES": bool(self.v_marks.get()),
            "IMAGE_PATH": self.image_path.get().strip() or None,
            "API_KEY": os.environ.get("ASTROMETRY_API_KEY", "axvhxwfkvnzreobn"),
        }

    def _start_analysis(self):
        if not self.engine_ready:
            messagebox.showinfo(APP_NAME, "エンジンを読み込み中です。少し待ってください。")
            return
        path = self.image_path.get().strip()
        if not path or not os.path.exists(path):
            messagebox.showwarning(APP_NAME, "画像を選んでください。")
            return
        if self.v_mode.get() == "STAR_NAME" and not self.v_name.get().strip():
            messagebox.showwarning(APP_NAME, "目標の天体名を入れてください。")
            return
        self.txt_log.delete("1.0", "end")
        self._clear_figures()
        for v in (self.v_r1, self.v_r2, self.v_r3):
            v.set("—")
        self.v_detail.set("解析しています...")
        self.out.select(self.f_log)
        params = self._params()
        self._save_settings()
        self._bg(lambda: self._analysis_worker(params), "解析しています...")

    def _analysis_worker(self, params):
        res = self.engine.analyze(params, self.log_run,
                                  cancelled=lambda: self.cancel_flag)
        self._post(lambda: self._show_result(res))

    def _clear_figures(self):
        for c in self.canvases:
            try:
                c.get_tk_widget().destroy()
            except Exception:
                pass
        self.canvases = []
        for f in (self.f_fig1, self.f_fig2):
            slot = self.fig_slots.pop(f, None)
            if slot and slot.get("job"):
                self.after_cancel(slot["job"])
            f.unbind("<Configure>")
            for w in f.winfo_children():
                w.destroy()

    def _show_result(self, res):
        if self.cancel_flag:
            self.v_detail.set("中止しました。")
            self.v_status.set("中止しました。")
            return
        if res.get("wcs") is None:
            self.v_detail.set("解けませんでした。ログを見てください。")
            self.v_status.set("解けませんでした。")
            return

        s = res.get("summary") or {}
        off = res.get("offsets")
        if off:
            sep, ra_min, dec_min, ra_dir, dec_dir = off
            self.v_r1.set(f"{sep.arcmin:.3f}′")
            self.v_r2.set(f"{abs(ra_min):.3f}′ {ra_dir}")
            self.v_r3.set(f"{abs(dec_min):.3f}′ {dec_dir}")
        else:
            for v in (self.v_r1, self.v_r2, self.v_r3):
                v.set("—")
        if s:
            self.v_detail.set(
                f"画像中心 {s['center'].to_string('hmsdms', precision=1)}   "
                f"画素スケール {s['scale']:.4f}″/px   "
                f"視野 {s['fov_x']:.2f}′ × {s['fov_y']:.2f}′   "
                f"所要 {res['seconds']:.1f} 秒")
        self.v_status.set("解析できました。")

        figs = res.get("figures") or []
        for fig, frame in zip(figs, (self.f_fig1, self.f_fig2)):
            self._embed(fig, frame)
        self.out.select(self.f_fig2 if len(figs) > 1 else self.f_fig1)

    def _embed(self, fig, frame):
        """図を 1 枚貼る。窓の幅が変わったら貼り直す。"""
        self.fig_slots[frame] = {"fig": fig, "w": 0, "job": None}
        frame.bind("<Configure>", lambda e, f=frame: self._refit(f))
        self._render(frame)

    def _refit(self, frame):
        slot = self.fig_slots.get(frame)
        if not slot:
            return
        w = frame.winfo_width()
        if abs(w - slot["w"]) < 40:          # ちらつき防止 (再描画は重い)
            return
        if slot["job"]:
            self.after_cancel(slot["job"])
        slot["job"] = self.after(350, lambda f=frame: self._render(f))

    def _render(self, frame):
        slot = self.fig_slots.get(frame)
        if not slot:
            return
        slot["job"] = None
        for w in frame.winfo_children():
            w.destroy()
        self.update_idletasks()

        fig = slot["fig"]
        w_px = max(frame.winfo_width(), 560) - 20
        h_px = max(frame.winfo_height(), 380) - 60      # ツールバーのぶん
        slot["w"] = frame.winfo_width()
        dpi = 100
        w_in, h_in = fig.get_size_inches()
        aspect = h_in / w_in
        w_new = w_px / dpi
        if w_new * aspect > h_px / dpi:                 # 縦がはみ出すなら合わせる
            w_new = (h_px / dpi) / aspect
        fig.set_dpi(dpi)
        fig.set_size_inches(w_new, w_new * aspect)
        try:
            fig.tight_layout()      # 大きさを変えたので組み直す (題が切れる)
        except Exception:
            pass

        canvas = FigureCanvasTkAgg(fig, master=frame)
        canvas.draw()
        tb = NavigationToolbar2Tk(canvas, frame, pack_toolbar=False)
        tb.update()
        tb.pack(side="bottom", fill="x")
        canvas.get_tk_widget().pack(fill="both", expand=True)
        slot["canvas"] = canvas
        # 貼り直しのたびに増えないよう、今ある枠ぶんだけを持ち直す
        self.canvases = [s["canvas"] for s in self.fig_slots.values()
                         if s.get("canvas")]

    # ============================================================ 準備 ===
    def _choose_indexdir(self):
        d = filedialog.askdirectory(title="星図データの置き場所")
        if d:
            self.v_indexdir.set(os.path.normpath(d))

    def _survey(self):
        self._bg(self._survey_worker, "環境を調べています...")

    def _survey_worker(self):
        self.log_env("状態を確認しています...")
        st = env.survey(self.index_dir)
        self.env_state = st
        for m in st["notes"]:
            self.log_env("  ⚠️ " + m)
        self._post(lambda: self._show_env(st))

    def _show_env(self, st):
        idx = st["index"]
        fov = idx.get("fov")
        rows = {
            "wsl": (st["wsl"], st["distro"] or ("使えます" if st["wsl"] else "入っていません")),
            "solver": (bool(st["solver"]), st["solver"] or "入っていません"),
            "index": (idx["ok"], (f"{idx['files']} ファイル / {env.human(idx['bytes'])}"
                                  if idx["ok"] else f"{idx['dir']} にありません")),
            "cfg": (bool(st["cfg_paths"]),
                    ", ".join(st["cfg_paths"]) or "書かれていません"),
            "ready": (st["ready"], "オフライン解析できます" if st["ready"]
                      else "まだ足りません"),
        }
        for key, (ok, text) in rows.items():
            self.tv.item(key, values=(("OK  " if ok else "—  ") + text,),
                         tags=("ok" if ok else "ng",))
        msg = "オフライン解析: " + ("使えます" if st["ready"] else "まだ準備が要ります")
        if fov:
            msg += f" / 解ける画角 {fov[0]:g}′〜{fov[1]:g}′"
        self.lbl_env.configure(text=msg)
        for s, var in self.scale_vars.items():          # 既にある段は外しておく
            if s in idx["scales"]:
                var.set(False)

    def _step(self, fn):
        """個別ボタン。fn(log) を呼ぶだけ。"""
        def work():
            fn(self.log_env)
            st = env.survey(self.index_dir)
            self.env_state = st
            self._post(lambda: self._show_env(st))
        self._bg(work, "実行しています...")

    def _recommend(self):
        try:
            focal = float(self.v_focal.get())
            sensor = float(self.v_sensor.get())
        except ValueError:
            messagebox.showinfo(
                APP_NAME,
                "焦点距離とセンサー横幅 [mm] を入れてください。\n"
                "例: 焦点距離 800、センサー横幅 7.4 (1/1.8 型)\n"
                "分からなければ、画像 1 枚を解析すると下に「逆算した焦点距離」が出ます。")
            return
        fov = env.fov_arcmin(focal, sensor)
        want = env.recommend_scales(fov)
        have = (self.env_state or {}).get("index", {}).get("scales", [])
        for s, var in self.scale_vars.items():
            var.set(s in want and s not in have)
        need = [s for s in want if s not in have]
        mb = sum(env.SCALE_MB.get(s, 0) for s in need)
        self.log_env(f"画角 {fov:.1f}′ → 必要な段 {want}")
        self.log_env(f"  まだ無いのは {need or 'なし'} (約 {mb/1024:.1f} GB)")

    def _download_index(self):
        scales = [s for s, v in self.scale_vars.items() if v.get()]
        if not scales:
            messagebox.showinfo(APP_NAME, "落とす段にチェックを入れてください。")
            return
        mb = sum(env.SCALE_MB.get(s, 0) for s in scales)
        if not messagebox.askokcancel(
                APP_NAME,
                f"{len(scales)} 段 / 約 {mb/1024:.1f} GB を\n"
                f"{self.v_indexdir.get()} へ落とします。よろしいですか?\n\n"
                "途中で中止しても、次に押せば続きから再開します。"):
            return
        self._bg(lambda: self._download_worker(scales), "星図データを落としています...")

    def _download_worker(self, scales):
        env.fetch_index(scales, self.index_dir, self.log_env,
                        progress=self._progress,
                        stop=lambda: self.cancel_flag)
        env.write_cfg(self.index_dir, self.log_env)
        st = env.survey(self.index_dir)
        self.env_state = st
        self._post(lambda: self._show_env(st))

    def _progress(self, done, total):
        def set_it():
            self.prog.stop()
            self.prog.configure(mode="determinate", maximum=max(total, 1), value=done)
            self.v_status.set(f"{env.human(done)} / {env.human(total)}")
        self._post(set_it)

    def _prepare_all(self):
        scales = [s for s, v in self.scale_vars.items() if v.get()]
        if not scales:
            if not messagebox.askokcancel(
                    APP_NAME,
                    "星図データの段が選ばれていません。\n"
                    "WSL と astrometry.net の準備だけ進めますか?\n\n"
                    "(段は下のチェックか「必要な段を選ぶ」で指定できます)"):
                return
        self._bg(lambda: self._prepare_worker(scales), "準備しています...")

    def _prepare_worker(self, scales):
        st = env.prepare_all(self.index_dir, scales, self.log_env,
                             progress=self._progress,
                             stop=lambda: self.cancel_flag)
        self.env_state = st
        self._post(lambda: self._show_env(st))

    # ============================================================ 設定保存 ===
    def _save_settings(self):
        data = {k: v.get() for k, v in {
            "mode": self.v_mode, "name": self.v_name, "ra": self.v_ra,
            "dec": self.v_dec, "focal": self.v_focal, "solve": self.v_solve,
            "radius": self.v_radius, "t_online": self.v_t_online,
            "t_offline": self.v_t_offline, "indexdir": self.v_indexdir,
            "sensor": self.v_sensor, "image": self.image_path,
        }.items()}
        data.update({"hint": self.v_hint.get(), "marks": self.v_marks.get(),
                     "ignore": self.v_ignore.get()})
        try:
            with io.open(settings_path(), "w", encoding="utf-8") as f:
                json.dump(data, f, ensure_ascii=False, indent=1)
        except OSError:
            pass

    def _load_settings(self):
        try:
            with io.open(settings_path(), encoding="utf-8") as f:
                d = json.load(f)
        except (OSError, ValueError):
            d = {}
        pairs = {"mode": self.v_mode, "name": self.v_name, "ra": self.v_ra,
                 "dec": self.v_dec, "focal": self.v_focal, "solve": self.v_solve,
                 "radius": self.v_radius, "t_online": self.v_t_online,
                 "t_offline": self.v_t_offline, "indexdir": self.v_indexdir,
                 "sensor": self.v_sensor, "image": self.image_path}
        for k, var in pairs.items():
            if isinstance(d.get(k), str) and d[k]:
                var.set(d[k])
        for k, var in (("hint", self.v_hint), ("marks", self.v_marks),
                       ("ignore", self.v_ignore)):
            if isinstance(d.get(k), bool):
                var.set(d[k])
        self._sync_mode()

    def _on_close(self):
        self._save_settings()
        self.destroy()


# ============================================================== 自己診断 ===

def selftest(image, target="UCAC4 660-021020", offline_first=True, log_out=None):
    """
    画面を組み立てて解析を 1 回通し、結果を [(項目, 可否, 補足)] で返す。

    exe に固めたあとでも同じ確認ができるように、テスト側ではなくこちらに置く
    (exe には test_gui.py が入らないため)。
      zahyou.exe --selftest "画像パス" --out 結果.txt
    """
    out = []

    def check(name, ok, msg=""):
        out.append((name, bool(ok), str(msg)))

    def pump(app, seconds, until=None):
        t0 = time.time()
        while time.time() - t0 < seconds:
            app.update()
            if until is not None and until():
                return True
            time.sleep(0.05)
        return until is None

    if not os.path.exists(image):
        check("画像がある", False, image)
        return out

    app = App()
    app.update()
    check("画面が組み立てられる", True)

    ok = pump(app, 180, lambda: app.engine_ready)
    if ok:
        check("エンジンを読み込める", True, app.engine.path or "")
    else:
        # 失敗したときは、なぜ落ちたかがログにしか出ない。ここへ持ってくる。
        tail = app.txt_log.get("1.0", "end").strip().splitlines()[-6:]
        check("エンジンを読み込める", False,
              (app.engine.path or "") + " | " + " / ".join(tail))
        app.destroy()
        return out

    pump(app, 120, lambda: app.env_state is not None)
    st = app.env_state or {}
    check("環境を調べられる", bool(st), st.get("distro", ""))
    offline = bool(st.get("ready")) and offline_first

    app.image_path.set(image)
    app.v_mode.set("STAR_NAME")
    app.v_name.set(target)
    app.v_solve.set("オフライン (WSL) のみ" if offline else "自動 (ネットがあれば使う)")
    app._sync_mode()
    app.update()

    t0 = time.time()
    app._start_analysis()
    done = pump(app, 420, lambda: not app.busy)
    check("解析が終わる", done, f"{time.time() - t0:.1f} 秒 / "
          f"{'オフライン' if offline else '自動'}")
    if done:
        pump(app, 3)
        check("距離が出る", app.v_r1.get() not in ("—", ""), app.v_r1.get())
        check("赤経方向が出る", app.v_r2.get() not in ("—", ""), app.v_r2.get())
        check("赤緯方向が出る", app.v_r3.get() not in ("—", ""), app.v_r3.get())
        check("図が 2 枚貼られる", len(app.canvases) == 2, f"{len(app.canvases)} 枚")
        check("画像中心などが出る", "画素スケール" in app.v_detail.get(),
              app.v_detail.get()[:70])
        check("ログが流れている", "解析結果" in app.txt_log.get("1.0", "end"))
    if log_out is not None:                     # 失敗したとき中身を見るため
        log_out.append(app.txt_log.get("1.0", "end"))
    app.destroy()
    return out


def _run_selftest(argv):
    image = argv[argv.index("--selftest") + 1]
    out_path = argv[argv.index("--out") + 1] if "--out" in argv else None
    log_out = []
    rows = selftest(image, log_out=log_out)
    width = max([len(r[0]) for r in rows] or [1])
    lines = [f"  {'PASS' if ok else 'FAIL'}  {name.ljust(width)}  {msg}"
             for name, ok, msg in rows]
    n_ok = sum(1 for _, ok, _ in rows if ok)
    lines.append("")
    lines.append(f"  {n_ok}/{len(rows)} passed")
    if log_out:
        lines += ["", "--- 解析ログ " + "-" * 46, log_out[0].rstrip()]
    text = "\n".join(lines)
    if out_path:
        with io.open(out_path, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    print(text)
    return 0 if n_ok == len(rows) and rows else 1


def _engine_check(argv):
    """
    エンジンだけを読み込んでみる (画面は出さない)。

    固めた exe で「読み込みが返ってこない」ときに、どこで止まっているかを
    faulthandler にスタックごと吐かせる。--out にすべて書く。
    """
    import faulthandler
    out_path = argv[argv.index("--out") + 1] if "--out" in argv else "engine.txt"
    f = io.open(out_path, "w", encoding="utf-8", errors="replace")
    f.write(f"frozen={getattr(sys, 'frozen', False)}\n"
            f"executable={sys.executable}\n"
            f"meipass={getattr(sys, '_MEIPASS', None)}\n")
    f.flush()
    faulthandler.dump_traceback_later(45, repeat=True, file=f)
    t0 = time.time()
    try:
        e = Engine()
        e.load()
        f.write(f"OK {time.time() - t0:.1f} 秒 / path={e.path} / font={e.font}\n")
        code = 0
    except Exception:
        f.write(f"NG {time.time() - t0:.1f} 秒\n" + traceback.format_exc())
        code = 1
    faulthandler.cancel_dump_traceback_later()
    f.close()
    return code


def main():
    if "--engine-check" in sys.argv:
        return _engine_check(sys.argv)
    if "--selftest" in sys.argv:
        return _run_selftest(sys.argv)
    App().mainloop()
    return 0


if __name__ == "__main__":
    sys.exit(main())

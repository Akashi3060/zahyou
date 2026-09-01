"""
WSL の solve-field を模した stub で、オフライン経路の組み立てを検証する。

このマシンには WSL が入っていないため solve-field 本体は動かせない。
そこで「WSL に送られたコマンド文字列」を受け取り、
  * 引数が solve-field の文法として妥当か
  * xylist が実在して読めるか
  * --scale-low/--scale-high が真のスケールを挟んでいるか
  * --ra/--dec/--radius が真の視野中心を含むか
を検査し、条件を満たしたときだけ .wcs を書き出す。
何回目の試行で成功させるかを変えて、リトライ梯子が働くことも確かめる。
"""
import io
import os
import re
import shlex
import shutil
import sys
import warnings

warnings.filterwarnings("ignore")
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.font_manager as fm
for _n in ("Yu Gothic", "Meiryo", "MS Gothic"):
    if any(f.name == _n for f in fm.fontManager.ttflist):
        matplotlib.rcParams["font.family"] = _n
        break
matplotlib.rcParams["axes.unicode_minus"] = False

import numpy as np
from astropy.io import fits

HERE = os.path.dirname(os.path.abspath(__file__))
# テストごとに真っさらな状態から始めたいので、記憶ファイルを専用のものにする
os.environ["ZAHYOU_CACHE"] = os.path.join(HERE, "tcase", "sim_cache.json")
os.makedirs(os.path.join(HERE, "tcase"), exist_ok=True)
# スタブが「解けた」ときに置く .wcs。無ければその場で作る
# (以前は手元の作業ディレクトリを指していたので、他の PC では動かなかった)
TRUTH_WCS = os.path.join(HERE, "tcase", "truth.wcs")
TEST_IMAGE = r"C:\Users\yoshi\Downloads\Capture_00001 00_10_33.fits"

TRUE_SCALE = 1.4616           # arcsec/px  (nova の解から)
TRUE_RA, TRUE_DEC = 61.8914, 41.9648

INDEX_LIST = ([f"index-41{n:02d}.fits" for n in range(7, 20)] +
              [f"index-5205-{n:02d}.fits" for n in range(48)] +
              [f"index-5206-{n:02d}.fits" for n in range(48)])

CFG = ("inparallel\ncpulimit 300\nautoindex\nadd_path /mnt/c/AstrometryData\n")


class Result:
    def __init__(self, rc=0, out="", err=""):
        self.returncode, self.stdout, self.stderr = rc, out, err


class Stub:
    def __init__(self, succeed_on=1, true_scale=None, true_radec=None,
                 indexes=None, shape=(548, 968)):
        self.calls = []
        self.solve_calls = 0
        self.succeed_on = succeed_on
        self.problems = []
        self.true_scale = TRUE_SCALE if true_scale is None else true_scale
        self.true_radec = (TRUE_RA, TRUE_DEC) if true_radec is None else true_radec
        self.indexes = INDEX_LIST if indexes is None else indexes
        self.shape = shape

    # -- solve-field の引数を検査する -------------------------------------
    def _check_solve(self, argv):
        p = self.problems
        known = {
            "--overwrite", "--no-plots", "--no-tweak",
        }
        takes_value = {
            "--dir", "--new-fits", "--rdls", "--match", "--solved",
            "--index-xyls", "--width", "--height", "--x-column", "--y-column",
            "--sort-column", "--cpulimit", "--objs", "--ra", "--dec",
            "--radius", "--scale-units", "--scale-low", "--scale-high",
            "--downsample", "--corr",
        }
        opts = {}
        target = None
        i = 1
        while i < len(argv):
            a = argv[i]
            if a in takes_value:
                if i + 1 >= len(argv):
                    p.append(f"{a} に値がない")
                    break
                opts[a] = argv[i + 1]
                i += 2
                continue
            if a in known:
                i += 1
                continue
            if a.startswith("-"):
                p.append(f"未知のオプション {a}")
                i += 1
                continue
            if target is None:
                target = a
            else:
                p.append(f"位置引数が 2 つ以上ある: {target!r} と {a!r}")
            i += 1

        if target is None:
            p.append("解析対象のファイルが指定されていない")
            return opts, None

        # /mnt/<drive>/... を Windows パスへ戻して実在を確かめる
        m = re.match(r"^/mnt/([a-z])/(.*)$", target)
        if not m:
            p.append(f"WSL パスになっていない: {target}")
            return opts, None
        win = f"{m.group(1).upper()}:\\" + m.group(2).replace("/", "\\")
        if not os.path.exists(win):
            p.append(f"渡されたファイルが存在しない: {win}")
            return opts, None

        is_xylist = win.lower().endswith(".xyls")
        if is_xylist:
            for k in ("--width", "--height", "--x-column", "--y-column"):
                if k not in opts:
                    p.append(f"xylist なのに {k} が無い")
            try:
                t = fits.getdata(win, 1)
                if len(t) == 0:
                    p.append("xylist が空")
                if not {"X", "Y", "FLUX"} <= set(t.columns.names):
                    p.append(f"xylist の列名が想定と違う: {t.columns.names}")
                if opts.get("--x-column") not in t.columns.names:
                    p.append("--x-column が実際の列名と一致しない")
                if (int(opts.get("--width", 0)) != self.shape[1]
                        or int(opts.get("--height", 0)) != self.shape[0]):
                    p.append(f"--width/--height が画像と違う: {opts.get('--width')}x{opts.get('--height')}")
            except Exception as e:
                p.append(f"xylist を読めない: {e}")
            if "--downsample" in opts:
                p.append("xylist に --downsample を付けている (画像専用)")
        else:
            try:
                d = fits.getdata(win)
                if d.ndim != 2:
                    p.append(f"solver 用 FITS が 2 次元でない: {d.shape}")
            except Exception as e:
                p.append(f"solver 用 FITS を読めない: {e}")

        out_dir = opts.get("--dir")
        if not out_dir:
            p.append("--dir が無い")
        return opts, (win, is_xylist, out_dir)

    # -- 解けるかどうかの判定 ---------------------------------------------
    def _would_solve(self, opts):
        if "--scale-low" in opts and "--scale-high" in opts:
            lo, hi = float(opts["--scale-low"]), float(opts["--scale-high"])
            if not (lo <= self.true_scale <= hi):
                return False, f"スケール範囲 {lo}-{hi} が真値 {self.true_scale} を外している"
        tra, tdec = self.true_radec
        if "--ra" in opts and "--dec" in opts:
            ra, dec, rad = float(opts["--ra"]), float(opts["--dec"]), float(opts.get("--radius", 1))
            d = np.hypot((ra - tra) * np.cos(np.radians(tdec)), dec - tdec)
            if d > rad:
                return False, f"探索円 {rad}° が視野中心から {d:.2f}° 外れている"
        return True, ""

    def __call__(self, command, timeout):
        self.calls.append(command)
        if command.strip() == "echo ok":
            return Result(0, "ok\n")
        if "command -v solve-field" in command:
            return Result(0, "/usr/bin/solve-field\n")
        if "cat /etc/astrometry.cfg" in command:
            return Result(0, CFG)
        if command.startswith("for d in"):
            return Result(0, "\n".join("/mnt/c/AstrometryData/" + n for n in self.indexes))
        if command.startswith("solve-field"):
            self.solve_calls += 1
            argv = shlex.split(command)
            opts, info = self._check_solve(argv)
            if info is None:
                return Result(1, "", "stub: 引数が不正\n")
            win, is_xylist, out_dir = info
            solvable, why = self._would_solve(opts)
            if not solvable:
                return Result(0, f"Did not solve (ran out of possibilities)\nstub: {why}\n")
            if self.solve_calls < self.succeed_on:
                return Result(0, "simplexy: found 17 sources.\n"
                                 "Did not solve (ran out of possibilities).\n")
            # 成功: .wcs を出力ディレクトリへ置く
            m = re.match(r"^/mnt/([a-z])/(.*)$", out_dir)
            win_dir = f"{m.group(1).upper()}:\\" + m.group(2).replace("/", "\\")
            base = os.path.splitext(os.path.basename(win))[0]
            shutil.copyfile(os.path.abspath(TRUTH_WCS),
                            os.path.join(win_dir, base + ".wcs"))
            return Result(0, "Field 1: solved with index index-5206-11.fits.\n"
                             "Field 1 solved: writing to file %s.wcs\n" % base)
        return Result(0, "")


def run_case(label, succeed_on, overrides, tag, stub_kwargs=None,
             fresh_cache=True):
    if fresh_cache and os.path.exists(os.environ["ZAHYOU_CACHE"]):
        os.remove(os.environ["ZAHYOU_CACHE"])
    src = io.open(os.path.join(HERE, "_cell2_check.py"), encoding="utf-8").read()
    src = src.replace("\n_result_wcs = run()\n", "\n")
    for k, v in overrides.items():
        pat = re.compile(rf"^{k} = .*$", re.M)
        assert pat.search(src), k
        src = pat.sub(lambda m, v=v, k=k: f"{k} = " + repr(v), src, count=1)

    g = {"__name__": "zahyou_sim", "ZAHYOU_PICKED": {"path": None, "name": None}}
    exec(compile(src, "cell2", "exec"), g)

    stub = Stub(succeed_on=succeed_on, **(stub_kwargs or {}))
    g["_bash"] = stub
    g["to_solver_path"] = lambda p: "/mnt/" + os.path.splitdrive(os.path.abspath(p))[0][0].lower() \
        + os.path.splitdrive(os.path.abspath(p))[1].replace("\\", "/")

    shots = []
    def fake_show():
        fn = f"{tag}_{len(shots)}.png"
        plt.savefig(fn, dpi=80, bbox_inches="tight")
        shots.append(fn)
        plt.close("all")
    plt.show = fake_show
    g["plt"].show = fake_show

    print("\n" + "#" * 70)
    print(f"# {label}   (stub は {succeed_on} 回目の solve-field で成功させる)")
    print("#" * 70)
    wcs = g["run"]()

    print(f"\n-- stub が受け取った solve-field: {stub.solve_calls} 回")
    for c in stub.calls:
        if c.startswith("solve-field"):
            print("   ", c[:150])
    if stub.problems:
        print("-- 引数の問題:")
        for p in stub.problems:
            print("   !", p)
    print("-- 図:", shots)
    return wcs, stub


INDEX_WITH_FINE = INDEX_LIST + [f"index-52{s}-{n:02d}.fits"
                                for s in ("02", "03", "04") for n in range(48)]

# 968 px 幅で 0.30″/px = 画角 4.84′。ASI290MM bin2 なら焦点距離およそ 4000 mm。
NARROW_SCALE = 0.30


def ensure_truth_wcs():
    """
    スタブが返す .wcs を用意する。中身は「解けたときに出てくるはずの WCS」で、
    nova の解 (TRUE_RA / TRUE_DEC / TRUE_SCALE) から組み立てる。
    実機の solve-field は要らない。
    """
    if os.path.exists(TRUTH_WCS):
        return
    from astropy.wcs import WCS
    w = WCS(naxis=2)
    w.wcs.ctype = ["RA---TAN", "DEC--TAN"]
    w.wcs.crval = [TRUE_RA, TRUE_DEC]
    w.wcs.crpix = [968 / 2.0, 548 / 2.0]
    d = TRUE_SCALE / 3600.0
    w.wcs.cd = np.array([[-d, 0.0], [0.0, d]])
    h = w.to_header()
    h["IMAGEW"], h["IMAGEH"] = 968, 548
    os.makedirs(os.path.dirname(TRUTH_WCS), exist_ok=True)
    fits.PrimaryHDU(header=h).writeto(TRUTH_WCS, overwrite=True)


def main():
    ensure_truth_wcs()
    common = {"IMAGE_PATH": TEST_IMAGE, "SOLVE_MODE": "OFFLINE",
              "INPUT_MODE": "COORDS", "OFFLINE_TIMEOUT": 300}
    blind = dict(common, RA_INPUT_STR="", DEC_INPUT_STR="",
                 USE_TARGET_AS_HINT=False)

    results = []
    results.append(("A", *run_case("A. 1 回目 (座標ヒント + スケール既知) で成功", 1,
                                   dict(common, FOCAL_LENGTH_MM=818.0), "simA"), True))
    results.append(("B", *run_case("B. スケール不明・ヒント有り、4 回目でようやく成功", 4,
                                   dict(common), "simB"), True))
    results.append(("C", *run_case("C. 目標未指定 (ヒント無し) でも解ける", 2,
                                   dict(blind), "simC"), True))
    # --- 長焦点 (狭画角) を、設定を一切書かずに解けるか --------------------
    results.append(("D", *run_case(
        f"D. 長焦点 {NARROW_SCALE}″/px (画角 4.8′) を設定なしで解く / 細かい index あり",
        1, dict(blind), "simD",
        stub_kwargs=dict(true_scale=NARROW_SCALE, indexes=INDEX_WITH_FINE)), True))
    # --- 細かい index が無ければ、黙って失敗せず理由を出すこと --------------
    results.append(("E", *run_case(
        f"E. 同じ画像で細かい index が無い → 解けないことを説明する",
        1, dict(blind), "simE",
        stub_kwargs=dict(true_scale=NARROW_SCALE, indexes=INDEX_LIST)), False))

    ok = True
    for name, w, s, expect_solved in results:
        solved = w is not None
        good = (solved == expect_solved) and not s.problems
        ok &= good
        print(f"\n[{name}] {'PASS' if good else 'FAIL'}  "
              f"解けた={solved} (期待={expect_solved}) / "
              f"solve-field {s.solve_calls} 回 / 引数の問題 {len(s.problems)} 件")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

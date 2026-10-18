# -*- coding: utf-8 -*-
"""README とマニュアルの画面写真(docs/img/app-*.png)を、Rust 版の zahyou.exe の --shot で撮り直す。

    python rs/tools/shoot_docs.py                        # 撮って docs/img へ書く
    python rs/tools/shoot_docs.py --out <フォルダー>      # 別の場所へ書く(見比べたいとき)
    python rs/tools/shoot_docs.py --offline              # 方法を「オフラインのみ」にして撮る
    python rs/tools/shoot_docs.py --only app-help        # 1 枚だけ(その写真を含む撮影だけ走らせる)

- exe は既定で rs/target/release/zahyou.exe(CARGO_TARGET_DIR があればその下)。--exe で指定できる。
- 撮るたびに一時フォルダーを作り、ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA をそこへ向ける。
  利用者の %LOCALAPPDATA%\\zahyou(cache.json・gui.json)は読みも書きもしない(前後で sha256 を比べて確かめる)。
- cache.json には目標の天体名の座標だけを入れておく(機材の画素スケールは覚えていない状態 = 初めて使う人と同じ)。
- **外へ何も送らない**: API キーの欄には既定のキーが入る(伏せ字で写る)ので、方法が「自動」のまま解析すると、ネットがあれば
  nova.astrometry.net へ星の位置を送ってしまう。そこで解析する撮影(--run)と、ネットの要らない撮影は、つながらない
  プロキシ(0.0.0.0:9)を環境変数 ALL_PROXY・HTTPS_PROXY・HTTP_PROXY で渡して、exe の通信をすべて手元で止める
  (zahyou-net の ureq は既定でこの環境変数を読む)。撮る前に同じ環境で `--update-check` を走らせ、本当に止まることを確かめる。
  止まらなければ、解析する撮影の gui.json に "api_key": "" を書いて撮る(欄は空と写る。警告を出す)。
  ネットが止まっているので「自動」はオフラインで解き、天体名は cache.json の座標を使う(ネットの無い場所と同じ)。
  127.0.0.1 の閉じたポートは、Windows では拒否されるまで約 2 秒かかる(疎通確認の宛先 2 つで約 4 秒、写真の「所要」が延びる)ので、
  すぐ失敗する 0.0.0.0 を使う(実測: 更新の確認のプロセス全体が 0.14 秒、127.0.0.1 なら 2.2 秒)。
- ［使い方］タブの撮影だけはネットを止めない(起動時の更新の確認 = GitHub の Releases の一覧を読むだけ。nova には触れない)。
  止めると「いまの版」の下に「更新を確認できませんでした (…)」と写るため。最新版なら何も写らない。
- 環境変数 ASTROMETRY_API_KEY は消してから起動する(欄の初めの値は既定のキーになる)。
- 星図データは既定の置き場所(C:\\AstrometryData)のものを使う。［準備］タブの「入っています」はその中身で変わる。
- 版名(題・［使い方］の「いまの版」)は exe の版のまま写る。配る版を建ててから撮ること。

切り抜きの枠は、窓の大きさ 1180×780(--shot の既定)の論理座標で書いてある。画面の拡大率(DPI)が違っても、
撮れた PNG の幅 ÷ 1180 を掛けて合わせる。画面の並びを変えたら、ここの枠も見直すこと(撮ったものを目で見る)。

必要なもの: Python 3.9 以上と Pillow(切り抜きと PNG の書き出し)。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

RS_DIR = Path(__file__).resolve().parents[1]
REPO_DIR = RS_DIR.parent
WIN_W, WIN_H = 1180, 780  # --shot の既定の窓の大きさ(論理座標)

TARGET = "UCAC4 660-021020"
TARGET_SEED = {"ra": 61.91199157844, "dec": 41.98626242835, "name": TARGET}
DEFAULT_IMAGE = Path(os.environ.get("USERPROFILE", str(Path.home()))) / "Downloads" / "Capture_00001 00_10_33.fits"

# 通信を手元で止めるプロキシ(0.0.0.0 へはつなげないので、外へ出る前にすぐ失敗する = os error 10049)
DEAD_PROXY = "http://0.0.0.0:9"
PROXY_VARS = ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY")

# 切り抜きの枠(論理座標 x0, y0, x1, y1)。None は窓全体。7.0.0-beta.1(左の列に送り棒と API キーの欄がある並び)で合わせた
NUMBERS = (371, 89, 1050, 178)  # 上の 3 つの数字と、その下の細い行(画像中心・画素スケール・視野・所要)
TARGET_BOX = (6, 91, 347, 320)  # 左の列の「目標」の枠
TOOLBAR = (376, 712, 742, 746)  # 図の下のボタン(全体・戻る・進む・移動・拡大・画像を保存)
FIGURE = (375, 214, 1173, 711)  # 図の欄(「撮影時の向き」「北が上」の図。マニュアルの図 1・2 にも使う)
APIKEY_BOX = (6, 380, 347, 742)  # --left-end で下まで送った左の列の「解析」の枠(方法・打ち切り・API キーの欄)
INDEX_BOX = (8, 262, 432, 722)  # ［準備］タブの「星図データ」の枠


def shots(image: str) -> list[dict]:
    """撮影の一覧。1 回の撮影から何枚か切り抜く。net が "block" なら通信を手元で止める("open" はネットのまま)"""
    run = ["--tab", "run", "--run", "--image", image]
    return [
        {"name": "analyze", "net": "block", "args": run + ["--target", TARGET, "--out-tab", "orig", "--light"],
         "crops": {"app-analyze": None, "app-numbers": NUMBERS, "app-target": TARGET_BOX, "app-toolbar": TOOLBAR,
                   "app-original": FIGURE}},
        {"name": "northup", "net": "block", "args": run + ["--target", TARGET, "--out-tab", "northup", "--light"],
         "crops": {"app-northup": FIGURE}},
        {"name": "dark", "net": "block", "args": run + ["--target", TARGET, "--out-tab", "orig", "--dark"],
         "crops": {"app-dark": None}},
        {"name": "blind", "net": "block", "args": run + ["--blind", "--out-tab", "orig", "--light"],
         "crops": {"app-blind": None, "app-blind-numbers": NUMBERS}},
        # 解析はしない(欄に既定のキーを伏せ字で入れたまま、左の列をいちばん下まで送って撮る)
        {"name": "apikey", "net": "block", "args": ["--tab", "run", "--image", image, "--left-end", "--light"],
         "crops": {"app-apikey": APIKEY_BOX}},
        {"name": "prepare", "net": "block", "args": ["--tab", "env", "--light"],
         "crops": {"app-prepare": None, "app-index": INDEX_BOX}},
        {"name": "help", "net": "open", "args": ["--tab", "help", "--light"],
         "crops": {"app-help": None}},
    ]


def default_exe() -> Path:
    target = os.environ.get("CARGO_TARGET_DIR")
    base = Path(target) if target else RS_DIR / "target"
    return base / "release" / "zahyou.exe"


def sha256(p: Path) -> str | None:
    if not p.exists():
        return None
    return hashlib.sha256(p.read_bytes()).hexdigest()


def user_files() -> dict[str, str | None]:
    """利用者の設定と記憶(撮影の前後で変わっていないことを確かめる)"""
    base = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "zahyou"
    return {n: sha256(base / n) for n in ("cache.json", "gui.json")}


def child_env(tmp: Path, block: bool) -> dict[str, str]:
    """一時フォルダーへ向けた環境。block なら通信を手元で止める"""
    env = dict(os.environ)
    env.pop("ASTROMETRY_API_KEY", None)  # 欄の初めの値を既定のキーにする(利用者のキーを写さない)
    for k in list(env):
        if k.upper() in PROXY_VARS + ("NO_PROXY",):
            del env[k]
    if block:
        for k in PROXY_VARS:
            env[k] = DEAD_PROXY
    env["ZAHYOU_CACHE"] = str(tmp / "cache.json")
    env["ZAHYOU_SETTINGS"] = str(tmp / "gui.json")
    env["LOCALAPPDATA"] = str(tmp / "AppData" / "Local")
    return env


def network_blocked(exe: Path) -> bool:
    """止めた環境で更新の確認を走らせ、外へつながらないことを確かめる"""
    tmp = Path(tempfile.mkdtemp(prefix="zahyou-shot-net-"))
    try:
        (tmp / "AppData" / "Local").mkdir(parents=True)
        out = tmp / "update-check.txt"
        subprocess.run([str(exe), "--update-check", "--out", str(out)], env=child_env(tmp, True), timeout=120)
        text = out.read_text(encoding="utf-8", errors="replace") if out.exists() else ""
        # 10049 = 使えない宛先(WSAEADDRNOTAVAIL)。0.0.0.0 のプロキシへつなごうとした = 環境変数が効いている。
        # ほかの失敗(名前を引けない等)はプロキシが効いた証拠にならないので、止まっていないとみなす
        return "state   : error" in text and "10049" in text
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def shoot(exe: Path, spec: dict, raw_dir: Path, gui: dict, block: bool) -> Path:
    """一時フォルダーの設定で exe に 1 枚撮らせる。撮れた PNG のパスを返す"""
    tmp = Path(tempfile.mkdtemp(prefix=f"zahyou-shot-{spec['name']}-"))
    try:
        (tmp / "AppData" / "Local").mkdir(parents=True)
        (tmp / "cache.json").write_text(
            json.dumps({"targets": {TARGET: TARGET_SEED}, "rigs": {}}, ensure_ascii=False, indent=1), encoding="utf-8")
        if gui:
            (tmp / "gui.json").write_text(json.dumps(gui, ensure_ascii=False), encoding="utf-8")
        out = raw_dir / f"{spec['name']}.png"
        out.unlink(missing_ok=True)
        r = subprocess.run([str(exe), "--shot", str(out), *spec["args"]], env=child_env(tmp, block), timeout=900)
        if r.returncode != 0 or not out.exists():
            raise SystemExit(f"撮れませんでした: {spec['name']}(終了コード {r.returncode})")
        return out
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def crop(raw: Path, box: tuple[int, int, int, int] | None, dest: Path) -> tuple[int, int]:
    with Image.open(raw) as im:
        im = im.convert("RGB")  # egui の写しは RGBA。透明は使っていないので落とす
        if box is not None:
            k = im.width / WIN_W
            if abs(im.height / WIN_H - k) > 0.01:
                raise SystemExit(f"{raw.name}: 窓の縦横比が {WIN_W}×{WIN_H} と違います({im.width}×{im.height})")
            im = im.crop(tuple(round(v * k) for v in box))
        im.save(dest, optimize=True)
        return im.size


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--exe", type=Path, default=default_exe())
    ap.add_argument("--image", type=Path, default=DEFAULT_IMAGE, help="解析させる画像(既定は基準の実写)")
    ap.add_argument("--out", type=Path, default=REPO_DIR / "docs" / "img")
    ap.add_argument("--raw", type=Path, help="切り抜く前の撮影をここに残す")
    ap.add_argument("--offline", action="store_true", help="方法を「オフラインのみ」にする")
    ap.add_argument("--only", action="append", default=[], help="この名前の写真だけ(app-help など。何度でも)")
    a = ap.parse_args()

    if not a.exe.exists():
        print(f"exe がありません: {a.exe}(cargo build --release -p zahyou)", file=sys.stderr)
        return 1
    if not a.image.exists():
        print(f"画像がありません: {a.image}", file=sys.stderr)
        return 1
    a.out.mkdir(parents=True, exist_ok=True)
    raw_dir = a.raw or Path(tempfile.mkdtemp(prefix="zahyou-shot-raw-"))
    raw_dir.mkdir(parents=True, exist_ok=True)

    plan = shots(str(a.image))
    if a.only:
        plan = [s for s in plan if any(n in s["crops"] for n in a.only)]
        if not plan:
            print(f"そういう名前の写真はありません: {a.only}", file=sys.stderr)
            return 1

    base_gui: dict = {"solve": "オフラインのみ"} if a.offline else {}
    run_gui = dict(base_gui)
    if any(s["net"] == "block" for s in plan):
        if network_blocked(a.exe):
            print("通信を手元で止めた環境で撮ります(同じ環境の更新の確認が外へ出ずに失敗することを確かめた)。")
        else:
            # 止まらなかった: 解析する撮影はキーを空にして、nova へ送らないようにする(欄は空と写る)
            run_gui["api_key"] = ""
            print("⚠ つながらないプロキシで通信を止められませんでした。解析する撮影は API キーの欄を空にして撮ります"
                  "(写真の欄は「空」と写ります)。", file=sys.stderr)

    before = user_files()
    try:
        for spec in plan:
            block = spec["net"] == "block"
            gui = run_gui if "--run" in spec["args"] else base_gui
            raw = shoot(a.exe, spec, raw_dir, gui, block)
            for name, box in spec["crops"].items():
                if a.only and name not in a.only:
                    continue
                w, h = crop(raw, box, a.out / f"{name}.png")
                print(f"  {name}.png  {w}×{h}  ({spec['name']}{'' if block else '・ネットあり'})")
    finally:
        if a.raw is None:
            shutil.rmtree(raw_dir, ignore_errors=True)
    after = user_files()
    if before != after:
        print(f"⚠ 利用者の %LOCALAPPDATA%\\zahyou が変わりました: {before} → {after}", file=sys.stderr)
        return 1
    print(f"利用者の cache.json・gui.json は変わっていません。書いた先: {a.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

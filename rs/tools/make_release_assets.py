# -*- coding: utf-8 -*-
"""Release(β版・正式版)に付ける資産を 1 つのフォルダーにそろえ、中身を検査して SHA256SUMS.txt を作る。

    python rs/tools/make_release_assets.py --exe <release ビルドの zahyou.exe> --out <リポジトリの外のフォルダー>

そろえるもの(v6 の Release に合わせる。名前は版から決まる):

    zahyou.exe                Rust 版の release ビルド(--exe。名前は必ず zahyou.exe = 更新の確認がこの名前で探す)
    zahyou_v7.ipynb           ノートブック版(リポジトリ直下の zahyou_v7.ipynb。名前は版から決める。β版なら zahyou_v7_beta.ipynb
                              = v6 と紛れないよう版の名前で)。セル A はエンジンを「同じフォルダーの zahyou_engine.py」で探すので、
                              ノートブックの名前を変えても動く
    zahyou_engine.py          ノートブック版の解析エンジン(リポジトリ直下のもの)
    zahyou-manual-v4.pdf      導入・運用マニュアル(リポジトリ直下の「…マニュアルv4.pdf」。v4 は dev/make_manual.py の VERSION)
    LICENSE.txt               LICENSE の写し
    THIRD-PARTY-NOTICES.md    rs/THIRD-PARTY-NOTICES.md(Rust 版の exe の同梱物。exe に埋め込んだものと同じ)
    SHA256SUMS.txt            上の全部の SHA256(`sha256sum -c SHA256SUMS.txt` で確かめられる形。改行は LF)

検査(1 つでも落ちたら、何も書かずに終了コード 1):

    version   exe の版情報(FileVersion・ProductVersion・数の版・プレリリースの印)と `--engine-check` の version= が、
              rs/Cargo.toml の [workspace.package] version と一致する
    paths     exe に、建てた人のホームフォルダーのパス(USERPROFILE。例 C:\\Users\\<名前>)が 1 か所も無い。
              区切り \\ と /・MSYS の /c/…・大文字小文字・UTF-8 と UTF-16 のどれでも数える。依存のパニック位置情報は、
              手順書 4 節の --remap-path-prefix を付けずに建てると、CARGO_HOME の下のパスのまま残る
    notices   exe に埋め込まれた同梱物の表示が rs/THIRD-PARTY-NOTICES.md と同じで、`gen_notices.py --check` で最新
              (cargo と rustc が要る。PATH に無ければ C:\\msys64\\ucrt64\\bin を足して探す)
    manual    dev/make_manual.py を一時フォルダーで組み直したものが、リポジトリの PDF とバイト単位で同じ
              (make_manual.py は作成日時などを固定して組むので、同じ中身なら同じバイトになる)
    notebook  dev/build_nb.py を一時フォルダーへ走らせた出力と比べて、直下の zahyou_engine.py が同じで、
              zahyou_v7.ipynb のセルの並びと中身が同じ(出力の例はリポジトリのものを残して配る)

`--skip <検査>` で検査を飛ばせる(試しのとき用。飛ばした検査は最後に「確かめていない」と出る)。
出力先は、空か無いフォルダーで、リポジトリの外に限る(直下に .gitignore が無いので、誤って commit しないように)。
文字のファイル(.ipynb・.py・.txt・.md)は改行を LF にそろえて書く(git の中と同じ形。作業ツリーが CRLF でも
SHA256 が変わらないように。v6 の Release の LICENSE.txt も LF)。exe と PDF はそのまま写す。
exe は `--engine-check` で 1 回起動する(画面は出さない。ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA は一時フォルダーへ向ける)。
build_nb.py は、どの出力先でも dev/_cell2_check.py を書く(.gitignore 済み)。

標準ライブラリだけで動く(マニュアルを組み直すには reportlab と fontTools、build_nb.py には何も要らない)。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

RS_DIR = Path(__file__).resolve().parents[1]
REPO_DIR = RS_DIR.parent
DEV_DIR = REPO_DIR / "dev"
REPO_NOTEBOOK = REPO_DIR / "zahyou_v7.ipynb"  # 配るノートブックの元(dev/build_nb.py の ROOT_NB)
MSYS_BIN = Path(r"C:\msys64\ucrt64\bin")

CHECKS = ("version", "paths", "notices", "manual", "notebook")
VS_FF_PRERELEASE = 0x2
VS_FIXEDFILEINFO_SIG = struct.pack("<I", 0xFEEF04BD)


# ------------------------------------------------------------------ 名前と版 ---

def cargo_version(cargo_toml: Path = RS_DIR / "Cargo.toml") -> str:
    """rs/Cargo.toml の [workspace.package] version"""
    with open(cargo_toml, "rb") as f:
        data = tomllib.load(f)
    return data["workspace"]["package"]["version"]


def split_version(version: str) -> tuple[tuple[int, int, int], str]:
    """'7.0.0-beta.1' → ((7, 0, 0), 'beta.1')。プレリリースが無ければ ''"""
    m = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?", version)
    if not m:
        raise ValueError(f"版の書き方が違います: {version!r}")
    return (int(m[1]), int(m[2]), int(m[3])), m[4] or ""


def notebook_asset_name(version: str) -> str:
    """配るノートブックの名前。7.0.0-beta.1 → zahyou_v7_beta.ipynb、7.0.0 → zahyou_v7.ipynb"""
    (major, _, _), pre = split_version(version)
    label = re.match(r"[A-Za-z]+", pre)
    return f"zahyou_v{major}_{label[0].lower()}.ipynb" if label else f"zahyou_v{major}.ipynb"


def manual_version(make_manual: Path = DEV_DIR / "make_manual.py") -> str:
    """dev/make_manual.py の VERSION(例 'v4')"""
    m = re.search(r'^VERSION = "(v\d+)"', make_manual.read_text(encoding="utf-8"), re.M)
    if not m:
        raise ValueError(f"{make_manual} に VERSION がありません")
    return m[1]


def manual_repo_path(version: str) -> Path:
    return REPO_DIR / f"天体画像解析プログラム「zahyou」導入・運用マニュアル{version}.pdf"


# ------------------------------------------------------------ exe の版情報 ---

def _align4(n: int) -> int:
    return (n + 3) & ~3


def pe_version_strings(data: bytes, keys=("FileVersion", "ProductVersion", "ProductName", "OriginalFilename")) -> dict[str, str]:
    """exe の版情報(StringFileInfo の String 構造)から文字列を読む。

    String 構造は wLength・wValueLength・wType(1 = 文字)・szKey(UTF-16、0 終わり)・4 バイト境界までの詰め物・Value。
    リソースは 4 バイト境界に置かれるので、ファイルの中の位置でそろえてよい。"""
    out: dict[str, str] = {}
    for key in keys:
        needle = key.encode("utf-16-le") + b"\x00\x00"
        start = 0
        while True:
            i = data.find(needle, start)
            if i < 0:
                break
            start = i + 2
            head = i - 6
            if head < 0 or head % 4:
                continue
            w_len, w_val, w_type = struct.unpack_from("<HHH", data, head)
            if w_type != 1 or not (0 < w_val < 256) or w_len < 6 + len(needle):
                continue
            v0 = _align4(i + len(needle))
            raw = data[v0:v0 + 2 * w_val]
            if len(raw) < 2 * w_val:
                continue
            out[key] = raw.decode("utf-16-le", errors="replace").rstrip("\x00")
            break
    return out


def pe_fixed_info(data: bytes) -> dict | None:
    """VS_FIXEDFILEINFO の数の版とフラグ(無ければ None)"""
    i = data.find(VS_FIXEDFILEINFO_SIG)
    if i < 0 or len(data) < i + 52:
        return None
    vals = struct.unpack_from("<13I", data, i)
    file_ms, file_ls, mask, flags = vals[2], vals[3], vals[6], vals[7]
    return {
        "file_version": (file_ms >> 16, file_ms & 0xFFFF, file_ls >> 16, file_ls & 0xFFFF),
        "prerelease": bool(flags & mask & VS_FF_PRERELEASE),
    }


def engine_check_version(exe: Path) -> str:
    """`zahyou.exe --engine-check` の version= の値(画面は出さない。利用者の設定には触れない)"""
    with tempfile.TemporaryDirectory(prefix="zahyou-assets-engine-") as t:
        tmp = Path(t)
        (tmp / "AppData" / "Local").mkdir(parents=True)
        env = dict(os.environ)
        env.update({"ZAHYOU_CACHE": str(tmp / "cache.json"), "ZAHYOU_SETTINGS": str(tmp / "gui.json"),
                    "LOCALAPPDATA": str(tmp / "AppData" / "Local")})
        out = tmp / "engine.txt"
        subprocess.run([str(exe), "--engine-check", "--out", str(out)], env=env, timeout=120,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        text = out.read_text(encoding="utf-8", errors="replace") if out.exists() else ""
    m = re.search(r"^version=(.+)$", text, re.M)
    return m[1].strip() if m else ""


def check_version(exe: Path, version: str) -> list[str]:
    """exe が version(Cargo の版)で建てたものか。食い違いの説明の一覧(空なら合格)"""
    problems = []
    data = exe.read_bytes()
    strings = pe_version_strings(data)
    if strings.get("FileVersion") != version:
        problems.append(f"exe の FileVersion が {strings.get('FileVersion')!r}(Cargo の版は {version!r})")
    if strings.get("ProductVersion") != f"v{version}":
        problems.append(f"exe の ProductVersion が {strings.get('ProductVersion')!r}(期待は 'v{version}')")
    if strings.get("OriginalFilename") not in (None, "zahyou.exe"):
        problems.append(f"exe の OriginalFilename が {strings.get('OriginalFilename')!r}")
    core, pre = split_version(version)
    fixed = pe_fixed_info(data)
    if fixed is None:
        problems.append("exe に数の版(VS_FIXEDFILEINFO)がありません")
    else:
        if fixed["file_version"] != (*core, 0):
            problems.append(f"exe の数の版が {fixed['file_version']}(期待は {(*core, 0)})")
        if fixed["prerelease"] != bool(pre):
            problems.append(f"exe のプレリリースの印が {fixed['prerelease']}(版 {version})")
    got = engine_check_version(exe)
    if got != f"v{version}":
        problems.append(f"--engine-check の version= が {got!r}(期待は 'v{version}')")
    return problems


# ------------------------------------------------------ 建てた人のホームのパス ---

NAME_CHARS = "0-9A-Za-z_.\\-"


def home_path_pattern(home: str, utf16: bool = False) -> re.Pattern[bytes]:
    """exe の中のホームフォルダーのパスを探す正規表現(大文字小文字は区別しない)。

    区切りは \\ と / のどちらでも、重なった \\\\(Debug の表示や JSON の書き出し)でもよい。ドライブは `C:` と
    MSYS の `/c` の両方。名前の後ろに文字が続くもの(home が C:\\Users\\taro のときの C:\\Users\\taro2)は別人なので数えない。"""
    codec = "utf-16-le" if utf16 else "utf-8"

    def lit(s: str) -> bytes:
        return re.escape(s.encode(codec))

    sep = b"(?:" + lit("\\") + b"|" + lit("/") + b"){1,2}"
    parts = [p for p in re.split(r"[\\/]+", home) if p]
    if not parts:
        raise ValueError(f"ホームフォルダーのパスが空です: {home!r}")
    drive = re.fullmatch(r"([A-Za-z]):", parts[0])
    if drive:
        head = b"(?:" + lit(parts[0]) + b"|" + lit("/" + drive[1]) + b")" + sep
        parts = parts[1:]
    else:
        head = sep
    body = sep.join(lit(p) for p in parts)
    tail = b"(?!" + b"[" + NAME_CHARS.encode() + b"]" + (b"\x00" if utf16 else b"") + b")"
    return re.compile(head + body + tail, re.I)


def count_home_paths(data: bytes, home: str) -> dict[str, int]:
    """exe の中のホームフォルダーのパスの数(UTF-8 と UTF-16 で別に数える)"""
    return {name: sum(1 for _ in home_path_pattern(home, utf16).finditer(data))
            for name, utf16 in (("UTF-8", False), ("UTF-16", True))}


def home_dirs() -> list[str]:
    """建てた人のホームフォルダー(この道具は exe を建てた PC で走らせる)。
    区切りと大文字小文字だけが違うもの(MSYS2 の Python の Path.home() は C:/Users/…)は 1 つにまとめる"""
    out: list[str] = []
    seen: set[str] = set()
    for c in (os.environ.get("USERPROFILE", ""), str(Path.home())):
        key = c.replace("\\", "/").rstrip("/").lower()
        if key and key not in seen:
            seen.add(key)
            out.append(c)
    return out


def check_paths(exe: Path, homes: list[str] | None = None) -> list[str]:
    """exe に建てた人のホームフォルダーのパスが入っていないか。見つかった説明の一覧(空なら合格)"""
    data = exe.read_bytes()
    problems = []
    for home in homes if homes is not None else home_dirs():
        n = count_home_paths(data, home)
        if sum(n.values()):
            problems.append(f"exe にホームフォルダーのパス {home} が {sum(n.values())} か所あります"
                            f"(UTF-8 {n['UTF-8']}・UTF-16 {n['UTF-16']}。依存のパニック位置情報なら、"
                            "手順書 4 節の --remap-path-prefix を付けずに建てた exe)")
    return problems


# ------------------------------------------------------------ 同梱物の表示 ---

def notices_embedded(exe_bytes: bytes, notices: bytes) -> bool:
    """exe に表示がそのまま埋め込まれているか(include_str!。作業ツリーの改行が CRLF でも LF でも)"""
    lf = notices.replace(b"\r\n", b"\n")
    return lf in exe_bytes or lf.replace(b"\n", b"\r\n") in exe_bytes


def tool_env() -> dict[str, str]:
    """cargo・rustc が PATH に無ければ MSYS2 UCRT64 を足した環境"""
    env = dict(os.environ)
    if not shutil.which("cargo") and (MSYS_BIN / "cargo.exe").exists():
        env["PATH"] = str(MSYS_BIN) + os.pathsep + env.get("PATH", "")
    return env


def check_notices(exe: Path) -> list[str]:
    problems = []
    notices = (RS_DIR / "THIRD-PARTY-NOTICES.md").read_bytes()
    if not notices_embedded(exe.read_bytes(), notices):
        problems.append("exe に埋め込まれた同梱物の表示が rs/THIRD-PARTY-NOTICES.md と違います(表示を作り直したあとで exe を建て直す)")
    r = subprocess.run([sys.executable, "-B", str(RS_DIR / "tools" / "gen_notices.py"), "--check"],
                       cwd=RS_DIR, env=tool_env(), capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        tail = (r.stdout + r.stderr).strip().splitlines()[-3:]
        problems.append("gen_notices.py --check が通りません: " + " / ".join(tail))
    return problems


# ---------------------------------------------------------------- マニュアル ---

def check_manual(version: str, work: Path) -> list[str]:
    """make_manual.py を組み直して、リポジトリの PDF と同じバイトか"""
    repo_pdf = manual_repo_path(version)
    if not repo_pdf.exists():
        return [f"{repo_pdf.name} がありません(python dev/make_manual.py で組む)"]
    fresh = work / "manual.pdf"
    env = dict(os.environ, PYTHONIOENCODING="utf-8")
    r = subprocess.run([sys.executable, "-B", str(DEV_DIR / "make_manual.py"), str(fresh)], cwd=DEV_DIR, env=env,
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0 or not fresh.exists():
        return ["make_manual.py で組めません: " + " / ".join((r.stdout + r.stderr).strip().splitlines()[-3:])]
    if fresh.read_bytes() != repo_pdf.read_bytes():
        return [f"{repo_pdf.name} が今の make_manual.py で組んだものと違います(python dev/make_manual.py で組み直す)"]
    return []


# ---------------------------------------------------------------- ノートブック ---

def _norm(text: str) -> str:
    return text.replace("\r\n", "\n")


def notebook_cells(path: Path) -> list[tuple[str | None, str, str]]:
    """(id, 種類, 中身) の並び。中身の改行は LF にそろえる"""
    nb = json.loads(path.read_bytes().decode("utf-8"))
    return [(c.get("id"), c.get("cell_type", ""), _norm("".join(c.get("source", [])))) for c in nb["cells"]]


def compare_notebooks(built: Path, repo: Path) -> list[str]:
    a, b = notebook_cells(built), notebook_cells(repo)
    if [c[0] for c in a] != [c[0] for c in b]:
        return [f"{repo.name} のセルの並び(id)が build_nb.py の出力と違います"]
    diff = [ca[0] for ca, cb in zip(a, b) if ca != cb]
    return [f"{repo.name} のセルの中身が build_nb.py の出力と違います: {', '.join(map(str, diff))}"] if diff else []


def compare_engines(built: Path, repo: Path) -> list[str]:
    if _norm(built.read_text(encoding="utf-8")) != _norm(repo.read_text(encoding="utf-8")):
        return [f"{repo.name} が build_nb.py の出力と違います(python dev/build_nb.py で作り直す)"]
    return []


def notebook_version(engine_text: str) -> str | None:
    """ノートブック版のログの見出しの版(dev/nb_main.py の NB_VERSION)。見つからなければ None"""
    m = re.search(r'^NB_VERSION = "([^"]*)"$', engine_text, re.M)
    return m.group(1) if m else None


def check_notebook_version(engine: Path, version: str) -> list[str]:
    got = notebook_version(_norm(engine.read_text(encoding="utf-8")))
    if got != f"v{version}":
        return [f"{engine.name} の NB_VERSION が {got!r}(期待は 'v{version}'。dev/nb_main.py を直して python dev/build_nb.py)"]
    return []


def check_notebook(work: Path, version: str) -> list[str]:
    out_nb = work / "zahyou.ipynb"
    env = dict(os.environ, PYTHONIOENCODING="utf-8")
    r = subprocess.run([sys.executable, "-B", str(DEV_DIR / "build_nb.py"), str(out_nb)], cwd=DEV_DIR, env=env,
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0 or not out_nb.exists():
        return ["build_nb.py が動きません: " + " / ".join((r.stdout + r.stderr).strip().splitlines()[-3:])]
    return (compare_engines(work / "zahyou_engine.py", REPO_DIR / "zahyou_engine.py")
            + compare_notebooks(out_nb, REPO_NOTEBOOK)
            + check_notebook_version(REPO_DIR / "zahyou_engine.py", version))


# ---------------------------------------------------------------- そろえる ---

TEXT_SUFFIXES = (".ipynb", ".py", ".txt", ".md")


@dataclass
class Plan:
    """配る資産(配る名前 → 元のファイル)"""
    files: dict[str, Path] = field(default_factory=dict)


def is_text_asset(name: str) -> bool:
    """改行を LF にそろえて書く資産か"""
    return name.lower().endswith(TEXT_SUFFIXES)


def plan_assets(exe: Path, version: str, manual_ver: str) -> Plan:
    p = Plan()
    p.files["zahyou.exe"] = exe
    p.files[notebook_asset_name(version)] = REPO_NOTEBOOK
    p.files["zahyou_engine.py"] = REPO_DIR / "zahyou_engine.py"
    p.files[f"zahyou-manual-{manual_ver}.pdf"] = manual_repo_path(manual_ver)
    p.files["LICENSE.txt"] = REPO_DIR / "LICENSE"
    p.files["THIRD-PARTY-NOTICES.md"] = RS_DIR / "THIRD-PARTY-NOTICES.md"
    return p


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256sums_text(out: Path, names: list[str]) -> str:
    """`sha256sum -c` で確かめられる形(「ハッシュ␣␣名前」、LF)"""
    return "".join(f"{sha256_file(out / n)}  {n}\n" for n in names)


def verify_sha256sums(out: Path) -> list[str]:
    problems = []
    for line in (out / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        digest, name = line.split("  ", 1)
        if not (out / name).exists():
            problems.append(f"{name} がありません")
        elif sha256_file(out / name) != digest:
            problems.append(f"{name} の SHA256 が SHA256SUMS.txt と違います")
    return problems


def check_out_dir(out: Path, allow_in_repo: bool = False) -> str | None:
    """出力先に書いてよいか。だめなら理由"""
    out = out.resolve()
    if out.exists() and (not out.is_dir() or any(out.iterdir())):
        return f"{out} は空のフォルダーではありません(資産を混ぜないため、空か無いフォルダーにする)"
    if not allow_in_repo and (out == REPO_DIR or REPO_DIR in out.parents):
        return f"{out} はリポジトリの中です(誤って commit しないよう、リポジトリの外にする)"
    return None


def assemble(plan: Plan, out: Path) -> list[str]:
    """資産を写して SHA256SUMS.txt を書く。書いた名前の並び(SHA256SUMS.txt を除く)"""
    missing = [str(src) for src in plan.files.values() if not src.exists()]
    if missing:
        raise FileNotFoundError("元のファイルがありません: " + ", ".join(missing))
    out.mkdir(parents=True, exist_ok=True)
    names = list(plan.files)
    for name, src in plan.files.items():
        if is_text_asset(name):
            (out / name).write_bytes(src.read_bytes().replace(b"\r\n", b"\n"))
        else:
            shutil.copyfile(src, out / name)
    (out / "SHA256SUMS.txt").write_bytes(sha256sums_text(out, names).encode("ascii"))
    return names


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--exe", type=Path, required=True, help="release ビルドの zahyou.exe")
    ap.add_argument("--out", type=Path, required=True, help="資産を置くフォルダー(空か無い、リポジトリの外)")
    ap.add_argument("--skip", action="append", default=[], choices=CHECKS, help="飛ばす検査(試しのとき用。何度でも)")
    ap.add_argument("--allow-in-repo", action="store_true", help=argparse.SUPPRESS)  # 試験用
    a = ap.parse_args(argv)

    if not a.exe.is_file():
        print(f"exe がありません: {a.exe}", file=sys.stderr)
        return 1
    why = check_out_dir(a.out, a.allow_in_repo)
    if why:
        print(why, file=sys.stderr)
        return 1

    version = cargo_version()
    manual_ver = manual_version()
    print(f"版 {version}(rs/Cargo.toml)/ マニュアル {manual_ver} / exe {a.exe}")
    results: dict[str, list[str] | None] = {}
    with tempfile.TemporaryDirectory(prefix="zahyou-assets-") as t:
        work = Path(t)
        steps = {
            "version": lambda: check_version(a.exe, version),
            "paths": lambda: check_paths(a.exe),
            "notices": lambda: check_notices(a.exe),
            "manual": lambda: check_manual(manual_ver, work / "manual"),
            "notebook": lambda: check_notebook(work / "notebook", version),
        }
        for name, run in steps.items():
            if name in a.skip:
                results[name] = None
                print(f"  SKIP {name}")
                continue
            (work / name).mkdir(exist_ok=True)
            results[name] = run()
            print(f"  {'OK  ' if not results[name] else 'NG  '} {name}")
            for p in results[name]:
                print(f"         {p}")
    failed = [n for n, r in results.items() if r]
    if failed:
        print(f"検査に落ちたので、資産は書いていません: {', '.join(failed)}", file=sys.stderr)
        return 1

    names = assemble(plan_assets(a.exe, version, manual_ver), a.out)
    problems = verify_sha256sums(a.out)
    if problems:
        print("SHA256SUMS.txt を読み直すと合いません: " + " / ".join(problems), file=sys.stderr)
        return 1
    print(f"\n{a.out}")
    for n in names + ["SHA256SUMS.txt"]:
        print(f"  {n:28s} {(a.out / n).stat().st_size:>12,d} バイト")
    print("\nSHA256SUMS.txt:")
    print((a.out / "SHA256SUMS.txt").read_text(encoding="ascii"), end="")
    skipped = [n for n, r in results.items() if r is None]
    if skipped:
        print(f"\n⚠ 確かめていない検査: {', '.join(skipped)}(Release に出す前に、飛ばさずにもう一度走らせる)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

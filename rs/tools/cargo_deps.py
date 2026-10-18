# -*- coding: utf-8 -*-
"""gen_notices.py と check_licenses.py が共通で使う部品。

- cargo metadata を読む(ターゲット x86_64-pc-windows-gnu に絞る)
- exe に入る依存を選ぶ(ワークスペースのメンバーから「通常の依存」の辺だけでたどれるもの)。
  ビルドだけ(build-dependencies の先)・試験だけ(dev-dependencies の先)は除く
- そのうち、コンパイルのときだけ動くもの(proc-macro と、proc-macro からしかたどれないもの)を見分ける
- SPDX のライセンス式を読む(AND・OR・WITH・括弧。古い書き方の「/」は OR とみなす)
- exe に埋め込まれる書体と、リンクのときに結合される実行時ライブラリの「ライセンス式」(cargo の外のもの)

標準ライブラリだけで動く(pip で何も入れない)。
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

TARGET = "x86_64-pc-windows-gnu"
TOOLS_DIR = Path(__file__).resolve().parent
RS_DIR = TOOLS_DIR.parent
REPO_DIR = RS_DIR.parent

# C やアセンブリをビルドする(cc などをビルド依存に持つ)と判定する目印
NATIVE_BUILD_DEPS = ("cc", "cmake", "bindgen", "pkg-config", "vcpkg", "nasm-rs", "autotools")


@dataclass
class Package:
    """exe に入る依存 1 つ分(cargo metadata の packages の要るところだけ)"""

    id: str
    name: str
    version: str
    license: str | None
    license_file: str | None
    manifest_dir: Path
    authors: list[str]
    repository: str | None
    proc_macro: bool
    links: str | None
    native_build_deps: list[str]
    compile_time_only: bool = False


@dataclass
class Deps:
    """依存の分類の結果"""

    runtime: list[Package]  # exe に入るもの(名前・版の順)
    build_only: int  # ビルドのときだけのものの数
    dev_only: int  # 試験のときだけのものの数
    workspace: list[str] = field(default_factory=list)  # ワークスペースのメンバーの名前


def load_metadata(path: str | None = None, cargo: str = "cargo") -> dict:
    """cargo metadata を読む。path が無ければ cargo を呼ぶ(--locked、ターゲットを絞る)。"""
    if path:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    cmd = [
        cargo, "metadata", "--format-version", "1", "--locked",
        "--filter-platform", TARGET,
        "--manifest-path", str(RS_DIR / "Cargo.toml"),
    ]
    try:
        out = subprocess.run(cmd, check=True, capture_output=True, **_no_window())
    except FileNotFoundError:
        raise SystemExit("!! cargo が見つかりません。Git Bash なら export PATH=/c/msys64/ucrt64/bin:$PATH")
    except subprocess.CalledProcessError as e:
        msg = e.stderr.decode("utf-8", "replace")
        raise SystemExit("!! cargo metadata が失敗しました:\n" + msg)
    return json.loads(out.stdout.decode("utf-8"))


def _no_window() -> dict:
    """Windows で子プロセスの窓(conhost)を出さない"""
    if os.name == "nt":
        return {"creationflags": 0x08000000}  # CREATE_NO_WINDOW
    return {}


def classify(meta: dict) -> Deps:
    """exe に入る依存・ビルドだけ・試験だけに分ける。"""
    pkgs = {p["id"]: p for p in meta["packages"]}
    ws = set(meta["workspace_members"])
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}

    def edges(nid: str, kinds: set) -> list[str]:
        out = []
        for d in nodes[nid]["deps"]:
            ks = {k.get("kind") for k in d.get("dep_kinds", [])}
            if ks & kinds:
                out.append(d["pkg"])
        return out

    def is_proc_macro(pid: str) -> bool:
        return any("proc-macro" in t["kind"] for t in pkgs[pid]["targets"])

    def reach(starts, first_kinds, later_kinds, stop_at_proc_macro=False) -> set:
        seen: set = set()
        stack = [t for s in starts for t in edges(s, first_kinds)]
        while stack:
            x = stack.pop()
            if x in seen or x in ws:
                continue
            if stop_at_proc_macro and is_proc_macro(x):
                continue
            seen.add(x)
            stack.extend(edges(x, later_kinds))
        return seen

    # exe に入るもの: 通常の依存(kind = null)の辺だけ
    runtime = reach(ws, {None}, {None})
    # そのうち実際にリンクされるもの: proc-macro の先へは進まない
    linked = reach(ws, {None}, {None}, stop_at_proc_macro=True)
    # ビルドスクリプト用: どこかで build の辺を通るもの(その先は通常 + build)
    build: set = set()
    for s in ws | runtime:
        build |= reach([s], {"build"}, {None, "build"})
    build -= runtime
    dev = reach(ws, {"dev"}, {None, "build"}) - runtime - build

    out = []
    for pid in runtime:
        p = pkgs[pid]
        native = [d["name"] for d in p["dependencies"]
                  if d.get("kind") == "build" and d["name"] in NATIVE_BUILD_DEPS]
        out.append(Package(
            id=pid,
            name=p["name"],
            version=p["version"],
            license=p.get("license"),
            license_file=p.get("license_file"),
            manifest_dir=Path(p["manifest_path"]).parent,
            authors=list(p.get("authors") or []),
            repository=p.get("repository"),
            proc_macro=is_proc_macro(pid),
            links=p.get("links"),
            native_build_deps=sorted(set(native)),
            compile_time_only=pid not in linked,
        ))
    out.sort(key=lambda p: (p.name, _version_key(p.version)))
    names = sorted(pkgs[i]["name"] for i in ws)
    return Deps(runtime=out, build_only=len(build), dev_only=len(dev), workspace=names)


def _version_key(v: str):
    parts = re.split(r"[.+-]", v)
    return [(0, int(x)) if x.isdigit() else (1, x) for x in parts]


# ---------------------------------------------------------------------------
# SPDX のライセンス式

class SpdxError(ValueError):
    pass


def spdx_parse(expr: str):
    """SPDX の式を木にする。

    戻り値: ("lic", id, exception or None) / ("and", [子...]) / ("or", [子...])
    古い書き方の「MIT/Apache-2.0」は OR とみなす。読めない式は SpdxError。
    """
    if expr is None or not expr.strip():
        raise SpdxError("ライセンス式が空です")
    s = expr.replace("/", " OR ")
    toks = re.findall(r"\(|\)|[A-Za-z0-9.+\-]+|\S", s)
    pos = 0

    def peek():
        return toks[pos] if pos < len(toks) else None

    def take():
        nonlocal pos
        t = peek()
        pos += 1
        return t

    def p_or():
        items = [p_and()]
        while peek() is not None and peek().upper() == "OR":
            take()
            items.append(p_and())
        return items[0] if len(items) == 1 else ("or", items)

    def p_and():
        items = [p_atom()]
        while peek() is not None and peek().upper() == "AND":
            take()
            items.append(p_atom())
        return items[0] if len(items) == 1 else ("and", items)

    def p_atom():
        t = take()
        if t is None:
            raise SpdxError(f"式が途中で終わっています: {expr!r}")
        if t == "(":
            node = p_or()
            if take() != ")":
                raise SpdxError(f"括弧が閉じていません: {expr!r}")
            return node
        if t in (")",) or t.upper() in ("AND", "OR", "WITH") or not re.fullmatch(r"[A-Za-z0-9.+\-]+", t):
            raise SpdxError(f"ライセンス名のところに {t!r} があります: {expr!r}")
        exc = None
        if peek() is not None and peek().upper() == "WITH":
            take()
            exc = take()
            if exc is None or not re.fullmatch(r"[A-Za-z0-9.+\-]+", exc):
                raise SpdxError(f"WITH の後に例外の名前がありません: {expr!r}")
        return ("lic", t, exc)

    node = p_or()
    if peek() is not None:
        raise SpdxError(f"余分な語 {peek()!r} があります: {expr!r}")
    return node


def spdx_ids(node) -> list[tuple[str, str | None]]:
    """式に出てくる (ライセンス, 例外) を出てきた順に(重複なし)"""
    out: list = []

    def walk(n):
        if n[0] == "lic":
            if (n[1], n[2]) not in out:
                out.append((n[1], n[2]))
        else:
            for c in n[1]:
                walk(c)

    walk(node)
    return out


def spdx_satisfied(node, ok) -> bool:
    """ok(id, exc) が真になる選び方があるか(OR はどれか 1 つ、AND は全部)"""
    if node[0] == "lic":
        return ok(node[1], node[2])
    if node[0] == "or":
        return any(spdx_satisfied(c, ok) for c in node[1])
    return all(spdx_satisfied(c, ok) for c in node[1])


# ---------------------------------------------------------------------------
# cargo の外にあって exe に入るもの(書体と、リンクで結合される実行時ライブラリ)

# egui の既定の書体(epaint_default_fonts が include_bytes! で exe に埋め込む)。
# (書体のファイル, 一緒に配られているライセンスのファイル, ライセンス式)
EMBEDDED_FONTS = [
    ("Hack-Regular.ttf", "Hack-Regular.txt", "MIT AND Bitstream-Vera"),
    ("Ubuntu-Light.ttf", "UFL.txt", "Ubuntu-font-1.0"),
    ("NotoEmoji-Regular.ttf", "OFL.txt", "OFL-1.1"),
    ("emoji-icon-font.ttf", "emoji-icon-font-mit-license.txt", "MIT"),
]
FONT_CRATE = "epaint_default_fonts"

# x86_64-pc-windows-gnu のリンクで結合される実行時ライブラリ(rustc --print link-args で確かめたもの:
# -lgcc -lgcc_eh -l:libpthread.a -lmingw32 -lmingwex と crt2.o、標準ライブラリの rlib)。
# (名前, ライセンス式, 表示の元になるファイル(sysroot からの相対))
TOOLCHAIN_PARTS = [
    ("Rust の標準ライブラリ(std・core・alloc とその依存)",
     "(MIT OR Apache-2.0) AND Unicode-3.0 AND BSD-2-Clause",
     "share/doc/rustc/COPYRIGHT-library.html"),
    ("compiler-rt(標準ライブラリの compiler_builtins に入っている C)",
     "Apache-2.0 WITH LLVM-exception",
     "share/doc/rustc/licenses/LLVM-exception.txt"),
    ("MinGW-w64 の実行時ライブラリ(crt2.o・libmingw32.a・libmingwex.a)",
     "ZPL-2.1 AND ISC AND BSD-2-Clause AND LicenseRef-Lucent-gdtoa AND SunPro AND MIT",
     "share/licenses/crt/COPYING.MinGW-w64-runtime.txt"),
    ("winpthreads(libpthread.a)",
     "MIT AND BSD-3-Clause",
     "share/licenses/winpthreads/COPYING"),
    ("libgcc.a・libgcc_eh.a(GCC の実行時ライブラリ。例外の巻き戻しなど)",
     "GPL-3.0-or-later WITH GCC-exception-3.1",
     "share/licenses/gcc-libs/COPYING.RUNTIME"),
]


def extra_components() -> list[tuple[str, str, str]]:
    """チェックの対象にする cargo の外のもの: (名前, ライセンス式, 種類 "font" / "toolchain")"""
    out = [(f"{FONT_CRATE} の書体 {ttf}", lic, "font") for ttf, _txt, lic in EMBEDDED_FONTS]
    out += [(name, lic, "toolchain") for name, lic, _src in TOOLCHAIN_PARTS]
    return out


def stdout_utf8() -> None:
    """コンソールが cp932 でも日本語を出せるようにする"""
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass

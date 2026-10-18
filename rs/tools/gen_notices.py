# -*- coding: utf-8 -*-
"""rs/THIRD-PARTY-NOTICES.md(Rust 版 zahyou.exe に添える同梱物の表示)を作る。

手で並べると必ずずれるので、cargo metadata と、各クレートの配布物
(%USERPROFILE%\\.cargo\\registry\\src の下)にあるライセンスファイルから機械的に作る。

    python rs/tools/gen_notices.py            # 書き出す
    python rs/tools/gen_notices.py --check    # 中身が古くなっていないか見るだけ(古ければ終了コード 1)

入れるもの:
- zahyou 自身の LICENSE と、MIT が及ぶ範囲
- 解き方の出典(astrometry.net の論文)と、星図データ(Tycho-2・Gaia DR2)の条件と定型の謝辞
- exe に入る依存(ターゲット x86_64-pc-windows-gnu。ビルドだけ・試験だけのものは除く)の
  クレート名・版・ライセンス式・著作権表示・ライセンス本文(同じ本文は 1 つにまとめる)。NOTICE は必ず入れる
- egui の既定の書体(exe に埋め込まれる)と、そのライセンス(OFL-1.1・Ubuntu Font Licence 1.0 ほか)
- C とアセンブリをビルドするクレート(ring)の説明
- リンクのときに結合される実行時ライブラリ(Rust の標準ライブラリ・MinGW-w64・winpthreads・libgcc)

標準ライブラリだけで動く。cargo と rustc は PATH にあること(Git Bash なら
export PATH=/c/msys64/ucrt64/bin:$PATH)。
"""
from __future__ import annotations

import argparse
import difflib
import html
import re
import struct
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

sys.dont_write_bytecode = True
import cargo_deps as cd  # noqa: E402

OUT = cd.RS_DIR / "THIRD-PARTY-NOTICES.md"

# ライセンスファイルとみなす名前(大文字小文字は問わない)
LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense)", re.I)
# 下へ探しに行かないフォルダー(試験・例・文書。exe に入らない)
SKIP_DIRS = {"tests", "test", "testdata", "test-data", "examples", "example", "benches", "bench",
             "fuzz", ".github", "target", "docs", "doc"}
# 名前の決まりに合わないが、ライセンスの本文であるもの(クレート名 → 相対パス)
EXTRA_FILES = {
    cd.FONT_CRATE: ["fonts/" + txt for _ttf, txt, _lic in cd.EMBEDDED_FONTS],
}
# 直下にないファイルのうち、クレートのライセンス式のどの名前の本文に当たるか(当たらないものは「追加の表示」)。
# epaint_default_fonts の fonts/ の MIT は書体のもので、コード部分の MIT OR Apache-2.0 の本文ではない
COVERS = {
    (cd.FONT_CRATE, "fonts/OFL.txt"): {"OFL-1.1"},
    (cd.FONT_CRATE, "fonts/UFL.txt"): {"Ubuntu-font-1.0"},
}
MAX_FILE = 512 * 1024

# 本文の種類の見分け(上から順に調べ、当てはまるものを全部つける)
KIND_SIGNS = [
    ("Apache-2.0", [r"Apache License", r"Version 2\.0, January 2004", r"TERMS AND CONDITIONS FOR USE"]),
    ("MIT", [r"Permission is hereby granted, free of charge, to any person obtaining",
             r"THE SOFTWARE IS PROVIDED"]),
    ("BSD-3-Clause", [r"Redistribution and use in source and binary forms",
                      r"(Neither the name|names of (its|the) contributors may)"]),
    ("BSD-2-Clause", [r"Redistribution and use in source and binary forms",
                      r"Redistributions in binary form must reproduce"]),
    ("ISC", [r"Permission to use, copy, modify, and(/or)? distribute this software for any purpose with or without fee",
             r"above copyright notice and this permission notice appear in all copies"]),
    ("0BSD", [r"Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted\. THE SOFTWARE"]),
    ("Zlib", [r"provided 'as-is'", r"must not be misrepresented"]),
    ("Unicode-3.0", [r"UNICODE LICENSE V3|Unicode License v3"]),
    ("BSL-1.0", [r"Boost Software License - Version 1\.0"]),
    ("Unlicense", [r"This is free and unencumbered software released into the public domain"]),
    ("CDLA-Permissive-2.0", [r"Community Data License Agreement"]),
    ("OFL-1.1", [r"SIL OPEN FONT LICENSE Version 1\.1"]),
    ("Ubuntu-font-1.0", [r"UBUNTU FONT LICEN[CS]E Version 1\.0"]),
    ("Bitstream-Vera", [r"BITSTREAM VERA LICENSE"]),
    ("ZPL-2.1", [r"Zope Public License"]),
]
KIND_ORDER = [k for k, _ in KIND_SIGNS]

# 本文を配布物に持たないクレートのためのライセンス(ほかのクレートにも本文が無いもの)
NO_TEXT_NOTE = {
    "CC0-1.0": "CC0 1.0 Universal(パブリック ドメインへの献呈。表示の義務はありません。"
               "本文: <https://creativecommons.org/publicdomain/zero/1.0/legalcode>)",
}

# 著作権の行とみなさない、"Copyright" の直後の語
COPY_STOP = {"notice", "notices", "holder", "holders", "holder(s)", "owner", "owners", "license", "licence",
             "licenses", "licences", "statement", "and", "law", "laws", "protection", "claim", "or", "of",
             "is", "in", "on", "to", "for", "interest", "infringement", "act", "the", "this", "rights"}
PLACEHOLDER = re.compile(
    r"\[yyyy\]|\{yyyy\}|<year>|\[year\]|\{year\}|<dates?>|<copyright holders?>|\[name of copyright owner\]"
    r"|\{name of copyright owner\}|<owner>|<name of author>|<author>|\bYEAR\b|\[fullname\]|<name>", re.I)
TITLE_LINE = re.compile(r"^(the\s+)?mit\s+licen[cs]e(\s*\(mit\))?\s*:?$|^licen[cs]e\s*:?$|^ISC License(\s*\(ISC\))?$"
                        r"|^zlib License$|^BSD \d-Clause License$", re.I)


# ---------------------------------------------------------------------------
# ライセンスファイルを読む

def read_text(p: Path) -> str | None:
    """テキストとして読む(BOM・改行の違い・行末の空白をならす)。読めないものは None。"""
    try:
        b = p.read_bytes()
    except OSError:
        return None
    if len(b) > MAX_FILE or b"\x00" in b[:4096]:
        return None
    t = b.decode("utf-8", errors="replace").lstrip("\ufeff")
    t = t.replace("\r\n", "\n").replace("\r", "\n").replace("\t", "    ")
    lines = [ln.rstrip() for ln in t.split("\n")]
    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    return "\n".join(lines)


def license_paths(pkg: cd.Package) -> list[tuple[str, Path]]:
    """クレートのライセンスファイル(相対パス, 実パス)。直下が先、その下は名前順。"""
    root = pkg.manifest_dir
    found: dict[str, Path] = {}
    if not root.is_dir():
        return []
    for p in sorted(root.iterdir()):
        if p.is_file() and LICENSE_NAME.match(p.name):
            found[p.name] = p
    # その下(ring の third_party/fiat/LICENSE、tracing-core の src/spin/LICENSE など、exe に入るコードの表示)
    for sub in sorted(root.iterdir()):
        if not sub.is_dir() or sub.name.lower() in SKIP_DIRS or sub.name.startswith("."):
            continue
        for p in sorted(sub.rglob("*")):
            rel_parts = p.relative_to(root).parts
            if any(x.lower() in SKIP_DIRS or x.startswith(".") for x in rel_parts[:-1]):
                continue
            if p.is_file() and LICENSE_NAME.match(p.name):
                found["/".join(rel_parts)] = p
    if pkg.license_file:
        lf = (root / pkg.license_file).resolve()
        if lf.is_file():
            try:
                rel = "/".join(lf.relative_to(root.resolve()).parts)
            except ValueError:
                rel = lf.name
            found.setdefault(rel, lf)
    for rel in EXTRA_FILES.get(pkg.name, []):
        p = root / rel
        if p.is_file():
            found.setdefault(rel, p)
    return sorted(found.items(), key=lambda kv: ("/" in kv[0], kv[0].lower()))


def detect_kinds(text: str) -> list[str]:
    """本文の種類(SPDX の名前)。改行や空白の違いは無視して、目印の文が全部あるかで決める。"""
    # 行頭の注釈の記号(// # * ;)は外して比べる(untrusted の LICENSE.txt は // で始まる)
    nt = _norm("\n".join(re.sub(r"^\s*(//+|#+|\*+|;+)\s?", "", ln) for ln in text.split("\n")))
    kinds = []
    for kind, signs in KIND_SIGNS:
        if all(re.search(s, nt, re.I) for s in signs):
            kinds.append(kind)
    # BSD-3 は BSD-2 の条件も満たすので、両方当たったら 3 だけにする。0BSD と ISC も同じ
    if "BSD-3-Clause" in kinds and "BSD-2-Clause" in kinds:
        kinds.remove("BSD-2-Clause")
    if "ISC" in kinds and "0BSD" in kinds:
        kinds.remove("0BSD")
    return kinds


def _copyright_match(line: str):
    """著作権の行らしければ (行の本体, 置き換え用の欄か) を返す。違えば None。"""
    s = line.strip()
    s = re.sub(r"^(?:[*#;!|>-]+|//+|/\*+)\s*", "", s).strip()
    m = re.match(r"(copyright|\(c\)|©)", s, re.I)
    if not m:
        return None
    rest = s[m.end():]
    if m.group(1).lower() == "copyright":
        if s[0] == "c" and not re.search(r"\d", s):  # 小文字で始まる文中の「copyright ...」
            return None
        if rest and not rest[0].isspace() and rest[0] not in "©(:":
            return None  # "Copyrights" や "COPYRIGHT," など
        r2 = re.sub(r"^\s*(\(c\)|©)", "", rest, flags=re.I).strip().lstrip(":").strip()
        if not r2:
            return None
        first = re.split(r"[\s,.;]+", r2)[0].lower()
        if first in COPY_STOP:
            return None
    elif m.group(1).lower() == "(c)":
        if not re.match(r"\s*\d", rest):  # 「(c) You must retain ...」のような箇条書きの記号
            return None
    elif len(rest.strip()) < 3:
        return None
    return s, bool(PLACEHOLDER.search(s))


def copyright_lines(text: str) -> list[str]:
    """本物の著作権の行(置き換え用の欄 [yyyy] などは除く)。重複は除き、出てきた順。"""
    out: list[str] = []
    for ln in text.split("\n"):
        m = _copyright_match(ln)
        if m and not m[1] and m[0] not in out:
            out.append(m[0])
    return out


def _norm(s: str) -> str:
    """比べるための正規化(空白・引用符・ダッシュの違いを無視)"""
    s = s.replace("\u201c", '"').replace("\u201d", '"').replace("\u2018", "'").replace("\u2019", "'")
    s = s.replace("\u2013", "-").replace("\u2014", "-").replace("``", '"').replace("''", '"')
    return re.sub(r"\s+", " ", s).strip()


def _strip_copyrights(lines: list[str]) -> list[str]:
    out = [ln for ln in lines if not _copyright_match(ln)]
    # 先頭の「MIT License」などの題は本文から外す(本文の比較で揃えるため)
    while out and (not out[0].strip() or TITLE_LINE.match(out[0].strip())):
        out.pop(0)
    # 連続する空行は 1 つに
    res: list[str] = []
    for ln in out:
        if not ln.strip() and res and not res[-1].strip():
            continue
        res.append(ln)
    while res and not res[-1].strip():
        res.pop()
    return res


def _tkey(lf: "LicFile") -> str:
    """本文をまとめるときの鍵。Apache-2.0 は見出しの URL の書き方(http / https / LICENSE-2.0 の有無)の違いを無視する"""
    k = _norm(lf.tbody or "")
    if lf.kinds == ["Apache-2.0"]:
        k = re.sub(r"https?://www\.apache\.org/licenses/(LICENSE-2\.0)?", "http://www.apache.org/licenses/", k)
    return k


def _dedent(lines: list[str]) -> list[str]:
    ind = min((len(ln) - len(ln.lstrip(" ")) for ln in lines if ln.strip()), default=0)
    return [ln[ind:] if ln.strip() else "" for ln in lines]


def template_body(text: str, kinds: list[str]) -> str | None:
    """著作権の行を除いた本文(同じライセンスのファイルどうしで揃うもの)。揃えられない形なら None。

    Apache-2.0 は「END OF TERMS AND CONDITIONS」までを本文とし、その後ろの付録(使い方の例)は、
    標準の付録そのもの(著作権の行を除く)であるときだけ落とす。前後にほかの文があれば None(原文のまま載せる)。
    """
    lines = text.split("\n")
    if kinds == ["Apache-2.0"]:
        hi = next((i for i, ln in enumerate(lines) if re.fullmatch(r"\s*Apache License\s*", ln)), None)
        ei = next((i for i, ln in enumerate(lines) if "END OF TERMS AND CONDITIONS" in ln), None)
        if hi is None or ei is None or ei < hi:
            return None
        if _strip_copyrights(lines[:hi]):
            return None
        rest = _norm("\n".join(ln for ln in lines[ei + 1:] if not _copyright_match(ln)))
        std_appendix = (rest.startswith(("APPENDIX: How to apply the Apache License to your work",
                                         'Licensed under the Apache License, Version 2.0 (the "License")'))
                        and rest.endswith("limitations under the License.") and len(rest) < 2000)
        if rest and not std_appendix:
            return None
        return "\n".join(_dedent(lines[hi:ei + 1]))
    body = _strip_copyrights(lines)
    return "\n".join(body) if body else None


# ---------------------------------------------------------------------------
# 束ねる

@dataclass
class LicFile:
    pkg: cd.Package
    rel: str
    text: str
    kinds: list[str]
    copyrights: list[str]
    tbody: str | None
    top: bool  # クレートの直下にあるか(直下のものがクレート自身のコードのライセンス)


@dataclass
class Group:
    kinds: list[str]
    body: str
    files: list[LicFile]
    template: bool  # 著作権の行をクレートごとに分けて出すか(False なら原文のまま)
    no: int = 0

    @property
    def title(self) -> str:
        return " + ".join(self.kinds) if self.kinds else "そのほかの表示"


@dataclass
class Collected:
    deps: cd.Deps
    files: dict[str, list[LicFile]]  # クレートの id → ライセンスファイル
    groups: list[Group]
    canonical: dict[str, Group]  # ライセンス名 → 標準の本文の群
    notes: dict[str, list[str]] = field(default_factory=dict)  # クレートの id → 補足
    warnings: list[str] = field(default_factory=list)


def notice_files(col: "Collected") -> list[LicFile]:
    """NOTICE ファイル(Apache-2.0 の 4 条 (d) で、配るときに添えなければならないもの)"""
    out = [lf for lst in col.files.values() for lf in lst if re.match(r"notice", Path(lf.rel).name, re.I)]
    return sorted(out, key=lambda lf: (lf.pkg.name, cd._version_key(lf.pkg.version), lf.rel))


def collect(deps: cd.Deps) -> Collected:
    files: dict[str, list[LicFile]] = {}
    warnings: list[str] = []
    for pkg in deps.runtime:
        lst = []
        for rel, path in license_paths(pkg):
            t = read_text(path)
            if t is None or not t.strip():
                warnings.append(f"{pkg.name} {pkg.version}: {rel} を読めませんでした")
                continue
            kinds = detect_kinds(t)
            lst.append(LicFile(pkg, rel, t, kinds, copyright_lines(t), template_body(t, kinds), "/" not in rel))
        files[pkg.id] = lst

    # 著作権の行を除いた本文が 2 つ以上のクレートで同じなら、その本文を 1 回だけ載せる
    by_tkey: dict[str, list[LicFile]] = {}
    for lst in files.values():
        for lf in lst:
            if lf.tbody:
                by_tkey.setdefault(_tkey(lf), []).append(lf)
    groups: list[Group] = []
    used: set[int] = set()
    for _key, lfs in by_tkey.items():
        if len({lf.pkg.id for lf in lfs}) < 2:
            continue
        lfs.sort(key=lambda lf: (lf.pkg.name, cd._version_key(lf.pkg.version), lf.rel))
        groups.append(Group(lfs[0].kinds, lfs[0].tbody, lfs, True))
        used.update(id(lf) for lf in lfs)
    # 残りは原文のまま。全文が同じものはまとめる
    by_vkey: dict[str, list[LicFile]] = {}
    for lst in files.values():
        for lf in lst:
            if id(lf) not in used:
                by_vkey.setdefault(_norm(lf.text), []).append(lf)
    for _key, lfs in by_vkey.items():
        lfs.sort(key=lambda lf: (lf.pkg.name, cd._version_key(lf.pkg.version), lf.rel))
        groups.append(Group(lfs[0].kinds, lfs[0].text, lfs, False))

    def gkey(g: Group):
        k = KIND_ORDER.index(g.kinds[0]) if g.kinds else len(KIND_ORDER)
        return (k, len(g.kinds), not g.template, -len(g.files), g.files[0].pkg.name, g.files[0].rel)

    groups.sort(key=gkey)
    for i, g in enumerate(groups, 1):
        g.no = i
    # 本文を持たないクレートのための「標準の本文」: そのライセンスだけの群のうち、まとめた群を先に、
    # 無ければ原文のままの群(本文が 1 クレートにしか無いもの)から
    canonical: dict[str, Group] = {}
    for g in sorted(groups, key=lambda g: (not g.template, g.no)):
        if len(g.kinds) == 1 and g.kinds[0] not in canonical:
            canonical[g.kinds[0]] = g

    col = Collected(deps, files, groups, canonical, warnings=warnings)
    # 配布物に本文が無いライセンスを補う(直下のファイルと COVERS に書いたものだけを、
    # そのクレートのライセンス式の本文とみなす)
    for pkg in deps.runtime:
        notes: list[str] = []
        try:
            node = cd.spdx_parse(pkg.license or "")
        except cd.SpdxError as e:
            col.warnings.append(f"{pkg.name} {pkg.version}: {e}")
            continue
        covered = {k for lf in files[pkg.id] if lf.top for k in lf.kinds}
        for lf in files[pkg.id]:
            covered |= COVERS.get((pkg.name, lf.rel), set())
        if not cd.spdx_satisfied(node, lambda i, _e: i in covered):
            refs = []
            for lic, _exc in cd.spdx_ids(node):
                if lic in covered:
                    continue
                if lic in canonical:
                    refs.append(f"{lic} は本文 {canonical[lic].no}")
                elif lic in NO_TEXT_NOTE:
                    notes.append(NO_TEXT_NOTE[lic])
                else:
                    col.warnings.append(f"{pkg.name} {pkg.version}: {lic} の本文が見つかりません")
                    notes.append(f"{lic} の本文が見つかりません")
            if refs:
                notes.insert(0, "配布物に本文が無いので標準の本文による(" + "、".join(refs) + ")")
            if pkg.authors:
                notes.append("作者(Cargo.toml): " + ", ".join(pkg.authors))
            if pkg.repository:
                notes.append(f"ソース: <{pkg.repository}>")
        col.notes[pkg.id] = notes
    return col


# ---------------------------------------------------------------------------
# 書体と実行時ライブラリ

def ttf_names(path: Path) -> dict[int, str]:
    """TrueType の name 表から文字列を取る(0 = 著作権, 1 = 族, 5 = 版, 13 = ライセンス)。壊れていれば空。"""
    try:
        data = path.read_bytes()
        num = struct.unpack(">H", data[4:6])[0]
        off = None
        for i in range(num):
            tag, _sum, o, _ln = struct.unpack(">4sIII", data[12 + 16 * i:28 + 16 * i])
            if tag == b"name":
                off = o
        if off is None:
            return {}
        _fmt, count, soff = struct.unpack(">HHH", data[off:off + 6])
        best: dict[int, tuple[int, str]] = {}
        for i in range(count):
            pid, eid, lid, nid, ln, o = struct.unpack(">HHHHHH", data[off + 6 + 12 * i:off + 18 + 12 * i])
            raw = data[off + soff + o:off + soff + o + ln]
            if pid == 3 and eid in (0, 1, 10):
                rank, s = (0 if lid == 0x409 else 1), raw.decode("utf-16-be", "replace")
            elif pid == 1 and eid == 0:
                rank, s = 2, raw.decode("mac_roman", "replace")
            else:
                continue
            if nid not in best or rank < best[nid][0]:
                best[nid] = (rank, s.strip())
        return {k: v for k, (_r, v) in best.items()}
    except (OSError, struct.error, IndexError):
        return {}


def run_text(cmd: list[str]) -> str | None:
    try:
        r = subprocess.run(cmd, check=True, capture_output=True, **cd._no_window())
        return r.stdout.decode("utf-8", "replace").strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def html_to_text(s: str) -> str:
    s = re.sub(r"(?is)<(script|style|head).*?</\1>", "", s)
    s = re.sub(r"(?i)<br\s*/?>|</p>|</h\d>|</li>|</div>", "\n", s)
    s = re.sub(r"<[^>]+>", "", s)
    s = html.unescape(s)
    lines = [re.sub(r"[ \t]+", " ", ln).strip() for ln in s.split("\n")]
    out: list[str] = []
    for ln in lines:
        if not ln and (not out or not out[-1]):
            continue
        out.append(ln)
    return "\n".join(out).strip()


@dataclass
class Toolchain:
    rustc: str
    gcc: str | None
    std_crates: list[str]
    texts: list[tuple[str, str, str, str]]  # (名前, 式, 元のファイル, 本文)


# 標準ライブラリの rlib のうち、exe に結合されないもの(試験・proc-macro・別の panic 方式など)
STD_NOT_LINKED = {"test", "getopts", "proc_macro", "profiler_builtins", "panic_abort", "rustc_literal_escaper",
                  "rustc_std_workspace_std", "sysroot"}


def toolchain(sysroot_arg: str | None) -> Toolchain:
    sysroot = sysroot_arg or run_text(["rustc", "--print", "sysroot"])
    if not sysroot:
        raise SystemExit("!! rustc が見つかりません(sysroot を読めない)。PATH を確かめるか --sysroot を渡す")
    root = Path(sysroot)
    rustc = run_text(["rustc", "--version"]) or "?"
    gcc = None
    gexe = root / "bin" / "gcc.exe"
    if gexe.exists():
        gcc = run_text([str(gexe), "-dumpversion"])
    libdir = root / "lib" / "rustlib" / cd.TARGET / "lib"
    std_crates = sorted({re.sub(r"^lib|-[0-9a-f]+\.rlib$", "", p.name) for p in libdir.glob("lib*.rlib")}
                        - STD_NOT_LINKED)
    if not std_crates:
        raise SystemExit(f"!! 標準ライブラリの rlib が見つかりません: {libdir}")
    texts = []
    for name, lic, rel in cd.TOOLCHAIN_PARTS:
        p = root / rel
        t = read_text(p)
        if t is None:
            raise SystemExit(f"!! 実行時ライブラリの表示のファイルがありません: {p}")
        if p.suffix == ".html":
            t = html_to_text(t)
        texts.append((name, lic, rel, t))
    return Toolchain(rustc, gcc, std_crates, texts)


# ---------------------------------------------------------------------------
# 書き出す

def fence(text: str) -> str:
    f = "```"
    while f in text:
        f += "`"
    return f"{f}text\n{text}\n{f}"


def md_cell(s: str) -> str:
    return s.replace("|", "\\|").replace("\n", " ")


HEAD = """\
# 同梱しているソフトウェアの表示(Rust 版 zahyou.exe)

この文書は `rs/tools/gen_notices.py` が、`cargo metadata` と各クレートの配布物にある
ライセンスファイルから機械的に作っています。手で書き足したものではありません。
依存を変えたら作り直してください(`python rs/tools/gen_notices.py`、古いかどうかは `--check`)。
同じものが `zahyou.exe` の中に入っていて、［使い方］タブの「同梱しているソフトウェアの表示」で読めます。

## MIT ライセンスが及ぶ範囲

下の LICENSE —— MIT ライセンス —— が及ぶのは **zahyou 自身のソースコードだけ**です。

    rs/app/ / rs/crates/ / rs/tools/ / rs/testdata/(Rust 版)
    zahyou_engine.py / zahyou_v6.ipynb / dev/ / docs/(Python 版)

オフライン解析のエンジン(`zahyou-solver`・`zahyou-index`・`zahyou-wcs`)も zahyou 自身のコードです。
`zahyou.exe` に入っている第三者のソフトウェア(この文書の後半)、星図データ
(index ファイル)とその元になった星表は含みません。

{license}

## オフライン解析の解き方 — astrometry.net の方式

オフライン解析は、zahyou の中の自前のエンジン(Rust の `zahyou-solver`・`zahyou-index`・
`zahyou-wcs`)が行います。Python 版のエンジン(`dev/solver_wcs.py`・`dev/solver_index.py`・
`dev/solver_core.py`)を Rust に移したもので、zahyou 自身のコードとして MIT ライセンスで配ります。
WSL も astrometry.net の実行ファイルも使いません。

解き方(星 4 つ(広い段は 3 つ)の並びの幾何ハッシュ → 照合 → ベイズの検証 → 改良)は、
下の論文と、astrometry.net の公開文書と BSD-3 の部分のソースを読んでまとめた調べ物をもとに、
Python 版で自分で書き直したものです。GPL の部分(FITS の入出力層 `qfits-an` など)は読んでいません。
**astrometry.net のコードは含んでいません**(取り込みも移植もしていません)。
FITS の読み込みは FITS 規格から書きました。索引ファイルの形式は、実物のファイルを読んで調べました
(`docs/index-format.md`)。

{lang}
- <https://astrometry.net/>

## 同梱していないもの

### 星図データ(index ファイル)

zahyou の実行ファイルには同梱していません。exe は［準備］タブが下記の原典から
落とします。中身の星のデータには、元になった星表の条件がかかります。

- 4100 シリーズ … Tycho-2 星表から作られたもの。<https://data.astrometry.net/4100/>
- 5200 シリーズ … Gaia DR2 と Tycho-2 から作られたもの。
  <https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/>

**Gaia のデータは CC BY-NC 3.0 IGO です。商用利用には ESA の許可が要ります。**
クレジットは ESA/Gaia/DPAC です。

{catalogs}

{gaia_ack}

**配布ページ(Nextcloud)に置いてある `AstrometryData*.zip` は、この
index ファイルをそのまま詰め直したものです。** 中身は上記の原典と同一で、
回線の細い場所でまとめて取れるようにしてあるだけです。
元データの権利は Gaia(ESA/Gaia/DPAC)および Tycho-2 の各制作者にあり、
上の条件(Gaia は非商用・クレジット)はこの zip にもそのままかかります。

### 天体名から座標を引くとき

`zahyou-net` が CDS(ストラスブール天文台)の Sesame(SIMBAD など)と VizieR に問い合わせます。
オンライン解析では Astrometry.net の Web サービス(nova.astrometry.net)も使います。
いずれも実行時の問い合わせで、データを同梱してはいません。

{simbad_ack}

### 画面の日本語の書体

画面の日本語は、Windows に入っている游ゴシック UI(無ければメイリオなど)を実行時に読み込んで
表示します。等幅の欄は、HackGen が入っていればそれを、無ければ Consolas を使います。
**これらの書体は exe に入っていません**(下の「画面の既定の書体」は別のものです)。
"""

# 解き方の出典・星表の論文・定型の謝辞。以前は Python の exe 用の表示(リポジトリ直下の THIRD-PARTY-NOTICES.md)と
# --check で突き合わせていたが、そちらは 7.0.0 の準備で消した。同じ趣旨の文は README・マニュアル(dev/make_manual.py)・
# 星図データの zip の README(dev/make_index_zips.py)にもあるので、直すときはそろえる
LANG_REF = """\
- Lang, D., Hogg, D. W., Mierle, K., Blanton, M., & Roweis, S. (2010),
  "Astrometry.net: Blind astrometric calibration of arbitrary astronomical
  images", *AJ* **139**, 1782"""
CATALOG_REFS = """\
- Høg, E., et al. (2000), "The Tycho-2 Catalogue of the 2.5 million brightest
  stars", *A&A* **355**, L27
- Gaia Collaboration, Prusti, T., et al. (2016), "The Gaia mission",
  *A&A* **595**, A1
- Gaia Collaboration, Brown, A. G. A., et al. (2018), "Gaia Data Release 2.
  Summary of the contents and survey properties", *A&A* **616**, A1"""
GAIA_ACK = """\
> This work has made use of data from the European Space Agency (ESA) mission
> Gaia (https://www.cosmos.esa.int/gaia), processed by the Gaia Data Processing
> and Analysis Consortium (DPAC,
> https://www.cosmos.esa.int/web/gaia/dpac/consortium). Funding for the DPAC
> has been provided by national institutions, in particular the institutions
> participating in the Gaia Multilateral Agreement."""
SIMBAD_ACK = """\
> This research has made use of the SIMBAD database and the VizieR catalogue
> access tool, operated at CDS, Strasbourg, France."""


def render(col: Collected, tc: Toolchain) -> str:
    deps = col.deps
    lic_text = read_text(cd.REPO_DIR / "LICENSE")
    if lic_text is None:
        raise SystemExit(f"!! zahyou の LICENSE がありません: {cd.REPO_DIR / 'LICENSE'}")
    out: list[str] = [HEAD.format(license=fence(lic_text), lang=LANG_REF, catalogs=CATALOG_REFS,
                                  gaia_ack=GAIA_ACK, simbad_ack=SIMBAD_ACK)]
    w = out.append
    pkgs = deps.runtime
    by_id = {p.id: p for p in pkgs}
    ct = [p for p in pkgs if p.compile_time_only]

    # --- 概要
    w("## 実行ファイルに入っているもの\n")
    w("`zahyou.exe` は Rust のプログラムで、下の第三者のソフトウェアを中に含んでいます。")
    w("それぞれの著作権は各プロジェクトに属し、各ライセンスの条件で配布されています。")
    w("ライセンス式が `A OR B` のものは、そのどれか 1 つの条件で使っています"
      "(本文は、配布物にあるものをすべて載せています)。\n")
    w(f"- Rust のクレート: **{len(pkgs)} 個**(`cargo metadata`、ターゲット `{cd.TARGET}`。"
      f"ビルドのときだけ使う {deps.build_only} 個と、試験のときだけ使う {deps.dev_only} 個は除く)")
    w(f"  - うち {len(ct)} 個は proc-macro(コンパイルのときにコードを生成するもの)とその依存です。"
      "exe にコードは入りませんが、念のため載せています")
    w("- egui の既定の書体 4 つ(exe に埋め込まれる。下の「画面の既定の書体」)")
    w("- リンクのときに結合される実行時ライブラリ(下の「Rust の標準ライブラリと実行時ライブラリ」)")
    w("- exe が読み込む DLL は Windows のシステムのものだけです(UCRT を含む)\n")
    counts: dict[str, int] = {}
    for p in pkgs:
        counts[p.license or "(なし)"] = counts.get(p.license or "(なし)", 0) + 1
    w("| ライセンス式(Cargo.toml の値) | クレートの数 |")
    w("|---|---|")
    for lic, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
        w(f"| {md_cell(lic)} | {n} |")
    w("")
    w("**GPL・LGPL・AGPL・MPL のクレートは入っていません**(`rs/tools/check_licenses.py` が、許可した")
    w("ライセンスだけで満たせない式を見つけると失敗します)。\n")

    # --- 書体
    font_pkg = next((p for p in pkgs if p.name == cd.FONT_CRATE), None)
    w("### 画面の既定の書体(exe に埋め込まれるもの)\n")
    if font_pkg is None:
        w(f"(`{cd.FONT_CRATE}` は依存に入っていません)\n")
    else:
        w(f"egui の既定の書体は `{cd.FONT_CRATE}` {font_pkg.version} が `include_bytes!` で exe に埋め込みます"
          "(eframe の機能 `default_fonts`)。書体ファイルの中の著作権表示(name 表の ID 0)と、")
        w("一緒に配られているライセンスのファイルは次のとおりです。本文は下の「ライセンス本文」にあります。\n")
        w("| 書体ファイル | 名前 | 著作権表示 | ライセンス | 本文 |")
        w("|---|---|---|---|---|")
        fmap = {lf.rel: lf for lf in col.files.get(font_pkg.id, [])}
        for ttf, txt, lic in cd.EMBEDDED_FONTS:
            names = ttf_names(font_pkg.manifest_dir / "fonts" / ttf)
            lf = fmap.get("fonts/" + txt)
            g = next((g for g in col.groups if lf in g.files), None) if lf else None
            ref = f"本文 {g.no}(`fonts/{txt}`)" if g else f"`fonts/{txt}` が見つかりません"
            if names.get(0):
                cr = names[0] + "(書体の中の表記)"
            elif lf and lf.copyrights:
                cr = " / ".join(lf.copyrights) + "(ライセンスのファイルの表記。書体の中には無い)"
            else:
                cr = "(表記なし)"
            w(f"| `{ttf}` | {md_cell(names.get(1, '?'))} | {md_cell(cr)} | {lic} | {ref} |")
        w("")
        w("Hack は Bitstream Vera と DejaVu をもとにした書体で、Hack の部分が MIT、元の Bitstream Vera の部分が")
        w("Bitstream Vera License です。Noto Emoji は SIL Open Font License 1.1、Ubuntu は Ubuntu Font Licence 1.0 です。")
        w("どれも、書体を変えずにソフトウェアに組み込んで配ることを認めています(書体だけを単体で売ることは認めていません)。")
        w(f"`{cd.FONT_CRATE}` のコード部分のライセンス(MIT OR Apache-2.0)の本文は配布物に入っていないので、")
        w("標準の本文によります(下の一覧の備考)。\n")

    # --- C とアセンブリ
    native = [p for p in pkgs if p.native_build_deps]
    w("### C とアセンブリを含むもの\n")
    if not native:
        w("C やアセンブリをビルドするクレートはありません。\n")
    for p in native:
        if p.name == "ring":
            w(f"`ring` {p.version}(TLS の暗号。`ureq` → `rustls` が HTTPS の通信で使う)は、ビルドのときに")
            w("**配布物に入っている C のソースを C コンパイラー(MSYS2 UCRT64 の gcc)で組み立て**、")
            w("アセンブリの部分は**配布物に入っている組み立て済みの目的ファイル**(BoringSSL 由来の perlasm を")
            w("NASM 形式で組み立てたもの。`pregenerated/*-nasm.o`)を使って、静的ライブラリ "
              f"`{p.links or 'ring_core'}` にまとめ、exe に結合しています。")
            w("これは ring 自身のコード(BoringSSL・fiat-crypto 由来の部分を含む)を Rust から呼んでいるもので、")
            w("**システムの C ライブラリ(OpenSSL など)への FFI ではありません**。exe が読み込む DLL は")
            w("Windows のシステムのものだけです。ライセンスは "
              f"{p.license}(下の一覧の `ring` の行。`LICENSE`・`LICENSE-BoringSSL`・`LICENSE-other-bits`・")
            w("`third_party/fiat/LICENSE`・`src/polyfill/once_cell/LICENSE-*`)。\n")
        else:
            w(f"`{p.name}` {p.version} はビルドのときに {', '.join(p.native_build_deps)} を使って C などを組み立てます"
              f"(ライセンス {p.license})。\n")

    # --- 実行時ライブラリ
    w("### Rust の標準ライブラリと実行時ライブラリ\n")
    w(f"`{cd.TARGET}` 向けにリンクすると、次のものが exe に静的に結合されます(`rustc --print link-args` で確かめたもの)。")
    w(f"コンパイラーは {tc.rustc}" + (f"、リンカーは MSYS2 UCRT64 の gcc {tc.gcc}" if tc.gcc else "") + "。\n")
    w("| 結合されるもの | ライセンス式 | 表示 |")
    w("|---|---|---|")
    for i, (name, lic, rel, _t) in enumerate(tc.texts, 1):
        w(f"| {md_cell(name)} | {md_cell(lic)} | 下の 実行時-{i}(`{rel}`) |")
    w("")
    w("- 標準ライブラリの rlib: " + "・".join(f"`{c}`" for c in tc.std_crates) + "。")
    w("  Rust の配布物の表示(実行時-1)のとおり、標準ライブラリは Apache-2.0 OR MIT です"
      "(本文は下の「ライセンス本文」の Apache-2.0・MIT と同じ)。")
    w("- `compiler_builtins` には LLVM の compiler-rt の C が入っています。LLVM の例外(実行時-2)により、")
    w("  組み込まれた目的コードについては Apache-2.0 の表示の条件(4 条 (a)(b)(d))がかかりません。")
    w("- libgcc と libgcc_eh は GPL-3.0-or-later に GCC 実行時ライブラリ例外 3.1 が付いたものです(実行時-5)。")
    w("  この例外のもとでは、これらを結合した exe を任意の条件で配れます(zahyou は MIT)。")
    w("- MinGW-w64 の表示(実行時-3)の最後の節にある Wine 由来のヘッダー(LGPL)は、C をコンパイルするときの")
    w("  宣言だけで、exe にコードは入らないと考えています(推測。結合されるライブラリの中身までは確かめていません)。\n")

    # --- 一覧
    w("### クレートの一覧\n")
    w("「本文」の番号は、下の「ライセンス本文」の番号です。\n")
    w("| クレート | 版 | ライセンス式 | 本文 | 備考 |")
    w("|---|---|---|---|---|")
    group_of: dict[int, Group] = {id(lf): g for g in col.groups for lf in g.files}
    for p in pkgs:
        nos = []
        for lf in col.files.get(p.id, []):
            g = group_of[id(lf)]
            if g.no not in nos:
                nos.append(g.no)
        notes = []
        if p.compile_time_only:
            notes.append("proc-macro" if p.proc_macro else "proc-macro の依存")
        if p.native_build_deps:
            notes.append("C とアセンブリ(上の説明)")
        if p.name == cd.FONT_CRATE:
            notes.append("書体を埋め込む(上の説明)")
        notes += col.notes.get(p.id, [])
        w(f"| `{p.name}` | {p.version} | {md_cell(p.license or '(なし)')} | "
          f"{', '.join(str(n) for n in sorted(nos)) or '—'} | {md_cell('。'.join(notes))} |")
    w("")

    # --- 本文
    w("## ライセンス本文\n")
    w("同じ本文は 1 回だけ載せ、その本文に当たるクレートと、それぞれの著作権表示を並べています。")
    w("著作権表示は各クレートのライセンスファイルの中の行をそのまま写したものです"
      "(ファイルに著作権の行が無いものは、代わりに Cargo.toml の作者を添えています)。")
    w("Apache-2.0 の本文の末尾にある付録(使い方の例)は、標準のものそのままのときは省いています。\n")
    nf = notice_files(col)
    if nf:
        w("Apache-2.0 の 4 条 (d) の NOTICE ファイルは次のとおりで、どれも中身をそのまま下に載せています: "
          + "、".join(f"`{lf.pkg.name}` {lf.pkg.version}(`{lf.rel}` → 本文 {group_of[id(lf)].no})" for lf in nf)
          + "。\n")
    else:
        w("Apache-2.0 の 4 条 (d) の NOTICE ファイルを配布物に持つクレートはありません"
          "(あれば、その中身をそのまま下に載せます)。\n")
    for g in col.groups:
        pk = sorted({lf.pkg.id for lf in g.files}, key=lambda i: (by_id[i].name, cd._version_key(by_id[i].version)))
        w(f"### 本文 {g.no}: {g.title}({len(pk)} クレート)\n")
        if g.template:
            w("当たるクレート(ファイル)と著作権表示:\n")
            for lf in g.files:
                if lf.copyrights:
                    cr = " / ".join(lf.copyrights)
                elif lf.top and lf.pkg.authors:
                    # 著作権の行の無いライセンスファイルは多い(serde・syn など)。直下のファイル(クレート自身の
                    # コードのライセンス)に限り、誰のものかが分かるよう Cargo.toml の作者を添える
                    cr = "(ファイルに著作権の行なし。Cargo.toml の作者: " + ", ".join(lf.pkg.authors) + ")"
                else:
                    cr = "(著作権の行なし)"
                w(f"- `{lf.pkg.name}` {lf.pkg.version}(`{lf.rel}`): {md_cell(cr)}")
        else:
            w("当たるクレート(ファイル)。著作権表示は本文の中にあります:\n")
            for lf in g.files:
                w(f"- `{lf.pkg.name}` {lf.pkg.version}(`{lf.rel}`)")
        w("")
        w(fence(g.body))
        w("")

    # --- 実行時ライブラリの表示
    w("## 実行時ライブラリの表示\n")
    w("MSYS2 UCRT64 のツールチェーンに入っている表示のファイルを、そのまま写しています"
      "(HTML のものは文字だけにしています)。\n")
    for i, (name, lic, rel, t) in enumerate(tc.texts, 1):
        w(f"### 実行時-{i}: {name}\n")
        w(f"ライセンス式: {lic}。元のファイル: `{rel}`(rustc の sysroot から)\n")
        w(fence(t))
        w("")
    text = "\n".join(out).rstrip() + "\n"
    return re.sub(r"\n{3,}", "\n\n", text)


def build(args) -> tuple[str, Collected]:
    meta = cd.load_metadata(args.metadata, args.cargo)
    deps = cd.classify(meta)
    col = collect(deps)
    tc = toolchain(args.sysroot)
    return render(col, tc), col


def main(argv=None) -> int:
    cd.stdout_utf8()
    ap = argparse.ArgumentParser(description="rs/THIRD-PARTY-NOTICES.md を作る")
    ap.add_argument("--check", action="store_true", help="書き出さずに、今のファイルが古くないかだけ見る")
    ap.add_argument("--out", default=str(OUT), help="書き出す先(既定: rs/THIRD-PARTY-NOTICES.md)")
    ap.add_argument("--metadata", help="cargo metadata の JSON(無ければ cargo を呼ぶ)")
    ap.add_argument("--cargo", default="cargo")
    ap.add_argument("--sysroot", help="rustc の sysroot(無ければ rustc --print sysroot)")
    args = ap.parse_args(argv)

    text, col = build(args)
    for m in col.warnings:
        print("!! " + m, file=sys.stderr)
    out = Path(args.out)
    if args.check:
        try:
            cur = out.read_bytes().decode("utf-8").replace("\r\n", "\n")
        except (OSError, UnicodeDecodeError):
            cur = None
        if cur != text:
            print(f"!! {out} は古くなっています。python rs/tools/gen_notices.py で作り直してください")
            if cur is not None:
                diff = list(difflib.unified_diff(cur.split("\n"), text.split("\n"), "今のファイル", "作り直したもの",
                                                 lineterm="", n=1))
                for ln in diff[:40]:
                    print("   " + ln)
                if len(diff) > 40:
                    print(f"   …(ほか {len(diff) - 40} 行)")
            return 1
        print(f"OK {out} は最新です(クレート {len(col.deps.runtime)} 個・本文 {len(col.groups)} 種)")
        return 1 if col.warnings else 0
    out.write_bytes(text.encode("utf-8"))
    print(f"書き出しました: {out}({len(text.encode('utf-8')):,} バイト。クレート {len(col.deps.runtime)} 個・"
          f"本文 {len(col.groups)} 種)")
    return 1 if col.warnings else 0


if __name__ == "__main__":
    sys.exit(main())

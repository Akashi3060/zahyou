# -*- coding: utf-8 -*-
"""Rust 版 zahyou.exe に入る依存のライセンスを、許可リストと突き合わせる。

    python rs/tools/check_licenses.py            # 問題があれば終了コード 1
    python rs/tools/check_licenses.py --strict   # OR の選べる式に禁止のライセンスが混ざっているだけでも失敗にする
    python rs/tools/check_licenses.py --list     # クレートごとの判定も全部出す

見るもの:
1. exe に入るクレート(cargo metadata、ターゲット x86_64-pc-windows-gnu。ビルドだけ・試験だけのものは除く)の
   ライセンス式が、許可リストのライセンスだけで満たせるか。GPL・LGPL・AGPL・MPL などが要る式は失敗。
   式が無いもの・読めないものも失敗(中身を人が確かめて、ここの OVERRIDES に書くまで)
2. C やアセンブリをビルドするクレート(cc などをビルド依存に持つもの)が、確かめ済みのもの(ring)だけか
3. exe に埋め込まれる書体と、リンクのときに結合される実行時ライブラリのライセンス式(cargo_deps.py に書いたもの)
4. どのクレートにもライセンスの本文が用意できるか(gen_notices.py と同じ集め方で)

標準ライブラリだけで動く。
"""
from __future__ import annotations

import argparse
import re
import sys

sys.dont_write_bytecode = True
import cargo_deps as cd  # noqa: E402

# 許可するライセンス(SPDX の名前)。どれも「著作権表示と本文を添えれば、改変の有無にかかわらず
# 自由に結合・配布できる」許容型のもの
ALLOWED = {
    "MIT", "MIT-0", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD",
    "Unicode-3.0", "Unicode-DFS-2016", "BSL-1.0", "CC0-1.0", "Unlicense", "CDLA-Permissive-2.0",
    # 書体(exe に埋め込まれる egui の既定の書体)
    "OFL-1.1", "Ubuntu-font-1.0", "Bitstream-Vera",
}
# 例外付きで許可するもの(ライセンス, 例外)
ALLOWED_WITH = {
    ("Apache-2.0", "LLVM-exception"),
}
# 実行時ライブラリ(cargo の外。cargo_deps.TOOLCHAIN_PARTS)にだけ許すもの
TOOLCHAIN_ALLOWED = {"ZPL-2.1", "SunPro", "LicenseRef-Lucent-gdtoa"}
TOOLCHAIN_ALLOWED_WITH = {
    # GCC 実行時ライブラリ例外: 結合した exe を任意の条件で配れる
    ("GPL-3.0-or-later", "GCC-exception-3.1"),
}
# 入っていたら目立たせるもの(許可リストに無いものは全部失敗だが、とくにこれらはコピーレフト)
COPYLEFT = re.compile(r"GPL|MPL|EUPL|CDDL|EPL|SSPL|OSL|CPAL|BUSL|Commons-Clause|CC-BY-NC|CC-BY-SA|"
                      r"RPL|Sleepycat|QPL|NCSA-?NC", re.I)
# C やアセンブリをビルドしてよいクレート(中身を確かめたもの)。
# ring: TLS の暗号。自分の C とアセンブリを cc で組み立てて静的に結合する(システムの C ライブラリへの FFI ではない)
ALLOWED_NATIVE = {"ring"}
# ライセンス式が無い・読めないクレートを、人が確かめて書き足すところ: {(名前, 版): 式}
OVERRIDES: dict[tuple[str, str], str] = {}


def _canon(s: str) -> str:
    return s.lower()


def make_ok(allowed: set, allowed_with: set):
    al = {_canon(x) for x in allowed}
    aw = {(_canon(a), _canon(b)) for a, b in allowed_with}

    def ok(lic: str, exc: str | None) -> bool:
        if exc is None:
            return _canon(lic) in al
        # 例外が付いていても、元のライセンスが許可されていれば許可(例外は許す方向にしか働かない)
        return _canon(lic) in al or (_canon(lic), _canon(exc)) in aw

    return ok


def judge(expr: str | None, toolchain: bool = False):
    """(合否, 理由, 式に出てくる禁止のライセンス) を返す"""
    allowed = ALLOWED | (TOOLCHAIN_ALLOWED if toolchain else set())
    allowed_with = ALLOWED_WITH | (TOOLCHAIN_ALLOWED_WITH if toolchain else set())
    ok = make_ok(allowed, allowed_with)
    try:
        node = cd.spdx_parse(expr or "")
    except cd.SpdxError as e:
        return False, str(e), []
    bad = [(lic, exc) for lic, exc in cd.spdx_ids(node) if not ok(lic, exc)]
    bad_names = [f"{lic} WITH {exc}" if exc else lic for lic, exc in bad]
    if cd.spdx_satisfied(node, ok):
        if bad_names:
            return True, "選べる式なので許可されたライセンスを選ぶ(" + "・".join(bad_names) + " は使わない)", bad_names
        return True, "", []
    return False, "許可リストに無いライセンスが要る: " + "・".join(bad_names), bad_names


def main(argv=None) -> int:
    cd.stdout_utf8()
    ap = argparse.ArgumentParser(description="exe に入る依存のライセンスを許可リストと突き合わせる")
    ap.add_argument("--metadata", help="cargo metadata の JSON(無ければ cargo を呼ぶ)")
    ap.add_argument("--cargo", default="cargo")
    ap.add_argument("--strict", action="store_true", help="OR の選べる式に禁止のライセンスがあるだけでも失敗")
    ap.add_argument("--list", action="store_true", help="クレートごとの判定を全部出す")
    ap.add_argument("--no-texts", action="store_true", help="ライセンスの本文が揃うかを見ない")
    args = ap.parse_args(argv)

    meta = cd.load_metadata(args.metadata, args.cargo)
    deps = cd.classify(meta)
    fails: list[str] = []
    notes: list[str] = []

    print(f"exe に入るクレート: {len(deps.runtime)} 個(ビルドだけ {deps.build_only} 個・試験だけ {deps.dev_only} 個は対象外。"
          f"ターゲット {cd.TARGET})")
    counts: dict[str, int] = {}
    for p in deps.runtime:
        expr = OVERRIDES.get((p.name, p.version), p.license)
        counts[expr or "(なし)"] = counts.get(expr or "(なし)", 0) + 1
        if not expr:
            lf = f"(license_file = {p.license_file})" if p.license_file else ""
            fails.append(f"{p.name} {p.version}: ライセンス式がありません{lf}。中身を確かめて OVERRIDES に書く")
            continue
        good, why, bad = judge(expr)
        tag = f"{p.name} {p.version}: {expr}"
        if not good:
            mark = "(コピーレフト)" if any(COPYLEFT.search(b) for b in bad) else ""
            fails.append(f"{tag} — {why}{mark}")
        elif bad:
            (fails if args.strict else notes).append(f"{tag} — {why}")
        if args.list:
            print(f"  {'OK ' if good else 'NG '} {tag}")

    print("\nライセンス式ごとの数:")
    for lic, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"  {n:4d}  {lic}")

    # C やアセンブリをビルドするもの
    native = [p for p in deps.runtime if p.native_build_deps]
    print("\nC やアセンブリをビルドするクレート: " + (", ".join(f"{p.name} {p.version}({'・'.join(p.native_build_deps)})"
                                                     for p in native) or "なし"))
    for p in native:
        if p.name not in ALLOWED_NATIVE:
            fails.append(f"{p.name} {p.version}: C などをビルドする({', '.join(p.native_build_deps)})。"
                         "中身を確かめてから ALLOWED_NATIVE に足す")

    # cargo の外のもの
    print("\ncargo の外で exe に入るもの:")
    for name, expr, kind in cd.extra_components():
        good, why, _bad = judge(expr, toolchain=(kind == "toolchain"))
        print(f"  {'OK ' if good else 'NG '} {name}: {expr}" + (f" — {why}" if why else ""))
        if not good:
            fails.append(f"{name}: {expr} — {why}")

    # ライセンスの本文が揃うか
    if not args.no_texts:
        import gen_notices  # 同じ集め方を使う
        col = gen_notices.collect(deps)
        for m in col.warnings:
            fails.append("本文: " + m)
        print(f"\nライセンスの本文: {len(col.groups)} 種(本文が揃わないもの {len(col.warnings)} 件)")

    if notes:
        print("\n注意(失敗ではない):")
        for m in notes:
            print("  - " + m)
    if fails:
        print("\nNG:")
        for m in fails:
            print("  - " + m)
        return 1
    print("\nOK 許可リストにないライセンスはありません(GPL・LGPL・AGPL・MPL などは入っていない)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

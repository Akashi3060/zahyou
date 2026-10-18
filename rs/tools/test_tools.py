# -*- coding: utf-8 -*-
"""rs/tools の試験(標準ライブラリの unittest だけで動く)。

    python -B rs/tools/test_tools.py

cargo が PATH にあれば、実際の依存で THIRD-PARTY-NOTICES.md が最新か(gen_notices.py --check)と、
許可リストの判定(check_licenses.py)も確かめる。無ければその 2 つは SKIP。
"""
from __future__ import annotations

import io
import json
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import cargo_deps as cd  # noqa: E402
import check_licenses as cl  # noqa: E402
import gen_notices as gn  # noqa: E402

MIT_BODY = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""

# Apache-2.0 の本文の代わり(見分けの目印と、付録の判定に要るところだけ)
APACHE_HEAD = """                              Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.
   (c) You must retain, in the Source form of any Derivative Works
   copyright notice that is included in or attached to the work

END OF TERMS AND CONDITIONS"""
APACHE_APPENDIX = """
APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information.

Copyright {who}

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License."""


class SpdxTest(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(cd.spdx_parse("MIT"), ("lic", "MIT", None))
        self.assertEqual(cd.spdx_parse("MIT/Apache-2.0"), ("or", [("lic", "MIT", None), ("lic", "Apache-2.0", None)]))
        n = cd.spdx_parse("(MIT OR Apache-2.0) AND Unicode-3.0")
        self.assertEqual(n[0], "and")
        self.assertEqual(cd.spdx_ids(n), [("MIT", None), ("Apache-2.0", None), ("Unicode-3.0", None)])
        n = cd.spdx_parse("GPL-3.0-or-later WITH GCC-exception-3.1")
        self.assertEqual(n, ("lic", "GPL-3.0-or-later", "GCC-exception-3.1"))
        # AND は OR より強く結びつく
        n = cd.spdx_parse("A OR B AND C")
        self.assertEqual(n, ("or", [("lic", "A", None), ("and", [("lic", "B", None), ("lic", "C", None)])]))

    def test_parse_errors(self):
        for bad in ["", "   ", "MIT AND", "(MIT", "MIT OR OR Apache-2.0", "MIT)", "MIT WITH", "MIT, Apache-2.0", None]:
            with self.assertRaises(cd.SpdxError, msg=repr(bad)):
                cd.spdx_parse(bad)

    def test_judge(self):
        self.assertTrue(cl.judge("MIT OR Apache-2.0")[0])
        self.assertTrue(cl.judge("Apache-2.0 AND ISC")[0])
        self.assertTrue(cl.judge("(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0")[0])
        self.assertTrue(cl.judge("Apache-2.0 WITH LLVM-exception")[0])
        for bad in ["GPL-3.0-only", "LGPL-2.1-or-later", "AGPL-3.0", "MPL-2.0", "MIT AND GPL-2.0",
                    "SSPL-1.0", "CC-BY-NC-4.0", "", "MIT AND"]:
            self.assertFalse(cl.judge(bad)[0], bad)
        # 選べる式なら通るが、禁止のものが混ざっていることは返す(--strict で失敗にする)
        good, _why, bad = cl.judge("MIT OR GPL-3.0-only")
        self.assertTrue(good)
        self.assertEqual(bad, ["GPL-3.0-only"])
        # GCC の例外付き GPL は、実行時ライブラリにだけ許す
        self.assertFalse(cl.judge("GPL-3.0-or-later WITH GCC-exception-3.1")[0])
        self.assertTrue(cl.judge("GPL-3.0-or-later WITH GCC-exception-3.1", toolchain=True)[0])
        self.assertFalse(cl.judge("GPL-3.0-or-later", toolchain=True)[0])
        self.assertFalse(cl.judge("ZPL-2.1")[0])
        self.assertTrue(cl.judge("ZPL-2.1", toolchain=True)[0])

    def test_extra_components_pass(self):
        for name, expr, kind in cd.extra_components():
            self.assertTrue(cl.judge(expr, toolchain=(kind == "toolchain"))[0], name)


class TextTest(unittest.TestCase):
    def test_copyright_lines(self):
        t = "\n".join([
            "Copyright (c) 2019 Daniel Gee",
            "   (c) You must retain, in the Source form",
            "      copyright notice that is included in or attached to the work",
            "Copyright [yyyy] [name of copyright owner]",
            "COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM",
            "Copyright Holder(s) under this licence",
            "// Copyright 2015-2016 Brian Smith.",
            "Copyright © 2015, Simonas Kazlauskas",
            "© 2005-2020 Rich Felker, et al.",
            "COPYRIGHT AND PERMISSION NOTICE",
            "Copyright (c) <year> <copyright holders>",
            "Copyright (c) 2019 Daniel Gee",
        ])
        self.assertEqual(gn.copyright_lines(t), [
            "Copyright (c) 2019 Daniel Gee", "Copyright 2015-2016 Brian Smith.",
            "Copyright © 2015, Simonas Kazlauskas", "© 2005-2020 Rich Felker, et al.",
        ])

    def test_detect(self):
        self.assertEqual(gn.detect_kinds("MIT License\n\nCopyright (c) 2020 X\n\n" + MIT_BODY), ["MIT"])
        isc = ("// Copyright 2015 B.\n//\n// Permission to use, copy, modify, and/or distribute this software for any\n"
               "// purpose with or without fee is hereby granted, provided that the above\n"
               "// copyright notice and this permission notice appear in all copies.\n")
        self.assertEqual(gn.detect_kinds(isc), ["ISC"])
        self.assertEqual(gn.detect_kinds(APACHE_HEAD), ["Apache-2.0"])
        self.assertEqual(gn.detect_kinds("MIT OR Apache-2.0"), [])

    def test_template_body(self):
        a = gn.template_body("MIT License\n\nCopyright (c) 2020 A\n\n" + MIT_BODY, ["MIT"])
        b = gn.template_body("Copyright (c) 1999 B\nCopyright (c) 2001 C\n\n" + MIT_BODY, ["MIT"])
        self.assertEqual(gn._norm(a), gn._norm(b))
        self.assertNotIn("Copyright (c) 2020", a)
        # Apache-2.0: 標準の付録(著作権の行を除く)は落とす。付録の外に文があれば原文のまま(None)
        x = gn.template_body(APACHE_HEAD + "\n" + APACHE_APPENDIX.format(who="2020 X"), ["Apache-2.0"])
        y = gn.template_body(APACHE_HEAD, ["Apache-2.0"])
        self.assertEqual(x, y)
        self.assertTrue(x.endswith("END OF TERMS AND CONDITIONS"))
        self.assertIsNone(gn.template_body(APACHE_HEAD + "\n\nThis file also contains code under other terms.",
                                           ["Apache-2.0"]))
        self.assertIsNone(gn.template_body("Some preface text.\n\n" + APACHE_HEAD, ["Apache-2.0"]))

    def test_read_text(self):
        d = Path(tempfile.mkdtemp())
        try:
            p = d / "LICENSE"
            p.write_bytes(b"\xef\xbb\xbf\r\n\r\nCopyright (c) 2020 X  \r\n\tbody\r\n\r\n")
            self.assertEqual(gn.read_text(p), "Copyright (c) 2020 X\n    body")
            q = d / "bin"
            q.write_bytes(b"\x00\x01\x02")
            self.assertIsNone(gn.read_text(q))
            self.assertIsNone(gn.read_text(d / "nothing"))
        finally:
            shutil.rmtree(d)

    def test_ttf_names_broken(self):
        d = Path(tempfile.mkdtemp())
        try:
            p = d / "x.ttf"
            for data in [b"", b"\x00\x01\x00\x00\x00\x05", b"\x00" * 100]:
                p.write_bytes(data)
                self.assertEqual(gn.ttf_names(p), {})
        finally:
            shutil.rmtree(d)


def fake_metadata(root: Path, extra_license: str | None = None) -> dict:
    """ワークスペース app → a(通常)→ pm(proc-macro)→ q、app → b(ビルド)、app → d(試験)の依存の形"""
    def pkg(name, lic, kind="lib", text=None, build_deps=()):
        d = root / name
        d.mkdir(parents=True, exist_ok=True)
        (d / "Cargo.toml").write_text("", encoding="utf-8")
        if text:
            (d / "LICENSE").write_text(text, encoding="utf-8")
        return {
            "id": f"{name} 1.0.0", "name": name, "version": "1.0.0", "license": lic, "license_file": None,
            "manifest_path": str(d / "Cargo.toml"), "authors": [f"{name} author"], "repository": None,
            "targets": [{"kind": [kind]}], "links": None,
            "dependencies": [{"name": b, "kind": "build"} for b in build_deps],
        }

    mit = lambda who: f"Copyright (c) 2020 {who}\n\n{MIT_BODY}"  # noqa: E731
    packages = [
        pkg("app", "MIT"),
        pkg("a", extra_license or "MIT", text=mit("A")),
        pkg("pm", "MIT", kind="proc-macro", text=mit("PM")),
        pkg("q", "MIT OR Apache-2.0", text=mit("Q")),
        pkg("b", "GPL-3.0-only"),
        pkg("d", "GPL-3.0-only"),
        pkg("cc", "MIT"),
    ]
    packages[1]["dependencies"].append({"name": "cc", "kind": "build"})

    def node(i, deps):
        return {"id": f"{i} 1.0.0", "deps": [{"pkg": f"{t} 1.0.0", "dep_kinds": [{"kind": k}]} for t, k in deps]}

    return {
        "packages": packages,
        "workspace_members": ["app 1.0.0"],
        "resolve": {"nodes": [
            node("app", [("a", None), ("b", "build"), ("d", "dev")]),
            node("a", [("pm", None), ("cc", "build")]),
            node("pm", [("q", None)]),
            node("q", []), node("b", []), node("d", []), node("cc", []),
        ]},
    }


class DepsTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_classify(self):
        deps = cd.classify(fake_metadata(self.dir))
        names = [p.name for p in deps.runtime]
        self.assertEqual(names, ["a", "pm", "q"])  # b(ビルド)・d(試験)・cc(ビルド)は入らない
        self.assertEqual(deps.build_only, 2)
        self.assertEqual(deps.dev_only, 1)
        ct = {p.name for p in deps.runtime if p.compile_time_only}
        self.assertEqual(ct, {"pm", "q"})
        self.assertEqual([p.name for p in deps.runtime if p.native_build_deps], ["a"])

    def test_collect_groups(self):
        deps = cd.classify(fake_metadata(self.dir))
        col = gn.collect(deps)
        # 3 クレートの MIT は著作権の行だけが違うので 1 つにまとまる
        self.assertEqual(len(col.groups), 1)
        g = col.groups[0]
        self.assertTrue(g.template)
        self.assertEqual(g.kinds, ["MIT"])
        self.assertEqual([lf.copyrights for lf in g.files],
                         [["Copyright (c) 2020 A"], ["Copyright (c) 2020 PM"], ["Copyright (c) 2020 Q"]])
        self.assertEqual(col.warnings, [])

    def test_notice_included(self):
        # Apache-2.0 の NOTICE はどこにも捨てずに載せる(試験のフォルダーの中のものは exe に入らないので拾わない)
        meta = fake_metadata(self.dir)
        (self.dir / "q" / "NOTICE").write_text("q includes software developed at Example Corp.\n", encoding="utf-8")
        (self.dir / "q" / "tests").mkdir()
        (self.dir / "q" / "tests" / "NOTICE").write_text("test images by someone\n", encoding="utf-8")
        col = gn.collect(cd.classify(meta))
        nf = gn.notice_files(col)
        self.assertEqual([(lf.pkg.name, lf.rel) for lf in nf], [("q", "NOTICE")])
        self.assertTrue(any(lf in g.files for g in col.groups for lf in nf))

    def test_check_fails_on_gpl_and_native(self):
        p = self.dir / "meta.json"
        p.write_text(json.dumps(fake_metadata(self.dir / "c", extra_license="LGPL-2.1-or-later")), encoding="utf-8")
        buf = io.StringIO()
        with redirect_stdout(buf), redirect_stderr(io.StringIO()):
            rc = cl.main(["--metadata", str(p)])
        self.assertEqual(rc, 1)
        out = buf.getvalue()
        self.assertIn("LGPL-2.1-or-later", out)
        self.assertIn("ALLOWED_NATIVE", out)  # a が cc でビルドする(ring 以外)


@unittest.skipUnless(shutil.which("cargo") and shutil.which("rustc"), "cargo が PATH に無い")
class RealTest(unittest.TestCase):
    def test_notices_up_to_date(self):
        buf = io.StringIO()
        with redirect_stdout(buf), redirect_stderr(io.StringIO()):
            rc = gn.main(["--check"])
        self.assertEqual(rc, 0, buf.getvalue())

    def test_licenses_allowed(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = cl.main([])
        self.assertEqual(rc, 0, buf.getvalue())
        self.assertIn("ring", buf.getvalue())


if __name__ == "__main__":
    cd.stdout_utf8()
    unittest.main(verbosity=2)

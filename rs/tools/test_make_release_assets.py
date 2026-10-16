# -*- coding: utf-8 -*-
"""make_release_assets.py の試験(標準ライブラリの unittest だけで動く)。

    python -B rs/tools/test_make_release_assets.py

release ビルドの exe があれば(環境変数 ZAHYOU_RELEASE_EXE、無ければ CARGO_TARGET_DIR か rs/target の release)、
その版情報が rs/Cargo.toml の版と合うかも確かめる。無ければその試験は SKIP。
"""
from __future__ import annotations

import json
import os
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import make_release_assets as mra  # noqa: E402


def string_struct(key: str, value: str) -> bytes:
    """StringFileInfo の String 構造を 1 つ作る(試験用)。4 バイト境界に置く前提"""
    k = key.encode("utf-16-le") + b"\x00\x00"
    v = value.encode("utf-16-le") + b"\x00\x00"
    head_len = 6 + len(k)
    pad = mra._align4(head_len) - head_len
    total = head_len + pad + len(v)
    return struct.pack("<HHH", total, len(value) + 1, 1) + k + b"\x00" * pad + v


def fixed_info(ver: tuple[int, int, int, int], prerelease: bool) -> bytes:
    ms, ls = (ver[0] << 16) | ver[1], (ver[2] << 16) | ver[3]
    flags = mra.VS_FF_PRERELEASE if prerelease else 0
    return struct.pack("<13I", 0xFEEF04BD, 0x10000, ms, ls, ms, ls, 0x3F, flags, 4, 1, 0, 0, 0)


def fake_exe(file_ver: str, product_ver: str, ver4=(7, 0, 0, 0), prerelease=True) -> bytes:
    """版情報らしいバイト列(本物の PE ではない。読み取りの試験用)"""
    blob = b"MZ" + b"\x00" * 62 + b"junk" * 16
    for key, value in (("ProductName", "zahyou"), ("FileVersion", file_ver), ("ProductVersion", product_ver),
                       ("OriginalFilename", "zahyou.exe")):
        blob += b"\x00" * (mra._align4(len(blob)) - len(blob))
        blob += string_struct(key, value)
    blob += b"\x00" * (mra._align4(len(blob)) - len(blob))
    return blob + fixed_info(ver4, prerelease) + b"tail"


def find_release_exe() -> Path | None:
    cands = []
    if os.environ.get("ZAHYOU_RELEASE_EXE"):
        cands.append(Path(os.environ["ZAHYOU_RELEASE_EXE"]))
    if os.environ.get("CARGO_TARGET_DIR"):
        cands.append(Path(os.environ["CARGO_TARGET_DIR"]) / "release" / "zahyou.exe")
    cands.append(mra.RS_DIR / "target" / "release" / "zahyou.exe")
    return next((p for p in cands if p.is_file()), None)


class NameTest(unittest.TestCase):
    def test_split_version(self):
        self.assertEqual(mra.split_version("7.0.0-beta.1"), ((7, 0, 0), "beta.1"))
        self.assertEqual(mra.split_version("7.0.0"), ((7, 0, 0), ""))
        for bad in ("v7.0.0", "7.0", "7.0.0-", "7.0.0 beta"):
            with self.assertRaises(ValueError, msg=bad):
                mra.split_version(bad)

    def test_notebook_asset_name(self):
        self.assertEqual(mra.notebook_asset_name("7.0.0-beta.1"), "zahyou_v7_beta.ipynb")
        self.assertEqual(mra.notebook_asset_name("7.0.0-beta.12"), "zahyou_v7_beta.ipynb")
        self.assertEqual(mra.notebook_asset_name("7.0.0-rc.1"), "zahyou_v7_rc.ipynb")
        self.assertEqual(mra.notebook_asset_name("7.0.0"), "zahyou_v7.ipynb")
        self.assertEqual(mra.notebook_asset_name("8.1.2"), "zahyou_v8.ipynb")

    def test_real_versions(self):
        # 本物の rs/Cargo.toml と dev/make_manual.py から読める
        core, _ = mra.split_version(mra.cargo_version())
        self.assertGreaterEqual(core[0], 7)
        self.assertRegex(mra.manual_version(), r"^v\d+$")

    def test_cargo_version_from_file(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "Cargo.toml"
            p.write_text('[workspace]\nmembers = []\n\n[workspace.package]\nversion = "7.0.0-beta.3"\n', encoding="utf-8")
            self.assertEqual(mra.cargo_version(p), "7.0.0-beta.3")


class PeTest(unittest.TestCase):
    def test_strings(self):
        data = fake_exe("7.0.0-beta.1", "v7.0.0-beta.1")
        s = mra.pe_version_strings(data)
        self.assertEqual(s["FileVersion"], "7.0.0-beta.1")
        self.assertEqual(s["ProductVersion"], "v7.0.0-beta.1")
        self.assertEqual(s["OriginalFilename"], "zahyou.exe")

    def test_ignores_loose_text(self):
        # 構造に入っていない "FileVersion" の文字(wType が 1 でない・境界がずれている)は拾わない
        loose = b"\x00\x00\x00\x00" + b"\x01\x00\x09\x00\x07\x00" + "FileVersion".encode("utf-16-le") + b"\x00\x00" + "x".encode("utf-16-le")
        data = loose + fake_exe("7.0.0", "v7.0.0", prerelease=False)
        self.assertEqual(mra.pe_version_strings(data)["FileVersion"], "7.0.0")

    def test_missing(self):
        self.assertEqual(mra.pe_version_strings(b"MZ" + b"\x00" * 100), {})
        self.assertIsNone(mra.pe_fixed_info(b"MZ" + b"\x00" * 100))

    def test_fixed_info(self):
        f = mra.pe_fixed_info(fake_exe("7.0.0-beta.1", "v7.0.0-beta.1", (7, 0, 0, 0), True))
        self.assertEqual(f, {"file_version": (7, 0, 0, 0), "prerelease": True})
        f = mra.pe_fixed_info(fake_exe("7.0.0", "v7.0.0", (7, 0, 0, 0), False))
        self.assertFalse(f["prerelease"])

    def test_notices_embedded(self):
        notices = b"# title\r\nline 1\r\nline 2\r\n"
        self.assertTrue(mra.notices_embedded(b"xx# title\nline 1\nline 2\nyy", notices))
        self.assertTrue(mra.notices_embedded(b"xx" + notices + b"yy", notices))
        self.assertFalse(mra.notices_embedded(b"xx# title\nline 1\nline 3\nyy", notices))


class NotebookTest(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        self.dir = Path(self.t.name)

    def tearDown(self):
        self.t.cleanup()

    def nb(self, name: str, cells: list[tuple[str, str]], outputs=False) -> Path:
        p = self.dir / name
        data = {"cells": [{"id": i, "cell_type": "code", "source": s.splitlines(keepends=True),
                           "outputs": [{"output_type": "stream", "text": "example"}] if outputs else [],
                           "metadata": {}, "execution_count": None} for i, s in cells],
                "metadata": {}, "nbformat": 4, "nbformat_minor": 5}
        p.write_bytes(json.dumps(data, ensure_ascii=False, indent=1).encode("utf-8"))
        return p

    def test_same_sources_with_outputs(self):
        built = self.nb("built.ipynb", [("a", "x = 1\n"), ("c", "run()\n")])
        repo = self.nb("repo.ipynb", [("a", "x = 1\n"), ("c", "run()\n")], outputs=True)
        self.assertEqual(mra.compare_notebooks(built, repo), [])

    def test_crlf_does_not_matter(self):
        built = self.nb("built.ipynb", [("a", "x = 1\ny = 2\n")])
        repo = self.nb("repo.ipynb", [("a", "x = 1\r\ny = 2\r\n")])
        self.assertEqual(mra.compare_notebooks(built, repo), [])

    def test_different_source(self):
        built = self.nb("built.ipynb", [("a", "x = 1\n"), ("c", "run()\n")])
        repo = self.nb("repo.ipynb", [("a", "x = 2\n"), ("c", "run()\n")])
        self.assertIn("a", mra.compare_notebooks(built, repo)[0])

    def test_different_ids(self):
        built = self.nb("built.ipynb", [("a", "x\n"), ("b", "y\n")])
        repo = self.nb("repo.ipynb", [("a", "x\n"), ("c", "y\n")])
        self.assertIn("並び", mra.compare_notebooks(built, repo)[0])

    def test_engines(self):
        a, b, c = self.dir / "a.py", self.dir / "b.py", self.dir / "c.py"
        a.write_bytes(b"x = 1\ny = 2\n")
        b.write_bytes(b"x = 1\r\ny = 2\r\n")
        c.write_bytes(b"x = 1\ny = 3\n")
        self.assertEqual(mra.compare_engines(a, b), [])
        self.assertTrue(mra.compare_engines(a, c))

    def test_notebook_version(self):
        e = self.dir / "e.py"
        e.write_bytes(b'x = 1\r\nNB_VERSION = "v7.0.0-beta.2"\r\n')
        self.assertEqual(mra.notebook_version(mra._norm(e.read_text(encoding="utf-8"))), "v7.0.0-beta.2")
        self.assertEqual(mra.check_notebook_version(e, "7.0.0-beta.2"), [])
        self.assertIn("NB_VERSION", mra.check_notebook_version(e, "7.0.0-beta.3")[0])
        e.write_bytes(b"x = 1\n")
        self.assertTrue(mra.check_notebook_version(e, "7.0.0-beta.2"))

    def test_real_notebook_version(self):
        # 直下の zahyou_engine.py の見出しの版が rs/Cargo.toml の版と同じ
        self.assertEqual(mra.check_notebook_version(mra.REPO_DIR / "zahyou_engine.py", mra.cargo_version()), [])


class AssembleTest(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        self.dir = Path(self.t.name)

    def tearDown(self):
        self.t.cleanup()

    def test_assemble_and_sums(self):
        src = self.dir / "src"
        src.mkdir()
        plan = mra.Plan()
        for name, body in (("zahyou.exe", b"MZ exe"), ("zahyou_v7_beta.ipynb", b"{}"), ("LICENSE.txt", b"MIT\r\n")):
            (src / name).write_bytes(body)
            plan.files[name] = src / name
        out = self.dir / "out"
        names = mra.assemble(plan, out)
        self.assertEqual(names, ["zahyou.exe", "zahyou_v7_beta.ipynb", "LICENSE.txt"])
        sums = (out / "SHA256SUMS.txt").read_bytes()
        self.assertNotIn(b"\r", sums, "SHA256SUMS.txt の改行は LF")
        lines = sums.decode("ascii").splitlines()
        self.assertEqual(len(lines), 3)
        self.assertRegex(lines[0], r"^[0-9a-f]{64}  zahyou\.exe$")
        self.assertEqual(mra.verify_sha256sums(out), [])
        # 文字のファイルは LF にそろえる(作業ツリーの改行によらず同じ SHA256)。exe はそのまま
        self.assertEqual((out / "LICENSE.txt").read_bytes(), b"MIT\n")
        self.assertEqual((out / "zahyou.exe").read_bytes(), b"MZ exe")
        # 中身が変わったら見つける
        (out / "LICENSE.txt").write_bytes(b"changed")
        self.assertEqual(len(mra.verify_sha256sums(out)), 1)

    def test_missing_source(self):
        plan = mra.Plan(files={"zahyou.exe": self.dir / "nothing.exe"})
        with self.assertRaises(FileNotFoundError):
            mra.assemble(plan, self.dir / "out")

    def test_out_dir_rules(self):
        self.assertIsNone(mra.check_out_dir(self.dir / "new"))
        self.assertIsNone(mra.check_out_dir(self.dir))  # 空のフォルダー
        (self.dir / "x").write_text("x")
        self.assertIn("空のフォルダーではありません", mra.check_out_dir(self.dir))
        self.assertIn("リポジトリの中", mra.check_out_dir(mra.REPO_DIR / "dist-test-never-created"))
        self.assertIsNone(mra.check_out_dir(mra.REPO_DIR / "dist-test-never-created", allow_in_repo=True))

    def test_text_assets(self):
        for name in ("zahyou_v7_beta.ipynb", "zahyou_engine.py", "LICENSE.txt", "THIRD-PARTY-NOTICES.md", "SHA256SUMS.txt"):
            self.assertTrue(mra.is_text_asset(name), name)
        for name in ("zahyou.exe", "zahyou-manual-v4.pdf"):
            self.assertFalse(mra.is_text_asset(name), name)

    def test_plan_names(self):
        p = mra.plan_assets(Path("zahyou.exe"), "7.0.0-beta.1", "v4")
        self.assertEqual(list(p.files), ["zahyou.exe", "zahyou_v7_beta.ipynb", "zahyou_engine.py",
                                         "zahyou-manual-v4.pdf", "LICENSE.txt", "THIRD-PARTY-NOTICES.md"])
        self.assertEqual(p.files["THIRD-PARTY-NOTICES.md"], mra.RS_DIR / "THIRD-PARTY-NOTICES.md")
        self.assertEqual(p.files["zahyou_v7_beta.ipynb"], mra.REPO_DIR / "zahyou_v6.ipynb")

    def test_main_refuses_bad_out(self):
        exe = self.dir / "zahyou.exe"
        exe.write_bytes(b"MZ")
        (self.dir / "full").mkdir()
        (self.dir / "full" / "x").write_text("x")
        self.assertEqual(mra.main(["--exe", str(exe), "--out", str(self.dir / "full")]), 1)
        self.assertEqual(mra.main(["--exe", str(self.dir / "none.exe"), "--out", str(self.dir / "o")]), 1)


@unittest.skipUnless(find_release_exe(), "release ビルドの zahyou.exe が無い(ZAHYOU_RELEASE_EXE で指定できる)")
class RealExeTest(unittest.TestCase):
    def test_version_matches_cargo(self):
        exe = find_release_exe()
        problems = mra.check_version(exe, mra.cargo_version())
        self.assertEqual(problems, [], f"{exe}: {problems}")

    def test_notices_embedded(self):
        exe = find_release_exe()
        self.assertTrue(mra.notices_embedded(exe.read_bytes(), (mra.RS_DIR / "THIRD-PARTY-NOTICES.md").read_bytes()))


if __name__ == "__main__":
    unittest.main(verbosity=2)

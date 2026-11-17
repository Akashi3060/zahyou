# 同梱しているソフトウェアの表示(zahyou-native。ノートブック版の Rust の解法)

この文書は `rs/tools/gen_notices.py --wheel` が、`cargo metadata` と各クレートの配布物にある
ライセンスファイルから機械的に作っています。手で書き足したものではありません。
依存を変えたら作り直してください(`python rs/tools/gen_notices.py --wheel`、古いかどうかは `--wheel --check`)。
同じものが wheel(`zahyou_native-*.whl`)の `.dist-info/licenses/` に入っています。

zahyou-native は、ノートブック版(`zahyou_v9.ipynb`)のオフライン解析を Rust で速く解くための Python の拡張モジュールです。
中身は exe と同じ Rust の解法(`zahyou-solver`・`zahyou-index`・`zahyou-wcs`)で、Python 版のエンジン
(`dev/solver_*.py`)を Rust に移したものです。

## MIT ライセンスが及ぶ範囲

下の LICENSE —— MIT ライセンス —— が及ぶのは **zahyou 自身のソースコードだけ**です
(`rs/py/` と、そこから使う `rs/crates/`)。wheel に入っている第三者のソフトウェア(この文書の後半)、
インデックスファイルとその元になった星表は含みません。

```text
MIT License

Copyright (c) 2025-2026 Yoshitaka Fujikawa / AstroKIT, Kyushu Institute of Technology

Permission is hereby granted, free of charge, to any person obtaining a copy
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
SOFTWARE.
```

## オフライン解析の解き方 — astrometry.net の方式

解き方(星 4 つ(広い画角のスケールは 3 つ)の並びの幾何ハッシュ → 照合 → ベイズの検証 → 改良)は、下の論文と、
astrometry.net の公開文書と BSD-3 の部分のソースを読んでまとめた調べ物をもとに、自分で書き直したものです。
**astrometry.net のコードは含んでいません**(取り込みも移植もしていません)。

- Lang, D., Hogg, D. W., Mierle, K., Blanton, M., & Roweis, S. (2010),
  "Astrometry.net: Blind astrometric calibration of arbitrary astronomical
  images", *AJ* **139**, 1782
- <https://astrometry.net/>

## 同梱していないもの

インデックスファイルは入っていません。条件(Gaia は CC BY-NC 3.0 IGO・商用は ESA の許可・
クレジットは ESA/Gaia/DPAC)と論文・謝辞は、exe の表示(`rs/THIRD-PARTY-NOTICES.md`)とノートブック版の
説明にあります。

## 拡張モジュールに入っているもの

zahyou-native は Rust のプログラムで、下の第三者のソフトウェアを中に含んでいます。
それぞれの著作権は各プロジェクトに属し、各ライセンスの条件で配布されています。
ライセンス式が `A OR B` のものは、そのどれか 1 つの条件で使っています(本文は、配布物にあるものをすべて載せています)。

- Rust のクレート: **63 個**(`cargo metadata`。Windows・Mac・Linux のどれかで使うものを全部。ビルドのときだけ使う 2 個と、テストのときだけ使う 0 個は除く)
  - うち 17 個は proc-macro(コンパイルのときにコードを生成するもの)とその依存です。拡張モジュールにコードは入りませんが、念のため載せています
- Rust の標準ライブラリ(Apache-2.0 OR MIT。本文は下の Apache-2.0・MIT と同じ)が静的に結合されます。その中の `compiler_builtins` には LLVM の compiler-rt(Apache-2.0 WITH LLVM-exception)が入っています
- 拡張モジュールが読み込むのは、OS と Python に付いてくるライブラリだけです

| ライセンス式(Cargo.toml の値) | クレートの数 |
|---|---|
| MIT OR Apache-2.0 | 39 |
| MIT | 7 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| MIT OR Apache-2.0 OR Zlib | 2 |
| MIT OR Zlib OR Apache-2.0 | 2 |
| Unlicense OR MIT | 2 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| Apache-2.0 OR MIT | 1 |
| Apache-2.0 WITH LLVM-exception | 1 |
| MIT/Apache-2.0 | 1 |
| Zlib | 1 |
| Zlib OR Apache-2.0 OR MIT | 1 |

### クレートの一覧

「本文」の番号は、下の「ライセンス本文」の番号です。

| クレート | バージョン | ライセンス式 | 本文 | 備考 |
|---|---|---|---|---|
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | 1, 3, 8 |  |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | 1, 3 |  |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | 1, 4, 9 |  |
| `byteorder-lite` | 0.1.0 | Unlicense OR MIT | 3, 12 |  |
| `cc` | 1.11.2 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | 1, 3 |  |
| `crc32fast` | 1.5.5 | MIT OR Apache-2.0 | 1, 3 |  |
| `crossbeam-deque` | 0.8.8 | MIT OR Apache-2.0 | 1, 3 |  |
| `crossbeam-epoch` | 0.9.21 | MIT OR Apache-2.0 | 1, 3 |  |
| `crossbeam-utils` | 0.8.23 | MIT OR Apache-2.0 | 1, 3 |  |
| `crunchy` | 0.2.4 | MIT | 3 |  |
| `either` | 1.19.0 | MIT OR Apache-2.0 | 1, 3 |  |
| `fax` | 0.2.7 | MIT | 3 |  |
| `fdeflate` | 0.3.7 | MIT OR Apache-2.0 | 1, 3 |  |
| `find-msvc-tools` | 0.1.15 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `flate2` | 1.1.11 | MIT OR Apache-2.0 | 1, 3 |  |
| `half` | 2.7.1 | MIT OR Apache-2.0 | 1, 3 |  |
| `heck` | 0.5.0 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `image` | 0.25.10 | MIT OR Apache-2.0 | 1, 3 |  |
| `indoc` | 2.0.8 | MIT OR Apache-2.0 | 1, 3 | proc-macro |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 | 1, 3 |  |
| `libc` | 0.2.191 | MIT OR Apache-2.0 | 1, 5 |  |
| `memchr` | 2.8.3 | Unlicense OR MIT | 3, 12, 13 |  |
| `memmap2` | 0.9.12 | MIT OR Apache-2.0 | 1, 3 |  |
| `memoffset` | 0.9.1 | MIT | 3 |  |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 | 1, 3, 9 |  |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 | 1, 3, 9 |  |
| `moxcms` | 0.8.1 | BSD-3-Clause OR Apache-2.0 | 1, 6 |  |
| `num-traits` | 0.2.19 | MIT OR Apache-2.0 | 1, 3 |  |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 | 1, 3 |  |
| `png` | 0.18.1 | MIT OR Apache-2.0 | 1, 3 |  |
| `portable-atomic` | 1.15.0 | Apache-2.0 OR MIT | 1, 3 |  |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `pxfm` | 0.1.30 | BSD-3-Clause OR Apache-2.0 | 1, 6 |  |
| `pyo3` | 0.25.1 | MIT OR Apache-2.0 | 1, 3 |  |
| `pyo3-build-config` | 0.25.1 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `pyo3-ffi` | 0.25.1 | MIT OR Apache-2.0 | 1, 3 |  |
| `pyo3-macros` | 0.25.1 | MIT OR Apache-2.0 | 1, 3 | proc-macro |
| `pyo3-macros-backend` | 0.25.1 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `python3-dll-a` | 0.2.15 | MIT | 3 | proc-macro の依存 |
| `quick-error` | 2.0.1 | MIT/Apache-2.0 | 1, 3 |  |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `rayon` | 1.12.0 | MIT OR Apache-2.0 | 1, 3 |  |
| `rayon-core` | 1.13.0 | MIT OR Apache-2.0 | 1, 3 |  |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | 1, 3 |  |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | 1, 3 |  |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 | 1, 3 | proc-macro |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | 1, 3 |  |
| `shlex` | 2.0.1 | MIT OR Apache-2.0 | 3, 14 | proc-macro の依存 |
| `simd-adler32` | 0.3.10 | MIT | 3 |  |
| `syn` | 2.0.119 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `syn` | 3.0.10 | MIT OR Apache-2.0 | 1, 3 | proc-macro の依存 |
| `target-lexicon` | 0.13.6 | Apache-2.0 WITH LLVM-exception | 2 | proc-macro の依存 |
| `tiff` | 0.11.3 | MIT | 3 |  |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | 1, 3, 11 | proc-macro の依存 |
| `unindent` | 0.2.4 | MIT OR Apache-2.0 | 1, 3 |  |
| `weezl` | 0.1.12 | MIT OR Apache-2.0 | 1, 3 |  |
| `zerocopy` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | 1, 3, 7 |  |
| `zerocopy-derive` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | 1, 3, 7 | proc-macro |
| `zlib-rs` | 0.6.8 | Zlib | 9 |  |
| `zmij` | 1.0.23 | MIT | 3 |  |
| `zune-core` | 0.5.3 | MIT OR Apache-2.0 OR Zlib | 1, 3, 10 |  |
| `zune-jpeg` | 0.5.15 | MIT OR Apache-2.0 OR Zlib | 1, 3, 10 |  |

## ライセンス本文

同じ本文は 1 回だけ載せ、その本文に当たるクレートと、それぞれの著作権表示を並べています。
著作権表示は各クレートのライセンスファイルの中の行をそのままコピーしたものです(ファイルに著作権の行が無いものは、代わりに Cargo.toml の作者を添えています)。
Apache-2.0 の本文の末尾にある付録(使い方の例)は、標準のものそのままのときは省いています。

Apache-2.0 の 4 条 (d) の NOTICE ファイルを配布物に持つクレートはありません(あれば、その中身をそのまま下に載せます)。

### 本文 1: Apache-2.0(51 クレート)

当たるクレート(ファイル)と著作権表示:

- `adler2` 2.0.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Jonas Schievink <jonasschievink@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>)
- `bitflags` 2.13.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `bytemuck` 1.25.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Lokathor <zefria@gmail.com>)
- `cc` 1.11.2(`LICENSE-APACHE`): (著作権の行なし)
- `cfg-if` 1.0.5(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Crichton <alex@alexcrichton.com>)
- `crc32fast` 1.5.5(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Sam Rijs <srijs@airpost.net>, Alex Crichton <alex@alexcrichton.com>)
- `crossbeam-deque` 0.8.8(`LICENSE-APACHE`): (著作権の行なし)
- `crossbeam-epoch` 0.9.21(`LICENSE-APACHE`): (著作権の行なし)
- `crossbeam-utils` 0.8.23(`LICENSE-APACHE`): (著作権の行なし)
- `either` 1.19.0(`LICENSE-APACHE`): (著作権の行なし)
- `fdeflate` 0.3.7(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `find-msvc-tools` 0.1.15(`LICENSE-APACHE`): (著作権の行なし)
- `flate2` 1.1.11(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Crichton <alex@alexcrichton.com>, Josh Triplett <josh@joshtriplett.org>)
- `half` 2.7.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Kathryn Long <squeeself@gmail.com>)
- `heck` 0.5.0(`LICENSE-APACHE`): (著作権の行なし)
- `image` 0.25.10(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `indoc` 2.0.8(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `itoa` 1.0.19(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `libc` 0.2.191(`LICENSE-APACHE`): (著作権の行なし)
- `memmap2` 0.9.12(`LICENSE-APACHE`): Copyright [2015] [Dan Burkert]
- `miniz_oxide` 0.8.9(`LICENSE-APACHE.md`): (ファイルに著作権の行なし。Cargo.toml の作者: Frommi <daniil.liferenko@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>, Rich Geldreich richgel99@gmail.com)
- `miniz_oxide` 0.9.1(`LICENSE-APACHE.md`): (ファイルに著作権の行なし。Cargo.toml の作者: Frommi <daniil.liferenko@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>, Rich Geldreich richgel99@gmail.com)
- `moxcms` 0.8.1(`LICENSE-APACHE.md`): Copyright 2024 Radzivon Bartoshyk
- `num-traits` 0.2.19(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `once_cell` 1.21.4(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Aleksey Kladov <aleksey.kladov@gmail.com>)
- `png` 0.18.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `portable-atomic` 1.15.0(`LICENSE-APACHE`): (著作権の行なし)
- `proc-macro2` 1.0.107(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>, Alex Crichton <alex@alexcrichton.com>)
- `pxfm` 0.1.30(`LICENSE-APACHE.md`): Copyright 2024 Radzivon Bartoshyk
- `pyo3` 0.25.1(`LICENSE-APACHE`): Copyright (c) 2017-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3` 0.25.1(`pyo3-runtime/LICENSE-APACHE`): Copyright (c) 2017-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-build-config` 0.25.1(`LICENSE-APACHE`): Copyright (c) 2017-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-ffi` 0.25.1(`LICENSE-APACHE`): Copyright (c) 2017-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-macros` 0.25.1(`LICENSE-APACHE`): Copyright (c) 2017-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-macros-backend` 0.25.1(`LICENSE-APACHE`): Copyright (c) 2017-present PyO3 Project and Contributors.  https://github.com/PyO3
- `quick-error` 2.0.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Paul Colomiets <paul@colomiets.name>, Colin Kiegel <kiegel@gmx.de>)
- `quote` 1.0.47(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `rayon` 1.12.0(`LICENSE-APACHE`): (著作権の行なし)
- `rayon-core` 1.13.0(`LICENSE-APACHE`): (著作権の行なし)
- `serde` 1.0.229(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_core` 1.0.229(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_derive` 1.0.229(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_json` 1.0.154(`LICENSE-APACHE`): (著作権の行なし)
- `syn` 2.0.119(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `syn` 3.0.10(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `unicode-ident` 1.0.27(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `unindent` 0.2.4(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `weezl` 0.1.12(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `zerocopy` 0.8.63(`LICENSE-APACHE`): Copyright 2023 The Fuchsia Authors
- `zerocopy-derive` 0.8.63(`LICENSE-APACHE`): Copyright 2023 The Fuchsia Authors
- `zune-core` 0.5.3(`LICENSE-APACHE`): (著作権の行なし)
- `zune-jpeg` 0.5.15(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: caleb <etemesicaleb@gmail.com>)

```text
                              Apache License
                        Version 2.0, January 2004
                     https://www.apache.org/licenses/LICENSE-2.0

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS
```

### 本文 2: Apache-2.0(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `target-lexicon` 0.13.6(`LICENSE`)

```text
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.

--- LLVM Exceptions to the Apache 2.0 License ----

As an exception, if, as a result of your compiling your source code, portions
of this Software are embedded into an Object form of such source code, you
may redistribute such embedded portions in such Object form without complying
with the conditions of Sections 4(a), 4(b) and 4(d) of the License.

In addition, if you combine or link compiled forms of this Software with
software that is licensed under the GPLv2 ("Combined Software") and if a
court of competent jurisdiction determines that the patent provision (Section
3), the indemnity provision (Section 9) or other Section of the License
conflicts with the conditions of the GPLv2, you may retroactively and
prospectively choose to deem waived or otherwise exclude such Section(s) of
the License, but only in their entirety and only with respect to the Combined
Software.
```

### 本文 3: MIT(57 クレート)

当たるクレート(ファイル)と著作権表示:

- `adler2` 2.0.1(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Jonas Schievink <jonasschievink@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>)
- `bitflags` 2.13.2(`LICENSE-MIT`): Copyright (c) 2014 The Rust Project Developers
- `byteorder-lite` 0.1.0(`LICENSE-MIT`): Copyright (c) 2015 Andrew Gallant
- `cc` 1.11.2(`LICENSE-MIT`): Copyright (c) 2014 Alex Crichton
- `cfg-if` 1.0.5(`LICENSE-MIT`): Copyright (c) 2014 Alex Crichton
- `crc32fast` 1.5.5(`LICENSE-MIT`): Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors
- `crossbeam-deque` 0.8.8(`LICENSE-MIT`): Copyright (c) 2019 The Crossbeam Project Developers
- `crossbeam-epoch` 0.9.21(`LICENSE-MIT`): Copyright (c) 2019 The Crossbeam Project Developers
- `crossbeam-utils` 0.8.23(`LICENSE-MIT`): Copyright (c) 2019 The Crossbeam Project Developers
- `crunchy` 0.2.4(`LICENSE`): Copyright 2017-2023 Eira Fransham.
- `either` 1.19.0(`LICENSE-MIT`): Copyright (c) 2015
- `fax` 0.2.7(`LICENSE`): Copyright © 2021 The pdf-rs contributers.
- `fdeflate` 0.3.7(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `find-msvc-tools` 0.1.15(`LICENSE-MIT`): Copyright (c) 2014 Alex Crichton
- `flate2` 1.1.11(`LICENSE-MIT`): Copyright (c) 2014-2026 Alex Crichton
- `half` 2.7.1(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Kathryn Long <squeeself@gmail.com>)
- `heck` 0.5.0(`LICENSE-MIT`): Copyright (c) 2015 The Rust Project Developers
- `image` 0.25.10(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `indoc` 2.0.8(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `itoa` 1.0.19(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `memchr` 2.8.3(`LICENSE-MIT`): Copyright (c) 2015 Andrew Gallant
- `memmap2` 0.9.12(`LICENSE-MIT`): Copyright (c) 2020 Yevhenii Reizner / Copyright (c) 2015 Dan Burkert
- `memoffset` 0.9.1(`LICENSE`): Copyright (c) 2017 Gilad Naaman
- `miniz_oxide` 0.8.9(`LICENSE`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.8.9(`LICENSE-MIT.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.9.1(`LICENSE`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.9.1(`LICENSE-MIT.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `num-traits` 0.2.19(`LICENSE-MIT`): Copyright (c) 2014 The Rust Project Developers
- `once_cell` 1.21.4(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Aleksey Kladov <aleksey.kladov@gmail.com>)
- `png` 0.18.1(`LICENSE-MIT`): Copyright (c) 2015 nwin
- `portable-atomic` 1.15.0(`LICENSE-MIT`): (著作権の行なし)
- `proc-macro2` 1.0.107(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>, Alex Crichton <alex@alexcrichton.com>)
- `pyo3` 0.25.1(`LICENSE-MIT`): Copyright (c) 2023-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3` 0.25.1(`pyo3-runtime/LICENSE-MIT`): Copyright (c) 2023-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-build-config` 0.25.1(`LICENSE-MIT`): Copyright (c) 2023-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-ffi` 0.25.1(`LICENSE-MIT`): Copyright (c) 2023-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-macros` 0.25.1(`LICENSE-MIT`): Copyright (c) 2023-present PyO3 Project and Contributors.  https://github.com/PyO3
- `pyo3-macros-backend` 0.25.1(`LICENSE-MIT`): Copyright (c) 2023-present PyO3 Project and Contributors.  https://github.com/PyO3
- `python3-dll-a` 0.2.15(`LICENSE`): Copyright (c) 2022 Sergey Kvachonok, Adam Reichold, messense <messense@icloud.com>
- `quick-error` 2.0.1(`LICENSE-MIT`): Copyright (c) 2015 The quick-error Developers
- `quote` 1.0.47(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `rayon` 1.12.0(`LICENSE-MIT`): Copyright (c) 2010 The Rust Project Developers
- `rayon-core` 1.13.0(`LICENSE-MIT`): Copyright (c) 2010 The Rust Project Developers
- `serde` 1.0.229(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_core` 1.0.229(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_derive` 1.0.229(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_json` 1.0.154(`LICENSE-MIT`): (著作権の行なし)
- `shlex` 2.0.1(`LICENSE-MIT`): Copyright (c) 2015 Nicholas Allegra (comex).
- `simd-adler32` 0.3.10(`LICENSE.md`): Copyright (c) [2021] [Marvin Countryman]
- `syn` 2.0.119(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `syn` 3.0.10(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `tiff` 0.11.3(`LICENSE`): Copyright (c) 2018 PistonDevelopers
- `unicode-ident` 1.0.27(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `unindent` 0.2.4(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `weezl` 0.1.12(`LICENSE-MIT`): Copyright (c) HeroicKatora 2020
- `zerocopy` 0.8.63(`LICENSE-MIT`): Copyright 2023 The Fuchsia Authors
- `zerocopy-derive` 0.8.63(`LICENSE-MIT`): Copyright 2023 The Fuchsia Authors
- `zmij` 1.0.23(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `zune-core` 0.5.3(`LICENSE-MIT`): Copyright (c) zune-image developers
- `zune-jpeg` 0.5.15(`LICENSE-MIT`): Copyright (c) zune-image developers

```text
Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### 本文 4: MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `bytemuck` 1.25.2(`LICENSE-MIT`)

```text
MIT License

Copyright (c) 2019 Daniel "Lokathor" Gee.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice (including the next paragraph) shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

### 本文 5: MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `libc` 0.2.191(`LICENSE-MIT`)

```text
Copyright (c) The Rust Project Developers

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### 本文 6: BSD-3-Clause(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `moxcms` 0.8.1(`LICENSE.md`): Copyright (c) Radzivon Bartoshyk. All rights reserved.
- `pxfm` 0.1.30(`LICENSE.md`): Copyright (c) Radzivon Bartoshyk. All rights reserved.

```text
Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

1.  Redistributions of source code must retain the above copyright notice, this
    list of conditions and the following disclaimer.

2.  Redistributions in binary form must reproduce the above copyright notice,
    this list of conditions and the following disclaimer in the documentation
    and/or other materials provided with the distribution.

3.  Neither the name of the copyright holder nor the names of its
    contributors may be used to endorse or promote products derived from
    this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### 本文 7: BSD-2-Clause(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `zerocopy` 0.8.63(`LICENSE-BSD`): Copyright 2019 The Fuchsia Authors.
- `zerocopy-derive` 0.8.63(`LICENSE-BSD`): Copyright 2019 The Fuchsia Authors.

```text
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### 本文 8: 0BSD(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `adler2` 2.0.1(`LICENSE-0BSD`)

```text
Copyright (C) Jonas Schievink <jonasschievink@gmail.com>

Permission to use, copy, modify, and/or distribute this software for
any purpose with or without fee is hereby granted.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN
AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT
OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

### 本文 9: Zlib(4 クレート)

当たるクレート(ファイル)と著作権表示:

- `bytemuck` 1.25.2(`LICENSE-ZLIB`): Copyright (c) 2019 Daniel "Lokathor" Gee.
- `miniz_oxide` 0.8.9(`LICENSE-ZLIB.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2020 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.9.1(`LICENSE-ZLIB.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2020 Frommi / Copyright (c) 2017-2024 oyvindln
- `zlib-rs` 0.6.8(`LICENSE`): (C) 2024 Trifecta Tech Foundation

```text
This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

### 本文 10: Zlib(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `zune-core` 0.5.3(`LICENSE-ZLIB`): (著作権の行なし)
- `zune-jpeg` 0.5.15(`LICENSE-ZLIB`): (ファイルに著作権の行なし。Cargo.toml の作者: caleb <etemesicaleb@gmail.com>)

```text
(C) zune-image developers

This software is provided 'as-is', without any express or implied
warranty. In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution.
```

### 本文 11: Unicode-3.0(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `unicode-ident` 1.0.27(`LICENSE-UNICODE`)

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
```

### 本文 12: Unlicense(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `byteorder-lite` 0.1.0(`UNLICENSE`): (著作権の行なし)
- `memchr` 2.8.3(`UNLICENSE`): (ファイルに著作権の行なし。Cargo.toml の作者: Andrew Gallant <jamslam@gmail.com>, bluss)

```text
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or
distribute this software, either in source code form or as a compiled
binary, for any purpose, commercial or non-commercial, and by any
means.

In jurisdictions that recognize copyright laws, the author or authors
of this software dedicate any and all copyright interest in the
software to the public domain. We make this dedication for the benefit
of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of
relinquishment in perpetuity of all present and future rights to this
software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <http://unlicense.org/>
```

### 本文 13: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `memchr` 2.8.3(`COPYING`)

```text
This project is dual-licensed under the Unlicense and MIT licenses.

You may use this code under the terms of either license.
```

### 本文 14: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `shlex` 2.0.1(`LICENSE-APACHE`)

```text
Copyright 2015 Nicholas Allegra (comex).

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

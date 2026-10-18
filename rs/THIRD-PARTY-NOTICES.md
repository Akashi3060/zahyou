# 同梱しているソフトウェアの表示(Rust 版 zahyou.exe)

この文書は `rs/tools/gen_notices.py` が、`cargo metadata` と各クレートの配布物にある
ライセンスファイルから機械的に作っています。手で書き足したものではありません。
依存を変えたら作り直してください(`python rs/tools/gen_notices.py`、古いかどうかは `--check`)。
同じものが `zahyou.exe` の中に入っていて、［使い方］タブの「同梱しているソフトウェアの表示」で読めます。

## MIT ライセンスが及ぶ範囲

下の LICENSE —— MIT ライセンス —— が及ぶのは **zahyou 自身のソースコードだけ**です。

    rs/app/ / rs/crates/ / rs/tools/ / rs/testdata/(Rust 版)
    zahyou_engine.py / zahyou_v7.ipynb / dev/ / docs/(Python 版)

オフライン解析のエンジン(`zahyou-solver`・`zahyou-index`・`zahyou-wcs`)も zahyou 自身のコードです。
`zahyou.exe` に入っている第三者のソフトウェア(この文書の後半)、星図データ
(index ファイル)とその元になった星表は含みません。

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

- Lang, D., Hogg, D. W., Mierle, K., Blanton, M., & Roweis, S. (2010),
  "Astrometry.net: Blind astrometric calibration of arbitrary astronomical
  images", *AJ* **139**, 1782
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

- Høg, E., et al. (2000), "The Tycho-2 Catalogue of the 2.5 million brightest
  stars", *A&A* **355**, L27
- Gaia Collaboration, Prusti, T., et al. (2016), "The Gaia mission",
  *A&A* **595**, A1
- Gaia Collaboration, Brown, A. G. A., et al. (2018), "Gaia Data Release 2.
  Summary of the contents and survey properties", *A&A* **616**, A1

> This work has made use of data from the European Space Agency (ESA) mission
> Gaia (https://www.cosmos.esa.int/gaia), processed by the Gaia Data Processing
> and Analysis Consortium (DPAC,
> https://www.cosmos.esa.int/web/gaia/dpac/consortium). Funding for the DPAC
> has been provided by national institutions, in particular the institutions
> participating in the Gaia Multilateral Agreement.

**配布ページ(Nextcloud)に置いてある `AstrometryData*.zip` は、この
index ファイルをそのまま詰め直したものです。** 中身は上記の原典と同一で、
回線の細い場所でまとめて取れるようにしてあるだけです。
元データの権利は Gaia(ESA/Gaia/DPAC)および Tycho-2 の各制作者にあり、
上の条件(Gaia は非商用・クレジット)はこの zip にもそのままかかります。

### 天体名から座標を引くとき

`zahyou-net` が CDS(ストラスブール天文台)の Sesame(SIMBAD など)と VizieR に問い合わせます。
オンライン解析では Astrometry.net の Web サービス(nova.astrometry.net)も使います。
いずれも実行時の問い合わせで、データを同梱してはいません。

> This research has made use of the SIMBAD database and the VizieR catalogue
> access tool, operated at CDS, Strasbourg, France.

### 画面の日本語の書体

画面の日本語は、Windows に入っている游ゴシック UI(無ければメイリオなど)を実行時に読み込んで
表示します。等幅の欄は、HackGen が入っていればそれを、無ければ Consolas を使います。
**これらの書体は exe に入っていません**(下の「画面の既定の書体」は別のものです)。

## 実行ファイルに入っているもの

`zahyou.exe` は Rust のプログラムで、下の第三者のソフトウェアを中に含んでいます。
それぞれの著作権は各プロジェクトに属し、各ライセンスの条件で配布されています。
ライセンス式が `A OR B` のものは、そのどれか 1 つの条件で使っています(本文は、配布物にあるものをすべて載せています)。

- Rust のクレート: **213 個**(`cargo metadata`、ターゲット `x86_64-pc-windows-gnu`。ビルドのときだけ使う 18 個と、試験のときだけ使う 1 個は除く)
  - うち 21 個は proc-macro(コンパイルのときにコードを生成するもの)とその依存です。exe にコードは入りませんが、念のため載せています
- egui の既定の書体 4 つ(exe に埋め込まれる。下の「画面の既定の書体」)
- リンクのときに結合される実行時ライブラリ(下の「Rust の標準ライブラリと実行時ライブラリ」)
- exe が読み込む DLL は Windows のシステムのものだけです(UCRT を含む)

| ライセンス式(Cargo.toml の値) | クレートの数 |
|---|---|
| MIT OR Apache-2.0 | 120 |
| Unicode-3.0 | 18 |
| MIT | 16 |
| Apache-2.0 OR MIT | 10 |
| Apache-2.0 | 9 |
| MIT OR Apache-2.0 OR Zlib | 5 |
| BSD-3-Clause | 3 |
| ISC | 3 |
| MIT/Apache-2.0 | 3 |
| Zlib | 3 |
| Apache-2.0/MIT | 2 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| BSL-1.0 | 2 |
| MIT OR Zlib OR Apache-2.0 | 2 |
| Unlicense OR MIT | 2 |
| Zlib OR Apache-2.0 OR MIT | 2 |
| (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | 1 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| Apache-2.0 AND ISC | 1 |
| Apache-2.0 AND MIT | 1 |
| Apache-2.0 OR ISC OR MIT | 1 |
| BSD-2-Clause | 1 |
| CC0-1.0 | 1 |
| CDLA-Permissive-2.0 | 1 |

**GPL・LGPL・AGPL・MPL のクレートは入っていません**(`rs/tools/check_licenses.py` が、許可した
ライセンスだけで満たせない式を見つけると失敗します)。

### 画面の既定の書体(exe に埋め込まれるもの)

egui の既定の書体は `epaint_default_fonts` 0.33.3 が `include_bytes!` で exe に埋め込みます(eframe の機能 `default_fonts`)。書体ファイルの中の著作権表示(name 表の ID 0)と、
一緒に配られているライセンスのファイルは次のとおりです。本文は下の「ライセンス本文」にあります。

| 書体ファイル | 名前 | 著作権表示 | ライセンス | 本文 |
|---|---|---|---|---|
| `Hack-Regular.ttf` | Hack | Copyright (c) 2018 Source Foundry Authors / Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved.(書体の中の表記) | MIT AND Bitstream-Vera | 本文 12(`fonts/Hack-Regular.txt`) |
| `Ubuntu-Light.ttf` | Ubuntu Light | Copyright 2011 Canonical Ltd.  Licensed under the Ubuntu Font Licence 1.0(書体の中の表記) | Ubuntu-font-1.0 | 本文 29(`fonts/UFL.txt`) |
| `NotoEmoji-Regular.ttf` | Noto Emoji | Copyright 2013 Google Inc. All Rights Reserved.(書体の中の表記) | OFL-1.1 | 本文 28(`fonts/OFL.txt`) |
| `emoji-icon-font.ttf` | emoji | Copyright (c) 2014 John Slegers(ライセンスのファイルの表記。書体の中には無い) | MIT | 本文 5(`fonts/emoji-icon-font-mit-license.txt`) |

Hack は Bitstream Vera と DejaVu をもとにした書体で、Hack の部分が MIT、元の Bitstream Vera の部分が
Bitstream Vera License です。Noto Emoji は SIL Open Font License 1.1、Ubuntu は Ubuntu Font Licence 1.0 です。
どれも、書体を変えずにソフトウェアに組み込んで配ることを認めています(書体だけを単体で売ることは認めていません)。
`epaint_default_fonts` のコード部分のライセンス(MIT OR Apache-2.0)の本文は配布物に入っていないので、
標準の本文によります(下の一覧の備考)。

### C とアセンブリを含むもの

`ring` 0.17.14(TLS の暗号。`ureq` → `rustls` が HTTPS の通信で使う)は、ビルドのときに
**配布物に入っている C のソースを C コンパイラー(MSYS2 UCRT64 の gcc)で組み立て**、
アセンブリの部分は**配布物に入っている組み立て済みの目的ファイル**(BoringSSL 由来の perlasm を
NASM 形式で組み立てたもの。`pregenerated/*-nasm.o`)を使って、静的ライブラリ `ring_core_0_17_14_` にまとめ、exe に結合しています。
これは ring 自身のコード(BoringSSL・fiat-crypto 由来の部分を含む)を Rust から呼んでいるもので、
**システムの C ライブラリ(OpenSSL など)への FFI ではありません**。exe が読み込む DLL は
Windows のシステムのものだけです。ライセンスは Apache-2.0 AND ISC(下の一覧の `ring` の行。`LICENSE`・`LICENSE-BoringSSL`・`LICENSE-other-bits`・
`third_party/fiat/LICENSE`・`src/polyfill/once_cell/LICENSE-*`)。

### Rust の標準ライブラリと実行時ライブラリ

`x86_64-pc-windows-gnu` 向けにリンクすると、次のものが exe に静的に結合されます(`rustc --print link-args` で確かめたもの)。
コンパイラーは rustc 1.94.0 (4a4ef493e 2026-03-02) (Rev2, Built by MSYS2 project)、リンカーは MSYS2 UCRT64 の gcc 15.2.0。

| 結合されるもの | ライセンス式 | 表示 |
|---|---|---|
| Rust の標準ライブラリ(std・core・alloc とその依存) | (MIT OR Apache-2.0) AND Unicode-3.0 AND BSD-2-Clause | 下の 実行時-1(`share/doc/rustc/COPYRIGHT-library.html`) |
| compiler-rt(標準ライブラリの compiler_builtins に入っている C) | Apache-2.0 WITH LLVM-exception | 下の 実行時-2(`share/doc/rustc/licenses/LLVM-exception.txt`) |
| MinGW-w64 の実行時ライブラリ(crt2.o・libmingw32.a・libmingwex.a) | ZPL-2.1 AND ISC AND BSD-2-Clause AND LicenseRef-Lucent-gdtoa AND SunPro AND MIT | 下の 実行時-3(`share/licenses/crt/COPYING.MinGW-w64-runtime.txt`) |
| winpthreads(libpthread.a) | MIT AND BSD-3-Clause | 下の 実行時-4(`share/licenses/winpthreads/COPYING`) |
| libgcc.a・libgcc_eh.a(GCC の実行時ライブラリ。例外の巻き戻しなど) | GPL-3.0-or-later WITH GCC-exception-3.1 | 下の 実行時-5(`share/licenses/gcc-libs/COPYING.RUNTIME`) |

- 標準ライブラリの rlib: `addr2line`・`adler2`・`alloc`・`cfg_if`・`compiler_builtins`・`core`・`gimli`・`hashbrown`・`libc`・`memchr`・`miniz_oxide`・`object`・`panic_unwind`・`rustc_demangle`・`rustc_std_workspace_alloc`・`rustc_std_workspace_core`・`std`・`std_detect`・`unwind`・`windows_targets`。
  Rust の配布物の表示(実行時-1)のとおり、標準ライブラリは Apache-2.0 OR MIT です(本文は下の「ライセンス本文」の Apache-2.0・MIT と同じ)。
- `compiler_builtins` には LLVM の compiler-rt の C が入っています。LLVM の例外(実行時-2)により、
  組み込まれた目的コードについては Apache-2.0 の表示の条件(4 条 (a)(b)(d))がかかりません。
- libgcc と libgcc_eh は GPL-3.0-or-later に GCC 実行時ライブラリ例外 3.1 が付いたものです(実行時-5)。
  この例外のもとでは、これらを結合した exe を任意の条件で配れます(zahyou は MIT)。
- MinGW-w64 の表示(実行時-3)の最後の節にある Wine 由来のヘッダー(LGPL)は、C をコンパイルするときの
  宣言だけで、exe にコードは入らないと考えています(推測。結合されるライブラリの中身までは確かめていません)。

### クレートの一覧

「本文」の番号は、下の「ライセンス本文」の番号です。

| クレート | 版 | ライセンス式 | 本文 | 備考 |
|---|---|---|---|---|
| `ab_glyph` | 0.2.32 | Apache-2.0 | 1 |  |
| `ab_glyph_rasterizer` | 0.1.10 | Apache-2.0 | 1 |  |
| `accesskit` | 0.21.1 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): The AccessKit contributors。ソース: <https://github.com/AccessKit/accesskit> |
| `accesskit_consumer` | 0.31.0 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): The AccessKit contributors。ソース: <https://github.com/AccessKit/accesskit> |
| `accesskit_windows` | 0.29.2 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): The AccessKit contributors。ソース: <https://github.com/AccessKit/accesskit> |
| `accesskit_winit` | 0.29.2 | Apache-2.0 | — | 配布物に本文が無いので標準の本文による(Apache-2.0 は本文 1)。作者(Cargo.toml): The AccessKit contributors。ソース: <https://github.com/AccessKit/accesskit> |
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | 1, 5, 20 |  |
| `ahash` | 0.8.12 | MIT OR Apache-2.0 | 1, 5 |  |
| `arboard` | 3.6.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `arrayref` | 0.3.9 | BSD-2-Clause | 16 |  |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 | 1, 5 |  |
| `base64` | 0.23.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `bit-set` | 0.8.0 | Apache-2.0 OR MIT | 1, 5 |  |
| `bit-vec` | 0.8.0 | Apache-2.0 OR MIT | 1, 5 |  |
| `bitflags` | 1.3.2 | MIT/Apache-2.0 | 1, 5 |  |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | 1, 5 |  |
| `block-buffer` | 0.10.4 | MIT OR Apache-2.0 | 1, 5 |  |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | 1, 7, 21 |  |
| `bytemuck_derive` | 1.12.1 | Zlib OR Apache-2.0 OR MIT | 1, 7, 21 | proc-macro |
| `byteorder-lite` | 0.1.0 | Unlicense OR MIT | 5, 26 |  |
| `bytes` | 1.12.1 | MIT | 5 |  |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | 1, 5 |  |
| `clipboard-win` | 5.4.1 | BSL-1.0 | — | 配布物に本文が無いので標準の本文による(BSL-1.0 は本文 25)。作者(Cargo.toml): Douman <douman@gmx.se>。ソース: <https://github.com/DoumanAsh/clipboard-win> |
| `codespan-reporting` | 0.12.0 | Apache-2.0 | 1 |  |
| `cpufeatures` | 0.2.17 | MIT OR Apache-2.0 | 1, 5 |  |
| `crc32fast` | 1.5.2 | MIT OR Apache-2.0 | 1, 5 |  |
| `crossbeam-deque` | 0.8.8 | MIT OR Apache-2.0 | 1, 5 |  |
| `crossbeam-epoch` | 0.9.21 | MIT OR Apache-2.0 | 1, 5 |  |
| `crossbeam-utils` | 0.8.23 | MIT OR Apache-2.0 | 1, 5 |  |
| `crypto-common` | 0.1.7 | MIT OR Apache-2.0 | 1, 5 |  |
| `cursor-icon` | 1.2.0 | MIT OR Apache-2.0 OR Zlib | 1, 5, 21 |  |
| `digest` | 0.10.7 | MIT OR Apache-2.0 | 1, 5 |  |
| `displaydoc` | 0.2.7 | MIT OR Apache-2.0 | 1, 5 | proc-macro |
| `document-features` | 0.2.12 | MIT OR Apache-2.0 | 1, 5 | proc-macro |
| `dpi` | 0.1.2 | Apache-2.0 AND MIT | 1, 8 |  |
| `ecolor` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>, Andreas Reich <reichandreas@gmx.de>。ソース: <https://github.com/emilk/egui> |
| `eframe` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/eframe> |
| `egui` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui> |
| `egui-wgpu` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Nils Hasenbanck <nils@hasenbanck.de>, embotech <opensource@embotech.com>, Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/egui-wgpu> |
| `egui-winit` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/egui-winit> |
| `egui_glow` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/egui_glow> |
| `either` | 1.19.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `emath` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/emath> |
| `epaint` | 0.33.3 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/epaint> |
| `epaint_default_fonts` | 0.33.3 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | 5, 12, 28, 29 | 書体を埋め込む(上の説明)。配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Emil Ernerfeldt <emil.ernerfeldt@gmail.com>。ソース: <https://github.com/emilk/egui/tree/main/crates/epaint_default_fonts> |
| `equivalent` | 1.0.3 | Apache-2.0 OR MIT | 1, 5 |  |
| `error-code` | 3.4.0 | BSL-1.0 | 25 |  |
| `fax` | 0.2.7 | MIT | 5 |  |
| `fdeflate` | 0.3.7 | MIT OR Apache-2.0 | 1, 5 |  |
| `flate2` | 1.1.10 | MIT OR Apache-2.0 | 1, 5 |  |
| `foldhash` | 0.1.5 | Zlib | 21 |  |
| `foldhash` | 0.2.0 | Zlib | 21 |  |
| `form_urlencoded` | 1.2.2 | MIT OR Apache-2.0 | 1, 5 |  |
| `fs4` | 0.13.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `generic-array` | 0.14.7 | MIT | 5 |  |
| `getrandom` | 0.2.17 | MIT OR Apache-2.0 | 1, 5 |  |
| `getrandom` | 0.3.4 | MIT OR Apache-2.0 | 1, 5 |  |
| `glow` | 0.16.0 | MIT OR Apache-2.0 OR Zlib | 1, 5, 21 |  |
| `glutin` | 0.32.3 | Apache-2.0 | 1 |  |
| `glutin-winit` | 0.5.0 | MIT | 5 |  |
| `glutin_egl_sys` | 0.7.1 | Apache-2.0 | 1 |  |
| `glutin_wgl_sys` | 0.6.1 | Apache-2.0 | 1 |  |
| `gpu-allocator` | 0.27.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `half` | 2.7.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `hashbrown` | 0.15.5 | MIT OR Apache-2.0 | 1, 5 |  |
| `hashbrown` | 0.16.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `hashbrown` | 0.17.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `hexf-parse` | 0.2.1 | CC0-1.0 | — | CC0 1.0 Universal(パブリック ドメインへの献呈。表示の義務はありません。本文: <https://creativecommons.org/publicdomain/zero/1.0/legalcode>)。作者(Cargo.toml): Kang Seonghoon <public+rust@mearie.org>。ソース: <https://github.com/lifthrasiir/hexf> |
| `http` | 1.5.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `httparse` | 1.10.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `icu_collections` | 2.3.0 | Unicode-3.0 | 23 |  |
| `icu_locale_core` | 2.3.0 | Unicode-3.0 | 23 |  |
| `icu_normalizer` | 2.3.0 | Unicode-3.0 | 23 |  |
| `icu_normalizer_data` | 2.3.0 | Unicode-3.0 | 23 |  |
| `icu_properties` | 2.3.0 | Unicode-3.0 | 23 |  |
| `icu_properties_data` | 2.3.0 | Unicode-3.0 | 23 |  |
| `icu_provider` | 2.3.1 | Unicode-3.0 | 23 |  |
| `idna` | 1.1.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `idna_adapter` | 1.2.2 | Apache-2.0 OR MIT | 1, 9 |  |
| `image` | 0.25.10 | MIT OR Apache-2.0 | 1, 5 |  |
| `indexmap` | 2.14.2 | Apache-2.0 OR MIT | 1, 5 |  |
| `itoa` | 1.0.18 | MIT OR Apache-2.0 | 1, 5 |  |
| `libloading` | 0.8.9 | ISC | 17 |  |
| `libm` | 0.2.16 | MIT | 3 |  |
| `litemap` | 0.8.3 | Unicode-3.0 | 23 |  |
| `litrs` | 1.0.1 | MIT OR Apache-2.0 | 1, 5 | proc-macro の依存 |
| `lock_api` | 0.4.14 | MIT OR Apache-2.0 | 1, 5 |  |
| `log` | 0.4.34 | MIT OR Apache-2.0 | 1, 5 |  |
| `memchr` | 2.8.3 | Unlicense OR MIT | 5, 26, 31 |  |
| `memmap2` | 0.9.11 | MIT OR Apache-2.0 | 1, 5 |  |
| `memoffset` | 0.9.1 | MIT | 5 |  |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 | 1, 5, 21 |  |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 | 1, 5, 21 |  |
| `moxcms` | 0.8.1 | BSD-3-Clause OR Apache-2.0 | 1, 13 |  |
| `naga` | 27.0.3 | MIT OR Apache-2.0 | 1, 5 |  |
| `nohash-hasher` | 0.2.0 | Apache-2.0 OR MIT | 1, 5 |  |
| `num-traits` | 0.2.19 | MIT OR Apache-2.0 | 1, 5 |  |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 | 1, 5 |  |
| `ordered-float` | 5.4.0 | MIT | 5 |  |
| `owned_ttf_parser` | 0.25.1 | Apache-2.0 | 1 |  |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 | 1, 5 |  |
| `parking_lot_core` | 0.9.12 | MIT OR Apache-2.0 | 1, 5 |  |
| `percent-encoding` | 2.3.2 | MIT OR Apache-2.0 | 1, 5 |  |
| `pin-project-lite` | 0.2.17 | Apache-2.0 OR MIT | 1, 5 |  |
| `png` | 0.17.16 | MIT OR Apache-2.0 | 1, 5 |  |
| `png` | 0.18.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `pollster` | 0.4.0 | Apache-2.0/MIT | 1, 5 |  |
| `potential_utf` | 0.1.6 | Unicode-3.0 | 23 |  |
| `presser` | 0.3.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | 1, 5 | proc-macro の依存 |
| `profiling` | 1.0.18 | MIT OR Apache-2.0 | — | 配布物に本文が無いので標準の本文による(MIT は本文 5、Apache-2.0 は本文 1)。作者(Cargo.toml): Philip Degarmo <aclysma@gmail.com>。ソース: <https://github.com/aclysma/profiling> |
| `pxfm` | 0.1.30 | BSD-3-Clause OR Apache-2.0 | 1, 13 |  |
| `quick-error` | 2.0.1 | MIT/Apache-2.0 | 1, 5 |  |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | 1, 5 | proc-macro の依存 |
| `range-alloc` | 0.1.5 | MIT OR Apache-2.0 | 1, 5 |  |
| `raw-window-handle` | 0.6.2 | MIT OR Apache-2.0 OR Zlib | 1, 5, 21 |  |
| `rayon` | 1.12.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `rayon-core` | 1.13.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `renderdoc-sys` | 1.1.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `rfd` | 0.15.4 | MIT | 5 |  |
| `ring` | 0.17.14 | Apache-2.0 AND ISC | 1, 4, 10, 17, 32, 33 | C とアセンブリ(上の説明) |
| `rustc-hash` | 1.1.0 | Apache-2.0/MIT | 1, 5 |  |
| `rustc-hash` | 2.1.3 | Apache-2.0 OR MIT | 1, 5 |  |
| `rustls` | 0.23.46 | Apache-2.0 OR ISC OR MIT | 1, 5, 17 |  |
| `rustls-pki-types` | 1.15.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `rustls-webpki` | 0.103.15 | ISC | 18 |  |
| `scopeguard` | 1.2.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | 1, 5 |  |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | 1, 5 |  |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 | 1, 5 | proc-macro |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | 1, 5 |  |
| `sha2` | 0.10.9 | MIT OR Apache-2.0 | 1, 5 |  |
| `simd-adler32` | 0.3.10 | MIT | 5 |  |
| `smallvec` | 1.16.3 | MIT OR Apache-2.0 | 1, 5 |  |
| `smol_str` | 0.2.2 | MIT OR Apache-2.0 | 1, 5 |  |
| `stable_deref_trait` | 1.2.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `static_assertions` | 1.1.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `strict-num` | 0.1.1 | MIT | 5 |  |
| `subtle` | 2.6.1 | BSD-3-Clause | 13 |  |
| `syn` | 2.0.119 | MIT OR Apache-2.0 | 1, 5 | proc-macro の依存 |
| `syn` | 3.0.9 | MIT OR Apache-2.0 | 1, 5 | proc-macro の依存 |
| `synstructure` | 0.14.0 | MIT | 5 | proc-macro の依存 |
| `thiserror` | 1.0.69 | MIT OR Apache-2.0 | 1, 5 |  |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | 1, 5 |  |
| `thiserror-impl` | 1.0.69 | MIT OR Apache-2.0 | 1, 5 | proc-macro |
| `thiserror-impl` | 2.0.21 | MIT OR Apache-2.0 | 1, 5 | proc-macro |
| `tiff` | 0.11.3 | MIT | 5 |  |
| `tiny-skia` | 0.11.4 | BSD-3-Clause | 14 |  |
| `tiny-skia-path` | 0.11.4 | BSD-3-Clause | 14 |  |
| `tinystr` | 0.8.4 | Unicode-3.0 | 23 |  |
| `tracing` | 0.1.44 | MIT | 5 |  |
| `tracing-attributes` | 0.1.31 | MIT | 5 | proc-macro |
| `tracing-core` | 0.1.36 | MIT | 5 |  |
| `ttf-parser` | 0.25.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `type-map` | 0.5.1 | MIT/Apache-2.0 | 2, 11 |  |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 | 1, 5, 34 |  |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | 1, 5, 24 |  |
| `unicode-segmentation` | 1.13.3 | MIT OR Apache-2.0 | 1, 5, 30 |  |
| `unicode-width` | 0.2.2 | MIT OR Apache-2.0 | 1, 5, 30 |  |
| `untrusted` | 0.9.0 | ISC | 19 |  |
| `ureq` | 3.4.2 | MIT OR Apache-2.0 | 1, 5 |  |
| `ureq-proto` | 0.6.4 | MIT OR Apache-2.0 | 1, 5 |  |
| `url` | 2.5.8 | MIT OR Apache-2.0 | 1, 5 |  |
| `utf8-zero` | 0.8.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `utf8_iter` | 1.0.4 | Apache-2.0 OR MIT | 1, 5, 35 |  |
| `web-time` | 1.1.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `webbrowser` | 1.2.4 | MIT OR Apache-2.0 | 1, 5 |  |
| `webpki-roots` | 1.0.9 | CDLA-Permissive-2.0 | 27 |  |
| `weezl` | 0.1.12 | MIT OR Apache-2.0 | 1, 5 |  |
| `wgpu` | 27.0.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `wgpu-core` | 27.0.3 | MIT OR Apache-2.0 | 1, 5 |  |
| `wgpu-core-deps-windows-linux-android` | 27.0.0 | MIT OR Apache-2.0 | 1, 5 |  |
| `wgpu-hal` | 27.0.4 | MIT OR Apache-2.0 | 1, 5 |  |
| `wgpu-types` | 27.0.1 | MIT OR Apache-2.0 | 1, 5 |  |
| `windows` | 0.58.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows` | 0.61.3 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-collections` | 0.2.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-core` | 0.58.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-core` | 0.61.2 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-future` | 0.2.1 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-implement` | 0.58.0 | MIT OR Apache-2.0 | 1, 6 | proc-macro |
| `windows-implement` | 0.60.2 | MIT OR Apache-2.0 | 1, 6 | proc-macro |
| `windows-interface` | 0.58.0 | MIT OR Apache-2.0 | 1, 6 | proc-macro |
| `windows-interface` | 0.59.3 | MIT OR Apache-2.0 | 1, 6 | proc-macro |
| `windows-link` | 0.1.3 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-numerics` | 0.2.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-result` | 0.2.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-result` | 0.3.4 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-strings` | 0.1.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-strings` | 0.4.2 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-sys` | 0.52.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-sys` | 0.59.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-targets` | 0.52.6 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows-threading` | 0.1.0 | MIT OR Apache-2.0 | 1, 6 |  |
| `windows_x86_64_gnu` | 0.52.6 | MIT OR Apache-2.0 | 1, 6 |  |
| `winit` | 0.30.13 | Apache-2.0 | 1 |  |
| `writeable` | 0.6.4 | Unicode-3.0 | 23 |  |
| `yoke` | 0.8.3 | Unicode-3.0 | 23 |  |
| `yoke-derive` | 0.8.4 | Unicode-3.0 | 23 | proc-macro |
| `zerocopy` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | 1, 5, 15 |  |
| `zerocopy-derive` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | 1, 5, 15 | proc-macro |
| `zerofrom` | 0.1.8 | Unicode-3.0 | 23 |  |
| `zerofrom-derive` | 0.1.8 | Unicode-3.0 | 23 | proc-macro |
| `zeroize` | 1.9.1 | Apache-2.0 OR MIT | 1, 5 |  |
| `zerotrie` | 0.2.5 | Unicode-3.0 | 23 |  |
| `zerovec` | 0.11.8 | Unicode-3.0 | 23 |  |
| `zerovec-derive` | 0.11.6 | Unicode-3.0 | 23 | proc-macro |
| `zlib-rs` | 0.6.8 | Zlib | 21 |  |
| `zmij` | 1.0.23 | MIT | 5 |  |
| `zune-core` | 0.5.3 | MIT OR Apache-2.0 OR Zlib | 1, 5, 22 |  |
| `zune-jpeg` | 0.5.15 | MIT OR Apache-2.0 OR Zlib | 1, 5, 22 |  |

## ライセンス本文

同じ本文は 1 回だけ載せ、その本文に当たるクレートと、それぞれの著作権表示を並べています。
著作権表示は各クレートのライセンスファイルの中の行をそのまま写したものです(ファイルに著作権の行が無いものは、代わりに Cargo.toml の作者を添えています)。
Apache-2.0 の本文の末尾にある付録(使い方の例)は、標準のものそのままのときは省いています。

Apache-2.0 の 4 条 (d) の NOTICE ファイルを配布物に持つクレートはありません(あれば、その中身をそのまま下に載せます)。

### 本文 1: Apache-2.0(148 クレート)

当たるクレート(ファイル)と著作権表示:

- `ab_glyph` 0.2.32(`LICENSE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Butler <alexheretic@gmail.com>)
- `ab_glyph_rasterizer` 0.1.10(`LICENSE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Butler <alexheretic@gmail.com>)
- `adler2` 2.0.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Jonas Schievink <jonasschievink@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>)
- `ahash` 0.8.12(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Tom Kaitchuck <Tom.Kaitchuck@gmail.com>)
- `arboard` 3.6.1(`LICENSE-APACHE.txt`): (著作権の行なし)
- `arrayvec` 0.7.8(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: bluss)
- `base64` 0.23.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Marshall Pierce <marshall@mpierce.org>)
- `bit-set` 0.8.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alexis Beingessner <a.beingessner@gmail.com>)
- `bit-vec` 0.8.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alexis Beingessner <a.beingessner@gmail.com>)
- `bitflags` 1.3.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `bitflags` 2.13.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `block-buffer` 0.10.4(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: RustCrypto Developers)
- `bytemuck` 1.25.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Lokathor <zefria@gmail.com>)
- `bytemuck_derive` 1.12.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Lokathor <zefria@gmail.com>)
- `cfg-if` 1.0.5(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Crichton <alex@alexcrichton.com>)
- `codespan-reporting` 0.12.0(`LICENSE`): (ファイルに著作権の行なし。Cargo.toml の作者: Brendan Zabarauskas <bjzaba@yahoo.com.au>)
- `cpufeatures` 0.2.17(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: RustCrypto Developers)
- `crc32fast` 1.5.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Sam Rijs <srijs@airpost.net>, Alex Crichton <alex@alexcrichton.com>)
- `crossbeam-deque` 0.8.8(`LICENSE-APACHE`): (著作権の行なし)
- `crossbeam-epoch` 0.9.21(`LICENSE-APACHE`): (著作権の行なし)
- `crossbeam-utils` 0.8.23(`LICENSE-APACHE`): (著作権の行なし)
- `crypto-common` 0.1.7(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: RustCrypto Developers)
- `cursor-icon` 1.2.0(`LICENSE-APACHE`): Copyright 2023 Kirill Chibisov
- `digest` 0.10.7(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: RustCrypto Developers)
- `displaydoc` 0.2.7(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Jane Lusby <jlusby@yaah.dev>)
- `document-features` 0.2.12(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Slint Developers <info@slint.dev>)
- `dpi` 0.1.2(`LICENSE`): (著作権の行なし)
- `either` 1.19.0(`LICENSE-APACHE`): (著作権の行なし)
- `equivalent` 1.0.3(`LICENSE-APACHE`): (著作権の行なし)
- `fdeflate` 0.3.7(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `flate2` 1.1.10(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Crichton <alex@alexcrichton.com>, Josh Triplett <josh@joshtriplett.org>)
- `form_urlencoded` 1.2.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The rust-url developers)
- `fs4` 0.13.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Dan Burkert <dan@danburkert.com>, Al Liu <scygliu1@gmail.com>)
- `getrandom` 0.2.17(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rand Project Developers)
- `getrandom` 0.3.4(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rand Project Developers)
- `glow` 0.16.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Joshua Groves <josh@joshgroves.com>, Dzmitry Malyshau <kvarkus@gmail.com>)
- `glutin` 0.32.3(`LICENSE`): Copyright 2022 Kirill Chibisov
- `glutin_egl_sys` 0.7.1(`LICENSE`): Copyright 2022 Kirill Chibisov
- `glutin_wgl_sys` 0.6.1(`LICENSE`): Copyright 2022 Kirill Chibisov
- `gpu-allocator` 0.27.0(`LICENSE-APACHE`): Copyright 2021 Traverse Research B.V.
- `half` 2.7.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Kathryn Long <squeeself@gmail.com>)
- `hashbrown` 0.15.5(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Amanieu d'Antras <amanieu@gmail.com>)
- `hashbrown` 0.16.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Amanieu d'Antras <amanieu@gmail.com>)
- `hashbrown` 0.17.1(`LICENSE-APACHE`): (著作権の行なし)
- `http` 1.5.0(`LICENSE-APACHE`): Copyright 2017 http-rs authors
- `httparse` 1.10.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Sean McArthur <sean@seanmonstar.com>)
- `idna` 1.1.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The rust-url developers)
- `idna_adapter` 1.2.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The rust-url developers)
- `image` 0.25.10(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `indexmap` 2.14.2(`LICENSE-APACHE`): (著作権の行なし)
- `itoa` 1.0.18(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `litrs` 1.0.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Lukas Kalbertodt <lukas.kalbertodt@gmail.com>)
- `lock_api` 0.4.14(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Amanieu d'Antras <amanieu@gmail.com>)
- `log` 0.4.34(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `memmap2` 0.9.11(`LICENSE-APACHE`): Copyright [2015] [Dan Burkert]
- `miniz_oxide` 0.8.9(`LICENSE-APACHE.md`): (ファイルに著作権の行なし。Cargo.toml の作者: Frommi <daniil.liferenko@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>, Rich Geldreich richgel99@gmail.com)
- `miniz_oxide` 0.9.1(`LICENSE-APACHE.md`): (ファイルに著作権の行なし。Cargo.toml の作者: Frommi <daniil.liferenko@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>, Rich Geldreich richgel99@gmail.com)
- `moxcms` 0.8.1(`LICENSE-APACHE.md`): Copyright 2024 Radzivon Bartoshyk
- `naga` 27.0.3(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: gfx-rs developers)
- `nohash-hasher` 0.2.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Parity Technologies <admin@parity.io>)
- `num-traits` 0.2.19(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `once_cell` 1.21.4(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Aleksey Kladov <aleksey.kladov@gmail.com>)
- `owned_ttf_parser` 0.25.1(`LICENSE`): (ファイルに著作権の行なし。Cargo.toml の作者: Alex Butler <alexheretic@gmail.com>)
- `parking_lot` 0.12.5(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Amanieu d'Antras <amanieu@gmail.com>)
- `parking_lot_core` 0.9.12(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Amanieu d'Antras <amanieu@gmail.com>)
- `percent-encoding` 2.3.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The rust-url developers)
- `pin-project-lite` 0.2.17(`LICENSE-APACHE`): (著作権の行なし)
- `png` 0.17.16(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `png` 0.18.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `pollster` 0.4.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Joshua Barretto <joshua.s.barretto@gmail.com>)
- `presser` 0.3.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Embark <opensource@embark-studios.com>, Gray Olson <gray@grayolson.com)
- `proc-macro2` 1.0.107(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>, Alex Crichton <alex@alexcrichton.com>)
- `pxfm` 0.1.30(`LICENSE-APACHE.md`): Copyright 2024 Radzivon Bartoshyk
- `quick-error` 2.0.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Paul Colomiets <paul@colomiets.name>, Colin Kiegel <kiegel@gmx.de>)
- `quote` 1.0.47(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `range-alloc` 0.1.5(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: the gfx-rs Developers)
- `raw-window-handle` 0.6.2(`LICENSE-APACHE.md`): (ファイルに著作権の行なし。Cargo.toml の作者: Osspial <osspial@gmail.com>)
- `rayon` 1.12.0(`LICENSE-APACHE`): (著作権の行なし)
- `rayon-core` 1.13.0(`LICENSE-APACHE`): (著作権の行なし)
- `renderdoc-sys` 1.1.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Eyal Kalderon <ebkalderon@gmail.com>)
- `ring` 0.17.14(`src/polyfill/once_cell/LICENSE-APACHE`): (著作権の行なし)
- `rustc-hash` 1.1.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `rustc-hash` 2.1.3(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `rustls` 0.23.46(`LICENSE-APACHE`): (著作権の行なし)
- `rustls-pki-types` 1.15.1(`LICENSE-APACHE`): Copyright 2023 Dirkjan Ochtman
- `scopeguard` 1.2.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: bluss)
- `serde` 1.0.229(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_core` 1.0.229(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_derive` 1.0.229(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_json` 1.0.154(`LICENSE-APACHE`): (著作権の行なし)
- `sha2` 0.10.9(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: RustCrypto Developers)
- `smallvec` 1.16.3(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The Servo Project Developers)
- `smol_str` 0.2.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Aleksey Kladov <aleksey.kladov@gmail.com>)
- `stable_deref_trait` 1.2.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Robert Grosse <n210241048576@gmail.com>)
- `static_assertions` 1.1.0(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Nikolai Vazquez)
- `syn` 2.0.119(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `syn` 3.0.9(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror` 1.0.69(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror` 2.0.21(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror-impl` 1.0.69(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror-impl` 2.0.21(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `ttf-parser` 0.25.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Caleb Maclennan <caleb@alerque.com>, Laurenz Stampfl <laurenz.stampfl@gmail.com>, Yevhenii Reizner <razrfalcon@gmail.com>, خالد حسني (Khaled Hosny) <khaled@aliftype.com>)
- `typenum` 1.20.1(`LICENSE-APACHE`): Copyright 2014 Paho Lurie-Gregg
- `unicode-ident` 1.0.27(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `unicode-segmentation` 1.13.3(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: kwantam <kwantam@gmail.com>, Manish Goregaokar <manishsmail@gmail.com>)
- `unicode-width` 0.2.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: kwantam <kwantam@gmail.com>, Manish Goregaokar <manishsmail@gmail.com>)
- `ureq` 3.4.2(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Martin Algesten <martin@algesten.se>, Jacob Hoffman-Andrews <ureq@hoffman-andrews.com>)
- `ureq-proto` 0.6.4(`LICENSE-APACHE.txt`): (ファイルに著作権の行なし。Cargo.toml の作者: Martin Algesten <martin@algesten.se>)
- `url` 2.5.8(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The rust-url developers)
- `utf8-zero` 0.8.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Simon Sapin <simon.sapin@exyr.org>, Martin Algesten <martin@algesten.se>)
- `utf8_iter` 1.0.4(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: Henri Sivonen <hsivonen@hsivonen.fi>)
- `web-time` 1.1.0(`LICENSE-APACHE`): Copyright 2023 dAxpeDDa
- `webbrowser` 1.2.4(`LICENSE-APACHE`): Copyright (c) 2015-2022 Amod Malviya
- `weezl` 0.1.12(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `wgpu` 27.0.1(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: gfx-rs developers)
- `wgpu-core` 27.0.3(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: gfx-rs developers)
- `wgpu-core-deps-windows-linux-android` 27.0.0(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: gfx-rs developers)
- `wgpu-hal` 27.0.4(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: gfx-rs developers)
- `wgpu-types` 27.0.1(`LICENSE.APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: gfx-rs developers)
- `windows` 0.58.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows` 0.61.3(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-collections` 0.2.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-core` 0.58.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-core` 0.61.2(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-future` 0.2.1(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-implement` 0.58.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-implement` 0.60.2(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-interface` 0.58.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-interface` 0.59.3(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-link` 0.1.3(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-link` 0.2.1(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-numerics` 0.2.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-result` 0.2.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-result` 0.3.4(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-strings` 0.1.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-strings` 0.4.2(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-sys` 0.52.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-sys` 0.59.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-sys` 0.61.2(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-targets` 0.52.6(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows-threading` 0.1.0(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `windows_x86_64_gnu` 0.52.6(`license-apache-2.0`): Copyright (c) Microsoft Corporation.
- `winit` 0.30.13(`LICENSE`): (ファイルに著作権の行なし。Cargo.toml の作者: The winit contributors, Pierre Krieger <pierre.krieger1708@gmail.com>)
- `zerocopy` 0.8.63(`LICENSE-APACHE`): Copyright 2023 The Fuchsia Authors
- `zerocopy-derive` 0.8.63(`LICENSE-APACHE`): Copyright 2023 The Fuchsia Authors
- `zeroize` 1.9.1(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: The RustCrypto Project Developers)
- `zune-core` 0.5.3(`LICENSE-APACHE`): (著作権の行なし)
- `zune-jpeg` 0.5.15(`LICENSE-APACHE`): (ファイルに著作権の行なし。Cargo.toml の作者: caleb <etemesicaleb@gmail.com>)

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
```

### 本文 2: Apache-2.0(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `type-map` 0.5.1(`LICENSE-APACHE`)

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

---

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
      boilerplate notice, with the fields enclosed by brackets "{}"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright 2017-NOW Actix Team

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

### 本文 3: Apache-2.0 + MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `libm` 0.2.16(`LICENSE.txt`)

```text
rust-lang/libm as a whole is available for use under the MIT license:

------------------------------------------------------------------------------
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
------------------------------------------------------------------------------

As a contributor, you agree that your code can be used under either the MIT
license or the Apache-2.0 license:

------------------------------------------------------------------------------
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

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
------------------------------------------------------------------------------

This Rust library contains the following copyrights:

    Copyright (c) 2018 Jorge Aparicio

Portions of this software are derived from third-party works licensed under
terms compatible with the above MIT license:

* musl libc https://www.musl-libc.org/. This library contains the following
  copyright:

      Copyright © 2005-2020 Rich Felker, et al.

* The CORE-MATH project https://core-math.gitlabpages.inria.fr/. CORE-MATH
  routines are available under the MIT license on a per-file basis.

The musl libc COPYRIGHT file also includes the following notice relevant to
math portions of the library:

------------------------------------------------------------------------------
Much of the math library code (src/math/* and src/complex/*) is
Copyright © 1993,2004 Sun Microsystems or
Copyright © 2003-2011 David Schultz or
Copyright © 2003-2009 Steven G. Kargl or
Copyright © 2003-2009 Bruce D. Evans or
Copyright © 2008 Stephen L. Moshier or
Copyright © 2017-2018 Arm Limited
and labelled as such in comments in the individual source files. All
have been licensed under extremely permissive terms.
------------------------------------------------------------------------------

Copyright notices are retained in src/* files where relevant.
```

### 本文 4: Apache-2.0 + BSD-3-Clause(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `ring` 0.17.14(`LICENSE-BoringSSL`)

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

Licenses for support code
-------------------------

Parts of the TLS test suite are under the Go license. This code is not included
in BoringSSL (i.e. libcrypto and libssl) when compiled, however, so
distributing code linked against BoringSSL does not trigger this license:

Copyright (c) 2009 The Go Authors. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
   * Neither the name of Google Inc. nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

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

BoringSSL uses the Chromium test infrastructure to run a continuous build,
trybots etc. The scripts which manage this, and the script for generating build
metadata, are under the Chromium license. Distributing code linked against
BoringSSL does not trigger this license.

Copyright 2015 The Chromium Authors. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
   * Neither the name of Google Inc. nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

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

### 本文 5: MIT(128 クレート)

当たるクレート(ファイル)と著作権表示:

- `adler2` 2.0.1(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Jonas Schievink <jonasschievink@gmail.com>, oyvindln <oyvindln@users.noreply.github.com>)
- `ahash` 0.8.12(`LICENSE-MIT`): Copyright (c) 2018 Tom Kaitchuck
- `arboard` 3.6.1(`LICENSE-MIT.txt`): Copyright (c) 2022 The Arboard contributors
- `arrayvec` 0.7.8(`LICENSE-MIT`): Copyright (c) Ulrik Sverdrup "bluss" 2015-2023
- `base64` 0.23.1(`LICENSE-MIT`): Copyright (c) 2025 Alice Maz, Marshall Pierce
- `bit-set` 0.8.0(`LICENSE-MIT`): Copyright (c) 2023 The Rust Project Developers
- `bit-vec` 0.8.0(`LICENSE-MIT`): Copyright (c) 2023 The Rust Project Developers
- `bitflags` 1.3.2(`LICENSE-MIT`): Copyright (c) 2014 The Rust Project Developers
- `bitflags` 2.13.2(`LICENSE-MIT`): Copyright (c) 2014 The Rust Project Developers
- `block-buffer` 0.10.4(`LICENSE-MIT`): Copyright (c) 2018-2019 The RustCrypto Project Developers
- `byteorder-lite` 0.1.0(`LICENSE-MIT`): Copyright (c) 2015 Andrew Gallant
- `bytes` 1.12.1(`LICENSE`): Copyright (c) 2018 Carl Lerche
- `cfg-if` 1.0.5(`LICENSE-MIT`): Copyright (c) 2014 Alex Crichton
- `cpufeatures` 0.2.17(`LICENSE-MIT`): Copyright (c) 2020-2025 The RustCrypto Project Developers
- `crc32fast` 1.5.2(`LICENSE-MIT`): Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors
- `crossbeam-deque` 0.8.8(`LICENSE-MIT`): Copyright (c) 2019 The Crossbeam Project Developers
- `crossbeam-epoch` 0.9.21(`LICENSE-MIT`): Copyright (c) 2019 The Crossbeam Project Developers
- `crossbeam-utils` 0.8.23(`LICENSE-MIT`): Copyright (c) 2019 The Crossbeam Project Developers
- `crypto-common` 0.1.7(`LICENSE-MIT`): Copyright (c) 2021 RustCrypto Developers
- `cursor-icon` 1.2.0(`LICENSE-MIT`): Copyright (c) 2023 Kirill Chibisov
- `digest` 0.10.7(`LICENSE-MIT`): Copyright (c) 2017 Artyom Pavlov
- `displaydoc` 0.2.7(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Jane Lusby <jlusby@yaah.dev>)
- `document-features` 0.2.12(`LICENSE-MIT`): Copyright (c) 2020 Olivier Goffart <ogoffart@sixtyfps.io>
- `either` 1.19.0(`LICENSE-MIT`): Copyright (c) 2015
- `epaint_default_fonts` 0.33.3(`fonts/emoji-icon-font-mit-license.txt`): Copyright (c) 2014 John Slegers
- `equivalent` 1.0.3(`LICENSE-MIT`): Copyright (c) 2016--2023
- `fax` 0.2.7(`LICENSE`): Copyright © 2021 The pdf-rs contributers.
- `fdeflate` 0.3.7(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `flate2` 1.1.10(`LICENSE-MIT`): Copyright (c) 2014-2026 Alex Crichton
- `form_urlencoded` 1.2.2(`LICENSE-MIT`): Copyright (c) 2013-2016 The rust-url developers
- `fs4` 0.13.1(`LICENSE-MIT`): Copyright (c) 2015 The Rust Project Developers
- `generic-array` 0.14.7(`LICENSE`): Copyright (c) 2015 Bartłomiej Kamiński
- `getrandom` 0.2.17(`LICENSE-MIT`): Copyright (c) 2018-2024 The rust-random Project Developers / Copyright (c) 2014 The Rust Project Developers
- `getrandom` 0.3.4(`LICENSE-MIT`): Copyright (c) 2018-2025 The rust-random Project Developers / Copyright (c) 2014 The Rust Project Developers
- `glow` 0.16.0(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Joshua Groves <josh@joshgroves.com>, Dzmitry Malyshau <kvarkus@gmail.com>)
- `glutin-winit` 0.5.0(`LICENSE`): Copyright © 2022 Kirill Chibisov
- `gpu-allocator` 0.27.0(`LICENSE-MIT`): Copyright (c) 2021 Traverse Research B.V.
- `half` 2.7.1(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Kathryn Long <squeeself@gmail.com>)
- `hashbrown` 0.15.5(`LICENSE-MIT`): Copyright (c) 2016 Amanieu d'Antras
- `hashbrown` 0.16.1(`LICENSE-MIT`): Copyright (c) 2016 Amanieu d'Antras
- `hashbrown` 0.17.1(`LICENSE-MIT`): Copyright (c) 2016 Amanieu d'Antras
- `http` 1.5.0(`LICENSE-MIT`): Copyright (c) 2017 http-rs authors
- `httparse` 1.10.1(`LICENSE-MIT`): Copyright (c) 2015-2025 Sean McArthur
- `idna` 1.1.0(`LICENSE-MIT`): Copyright (c) 2013-2025 The rust-url developers
- `image` 0.25.10(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: The image-rs Developers)
- `indexmap` 2.14.2(`LICENSE-MIT`): Copyright (c) 2016--2017
- `itoa` 1.0.18(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `litrs` 1.0.1(`LICENSE-MIT`): Copyright (c) 2020 Project Developers
- `lock_api` 0.4.14(`LICENSE-MIT`): Copyright (c) 2016 The Rust Project Developers
- `log` 0.4.34(`LICENSE-MIT`): Copyright (c) 2014 The Rust Project Developers
- `memchr` 2.8.3(`LICENSE-MIT`): Copyright (c) 2015 Andrew Gallant
- `memmap2` 0.9.11(`LICENSE-MIT`): Copyright (c) 2020 Yevhenii Reizner / Copyright (c) 2015 Dan Burkert
- `memoffset` 0.9.1(`LICENSE`): Copyright (c) 2017 Gilad Naaman
- `miniz_oxide` 0.8.9(`LICENSE`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.8.9(`LICENSE-MIT.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.9.1(`LICENSE`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.9.1(`LICENSE-MIT.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2017 Frommi / Copyright (c) 2017-2024 oyvindln
- `naga` 27.0.3(`LICENSE.MIT`): Copyright (c) 2025 The gfx-rs developers
- `nohash-hasher` 0.2.0(`LICENSE-MIT`): Copyright 2018 Parity Technologies (UK) Ltd.
- `num-traits` 0.2.19(`LICENSE-MIT`): Copyright (c) 2014 The Rust Project Developers
- `once_cell` 1.21.4(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Aleksey Kladov <aleksey.kladov@gmail.com>)
- `ordered-float` 5.4.0(`LICENSE-MIT`): Copyright (c) 2015 Jonathan Reem
- `parking_lot` 0.12.5(`LICENSE-MIT`): Copyright (c) 2016 The Rust Project Developers
- `parking_lot_core` 0.9.12(`LICENSE-MIT`): Copyright (c) 2016 The Rust Project Developers
- `percent-encoding` 2.3.2(`LICENSE-MIT`): Copyright (c) 2013-2025 The rust-url developers
- `pin-project-lite` 0.2.17(`LICENSE-MIT`): (著作権の行なし)
- `png` 0.17.16(`LICENSE-MIT`): Copyright (c) 2015 nwin
- `png` 0.18.1(`LICENSE-MIT`): Copyright (c) 2015 nwin
- `pollster` 0.4.0(`LICENSE-MIT`): Copyright (c) 2020-2021 Joshua Barretto
- `presser` 0.3.1(`LICENSE-MIT`): Copyright (c) 2019 Embark Studios
- `proc-macro2` 1.0.107(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>, Alex Crichton <alex@alexcrichton.com>)
- `quick-error` 2.0.1(`LICENSE-MIT`): Copyright (c) 2015 The quick-error Developers
- `quote` 1.0.47(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `range-alloc` 0.1.5(`LICENSE.MIT`): Copyright (c) 2023 The gfx-rs developers
- `raw-window-handle` 0.6.2(`LICENSE-MIT.md`): Copyright (c) 2019 Osspial
- `rayon` 1.12.0(`LICENSE-MIT`): Copyright (c) 2010 The Rust Project Developers
- `rayon-core` 1.13.0(`LICENSE-MIT`): Copyright (c) 2010 The Rust Project Developers
- `renderdoc-sys` 1.1.0(`LICENSE-MIT`): Copyright (c) 2022 Eyal Kalderon
- `rfd` 0.15.4(`LICENSE`): Copyright (c) 2022 Bartłomiej Maryńczak
- `rustc-hash` 1.1.0(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `rustc-hash` 2.1.3(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: The Rust Project Developers)
- `rustls` 0.23.46(`LICENSE-MIT`): Copyright (c) 2016 Joseph Birr-Pixton <jpixton@gmail.com>
- `rustls-pki-types` 1.15.1(`LICENSE-MIT`): Copyright (c) 2023 Dirkjan Ochtman <dirkjan@ochtman.nl>
- `scopeguard` 1.2.0(`LICENSE-MIT`): Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers
- `serde` 1.0.229(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_core` 1.0.229(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_derive` 1.0.229(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>)
- `serde_json` 1.0.154(`LICENSE-MIT`): (著作権の行なし)
- `sha2` 0.10.9(`LICENSE-MIT`): Copyright (c) 2006-2009 Graydon Hoare / Copyright (c) 2009-2013 Mozilla Foundation / Copyright (c) 2016 Artyom Pavlov
- `simd-adler32` 0.3.10(`LICENSE.md`): Copyright (c) [2021] [Marvin Countryman]
- `smallvec` 1.16.3(`LICENSE-MIT`): Copyright (c) 2018 The Servo Project Developers
- `smol_str` 0.2.2(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Aleksey Kladov <aleksey.kladov@gmail.com>)
- `stable_deref_trait` 1.2.1(`LICENSE-MIT`): Copyright (c) 2017 Robert Grosse
- `static_assertions` 1.1.0(`LICENSE-MIT`): Copyright (c) 2017 Nikolai Vazquez
- `strict-num` 0.1.1(`LICENSE`): Copyright (c) 2022 Yevhenii Reizner
- `syn` 2.0.119(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `syn` 3.0.9(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `synstructure` 0.14.0(`LICENSE`): Copyright 2016 Nika Layzell
- `thiserror` 1.0.69(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror` 2.0.21(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror-impl` 1.0.69(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `thiserror-impl` 2.0.21(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `tiff` 0.11.3(`LICENSE`): Copyright (c) 2018 PistonDevelopers
- `tracing` 0.1.44(`LICENSE`): Copyright (c) 2019 Tokio Contributors
- `tracing-attributes` 0.1.31(`LICENSE`): Copyright (c) 2019 Tokio Contributors
- `tracing-core` 0.1.36(`LICENSE`): Copyright (c) 2019 Tokio Contributors
- `tracing-core` 0.1.36(`src/spin/LICENSE`): Copyright (c) 2014 Mathijs van de Nes
- `ttf-parser` 0.25.1(`LICENSE-MIT`): Copyright (c) 2018 Yevhenii Reizner
- `typenum` 1.20.1(`LICENSE-MIT`): Copyright (c) 2014 Paho Lurie-Gregg
- `unicode-ident` 1.0.27(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: David Tolnay <dtolnay@gmail.com>)
- `unicode-segmentation` 1.13.3(`LICENSE-MIT`): Copyright (c) 2015 The Rust Project Developers
- `unicode-width` 0.2.2(`LICENSE-MIT`): Copyright (c) 2015 The Rust Project Developers
- `ureq` 3.4.2(`LICENSE-MIT`): Copyright (c) 2019 Martin Algesten
- `ureq-proto` 0.6.4(`LICENSE-MIT.txt`): Copyright 2022 Martin Algesten
- `url` 2.5.8(`LICENSE-MIT`): Copyright (c) 2013-2025 The rust-url developers
- `utf8-zero` 0.8.1(`LICENSE-MIT`): (ファイルに著作権の行なし。Cargo.toml の作者: Simon Sapin <simon.sapin@exyr.org>, Martin Algesten <martin@algesten.se>)
- `utf8_iter` 1.0.4(`LICENSE-MIT`): Copyright Mozilla Foundation
- `web-time` 1.1.0(`LICENSE-MIT`): Copyright (c) 2023 dAxpeDDa
- `webbrowser` 1.2.4(`LICENSE-MIT`): Copyright (c) 2015-2022 Amod Malviya
- `weezl` 0.1.12(`LICENSE-MIT`): Copyright (c) HeroicKatora 2020
- `wgpu` 27.0.1(`LICENSE.MIT`): Copyright (c) 2025 The gfx-rs developers
- `wgpu-core` 27.0.3(`LICENSE.MIT`): Copyright (c) 2025 The gfx-rs developers
- `wgpu-core-deps-windows-linux-android` 27.0.0(`LICENSE.MIT`): Copyright (c) 2025 The gfx-rs developers
- `wgpu-hal` 27.0.4(`LICENSE.MIT`): Copyright (c) 2025 The gfx-rs developers
- `wgpu-types` 27.0.1(`LICENSE.MIT`): Copyright (c) 2025 The gfx-rs developers
- `zerocopy` 0.8.63(`LICENSE-MIT`): Copyright 2023 The Fuchsia Authors
- `zerocopy-derive` 0.8.63(`LICENSE-MIT`): Copyright 2023 The Fuchsia Authors
- `zeroize` 1.9.1(`LICENSE-MIT`): Copyright (c) 2018-2026 The RustCrypto Project Developers
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

### 本文 6: MIT(23 クレート)

当たるクレート(ファイル)と著作権表示:

- `windows` 0.58.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows` 0.61.3(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-collections` 0.2.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-core` 0.58.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-core` 0.61.2(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-future` 0.2.1(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-implement` 0.58.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-implement` 0.60.2(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-interface` 0.58.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-interface` 0.59.3(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-link` 0.1.3(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-link` 0.2.1(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-numerics` 0.2.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-result` 0.2.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-result` 0.3.4(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-strings` 0.1.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-strings` 0.4.2(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-sys` 0.52.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-sys` 0.59.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-sys` 0.61.2(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-targets` 0.52.6(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows-threading` 0.1.0(`license-mit`): Copyright (c) Microsoft Corporation.
- `windows_x86_64_gnu` 0.52.6(`license-mit`): Copyright (c) Microsoft Corporation.

```text
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
    SOFTWARE
```

### 本文 7: MIT(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `bytemuck` 1.25.2(`LICENSE-MIT`): Copyright (c) 2019 Daniel "Lokathor" Gee.
- `bytemuck_derive` 1.12.1(`LICENSE-MIT`): Copyright (c) 2019 Daniel "Lokathor" Gee.

```text
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice (including the next paragraph) shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

### 本文 8: MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `dpi` 0.1.2(`LICENSE-LIBM-MIT`)

```text
rust-lang/libm as a whole is available for use under the MIT license:

------------------------------------------------------------------------------
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
------------------------------------------------------------------------------

This Rust library contains the following copyrights:

    Copyright (c) 2018 Jorge Aparicio

Portions of this software are derived from third-party works licensed under
terms compatible with the above MIT license:

* musl libc https://www.musl-libc.org/. This library contains the following
  copyright:

      Copyright © 2005-2020 Rich Felker, et al.

* The CORE-MATH project https://core-math.gitlabpages.inria.fr/. CORE-MATH
  routines are available under the MIT license on a per-file basis.

The musl libc COPYRIGHT file also includes the following notice relevant to
math portions of the library:

------------------------------------------------------------------------------
Much of the math library code (src/math/* and src/complex/*) is
Copyright © 1993,2004 Sun Microsystems or
Copyright © 2003-2011 David Schultz or
Copyright © 2003-2009 Steven G. Kargl or
Copyright © 2003-2009 Bruce D. Evans or
Copyright © 2008 Stephen L. Moshier or
Copyright © 2017-2018 Arm Limited
and labelled as such in comments in the individual source files. All
have been licensed under extremely permissive terms.
------------------------------------------------------------------------------
```

### 本文 9: MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `idna_adapter` 1.2.2(`LICENSE-MIT`)

```text
Copyright (c) The rust-url developers

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

### 本文 10: MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `ring` 0.17.14(`src/polyfill/once_cell/LICENSE-MIT`)

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
SHALL THE AUTHOR OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### 本文 11: MIT(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `type-map` 0.5.1(`LICENSE-MIT`)

```text
Copyright (c) 2022 Jacob Brown

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

---

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

---

Copyright (c) 2017-NOW Actix Team

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

### 本文 12: MIT + Bitstream-Vera(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `epaint_default_fonts` 0.33.3(`fonts/Hack-Regular.txt`)

```text
The work in the Hack project is Copyright 2018 Source Foundry Authors and licensed under the MIT License

The work in the DejaVu project was committed to the public domain.

Bitstream Vera Sans Mono Copyright 2003 Bitstream Inc. and licensed under the Bitstream Vera License with Reserved Font Names "Bitstream" and "Vera"
MIT License

Copyright (c) 2018 Source Foundry Authors

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
BITSTREAM VERA LICENSE

Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved. Bitstream Vera is a trademark of Bitstream, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy of the fonts accompanying this license ("Fonts") and associated documentation files (the "Font Software"), to reproduce and distribute the Font Software, including without limitation the rights to use, copy, merge, publish, distribute, and/or sell copies of the Font Software, and to permit persons to whom the Font Software is furnished to do so, subject to the following conditions:

The above copyright and trademark notices and this permission notice shall be included in all copies of one or more of the Font Software typefaces.

The Font Software may be modified, altered, or added to, and in particular the designs of glyphs or characters in the Fonts may be modified and additional glyphs or characters may be added to the Fonts, only if the fonts are renamed to names not containing either the words "Bitstream" or the word "Vera".

This License becomes null and void to the extent applicable to Fonts or Font Software that has been modified and is distributed under the "Bitstream Vera" names.

The Font Software may be sold as part of a larger software package but no copy of one or more of the Font Software typefaces may be sold by itself.

THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL BITSTREAM OR THE GNOME FOUNDATION BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM OTHER DEALINGS IN THE FONT SOFTWARE.

Except as contained in this notice, the names of Gnome, the Gnome Foundation, and Bitstream Inc., shall not be used in advertising or otherwise to promote the sale, use or other dealings in this Font Software without prior written authorization from the Gnome Foundation or Bitstream Inc., respectively. For further information, contact: fonts at gnome dot org.
```

### 本文 13: BSD-3-Clause(3 クレート)

当たるクレート(ファイル)と著作権表示:

- `moxcms` 0.8.1(`LICENSE.md`): Copyright (c) Radzivon Bartoshyk. All rights reserved.
- `pxfm` 0.1.30(`LICENSE.md`): Copyright (c) Radzivon Bartoshyk. All rights reserved.
- `subtle` 2.6.1(`LICENSE`): Copyright (c) 2016-2017 Isis Agora Lovecruft, Henry de Valence. All rights reserved. / Copyright (c) 2016-2024 Isis Agora Lovecruft. All rights reserved.

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

### 本文 14: BSD-3-Clause(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `tiny-skia` 0.11.4(`LICENSE`): Copyright (c) 2011 Google Inc. All rights reserved. / Copyright (c) 2020 Yevhenii Reizner All rights reserved.
- `tiny-skia-path` 0.11.4(`LICENSE`): Copyright (c) 2011 Google Inc. All rights reserved. / Copyright (c) 2020 Yevhenii Reizner All rights reserved.

```text
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of the copyright holder nor the names of its
    contributors may be used to endorse or promote products derived
    from this software without specific prior written permission.

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

### 本文 15: BSD-2-Clause(2 クレート)

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

### 本文 16: BSD-2-Clause(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `arrayref` 0.3.9(`LICENSE`)

```text
Copyright (c) 2015 David Roundy <roundyd@physics.oregonstate.edu>
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### 本文 17: ISC(3 クレート)

当たるクレート(ファイル)と著作権表示:

- `libloading` 0.8.9(`LICENSE`): Copyright © 2015, Simonas Kazlauskas
- `ring` 0.17.14(`LICENSE-other-bits`): Copyright 2015-2025 Brian Smith.
- `rustls` 0.23.46(`LICENSE-ISC`): Copyright (c) 2016, Joseph Birr-Pixton <jpixton@gmail.com>

```text
Permission to use, copy, modify, and/or distribute this software for any purpose with or without
fee is hereby granted, provided that the above copyright notice and this permission notice appear
in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
```

### 本文 18: ISC(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `rustls-webpki` 0.103.15(`LICENSE`)

```text
Except as otherwise noted, this project is licensed under the following
(ISC-style) terms:

Copyright 2015 Brian Smith.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHORS DISCLAIM ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

The files under third-party/chromium are licensed as described in
third-party/chromium/LICENSE.
```

### 本文 19: ISC(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `untrusted` 0.9.0(`LICENSE.txt`)

```text
// Copyright 2015-2016 Brian Smith.
//
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHORS DISCLAIM ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

### 本文 20: 0BSD(1 クレート)

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

### 本文 21: Zlib(10 クレート)

当たるクレート(ファイル)と著作権表示:

- `bytemuck` 1.25.2(`LICENSE-ZLIB`): Copyright (c) 2019 Daniel "Lokathor" Gee.
- `bytemuck_derive` 1.12.1(`LICENSE-ZLIB`): Copyright (c) 2019 Daniel "Lokathor" Gee.
- `cursor-icon` 1.2.0(`LICENSE-ZLIB`): Copyright (c) 2023 Kirill Chibisov
- `foldhash` 0.1.5(`LICENSE`): Copyright (c) 2024 Orson Peters
- `foldhash` 0.2.0(`LICENSE`): Copyright (c) 2024 Orson Peters
- `glow` 0.16.0(`LICENSE-ZLIB`): (ファイルに著作権の行なし。Cargo.toml の作者: Joshua Groves <josh@joshgroves.com>, Dzmitry Malyshau <kvarkus@gmail.com>)
- `miniz_oxide` 0.8.9(`LICENSE-ZLIB.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2020 Frommi / Copyright (c) 2017-2024 oyvindln
- `miniz_oxide` 0.9.1(`LICENSE-ZLIB.md`): Copyright 2013-2014 RAD Game Tools and Valve Software / Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC / Copyright (c) 2020 Frommi / Copyright (c) 2017-2024 oyvindln
- `raw-window-handle` 0.6.2(`LICENSE-ZLIB.md`): Copyright (c) 2020 Osspial
- `zlib-rs` 0.6.8(`LICENSE`): (C) 2024 Trifecta Tech Foundation

```text
This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

### 本文 22: Zlib(2 クレート)

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

### 本文 23: Unicode-3.0(18 クレート)

当たるクレート(ファイル)と著作権表示:

- `icu_collections` 2.3.0(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `icu_locale_core` 2.3.0(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `icu_normalizer` 2.3.0(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `icu_normalizer_data` 2.3.0(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `icu_properties` 2.3.0(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `icu_properties_data` 2.3.0(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `icu_provider` 2.3.1(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `litemap` 0.8.3(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `potential_utf` 0.1.6(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `tinystr` 0.8.4(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `writeable` 0.6.4(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `yoke` 0.8.3(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `yoke-derive` 0.8.4(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `zerofrom` 0.1.8(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `zerofrom-derive` 0.1.8(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `zerotrie` 0.2.5(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `zerovec` 0.11.8(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.
- `zerovec-derive` 0.11.6(`LICENSE`): Copyright © 2020-2024 Unicode, Inc.

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

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

SPDX-License-Identifier: Unicode-3.0

—

Portions of ICU4X may have been adapted from ICU4C and/or ICU4J.
ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.
```

### 本文 24: Unicode-3.0(1 クレート)

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

### 本文 25: BSL-1.0(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `error-code` 3.4.0(`LICENSE`)

```text
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### 本文 26: Unlicense(2 クレート)

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

### 本文 27: CDLA-Permissive-2.0(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `webpki-roots` 1.0.9(`LICENSE`)

```text
# Community Data License Agreement - Permissive - Version 2.0

This is the Community Data License Agreement - Permissive, Version
2.0 (the "agreement"). Data Provider(s) and Data Recipient(s) agree
as follows:

## 1. Provision of the Data

1.1. A Data Recipient may use, modify, and share the Data made
available by Data Provider(s) under this agreement if that Data
Recipient follows the terms of this agreement.

1.2. This agreement does not impose any restriction on a Data
Recipient's use, modification, or sharing of any portions of the
Data that are in the public domain or that may be used, modified,
or shared under any other legal exception or limitation.

## 2. Conditions for Sharing Data

2.1. A Data Recipient may share Data, with or without modifications, so
long as the Data Recipient makes available the text of this agreement
with the shared Data.

## 3. No Restrictions on Results

3.1. This agreement does not impose any restriction or obligations
with respect to the use, modification, or sharing of Results.

## 4. No Warranty; Limitation of Liability

4.1. All Data Recipients receive the Data subject to the following
terms:

THE DATA IS PROVIDED ON AN "AS IS" BASIS, WITHOUT REPRESENTATIONS,
WARRANTIES OR CONDITIONS OF ANY KIND, EITHER EXPRESS OR IMPLIED
INCLUDING, WITHOUT LIMITATION, ANY WARRANTIES OR CONDITIONS OF TITLE,
NON-INFRINGEMENT, MERCHANTABILITY OR FITNESS FOR A PARTICULAR PURPOSE.

NO DATA PROVIDER SHALL HAVE ANY LIABILITY FOR ANY DIRECT, INDIRECT,
INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING
WITHOUT LIMITATION LOST PROFITS), HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE DATA OR RESULTS,
EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

## 5. Definitions

5.1. "Data" means the material received by a Data Recipient under
this agreement.

5.2. "Data Provider" means any person who is the source of Data
provided under this agreement and in reliance on a Data Recipient's
agreement to its terms.

5.3. "Data Recipient" means any person who receives Data directly
or indirectly from a Data Provider and agrees to the terms of this
agreement.

5.4. "Results" means any outcome obtained by computational analysis
of Data, including for example machine learning models and models'
insights.
```

### 本文 28: OFL-1.1(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `epaint_default_fonts` 0.33.3(`fonts/OFL.txt`)

```text
This Font Software is licensed under the SIL Open Font License,
Version 1.1.

This license is copied below, and is also available with a FAQ at:
http://scripts.sil.org/OFL

-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font
creation efforts of academic and linguistic communities, and to
provide a free and open framework in which fonts may be shared and
improved in partnership with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply to
any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software
components as distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to,
deleting, or substituting -- in part or in whole -- any of the
components of the Original Version, by changing formats or by porting
the Font Software to a new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed,
modify, redistribute, and sell modified and unmodified copies of the
Font Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components, in
Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the
corresponding Copyright Holder. This restriction only applies to the
primary font name as presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created using
the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```

### 本文 29: Ubuntu-font-1.0(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `epaint_default_fonts` 0.33.3(`fonts/UFL.txt`)

```text
-------------------------------
UBUNTU FONT LICENCE Version 1.0
-------------------------------

PREAMBLE
This licence allows the licensed fonts to be used, studied, modified and
redistributed freely. The fonts, including any derivative works, can be
bundled, embedded, and redistributed provided the terms of this licence
are met. The fonts and derivatives, however, cannot be released under
any other licence. The requirement for fonts to remain under this
licence does not require any document created using the fonts or their
derivatives to be published under this licence, as long as the primary
purpose of the document is not to be a vehicle for the distribution of
the fonts.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this licence and clearly marked as such. This may
include source files, build scripts and documentation.

"Original Version" refers to the collection of Font Software components
as received under this licence.

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to
a new environment.

"Copyright Holder(s)" refers to all individuals and companies who have a
copyright ownership of the Font Software.

"Substantially Changed" refers to Modified Versions which can be easily
identified as dissimilar to the Font Software by users of the Font
Software comparing the Original Version with the Modified Version.

To "Propagate" a work means to do anything with it that, without
permission, would make you directly or secondarily liable for
infringement under applicable copyright law, except executing it on a
computer or modifying a private copy. Propagation includes copying,
distribution (with or without modification and with or without charging
a redistribution fee), making available to the public, and in some
countries other activities as well.

PERMISSION & CONDITIONS
This licence does not grant any rights under trademark law and all such
rights are reserved.

Permission is hereby granted, free of charge, to any person obtaining a
copy of the Font Software, to propagate the Font Software, subject to
the below conditions:

1) Each copy of the Font Software must contain the above copyright
notice and this licence. These can be included either as stand-alone
text files, human-readable headers or in the appropriate machine-
readable metadata fields within text or binary files as long as those
fields can be easily viewed by the user.

2) The font name complies with the following:
(a) The Original Version must retain its name, unmodified.
(b) Modified Versions which are Substantially Changed must be renamed to
avoid use of the name of the Original Version or similar names entirely.
(c) Modified Versions which are not Substantially Changed must be
renamed to both (i) retain the name of the Original Version and (ii) add
additional naming elements to distinguish the Modified Version from the
Original Version. The name of such Modified Versions must be the name of
the Original Version, with "derivative X" where X represents the name of
the new work, appended to that name.

3) The name(s) of the Copyright Holder(s) and any contributor to the
Font Software shall not be used to promote, endorse or advertise any
Modified Version, except (i) as required by this licence, (ii) to
acknowledge the contribution(s) of the Copyright Holder(s) or (iii) with
their explicit written permission.

4) The Font Software, modified or unmodified, in part or in whole, must
be distributed entirely under this licence, and must not be distributed
under any other licence. The requirement for fonts to remain under this
licence does not affect any document created using the Font Software,
except any version of the Font Software extracted from a document
created using the Font Software may only be distributed under this
licence.

TERMINATION
This licence becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM OTHER
DEALINGS IN THE FONT SOFTWARE.
```

### 本文 30: そのほかの表示(2 クレート)

当たるクレート(ファイル)と著作権表示:

- `unicode-segmentation` 1.13.3(`COPYRIGHT`): (ファイルに著作権の行なし。Cargo.toml の作者: kwantam <kwantam@gmail.com>, Manish Goregaokar <manishsmail@gmail.com>)
- `unicode-width` 0.2.2(`COPYRIGHT`): (ファイルに著作権の行なし。Cargo.toml の作者: kwantam <kwantam@gmail.com>, Manish Goregaokar <manishsmail@gmail.com>)

```text
Licensed under the Apache License, Version 2.0
<LICENSE-APACHE or
http://www.apache.org/licenses/LICENSE-2.0> or the MIT
license <LICENSE-MIT or http://opensource.org/licenses/MIT>,
at your option. All files in the project carrying such
notice may not be copied, modified, or distributed except
according to those terms.
```

### 本文 31: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `memchr` 2.8.3(`COPYING`)

```text
This project is dual-licensed under the Unlicense and MIT licenses.

You may use this code under the terms of either license.
```

### 本文 32: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `ring` 0.17.14(`LICENSE`)

```text
*ring* uses an "ISC" license, like BoringSSL used to use, for new code
files. See LICENSE-other-bits for the text of that license.

See LICENSE-BoringSSL for code that was sourced from BoringSSL under the
Apache 2.0 license. Some code that was sourced from BoringSSL under the ISC
license. In each case, the license info is at the top of the file.

See src/polyfill/once_cell/LICENSE-APACHE and src/polyfill/once_cell/LICENSE-MIT
for the license to code that was sourced from the once_cell project.
```

### 本文 33: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `ring` 0.17.14(`third_party/fiat/LICENSE`)

```text
The Apache License, Version 2.0 (Apache-2.0)

Copyright 2015-2020 the fiat-crypto authors (see the AUTHORS file)

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

### 本文 34: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `typenum` 1.20.1(`LICENSE`)

```text
MIT OR Apache-2.0
```

### 本文 35: そのほかの表示(1 クレート)

当たるクレート(ファイル)。著作権表示は本文の中にあります:

- `utf8_iter` 1.0.4(`COPYRIGHT`)

```text
Copyright Mozilla Foundation

Licensed under the Apache License (Version 2.0), or the MIT license,
(the "Licenses") at your option. You may not use this file except in
compliance with one of the Licenses. You may obtain copies of the
Licenses at:

   https://www.apache.org/licenses/LICENSE-2.0
   https://opensource.org/licenses/MIT

Unless required by applicable law or agreed to in writing, software
distributed under the Licenses is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the Licenses for the specific language governing permissions and
limitations under the Licenses.

--

Test code is dedicated to the Public Domain when so designated (see
the individual files for PD/CC0-dedicated sections).

--

The implementation for Utf8CharIndices was adapted from the
CharIndices implementation of the Rust standard library at revision
ab32548539ec38a939c1b58599249f3b54130026
(https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/library/core/src/str/iter.rs).

Excerpt from https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/COPYRIGHT ,
which refers to
https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/LICENSE-APACHE
and
https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/LICENSE-MIT
:

For full authorship information, see the version control history or
https://thanks.rust-lang.org

Except as otherwise noted (below and/or in individual files), Rust is
licensed under the Apache License, Version 2.0 <LICENSE-APACHE> or
<http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
<LICENSE-MIT> or <http://opensource.org/licenses/MIT>, at your option.
```

## 実行時ライブラリの表示

MSYS2 UCRT64 のツールチェーンに入っている表示のファイルを、そのまま写しています(HTML のものは文字だけにしています)。

### 実行時-1: Rust の標準ライブラリ(std・core・alloc とその依存)

ライセンス式: (MIT OR Apache-2.0) AND Unicode-3.0 AND BSD-2-Clause。元のファイル: `share/doc/rustc/COPYRIGHT-library.html`(rustc の sysroot から)

```text
Copyright notices for The Rust Standard Library

Table of Contents

Short version for non-lawyers

Longer version

In-tree files

Out-of-tree dependencies

Short version for non-lawyers

The Rust Standard Library is dual-licensed under Apache 2.0 and MIT terms.

Longer version

Copyrights in the Rust Standard Library are retained by their contributors. No copyright assignment is required to contribute to the Rust project.

Some files include explicit copyright notices and/or license notices. For full authorship information, see the version control history or https://thanks.rust-lang.org.

Except as otherwise noted (below and/or in individual files), the Rust Standard Library is licensed under the Apache License, Version 2.0 or the MIT license, at your option.

This file describes the copyright and licensing information for the source code within The Rust Project git tree related to the Rust Standard Library, and the third-party dependencies used when building the Rust Standard Library.

In-tree files

The following licenses cover the in-tree source files that were used in this release:

File/Directory: .

License: Apache-2.0 OR MIT

Copyright: The Rust Project Developers (see https://thanks.rust-lang.org)

Exceptions:

File/Directory: library/backtrace

License: Apache-2.0 OR MIT

Copyright: 2014 Alex Crichton

Copyright: The Rust Project Developers (see https://thanks.rust-lang.org)

File/Directory: library/core/src/unicode/unicode_data.rs

License: Unicode-3.0

Copyright: 1991-2024 Unicode, Inc

File/Directory: library/std/src/sync/mpmc

License: Apache-2.0 OR MIT

Copyright: 2019 The Crossbeam Project Developers

Copyright: The Rust Project Developers (see https://thanks.rust-lang.org)

File/Directory: library/std/src/sys/sync/mutex/fuchsia.rs

License: BSD-2-Clause AND (Apache-2.0 OR MIT)

Copyright: 2016 The Fuchsia Authors

Copyright: The Rust Project Developers (see https://thanks.rust-lang.org)

Out-of-tree dependencies

The following licenses cover the out-of-tree crates that were used in the
Rust Standard Library in this release:
```

### 実行時-2: compiler-rt(標準ライブラリの compiler_builtins に入っている C)

ライセンス式: Apache-2.0 WITH LLVM-exception。元のファイル: `share/doc/rustc/licenses/LLVM-exception.txt`(rustc の sysroot から)

```text
---- LLVM Exceptions to the Apache 2.0 License ----

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

### 実行時-3: MinGW-w64 の実行時ライブラリ(crt2.o・libmingw32.a・libmingwex.a)

ライセンス式: ZPL-2.1 AND ISC AND BSD-2-Clause AND LicenseRef-Lucent-gdtoa AND SunPro AND MIT。元のファイル: `share/licenses/crt/COPYING.MinGW-w64-runtime.txt`(rustc の sysroot から)

```text
MinGW-w64 runtime licensing
***************************

This program or library was built using MinGW-w64 and statically
linked against the MinGW-w64 runtime. Some parts of the runtime
are under licenses which require that the copyright and license
notices are included when distributing the code in binary form.
These notices are listed below.

========================
Overall copyright notice
========================

Copyright (c) 2009, 2010, 2011, 2012, 2013 by the mingw-w64 project

This license has been certified as open source. It has also been designated
as GPL compatible by the Free Software Foundation (FSF).

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

   1. Redistributions in source code must retain the accompanying copyright
      notice, this list of conditions, and the following disclaimer.
   2. Redistributions in binary form must reproduce the accompanying
      copyright notice, this list of conditions, and the following disclaimer
      in the documentation and/or other materials provided with the
      distribution.
   3. Names of the copyright holders must not be used to endorse or promote
      products derived from this software without prior written permission
      from the copyright holders.
   4. The right to distribute this software or to use it for any purpose does
      not give you the right to use Servicemarks (sm) or Trademarks (tm) of
      the copyright holders.  Use of them is covered by separate agreement
      with the copyright holders.
   5. If any files are modified, you must cause the modified files to carry
      prominent notices stating that you changed the files and the date of
      any change.

Disclaimer

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS ``AS IS'' AND ANY EXPRESSED
OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES
OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO
EVENT SHALL THE COPYRIGHT HOLDERS BE LIABLE FOR ANY DIRECT, INDIRECT,
INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA,
OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE,
EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

========================================
getopt, getopt_long, and getop_long_only
========================================

Copyright (c) 2002 Todd C. Miller <Todd.Miller@courtesan.com>

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

Sponsored in part by the Defense Advanced Research Projects
Agency (DARPA) and Air Force Research Laboratory, Air Force
Materiel Command, USAF, under agreement number F39502-99-1-0512.

        *       *       *       *       *       *       *

Copyright (c) 2000 The NetBSD Foundation, Inc.
All rights reserved.

This code is derived from software contributed to The NetBSD Foundation
by Dieter Baron and Thomas Klausner.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
 1. Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.
 2. Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in the
    documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE NETBSD FOUNDATION, INC. AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION OR CONTRIBUTORS
BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.

===============================================================
gdtoa: Converting between IEEE floating point numbers and ASCII
===============================================================

The author of this software is David M. Gay.

Copyright (C) 1997, 1998, 1999, 2000, 2001 by Lucent Technologies
All Rights Reserved

Permission to use, copy, modify, and distribute this software and
its documentation for any purpose and without fee is hereby
granted, provided that the above copyright notice appear in all
copies and that both that the copyright notice and this
permission notice and warranty disclaimer appear in supporting
documentation, and that the name of Lucent or any of its entities
not be used in advertising or publicity pertaining to
distribution of the software without specific, written prior
permission.

LUCENT DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,
INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.
IN NO EVENT SHALL LUCENT OR ANY OF ITS ENTITIES BE LIABLE FOR ANY
SPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER
IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.

        *       *       *       *       *       *       *

The author of this software is David M. Gay.

Copyright (C) 2005 by David M. Gay
All Rights Reserved

Permission to use, copy, modify, and distribute this software and its
documentation for any purpose and without fee is hereby granted,
provided that the above copyright notice appear in all copies and that
both that the copyright notice and this permission notice and warranty
disclaimer appear in supporting documentation, and that the name of
the author or any of his current or former employers not be used in
advertising or publicity pertaining to distribution of the software
without specific, written prior permission.

THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,
INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.  IN
NO EVENT SHALL THE AUTHOR OR ANY OF HIS CURRENT OR FORMER EMPLOYERS BE
LIABLE FOR ANY SPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY
DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS
SOFTWARE.

        *       *       *       *       *       *       *

The author of this software is David M. Gay.

Copyright (C) 2004 by David M. Gay.
All Rights Reserved
Based on material in the rest of /netlib/fp/gdota.tar.gz,
which is copyright (C) 1998, 2000 by Lucent Technologies.

Permission to use, copy, modify, and distribute this software and
its documentation for any purpose and without fee is hereby
granted, provided that the above copyright notice appear in all
copies and that both that the copyright notice and this
permission notice and warranty disclaimer appear in supporting
documentation, and that the name of Lucent or any of its entities
not be used in advertising or publicity pertaining to
distribution of the software without specific, written prior
permission.

LUCENT DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,
INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.
IN NO EVENT SHALL LUCENT OR ANY OF ITS ENTITIES BE LIABLE FOR ANY
SPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER
IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.

=========================
Parts of the math library
=========================

Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.

Developed at SunSoft, a Sun Microsystems, Inc. business.
Permission to use, copy, modify, and distribute this
software is freely granted, provided that this notice
is preserved.

        *       *       *       *       *       *       *

Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.

Developed at SunPro, a Sun Microsystems, Inc. business.
Permission to use, copy, modify, and distribute this
software is freely granted, provided that this notice
is preserved.

        *       *       *       *       *       *       *

FIXME: Cephes math lib
Copyright (C) 1984-1998 Stephen L. Moshier

It sounds vague, but as to be found at
<http://lists.debian.org/debian-legal/2004/12/msg00295.html>, it gives an
impression that the author could be willing to give an explicit
permission to distribute those files e.g. under a BSD style license. So
probably there is no problem here, although it could be good to get a
permission from the author and then add a license into the Cephes files
in MinGW runtime. At least on follow-up it is marked that debian sees the
version a-like BSD one. As MinGW.org (where those cephes parts are coming
from) distributes them now over 6 years, it should be fine.

=================================================
Some string, memory and time conversion functions
=================================================

Copyright © 2005-2020 Rich Felker, et al.

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

===================================
Headers and IDLs imported from Wine
===================================

Some header and IDL files were imported from the Wine project. These files
are prominent maked in source. Their copyright belongs to contributors and
they are distributed under LGPL license.

Disclaimer

This library is free software; you can redistribute it and/or
modify it under the terms of the GNU Lesser General Public
License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version.

This library is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
Lesser General Public License for more details.
```

### 実行時-4: winpthreads(libpthread.a)

ライセンス式: MIT AND BSD-3-Clause。元のファイル: `share/licenses/winpthreads/COPYING`(rustc の sysroot から)

```text
Copyright (c) 2011 mingw-w64 project

Permission is hereby granted, free of charge, to any person obtaining a
copy of this software and associated documentation files (the "Software"),
to deal in the Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

/*
 * Parts of this library are derived by:
 *
 * Posix Threads library for Microsoft Windows
 *
 * Use at own risk, there is no implied warranty to this code.
 * It uses undocumented features of Microsoft Windows that can change
 * at any time in the future.
 *
 * (C) 2010 Lockless Inc.
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without modification,
 * are permitted provided that the following conditions are met:
 *
 *
 *  * Redistributions of source code must retain the above copyright notice,
 *    this list of conditions and the following disclaimer.
 *  * Redistributions in binary form must reproduce the above copyright notice,
 *    this list of conditions and the following disclaimer in the documentation
 *    and/or other materials provided with the distribution.
 *  * Neither the name of Lockless Inc. nor the names of its contributors may be
 *    used to endorse or promote products derived from this software without
 *    specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AN
 * ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.
 * IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT,
 * INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING,
 * BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
 * DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
 * LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE
 * OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED
 * OF THE POSSIBILITY OF SUCH DAMAGE.
 */
```

### 実行時-5: libgcc.a・libgcc_eh.a(GCC の実行時ライブラリ。例外の巻き戻しなど)

ライセンス式: GPL-3.0-or-later WITH GCC-exception-3.1。元のファイル: `share/licenses/gcc-libs/COPYING.RUNTIME`(rustc の sysroot から)

```text
GCC RUNTIME LIBRARY EXCEPTION

Version 3.1, 31 March 2009

Copyright (C) 2009 Free Software Foundation, Inc. <http://fsf.org/>

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.

This GCC Runtime Library Exception ("Exception") is an additional
permission under section 7 of the GNU General Public License, version
3 ("GPLv3"). It applies to a given file (the "Runtime Library") that
bears a notice placed by the copyright holder of the file stating that
the file is governed by GPLv3 along with this Exception.

When you use GCC to compile a program, GCC may combine portions of
certain GCC header files and runtime libraries with the compiled
program. The purpose of this Exception is to allow compilation of
non-GPL (including proprietary) programs to use, in this way, the
header files and runtime libraries covered by this Exception.

0. Definitions.

A file is an "Independent Module" if it either requires the Runtime
Library for execution after a Compilation Process, or makes use of an
interface provided by the Runtime Library, but is not otherwise based
on the Runtime Library.

"GCC" means a version of the GNU Compiler Collection, with or without
modifications, governed by version 3 (or a specified later version) of
the GNU General Public License (GPL) with the option of using any
subsequent versions published by the FSF.

"GPL-compatible Software" is software whose conditions of propagation,
modification and use would permit combination with GCC in accord with
the license of GCC.

"Target Code" refers to output from any compiler for a real or virtual
target processor architecture, in executable form or suitable for
input to an assembler, loader, linker and/or execution
phase. Notwithstanding that, Target Code does not include data in any
format that is used as a compiler intermediate representation, or used
for producing a compiler intermediate representation.

The "Compilation Process" transforms code entirely represented in
non-intermediate languages designed for human-written code, and/or in
Java Virtual Machine byte code, into Target Code. Thus, for example,
use of source code generators and preprocessors need not be considered
part of the Compilation Process, since the Compilation Process can be
understood as starting with the output of the generators or
preprocessors.

A Compilation Process is "Eligible" if it is done using GCC, alone or
with other GPL-compatible software, or if it is done without using any
work based on GCC. For example, using non-GPL-compatible Software to
optimize any GCC intermediate representations would not qualify as an
Eligible Compilation Process.

1. Grant of Additional Permission.

You have permission to propagate a work of Target Code formed by
combining the Runtime Library with Independent Modules, even if such
propagation would otherwise violate the terms of GPLv3, provided that
all Target Code was generated by Eligible Compilation Processes. You
may then convey such a combination under terms of your choice,
consistent with the licensing of the Independent Modules.

2. No Weakening of GCC Copyleft.

The availability of this Exception does not imply any general
presumption that third-party software is unaffected by the copyleft
requirements of the license of GCC.
```

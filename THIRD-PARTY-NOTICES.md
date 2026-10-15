# 同梱しているソフトウェアの表示

## MIT ライセンスが及ぶ範囲

同梱の [LICENSE](LICENSE) —— MIT ライセンス —— が及ぶのは
**zahyou 自身のソースコードだけ**です。

    zahyou_engine.py / zahyou.ipynb / zahyou_v6.ipynb / gui/ / dev/ / docs/

オフライン解析のエンジン(`dev/solver_*.py`。`zahyou_engine.py` に組み込まれる)も
zahyou 自身のコードです。`zahyou.exe` に同梱した第三者のライブラリ、星図データ
(index ファイル)とその元になった星表は含みません。それぞれの扱いと、
解き方の出典を以下にまとめます。

## 実行ファイルに入っているもの

`zahyou.exe` は Python のプログラムを 1 つの実行ファイルにまとめたものです。
そのため、下に挙げた第三者のライブラリが実行ファイルの中に入っています。
それぞれの著作権は各プロジェクトに属し、各ライセンスの条件で配布されています。

この一覧は、実際に出来上がった実行ファイルの中身から機械的に作っています
(`dev/make_notices.py`)。手で書き足したものではありません。

### 解析の中核

| ライブラリ | 版 | ライセンス |
|---|---|---|
| PyAVM | 0.9.6 | MIT |
| astropy | 7.1.0 | BSD-3-Clause |
| astropy-iers-data | 0.2025.8.18.0.40.14 | BSD License |
| astropy_healpix | 1.1.2 | BSD 3-Clause |
| astroquery | 0.4.10 | BSD |
| numpy | 2.2.6 | BSD-3-Clause |
| photutils | 2.3.0 | BSD-3-Clause |
| pyerfa | 2.0.1.5 | BSD 3-Clause License |
| pyvo | 1.7 | BSD-3-Clause |
| reproject | 0.15.0 | BSD 3-Clause |
| scipy | 1.16.1 | BSD-3-Clause |

### 画面と図

| ライブラリ | 版 | ライセンス |
|---|---|---|
| contourpy | 1.3.2 | BSD License |
| cycler | 0.12.1 | BSD-3-Clause |
| kiwisolver | 1.4.8 | BSD-3-Clause |
| matplotlib | 3.10.3 | Matplotlib License (PSF ベース) |
| pillow | 11.2.1 | MIT-CMU |
| pyparsing | 3.2.3 | MIT License |

### 配列の分割処理

| ライブラリ | 版 | ライセンス |
|---|---|---|
| cloudpickle | 3.1.1 | BSD-3-Clause |
| dask | 2025.7.0 | BSD-3-Clause |
| donfig | 0.8.1.post1 | MIT |
| fsspec | 2025.7.0 | BSD License |
| locket | 1.0.0 | BSD-2-Clause |
| networkx | 3.6.1 | BSD-3-Clause |
| numcodecs | 0.16.2 | MIT |
| partd | 1.4.2 | BSD |
| toolz | 1.0.0 | BSD |
| zarr | 3.1.1 | MIT |

### そのほか(上のライブラリが内部で使うもの)

| ライブラリ | 版 | ライセンス |
|---|---|---|
| Jinja2 | 3.1.6 | BSD License |
| MarkupSafe | 3.0.3 | BSD-3-Clause |
| PyYAML | 6.0.2 | MIT |
| Pygments | 2.19.1 | BSD-2-Clause |
| attrs | 25.4.0 | MIT |
| beautifulsoup4 | 4.13.4 | MIT License |
| certifi | 2025.8.3 | MPL-2.0 |
| cffi | 2.0.0 | MIT |
| charset-normalizer | 3.4.3 | MIT |
| colorama | 0.4.6 | BSD License |
| comm | 0.2.2 | BSD License |
| cryptography | 46.0.4 | Apache-2.0 OR BSD-3-Clause |
| debugpy | 1.8.14 | MIT |
| html5lib | 1.1 | MIT License |
| idna | 3.10 | BSD License |
| importlib_metadata | 8.7.1 | Apache-2.0 |
| ipykernel | 6.29.5 | BSD License |
| jaraco.classes | 3.4.0 | MIT License |
| jaraco.context | 6.0.1 | MIT License |
| jaraco.functools | 4.3.0 | MIT |
| jupyter_client | 8.6.3 | BSD License |
| jupyter_core | 5.7.2 | BSD License |
| keyring | 25.6.0 | MIT License |
| markdown-it-py | 4.0.0 | MIT License |
| mdurl | 0.1.2 | MIT License |
| more-itertools | 10.7.0 | MIT License |
| mpmath | 1.3.0 | BSD |
| nest-asyncio | 1.6.0 | BSD |
| packaging | 25.0 | Apache Software License |
| platformdirs | 4.3.8 | MIT |
| psutil | 7.0.0 | BSD-3-Clause |
| pycparser | 3.0 | BSD-3-Clause |
| python-dateutil | 2.9.0.post0 | Apache-2.0 / BSD-3-Clause |
| pytz | 2025.2 | MIT |
| pywin32 | 310 | PSF-2.0 |
| pywin32-ctypes | 0.2.3 | BSD-3-Clause |
| pyzmq | 26.4.0 | BSD License |
| requests | 2.32.5 | Apache-2.0 |
| rich | 14.3.2 | MIT |
| setuptools | 82.0.0 | MIT |
| six | 1.17.0 | MIT |
| soupsieve | 2.7 | MIT |
| tornado | 6.5.1 | Apache-2.0 |
| tqdm | 4.67.1 | MPL-2.0 AND MIT |
| traitlets | 5.14.3 | BSD License |
| typing_extensions | 4.14.1 | PSF-2.0 |
| urllib3 | 2.5.0 | MIT |
| webencodings | 0.5.1 | BSD |
| zipp | 3.23.0 | MIT |

## コピーレフトのライブラリについて

**実行ファイルに GPL / LGPL のコードは入っていません。**

- **MPL-2.0** のものが 2 つあります(`certifi` と `tqdm`)。MPL はファイル単位の
  条件で、手を加えずにそのまま同梱するぶんには制限がありません。改変した場合は
  そのファイルのソースを開示する必要がありますが、**改変していません**。
  原本はそれぞれ <https://github.com/certifi/python-certifi> と
  <https://github.com/tqdm/tqdm> にあります。
- 以前は `crc32c`(LGPL-2.1-or-later)が入っていました。`reproject` →
  `zarr` → `numcodecs[crc32c]` という連鎖でたぐられていたものです。
  チェックサムの符号化器は使っていないので、ビルド時に外しました。
  外しても小分け再投影の結果は変わりません(平均も有効画素数も一致)。

## オフライン解析の解き方 — astrometry.net の方式

オフライン解析は、zahyou の中の自前のエンジン(`dev/solver_wcs.py`・
`dev/solver_index.py`・`dev/solver_core.py`)が行います。zahyou 自身のコードとして
MIT ライセンスで配ります。

以前の版(v6 まで)は、[準備]タブが WSL の Ubuntu へ入れた astrometry.net
(GPL-3.0-or-later)の `solve-field` を、別のプロセスとして呼んでいました。
**この版は呼びません。** WSL も astrometry.net の実行ファイルも使いません。

解き方(星 4 つ(広い段は 3 つ)の並びの幾何ハッシュ → 照合 → ベイズの検証 → 改良)は、
下の論文と、astrometry.net の公開文書と BSD-3 の部分のソース(`solver.c`・`verify.c`・
`tweak2.c` など)を読んでまとめた調べ物をもとに、自分で書き直したものです。
調べ物はソースの表現を写さず、式と条件を自分の言葉で書き直したもので、
エンジンを書くときにはソースを読んでいません。GPL の部分(FITS の入出力層
`qfits-an` など)は、調べ物でも読んでいません。
**astrometry.net のコードは含んでいません**(取り込みも移植もしていません)。
FITS の読み込みは FITS 規格から書きました。
索引ファイルの形式は、実物のファイルを読んで調べました
([docs/index-format.md](docs/index-format.md))。

- Lang, D., Hogg, D. W., Mierle, K., Blanton, M., & Roweis, S. (2010),
  "Astrometry.net: Blind astrometric calibration of arbitrary astronomical
  images", *AJ* **139**, 1782
- <https://astrometry.net/>

## 同梱していないもの

次のものは**実行ファイルに入っていません**。使う人の PC に、使う人自身が
入れるか、実行時に取ってくるものです。

### 星図データ(index ファイル)

zahyou の実行ファイルには同梱していません。exe は[準備]タブが下記の原典から
落とします。ノートブック版の人向けには、作者が原典をそのまま詰め直した zip を
配布ページに置いています(この節の最後)。中身の星のデータには、元になった星表の
条件がかかります。

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
`dev/make_index_zips.py` で作る zip の中の README にも、この条件・クレジット・謝辞を
入れています。

### 天体名から座標を引くとき

`astroquery` 経由で SIMBAD と VizieR(いずれも CDS, ストラスブール天文台)に
問い合わせます。オンライン解析では Astrometry.net の Web サービスも使います。
いずれも実行時の問い合わせで、データを同梱してはいません。

> This research has made use of the SIMBAD database and the VizieR catalogue
> access tool, operated at CDS, Strasbourg, France.

### マニュアル PDF の書体

本文は **Noto Sans JP**(SIL Open Font License 1.1)、
等幅部分は **HackGen** で組んでいます。OFL は PDF への埋め込みを認めています。

# 同梱しているソフトウェアの表示

## MIT ライセンスが及ぶ範囲

同梱の [LICENSE](LICENSE) —— MIT ライセンス —— が及ぶのは
**zahyou 自身のソースコードだけ**です。

    zahyou_engine.py / zahyou.ipynb / zahyou_v6.ipynb / gui/ / dev/ / docs/

`zahyou.exe` に同梱した第三者のライブラリ、astrometry.net、星図データ
(index ファイル)とその元になった星表は含みません。それぞれの扱いを
以下にまとめます。

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

## 同梱していないもの

次のものは**実行ファイルに入っていません**。使う人の PC に、使う人自身が
入れるか、実行時に取ってくるものです。

### astrometry.net — GPL-3.0-or-later

オフライン解析の計算そのものを担うプログラムです。
[準備]タブが WSL の Ubuntu へ `apt install astrometry.net` で入れ、
zahyou は `solve-field` を**別のプロセスとして呼ぶ**だけです。
zahyou の中にコードを取り込んでいないので、zahyou 自身のライセンスとは
独立しています。

- <https://astrometry.net/> / <https://github.com/dstndstn/astrometry.net>
- Lang, D., Hogg, D. W., Mierle, K., Blanton, M., Roweis, S. (2010),
  "Astrometry.net: Blind astrometric calibration of arbitrary astronomical
  images", *AJ* **139**, 1782

### 星図データ(index ファイル)

[準備]タブが下記から直接ダウンロードします。zahyou には同梱していません。

- 4100 シリーズ … Tycho-2 星表から作られたもの。<https://data.astrometry.net/4100/>
- 5200 シリーズ … Tycho-2 と Gaia DR2 から作られたもの。
  <https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/>

**配布ページ(Nextcloud)に置いてある `AstrometryData*.zip` は、この
index ファイルをそのまま詰め直したものです。** 中身は上記の原典と同一で、
回線の細い場所でまとめて取れるようにしてあるだけです。
元データの権利は Gaia(ESA/Gaia/DPAC)および Tycho-2 の各制作者にあります。

### 天体名から座標を引くとき

`astroquery` 経由で SIMBAD と VizieR(いずれも CDS, ストラスブール天文台)に
問い合わせます。オンライン解析では Astrometry.net の Web サービスも使います。
いずれも実行時の問い合わせで、データを同梱してはいません。

> This research has made use of the SIMBAD database and the VizieR catalogue
> access tool, operated at CDS, Strasbourg, France.

### マニュアル PDF の書体

本文は **Noto Sans JP**(SIL Open Font License 1.1)、
等幅部分は **HackGen** で組んでいます。OFL は PDF への埋め込みを認めています。

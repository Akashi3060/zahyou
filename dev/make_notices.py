# -*- coding: utf-8 -*-
"""THIRD-PARTY-NOTICES.md を、実際に exe へ入ったものから作る。

手で並べると必ずずれるので、PyInstaller が書いた TOC を読んで、
そこに現れたパッケージだけを列挙する。

    python dev/make_notices.py            # 書き出す
    python dev/make_notices.py --check    # 中身が古くなっていないか見るだけ
"""
from __future__ import annotations

import importlib.metadata as md
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOC = os.path.join(ROOT, "gui", "build", "onefile", "zahyou", "Analysis-00.toc")
OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.md")

# 配布物名 -> 表示するライセンス名。メタデータの License 欄は本文が
# まるごと入っていたり空だったりするので、読める形に直す。
FIX = {
    "cycler": "BSD-3-Clause",
    "kiwisolver": "BSD-3-Clause",
    "numpy": "BSD-3-Clause",
    "scipy": "BSD-3-Clause",
    "matplotlib": "Matplotlib License (PSF ベース)",
    "python-dateutil": "Apache-2.0 / BSD-3-Clause",
    "pywin32": "PSF-2.0",
}
# 大きな塊はまとめて出す。全部を 1 行ずつ並べても読まれない。
GROUPS = [
    ("解析の中核", ["astropy", "astroquery", "numpy", "scipy", "photutils",
                    "reproject", "astropy_healpix", "pyerfa", "astropy-iers-data",
                    "PyAVM", "pyvo"]),
    ("画面と図", ["matplotlib", "pillow", "contourpy", "cycler", "kiwisolver",
                  "pyparsing", "fonttools"]),
    ("配列の分割処理", ["dask", "zarr", "numcodecs", "fsspec", "toolz", "partd",
                        "locket", "cloudpickle", "donfig", "networkx"]),
]


def bundled():
    if not os.path.exists(TOC):
        raise SystemExit("!! %s が無い。先に exe を建てること" % TOC)
    s = open(TOC, encoding="utf-8", errors="replace").read()
    tops = {n.split(".")[0] for n, _s, _k in re.findall(
        r"\('([^']+)',\s*'([^']*)',\s*'(PYMODULE|EXTENSION)'\)", s)}
    mapping = md.packages_distributions()
    return {d for t in tops for d in mapping.get(t, [])}


def licence_of(dist):
    if dist in FIX:
        return FIX[dist]
    try:
        m = md.metadata(dist)
    except Exception:
        return "?"
    lic = (m.get("License-Expression") or "").strip()
    if lic:
        return lic
    raw = (m.get("License") or "").strip()
    if raw and len(raw) < 40 and "\n" not in raw:
        return raw
    for c in m.get_all("Classifier") or []:
        if c.startswith("License ::"):
            return c.split("::")[-1].strip()
    return "?"


def version_of(dist):
    try:
        return md.version(dist)
    except Exception:
        return "?"


HEAD = """# 同梱しているソフトウェアの表示

`zahyou.exe` は Python のプログラムを 1 つの実行ファイルにまとめたものです。
そのため、下に挙げた第三者のライブラリが実行ファイルの中に入っています。
それぞれの著作権は各プロジェクトに属し、各ライセンスの条件で配布されています。

この一覧は、実際に出来上がった実行ファイルの中身から機械的に作っています
(`dev/make_notices.py`)。手で書き足したものではありません。

"""

TAIL = """
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
"""


def build():
    dists = bundled()
    used = set()
    body = []
    for title, names in GROUPS:
        rows = [(d, version_of(d), licence_of(d))
                for d in names if d in dists]
        if not rows:
            continue
        used |= {d for d, _v, _l in rows}
        body.append("### %s\n" % title)
        body.append("| ライブラリ | 版 | ライセンス |")
        body.append("|---|---|---|")
        for d, v, lic in sorted(rows):
            body.append("| %s | %s | %s |" % (d, v, lic))
        body.append("")
    rest = sorted(dists - used)
    if rest:
        body.append("### そのほか(上のライブラリが内部で使うもの)\n")
        body.append("| ライブラリ | 版 | ライセンス |")
        body.append("|---|---|---|")
        for d in rest:
            body.append("| %s | %s | %s |" % (d, version_of(d), licence_of(d)))
        body.append("")

    text = HEAD + "\n".join(body) + TAIL
    return text, len(dists)


def main():
    text, n = build()
    if "--check" in sys.argv:
        old = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        same = old.replace("\r\n", "\n") == text
        print("%s (%d 配布物)" % ("最新です" if same else "!! 古くなっています", n))
        raise SystemExit(0 if same else 1)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print("wrote %s  (%d 配布物)" % (OUT, n))


if __name__ == "__main__":
    main()

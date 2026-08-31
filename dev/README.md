# dev/ — zahyou_v6.ipynb の材料とテスト

`zahyou_v6.ipynb` はここのファイルから **生成** している。ノートブックを直接編集すると
次の生成で上書きされるので、直すときはこちらを直すこと。

`build_nb.py` は **2 つの成果物**を書き出す。

* `zahyou_v6.ipynb` … markdown + 3 つの短いセル（合計 90 行ほど）
* `zahyou_engine.py` … 解析の中身（約 1570 行）。セル A がこれを読み込む

**なぜ分けたか**: 1500 行をノートブックに貼ると、画像を選ぶ場所と結果が出る場所が
遠く離れてスクロールが大変になる。cell metadata の `jupyter.source_hidden` で
畳もうとしたが、**VS Code はこれを無視する**（実機で確認）。VS Code には
「畳んだ状態で配る」しくみが無いので、構造の側で短くした。

| ファイル | 中身 |
|---|---|
| `nb_loader.py` | セル A（pip 行 + エンジンの読み込み） |
| `nb_pick.py` | セル B（`pick_image()` の 1 行） |
| `nb_settings.py` | セル C 冒頭のユーザー設定項目 |
| `nb_ui.py` | 日本語フォント設定・画像選択ボタン（エンジンに入る） |
| `zahyou_core.py` | 画像読み込み・前処理・星検出・スケール推定・solve-field 呼び出し・目標の名前解決 |
| `zahyou_plot.py` | WCS の整形・描画・ずれの計算 |
| `nb_main.py` | `run()` 本体 |
| `build_nb.py` | 上を組み立てて .ipynb と zahyou_engine.py を書き出す |

```
python build_nb.py               # dev/zahyou.ipynb と dev/zahyou_engine.py を生成
#                                  → リポジトリ直下へコピーする
python test_robust.py            # いろいろな画像で星が検出できるか (31 ケース)
python test_offline_sim.py       # solve-field を stub で置き換えてオフライン経路を検証 (5 ケース)
python test_wsl_e2e.py           # 本物の WSL + solve-field で端から端まで (7 ケース)
python test_notebook_run.py      # 本物のカーネルでセル A/B を実行し、画面に出るものを数える
```

`test_offline_sim.py` は `../work/xyls/solution.wcs`（既知の正解 WCS）を参照する。
手元に無ければ、一度オンラインで解いて出力された `.wcs` を置けばよい。

---

## 星図データ (index files) の管理

astrometry.net の index ファイルは **末尾 2 桁が対応する画角の段**を表す。

| 段 | 画角 | ASI290MM (1936px / 2.9µm) での焦点距離 |
|---|---|---|
| `index-5200` | 2′ 〜 2.8′ | 6890 〜 9650 mm |
| `index-5201` | 2.8′ 〜 4′ | 4830 〜 6890 mm |
| `index-5202` | 4′ 〜 5.6′ | 3450 〜 4830 mm |
| `index-5203` | 5.6′ 〜 8′ | 2400 〜 3450 mm |
| `index-5204` | 8′ 〜 11′ | 1750 〜 2400 mm |
| `index-5205` | 11′ 〜 16′ | 1200 〜 1750 mm |
| `index-5206` | 16′ 〜 22′ | 880 〜 1200 mm |
| `index-4107` 〜 `4119` | 22′ 〜 2000′ | 〜 880 mm |

画角 = 画像の横幅[px] × 画素スケール[″/px] ÷ 60
画素スケール[″/px] = 206.265 × 画素サイズ[µm] ÷ 焦点距離[mm]

### 取得

```
python fetch_index.py 5204 5203 5202          # C:\AstrometryData へ
python fetch_index.py --dest D:\idx 5201      # 置き場所を変える
python fetch_index.py --check 5202 5203 5204  # 検証だけ
```

- 5200 LITE シリーズ (Tycho-2 + Gaia DR2) を portal.nersc.gov から取る
- 各段 48 ファイル（nside=2 の HEALPix で全天を分割）
- 途中で止めても Range ヘッダで続きから再開する
- 落とし終えたら「サイズ一致」と「FITS として開けて `INDEXID` / `HPNSIDE` /
  `NQUADS` が正しい」まで確かめる。壊れていれば消して次回取り直す

段ごとの実サイズ:

| 段 | 実サイズ | zip 後 |
|---|---|---|
| 5200 | 16.0 GB | 約 12 GB |
| 5201 | 9.4 GB | 約 7 GB |
| 5202 | 4.5 GB | 3.4 GB |
| 5203 | 2.3 GB | 1.7 GB |
| 5204 | 1.2 GB | 0.87 GB |

### 再配布用の zip

```
python make_index_zips.py 5204 5203 5202 --out C:\AstrometryData\dist
```

段ごとに `AstrometryData-index-<段>.zip` を作る。中身は `AstrometryData/index-....fits`
なので、`C:\` 直下に展開すればマニュアル 2.3 の配置になる。
段ごとの README と `SHA256SUMS.txt` も一緒に出力する。

### WSL 側の設定

`/etc/astrometry.cfg` に `autoindex` と `add_path` があれば、
足したファイルは自動で認識される。追加設定は要らない。

```
inparallel
cpulimit 300
autoindex
add_path /mnt/c/AstrometryData
```

### /mnt/c と ext4 のどちらに置くか

**ふつうは `/mnt/c` のままでよい。** 実測 (Ubuntu 26.04 / index 253 ファイル):

| | 96 MB の読み込み | 1 枚の解析 (キャッシュを消して) |
|---|---|---|
| `/mnt/c` (Windows 側) | 0.25 秒 | **約 3 秒** |
| ext4 (WSL 内) | 0.03 秒 | — |

生の読み出しは 10 倍違うが、solve-field が実際に読むのは星図データの
ごく一部なので、解析時間の差はほとんど出ない。
どうしても詰めたいときだけコピーする (WSL の VHDX が 10 GB ほど増え、
空けても自動では縮まない):

```
cp -r /mnt/c/AstrometryData ~/AstrometryData
sudo sed -i "s#/mnt/c/AstrometryData#$HOME/AstrometryData#" /etc/astrometry.cfg
```

---

## WSL のセットアップ

管理者 PowerShell で `wsl --install` → 再起動 → Ubuntu の初期設定 を済ませたあと、
残り（astrometry.net の導入・astrometry.cfg・星図データの配置・疎通確認）を
まとめて行う:

```
powershell -ExecutionPolicy Bypass -File setup_wsl.ps1 -CopyIndexToWsl
```

`-CopyIndexToWsl` は省略してよい（上記のとおり効果はほぼ無い）。
`-TestImage` を付けると、最後に `test_wsl_e2e.py` で実際に 1 枚解いて確かめる。

## マニュアル PDF

```
python make_manual.py                 # ../天体画像解析プログラム「zahyou」導入・運用マニュアルv3.pdf
```

| ファイル | 中身 |
|---|---|
| `make_manual.py` | 本文と構成 |
| `manual_style.py` | フォント・配色・見出し・表・囲みなどの部品 |
| `manual_figures.py` | セル構成図・記号凡例（その場で描く図） |
| `fonts/` | Noto Sans JP の静的ウェイト（SIL OFL / 埋め込み制限なし） |
| `assets/` | 本文に貼る結果画像 |

見出しは `h1()` / `h2()` が目次用の目印を仕込むので、`multiBuild` で
ページ番号つきの目次が自動生成される。

---

`test_notebook_run.py` には `nbclient` が要る (`pip install nbclient`)。
「セルの最後の式の値も自動表示される」ため、`pick_image()` がウィジェットを返すと
画面に 2 つ並ぶ。目視でしか気づけない類なので、このテストで機械的に数えている。

---

必要なければ `dev/` ごと消してよい（`zahyou_v6.ipynb` と `zahyou_engine.py` だけで動く）。

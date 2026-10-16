# dev/ — zahyou_v6.ipynb の材料とテスト

`zahyou_v6.ipynb` はここのファイルから **生成** している。ノートブックを直接編集すると
次の生成で上書きされるので、直すときはこちらを直すこと。

`build_nb.py` は **2 つの成果物**を書き出す（引数なしで走らせると、リポジトリ直下の 2 つを書き換える）。

* `zahyou_v6.ipynb` … markdown + 3 つの短いセル（合計 90 行ほど）。直下のものは**セルの中身だけを id ごとに
  差し替え**、出力の例は残す
* `zahyou_engine.py` … 解析の中身（オフライン解析の自前のエンジンを含む）。セル A がこれを読み込む。
  GUI・exe・`test_solver_real.py --engine` も直下のこれを使う

あわせて試験用に `dev/zahyou.ipynb` と `dev/zahyou_engine.py` も書く（git には入れない。`test_notebook_run.py` が使う）。
直下に残っている `zahyou.ipynb` は WSL 時代の別物で、生成物ではない。

**なぜ分けたか**: 1500 行をノートブックに貼ると、画像を選ぶ場所と結果が出る場所が
遠く離れてスクロールが大変になる。cell metadata の `jupyter.source_hidden` で
畳もうとしたが、**VS Code はこれを無視する**（実機で確認）。VS Code には
「畳んだ状態で配る」しくみが無いので、構造の側で短くした。

| ファイル | 中身 |
|---|---|
| `nb_loader.py` | セル A（pip 行 + エンジンの読み込み） |
| `nb_pick.py` | セル B（`pick_image()` の 1 行） |
| `nb_settings.py` | セル C 冒頭のユーザー設定項目（星図データの置き場所 `INDEX_DIR` もここ） |
| `solver_wcs.py` | 幾何: 単位ベクトル ↔ 赤経赤緯、TAN 投影、TAN/SIP の当てはめ、FITS ヘッダー |
| `solver_index.py` | 星図データ（索引ファイル）: FITS の表を生のバイトで読む、kd-tree の探索、段とタイルの選び方、検査 |
| `solver_core.py` | 解法: 星の並べ替え、quad 作り、コード、照合、検証（対数オッズ）、改良、探索の本体 `sv_solve` |
| `solver_synth.py` | 試験用: 索引の星から「正解つきの合成星野」を作る（エンジンには入れない） |
| `zahyou_core.py` | 画像読み込み・前処理・星検出・スケール推定・オフライン解析の段取り（`solve_offline` → `sv_solve`）・オンライン解析・目標の名前解決 |
| `zahyou_plot.py` | WCS の整形・描画・ずれの計算 |
| `nb_ui.py` | 日本語フォント設定・画像選択ボタン（エンジンに入る） |
| `nb_main.py` | `run()` 本体 |
| `build_nb.py` | 上を組み立てて .ipynb と zahyou_engine.py を書き出す |

**この版（v7-dev）で消したもの**: `setup_wsl.ps1`（WSL へ astrometry.net を入れる）、
`test_wsl_e2e.py`（本物の WSL の solve-field で解く）、`test_offline_sim.py`（solve-field をスタブに替えた試験）。
エンジンからは WSL を呼ぶ関数（`_run`・`_bash`・`wsl_available`・`to_solver_path`・`run_solve_field`・
`_wcs_from_dir`・`_clean_outputs`）と、solve-field に渡すファイルを書く `write_xylist`・`write_solver_fits` を消した。

## 組み立ての順と約束

`build_nb.py` は

    solver_wcs → solver_index → solver_core → zahyou_core → zahyou_plot → nb_ui → nb_main

の順につないで `zahyou_engine.py` を作る（ノートブック版と GUI 版が同じ 1 ファイルを使う形は変えない）。
`solver_synth.py` は試験用なので入れない。

1 つの名前空間につなぐので、`solver_*.py` は次を守る。

- 冒頭は「`#` のコメント → import → 本体」の順。**モジュールの docstring を置かない**
  （`strip_module_head` が外すのは冒頭のコメント・import だけ）
- ほかの solver モジュールの名前は冒頭の `from solver_wcs import *` などで取り込む（組み立てのときに消える）
- 最上位の名前は `sv_` / `Sv` / `_sv_` / `SV_` で始める（`zahyou_core` などの名前とぶつけない）
- numpy と標準ライブラリだけを使う（scipy・astropy は使わない。別の言語へ移しやすく、依存も増やさない）
- 画素の座標は 0 始まり（x = 列、y = 行。`detect_sources` と同じ）。FITS のヘッダーに書くときだけ CRPIX に 1 を足す

設計は [../docs/design-native-solver.md](../docs/design-native-solver.md)、
索引ファイルの形式は [../docs/index-format.md](../docs/index-format.md)（実物を読んで調べたもの）。

## 試験

```
python build_nb.py               # 直下の zahyou_engine.py・zahyou_v6.ipynb (セルの中身だけ) と、
#                                    試験用の dev/zahyou.ipynb・dev/zahyou_engine.py を書く
python test_solver_wcs.py        # solver_wcs:   TAN の往復・当てはめ・FITS ヘッダー
python test_solver_index.py      # solver_index: 索引の読み込み・kd-tree の探索・ファイルの検査
python test_solver_core.py       # solver_core:  quad 作り・コード・照合・検証・改良
python test_solver_synth.py      # 合成星野をブラインドとヒントで解く
python test_solver_real.py       # 実写を解いて、v6 で求めた正解と比べる
python test_solver_real.py --engine --both   # 同じことを、組み立てた直下の zahyou_engine.py と sv_solve 直接で
python test_robust.py            # いろいろな画像で星が検出できるか (38 ケース)
python test_online_settings.py   # オンライン解析が nova へ送る中身 (公開しない指定 publicly_visible="n" など)。
#                                    astroquery の送信を偽物に差し替えるのでネットには出ない
python test_notebook_run.py      # 本物のカーネルでセル A/B を実行し、画面に出るものを数える
```

`test_solver_*.py` はどれも単体で走らせられる（`test_robust.py` と同じ書き方）。
項目ごとに PASS / FAIL を出して最後に集計し、1 つでも失敗があれば終了コード 1 で終わる。
どの試験も WSL は使わない。星図データを読む試験は既定の置き場所（`C:\AstrometryData`）を使う。

| 試験 | 中身 | 合格の基準 |
|---|---|---|
| `test_solver_index.py` | 索引の読み込み（コードを星から計算し直して保存値と整数まで一致）、kd-tree の探索 = 総当たり | 全件一致 |
| `test_solver_synth.py` | 索引の星から作った合成星野（5202〜5206・4107〜4110・三角形の 4111〜4119、回転・鏡像・スケール・タイル境界・赤経 0°・極付近、星の欠け 30%・偽の星 30%）をブラインドとヒントで解く | 中心 2″（か 0.5 px）以内、スケール 0.2%、回転 0.1°、鏡像一致。一様乱数の星野は 1 つも解けない |
| `test_solver_real.py` | v6 の WSL 版で解いた正解（`solver_truth.json`）と同じ画像を解く（画像が手元にあるときだけ。パスはリポジトリからの相対か `$ZAHYOU_TEST_IMAGES/` から。陽性の画像が 1 枚も走らなければ終了コード 2） | 中心 2″、スケール 0.2%、回転 0.1°、鏡像一致。ピンぼけ・光条・乱数の星野は解けず、締め切りの超過 1 秒以内 |
| `test_robust.py` | 読み込み・前処理・検出（図の星と、解析エンジンに渡す深い検出の星） | 38 ケース（うち 3 つは手元に実写があるときだけ走る。置き場所は `ZAHYOU_TEST_IMAGES`（既定はホームフォルダー）の下の `Downloads`） |

画面の試験（`gui/test_*.py`）は [../gui/README.md](../gui/README.md) にある。

### 別の言語へ移すときの備え

この Python 版は、次の段階で別の言語（Rust など）へ移すときの**基準実装**も兼ねる。

- `sv_solve(..., trace=path)` で、照合した quad・当てはめた WCS・検証の対数オッズを JSON に書き出せる。
  移植先の試験は、同じ入力でこの値と一致するかを見る
- 合成星野（`solver_synth.py`）は乱数の種を固定し、作った星のリストも JSON で残す（移植先で同じ入力を使う）

---

## 星図データ (index files) の管理

astrometry.net の index ファイルは **末尾 2 桁が対応する画角の段**を表す。
zahyou は v6 と同じ物をそのまま読む（作り直さない）。

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

### エンジンからの読み方

置き場所のフォルダー（既定 `C:\AstrometryData`。ノートブック版はセル C の `INDEX_DIR`、
GUI は［準備］タブの欄）の `index-*.fits` を `SvIndexCatalog` が走査し、
ヘッダーを検査する（`sv_validate_index_file`: 大きさ・`INDEXID` と名前・`HEALPIX` とタイル番号）。
表は使うときに mmap し、整数のまま kd-tree をたどる（全天を探すときも全部を実数に展開しない）。
設定ファイルは要らない。v6 の WSL の `/etc/astrometry.cfg`（`add_path`）はもう使わない。

### 再配布用の zip

```
python make_index_zips.py 5204 5203 5202 --out C:\AstrometryData\dist
```

段ごとに `AstrometryData-index-<段>.zip` を作る。中身は `AstrometryData/index-....fits`
なので、`C:\` 直下に展開すればマニュアル 2.3 の配置になる。
段ごとの README と `SHA256SUMS.txt` も一緒に出力する。
中身の星のデータには元の星表（Tycho-2 / Gaia DR2）の条件がかかる
（[../THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md)）。zip の中の README には、その条件
（Gaia は CC BY-NC 3.0 IGO・商用は ESA の許可）・クレジット ESA/Gaia/DPAC・Gaia の定型の謝辞・論文を入れてある。
**README を直したら zip を作り直して、配布ページの zip も差し替える**（人の作業）。

## マニュアル PDF

```
python make_manual.py                 # ../天体画像解析プログラム「zahyou」導入・運用マニュアルv3.pdf
```

マニュアルの本文は v6（WSL を使う版）のまま。試作版に合わせた書き直しはまだしていない。

| ファイル | 中身 |
|---|---|
| `make_manual.py` | 本文と構成 |
| `manual_style.py` | フォント・配色・見出し・表・囲みなどの部品 |
| `manual_figures.py` | セル構成図・記号凡例（その場で描く図） |
| `fonts/` | Noto Sans JP の静的ウェイト（SIL OFL / 埋め込み制限なし） |
| `assets/` | 本文に貼る結果画像 |

見出しは `h1()` / `h2()` が目次用の目印を仕込むので、`multiBuild` で
ページ番号つきの目次が自動生成される。

## 同梱物の表示

```
python make_notices.py                # ../THIRD-PARTY-NOTICES.md を書き出す
python make_notices.py --check        # 中身が古くなっていないか見るだけ
```

ライブラリの表は、建てた exe の TOC（`gui/build/onefile/zahyou/Analysis-00.toc`）から作る。
表の前後の文（出典・星図データの条件・Gaia の謝辞）は `make_notices.py` の `HEAD` / `TAIL` に書いてあるので、
NOTICES を直すときはそちらを直してから書き出すこと。

---

`test_notebook_run.py` には `nbclient` が要る (`pip install nbclient`)。
「セルの最後の式の値も自動表示される」ため、`pick_image()` がウィジェットを返すと
画面に 2 つ並ぶ。目視でしか気づけない類なので、このテストで機械的に数えている。

---

必要なければ `dev/` ごと消してよい（`zahyou_v6.ipynb` と `zahyou_engine.py` だけで動く）。

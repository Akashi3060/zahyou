# zahyou デスクトップ版 (exe)

ノートブックを開かずに、**exe をダブルクリックするだけ**で使える版です。
解析そのものはノートブック版とまったく同じ ([`zahyou_engine.py`](../zahyou_engine.py)
を読み込んで動かしています)。

```
gui/
├── zahyou_gui.py    画面。1 ウィンドウ・3 タブ
├── zahyou_env.py    WSL / astrometry.net / 星図データ の確認と導入
├── zahyou.spec      PyInstaller の設定
├── build_exe.ps1    exe を建てるスクリプト
└── test_gui.py      画面を組み立てて解析を 1 回通すテスト
```

## 画面

| タブ | 中身 |
|---|---|
| **解析** | 画像を選ぶ / 目標を打ち込む / 実行 → 大きな数字と図 2 枚 |
| **準備** | WSL・astrometry.net・星図データ を「まとめて準備する」で用意 |
| **使い方** | 短い手引き |

結果は画面の上に 3 つの数字で出ます — **中心から目標までの距離**、**赤経方向**、
**赤緯方向**。その下のタブで「撮影時の向き」「北が上」の図と、解析ログを見られます。
図の下のボタン（全体 / 戻る / 進む / 移動 / 拡大 / 画像を保存）で拡大や保存ができます。

目標に **「指定しない (完全ブラインド)」** を選ぶと、目標を決めずに
「この画像はどこを向いているか」だけを求めます。
上の 3 つの数字は **画像中心の赤経・赤緯・画素スケール** に変わります。

### 暗いところで使う

右下の **「🌙 暗く」** を押すと、黒基調の配色に切り替わります。
画面・タイトルバー・**図の中まで**まとめて暗くなるので、観測中に目が眩みません。
もう一度押すと戻ります。選んだ配色は次に開いたときも残ります。

設定 (目標・焦点距離・解析方法・配色) は `%LOCALAPPDATA%\zahyou\gui.json` に
覚えるので、次に開いたときは前回のままです。
`%LOCALAPPDATA%` は `C:\Users\（ログイン名）\AppData\Local` のこと
(例: `C:\Users\taro\AppData\Local\zahyou\gui.json`)。
`AppData` は隠しフォルダーなので、エクスプローラーのアドレス欄に
`%LOCALAPPDATA%\zahyou` と打つのが早いです。
[使い方] タブには、その PC での実際の場所が出ます。

## 使うだけの人へ

**建てた PC 以外で初めて実行すると、SmartScreen の警告が出ます。**
「Windows によって PC が保護されました」→ **「詳細情報」→「実行」**。
exe に署名を付けていない (コード署名証明書が有料) ためで、
ダウンロードしたファイルに付く印 (Mark of the Web) と署名なしの組み合わせで
Windows が警告します。建てた PC でその印が付かないので、開発中は気づけません。
本物か確かめたい場合は `Get-FileHash .\zahyou.exe -Algorithm SHA256` を
Releases に載せた値と見比べてください。

1. `zahyou.exe` をダブルクリック (インストール不要。初回は起動に 4 秒ほど)
2. **[解析]** タブで「参照...」→ 画像を選ぶ
3. 目標の天体名 (例 `UCAC4 660-021020`) か、赤経・赤緯を入れる
4. 「解析を実行」

インターネットがあればこれだけで動きます。**ネットの無いところで使うなら**、
先に **[準備]** タブで「まとめて準備する」を押してください。
WSL から星図データの設定まで、足りないものを順に用意します。
実測では、WSL を消した状態から **76 秒・UAC なし・再起動なし**で
「オフライン解析できます」になりました
（Windows の機能としての WSL が入っていない PC では、
UAC と再起動が 1 回だけ要ります）。

## 入れ替える / やめる

新しい版は **`zahyou.exe` を上書きするだけ**。設定 (`%LOCALAPPDATA%\zahyou`) も
覚えた座標もそのまま引き継がれる。WSL と星図データは作り直さなくてよい。

やめるときに消すのは 4 か所 ― `zahyou.exe` /
`%LOCALAPPDATA%\zahyou` (設定と記憶) / `C:\AstrometryData` (星図データ) /
WSL の中の astrometry.net (`sudo apt remove --purge astrometry.net`、
設定は `/etc/astrometry.cfg.bak-zahyou` から戻せる)。
WSL 自体はほかの用事にも使うので、消す前に確かめること。

## 建て方

```powershell
powershell -ExecutionPolicy Bypass -File build_exe.ps1 -Test
```

| 指定 | 出来るもの | 大きさ | 起動 |
|---|---|---|---|
| 既定 | `dist\onefile\zahyou.exe` | 102 MB | 4.1 秒 |
| `-Folder` | `dist\zahyou\` (+ zip) | 221 MB | 2.0 秒 |

`-Test` を付けると、建てた exe で自己診断 (実際に画像を 1 枚解く) まで走ります。
`-Clean` で `build` / `dist` を消してから建て直します。

足りない Python パッケージは自動で入れます (PyInstaller を含む)。

## 引っかかったところ

固めるときに踏んだもの。**同じ罠に戻らないように残しておきます。**

- **`collect_all("astropy")` / `collect_all("astroquery")` を使ってはいけない。**
  「パッケージの全サブモジュール」を hiddenimports に足すので、テスト用・任意依存の
  import までたどり、**torch と cv2 まで引きずり込んで 4.4 GB になった**。
  要るモジュールだけを名指しし、データファイルは `collect_data_files` で拾う → 221 MB。
- **`astropy.tests` を excludes に入れてはいけない。** `astropy/__init__.py` が
  `astropy.tests.runner` を import するので、astropy ごと落ちる
  (`ModuleNotFoundError: No module named 'astropy.tests.runner'`)。
- **`reproject` は import しただけで `dask` を要求する。** 「北が上」の図で使うので、
  dask を外すと解析は成功するのに図だけ出ない、という分かりにくい壊れ方をする。
- **`photutils` はデータファイルまで入れる。** `astroquery.astrometry_net` が
  読み込み、`photutils/__init__.py` が自分の `CITATION.rst` を開くので、
  `.py` だけ入れると **オンライン解析だけが** `FileNotFoundError` で落ちる
  (オフラインへ勝手に切り替わるので、動いているように見えてしまう)。
- **エンジンは実行時に `exec` で読む**ので、PyInstaller の静的解析からは
  中の import が見えない。`zahyou.spec` の `hiddenimports` が命綱。
- **Tk の変数 (`StringVar` など) をワーカースレッドから読んではいけない。**
  例外にならず黙って失敗するので、原因が分からなくなる。
  ワーカーが見るのは素の属性 (`self.index_dir`) にしてある。
- **`except Exception as e:` の `e` は except 節を抜けると消える。**
  `self._post(lambda: self._fail(e, tb))` と書くと後で `NameError` になり、
  **失敗の中身が画面にもログにも出なくなる**。既定値で束縛して渡すこと。
- **matplotlib のツールバーは暗い配色で使えない。** アイコンが「黒い絵」なので、
  背景を暗くすると何も見えなくなる (matplotlib は *ボタンの背景色* が暗いときだけ
  アイコンを塗り替えるが、tk のボタンは親の色を継がないので効かない)。
  `NavigationToolbar2Tk` は状態機械としてだけ借りて、見た目は ttk で作り直した。
- exe は `--windowed` なので `print` の出し先が無い。困ったときは:

```powershell
zahyou.exe --engine-check --out engine.txt                     # エンジンだけ読む
zahyou.exe --selftest "画像パス" --out selftest.txt            # 解析まで通す
```

`--engine-check` は 45 秒ごとに faulthandler でスタックを吐くので、
「返ってこない」ときにどこで止まっているか分かります。

## テスト

```powershell
python test_gui.py                    # ソースのまま
dist\onefile\zahyou.exe --selftest "画像パス" --out selftest.txt            # 固めたあと
dist\onefile\zahyou.exe --selftest "画像パス" --out selftest.txt --online   # 経路を指定
```

どちらも同じ `zahyou_gui.selftest()` を呼びます (exe には test_gui.py が
入らないので、本体側に置いてあります)。`--online` / `--offline` で解析の経路を
指定できます。固めた exe での実測 14/14:

```
  PASS  画面が組み立てられる
  PASS  エンジンを読み込める
  PASS  環境を調べられる     Ubuntu 26.04 LTS
  PASS  解析が終わる       9.2 秒 / オフライン (WSL) のみ
  PASS  距離が出る        1.583′
  PASS  赤経方向が出る      0.918′ 東
  PASS  赤緯方向が出る      1.290′ 北
  PASS  図が 2 枚貼られる   2 枚
  PASS  画像中心などが出る    画素スケール 1.4616″/px  視野 23.58′ × 13.35′
  PASS  ログが流れている
  PASS  目標なしでも解ける    画像中心 (赤経) = 4h07m33.9s
  PASS  向きが数字で出る     画像中心 (赤緯) = +41d57m53s / 画素スケール = 1.462″/px
  PASS  ボタンが見切れていない
  PASS  配色を切り替えられる   最後は dark
```

オンライン経路 (`--online`) でも 13.6 秒で同じ答えでした。
ノートブック版とも一致します。

「ボタンが見切れていない」は、入れ物が狭くて Tk がラベルを削っていないかを
`winfo_width()` と `winfo_reqwidth()` の差で見ています
(**画面を見ないと気づけない類の不具合なので機械で見る**。
実際に「必要な段を選ぶ」が「必要な段を選.」になっていた)。

## 星図データについて

[準備] タブは画角ごとの段を選んで落とせます。焦点距離とセンサー横幅を入れて
「必要な段を選ぶ」を押すと、要る段だけに印が付きます
(例: 焦点距離 800 mm / センサー横幅 7.4 mm → 画角 31.8′ → index-4107〜4109)。

**[解析] タブで画像を選んでおくと、その画像の縦横も読んで「短辺」まで含めて
選びます。** 掩蔽観測ではローリングシャッターの影響を抑えたりコマ落ちを防ぐために
読み出す範囲 (特に縦) を切り詰めるので、横幅は同じでも短辺の画角だけが
極端に狭くなります。星の並び (四角形) は短いほうの辺に収まる大きさまでしか
作れないので、**必要な段は短辺で決まります**。
例えば ASI290MM を 1936 x 400 画素に切り出すと、横 24′ に対して縦は 5.0′ ―
横だけ見ると index-4107 で足りるように見えて、実際には index-5202 まで要ります。

取得先は astrometry.net の公式配布です。

- `index-41xx` … <http://data.astrometry.net/4100/> (画角 22′〜2000′、全部でも 320 MB)
- `index-52xx` … <https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/>
  (画角 2′〜22′、1 段あたり 48 ファイル。段によっては 20 GB を超える)

途中で中止しても、次に押せば続きから落とし直します (Range ヘッダで再開)。

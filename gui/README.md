# zahyou デスクトップ版 (exe)

ノートブックを開かずに、**exe をダブルクリックするだけ**で使える版です。
解析そのものはノートブック版とまったく同じ ([`zahyou_engine.py`](../zahyou_engine.py)
を読み込んで動かしています)。

この版 (`APP_VER = "v7-dev"`、試作) は、オフライン解析を**エンジンの中の自前のソルバー**
(`dev/solver_wcs.py` / `solver_index.py` / `solver_core.py`。`zahyou_engine.py` に組み込まれる)
で行います。WSL も astrometry.net の実行ファイルも使いません。星図データ (索引ファイル) は
v6 と同じ物を同じ場所に置きます。設計は [docs/design-native-solver.md](../docs/design-native-solver.md)。

```
gui/
├── zahyou_gui.py    画面。1 ウィンドウ・3 タブ
├── zahyou_env.py    星図データの取得と検査 (置き場所の走査・ヘッダーの検査)
├── zahyou.spec      PyInstaller の設定
├── build_exe.ps1    exe を建てるスクリプト
├── test_gui.py      画面を組み立てて解析を 1 回通すテスト
├── test_buttons.py  押せるボタンを全部押して、反応が返るか見る
├── test_result_tab.py  図の表示を覚えているか
└── test_recommend.py   「おすすめをまとめて落とす」の段の選び方
```

## 画面

| タブ | 中身 |
|---|---|
| **解析** | 画像を選ぶ / 目標を打ち込む / 実行 → 大きな数字と図 2 枚 |
| **準備** | 星図データを落として検査する (「まとめて準備する」で足りない段を落とす) |
| **使い方** | 短い手引き |

白地に黒い星の**星図**も、明暗が逆であることを見分けて自動で反転してから解きます
(判定は `looks_inverted()`。背景の中央値が明るさの幅の上端にあるかを見る。
パーセンタイルで裾を測ると、星が数十個しかない星図で見落とす)。

結果は画面の上に 3 つの数字で出ます — **中心から目標までの距離**、**赤経方向**、
**赤緯方向**。その下のタブで「撮影時の向き」「北が上」の図と、解析ログを見られます。
図の下のボタン（全体 / 戻る / 進む / 移動 / 拡大 / 画像を保存）で拡大や保存ができます。

図の表示は、はじめては「撮影時の向き」(撮ったままの向き)。
切り替えると次からはその表示で出ます (`result_tab` として覚えます)。

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
足りない星図データ (おすすめの段) を落として検査します。
一覧表 (「星図データ」「検査」「総合」の 3 行) の「総合」が
「オフライン解析できます」になれば用意は済んでいます。
解析のエンジンは exe の中にあるので、ほかに入れるものはありません。

## 入れ替える / やめる

**[使い方] タブの「更新を確認」→「更新する」**で入れ替わる。
GitHub の Releases API が資産ごとに `digest` (sha256) を返すので、
動いている exe のハッシュと突き合わせて判定する
(バージョン名だけだと「同じ版を建て直したもの」が分からない)。

**試作版 (`v7-dev`) は更新の確認をしない。** Releases の最新は v6 なので、
確かめると「新しい版がある」と判定して v6 の exe へ戻してしまう。
試作版は手で上書きして入れ替える。

入れ替えは動いている exe を自分で上書きできないので、
PowerShell に任せる ― 落とす → SHA256 で確かめる →
`zahyou.exe.new` として横に置く → アプリ終了を待つ →
今のものを `.bak` へ退避 → 置き換え → 起動し直す。
失敗したら `.bak` から戻すので、消えて無くなることはない。

**PowerShell へ渡す環境からは `_PYI_*` を落とす**(`child_env()`)。
1 ファイル版の exe は、中身を一時フォルダーへ展開してから自分自身を
もう一度起動する二段構えで、二段目であることが環境変数で伝わる。
そのまま渡すと**新しい exe まで「自分は二段目だ」と思い込み**、親が
powershell.exe で自分と違うという理由で

    Security validation failure: parent process has different executable!

を出して起動できない。入れ替え自体は済んでいるので手で起動し直せば
新しい版が上がるが、更新した人には「エラーが出た」ようにしか見えない。

しくみだけ確かめるなら:

```powershell
zahyou.exe --update-check --out check.txt     # 調べるだけ
zahyou.exe --update-apply --out apply.txt     # 入れ替えまで (起動し直さない)
```

手で入れ替えるなら `zahyou.exe` を上書きするだけ。
設定 (`%LOCALAPPDATA%\zahyou`) も覚えた座標もそのまま引き継がれる。
星図データは作り直さなくてよい (v6 と同じ物を同じ場所で使う)。

やめるときに消すのは 3 か所 ― `zahyou.exe` /
`%LOCALAPPDATA%\zahyou` (設定と記憶) / `C:\AstrometryData` (星図データ)。

v6 の [準備] タブが WSL へ入れた astrometry.net は、この版では使わない。
片付けはどちらか一方: (1) 消すなら `sudo apt remove --purge astrometry.net` と
`sudo rm /etc/astrometry.cfg.bak-zahyou` (purge で設定ファイルも消えるので書き戻さない)、
(2) astrometry.net をほかの用事に残すなら `sudo cp /etc/astrometry.cfg.bak-zahyou /etc/astrometry.cfg`
で設定だけ戻す (戻したら .bak は消してよい)。README の「v6 で WSL に入れた astrometry.net」と同じ。
WSL 自体はほかの用事にも使うので、`wsl --unregister` する前に、ほかに使っていないことを確かめること。

## オフライン解析のつなぎ方

- 解くのは同じプロセスの中 (`solve_offline` → `sv_solve`)。別のプログラムは起動しない。
- 星図データの置き場所は [準備] タブの欄 (`gui.json` の `indexdir`) を**エンジンへ直接渡す**
  (`_params` の `INDEX_DIR`)。v6 は WSL の `/etc/astrometry.cfg` を経由していた。
- 中止は `threading.Event` を立てて渡す (`CANCEL_EVENT`)。エンジンは quad の束を 1 つ処理するごと・
  検証 1 回ごとにこれと締め切りを確かめる。v6 は WSL の `solve-field` を `pkill` していて、ほとんど効かなかった。
  オンライン解析 (`solve_online`) も同じ合図を 0.5 秒ごとに見て、立てば戻る。中止のあとに画像そのものを
  nova へ送ることはしない (以前は星の位置で解けないと、中止したあとでも画像を送っていた)。
- nova へ送るときは、星の位置でも画像でも `publicly_visible = "n"` (公開しない) を付ける (`solve_online` の
  `settings`)。付けないとサーバーの既定で公開になり、API キーの持ち主のアカウント名で一覧などに載る
  (astroquery も既定値を自分では付けない)。"n" でも完全には隠れない可能性があるので、[使い方] タブと README には
  「nova に送ったものは第三者に見られることがある」と書いてある。送る中身は `dev/test_online_settings.py` が
  astroquery の送信を偽物に差し替えて確かめる (ネットには出ない)。
- 解析が終わるたびに、エンジンは索引ファイルの mmap を放す (目録は覚えたまま)。GUI を開いたままでも、
  Explorer やほかのプロセスから星図データを消したり置き換えたりできる。
- 解析の方法の選択肢は「オフラインのみ」。`gui.json` に残っている v6 の「オフライン (WSL) のみ」はそのまま読み替える。

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
python test_gui.py                    # 画面と解析 1 回
python test_buttons.py                # ボタンの手応え
python test_result_tab.py             # 図の表示の記憶
python test_recommend.py              # おすすめの段の選び方
dist\onefile\zahyou.exe --selftest "画像パス" --out selftest.txt            # 固めたあと
dist\onefile\zahyou.exe --selftest "画像パス" --out selftest.txt --online   # 経路を指定
```

どれも WSL を使わない。オフラインで解く試験 (`test_gui.py`・`test_result_tab.py`) は
星図データ (既定 `C:\AstrometryData`) を読む。解き方そのものの試験は `dev/` 側にある
(`test_solver_*.py`。[dev/README.md](../dev/README.md))。

`test_recommend.py` は段の選び方そのものも見ています。段の「隣」を
**番号順ではなく画角の順**で数えているか、が要点 —— 4100 系 (22′〜2000′) と
5200 系 (2′〜22′) は別のはしごなので、番号で並べると 4119 (1400′〜2000′) の
隣が 5200 (2′〜2.8′) になります。実際、35mm レンズの画角 (54°) に対して
15.9 GB の 5200 を勧めていました。

どちらも同じ `zahyou_gui.selftest()` を呼びます (exe には test_gui.py が
入らないので、本体側に置いてあります)。`--online` / `--offline` で解析の経路を
指定できます。確かめる項目は、画面が組み立てられる / エンジンを読み込める /
環境 (星図データ) を調べられる / 解析が終わる / 距離・赤経方向・赤緯方向が出る /
図が 2 枚貼られる / 画像中心などが出る / ログが流れている / 目標なしでも解ける /
向きが数字で出る / ボタンが見切れていない / 配色を切り替えられる。

v6 の固めた exe では全部通り、オンライン経路とも、ノートブック版とも同じ答えでした。
試作版 (v7-dev) は、2026-10-16 に Core Ultra 7 265K (20 論理コア)・Python 3.13 で
`python gui/zahyou_gui.py --selftest dev/realcase/real16.fits --offline` を走らせて 17/17 通り、
解析は 2.6 秒 (状態バーの表示は 2.2 秒。機材の画素スケールを覚えている状態) でした
(exe ではなく Python から。exe は解析エンジンを直したあと建て直していません)。

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
落とすあいだは `.part` に書き、終わってから名前を変えて、ヘッダーを検査します
(大きさがヘッダーから計算した長さと合うか、`INDEXID` とファイル名、`HEALPIX` とタイル番号。
`sv_validate_index_file()`)。「状態を確認」も置き場所の `index-*.fits` を全部この検査にかけます。

星図データの形式は [docs/index-format.md](../docs/index-format.md)、
元になった星表 (Tycho-2 / Gaia DR2) の条件は [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md) にあります。

# zahyou の Rust 版(rs/)

天体写真から望遠鏡の向き(画像の中心と目標までのずれ)を割り出す zahyou を Rust で作り直したもの。
設計と進み具合は [../docs/rust-port.md](../docs/rust-port.md)、解き方(基準実装の Python 版)は
[../docs/design-native-solver.md](../docs/design-native-solver.md)、索引ファイルの形式は [../docs/index-format.md](../docs/index-format.md)。

```
crates/zahyou-wcs     座標・TAN/SIP の WCS・当てはめ・FITS のカード
crates/zahyou-index   索引ファイル(astrometry.net の index-*.fits)の読み取りと探索(unsafe は mmap の 1 か所だけ)
crates/zahyou-image   画像の読み込み・前処理・星の検出(標準の検出と、解法に渡す深い検出)
crates/zahyou-solver  解法(quad 法)と、オフライン解析の試行のはしご(solve_offline)
crates/zahyou-plot    図(撮影時の向き・北が上)と PNG
crates/zahyou-net     更新・星図データのダウンロード・天体名の解決・nova.astrometry.net
app                   画面(eframe/egui)と、画面を出さない入口。exe は zahyou.exe
testdata              Python 基準実装から作った照合用の値(JSON)と、それを作るスクリプト(gen_*.py)
app/testdata          解析の流れ全体(本物の解法つき)を Python の run() と比べる値(gen_run_solver.py)
tools                 同梱物の表示(THIRD-PARTY-NOTICES.md)を作る・ライセンスを確かめるスクリプト
```

## 用意するもの

- MSYS2 の UCRT64 の Rust(`pacman -S mingw-w64-ucrt-x86_64-rust`、1.94 で確かめた。ターゲット `x86_64-pc-windows-gnu`、
  リンカーは UCRT64 の gcc)。**rustup は入れない**(rustup の gnu は MSVCRT 前提で、UCRT64 の gcc と混ぜると壊れる)。
- シェルは Git Bash。最初に `export PATH=/c/msys64/ucrt64/bin:$PATH`。
- 試験の一部(照合値の作り直しと Python 側の速さ)には Python 3.13 と numpy・scipy・astropy・matplotlib
  (`C:\Users\yoshi\AppData\Local\Programs\Python\Python313\python.exe`)。
- 星図データ `C:\AstrometryData`(index-*.fits)。無ければ、それが要る試験は `SKIP` と出して通す。

## ビルド

```bash
export PATH=/c/msys64/ucrt64/bin:$PATH
cd rs
cargo build --release -p zahyou          # target/release/zahyou.exe(依存はシステムの DLL だけ)
cargo clippy --workspace --all-targets -- -D warnings
```

`CARGO_TARGET_DIR` で置き場所を変えられる(何人かで同時にビルドするときは別々にする)。release は `lto = "fat"`・
`codegen-units = 1` なので 2〜3 分かかる。eframe は 0.33 系(0.36 は Rust 1.95 以上が要る)。

## 試験

```bash
cargo test --workspace                                   # 既定(3 分ほど)
cargo test --workspace -- --ignored --nocapture          # 重いもの・外に問い合わせるもの(15 分ほど)
cargo test -p zahyou run_solver_vs_python -- --nocapture # 解析の流れ全体を本物の解法で Python と比べる(18 場面)
cargo test -p zahyou run_solver_vs_python_all -- --ignored --nocapture   # 締め切りまで探す 2 場面も込みで 20 場面(1 分ほど)
```

- 試験の画像は `$ZAHYOU_TEST_IMAGES/…`(既定はホームフォルダー)か、リポジトリからの相対。リポジトリに無い
  `dev/realcase`・`dev/tcase`(.gitignore)は `ZAHYOU_PY_REPO`(既定 `C:\Users\yoshi\Documents\GitHub\zahyou`)の同じ場所を見る。
  中身(sha256)が照合値を作ったときと違えば SKIP。
- `--ignored` の中身: 解法の全部の合成星野・全部の実写・解けてはいけない画像(締め切り 30 秒)と、GitHub の Releases・
  nova.astrometry.net への疎通・星図データの置き場所への HEAD(読むだけ。画像も星のリストも送らない)。
  外へ問い合わせたくなければ `ZAHYOU_OFFLINE_TESTS=1`(天体名の試験を飛ばす)と `--skip live`。
- 解法の試験の細かい説明と環境変数は [crates/zahyou-solver/tests/README.md](crates/zahyou-solver/tests/README.md)。

### 利用者のファイルを書き換えない

画面・解析は `%LOCALAPPDATA%\zahyou\cache.json`(天体名の座標と機材の画素スケール)と `gui.json`(画面の設定)を
読み書きする。試験や exe を手で動かすときは、一時ファイルへ向けてから動かす:

```bash
T=$(mktemp -d)
export ZAHYOU_CACHE="$T/cache.json" ZAHYOU_SETTINGS="$T/gui.json" LOCALAPPDATA="$T"
```

`--selftest` と `--shot` は gui.json だけを自分で一時ファイルへ向ける。**cache.json は向けない**(Python 版の自己診断と
同じく、解けた画素スケールを覚える)ので、手で動かすときは上の 3 つを設定する。

## exe の入口

```text
zahyou.exe [--dx12|--glow] [--gpu display|low|high|warp]   画面(既定は DirectX 12・主の画面の GPU。開けなければ
                                              OpenGL → DirectX 12 のソフトウェア描画 WARP の順に開き直す)
zahyou.exe --selftest <画像> [--offline|--online] [--out f]   自己診断(解析を 2 回 + 画面の項目。25 項目)
zahyou.exe --shot <png> [--tab run|env|help] [--run] [--blind] [--image p] [--out-tab orig|northup|log]
           [--dark|--light] [--size WxH] [--left-end]   画面を自分で撮って閉じる(--left-end は［解析］の左の列を
                                              いちばん下 = API キーの欄まで送って撮る)
zahyou.exe --bench [--out f.json]             解析の段ごとの速さ(ZAHYOU_BENCH_IMAGES に ; 区切りで画像)
zahyou.exe --bench-gui [<画像>] [--out f]     画面まわりの速さ
zahyou.exe --bench-ui <画像> [--glow|--dx12] [--gpu …] [--mem lean|perf] [--no-vsync] [--cycles N] [--out f.json]
                                              描画方式の物差し(本物の窓で図を自動で動かす。docs/rust-port.md 9 節)
zahyou.exe --engine-check / --update-check [--out f]
隠し: --debug-wcs <.wcs>(オフライン解析の代わりにその WCS を解にする)・--first-frame <txt>・--no-startup-checks・
      --mem lean|perf・--no-vsync・--no-hw(OpenGL をハードウェア支援なしで開く)・--render-step N(開き直しが自分で付ける)
```

exe は windows サブシステムなので、結果はコンソールに出ないことがある。`--out` のファイルを見る。

- **オンライン解析の API キー**: ［解析］の「API キー」の欄(gui.json の `api_key`)。初めは作者の既定のキー
  (環境変数 ASTROMETRY_API_KEY があればそちら)が入っている。そのため「自動」は、ネットがあれば nova.astrometry.net に
  星の位置を送る(解けなければ画像も。v6 と同じ)。`--shot --run` で撮るときや試しに解かせるときに外へ送りたくなければ、
  一時の gui.json の `"solve"` を `"オフラインのみ"` にするか、`"api_key": ""` を書いておく。`--selftest --offline` は送らない。
- 版名は `Cargo.toml` の `[workspace.package] version` だけで決まる(`v` を付けたものが画面・ログ・更新の確認の版名。
  exe の版情報も同じ)。`-beta.N` などのプレリリースの版は窓の題・起動のログ・［使い方］に「β版」と添え、更新の確認は
  GitHub のプレリリースも含めて同じ大きな版(v7)の新しいものを探す。`-dev` で終わる版は更新を確かめない。

## 照合値を作り直す(Python 基準実装 = リポジトリの dev/)

```bash
PY=/c/Users/yoshi/AppData/Local/Programs/Python/Python313/python.exe
$PY -B testdata/gen_wcs.py ; $PY -B testdata/gen_index.py ; $PY -B testdata/gen_image.py
$PY -B testdata/gen_plot.py ; $PY -B testdata/gen_net.py ; $PY -B testdata/gen_pipeline.py
$PY -B testdata/gen_solver.py                 # 合成 6〜7 分・実写 10 分ほど
$PY -B app/testdata/gen_run_solver.py         # run() を本物の解法で 20 場面(3 分ほど。焦点距離の書き違いの場面が 2 分)
```

どれも利用者のファイルには触れない(cache.json は一時ファイル)。作り直したら `git diff` で変わった値を確かめる。

## 速さの物差し

```bash
cargo build --release -p zahyou && ZAHYOU_CACHE=... target/release/zahyou.exe --bench --out rust.json
$PY -B testdata/bench_pipeline.py             # Python 側(解析の段ごと)
cargo run --release -p zahyou-solver --example bench_solver -- --out rust.json --python py.json
$PY -B testdata/bench_solver.py --out py.json # Python 側(解法。同じ星・同じ条件)
cargo run --release -p zahyou-image --example bench_image   # 画像の読み込み・前処理・検出
```

Python と比べるときは、同じ機械で、ほかの重い処理(ビルドなど)が動いていないときに測る。

## 同梱物の表示とライセンス

```bash
python tools/check_licenses.py                # 依存のライセンスが許可リストの中か(GPL・LGPL・MPL などは失敗)
python tools/gen_notices.py --check           # THIRD-PARTY-NOTICES.md が依存と合っているか(古ければ 1)
python tools/gen_notices.py                   # 作り直す(Cargo.lock か Rust・gcc の版が変わったら)
python -B tools/test_tools.py
```

THIRD-PARTY-NOTICES.md は exe に埋め込んで、［使い方］タブから読める。

## 約束

- 自分で書く `unsafe` は `zahyou-index` の mmap だけ。ほかの crate は `#![forbid(unsafe_code)]`。
- 壊れた入力で panic しない(試験で切り詰めたファイル・壊したヘッダー・壊した索引を読ませる)。
- コメントは日本語。外部のクレートは許容型のライセンスだけ。C のライブラリへの FFI は入れない。

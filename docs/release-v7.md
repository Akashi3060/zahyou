# v7 を配る手順(β版 → 正式版)

2026-10-16 作成、同日に 2026-10-16 の決定(API キーの欄・DirectX 12 を既定・v7 は当面β版)と、2 回目の決定
(画像は同意を取らずに送ってよい・β の Release にも exe 以外を付ける)、3 回目の決定(nova へは公開しない指定を付けて送る)と、
GitHub の Release の扱いを公式の説明で確かめた結果(0.1 節)に合わせて書き直した(下書き)。
v6 までの手順(`gui/build_exe.ps1` = PyInstaller、`dev/make_notices.py`)を、cargo の release ビルドの exe・
`rs/THIRD-PARTY-NOTICES.md`・SHA256 の作り直しに置き換えたもの。
ビルドと試験の細かい説明は [../rs/README.md](../rs/README.md)、決定と実測は [rust-port.md](rust-port.md) の 9・10 節
(β版の出し方の元は 10.7 節)。

**v7 は当面β版(7.0.0-beta.N)を GitHub のプレリリースで出す。1〜2 か月使って安定を確かめてから、正式版 7.0.0 を
main に統合してふつうの Release で出す。** 1〜7 節がβ版、8 節が正式版。push・タグ・Release・配布ページの更新は、
どれもユーザーの了承のあとで行う。

---

## 0. 決まったことと、残っていること

決まったこと(2026-10-16。rust-port.md 10.1・10.9 節):

| | 決定 | 実装 |
|---|---|---|
| API キー | ［解析］タブの「解析」の枠に欄(伏せ字・「表示」・「元に戻す」)。**既定は元のキー(v6 と同じ)** | gui.json の `api_key` に平文で保存。空なら問い合わせない。環境変数 ASTROMETRY_API_KEY は gui.json に `api_key` が無いときの初めの値だけ。キーの値は文書・ログ・画面の文に書かない |
| 描画 | **DirectX 12 を既定**(主の画面の GPU・小さな塊)。開けなければ OpenGL → ソフトウェア描画(WARP) | 手で選ぶ引数 `--dx12` / `--glow` / `--gpu display\|low\|high\|warp`。想定は「Windows 11 が動く、RAM 8 GB 以上のノート PC」 |
| v6 からの移り方 | **v7 は当面β版。v6 の自動更新の対象にしない** | 版 7.0.0-beta.N をプレリリースで出す(v6 の更新係が見る latest に出ない)。β 自身は v7 の新しいβ版と正式版を探す |
| 画像を外へ送ること | **同意を取らずに送ってよい**(今の動きのまま。v6 と同じ) | 方法が「自動」か「オンラインのみ」で、キーがあり、ネットにつながっていると、まず検出した星の位置(と画像の大きさ・手がかり)を、それで解けなければ画像ファイルそのもの(ファイル名も)を nova.astrometry.net へ送る。送らない方法(方法を「オフラインのみ」/ API キーの欄を空)と一緒に、README・マニュアル 2.3・［使い方］タブに書いた |
| nova へ送るときの公開の扱い(3 回目) | **公開しない指定 `publicly_visible = "n"` を付ける**(Rust 版と Python 版(ノートブックを含む)の両方)。文書には「nova に送ったものは第三者に見られることがある」と書く | Rust: `zahyou_net::nova::PUBLICLY_VISIBLE`・`private_settings`(星の位置のリストと画像の両方の request-json の先頭に付ける。呼ぶ側が別の値を渡しても `"n"`)。Python: `dev/zahyou_core.py` の `solve_online` の設定(同じく先頭。native-solver で直し、あとで取り込む)。試験は偽の nova が受け取った本文と astroquery の記録(`rs/testdata/gen_net.py`)で。README・マニュアル 2.3・［使い方］タブ・rs/README.md に「公開しない指定を付けて送る。ただし第三者に見られることがある」を足した(0.1 節の注意のとおり、`"n"` でも完全には隠れない場合がある) |
| β の Release の資産 | **exe 以外も付ける**(v6 の Release に合わせる) | `zahyou.exe`・`zahyou_v7_beta.ipynb` と `zahyou_engine.py`・`zahyou-manual-v4.pdf`・`LICENSE.txt`・`THIRD-PARTY-NOTICES.md`(rs/ のもの)・`SHA256SUMS.txt`。`rs/tools/make_release_assets.py` がそろえて検査する(6 節)。ノートブックはエンジンを名前で探すので、改名しても動く(確かめた) |

残っていること:

| | 何を決めるか | 今の状態 |
|---|---|---|
| 1 | 本物の nova へキーで送る道の確かめ | 偽の nova でだけ試した(rust-port.md 10.8)。β の間に一度、ユーザーの判断で「オンラインのみ」で解いて確かめる(星の位置と、解けなければ画像が nova へ送られる)。そのとき、送ったものが公開しない扱いになっているか(画像のページが他人から見えないか、「最近送られた画像」の一覧に出るか)も見る(0.1 節の注意は実サイトでは未確認) |

(以前ここにあった「GitHub の latest とプレリリースの扱いの確かめ」と「nova が API で送った画像を公開するか」は、0.1 節で確かめて
決着した。nova は 3 回目の決定で公開しない指定を付けることになった。「ノートブック版のログの見出しの版の文字」は 2 節の
`NB_VERSION` で直した。)

### 0.1 確かめた事実(2026-10-16。claude-research のセッションが公式の文書とソースを読んだ結果)

調べ物は `C:\Users\yoshi\Documents\claude-research\notes\zahyou-v7-release-facts-20261016.md`。**文書とソースを読んだだけ**で、
Release を作って試したり、nova へ送ったりはしていない。ソースは main / trunk のもので、稼働中のサイトが同じ版かは確かめていない。

**GitHub の Release**(以前は記憶による前提だったもの):

| 事実 | 出典 |
|---|---|
| REST API の `GET /repos/{owner}/{repo}/releases/latest` は、**プレリリースでも下書きでもない最も新しい Release** を返す → β をプレリリースで出しても、`releases/latest` を見る v6 の「更新を確認」には届かない | [REST API: releases](https://docs.github.com/en/rest/releases/releases)(Get the latest release) |
| **プレリリースと下書きは latest にできない**(`make_latest` の説明に明記) | 同上(Create / Update a release の `make_latest`) |
| **新しく公開する Release の `make_latest` は既定で `true`** → 正式版(ふつうの Release)は、何も指定しなければ latest になる。値は `true` / `false` / `legacy`(作成日と版で決める) | 同上・[Changelog 2022-10-21](https://github.blog/changelog/2022-10-21-explicitly-set-the-latest-release/) |
| ただし Docs の画面の説明は「Set as latest release を選ばなければセマンティックバージョニングで自動」と書いていて、API の既定 `true` と書き方が食い違う(docs の issue でも指摘)→ **正式版では `--latest` を明示する** | [Managing releases in a repository](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository)・[github/docs#23508](https://github.com/github/docs/issues/23508) |
| `gh release create` は **`--prerelease` と `--latest=false` を一緒に付けられる**(gh に両者の排他の検査は無い)。`--latest` / `--latest=false` を付けたときだけ `make_latest` を送り、付けなければ API の既定に従う。`prerelease` は毎回送る | [gh manual: gh release create](https://cli.github.com/manual/gh_release_create)・[gh のソース create.go](https://github.com/cli/cli/blob/trunk/pkg/cmd/release/create/create.go) |
| (未確認)プレリリースを後から正式版に変えたとき(`gh release edit --prerelease=false`)に latest になるかは、文書に明記が無い → そうするときも `--latest` / `--latest=false` を明示する | — |

→ β は `--prerelease --latest=false`(`--latest=false` は無くても latest にはならないが、意図を明示するため重ねる。7 節)、
正式版は `--latest`(8 節)、正式版のあとの v6 の修正版は `--latest=false`(8 節の 6)。

**nova.astrometry.net の公開の扱い**(3 回目の決定の根拠):

| 事実 | 出典 |
|---|---|
| **`publicly_visible` を付けずに送ると公開(`'y'`)になる**。画像でも星の位置のリストでも同じ受付処理。値は `"y"` / `"n"`。`allow_commercial_use`・`allow_modifications` の既定は `'d'`(API キーの持ち主のプロフィールの既定ライセンス → サイトの既定) | [API 文書](https://astrometry.net/doc/net/api.html)・サーバーの [net/api.py](https://github.com/dstndstn/astrometry.net/blob/main/net/api.py)・[net/abstract_models.py](https://github.com/dstndstn/astrometry.net/blob/main/net/abstract_models.py)・[net/models.py](https://github.com/dstndstn/astrometry.net/blob/main/net/models.py) |
| 公開だと、画像のページ(`/user_images/<id>`)が誰でも見られ、トップの「Recently Submitted Images」・Explore・利用者ごとの一覧・タグや天域の検索に、**API キーの持ち主のアカウント名で**載る | [net/views/image.py](https://github.com/dstndstn/astrometry.net/blob/main/net/views/image.py)・[nova のトップ](https://nova.astrometry.net/) |
| `publicly_visible` は提出の属性として保存されるだけで、**解く処理の条件には使われない**(`"n"` にしても解き方は変わらない) | net/api.py |
| **注意(サーバーのコードの読み。実サイトでは未確認)**: `"n"` にすると画像の詳細ページは本人以外に見えなくなるが、「最近」「全件」「利用者ごと」「検索」の一覧は公開の印で絞られておらず、画像ファイル・サムネイルの配信と API の `/api/jobs/<id>/…` も公開の印や持ち主を確かめていない → `"n"` は「完全な非公開」ではなく「詳細ページを隠す」程度の可能性がある。だから利用者には「送ったものは第三者に見られることがある」と書く | net/models.py・net/views/image.py・[一覧のテンプレート](https://github.com/dstndstn/astrometry.net/blob/main/net/templates/user_image/image_set.html) |
| astroquery の `AstrometryNet` は、渡された設定だけを送り、`publicly_visible` の既定値を自分では付けない(v6 も付けていなかった = サーバーの既定どおり公開として受け付けられていたはず。実サイトでは確かめていない) | [astroquery の文書](https://astroquery.readthedocs.io/en/latest/astrometry_net/astrometry_net.html)・[core.py](https://github.com/astropy/astroquery/blob/main/astroquery/astrometry_net/core.py) |

## 1. v6 の利用者がどうなるか

- v6 の exe の「更新を確認」(起動時にも既定で確かめる)は、`https://api.github.com/repos/Akashi3060/zahyou/releases/latest` の
  資産 **`zahyou.exe`** の digest(SHA256)を手元の exe と比べ、**違えば「新しい版があります」**と出す。版名は見ない。
- **β版をプレリリースで出している間は、latest は v6 のまま**なので、v6 の利用者には何も出ない(GitHub の API はプレリリースを latest として
  返さず、プレリリースは latest にできない。0.1 節。出したあとに 7 節の 6 (a)・(b) で確かめる)。
- **v7 の正式版を Latest で出した時点で、v6 の全員に「新しい版があります」が出る**(新しく公開する正式の Release は既定で latest になる。
  8 節では `--latest` を明示する)。「更新する」で v7 へ入れ替わり、
  v6 の exe が `zahyou.exe.bak`(102 MB)として残る(v7 が消すのは `.old` だけ)。README とマニュアル 9.1 にそう書いた。
- β の利用者が v6 に戻すときは v6 の zahyou.exe を上書きするだけ。設定はそのまま読める(方法は v6 の文言で書く。10.5 節)。
  API キーの欄の値は v6 では使わず、v6 で保存すると消える(次に v7 を開くと既定のキー)。

## 2. 版を上げる

版は **`rs/Cargo.toml` の `[workspace.package] version` の 1 か所**で決まる(10.4 節)。`APP_VER` = `"v" + CARGO_PKG_VERSION`
(窓の題・ログ・［使い方］・更新の確認)、exe の版情報(FileVersion `7.0.0-beta.N`・ProductVersion `v7.0.0-beta.N`・数の版 7.0.0.0・
プレリリースなら VS_FF_PRERELEASE)も付いて変わる。`-beta.N` などのプレリリースの版は、題・起動のログ・［使い方］に「(β版)」が付く。

- β の N を上げるたびにここを直す(`Cargo.lock` の zahyou 自身の版も変わる)。
- **ノートブック版の版も一緒に直す**: `dev/nb_main.py` の `NB_VERSION`(`"v"` + 同じ版。ログの見出しに出る)→ `python dev/build_nb.py`。
  食い違うと 6 節の notebook の検査が止める。
- 解析ログの見出しの版名は、Python の基準値と比べるときに伏せてある(10.4 節)ので、版を上げても `run_solver_vs_python` は落ちない。
- 版を上げた exe を Release より前に動かしても、更新の確認は v6 を勧めない(β は同じ大きな版の新しいものだけを探す。正式版は
  latest が今より古ければ「最新版です」。10.4 節)。

## 3. 同梱物の表示(先に作る。exe に埋め込まれる)

`rs/THIRD-PARTY-NOTICES.md` は `include_str!` で **exe に埋め込まれる**(［使い方］タブの「同梱しているソフトウェアの表示」)。
表示を作り直したら exe も建て直す。**表示 → exe の順**にする。

```bash
export PATH=/c/msys64/ucrt64/bin:$PATH
cd rs
python tools/check_licenses.py           # 依存のライセンスが許可リストの中か(GPL・LGPL・MPL などは失敗)
python tools/gen_notices.py --check      # 古ければ終了コード 1
python tools/gen_notices.py              # 古ければ作り直す(Cargo.lock・Rust・gcc の版が変わったとき)
python -B tools/test_tools.py
```

- 依存を足したら、必ず `check_licenses.py` を通す(v6 では `crc32c`(LGPL)が連鎖で入っていた)。
- 配る `THIRD-PARTY-NOTICES.md` は **`rs/THIRD-PARTY-NOTICES.md`**(リポジトリ直下のものは試作の Python の exe 用)。

## 4. exe を建てて確かめる

```bash
export PATH=/c/msys64/ucrt64/bin:$PATH
cd rs
export CARGO_TARGET_DIR=$(pwd)/target-release      # ほかのビルドと分ける(.gitignore の /target-*/)
# 作者のホームのパスを exe に残さない(下の 4.1 節): 依存のソース(CARGO_HOME)を /cargo、rs/ を /zahyou と書き換えて建てる。
# 区切りは \x1f(CARGO_ENCODED_RUSTFLAGS の決まり。パスに空白があっても切れない)。RUSTFLAGS より優先される
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$(cygpath -w "${CARGO_HOME:-$USERPROFILE/.cargo}")=/cargo"$'\x1f'"--remap-path-prefix=$(cygpath -w "$(pwd)")=/zahyou"
cargo clean --release -p zahyou                    # 埋め込みの表示と版情報を確実に入れ直す
cargo build --release -p zahyou --locked           # lto fat なので 2〜3 分(フラグを付けた初回は依存も全部建てる)
EXE=$CARGO_TARGET_DIR/release/zahyou.exe
ls -l "$EXE"; sha256sum "$EXE"
rustc -vV; cargo -V; gcc --version | head -1      # Release 本文に書く(作った道具の版)
```

exe に作者のホームのパスが残っていないかを数える(0 か所でなければ、上の `CARGO_ENCODED_RUSTFLAGS` を付けずに建てている。
6 節の `make_release_assets.py` も paths の検査で止める):

```bash
python -B - "$(cygpath -w "$EXE")" <<'EOF'
import sys
sys.path.insert(0, "tools")
import make_release_assets as m
d = open(sys.argv[1], "rb").read()
for h in m.home_dirs():
    print(h, m.count_home_paths(d, h))          # {'UTF-8': 0, 'UTF-16': 0} なら合格
print("/cargo:", d.count(b"/cargo\\registry\\src\\"), "/zahyou:", d.count(b"/zahyou\\"))   # 置き換えた形(β2 の準備で 501 と 2)
EOF
```

試験(全部で 17 分ほど。外への問い合わせを避けるなら `ZAHYOU_OFFLINE_TESTS=1` と `--skip live`):

```bash
cargo test --workspace -- --include-ignored
cargo clippy --workspace --all-targets -- -D warnings
```

exe そのものを確かめる。**利用者の `%LOCALAPPDATA%\zahyou` に触れないよう、3 つとも一時へ向ける**(`--selftest` は cache.json を一時へ向けない):

```bash
T=$(mktemp -d)
export ZAHYOU_CACHE="$(cygpath -w $T/cache.json)" ZAHYOU_SETTINGS="$(cygpath -w $T/gui.json)" LOCALAPPDATA="$(cygpath -w $T)"
IMG="$USERPROFILE\\Downloads\\Capture_00001 00_10_33.fits"
"$EXE" --engine-check --out "$(cygpath -w $T/engine.txt)"; cat $T/engine.txt
"$EXE" --selftest "$IMG" --offline --out "$(cygpath -w $T/selftest.txt)"; echo rc=$?; tail -30 $T/selftest.txt   # 25/25 で rc=0
"$EXE" --first-frame "$(cygpath -w $T/ff.txt)"; cat $T/ff.txt          # 最初の描画までの時間・書体・renderer=(開いた描き方と GPU)
"$EXE" --update-check --out "$(cygpath -w $T/upd.txt)"; cat $T/upd.txt # GitHub の Releases の一覧を読むだけ
```

- 期待値: `--selftest --offline` が **25/25**(空の cache.json で 1.594′・0.916′ 東・1.304′ 北・1.4610″/px。10.6 節)。
  exe のプロパティでファイル バージョン `7.0.0-beta.N`。`--update-check` は、まだ出していない版なら「最新版です (v7.0.0-beta.N)」。
- 手でも 1 回起動して、［解析］で基準の画像を解き、［準備］の表が「オフライン解析できます」、題・［使い方］に `v7.0.0-beta.N (β版)`、
  ログの 2 行目に「描画: DirectX 12 / …」が出ることを見る(上の 3 つの環境変数を設定したシェルから起動する)。
  **このとき方法が「自動」でネットがあると、既定のキーで nova へ星の位置を送る**。送りたくなければ、一時の gui.json に
  `{"api_key": ""}` か `{"solve": "オフラインのみ"}` を書いてから起動する。

### 4.1 作者のパスを exe に残さない(`--remap-path-prefix`。2026-10-17、β2 から)

公開した v7.0.0-beta.1 の exe には、作者のホームのパス(`C:\Users\<名前>`)が 504 か所入っていた(公開した資産は差し替えない。7 節の 3)。

| どこから | 数 | 今の形 |
|---|---|---|
| 依存のクレートのパニック位置情報。cargo は依存のソースを CARGO_HOME の下の絶対パス(`…\.cargo\registry\src\index.crates.io-…\<クレート>-<版>\src\…`)で rustc に渡す | 501 | `/cargo\registry\src\index.crates.io-…\tiff-0.11.3\src\…` |
| ビルドスクリプトが OUT_DIR に作って `include!` するもの(glutin_egl_sys・glutin_wgl_sys の bindings。CARGO_TARGET_DIR の下) | 2 | `/zahyou\target-release\release\build\glutin_egl_sys-…\out/egl_bindings.rs` |
| `--bench` の既定の実写の画像(`selftest.rs` の定数) | 1 | 定数をやめ、`$ZAHYOU_TEST_IMAGES`(無ければ USERPROFILE)の `Downloads\Capture_00001 00_10_33.fits` を実行時に組み立てる(`selftest::bench_real()`。試験の画像と同じ決まり) |

- ワークスペースのソースは cargo が相対パス(`app\src\main.rs`)で渡すので、もともと名前は入らない(置き換えにも当たらない)。
  `strip = true` は記号とデバッグ情報を消すが、パニックの位置情報(ファイル名・行・列)は文字列のデータなので残る。ログの「場所: …」
  (ui/worker.rs の panic のフック)に出るのはこれ。置き換えたあともクレート名・版・ファイル・行は読めるので、問い合わせの役には立つ。
- rs/ は `\` の形(`cygpath -w`)で渡したが、MSYS が `C:/Users/…` の形に直した CARGO_TARGET_DIR の下のパスも置き換わった(rustc は前方一致を
  パスの部品ごとに比べ、`\` と `/` を区別しない)。置き換えたあとの区切りは `/` と `\` が混ざるが、害はない。
- 残る作者の名前は、MIT の著作権の行(埋め込んだ同梱物の表示)と版情報の LegalCopyright の `Yoshitaka Fujikawa` だけで、これは意図したもの。

**手順書の環境変数で渡す理由**(`rs/.cargo/config.toml` に固定値を書かない):

1. パスは PC と作業フォルダーごとに違う(CARGO_HOME、ワークツリー `zahyou` と `zahyou-rs`、ほかの人の PC)。固定値は書いた 1 か所でしか効かず、
   ほかの場所では黙って何もしない。しかも作者のホームのパスをリポジトリに書くことになる。環境変数なら、その場のパスから作れる。
2. `config.toml` の rustflags は rs/ の中の全部のビルド(試験・clippy・rust-analyzer・ほかのセッションの `rs/target`)に効き、入れた途端に
   それらも依存から建て直しになる。プロファイルごとの rustflags と、cargo 自身がこの置き換えをする `trim-paths` は、1.94 ではまだ不安定機能
   (`[profile.release] trim-paths = "all"` は「feature `trim-paths` is required」で止まるのを確かめた)。安定したら `trim-paths` に移す。
3. 付け忘れても配らずに済む: 6 節の `make_release_assets.py` の paths の検査が、ホームのパスが残った exe では資産を書かない。

**フラグと建て直し**(2026-10-17 に cargo 1.94 で確かめた):

- cargo はフラグを指紋と成果物の名前のハッシュに入れる。付けた初回は依存も全部建てる(空の `target-check` から release で 2 分 28 秒。
  β1 の記録の 2 分 33 秒とほぼ同じ)。付けたもの・付けないものは並べて残るので、行き来しても建て直しを繰り返さない(zahyou-wcs の試験のビルドで、
  付ける → 付けない で 6 つ(zahyou-wcs と依存)を建て直し、そのあとの 付ける → 付けない は全部 Fresh)。
- 環境変数はそのシェルでだけ効く。**上の試験(`cargo test`・`cargo clippy`)を同じシェルで流すと、試験のビルドにも同じ置き換えが付く**
  (`cargo -v` で全部の rustc の呼び出しに 2 つとも付いているのを見た)。ほかのシェルの `rs/target` のビルドには効かない。
- 付けたままでも試験は通る(10 節)。試験はパニック位置の文字列を見ていない。pipeline.rs の試験の `file!()` はワークスペースの相対パスなので、
  置き換えに当たらない。

## 5. 画面写真とマニュアル

```bash
python rs/tools/shoot_docs.py --exe "$(cygpath -w $EXE)"     # docs/img/app-*.png(13 枚)を撮り直す
python dev/make_manual.py                                     # 天体画像解析プログラム「zahyou」導入・運用マニュアルv4.pdf
```

- `shoot_docs.py` は撮るたびに一時の設定・記憶・LOCALAPPDATA を作り、終わったら利用者の cache.json・gui.json の sha256 が
  変わっていないことを確かめる。**解析する撮影は exe の通信を手元で止めて撮る**(つながらないプロキシ `0.0.0.0:9` を
  ALL_PROXY・HTTPS_PROXY・HTTP_PROXY で渡す。撮る前に同じ環境の `--update-check` が外へ出ずに失敗することを確かめ、止まらなければ
  `"api_key": ""` に切り替えて警告する)。API キーの欄は既定のキーの伏せ字のまま写る。［使い方］の 1 枚だけはネットのまま撮る
  (起動時の更新の確認 = GitHub の一覧を読むだけ。止めると「更新を確認できませんでした」と写る)。
- 版を上げたら、少なくとも `app-help.png`(「いまの版」)は撮り直す(`--only app-help`)。
- 画面の並びが変わったら、切り抜きの枠(`shoot_docs.py` の NUMBERS など)を、撮ったものを見て直す。
- **ページ数を書く箇所が複数ある**(Release 本文と配布ページの txt に書くなら、PDF を組んでから数える)。2026-10-16 の下書きは 33 ページ。
- `make_manual.py` は作成日時などを固定して組む(ReportLab の invariant)ので、同じ make_manual.py とフォントからは同じバイトの PDF になる。
  6 節の道具は組み直して、リポジトリの PDF と突き合わせる。make_manual.py を直したら、PDF も組み直して一緒に commit する。
- ノートブック版を直したら `python dev/build_nb.py`(直下の `zahyou_engine.py` と `zahyou_v6.ipynb` のセルの中身を作り直す)。
- 撮影に使う画像 `Capture_00001 00_10_33.fits` と、星図データ(既定の `C:\AstrometryData`)は作者の PC にあるもの。

## 6. 資産をそろえて SHA256 を作る(`rs/tools/make_release_assets.py`)

```bash
DIST=$USERPROFILE/zahyou-dist-v7.0.0-beta.N        # リポジトリの外。空か無いフォルダー
python rs/tools/make_release_assets.py --exe "$(cygpath -w $EXE)" --out "$(cygpath -w $DIST)"
(cd "$DIST" && sha256sum -c SHA256SUMS.txt)
```

そろえるもの(v6 の Release に合わせる。名前は版から決まる):

| 資産の名前 | 元 | 中身の検査 |
|---|---|---|
| `zahyou.exe` | 4 節の `$EXE`(**名前は必ず `zahyou.exe`**。更新の確認はこの名前で探す) | 版情報(FileVersion・ProductVersion・数の版・プレリリースの印)と `--engine-check` の version= が `rs/Cargo.toml` の版と同じ。**建てた人のホームのパス(USERPROFILE)が 0 か所**(paths。区切り `\` `/`・MSYS の `/c/…`・大文字小文字・UTF-8 と UTF-16 のどれでも数える。4.1 節) |
| `zahyou_v7_beta.ipynb` | リポジトリ直下の `zahyou_v6.ipynb`(v6 と紛れないよう版の名前で。正式版なら `zahyou_v7.ipynb`) | セルの並びと中身が `dev/build_nb.py` の出力と同じ(出力の例はリポジトリのものを残す) |
| `zahyou_engine.py` | リポジトリ直下 | `dev/build_nb.py` の出力と同じ・`NB_VERSION` が `"v"` + `rs/Cargo.toml` の版 |
| `zahyou-manual-v4.pdf` | `天体画像解析プログラム「zahyou」導入・運用マニュアルv4.pdf` | `dev/make_manual.py` で組み直したものとバイト単位で同じ |
| `LICENSE.txt` | `LICENSE` | — |
| `THIRD-PARTY-NOTICES.md` | `rs/THIRD-PARTY-NOTICES.md`(exe の［使い方］タブでも読める) | exe に埋め込んだものと同じ・`gen_notices.py --check` で最新 |
| `SHA256SUMS.txt` | 上の 6 つの SHA256(`sha256sum -c` の形、LF) | 書いたあと読み直して照合 |

- 検査が 1 つでも落ちたら、何も書かずに終了コード 1。`--skip version|paths|notices|manual|notebook` は試しのとき用(最後に「確かめていない」と出る)。
- paths の検査は、この道具を走らせた PC の USERPROFILE を探す(exe を建てた PC で走らせる前提)。
- 出力先は、空か無いフォルダーで、リポジトリの外に限る(直下に .gitignore が無いので)。
- 文字のファイル(.ipynb・.py・.txt・.md)は改行を LF にそろえて書く(作業ツリーが CRLF でも SHA256 が変わらない。`LICENSE.txt` は
  v6 の Release のものと同じ SHA256 になった)。exe と PDF はそのまま写す。
- 試験: `python -B rs/tools/test_make_release_assets.py`(release の exe があれば `ZAHYOU_RELEASE_EXE` で渡すと版情報・同梱物の表示・
  ホームのパスが無いことも確かめる。31 本。4 節の置き換えを付けずに建てた exe なら、ホームのパスの試験が落ちる)。
- 2026-10-16 に、`7.0.0-beta.1`(help_tab.rs を直したあとの release の exe、14,039,040 バイト)で 4 つの検査が通り、`sha256sum -c` で 6 つとも OK。
  公開しない指定を入れたあと(exe 14,041,088 バイト・マニュアル v4 を組み直したもの)も同じく通った(rust-port.md 10.10)。
  **ノートブックの資産(`zahyou_v7_beta.ipynb`・`zahyou_engine.py`)は、Python 版の公開しない指定(native-solver)を取り込んで
  `dev/build_nb.py` で作り直してからそろえる**(取り込む前のものは `publicly_visible` を付けずに送る)。

SHA256 は Release の本文にも書く(SHA256SUMS.txt を貼る)。正式版では配布ページの `はじめにお読みください.txt` にも書く(8 節)。

## 7. β版を GitHub に出す(rust-port.md 10.7 節。ユーザーの了承のあとで)

1. `native-rs` を push する(**main には統合しない**)。タグは **`v7.0.0-beta.N`**(v + Cargo の版)を、配る exe を建てた `native-rs` の commit に付ける。
2. Release は **「Set as a pre-release」に印、「Set as the latest release」は付けない**(gh では `--prerelease --latest=false`。0.1 節)。
   下書きで作り、中身を見てから出す:

```bash
# 資産は 6 節でそろえた $DIST の 7 つ(SHA256SUMS.txt も)。本文は $DIST の外に書いておく
gh release create v7.0.0-beta.N --repo Akashi3060/zahyou --target native-rs --prerelease --latest=false --draft \
  --title "zahyou v7.0.0-beta.N (β版)" --notes-file release-notes-v7.0.0-beta.N.md \
  "$DIST/zahyou.exe" "$DIST/zahyou_v7_beta.ipynb" "$DIST/zahyou_engine.py" "$DIST/zahyou-manual-v4.pdf" \
  "$DIST/LICENSE.txt" "$DIST/THIRD-PARTY-NOTICES.md" "$DIST/SHA256SUMS.txt"
# 出す前に確かめる: pre-release の印が付いている・Latest でない・下書き・資産が 7 つで digest が SHA256SUMS.txt と同じ
gh release view v7.0.0-beta.N --repo Akashi3060/zahyou --json isPrerelease,isDraft,assets \
  --jq '"prerelease=\(.isPrerelease) draft=\(.isDraft)", (.assets[]|[.name,.size,.digest]|@tsv)'
gh api repos/Akashi3060/zahyou/releases/latest --jq .tag_name                                # v6 のまま
gh release edit v7.0.0-beta.N --repo Akashi3060/zahyou --draft=false --latest=false         # 出す(--prerelease は付いたまま)
```

   `--latest=false` は無くても latest にはならない(プレリリースは latest にできない)が、意図を明示するため作るときと出すときの両方に付ける
   (gh は `--prerelease` と一緒に受け付ける。0.1 節)。下書きと一緒に `make_latest` を送ったときの API の扱いは確かめていない。
   作るときに断られたら、作るときの `--latest=false` を外し、出すとき(`gh release edit`)だけに付ける。
3. **公開したβの資産は差し替えない**。直したら N を上げて新しい Release にする(βの更新の確認は「今より新しい版」しか出さないので、
   同じタグの建て直しはβの利用者に届かない)。
4. **間違えて正式の Release(pre-release の印なし)で出すと、そのまま v6 の全員に入れ替えが出る**。印を付け忘れたら、すぐ pre-release に直す。
5. Release 本文に書くこと(v6 の本文を土台に): β版であること・v6 の更新には出ないこと・v6 との行き来(1 節)、ダウンロードの警告(5 枚の図。
   exe の中身によらない)、画面の写真(raw の URL は **`…/zahyou/v7.0.0-beta.N/docs/img/…`(タグ)**で書く。`main` には β の写真が無い)、
   v6 からの変更(WSL 不要・Rust・速さと大きさ。数字は README の「実測値」と同じもの)、API キーの欄と、外へ送るもの(まず星の位置、
   解けなければ画像ファイルそのもの。送る前に聞かない。**公開しない指定 `publicly_visible = n` を付けて送るが、nova に送ったものは
   第三者に見られることがある**)と送らない方法(方法を「オフラインのみ」/ API キーの欄を空)、
   付けた資産の一覧(6 節)と SHA256SUMS.txt の中身、ノートブックの名前(`zahyou_v7_beta.ipynb`。中身はリポジトリの `zahyou_v6.ipynb`)、
   作った道具の版(rustc・gcc)、動作環境(想定は Windows 11・RAM 8 GB 以上のノート PC、確かめた PC は 1 台だけ)、
   描き方の手動の選び方(`--glow`・`--gpu`)、問い合わせのときに欲しいもの(ログの「描画: …」の行、遅いときは `--bench-ui` の記録)、
   ライセンス(Rust のクレート 213 個、GPL・LGPL・MPL なし)。
6. 出したあとに確かめる(10.7 節の 6):
   - (a) `gh api repos/Akashi3060/zahyou/releases/latest --jq .tag_name` が **`v6` のまま**
   - (b) v6 の exe の「更新を確認」が「最新版です」(v6 の exe を一時の LOCALAPPDATA で起動して)
   - (c) 一つ前のβ版の exe の `--update-check` が「新しい版があります: v7.0.0-beta.N」(N ≥ 2 のとき)
   - (d) 新しいβ版の exe の `--update-check` が「最新版です (v7.0.0-beta.N / 日付 版)」
   - (e) 作者以外の PC(か、別の Windows のユーザー)で Edge から落とし、README の 5 枚の警告の順に出るか(今の図は v6 の exe のとき)。
     想定のノート PC(Windows 11・RAM 8 GB)で 1 台は起動と解析を確かめ、README の「確かめた PC は 1 台だけ」を書き足す
7. β の間に v6 の修正版を出すときは、ふつうの Release(Latest)でよい(β の利用者は大きな版 7 しか見ないので影響しない)。
   gh では `--latest` を明示する(0.1 節。既定でも latest になるはずだが、Docs の説明と API の既定の書き方が食い違っているため)。
8. 配布ページ(Nextcloud)には β を置かない(v6 の利用者向けのまま)。置くなら 8 節の手順で、v6 と取り違えない名前にする。

## 8. 正式版(7.0.0)を出す(1〜2 か月使って安定を確かめてから)

1. `native-rs` を main に統合する(PR。`native-rs` は Python 版の `native-solver` を取り込んでいる)。`rs/Cargo.toml` の版を `7.0.0` にし、
   2〜6 節をやり直す(版を上げると「(β版)」は自然に消える。ノートブックの資産名は `zahyou_v7.ipynb` になる)。
2. README を正式版の形にする: β版のタグへのリンク(`releases/tag/v7.0.0-beta.N`)を `releases/latest` に戻し、「いまの v7 はβ版です」と
   「v6 とβ版」の段落、ノートブックの入手先の β の注記を書き換える(README 冒頭の HTML コメントに書いてある)。マニュアルの表紙の「版」の行と
   2.1・9.1・付録 B も同じ。
3. **v6 の Release 本文の画像を v6 のタグに固定する。** v6 の本文は `raw.githubusercontent.com/Akashi3060/zahyou/main/docs/img/app-*.png`
   (`app-analyze`・`app-numbers`・`app-prepare`)を見ているので、main の写真を v7 にすると、v6 の Release ページに v7 の画面
   (WSL の行の無い［準備］タブ)が出る。`main` を `v6` に置き換えた本文で `gh release edit v6 --notes-file …`。**main への統合の前に**行う。
4. タグ `v7.0.0`(main の統合したあとの commit)で **ふつうの Release(Latest)**。ここで v6 の全員と β の利用者に入れ替えが出る(1 節)。
   新しく公開する正式の Release は API の既定で latest になるが、Docs の画面の説明と書き方が食い違っている(0.1 節)ので、**`--latest` を明示する**。
   β と同じく下書きで作って中身を見てから出す(資産は 6 節。ノートブックは `zahyou_v7.ipynb`)。下書きは latest にできないので、
   `--latest` は出すとき(`gh release edit`)に付ける:

```bash
gh release create v7.0.0 --repo Akashi3060/zahyou --target main --draft \
  --title "zahyou v7.0.0" --notes-file release-notes-v7.0.0.md \
  "$DIST/zahyou.exe" "$DIST/zahyou_v7.ipynb" "$DIST/zahyou_engine.py" "$DIST/zahyou-manual-v4.pdf" \
  "$DIST/LICENSE.txt" "$DIST/THIRD-PARTY-NOTICES.md" "$DIST/SHA256SUMS.txt"
gh release view v7.0.0 --repo Akashi3060/zahyou --json isPrerelease,isDraft,assets \
  --jq '"prerelease=\(.isPrerelease) draft=\(.isDraft)", (.assets[]|[.name,.size,.digest]|@tsv)'   # prerelease=false
gh release edit v7.0.0 --repo Akashi3060/zahyou --draft=false --latest                       # 出す(ここで v6 の全員に入れ替えが出る)
gh api repos/Akashi3060/zahyou/releases/latest --jq .tag_name                                # v7.0.0
```

5. 出したあと: `releases/latest` が `v7.0.0`、資産 `zahyou.exe` の digest が 6 節の SHA256、v6 の exe(一時の LOCALAPPDATA)の
   「更新を確認」→「更新する」で v7 が起動し、`zahyou.exe.bak` が残り、v6 で書いた gui.json の設定(方法・目標・配色)が出ること
   (v6 の入れ替えの台本は環境変数を引き継いで v7 を起動するので、v7 も一時の LOCALAPPDATA を使う)。
6. 正式版のあとに v6 の修正版を出すときは `--latest=false`(付けないと、新しく公開する正式の Release は既定で latest になる。0.1 節。
   latest が v6 に戻ると、v6 の利用者向けの意図とずれる。v7 の利用者は latest が古ければ入れ替えに誘わないので守られる)。
7. 戻すとき(v7 に問題があったとき): `gh release edit v6 --latest`。v6 の利用者はそのまま。v7 の利用者は latest が古い版なので
   入れ替えに誘われない(10.4 節)— 戻ってほしいなら案内して手で上書きしてもらう。

### 配布ページ(Nextcloud)を差し替える(正式版のとき。ユーザーの了承のあとで)

公開共有(token `M2sNsnA2K4oJ4Co`)は**閲覧のみ**で、`dev/upload_share.py` の PUT は 403。`R:`(YoFu)からも書けない。
実体は `/mnt/tank/TrueNAS/AstroKIT/掩蔽観測班/外部共有フォルダー/天体画像解析プログラム「zahyou」`(所有者 AstroKIT)。

- **今あるファイルの中身を替える**(`zahyou.exe`・`THIRD-PARTY-NOTICES.md`・`LICENSE.txt`・`zahyou_engine.py`・`zahyou_v6.ipynb`・
  `はじめにお読みください.txt`): TrueNAS の root(一時 cronjob)で **`cat 新しいもの > 今のもの`**。inode が変わらないので、所有者・ACL・
  公開共有の設定はそのまま。作業ファイルは書ける `R:\AstroKIT\` に置き、終わったら root で消す(root が書いたものは `chown YoFu:YoFu` しないと
  Windows から消せない)。手順の型は CLAUDE.md の TrueNAS の節と、メモリの zahyou-release-pipeline。
- 配布ページのファイルの名前は Release の資産と違う(配布ページは `zahyou_v6.ipynb`・`…マニュアルv3.pdf`、資産は `zahyou_v7.ipynb`・
  `zahyou-manual-v4.pdf`)。v6 の名前の中身を v7 のものに替えるか、v7 の名前のファイルを足すか(下)を決める。`SHA256SUMS.txt` は
  配布ページでは星図データの zip 用なので、Release の `SHA256SUMS.txt` で上書きしない。
- **新しい名前のファイル**(`…マニュアルv4.pdf`)は `cat >` では作れない(新しい inode になる)。作ったあと所有者と ACL を今あるファイル
  (例 `…マニュアルv3.pdf`)と同じにしたか、`stat` と ACL の表示で見比べる。v3 の PDF を残すか消すかも決める。
- `はじめにお読みください.txt` は v6 の記述が多い。直すところ: 大きさ(102 MB)・起動(4 秒)・誤検知の理由(Python をまとめているため)・
  SHA256・［準備］タブ(WSL・解析エンジン・設定ファイル)・入れ替え(`zahyou.exe.bak`、「WSL は作り直さなくて構いません」、題 `zahyou v6`)・
  消すもの(102 MB、WSL の中の解析エンジン)・動作環境の表(WSL2)・マニュアルの名前とページ数(v3・33 ページ)・ライセンス(astrometry.net を
  ［準備］タブが WSL へ入れる)。API キーの欄のことも足す。README の文に合わせる。
- 仕上げに、**公開 URL から実際に落として SHA256 を照合する**(外部ストレージなので、Nextcloud の一覧が新しい中身を見ているかも同時に確かめられる)。

## 9. 後片付け

- メモリの `zahyou-release-pipeline` を Rust の手順(この文書)に合わせて書き直す(今は PyInstaller の `build_exe.ps1 -Clean -Test` と
  `dev/make_notices.py` が書いてある)。
- β の期間が過ぎて v6 を配らなくなったら、`Solve::stored_label` を `label()` に戻してよい(rust-port.md 10.5 節)。
- `gui/build_exe.ps1`・`gui/zahyou.spec`・`dev/make_notices.py`(PyInstaller の exe 用)を残すか(v6 と試作版の記録として残すなら、
  README の「中身に興味がある方へ」の説明はそのまま)。

## 10. 出した記録

### v7.0.0-beta.1(2026-10-16 15:58 JST に公開。ユーザーの指示「すべて揃えてからβ版のリリースまで」)

- タグ `v7.0.0-beta.1` = `native-rs` の `8b7060c`(exe を建てたのは `42a9d31`。間の 1 つは README の exe の大きさだけで、exe の中身は同じ)。
  Release: <https://github.com/Akashi3060/zahyou/releases/tag/v7.0.0-beta.1>(Pre-release。本文の元は手元の
  `release-notes-v7.0.0-beta.1.md` で、リポジトリには入れていない)
- 道具: rustc 1.94.0 / cargo 1.94.0(MSYS2 UCRT64、`x86_64-pc-windows-gnu`)、gcc 15.2.0。`cargo build --release -p zahyou --locked` 2 分 33 秒
- 試験: `cargo test --workspace -- --include-ignored` **279 本通過・失敗 0**(17 分 19 秒)、clippy 指摘 0、check_licenses・`gen_notices --check`・
  test_tools(15 本)OK、`test_make_release_assets.py` 24 本(release の exe を要る 2 本は飛ばした)。exe の `--selftest --offline` 25/25、
  `--engine-check` の version= v7.0.0-beta.1、`--first-frame` DX12 / RTX 3060。Python の `dev/test_online_settings.py` 14/14
- 資産(6 節の 4 つの検査が全部 OK): `zahyou.exe` 14,041,088 バイト `fc39b8d8…dfceb5`、`zahyou_v7_beta.ipynb` `03839c92…259d`、
  `zahyou_engine.py` `27186b85…8b8e`、`zahyou-manual-v4.pdf`(33 ページ)`d62c2e83…81e2`、`LICENSE.txt` `4efc7bab…a3f8`、
  `THIRD-PARTY-NOTICES.md` `f037a257…bdd4`。GitHub の各資産の digest は SHA256SUMS.txt と全部同じ
- 出したあと(7 節の 6): (a) latest は `v6` のまま。(b) Release から落とした v6 の exe(`5f771f33…3605`)の `--update-check` は
  「最新版です (v6 / 2026-09-17 版)」。(d) Release から落としたβの exe(SHA256 は SHA256SUMS.txt と一致)の `--update-check` は
  「最新版です (v7.0.0-beta.1 / 2026-10-16 版)」。どれも ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時に向けて動かし、
  利用者の cache.json・gui.json の sha256 は前後で同じ。**(e) 別の PC・想定のノート PC はまだ**(0 節の 1 の本物の nova の確かめも、まだ)
- 分かったこと:
  - `gh release create … --prerelease --latest=false --draft` は断られずに通った(7 節の 2 の「確かめていない」の答え)。
    タグを先に push しておき `--verify-tag` で作った。下書きの間の URL は `releases/tag/untagged-…` になる
  - Git Bash の `python` が MSYS2 UCRT64 の Python 3.14 を指していて、reportlab が無く manual の検査が落ちた(資産は書かれない)。
    `make_release_assets.py` は Windows の Python 3.13(`%LOCALAPPDATA%\Programs\Python\Python313\python.exe`)で走らせる
  - 画面写真(14:24 に撮影)のあとに［使い方］の手引きの文が増えたが、`app-help.png` に写る範囲(上の 3 節)は変わっていないので撮り直していない
  - (2026-10-17 に見つけた)公開した exe(`fc39b8d8…`)に**作者のホームのパス `C:\Users\<名前>` が 504 か所**入っていた。依存のクレートの
    パニック位置情報が 501、glutin のビルドスクリプトが OUT_DIR に作ったものが 2、`--bench` の既定の画像が 1。資産は差し替えず(7 節の 3)、
    β2 から消す(4.1 節・下の「β2 の準備」)

### β2 の準備: 作者のパスを exe から消す(2026-10-17。このあと下の v7.0.0-beta.2 で出した)

- 変えたもの: 4 節の `CARGO_ENCODED_RUSTFLAGS`(`--remap-path-prefix` を 2 つ。理由は 4.1 節)、`selftest::bench_real()`(`--bench` の既定の画像を
  実行時に組み立てる)、`make_release_assets.py` の paths の検査と試験(31 本)。
- 確かめ方: 4 節の手順で `CARGO_TARGET_DIR=rs/target-check`(試しの置き場。.gitignore の `/target-*/`)に release を建てた(2 分 28 秒。
  `cargo build -v` の rustc の呼び出し 274 回の全部に 2 つとも付いた)。exe のバイト列を Python で数えた結果:

| | β1 の exe(公開したもの) | 置き換えて建てた exe |
|---|---|---|
| 大きさ | 14,041,088 バイト | 14,033,408 バイト |
| `C:\Users\yoshi` | 502 | **0** |
| `C:/Users/yoshi` | 2 | **0** |
| `yoshi`(小文字のまま) | 504 | **0** |
| `/cargo\registry\src\index.crates.io-…`(依存のパニック位置情報) | 0 | 501 |
| `/zahyou\target-check\release\build\glutin_*_sys-…\out/…`(OUT_DIR) | 0 | 2 |
| ワークスペースの相対パス(`app\src\main.rs` など) | 40 | 40 |
| 大文字小文字を無視した `yoshi` | 505(UTF-16 で 1) | 1(UTF-16 で 1)= MIT の著作権の行と版情報の LegalCopyright の `Yoshitaka` |

- exe: `--engine-check` の version= v7.0.0-beta.1、`--selftest "…Capture_00001 00_10_33.fits" --offline` **25/25**(1.594′・0.916′ 東・1.304′ 北・
  1.4610″/px)。`make_release_assets.py` を通しで走らせ、5 つの検査(version・paths・notices・manual・notebook)が全部 OK、`sha256sum -c` で
  6 つとも OK(exe 以外の資産の SHA256 は β1 と同じ)。手順書 4 節の数え方も書いたとおりに走らせ、置き換えた exe で 0 か所、β1 の exe で 504 か所と出た。
  `--bench` は既定の実写(ホームの `Downloads\Capture_00001 00_10_33.fits`)を実行時に見つけて、合成 6000×4000 と一緒に測った。
  どれも ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時に向けて動かし、利用者の cache.json・gui.json の sha256 は前後で同じ
- 試験: 置き換えを付けたまま `ZAHYOU_OFFLINE_TESTS=1 cargo test --workspace -- --skip live` **266 本通過・失敗 0**(無視 9 本は回していない。
  2 分 59 秒)、`cargo clippy --workspace --all-targets -- -D warnings` 指摘 0。`test_make_release_assets.py` 31 本(置き換えた exe を `ZAHYOU_RELEASE_EXE` で渡して。Windows の
  Python 3.13 と MSYS2 の 3.14 の両方)
- この exe(`d262e975…0921`)は配らない。β2 は 2 節で版を上げ、4 節の手順(置き換え込み)で `target-release` に建て直す。`rs/target-check` は消してよい

### v7.0.0-beta.2(2026-10-17 07:23 JST に公開。ユーザーの指示「beta で配布する exe を新しいものに更新して」)

- 公開した β1 の資産は差し替えず(7 節の 3)、版を上げた新しいプレリリースで出した。β1 の exe の更新の確認に β2 が出る
- タグ `v7.0.0-beta.2` = `native-rs` の `0b73d36`(版上げ・ノートブック・マニュアル・README・app-help.png)。exe は README・写真・マニュアルを
  直す前の同じ木(rs/ は同じ)で建てた。Release: <https://github.com/Akashi3060/zahyou/releases/tag/v7.0.0-beta.2>
  (本文は β1 の本文に「β1 からの変更」の節を足し、SHA256・大きさ・試験の表を直したもの。リポジトリには入れていない)
- 建て方: 4 節のとおり `CARGO_ENCODED_RUSTFLAGS`(置き換え 2 つ)を付けて `target-release` で `cargo clean --release -p zahyou` →
  `cargo build --release -p zahyou --locked`(2 分 36 秒)。exe 14,033,408 バイト `ae4bc3c9…4a41`。作者のホームのパス 0 か所
  (UTF-8・UTF-16)、置き換えた位置情報 `/cargoegistry\src\…` 501・`/zahyou\…` 2
- 試験(同じフラグ・同じ `target-release` で): `cargo test --workspace -- --include-ignored` **279 本通過・失敗 0**(16 分 48 秒)、clippy 指摘 0、
  check_licenses・`gen_notices --check`(213 個、作り直し不要)・test_tools OK、`test_make_release_assets.py` 31 本(この exe を渡して)、
  Python の `dev/test_online_settings.py` 14/14。exe: `--engine-check` の version= v7.0.0-beta.2、`--selftest --offline` 25/25、
  版情報 7.0.0-beta.2・v7.0.0-beta.2・7.0.0.0・IsPreRelease True。`--first-frame` DX12 350〜355 ms・OpenGL 190〜239 ms(各 3 回。README に反映)
- 文書: README・マニュアル v4(33 ページ、組み直し)・ノートブックの `NB_VERSION` を beta.2 に。`app-help.png` を β2 の exe で撮り直した(`--only app-help`)
- 資産(6 節の 5 つの検査が全部 OK、`sha256sum -c` OK、GitHub の digest も全部一致): `zahyou.exe` `ae4bc3c9…4a41`、
  `zahyou_v7_beta.ipynb` `03839c92…259d`(β1 と同じ)、`zahyou_engine.py` `54c4d0d5…22c7`、`zahyou-manual-v4.pdf` `c5a59af1…ae1a`、
  `LICENSE.txt`・`THIRD-PARTY-NOTICES.md` は β1 と同じ
- 出したあと: (a) latest は `v6` のまま。(b) v6 の exe の `--update-check` は「最新版です (v6 / 2026-09-17 版)」。
  (c) β1 の exe は「新しい版があります: v7.0.0-beta.2 (2026-10-16 版 / 14 MB)」。(d) Release から落とした β2 の exe(SHA256 一致)は
  「最新版です (v7.0.0-beta.2 / 2026-10-16 版)」。日付は GitHub の公開時刻(UTC。22:23Z)の日付なので、日本時間の 10-17 より 1 日前に見える。
  どれも一時の設定で動かし、利用者の cache.json・gui.json の sha256 は前後で同じ。**(e) 別の PC・想定のノート PC は、β1 と同じくまだ**

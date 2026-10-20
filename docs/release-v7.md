# v7 を配る手順(β版 → 正式版)

2026-10-16 作成、同日に 2026-10-16 の決定(API キーの欄・DirectX 12 を既定・v7 は当面β版)と、2 回目の決定
(画像は同意を取らずに送ってよい・β の Release にも exe 以外を付ける)、3 回目の決定(nova へは公開しない指定を付けて送る)と、
GitHub の Release の扱いを公式の説明で確かめた結果(0.1 節)に合わせて書き直した。2026-10-18 に、正式版 7.0.0 の準備
(8 節の 1・2 と 9 節。版上げ・README とマニュアルの本文・ノートブックの改名・v6 用の古いものの削除)に合わせて直した。
v6 までの手順(`gui/build_exe.ps1` = PyInstaller、`dev/make_notices.py`)を、cargo の release ビルドの exe・
`rs/THIRD-PARTY-NOTICES.md`・SHA256 の作り直しに置き換えたもの(v6 の 2 つの道具は、7.0.0 の準備でリポジトリから消した。9 節)。
ビルドと試験の細かい説明は [../rs/README.md](../rs/README.md)、決定と実測は [rust-port.md](rust-port.md) の 9・10 節
(β版の出し方の元は 10.7 節)。

**v7 は当初、β版(7.0.0-beta.N)を GitHub のプレリリースで出し、1〜2 か月使って安定を確かめてから正式版 7.0.0 を
main に統合してふつうの Release で出す、とした。** 1〜7 節がβ版、8 節が正式版。push・タグ・Release・配布ページの更新は、
どれもユーザーの了承のあとで行う。
**2026-10-18 にユーザーが「回数については目安程度で、動作に問題が無いことが確認できれば、v7 をリリースしたい」とした。**
`native-rs` の版は 7.0.0 に上げ、README・マニュアルを正式版の形にした(8 節の 1・2)。残りは 8 節の冒頭(「2026-10-18 の時点」)。

---

## 0. 決まったことと、残っていること

決まったこと(2026-10-16。rust-port.md 10.1・10.9 節):

| | 決定 | 実装 |
|---|---|---|
| API キー | ［解析］タブの「解析」の枠に欄(伏せ字・「表示」・「元に戻す」)。**既定は元のキー(v6 と同じ)** | gui.json の `api_key` に平文で保存。空なら問い合わせない。環境変数 ASTROMETRY_API_KEY は gui.json に `api_key` が無いときの初めの値だけ。キーの値は文書・ログ・画面の文に書かない |
| 描画 | **DirectX 12 を既定**(主の画面の GPU・小さな塊)。開けなければ OpenGL → ソフトウェア描画(WARP) | 手で選ぶ引数 `--dx12` / `--glow` / `--gpu display\|low\|high\|warp`。想定は「Windows 11 が動く、RAM 8 GB 以上のノート PC」 |
| v6 からの移り方 | **v7 は当面β版。v6 の自動更新の対象にしない** | 版 7.0.0-beta.N をプレリリースで出す(v6 の更新係が見る latest に出ない)。β 自身は v7 の新しいβ版と正式版を探す。**→ 2026-10-18 に正式版へ切り替えた(8 節)** |
| 画像を外へ送ること | **同意を取らずに送ってよい**(今の動きのまま。~~v6 と同じ~~ → 送る前に聞かないのは v6 と同じだが、v6 の exe は画像ファイルを送っていなかった。10 節の 2026-10-19 の訂正) | 方法が「自動」か「オンラインのみ」で、キーがあり、ネットにつながっていると、まず検出した星の位置(と画像の大きさ・手がかり)を、それで解けなければ画像ファイルそのもの(ファイル名も)を nova.astrometry.net へ送る。送らない方法(方法を「オフラインのみ」/ API キーの欄を空)と一緒に、README・マニュアル 2.3・［使い方］タブに書いた |
| nova へ送るときの公開の扱い(3 回目) | **公開しない指定 `publicly_visible = "n"` を付ける**(Rust 版と Python 版(ノートブックを含む)の両方)。文書には「nova に送ったものは第三者に見られることがある」と書く | Rust: `zahyou_net::nova::PUBLICLY_VISIBLE`・`private_settings`(星の位置のリストと画像の両方の request-json の先頭に付ける。呼ぶ側が別の値を渡しても `"n"`)。Python: `dev/zahyou_core.py` の `solve_online` の設定(同じく先頭。native-solver の `b878079` で直し、`53e485e` で native-rs に取り込んだ。β1 の資産から入っている)。試験は偽の nova が受け取った本文と astroquery の記録(`rs/testdata/gen_net.py`)で。README・マニュアル 2.3・［使い方］タブ・rs/README.md に「公開しない指定を付けて送る。ただし第三者に見られることがある」を足した(0.1 節の注意のとおり、`"n"` でも完全には隠れない場合がある) |
| β の Release の資産 | **exe 以外も付ける**(v6 の Release に合わせる) | `zahyou.exe`・`zahyou_v7_beta.ipynb` と `zahyou_engine.py`・`zahyou-manual-v4.pdf`・`LICENSE.txt`・`THIRD-PARTY-NOTICES.md`(rs/ のもの)・`SHA256SUMS.txt`。`rs/tools/make_release_assets.py` がそろえて検査する(6 節)。ノートブックはエンジンを名前で探すので、改名しても動く(確かめた) |
| 超広角の画像(2026-10-18) | **考慮しない**(望遠鏡の用途では当たらない) | 索引 4115〜4119 の大きな quad で、写真側の code を画素の平面のまま作るために取りこぼす件(index-format.md 8.4・14 の 9。合成の試験では短辺 33° 以上で解けない)は直さない。README の「対応画角 4′〜2000′」もそのまま |
| 動作環境(2026-10-18) | **RAM 8 GB 以上(推奨 16 GB)**。作者がノート PC(Core i7-1360P・RAM 16 GB・SSD 1 TB)で軽快に動くことを確かめた | README の「動作環境」・マニュアル 3 章に書いた。RAM 8 GB の PC は確かめていない(zahyou が使う分から、足りる見込み) |

残っていること:

| | 何を決めるか | 今の状態 |
|---|---|---|
| 1 | ~~本物の nova へキーで送る道の確かめ~~ | **2026-10-18 に済み**(ユーザーの指示。10 節の「2026-10-18 の追記」)。β2 の exe の `--selftest … --online` で 3 回送り、星の位置だけで 3 回とも解けた(画像は送っていない)。公開しない指定の効き方は実サイトで確かめた |

(以前ここにあった「GitHub の latest とプレリリースの扱いの確かめ」と「nova が API で送った画像を公開するか」は、0.1 節で確かめて
決着した。nova は 3 回目の決定で公開しない指定を付けることになった。「ノートブック版のログの見出しの版の文字」は 2 節の
`NB_VERSION` で直した。)

### 0.1 確かめた事実(2026-10-16。claude-research のセッションが公式の文書とソースを読んだ結果)

調べ物は作者の手元のノート `claude-research\notes\zahyou-v7-release-facts-20261016.md`(リポジトリの外)。**文書とソースを読んだだけ**で、
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
| **注意(サーバーのコードの読み。2026-10-18 に実サイトでおおむね確かめた。10 節)**: `"n"` にすると画像の詳細ページは本人以外に見えなくなるが、「最近」「全件」「利用者ごと」「検索」の一覧は公開の印で絞られておらず、画像ファイル・サムネイルの配信と API の `/api/jobs/<id>/…` も公開の印や持ち主を確かめていない → `"n"` は「完全な非公開」ではなく「詳細ページを隠す」程度の可能性がある。だから利用者には「送ったものは第三者に見られることがある」と書く | net/models.py・net/views/image.py・[一覧のテンプレート](https://github.com/dstndstn/astrometry.net/blob/main/net/templates/user_image/image_set.html) |
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

ただし版の文字は exe の外にも書いてある。**版を上げるときに直す場所は全部で次の 7 つ**(β2 の版上げ `0b73d36` で直したものと同じ。
上から順に):

| | 直す場所 | 直し方 | 直し忘れたら |
|---|---|---|---|
| 1 | `rs/Cargo.toml` の `[workspace.package] version` | 手で(`7.0.0-beta.N` / 正式版は `7.0.0`) | (元) |
| 2 | `rs/Cargo.lock`(ワークスペースの 7 つの crate の版) | `cargo check --workspace` などで cargo が書き換える(手で直さない)。4 節の `--locked` より前に済ませる | 4 節の `cargo build --locked` が止まる |
| 3 | `dev/nb_main.py` の `NB_VERSION`(ノートブック版のログの見出し) | `"v"` + 同じ版 → `python dev/build_nb.py`(直下の `zahyou_engine.py` を作り直す) | 試験 `version_in_docs_matches_cargo` と 6 節の notebook の検査 |
| 4 | `dev/make_manual.py` の 3 か所(冒頭の説明・表紙の表の「版」の行・9.1 の「ウィンドウの題」) | 手で | 試験 `version_in_docs_matches_cargo` |
| 5 | `README.md`。正式版の形(2026-10-18 から)では 2 か所(「起動と大きさ」の表の見出し・入れ替えの節の「ウィンドウの題」)。β版の形のときは 8 行だった(冒頭の HTML コメント・「いまの v7 はβ版」・Releases へのリンク 3 つ・app-help.png の注記・この 2 か所) | 手で。`grep -n "v7\.0\.[0-9]" README.md` で出る(β版の形なら `grep -n "beta\.[0-9]" README.md`)。exe の大きさと起動の時間は 4 節で建てた exe の実測に | 試験 `version_in_docs_matches_cargo`(β の版の文字だけ。正式版どうしの版上げ(7.0.0 → 7.0.1 など)の直し忘れと数字は分からない) |
| 6 | `docs/img/app-help.png`(［使い方］の「いまの版」が写る) | 4 節で建てた exe で `python rs/tools/shoot_docs.py --exe … --only app-help`(5 節) | 試験では分からない。撮ったものを目で見る |
| 7 | マニュアル PDF(`…マニュアルv4.pdf`。4 の文字と 6 の写真を貼っている) | 4 と 6 のあとで `python dev/make_manual.py`(5 節) | 6 節の manual の検査(組み直したものとバイト単位で比べる) |

- 試験 `version_in_docs_matches_cargo`(`rs/app/src/pipeline.rs`)は、README・`dev/make_manual.py`・`dev/nb_main.py`・`zahyou_engine.py` の
  「数.数.数-beta.数」をすべて拾い、`Cargo.toml` の版と違うものがあれば場所を出して落ちる(`-beta.N` の書き方は拾わない)。
  正式版(`7.0.0`)にしたら、β の版の文字が 1 つでも残っていれば落ちる(8 節の 2 で README とマニュアルを正式版の形にする)。
- 試験に版の文字は書かない(2026-10-17 から)。それまでは `version_text` の試験が `if 版 == "7.0.0-beta.2" { … }` の形で版の文字を持ち、
  版を上げて直し忘れると条件が外れて黙って通っていた(β2 の版上げではこの文字も直す場所の 1 つだった)。今の `version_text` は、
  cargo が分けた大・中・小・プレリリースと更新の確認の読み方(`parse_version`)が同じかを、版によらず見る。
- `docs/release-v7.md`(この文書の 10 節の記録)と `docs/rust-port.md` の版の文字は、その時の記録なので直さない。
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
- 配る `THIRD-PARTY-NOTICES.md` は **`rs/THIRD-PARTY-NOTICES.md`**(リポジトリ直下にあった試作の Python の exe 用のものは、
  7.0.0 の準備で消した。`gen_notices.py` は以前それと星図データの条件の文を突き合わせていたが、それもやめた)。

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

exe そのものを確かめる。**利用者の `%LOCALAPPDATA%\zahyou` に触れないよう、3 つとも一時へ向ける**(`--selftest` は 2026-10-17 から自分で gui.json と cache.json を一時へ向けるが、`--first-frame` は gui.json だけ、`--engine-check`・`--update-check` と手で開く画面は向けない):

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
  正式版(`7.0.0`)なら、ファイル バージョン `7.0.0`(プレリリースの印なし)、`--update-check` は Release の前なら
  「最新版です (v7.0.0)。公開されている最新版 v6 より新しい版を使っています。」(latest が今より古い版なら誘わない)。
- 手でも 1 回起動して、［解析］で基準の画像を解き、［準備］の表が「オフライン解析できます」、題・［使い方］に `v7.0.0-beta.N (β版)`
  (正式版なら `v7.0.0`。「(β版)」は付かない)、
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
- ノートブック版を直したら `python dev/build_nb.py`(直下の `zahyou_engine.py` と `zahyou_v7.ipynb` のセルの中身を作り直す)。
- 撮影に使う画像 `Capture_00001 00_10_33.fits` と、星図データ(既定の `C:\AstrometryData`)は作者の PC にあるもの。

## 6. 資産をそろえて SHA256 を作る(`rs/tools/make_release_assets.py`)

```bash
DIST=$USERPROFILE/zahyou-dist-v7.0.0               # リポジトリの外。空か無いフォルダー(β版なら zahyou-dist-v7.0.0-beta.N)
python rs/tools/make_release_assets.py --exe "$(cygpath -w $EXE)" --out "$(cygpath -w $DIST)"
(cd "$DIST" && sha256sum -c SHA256SUMS.txt)
```

そろえるもの(v6 の Release に合わせる。名前は版から決まる):

| 資産の名前 | 元 | 中身の検査 |
|---|---|---|
| `zahyou.exe` | 4 節の `$EXE`(**名前は必ず `zahyou.exe`**。更新の確認はこの名前で探す) | 版情報(FileVersion・ProductVersion・数の版・プレリリースの印)と `--engine-check` の version= が `rs/Cargo.toml` の版と同じ。**建てた人のホームのパス(USERPROFILE)が 0 か所**(paths。区切り `\` `/`・MSYS の `/c/…`・大文字小文字・UTF-8 と UTF-16 のどれでも数える。4.1 節) |
| `zahyou_v7.ipynb`(β版なら `zahyou_v7_beta.ipynb`) | リポジトリ直下の `zahyou_v7.ipynb`(2026-10-18 に `zahyou_v6.ipynb` から改名した。配る名前は版から決まる。β版の名前は v6 と紛れないよう) | セルの並びと中身が `dev/build_nb.py` の出力と同じ(出力の例はリポジトリのものを残す) |
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
  ホームのパスが無いことも確かめる。32 本(2026-10-18 に改名の試験を足した)。4 節の置き換えを付けずに建てた exe なら、ホームのパスの試験が落ちる)。
- 2026-10-16 に、`7.0.0-beta.1`(help_tab.rs を直したあとの release の exe、14,039,040 バイト)で 4 つの検査が通り、`sha256sum -c` で 6 つとも OK。
  公開しない指定を入れたあと(exe 14,041,088 バイト・マニュアル v4 を組み直したもの)も同じく通った(rust-port.md 10.10)。
  ノートブックの資産(`zahyou_v7_beta.ipynb`・`zahyou_engine.py`)の公開しない指定は、Python 版の直し(native-solver の `b878079`)を
  `53e485e` で取り込み、`dev/build_nb.py` で作り直してある(β1 の資産から `solve_online` が `publicly_visible: "n"` を付ける。
  以前ここに「取り込んでからそろえる」と書いていた残りは、これで済んだ)。

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
   付けた資産の一覧(6 節)と SHA256SUMS.txt の中身、ノートブックの名前(`zahyou_v7_beta.ipynb`。中身はリポジトリの `zahyou_v7.ipynb`)、
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

## 8. 正式版(7.0.0)を出す(動作に問題が無いことを確かめてから。2026-10-18 の決定。当初は「1〜2 か月使ってから」)

**2026-10-18 の時点**(ユーザーの指示「回数については目安程度で、動作に問題が無いことが確認できれば、v7 をリリースしたい」。`native-rs` で):

- 済み: 版を `7.0.0` に(2 節の表の 1〜5)、README・マニュアルの本文(`dev/make_manual.py`)を正式版の形に(下の 2)、
  ノートブックを `zahyou_v6.ipynb` から `zahyou_v7.ipynb` に改名、v6 用の古いものを消した(9 節)、`rs/THIRD-PARTY-NOTICES.md` を作り直した
  (MIT の範囲の一覧の 1 行。exe に埋め込まれる)。
- 残っていること(この順に):
  1. 4 節で 7.0.0 の exe を建て、試験と exe を確かめる(期待値は 4 節の正式版の形)。README の exe の大きさと起動の時間を、その exe の実測に
     (今の数字は β2 の exe のもの。表の見出しだけ `v7.0.0` にしてある)
  2. 5 節: 画面写真を 7.0.0 の exe で撮り直し、`python dev/make_manual.py` でマニュアル PDF を組み直して commit する。
     2026-10-18 の点検で、［解析］の画像欄に作者のホームのパス(`C:\Users\<名前>\Downloads\…`)が写っている写真があると分かったので、
     `app-help.png` だけでなく全部を、名前の入らない場所(`C:\Users\Public\…`)に写した基準の画像で撮り直す(`shoot_docs.py --image`)
     (リポジトリの PDF は β2 の本文のまま。組み直すまで 6 節の manual の検査は通らない)
  3. 6 節で資産をそろえる(ノートブックの資産は `zahyou_v7.ipynb`)。なお直下の `zahyou_v7.ipynb` に残している出力の例(セル C の見出し)は
     v6 で走らせたときのまま `zahyou v6` と出ている(`build_nb.py` はセルの中身だけを差し替え、出力の例は残す)。差し替えるなら、
     ネットを切るか `SOLVE_MODE = 'OFFLINE'` で走らせ直して保存する(任意)
  4. 下の 3(v6 の Release 本文のリンクの固定。main への統合の前)→ 1 の統合(PR)→ 4 の Release → 5 の確かめ → 配布ページの差し替え(下)

1. `native-rs` を main に統合する(PR。`native-rs` は Python 版の `native-solver` を取り込んでいる)。`rs/Cargo.toml` の版を `7.0.0` にし、
   2〜6 節をやり直す(版を上げると「(β版)」は自然に消える。ノートブックの資産名は `zahyou_v7.ipynb` になる)。
   → 版を `7.0.0` にしたのは 2026-10-18(済み)。統合と 4〜6 節は残り(上)。
2. README を正式版の形にする: β版のタグへのリンク(`releases/tag/v7.0.0-beta.N`)を `releases/latest` に戻し、「いまの v7 はβ版です」と
   「v6 とβ版」の段落、ノートブックの入手先の β の注記を書き換える(README 冒頭の HTML コメントに書いてある)。マニュアルの表紙の「版」の行と
   2.1・9.1・付録 B も同じ。2 節の試験 `version_in_docs_matches_cargo` が、β の版の文字の残りを場所つきで出す。
   → **済み(2026-10-18)**。「いまの v7 はβ版です」は「v6 を使っている方へ」の案内に、「v6 とβ版」は「v6 から入れ替える」
   (v6 の「更新する」で入れ替わる・`zahyou.exe.bak` が残る・設定と星図データはそのまま・WSL は要らない・β版の利用者には 7.0.0 が出る)に、
   ノートブックの資産名と入手先(Releases の最新と配布ページ)を v7 に。マニュアルも同じ(表紙・2.1・4.3・4.5・9.1・付録 B)。
3. **v6 の Release 本文のリンクを、v6 の exe を建てた木に固定する。** v6 の本文のリンクは 13 個で、うち 12 個が `main` を指す
   (`raw.githubusercontent.com/Akashi3060/zahyou/main/docs/img/` の画面写真 3 枚(`app-analyze`・`app-numbers`・`app-prepare`)と
   ダウンロードの警告の図 6 枚、`github.com/Akashi3060/zahyou/blob/main/` の `LICENSE`・`THIRD-PARTY-NOTICES.md`・`gui/README.md`)。
   残りの 1 個は既定のブランチの README の見出し `github.com/Akashi3060/zahyou#動作環境`。main を v7 にすると、v6 の Release ページに
   v7 の画面(WSL の行の無い［準備］タブ)と v7 の文書が出る。さらに 7.0.0 の準備で直下の `THIRD-PARTY-NOTICES.md` と `gui/` を消したので、
   固定しないとこの 2 つのリンクは 404 になる。固定した本文で `gh release edit v6 --notes-file …`。**main への統合の前に**行う。
   - **固定先は `b25a42f74236b2d05bf984993a8b4b44eaf4ee00`**(今の main の先頭。「更新したあと新しい exe が起動しない問題を直した (#23)」、
     2026-09-18)。v6 の配っている exe を建てた木と同じ木(木のハッシュ `84dacb9d…`)で、リンク先の 12 個が全部ある(2026-10-17 に
     `git cat-file -e b25a42f:<パス>` で確かめた)。URL の `main` を 40 桁の SHA に置き換える
     (`raw.githubusercontent.com/Akashi3060/zahyou/b25a42f74236b2d05bf984993a8b4b44eaf4ee00/docs/img/app-analyze.png`、
     `github.com/Akashi3060/zahyou/blob/b25a42f74236b2d05bf984993a8b4b44eaf4ee00/LICENSE`)。README の見出しは
     `github.com/Akashi3060/zahyou/blob/b25a42f74236b2d05bf984993a8b4b44eaf4ee00/README.md#動作環境`(b25a42f の README に `# 動作環境` がある)。
     読める名前にしたければ、b25a42f に新しいタグ(例 `v6-docs`)を付けて push し、SHA の代わりに使う(タグもユーザーの了承のあとで)。
   - **タグ `v6`(= `ecf7618`、2026-08-31)には固定しない。** その木には `docs/`・`gui/`・`LICENSE`・`THIRD-PARTY-NOTICES.md` が無く、
     `main` を `v6` に置き換えると 12 個が全部 404 になる(以前のこの節は「`main` を `v6` に置き換える」と書いていたが、成り立たない)。
     `v6` はブランチ(`fecece4`)とタグの両方にある名前でもあり、`git` でも「ambiguous」と言われる。
   - **`3936e02` も使わない。** v6 の exe を建てた PR #23 の commit そのもの(木は b25a42f と同じ)だが、main には squash で `b25a42f` として入り、
     `3936e02` はどのブランチ・タグからもたどれない。GitHub 側でいつまで引けるか分からない。
   - 書き換えたら、本文のリンクを 13 個とも開いて 200 になることを確かめてから出す。
   **統合(PR の merge)から Release までは間を置かない。** 統合すると main の README が「Releases の最新(v7)から落とす」になるのに、
   Release を出すまで latest は v6 のままで、食い違うため。資産・本文・確かめ(5 節まで)は統合の前に全部そろえておく。
   正式版の Release 本文に書くこと(β版の 7 節の 5 とは逆の内容になる): v7 になって何が変わったか(WSL が要らない・Rust・速さと大きさ)、
   **v6 からの入れ替え**(v6 の［使い方］の「更新を確認」→「更新する」で入れ替わる・`zahyou.exe.bak` が残るので動いたら消してよい・
   設定と覚えた座標と星図データはそのまま・WSL の astrometry.net は要らなくなる・v6 への戻し方)、β版の利用者には β版の「更新を確認」で
   7.0.0 が出ること、ダウンロードの警告(5 枚の図)、画面の写真(raw の URL はタグ `v7` で書く)、API キーの欄と外へ送るもの
   (2026-10-18 に実サイトで確かめた、第三者から何が見えて何が隠れるか)、資産の一覧と SHA256SUMS.txt、ノートブックの名前 `zahyou_v7.ipynb`、
   動作環境(RAM 8 GB 以上・推奨 16 GB、確かめた PC 2 台)、作った道具の版、ライセンス(クレート 213 個、GPL・LGPL・MPL なし)。
4. タグ **`v7`**(main の統合したあとの commit。v5・v6 と同じ付け方。exe の版は `7.0.0` のまま。2026-10-19 のユーザーの指示で
   `v7.0.0` から付け替えた)で **ふつうの Release(Latest)**。ここで v6 の全員と β の利用者に入れ替えが出る(1 節)。
   新しく公開する正式の Release は API の既定で latest になるが、Docs の画面の説明と書き方が食い違っている(0.1 節)ので、**`--latest` を明示する**。
   β と同じく下書きで作って中身を見てから出す(資産は 6 節。ノートブックは `zahyou_v7.ipynb`)。下書きは latest にできないので、
   `--latest` は出すとき(`gh release edit`)に付ける:

```bash
gh release create v7 --repo Akashi3060/zahyou --verify-tag --draft \
  --title "zahyou v7.0.0" --notes-file release-notes-v7.0.0.md \
  "$DIST/zahyou.exe" "$DIST/zahyou_v7.ipynb" "$DIST/zahyou_engine.py" "$DIST/zahyou-manual-v4.pdf" \
  "$DIST/LICENSE.txt" "$DIST/THIRD-PARTY-NOTICES.md" "$DIST/SHA256SUMS.txt"
gh release view v7 --repo Akashi3060/zahyou --json isPrerelease,isDraft,assets \
  --jq '"prerelease=\(.isPrerelease) draft=\(.isDraft)", (.assets[]|[.name,.size,.digest]|@tsv)'   # prerelease=false
gh release edit v7 --repo Akashi3060/zahyou --draft=false --latest                           # 出す(ここで v6 の全員に入れ替えが出る)
gh api repos/Akashi3060/zahyou/releases/latest --jq .tag_name                                # v7
```

5. 出したあと: `releases/latest` が `v7.0.0`、資産 `zahyou.exe` の digest が 6 節の SHA256、v6 の exe(一時の LOCALAPPDATA)の
   「更新を確認」→「更新する」で v7 が起動し、`zahyou.exe.bak` が残り、v6 で書いた gui.json の設定(方法・目標・配色)が出ること
   (v6 の入れ替えの台本は環境変数を引き継いで v7 を起動するので、v7 も一時の LOCALAPPDATA を使う)。
6. 正式版のあとに v6 の修正版を出すときは `--latest=false`(付けないと、新しく公開する正式の Release は既定で latest になる。0.1 節。
   latest が v6 に戻ると、v6 の利用者向けの意図とずれる。v7 の利用者は latest が古ければ入れ替えに誘わないので守られる)。
7. 戻すとき(v7 に問題があったとき): `gh release edit v6 --latest`。v6 の利用者はそのまま。v7 の利用者は latest が古い版なので
   入れ替えに誘われない(10.4 節)— 戻ってほしいなら案内して手で上書きしてもらう。

### 配布ページ(Nextcloud)を差し替える(正式版のとき。ユーザーの了承のあとで)

公開共有(token `M2sNsnA2K4oJ4Co`)は**閲覧のみ**で、WebDAV の PUT は 403(v6 のころの `dev/upload_share.py` で確かめた。この道具は
7.0.0 の準備で消した)。`R:`(YoFu)からも書けない。
実体は `/mnt/tank/TrueNAS/AstroKIT/掩蔽観測班/外部共有フォルダー/天体画像解析プログラム「zahyou」`(所有者 AstroKIT)。

**差し替えたあとの配布ページに置くもの**(2026-10-18 のユーザーの決定): `zahyou.exe`・`zahyou_v7.ipynb`・`zahyou_engine.py`・
マニュアル v4(`…マニュアルv4.pdf`)・`LICENSE.txt`・`THIRD-PARTY-NOTICES.md`(`rs/` のもの)・`はじめにお読みください.txt`・星図データの zip
(と、その `SHA256SUMS.txt`)。v6 の `zahyou_v6.ipynb` と `…マニュアルv3.pdf` は置かない(消すのも了承のあとで)。

- **今あるファイルの中身を替える**(`zahyou.exe`・`THIRD-PARTY-NOTICES.md`・`LICENSE.txt`・`zahyou_engine.py`・
  `はじめにお読みください.txt`): TrueNAS の root(一時 cronjob)で **`cat 新しいもの > 今のもの`**。inode が変わらないので、所有者・ACL・
  公開共有の設定はそのまま。作業ファイルは書ける `R:\AstroKIT\` に置き、終わったら root で消す(root が書いたものは `chown YoFu:YoFu` しないと
  Windows から消せない)。手順の型は CLAUDE.md の TrueNAS の節と、メモリの zahyou-release-pipeline。
- 配布ページの名前は Release の資産と一部違う(マニュアルは配布ページでは `…マニュアルv4.pdf`、資産は `zahyou-manual-v4.pdf`。中身は同じ)。
  `SHA256SUMS.txt` は配布ページでは星図データの zip 用なので、Release の `SHA256SUMS.txt` で上書きしない。
- **新しい名前のファイル**(`zahyou_v7.ipynb`・`…マニュアルv4.pdf`)は `cat >` では作れない(新しい inode になる)。作ったあと所有者と ACL を
  今あるファイル(例 `…マニュアルv3.pdf`)と同じにしたか、`stat` と ACL の表示で見比べる。そのあとで `zahyou_v6.ipynb` と `…マニュアルv3.pdf` を消す。
- `はじめにお読みください.txt` は v6 の記述が多い。直すところ: 大きさ(102 MB)・起動(4 秒)・誤検知の理由(Python をまとめているため)・
  SHA256・［準備］タブ(WSL・解析エンジン・設定ファイル)・入れ替え(`zahyou.exe.bak`、「WSL は作り直さなくて構いません」、題 `zahyou v6`)・
  消すもの(102 MB、WSL の中の解析エンジン)・動作環境の表(WSL2)・マニュアルの名前とページ数(v3・33 ページ)・ライセンス(astrometry.net を
  ［準備］タブが WSL へ入れる)。API キーの欄のことも足す。README の文に合わせる。
- 仕上げに、**公開 URL から実際に落として SHA256 を照合する**(外部ストレージなので、Nextcloud の一覧が新しい中身を見ているかも同時に確かめられる)。

## 9. 後片付け

- メモリの `zahyou-release-pipeline` を Rust の手順(この文書)に合わせて書き直す(今は PyInstaller の `build_exe.ps1 -Clean -Test` と
  `dev/make_notices.py` が書いてある)。
- β の期間が過ぎて v6 を配らなくなったら、`Solve::stored_label` を `label()` に戻してよい(rust-port.md 10.5 節)。
- ~~`gui/build_exe.ps1`・`gui/zahyou.spec`・`dev/make_notices.py`(PyInstaller の exe 用)を残すか(v6 と試作版の記録として残すなら、
  README の「中身に興味がある方へ」の説明はそのまま)。~~ → **済み(2026-10-18、ユーザーの決定で消した)**。消したのは `gui/` の全部
  (Python の試作の画面・v6 の更新係 `zahyou_update.py`・`zahyou_env.py`・PyInstaller の `build_exe.ps1`・`zahyou.spec`・画面の試験
  `test_*.py`・`README.md`・`.gitignore`)、`dev/make_notices.py`、`dev/upload_share.py`、直下の `THIRD-PARTY-NOTICES.md`(Python の exe 用)、
  直下の `zahyou.ipynb`(2025-11 の v5 のノートブック)、直下のマニュアル v3 の PDF。どれも commit `0b73d36`(旧タグ v7.0.0-beta.2)の木に残っている
  (`git show 0b73d36:gui/zahyou_update.py` など)。照合値を作る `rs/testdata/gen_net.py` は `gui/` の Python 版を読むので、
  作り直すときはそのタグの木を `ZAHYOU_PY_REPO` に指す(rs/README.md)。

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
| `C:\Users\<名前>` | 502 | **0** |
| `C:/Users/<名前>` | 2 | **0** |
| `<名前>`(作者のユーザー名、小文字のまま) | 504 | **0** |
| `/cargo\registry\src\index.crates.io-…`(依存のパニック位置情報) | 0 | 501 |
| `/zahyou\target-check\release\build\glutin_*_sys-…\out/…`(OUT_DIR) | 0 | 2 |
| ワークスペースの相対パス(`app\src\main.rs` など) | 40 | 40 |
| 大文字小文字を無視した `<名前>` | 505(UTF-16 で 1) | 1(UTF-16 で 1)= MIT の著作権の行と版情報の LegalCopyright の `Yoshitaka` |

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
  (UTF-8・UTF-16)、置き換えた位置情報 `/cargo\registry\src\…` 501・`/zahyou\…` 2
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

### 2026-10-18 の追記(ユーザーの指示: 本物の nova での確かめ・動作環境・β1 の案内・超広角・公開ソースのパス)

- **本物の nova.astrometry.net へ送った**(ユーザーが了承。画像は「基準の実写」を選んだ): 配った β2 の exe(`ae4bc3c9…4a41`)で
  `--selftest "…Capture_00001 00_10_33.fits" --online` を、ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時に向けて 1 回(12:52〜12:53 JST)。
  自己診断の中で解析を 3 回通すので、提出は 3 件(ジョブ 17123912・17123913・17123914)。3 件とも**星の位置 106 個だけ**で解けた
  (画像そのものは送っていない。nova の original_filename は `json-xy`)。自己診断は 25/25、1.592′・0.915′ 東・1.304′ 北・1.4606″/px
  (オフラインの 1.594′・1.4610″/px と 0.3″ 以内)。キーは既定のキー(環境変数は無し)。利用者の cache.json・gui.json の sha256 は前後で同じ
- **公開しない指定の効き方(実サイト)**: ログインしていない第三者の立場で、調べ物のセッション(claude-research、cookie なしの curl。
  ログイン・送信・人間確認のボタンは押していない)が確かめた。結果は `claude-research\notes\zahyou-nova-visibility-check-20261018.md`(リポジトリの外)
  | | 第三者から |
  |---|---|
  | トップの Recently Submitted Images・Explore・Images の一覧・利用者の Images タブ | **載らない**(前後の番号は載り、3 件だけ飛ばされる) |
  | 詳細ページ(`/user_images/<id>`・`/status/<id>`) | 人間確認の画面へ転送(その先は試していない。コード上は本人以外に見せない) |
  | 検索ページ(`/user_images/search`)・利用者のページ(`/users/<id>`・`/submissions`) | **載る**(番号・サムネイルの URL・時刻・アカウント名・提出数) |
  | 番号が分かれば: `/api/jobs/<id>/info`・`calibration`、`/wcs_file/`、`/axy_file/`、`/rdls_file/`、`/joblog2/`、`/wwt_image/` | **人間確認なしで取れる**(解いた位置と向き・画素スケール、送った星の座標 106 点と画像の大きさ・位置の手がかり、星を点で描いた絵) |

  → 文書の「公開しない指定を付けて送る。ただし第三者に見られることがある」は正しかった。README と マニュアル 2.3 に、何が見えて何が隠れるかを
  具体的に書き足した(exe の［使い方］タブの文は前のままで正しいので、次に exe を建てるときに合わせるかを決める)。
  なお、**作者のアカウントには、指定なしで送った過去の提出(Images タブに 1560 枚。v6 の astroquery 経由など)が公開のまま並んでいる**。
  隠すには本人がログインして各画像の hide を使う(こちらでは操作していない)
- **動作環境**: 作者がノート PC(Core i7-1360P・RAM 16 GB・SSD 1 TB)で軽快に動くことを確かめた(ユーザーの報告)。README・マニュアル 3 章を
  「RAM 8 GB 以上(推奨 16 GB)」「確かめた PC は 2 台」に直した。7 節の 6 (e) のうち、ダウンロードの警告の出方・RAM 8 GB の PC は、まだ
- **β1 の Release 本文の冒頭に β2 の案内**を足した(`gh release edit v7.0.0-beta.1 --notes-file`。Pre-release のまま、latest は v6 のまま)
- **超広角は考慮しない**(0 節の「決まったこと」)。**公開ソースの作者のパス**: 試験と生成スクリプトは f2e1fd5 で外し、残っていた docs の 10 か所と
  `rs/testdata/image_formats.json` の 1 か所(Python のエラー文の中。試験は文の先頭だけを見る)を `C:\Users\<名前>`・`%USERPROFILE%` に伏せた。
  著作権の表示の `Yoshitaka Fujikawa` は意図したもの。**git の履歴には前の commit のパスが残る**(消すには履歴の書き換えが要り、公開済みのタグも
  変わるので行わない)

### v7.0.0(タグ `v7`。2026-10-19 08:38 JST に正式版として公開。ユーザーの指示「動作に問題が無いことが確認できれば、v7 をリリースしたい」)

- 判断の材料(2026-10-18): 使用回数は会話記録・PC の上の跡・GitHub・nova から数えた。実際の利用は、作者の 10/17 の予備観測の画像 9 枚を
  β2 のエンジンで解いたもの(9 枚とも解けた)と、ノート PC での試用。配った exe(β1・β2)の起動は確かめを含めて 27 回、すべて成功、自己診断は毎回 25/25。
  β1 の公開後に、異常終了・誤った答え・本番の試験の失敗は 0 件(見つかったのは作者のパスの混入〔β2 で直した〕、自己診断が利用者の cache.json に
  書く癖〔bd0ecca で直し 7.0.0 に入った〕など、解析の答えに関わらないものだけ)。nova の非公開の提出は 10-14 以後 Claude の確かめの 3 件だけで、
  10/17 の現場の解析は v6 だったとみられる
- 準備(native-rs): v6 用の古いものを消した(gui/・dev/make_notices.py・dev/upload_share.py・直下の THIRD-PARTY-NOTICES.md・zahyou.ipynb・
  マニュアル v3)、ノートブックを zahyou_v7.ipynb に改名、版を 7.0.0 に、README・マニュアル・［使い方］タブを正式版の形に(dfeaa69・c865d0b・
  ede638c・b2ad026・7f1cfbb)。画面写真 13 枚を 7.0.0 の exe で撮り直した(基準の画像は作者の名前の入らない C:\Users\Public\Documents\zahyou-shot に
  写して開いた。［解析］の画像欄に作者のホームのパスが写っていたため)。マニュアル v4 を組み直した(33 ページ)。7739799
- exe: 4 節のとおり CARGO_ENCODED_RUSTFLAGS(置き換え 2 つ)を付けて target-release で建てた(2 分 11 秒)。14,040,576 バイト `5ab3dec6…bae4`、
  作者のホームのパス 0 か所、版情報 7.0.0・v7.0.0・プレリリースの印なし。`--selftest --offline` 25/25、`--engine-check` の version= v7.0.0。
  `--first-frame` DX12 376〜387 ms・OpenGL 438〜456 ms(同じ日に交互に測った β2 の exe も同じ値。β2 の公開の日の 350 ms・190〜239 ms との差は PC の状態)
- 試験(同じフラグ・target-release): `cargo test --workspace -- --include-ignored` **284 本通過・失敗 0**(16 分 42 秒)、clippy 指摘 0、
  check_licenses・`gen_notices --check`・test_tools OK、`test_make_release_assets.py` 32 本(この exe を渡して)、`dev/test_online_settings.py` 14/14
- 資産(6 節の 5 つの検査が全部 OK、`sha256sum -c` OK、GitHub の digest も全部一致): `zahyou.exe` `5ab3dec6…bae4`、`zahyou_v7.ipynb` `ab742a79…f784`、
  `zahyou_engine.py` `3c0a2b20…3993`、`zahyou-manual-v4.pdf` `d71eb1d0…76cd`、`LICENSE.txt` `4efc7bab…a3f8`(v6 と同じ)、`THIRD-PARTY-NOTICES.md` `2105f522…aa62`
- 順番: native-rs を push → **v6 の Release 本文のリンク 13 個を b25a42f に固定**(13 個とも 200 を確かめてから。8 節の 3)→ PR #24 を merge commit で
  統合(main = `b70f73a`、木は native-rs の 7739799 と同じ。CI は無い)→ タグ `v7.0.0` で下書き → 本文のリンク 15 個が 200 → `--latest` で公開
  (2026-10-18 23:38 UTC)→ ユーザーの指示でタグを **`v7`** に付け替えた(同じ commit に `v7` を付けて push → `gh release edit v7.0.0 --tag v7`
  で Release を移し、本文の `…/v7.0.0/…` を `…/v7/…` に → どの Release も使わなくなった `v7.0.0` のタグを消した)
- 出したあと(5 節): latest は `v7`。Release から落とした exe の SHA256 は SHA256SUMS.txt と一致。更新の確認(一時の設定で): 7.0.0 は
  「最新版です (v7 / 2026-10-18 版)」、β1・β2・v6 は「新しい版があります: v7」。**v6 の exe の写しで `--update-apply`** を一時のフォルダーで動かし、
  zahyou.exe が 7.0.0(`5ab3dec6…`)に入れ替わり、v6 の exe が zahyou.exe.bak(`5f771f33…`)として残ることを確かめた。利用者の cache.json・gui.json は前後で同じ
- **配布ページ(Nextcloud)を v7 に差し替えた**(ユーザーの決定どおり)。TrueNAS の root(一時 cronjob)で: zahyou.exe・zahyou_engine.py・
  THIRD-PARTY-NOTICES.md・はじめにお読みください.txt(v7 用に書き直し、BOM 付き UTF-8・CRLF)は `cat >` で中身だけ入れ替え(inode・所有者
  AstroKIT:YoFu・ACL は前のまま)、zahyou_v7.ipynb・マニュアル v4 を足した(所有者と ACL は既存のファイルと同じ)、zahyou_v6.ipynb・マニュアル v3・
  マニュアル v2・スクリーンショット フォルダーを消した(tank/TrueNAS のスナップショットに残る)。LICENSE.txt は同じ中身なので触れていない。
  公開 URL(public.php/webdav)の一覧と、落とした 6 つの SHA256 が Release の資産と一致することを確かめた
- 公開のあとに、［使い方］の写真(app-help.png)を撮り直してマニュアルを組み直した(このブランチ。公開の前に撮ったものは「最新版です (v7.0.0)。
  公開されている最新版 v6 より新しい版を使っています」と出ていた)。
- **2026-10-19 09:0x に、マニュアルの資産を公開のあとの写真で組み直したものに差し替えた**(ユーザーの指示「スクショを新しいものにして」)。
  main(c42dde2)のマニュアル(`8bd78680…b04d`、33 ページ。今の make_manual.py で組み直したものと同じ)を、`make_release_assets.py` で
  資産一式ごと作り直し(5 つの検査 OK。マニュアル以外の 5 つの SHA256 は前と同じ)、`gh release upload v7 --clobber` で
  `zahyou-manual-v4.pdf` と `SHA256SUMS.txt` を差し替え、本文の SHA256SUMS の行も直した。exe は変えていないので更新の届き方には関係しない。
  配布ページのマニュアルも root の `cat >` で同じものに(inode・所有者・ACL は前のまま)。公開 URL と GitHub から落として SHA256 が一致した。
  そのため、資産のマニュアルはタグ `v7`(b70f73a)の木のものではなく、main の c42dde2 のもの(違いは［使い方］の写真だけ)

### 2026-10-19 の後片付け(β の Release を消した・nova の過去の公開分は残す)

- **β版 2 つの Release を消した**(ユーザーの指示「β版二つをリリース自体から消したい」)。`gh release delete v7.0.0-beta.1 --yes`・
  `gh release delete v7.0.0-beta.2 --yes`(`--cleanup-tag` は付けない)。消したあとの Releases は v7(latest)・v6〜v2 だけで、β の資産の URL
  (`…/releases/download/v7.0.0-beta.N/zahyou.exe` など)は 404。β1 の exe に作者のパスが 504 か所入っていた件(上の「v7.0.0-beta.1」)の始末を兼ねる
- **タグ `v7.0.0-beta.1`(8b7060c)・`v7.0.0-beta.2`(0b73d36)は残した**(→ 2026-10-20 に消した。下の「2026-10-20 の整理」)。`dev/README.md`・`rs/README.md`・`zahyou-net/src/lib.rs` が
  「消した `gui/` などはタグ v7.0.0-beta.2 の木に残っている」と指しているため。タグのページは開けるが、exe は落とせない
- 消す前の本文・資産の一覧・SHA256SUMS.txt はリポジトリの外の `Documents\claude\zahyou\release-backup\beta-releases-20261019\` に控えた
- β の exe の「更新を確認」は Releases の一覧(プレリリースを含む)から同じ大きな版の新しいものを探すので、β が無くなっても v7 が出る
  (README の「β版を使っていた方」の 1 行はそのままで正しい)
- **nova の作者のアカウント(利用者 52918)に公開のまま残る過去の提出は、非公開にしないことにした**(ユーザーの決定)。ログインせずに
  `/users/52918/images` を数え直すと 1,560 件(30 件ずつ 52 ページ。番号は 11022092〜16539871)。試したこと:
  - API キーの `/api/login` で得たセッションは、サイトの画面ではログインにならない(`/dashboard/images` が `/signin` へ転送)。
    `/user_images/<id>/hide` は持ち主でなくても 302 で画像のページへ戻すだけで、何も変えない(1 件試して、一覧に残ったまま)
  - ブラウザーで本人にサインインしてもらう方法は、GitHub でサインインしたアカウントが別の利用者(56859)だったので止めた。
    52918 がどの外部アカウントに結びついているかは分からない。人間確認(`/ask_human`)のボタンは押していない
  - 非公開にするなら、52918 のアカウントでサインインして各画像の hide を使う。番号の一覧と、サインインしたブラウザーから順に送るための
    スクリプトはリポジトリの外の `Documents\claude\zahyou\nova-hide\`

### 2026-10-19 の訂正(v6 の exe は画像ファイルを送っていなかった)

ブログの記事 (4) を書くときの検証で分かった。README・マニュアル 2.3 の「送る前に確かめることはせずに次のものを送ります(v6 と同じです)」は
誤りだった。送る前に聞かないのは v6 と同じだが、**v6 の exe は画像ファイルを送っていなかった**。コードと配った exe を読んで確かめた
(本物の nova へ画像を送る試験はしていない)。

- v6(b25a42f)の `solve_online`: zahyou が検出した星の位置を `solve_from_source_list` で送り、解けなければ astroquery の
  `solve_from_image(選んだ画像のパス)` を呼ぶ(星が 0 個ならはじめからこちら)。`force_image_upload` も `publicly_visible` も付けていない
- astroquery 0.4.10 の `solve_from_image`(`astrometry_net/core.py` 408 行〜): 画像を上げるのは `force_image_upload or self._no_source_detector`
  のときだけ。`photutils.detection` の DAOStarFinder を import できれば、`CCDData.read(画像, unit='adu')` → `sigma_clipped_stats` →
  DAOStarFinder(fwhm=3、しきい値 5σ)で星を数え直し、`solve_from_source_list` で位置を送る。この経路には「photutils の機能を外す」という
  AstropyDeprecationWarning が付いている(将来の版では画像を上げる側に変わりうる)。`force_image_upload` は 0.4.8 から非推奨(渡すと警告、値は効く)
- **配った v6 の exe**(Release の `zahyou.exe`、107,288,075 バイト)の PyInstaller の中身を読んだ(実行はしていない): `astroquery.version` は 0.4.10、
  `photutils.detection.daofinder` と、import に要る `photutils\CITATION.rst` が入っている。中のエンジンは Release の `zahyou_engine.py` と同じバイトで、
  `force_image_upload` を含まない。`gui/zahyou.spec` も photutils を同梱する(データと hiddenimports)
- `CCDData.read` は FITS しか読めない(astropy 7.1.0 で JPEG・PNG・TIFF は `IORegistryError: Format could not be identified`)。そのため v6 の exe では、
  FITS 以外の画像は 2 段目が例外になり「⚠️ オンライン解析でエラー」→「自動」なら WSL へ移っていた
- v6 のノートブック版: 案内の `!pip install` に photutils が無いので、その手順どおりの環境では画像ファイルを上げていた。photutils のある環境では exe と同じ
- v7 の exe(Rust): 星の位置のリストに nova が「解けない」と答えたとき、または星が 0 個のとき(`pipeline.rs` は 0 個でもオンラインへ進む)、
  画像ファイルの中身とファイル名をそのまま `api/upload` で送る(`zahyou-net/src/nova.rs` の `solve_online_inner`)。時間切れ・中止では送らない
- **v7 のノートブック版**(`zahyou_engine.py` の `solve_online`)は、v6 と同じく `solve_from_image` を `force_image_upload` なしで呼ぶ。案内どおり
  (photutils なし)なら画像ファイルを送るが、photutils の入った環境では v6 の exe と同じく数え直した位置を送り、FITS 以外はそこで失敗する
  (そのときログの「画像そのものを送ります」は事実と違う)。`dev/test_online_settings.py` の 5 はこの経路を試していた。ノートブックの文
  (`dev/nb_settings.py` の「送ることがあります」)は外れていないので直していない。~~exe と同じ動きにそろえるかは決めていない~~
  → **同じ日に exe とそろえた**(下の「2026-10-19 の直し」)

直したもの: README の「nova.astrometry.net へ送るもの」とマニュアル 2.3(v6 との違いを 1 段落にし、星が 0 個のときの一文を足した)、
`rs/README.md`、`rust-port.md` 10.2 節の 1 行、この手順書の 0 節の表。［使い方］タブ(`help_tab.rs`)は v6 に触れていないので直していない
(星が 0 個のときの一文は無い。次に exe を建てるときに合わせるか決める)。マニュアル v4 を組み直した(33 ページのまま、4,057,790 バイト
`4efcd99c…ec10`。2 回組んで同じバイト)。6 節の `make_release_assets.py` を公開した v7.0.0 の exe で走らせ、5 つの検査が全部 OK、
`sha256sum -c` OK。マニュアル以外の 5 つの SHA256 は v7 の Release の資産と同じ。`test_make_release_assets.py` 32 本 OK(この exe を渡して)。
~~**v7 の Release の資産のマニュアル(`8bd78680…b04d`)と配布ページのマニュアルは前のまま**(差し替えるかはユーザーの了承を得てから)~~ → 下のとおり差し替えた

**2026-10-19 14:4x に、マニュアルの資産と配布ページのマニュアルを組み直したものに差し替えた**(ユーザーの指示「PR #28 を merge したあと、
Release v7 の資産のマニュアルと SHA256SUMS.txt、配布ページのマニュアルを、組み直したもの(4efcd99c…ec10)に差し替えて」)。

- PR #28 を merge commit で統合(main = `1891069`)。main から `make_release_assets.py`(公開した v7.0.0 の exe)で資産一式を作り直し、
  5 つの検査が全部 OK、`sha256sum -c` OK。マニュアル以外の 5 つの SHA256 は前と同じ(出力は `%USERPROFILE%\zahyou-dist-v7-manual3`)
- Release v7: `gh release upload v7 --clobber` で `zahyou-manual-v4.pdf`(4,057,790 バイト `4efcd99c…ec10`)と `SHA256SUMS.txt`
  (`aa06349d…1f7f`)を差し替えた。本文は SHA256SUMS の行のほかに、「nova.astrometry.net へ送るもの」の「(v6 と同じです)」を README と
  同じ文に直した(PR #28 と同じ訂正。v6 の exe との違いの段落と、星が 0 個のときの一文)。Release から 7 つとも落として `sha256sum -c` OK、
  GitHub の digest も一致、latest は `v7` のまま。exe は変えていないので更新の届き方には関係しない。前の本文・マニュアル・SHA256SUMS.txt・
  資産の一覧はリポジトリの外の `Documents\claude\zahyou\release-backup\v7-manual-swap2-20261019\`
- 配布ページ: 新しいマニュアルを `R:\_claude-tmp\zahyou-manual-swap2\` に置き、一時 cronjob(root)で SHA256 が合うときだけ
  `cat 新しいもの > …マニュアルv4.pdf`。inode(232463)・所有者 AstroKIT:YoFu・644・ACL(owner@・group@・everyone@ の 3 つ)は前と同じ。
  作業フォルダーは root で消し、cronjob も消した。公開 URL(public.php/webdav)から落として `4efcd99c…ec10` で一致、一覧の時刻も新しい。
  `はじめにお読みください.txt` は exe の SHA256 とマニュアルのページ数(33 ページ)だけを書いていて、どちらも変わらないので触れていない
  (nova の段落も「v6 と同じ」とは書いていない)
- そのため、資産のマニュアルはタグ `v7`(b70f73a)の木のものではなく、main の `1891069` のもの(違いは［使い方］の写真と 2.3 の nova の段落)

### 2026-10-19 の直し(ノートブック版も画像ファイルを送るようにした)

上の訂正で分かった v7 のノートブック版の食い違いを、exe と同じ動きにそろえた。直し方はユーザーが 4 つの案から選んだ(案 B)。

- 直したもの: `dev/zahyou_core.py` の `solve_online` で、作った `AstrometryNet()` のインスタンスにだけ `ast._no_source_detector = True` を立てる。
  astroquery 0.4.10 の `solve_from_image` は `force_image_upload or self._no_source_detector` のとき画像を `api/upload` で上げるので、
  photutils の有無によらず画像ファイルそのもの(ファイル名も)を送る = exe(`nova.rs` の `solve_online_inner`)と同じ。非推奨の警告は出ない。
  クラスの既定は変えない(ほかの `AstrometryNet` には効かない)。`publicly_visible = "n"` はそのまま
- 選ばなかった案(偽の nova で確かめた。手元は astroquery 0.4.10・photutils 2.3.0・astropy 7.1.0・Python 3.13):
  - A `force_image_upload=True`: 画像は送るが、0.4.8 から非推奨で呼ぶたびに AstropyDeprecationWarning が出る。予告どおり引数が消えると `**settings` に落ち、
    `_validate_settings` が `ValueError: Setting force_image_upload is not allowed` を出して画像の段が必ず失敗する。案内の pip 行は astroquery の版を固定していない
  - C 自前で `api/upload`(ログイン → multipart → 結果待ち → WCS): astroquery の `solve_from_image` に頼らなくなるが、40〜60 行増え、試験も組み直しになる
  - D ログの文だけ直す: photutils の入った環境で JPEG・PNG・TIFF の 2 段目が失敗するのが残る
- 案 B の弱み: astroquery の内部の属性に頼る。予告どおり photutils の経路が消えれば、旗が読まれなくなるだけで画像は送られる。photutils の経路を残したまま
  属性の名前だけが変わると、直す前の動き(数え直した位置を送る・FITS 以外は失敗)に戻る。試験の `patched_nova` がクラスの `_no_source_detector` を読むので、
  名前が消えれば `dev/test_online_settings.py` が AttributeError で落ちて気づける
- 試験(`dev/test_online_settings.py`。偽の nova で、ネットには出ない): 5 を「photutils ありでも画像を送る」に変えた(FITS と JPEG の 2 通り。送ったファイル名・
  photutils の非推奨の警告が出ないこと・クラスの既定が変わらないこと)。4(星の位置 → 画像)は photutils なし・ありの 2 通りに、6(API キーはログインのときだけ)は
  自前の呼び出しで「2 回受け付けてもログインは 1 回」に。**直す前のコード(main の `1891069`)では新しい 3 本(4 の photutils あり、5 の FITS・JPEG)が落ちる**
  (JPEG は `IORegistryError` で何も送らない)。直したあとは 18/18(正本と、組み立てた直下の `zahyou_engine.py`)
- `python dev/build_nb.py` で組み直した。変わったのは `zahyou_engine.py` だけで、`zahyou_v7.ipynb` のセルは同じ(「中身を差し替えたセル: なし」)。
  README・マニュアル 2.3 は exe の動きで書いてあり、v7 のノートブック版の 2 段目には触れていないので直していない(マニュアルも組み直していない)。
  ノートブックの文(`nb_settings.py` の「それで解けなければ画像そのものを送ることがあります」)はそのまま正しい。本物の nova へ送る試験はしていない。
  `test_notebook_run.py` は手元に nbclient が無いので走らせていない(エンジンの読み込みは `test_online_settings.py` が直下のものを exec して確かめている)
- 6 節の `make_release_assets.py` を公開した v7.0.0 の exe(`5ab3dec6…bae4`)で走らせ、5 つの検査が全部 OK、`sha256sum -c` OK。
  Release v7 の資産と違うのは **`zahyou_engine.py`(`3c0a2b20…3993`、257,532 バイト → `e50459f5…7598`、258,633 バイト)と `SHA256SUMS.txt` だけ**。
  `zahyou_v7.ipynb`(`ab742a79…f784`)・exe・マニュアル(`4efcd99c…ec10`。上の 14:4x の差し替えのあとの値)・`LICENSE.txt`・`THIRD-PARTY-NOTICES.md` は同じ。
  `test_make_release_assets.py` 32 本 OK(この exe を渡して)
  - 新しい worktree では manual の検査が落ちた。`dev/fonts`(.gitignore)の Noto Sans JP を Windows の可変フォントから作り直すと、作るたびに違うバイトになり、
    組んだ PDF も変わるため。PR #28 の worktree(`zahyou-nova-doc`)の `dev/fonts` を写すと通った(HackGen は毎回同じバイト)
- ~~**Release v7 の資産の `zahyou_engine.py` と配布ページの `zahyou_engine.py` は前のまま**(差し替えるかはユーザーの了承を得てから。版は 7.0.0 のまま)~~ → 下のとおり差し替えた

**2026-10-19 15:0x に、エンジンの資産と配布ページのエンジンを直したものに差し替えた**(ユーザーの指示「PR #30 を merge したあとで、
Release v7 の zahyou_engine.py と SHA256SUMS.txt、配布ページの zahyou_engine.py を差し替えてください(版は 7.0.0 のまま)」)。

- PR #30 を merge commit で統合(main = `a5e61e1`)。main の木から `make_release_assets.py`(公開した v7.0.0 の exe `5ab3dec6…bae4`)で資産一式を作り直し、
  5 つの検査が全部 OK、`sha256sum -c` OK。エンジン以外の 5 つの SHA256 は前と同じ(出力は `%USERPROFILE%\zahyou-dist-v7-engine`)
- Release v7: `gh release upload v7 --clobber` で `zahyou_engine.py`(258,633 バイト `e50459f5…7598`)と `SHA256SUMS.txt`(`d2367332…1673`)を差し替えた。
  本文は「`SHA256SUMS.txt` の中身」のエンジンの行だけを書き換えた(元の本文との違いはその 1 行。nova へ送るものの段落は exe の動きで書いてあり、
  ノートブックの 2 段目に触れていないので直していない)。Release から 7 つとも落として `sha256sum -c` OK、GitHub の digest も一致、draft でも
  prerelease でもなく、latest は `v7` のまま。exe は変えていないので更新の届き方には関係しない。前の本文・エンジン・SHA256SUMS.txt・資産の一覧と、
  配布ページの差し替えのスクリプトと記録は、リポジトリの外の `Documents\claude\zahyou\release-backup\v7-engine-swap-20261019\`
- 配布ページ: 新しいエンジンを `R:\_claude-tmp\zahyou-engine-swap\` に置き、一時 cronjob(root)で、今の中身が `3c0a2b20…3993` で新しいものが
  `e50459f5…7598` のときだけ `cat 新しいもの > zahyou_engine.py`。inode(2097915)・所有者 AstroKIT:YoFu・644・ACL(owner@・group@・everyone@ の 3 つ)は
  前と同じ。作業フォルダーは root で消し、cronjob も消した。公開 URL(public.php/webdav)から落として `e50459f5…7598` で一致。
  一覧(PROPFIND)は、落とす前は古い大きさと時刻のままで、落としたあとに 258,633 バイト・15:02:39 JST になった(外部ストレージの変化は触れたときに拾う)。
  `はじめにお読みください.txt` は exe の SHA256 だけを書いていて、nova の段落(「それで解けなければ画像ファイルそのものを送ります」)も今の動きと合うので触れていない
- 前のエンジン(`3c0a2b20…3993`)を落とした人には、photutils の入った環境で 2 段目が画像を送らない(FITS 以外は失敗する)ままになる。版は 7.0.0 のままなので、
  ノートブックの版の表示では見分けられない(ファイルの SHA256 で見分ける)

### 2026-10-19 の直し(動作環境から「リモート デスクトップ」「GPU のドライバーが無い PC」を外した)

ユーザーの指示(親のセッション経由):「掩蔽観測のときリモートデスクトップで使う人はいないはずなので記載しなくてよい」
「掩蔽観測で使用するノートPCには内蔵GPUなどがあるはずなのでGPUドライバーが無いPCについても記載しなくてよい」。マニュアルの資産と配布ページも差し替える。

- PR #32(docs-drop-rdp-gpu、`a718326`)を merge commit で統合(main = `95dcbb9`)。README の「確かめた PC は 2 台です」の囲みと
  マニュアル 3 章の同じ囲みを「RAM 8 GB の PC では、まだ確かめていません。」だけにした(RAM 8 GB は外す指示に入っていないので残した)。
  マニュアル v4 は 33 ページのまま 4,057,726 バイト `ec84638a…4644`(2 回組んで同じバイト)。
  exe の中の文(`rs/app/src/main.rs` 164 行の、画面が開けないときの助言「リモート デスクトップや仮想マシンの中では…」)と、
  開発の記録(`rust-port.md`)・コードのコメントは触っていない(指示どおり。exe を建て直すことになるため)
- main から `make_release_assets.py`(公開した v7.0.0 の exe)で資産一式を作り直し、5 つの検査が全部 OK、`sha256sum -c` OK。
  マニュアル以外の 5 つの SHA256 は、PR #31 で差し替えたあとの Release の資産と同じ(出力は `%USERPROFILE%\zahyou-dist-v7-manual4`)
- Release v7: `gh release upload v7 --clobber` で `zahyou-manual-v4.pdf`(`ec84638a…4644`)と `SHA256SUMS.txt`(`10d169ff…a5ca`)を差し替えた。
  本文は SHA256SUMS のマニュアルの行と、動作環境の「RAM 8 GB の PC・リモート デスクトップ・GPU のドライバーが無い PC では、まだ確かめていません。」を
  「RAM 8 GB の PC では、まだ確かめていません。」に(違いはその 2 行)。Release から 7 つとも落として `sha256sum -c` OK、本文も用意したものと同じ、
  latest は `v7` のまま。前の本文・マニュアル・SHA256SUMS.txt・資産の一覧・配布ページの txt は、リポジトリの外の
  `Documents\claude\zahyou\release-backup\v7-manual-swap3-20261019\`
- 配布ページ: マニュアルと `はじめにお読みください.txt`(動作環境の段落に同じ文があった)を、一時 cronjob(root)で、今の中身と新しい中身の
  SHA256 が両方とも思ったとおりのときだけ `cat 新しいもの > …` で入れ替えた。マニュアルは `4efcd99c…ec10` → `ec84638a…4644`(inode 232463)、
  txt は 15,140 バイト `d69ee6fa…4391` → 15,063 バイト `ea15d975…d664`(inode 2097551。BOM 付き UTF-8・CRLF のまま、その 1 文を 2 行から 1 行にしただけ)。
  所有者 AstroKIT:YoFu・644・ACL(owner@・group@・everyone@ の 3 つ)は前と同じ。作業フォルダー(`R:\_claude-tmp\zahyou-manual-swap3`)は root で消し、
  cronjob も消した。公開 URL(public.php/webdav)から 2 つとも落として SHA256 が一致

### 2026-10-19 の直し(マニュアル 6.3 の「ビニングやレデューサーで画角を広げて」)

ユーザーの指示(親のセッション経由):「マニュアル 6.3 節の「ビニングやレデューサーで画角を広げて」を、6.2 節に合わせて
「レデューサーで画角を広げて(ビニングしても画角は変わりません)」に直す」。マニュアルの資産と配布ページも差し替える。

- PR #34(docs-binning-fov、`48047a2`)を merge commit で統合(main = `de75909`)。6.3 節の囲みを
  「レデューサーで画角を広げてください（ビニングしても画角は変わりません）。」に。マニュアル v4 は 33 ページのまま 4,057,756 バイト
  `92faed8a…eace1`(2 回組んで同じバイト)。同じ書き方は README・［使い方］タブ・exe とエンジンの文・Release 本文・配布ページの txt には無く、
  リポジトリでは v5 の開発メモ `CODE_REVIEW_v5.md` 380 行(「ビニングやモザイクで画角を稼ぐか」)だけ(触っていない)
- main から `make_release_assets.py`(公開した v7.0.0 の exe)で資産一式を作り直し、5 つの検査が全部 OK、`sha256sum -c` OK。
  マニュアル以外の 5 つの SHA256 は今の Release の資産と同じ(出力は `%USERPROFILE%\zahyou-dist-v7-manual5`)
- Release v7: `gh release upload v7 --clobber` で `zahyou-manual-v4.pdf`(`92faed8a…eace1`)と `SHA256SUMS.txt`(`a0297e0a…d53d`)を差し替え、
  本文は SHA256SUMS のマニュアルの行だけを直した。Release から 7 つとも落として `sha256sum -c` OK、本文も用意したものと同じ、latest は `v7` のまま。
  前の本文・マニュアル・SHA256SUMS.txt・資産の一覧は、リポジトリの外の `Documents\claude\zahyou\release-backup\v7-manual-swap4-20261019\`
- 配布ページ: マニュアルだけを、一時 cronjob(root)で、今の中身が `ec84638a…4644` で新しいものが `92faed8a…eace1` のときだけ `cat >` で入れ替えた
  (inode 232463・所有者 AstroKIT:YoFu・644・ACL は前と同じ)。`はじめにお読みください.txt` にはマニュアルの SHA256 もこの文も無いので触れていない。
  作業フォルダー(`R:\_claude-tmp\zahyou-manual-swap4`)は root で消し、cronjob も消した。公開 URL から落として `92faed8a…eace1` で一致

### 2026-10-20 の整理(ブランチ・β のタグ・作業フォルダーを main に一本化)

ユーザーの指示「ブランチとタグが増えすぎなので整理して。main に統合できるものは一本化して」。

- **ブランチは main だけにした**。消したのは 15 本(手元と GitHub の両方): native-solver・native-rs・v6・v7-after-release・
  v7-beta-cleanup・v7-manual-swap〜v7-manual-swap4・v7-engine-swap・nb-nova-image-upload・docs-nova-v6-upload・
  docs-drop-rdp-gpu・docs-binning-fov(と、この整理の PR のブランチ)。v6 以外の 14 本は、main に無い commit が 0 だった。
  v6 ブランチの 2 commit(97215f5・fecece4。2026-08-31)は、main の ecf7618(タグ v6 = Release v6)と同じ作業の前の形で、
  main はその先の b25a42f まで進んでいる(Release v6 の本文のリンクも main の b25a42f を指す)ので、統合せずに消した。
  v6 はタグとブランチが同じ名前で、`git` が「v6 is ambiguous」と言っていたのも、これで無くなった
- **β のタグ 2 つ(v7.0.0-beta.1・v7.0.0-beta.2)を消した**。どちらの commit も main の歴史にあるので失われない。
  消した gui/ などを指す手順と注釈(dev/README.md・rs/README.md・zahyou-net/src/lib.rs・rs/testdata/gen_net.py・この文書の 6 節)は、
  先に commit `0b73d36`(旧タグ v7.0.0-beta.2)を指す形に直した。残るタグは Release が使う v2〜v7 の 6 つ
- **作業フォルダーは `GitHub\zahyou` 1 つ(main)にした**。git worktree の zahyou-rs・zahyou-nova-doc・zahyou-nova-nb を外した
  (どれも変更・未 push 無し)。zahyou-rs の rs/target*(ビルドの作り置き)も一緒に消えたので、次に exe を建てるときは最初から建て直す。
  `dev/fonts` の NotoSansJP は zahyou-nova-doc のものに入れ替えた(main のマニュアル v4 は、このフォントで組むと一字一句同じ PDF になる。
  ほかの 2 組では一致しなかった)
- 整理の前の全部の参照(ブランチ・タグ・stash)は、リポジトリの外の `Documents\claude\zahyou\repo-backup\` に bundle で控えた

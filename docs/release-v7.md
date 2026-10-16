# v7 を配る手順(β版 → 正式版)

2026-10-16 作成、同日に 2026-10-16 の決定(API キーの欄・DirectX 12 を既定・v7 は当面β版)に合わせて書き直した(下書き)。
v6 までの手順(`gui/build_exe.ps1` = PyInstaller、`dev/make_notices.py`)を、cargo の release ビルドの exe・
`rs/THIRD-PARTY-NOTICES.md`・SHA256 の作り直しに置き換えたもの。
ビルドと試験の細かい説明は [../rs/README.md](../rs/README.md)、決定と実測は [rust-port.md](rust-port.md) の 9・10 節
(β版の出し方の元は 10.7 節)。

**v7 は当面β版(7.0.0-beta.N)を GitHub のプレリリースで出す。1〜2 か月使って安定を確かめてから、正式版 7.0.0 を
main に統合してふつうの Release で出す。** 1〜7 節がβ版、8 節が正式版。push・タグ・Release・配布ページの更新は、
どれもユーザーの了承のあとで行う。

---

## 0. 決まったことと、残っていること

決まったこと(2026-10-16。rust-port.md 10.1 節):

| | 決定 | 実装 |
|---|---|---|
| API キー | ［解析］タブの「解析」の枠に欄(伏せ字・「表示」・「元に戻す」)。**既定は元のキー(v6 と同じ)** | gui.json の `api_key` に平文で保存。空なら問い合わせない。環境変数 ASTROMETRY_API_KEY は gui.json に `api_key` が無いときの初めの値だけ。キーの値は文書・ログ・画面の文に書かない |
| 描画 | **DirectX 12 を既定**(主の画面の GPU・小さな塊)。開けなければ OpenGL → ソフトウェア描画(WARP) | 手で選ぶ引数 `--dx12` / `--glow` / `--gpu display\|low\|high\|warp`。想定は「Windows 11 が動く、RAM 8 GB 以上のノート PC」 |
| v6 からの移り方 | **v7 は当面β版。v6 の自動更新の対象にしない** | 版 7.0.0-beta.N をプレリリースで出す(v6 の更新係が見る latest に出ない)。β 自身は v7 の新しいβ版と正式版を探す |

残っていること:

| | 何を決めるか | 今の状態 |
|---|---|---|
| 1 | **画像そのものを外へ送ることへの同意の取り方** | v6 と同じく、送る前に聞かない(「自動」で既定のキーがあると、ネットがあれば星の位置を nova へ送り、解けなければ画像も送る)。README とマニュアルに `決めること:` の印 |
| 2 | β の Release に何を付けるか(exe だけか、THIRD-PARTY-NOTICES.md・LICENSE.txt・マニュアル v4・ノートブック 2 つも付けるか) | 10.7 節は exe だけを挙げている。README は「β の間のノートブックはブランチの直下」と書いた(付けるならタグへのリンクにする) |
| 3 | ノートブックの名前(`zahyou_v6.ipynb` のままか)と版の文字(`dev/nb_main.py` のログの見出しは `zahyou v7-dev`) | ノートブックは β の Release に付けるかどうかと一緒に決める |
| 4 | GitHub の「latest はプレリリースを返さない」「プレリリースは latest にできない」「新しい正式の Release は既定で latest になる」の確かめ | 記憶による(rust-port.md 10.8)。claude-research のセッションで公式の説明を確かめてから出す |
| 5 | 本物の nova へキーで送る道の確かめ | 偽の nova でだけ試した(10.8)。β の間に一度、ユーザーの判断で「オンラインのみ」で解いて確かめる(星の位置と、解けなければ画像が nova へ送られる) |

## 1. v6 の利用者がどうなるか

- v6 の exe の「更新を確認」(起動時にも既定で確かめる)は、`https://api.github.com/repos/Akashi3060/zahyou/releases/latest` の
  資産 **`zahyou.exe`** の digest(SHA256)を手元の exe と比べ、**違えば「新しい版があります」**と出す。版名は見ない。
- **β版をプレリリースで出している間は、latest は v6 のまま**なので、v6 の利用者には何も出ない(0 節の 4 で確かめる)。
- **v7 の正式版を Latest で出した時点で、v6 の全員に「新しい版があります」が出る**。「更新する」で v7 へ入れ替わり、
  v6 の exe が `zahyou.exe.bak`(102 MB)として残る(v7 が消すのは `.old` だけ)。README とマニュアル 9.1 にそう書いた。
- β の利用者が v6 に戻すときは v6 の zahyou.exe を上書きするだけ。設定はそのまま読める(方法は v6 の文言で書く。10.5 節)。
  API キーの欄の値は v6 では使わず、v6 で保存すると消える(次に v7 を開くと既定のキー)。

## 2. 版を上げる

版は **`rs/Cargo.toml` の `[workspace.package] version` の 1 か所**で決まる(10.4 節)。`APP_VER` = `"v" + CARGO_PKG_VERSION`
(窓の題・ログ・［使い方］・更新の確認)、exe の版情報(FileVersion `7.0.0-beta.N`・ProductVersion `v7.0.0-beta.N`・数の版 7.0.0.0・
プレリリースなら VS_FF_PRERELEASE)も付いて変わる。`-beta.N` などのプレリリースの版は、題・起動のログ・［使い方］に「(β版)」が付く。

- β の N を上げるたびにここを直す(`Cargo.lock` の zahyou 自身の版も変わる)。
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
cargo clean --release -p zahyou                    # 埋め込みの表示と版情報を確実に入れ直す
cargo build --release -p zahyou --locked           # lto fat なので 2〜3 分
EXE=$CARGO_TARGET_DIR/release/zahyou.exe
ls -l "$EXE"; sha256sum "$EXE"
rustc -vV; cargo -V; gcc --version | head -1      # Release 本文に書く(作った道具の版)
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
- 撮影に使う画像 `Capture_00001 00_10_33.fits` と、星図データ(既定の `C:\AstrometryData`)は作者の PC にあるもの。

## 6. SHA256 を作る

資産を 1 つのフォルダーに集めてから数える。リポジトリの直下には .gitignore が無いので、**リポジトリの外**に置く
(例 `%USERPROFILE%\zahyou-dist-v7.0.0-beta.N\`。下のコマンドの `dist` はここを指す)。

| 資産の名前 | 元 | β |
|---|---|---|
| `zahyou.exe` | 4 節の `$EXE`(**名前は必ず `zahyou.exe`**。更新の確認はこの名前で探す) | 必ず |
| `THIRD-PARTY-NOTICES.md` | `rs/THIRD-PARTY-NOTICES.md`(exe の［使い方］タブでも読める) | 0 節の 2 |
| `LICENSE.txt` | `LICENSE` の写し | 0 節の 2 |
| `zahyou-manual-v4.pdf` | `天体画像解析プログラム「zahyou」導入・運用マニュアルv4.pdf` | 0 節の 2 |
| `zahyou_v6.ipynb` と `zahyou_engine.py` | ノートブック版 | 0 節の 2・3 |

```powershell
Get-FileHash .\dist\* -Algorithm SHA256 | Format-Table Hash, Path -AutoSize
```

SHA256 は Release の本文に書く。正式版では配布ページの `はじめにお読みください.txt` にも書く(8 節)。

## 7. β版を GitHub に出す(rust-port.md 10.7 節。ユーザーの了承のあとで)

1. `native-rs` を push する(**main には統合しない**)。タグは **`v7.0.0-beta.N`**(v + Cargo の版)を、配る exe を建てた `native-rs` の commit に付ける。
2. Release は **「Set as a pre-release」に印、「Set as the latest release」は付けない**。下書きで作り、中身を見てから出す:

```bash
gh release create v7.0.0-beta.N --repo Akashi3060/zahyou --target native-rs --prerelease --draft \
  --title "zahyou v7.0.0-beta.N (β版)" --notes-file dist/release-notes.md dist/zahyou.exe   # 付ける資産は 0 節の 2
gh release view v7.0.0-beta.N --repo Akashi3060/zahyou --json isPrerelease,isDraft,assets \
  --jq '.isPrerelease, (.assets[]|[.name,.size,.digest]|@tsv)'                              # true と、6 節の SHA256
gh release edit v7.0.0-beta.N --repo Akashi3060/zahyou --draft=false                        # 出す(--prerelease は付いたまま)
```

   (`--latest=false` も付けたいが、gh がプレリリースと一緒に受け付けるかは確かめていない。10.7 節)
3. **公開したβの資産は差し替えない**。直したら N を上げて新しい Release にする(βの更新の確認は「今より新しい版」しか出さないので、
   同じタグの建て直しはβの利用者に届かない)。
4. **間違えて正式の Release(pre-release の印なし)で出すと、そのまま v6 の全員に入れ替えが出る**。印を付け忘れたら、すぐ pre-release に直す。
5. Release 本文に書くこと(v6 の本文を土台に): β版であること・v6 の更新には出ないこと・v6 との行き来(1 節)、ダウンロードの警告(5 枚の図。
   exe の中身によらない)、画面の写真(raw の URL は **`…/zahyou/v7.0.0-beta.N/docs/img/…`(タグ)**で書く。`main` には β の写真が無い)、
   v6 からの変更(WSL 不要・Rust・速さと大きさ。数字は README の「実測値」と同じもの)、API キーの欄と、外へ送るもの(星の位置、解けなければ画像)、
   `zahyou.exe` の SHA256、作った道具の版(rustc・gcc)、動作環境(想定は Windows 11・RAM 8 GB 以上のノート PC、確かめた PC は 1 台だけ)、
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
8. 配布ページ(Nextcloud)には β を置かない(v6 の利用者向けのまま)。置くなら 8 節の手順で、v6 と取り違えない名前にする。

## 8. 正式版(7.0.0)を出す(1〜2 か月使って安定を確かめてから)

1. `native-rs` を main に統合する(PR。`native-rs` は Python 版の `native-solver` を取り込んでいる)。`rs/Cargo.toml` の版を `7.0.0` にし、
   2〜6 節をやり直す(版を上げると「(β版)」は自然に消える)。
2. README を正式版の形にする: β版のタグへのリンク(`releases/tag/v7.0.0-beta.N`)を `releases/latest` に戻し、「いまの v7 はβ版です」と
   「v6 とβ版」の段落、ノートブックの入手先の β の注記を書き換える(README 冒頭の HTML コメントに書いてある)。マニュアルの表紙の「版」の行と
   2.1・9.1・付録 B も同じ。
3. **v6 の Release 本文の画像を v6 のタグに固定する。** v6 の本文は `raw.githubusercontent.com/Akashi3060/zahyou/main/docs/img/app-*.png`
   (`app-analyze`・`app-numbers`・`app-prepare`)を見ているので、main の写真を v7 にすると、v6 の Release ページに v7 の画面
   (WSL の行の無い［準備］タブ)が出る。`main` を `v6` に置き換えた本文で `gh release edit v6 --notes-file …`。**main への統合の前に**行う。
4. タグ `v7.0.0`(main の統合したあとの commit)で **ふつうの Release(Latest)**。ここで v6 の全員と β の利用者に入れ替えが出る(1 節)。
5. 出したあと: `releases/latest` が `v7.0.0`、資産 `zahyou.exe` の digest が 6 節の SHA256、v6 の exe(一時の LOCALAPPDATA)の
   「更新を確認」→「更新する」で v7 が起動し、`zahyou.exe.bak` が残り、v6 で書いた gui.json の設定(方法・目標・配色)が出ること
   (v6 の入れ替えの台本は環境変数を引き継いで v7 を起動するので、v7 も一時の LOCALAPPDATA を使う)。
6. 正式版のあとに v6 の修正版を出すときは `--latest=false`(latest が v6 に戻ると、v6 の利用者向けの意図とずれる。v7 の利用者は
   latest が古ければ入れ替えに誘わないので守られる)。
7. 戻すとき(v7 に問題があったとき): `gh release edit v6 --latest`。v6 の利用者はそのまま。v7 の利用者は latest が古い版なので
   入れ替えに誘われない(10.4 節)— 戻ってほしいなら案内して手で上書きしてもらう。

### 配布ページ(Nextcloud)を差し替える(正式版のとき。ユーザーの了承のあとで)

公開共有(token `M2sNsnA2K4oJ4Co`)は**閲覧のみ**で、`dev/upload_share.py` の PUT は 403。`R:`(YoFu)からも書けない。
実体は `/mnt/tank/TrueNAS/AstroKIT/掩蔽観測班/外部共有フォルダー/天体画像解析プログラム「zahyou」`(所有者 AstroKIT)。

- **今あるファイルの中身を替える**(`zahyou.exe`・`THIRD-PARTY-NOTICES.md`・`LICENSE.txt`・`zahyou_engine.py`・`zahyou_v6.ipynb`・
  `はじめにお読みください.txt`): TrueNAS の root(一時 cronjob)で **`cat 新しいもの > 今のもの`**。inode が変わらないので、所有者・ACL・
  公開共有の設定はそのまま。作業ファイルは書ける `R:\AstroKIT\` に置き、終わったら root で消す(root が書いたものは `chown YoFu:YoFu` しないと
  Windows から消せない)。手順の型は CLAUDE.md の TrueNAS の節と、メモリの zahyou-release-pipeline。
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

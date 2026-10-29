# zahyou-solver の試験

Python 基準実装(`dev/solver_core.py` ほか)と同じ答えになるか、正解(`dev/solver_truth.json` と合成星野の正解)に対して
設計書 8 節の基準(中心 2″ か 0.5 px・スケール 0.2 %・回転 0.1°・鏡像。3I は `criteria_override` で中心 3″・回転 0.2°)を
満たすか、解けてはいけないものを解かないか、締め切り・中止・壊れた入力・壊れた索引で正しくふるまうかを確かめる。

基準値は `rs/testdata/solver_*.json`(`rs/testdata/gen_solver.py` が Python で作る。入力の星・条件と、Python の
`sv_solve` の trace の要点)。星図データ(`C:\AstrometryData`)・基準値・画像が無いものは `SKIP` と出して通す(星図データを使うものは Windows 以外でも。索引の表を開くのは Windows だけ)。

## 回し方

ビルドは Git Bash で `export PATH=/c/msys64/ucrt64/bin:$PATH` してから `rs/` で。

```text
cargo test -p zahyou-solver -- --nocapture                     既定(速いものだけ。1〜2 分)
cargo test -p zahyou-solver -- --ignored --nocapture           重いもの(全部の合成星野・全部の実写・陰性を締め切り 30 秒で。15 分ほど)
cargo test -p zahyou-solver --test synth -- --ignored --nocapture
ZAHYOU_SYNTH_CASES=ra0,tile_corner cargo test -p zahyou-solver --test synth -- --ignored --nocapture
cargo run --release -p zahyou-solver --example bench_solver -- --out rust.json [--python py.json]
python -B rs/testdata/bench_solver.py --out py.json             (Python 側の速さを同じ星・同じ条件で)
python -B rs/testdata/gen_solver.py                             (基準値を作り直す。合成 6〜7 分・実写 10 分ほど)
```

`--nocapture` を付けると 1 件ごとに `PASS` / `FAIL` / `SKIP` / `----`(突き合わせない)の行が出る。

| ファイル | 中身 | 既定 | `--ignored` |
|---|---|---|---|
| `synth.rs` | 合成星野 31 件(5202〜5206・4107〜4119・回転・鏡像・タイルの辺と角・赤経 0°・極・ヒント・スケール・SIP・星 20/300 個・横 30°)を解き、Python と同じ解(索引・対数オッズ 1e-6・一致数・WCS 1e-6 px・quad と仮説の数)、正解の基準、trace | Python で 2 秒未満の 20 件 | 全部(ra0 は Python で 162 秒) |
| `real.rs` | 実写 17 件。(1) Python の検出の星で試行のはしごを Python と同じ順に解く (2) 画像があれば zahyou-image → `solve_offline`(利用者の経路)。正解の基準、基準の画像は目標までのずれ(1.583′・0.918′ 東・1.290′ 北)も | real_orig と +ヒント | 全部 |
| `negative.rs` | 解けてはいけないもの: 乱数の星野(rand60・rand300・rand_hint・一様 150 個)、実写の陰性 12 件(ピンぼけ・光条・dev/tcase の乱数の星野)。締め切りまで探して解なし、超過 1 秒以内。Python が quad を試し尽くしたもの(rand_hint・tcase_few)は試した数も、乱数の星野は最初の 24 個の仮説も Python と比べる。解の判定の 2 つの条件(設計書 3.7 節の 6)が tcase_dense の深い星の誤った解を、どちらか 1 つだけでも止め(探索の終わりの行に見送った候補のまとめが付く)、両方を外すと前と同じ誤った解(仮説 16,187,319 個目)に届くか(`negative_dense_accept_rule`、締め切り 180 秒) | rand60・rand_hint・capture_005514・tcase_few(締め切り 8 秒) | 全部(30 秒。`ZAHYOU_NEG_DEADLINE` で変える)・利用者の経路・判定の条件(約 100 秒) |
| `control.rs` | 中止(立ててから 0.2 秒以内)・過ぎた締め切り・`solve_offline` の中止と全体の締め切り・壊れた入力 21 通り(panic しない・解かない)・中身の壊れた索引(他の索引で解き、ログで知らせる)・4 スレッドで同じ答え | 全部 | — |
| `parts.rs` | 部品: 星の並べ替え・newpoint の quad 作り(総当たりと一致)・コードと正準化・鏡像・AB の範囲・4 点の当てはめ・検証の積み上げ・小さな式・解の判定(試した数で上げる値・tune-up の一致の増え)・判定で見送った候補のまとめ(文 8 通りと、見送らせて試し尽くした探索のログの行が Python と同じこと)、`verify_wcs`(真の WCS・乱数・回転・鏡像・ずらしたもの 30 通りと、Python の解の quad と WCS を渡すと Python の解の対数オッズになること 31 件) | 全部 | — |
| `selfcheck.rs` | 物差しの自己点検: 突き合わせの関数が Python の値そのものなら一致、1 か所ずらすとそこだけを食い違いとして挙げるか(解法は呼ばない) | 全部 | — |

Python との突き合わせの細かさ: 解は索引・quad の 4 点(画像側・索引側)・一致数が同じ、対数オッズ 1e-6、最終の WCS は
画像の 5 × 5 の格子で 1e-6 px、最後の一致の並び(星の番号・ずれ 1e-6 px)、試した quad と仮説の数。trace は使った索引の
一覧(数と sha256)・スケールの範囲・束ごとの quad・当たり・仮説の数・足切りの数・tune-up・最初の 24 個の仮説
(quad の 4 点・鏡像・CRVAL 1e-6″・CRPIX 1e-6 px・CD 1e-9(相対)・スケール・対数オッズ・一致数)・解いた仮説の順番。
Rust の trace は Python と同じく仮説のスケールを 6 桁・対数オッズを 4 桁に丸めて書くので、その 2 つは丸めの半分 + 1e-6 まで許す
(gen_solver.py の Python 側は丸めない値)。

## 環境変数

| 名前 | 既定 | |
|---|---|---|
| `ZAHYOU_INDEX_DIR` | `C:\AstrometryData` | 星図データ |
| `ZAHYOU_TESTDATA` | `rs/testdata` | 基準値の JSON |
| `ZAHYOU_TEST_IMAGES` | ホームフォルダー | `$ZAHYOU_TEST_IMAGES/` で始まる画像の基点(`Downloads/…` が無ければ、その下の `Documents/claude/zahyou/testdata/images/Downloads/…` の控えも見る) |
| `ZAHYOU_PY_REPO` | `~/Documents/GitHub/zahyou` | `dev/realcase`・`dev/tcase`(.gitignore)の画像を探す Python 版のリポジトリ |
| `ZAHYOU_SOLVER_DEADLINE` | 120 | 解ける場合の 1 回の締め切り [秒] |
| `ZAHYOU_NEG_DEADLINE` | 8(既定)/ 30(`--ignored`) | 陰性の締め切り [秒] |
| `ZAHYOU_SYNTH_CASES` | — | 合成星野を名前で選ぶ(カンマ区切り) |

試験は利用者のファイル(`%LOCALAPPDATA%\zahyou` の `cache.json`・`gui.json`)を読みも書きもしない。

## 試験が使う解法の crate の口

- `zahyou_solver::parts`(`order_stars`・`field_order`・`quad_geometry`・`quads_newest`・`field_codes`・`code_variants`・
  `quad_ab_range`・`fit_similarity_quad`・`verify_core`・`logodds_weight`・`f_upper`・`healpix_side_rad`)と `Parity`。
- `solve_detailed`(`SolveOptions.trace` に一時ファイルを渡して trace を書かせ、読んで Python と比べる。`common::solve_traced`)。
  解の quad の 4 点と最後の一致は `SolveDetail` から。
- `verify_wcs`(Python の `sv_verify_wcs`)。`solver_parts.json` の `verify` に真の WCS・乱数の WCS・10° 回転・鏡像・
  少しずらしたものの Python の値が入っている。

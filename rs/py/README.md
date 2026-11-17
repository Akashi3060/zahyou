# rs/py — ノートブック版から Rust の解法を使う拡張モジュール(zahyou-native)

ノートブック版(`zahyou_v<版>.ipynb` + `zahyou_engine.py`)のオフライン解析を、exe と同じ Rust の解法で解くための
Python の拡張モジュール。**利用者は Rust を入れなくてよい**(ビルドした wheel を pip で入れるだけ)。

- 入れ方(利用者): ノートブックのセル A の 2 つ目の pip の行の `#` を外して一度実行する
  (`pip install zahyou-native --no-index --find-links https://github.com/Akashi3060/zahyou/releases/expanded_assets/v<バージョン>`)。
  `--find-links` は、そのバージョンの Release のページの添付ファイルから wheel を探す。`--no-index` で PyPI は見ない
  (同じ名前の別物を入れないため)。入れなくてもノートブックは今までどおり Python の解法で動く。
- 対応: Windows x86_64・Mac(Apple シリコンと Intel。universal2)・Linux x86_64・Linux aarch64(manylinux 2.28。Google Colab を含む)。
  Python 3.9 以降(abi3 の wheel 1 つでどのバージョンでも動く)。
- 速さ(2026-11-17、Core Ultra 7 265K、合成星野のブラインド): ra0 217 → 6.0 秒、tile_corner 36 → 2.0 秒、polein 40 → 2.2 秒。
  解は Python と同じ(インデックス・対数オッズ・一致の数・試した仮説の数まで)。

## 中身

| ファイル | 中身 |
|---|---|
| `Cargo.toml` | クレート `zahyou-native`(cdylib、PyO3 0.25、abi3-py39)。**exe の workspace(`../Cargo.toml`)には入れない**(exe のビルド・テスト・同梱物の表示に影響させない)。そのため版は受け継げず、`version` を手で exe と同じにする(テスト `version_in_docs_matches_cargo` が見る。`docs/release-v7.md` 2 節の表の 12) |
| `Cargo.lock` | 依存のバージョン(`--locked` でビルドする) |
| `pyproject.toml` | maturin の設定。表示を wheel の `.dist-info/licenses/` に入れる |
| `src/lib.rs` | `zahyou_native.solve(...)` と `API`・`__version__` |
| `THIRD-PARTY-NOTICES.md` | wheel の同梱物の表示(`python rs/tools/gen_notices.py --wheel` が作る。zahyou の LICENSE も冒頭に入る) |

`zahyou_native.solve(xy, flux, width, height, index_dir, scale_lo=None, scale_hi=None, hint=None, timeout=None, cancel=None, log=None)`
は Python の `sv_solve` と同じ探索をして、解の dict(`wcs` = crval_xyz・crpix(0 始まり)・cd・SIP の a / b / ap / bp、
`logodds`・`nmatch`・`index_name`・`hypotheses` など)か None を返す。中止は `InterruptedError`。
解法は別のスレッドで走り、呼んだスレッドは GIL を放して待つ(0.1 秒ごとにログを渡し、`cancel()` を呼び、Ctrl+C を見る)。
目録(インデックスファイルの一覧)は呼ぶたびに作って閉じる(Windows でインデックスファイルを開いたままにしない)。

エンジン側(`dev/zahyou_core.py`)は `native_solver()` で読み込み、`_solve_stars` で `sv_solve` の代わりに使う。
`API` が `_NATIVE_API` と違えば使わない(呼び方を変えたら両方を上げる)。**環境変数 `ZAHYOU_SOLVER=python` で Python の解法に戻せる**。
解析のログの「解析エンジン: Rust 版 / Python 版」の行で、どちらで解いたかが分かる。
Python のテスト(`dev/test_solver_*.py`)は `sv_solve` を直接呼ぶので、拡張を入れても Python の解法を確かめる。

## ビルドする(手元で試すとき)

```bash
export PATH=/c/msys64/ucrt64/bin:$PATH
export CARGO_TARGET_DIR=$TEMP/zahyou-py-target       # rs/target は exe 用。共有しない
python -m venv $TEMP/venv-maturin && $TEMP/venv-maturin/Scripts/python -m pip install maturin
cd rs/py && $TEMP/venv-maturin/Scripts/maturin build --release --locked --out $TEMP/wheels
```

**手元でビルドした wheel は配らない**(依存のパニック位置情報にビルドした人のホームフォルダーのパスが残る。
`make_release_assets.py` の wheels の検査が落とす)。配るのは GitHub Actions でビルドしたもの。

## ビルドする(配るもの): GitHub Actions の `wheels` ワークフロー

`.github/workflows/wheels.yml`。4 つの OS・CPU 向けにビルドし、入れて `solve` を呼べることを確かめ(インデックスファイルの無いフォルダーで
「解なし」になるまで)、成果物 `zahyou-native-wheels` に 4 つをまとめる。Release には触れない。
動くのは、`rs/py`・解法のクレート・ワークフローを変えた PR、`v*` のタグの push、手で動かしたとき。

```bash
gh workflow run wheels.yml --repo Akashi3060/zahyou --ref main          # 出す commit で
gh run list --repo Akashi3060/zahyou --workflow wheels.yml --limit 1     # 終わるのを待って ID を見る
gh run download <ID> --repo Akashi3060/zahyou -n zahyou-native-wheels -D "$WHEELS"
```

そのあと `make_release_assets.py --wheels "$WHEELS"` で、ほかの配布物と一緒にそろえて SHA256SUMS.txt に入れ、
Release に添える(`docs/release-v7.md` の 6 節・8 節)。**Release に wheel が無いと、セル A の 2 つ目の行は
「No matching distribution」で失敗する**(ノートブックは Python の解法で動き続ける)。

## 依存を変えたら

```bash
python rs/tools/check_licenses.py --wheel     # Windows・Mac・Linux のどれかで使う依存を全部見る
python rs/tools/gen_notices.py --wheel        # 表示を作り直す(--check で古いかだけ見る)
```

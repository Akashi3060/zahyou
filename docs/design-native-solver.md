# 設計書: WSL を使わない自前の解析エンジン(native-solver)

2026-10-15 作成。ブランチ `native-solver`。

## 0. 目的と範囲

- オフライン解析を **WSL も astrometry.net の実行ファイルも使わず**、zahyou のプロセスの中で行う。
- 星図データは今までと同じ **astrometry.net の索引ファイル**(`index-4107`〜`4119`、`index-5202`〜`5206-NN`)を
  そのまま読む。配布物・置き場所(`C:\AstrometryData`)・［準備］タブのダウンロードは変えない。
- 解き方は Lang et al. (2010) の方式(4 星の幾何ハッシュ → 照合 → ベイズ検証 → 改良)を、論文と、astrometry.net の
  公開文書と BSD-3 の部分のソースを読んでまとめた調べ物から自分で書き直す。astrometry.net のコードは取り込まない
  (MIT のまま配る。11 節)。
- この Python 版は、次の段階で別の言語(Rust など)へ移すときの**基準実装**も兼ねる。移植先で同じ値が出るかを
  確かめられるよう、途中の値を書き出せる作りにする(12 節)。

変えないもの: オンライン解析(nova.astrometry.net)、天体名の解決、図 2 枚、結果の数字、設定ファイル
(`gui.json`・`cache.json`)、ノートブック版と GUI 版が同じ `zahyou_engine.py` を使うこと。

## 1. 全体の構成

```
dev/solver_wcs.py     幾何: 単位ベクトル ↔ 赤経赤緯、TAN 投影、TAN/SIP の当てはめ、FITS ヘッダー
dev/solver_index.py   索引ファイル: FITS の表を生のバイトで読む、kd-tree の探索、タイルの選び方、検査
dev/solver_core.py    解法: 星の並べ替え、quad 作り、コード、照合、検証(対数オッズ)、改良、探索の本体 sv_solve
dev/solver_synth.py   試験用: 索引の星から「正解つきの合成星野」を作る(エンジンには入れない)
dev/zahyou_core.py    既存。WSL の部分(_bash・run_solve_field など)を消し、solve_offline が sv_solve を呼ぶ
gui/zahyou_env.py     既存。WSL の導入・設定ファイルを消し、星図データの検査を足す
gui/zahyou_gui.py     既存。［準備］タブの WSL の行、中止(pkill)、文言を直す
```

- `build_nb.py` は `solver_wcs` → `solver_index` → `solver_core` → `zahyou_core` → `zahyou_plot` → `nb_ui` → `nb_main`
  の順につないで `zahyou_engine.py` を作る(ノートブック版と GUI 版が同じ 1 ファイルを使う今の形を保つ)。
- **つなぐための約束**(`strip_module_head` が冒頭のコメント・import だけを外すため):
  - 各 `solver_*.py` は「`#` のコメント → import → 本体」の順。冒頭に docstring を置かない。
  - ほかの solver モジュールの名前は冒頭の `from solver_wcs import *` などで取り込む(組み立て時に消える)。
  - 1 つの名前空間に入るので、**solver_*.py の最上位の名前はすべて `sv_` / `Sv` / `_sv_` / `SV_` で始める**。
  - numpy と標準ライブラリだけを使う(scipy・astropy は使わない。移植しやすく、依存も増やさない)。
- 画素の座標は**0 始まり**(x = 列、y = 行。`detect_sources` と同じ)。FITS のヘッダーに書くときだけ CRPIX に 1 を足す。

## 2. 索引ファイル(`solver_index.py`)

形式は実物を解読して確かめた([index-format.md](index-format.md))。要点:

- 列が 1 つの BINTABLE が並ぶ。表は TTYPE1 の名前(`quads`、`kdtree_{header,lr,split,range,data}_{codes,stars}`、`sweep`)で探し、
  **生のバイト列をリトルエンディアン**で読む(`ENDIAN = '04:03:02:01'`。違う値なら読まずにエラー)。4100 系の MAG 表だけは標準の FITS どおりビッグエンディアン。
- 整数 → 実数は `minval + int / scale`。星は単位ベクトル(長さを 1 に直してから使う)。
- コード i = quad i、quad の星番号 = 星のデータの行番号(並べ替え表は無い)。
- kd-tree はヒープ順の完全二分木。葉 k はデータ `lr[k-1]+1 … lr[k]`。split の下位 ⌈log₂D⌉ ビットが分割次元、残りが分割位置。
  左は `x[dim] ≤ val`、右は `x[dim] ≥ val`。**この木をそのまま使って探す**(作り直さない。読み込みは mmap、整数のまま)。
- 5200 系のタイル番号は nside 2 の **xy 方式**の HEALPix(`NN = base·4 + x·2 + y`)。星はタイルの中だけにある。

```python
class SvIndexFile:                       # 1 ファイル。ヘッダーは開くときに読み、表は使うときに mmap する
    path, name, indexid, healpix, hpnside, dimquads, nquads, nstars
    scale_lo_rad, scale_hi_rad           # quad の AB の角距離の範囲(SCALE_L / SCALE_U)
    jitter_arcsec                        # 星の木のヘッダーの JITTER(無ければ 1.0)
    cutnside                             # 一様化の HEALPix の nside
    tile_center_xyz, tile_radius_rad     # 5200 系のタイルの中心と外接半径(全天の 4100 系は None)
    def code_ball(queries: (Q,D) float, radius: float) -> (qi: int[], quad: int[])   # ‖q − c‖ ≤ radius をすべて
    def quad_stars(quad_ids) -> (n, dimquads) int
    def star_xyz(star_ids) -> (n,3) float     # 長さ 1 に正規化済み
    def stars_in_cone(center_xyz, radius_rad) -> int[]
    def sweep(star_ids) -> uint8[]

class SvIndexCatalog:                    # 置き場所のフォルダー全体
    def __init__(index_dir)              # index-*.fits を走査し、ヘッダーを検査(12 節の validate)
    files, problems, series              # series: indexid → [SvIndexFile](5200 系は 48 タイル)
    def coverage_arcmin() -> (lo, hi)    # 手持ちの段の quad の大きさの範囲
    def select(scale_lo, scale_hi, width, height, hint_xyz=None, hint_radius_rad=None) -> [SvIndexFile]
def sv_validate_index_file(path) -> (ok: bool, message: str)   # 大きさ = ヘッダーから計算した長さ、INDEXID と名前、HEALPIX とタイル番号
```

- `code_ball` は多数の問い合わせをまとめて受け、木を幅優先で一斉にたどる(numpy の配列演算。節点ごとの箱は
  たどりながら狭める)。距離の比較は整数の単位で行い、量子化の分(D × 0.25 目盛り²)だけ半径を広げてから実数で最終判定する。
- `select`: 画像の短辺・長辺と画素スケールの範囲から、quad の大きさ(ピクセル)が
  `[0.1 × min(W,H), hypot(W,H)]` に入りうる段だけを選ぶ(Lang et al. と solve-field の既定)。ヒントがあれば、5200 系はヒントの円
  (半径 + 画像の対角の半分)と重なるタイルだけ。何も重ならなければ最も近い段。

## 3. 解法(`solver_core.py`)

### 3.1 入口

```python
@dataclass
class SvParams:
    codetol: float = 0.01                 # コードの許容誤差(4 次元の L2)
    distractor: float = 0.25              # 画像の星が偽物である割合
    verify_pix: float = 1.0               # 星の位置誤差 [px]
    logodds_solve: float = ln(1e9)        # これ以上で解(Lang et al. の効用から)
    logodds_tune: float = ln(1e6)         # これ以上・solve 未満なら 1 次で改良して検証し直す
    logodds_bail: float = ln(1e-100)      # 検証の打ち切り
    parity: str = "both"                  # "both" / "normal" / "mirror"
    max_quad_stars: int = 60              # quad 作りに使う星(明るい順)の上限
    max_verify_stars: int = 300           # 検証に使う星の上限
    quad_min_frac: float = 0.1            # quad の AB の最小 = 0.1 × 短辺
    uniformize_boxes: int = 10            # 探索の前の並べ替え(画面全体に散らす)
    tweak_order: int = 2                  # 改良の SIP の次数(星が足りなければ下げる)

@dataclass
class SvSolution:
    wcs: SvWcs                            # TAN(+SIP)。crpix は 0 始まり
    logodds: float; nmatch: int; ndistract: int; nfield: int; nref: int
    index_name: str; quad_field: (4,) int; quad_index: (4,) int
    quads_tried: int; hypotheses: int; seconds: float
    matches: list[(field_i, ref_xyz, dist_px)]

def sv_solve(xy, flux, width, height, catalog, *, scale_lo=None, scale_hi=None,
             hint=None,          # (ra_deg, dec_deg, radius_deg) か None
             deadline=None,      # time.monotonic() の締め切り
             cancel=None,        # 呼ぶと True/False を返す関数。True なら SvCancelled を投げる
             log=None,           # 1 行ずつの文字列を受ける関数
             params=None, trace=None) -> SvSolution | None
```

- 星は**明るい順に並んだ** (x, y)(0 始まり)。`flux` は並べ替えの検算と同率の扱いに使う。
- スケール不明なら、手持ちの段から画像の幅で逆算した範囲(今の `blind_scale_bounds` と同じ)。
- 締め切りと中止は、quad の束を 1 つ処理するごと・検証 1 回ごとに確かめる(1 秒以内に止まること)。

### 3.2 星の並べ替え

1. 明るい順(フラックスの降順)。
2. **一様化**(Lang et al. / solve-field の既定): 画像を NX × NY の箱(NX = max(1, round(√(n·W/H)))、NY = max(1, round(n/NX))、
   n = `uniformize_boxes`)に分け、「各箱の 1 番目を元の順に → 各箱の 2 番目 → …」と並べ直す。明るい星が一部に固まっていても、
   探索の序盤に画面全体の星が入る。
3. quad 作りには先頭 `max_quad_stars` 個、検証には先頭 `max_verify_stars` 個を使う。

### 3.3 quad 作り(newpoint 方式)

星を 0, 1, 2, … と並べた順に 1 つずつ足し、**新しく足した星 n を必ず含む quad だけ**を作る(上位 n 個の星で作れる
quad を n の小さい順に全部試すことになる)。

- (a) n を AB の一方にする: A = m < n。|AB| が範囲内(3.5)なら、m・n 以外で番号 < n の星のうち **AB を直径とする円の中**の星から
  内側の星(4 星なら C, D の 2 個、3 星なら C の 1 個)を選ぶ。
- (b) n を内側の星にする: 番号 < n の組 (A, B) で、|AB| が範囲内かつ n が円の中なら、4 星なら残り 1 個を番号 < n の円の中の星から選ぶ。
- 円の中なら AB は自動的に最も離れた組になる(索引側と同じ性質)。
- 計算は numpy でまとめて行う。quad は「足した星の束」(例: 星 4〜8 個目、9〜12 個目、…)ごとに作って照合する。

### 3.4 コードと照合

- 画像側のコード(ピクセルのまま。A → (0,0)、B → (1,1) の相似変換):
  `s = dx² + dy²`、`cosθ = (dx + dy)/s`、`sinθ = (dy − dx)/s`、星 P は `x = u·cosθ + v·sinθ`、`y = −u·sinθ + v·cosθ`
  (u, v は P − A)。コードは (cx, cy, dx, dy)(3 星は (cx, cy))。
- 正準化(索引と同じ): 4 星は `cx + dx > 1` なら A↔B(全成分 v → 1 − v)、続いて `cx > dx` なら C↔D。3 星は `cx > 1/2` なら A↔B。
- **境界の近くは両方試す**: `|cx + dx − 1| < 1.5·codetol` なら A↔B した形も、`|cx − dx| < 1.5·codetol` なら C↔D した形も問い合わせる
  (3 星は `|cx − 1/2| < 1.5·codetol`)。
- **鏡像**: (x, y) を入れ替えた形 (cy, cx, dy, dx) を作り、同じく正準化して問い合わせる(parity = both の既定)。
  索引のコードは x = 北・y = 東で計算されているので、FITS の向き(y が上)の普通の空は「そのまま」で、鏡像の空や
  行 0 が上端の画像は「入れ替え」で当たる。どちらを当たりにするかは当てはめ(3.6)が決めるので、両方を常に試せばよい。
- 照合: `code_ball(q, codetol)`。当たった各 quad について、画像側の星の順(正準化で入れ替えた順)と索引の quad の星 A, B, C, D を
  1 対 1 に対応させる。

### 3.5 quad の大きさの範囲(索引ごと)

画素スケールの範囲 [s_lo, s_hi](″/px)と索引の SCALE_L, SCALE_U(″)から

```
fudge = SCALE_U × codetol / (√2 × s_hi)
minAB = max(SCALE_L / s_hi − fudge, quad_min_frac × min(W, H))
maxAB = SCALE_U / s_lo + fudge
```

照合のあとに安く捨てる: 索引の A–B の角距離 ÷ 画像の |AB|(ピクセル)が [s_lo·(1 − codetol), s_hi·(1 + codetol)] の外なら捨てる。
ヒントがあれば、索引の quad の中心がヒントの円(半径 + 画像の対角の半分)の外なら捨てる。

### 3.6 4 点からの当てはめ

接点 r = 4 星の単位ベクトルの和の方向。各星を r で TAN 投影(東 ξ、北 η、ラジアン)し、画像側の点と重心を引いて
`C = Σ fᵢ pᵢᵀ` を特異値分解(`C = U S Vᵀ`、`R = V Uᵀ`。**行列式の符号は見ない = 鏡像も表せる**)、
`scale = √(Σ|pᵢ|² / Σ|fᵢ|²)`、`CD = scale × R`、CRPIX = 画像側の重心、CRVAL = r。
画素スケールが [s_lo, s_hi] の外なら捨てる。

### 3.7 検証(対数オッズ)

Lang et al. (2010) 2.4 節の判定。仮説「この位置合わせは正しい」と「偶然」の尤度比 K を、画像の星を 1 つずつ見て積み上げる。

1. 参照星: 画像中心から半径 `hypot(W,H)/2 × scale × 1.05` の円の中の索引の星を、**同じ系列の全タイル**から集める
   (タイルの境界をまたぐ視野でも星が欠けないように)。画像に投影し、画像の中(−0.5 ≤ x ≤ W−0.5、同 y)だけ残し、
   `sweep` の小さい順(= 明るい順)に並べる。quad の 4 星は参照星からも画像の星からも外す。
2. 位置の分散: `pix² = verify_pix² + (jitter / scale)²`、`σᵢ² = pix² × (1 + Rᵢ² / Q²)`
   (Rᵢ = 星 i と quad の中心 qc = (A+B)/2 の距離、Q² = |AB|²/4)。4 点からの解は quad から離れるほど誤差が大きいため。
3. 関連半径: `ror² = Q² × max(1, W·H·(1 − d) / (4π · NR · pix²) − 1)`。qc から ror より遠い星は画像側も参照側も外す。
   有効面積 A は画像のうち ror の円に入る部分(画像を 10 × 10 に区切って数える近似でよい)。
4. 画像の星の並び: 検証の一様化(画像を「索引の CUTNSIDE の HEALPix の 1 辺」の大きさの箱に分け、各箱から 1 個ずつ順に)。
5. 積み上げ(d = distractor、NR = 参照星の数、μ = それまでの一致の数):
   ```
   logbg = −ln A
   logd(μ) = ln(d + (1 − d)·μ / NR) + logbg        # 偽物として説明したときの尤度
   各星 i について:
     nn = 5σᵢ 以内で最も近い参照星
     logfg = ln((1 − d) / (2π σᵢ² NR)) − dist²/(2σᵢ²)(nn が無ければ −∞)
     logfg < logd(μ) なら偽物とし、logfg = logd(μ)
     nn がすでに別の星 j と組になっていたら、j を外して i に付け替えたほうが尤度が上がるときだけ付け替える(1 対 1)
     一致なら μ += 1
     logodds += logfg − logbg
     logodds < logodds_bail なら打ち切り
   返すのは、並びの先頭から何個かまでを使ったときの logodds の最大値
   ```
6. 判定: `logodds ≥ logodds_solve` で解。`logodds_tune ≤ logodds < logodds_solve` なら 1 次で改良(3.8)して検証し直す。
   最初に解になった仮説で探索を終える。

### 3.8 改良(tweak)

- 一致した組を、今の WCS で投影し直した参照星との最近傍で取り直し、重みつき最小二乗で TAN(CD と CRVAL)を当て直す。
  これを 10 回、σ の「quad からの距離による広がり」を `γ = 0.7^k` で弱めながら繰り返す。
- 当てはめの CRPIX は反復でも最後でも**画像の中心**に置き、接点の初めの値は今の WCS で画像の中心が写る向きにする
  (当てた点の重心に置くと、点が片側に寄ったとき広い視野では接点の違う TAN になり、40° で中心が 5〜6 px、
  星が左 1/3 だけの 60° で 7〜15 px ずれた解を返した)。
- 当てはめ(`sv_fit_wcs_lsq`)は、残った定数項(CRPIX が写る (ξ, η) を画素に直したもの)が 1e-8 px 以下になるまで
  接点を動かし直す(上限 40 回。上限で 0.01 px を超えて残れば当てはめの失敗)。3 回で打ち切っていた版は、
  星が CRPIX から離れて固まる広い視野で数 px 残した。
- 解として返す前の最後の当て直しは、quad の星を除いた一致が 6 個以上あれば一致だけで行う(quad には偽の星が
  入りうる)。tune-up では quad の星も入れる。
- SIP 2 次(逆変換は 3 次を格子で当てる)を付けるのは、次を全部満たすときだけ。足りなければ TAN のまま。
  (1) 一致が 3 × (未知数) 以上、(2) 一致した星が画像を 3 × 3 に分けた箱の 7 つ以上にある(片寄った星で当てると、
  星の無い側へ外挿して隅が 7〜13 px 動いた)、(3) TAN に比べた残差の減り方が F 検定の上側 1 % で有意
  (「自由度で割った残差が下がれば採る」は F > 1 と同じで、歪みが無くても 4 割ほど通った)。
- 最後に γ = 1 で検証し直し、その対数オッズと一致数を結果にする。CRPIX は画像の中心(結果の読みやすさのため)。
- solve を超えた仮説の改良の途中で締め切りが来ても、改良を済ませて解を返す(解は見つかっているので)。

### 3.9 出力

`SvWcs.to_fits_header(width, height)` が astropy で読めるヘッダーを返す:
`CTYPE1/2 = 'RA---TAN'/'DEC--TAN'`(SIP があれば `-SIP`)、`CRVAL`、`CRPIX`(1 始まり)、`CD1_1 … CD2_2`、`IMAGEW`/`IMAGEH`、
SIP の `A_ORDER`・`A_p_q`・`B_…`・`AP_…`・`BP_…`、それに `COMMENT` で索引名・対数オッズ・一致数・試した quad の数。
`RADESYS = 'ICRS'`(索引の星表は ICRS)。

## 4. 探索の段取り(`zahyou_core.solve_offline`)

今の「試行のはしご」の考え方(ヒント+狭いスケール → 全天+全域)を保ち、**全体を 1 つの締め切り(壁時計)で厳密に守る**。

| 順 | 条件 | ヒント | スケール | 残り時間の何割まで |
|---|---|---|---|---|
| 1 | ヒントあり・スケール既知 | あり | 既知 ±25% | 25% |
| 2 | ヒントあり | あり | 全域 | 25% |
| 3 | スケール既知 | なし | 既知 ±25% | 25% |
| 4 | 常に | なし | 全域 | 50% |

- 条件ごとに、**深い検出の星 → 標準の検出の星**の順に試す(星が 3 個未満の組は飛ばす。最後の試行は残り全部)。
  「標準」は図に緑の丸で出す `detect_sources` で、止める数を 12 から 60 に上げたもの(明るい順に最大 400)。
  「深い」は解析エンジンのためだけの `detect_sources_deep`: 星の幅に合わせてぼかした像の局所の極大を、雑音だけの画像で
  偽の極大が数個になるしきい値で拾い、肩・伸び・十字のホットピクセル・まわりの輪より明るくないものを捨てる
  (星の少ない 3I の画像で星が 8 → 15 個になった)。sv_solve は quad を試し尽くせば締め切り前に戻るので、狭い条件は
  両方の星で試しても安い。
- 残りが 2 秒を切ったら次の試行を始めない。
- ログは今と同じ形: `[i/N] ラベル  (最大 t 秒)` → `✅ 解析成功 (ラベル)`。失敗時の案内(星が少ない・段が足りない・焦点距離)は今のまま。
- **中止**: GUI が `threading.Event` を渡し、`cancel` で確かめる。WSL の `pkill` は消す。

## 5. エンジンとの接続(守る約束)

- `solve_offline(bundle, sources, work_dir, timeout=300, focal_length_mm=None, hint_radec=None, hint_radius_deg=5.0,
  verbose=True, diagnostics=None, index_dir=None, prep=None, cancel=None) -> fits.Header | None`
  - 例外は投げない(中止のときも None)。返すヘッダーは `WCS(normalize_wcs_header(h, shape))` が celestial で、CRPIX は 1 始まり。
  - `prep = (sub, mask, sigma)`(`run()` が持っている前処理の結果。深い検出に使う。無ければ作り直す)。
- `solver_diagnostics(index_dir=None) -> {"ok", "messages", "indexes", "cfg", "paths"}`: フォルダーを走査して索引を検査する。
  `cfg` は空文字、`paths` は `[index_dir]`(互換のため形は残す)。
- 新しい設定(ノートブックのセル C と GUI の `_params`): `INDEX_DIR`(既定 `C:\AstrometryData`)、`CANCEL_EVENT`(GUI のみ)。
- 消すもの: `_run`・`_bash`・`wsl_available`・`to_solver_path`・`run_solve_field`・`_wcs_from_dir`・`_clean_outputs`、
  `write_xylist`・`write_solver_fits`(デバッグ用に残す必要がなくなる)、WSL の案内文。
- ついでに直すもの(新しいソルバーの精度と表示に関わるため):
  - ベイヤー画像の 2×2 平滑化で星の位置が (+0.5, +0.5) px ずれる → 対称な 3×3 の二項カーネルにする。
  - 「画面上方向の位置角」と「鏡像」を、表示の上下反転(`flip_y`)を考えた画面の見た目で出す。

## 6. GUI の変更

- ［準備］タブ: 表は「星図データ」「検査」「総合」の 3 行。「個別に: WSL / astrometry.net / 設定ファイル」の行を消す。
  「まとめて準備する」は足りない段を落とす(おすすめ)だけ、「状態を確認」は走査と検査。ダウンロードは `.part` に書いてから
  名前を変え、ヘッダーを検査する。
- ［解析］タブ: 方法の選択肢「オフライン (WSL) のみ」→「オフラインのみ」。`gui.json` の古い文言はそのまま読み替える。
  「オフライン解析: 使えます」は星図データだけで判断する。
- 中止: `threading.Event` を立てる(数秒以内に止まる)。
- 版の表示: `APP_VER = "v7-dev"`。**試作版では更新の確認をしない**(しないと v6 の exe へ「更新」されてしまう)。
- ［使い方］タブ: WSL の 4 工程の説明を消し、星図データだけでよいことと、ライセンス(11 節)を書き直す。

## 7. 性能の目標

| 場面 | 目標(Python 版) | 備考 |
|---|---|---|
| 968×548 の実写、ヒントなし | 5 秒以内 | WSL 版は 3〜4 秒(うち 2.5 秒は 9P 越しの索引の読み込み) |
| 同、ヒントあり | 2 秒以内 | |
| 解けない画像 | 締め切り(既定 300 秒)を 1 秒以上超えない | WSL 版は最大 45 秒超えていた。試験の基準(`solver_truth.json` の `negative_overrun_s`)はこの値 |
| 中止 | 押してから 2 秒以内 | WSL 版はほぼ効かなかった |
| メモリ | 索引は mmap。解析中の増加 500 MB 以内(全天探索でも) | 最小構成 4 GB の PC を保つ |

Python 版では「難しい画像」(WSL 版で quad を数十万個試すもの)は遅くてよい。速さの本命は第 2 段の移植。

## 8. 試験

| ファイル | 中身 | 合格の基準 |
|---|---|---|
| `dev/test_solver_wcs.py` | TAN の往復・astropy との一致・当てはめ(広い視野で接点が収束するまで)・FITS ヘッダー | 全件一致 |
| `dev/test_solver_index.py` | 索引の読み込み(コードを星から計算し直して保存値と整数まで一致)、kd-tree の探索 = 総当たり、ファイルの検査(中身の壊れた索引も) | 全件一致 |
| `dev/test_solver_core.py` | quad 作り・コード・照合・検証の対数オッズ・改良(広い視野で星が片寄っても中心 0.5 px、SIP の判定)・締め切りと中止・壊れた索引 | 項目ごと |
| `dev/test_solver_synth.py` | 索引の星から作った合成星野(5202〜5206・4107〜4110・三角形の 4111〜4119、回転・鏡像・スケール・タイル境界・赤経 0°・極付近、星の欠け 30%・偽の星 30%)をブラインドとヒントで解く | 中心 2″(か 0.5 px)以内、スケール 0.2%、回転 0.1°、鏡像一致。一様乱数の星野は 1 つも解けない |
| `dev/test_solver_real.py` | WSL 版の正解(`dev/solver_truth.json`)と同じ画像(手元にあるときだけ)を解く | 中心 2″、スケール 0.2%、回転 0.1°、鏡像一致。ピンぼけ・光条・乱数の星野は解けない、かつ締め切りを 1 秒以上超えない(7 節) |
| `dev/test_robust.py` | 既存(読み込み・検出) | 既存どおり |
| `gui/test_*.py` | 既存(WSL の差し替えを外す) | 既存どおり |

`test_offline_sim.py`(solve-field の CLI のスタブ)と `test_wsl_e2e.py`・`setup_wsl.ps1` は役目を終えるので消す。

## 9. 偽の解を出さないために

- 判定の閾値は Lang et al. の効用から出る 1e9 のまま(WSL 版で解けなかった画像の偽の候補は最大 e^17.7)。
- 一様乱数の星野・ピンぼけ・光条の画像を回帰試験に入れ、締め切りまで探しても解が出ないことを毎回確かめる。
- 解の妥当性(`_sanity_ok`)は今のまま残す。

## 10. 置き場所・設定

- 星図データの置き場所は GUI の［準備］タブの欄(`gui.json` の `indexdir`)を**エンジンへ直接渡す**(今は WSL の
  `/etc/astrometry.cfg` を経由していた)。ノートブック版はセル C の `INDEX_DIR`。

## 11. ライセンスと出典

- 解き方: Lang, D., Hogg, D. W., Mierle, K., Blanton, M., & Roweis, S. 2010, AJ 139, 1782(arXiv:0910.2233)。論文と、
  astrometry.net の公開文書と BSD-3 の部分のソース(solver.c・verify.c・tweak2.c など)を読んでまとめた調べ物
  (ソースの表現は写さず、式と条件を自分の言葉で書き直したもの)をもとに自分で書いた。エンジンを書くときにはソースを
  読んでいない。GPL の部分(qfits など)は調べ物でも読んでいない。astrometry.net のコードは含まない。
  FITS の読み込みは FITS 規格から書いた。
- 索引ファイルの形式: 実物を読んで調べた([index-format.md](index-format.md))。
- 索引ファイルは**同梱しない**(利用者が［準備］タブで取ってくる)。中身の星データには元の星表の条件がかかる:
  - 4100 系: Tycho-2(Høg et al. 2000, A&A 355, L27)。
  - 5200 系: Gaia DR2(ESA/Gaia/DPAC。CC BY-NC 3.0 IGO、商用利用は ESA の許可が要る)と Tycho-2。
  - Gaia の定型の謝辞を THIRD-PARTY-NOTICES に載せる。

## 12. 第 2 段(言語の移植)への備え

- 解法の関数は numpy の配列を受けて配列を返す形にそろえ、隠れた状態を持たない(移植先で 1 関数ずつ写せる)。
- `sv_solve(..., trace=path)` で、照合した quad・当てはめた WCS・検証の対数オッズを JSON に書き出せるようにする。
  移植先の試験は、同じ入力でこの値と一致するかを見る。
- 合成星野の生成(`solver_synth.py`)は乱数の種を固定し、作った星のリストも JSON で残す(移植先で同じ入力を使う)。

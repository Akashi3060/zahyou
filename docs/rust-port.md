# 設計書: zahyou の Rust 版(第 2 段)

2026-10-15 作成、2026-10-16 に第 3 波(解法の組み込み)の結果を 7 節に書き足した。ブランチ `native-rs`(作業フォルダー `C:\Users\yoshi\Documents\GitHub\zahyou-rs`)。
ビルドと試験の手順は [../rs/README.md](../rs/README.md)。
第 1 段(Python 版の自前エンジン、ブランチ `native-solver`)の設計は [design-native-solver.md](design-native-solver.md)。

## 0. 要件と言語の選定

ユーザーの要件: **メモリ安全性が高い**・**Python より速い(特にオフライン解析)**。比較の結果
**Rust + egui/eframe** にした(4 観点の採点で総合 1 位。速さとメモリ安全・配布で 1 位、GUI では次点の WPF に劣るが
試作で確かめる。経緯はこの節の末尾)。

- メモリ安全: 自分で書く `unsafe` は索引ファイルの mmap の 1 か所だけ(`zahyou-index`)。ほかの crate は `#![forbid(unsafe_code)]`。
  並列は rayon で、データ競合は型検査(Send/Sync)でコンパイル時に拒否される。
- 速さの目標(Python 版・WSL 版と同じ機械で比べる):

| 項目 | いま | 目標 |
|---|---|---|
| 24 MP の前処理 | 8.49 秒 | 1 秒以下 |
| 24 MP の検出 | 0.83 秒 | 0.2 秒以下 |
| 24 MP の北が上(再投影) | 21.8 秒 | 1 秒以下 |
| 図 2 枚の描画と PNG | 2.71 + 8.54 秒 | 2 秒以下 |
| 968×548 の実写をヒントなしで解く | WSL 版 3〜4 秒 | 0.5 秒以下 |
| 難しい画像(quad を数十万個試すもの) | astrometry.net の C で CPU 8.9〜14.5 秒 | C 以下、Python 基準実装の 10 倍以上 |
| 解けない画像 | 締め切りを最大 45 秒超える | 超過 1 秒以内 |
| 起動 | 約 4 秒 | 1 秒以内 |
| exe | 102 MB | 25 MB 以下 |

- 試作 P1(2026-10-15)の結果: eframe 0.33 + wgpu(DX12)/glow、rfd、ureq(rustls)、memmap2、rayon が GNU ターゲットで
  ビルドでき、exe 9.7 MB、依存はシステムの DLL だけ(UCRT は Windows 10 以降に標準)。日本語は `C:\Windows\Fonts` の
  游ゴシック UI(`YuGothR.ttc` の 1 番 Semilight / `YuGothM.ttc`)で表示できた(フォントは同梱しない)。
  **日本語 IME(P2)は 2026-10-15 にユーザーが試して合格**(変換中の文字が欄の中に出て確定でき、変換中の Enter で誤動作しない)。
  GPU の無い環境・リモートデスクトップ(P4)は未確認。

## 1. ツールチェーン

- MSYS2 UCRT64 の pacman の `mingw-w64-ucrt-x86_64-rust` 1.94.0(ターゲット `x86_64-pc-windows-gnu`、リンカーは UCRT64 の gcc)。
  rustup は入れない(rustup の gnu は MSVCRT 前提で、UCRT64 の gcc と混ぜると壊れる)。
- シェルでは `export PATH=/c/msys64/ucrt64/bin:$PATH` してから `cargo`。
- eframe は **0.33 系**(0.36 は Rust 1.95 以上が要る)。Rust を上げたら 0.36 へ上げる(egui に触るのは `app` だけ)。
- 将来 MSVC(Visual Studio Build Tools)に移るときは、ターゲットとリンクの設定だけを変える(コードは共通)。

## 2. crate の構成と担当

```
rs/
  crates/zahyou-wcs     単位ベクトル・TAN 投影(geom.rs)、WCS(TAN/SIP)、当てはめ、FITS ヘッダーのカード、目標までのずれ
  crates/zahyou-index   索引ファイル: ヘッダーの検査、mmap、保存済みの kd-tree の探索、タイルの選び方(unsafe は mmap の 1 か所)
  crates/zahyou-image   画像の読み込み(FITS・PNG/JPEG/TIFF)、前処理(背景・σ クリップ・ホットピクセル・縁・焼き込み文字)、星の検出
  crates/zahyou-solver  解法(Python の dev/solver_core.py と、zahyou_core.py の試行のはしご solve_offline を移したもの)
  crates/zahyou-plot    図: 伸張、座標格子・印・矢印の表示リスト、北が上の再投影、PNG の書き出し(tiny-skia)
  crates/zahyou-net     更新の確認と入れ替え、星図データのダウンロード、天体名の解決(Sesame + VizieR)、nova.astrometry.net
  app                   eframe の画面(解析・準備・使い方の 3 タブ)と、画面を出さない入口(--selftest など)。exe 名は zahyou.exe
  testdata              Python 基準実装から作った照合用の値(JSON、小さいものだけ)と、それを作るスクリプト
  app/testdata          解析の流れ全体を本物の解法つきで Python の run() と比べる値(gen_run_solver.py)
  tools                 同梱物の表示(THIRD-PARTY-NOTICES.md)とライセンスの検査
```

依存の向き: `wcs` ← `index` ← `solver` ← `app`、`wcs` ← `plot` ← `app`、`image` ← `solver`/`app`、`index` ← `net`(ダウンロードの検査)。

## 3. 共通の約束

- **画素は 0 始まり**(x = 列、y = 行)。FITS のヘッダーに書くときだけ CRPIX に 1 を足す(Python 版と同じ)。
- 画像は `Image2D { width, height, data: Vec<f32> }`(行優先)。**行 0 は Python 版の `bundle.data` の行 0 と同じ**
  (FITS は先頭の行 = 画像の下、PNG/JPEG/TIFF は画面の上)。表示の上下反転は `flip_y` で持つ(Python 版と同じ意味)。
- 鏡像(parity)は**配列座標での det(CD) の符号**で 1 つに決める。画面の見た目の表示は `flip_y` と組み合わせて出す。
- 例外は使わず `Result`。壊れたファイルで panic しない(試験で切り詰めたファイル・ヘッダーを壊したファイルを読ませる)。
- コメントと doc コメントは日本語(既存の Python 版の言い回しに合わせる)。
- **Python 基準実装との突き合わせ**: `rs/testdata/gen_*.py` が Python 版(`C:\Users\yoshi\Documents\GitHub\zahyou\dev` の
  solver_*.py・zahyou_core.py)で値を計算して `rs/testdata/*.json` に書く。Rust の試験はそれと比べる。
  大きな画像はリポジトリに入れず、手元にあれば使う(無ければ SKIP と出して通す)。
- 索引ファイルや手元の画像が要る試験は、無ければ SKIP(`C:\AstrometryData`、`C:\Users\yoshi\Downloads\...`)。
- 外部のクレートは許容型のライセンス(MIT/Apache-2.0/BSD/Zlib/ISC など)だけ。GPL・LGPL は入れない。C のライブラリへの FFI(cfitsio など)は入れない。

## 4. crate 間の API(係どうしの約束)

### 4.1 zahyou-wcs

```rust
pub type Vec3 = [f64; 3];                         // geom.rs(作成済み。名前と挙動を変えない)
pub fn radec_to_xyz(ra_deg, dec_deg) -> Vec3; pub fn xyz_to_radec(v) -> (f64, f64);
pub fn tan_basis(r) -> (Vec3, Vec3); pub fn tan_project(s, r) -> Option<(f64, f64)>; pub fn tan_deproject(xi, eta, r) -> Vec3;
pub fn angle_between(a, b) -> f64; chord_from_angle; angle_from_chord; dot; cross; normalize; ...

pub struct Sip { pub order: usize, pub a: Vec<f64>, pub b: Vec<f64>, pub ap_order: usize, pub ap: Vec<f64>, pub bp: Vec<f64> }
    // 係数は (order+1)×(order+1) の行列を行優先(添字 [p][q] = u^p v^q)。規約は FITS SIP(Shupe et al. 2005)
pub struct Wcs { pub crval: Vec3, pub crpix: [f64; 2] /* 0 始まり */, pub cd: [[f64; 2]; 2] /* 度/px */, pub sip: Option<Sip> }
impl Wcs {
    pub fn pix_to_xyz(&self, x: f64, y: f64) -> Vec3;
    pub fn xyz_to_pix(&self, v: Vec3) -> Option<(f64, f64)>;
    pub fn pix_to_radec(&self, x, y) -> (f64, f64); pub fn radec_to_pix(&self, ra, dec) -> Option<(f64, f64)>;
    pub fn pixel_scale_arcsec(&self) -> f64; pub fn parity(&self) -> i32; pub fn crval_radec(&self) -> (f64, f64);
    pub fn header_cards(&self, width: usize, height: usize, comments: &[String]) -> Vec<Card>;   // Python の to_header_cards と同じ中身
    pub fn from_header_cards(cards: &[Card]) -> Result<Wcs, String>;                             // 画像に入っている WCS を読む
}
pub fn fit_similarity(field: &[[f64; 2]], stars: &[Vec3]) -> Option<Wcs>;
pub fn fit_wcs_lsq(field, stars, weights: Option<&[f64]>, crpix: Option<[f64; 2]>, sip_order: usize, crval: Option<Vec3>, iterations: usize, size: Option<(f64, f64)>) -> Option<Wcs>;
pub fn spherical_offsets(center_radec: (f64, f64), target_radec: (f64, f64)) -> (f64, f64, f64);  // (角距離, 赤経方向 東が正, 赤緯方向 北が正) ラジアン
```

### 4.2 zahyou-index

```rust
pub struct IndexFile { /* path, name, indexid, healpix: i32, hpnside, dimquads, nquads, nstars,
                          scale_lo_rad, scale_hi_rad, jitter_arcsec, cutnside, tile_center: Option<Vec3>, tile_radius_rad: Option<f64> */ }
impl IndexFile {
    pub fn open(path) -> Result<IndexFile, IndexError>;            // ヘッダーだけ読む。表は使うときに mmap
    pub fn code_ball(&self, queries: &[f64] /* Q×D */, radius: f64) -> Vec<(u32, u32)>;   // (問い合わせ番号, quad 番号)
    pub fn quad_stars(&self, quad: u32) -> [u32; 4];              // DIMQUADS=3 なら [3] は u32::MAX
    pub fn star_xyz(&self, id: u32) -> Vec3;                      // 長さ 1 に正規化
    pub fn sweep(&self, id: u32) -> u8;
    pub fn stars_in_cone(&self, center: Vec3, radius_rad: f64) -> Vec<u32>;
}
pub struct Catalog { /* files, problems: Vec<(PathBuf, String)>, series: BTreeMap<u32, Vec<usize>> */ }
impl Catalog {
    pub fn scan(dir) -> Catalog; pub fn coverage_arcmin(&self) -> Option<(f64, f64)>;
    pub fn select(&self, scale_lo, scale_hi, width, height, hint: Option<(Vec3, f64)>) -> Vec<usize>;
    pub fn tiles_near(&self, indexid: u32, center: Vec3, radius_rad: f64) -> Vec<usize>;
    pub fn close(&self);                                           // 開いている表の mmap を放す(次に使うとき開き直す)
}
// IndexFile にはほかに data() -> Result<Arc<IndexData>, IndexError>(開いた表。壊れていれば Err)・is_open()・
// try_code_ball()・close()。IndexData の quad_stars・star_xyz・sweep は範囲外を None で返す(壊れた索引で panic しない)
// IndexError は Io(開けない・読めない。ほかのプロセスが書き込み用に開いている共有違反もここ)・Format・Unsupported・Argument。
// 中身が壊れているのは Format だけ(Io は一時的なことがある)
pub fn validate_index_file(path) -> Result<(), String>;
pub fn healpix_xy_center(hp: u32, nside: u32) -> Vec3; pub fn healpix_xy_from_xyz(v: Vec3, nside: u32) -> u32;
```
Python 版 `dev/solver_index.py`(試験 `dev/test_solver_index.py`)と同じ結果を返すこと。

### 4.3 zahyou-image

```rust
pub struct Image2D { pub width: usize, pub height: usize, pub data: Vec<f32> }
pub struct Loaded { pub data: Image2D, pub display: Image2D, pub header: Vec<Card>, pub note: String, pub flip_y: bool }
pub fn load_image_any(path) -> Result<Loaded, LoadError>;        // Python の load_image_any と同じ値
pub struct Prep { pub sub: Image2D, pub mask: Vec<bool>, pub sigma: f64, pub info: Vec<String> }
pub fn preprocess(data: &Image2D, mask_overlay: bool) -> Prep;
pub struct Source { pub x: f64, pub y: f64, pub flux: f64, pub peak: f64, pub area: usize, pub elong: f64, pub fwhm: f64, pub snr: f64 }
pub struct DetectOpts { pub min_sources: usize /* 12 */, pub max_sources: usize /* 400 */, pub max_elongation: f64, pub min_area: usize, pub min_snr: f64 }
pub fn detect_sources(prep: &Prep, opts: &DetectOpts) -> (Vec<Source>, Option<f64>);
pub fn looks_inverted(data: &Image2D) -> bool;

// 深い検出(解法に渡す星。Python の detect_sources_deep(sub, mask, sigma, info=dict) の 2026-10-16 版とビット一致)
pub struct DeepOpts { pub max_sources: usize /* 400 */, pub target_false: f64 /* 2.0 */, pub min_u: f64 /* 4.0 */ }
pub struct DeepInfo { pub s_psf: Option<f64>, pub n_psf, pub s, pub u, pub noise, pub candidates, pub kept, pub rejected: DeepRejected, pub sharp_max, pub seconds }
pub fn detect_deep(prep: &Prep, opts: &DeepOpts) -> (Vec<Source>, Option<f64> /* u */, Option<DeepInfo>);   // 星の幅に合わせてぼかして極大を拾う
pub fn detect_sources_deep(prep: &Prep, opts: &DetectOpts) -> (Vec<Source>, Option<f64>);   // 互換(使うのは max_sources だけ)
```
深い検出の Source は Python と同じ意味: x, y は 3×3 の対数放物線の頂、flux はガウスの全光量の目安、fwhm = 2.3548·σ_psf、
snr は頂の値 / 局所の雑音、area は FWHM の円の面積、elong は 1。以前の「2×2 にまとめて検出」は消した。
`Card` は FITS のカード 1 枚(キー・値・コメント)。zahyou-wcs に置く(`pub enum CardValue { Int, Float, Str, Bool }`)。

### 4.4 zahyou-net(画面に依存しない。ブロッキングの関数を、呼ぶ側がワーカーで動かす)

```rust
pub fn check_update(current_exe: &Path, version: &str, stop: &dyn Fn() -> bool) -> UpdateCheck;  // GitHub Releases latest の digest と比べる(-dev で終わる版は問い合わせない)
pub fn download_update(...) / apply_update(...)    // 改名方式(実行中の exe を .old へ → .new を本名へ → 起動し直す)。PowerShell を使わない
pub fn index_names(scale: u32) -> Vec<String>; pub fn index_url(name) -> String; pub fn remote_sizes(...);
pub fn fetch_index(names, dest_dir, progress: &dyn Fn(u64, u64), stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> FetchResult;  // Range で再開、.part → 改名、ヘッダー検査
pub fn resolve_target_name(name: &str) -> Result<(f64, f64, String), String>;   // Sesame → VizieR の Gaia DR3 で 5″ 以内の最も近い星
pub fn internet_available() -> bool;
pub fn solve_online(req: &OnlineRequest, stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> Result<Option<NovaSolution>, NetError>;
// nova::NovaClient の login・submit_source_list・submit_image・monitor・wcs_file はどれも stop を受け取る。
// 要求は別のスレッドで投げて 50 ms ごとに stop を見ながら待ち、中止なら待たずに Cancelled(送っている画像の本文も止める)。
// 中止のあとで新しい要求(特に画像の送信)は始めない
```
v6 と同じ `%LOCALAPPDATA%\zahyou\cache.json`(targets / rigs)と `gui.json` を読み書きする(キーと形を変えない)。設定の読み書きは `app` が持つ。

### 4.5 zahyou-solver(第 3 波で中身を入れた)

```rust
pub struct FieldStar { pub x: f64, pub y: f64, pub flux: f64 }
pub struct SolveRequest { pub stars: Vec<FieldStar>, pub width, pub height, pub scale_lo: Option<f64>, pub scale_hi: Option<f64>, pub hint: Option<(f64, f64, f64)> }
pub enum Parity { Both /* 既定 */, Normal, Mirror }                   // Python の SvParams.parity("both" / "normal" / "mirror")
pub struct Params { codetol, distractor, verify_pix, logodds_solve, logodds_tune, logodds_bail, max_quad_stars, max_verify_stars, quad_min_frac, uniformize_boxes, tweak_order, parity }  // Python の SvParams と同じ既定値
pub struct Control<'a> { pub deadline: Option<Instant>, pub cancel: &'a AtomicBool, pub log: &'a (dyn Fn(&str) + Sync) }
pub struct Solution { pub wcs: Wcs, pub logodds, pub nmatch, pub ndistract, pub nfield, pub nref, pub index_name: String, pub quads_tried: u64, pub hypotheses: u64, pub seconds: f64 }
pub enum SolveError { Cancelled }
pub fn solve(req: &SolveRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<Option<Solution>, SolveError>;           // sv_solve
pub fn solve_detailed(req, catalog, params, ctl, opts: &SolveOptions /* trace: Option<PathBuf> */) -> Result<Option<SolveDetail>, SolveError>;
    // SolveDetail { solution, quad_field: Vec<usize>, quad_index: Vec<u32>, matches: Vec<StarMatch { field, star: Vec3, dist_px }> }
    // trace は Python の sv_solve の trace と同じ形の JSON(有限でない値は null、キーは名前順)
pub fn verify_wcs(stars, width, height, catalog, indexid: u32, wcs: &Wcs, quad: Option<(&[usize], &str, &[u32])>, params) -> Result<VerifyReport, String>;  // sv_verify_wcs
pub mod parts;   // Python の部品と同じ値の関数(order_stars・field_order・quad_geometry・quads_newest・field_codes・… 試験用)
pub fn index_coverage(names) -> Option<(f64, f64)>; pub fn blind_scale_bounds(width_px, names) -> (f64, f64);   // zahyou_core と同じ

pub struct OfflineRequest<'a> { pub prep: &'a Prep, pub sources: &'a [Source], pub width, pub height, pub scale: Option<f64>, pub hint: Option<(f64, f64, f64)>, pub timeout_s: f64 }
pub fn solve_offline(req: &OfflineRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<Option<Solution>, SolveError>; // 試行のはしご
pub fn solve_offline_detailed(req, catalog, params, ctl) -> Result<OfflineOutcome, SolveError>;   // app はこちら
pub struct OfflineOutcome { pub solution: Option<Solution>, pub n_deep: usize, pub stop: OfflineStop }
pub enum OfflineStop { Tried /* 解けた・全部試した・時間切れ */, TooFewStars, NoIndex, Failed /* panic */ }
```
- 入力がおかしい(ヒントが数でない・赤緯が ±90° の外・スケールや大きさが変)ときは、理由をログに書いて `Ok(None)`
  (Python は ValueError。`SolveError` に変種を足すと呼ぶ側の網羅の match が壊れるので足さない)。
- `solve_offline(_detailed)` は深い検出(`detect_deep`)もする。ログは Python の `_solve_offline` の深い検出・`[i/N] ラベル (最大 t 秒)`・
  探索(`    · `)・解の要約・`✅ 解析成功 (ラベル)`・時間切れ・星が 1〜2 個の「❌ 星が N 個しかない」・途中の失敗。
  事前のログ(画素スケール・手持ちの段)と、解けなかったときの案内・「■ 中止しました。」は app が出す(`OfflineStop` と
  `n_deep` で Python と同じ分岐を選ぶ。案内の「写っている星が N 個」は max(標準, 深い))。
- 終わったら索引の mmap を別のスレッドで放す(`Catalog::close`。Python の finally の catalog.close())。目録とヘッダーの検査の
  結果は残り、次の解析で開き直す。
- 中身の壊れた索引は、その 1 回の solve / solve_offline の間だけ覚えて外す(Python は目録に残すので、Rust は次の解析でもう一度
  見つけてログに出す)。
- 開けない索引(OS のエラー。ほかのプロセスが書き込み用に開いているなど)は「壊れている」とは言わず、その探索(試行)の中でだけ外し、
  次の試行で開き直す(ログは「星図データを今は開けないので、この探索では使いません: …」)。Python は OSError でその試行全体を
  「⚠️ 解析エンジンでエラー」にするが、Rust はほかの索引で続ける。

## 5. 進め方

1. 並列(Python のソルバー本体を待たない): zahyou-wcs、zahyou-index、zahyou-image、zahyou-net。(済み、第 1 波)
2. 並列: zahyou-plot、app の殻(タブ・設定・配色・ワーカー・中止・ログ)。(済み、第 2 波)
3. Python 版の solver_core.py が試験に通ったら zahyou-solver(trace の JSON で 1 段ずつ突き合わせる)、続いて app に組み込む。(済み、第 3 波)
4. 速さの物差し `zahyou.exe --bench`(JSON)で、Python 版と同じ入力・同じ機械で比べる。(7 節)

## 6. 実装の状況(2026-10-16、第 1・2 波)

- 試作 P1 の小さなアプリ(`rs/proto`)は、記録をこの設計書の 0 節に残して消した(unsafe を 2 か所含み、exe にも入らないため)。
- crate 間の API は 4 節から次の点が変わった(第 3 波で 4 節に書き戻した):
  - `zahyou_net::check_update(current_exe, version, stop)` は版名を受け取る(`-dev` で終わる版は問い合わせない)。
  - 深い検出は第 3 波で `detect_deep`(DeepOpts・DeepInfo)に作り直した(4.3 節)。`DetectOpts::deep()` は互換のために残っている。
  - そのほか `IndexFile::data()`、`FetchOptions`、`zahyou_plot::RenderOptions.view`(見えている範囲のまま保存)、`View::new` が足された。
- 試験: `cargo test --workspace` で 178 本が通過(app 45 / image 30 / index 16 / net 33 / plot 29 / wcs 25)。`cargo clippy -D warnings` で指摘 0。
  自分で書いた unsafe は `zahyou-index` の mmap の 1 か所だけ。依存 213 個に GPL・LGPL・AGPL・MPL は無い。
- Python との一致: 画像の読み込み・前処理・検出は試験画像 37 枚でビット単位で一致。WCS は 1e-10 px、索引の探索は全項目一致。
- exe: 12.9 MB、依存はシステムの DLL だけ。最初の描画まで 0.36 秒(DX12)/ 0.19 秒(glow)。
- 速さ(Core Ultra 7 265K、Python → Rust): 24 MP の解析全体(解法を除く)26.8 → 1.03 秒、前処理 8.73 → 0.55 秒、検出 0.81 → 0.04 秒、
  北が上の再投影 11.0 → 0.06 秒、PNG 2 枚 5.6 → 0.04 秒。968×548 の解析全体 1.47 → 0.08 秒。
- 第 2 波の時点の未了(解法と、exe 用の同梱物の表示)は第 3 波で済んだ(7 節)。

## 7. 第 3 波(2026-10-16): 解法を入れて全体をつないだ結果

### 7.1 したこと

- zahyou-solver: Python の `sv_solve`・`sv_verify_wcs`・試行のはしご(`solve_offline`)を移した(solver 係)。試験は solver-tests 係
  (合成星野 31 件・実写 17 件・解けてはいけないもの・中止・壊れた入力と壊れた索引・部品。`crates/zahyou-solver/tests/README.md`)。
- zahyou-image: 深い検出を 2026-10-16 版の Python に作り直した(deep 係。37 枚でビット一致)。
- 同梱物の表示: `rs/THIRD-PARTY-NOTICES.md` を作って exe に埋め込み、［使い方］タブから読めるようにした。`rs/tools/` に作り直しと
  ライセンスの検査(notices 係)。
- app(integrate 係):
  - 解析の流れ(pipeline.rs)は本物の `solve_offline_detailed` を呼ぶ。解けなかったときの案内を Python の分岐に合わせた
    (星が 1〜2 個なら解法の「❌ 星が N 個しかない」だけ、0 個なら「❌ 星が 1 つも無い」、途中の失敗なら案内なし。
    「写っている星が N 個と少なめ」の N は max(標準, 深い))。以前は「すべての条件で解けませんでした」も重ねて出し、N は標準の数だった。
    深い検出を app でもう一度走らせて数えるのもやめた。
  - そのために解法へ `solve_offline_detailed`・`OfflineOutcome`・`OfflineStop` を足し、深い検出のログ(「星の幅 (FWHM … px)」)を
    `DeepInfo` の s_psf・s から Python と同じ式で書くようにした(星の fwhm から逆算していた。つなぎ目の直しとして solver の crate を触った)。
  - `--bench` に「解析全体 (run・目標なし)」の段を足した(画像に WCS が無いときだけ)。解法の段は空の一時 cache.json で測る
    (利用者の機材の記憶で条件が変わらないように)。
  - 画面のログで「⚠️」の異体字セレクター U+FE0F が □ になっていた(HackGen の無い PC の等幅の欄)。ログに足すときに落とす。
  - 本物の解法つきの run() を Python の run() と比べる試験 `run_solver_vs_python` と、その基準値 `app/testdata/run_solver.json`
    (`gen_run_solver.py`、20 場面。既定の試験は締め切りまで探す 2 場面を除く 18 場面、`run_solver_vs_python_all`(#[ignore])で全部。
    星 2 個・0 個の小さな画像 few_two.fits・few_none.fits もここに置く)。
  - 画面から中止したときの遅れを測る試験 `中止の遅れを測る`(#[ignore])。
  - **同じ目録を 2 つ以上の解析が同時に使うと止まる(デッドロック)不具合を直した**。`zahyou_index::IndexFile::data` が錠を
    持ったまま索引を開き、開くときの木の検査が rayon で並列に走る。rayon の作業スレッドは待つ間に別の仕事を横取りするので、
    同じ索引を開こうとする仕事を横取りすると自分の持つ錠を待って止まる。解法の `Store::data`(OnceLock の初期化の中で開く)も
    同じ形。どちらも「錠の外で開いて、先に入ったほうを使う」にした(つなぎ目の直しとして zahyou-index の file.rs と
    zahyou-solver の verify.rs を触った)。画面の重い処理は 1 本ずつなので、ふつうの操作では起きにくいとみている(推測)が、
    試験を並べて回したときに実際に止まった。試験 `concurrent_runs_do_not_deadlock`(同じ画像を 3 本 + 別の画像 1 本、5 回ずつ同時に)は、
    直す前のコードでは 3 回とも 180 秒で打ち切り、直したあとは 3 回とも 2.2 秒で通った。

### 7.2 試験(実測)

- `cargo test --workspace`: 227 本通過・失敗 0(app 52 / solver 35 / image 37 / plot 29 / net 33 / index 16 / wcs 25)。
  #[ignore] の 12 本(解法の全部の合成星野・全部の実写・陰性を締め切り 30 秒で、外への疎通 3 本、画面からの中止、
  run() の突き合わせの遅い 2 場面込みの版)も手で回して全部通過(2 回回した。試験を並べて CPU を取り合った状態で、陰性の締め切りの
  超過は最大 +0.10 秒、画面からの中止の遅れは最大 0.31 秒。単独で測った値は 7.3 節)。
- `cargo clippy --workspace --all-targets -- -D warnings` の指摘 0。`tools/check_licenses.py`・`tools/gen_notices.py --check` は OK(依存 213 個)。
- **本物の解法つきの run() と Python の run()(sv_solve も本物)**: 20 場面・648 行のログが、時刻の数を除いて 1 行ずつ同じ。
  解けたか・図の枚数・画像中心(1e-3″)・画素スケール・位置角・パリティ・目標までのずれ・覚えた機材の画素スケールも同じ。
  場面: 基準の実写(Capture_00001 00_10_33)を目標あり・目標なし・座標(ヒントなし)・焦点距離・焦点距離の書き違い・機材の記憶、
  上下反転の FITS・PNG・JPEG、3I・Phaethon・IMX432・Siril・C2025A6・星図の PNG を目標なし、星 2 個・0 個の画像、乱数の星野(tcase/few)、
  ピンぼけ(締め切り 20 秒)、星図データの置き場所が無い。締め切りで打ち切る 2 場面は試行のはしごの行を除いて比べる。
- 本節の計測に使った 7 枚では、Rust と Python の深い検出の星がビット単位で同じことも確かめた。
- release の exe の `--selftest`(オフライン、--debug-wcs なし): **24/24 通過**。基準の実写で全体 0.8〜1.1 秒(解析 3 回と画面の項目。2 回測った)、
  24 MP の合成画像でも 24/24・4.1 秒。距離 1.595′・0.916′ 東・1.305′ 北・1.4610″/px は Python 基準実装と同じ値。WSL 版の解
  (1.583′・0.918′ 東・1.290′ 北・1.4616″/px)とは画像中心で 0.9″ 違う(正解の基準 2″ の内。原因は確かめていない。
  改良の 2 次の SIP の当て方の差とみている)。
- `--shot` で結果の画面(撮影時の向き・北が上・ログ、目標なしの C2025A6、最小の窓 1000×660)を撮って見た。崩れ・はみ出しは無い
  (上の □ は直した)。
- 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json の sha256 は作業の前後で同じ。

### 7.3 速さ(実測)

同じ機械(Core Ultra 7 265K・20 スレッド)で、ほかの重い処理が動いていないときに測った。Python は dev/ の基準実装、Rust は release
(lto fat)。秒は 3 回の最短(Python で 1 回 20 秒を超えるものは 1 回)。WSL 版(astrometry.net 0.98 の C)は記録
(`oracle/truth.json`・`tests-data`)の、solve-field 1 回の壁時計(うち約 2.4〜2.7 秒は 253 本の索引を /mnt/c から開く時間)。

**解法だけ(sv_solve / solve。7 枚の実写を同じ深い検出の星で。ヒントは正解の中心から 5°、スケール既知は正解 ×0.75〜1.25)**。
どれも Python と同じ解(索引と対数オッズ)。

| 画像 | 大きさ | 星 | ブラインド Python → Rust [秒] | ヒント | スケール既知 | WSL [秒] |
|---|---|---|---|---|---|---|
| 基準の実写 | 968×548 | 59 | 0.1090 → 0.0018(61 倍) | 0.0450 → 0.0018(25 倍) | 0.1150 → 0.0015(77 倍) | 3.6 |
| 3I | 968×548 | 15 | 0.0572 → 0.0009(64 倍) | 0.0190 → 0.0003(62 倍) | 0.0829 → 0.0012(71 倍) | 2.6※ |
| Phaethon | 536×368 | 35 | 0.0518 → 0.0006(93 倍) | 0.0280 → 0.0014(20 倍) | 0.0918 → 0.0012(80 倍) | 2.3 |
| IMX432 | 1608×1104 | 140 | 0.0850 → 0.0005(179 倍) | 0.0845 → 0.0017(51 倍) | 0.0861 → 0.0006(133 倍) | 2.2 |
| Siril | 1608×1104 | 400 | 0.1650 → 0.0005(319 倍) | 0.1631 → 0.0011(151 倍) | 0.1724 → 0.0007(259 倍) | 2.3 |
| C2025A6 | 2704×1536 | 80 | 2.29 → 0.0785(29 倍) | 0.1685 → 0.0154(11 倍) | 0.5420 → 0.0129(42 倍) | 3.2 |
| 星図の PNG | 2015×1001 | 400 | 3.78 → 0.1042(36 倍) | 0.4315 → 0.0142(30 倍) | 0.8095 → 0.0144(56 倍) | 4.0 |

※ 3I は WSL 版では星のリストのブラインドで解けず、画像を直接・スケール既知で解けたときの値。
深い検出そのもの: 968×548 で 0.165 → 0.022 秒、2704×1536 で 0.725 → 0.040 秒(Python は 1 回の値)。

**合成の難しい場合(ブラインド)**: tile_corner 34.26 → 1.05 秒(33 倍)、ra0(赤経 0° をまたぐ)154.4 → 5.20 秒(30 倍)、
polein(視野に天の極)30.67 → 1.03 秒(30 倍)。同じ解。参考: astrometry.net の C は難しい実写で CPU 8.9〜14.5 秒(0 節。同じ画像ではない)。

**解析全体 run()(画像の読み込みから図まで。Python の秒は gen_run_solver.py が同じ機械で測った値)**:

| 場面 | Python [秒] | Rust [秒] |
|---|---|---|
| 基準の実写・目標あり(ヒント) | 2.13 | 0.12 |
| 基準の実写・目標なし | 0.69 | 0.09 |
| C2025A6(2704×1536)・目標なし | 6.09 | 0.36 |
| 星図の PNG(2015×1001)・目標なし | 5.75 | 0.31 |
| IMX432(1608×1104)・目標なし | 1.54 | 0.11 |
| 焦点距離の書き違い(2500 mm、実際は 818 mm)・目標なし | 132.4 | 26.3 |
| ピンぼけ(解けない)・締め切り 20 秒 | 20.54 | 20.04 |

焦点距離の書き違いでは、外れたスケールの 2 つの試行を Python は締め切り(75・56 秒)で打ち切った(quad 6.7 万・5.6 万個)。
Rust は同じ試行で quad を全部試し切り(31 万・33 万個)、13 秒ずつで次の試行(スケール全域)へ進んで解いた。
Python の図は matplotlib の図を作るまで(画面に描く・PNG にするのは含まない)。

**24 MP(6000×4000。索引の本物の星 2,892 個を描いた、解ける合成画像)**:

| | Python の run() | Rust |
|---|---|---|
| 目標あり(ヒント。図 2 枚・北が上の再投影まで) | 37.1 秒 | 1.12 秒 |
| 目標なし(ブラインド。図 1 枚) | 20.5 秒 | 1.07 秒 |

Rust の段ごと(`--bench`): 読み込み 0.10・前処理 0.53・検出 0.034・解法(深い検出とブラインドの試行のはしご)0.35・
北が上の再投影 0.088・図 2 枚 0.32・PNG 2 枚 0.034 秒。

**解けてはいけない画像の締め切り(利用者の経路 solve_offline、締め切り 20 秒、ピンぼけ・光条・乱数の星野 2 枚)**:
超過は Python +0.23〜+0.30 秒、Rust +0.002〜+0.006 秒。WSL 版(締め切り 300 秒)は +5.7〜+69.7 秒の超過が記録されている。

**中止の効き**: 解析エンジン(solve_offline の 0.5〜6 秒後に中止、2 枚 × 4 回)は Python +0.021〜+0.166 秒、Rust +0.002〜+0.017 秒。
画面から「中止」を押す(Rust の release、0.5〜8 秒後に 6 回)と 0.031 秒(画面を回す 30 ms の 1 回分)で止まり、状態バーは「中止しました。」。
Python の画面からは測っていない。

**メモリ(私有 / working set の最大。50 ms ごとに見た値)**:

| | Python | Rust |
|---|---|---|
| 24 MP を 1 回・目標なし | 2,545 / 1,965 MB(run() だけ、画面なし) | 1,173 / 2,143 MB(画面 DX12 + 1 回) |
| 24 MP を 1 回・目標あり | 3,008 / 1,779 MB | 1,377 / 1,058 MB |
| 968×548 を 1 回・目標なし | 1,373 / 170 MB | 657 / 339 MB(DX12)、206 / 223 MB(OpenGL) |
| 画面を出して 3 秒後 | 1,356 / 138 MB(v7-dev の exe) | 563 / 243 MB(DX12)、135 / 131 MB(OpenGL) |

- Rust の私有メモリのうち約 450 MB は DX12 のドライバーの確保(OpenGL なら画面を出すだけで 92〜135 MB)。Python の私有メモリの大半は
  numpy・OpenBLAS の確保で、working set は小さい。
- Rust の 24 MP ブラインドの working set 2.1 GB の多くは mmap した索引のページ(ファイルが裏打ちする共有のページで、私有ではない)。

**起動(exe を起こしてから窓が出るまで、3 回)**: Rust 0.57〜0.82 秒(DX12。最初の描画は 0.37 秒)、0.55〜0.57 秒(OpenGL、0.19 秒)。
Python v7-dev の onefile exe 2.73〜2.92 秒(v6 の exe は 0 節の約 4 秒)。

**exe の大きさ**: Rust 13,838,336 バイト(13.2 MiB。依存はシステムの DLL だけ)、Python v7-dev の onefile 107,335,347 バイト(102.4 MiB)。

### 7.4 0 節の目標との照らし合わせ

| 項目 | 目標 | 結果 |
|---|---|---|
| 24 MP の前処理 | 1 秒以下 | 0.53 秒 |
| 24 MP の検出 | 0.2 秒以下 | 0.034 秒(深い検出は deep 係の 6000×4000 で 0.16 秒) |
| 24 MP の北が上(再投影) | 1 秒以下 | 0.088 秒 |
| 図 2 枚の描画と PNG | 2 秒以下 | 0.32 + 0.034 秒 |
| 968×548 の実写をヒントなしで解く | 0.5 秒以下 | 解法 0.002 秒・run() 全体 0.09 秒 |
| 難しい画像 | C 以下、Python の 10 倍以上 | 1.0〜5.2 秒(C の CPU 8.9〜14.5 秒以下)、Python の 30〜33 倍 |
| 解けない画像 | 締め切りの超過 1 秒以内 | +0.006 秒以内 |
| 起動 | 1 秒以内 | 窓まで 0.55〜0.82 秒 |
| exe | 25 MB 以下 | 13.8 MB |

### 7.5 残っていること

- 画面の既定は DX12(第 2 波で安定を優先して決めた)。私有メモリが約 450 MB 多い。OpenGL を既定にするかは人が決める。
- 中身の壊れた索引の記録は 1 回の解析の間だけ(Python は目録に残す)。持ち越すには zahyou-index の Catalog に書き込める記録が要る。
- 解法の 1 スレッドの速さは Python の 2.7〜3.8 倍どまり(残りの時間の大半は zahyou-index の stars_in_cone と code_ball。solver 係の実測)。
- rs/testdata の基準値の一部は古い Python から作られている: `pipeline_run.json`(深い検出が 2×2 だった頃。app の試験は解法の行を
  除いて比べるので通る)、`solver_real.json` の stars_deep(deep 係によれば 6 件が今の Python と違う)。作り直すのは
  `gen_pipeline.py`(既定で Python 版のリポジトリ `C:\Users\yoshi\Documents\GitHub\zahyou` の dev を読む)と `gen_solver.py`。
- `rs/testdata/bench_solver.py`(solver-tests 係が作った Python 側の物差し)を残すかは人が決める。
- オンライン解析(nova への送信)と、画像そのものを外へ送る同意の取り方は確かめていない・作っていない(第 2 波から)。
  中止の効きと、API キーが無いときの扱いは 8 節で直した(偽の nova で確かめた。本物の nova へは送っていない)。
- 自己診断は Python 版と同じく、解けた画素スケールを本物の cache.json に覚える(一時ファイルへ向けるのは gui.json だけ)。
- Python の画面からの中止の遅れと、v6 の exe の起動は今回測っていない。

## 8. 第 3 波の点検の直し(2026-10-16、fix 係)

点検(fidelity・app-safety)で確かめられた指摘を直した。どれも今のコードで当てはまることを先に確かめた。

### 8.1 直したこと

- **「まとめて準備する」が容量の確認なしに何 GB も落とし始める**(high): 段に印があるとき、「おすすめをまとめて落とす」を
  キャンセルしたあとに起きた。Python の `_prepare_all` と同じく、落とす段があれば印の有無によらず、押したときの置き場所で
  数え直して容量を聞く(印ありなら「印を付けた段をそろえます。」)。「おすすめをまとめて落とす」をキャンセルしたら印を押す前に
  戻す。`survey::prepare_all` に `allowed: Option<&[u32]>` を足し、了承していない段が足りなくなっていたら落とさずに止める
  (Python の `allowed` と同じ文)。
- **置き場所を変えたあと、前の置き場所の結果(env_state)で判断する**(high): 「おすすめをまとめて落とす」「まとめて準備する」は
  押したときの `index_dir()` で `survey` をし直して数える(Python の `_have_scales`。253 本で 0.02 秒)。「必要な段を選ぶ」は
  ワーカーの中で数え直す。置き場所を変えたら(「...」でも手入力でも)表を「未確認」に戻し、調べている間に置き場所が変わっていたら、
  届いた古い結果は使わない。
- **オンライン解析の中止**(medium): 状態の問い合わせの最中に中止して nova が「解けない」と答えても、画像を送らない
  (monitor は各 GET のあとにも中止を見る。`solve_online_with` は Failed のあとと、login・画像の送信の前に見る)。
  要求はどれも別のスレッドで投げ、50 ms ごとに中止を見ながら待つ(中止なら応答を待たずに戻る。Python の `_with_deadline` と同じく、
  投げた要求は裏で時間切れまで続くことがある)。画像の本文は、中止で読み出しを止める読み手で送る(Content-Length は付けるので
  送り方は前と同じ。Python の記録と比べる試験もそのまま通る)。`NovaClient` の login・submit_*・wcs_file は stop を受け取る。
- **API キーが無いとき**(medium、一部): 「自動」でキーが無ければ「⚠️ オンライン解析でエラー」と書かずに問い合わせを飛ばし
  (「API キーが無いので、問い合わせません」)、オフラインで解く。「オンラインのみ」はキーが無ければ始める前に知らせて始めない。
  使い方タブにキーの入れ方(環境変数 ASTROMETRY_API_KEY)を書いた。キーを Python と同じ既定値に戻すか、画面に欄を設けるかは
  人が決める(8.4)。
- **探索半径の検査**(low): 負・0・数でない値は、Python の `_start_analysis` と同じ文で知らせて始めない。条件を作るときも
  正の数だけを使う(Python の `_num(positive=True)`)。数の欄は Python の float() と同じく全角の数字(０〜９)も読む。
- **ダイアログの持ち主**(low): 知らせ・確認・ファイルの選択のダイアログを、本体の窓に従うモーダルにした(rfd の `set_parent`)。
  eframe の Frame は update の間しか借りられないので、画面の部品は処理を `Action` として頼み、描き終えたあとに Frame を持ち主に
  して行う(unsafe は使わない)。自己診断(窓なし)は今のまま。
- **画面を開けないときに何も出さない**(low): DX12 で開けずに OpenGL で開き直すとき起動し直せなかったとき、OpenGL でも
  開けなかったとき、最初の描画の前に画面のスレッドが panic したときに、理由と回避の方法を箱で出す(exe は windows サブシステム
  なので標準エラーは見えない)。描けたあとの panic も箱で知らせる(開き直さない)。撮影(--shot)では出さない。
- **更新で、設定を保存する前に新しい exe を起動していた**(low): 確認のあと、ワーカーに渡す前に保存する(update.rs の約束)。
  QuitForUpdate での保存は、そのあとの編集のために残した。
- **ほかのプロセスが書き込み用に開いている索引を「壊れている」として外す**(low、fidelity): 索引を開けない OS のエラー
  (`IndexError::Io`。共有違反はそれと分かる文にした)を、中身の問題(Format など)と分けた。解法は `Abort::Unavailable` で
  その探索(試行)の中でだけ外し、「星図データを今は開けないので、この探索では使いません: …」と書く。OnceLock には入れないので、
  次の試行で開き直す。`FILE_SHARE_READ` で開くのはそのまま(mmap を安全に使うため)。
- **深い検出の同点の順**(low、fidelity): 基準実装(dev/zahyou_core.py)の `np.argsort` が安定でなく、同点の順が CPU で変わる。
  直すのは基準実装の側(担当外なので触っていない。8.4)。deep.rs の注記を実測に合わせて書き直した。

### 8.2 試験(実測)

| 試験 | 結果 |
|---|---|
| `cargo test --workspace -- --include-ignored`(重いもの・外への疎通込み) | **247 本通過・失敗 0**(15 分 53 秒)。app 59 / solver 42 / image 37 / net 39 / plot 29 / wcs 25 / index 16 |
| うち新しく足した試験 | 8 本: 書き込み中の索引(solver)、中止と「解けない」が重なっても画像を送らない・遅い応答を待たずに中止(net)、了承していない段は落とさない・探索半径の検査 2・まとめて準備は必ず聞く・置き場所を変えたら前の結果で判断しない(app) |
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0 |
| `tools/check_licenses.py`・`tools/gen_notices.py --check`・`tools/test_tools.py` | OK(依存 213 個、本文 35 種)・最新・15 本通過 |
| release の `--selftest --offline` | **24/24**(0.81〜1.10 秒、3 回)。cache.json に天体名の座標を覚えていれば 1.595′・0.916′ 東・1.305′ 北・1.4610″/px で 7.2 節と同じ(空の cache.json では座標の出どころが違い 1.594′・1.304′ 北) |
| `--shot`(使い方・準備・結果の撮影時の向き・暗い配色のログ、最小の窓) | 崩れ・はみ出しなし |
| ダイアログの持ち主(UI オートメーション。知らせと「参照...」) | 持ち主 = 本体、本体は操作できない、本体を手前に出してもダイアログが上(直す前は持ち主なし・操作できる・後ろに隠れた) |
| 画面を開けないとき(WGPU_BACKEND=metal で DX12 を失敗させる。OpenGL の失敗は作業フォルダーの写しだけに差し込んだ) | DX12 だけ: OpenGL で開き直して窓が出る。両方: 「zahyou の画面を開けませんでした (OpenGL: …)。グラフィックスのドライバーを…」の箱、終了コード 1(直す前は何も出ず 0) |
| 書き込み中の索引(index-4113〜4117 の写し、4116 を別のプロセスが r+b で開いたまま) | 「今は開けないので、この探索では使いません」と出して index-4117 で解く(「壊れている」とは言わない)。4116 しか無ければ解けない(共有モードによる設計上の違い。8.4) |
| 基準実装の argsort を実行時に kind="stable" に差し替えた深い検出(rs/testdata の 47 枚) | 47 枚とも差し替え前とビットまで同じ(基準実装を直しても照合値は変わらない見込み) |
| 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json | sha256 は作業の前後で同じ |

### 8.3 速さ(実測。7.3 節の Rust の値を、直したあとのコードで測り直したもの)

同じ機械・ほかの重い処理なし。Python の値は 7.3 節と同じ(基準実装は変えていない)。

**解法(7 枚の実写 × 3 条件、3 回ずつ 3 組の最短)**: 21 条件とも Python と同じ解(索引と対数オッズ)。

| 画像 | ブラインド Python → Rust [秒] | ヒント | スケール既知 |
|---|---|---|---|
| 基準の実写 | 0.1090 → 0.0016(70 倍) | 0.0450 → 0.0019(24 倍) | 0.1150 → 0.0013(86 倍) |
| 3I | 0.0572 → 0.0009(64 倍) | 0.0190 → 0.0003(57 倍) | 0.0829 → 0.0012(70 倍) |
| Phaethon | 0.0518 → 0.0006(88 倍) | 0.0280 → 0.0011(25 倍) | 0.0918 → 0.0011(81 倍) |
| IMX432 | 0.0850 → 0.0005(169 倍) | 0.0845 → 0.0014(59 倍) | 0.0861 → 0.0006(143 倍) |
| Siril | 0.1650 → 0.0005(333 倍) | 0.1631 → 0.0011(142 倍) | 0.1724 → 0.0006(280 倍) |
| C2025A6 | 2.29 → 0.0815(28 倍) | 0.1685 → 0.0142(12 倍) | 0.5420 → 0.0122(44 倍) |
| 星図の PNG | 3.78 → 0.1046(36 倍) | 0.4315 → 0.0145(30 倍) | 0.8095 → 0.0150(54 倍) |

**合成の難しい場合(ブラインド、3 回の最短)**: tile_corner 1.078 秒・ra0 5.180 秒・polein 1.034 秒(7.3 節は 1.05・5.20・1.03 秒)。同じ解。

**--bench(3 回の最短。24 MP は 6000×4000 の解ける合成画像、もう 1 枚は基準の実写 968×548)**:

| 段 | 24 MP | 968×548 |
|---|---|---|
| 読み込み | 0.102 | 0.003 |
| 前処理 | 0.484 | 0.021 |
| 検出 | 0.032 | 0.002 |
| 解法(深い検出と試行のはしご) | 0.331 | 0.024 |
| 北が上の再投影 | 0.085 | 0.003 |
| 図 2 枚 | 0.315 | 0.021 |
| PNG 2 枚 | 0.033 | 0.026 |
| 解析全体 run()・目標あり | 1.096 | 0.067 |
| 解析全体 run()・目標なし | 1.064 | 0.055 |

**締め切りの超過(solve_offline、締め切り 20 秒、解けない 4 枚)**: +0.000〜+0.006 秒。**中止の遅れ(2 枚 × 4 回)**: +0.002〜+0.013 秒。
**オンライン解析の中止(偽の nova、応答を 5 秒遅らせる)**: 状態の問い合わせ中 0.025 秒・画像の送信中 0.013 秒で戻る(直す前はどちらも 5.00 秒)。
**最初の描画まで(3 回)**: DX12 0.345〜0.374 秒、OpenGL 0.180〜0.184 秒。**画面を出して 3 秒後の私有 / working set**: DX12 563〜565 / 243〜244 MB、
OpenGL 133〜134 / 131 MB。**exe**: 13,897,216 バイト(+58,880 バイト)。

### 8.4 残っていること(重大度の順)

- **API キー**(medium、人が決める): Rust 版はキーをソースに持たないので、環境変数を入れていない利用者はオンライン解析を使えない
  (星図データをまだ持っていない最初の利用者は、Python 版なら解けたが Rust 版では解く手段が無い)。Python と同じ既定のキーを使うか、
  画面にキーの欄を設けて資格情報マネージャーなどへ保存するか。画像そのものを外へ送ることへの同意の取り方も未決(7.5)。
- **深い検出の同点の順**(low、基準実装の直し): dev/zahyou_core.py の 604 行 `np.argsort(-sm1[ys, xs])` と 750 行 `np.argsort(-vals)` に
  `kind="stable"` を付ける。実行時に差し替えた試しで、平らな頂の合成 54 枚が Rust とビットまで一致し、rs/testdata の 47 枚は変わらない。
  直したら deep.rs 冒頭の注記を「Python も安定な並べ替えなので同点の順も同じ」に書き換える。
- **書き込み中の索引**(low、設計上の違い): ほかのプロセスが書き込み用に開いている間は、Rust はその索引を開けない(mmap を安全に
  使うための共有モード)。その索引しか合わない画像は、閉じられるまで Rust では解けない(Python は解く)。文言は正しくなり、次の試行で開き直す。
- 解析を実行中も「参照...」は押せる(点検の「ついでに」の案。ダイアログはモーダルになり、更新の前に設定を保存するようになったので、
  更新との取り合いは起きない。押せなくすると解析中に次の画像を選べなくなるので変えていない)。
- 画面を開けないときの箱は、OpenGL の失敗を写しに差し込んで確かめた。DX12 と OpenGL が自然に両方失敗する機械では確かめていない。
- 7.5 節のほかの項目はそのまま。

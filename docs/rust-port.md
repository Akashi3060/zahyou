# 設計書: zahyou の Rust 版(第 2 段)

2026-10-15 作成、2026-10-16 に第 3 波(解法の組み込み)の結果を 7 節に、同日のユーザーの決定(API キーの欄・描画方式・β 版)を
9・10 節に書き足した。2026-10-17 の保守(試験の正解データ・試験の一時フォルダー・自己診断の記憶・文書と版の試験)は 11 節
(11.1 節の残り = 試験と生成スクリプトの既定のパスは 2026-10-18 に決着)。ブランチ `native-rs`(作業フォルダー `%USERPROFILE%\Documents\GitHub\zahyou-rs`)。
ビルドと試験の手順は [../rs/README.md](../rs/README.md)。
第 1 段(Python 版の自前エンジン、ブランチ `native-solver`)の設計は [design-native-solver.md](design-native-solver.md)。

## 0. 要件と言語の選定

ユーザーの要件: **メモリ安全性が高い**・**Python より速い(特にオフライン解析)**。比較の結果
**Rust + egui/eframe** にした(4 観点の採点で総合 1 位。速さとメモリ安全・配布で 1 位、GUI では次点の WPF に劣るが
試作で確かめる。経緯はこの節の末尾)。

- メモリ安全: 自分で書く `unsafe` は索引ファイルの mmap の 1 か所だけ(`zahyou-index`。Windows だけで使う。12 節)。ほかの crate は `#![forbid(unsafe_code)]`。
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

依存の向き: `wcs` ← `index` ← `solver` ← `app`、`wcs` ← `image` ← `solver`/`plot`/`app`、`wcs` ← `plot` ← `app`、`index` ← `net`(ダウンロードの検査)← `app`。`plot` は `image` の `Image2D`・`Source` と、numpy とビット一致する σ クリップ(伸張の範囲)を使う(2026-10-17 に各 crate の Cargo.toml と突き合わせて `image` ← `plot` と `wcs` ← `image` を足した)。

## 3. 共通の約束

- **画素は 0 始まり**(x = 列、y = 行)。FITS のヘッダーに書くときだけ CRPIX に 1 を足す(Python 版と同じ)。
- 画像は `Image2D { width, height, data: Vec<f32> }`(行優先)。**行 0 は Python 版の `bundle.data` の行 0 と同じ**
  (FITS は先頭の行 = 画像の下、PNG/JPEG/TIFF は画面の上)。表示の上下反転は `flip_y` で持つ(Python 版と同じ意味)。
- 鏡像(parity)は**配列座標での det(CD) の符号**で 1 つに決める。画面の見た目の表示は `flip_y` と組み合わせて出す。
- 例外は使わず `Result`。壊れたファイルで panic しない(試験で切り詰めたファイル・ヘッダーを壊したファイルを読ませる)。
- コメントと doc コメントは日本語(既存の Python 版の言い回しに合わせる)。
- **Python 基準実装との突き合わせ**: `rs/testdata/gen_*.py` が Python 版(`%USERPROFILE%\Documents\GitHub\zahyou\dev` の
  solver_*.py・zahyou_core.py)で値を計算して `rs/testdata/*.json` に書く。Rust の試験はそれと比べる。
  大きな画像はリポジトリに入れず、手元にあれば使う(無ければ SKIP と出して通す)。
- 索引ファイルや手元の画像が要る試験は、無ければ SKIP(`C:\AstrometryData`、`%USERPROFILE%\Downloads\...`)。
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
pub fn check_update(current_exe: &Path, version: &str, stop: &dyn Fn() -> bool) -> UpdateCheck;  // 正式版: GitHub Releases latest の digest と比べる(latest が今より古い版なら入れ替えに誘わない)。
    // β などプレリリースの版: プレリリースも含めた一覧(releases?per_page=100)から、大きな版が同じで今より新しいものを選んで比べる。-dev で終わる版は問い合わせない(10.4 節)
// update::{Version, parse_version, is_dev_version, prerelease_label("β版"), select_prerelease_update, releases_list_url}(lib.rs では再公開していない)
// nova::{DEFAULT_API_KEY, initial_api_key, ApiKey(Debug は伏せ字・Display なし), redact_key, REDACTED}(10.2 節)
pub fn download_update(...) / apply_update(...)    // 改名方式(実行中の exe を .old へ → .new を本名へ → 起動し直す)。PowerShell を使わない
pub fn index_names(scale: u32) -> Vec<String>; pub fn index_url(name) -> String; pub fn remote_sizes(...);
pub fn fetch_index(names, dest_dir, progress: &dyn Fn(u64, u64), stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> FetchResult;  // Range で再開、.part → 改名、ヘッダー検査
pub fn resolve_target_name(name: &str) -> Result<(f64, f64, String), String>;   // Sesame → VizieR の Gaia DR3 で 5″ 以内の最も近い星
pub fn internet_available() -> bool;
pub fn solve_online(req: &OnlineRequest, stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> Result<Option<NovaSolution>, NetError>;
// nova::NovaClient の login・submit_source_list・submit_image・monitor・wcs_file はどれも stop を受け取る。
// submit_source_list・submit_image は request-json の先頭に publicly_visible: "n"(公開しない)を付ける(nova::PUBLICLY_VISIBLE・private_settings。10.10 節)
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

- ~~画面の既定は DX12(第 2 波で安定を優先して決めた)。私有メモリが約 450 MB 多い。OpenGL を既定にするかは人が決める。~~
  → 2026-10-16 に決着(DX12 のまま、GPU メモリを小さな塊で確保して OpenGL との差を 117 MB に縮めた。9 節・10 節)。
- 中身の壊れた索引の記録は 1 回の解析の間だけ(Python は目録に残す)。持ち越すには zahyou-index の Catalog に書き込める記録が要る。
- 解法の 1 スレッドの速さは Python の 2.7〜3.8 倍どまり(残りの時間の大半は zahyou-index の stars_in_cone と code_ball。solver 係の実測)。
- rs/testdata の基準値の一部は古い Python から作られている: `pipeline_run.json`(深い検出が 2×2 だった頃。app の試験は解法の行を
  除いて比べるので通る)、`solver_real.json` の stars_deep(deep 係によれば 6 件が今の Python と違う)。作り直すのは
  `gen_pipeline.py`(既定で Python 版のリポジトリ `%USERPROFILE%\Documents\GitHub\zahyou` の dev を読む)と `gen_solver.py`。
- `rs/testdata/bench_solver.py`(solver-tests 係が作った Python 側の物差し)を残すかは人が決める。
- オンライン解析(nova への送信)と、画像そのものを外へ送る同意の取り方は確かめていない・作っていない(第 2 波から)。
  → 同意の取り方は 2026-10-16 に決着: 同意を取らずに送ってよい(10.9 節)。本物の nova へ送る道は今も試していない(10.8)。
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

- ~~**API キー**(medium、人が決める): Rust 版はキーをソースに持たないので、環境変数を入れていない利用者はオンライン解析を使えない
  (星図データをまだ持っていない最初の利用者は、Python 版なら解けたが Rust 版では解く手段が無い)。Python と同じ既定のキーを使うか、
  画面にキーの欄を設けて資格情報マネージャーなどへ保存するか。~~ → 2026-10-16 に決着: 画面に欄を設け、既定は作者のキー(v6 と同じ)、
  gui.json に平文で覚える(10.2 節)。画像そのものを外へ送ることは、同意を取らずに送ってよいと決まった(10.9 節。v6 と同じく、送る前には聞かない)。
- ~~**深い検出の同点の順**(low、基準実装の直し): dev/zahyou_core.py の 604 行 `np.argsort(-sm1[ys, xs])` と 750 行 `np.argsort(-vals)` に
  `kind="stable"` を付ける。実行時に差し替えた試しで、平らな頂の合成 54 枚が Rust とビットまで一致し、rs/testdata の 47 枚は変わらない。
  直したら deep.rs 冒頭の注記を「Python も安定な並べ替えなので同点の順も同じ」に書き換える。~~ → 2026-10-16 に決着: native-solver の
  `7922b07`(2 か所に `kind="stable"`。test_robust 38/38・実写の回帰 22/22 で結果は変わらず)を `53e485e` で native-rs に取り込み、
  直下の `zahyou_engine.py` にも入っている。deep.rs 冒頭の注記も「基準実装の Python も 2026-10-16 から安定な並べ替え」に書き換え済み。
- **書き込み中の索引**(low、設計上の違い): ほかのプロセスが書き込み用に開いている間は、Rust はその索引を開けない(mmap を安全に
  使うための共有モード)。その索引しか合わない画像は、閉じられるまで Rust では解けない(Python は解く)。文言は正しくなり、次の試行で開き直す。
- 解析を実行中も「参照...」は押せる(点検の「ついでに」の案。ダイアログはモーダルになり、更新の前に設定を保存するようになったので、
  更新との取り合いは起きない。押せなくすると解析中に次の画像を選べなくなるので変えていない)。
- 画面を開けないときの箱は、OpenGL の失敗を写しに差し込んで確かめた。DX12 と OpenGL が自然に両方失敗する機械では確かめていない。
- 7.5 節のほかの項目はそのまま。

## 9. 描画方式の実測と決定(2026-10-16、render 係)

ユーザーの決定(2026-10-16): 画面の描画(DX12 か OpenGL か)は、性能が良くて安定している方を既定にする。想定する使用環境は
**Windows 11 が動く、RAM 8 GB 以上のノート PC**。7.5 節の「OpenGL を既定にするかは人が決める」はこれで決着した。

### 9.1 決めたこと(実装は app/src/main.rs の `RenderChoice`・`RENDER_ORDER`・`LEAN_MEMORY_DEFAULT`)

- **既定は DirectX 12(wgpu)**。GPU は **主の画面をつないでいる GPU**(wgpu の `PowerPreference::None` = DXGI の列挙の先頭。
  ノート PC ではふつう内蔵 GPU)。GPU メモリは **小さな塊で確保する**(wgpu の `MemoryHints::MemoryUsage`。egui の既定は
  `Performance` = 256 MiB と 64 MiB の塊)。シェーダーのコンパイラーは wgpu の既定の FXC(Windows に入っている d3dcompiler_47.dll。
  DXC は dxcompiler.dll の同梱が要るので使わない)。提示は垂直同期(AutoVsync)。
- **開けなかったときの順は DX12 → OpenGL → DX12 のソフトウェア描画(WARP)**。winit のイベントループは 1 プロセスに 1 回しか
  作れないので、開けなければ `--render-step N` を付けて exe を起動し直す。それまでに開けなかった描き方と理由は環境変数
  `ZAHYOU_RENDER_FAILED` で子へ渡し(`--render-step` が付いたときだけ読む)、最後まで開けなければ全部の理由を 1 つの箱で知らせる。
- 手で選ぶ: `--dx12` / `--glow`(指定したら、開けなくても次を試さない)、`--gpu display|low|high|warp`(DX12 の GPU。
  開けなければ次を試すのは同じ)。物差しと試験用の隠しの引数: `--mem lean|perf`・`--no-vsync`・`--no-hw`(OpenGL を
  ハードウェア支援なしで開く = Windows の OpenGL 1.1)。egui の既定と同じく環境変数 `WGPU_BACKEND` も見る(試験で `metal` にして
  DX12 を失敗させる)。
- 開いた描き方と GPU は `RENDER_INFO`(例「DirectX 12 / NVIDIA GeForce RTX 3060 (DiscreteGpu, ドライバー …)」)。`--first-frame` の
  記録に `renderer=` として書く。画面のログの 2 行目にも「描画: …」として出す(10.5 節で足した)。
- 物差し: 隠しの起動 `zahyou.exe --bench-ui <画像> [--glow|--dx12] [--gpu …] [--mem …] [--no-vsync] [--no-hw] [--cycles N] [--out f.json]`
  (app/src/ui/bench_ui.rs)。本物の窓(1180×780、表示倍率 125% で 1475×975 px)に、解析の結果と同じ形の図(画像・座標格子・
  星の丸 300 個・中心の ×・矢印・目標)を出し、拡大(全体 → 3 倍、240 フレーム)・移動(3 倍のまま画像の上を回る、240)・
  全体表示との行き来(15 フレームごと、120)を 2 周自動で動かして、段ごとにフレームの間隔と 1 フレームの処理時間(eframe の
  cpu_usage = 垂直同期の待ちを除いた時間)の中央値・99 パーセンタイル・最大を JSON に書く。1 周目は縮小の段のテクスチャを
  初めて GPU に上げるので重く、2 周目は上げ終わったあと。メモリ・CPU 時間・GPU メモリは外から測る(私有・working set・CPU 時間は
  50〜100 ms ごと、GPU のプロセスメモリは性能カウンター `GPU Process Memory` を typeperf で 1 秒ごと)。段の始まりと終わりを
  UNIX 時刻で書くので突き合わせられる。DX12 なら wgpu の割り当て器の報告(確保した塊の合計)も書く。

### 9.2 測った機械と条件

- Core Ultra 7 265K、RAM 192 GB、**GPU は NVIDIA GeForce RTX 3060**(ドライバー 32.0.16.1062 = 610.62)と **内蔵の Intel Graphics**
  (32.0.101.8331)。DXGI が見せるアダプターは RTX 3060・Intel Graphics・Microsoft Basic Render Driver(WARP)の 3 つ。
- 画面は 3 枚とも RTX 3060 につながっている(主の画面 2560×1440・約 75 Hz・表示倍率 125%)。**内蔵 GPU は画面を持たない**ので、
  内蔵 GPU で描いた絵は NVIDIA の画面へ写して出す(ノート PC で外付け GPU を使うときの写しの逆向き)。ノート PC の内蔵 GPU は
  内蔵の画面を直接持つので、ここの内蔵 GPU の値は写しのぶん不利な側。
- **OpenGL は窓を出した画面の GPU のドライバーで動く**(ここでは NVIDIA の OpenGL 3.3)。この PC では内蔵 GPU の OpenGL を測れない。
  ノート PC では、OpenGL がどちらの GPU で動くかは画面をつないでいる GPU と NVIDIA のコントロール パネル(優先するグラフィックス
  プロセッサ)などの会社の設定で決まり、アプリからは選べない。
- 画像: 合成の FITS(float32、星と雑音、TAN の WCS)。24 MP = 6000×4000、61 MP = 9504×6336(α7R IV/V と同じ画素数。
  wgpu の上限 8192 px を超えるので 4096 px のタイルに分けて貼る。OpenGL は NVIDIA の上限が大きいので 1 枚)。
- release(lto fat)の exe。利用者の設定は ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時フォルダーへ向けた。
  ほかの係のビルドが同時に動いていたことがあるので、設定を交互に並べて 3 回(垂直同期なしは 2 回)測り、中央値と幅を書く。

### 9.3 結果(実測)

**画面を出しただけ**(本物の画面、`--no-startup-checks`、起動から 3 秒の時点。3 回の中央値 [最小〜最大])

| 描き方 | GPU | 最初の描画まで | 私有メモリ | working set |
|---|---|---|---|---|
| **DX12・小さな塊(既定)** | RTX 3060 | 360 ms [349〜372] | **248 MB** [247〜248] | 175 MB [175〜202] |
| DX12・大きな塊(egui の既定。直す前) | RTX 3060 | 364 ms [362〜366] | 556 MB [556〜557] | 235 MB |
| OpenGL | RTX 3060 | **201 ms** [190〜210] | **131 MB** [129〜183] | 128 MB [127〜185] |
| DX12・小さな塊 | 内蔵 Intel | 375 ms [372〜377] | 296 MB | 274 MB |
| DX12 WARP(ソフトウェア) | CPU | 308 ms [303〜309] | 274 MB | 177 MB |

7.3・8.3 節の「DX12 は私有 563 MB」は大きな塊の値。小さな塊にすると 308 MB 減った(残りの差は OpenGL より 117 MB)。
WARP は起動から 1.5 秒の間に CPU を 11 秒使う(全部のコアでシェーダーを用意しているとみている。推測)。その後は 0。

**図を動かす**(`--bench-ui`、垂直同期あり = 75 Hz、3 回の中央値)。「2 周目」はテクスチャを上げ終わったあとの拡大・移動・行き来の
3 段の合計(720 フレーム)。間隔は 75 Hz の 1 フレーム 13.33 ms が下限。CPU は 1 コア = 100%。

| 画像 | 描き方 | 2 周目の間隔 中央値 / p99 / 最大 [ms] | 処理 中央値 / p99 [ms] | 1 周目の最大 [ms] | CPU | 表示中の私有 / WS [MB] | GPU メモリ [MB] |
|---|---|---|---|---|---|---|---|
| 24 MP | **DX12 既定** | 13.33 / 15.9 / 16.7 | 3.8 / 5.5 | 54 | 32% | 529 / 324 | 専用 203・共有 38 |
| 24 MP | DX12 大きな塊 | 13.34 / 16.1 / 18.3 | 3.9 / 5.7 | 49 | 33% | 701 / 378 | 専用 318・共有 94 |
| 24 MP | OpenGL | 13.32 / 14.6 / 15.7 | 4.0 / 6.5 | 47 | 35% | 457 / 310 | 専用 176・共有 48 |
| 24 MP | DX12 内蔵 GPU | 13.31 / 16.7 / 18.7 | 3.9 / 6.1 | 54 | 35% | 590 / 565 | 内蔵の共有 270 |
| 61 MP | **DX12 既定** | 13.34 / 17.0 / 18.1 | 3.8 / 6.1 | 129 | 33% | 906 / 549 | 専用 384・共有 42 |
| 61 MP | DX12 大きな塊 | 13.34 / 17.0 / 19.7 | 3.9 / 6.0 | 122 | 33% | 1,209 / 627 | 専用 574・共有 158 |
| 61 MP | OpenGL | 13.32 / 14.6 / 14.9 | 4.0 / 6.3 | 108 | 38% | 826 / 493 | 専用 367・共有 100 |
| 61 MP | DX12 内蔵 GPU | 13.34 / 16.4 / 17.6 | 4.0 / 6.0 | 122 | 36% | 974 / 948 | 内蔵の共有 471 |

- 2 周目に 20 ms を超えたフレームは、内蔵 GPU の 3 回のうち 1 回ずつ(24 MP で 8 フレーム、61 MP で 1 フレーム・40 ms)だけで、
  ほかは 0。1 周目に 20 ms を超えたのは 1〜4 フレームで、どれも原寸や縮小の段のテクスチャを初めて GPU に上げたフレーム
  (61 MP の原寸は DX12 では 4096 px のタイル 6 枚で 240 MB。OpenGL は NVIDIA の上限が 32768 px なので 1 枚)。
- 「表示中」は最後の 2 秒(全体表示のまま)の終わり。読み込みの瞬間の私有の最大は 61 MP で 1,289〜1,458 MB(大きな塊は 1,716 MB。
  画像と図を CPU 側で作る分が大きい)。DX12 の割り当て器の確保は 61 MP で 333 MB(大きな塊なら 640 MB)。
- 内蔵 GPU は描く絵が共有メモリ(RAM)に載るので、working set が NVIDIA より 400 MB ほど大きい(61 MP)。

**垂直同期なし**(上限なしの速さ。2 回の中央値。2 周目の 3 段)

| 画像 | DX12 既定(RTX 3060) | OpenGL(RTX 3060) | DX12 内蔵 GPU |
|---|---|---|---|
| 24 MP | 257 fps(間隔 4.13 ms・p99 6.6) | 240 fps(4.40・7.3) | 246 fps(4.25・7.2) |
| 61 MP | 255 fps(4.12・6.7) | 237 fps(4.42・7.0) | 239 fps(4.31・6.8) |

どれも CPU が 1 コアに張り付いている(98〜104%)。1 フレームの処理 3.6〜3.8 ms の多くは CPU 側(拡大のたびに座標格子を作り直す・
三角形に分ける)で、GPU は詰まっていない。**内蔵 GPU でも RTX 3060 と同じ速さ**(画面への写しを含めて)。

**WARP**(`--gpu warp`、24 MP、1 周だけ・1 回): 窓は開いて図も正しく描けたが、拡大・移動で 1 フレーム 467 ms(2.1 fps)、CPU 7.8 コア分。
何も描かない窓でも 77 ms(起動直後の 1 秒)。最後の手段としてだけ使える速さ。

### 9.4 安定性の材料

実機で確かめたこと:

- **DX12 は GPU のドライバーが無くても WARP で開ける**: wgpu の DXGI の列挙はソフトウェアの WARP(Microsoft Basic Render Driver、
  `DeviceType::Cpu`)を残す(wgpu-hal は「統合 GPU のふりをする WARP」だけを外す)。`--gpu warp` で窓が開き、図を描けた(遅い。9.3)。
  GPU が無い機械では wgpu が自分で WARP を選ぶ(列挙の最後にあるので)とみている(この PC では実物の GPU を外せないので推測)。
- **OpenGL は 1.1 しか無い環境では開けない**: `--no-hw`(glutin に「ハードウェア支援なし」を頼む)で Windows の OpenGL 1.1 を
  選ばせると `egui_glow requires opengl 2.0+` で開けない。GPU の会社のドライバーが無い環境(Microsoft Basic Display Adapter)は
  この状態になるとみている(推測)。
- **開けなかったときの順**(release の exe、`--first-frame` で撮影扱い = 箱を出さない。ほかは 1 回ずつ):

| 場合 | 結果 |
|---|---|
| A: `WGPU_BACKEND=metal`(DX12 だけ開けない) | DX12 で失敗 → OpenGL で開き直して開けた(`renderer=OpenGL 3.3 / NVIDIA 610.62`) |
| B: `--render-step 1 --no-hw`(OpenGL から始めて OpenGL が開けない) | OpenGL で失敗 → WARP で開けた(`renderer=DirectX 12 / Microsoft Basic Render Driver`) |
| C: `WGPU_BACKEND=metal --no-hw`(3 つとも開けない) | DX12 → OpenGL → WARP と順に失敗し、終了コード 1 |
| D: `--no-hw` だけ | DX12 で開けた(`--no-hw` は OpenGL にしか効かない) |
| E・G: `--glow --no-hw`・`--dx12`(+ metal) | 指定した描き方だけ試して失敗し、開き直さない(終了コード 1) |
| F: C を撮影なしで | 箱は最後のプロセスの 1 つだけ。中身は「zahyou の画面を開けませんでした。」と 3 行(DirectX 12 / OpenGL / DirectX 12 (ソフトウェア描画 WARP) の理由)とドライバーの案内(UI オートメーションで読んで閉じた) |

確かめていないこと(推測・資料):

- Windows 11 のシステム要件には「DirectX 12 以上に対応したグラフィックス / WDDM 2.0 ドライバー」がある(Microsoft の要件の記載。
  このセッションでは外の資料を読んでいないので、覚えている内容)。要件を満たす PC なら DX12 のハードウェアの経路があり、
  ドライバーが壊れていても WARP がある。OpenGL は GPU の会社のドライバー(ICD)が入っていて、それが窓の画面の GPU のものである
  ときだけ使える。
- **リモート デスクトップ**: 試していない(この PC へつなぐと利用者の画面のセッションを奪うため)。推測: DX12 は RDP のセッションでも
  GPU か WARP で開ける。OpenGL は RDP の仮想の画面に GPU の会社の ICD が無いと 1.1 になり開けない(会社とドライバーの版による)。
  DX12 が先なので、OpenGL が開けなくても困らない。
- Intel・AMD の内蔵 GPU の OpenGL ドライバーの出来(この PC では内蔵 GPU の OpenGL を動かせない)。一般には Windows の OpenGL は
  DirectX より手当てが薄いと言われる(推測。測っていない)。
- Windows の「グラフィックの設定」でアプリごとに選んだ GPU が DXGI の列挙の順(= `GpuPick::Display` の選ぶ GPU)に効くか
  (OS の設定を変えないため試していない)。効かなくても `--gpu high|low` で選べる。
- ノート PC の電池の減り(この PC はデスクトップで測れない)。外付け GPU を起こす遅れと電力は、内蔵 GPU で描けば掛からない(推測)。

### 9.5 決めた理由

| 観点 | DX12(既定) | OpenGL |
|---|---|---|
| 窓が開く確かさ(Windows 11) | GPU のドライバーが無くても WARP で開く(実機) | 会社のドライバー次第。1.1 しか無いと開けない(実機) |
| 図を動かす速さ(75 Hz) | 24・61 MP とも全フレームが垂直同期に間に合う。p99 15.9〜17.0 ms | 同じ。p99 14.6 ms |
| 上限なしの速さ | 255〜257 fps | 237〜240 fps |
| テクスチャを初めて上げるときの引っかかり | 61 MP で 129 ms(1 周目だけ) | 108 ms |
| 最初の描画まで | 0.36 秒 | 0.20 秒 |
| 画面だけの私有 / 61 MP 表示中の私有 | 248 / 906 MB | 131 / 826 MB |
| ノート PC の GPU を選べるか | 選べる(主の画面の GPU = ふつう内蔵) | 選べない |

- **速さは同等**(どちらも 61 MP で全フレームが 75 Hz に間に合い、上限なしでは DX12 が 7% 速い。OpenGL は p99 と起動が少しよい)。
  差は人が気づく大きさではない。
- **安定は DX12**: Windows 11 の PC なら必ずあり、ドライバーが無い・壊れている・リモート デスクトップでも WARP で開ける。OpenGL は
  ドライバー次第で、開けても描き方の不具合はアプリから見えない(開き直しでは救えない)。想定のノート PC は内蔵 GPU(Intel・AMD)が
  多く、そこで使われるのは DX12 のほうが多い経路(推測)。
- **メモリ**: 小さな塊にしたので、OpenGL との差は画面だけで 117 MB、61 MP 表示中で 80 MB(RAM 8 GB の 1〜1.5%)。大きな塊のままだと
  308 MB 多く、8 GB の PC には重い。小さな塊で速さは変わらなかった(p99 15.9 / 17.0 ms 対 16.1 / 17.0 ms)ので採った。
- **起動**の 0.16 秒の差は、目標の 1 秒以内(0 節)に収まる。
- **GPU の選び方(電池と速さ)**: 主の画面の GPU(`PowerPreference::None`)にした。ふつうのノート PC(内蔵の画面が内蔵 GPU)では
  内蔵 GPU で描くので電池を持たせ、外付け GPU を起こさない。測った限り内蔵 GPU でも速さは同じ(9.3。CPU 側が律速)。主の画面が外付け
  GPU にある PC(このデスクトップ、外付け GPU だけで動かすノート PC、外付け GPU の端子の画面を主にしたとき)では外付け GPU で描き、
  GPU の間の写しが要らない。egui の既定の `HighPerformance` は、いつも外付け GPU を起こすので採らない。`LowPower`(内蔵 GPU を先に)は、
  主の画面が外付け GPU にあるときも内蔵 GPU で描いて写す(この PC で動いて速さも同じだったが、写しが余分)ので既定にしない。
- **OpenGL を 2 番目に残す**のは、DX12 のハードウェアの経路だけが壊れているとき(ドライバーの不具合で D3D12 の装置を作れない)に
  WARP より速く描けるから。WARP は最後の手段(2 fps)。

### 9.6 残っていること

- ~~開いた描き方と GPU(`RENDER_INFO`)を画面のログ(起動の 1 行目)と `--engine-check`・自己診断の記録に出す~~ → 画面のログの
  2 行目に出した(10.5 節)。`--engine-check` と `--selftest` は窓を開かないので出せない(問い合わせのときは画面のログを写してもらう)。
- ~~rs/README.md の「exe の入口」に `--bench-ui`・`--dx12`・`--gpu` を足す~~ → 足した(10.5 節)。
- リモート デスクトップ・GPU のドライバーが無い PC・Intel や AMD の内蔵 GPU の OpenGL・ノート PC の電池は確かめていない(9.4)。
  β の間に利用者の PC で窓が開かなかった・遅いという報告があれば、`zahyou.exe --bench-ui <画像> --out f.json`(既定の描き方)と
  `--glow` の記録をもらう。
- 61 MP の原寸のテクスチャを初めて上げるフレームが 108〜129 ms 引っかかる(拡大して原寸の段に入る最初の 1 回)。タイルを何フレームかに
  分けて上げれば消せる(figure.rs。担当外)。
- 図の縮小の段は、GPU に上げたあとも CPU 側に残る(61 MP で約 320 MB)。内蔵 GPU では GPU 側も RAM なので二重になる。上げ終えた段を
  CPU 側から捨てれば減らせる(figure.rs。担当外。画像を保存するときは zahyou-plot の図から描き直すので要らないとみている)。
- 1 フレームの CPU の処理 3.6〜4.0 ms(拡大・移動中)は、遅いノート PC の CPU では 2〜3 倍になるとみている(推測)。それでも 60 Hz の
  16.7 ms に収まる見込み。

## 10. 2026-10-16 の決定(API キー・描画方式・β 版)と点検

### 10.1 ユーザーの決定と担当

| | 決定(2026-10-16) | 担当 | 結果 |
|---|---|---|---|
| 1 | オンライン解析の API キーは、画面に入力欄を作って利用者が入力する。既定では元々の API キー(作者のもの)が入った状態にする | apikey 係 | ［解析］の枠に欄。既定は v6 と同じキー(10.2) |
| 2 | 画面の描画(DX12 か OpenGL か)は、性能が良くて安定している方を既定にする。想定は Windows 11 が動く RAM 8 GB 以上のノート PC | render 係 | DX12・主の画面の GPU・小さな塊(9 節、10.3) |
| 3 | v7 は当面β版。v6 の自動更新の対象にはしない。1〜2 か月使って安定を確かめてから、正式版として main に統合する | beta 係 | 版 7.0.0-beta.1、プレリリースで出す(10.4・10.7) |

点検とつなぎ目の直しは check 係(10.5・10.6)。キーの値はこの文書にも、ログ・試験の出力・画面の文にも書かない。

### 10.2 API キーの欄(apikey 係)

- `zahyou_net::nova::DEFAULT_API_KEY` に v6 の既定値を写した(`main:gui/zahyou_gui.py` の `_params` と `dev/nb_settings.py` の 2 か所と、
  長さ 16 と sha256 で一致を確かめた)。初めの値は `initial_api_key()` = 環境変数 ASTROMETRY_API_KEY、無ければ既定のキー(v6 の
  `os.environ.get(…, 既定)` と同じ)。
- 型 `ApiKey` は Debug で「伏せ字 N 文字」と書き、Display を持たない。`OnlineRequest`・`Settings`・`RunParams` の Debug も伏せる。
  `solve_online_with` はログの行と失敗の文にキーが混じっていれば `redact_key` で「(API キー)」に置き換える(nova の応答を文に入れる箇所があるため)。
- 画面: ［解析］の枠の「打ち切り」の下に「API キー (nova.astrometry.net)」の欄・「表示」・「元に戻す」(既定のキーのときは押せない)。
  欄は既定で伏せ字(egui の「•」)。欄から離れたときと「元に戻す」で gui.json に保存する。欄の下に、既定のキーか・自分のキーか・空か
  と「方法が『自動』『オンラインのみ』のときに使います」。
- gui.json: キー `api_key` を v6 のキーの並びのあとに足した。無い・文字列でないときは初めの値。空の文字列も覚える(利用者が空にした =
  問い合わせない)。平文(指示どおり。資格情報マネージャーには入れていない)。
- 解析: 画面からは欄の値(`RunParams.api_key = Some`)。空なら「自動」は「[オンライン] API キーが空なので、nova.astrometry.net には
  問い合わせません。」と書いてオフラインへ、「オンラインのみ」は始める前に「API キーが空です。…」と知らせて始めない。
  `None`(自己診断・速さの物差し)は初めの値。
- **既定のキーが入るので、「自動」はネットがあれば nova に星の位置を送る(解けなければ画像も)**。~~v6 と同じ動きで、~~決定 1 のとおり。
  (2026-10-19 に訂正: v6 の exe は画像を送っていなかった。同梱の astroquery 0.4.10 は photutils があると画像を上げず、自分で星を数えて
  位置を送る。exe が画像ファイルを送るのは v7 から。docs/release-v7.md の 2026-10-19 の記録)
  試験は RunEnv の `online: Some(false)` か空のキーで、本物の nova へは送らない(10.6)。`--shot --run` は gui.json の設定のまま
  動くので、送りたくなければ一時の gui.json を「オフラインのみ」か `"api_key": ""` にする(rs/README.md)。
- 自己診断に「API キーの欄がある」を足した(25 項目)。補足は「欄あり / 伏せ字 / 既定のキーと一致 (16 文字)」の形で、値は出さない。
  「部品が窓の中にある」は、［解析］タブでは左の列をいちばん下まで送った状態で見る(最小の窓 1000×660 では欄が下に隠れるため)。

### 10.3 描画方式(render 係。詳細は 9 節)

- 既定は **DirectX 12**、GPU は主の画面をつないでいる GPU(ノート PC ではふつう内蔵 GPU)、GPU メモリは小さな塊。開けなければ
  OpenGL → DX12 のソフトウェア描画(WARP)の順に起動し直す。
- 根拠(9.5): 速さは同等(24・61 MP とも 75 Hz の全フレームに間に合う。上限なしでは DX12 が 7% 速い)。安定は DX12(Windows 11 なら
  必ずあり、ドライバーが無くても WARP で開く。OpenGL は会社のドライバー次第で、1.1 しか無いと開けない — どちらも実機で確かめた)。
  メモリの差は小さな塊にして画面だけで 117 MB(以前は 425 MB)、61 MP 表示中で 80 MB = 8 GB の 1〜1.5%。
  起動は OpenGL が 0.16〜0.20 秒早いが、目標の 1 秒以内に収まる。
- 確かめていないもの: リモート デスクトップ、GPU のドライバーが無い PC、Intel・AMD の OpenGL、ノート PC の電池(9.4・9.6)。

### 10.4 β版(beta 係)

- 版名は `rs/Cargo.toml` の `[workspace.package] version` = **`7.0.0-beta.1`** の 1 か所で決まる。`APP_VER` = `"v" + CARGO_PKG_VERSION`
  (画面・ログ・更新の確認)。解析ログの見出しも同じ版名(Python の基準値と比べるときは見出しの版名だけ伏せる)。
- exe の版情報(build.rs): FileVersion(文字列)`7.0.0-beta.1`、ProductVersion `v7.0.0-beta.1`、数の版 7.0.0.0(β の番号は入れない。
  4 つ目に入れると正式版 7.0.0.0 より大きくなる)、プレリリースの版なら FILEFLAGS に VS_FF_PRERELEASE。
- 更新の確認(`zahyou_net::update`): 呼び出し方は変えていない。
  - `-dev` で終わる版: 問い合わせない(前のまま)。
  - プレリリースの版(β): `releases?per_page=100` の一覧から、大きな版(major)が同じで semver で今より新しいもののうち最も新しいもの
    (下書き・`-dev` のタグ・v6 など大きな版の違うもの・資産 zahyou.exe の無いものは使わない)を選び、その資産の digest と動いている exe を比べる。
    次のβ版も v7 の正式版も受け取る。semver の比較は自前(7.0.0-beta.1 < beta.2 < beta.10 < rc.1 < 7.0.0)。
  - 正式版の版: 前と同じく `releases/latest`。latest が今より古い版なら「最新版です」(v7 正式版のあとに v6 の修正版が latest になっても戻さない)。
- **v6 には届かない理由**(check 係が v6 のソースで確かめた): v6 の `gui/zahyou_update.py` は `releases/latest` の資産 zahyou.exe の digest と
  自分の exe を比べるだけで、版名は比べない。GitHub の latest はプレリリースを返さない(REST API の文書で確かめた。10.10)ので、
  β をプレリリースで出している間は v6 の「更新を確認」は v6 の最新を返す。**v7 の正式版を Latest で出した時点で、v6 の全員に「新しい版があります」が出る**。

### 10.5 つなぎ目の直し(check 係)

- **「方法」を v6 の文言で gui.json に書く**(settings.rs `Solve::stored_label`): v7 は「オフラインのみ」を「オフラインのみ」と書いていたが、
  v6 の選択肢は「オフライン (WSL) のみ」なので、v6 に戻すと v6 は知らない文言として「自動」に戻していた。そうなると、オフラインを
  選んでいた人が β を試して v6 に戻したとたん、v6 が黙って nova へ問い合わせる(v6 も既定のキーを持つ)。v6 の文言で書き、v7 は
  読むときに読み替える(前から)。v6 を配らなくなったら `label()` に戻してよい。試験 `方法は_v6_の文言で書く`。
- **「β版」の語**: `prerelease_label(APP_VER)` から作る `prerelease_suffix()`(main.rs)で、窓の題「zahyou v7.0.0-beta.1 (β版) — …」・
  起動のログの 1 行目「zahyou v7.0.0-beta.1 (β版・Rust 版)   フォント: …」(app.rs `startup_line`)・［使い方］の「いまの版: … (β版)」。
  正式版に上げれば自然に消える。解析ログの見出しには添えない(Python の基準値と比べる行のため)。
- **［使い方］の「新しい版に入れ替える / やめる」**に、β の間だけ 3 行(「更新を確認」は次のβ版と v7 の正式版を探す・β版は v6 の
  「更新を確認」には出てこない・v6 に戻すときは v6 の zahyou.exe を上書きする(API キーの欄の値は v6 では使わない))。
- **開いた描き方と GPU** を画面のログの 2 行目に「描画: DirectX 12 / NVIDIA GeForce RTX 3060 (DiscreteGpu, ドライバー …)」と出す(9.6)。
- 撮影の隠しの引数 `--left-end`(［解析］の左の列をいちばん下まで送って撮る。小さい窓で隠れる API キーの欄を目で確かめるため)。
- rs/README.md: exe の入口(`--dx12`・`--glow`・`--gpu`・`--bench-ui`・隠しの引数)、自己診断 25 項目、API キーと外へ送ること、版名の決まり方。
- 4.4 節(check_update の説明)・7.5・8.4・9.1・9.6 節の決着した項目を書き換えた。
- 足した試験 3 本: `窓の題に_beta_版を添える`(main.rs)、`beta_版の間だけ更新の扱いを説明する`(help_tab.rs。起動のログの 1 行目も)、
  `方法は_v6_の文言で書く`(settings.rs)。試験の名前にギリシャ文字は使えない(clippy の mixed_script_confusables)。

### 10.6 試験と実測(check 係、2026-10-16)

同じ機械(Core Ultra 7 265K)。exe と試験は ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時フォルダーへ向けた。

| 試験 | 結果 |
|---|---|
| `cargo test --workspace -- --include-ignored`(3 係の変更がそろった木、check 係の直しの前) | **274 本通過・失敗 0**(16 分 39 秒)。app 76 / solver 42 / image 37 / net 49 / plot 29 / wcs 25 / index 16。重い 12 本(合成星野・実写の全部、陰性の締め切り、外への疎通 3 本、画面からの中止、run() の全 20 場面)を含む |
| check 係の直しのあと `cargo test -p zahyou -- --include-ignored` | **79 本通過・失敗 0**(足した 3 本を含む。ほかの crate は直していない) |
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0(直しの前と後) |
| `tools/check_licenses.py`・`tools/gen_notices.py --check`・`tools/test_tools.py` | OK(依存 213 個・本文 35 種。依存は増えていないので NOTICES は作り直していない)・最新・15 本通過 |
| release の `--selftest --offline`(基準の実写) | **25/25 を 3 回**(プロセス全体 0.97〜1.26 秒。解析 0.1 秒・画面から 0.4 秒)。1.594′ / 0.916′ 東 / 1.304′ 北 / 1.4610″/px(空の cache.json。8.2 節と同じ) |
| `--shot`:［解析］(明るい・暗い・最小の窓 1000×660・最小の窓で左の列を下まで)・［使い方］・ログ | 欄・「表示」・「元に戻す」・説明が枠に収まる。伏せ字は「•」16 個。最小の窓では欄は左の列を送ると出る(送り棒あり)。説明の「オンラインの/み」の改行位置だけ見た目が惜しい。題・ログ・いまの版に「β版」、ログに「描画: …」 |
| キーの値が出ないこと | 試験の出力(274 本の全文)・自己診断・`--engine-check`・`--bench`・撮影の画像を含む check 係の作業フォルダーの 69 ファイル(exe を除く)で **0 件**(gui.json だけが設計どおり平文で持つ)。画面は伏せ字を目で見た |
| exe の版情報 | FileVersion 7.0.0-beta.1・ProductVersion v7.0.0-beta.1・数の版 7.0.0.0・IsPreRelease True。14,038,016 バイト(13.4 MiB) |
| `--update-check`(本物の GitHub。一覧を読むだけ) | `up_to_date`「最新版です (v7.0.0-beta.1)。」(今の Releases に v7 のタグは無い) |
| 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json | sha256 は作業の前後で同じ |

**gui.json の互換**(本物の exe。一時の場所に置いた gui.json で画面を開き、WM_CLOSE で閉じて保存させた。v6 の読み書きはソースの規則を写して当てた):

| 置いた gui.json | 保存後 | v6 が読むと |
|---|---|---|
| v6 の実物の写し(api_key なし、方法「オフライン (WSL) のみ」) | v6 の 18 キーはすべて同じ値・同じ並び・同じ書式(indent 1)。`api_key` が末尾に足され、既定のキー(16 文字)と一致 | 全部読める(焦点距離・センサーは元から空で、v6 も既定のまま)。**直す前は方法が「オフラインのみ」に書き換わり、v6 は「自動」に戻していた**(10.5) |
| `"api_key": ""` | 空のまま(既定に戻らない) | v6 は api_key を読まない |
| `"api_key"`: 自分のキー(17 文字) | そのまま | 同上。**v6 が保存すると api_key は消え、次の v7 は既定のキー(か環境変数)に戻る** |

**速さ**(`--bench`。24 MP は r3-integrate の解ける合成画像 synth24.fits、もう 1 枚は基準の実写。3 回の最短):

| 段 | 24 MP | 968×548 | 8.3 節の 24 MP |
|---|---|---|---|
| 前処理 | 0.537 | 0.022 | 0.484 |
| 解法 | 0.360 | 0.019 | 0.331 |
| 図 2 枚 | 0.340 | 0.021 | 0.315 |
| 解析全体 run()・目標あり / なし | 1.193 / 1.160 | 0.069 / 0.057 | 1.096 / 1.064 |

今日の機械は 8.3 節のときより全段で約 9% 遅かった。コードの変わっていない段(前処理・図)も同じ割合なので、同じ日に 8.3 節の exe
(cargo-target-main の 13,897,216 バイト)と交互に測ると run() 1.190 / 1.164 秒 対 1.204〜1.221 / 1.154〜1.159 秒で、**差は無い**
(機械の状態の差とみている)。`--bench` の既定の合成画像(乱数の星)は解けない画像なので、解法の段は締め切りの 60 秒になる(前から)。

**起動**(`--first-frame`、DX12 と OpenGL を交互に 3 回): 最初の描画まで DX12 368〜404 ms、OpenGL 200〜206 ms。
撮影ですぐ閉じたときのプロセス全体は DX12 1.18〜1.20 秒・OpenGL 0.55〜0.60 秒(DX12 は閉じてから終わるまでに約 0.5 秒かかる。
窓が消えるのが先か後かは見ていない)。

### 10.7 β版の出し方(配布手順書 docs/release-v7.md へ反映する内容)

1. `rs/Cargo.toml` の `[workspace.package] version` を `7.0.0-beta.N` にする(題・ログ・exe の版情報が付いて変わる)。`cargo build --release -p zahyou`、
   `--selftest --offline` が全項目通過、exe のプロパティでファイル バージョン `7.0.0-beta.N` を確かめる。
2. タグは `v7.0.0-beta.N`(v + Cargo の版)、`native-rs` の commit に付ける。**main には統合しない**。
3. GitHub の Release は **「Set as a pre-release」に印、「Set as the latest release」は付けない**。題名の例「zahyou v7.0.0-beta.N (β版)」。
   gh なら `gh release create v7.0.0-beta.N zahyou.exe --prerelease --latest=false --target native-rs --title "zahyou v7.0.0-beta.N (β版)" --notes-file …`
   (gh は `--prerelease` と `--latest=false` を一緒に受け付ける。`--latest=false` は無くても latest にはならないが、意図を明示するため重ねる。10.10)。
4. 資産の名前は必ず **`zahyou.exe`**(更新の確認はこの名前で探す。digest は GitHub が付ける)。
5. **公開した β の資産は差し替えない**。直したら N を上げて新しい Release にする(β の更新の確認は「今より新しい版」しか出さないので、
   同じタグの建て直しは β の利用者に届かない)。
6. 出したあとの確かめ: (a) `https://api.github.com/repos/Akashi3060/zahyou/releases/latest` の tag_name が v6 のまま、(b) v6 の exe の
   「更新を確認」が「最新版です」、(c) 一つ前のβ版の exe の `--update-check` が「新しい版があります: v7.0.0-beta.N」、
   (d) 新しいβ版の exe の `--update-check` が「最新版です (v7.0.0-beta.N / 日付 版)」。
7. **間違えて正式の Release(pre-release の印なし)で出すと、そのまま v6 の全員に入れ替えが出る**。印を付け忘れたら、すぐ pre-release に直す。
8. β の間に v6 の修正版を出すときは、ふつうの Release(Latest)でよい(β の利用者は大きな版 7 しか見ないので影響しない)。
9. 正式版: 1〜2 か月使って安定を確かめてから、`native-rs` を main に統合し、`7.0.0` に上げて **ふつうの Release(Latest)** で出す。
   gh では `--latest` を明示する(新しく公開する正式の Release は API の既定で latest になるが、Docs の画面の説明と書き方が食い違うため。10.10)。
   これで v6 の利用者にも入れ替えが出る(v6 は digest しか比べないので)。β の利用者にも出る。
10. 正式版のあとに v6 の修正版を出すときは `--latest=false`(latest が v6 に戻ると、v6 の利用者向けの意図とずれる。v7 の利用者は
    latest が古ければ入れ替えに誘わないので守られる)。
11. 利用者への案内: β から v6 へ戻すときは v6 の zahyou.exe を上書きする。設定は引き継がれる(方法も 10.5 で直した)。API キーの欄の値は
    v6 では使わず、v6 で保存すると消える(次に v7 を開くと既定のキー)。

### 10.8 残っていること

- ~~**別のセッションの下書き(未 commit)の前提が変わった**(check 係は触っていない): `rs/tools/shoot_docs.py` と `docs/release-v7.md`(118 行
  付近)は「環境変数 ASTROMETRY_API_KEY を消せば nova へ送らない」前提だが、既定のキーが入るので成り立たない。撮影の一時の gui.json に
  `"api_key": ""` を書くか方法を「オフラインのみ」にする(shoot_docs.py は `offline` の指定があるときだけ `{"solve": "オフラインのみ"}` を
  書くので、それ以外の撮影で `--run` を使うと、ネットがあれば nova へ送る — ソースを読んだだけで、動かしてはいない)。［解析］の左の列に欄が増えたので、切り抜きの枠(app-analyze など)の見直しも要る。
  README.md 185〜190・545 行と dev/make_manual.py 436〜440 行は、まだ「環境変数に入れて setx」と説明している(画面の欄に変わった)。
  docs/release-v7.md の「決めること 1」の API キーの部分と、β の出し方(10.7)も反映が要る。~~ → 64526af で反映した(shoot_docs.py は
  exe の通信を手元で止めて撮る。README・マニュアル・release-v7.md は欄の説明とβの出し方に)。
- 本物の nova へキーで送る道は試していない(偽の nova だけ)。β の間に一度、利用者の判断で「オンラインのみ」で解いて確かめたい
  (星の位置と、解けなければ画像が nova へ送られる)。
- ~~GitHub の「latest はプレリリースを返さない」「プレリリースは latest にできない」「新しい正式の Release は既定で latest になる」は記憶による
  (このフォルダーでは外の資料を読まない約束)。claude-research のセッションで公式の説明を確かめたい。~~ → claude-research が公式の文書と
  gh のソースで確かめた。3 つとも記憶のとおりで、gh は `--prerelease` と `--latest=false` を一緒に受け付ける(10.10)。
- ~~画像そのものを外へ送ることへの同意の取り方は未決(7.5。v6 と同じく、送る前に聞かない)。~~ → 同意を取らずに送ってよいと決まった(10.9 節)。
- ~~**nova.astrometry.net が、API で送った画像を公開するか**を確かめていない。zahyou は送るときに公開の扱い(`publicly_visible` など)を
  指定していない(Python の v6 も同じ)。nova の既定では公開されるとみている(記憶による。このフォルダーでは外の資料を読まない約束なので、
  claude-research で確かめたい)。公開されるなら、利用者への説明に足すか、送るときに公開しない指定を付けるかを決める。~~ → 付けないと公開
  (`'y'`)だった。公開しない指定 `publicly_visible = "n"` を付けると決まり、Rust 版に入れた(10.10)。
- gui.json の API キーは平文(決定どおり)。
- 描画: リモート デスクトップ・ドライバーの無い PC・Intel や AMD の OpenGL・ノート PC の電池は未確認。61 MP の最初のテクスチャの引っかかり
  (108〜129 ms)と、縮小の段を CPU 側にも持つこと(61 MP で約 320 MB)は figure.rs で減らせる(9.6)。DX12 は閉じてから終わるまで約 0.5 秒。
- 最小の窓(1000×660)では API キーの欄は左の列を送らないと見えない。欄の説明の改行が「オンラインの/み」で切れる(見た目だけ)。
- β の期間が過ぎて v6 を配らなくなったら、`Solve::stored_label` を `label()` に戻してよい(10.5)。

### 10.9 2026-10-16 の 2 回目の決定(画像の送信・β の Release の資産)

| | 決定 | 反映 |
|---|---|---|
| 4 | 画像そのものを外へ送ることは、**同意を取らずに送ってよい**。今の動きのまま(方法が「自動」か「オンラインのみ」で、星の位置で解けなければ画像も nova.astrometry.net へ送る。送る前に聞かない) | 何が送られるか(まず検出した星の位置と画像の大きさ、目標の方向・画素スケールが分かればその手がかり。星の位置で解けなければ画像ファイルそのもの(ファイル名も、作り変えずに))と、送らない方法(方法を「オフラインのみ」にする / API キーの欄を空にする)を、README・マニュアル 2.3・［使い方］タブ(help_tab.rs。試験 `手引きは外へ送るものと止め方を説明する`)にそろえて書いた。マニュアル 7 章の「星の位置で照合するので取り違えが起きない」は、zahyou の中のエンジンの話と分かるように直し、オンラインでは画像ファイルを作り変えずに送ることを足した |
| 5 | β の Release にも exe 以外を付ける(v6 の Release に合わせる) | 資産は `zahyou.exe`・`zahyou_v7_beta.ipynb` と `zahyou_engine.py`(ノートブック版)・`zahyou-manual-v4.pdf`・`LICENSE.txt`・`THIRD-PARTY-NOTICES.md`(rs/ のもの)・`SHA256SUMS.txt`。`rs/tools/make_release_assets.py` がそろえて中身を検査する(docs/release-v7.md 6 節) |

- `cargo test -p zahyou`(help_tab.rs を直したあと): 78 本通過・失敗 0(重い 2 本は既定どおり飛ばした)。
- ノートブックはエンジンを「同じフォルダーの `zahyou_engine.py`」という名前で探す(セル A)ので、ノートブックの名前を変えても動く。
  資産の名前に改名したノートブックで、セル A とセル C(方法「オフラインのみ」)を動かして確かめた(docs/release-v7.md 6 節)。

### 10.10 2026-10-16 の 3 回目の決定(nova の公開)と、GitHub の事実の確かめ(rust 係)

| | 決定 | 反映 |
|---|---|---|
| 6 | オンライン解析で nova.astrometry.net に送るとき、**`publicly_visible = "n"`(公開しない)を付ける**。Rust 版と Python 版(ノートブックを含む)の両方。文書に「nova に送ったものは第三者に見られることがある」と書く | 下のとおり。Python 版(`dev/zahyou_core.py` の `solve_online`。ノートブックはこれを使う)は native-solver の係が直し、あとで人が取り込む(→ native-solver の `b878079` を `53e485e` で取り込み済み。11 節) |

**根拠**(claude-research の調べ物 `notes/zahyou-v7-release-facts-20261016.md`。公式の文書とサーバー・astroquery・gh のソースを読んだだけで、
nova へ送ったり Release を作ったりはしていない。出典の URL は docs/release-v7.md 0.1 節):

- nova のサーバーは `publicly_visible` が無いと `'y'`(公開)にする(画像も星の位置のリストも同じ受付処理)。公開だと画像のページ・「最近送られた
  画像」・Explore・利用者ごとの一覧などに、API キーの持ち主のアカウント名で載る。astroquery は既定値を自分では付けないので、v6 も Rust 版も
  これまでは公開で送っていたことになる(既定のキーなら作者のアカウントで。サーバーのソースの読みで、実サイトの一覧で確かめたのではない)。
- `"n"` は保存されるだけで、解く条件には使われない(解き方は変わらない)。
- ただしサーバーのコードの読みでは、`"n"` でも一覧(最近・全件・利用者ごと・検索)・画像ファイルとサムネイル・API の `/api/jobs/<id>/…` は
  公開の印を見ていない → 完全な非公開ではない可能性がある(**実サイトでは未確認**)。だから利用者向けの文書には「第三者に見られることがある」と書いた。
- GitHub: `releases/latest` はプレリリースと下書きを返さない。プレリリースは latest にできない。新しく公開する Release の `make_latest` は既定 `true`
  (正式版は既定で latest)。Docs の画面の説明(選ばなければセマンティックバージョニングで自動)とは書き方が食い違う。`gh release create` は
  `--prerelease` と `--latest=false` を一緒に受け付け、`--latest` を付けたときだけ `make_latest` を送る。→ 10.4 と 10.7 の「記憶による」を書き換え、
  docs/release-v7.md の手順を β は `--prerelease --latest=false`、正式版は `--latest` を明示する形にした。

**Rust 版の実装**(`crates/zahyou-net/src/nova.rs`):

- `PUBLICLY_VISIBLE = "n"` と `private_settings(settings)`: 先頭に `("publicly_visible", "n")`、続けて呼ぶ側の設定(呼ぶ側が `publicly_visible` を
  入れていても捨てて `"n"`)。星の位置のリストの本文(`source_list_body`)と画像の本文(`upload_body`)の両方がこれを通るので、
  `NovaClient::submit_source_list`・`submit_image` を誰がどう呼んでも `"n"` で送る。ログインの本文には付けない。
- 並びは Python 版と同じ先頭(Python は `settings = {"publicly_visible": "n"}` に手がかりを足す。astroquery は渡された順のまま `json.dumps` する)ので、
  request-json は 1 文字まで Python と同じ。

**試験**(本物の nova へは送っていない。偽の nova・astroquery の記録・送る本文の検査だけ):

- `rs/testdata/gen_net.py` で `net_python.json` を作り直した。星の位置のリストの 4 場面は前と同じく Python の `solve_online` を通して astroquery の
  要求を記録(native-solver の作業ツリーの、`publicly_visible` を足した `dev/zahyou_core.py` を読んだ)。画像の場面は、前は astroquery を手で
  呼んでいたのを、`solve_online` に星のリストを渡さず astroquery の `_no_source_detector` を立てて記録するように変えた(Python 版が付ける設定が
  そのまま入る)。送る要求の request-json に `publicly_visible='n'` が無ければ値を作らずに止まる(`check_private`)。作り直しで変わったのは
  送る 5 本の要求の本文だけ(星図データ・書き方・天体名・更新の部分は前と同じ値になることを確かめた)。
- `tests/parity.rs` `nova_bodies_match_python`: 画像の request-json を `upload_body` の本文から取り出して Python の記録と比べる形にした。
- `tests/nova_flow.rs` に `sends_publicly_visible_n_with_source_list_and_image` を足した: 偽の nova に星の位置のリスト(解けない)→ 画像(解ける)と
  送らせ、受け取った本文(form と multipart)から request-json を取り出して、`publicly_visible` が `"n"` で 1 つだけ先頭にあること・手がかりと
  星の位置が残っていること・ログインには無いことを見る。呼ぶ側が `"y"` を渡しても `"n"` で送ることも、窓口の関数で直接見る。
  `private_settings` の行を外すと、この試験と Python の記録と比べる 2 本(`source_list_flow_matches_python`・`image_upload_flow_matches_python`)が落ちる
  ことを確かめた(戻してから試験を通した)。
- 手引き: ［使い方］(help_tab.rs)に 3 行足し、試験 `手引きは外へ送るものと止め方を説明する` に「公開しない指定」「第三者に見られることがあります」
  「完全には隠れない場合があります」を足した。README・マニュアル v4(2.3。PDF を組み直した。33 ページのまま)・rs/README.md も同じ趣旨。

**試験の結果**(実測。同じ機械。cargo の試験と exe は LOCALAPPDATA・ZAHYOU_CACHE・ZAHYOU_SETTINGS を一時のフォルダーへ向け、
`ZAHYOU_OFFLINE_TESTS=1`。exe はさらに ALL_PROXY・HTTPS_PROXY・HTTP_PROXY をつながらない `0.0.0.0:9` にして、`--update-check` が
外へ出ずに失敗することを先に確かめた):

| 試験 | 結果 |
|---|---|
| `cargo test -p zahyou-net -p zahyou` | **通過 124 本・失敗 0**(zahyou 78・zahyou-net 46。外への問い合わせの 3 本と、重い・手で走らせる 3 本は既定どおり飛ばした。天体名を CDS に引く 1 本は ZAHYOU_OFFLINE_TESTS で飛ばした)。1 分 30 秒 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0 |
| `tools/test_tools.py`・`tools/gen_notices.py --check`・`tools/check_licenses.py` | 15 本通過・最新(クレート 213 個・本文 35 種。依存は増えていない)・許可リストの外なし |
| `tools/test_make_release_assets.py`(release の exe を `ZAHYOU_RELEASE_EXE` で渡した) | 22 本通過 |
| `tools/make_release_assets.py`(一時のフォルダーへ) | version・notices・manual・notebook の 4 つの検査が通り、`sha256sum -c` で 6 つとも OK。組み直したマニュアル v4 はリポジトリの PDF とバイト単位で同じ |
| release の `--selftest --offline`(基準の実写) | **25/25**・rc=0(1.594′ / 0.916′ 東 / 1.304′ 北 / 1.4610″/px)。exe 14,041,088 バイト |
| マニュアル v4 の組み直し | 直す前の make_manual.py(HEAD)で組むとリポジトリの PDF と同じバイト(作成日時の固定が効いている)。直したあとは 2.3 の「送るもの」に 1 段落増え、33 ページのまま |
| 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json | sha256 は作業の前後で同じ |

- 本物の nova の扱い(`"n"` で本当に一覧から隠れるか)は確かめていない(送っていない)。docs/release-v7.md 0 節の残りの 1 に、β の間に一度
  「オンラインのみ」で送って確かめるときの見どころを足した。
- ~~β の資産のノートブック(`zahyou_v7_beta.ipynb`・`zahyou_engine.py`)は、Python 版の直し(native-solver)を取り込んで `dev/build_nb.py` で
  作り直すまで、`publicly_visible` を付けずに送る。β を出す前に取り込むこと。~~ → 2026-10-16 に決着: `b878079` を `53e485e` で取り込み、
  `dev/build_nb.py` で作り直した。β1(`8b7060c`)と β2(`0b73d36`)の `zahyou_engine.py` は `solve_online` で `publicly_visible: "n"` を付ける。

## 11. 2026-10-17 の保守(試験の正解データ・試験の一時フォルダー・自己診断の記憶・文書と版)

β2 を出したあとに残りの作業を洗い出し、ユーザーの判断が要らず、コードと文書だけで済むものを直した(push・タグ・Release はしていない)。
commit は `37adbc4`(正解データ)・`0b394e3`(一時フォルダー)・`bd0ecca`(自己診断の記憶)と、この節を書いた文書と版の試験の commit。

### 11.1 試験の正解データをリポジトリの外の残る場所へ

- WSL 版 astrometry.net の正解(`oracle`: 366 ファイル・4,944,704 バイト。`tests-data`: 159 ファイル・160,570,516 バイト)が、作ったときの
  Claude の一時の作業場所(scratchpad。いずれ消える)にしか無く、`rs/testdata/gen_pipeline.py` の `--oracle` の既定と
  `dev/solver_truth.json` の `sources` がそこを指していた(`--oracle` の既定は作者のホームの絶対パスでもあった)。
- **写し先は `%USERPROFILE%\Documents\claude\zahyou\testdata\{oracle,tests-data}`**(リポジトリの外)。全ファイルの sha256 を元と照合して一致
  (写し先の `SHA256SUMS.txt` に 525 行。`sha256sum -c` で照合できる)。元は消していない。写し先の `README.md` に出どころと使うものを書いた。
- 置き場所の比べ方(作者の撮った画像の写し(`tests-data\truth\<tag>\image.fits` など)を含むので、リポジトリに入れない前提):

| 候補 | 判断 |
|---|---|
| リポジトリの中の .gitignore の下(`rs/target-*` など) | 採らない。`cargo clean`・`git clean -fdx`・ワークツリーの削除で消え、.gitignore を直したときに作者のデータが commit に入りうる |
| `Documents\GitHub\zahyou-testdata`(リポジトリの横) | 採らない。バックアップが無く、リポジトリを置く場所に git でないフォルダーが混ざる |
| R:(TrueNAS の `tank/TrueNAS`) | 採らない。試験が NAS につながることに依存し、R: は Nextcloud の外部ストレージとして Web からも見える |
| `%LOCALAPPDATA%\zahyou` | 採らない。アプリの記憶と設定の置き場所で、試験はここを一時へ向けて回す決まり |
| **`Documents\claude\zahyou\testdata`** | **採った**。作者の作業フォルダーの zahyou の材料(ブログの下書きなど)の横。試験の画像の決まり `$ZAHYOU_TEST_IMAGES/Documents/claude/…` がすでに同じ形で使っている(nextcloud-fitsviewer の見本の FITS)。毎日の暗号化バックアップの対象で、`.fits` と 5 MiB 超えを除く WCS・JSON・PNG(約 13 MB)が入る |

- ソースには作者のホームの絶対パスを書かない: `gen_pipeline.py` の `--oracle` の既定は `$ZAHYOU_ORACLE`、無ければ `$ZAHYOU_TEST_IMAGES`
  (無ければ `~`)の下の `Documents/claude/zahyou/testdata/oracle`。同じファイルの Python 版のリポジトリ(`$ZAHYOU_PY_REPO`、無ければ
  `~/Documents/GitHub/zahyou`)と試験画像(`$ZAHYOU_TEST_IMAGES/Downloads/…`)も同じ形にした。この PC では前と同じパスになる。
  `dev/solver_truth.json` の `sources` も同じ決まりのパスで書き、置き場所の説明(`where`)を足した。
- 確かめ: 写した oracle で `gen_pipeline.py` を一時の写しから走らせ(34 秒)、正解 WCS から作る値(`pipeline_run.json` の `wcs_cards` と
  各 run の `oracle_cards`、`pipeline_summary.json` の全部)がリポジトリの値と同じになった。`pipeline_scale.json`・`pipeline_run.json` の
  ほかの値は、Python 版(native-solver)の新しいログの文と星図データの案内の文で変わる(今回は作り直していない)。
- ~~残り: ほかの生成スクリプト・Rust の試験・`gui/test_*.py` に、作者のホームの絶対パスが既定値としてまだ約 25 か所残る。生成した
  `pipeline_run.json` の `image` も絶対パス~~ → **2026-10-18 に決着**。`gen_pipeline.py` と同じ決まり(環境変数、無ければホームフォルダー)に
  そろえた。この PC では全部前と同じパスになる。リポジトリに残る作者のホームのパス `C:\Users\<名前>` は docs の記録と、下の `image_formats.json` の 1 か所だけ:
  - Python 版のリポジトリ: `$ZAHYOU_PY_REPO`、無ければ `~/Documents/GitHub/zahyou`(`gen_net.py`・`gen_solver.py`・`app/testdata/gen_run_solver.py`、
    Rust の `zahyou-solver/tests/common` の `py_repo_dir` と `pipeline.rs` の `pyref::resolve_image`)。
  - dev フォルダー: 変数名はそのまま(`gen_index.py`・`bench_index.py` は `ZAHYOU_DEV`、`gen_plot.py`・`gen_wcs.py` は `ZAHYOU_PY_DEV`)で、
    既定を「Python 版のリポジトリ(上と同じ決まり)の下の `dev`」にした。
  - 試験の画像: `$ZAHYOU_TEST_IMAGES`(無ければホームフォルダー)の下(`gui/test_buttons.py`・`test_gui.py`・`test_result_tab.py`、
    `ui/gui_selftest.rs` の 2 つの試験は共通の `test_image`)。Rust のホームフォルダーは `USERPROFILE`(無ければ `HOME`)。
    `zahyou-solver/tests/common` の最後の逃げ道だった作者のホームのパスも外した。
  - 試験の Python(`zahyou-net/tests/common` の `python_exe`): `ZAHYOU_PYTHON`、無ければ `~/AppData/Local/Programs/Python/Python313/python.exe`、
    それも無ければ PATH の `python`。`%LOCALAPPDATA%` から組み立てないのは、試験は LOCALAPPDATA を一時へ向けて回す決まりだから。
  - `pipeline_run.json`: 作り直さず、絶対パスの 3 か所だけを書き換えた(`image` → `$ZAHYOU_TEST_IMAGES/Downloads/Capture_00001 00_10_33.fits`、
    `runs` の oracle_flip・oracle_png の `image_path` → `dev/realcase/real_flip.fits`・`dev/realcase/real.png`)。作り直すと上の確かめのとおり
    Python 版の新しいログの文でほかの値も変わるため。書き換えた 3 か所を元の値に戻すと元の JSON と同じになることを確かめた(ほかのバイトは同じ)。
    読み側(`pyref::run_images` と `run_log_vs_python` の dev/realcase の場面)は `resolve_image` を通す(前の絶対パスの JSON もそのまま開ける)。
    `gen_pipeline.py` も同じ形で書くように直した(画像が無いときの `skipped` の文も)。直した `gen_pipeline.py` を一時の写しから走らせ、
    作った `pipeline_run.json` の `image`・`image_path`・sha256・正解の WCS がリポジトリの値と同じで、作者のホームの絶対パスが 0 個なのを確かめた。
  - 説明の文: `rs/README.md`(Python の場所・`ZAHYOU_PY_REPO` の既定・照合値の作り直しの `PY=`)と `zahyou-solver/tests/README.md` の表も同じ書き方に。
  - 確かめ: 直した既定値の代入を各ファイルから ast で取り出して評価し、14 か所が前のパスと一致、環境変数で変えた 7 通りも期待どおり。
    `cargo test -p zahyou -p zahyou-net -p zahyou-solver -- --skip live` は 164 本通過・失敗 0(無視 9。11.5 節と同じ本数)。`--nocapture` で回し直し、
    画像・Python を使う試験に SKIP が無いこと(run() のログは 17 通り全部、dev/realcase の 2 場面も照合。zahyou-net は新しい既定の Python で
    http.server を立てた)を確かめた。clippy の指摘 0。`tools/test_tools.py` 15 本・`gen_notices.py --check`・`check_licenses.py` OK、
    `tools/test_make_release_assets.py` 31 本通過(`target-release` の exe を `ZAHYOU_RELEASE_EXE` で渡した)。利用者の `%LOCALAPPDATA%\zahyou` の
    cache.json・gui.json の sha256 は前後で同じ。`gui/test_*.py` は走らせていない(`test_result_tab.py` は Python 版の設定 gui.json を退避・削除・復元するため)。
  - 残すもの: `rs/testdata/image_formats.json` の 1 か所は、Pillow のエラー文に入った一時ファイルのパス(`…\AppData\Local\Temp\tmp…\not_an_image.png`)で、
    既定値ではなく Python が出した文の記録(Rust の試験は文の頭だけ比べる)。この文書の 3 節・7.5 節などに書いた既定のパスも、そのときの記録として残す。

### 11.2 zahyou-net の試験が一時フォルダーを残さない

- 試験の `temp_dir` は `%TEMP%\zahyou-net-tests\<名前>-<pid>` を作るだけで消しておらず、試験を回すたびにたまっていた(511 個・75 MB)。
- `temp_dir` は `TempDir`(`&Path` として使え、値が消えるときに中身ごと消す)を返すようにした。試験が落ちて巻き戻るときは、中を調べられるよう
  消さずに場所を出す。親の `zahyou-net-tests` は、並んで走るほかの試験とぶつからないよう消さない。
- `ZAHYOU_OFFLINE_TESTS=1` で `cargo test -p zahyou-net -- --skip live` を回し、前後でフォルダーの数が変わらない(511 → 511、空から回して 0 → 0)。
- すでに残っていた 511 個は、全部が試験の名前(`temp_dir("…")` の 28 種と、点検で一度だけ作られた `verify-v1`〜`v3`)に `-<pid>` が付いたものと
  確かめて消した(一覧は共有のノートのフォルダーの `zahyou-net-tests-leftovers-20261017.tsv`)。
- ほかの crate(zahyou-image・zahyou-solver)の試験の一時フォルダーは、cargo の `CARGO_TARGET_TMPDIR`(target の下)に固定の名前
  (`zahyou-image-tests\<sub>`・`zahyou-solver-tests\<sub>`)で作るので、回すたびに増えはしない。

### 11.3 `--selftest` は利用者の記憶(cache.json)も一時へ向ける

- 以前の `--selftest` は gui.json だけを一時へ向け、cache.json は利用者の `%LOCALAPPDATA%\zahyou\cache.json` を読み書きしていた
  (Python 版の自己診断に合わせ、解けた画素スケールを覚える形。rs/README.md に「cache.json は向けない」と書いてあった)。
  手順書は 3 つの環境変数を一時へ向けるよう書いていたが、付け忘れると、自己診断の画像の機材の画素スケールと目標の座標が利用者の記憶に足され、
  結果も記憶で変わる(覚えていれば 1.595′、空なら 1.594′。8.2 節)。手順書の書き方では足りないと判断し、exe の側で直した。
- `zahyou_net::cache::override_path`(プロセスの cache.json を差し替える。1 回だけ、ZAHYOU_CACHE より優先)を足し、`--selftest` は
  ZAHYOU_CACHE が無ければ `%TEMP%\zahyou-selftest-<pid>-cache.json`(空から)へ向けて、終わったら消す。ZAHYOU_CACHE があればそちら
  (覚えた値で確かめたいときは、利用者の cache.json の写しを指す)。画面の解析(`bridge::run` → `pipeline::run`)も `cache_path()` を通るので、
  画面の項目の解析にも効く。`--shot`・`--first-frame` は今までどおり gui.json だけを向ける。
- 試験: `app/tests/selftest_isolation.rs` が、ZAHYOU_CACHE・ZAHYOU_SETTINGS を外し、LOCALAPPDATA と TEMP を試験のフォルダーへ向けて本物の exe で
  `--selftest`(リポジトリの `app/testdata/few_two.fits`、オフライン、起動時の更新の確認はつながらないプロキシ `0.0.0.0:9` で止める)を走らせ、
  `%LOCALAPPDATA%\zahyou` ができないこと・一時の記憶が残らないことを見る。直す前の exe で落ちる(`%LOCALAPPDATA%\zahyou` を作った)ことを確かめた。
  ほかに `zahyou-net/tests/cache_override.rs`(差し替えはプロセス全体に効くので 1 本だけのファイル)と、行き先を決める関数の試験。
- 実写(`Capture_00001 00_10_33.fits`)で debug の exe を 3 つの環境変数なしで `--selftest --offline`: **25/25**・rc=0、0.916′ 東・1.304′ 北・
  1.4610″/px(手順書の「空の cache.json」の値と同じ)、一時の LOCALAPPDATA と TEMP は空のまま。
- 使われていなかった `selftest::selftest()`(画面の項目なしの入口。`#[allow(dead_code)]`)を消した。

### 11.4 文書と版の試験

- **版の試験**: `version_text` は `if 版 == "7.0.0-beta.2" { … }` の形で版の文字を持ち、版を上げて直し忘れると条件が外れて黙って通っていた。
  版の文字を試験から外し、cargo が分けた大・中・小・プレリリースと `parse_version` の読み方が同じかを見る形にした。さらに
  `version_in_docs_matches_cargo` を足した: README・`dev/make_manual.py`・`dev/nb_main.py`・`zahyou_engine.py` の「数.数.数-beta.数」をすべて拾い、
  Cargo.toml の版と違えば場所を出して落ちる(README の 1 か所を古い版にして落ちることを確かめた)。`NB_VERSION` も見る。
- `docs/release-v7.md` 2 節: 版を上げるときに直す場所を全部(Cargo.toml・Cargo.lock・`NB_VERSION` → build_nb.py・make_manual.py の 3 か所・
  README の 8 行・app-help.png の撮り直し・マニュアルの組み直し)と、直し忘れたときに何が止めるかの表にした。
- `docs/release-v7.md` 8 節の 3: 「v6 の Release 本文の `main` を `v6` に置き換える」は成り立たない(タグ `v6` = `ecf7618` の木に `docs/`・`gui/`・
  `LICENSE`・`THIRD-PARTY-NOTICES.md` が無く、本文のリンク 13 個のうち `main` を含む 12 個が全部 404 になる)。固定先を v6 の exe を建てた木と
  同じ木の `b25a42f`(40 桁の SHA。読める名前が要るなら新しいタグ)に直し、README の見出しへのリンクも固定する形にした。`3936e02` はどのブランチにも
  無いので使わない。
- `docs/release-v7.md` 0 節・6 節と、この文書の 8.4・10.10: 「深い検出の同点の順」と「Python 版の公開しない指定をあとで取り込む」は
  `7922b07`・`b878079` を `53e485e` で取り込んで済んでいたので、決着を書いた。
- `docs/release-v7.md` 4 節・`rs/README.md`: `--selftest` が cache.json も一時へ向けるようになったことを書いた(11.3 節)。
- 2 節の依存の図に `image` ← `plot` と `wcs` ← `image` を足した。`zahyou-solver/src/lib.rs` の「code_ball の問い合わせ 2048 個」を実装の
  `BALL_CHUNK` = 512 に合わせた(Python は 2048)。
- `dev/README.md`: 「この版(v7-dev)」を v7 に、マニュアルの節を v4 に(v3 の本文のままという記述と、星図データの配置の「2.3」→ v4 の 4.4)、
  同梱物の表示の節は Python の exe 用で、配る exe のものは `rs/THIRD-PARTY-NOTICES.md` だと書いた。`dev/make_index_zips.py` の説明の「2.3」も同じ。
- `gui/README.md`: 冒頭に「いま配っている v7 の exe はこのフォルダーのものではない(Rust 版は rs/)」と、ここが v6 と試作版の Python の画面の記録だと書いた。

### 11.5 試験(実測)

同じ機械(Core Ultra 7 265K)。cargo の試験は ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時のフォルダーへ向け、`ZAHYOU_OFFLINE_TESTS=1`。

| 試験 | 結果 |
|---|---|
| `cargo test -p zahyou -p zahyou-net -p zahyou-solver -- --skip live` | **164 本通過・失敗 0**(zahyou 82・zahyou-net 46・zahyou-solver 36。無視 9 本は既定どおり回していない)。1 分 31 秒 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0 |
| `tools/test_tools.py`・`tools/gen_notices.py --check`・`tools/check_licenses.py` | 15 本通過・最新(クレート 213 個)・許可リストの外なし |
| `tools/test_make_release_assets.py`(β2 の exe を `ZAHYOU_RELEASE_EXE` で渡した) | 31 本通過 |
| 試験の前後の `%TEMP%\zahyou-net-tests` | 0 個 → 0 個 |
| 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json | sha256 は作業の前後で同じ |

- 変えていない crate(zahyou-wcs・zahyou-index・zahyou-image・zahyou-plot)の試験と、`--include-ignored` の重い試験は回していない。
  exe の release ビルドはしていない(配る exe は β2 のまま)。

## 12. 2026-10-24 の保守(mmap の安全の前提)

「zahyou の unsafe を無くせるか」を調べた結果は「無くすには mmap をやめるしかない」だった。std に mmap は無く、memmap2 のファイル用の
作成関数は全部 `unsafe fn`。やめる案は 3 つあり、どれにも代償がある。

- ファイルごと読む: 4′ までのブラインドで RAM 8 GB を超えうる。
- 必要な所だけ読む: 探索の内側に読み出しが入り、並列が効きにくい。
- 上限つきの自前キャッシュ: kdtree.rs・file.rs の大半を書き直す。

外しても exe から unsafe は消えない(依存の中に残り、減る依存は memmap2 の 1 個)。そこで **外さず**、次の 3 つを入れた(ユーザーの決定)。

- **SAFETY の書き違いを直した。** 2 か所あった。
  - `IndexData` は共有モードで開いたハンドル `_file` を「Mmap と同じ寿命で持つ」と書いていた。ところが構造体のフィールドは宣言の順に
    捨てられるので、`_file` は `mmap` より先に閉じていた。守りが保たれていたのは、memmap2 が DuplicateHandle で複製したハンドル
    (同じ open を指す)を Mmap の中に持ち、UnmapViewOfFile の後で閉じるため(memmap2-0.9.11 `src/windows.rs` の `MmapInner::new` と `Drop`)。
    フィールドを `mmap` → `_file` の順に入れ替えて文字どおりにし、複製したハンドルのこともコメントに書いた。
  - 「このプロセスもこのファイルに書き込まない」は理由になっていなかった。同じ exe の zahyou-net は、落とし直しで索引を消したり `.part` へ
    移して書き足したりする(`index_dl.rs`・`http.rs`)。それでも mmap 中の索引に書けないのは、共有モードの照合がプロセスを区別しないため
    (試験 `close_releases_file` は同じプロセスからの削除と書き込み用の open が失敗することを確かめている)。コメントをそう書き直した。
- **mmap を Windows だけにした。** `map_readonly` は `#[cfg(windows)]`。ほかの OS は使用中の書き換えを止めないので、
  `IndexData::map` がビッグエンディアンのときと同じ形で `IndexError::Unsupported` を返し、表を開かない(解法では「壊れている」の側に入る)。
  `zahyou-index` の lint は、Windows では `deny(unsafe_code)`、ほかでは `forbid(unsafe_code)`。Windows での挙動は変わらない。
  試験も合わせた。星図データを使う試験は、Windows 以外では SKIP する(`zahyou-index` と `zahyou-solver` の `tests/common` と、
  解法の単体試験 `index_open_for_writing_is_not_called_broken`)。そのうえで、意味の無くなった `if cfg!(windows)` を外した。
  Windows 以外ではコンパイルして確かめていない(手元に他の OS の標準ライブラリが無い。zahyou は Windows の配布物)。
- **README に星図データの置き場所の注意を書いた。** 内蔵ディスクに置く、外付けのドライブは解析の途中で抜かない、NAS は勧めない。
  理由は次の 2 つ。どちらも一般知識で、zahyou では試していない。
  - mmap したページを読む途中でドライブが外れると、Windows は例外でプロセスを止め、`catch_unwind` では受けられない。
    まだ開いていない索引なら、開くときの OS のエラーで「今は開けない」と出て、その探索では使わずに続く。
  - ネットワーク上の置き場所では、サーバーの種類によっては、サーバー側やほかの経路からの書き換えを共有モードで止めきれない。
  ノートブック版(np.memmap)の節と、「うまくいかないとき」の表にも書いた(閉じるときの行を足し、「今は開けない」の行に外付け・NAS の場合を足した)。
  wiki(星図データの準備・ノートブック版・困ったとき)も同じに直した。

試験(実測、Core Ultra 7 265K)。cargo の試験は ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時のフォルダーへ向け、`ZAHYOU_OFFLINE_TESTS=1`。

| 試験 | 結果 |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0 |
| `cargo test --workspace -- --skip live` | **271 本通過・失敗 0**(無視 9 本は既定どおり回していない)。1 分 47 秒 |
| `close_releases_file`(星図データの実物で) | 開いている間は削除・書き込み用に開くことを拒否、`close()` とハンドルを放した後は削除できた |
| 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json | sha256 は作業の前後で同じ |

- 次の版へ回したもの(exe か配っている資産が変わるため): アプリの［使い方］タブへの注意の追記。星図データを `\\` で始まるネットワーク上の場所に
  置いたときに画面で注意を出す処理。マニュアル(v4)の 4.4 節と 8 章の表(`dev/make_manual.py`)。ノートブックのセル C の `INDEX_DIR` の説明
  (`dev/nb_settings.py`)。

## 13. 2026-10-24 の高速化(出力はビット単位で同じ)

ユーザーの依頼「コードの書き方で速くできる所を直す。安定性・安全性の低下などトレードオフが無いか調べる」で入れた。進め方は次のとおり。

1. 5 つの観点(画像・図・解法・つなぎと画面・ビルド)で候補を洗い出し、1 件ずつ反証させた。候補は 33 件。
2. 触るファイルが重ならない 5 つの組に分け、別々の worktree で入れた。
3. 各組は、旧版の式を `#[cfg(test)]` の参照実装として残し、新旧の出力が `to_bits` まで一致する試験を足した。
4. 組ごとに独立に点検させて直し、まとめたあと差分全体をもう一度点検させた。観点はつながり・ビット一致・壊れにくさ・試験の質。

決まりは次のとおり。

- Python 基準実装との一致(ビット単位)を崩さない。
- unsafe と新しい依存を足さない。
- 壊れた入力で panic しない。締め切り・中止の効き、決定性を保つ。
- 私有メモリの最大を増やさない。

### 13.1 入れたもの

- **読み込み(zahyou-image)**
  - `Loaded` の検出用の面と表示用の面を `Arc<Image2D>` で共有した。加工の無い画像では、96 MB(24 MP)の直列の写しが無くなる。
  - 0 埋めの確保を並列にした。
  - FITS の値の変換を、動的な呼び出しから BITPIX ごとの静的な閉包に変えた。
  - 非有限値の数と最小・最大を 1 回の走査で求めるようにした。
  - カラーの画像を float32 の 3 面に写さず、区切りごとに輝度にする。
  - 向きの並べ替えと RGBA→RGB を並列にした。
  - 信号の NaN は、旧版の f32→f64→f32 の往復と同じく「静か」のビットを立てる。往復をコンパイラーが省くので、整数で明示した。
- **前処理・検出(zahyou-image)**
  - σ クリップの最後の値を使い回した。
  - 背景の引き算・ホットピクセル・縁の帯の判定で、一時配列と走査を減らした。
  - ホットピクセルもマスクも無いときは、2 回目の σ クリップを飛ばす(sub が変わらないので同じ値)。
  - 検出と深い検出は、マスクが空なら写さずに借りる。
  - 書き先の配列の使い回しは、私有メモリの最大が増えたので戻した(dbd4c14)。
- **図(zahyou-plot)**
  - 伸張の範囲は、全部有限なら有限値を写さない。
  - 北が上の再投影の NaN 埋めと行の作業領域を並列にした。
  - 格子の WCS の変換を、線・枠の辺・範囲ごとにまとめた(接点の基底は 1 回だけ計算し、点ごとの式は同じ)。
- **画面(app/src/ui)**
  - 図をワーカーから画面へ写さずに移す。
  - RGBA → Color32 の詰め替えを並列にした。
  - 縮小の段を `Arc<ColorImage>` で持ち、テクスチャへ上げるときに写さない。
  - 前回の結果を持ち続けない。
  - run() で、図を描く前に前処理の結果と検出用の面を放す。
- **索引と解法(zahyou-index・zahyou-solver)**
  - 足切りの円錐を「1 つ見つけたらやめる」版にし、星の木の葉に前ふるいを入れた。
  - コード木の葉にも前ふるいを入れた。捨ててよい余裕の根拠はコメントにあり、境目の近くの問いで前ふるいの有無が一致する試験を足した。
  - 系列の仮説を、仮説そのものではなく参照で並べ替える。
  - 検証の参照星は、円錐の判定に使った単位ベクトルをそのまま使う。
  - 検証の積み上げでは、ln を μ が増えたときだけ計算する。
  - 並べ替え 2 つを不安定にした。鍵が全順序になる場合だけで、NaN があるときは安定のまま。
  - 検証の並列の仕事を 16〜31 仮説にそろえた(`with_max_len`)。

### 13.2 入れなかったもの(理由)

- **図 2 枚を並列に作る**: 24 MP で私有メモリの最大が約 +190 MB になる。
- **PGO**: 学習の経路が壊れていて、配る exe に黙って効かない。
- **JPEG の復号の並列化**: 測っておらず、壊れたファイルの扱いを変える危険がある。
- **σ クリップの読みの融合**: numpy と同じ足す順序を写した所で、得は 20〜30 ms と小さい。
- **タイルの判定のまとめ計算**: 効果は約 1% で、境目の証明が無い。
- **テクスチャを先にタイルに分ける**: 継ぎ目が出うるうえ、一時的に私有メモリが増える。
- **オンラインの判定を前倒し**: 中止しても nova へ要求が出るようになる。
- **フォントを別スレッドで読む**: `fonts_s` は約 10 ms で、効果が小さい。

### 13.3 試験(実測、Core Ultra 7 265K)

cargo の試験は ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時のフォルダーへ向け、`ZAHYOU_OFFLINE_TESTS=1` で回した。

| 試験 | 結果 |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0 |
| `cargo test --workspace -- --skip live` | **309 本通過・失敗 0**(無視 9 本)。12 節の 271 本から 38 本増えた(多くは旧版との `to_bits` の一致の試験) |
| 重い試験 `--ignored`(陰性 3・合成の全件・実写 2・app 2) | すべて通過(陰性は下の KNOWN を含む) |
| 利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json | sha256 は作業の前後で同じ |

読み込みの組は、加えて旧版と新版を release で別々に建て、51 枚の画像で data・display のハッシュが全部同じことを確かめた。51 枚の内訳は、pipeline の 37 枚・信号の NaN 入りの FITS・カラーの PNG/JPEG 13 枚・24 MP の合成。

**陰性の試験の既知の誤った解(KNOWN)**:
- `negative_real_stars_all`(締め切り 30 秒)の tcase_dense の深い星は、速くなったぶん 21 秒で誤った解に届くようになった。解は index-4112・対数オッズ 22.8・一致 5 で、仮説 16,187,319 個目。
- 変える前の版でも 31.6 秒で同じ解に届く。新旧の出力はバイト単位で同じだったので、新しい誤りではなく、締め切りが隠していたもの。
- 試験は、この解と対数オッズのビット・一致の数・仮説の番号まで同じときだけ KNOWN として数えないようにした(ff7dbda)。別の解や、仮説の番号が違う場合は FAIL のまま。
- 5 個の一致で解の判定を通ってしまう問題は、解法の課題として残る。直すと Python と答えが変わる。
- `negative_real_offline_all`(利用者の経路)には KNOWN の除外が無い。仮説 1,619 万個を約 15 秒で回せる速い機械では FAIL として出うる。
- **→ 14 節で解の判定を直し、KNOWN の除外は外した**(Rust と Python の両方。試験データの答えは変わらない)。

### 13.4 速さ(同じ機械、ほかの重い処理なし。左が main = bc1d2be、右がこの変更)

**--bench(3 回の最短)**:

| 段 | 24 MP | 968×548 の実写 |
|---|---|---|
| 読み込み | 0.103 → **0.051** | 0.004 → 0.003 |
| 前処理 | 0.487 → **0.416** | 0.036 → 0.030 |
| 検出 | 0.035 → 0.039 | 0.003 → 0.004 |
| 北が上の再投影 | 0.059 → **0.031** | 0.003 → 0.002 |
| 図 2 枚 | 0.349 → **0.224** | 0.031 → 0.025 |
| PNG 2 枚 | 0.033 → 0.033 | 0.026 → 0.027 |
| 解析全体 run() | 0.953 → **0.732** | 0.086 → **0.070**(目標なし 0.065 → 0.058) |

24 MP は `--bench` の合成画像(解けない星野。run() は画像の WCS を使う)。解法の段は、解けない探索を試し尽くすまでが 60.0 秒(締め切り)から 41.6 秒になった。

**解法(bench_solver、3 回の最短。13 件とも Python と同じ解)**:

| 場面 | 秒 |
|---|---|
| ra0 | 5.55 → **3.42**(1.63 倍) |
| tile_corner | 1.11 → **0.74**(1.50 倍) |
| polein | 1.08 → **0.71**(1.52 倍) |
| w25deg | 0.448 → 0.305 |
| many300 | 0.285 → 0.184 |
| rot090 | 0.153 → 0.111 |
| bb | 0.054 → 0.036 |
| 実写のブラインド | 0.0017 → 0.0014 |

締め切りの超過は +0.003 → +0.002 秒。

**索引(bench_index)**: 結果は全部 Python と一致。

- stars_in_cone: 4107 で 0.8 → 0.4 μs、5202-00 で 5.0 → 2.9 μs。
- code_ball(2 回目から): 4107 で 0.175 → 0.162 ms。

**図(bench_plot、24 MP)**: 伸張の範囲の値と PNG の大きさは同じ。

| 段 | 秒 |
|---|---|
| 伸張の範囲 | 0.183 → 0.144 |
| 北が上の描き直し | 0.150 → 0.095 |
| 図 2(描き直し込み) | 0.270 → 0.200 |
| 図 2 枚と PNG(通し) | 0.511 → 0.401 |

**画面の準備(--bench-gui、24 MP、3 回)**: RGBA の詰め替えと縮小の段の `pyramid_s` は 0.071 → 0.050 秒、`load_s` は 0.103 → 0.056 秒。

**私有メモリ(--bench-gui の 24 MP で、読み込み・前処理・検出・縮小の段)**: 最大の working set が 585 → 493 MB。私有メモリは 10 ms ごとに見た最大で 583 → 430〜491 MB。

- 画面の操作(図の拡大・移動の 1 フレーム、初めて原寸へ拡大したときの引っかかり)は測っていない。9 節の `--bench-ui` で測れる。
- 4〜8 スレッドのノート PC では測っていない。直列の写しを減らした変更は、スレッドが少ないほど効くはず(推測)。

## 14. 2026-10-24 の解の判定の直し(誤った解を止める 2 つの条件)

13.3 節の KNOWN(tcase_dense の深い星の誤った解)を、試験の除外ではなく解法の側で直した。Rust と Python の両方を同じ形で直した。
式と理由は設計書 3.7 節の 6。

### 14.1 何が起きていたか

- tcase_dense の深い星 400 個を全天ブラインドで解くと、仮説 16,186,850 個目(区切りの終わりで 16,187,319)の index-4112 の三角形で誤った解になった。
  4 点の検証は 18.7(一致 5・偽物 4)で、tune-up(同じ 5 個 + quad の 3 星への 1 次の当て直し)の後に 22.77 になった。新しく一致した星は 0 個だった。
- 利用者の経路(ブラインド・締め切り 60 秒)でも、比較用の画像 f-tcase-dense が同じ誤った解になる。v7.0.0 は遅いので時間内に届かず、「解なし」だった。
- 星図データを 4107〜4119 だけにすると、実写 r-oc-ser-194114 も同じ形の誤った解になる(index-4111、15.8 → 22.2、一致 4 → 4、仮説 2,663,997 個目)。
- 判定を受け入れずに最後まで探させ、4 点の検証で ln 1e6 以上になった候補 8,364 件を記録した。
  今の判定なら通ってしまう誤った候補は 5 件で、5 件とも次の点がそろっていた。
  - tune-up 経由
  - 一致の数は改良の前後で同じ(3〜5 個)
  - 仮説の番号は 266 万個より後
  - オッズは 21.4〜22.8
- astrometry.net の本家も同じ作りになっている。受け入れの条件は対数オッズだけで、一致数の下限は無く、tune-up は同じ星で採点し直す。
  比較(10-21)でも、0.98 が既定の設定で一致 4 個・21.0(index-4112)の誤答を出していた。

### 14.2 入れたもの

| 条件 | 中身 | 値(`Params` / `SvParams`) | 部品(Rust / Python) |
|---|---|---|---|
| 試した数で上げる | 最後の改良の後の対数オッズ ≥ ln 1e9 + max(0, ln(N / 1e5))。N はこの探索で、検証の区切りの終わりまでに試した仮説の数 | `accept_hyps0 = 1e5`(無限大で切れる) | `parts::accept_logodds` / `sv_accept_logodds` |
| tune-up は一致が増えたときだけ | 試した仮説が 1e5 個を超えてから tune-up で solve に届いた仮説は、最後の改良の後の一致数 ≥ 4 点の検証の一致数 + 1 | `tune_min_gain = 1`(0 で切れる)・`tune_gain_hyps0 = 1e5`(無限大で切れる) | `parts::tuneup_accepted` / `sv_tuneup_accepted` |

- 置き場所は Rust の `search.rs` の `verify_and_pick` と、Python の `_sv_verify_and_pick`。tune-up に入る範囲と、tune-up で solve に届いたかの判定は変えていない。
  そのため trace の tuneups は今までと同じになる。
- trace の params には 3 つの値が足される(試験は params を突き合わせない)。
- 直下の `zahyou_engine.py` は `dev/build_nb.py` で作り直した(違いは解法の所だけ。`zahyou_v7.ipynb` は変わらない)。

### 14.3 入れなかったもの(理由)

- **一致数の下限**: 実写に、一致 4 個・26.9 の正しい解(r-3i-ser-193002)がある。下限 5 か 6 で、この画像は解けなくなった(直接解き直して確かめた)。
- **しきい値の引き上げ(1e12 など)**: 誤った候補(最大 22.8)と、正しい解の最小(26.9)の差が 4 しか無い。1e12 では r-3i-ser-193002 が別の仮説に移り、答えが変わる。
- **tune-up をやめる**: 30° の視野では、tune-up で上がる本物の候補(一致 +15〜+103 個)が多数ある。試験の ra0 の trace(tuneups)も変わる。
- **当て直しの自由度の罰(tune-up の値から 6 を引く)**: 記録の上では同じ効き目だった。ただし偽の当て直しで +8.5 上がった例があり、余裕が 4 ほどなので、一致の数で見るほうにした。
- **値上げの起点を 1e6 にする**: 4100 系の実写の誤った解(266 万個目、要る値 21.7、実際は 22.2)が止まらない。
- **一致の増えを最初から求める**: 初めは試した数によらず求めていたが、Python の `test_solver_core.py` の実写 3I_00010 が 3 条件とも解けなくなった。
  この試験は main の検出を使うので、星は 8 個だけになる。正しい解は仮説 4,553 個目で、tune-up で 21.9・一致 4 → 4 だった
  (星が全部一致済みで、改良しても増えようがない)。そこで、求めるのは試した仮説が 1e5 個を超えてからにした。
  見つかった誤った解 5 件は、どれも 266 万個目より後なので、絞っても止まる。

### 14.4 確かめ(Core Ultra 7 265K)

cargo の試験は ZAHYOU_CACHE・ZAHYOU_SETTINGS・LOCALAPPDATA を一時のフォルダーへ向け、`ZAHYOU_OFFLINE_TESTS=1` で回した。
利用者の `%LOCALAPPDATA%\zahyou` の cache.json・gui.json は、更新時刻が作業の前(10-16・09-17)のまま。

| 試験 | 結果 |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | 指摘 0 |
| `cargo test --workspace -- --skip live` | **310 本通過・失敗 0**(無視 10 本)。13 節の 309 本から 1 本(`accept_rule_matches_python`)増えた |
| `cargo test -p zahyou-solver -- --ignored` | 7 本通過(陰性 4・実写 2・合成 1、12 分)。PASS 203 行・SKIP 0 |
| `cargo test -p zahyou -- --ignored --skip live`(`run_solver_vs_python_all` ほか) | 2 本通過 |
| `negative_dense_accept_rule`(新しい試験) | 既定・一致の増えだけ・値上げだけの 3 通りは、仮説 20,229,488 個を試し尽くして解なし(約 26 秒)。両方を外すと前と同じ誤った解(22.773709888689385・一致 5・仮説 16,187,319 個目) |
| `dev/test_solver_core.py`(部品 + 実写) | 43/43 通過(新しい case_20 を含む。3I_00010 の星 8 個も解ける) |
| `dev/test_solver_synth.py` | 63/64。ra0 は締め切り 60 秒で解けない(Python で 162 秒かかる場面。直す前の版でも同じく落ちる) |
| `dev/test_solver_real.py --engine --both`(組み立てた `zahyou_engine.py` と sv_solve 直接) | 24/24 通過(38 件は、この作業ツリーに無い dev/tcase などの画像で飛ばした) |
| 利用者の経路(比較の zbench)で比較の画像 155 枚、ブラインド・締め切り 60 秒 | 正解の 142 枚(実写 64・合成 52・形式違い 26)は、解の索引・対数オッズ・一致数・仮説の数・スケールまで直す前と同じ。陰性・解けない 13 枚は全部解なし(直す前は f-tcase-dense が誤答) |

直す前に、判定を受け入れずに最後まで探させて候補を記録した。その記録に当てはめると、4100 系だけの星図データの組(実写 66・合成 53)では、正解 31 + 39 は変わらない。誤答 1(r-oc-ser-194114)は解なしになる。

### 14.5 Python との関係

- 試験データ(`rs/testdata/solver_*.json`・`app/testdata/run_solver.json`)では、答えも trace も変わらない。
  - Python が 10 万個を超えて解くのは、高いオッズ(100 以上)の陽性だけ。
  - tune-up で解になる場面は無い(ra0 の tune-up は solve に届かない)。
  - Python の `test_solver_core.py` の 3I_00010(星 8 個)は tune-up で解けるが、仮説 1e5 個より前なので変わらない。
- そのため、基準値は作り直していない。作り直したのは `solver_parts.json` だけで、2 つの部品の値を足した(ほかの値は前と同じ)。
- 2 つの式は同じ演算の順で書いた。Rust・Python とも C の `log` を通る。試験(`accept_rule_matches_python`)は 2 ulp まで許す。

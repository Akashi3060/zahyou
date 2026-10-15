# 設計書: zahyou の Rust 版(第 2 段)

2026-10-15 作成。ブランチ `native-rs`(作業フォルダー `C:\Users\yoshi\Documents\GitHub\zahyou-rs`)。
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
  crates/zahyou-solver  解法(Python の dev/solver_core.py を移す)。Python 版が固まってから
  crates/zahyou-plot    図: 伸張、座標格子・印・矢印の表示リスト、北が上の再投影、PNG の書き出し(tiny-skia)
  crates/zahyou-net     更新の確認と入れ替え、星図データのダウンロード、天体名の解決(Sesame + VizieR)、nova.astrometry.net
  app                   eframe の画面(解析・準備・使い方の 3 タブ)と、画面を出さない入口(--selftest など)。exe 名は zahyou.exe
  testdata              Python 基準実装から作った照合用の値(JSON、小さいものだけ)と、それを作るスクリプト
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
}
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
pub fn detect_sources_deep(prep: &Prep, opts: &DetectOpts) -> (Vec<Source>, Option<f64>);   // 2×2 にまとめて検出 → (2x+0.5, 2y+0.5)
pub fn looks_inverted(data: &Image2D) -> bool;
```
`Card` は FITS のカード 1 枚(キー・値・コメント)。zahyou-wcs に置く(`pub enum CardValue { Int, Float, Str, Bool }`)。

### 4.4 zahyou-net(画面に依存しない。ブロッキングの関数を、呼ぶ側がワーカーで動かす)

```rust
pub fn check_update(current_exe: &Path, stop: &dyn Fn() -> bool) -> UpdateCheck;  // GitHub Releases latest の digest と比べる
pub fn download_update(...) / apply_update(...)    // 改名方式(実行中の exe を .old へ → .new を本名へ → 起動し直す)。PowerShell を使わない
pub fn index_names(scale: u32) -> Vec<String>; pub fn index_url(name) -> String; pub fn remote_sizes(...);
pub fn fetch_index(names, dest_dir, progress: &dyn Fn(u64, u64), stop: &dyn Fn() -> bool, log: &dyn Fn(&str)) -> FetchResult;  // Range で再開、.part → 改名、ヘッダー検査
pub fn resolve_target_name(name: &str) -> Result<(f64, f64, String), String>;   // Sesame → VizieR の Gaia DR3 で 5″ 以内の最も近い星
pub fn internet_available() -> bool;
```
v6 と同じ `%LOCALAPPDATA%\zahyou\cache.json`(targets / rigs)と `gui.json` を読み書きする(キーと形を変えない)。設定の読み書きは `app` が持つ。

## 5. 進め方

1. 並列(Python のソルバー本体を待たない): zahyou-wcs、zahyou-index、zahyou-image、zahyou-net。
2. 並列: zahyou-plot、app の殻(タブ・設定・配色・ワーカー・中止・ログ)。
3. Python 版の solver_core.py が試験に通ったら zahyou-solver(trace の JSON で 1 段ずつ突き合わせる)、続いて app に組み込む。
4. 速さの物差し `zahyou.exe --bench`(JSON)で、Python 版と同じ入力・同じ機械で比べる。

### 4.5 zahyou-solver(仮の実装を置いてある。中身は第 3 段)

```rust
pub struct FieldStar { pub x: f64, pub y: f64, pub flux: f64 }
pub struct SolveRequest { pub stars: Vec<FieldStar>, pub width, pub height, pub scale_lo: Option<f64>, pub scale_hi: Option<f64>, pub hint: Option<(f64, f64, f64)> }
pub struct Params { codetol, distractor, verify_pix, logodds_solve, logodds_tune, logodds_bail, max_quad_stars, max_verify_stars, quad_min_frac, uniformize_boxes, tweak_order }  // Python の SvParams と同じ既定値
pub struct Control<'a> { pub deadline: Option<Instant>, pub cancel: &'a AtomicBool, pub log: &'a (dyn Fn(&str) + Sync) }
pub struct Solution { pub wcs: Wcs, pub logodds, pub nmatch, pub ndistract, pub nfield, pub nref, pub index_name: String, pub quads_tried: u64, pub hypotheses: u64, pub seconds: f64 }
pub enum SolveError { Cancelled }
pub fn solve(req: &SolveRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<Option<Solution>, SolveError>;           // sv_solve
pub struct OfflineRequest<'a> { pub prep: &'a Prep, pub sources: &'a [Source], pub width, pub height, pub scale: Option<f64>, pub hint: Option<(f64, f64, f64)>, pub timeout_s: f64 }
pub fn solve_offline(req: &OfflineRequest, catalog: &Catalog, params: &Params, ctl: &Control) -> Result<Option<Solution>, SolveError>; // 試行のはしご
```

## 6. 実装の状況(2026-10-16、第 1・2 波)

- 試作 P1 の小さなアプリ(`rs/proto`)は、記録をこの設計書の 0 節に残して消した(unsafe を 2 か所含み、exe にも入らないため)。
- crate 間の API は 4 節から次の点が変わった(コードが正):
  - `zahyou_net::check_update(current_exe, version, stop)` は版名を受け取る(`-dev` で終わる版は問い合わせない)。
  - Python の `detect_sources_deep` と同じ結果にするには `DetectOpts::deep()`(min_sources 60)を渡す。
  - そのほか `IndexFile::data()`、`FetchOptions`、`zahyou_plot::RenderOptions.view`(見えている範囲のまま保存)、`View::new` が足された。
- 試験: `cargo test --workspace` で 178 本が通過(app 45 / image 30 / index 16 / net 33 / plot 29 / wcs 25)。`cargo clippy -D warnings` で指摘 0。
  自分で書いた unsafe は `zahyou-index` の mmap の 1 か所だけ。依存 213 個に GPL・LGPL・AGPL・MPL は無い。
- Python との一致: 画像の読み込み・前処理・検出は試験画像 37 枚でビット単位で一致。WCS は 1e-10 px、索引の探索は全項目一致。
- exe: 12.9 MB、依存はシステムの DLL だけ。最初の描画まで 0.36 秒(DX12)/ 0.19 秒(glow)。
- 速さ(Core Ultra 7 265K、Python → Rust): 24 MP の解析全体(解法を除く)26.8 → 1.03 秒、前処理 8.73 → 0.55 秒、検出 0.81 → 0.04 秒、
  北が上の再投影 11.0 → 0.06 秒、PNG 2 枚 5.6 → 0.04 秒。968×548 の解析全体 1.47 → 0.08 秒。
- 未了: 解法(zahyou-solver は仮の実装)。Rust 版の exe 用の同梱物の表示(THIRD-PARTY-NOTICES。egui が埋め込むフォントの OFL-1.1 と Ubuntu Font Licence を含む)。

//! 画面と解析(pipeline)・図(zahyou-plot)のつなぎ目。
//!
//! 画面のほかの部分は pipeline と zahyou-plot の型を直接触らず、ここで画面用の形
//! (ResultView・Scene)に直してから使う。pipeline や zahyou-plot の形が変わったら、ここだけを直す。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use eframe::egui::Color32;

use super::scene::{self, AxisLabel, GridSet, HAlign, Prim, Pyramid, Scene, Side, Tick, TickLabel, VAlign};
use super::settings::{Mode, Settings, Solve};
use crate::pipeline;

/// 図 1 枚(画面に描く形と、保存に使う元の図)
#[derive(Clone)]
pub struct FigureOut {
    pub scene: Arc<Scene>,
    pub source: Arc<zahyou_plot::Figure>,
}

/// 解析 1 回の結果(画面に出す形)
#[derive(Clone, Default)]
pub struct ResultView {
    pub solved: bool,
    pub cancelled: bool,
    /// 3 つの (見出し, 値)。目標があればずれ、無ければ画像中心と画素スケール
    pub numbers: Option<[(String, String); 3]>,
    /// 数字の下の 1 行
    pub detail: Option<String>,
    /// 状態バーに残す文
    pub status: String,
    /// [撮影時の向き, 北が上](目標が無いときは 1 枚)
    pub figures: Vec<FigureOut>,
}

/// 数を読む。空なら None、読めなければ None(v6 と同じく黙って既定値に戻す)。
/// Python の float() と同じく全角の数字(０〜９)も読む(全角の「．」「－」は Python も読まないので読まない)
fn num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let ascii: String = t
        .chars()
        .map(|c| match c {
            '\u{FF10}'..='\u{FF19}' => char::from(b'0' + (c as u32 - 0xFF10) as u8),
            c => c,
        })
        .collect();
    ascii.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// 正の数を読む(Python の `_num(positive=True)`)。空・読めない・0 以下なら None
fn num_positive(s: &str) -> Option<f64> {
    num(s).filter(|v| *v > 0.0)
}

/// 探索半径の欄が使えないときの知らせ(Python の _start_analysis と同じ文)。空欄は 5° で探すので問題なし
pub fn radius_problem(s: &str) -> Option<String> {
    let r = s.trim();
    if r.is_empty() || num_positive(r).is_some() {
        return None;
    }
    Some(format!("探索半径の「{r}」は使えません。\n0 より大きい数 (度) を入れてください。空欄なら 5° で探します。"))
}

/// 画面の入力から解析の条件を作る(Python の _params)。API キーは［解析］タブの欄の値をそのまま渡す
/// (空なら空のまま。pipeline は環境変数や既定のキーで埋めずに、問い合わせを飛ばす)。
pub fn params(s: &Settings) -> pipeline::RunParams {
    pipeline::RunParams {
        mode: match s.mode {
            Mode::StarName => pipeline::InputMode::StarName,
            Mode::Coords => pipeline::InputMode::Coords,
            Mode::None => pipeline::InputMode::None,
        },
        name: s.name.trim().to_string(),
        ra_str: s.ra.trim().to_string(),
        dec_str: s.dec.trim().to_string(),
        focal_mm: num(&s.focal),
        solve_mode: match s.solve {
            Solve::Auto => pipeline::SolveMode::Auto,
            Solve::Online => pipeline::SolveMode::Online,
            Solve::Offline => pipeline::SolveMode::Offline,
        },
        // v6 は int() で切り捨てていた
        online_timeout_s: num(&s.t_online).map(|v| v.trunc()).unwrap_or(120.0),
        offline_timeout_s: num(&s.t_offline).map(|v| v.trunc()).unwrap_or(300.0),
        use_target_as_hint: s.hint,
        search_radius_deg: num_positive(&s.radius).unwrap_or(5.0),
        ignore_existing_wcs: s.ignore,
        show_sources: s.marks,
        image_path: PathBuf::from(s.image.trim()),
        index_dir: PathBuf::from(if s.indexdir.trim().is_empty() { pipeline::DEFAULT_INDEX_DIR } else { s.indexdir.trim() }),
        api_key: Some(s.api_key.clone()),
    }
}

/// 「解析を実行」の前の API キーの検査: 「オンラインのみ」で欄が空なら知らせの文(始めない)。
/// 「自動」は空でも始める(問い合わせを飛ばしてオフラインで解く)、「オフラインのみ」はキーを使わない。
pub fn api_key_problem(s: &Settings) -> Option<&'static str> {
    (s.solve == Solve::Online && s.api_key.usable().is_none()).then_some(super::app::ONLINE_NEEDS_KEY)
}

/// 解析を 1 回通す(ワーカーから呼ぶ)。図は画面に描く形まで直しておく(縮小の段づくりも済ませる)。
///
/// `debug_wcs` を渡すと、オフライン解析の代わりにそのファイルの WCS を「解」として返す
/// (隠しの引数 --debug-wcs。解法と切り離して結果の画面を確かめるため)。
pub fn run(p: &pipeline::RunParams, debug_wcs: Option<&Path>, cancel: &AtomicBool, log: &(dyn Fn(&str) + Sync)) -> ResultView {
    let mut r = match debug_wcs {
        None => pipeline::run(p, cancel, log),
        Some(path) => match read_wcs_file(path) {
            Ok(wcs) => {
                let solver = move |_req: &zahyou_solver::OfflineRequest,
                                   _cat: &zahyou_index::Catalog,
                                   _par: &zahyou_solver::Params,
                                   _ctl: &zahyou_solver::Control|
                      -> Result<zahyou_solver::OfflineOutcome, zahyou_solver::SolveError> {
                    Ok(pipeline::tried(Some(debug_solution(wcs.clone()))))
                };
                let env = pipeline::RunEnv { offline_solver: Some(&solver), online: Some(false), ..Default::default() };
                let mut p2 = p.clone();
                p2.solve_mode = pipeline::SolveMode::Offline;
                log(&format!("(--debug-wcs: オフライン解析の代わりに {} の WCS を使います)", path.display()));
                pipeline::run_with(&p2, &env, cancel, log)
            }
            Err(e) => {
                log(&format!("❌ --debug-wcs の WCS を読めません: {e}"));
                pipeline::RunResult::default()
            }
        },
    };
    // 図は r から抜き出す(このあと r.figures は空。下で r から読むのは図を読まない関数だけ)
    let figures = take_figures(&mut r);
    ResultView {
        solved: r.wcs.is_some(),
        cancelled: r.cancelled,
        numbers: r.numbers(),
        detail: r.detail(),
        status: r.status_text(),
        figures,
    }
}

/// 解析の結果から図を抜き出して画面の形にする。図は写さずに移す(r.figures は空になる)。
///
/// 以前は `r.figures.iter()` で回して `Arc::new(f.clone())` で図を丸ごと写していた。rgba は幅×高さ×4 バイト
/// (24 MP で 96 MB、北が上の図は回転の外接矩形のぶん大きくなる)で、写す間は元と写しが両方メモリにあった。
/// 中身は同じ値を移すだけなので、画面の形(figure_to_scene)も保存(save_figure が読む source)もバイト単位で同じ。
/// 前提: RunResult の numbers()・detail()・status_text() は figures を読まない(pipeline.rs。offsets・summary・
/// seconds・cancelled・wcs だけ)。r から図を読む処理をこの後に足すと空になっているので、足すならこの前に置く。
/// 部分的な move(`r.figures.into_iter()`)にしないのは、そのあと &self のメソッドを呼べなくなるため。
fn take_figures(r: &mut pipeline::RunResult) -> Vec<FigureOut> {
    std::mem::take(&mut r.figures)
        .into_iter()
        .map(|f| {
            let scene = Arc::new(figure_to_scene(&f));
            FigureOut { scene, source: Arc::new(f) }
        })
        .collect()
}

/// --debug-wcs の WCS を「解」の形にする
pub fn debug_solution(wcs: zahyou_wcs::Wcs) -> zahyou_solver::Solution {
    zahyou_solver::Solution {
        wcs,
        logodds: 100.0,
        nmatch: 0,
        ndistract: 0,
        nfield: 0,
        nref: 0,
        index_name: "(--debug-wcs)".into(),
        quads_tried: 0,
        hypotheses: 0,
        seconds: 0.0,
    }
}

/// FITS のヘッダーだけのファイル(solve-field の .wcs)から WCS を読む
pub fn read_wcs_file(path: &Path) -> Result<zahyou_wcs::Wcs, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut cards = Vec::new();
    for chunk in bytes.chunks(80) {
        let s: String = chunk.iter().map(|&b| if b.is_ascii() { b as char } else { '?' }).collect();
        if s.starts_with("END") && s[3..].trim().is_empty() {
            break;
        }
        cards.push(s);
    }
    let cards = pipeline::cards_from_strings(&cards);
    zahyou_wcs::Wcs::from_header_cards(&cards)
}

fn color(c: zahyou_plot::Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn side(s: zahyou_plot::Side) -> Side {
    match s {
        zahyou_plot::Side::Bottom => Side::Bottom,
        zahyou_plot::Side::Right => Side::Right,
        zahyou_plot::Side::Top => Side::Top,
        zahyou_plot::Side::Left => Side::Left,
    }
}

fn prim(p: &zahyou_plot::Prim) -> Prim {
    use zahyou_plot::Prim as P;
    match p {
        P::Polyline { points, color: c, width_pt, dash_pt } => {
            Prim::Polyline { pts: points.clone(), color: color(*c), width: *width_pt, dash: *dash_pt }
        }
        P::Circle { center, diameter_pt, color: c, width_pt } => {
            Prim::Circle { c: *center, diameter: *diameter_pt, color: color(*c), width: *width_pt }
        }
        P::Cross { center, size_pt, color: c, width_pt } => {
            Prim::Cross { c: *center, size: *size_pt, color: color(*c), width: *width_pt }
        }
        P::Arrow { from, to, color: c, tail_width_pt, head_width_pt, head_length_pt, edge_width_pt } => Prim::Arrow {
            from: *from,
            to: *to,
            color: color(*c),
            tail: *tail_width_pt,
            head_w: *head_width_pt,
            head_l: *head_length_pt,
            edge: *edge_width_pt,
        },
        P::Text { anchor, offset_pt, text, size_pt, color: c, bold, halign, valign } => Prim::Text {
            at: *anchor,
            offset: *offset_pt,
            text: text.clone(),
            size: *size_pt,
            color: color(*c),
            bold: *bold,
            halign: match halign {
                zahyou_plot::HAlign::Left => HAlign::Left,
                zahyou_plot::HAlign::Center => HAlign::Center,
                zahyou_plot::HAlign::Right => HAlign::Right,
            },
            valign: match valign {
                zahyou_plot::VAlign::Top => VAlign::Top,
                zahyou_plot::VAlign::Center => VAlign::Center,
                zahyou_plot::VAlign::Baseline => VAlign::Baseline,
                zahyou_plot::VAlign::Bottom => VAlign::Bottom,
            },
        },
    }
}

/// 格子の線か(zahyou-plot は格子を表示リストの先頭に deepskyblue の破線で入れる)
fn is_grid_line(p: &zahyou_plot::Prim) -> bool {
    matches!(p, zahyou_plot::Prim::Polyline { color, dash_pt: Some(_), .. } if *color == zahyou_plot::style::grid_color())
}

/// zahyou-plot の格子(線・目盛り・文字)→ 画面の形
fn grid_set(g: &zahyou_plot::Grid) -> GridSet {
    let gc = zahyou_plot::style::grid_color();
    GridSet {
        lines: g
            .lines
            .iter()
            .map(|l| Prim::Polyline {
                pts: l.points.clone(),
                color: color(gc),
                width: zahyou_plot::style::GRID_WIDTH,
                dash: Some(zahyou_plot::style::GRID_DASH),
            })
            .collect(),
        ticks: g.ticks.iter().map(|t| Tick { pos: t.pos, angle_deg: t.angle_deg }).collect(),
        labels: g
            .labels
            .iter()
            .map(|l| TickLabel {
                side: side(l.side),
                pos: l.pos,
                spans: l.spans().into_iter().map(|s| (s.text, s.superscript)).collect(),
            })
            .collect(),
    }
}

/// zahyou-plot の図 → 画面に描く形(縮小の段もここで作る。ワーカーで呼ぶ)
pub fn figure_to_scene(f: &zahyou_plot::Figure) -> Scene {
    let mut s = Scene::empty();
    s.title = f.title.clone();
    s.width = f.width as f64;
    s.height = f.height as f64;
    s.flip_y = f.flip_y;
    s.image = scene::rgba_image(f.width, f.height, &f.rgba).map(Pyramid::build);
    s.axis_labels = f.axis_labels.iter().map(|a| AxisLabel { side: side(a.side), text: a.text.clone() }).collect();
    s.home = GridSet {
        lines: f.prims.iter().filter(|p| is_grid_line(p)).map(prim).collect(),
        ticks: f.ticks.iter().map(|t| Tick { pos: t.pos, angle_deg: t.angle_deg }).collect(),
        labels: f
            .tick_labels
            .iter()
            .map(|l| TickLabel {
                side: side(l.side),
                pos: l.pos,
                spans: l.spans().into_iter().map(|s| (s.text, s.superscript)).collect(),
            })
            .collect(),
    };
    s.prims = f.prims.iter().filter(|p| !is_grid_line(p)).map(prim).collect();
    // 拡大・移動したときの格子(WCSAxes と同じく、見えている範囲で目盛りを選び直す)
    let (wcs, w, h, flip) = (f.wcs.clone(), f.width, f.height, f.flip_y);
    s.regrid = Some(Arc::new(move |[x0, x1, y0, y1]: [f64; 4]| {
        if !(x0 < x1 && y0 < y1) {
            return None;
        }
        // View::new は目盛りの向きを測る幅も拡大した範囲に合わせる(View::image の x・y だけを書き換えると
        // 幅が画像全体のときのままになる)。y は (画面の下端, 上端) なので、上下反転の表示なら逆順に渡す
        let v = zahyou_plot::View::new((x0, x1), if flip { (y1, y0) } else { (y0, y1) });
        super::worker::guarded(|| grid_set(&zahyou_plot::grid_for_view(&wcs, w, h, &v))).ok()
    }));
    s
}

/// 図を PNG に保存する(暗い配色なら背景も暗いまま。v6 と同じ)。
/// `view` は拡大・移動した表示の範囲 [x の最小, x の最大, y の最小, y の最大](画像の座標)。v6 の「画像を保存」は
/// matplotlib のツールバーと同じく見えている範囲のまま保存するので、拡大しているときはその範囲を渡す(None なら全体)。
pub fn save_figure(f: &zahyou_plot::Figure, path: &Path, dark: bool, view: Option<[f64; 4]>) -> Result<(), String> {
    let opts = zahyou_plot::RenderOptions {
        theme: if dark { zahyou_plot::Theme::dark() } else { zahyou_plot::Theme::light() },
        view,
        ..Default::default()
    };
    zahyou_plot::save_png(f, &opts, path).map_err(|e| e.to_string())
}

/// 解析が開いている索引ファイルを手放させる(落とし直す前に。Windows では mmap 中のファイルを置き換えられない)
pub fn release_indexes() {
    pipeline::release_index_catalog();
}

/// 画像の縦横の画素数(準備タブの「必要な段を選ぶ」で短辺を見るため)
pub fn image_size(path: &Path) -> Result<(usize, usize), String> {
    let l = zahyou_image::load_image_any(path).map_err(|e| e.to_string())?;
    Ok((l.data.width, l.data.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 入力を解析の条件に直す() {
        let mut s = Settings {
            t_online: "90.9".into(),
            t_offline: "数字でない".into(),
            radius: "".into(),
            focal: " 800 ".into(),
            indexdir: "  ".into(),
            mode: Mode::None,
            solve: Solve::Offline,
            ..Default::default()
        };
        let p = params(&s);
        assert_eq!(p.online_timeout_s, 90.0);
        assert_eq!(p.offline_timeout_s, 300.0);
        assert_eq!(p.search_radius_deg, 5.0);
        assert_eq!(p.focal_mm, Some(800.0));
        assert_eq!(p.index_dir, PathBuf::from(pipeline::DEFAULT_INDEX_DIR));
        assert_eq!(p.mode, pipeline::InputMode::None);
        assert_eq!(p.solve_mode, pipeline::SolveMode::Offline);
        s.focal = "inf".into();
        assert_eq!(params(&s).focal_mm, None);
        // 探索半径は正の数だけ(0 以下・数でない値は 5°。画面は始める前に止める)
        for (r, want) in [("-3", 5.0), ("0", 5.0), ("abc", 5.0), ("nan", 5.0), ("2.5", 2.5), ("\u{FF13}", 3.0), ("\u{FF11}.\u{FF15}", 1.5)] {
            s.radius = r.into();
            assert_eq!(params(&s).search_radius_deg, want, "{r}");
        }
    }

    #[test]
    fn api_キーの欄の値を解析に渡す() {
        use zahyou_net::nova::{ApiKey, DEFAULT_API_KEY};
        let mut s = Settings { api_key: ApiKey::default_key(), solve: Solve::Online, ..Default::default() };
        // 値は assert! で比べる(失敗したときに値を書かない)
        let p = params(&s);
        assert!(p.api_key.as_ref().map(|k| k.expose()) == Some(DEFAULT_API_KEY), "既定のキーが渡らない");
        assert_eq!(api_key_problem(&s), None);
        // 解析の条件を Debug で書いても値は出ない
        assert!(!format!("{p:?}").contains(DEFAULT_API_KEY), "解析の条件の Debug に API キーの値が出た");
        // 空にしたら空のまま渡す(環境変数や既定のキーで埋めない)。「オンラインのみ」なら始める前に止める
        for blank in ["", "   "] {
            s.api_key = ApiKey::new(blank);
            let p = params(&s);
            assert!(p.api_key.as_ref().is_some_and(|k| k.usable().is_none()), "空の欄が空で渡らない");
            s.solve = Solve::Online;
            let m = api_key_problem(&s).expect("オンラインのみで空なのに止めない");
            assert!(m.starts_with("API キーが空です。"), "{m}");
            // 「自動」は始める(問い合わせを飛ばしてオフライン)・「オフラインのみ」はキーを使わない
            for solve in [Solve::Auto, Solve::Offline] {
                s.solve = solve;
                assert_eq!(api_key_problem(&s), None, "{solve:?}");
            }
        }
        s.api_key = ApiKey::new(" my-own-key-0123 ");
        s.solve = Solve::Online;
        assert_eq!(api_key_problem(&s), None);
        assert!(params(&s).api_key.is_some_and(|k| k.usable() == Some("my-own-key-0123")));
    }

    #[test]
    fn 探索半径の検査() {
        assert_eq!(radius_problem(""), None);
        assert_eq!(radius_problem("  "), None);
        assert_eq!(radius_problem("5"), None);
        assert_eq!(radius_problem(" 0.5 "), None);
        // 全角の数字は Python の float() と同じく読む。全角の「．」は Python も読まない
        assert_eq!(radius_problem("\u{FF15}"), None);
        assert!(radius_problem("\u{FF11}\u{FF0E}\u{FF15}").is_some());
        for r in ["-3", "0", "abc", "nan", "inf", "-0"] {
            let m = radius_problem(r).expect(r);
            assert!(m.starts_with(&format!("探索半径の「{r}」は使えません。")), "{m}");
        }
    }

    #[test]
    fn 壊れた_wcs_ファイルは_err() {
        let dir = std::env::temp_dir().join(format!("zahyou-gui-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for (name, bytes) in [
            ("empty.wcs", Vec::new()),
            ("junk.wcs", vec![0xFFu8; 1000]),
            ("half.wcs", b"SIMPLE  =                    T / x".to_vec()),
        ] {
            let p = dir.join(name);
            std::fs::write(&p, &bytes).unwrap();
            assert!(read_wcs_file(&p).is_err(), "{name}");
        }
        assert!(read_wcs_file(&dir.join("無い.wcs")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 正解の_wcs_を読める() {
        // WSL 版(astrometry.net)の正解 WCS。rs/testdata/pipeline_run.json の wcs_cards(80 文字のカード)に
        // 同じものが入っているので、それを solve-field の .wcs と同じ形(END まで、2880 バイト単位)に書いて読む
        let Some(d) = crate::pipeline::pyref::load("pipeline_run.json") else { return };
        let Some(cards) = d["wcs_cards"].as_array() else {
            eprintln!("SKIP: pipeline_run.json に wcs_cards が無い");
            return;
        };
        let mut bytes: Vec<u8> = Vec::new();
        for c in cards {
            bytes.extend_from_slice(format!("{:<80}", c.as_str().unwrap()).as_bytes());
        }
        bytes.extend_from_slice(format!("{:<80}", "END").as_bytes());
        bytes.resize(bytes.len().div_ceil(2880) * 2880, b' ');
        let dir = std::env::temp_dir().join(format!("zahyou-wcsfile-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("solution.wcs");
        std::fs::write(&p, &bytes).unwrap();
        let w = read_wcs_file(&p).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        // oracle.md: 中心 61.891414 / +41.964760、1.46161″/px
        let (ra, dec) = w.pix_to_radec(483.5, 273.5);
        assert!((ra - 61.891414).abs() < 1e-5 && (dec - 41.964760).abs() < 1e-5, "{ra} {dec}");
        assert!((w.pixel_scale_arcsec() - 1.46161).abs() < 1e-4);
    }
}

#[cfg(test)]
mod plot_tests {
    use super::*;

    /// 星を 3 つ置いた小さな画像と、TAN の WCS で図を作る
    fn small_figure() -> zahyou_plot::Figure {
        let (w, h) = (120usize, 80usize);
        let mut data = vec![10.0f32; w * h];
        for (x, y) in [(30, 20), (60, 40), (90, 60)] {
            data[y * w + x] = 200.0;
        }
        let img = zahyou_image::Image2D::new(w, h, data).unwrap();
        let s = 2.0 / 3600.0;
        let wcs = zahyou_wcs::Wcs::tan(zahyou_wcs::radec_to_xyz(61.9, 41.97), [59.5, 39.5], [[-s, 0.0], [0.0, s]]);
        zahyou_plot::figure_original(&img, &wcs, &[], Some((61.905, 41.975)), false, false).unwrap()
    }

    #[test]
    fn 図を画面の形に直す() {
        let f = small_figure();
        let s = figure_to_scene(&f);
        assert_eq!((s.width, s.height), (120.0, 80.0));
        assert!(s.image.is_some());
        assert!(!s.home.lines.is_empty(), "格子の線");
        assert!(!s.home.labels.is_empty(), "目盛りの文字");
        assert!(s.prims.iter().any(|p| matches!(p, Prim::Cross { .. })), "中心の ×");
        assert!(s.prims.iter().all(|p| !matches!(p, Prim::Polyline { dash: Some(_), .. })), "格子は prims に入れない");
        assert_eq!(s.axis_labels.len(), 2);
        // 拡大した範囲で作り直せる
        let g = (s.regrid.as_ref().unwrap())([20.0, 60.0, 10.0, 40.0]).unwrap();
        assert!(!g.lines.is_empty());
        assert!((s.regrid.as_ref().unwrap())([5.0, 5.0, 0.0, 1.0]).is_none(), "幅 0 の範囲は作らない");
    }

    /// 旧版の図の受け渡し(図を丸ごと写していた)。take_figures と同じ結果になるかを見る参照実装
    fn figures_by_clone(r: &pipeline::RunResult) -> Vec<FigureOut> {
        r.figures.iter().map(|f| FigureOut { scene: Arc::new(figure_to_scene(f)), source: Arc::new(f.clone()) }).collect()
    }

    /// 画像の段ごとの (大きさ, 画素)
    type LevelPixels = Vec<([usize; 2], Vec<Color32>)>;

    /// 画面の形の中身を比べられる形にする(画像の段の画素は別に返す。regrid は関数なので同じ範囲で呼んだ結果)
    fn scene_dump(s: &Scene) -> (String, LevelPixels) {
        let text = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            s.title,
            s.width.to_bits(),
            s.height.to_bits(),
            s.flip_y,
            s.axis_labels,
            s.home,
            s.prims,
            s.regrid.as_ref().map(|g| g([20.0, 60.0, 10.0, 40.0])),
        );
        let px = s.image.iter().flat_map(|p| p.levels.iter().map(|l| (l.size, l.pixels.clone()))).collect();
        (text, px)
    }

    #[test]
    fn 図は写さずに移す() {
        let summary = pipeline::Summary {
            center_ra: 61.9,
            center_dec: 41.97,
            scale: 2.0,
            pa_screen_up: 12.5,
            mirrored_on_screen: false,
            fov_x: 4.0,
            fov_y: 2.7,
        };
        let mut r = pipeline::RunResult {
            figures: vec![small_figure(), small_figure()],
            summary: Some(summary),
            seconds: 1.25,
            ..Default::default()
        };
        let old = figures_by_clone(&r);
        let before = (r.numbers(), r.detail(), r.status_text());
        let ptrs: Vec<*const u8> = r.figures.iter().map(|f| f.rgba.as_ptr()).collect();
        let new = take_figures(&mut r);
        assert!(r.figures.is_empty());
        // 図を抜いても数字・説明・状態の文は変わらない(どれも図を読まない)
        assert_eq!((r.numbers(), r.detail(), r.status_text()), before);
        assert!(before.0.is_some() && before.1.is_some(), "比べる値が空");
        assert_eq!(new.len(), old.len());
        for (i, (n, o)) in new.iter().zip(&old).enumerate() {
            // 写していない(rgba の領域がそのまま)
            assert_eq!(n.source.rgba.as_ptr(), ptrs[i], "図 {i} を写している");
            // 保存に使う元の図も、画面の形も、旧版とバイト単位で同じ(f64 の Debug は往復できる最短の表記)
            assert_eq!(format!("{:?}", n.source), format!("{:?}", o.source), "図 {i}");
            assert_eq!(scene_dump(&n.scene), scene_dump(&o.scene), "図 {i} の画面の形");
        }
    }

    #[test]
    fn 図を_png_に保存する() {
        let f = small_figure();
        let dir = std::env::temp_dir().join(format!("zahyou-save-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for dark in [false, true] {
            let p = dir.join(format!("fig-{dark}.png"));
            save_figure(&f, &p, dark, None).unwrap();
            let b = std::fs::read(&p).unwrap();
            assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n");
        }
        // 拡大した範囲のまま保存できる(全体を保存したものとは中身が違う)
        let full = std::fs::read(dir.join("fig-false.png")).unwrap();
        let p = dir.join("fig-zoom.png");
        save_figure(&f, &p, false, Some([20.0, 60.0, 10.0, 40.0])).unwrap();
        let zoom = std::fs::read(&p).unwrap();
        assert_eq!(&zoom[..8], b"\x89PNG\r\n\x1a\n");
        assert_ne!(zoom, full, "拡大した範囲が保存に効いていない");
        // 書けない場所は Err(panic しない)
        assert!(save_figure(&f, &dir.join("無い/フォルダー/x.png"), false, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

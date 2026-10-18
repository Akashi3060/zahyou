//! 画面の自分の撮影(--shot <png>)。試作 P1 の方式: 数フレーム描いてから egui に画面の写しを頼み、
//! 返ってきた画素を PNG に書いて閉じる。崩れ・豆腐(□)・はみ出しを目で確かめるため。
//!
//!   zahyou.exe --shot out.png [--tab run|env|help] [--out-tab orig|northup|log] [--dark|--light]
//!              [--run] [--image <画像>] [--debug-wcs <WCS>] [--size 1180x780] [--first-frame <txt>]
//!              [--fig-view x0,x1,y0,y1](図を拡大した表示で撮る)
//!
//! --run は起動後に「解析を実行」を押し、終わるのを待ってから撮る。設定(gui.json)には書かない。

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use super::app::{TopTab, ZahyouApp};
use super::settings::ResultTab;

#[derive(Clone, Debug)]
pub struct ShotPlan {
    pub path: Option<PathBuf>,
    pub tab: Option<TopTab>,
    pub out_tab: Option<ResultTab>,
    /// 解析を 1 回通してから撮る
    pub run: bool,
    /// 最初の描画までの時間を書くファイル
    pub first_frame: Option<PathBuf>,
    /// 図の表示の範囲 [x0, x1, y0, y1](拡大した表示を撮るとき)
    pub fig_view: Option<[f64; 4]>,
    started: bool,
    finished_at: Option<u64>,
    requested_at: Option<u64>,
    begun: Instant,
}

impl ShotPlan {
    pub fn new(path: Option<PathBuf>) -> ShotPlan {
        ShotPlan {
            path,
            tab: None,
            out_tab: None,
            run: false,
            first_frame: None,
            fig_view: None,
            started: false,
            finished_at: None,
            requested_at: None,
            begun: Instant::now(),
        }
    }
}

/// 1 フレーム描いたあとに呼ぶ
pub fn after_frame(app: &mut ZahyouApp, ctx: &egui::Context) {
    let Some(mut plan) = app.shot.take() else {
        return;
    };
    // 最初の描画までの時間(起動の速さの物差し)
    if app.frames == 1
        && let Some(p) = &plan.first_frame
    {
        let ms = app.first_frame_ms.unwrap_or(0.0);
        let _ = std::fs::write(p, format!("first_frame_ms={ms:.1}\nfonts={}\n", app.fonts.summary()));
        if plan.path.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
    }
    let Some(path) = plan.path.clone() else {
        app.shot = Some(plan);
        return;
    };
    ctx.request_repaint();
    // 解析を通す
    if plan.run && !plan.started && app.frames >= 3 {
        plan.started = true;
        app.start_analysis(None);
    }
    let ready = if plan.run { plan.started && !app.busy } else { app.frames >= 4 };
    if ready && plan.finished_at.is_none() {
        plan.finished_at = Some(app.frames);
        if let Some(t) = plan.tab {
            app.tab = t;
        }
        if let Some(t) = plan.out_tab {
            app.out_tab = t;
        }
        if let Some([x0, x1, y0, y1]) = plan.fig_view {
            for f in app.figs.iter_mut().flatten() {
                f.set_view(x0, x1, y0, y1);
            }
        }
    }
    // タブを替えてから数フレーム落ち着かせて撮る(テクスチャの読み込み・文字の配置)
    if let Some(f) = plan.finished_at
        && plan.requested_at.is_none()
        && app.frames >= f + 6
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        plan.requested_at = Some(app.frames);
    }
    let shots: Vec<_> = ctx.input(|i| {
        i.raw
            .events
            .iter()
            .filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
            .collect()
    });
    if let Some(img) = shots.first() {
        let [w, h] = img.size;
        let buf: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        if let Err(e) = image::save_buffer(&path, &buf, w as u32, h as u32, image::ExtendedColorType::Rgba8) {
            eprintln!("撮影した画面を書けません: {e}");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    // 10 分で諦める
    if plan.begun.elapsed().as_secs() > 600 {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    app.shot = Some(plan);
}

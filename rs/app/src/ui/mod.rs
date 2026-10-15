//! 画面(egui / eframe)。Python 版 gui/zahyou_gui.py と同じ 3 つのタブ(解析・準備・使い方)と下のバー。
//!
//! - app.rs        状態・ワーカーとの受け渡し・ボタンの処理
//! - run_tab.rs / env_tab.rs / help_tab.rs   各タブの並べ方
//! - figure.rs / scene.rs   図の表示(テクスチャ・拡大・移動・目盛り)
//! - bridge.rs     解析(pipeline)と図(zahyou-plot)とのつなぎ目
//! - survey.rs     星図データの状態調べ(準備タブ)
//! - settings.rs   gui.json(v6 と同じキー)
//! - theme.rs / style.rs / fonts.rs / widgets.rs   見た目
//! - worker.rs     重い処理のワーカー(busy・mpsc・中止・panic の受け止め)
//! - shot.rs / gui_selftest.rs / cli.rs   画面の撮影・自己診断・画面を出さない入口

pub mod app;
pub mod bridge;
pub mod cli;
pub mod env_tab;
pub mod figure;
pub mod fonts;
pub mod gui_selftest;
pub mod help_tab;
pub mod run_tab;
pub mod scene;
pub mod settings;
pub mod shot;
pub mod style;
pub mod survey;
pub mod theme;
pub mod widgets;
pub mod worker;

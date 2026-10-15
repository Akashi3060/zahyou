//! 日本語のフォント。exe には同梱せず、Windows に入っているものを読む。
//!
//! - 画面: 游ゴシック UI(YuGothM.ttc の「Yu Gothic UI Regular」→ YuGothR.ttc の 1 番 → meiryo.ttc の「Meiryo UI」)
//! - 太字: YuGothB.ttc の「Yu Gothic UI Bold」→ meiryob.ttc の「Meiryo UI Bold」→ 画面のフォント
//! - 等幅: HackGen Console / HackGen(C:\Windows\Fonts と %LOCALAPPDATA%\Microsoft\Windows\Fonts)→ Consolas
//!
//! どれも見つからなければ egui の既定のフォント(日本語は豆腐になる)のまま動く。
//! 記号と絵文字(✅ ❌ ⚠️ 🌙 ☀ など)は egui 既定の Noto Emoji が拾う。

use std::path::{Path, PathBuf};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// 太字の族の名前(`RichText::family(FontFamily::Name(BOLD.into()))`)
pub const BOLD: &str = "bold";
/// パスを書く欄の族の名前。游ゴシックなど日本語のフォントは「\」を「¥」の形で描くので、
/// 英字は Segoe UI で描き、日本語だけ游ゴシック UI に回す
pub const PATH: &str = "path";

/// どのフォントを読んだか(ログと自己診断に出す)
#[derive(Clone, Debug, Default)]
pub struct FontReport {
    pub ui: Option<String>,
    pub bold: Option<String>,
    pub mono: Option<String>,
}

impl FontReport {
    pub fn summary(&self) -> String {
        format!(
            "画面 {} / 太字 {} / 等幅 {}",
            self.ui.as_deref().unwrap_or("なし"),
            self.bold.as_deref().unwrap_or("なし"),
            self.mono.as_deref().unwrap_or("なし")
        )
    }
}

fn windows_fonts() -> PathBuf {
    let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("Fonts")
}

fn user_fonts() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Fonts"))
}

/// .ttc / .ttf の各面の「完全な名前」(name ID 4)。読めなければ空。
fn face_names(data: &[u8]) -> Vec<(u32, String)> {
    let n = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    (0..n)
        .filter_map(|i| {
            let f = ttf_parser::Face::parse(data, i).ok()?;
            let name = f
                .names()
                .into_iter()
                .filter(|nm| nm.name_id == ttf_parser::name_id::FULL_NAME && nm.is_unicode())
                .filter_map(|nm| nm.to_string())
                .find(|s| s.is_ascii())?;
            Some((i, name))
        })
        .collect()
}

/// 候補 (ファイル, 面の名前 or 面の番号) を順に試し、最初に見つかったものを返す。
enum Pick<'a> {
    Name(&'a str),
    Index(u32),
}

fn load_face(path: &Path, pick: &Pick) -> Option<(FontData, String)> {
    let data = std::fs::read(path).ok()?;
    let faces = face_names(&data);
    let (idx, name) = match pick {
        Pick::Name(want) => faces.iter().find(|(_, n)| n.eq_ignore_ascii_case(want)).cloned()?,
        Pick::Index(i) => faces.iter().find(|(j, _)| j == i).cloned()?,
    };
    let mut fd = FontData::from_owned(data);
    fd.index = idx;
    Some((fd, format!("{name} ({})", path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())))
}

fn first_of(cands: &[(PathBuf, Pick)]) -> Option<(FontData, String)> {
    cands.iter().find_map(|(p, pick)| load_face(p, pick))
}

/// HackGen の通常の太さのファイルを探す(Console を先に)。
fn find_hackgen() -> Option<PathBuf> {
    let mut dirs = vec![windows_fonts()];
    if let Some(u) = user_fonts() {
        dirs.push(u);
    }
    for want in ["HackGenConsole-Regular.ttf", "HackGen-Regular.ttf", "HackGen35Console-Regular.ttf", "HackGen35-Regular.ttf"] {
        for d in &dirs {
            let p = d.join(want);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // 名前の付け方が違う版(HackGenConsoleNF など)
    for d in &dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            let mut hits: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                    n.starts_with("hackgen") && n.contains("regular") && (n.ends_with(".ttf") || n.ends_with(".otf"))
                })
                .collect();
            hits.sort();
            if let Some(p) = hits.into_iter().next() {
                return Some(p);
            }
        }
    }
    None
}

/// フォントを読んで egui に登録する。
pub fn setup(ctx: &egui::Context) -> FontReport {
    let (defs, report) = build();
    ctx.set_fonts(defs);
    report
}

/// 登録するフォントの定義を作る(画面を出さない自己診断からも使う)。
pub fn build() -> (FontDefinitions, FontReport) {
    let fonts = windows_fonts();
    let mut defs = FontDefinitions::default();
    let mut report = FontReport::default();

    let ui = first_of(&[
        (fonts.join("YuGothM.ttc"), Pick::Name("Yu Gothic UI Regular")),
        (fonts.join("YuGothR.ttc"), Pick::Index(1)),
        (fonts.join("meiryo.ttc"), Pick::Name("Meiryo UI")),
        (fonts.join("meiryo.ttc"), Pick::Index(2)),
        (fonts.join("msgothic.ttc"), Pick::Index(1)),
    ]);
    let bold = first_of(&[
        (fonts.join("YuGothB.ttc"), Pick::Name("Yu Gothic UI Bold")),
        (fonts.join("meiryob.ttc"), Pick::Name("Meiryo UI Bold")),
    ]);
    let mono = find_hackgen()
        .and_then(|p| load_face(&p, &Pick::Index(0)))
        .or_else(|| load_face(&fonts.join("consola.ttf"), &Pick::Index(0)));

    let latin = load_face(&fonts.join("segoeui.ttf"), &Pick::Index(0));
    let defaults_prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let defaults_mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();

    let mut prop: Vec<String> = Vec::new();
    let mut monos: Vec<String> = Vec::new();
    let mut bolds: Vec<String> = Vec::new();

    if let Some((fd, name)) = mono {
        defs.font_data.insert("zh-mono".into(), fd.into());
        monos.push("zh-mono".into());
        report.mono = Some(name);
    }
    if let Some((fd, name)) = bold {
        defs.font_data.insert("zh-bold".into(), fd.into());
        bolds.push("zh-bold".into());
        report.bold = Some(name);
    }
    if let Some((fd, name)) = ui {
        defs.font_data.insert("zh-ui".into(), fd.into());
        prop.push("zh-ui".into());
        // 等幅に日本語が無いとき(Consolas)は、画面のフォントで補う
        monos.push("zh-ui".into());
        bolds.push("zh-ui".into());
        report.ui = Some(name);
    }
    let mut paths: Vec<String> = Vec::new();
    if let Some((fd, _)) = latin {
        defs.font_data.insert("zh-latin".into(), fd.into());
        paths.push("zh-latin".into());
    }
    paths.extend(prop.iter().cloned());
    paths.extend(defaults_prop.iter().cloned());
    prop.extend(defaults_prop.iter().cloned());
    monos.extend(defaults_mono.iter().cloned());
    bolds.extend(defaults_prop.iter().cloned());
    dedup(&mut prop);
    dedup(&mut monos);
    dedup(&mut bolds);
    dedup(&mut paths);
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Monospace, monos);
    defs.families.insert(FontFamily::Name(BOLD.into()), bolds);
    defs.families.insert(FontFamily::Name(PATH.into()), paths);
    (defs, report)
}

fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

/// 太字の FontId
pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// パスを書く欄の FontId(「\」が「¥」に見えないように)
pub fn path(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(PATH.into()))
}

/// Tk の「ポイント」を egui の論理画素へ(96 dpi: 1pt = 4/3 px)
pub const fn pt(p: f32) -> f32 {
    p * 4.0 / 3.0
}

//! 設定(%LOCALAPPDATA%\zahyou\gui.json)。v6 と同じキー・同じ形で読み書きする。
//!
//! - UTF-8(BOM なし)の JSON。Python の `json.dump(ensure_ascii=False, indent=1)` と同じ書き方
//! - 方法(`solve`)は画面の表示文字列のまま保存する(v6 と同じ)。旧い「オフライン (WSL) のみ」は読み替える
//! - 文字列は空でないときだけ反映する(v6 の癖をそのまま引き継ぐ: 天体名を空にして閉じると次は既定に戻る)
//! - 書けないときは黙って諦める

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::theme::ThemeName;

/// 目標の指定のしかた(gui.json の "mode")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    StarName,
    Coords,
    None,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::StarName => "STAR_NAME",
            Mode::Coords => "COORDS",
            Mode::None => "NONE",
        }
    }
    pub fn from_key(s: &str) -> Option<Mode> {
        match s {
            "STAR_NAME" => Some(Mode::StarName),
            "COORDS" => Some(Mode::Coords),
            "NONE" => Some(Mode::None),
            _ => None,
        }
    }
}

/// 解析の方法(画面の表示文字列とコード)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solve {
    Auto,
    Online,
    Offline,
}

/// 選択肢(表示の文言, 値)。並びは画面のコンボボックスの順
pub const SOLVE_CHOICES: [(&str, Solve); 3] = [
    ("自動 (インターネットがあれば使う)", Solve::Auto),
    ("オンラインのみ", Solve::Online),
    ("オフラインのみ", Solve::Offline),
];
pub const SOLVE_DEFAULT: &str = "自動 (インターネットがあれば使う)";
/// 旧い文言 → 今の文言(読み替えないと、v6 で「オフライン」を選んでいた人が黙って「自動」に戻る)
pub const SOLVE_RENAMED: [(&str, &str); 1] = [("オフライン (WSL) のみ", "オフラインのみ")];

impl Solve {
    pub fn label(self) -> &'static str {
        SOLVE_CHOICES.iter().find(|(_, s)| *s == self).map(|(l, _)| *l).unwrap_or(SOLVE_DEFAULT)
    }
    pub fn from_label(s: &str) -> Option<Solve> {
        let s = SOLVE_RENAMED.iter().find(|(old, _)| *old == s).map(|(_, new)| *new).unwrap_or(s);
        SOLVE_CHOICES.iter().find(|(l, _)| *l == s).map(|(_, v)| *v)
    }
}

/// 結果をどの表示で出すか(gui.json の "result_tab")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultTab {
    Orig,
    NorthUp,
    Log,
}

impl ResultTab {
    pub fn key(self) -> &'static str {
        match self {
            ResultTab::Orig => "orig",
            ResultTab::NorthUp => "northup",
            ResultTab::Log => "log",
        }
    }
    pub fn from_key(s: &str) -> Option<ResultTab> {
        match s {
            "orig" => Some(ResultTab::Orig),
            "northup" => Some(ResultTab::NorthUp),
            "log" => Some(ResultTab::Log),
            _ => None,
        }
    }
}

pub const DEFAULT_INDEX_DIR: &str = r"C:\AstrometryData";

/// 画面の入力欄の値(v6 の Tk 変数に当たる)。数値も文字列のまま持つ(保存も文字列)。
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub name: String,
    pub ra: String,
    pub dec: String,
    pub focal: String,
    pub solve: Solve,
    pub radius: String,
    pub t_online: String,
    pub t_offline: String,
    pub indexdir: String,
    pub sensor: String,
    pub image: String,
    pub hint: bool,
    pub marks: bool,
    pub ignore: bool,
    pub theme: ThemeName,
    pub update_auto: bool,
    pub result_tab: ResultTab,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mode: Mode::StarName,
            name: "UCAC4 660-021020".into(),
            ra: "04h 07m 38.877s".into(),
            dec: "+41d 59m 10.512s".into(),
            focal: String::new(),
            solve: Solve::Auto,
            radius: "5.0".into(),
            t_online: "120".into(),
            t_offline: "300".into(),
            indexdir: DEFAULT_INDEX_DIR.into(),
            sensor: String::new(),
            image: String::new(),
            hint: true,
            marks: true,
            ignore: true,
            theme: ThemeName::Light,
            update_auto: true,
            result_tab: ResultTab::Orig,
        }
    }
}

/// 設定ファイルの差し替え先(自己診断・画面の撮影で使う。環境変数は Rust 2024 では書き換えにくいので)
static OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// 設定ファイルを別の場所にする(起動直後に 1 回だけ)
pub fn override_path(p: PathBuf) {
    let _ = OVERRIDE.set(p);
}

/// 既定の置き場所(%LOCALAPPDATA%\zahyou\gui.json。LOCALAPPDATA が無ければ ~)。
/// 環境変数 ZAHYOU_SETTINGS で別のファイルにできる(自己診断と画面の撮影は利用者の設定に触れないため)。
pub fn settings_path() -> PathBuf {
    if let Some(p) = OVERRIDE.get() {
        return p.clone();
    }
    if let Some(p) = std::env::var_os("ZAHYOU_SETTINGS") {
        return PathBuf::from(p);
    }
    settings_dir().join("gui.json")
}

/// 設定と記憶(cache.json)の置き場所
pub fn settings_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("zahyou")
}

impl Settings {
    /// 読む。読めなければ既定値(v6 と同じく黙って)。
    pub fn load() -> Settings {
        let path = settings_path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        Settings::from_json(&text)
    }

    /// JSON の中身から作る(壊れていれば既定値)。
    pub fn from_json(text: &str) -> Settings {
        let mut s = Settings::default();
        let d: Map<String, Value> = match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
            Ok(Value::Object(m)) => m,
            _ => return s,
        };
        let get_str = |k: &str| -> Option<String> {
            match d.get(k) {
                Some(Value::String(v)) if !v.is_empty() => Some(v.clone()),
                _ => None,
            }
        };
        let get_bool = |k: &str| -> Option<bool> {
            match d.get(k) {
                Some(Value::Bool(b)) => Some(*b),
                _ => None,
            }
        };
        if let Some(m) = get_str("mode").and_then(|v| Mode::from_key(&v)) {
            s.mode = m;
        }
        for (k, slot) in [
            ("name", &mut s.name),
            ("ra", &mut s.ra),
            ("dec", &mut s.dec),
            ("focal", &mut s.focal),
            ("radius", &mut s.radius),
            ("t_online", &mut s.t_online),
            ("t_offline", &mut s.t_offline),
            ("indexdir", &mut s.indexdir),
            ("sensor", &mut s.sensor),
            ("image", &mut s.image),
        ] {
            if let Some(v) = get_str(k) {
                *slot = v;
            }
        }
        // 知らない文言は既定(自動)に戻す。旧い文言は読み替える
        if let Some(v) = get_str("solve") {
            s.solve = Solve::from_label(&v).unwrap_or(Solve::Auto);
        }
        for (k, slot) in [
            ("hint", &mut s.hint),
            ("marks", &mut s.marks),
            ("ignore", &mut s.ignore),
            ("update_auto", &mut s.update_auto),
        ] {
            if let Some(b) = get_bool(k) {
                *slot = b;
            }
        }
        if let Some(t) = get_str("theme").and_then(|v| ThemeName::from_key(&v)) {
            s.theme = t;
        }
        if let Some(t) = get_str("result_tab").and_then(|v| ResultTab::from_key(&v)) {
            s.result_tab = t;
        }
        s
    }

    /// v6 と同じ順・同じ形の JSON(indent=1、日本語はそのまま)。
    pub fn to_json(&self) -> String {
        let mut m = Map::new();
        let st = |v: &str| Value::String(v.to_string());
        m.insert("mode".into(), st(self.mode.key()));
        m.insert("name".into(), st(&self.name));
        m.insert("ra".into(), st(&self.ra));
        m.insert("dec".into(), st(&self.dec));
        m.insert("focal".into(), st(&self.focal));
        m.insert("solve".into(), st(self.solve.label()));
        m.insert("radius".into(), st(&self.radius));
        m.insert("t_online".into(), st(&self.t_online));
        m.insert("t_offline".into(), st(&self.t_offline));
        m.insert("indexdir".into(), st(&self.indexdir));
        m.insert("sensor".into(), st(&self.sensor));
        m.insert("image".into(), st(&self.image));
        m.insert("hint".into(), Value::Bool(self.hint));
        m.insert("marks".into(), Value::Bool(self.marks));
        m.insert("ignore".into(), Value::Bool(self.ignore));
        m.insert("theme".into(), st(self.theme.key()));
        m.insert("update_auto".into(), Value::Bool(self.update_auto));
        m.insert("result_tab".into(), st(self.result_tab.key()));
        let mut buf = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
        if serde::Serialize::serialize(&Value::Object(m), &mut ser).is_err() {
            return String::new();
        }
        String::from_utf8(buf).unwrap_or_default()
    }

    /// 書く。書けなければ黙って諦める(v6 と同じ)。
    pub fn save(&self) {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, self.to_json());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 旧い文言を読み替える() {
        let s = Settings::from_json(r#"{"solve": "オフライン (WSL) のみ", "mode": "NONE", "hint": false}"#);
        assert_eq!(s.solve, Solve::Offline);
        assert_eq!(s.mode, Mode::None);
        assert!(!s.hint);
        let s = Settings::from_json(r#"{"solve": "知らない文言"}"#);
        assert_eq!(s.solve, Solve::Auto);
    }

    #[test]
    fn 空の文字列は既定のまま() {
        let s = Settings::from_json(r#"{"name": "", "ra": 3}"#);
        assert_eq!(s.name, "UCAC4 660-021020");
        assert_eq!(s.ra, "04h 07m 38.877s");
    }

    #[test]
    fn 壊れた設定は既定値() {
        assert_eq!(Settings::from_json("{"), Settings::default());
        assert_eq!(Settings::from_json("[1,2]"), Settings::default());
        assert_eq!(Settings::from_json(""), Settings::default());
    }

    #[test]
    fn v6_と同じ形で書く() {
        let s = Settings::default();
        let j = s.to_json();
        // Python の json.dump(indent=1, ensure_ascii=False) と同じ
        assert!(j.starts_with("{\n \"mode\": \"STAR_NAME\",\n \"name\": \"UCAC4 660-021020\",\n"));
        assert!(j.contains("\"solve\": \"自動 (インターネットがあれば使う)\""));
        assert!(j.contains("\"indexdir\": \"C:\\\\AstrometryData\""));
        assert!(j.ends_with("\"result_tab\": \"orig\"\n}"));
        assert_eq!(Settings::from_json(&j), s);
    }
}

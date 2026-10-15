//! 星図データの状態調べ(Python の gui/zahyou_env.py の index_state / survey / report / prepare_all)。
//!
//! 画面のコードは書かない(log の関数だけ受け取る)。ダウンロードそのものは zahyou-net の fetch_index。
//! 検査は zahyou-net の軽い検査(大きさ・ENDIAN・INDEXID・HEALPIX)のあとに zahyou-index の検査
//! (解析で使う表がそろっているか)も通す。解析の目録と同じ判定になる。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zahyou_net::{human, index_names, parse_index_name, scale_fov};

/// 置き場所の走査の結果(Python の index_state)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexState {
    pub dir: PathBuf,
    /// index-*.fits の数
    pub files: usize,
    /// 合計の大きさ [byte]
    pub bytes: u64,
    /// ファイルが全部そろっている段(survey のあとは「検査にも通った段」)
    pub scales: Vec<u32>,
    /// 一部だけある段 {段: ある数}
    pub partial: BTreeMap<u32, usize>,
    /// ファイルがある段の画角の範囲 [分角]
    pub fov: Option<(f64, f64)>,
    pub ok: bool,
    pub names: Vec<String>,
}

/// 検査の結果
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckState {
    pub ok: bool,
    pub checked: usize,
    pub bad: Vec<(String, String)>,
}

/// survey の結果(画面の表とログに出す)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Survey {
    pub index: IndexState,
    pub check: CheckState,
    /// オフライン解析できるか(検査に通った星図データが 1 本でもある)
    pub ready: bool,
    pub notes: Vec<String>,
}

/// 置き場所を走査する(中身は見ない)。
pub fn index_state(index_dir: &Path) -> IndexState {
    let mut st = IndexState { dir: index_dir.to_path_buf(), ..Default::default() };
    let Ok(rd) = std::fs::read_dir(index_dir) else {
        return st;
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            parse_index_name(&n).map(|_| n)
        })
        .collect();
    names.sort();
    let mut count: BTreeMap<u32, usize> = BTreeMap::new();
    for name in &names {
        st.files += 1;
        st.bytes += std::fs::metadata(index_dir.join(name)).map(|m| m.len()).unwrap_or(0);
        if let Some((s, _)) = parse_index_name(name) {
            *count.entry(s).or_insert(0) += 1;
        }
    }
    st.names = names;
    st.scales = count.iter().filter(|(s, n)| **n >= index_names(**s).len()).map(|(s, _)| *s).collect();
    st.partial = count.iter().filter(|(s, n)| **n < index_names(**s).len()).map(|(s, n)| (*s, *n)).collect();
    let covered: Vec<(f64, f64)> = count.keys().filter_map(|s| scale_fov(*s)).collect();
    if !covered.is_empty() {
        let lo = covered.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        let hi = covered.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max);
        st.fov = Some((lo, hi));
    }
    st.ok = st.files > 0;
    st
}

/// 1 本の検査(軽い検査 + zahyou-index の検査)。同じファイルは大きさと更新時刻が変わるまで調べ直さない。
pub fn check_file(path: &Path) -> Result<(), String> {
    zahyou_net::check_index_file_full(path)
}

/// 星図データを走査して、1 本ずつ検査する。
pub fn survey(index_dir: &Path) -> Survey {
    let mut idx = index_state(index_dir);
    // 253 本でも速い(ヘッダーだけ)が、並列にしておく
    use rayon::prelude::*;
    let bad: Vec<(String, String)> = idx
        .names
        .par_iter()
        .filter_map(|name| check_file(&index_dir.join(name)).err().map(|why| (name.clone(), why)))
        .collect();
    let bad_scales: Vec<u32> = bad.iter().filter_map(|(n, _)| parse_index_name(n).map(|p| p.0)).collect();
    idx.scales.retain(|s| !bad_scales.contains(s));
    let good = idx.files - bad.len();
    let mut notes = Vec::new();
    if !idx.ok {
        notes.push(format!("{} に星図データ (index-*.fits) がありません。", index_dir.display()));
    }
    for (s, n) in &idx.partial {
        notes.push(format!(
            "index-{s} は {} ファイル中 {n} しかありません (落とすのを途中で止めたもの)。もう一度落とせば続きから補います。",
            index_names(*s).len()
        ));
    }
    if !bad.is_empty() {
        notes.push(format!(
            "中身のおかしい星図データが {} 個あります (解析では読み飛ばします)。その段を落とし直してください。",
            bad.len()
        ));
        for (name, why) in bad.iter().take(5) {
            notes.push(format!("  {name}: {why}"));
        }
        if bad.len() > 5 {
            notes.push(format!("  ほか {} 個", bad.len() - 5));
        }
    }
    let check = CheckState { ok: idx.files > 0 && bad.is_empty(), checked: idx.files, bad };
    Survey { index: idx, check, ready: good > 0, notes }
}

/// 画角の範囲を Python の `{x:g}` と同じ形で(4.0 → "4"、2.8 → "2.8")
pub fn g(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        let s = format!("{x}");
        s
    }
}

/// survey の結果をログに書く(Python の report)。
pub fn report(st: &Survey, log: &dyn Fn(&str)) {
    let idx = &st.index;
    let chk = &st.check;
    if idx.ok {
        log(&format!("  星図データ : {} ファイル / {}  ({})", idx.files, human(idx.bytes as f64), idx.dir.display()));
    } else {
        log(&format!("  星図データ : ありません  ({})", idx.dir.display()));
    }
    if chk.checked > 0 {
        if chk.bad.is_empty() {
            log(&format!("  検査       : {} ファイルとも正常", chk.checked));
        } else {
            log(&format!("  検査       : {} / {} ファイルに問題", chk.bad.len(), chk.checked));
        }
    }
    if let Some((lo, hi)) = idx.fov {
        log(&format!("  解ける画角 : {}′ 〜 {}′", g(lo), g(hi)));
    }
    for m in &st.notes {
        log(&format!("  ⚠️ {m}"));
    }
}

/// 段の一覧をログ用に「[5202, 5203]」の形で(Python の list の repr と同じ)
pub fn list_repr(v: &[u32]) -> String {
    let inner: Vec<String> = v.iter().map(|s| s.to_string()).collect();
    format!("[{}]", inner.join(", "))
}

/// 段 → 落とすファイル名
pub fn names_of(scales: &[u32]) -> Vec<String> {
    scales.iter().flat_map(|s| index_names(*s)).collect()
}

/// 「まとめて準備」。足りない段を落として、検査する(Python の prepare_all)。戻り値は survey。
pub fn prepare_all(
    index_dir: &Path,
    scales: &[u32],
    log: &dyn Fn(&str),
    progress: &dyn Fn(u64, u64),
    stop: &dyn Fn() -> bool,
) -> Survey {
    let sep = "=".repeat(56);
    log(&sep);
    log("1. 星図データ");
    let st = survey(index_dir);
    let mut want: Vec<u32> = if scales.is_empty() { zahyou_net::recommended_scales() } else { scales.to_vec() };
    want.sort_unstable();
    let need: Vec<u32> = want.iter().copied().filter(|s| !st.index.scales.contains(s)).collect();
    if !need.is_empty() {
        let list: Vec<String> = need.iter().map(|s| s.to_string()).collect();
        log(&format!("  足りない段: {}", list.join(", ")));
        let r = zahyou_net::fetch_index(&names_of(&need), index_dir, progress, stop, log);
        if !r.ok {
            let st = survey(index_dir);
            report(&st, log);
            return st;
        }
    } else {
        let idx = &st.index;
        log(&format!("  OK  {} ファイル / {}", idx.files, human(idx.bytes as f64)));
    }
    log(&sep);
    log("2. 検査");
    let st = survey(index_dir);
    report(&st, log);
    if st.ready {
        log("  準備できました。オフライン解析できます。");
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 画角の書き方() {
        assert_eq!(g(4.0), "4");
        assert_eq!(g(2.8), "2.8");
        assert_eq!(g(2000.0), "2000");
        assert_eq!(list_repr(&[5202, 5203]), "[5202, 5203]");
        assert_eq!(list_repr(&[]), "[]");
    }

    #[test]
    fn 無い置き場所() {
        let st = survey(Path::new(r"C:\no\such\dir\zahyou-test"));
        assert!(!st.ready);
        assert!(!st.index.ok);
        assert_eq!(st.notes.len(), 1);
    }

    #[test]
    fn 手元の星図データ() {
        let dir = Path::new(r"C:\AstrometryData");
        if !dir.is_dir() {
            eprintln!("SKIP: C:\\AstrometryData が無い");
            return;
        }
        let st = survey(dir);
        eprintln!("{} ファイル / ready={} / 画角 {:?}", st.index.files, st.ready, st.index.fov);
        assert!(st.index.files > 0);
    }
}

#[cfg(test)]
mod broken_tests {
    use super::*;

    #[test]
    fn 途中の段と壊れたファイル() {
        let dir = std::env::temp_dir().join(format!("zahyou-survey-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 5206 の 48 タイルのうち 2 つだけ(しかも中身は壊れている)、4107 は 1 本で揃うが中身が壊れている
        std::fs::write(dir.join("index-5206-00.fits"), b"not a fits").unwrap();
        std::fs::write(dir.join("index-5206-01.fits"), vec![b' '; 2880]).unwrap();
        std::fs::write(dir.join("index-4107.fits"), Vec::<u8>::new()).unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        let st = survey(&dir);
        assert_eq!(st.index.files, 3);
        assert_eq!(st.index.partial.get(&5206), Some(&2));
        assert!(st.index.scales.is_empty(), "壊れた段は「そろった段」に入れない: {:?}", st.index.scales);
        assert_eq!(st.check.bad.len(), 3);
        assert!(!st.ready);
        assert!(st.notes.iter().any(|n| n.contains("48 ファイル中 2")));
        assert!(st.notes.iter().any(|n| n.contains("中身のおかしい星図データが 3 個")));
        let lines = std::sync::Mutex::new(Vec::new());
        report(&st, &|l| lines.lock().unwrap().push(l.to_string()));
        assert!(lines.lock().unwrap().iter().any(|l| l.contains("3 / 3 ファイルに問題")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

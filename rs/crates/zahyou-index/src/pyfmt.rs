//! Python 版と同じ文面のメッセージを作るための書式(`repr`・`{:,}`・`str.strip`)。
//!
//! 検査のメッセージは画面にそのまま出るので、Python 版 `dev/solver_index.py` と同じ文字列にする
//! (試験で Python の出力と突き合わせる)。

/// Python の `str.strip()` が取り除く空白(ASCII の範囲。ヘッダーは ASCII + U+FFFD だけなので十分)。
pub(crate) fn is_py_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\x1f')
}

/// Python の `str.strip()`(引数なし)。Rust の `trim` は \x1c〜\x1f を残すので別に書く。
pub(crate) fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Python の `str.rstrip()`。
pub(crate) fn py_rstrip(s: &str) -> &str {
    s.trim_end_matches(is_py_space)
}

/// 整数を 3 桁ごとのカンマで区切る(Python の `f"{n:,}"`)。
pub(crate) fn commas(n: i128) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    let first = digits.len() % 3;
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (i + 3 - first).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 実数の `repr`(Python 3 の `str(float)` と同じ。最短で元に戻る桁、指数が −4 未満か 16 以上なら指数表記)。
pub(crate) fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    // Rust の {:e} は最短で元に戻る仮数を出す(例 "1.5e16"、"-2e-5"、"0e0")
    let s = format!("{:e}", x);
    let (mant, exp) = match s.split_once('e') {
        Some(p) => p,
        None => return s,
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let (neg, mant) = match mant.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mant),
    };
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-4..16).contains(&exp) {
        // 固定小数点。decpt = 小数点の位置(先頭の桁の前からの桁数)
        let decpt = exp + 1;
        let nd = digits.len() as i32;
        if decpt <= 0 {
            out.push_str("0.");
            for _ in 0..(-decpt) {
                out.push('0');
            }
            out.push_str(&digits);
        } else if decpt >= nd {
            out.push_str(&digits);
            for _ in 0..(decpt - nd) {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            out.push_str(&digits[..decpt as usize]);
            out.push('.');
            out.push_str(&digits[decpt as usize..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.unsigned_abs()));
    }
    out
}

/// 文字列の `repr`(Python と同じ引用符の選び方とエスケープ。ASCII と U+FFFD の範囲で正しい)。
pub(crate) fn str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_like_formats() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1000), "1,000");
        assert_eq!(commas(4454259), "4,454,259");
        assert_eq!(commas(-1234567), "-1,234,567");
        // Python の repr(float) と同じ
        for (x, want) in [
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (-0.0, "-0.0"),
            (0.0063995, "0.0063995"),
            (2147483647.5, "2147483647.5"),
            (std::f64::consts::PI, "3.141592653589793"),
            (1.5e-300, "1.5e-300"),
        ] {
            assert_eq!(float_repr(x), want, "{x}");
        }
        assert_eq!(str_repr("abc"), "'abc'");
        assert_eq!(str_repr("a'b"), "\"a'b\"");
        assert_eq!(str_repr("a'b\""), "'a\\'b\"'");
        assert_eq!(str_repr("\u{0}x\u{fffd}"), "'\\x00x\u{fffd}'");
        assert_eq!(py_strip("\x1c ab \x1f"), "ab");
    }
}

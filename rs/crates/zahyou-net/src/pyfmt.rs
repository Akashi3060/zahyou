//! Python と同じ文字列を作るための小道具。
//!
//! cache.json(`json.dump(..., ensure_ascii=False, indent=1)`)と、nova.astrometry.net へ送る
//! `request-json`(`json.dumps(settings)` を `urlencode` したもの)を Python 版とバイト単位で同じにする。
//! 実数は Python の `repr(float)` と同じ書き方(最短で元に戻る桁、指数は `1e-05` / `1e+16` の形)。

use serde_json::Value;

/// Python の `repr(x)`(float)と同じ文字列。NaN・無限大は JSON と同じ `NaN` / `Infinity` / `-Infinity`。
pub fn py_float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let (digits, exp) = shortest_digits(x);
    let sign = if x < 0.0 { "-" } else { "" };
    // Python は 10 進の指数が -4 未満か 16 以上のとき指数表記にする
    if !(-4..16).contains(&exp) {
        let mant = if digits.len() == 1 { digits.clone() } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        let esign = if exp < 0 { '-' } else { '+' };
        return format!("{sign}{mant}e{esign}{:02}", exp.abs());
    }
    if exp < 0 {
        return format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize));
    }
    let k = exp as usize + 1; // 整数部の桁数
    if digits.len() <= k {
        format!("{sign}{digits}{}.0", "0".repeat(k - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..k], &digits[k..])
    }
}

/// 元に戻る最短の 10 進の桁(先頭が 0 でない数字の列)と、10 進の指数(値 = d.ddd × 10^指数)。
///
/// Python(David Gay の dtoa のモード 0)は、同じ桁数で元に戻る候補が 2 つあるとき真の値に近いほうを、
/// ちょうど中間なら偶数のほうを選ぶ。Rust の `{:e}` は最短の桁数は同じだが、中間のときに上へ丸めることがある
/// (例 -19306645364580.8125 → Python は …580.812、`{:e}` は …580.813)。そこで桁数だけ `{:e}` から取り、
/// その桁数で正しく丸めた値(`{:.N e}` は真の値から偶数丸め)が元に戻るならそちらを使う。
fn shortest_digits(x: f64) -> (String, i32) {
    let split = |sci: &str| -> (String, i32) {
        let (m, e) = sci.split_once('e').unwrap_or((sci, "0"));
        let d: String = m.chars().filter(|c| c.is_ascii_digit()).collect();
        (d, e.parse::<i32>().unwrap_or(0))
    };
    let ax = x.abs();
    let (short, e_short) = split(&format!("{ax:e}"));
    let n = short.len().max(1);
    let exact = format!("{ax:.*e}", n - 1);
    if exact.parse::<f64>().ok() == Some(ax) {
        let (d, e) = split(&exact);
        // 末尾の 0 は落とす(`{:.N e}` は桁数ぶん埋めるので)
        let d = d.trim_end_matches('0');
        if !d.is_empty() {
            return (d.to_string(), e);
        }
    }
    (short, e_short)
}

/// Python の `json.dumps` と同じ書き方で JSON を書く。
///
/// - `indent = None`: 区切りは `", "` と `": "`(json.dumps の既定)
/// - `indent = Some(n)`: 改行して n 文字ずつ字下げ、区切りは `","` と `": "`(Python 3.4 以降の既定)
/// - `ensure_ascii`: 真なら ASCII 以外を `\uXXXX`(BMP の外はサロゲート対)にする
pub fn py_json(v: &Value, indent: Option<usize>, ensure_ascii: bool) -> String {
    let mut out = String::new();
    write_value(v, indent, ensure_ascii, 0, &mut out);
    out
}

fn write_value(v: &Value, indent: Option<usize>, ascii: bool, level: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                out.push_str(&py_float_repr(n.as_f64().unwrap_or(f64::NAN)));
            }
        }
        Value::String(s) => write_str(s, ascii, out),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                sep(i, indent, level + 1, out);
                write_value(item, indent, ascii, level + 1, out);
            }
            close(indent, level, out);
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in m.iter().enumerate() {
                sep(i, indent, level + 1, out);
                write_str(k, ascii, out);
                out.push_str(": ");
                write_value(item, indent, ascii, level + 1, out);
            }
            close(indent, level, out);
            out.push('}');
        }
    }
}

fn sep(i: usize, indent: Option<usize>, level: usize, out: &mut String) {
    match indent {
        None => {
            if i > 0 {
                out.push_str(", ");
            }
        }
        Some(n) => {
            if i > 0 {
                out.push(',');
            }
            out.push('\n');
            out.push_str(&" ".repeat(n * level));
        }
    }
}

fn close(indent: Option<usize>, level: usize, out: &mut String) {
    if let Some(n) = indent {
        out.push('\n');
        out.push_str(&" ".repeat(n * level));
    }
}

/// Python の `json.encoder` と同じ文字列の書き方。
fn write_str(s: &str, ascii: bool, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            // ensure_ascii=True は ' '〜'~' の外を全部 \uXXXX にする(DEL も)
            c if ascii && !(' '..='~').contains(&c) => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `urllib.parse.quote(s, safe=safe)`(UTF-8)。英数字と `_.-~` と `safe` の文字はそのまま。
pub fn py_quote(s: &str, safe: &str) -> String {
    quote_impl(s, safe, false)
}

/// `urllib.parse.quote_plus(s)`(UTF-8、safe='')。空白は `+`。
pub fn py_quote_plus(s: &str) -> String {
    quote_impl(s, "", true)
}

fn quote_impl(s: &str, safe: &str, plus: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        let c = b as char;
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || (b.is_ascii() && safe.contains(c)) {
            out.push(c);
        } else if plus && b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `urllib.parse.urlencode(pairs)`(requests が `data=dict` を送るときの本文と同じ)。
pub fn py_urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", py_quote_plus(k), py_quote_plus(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// 整数部を 3 桁ごとに `,` で区切る(Python の `f"{n:,}"` / `f"{x:,.1f}"` と同じ)。
pub fn group_thousands(s: &str) -> String {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s),
    };
    let (int, frac) = match rest.find('.') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{sign}{grouped}{frac}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // …580.8125 はちょうど表せる値(同点の丸めを見るため、わざと全部の桁を書く)
    #[allow(clippy::excessive_precision)]
    fn float_repr_basic() {
        assert_eq!(py_float_repr(1.0), "1.0");
        assert_eq!(py_float_repr(0.1), "0.1");
        assert_eq!(py_float_repr(1e16), "1e+16");
        assert_eq!(py_float_repr(1e-5), "1e-05");
        assert_eq!(py_float_repr(0.0001), "0.0001");
        assert_eq!(py_float_repr(1234567890123456.0), "1234567890123456.0");
        assert_eq!(py_float_repr(-2.5e-7), "-2.5e-07");
        assert_eq!(py_float_repr(1.5e300), "1.5e+300");
        assert_eq!(py_float_repr(-0.0), "-0.0");
        // ちょうど中間(…580.8125)は偶数のほうへ(Python と同じ。`{:e}` は …813 にする)
        assert_eq!(py_float_repr(-19306645364580.8125), "-19306645364580.812");
        assert_eq!(py_float_repr(5e-324), "5e-324");
        assert_eq!(py_float_repr(123456.789), "123456.789");
        assert_eq!(py_float_repr(0.000123), "0.000123");
    }

    #[test]
    fn quote_forms() {
        assert_eq!(py_quote("UCAC4 660-021020", "/"), "UCAC4%20660-021020");
        assert_eq!(py_quote_plus("a b/c"), "a+b%2Fc");
        assert_eq!(py_quote("ベガ", "/"), "%E3%83%99%E3%82%AC");
    }

    #[test]
    fn thousands() {
        assert_eq!(group_thousands("1023.9"), "1,023.9");
        assert_eq!(group_thousands("999.0"), "999.0");
        assert_eq!(group_thousands("1234567"), "1,234,567");
        assert_eq!(group_thousands("-1000.5"), "-1,000.5");
    }
}

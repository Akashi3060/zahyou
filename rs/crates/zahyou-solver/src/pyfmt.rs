//! Python の書式(ログの文言と trace の値を Python 版とそろえるため)。

/// `f"{v:,}"`(3 桁ごとのカンマ)。
pub(crate) fn fmt_int(v: u64) -> String {
    let s = v.to_string();
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*c as char);
    }
    out
}

/// 小数点以下の末尾の 0(と小数点そのもの)を落とす。
fn strip_zeros(s: &str) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() }
}

/// `f"{v:.{prec}g}"`(有効数字 prec 桁。指数が −4 未満か prec 以上なら指数表記、末尾の 0 は落とす)。
pub(crate) fn fmt_g(v: f64, prec: usize) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".to_string() } else { "0".to_string() };
    }
    let p = prec.max(1);
    let e = format!("{:.*e}", p - 1, v);
    let Some((mant, exp)) = e.split_once('e') else { return e };
    let exp: i32 = exp.parse().unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
        format!("{}e{}{:02}", strip_zeros(mant), if exp < 0 { '-' } else { '+' }, exp.unsigned_abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        strip_zeros(&format!("{v:.decimals$}"))
    }
}

/// Python の `repr(float)` / `str(float)`(最短で戻る桁。指数が −4 未満か 16 以上なら指数表記)。
pub(crate) fn float_repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".to_string() } else { "0.0".to_string() };
    }
    // 最短で戻る桁(Rust の {:e} は最短の桁を指数表記で出す)
    let e = format!("{v:e}");
    let Some((mant, exp)) = e.split_once('e') else { return e };
    let exp: i32 = exp.parse().unwrap_or(0);
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let sign = if neg { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        if exp < 0 {
            format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize))
        } else if exp + 1 >= n {
            format!("{sign}{digits}{}.0", "0".repeat((exp + 1 - n) as usize))
        } else {
            let (a, b) = digits.split_at((exp + 1) as usize);
            format!("{sign}{a}.{b}")
        }
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        format!("{sign}{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.unsigned_abs())
    }
}

/// Python の `round(v, nd)`(10 進で最も近い値。ちょうど半分は偶数側)。
pub(crate) fn py_round(v: f64, nd: usize) -> f64 {
    if !v.is_finite() {
        return v;
    }
    format!("{v:.nd$}").parse::<f64>().unwrap_or(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_python() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(999), "999");
        assert_eq!(fmt_int(1000), "1,000");
        assert_eq!(fmt_int(1234567), "1,234,567");
        // f"{v:.3g}"
        assert_eq!(fmt_g(0.7772951539838913, 3), "0.777");
        assert_eq!(fmt_g(1.2954919233064854, 3), "1.3");
        assert_eq!(fmt_g(3600.0, 3), "3.6e+03");
        assert_eq!(fmt_g(0.00001234, 3), "1.23e-05");
        assert_eq!(fmt_g(0.0001234, 3), "0.000123");
        assert_eq!(fmt_g(123.0, 3), "123");
        assert_eq!(fmt_g(999.6, 3), "1e+03");
        assert_eq!(fmt_g(5.0, 6), "5");
        assert_eq!(fmt_g(2.5, 6), "2.5");
        assert_eq!(fmt_g(f64::INFINITY, 3), "inf");
        // repr
        assert_eq!(float_repr(75.0), "75.0");
        assert_eq!(float_repr(-1.0), "-1.0");
        assert_eq!(float_repr(0.1), "0.1");
        assert_eq!(float_repr(1e-5), "1e-05");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(1234567890123456.0), "1234567890123456.0");
        assert_eq!(float_repr(1.5e300), "1.5e+300");
        assert_eq!(float_repr(145.0), "145.0");
        assert_eq!(float_repr(f64::NAN), "nan");
        // round
        assert_eq!(py_round(1.23456789, 4), 1.2346);
        assert_eq!(py_round(0.125, 2), 0.12);
    }
}

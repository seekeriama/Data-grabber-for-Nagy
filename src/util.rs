//! Các hàm nhỏ bắt chước hành vi VB.NET của chương trình gốc.

/// Giống `Microsoft.VisualBasic.Conversion.Val`: đọc số ở đầu chuỗi, bỏ qua
/// khoảng trắng / tab / xuống dòng, dừng ở ký tự đầu tiên không phải số.
/// "R= 17.42 Ohm" -> (sau khi tách "=") " 17.42 Ohm" -> 17.42 ; "+4.53E+01" -> 45.3
pub fn vb_val(s: &str) -> f64 {
    let mut out = String::new();
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_exp = false;
    let mut exp_sign_ok = false;
    for c in s.chars() {
        if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
            continue;
        }
        match c {
            '+' | '-' if out.is_empty() => out.push(c),
            '+' | '-' if exp_sign_ok => {
                out.push(c);
                exp_sign_ok = false;
            }
            '0'..='9' => {
                out.push(c);
                seen_digit = true;
                exp_sign_ok = false;
            }
            '.' if !seen_dot && !seen_exp => {
                out.push(c);
                seen_dot = true;
            }
            'e' | 'E' | 'd' | 'D' if seen_digit && !seen_exp => {
                out.push('e');
                seen_exp = true;
                exp_sign_ok = true;
            }
            _ => break,
        }
    }
    // Bỏ phần mũ dở dang ("1.5e" / "1.5e-")
    while out.ends_with('e') || out.ends_with('+') || out.ends_with('-') {
        out.pop();
    }
    out.parse::<f64>().unwrap_or(0.0)
}

/// Định dạng số thực như `Double.ToString()` của .NET Framework (15 chữ số có nghĩa).
/// 17.42 + (-0.4) = 17.020000000000003 -> "17.02"
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return if v.is_nan() { "NaN".into() } else if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v == 0.0 {
        return "0".into();
    }
    let rounded: f64 = format!("{:.14e}", v).parse().unwrap_or(v);
    let abs = rounded.abs();
    if (1e-5..1e15).contains(&abs) {
        format!("{}", rounded)
    } else {
        // .NET dùng dạng mũ cho số rất nhỏ / rất lớn: 1E-07, 1.5E+16
        let s = format!("{:e}", rounded); // "1e-7" / "1.5e16"
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        format!("{}E{}{:02}", m, if e < 0 { '-' } else { '+' }, e.abs())
    }
}

/// `Math.Round(Decimal)` của .NET: làm tròn về số nguyên gần nhất, 0.5 -> số chẵn.
pub fn round_half_even(v: f64) -> f64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 {
        2.0 * (v / 2.0).round()
    } else {
        r
    }
}

/// Thời gian đo theo đúng định dạng file mẫu: 17/10/2026 19:46:33
pub fn fmt_time(t: &chrono::NaiveDateTime) -> String {
    t.format("%d/%m/%Y %H:%M:%S").to_string()
}

/// Hiển thị điện trở có tiền tố SI (giống OhmsBoxDisplay của bản 4PP gốc).
pub fn fmt_ohms_si(v: f64) -> String {
    // Keithley trả 9.91E+37 khi hở mạch (đầu đo đang nhấc)
    if v.abs() >= 1e30 {
        return "OVERFLOW".into();
    }
    if !v.is_finite() || v == 0.0 {
        return format!("{} Ohms", fmt_num(if v.is_finite() { v } else { 0.0 }));
    }
    let prefixes = [(1e9, "G"), (1e6, "M"), (1e3, "k"), (1.0, ""), (1e-3, "m"), (1e-6, "u"), (1e-9, "n")];
    let a = v.abs();
    for (scale, p) in prefixes {
        if a >= scale {
            return format!("{} {}Ohms", fmt_num((v / scale * 100.0).round() / 100.0), p);
        }
    }
    format!("{} nOhms", fmt_num((v / 1e-9 * 100.0).round() / 100.0))
}

/// Làm tròn 2 chữ số như `Math.Round(x, 2)` (banker's rounding không đáng kể ở đây).
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn val_like_vb() {
        assert_eq!(vb_val(" 17.42"), 17.42);
        assert_eq!(vb_val("17.42 Ohm/sq\r"), 17.42);
        assert_eq!(vb_val("+4.532360"), 4.53236);
        assert_eq!(vb_val("+01"), 1.0);
        assert_eq!(vb_val("-03"), -3.0);
        assert_eq!(vb_val("1 2 3"), 123.0);
        assert_eq!(vb_val("abc"), 0.0);
        assert_eq!(vb_val(""), 0.0);
        assert_eq!(vb_val("1.5e"), 1.5);
        assert_eq!(vb_val("2.5E-3x"), 0.0025);
    }

    #[test]
    fn num_like_dotnet() {
        assert_eq!(fmt_num(17.42 + -0.4), "17.02");
        assert_eq!(fmt_num(-0.4), "-0.4");
        assert_eq!(fmt_num(0.0), "0");
        assert_eq!(fmt_num(16.0), "16");
        assert_eq!(fmt_num(1e-7), "1E-07");
        assert_eq!(fmt_num(123456.789), "123456.789");
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
    }

    #[test]
    fn banker_round() {
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(3.5), 4.0);
        assert_eq!(round_half_even(29.4), 29.0);
        assert_eq!(round_half_even(-2.5), -2.0);
    }

    #[test]
    fn si() {
        assert_eq!(fmt_ohms_si(45.326), "45.33 Ohms");
        assert_eq!(fmt_ohms_si(4532.6), "4.53 kOhms");
        assert_eq!(fmt_ohms_si(0.0123), "12.3 mOhms");
        assert_eq!(fmt_ohms_si(9.91e37), "OVERFLOW");
    }
}

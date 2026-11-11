//! Giao thức Keithley (SourceMeter, SCPI qua RS-232) cho đầu đo 4 điểm.
//! Chuyển từ f4PPMeasure: ReadFromKeithley, UT_Method, VoltsValFunc, Slope_and_Error, SendComms.
//!
//! Cổng: 57600 baud, 8N1, XON/XOFF, ReadTimeout 500 ms, NewLine "\n".

use crate::link::{sleep_ms, Flow, Link, SerialLink};
use crate::util::vb_val;
use std::f64::consts::{LN_2, PI};
use std::io;
use std::time::Duration;

pub const BAUD: u32 = 57600;
pub const READ_TIMEOUT: Duration = Duration::from_millis(500);

/// π / ln 2 — hệ số đổi điện trở 4 điểm thành điện trở tấm.
pub fn k_sheet() -> f64 {
    PI / LN_2
}

pub fn open(port: &str) -> io::Result<SerialLink> {
    SerialLink::open(port, BAUD, Flow::XonXoff)
}

/// "*IDN?" — trả về chuỗi nhận dạng; đúng máy nếu có chữ "KEITHLEY".
pub fn idn(link: &mut dyn Link) -> io::Result<String> {
    link.write_line("*IDN?")?;
    sleep_ms(100);
    link.read_line(READ_TIMEOUT)
}

/// Nhãn Device Model như bản gốc: hai phần đầu của chuỗi IDN.
pub fn model_label(idn: &str) -> String {
    let p: Vec<&str> = idn.split(',').collect();
    format!("{} {}", p.first().unwrap_or(&"").trim(), p.get(1).unwrap_or(&"").trim()).trim().to_string()
}

/// SendComms: gửi từng lệnh, nghỉ 20 ms giữa các lệnh, 50 ms ở cuối.
pub fn send_comms(link: &mut dyn Link, cmds: &[String]) -> io::Result<()> {
    for c in cmds {
        link.write_line(c)?;
        sleep_ms(20);
    }
    sleep_ms(50);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KMeas {
    pub mantissa: f64,
    pub exponent: i32,
    pub ohms: f64,
}

/// Tách "+4.532360E+01" thành phần định trị / mũ như bản gốc (Split "E" + Val).
pub fn parse_sci(s: &str) -> (f64, f64) {
    let mut it = s.split(['E', 'e']);
    let m = vb_val(it.next().unwrap_or(""));
    let e = vb_val(it.next().unwrap_or(""));
    (m, e)
}

/// Các lệnh của ReadFromKeithley: đo 4 dây có bù offset, máy tự nhân RES × π/ln2.
pub fn read_ohms_commands() -> Vec<String> {
    vec![
        String::new(),
        "*RST".into(),
        ":calc:math:name \"SheetRes\"".into(),
        ":calc:math:unit \"Rsh\"".into(),
        // VB Str(π/ln2) = " 4.53236014182719" (có dấu cách đầu cho số dương)
        ":CALC:MATH:EXPR (RES* 4.53236014182719)".into(),
        ":SENS:FUNC \"RES\"".into(),
        ":SYST:BEEP:STAT off".into(),
        ":sens:res:ocom on".into(),
        ":syst:rsen on".into(),
        ":CALC:STAT ON".into(),
        ":OUTP ON".into(),
        ":INIT".into(),
        ":calc:data?".into(),
    ]
}

/// ReadFromKeithley. Lỗi đọc (quá giờ) -> 0 Ohm như bản gốc; lỗi ghi cổng -> Err.
pub fn read_ohms(link: &mut dyn Link) -> io::Result<KMeas> {
    send_comms(link, &read_ohms_commands())?;
    sleep_ms(60);
    let line = link.read_line(READ_TIMEOUT);
    sleep_ms(20);
    let (m, e) = match line {
        Ok(s) if !s.is_empty() => parse_sci(&s),
        _ => (0.0, 0.0),
    };
    Ok(KMeas { mantissa: m, exponent: e.round() as i32, ohms: m * 10f64.powf(e) })
}

/// Dải dòng cho UT_Method: (lệnh range, dòng bắt đầu µA, bước µA).
pub fn ut_plan(test_split: &[&str], initial_ohms: f64) -> (String, i64, i64) {
    const R10M: &str = ":syst:rsen 1; :sour:func curr; :sour:curr:rang 10e-3; :sens:func \"volt\"; :form:elem volt";
    const R1M: &str = ":syst:rsen 1; :sour:func curr; :sour:curr:rang 1e-3; :sens:func \"volt\"; :form:elem volt";
    const R10U: &str = ":syst:rsen 1; :sour:func curr; :sour:curr:rang 10e-6; :sens:func \"volt\"; :form:elem volt";
    let is_cal = test_split.first().map(|s| *s == "Calibration").unwrap_or(false);
    let range = if is_cal {
        if test_split.get(1) == Some(&"6") { R10M } else { R1M }
    } else if initial_ohms < 20.0 {
        R10M
    } else if initial_ohms < 50000.0 {
        R1M
    } else {
        R10U
    };
    let (start, step) = if is_cal {
        match test_split.get(1).copied().unwrap_or("") {
            "6" => (-2400, 1600),
            "16" => (-900, 600),
            "60" => (-300, 200),
            "500" => (-30, 20),
            _ => (0, 0),
        }
    } else if initial_ohms < 20.0 {
        (-7200, 4800)
    } else if initial_ohms < 200.0 {
        (-720, 480)
    } else if initial_ohms < 20000.0 {
        (-180, 120)
    } else if initial_ohms < 50000.0 {
        (-120, 80)
    } else {
        (-9, 6)
    };
    (range.to_string(), start, step)
}

/// Một điểm của phép quét I–V: (V, I, Rs ước tính).
pub type UtPoint = [f64; 3];

/// UT_Method + VoltsValFunc: quét 4 mức dòng, mỗi mức đọc 3 lần điện áp.
/// `on_live(Rs)` được gọi sau mỗi lần đọc để hiển thị (ô vàng "Measuring Sample").
pub fn ut_method(link: &mut dyn Link, test_split: &[&str], initial_ohms: f64, on_live: &mut dyn FnMut(f64)) -> io::Result<Vec<UtPoint>> {
    let k = k_sheet();
    let (range, start, step) = ut_plan(test_split, initial_ohms);
    send_comms(link, &[String::new(), "*RST".into(), ":SYST:BEEP:STAT 0".into(), range, "OUTP ON".into()])?;
    let mut out = Vec::with_capacity(12);
    let mut cur = start;
    for _ in 0..4 {
        // Bản gốc tạo cả dạng "e-5" nhưng ghi đè ngay bằng dạng "e-6" (cùng giá trị)
        send_comms(link, &[format!("sour:curr:lev {}e-6", cur)])?;
        let amps = cur as f64 / 1_000_000.0;
        link.write_line(":read?")?;
        sleep_ms(50);
        let _ = link.read_line(READ_TIMEOUT); // lần đọc bỏ đi (bản gốc bỏ qua lỗi ở đây)
        // VoltsValFunc
        for _ in 0..3 {
            link.write_line(":read?")?;
            sleep_ms(20);
            let s = link.read_line(READ_TIMEOUT)?;
            let (m, e) = if s.is_empty() { (0.0, 0.0) } else { parse_sci(&s) };
            let v = m * 10f64.powf(e);
            let rs = m / amps * 10f64.powf(e) * PI / LN_2;
            on_live(rs);
            out.push([v, amps, k * v / amps]);
        }
        cur += step;
    }
    link.write_line("*rst")?;
    Ok(out)
}

/// Slope_and_Error: hồi quy V theo I. Trả về [độ dốc, hệ số chặn, hệ số tương quan r].
pub fn slope_and_error(pts: &[UtPoint]) -> [f64; 3] {
    let n = pts.len() as f64;
    if pts.is_empty() {
        return [f64::NAN, f64::NAN, f64::NAN];
    }
    let mut mx = 0.0;
    let mut my = 0.0;
    for p in pts {
        mx += p[1] / n;
        my += p[0] / n;
    }
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for p in pts {
        sxx += (p[1] - mx).powi(2);
        syy += (p[0] - my).powi(2);
        sxy += (p[1] - mx) * (p[0] - my);
    }
    let sx = (sxx / n).sqrt();
    let sy = (syy / n).sqrt();
    let r = sxy / (sx * sy * n);
    let slope = r * (sy / sx);
    let intercept = my - slope * mx;
    [slope, intercept, r]
}

/// Close_Form_Properly của màn hình đo 4PP: tắt nguồn ra.
pub fn output_off(link: &mut dyn Link) -> io::Result<()> {
    link.write_line(":OUTP OFF")?;
    sleep_ms(20);
    link.write_line(":syst:rsen 0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sci_and_label() {
        assert_eq!(parse_sci("+4.532360E+01"), (4.53236, 1.0));
        assert_eq!(parse_sci("-1.234E-03\r"), (-1.234, -3.0));
        assert_eq!(model_label("KEITHLEY INSTRUMENTS INC.,MODEL 2400,1234567,C30"), "KEITHLEY INSTRUMENTS INC. MODEL 2400");
    }

    #[test]
    fn plan_matches_original_table() {
        assert_eq!(ut_plan(&["5", "Points"], 15.0).1, -7200);
        assert_eq!(ut_plan(&["5", "Points"], 150.0), (ut_plan(&["x"], 150.0).0, -720, 480));
        assert_eq!(ut_plan(&["x"], 1000.0).1, -180);
        assert_eq!(ut_plan(&["x"], 30000.0).1, -120);
        assert_eq!(ut_plan(&["x"], 60000.0).1, -9);
        assert!(ut_plan(&["x"], 60000.0).0.contains("10e-6"));
        assert_eq!(ut_plan(&["Calibration", "16", "Ohm"], 15.0).1, -900);
        assert!(ut_plan(&["Calibration", "6", "Ohm"], 15.0).0.contains("10e-3"));
        assert!(ut_plan(&["Calibration", "500", "Ohm"], 15.0).0.contains("1e-3"));
    }

    #[test]
    fn regression() {
        // V = 10 Ohm × I + 0.001
        let pts: Vec<UtPoint> = [-0.0072, -0.0024, 0.0024, 0.0072].iter().map(|&i| [10.0 * i + 0.001, i, 0.0]).collect();
        let [s, b, r] = slope_and_error(&pts);
        assert!((s - 10.0).abs() < 1e-9);
        assert!((b - 0.001).abs() < 1e-12);
        assert!((r - 1.0).abs() < 1e-12);
    }
}

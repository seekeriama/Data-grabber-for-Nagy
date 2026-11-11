//! Giao thức máy Nagy (fStart.StartNagyConnection + fNagyMeasure.ReadFromNagy).
//!
//! Cổng: 9600 baud, 8N1, không bắt tay, mã windows-1252.
//! Kết nối: gửi "£\r\n" rồi "l0\r\n" (WriteLine thêm "\n"), chờ 100 ms, máy phải trả về chuỗi có "l0".
//! Đo: máy tự gửi kết quả dạng "R=17.42..." khi đặt đầu đo; "OVER" khi quá thang (ghi 0).

use crate::link::{decode_1252, sleep_ms, Link, SerialLink, Flow};
use crate::util::vb_val;
use std::io;
use std::time::{Duration, Instant};

pub const BAUD: u32 = 9600;

pub fn open(port: &str) -> io::Result<SerialLink> {
    SerialLink::open(port, BAUD, Flow::None)
}

/// Bắt tay như bản gốc. Ok(true) nếu đúng là máy Nagy.
pub fn handshake(link: &mut dyn Link) -> io::Result<bool> {
    link.write_line("\u{00A3}\r\n")?;
    link.write_line("l0\r\n")?;
    sleep_ms(100);
    let resp = decode_1252(&link.read_existing()?);
    Ok(resp.contains("l0"))
}

/// Gom dữ liệu từ máy, tách ra từng kết quả "R=...".
#[derive(Default)]
pub struct Reader {
    buf: String,
    last_rx: Option<Instant>,
}

/// Kết quả thô (chưa cộng offset). OVER -> 0.
pub fn parse_reading(text: &str) -> Option<f64> {
    if !text.contains("R=") {
        return None;
    }
    if text.contains("OVER") {
        return Some(0.0);
    }
    let seg = text.split(['\r', '\n']).find(|s| s.contains("R="))?;
    let after = &seg[seg.find('=')? + 1..];
    Some(vb_val(after))
}

impl Reader {
    pub fn clear(&mut self, link: &mut dyn Link) -> io::Result<()> {
        link.read_existing()?;
        self.buf.clear();
        self.last_rx = None;
        Ok(())
    }

    /// Đọc thêm từ cổng; trả về một kết quả khi đã nhận đủ dòng "R=...".
    /// Dòng được coi là đủ khi có ký tự xuống dòng sau "R=", hoặc máy đã im 300 ms.
    pub fn poll(&mut self, link: &mut dyn Link) -> io::Result<Option<f64>> {
        let got = link.read_existing()?;
        if !got.is_empty() {
            self.buf.push_str(&decode_1252(&got));
            self.last_rx = Some(Instant::now());
        }
        let Some(pos) = self.buf.find("R=") else {
            // Bỏ rác cũ nếu không có kết quả, giữ phần đuôi phòng "R" bị cắt đôi
            if self.buf.len() > 256 {
                let keep = self.buf.split_off(self.buf.len() - 4);
                self.buf = keep;
            }
            return Ok(None);
        };
        let tail = &self.buf[pos..];
        let complete = tail.contains('\r') || tail.contains('\n');
        let quiet = self.last_rx.map(|t| t.elapsed() >= Duration::from_millis(300)).unwrap_or(false);
        if !(complete || quiet) {
            return Ok(None);
        }
        let end = tail.find(['\r', '\n']).map(|i| pos + i + 1).unwrap_or(self.buf.len());
        let chunk: String = self.buf[..end].to_string();
        self.buf.drain(..end);
        Ok(parse_reading(&chunk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        assert_eq!(parse_reading("R=17.42\r"), Some(17.42));
        assert_eq!(parse_reading("R= 16.94 Ohm/sq\r\n"), Some(16.94));
        assert_eq!(parse_reading("xx\rR=1.5\r"), Some(1.5));
        assert_eq!(parse_reading("R=OVER\r"), Some(0.0));
        assert_eq!(parse_reading("l0\r\n"), None);
    }
}

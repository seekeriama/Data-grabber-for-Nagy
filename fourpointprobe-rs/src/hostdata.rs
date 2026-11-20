//! "Get Host Data" (fStart.HostData_Click): line OT1 lấy sản phẩm / Run / phút từ máy chủ dây chuyền.
//!
//! Bản gốc đọc http://ot01_h1/reports/samples/sample.flaws.00, tách theo dòng:
//!   dòng 4: "...: <mô tả sản phẩm 4 từ>" -> Host Product; từ thứ 4 là mã sản phẩm
//!   dòng 7: "...: <run number>"  -> cắt phần thập phân
//!   dòng 8: "...: <run minute>"  -> làm tròn (0.5 về số chẵn)

use crate::util::round_half_even;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const OT1_URL: &str = "http://ot01_h1/reports/samples/sample.flaws.00";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostData {
    pub host_product: String,
    pub product: Option<String>,
    pub run_number: Option<String>,
    pub run_minute: Option<String>,
}

fn field(lines: &[&str], i: usize) -> Result<String, String> {
    let l = lines.get(i).ok_or_else(|| format!("thiếu dòng {}", i + 1))?;
    let v = l.split(':').nth(1).ok_or_else(|| format!("dòng {} không có ':'", i + 1))?;
    Ok(v.trim().to_string())
}

fn decimal(s: &str) -> Result<f64, String> {
    s.trim().parse::<f64>().map_err(|_| format!("không phải số: {s:?}"))
}

fn int_text(v: f64) -> String {
    format!("{}", v as i64)
}

pub fn parse(body: &str) -> Result<HostData, String> {
    let lines: Vec<&str> = body.split('\n').collect();
    let host_product = field(&lines, 3)?;
    let mut hd = HostData { host_product: host_product.clone(), ..Default::default() };
    let parts: Vec<&str> = host_product.split(' ').collect();
    if parts.len() == 4 {
        hd.product = Some(parts[3].trim().to_string());
        hd.run_number = Some(int_text(decimal(&field(&lines, 6)?)?.trunc()));
        hd.run_minute = Some(int_text(round_half_even(decimal(&field(&lines, 7)?)?)));
    }
    Ok(hd)
}

/// GET qua HTTP thường (không TLS), giống WebClient.OpenRead.
pub fn fetch(url: &str, timeout: Duration) -> Result<String, String> {
    let rest = url.strip_prefix("http://").ok_or("chỉ hỗ trợ http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "cổng không hợp lệ")?),
        None => (hostport, 80),
    };
    let addr = (host, port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.next().ok_or("không phân giải được tên máy")?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{host}: {e}"))?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    write!(s, "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    if let Err(e) = s.read_to_end(&mut buf) {
        // một số máy chủ đóng kết nối kiểu "reset" sau khi gửi xong
        if buf.is_empty() {
            return Err(e.to_string());
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("phản hồi HTTP không hợp lệ")?;
    let status = head.lines().next().unwrap_or("");
    if !status.contains(" 200") {
        return Err(format!("máy chủ trả về: {status}"));
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample() {
        let body = "Sample report\nLine: OT1\nDate: 17/10/2026\nProduct: Pilkington TEC 15 TE15\nThickness: 3.2\nWidth: 3210\nRun: 5497.0\nMinute: 120.5\n";
        let h = parse(body).unwrap();
        assert_eq!(h.host_product, "Pilkington TEC 15 TE15");
        assert_eq!(h.product.as_deref(), Some("TE15"));
        assert_eq!(h.run_number.as_deref(), Some("5497"));
        assert_eq!(h.run_minute.as_deref(), Some("120"));
        let h = parse("a\nb\nc\nProduct: Clear float\n").unwrap();
        assert_eq!(h.product, None);
    }

    #[test]
    fn fetch_local_http() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut req = Vec::new();
            let mut b = [0u8; 64];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = c.read(&mut b).unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&b[..n]);
            }
            c.write_all(b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nx\ny\nz\nP: A B C T203\ne\nf\nRun: 12.9\nMin: 7.5\n").unwrap();
        });
        let body = fetch(&format!("http://127.0.0.1:{port}/reports/samples/sample.flaws.00"), Duration::from_secs(2)).unwrap();
        let h = parse(&body).unwrap();
        assert_eq!(h.product.as_deref(), Some("T203"));
        assert_eq!((h.run_number.as_deref(), h.run_minute.as_deref()), (Some("12"), Some("8")));
    }
}

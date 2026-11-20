//! Lớp giao tiếp nối tiếp. Tách thành trait để kiểm thử bằng thiết bị giả lập.

use std::io::{self, Read};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Hệ số thời gian chờ (%), 100 = đúng như bản gốc. Kiểm thử đặt 0 cho nhanh.
pub static TIME_SCALE_PCT: AtomicU32 = AtomicU32::new(100);

pub fn sleep_ms(ms: u64) {
    let pct = TIME_SCALE_PCT.load(Ordering::Relaxed) as u64;
    if pct > 0 && ms > 0 {
        std::thread::sleep(Duration::from_millis(ms * pct / 100));
    }
}

pub trait Link: Send {
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<()>;
    /// Giống SerialPort.ReadExisting: lấy hết những gì đang có, không chờ.
    fn read_existing(&mut self) -> io::Result<Vec<u8>>;
    /// Giống SerialPort.ReadLine (NewLine = "\n"): chờ tới "\n", quá hạn thì lỗi TimedOut.
    fn read_line(&mut self, timeout: Duration) -> io::Result<String>;
    fn name(&self) -> String;

    /// SerialPort.WriteLine: thêm "\n" (NewLine mặc định của .NET).
    fn write_line(&mut self, s: &str) -> io::Result<()> {
        let mut b = encode_1252(s);
        b.push(b'\n');
        self.write_bytes(&b)
    }
}

/// Mã hóa windows-1252 cho các ký tự dùng ở đây (ASCII + "£" = 0xA3).
pub fn encode_1252(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if u < 0x100 {
                u as u8
            } else {
                b'?'
            }
        })
        .collect()
}

pub fn decode_1252(b: &[u8]) -> String {
    b.iter().map(|&x| x as char).collect()
}

/// Cổng COM thật.
pub struct SerialLink {
    port: Box<dyn serialport::SerialPort>,
    buf: Vec<u8>,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    None,
    XonXoff,
}

impl SerialLink {
    /// 8 bit dữ liệu, không parity, 1 stop bit (như bản gốc).
    pub fn open(name: &str, baud: u32, flow: Flow) -> io::Result<SerialLink> {
        let port = serialport::new(name, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(match flow {
                Flow::None => serialport::FlowControl::None,
                Flow::XonXoff => serialport::FlowControl::Software,
            })
            .timeout(Duration::from_millis(20))
            .open()
            .map_err(io::Error::other)?;
        Ok(SerialLink { port, buf: Vec::new(), name: name.to_string() })
    }

    fn fill(&mut self) -> io::Result<usize> {
        let mut tmp = [0u8; 512];
        match self.port.read(&mut tmp) {
            Ok(n) => {
                self.buf.extend_from_slice(&tmp[..n]);
                Ok(n)
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl Link for SerialLink {
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        use std::io::Write;
        self.port.write_all(data)?;
        self.port.flush()
    }

    fn read_existing(&mut self) -> io::Result<Vec<u8>> {
        loop {
            let avail = self.port.bytes_to_read().map_err(io::Error::other)?;
            if avail == 0 {
                break;
            }
            if self.fill()? == 0 {
                break;
            }
        }
        Ok(std::mem::take(&mut self.buf))
    }

    fn read_line(&mut self, timeout: Duration) -> io::Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                return Ok(decode_1252(&line[..line.len() - 1]));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "ReadLine timeout"));
            }
            self.fill()?;
        }
    }

    fn name(&self) -> String {
        self.name.clone()
    }
}

pub fn list_ports() -> Vec<String> {
    let mut v: Vec<String> = serialport::available_ports().map(|p| p.into_iter().map(|x| x.port_name).collect()).unwrap_or_default();
    v.sort();
    v
}

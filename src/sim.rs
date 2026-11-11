//! Thiết bị giả lập để kiểm thử và chạy thử chương trình khi không có máy
//! (FourPointProbe.exe --simulate 4pp | nagy).

use crate::link::Link;
use std::collections::VecDeque;
use std::f64::consts::{LN_2, PI};
use std::io;
use std::time::{Duration, Instant};

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 11) as f64) / ((1u64 << 53) as f64)
}

fn take_line(out: &mut VecDeque<u8>) -> Option<String> {
    let i = out.iter().position(|&b| b == b'\n')?;
    let line: Vec<u8> = out.drain(..=i).collect();
    Some(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned())
}

/// Keithley giả: mỗi lần đặt đầu đo là một mẫu có điện trở tấm lấy lần lượt trong `sheet_ohms`.
pub struct FakeKeithley {
    pub sheet_ohms: Vec<f64>,
    pub touch: usize,
    pub down: bool,
    pub reads_up: u32,
    up_left: u32,
    current_a: f64,
    out: VecDeque<u8>,
    inbuf: Vec<u8>,
    seed: u64,
    /// nhiễu tương đối (vd 0.0005 = 0.05%)
    pub noise: f64,
    /// số lần quét I–V đầu tiên trả dữ liệu hỏng (kiểm thử r < 0.99)
    pub bad_sweeps: u32,
    in_sweep: bool,
    pub log: Vec<String>,
}

impl FakeKeithley {
    pub fn new(sheet_ohms: Vec<f64>) -> Self {
        FakeKeithley {
            sheet_ohms,
            touch: 0,
            down: false,
            reads_up: 2,
            up_left: 2,
            current_a: 0.0,
            out: VecDeque::new(),
            inbuf: vec![],
            seed: 42,
            noise: 0.0005,
            bad_sweeps: 0,
            in_sweep: false,
            log: vec![],
        }
    }

    fn rs(&mut self) -> f64 {
        let base = self.sheet_ohms[self.touch % self.sheet_ohms.len()];
        base * (1.0 + (lcg(&mut self.seed) - 0.5) * 2.0 * self.noise)
    }

    fn reply(&mut self, s: String) {
        self.out.extend(s.bytes());
        self.out.extend(b"\r\n");
    }

    fn handle(&mut self, cmd: &str) {
        self.log.push(cmd.to_string());
        let c = cmd.trim();
        if c == "*IDN?" {
            self.reply("KEITHLEY INSTRUMENTS INC.,MODEL 2400,4073543,C33 Mar 31 2015 09:32:39/A02  /S/K".into());
        } else if c == ":calc:data?" {
            if !self.down {
                if self.up_left == 0 {
                    self.down = true;
                } else {
                    self.up_left -= 1;
                }
            }
            let v = if self.down { self.rs() } else { 9.91e37 };
            self.reply(format!("{:+.6E}", v));
        } else if let Some(x) = c.strip_prefix("sour:curr:lev ") {
            self.current_a = x.trim().parse::<f64>().unwrap_or(0.0);
            self.in_sweep = true;
        } else if c == ":read?" {
            let v = if self.down {
                let mut v = self.current_a * self.rs() * LN_2 / PI;
                if self.bad_sweeps > 0 {
                    v = (lcg(&mut self.seed) - 0.5) * 0.01; // nhiễu hoàn toàn, không tuyến tính
                }
                v
            } else {
                9.91e37
            };
            self.reply(format!("{:+.6E}", v));
        } else if c == "*rst" && self.in_sweep {
            // kết thúc UT_Method -> người vận hành nhấc đầu đo, chuyển sang điểm sau
            self.in_sweep = false;
            if self.bad_sweeps > 0 {
                self.bad_sweeps -= 1;
                return;
            }
            self.touch += 1;
            self.down = false;
            self.up_left = self.reads_up;
        }
    }
}

impl Link for FakeKeithley {
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        self.inbuf.extend_from_slice(data);
        while let Some(i) = self.inbuf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.inbuf.drain(..=i).collect();
            let s = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            self.handle(&s);
        }
        Ok(())
    }
    fn read_existing(&mut self) -> io::Result<Vec<u8>> {
        Ok(self.out.drain(..).collect())
    }
    fn read_line(&mut self, _timeout: Duration) -> io::Result<String> {
        take_line(&mut self.out).ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "ReadLine timeout"))
    }
    fn name(&self) -> String {
        "SIM-KEITHLEY".into()
    }
}

/// Nagy giả: trả "R=..." theo hàng đợi (kiểm thử) hoặc tự sinh theo thời gian (chạy thử).
pub struct FakeNagy {
    pub readings: VecDeque<String>,
    /// số lần poll giữa hai kết quả (chế độ kiểm thử)
    pub polls_between: u32,
    polls: u32,
    /// chế độ chạy thử: tự sinh kết quả mỗi `period`
    pub period: Option<Duration>,
    last: Instant,
    out: VecDeque<u8>,
    pending_tail: Option<String>,
    seed: u64,
    /// Nếu có: chỉ phát kết quả thứ n khi gate >= n (kiểm thử điều khiển thời điểm đặt đầu đo)
    pub gate: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    emitted: usize,
}

impl FakeNagy {
    pub fn with_readings(r: &[&str]) -> Self {
        FakeNagy {
            readings: r.iter().map(|s| s.to_string()).collect(),
            polls_between: 3,
            polls: 0,
            period: None,
            last: Instant::now(),
            out: VecDeque::new(),
            pending_tail: None,
            seed: 7,
            gate: None,
            emitted: 0,
        }
    }
    pub fn auto(period: Duration) -> Self {
        let mut f = FakeNagy::with_readings(&[]);
        f.period = Some(period);
        f
    }
}

impl Link for FakeNagy {
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        if data.windows(2).any(|w| w == b"l0") {
            self.out.extend(b"l0\r\n");
        }
        Ok(())
    }
    fn read_existing(&mut self) -> io::Result<Vec<u8>> {
        if let Some(t) = self.pending_tail.take() {
            self.out.extend(t.bytes());
        } else if let Some(p) = self.period {
            if self.last.elapsed() >= p {
                self.last = Instant::now();
                let v = 17.0 + lcg(&mut self.seed) * 0.8;
                self.out.extend(format!("R={:.2}\r\n", v).bytes());
            }
        } else {
            self.polls += 1;
            let open = self.gate.as_ref().map(|g| g.load(std::sync::atomic::Ordering::SeqCst) > self.emitted).unwrap_or(true);
            if open && self.polls >= self.polls_between {
                self.polls = 0;
                if let Some(r) = self.readings.pop_front() {
                    self.emitted += 1;
                    // gửi làm hai mảnh để kiểm tra việc ghép dòng
                    let msg = format!("{r}\r\n");
                    let cut = msg.len().min(3);
                    self.out.extend(msg[..cut].bytes());
                    self.pending_tail = Some(msg[cut..].to_string());
                }
            }
        }
        Ok(self.out.drain(..).collect())
    }
    fn read_line(&mut self, _timeout: Duration) -> io::Result<String> {
        take_line(&mut self.out).ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "ReadLine timeout"))
    }
    fn name(&self) -> String {
        "SIM-NAGY".into()
    }
}

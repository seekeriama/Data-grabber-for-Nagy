//! Vòng đo (chạy trên luồng riêng), chuyển từ btnMeasure_Click / GetNewMeas / ReplaceRecord
//! của f4PPMeasure và fNagyMeasure.
//!
//! Giao diện gửi lệnh (Remeasure / Stop), luồng đo gửi sự kiện (trạng thái, giá trị, kết quả).

use crate::catalog::{Device, CONTINUOUS, SAMPLE_SIZES};
use crate::config::Poly;
use crate::csvout::{Record, RunInfo};
use crate::keithley::{self, k_sheet, KMeas};
use crate::link::{sleep_ms, Link};
use crate::nagy;
use crate::util::{fmt_num, fmt_ohms_si};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// chữ trắng nền đỏ sẫm ("System Connected")
    Connected,
    /// xám: chờ / nhấc đầu đo
    Idle,
    /// xanh lá nhạt: đang đọc ổn định
    Reading,
    /// vàng: đang đo
    Measuring,
    /// xanh lá: xong
    Done,
    /// đỏ: lỗi
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Evt {
    Status(String, Tone),
    Live(String, Tone),
    SampleName(String),
    /// None = kết quả mới; Some(i) = đo lại, thay kết quả thứ i
    Record(Option<usize>, Record),
    SubmitEnabled(bool),
    /// Lỗi giao tiếp (hiển thị, vòng đo vẫn tiếp tục)
    CommError(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Remeasure(usize),
    Stop,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub device: Device,
    pub info: RunInfo,
    /// (PointName, ERSName) theo thứ tự đo
    pub items: Vec<(String, String)>,
    pub sample_size: Option<usize>,
    pub upper_spec: f64,
    pub poly: Poly,
}

impl Session {
    pub fn continuous(&self) -> bool {
        self.info.test_name == CONTINUOUS
    }
    fn test_split(&self) -> Vec<&str> {
        self.info.test_name.split(' ').collect()
    }
    fn is_calibration(&self) -> bool {
        self.test_split().first() == Some(&"Calibration")
    }
    fn point_name(&self, i: usize, count: usize) -> (String, String) {
        if self.continuous() {
            let (n, e) = self.items.first().cloned().unwrap_or(("Sample".into(), "Not Stored".into()));
            (format!("{}_{:02}", n, count), e)
        } else {
            self.items[i].clone()
        }
    }
}

enum Flow {
    Go,
    Stop,
}

struct Ctx<'a> {
    link: &'a mut dyn Link,
    rx: &'a Receiver<Cmd>,
    emit: &'a mut dyn FnMut(Evt),
    queue: VecDeque<usize>,
    records: Vec<Record>,
    stop: bool,
}

impl Ctx<'_> {
    /// Thay cho Application.DoEvents: nhận lệnh từ giao diện.
    fn pump(&mut self) -> Flow {
        loop {
            match self.rx.try_recv() {
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => {
                    self.stop = true;
                    return Flow::Stop;
                }
                Ok(Cmd::Remeasure(i)) => {
                    if i < self.records.len() && !self.queue.contains(&i) {
                        self.queue.push_back(i);
                    }
                }
                Err(TryRecvError::Empty) => return if self.stop { Flow::Stop } else { Flow::Go },
            }
        }
    }

    fn status(&mut self, s: &str, t: Tone) {
        (self.emit)(Evt::Status(s.to_string(), t));
    }
    fn live(&mut self, s: String, t: Tone) {
        (self.emit)(Evt::Live(s, t));
    }

    fn push_record(&mut self, r: Record) {
        self.records.push(r.clone());
        (self.emit)(Evt::Record(None, r));
    }

    fn replace_record(&mut self, i: usize, value: f64) {
        let mut r = self.records[i].clone();
        r.value = value;
        r.time = chrono::Local::now().naive_local();
        self.records[i] = r.clone();
        (self.emit)(Evt::Record(Some(i), r));
    }

    /// Sau khi đo xong: chờ lệnh đo lại hoặc dừng.
    fn idle_wait(&mut self) -> Option<usize> {
        loop {
            if let Flow::Stop = self.pump() {
                return None;
            }
            if let Some(i) = self.queue.pop_front() {
                return Some(i);
            }
            sleep_ms(30);
            if crate::link::TIME_SCALE_PCT.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                std::thread::yield_now();
            }
        }
    }
}

/// Chạy một lượt đo. Trả lại cổng khi kết thúc (Submit / Exit).
pub fn run(session: Session, mut link: Box<dyn Link>, rx: Receiver<Cmd>, mut emit: impl FnMut(Evt)) -> Box<dyn Link> {
    {
        let mut ctx = Ctx { link: link.as_mut(), rx: &rx, emit: &mut emit, queue: VecDeque::new(), records: vec![], stop: false };
        ctx.status("System Connected", Tone::Connected);
        match session.device {
            Device::Nagy => run_nagy(&session, &mut ctx),
            Device::FourPoint => {
                run_4pp(&session, &mut ctx);
                if let Err(e) = keithley::output_off(ctx.link) {
                    (ctx.emit)(Evt::CommError(format!("Error in Communication: {e}")));
                }
            }
        }
    }
    link
}

// ----------------------------------------------------------------------------------------
// Nagy
// ----------------------------------------------------------------------------------------

enum NagyWait {
    Value(f64),
    Stop,
}

/// ReadFromNagy: chờ máy gửi "R=..." rồi cộng offset. Lệnh đo lại được xử lý trong lúc chờ.
fn nagy_wait(s: &Session, ctx: &mut Ctx, reader: &mut nagy::Reader, current_name: &str, allow_remeasure: bool) -> NagyWait {
    let mut remeasuring: Option<usize> = None;
    loop {
        if let Flow::Stop = ctx.pump() {
            return NagyWait::Stop;
        }
        if allow_remeasure && remeasuring.is_none() {
            if let Some(i) = ctx.queue.pop_front() {
                let n = ctx.records[i].point.clone();
                ctx.status(&format!("Remeasuring {n}"), Tone::Done);
                (ctx.emit)(Evt::SampleName(n));
                remeasuring = Some(i);
            }
        }
        match reader.poll(ctx.link) {
            Ok(Some(raw)) => {
                let v = raw + s.info.offset_nagy;
                ctx.live(format!("{} Ohms", fmt_num(v)), Tone::Idle);
                match remeasuring.take() {
                    Some(i) => {
                        ctx.replace_record(i, v);
                        ctx.status("Measurement Complete", Tone::Done);
                        (ctx.emit)(Evt::SampleName(current_name.to_string()));
                    }
                    None => return NagyWait::Value(v),
                }
            }
            Ok(None) => {}
            Err(e) => {
                (ctx.emit)(Evt::CommError(format!("Error in reading Nagy Measurement: {e}")));
                sleep_ms(1000);
            }
        }
        sleep_ms(100);
    }
}

fn run_nagy(s: &Session, ctx: &mut Ctx) {
    let mut reader = nagy::Reader::default();
    // BeginTest_Click: ReadExisting để bỏ dữ liệu cũ
    let _ = reader.clear(ctx.link);
    let n = if s.continuous() { 1 } else { s.items.len() };
    let mut count = 0usize;
    loop {
        for i in 0..n {
            if s.continuous() {
                count += 1;
            }
            let (name, ers) = s.point_name(i, count);
            (ctx.emit)(Evt::SampleName(name.clone()));
            let v = match nagy_wait(s, ctx, &mut reader, &name, true) {
                NagyWait::Value(v) => v,
                NagyWait::Stop => return,
            };
            ctx.push_record(Record { point: name, ers, value: v, time: chrono::Local::now().naive_local() });
            ctx.status("Measurement Complete", Tone::Done);
        }
        (ctx.emit)(Evt::SubmitEnabled(true));
        if !s.continuous() {
            break;
        }
    }
    // Đã đủ điểm: vẫn cho bấm vào kết quả để đo lại (ReplaceRecord)
    while let Some(i) = ctx.idle_wait() {
        let name = ctx.records[i].point.clone();
        ctx.status(&format!("Remeasuring {name}"), Tone::Done);
        (ctx.emit)(Evt::SampleName(name.clone()));
        match nagy_wait(s, ctx, &mut reader, &name, false) {
            NagyWait::Value(v) => {
                ctx.replace_record(i, v);
                ctx.status("Measurement Complete", Tone::Done);
                (ctx.emit)(Evt::SampleName("Continue".into()));
            }
            NagyWait::Stop => return,
        }
    }
}

// ----------------------------------------------------------------------------------------
// 4-Point Probe (Keithley)
// ----------------------------------------------------------------------------------------

struct Probe {
    initial: f64,
    last: f64,
    down: bool,
    arr: [f64; 2],
    precision: f64,
}

enum Meas {
    Value(f64),
    Bad,
    Stop,
}

fn read(ctx: &mut Ctx) -> KMeas {
    match keithley::read_ohms(ctx.link) {
        Ok(m) => m,
        Err(e) => {
            (ctx.emit)(Evt::CommError(format!("Error in Communication: {e}")));
            sleep_ms(500);
            KMeas::default()
        }
    }
}

/// Hai lần đọc liên tiếp lệch nhau < precision thì coi là đầu đo đã đặt ổn định.
fn stable(arr: &mut [f64; 2], ohms: f64, precision: f64) -> bool {
    arr[1] = arr[0];
    arr[0] = ohms;
    arr[1] != 0.0 && ((arr[0] - arr[1]) / arr[1]).abs() < precision
}

/// GetNewMeas
fn get_new_meas(s: &Session, ctx: &mut Ctx, p: &mut Probe) -> Meas {
    loop {
        if let Flow::Stop = ctx.pump() {
            return Meas::Stop;
        }
        let m = read(ctx);
        p.last = p.initial;
        // Cố ý viết dạng phủ định: NaN cũng bị coi là lệch (giống lệnh bge.un của bản gốc)
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let moved = !(((m.ohms - p.last) / p.last).abs() < p.precision);
        if moved {
            ctx.status("Lower Probe", Tone::Measuring);
            ctx.live(fmt_ohms_si(m.ohms), Tone::Idle);
            p.down = false;
            p.initial = 9_999_999.0;
            return Meas::Bad;
        }
        p.initial = m.ohms;
        let corr = s.sample_size.and_then(|i| SAMPLE_SIZES.get(i)).map(|x| x.1).unwrap_or(0.0);
        let mut r;
        if s.info.test_name == "2 Inch Ohms" || s.info.test_name == "4 Inch Ohms" {
            r = (p.last + p.initial) / 2.0 * corr;
        } else {
            let split = s.test_split();
            let initial = p.initial;
            let emit = &mut *ctx.emit;
            let pts = keithley::ut_method(&mut *ctx.link, &split, initial, &mut |rs| {
                emit(Evt::Live(fmt_ohms_si(rs), Tone::Measuring));
                emit(Evt::Status("Measuring Sample".into(), Tone::Measuring));
            });
            let ok = match pts {
                Ok(pts) => {
                    let [slope, _icpt, rr] = keithley::slope_and_error(&pts);
                    r = slope * corr * k_sheet();
                    rr >= 0.99
                }
                Err(e) => {
                    (ctx.emit)(Evt::CommError(format!("Error in Measurement: {e}")));
                    r = f64::NAN;
                    false
                }
            };
            if !ok {
                ctx.status("Error in Measurement", Tone::Error);
                ctx.live(fmt_ohms_si(m.ohms), Tone::Error);
                ctx.status("Remeasure", Tone::Error);
                continue;
            }
        }
        if !s.is_calibration() {
            r = s.poly.apply(r);
        }
        ctx.live(fmt_ohms_si(r), Tone::Done);
        ctx.status("", Tone::Done);
        return Meas::Value(r);
    }
}

/// ReplaceRecord (4PP): đặt đầu đo lên điểm cần đo lại, đo, thay kết quả.
fn fpp_remeasure(s: &Session, ctx: &mut Ctx, p: &mut Probe, idx: usize) -> Flow {
    let name = ctx.records[idx].point.clone();
    ctx.status(&format!("Remeasuring {name}"), Tone::Done);
    (ctx.emit)(Evt::SampleName(name));
    loop {
        let mut arr = [0.0; 2];
        loop {
            if let Flow::Stop = ctx.pump() {
                return Flow::Stop;
            }
            let m = read(ctx);
            if m.ohms < 2_500_000.0 && m.ohms > 0.0 && stable(&mut arr, m.ohms, p.precision) {
                ctx.live(fmt_ohms_si(m.ohms), Tone::Reading);
                ctx.status("Reading Initiated", Tone::Reading);
                p.initial = m.ohms;
                break;
            }
            ctx.live(fmt_ohms_si(m.ohms), Tone::Idle);
            ctx.status("Lower Probe", Tone::Idle);
        }
        match get_new_meas(s, ctx, p) {
            Meas::Value(v) => {
                ctx.replace_record(idx, v);
                // chờ nhấc đầu đo trước khi đo điểm tiếp theo
                p.down = true;
                return Flow::Go;
            }
            Meas::Bad => {
                sleep_ms(500);
                continue;
            }
            Meas::Stop => return Flow::Stop,
        }
    }
}

fn run_4pp(s: &Session, ctx: &mut Ctx) {
    let mut p = Probe {
        initial: 9_999_999.0,
        last: 0.0,
        down: false,
        arr: [0.0; 2],
        precision: if s.is_calibration() { 0.1 } else { 0.01 },
    };
    let buffer_layer = s.info.test_name == "Buffer Layer";
    let n = if s.continuous() { 1 } else { s.items.len() };
    let mut count = 0usize;
    'outer: loop {
        let mut i = 0;
        while i < n {
            if s.continuous() {
                count += 1;
            }
            let (name, ers) = s.point_name(i, count);
            'point: loop {
                (ctx.emit)(Evt::SampleName(name.clone()));
                // Đo lại được yêu cầu: làm ngay rồi đo lại điểm hiện tại từ đầu (bEventInterruption)
                if let Some(idx) = ctx.queue.pop_front() {
                    if let Flow::Stop = fpp_remeasure(s, ctx, &mut p, idx) {
                        return;
                    }
                    continue 'point;
                }
                // Pha 1: chờ nhấc đầu đo khỏi điểm trước
                while p.down {
                    if let Flow::Stop = ctx.pump() {
                        return;
                    }
                    if !ctx.queue.is_empty() {
                        continue 'point;
                    }
                    let m = read(ctx);
                    ctx.live(fmt_ohms_si(m.ohms), Tone::Idle);
                    if m.ohms > 2.0 * p.initial || m.ohms < 0.0 {
                        p.down = false;
                    }
                }
                // Pha 2: chờ đặt đầu đo, hai lần đọc liên tiếp ổn định
                while !p.down {
                    if let Flow::Stop = ctx.pump() {
                        return;
                    }
                    if !ctx.queue.is_empty() {
                        continue 'point;
                    }
                    let m = read(ctx);
                    let valid = if buffer_layer { m.ohms < 1_000_000.0 && m.ohms > 0.0 } else { m.ohms < 10.0 * s.upper_spec && m.ohms > 0.0 };
                    if valid && stable(&mut p.arr, m.ohms, p.precision) {
                        ctx.live(fmt_ohms_si(m.ohms), Tone::Reading);
                        ctx.status("Reading Initiated", Tone::Reading);
                        p.last = p.arr[1];
                        p.initial = p.arr[0];
                        p.down = true;
                    } else {
                        ctx.live(fmt_ohms_si(m.ohms), Tone::Idle);
                        ctx.status("Lower Probe", Tone::Idle);
                    }
                }
                match get_new_meas(s, ctx, &mut p) {
                    Meas::Value(v) => {
                        ctx.push_record(Record { point: name.clone(), ers: ers.clone(), value: v, time: chrono::Local::now().naive_local() });
                        break 'point;
                    }
                    Meas::Bad => {
                        sleep_ms(500);
                        continue 'point;
                    }
                    Meas::Stop => return,
                }
            }
            i += 1;
        }
        (ctx.emit)(Evt::SubmitEnabled(true));
        ctx.status("Measurement Complete", Tone::Done);
        if !s.continuous() {
            break 'outer;
        }
    }
    while let Some(idx) = ctx.idle_wait() {
        if let Flow::Stop = fpp_remeasure(s, ctx, &mut p, idx) {
            return;
        }
        ctx.status("Measurement Complete", Tone::Done);
    }
}

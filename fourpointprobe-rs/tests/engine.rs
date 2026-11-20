//! Kiểm thử vòng đo với thiết bị giả lập.

use fpp::catalog::{sample_size_index, Catalog, Device};
use fpp::config::Poly;
use fpp::csvout::{nagy_block, FileFormat, PendingStore, Record, RunInfo};
use fpp::engine::{run, Cmd, Evt, Session};
use fpp::link::{Link, TIME_SCALE_PCT};
use fpp::sim::{FakeKeithley, FakeNagy};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

struct Harness {
    tx: Sender<Cmd>,
    rx: Receiver<Evt>,
    handle: Option<JoinHandle<Box<dyn Link>>>,
    records: Vec<Record>,
    statuses: Vec<String>,
    names: Vec<String>,
    submit: bool,
}

impl Harness {
    fn start(s: Session, link: Box<dyn Link>) -> Harness {
        TIME_SCALE_PCT.store(0, Ordering::Relaxed);
        let (tx, crx) = channel();
        let (etx, rx) = channel();
        let handle = std::thread::spawn(move || run(s, link, crx, move |e| {
            let _ = etx.send(e);
        }));
        Harness { tx, rx, handle: Some(handle), records: vec![], statuses: vec![], names: vec![], submit: false }
    }

    fn pump_until(&mut self, what: &str, cond: impl Fn(&Harness) -> bool) {
        let t0 = Instant::now();
        while !cond(self) {
            assert!(t0.elapsed() < Duration::from_secs(20), "timeout waiting for {what}; statuses={:?}", self.statuses.iter().rev().take(8).collect::<Vec<_>>());
            if let Ok(e) = self.rx.recv_timeout(Duration::from_millis(50)) {
                match e {
                    Evt::Record(None, r) => self.records.push(r),
                    Evt::Record(Some(i), r) => self.records[i] = r,
                    Evt::Status(s, _) => self.statuses.push(s),
                    Evt::SampleName(n) => self.names.push(n),
                    Evt::SubmitEnabled(b) => self.submit = b,
                    Evt::Live(..) => {}
                    Evt::CommError(e) => panic!("comm error: {e}"),
                }
            }
        }
    }

    fn stop(mut self) -> Box<dyn Link> {
        self.tx.send(Cmd::Stop).unwrap();
        self.handle.take().unwrap().join().unwrap()
    }
}

fn session(device: Device, product: &str, test: &str, offset: f64) -> Session {
    let cat = Catalog::embedded();
    let dc = cat.for_device(device);
    let items = dc.items_for_test(test);
    Session {
        device,
        info: RunInfo { line: "VG1".into(), run_number: "1981".into(), run_minute: "29".into(), test_name: test.into(), offset_nagy: offset },
        sample_size: sample_size_index(&dc.test(test).unwrap().sample_size),
        upper_spec: dc.product(product).unwrap().spec_upper,
        items,
        poly: Poly::default(),
    }
}

#[test]
fn nagy_five_points_with_offset_and_csv() {
    let link = FakeNagy::with_readings(&["R=17.42", "R=17.71", "R=17.70", "R=OVER", "R= 16.94 "]);
    let mut h = Harness::start(session(Device::Nagy, "T203", "5 Points", -0.4), Box::new(link));
    h.pump_until("5 records", |h| h.records.len() == 5 && h.submit);
    let vals: Vec<String> = h.records.iter().map(|r| fpp::util::fmt_num(r.value)).collect();
    assert_eq!(vals, ["17.02", "17.31", "17.3", "-0.4", "16.54"]);
    let pts: Vec<&str> = h.records.iter().map(|r| r.point.as_str()).collect();
    assert_eq!(pts, ["Left Edge", "Left Center", "Center", "Right Center", "Right Edge"]);

    // CSV cho cả 3 định dạng
    let dir = std::env::temp_dir().join(format!("fpp_engine_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = session(Device::Nagy, "T203", "5 Points", -0.4);
    let b = nagy_block(&dir, &s.info, &h.records, h.records[0].time).unwrap();
    let (ok, failed) = PendingStore::default().write_block(&b, &FileFormat::ALL);
    assert!(failed.is_empty());
    assert_eq!(ok.len(), 3);
    let text = std::fs::read_to_string(&ok[1]).unwrap();
    assert!(ok[1].file_name().unwrap().to_string_lossy().starts_with("ERS Five-Points-"));
    assert!(text.lines().nth(1).unwrap().ends_with(",1981,29,-0.4,17.02,17.31,17.3,-0.4,16.54"), "{text}");
    h.stop();
}

#[test]
fn nagy_remeasure_after_complete_and_midway() {
    use std::sync::{atomic::AtomicUsize, Arc};
    let gate = Arc::new(AtomicUsize::new(2));
    let mut link = FakeNagy::with_readings(&["R=10", "R=11", "R=99", "R=12", "R=13", "R=14", "R=55"]);
    link.gate = Some(gate.clone());
    let mut h = Harness::start(session(Device::Nagy, "T203", "5 Points", 0.0), Box::new(link));
    h.pump_until("second record", |h| h.records.len() == 2);
    // đo lại "Left Edge" giữa chừng: kết quả kế tiếp (99) thuộc về Left Edge
    h.tx.send(Cmd::Remeasure(0)).unwrap();
    h.pump_until("remeasure status", |h| h.statuses.iter().any(|s| s == "Remeasuring Left Edge"));
    gate.store(3, Ordering::SeqCst);
    h.pump_until("remeasure 0", |h| h.records[0].value == 99.0);
    gate.store(6, Ordering::SeqCst);
    h.pump_until("all", |h| h.records.len() == 5 && h.submit);
    let vals: Vec<f64> = h.records.iter().map(|r| r.value).collect();
    assert_eq!(vals, [99.0, 11.0, 12.0, 13.0, 14.0]);
    assert_eq!(h.records[2].point, "Center", "điểm hiện tại giữ đúng tên sau khi đo lại");
    // đo lại sau khi xong
    h.tx.send(Cmd::Remeasure(3)).unwrap();
    h.pump_until("remeasure status 3", |h| h.statuses.iter().any(|s| s == "Remeasuring Right Center"));
    gate.store(7, Ordering::SeqCst);
    h.pump_until("remeasure 3", |h| h.records[3].value == 55.0);
    assert!(h.statuses.iter().any(|s| s == "Remeasuring Right Center"));
    h.stop();
}

#[test]
fn nagy_continuous_names() {
    let link = FakeNagy::with_readings(&["R=1", "R=2", "R=3"]);
    let mut h = Harness::start(session(Device::Nagy, "TE50", "Continuous Measure", 0.0), Box::new(link));
    h.pump_until("3 records", |h| h.records.len() == 3);
    let pts: Vec<&str> = h.records.iter().map(|r| r.point.as_str()).collect();
    assert_eq!(pts, ["Sample_01", "Sample_02", "Sample_03"]);
    assert!(h.submit);
    h.stop();
}

#[test]
fn fpp_five_points_ut_method() {
    let truth = vec![45.2, 45.6, 45.9, 45.4, 45.1];
    let link = FakeKeithley::new(truth.clone());
    let mut h = Harness::start(session(Device::FourPoint, "T155", "5 Points", 0.0), Box::new(link));
    h.pump_until("5 records", |h| h.records.len() == 5 && h.submit);
    // Sample Size duy nhất "10cm x 10cm" -> hệ số 0.9981
    for (r, t) in h.records.iter().zip(&truth) {
        assert!((r.value - t * 0.9981).abs() / t < 0.003, "{} vs {}", r.value, t);
    }
    let link = h.stop();
    let _ = link; // cổng được trả lại cho màn hình Setup
}

#[test]
fn fpp_bad_fit_then_retry_and_output_off() {
    let mut k = FakeKeithley::new(vec![120.0, 121.0, 122.0]);
    k.bad_sweeps = 1;
    let mut h = Harness::start(session(Device::FourPoint, "OPVW", "3 Points", 0.0), Box::new(k));
    h.pump_until("3 records", |h| h.records.len() == 3 && h.submit);
    assert!(h.statuses.iter().any(|s| s == "Error in Measurement"));
    assert!(h.statuses.iter().any(|s| s == "Remeasure"));
    assert!((h.records[0].value - 120.0).abs() < 0.5);
    h.stop();
}

#[test]
fn fpp_four_inch_uses_average_and_sample_size_factor() {
    let k = FakeKeithley::new(vec![10.0]);
    let mut s = session(Device::FourPoint, "ENAC", "4 Inch Ohms", 0.0);
    s.items.truncate(2);
    s.sample_size = Some(0); // 10cm x 10cm -> 0.9981
    let mut h = Harness::start(s, Box::new(k));
    // không quét I–V nên giả lập không tự "nhấc đầu đo": chỉ cần điểm đầu
    h.pump_until("1 record", |h| !h.records.is_empty());
    let v = h.records[0].value;
    assert!((v - 10.0 * 0.9981).abs() < 0.02, "{v}");
    h.stop();
}

#[test]
fn fpp_poly_and_remeasure_after_complete() {
    let mut s = session(Device::FourPoint, "OPVW", "3 Points", 0.0);
    s.poly = Poly { a: 0.0, b: 2.0, c: 1.0, low: 0.0, high: 600.0 };
    let k = FakeKeithley::new(vec![100.0, 101.0, 102.0, 150.0]);
    let mut h = Harness::start(s, Box::new(k));
    h.pump_until("3 records", |h| h.records.len() == 3 && h.submit);
    assert!((h.records[0].value - 201.0).abs() < 1.0, "{}", h.records[0].value);
    h.tx.send(Cmd::Remeasure(1)).unwrap();
    h.pump_until("remeasured", |h| (h.records[1].value - 301.0).abs() < 1.5);
    assert!(h.statuses.iter().any(|s| s == "Remeasuring Center"));
    let link = h.stop();
    drop(link);
}

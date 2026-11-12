//! Ghi kết quả ra CSV, theo 1–3 định dạng tên file Default / ERS / Sequoia.
//!
//! Tên file gốc giữ nguyên; ERS và Sequoia chỉ thêm tiền tố:
//!   Default -> Five-Points-20261017.csv
//!   ERS     -> ERS_Five-Points-20261017.csv
//!   Sequoia -> Sequoia_Five-Points-20261017.csv
//! Nội dung (cột, dòng) giống hệt nhau ở mọi định dạng.

use crate::util::{fmt_num, fmt_time};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FileFormat {
    Default,
    #[serde(rename = "ERS")]
    Ers,
    Sequoia,
}

impl FileFormat {
    pub const ALL: [FileFormat; 3] = [FileFormat::Default, FileFormat::Ers, FileFormat::Sequoia];

    pub fn label(self) -> &'static str {
        match self {
            FileFormat::Default => "Default",
            FileFormat::Ers => "ERS",
            FileFormat::Sequoia => "Sequoia",
        }
    }

    pub fn prefix(self) -> &'static str {
        match self {
            FileFormat::Default => "",
            FileFormat::Ers => "ERS_",
            FileFormat::Sequoia => "Sequoia_",
        }
    }

    /// Bỏ trùng, sắp theo thứ tự Default, ERS, Sequoia; rỗng -> [Default].
    pub fn normalize(v: &[FileFormat]) -> Vec<FileFormat> {
        let mut out: Vec<FileFormat> = FileFormat::ALL.iter().copied().filter(|f| v.contains(f)).collect();
        if out.is_empty() {
            out.push(FileFormat::Default);
        }
        out
    }
}

/// Một kết quả đo (một dòng trong lstDataToCSV của bản gốc).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub point: String,
    pub ers: String,
    pub value: f64,
    pub time: NaiveDateTime,
}

/// Thông tin chung của một lượt đo (từ màn hình Setup).
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    pub line: String,
    pub run_number: String,
    pub run_minute: String,
    pub test_name: String,
    pub offset_nagy: f64,
}

/// Một khối dữ liệu cần nối vào một file (tên file chưa có tiền tố định dạng).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsvBlock {
    pub dir: PathBuf,
    pub file_name: String,
    pub header: String,
    pub lines: Vec<String>,
}

/// Một lần ghi cụ thể (đã có đường dẫn đầy đủ).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingWrite {
    pub path: PathBuf,
    pub header: String,
    pub lines: Vec<String>,
}

impl CsvBlock {
    pub fn targets(&self, formats: &[FileFormat]) -> Vec<PendingWrite> {
        FileFormat::normalize(formats)
            .into_iter()
            .map(|f| PendingWrite {
                path: self.dir.join(format!("{}{}", f.prefix(), self.file_name)),
                header: self.header.clone(),
                lines: self.lines.clone(),
            })
            .collect()
    }
}

/// Tên file + tiêu đề cho Nagy (btnSubmit_Click của fNagyMeasure).
pub fn nagy_file_and_header(test: &str) -> (String, String) {
    let base = "ProductLine,Measurement time,RunNumber,RunMinute,Offset,";
    match test {
        "5 Points" => ("Five-Points-".into(), format!("{base}LEMeasurement,LCMeasurement,CCMeasurement,RCMeasurement,REMeasurement")),
        "3 Points" => ("Three-Point-".into(), format!("{base}LEMeasurement,CCMeasurement,REMeasurement")),
        crate::catalog::CONTINUOUS => ("Continuous-".into(), format!("{base}Data")),
        // Test thêm qua fpp_catalog.json: đặt tên theo test, cột theo tên điểm
        other => {
            let safe: String = other.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            (format!("{safe}-"), format!("{base}Data"))
        }
    }
}

/// Khối CSV Nagy. 5/3 điểm: một dòng cho cả lượt; Continuous: mỗi kết quả một dòng.
/// RunMinute ghi dạng số (không có ="..."), không có dấu phẩy thừa cuối dòng.
pub fn nagy_block(save_dir: &Path, info: &RunInfo, records: &[Record], date: NaiveDateTime) -> Option<CsvBlock> {
    if records.is_empty() {
        return None;
    }
    let (prefix, header) = nagy_file_and_header(&info.test_name);
    let offset = fmt_num(info.offset_nagy);
    let lead = |r: &Record| format!("{},{},{},{},{}", info.line, fmt_time(&r.time), info.run_number, info.run_minute, offset);
    let lines = if info.test_name == crate::catalog::CONTINUOUS {
        records.iter().map(|r| format!("{},{}", lead(r), fmt_num(r.value))).collect()
    } else {
        let vals: Vec<String> = records.iter().map(|r| fmt_num(r.value)).collect();
        vec![format!("{},{}", lead(&records[0]), vals.join(","))]
    };
    Some(CsvBlock {
        dir: save_dir.to_path_buf(),
        file_name: format!("{}{}.csv", prefix, date.format("%Y%m%d")),
        header,
        lines,
    })
}

/// Khối CSV 4-Point Probe (btnSubmit_Click của f4PPMeasure).
/// Lượt có điểm "Not Stored" (Continuous, Gage R&R, Calibration) ghi vào thư mục Records.
pub fn fpp_block(save_dir: &Path, records_dir: &Path, info: &RunInfo, records: &[Record], date: NaiveDateTime) -> Option<CsvBlock> {
    if records.is_empty() {
        return None;
    }
    let ers_storage = info.test_name != crate::catalog::CONTINUOUS && !records.iter().any(|r| r.ers == "Not Stored");
    let lines = records
        .iter()
        .map(|r| format!("{},{},{},{},{},{}", info.line, info.run_number, info.run_minute, r.point, r.ers, fmt_num(r.value)))
        .collect();
    Some(CsvBlock {
        dir: if ers_storage { save_dir.to_path_buf() } else { records_dir.to_path_buf() },
        file_name: format!("{}.csv", date.format("%Y%m%d")),
        header: "Line,Run#,QCMin,TestType,SampleName,Data".into(),
        lines,
    })
}

/// Nối dữ liệu vào file; tạo thư mục, và ghi tiêu đề nếu file chưa tồn tại.
pub fn append(w: &PendingWrite) -> std::io::Result<()> {
    if let Some(dir) = w.path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let is_new = !w.path.exists();
    let mut text = String::new();
    if is_new {
        text.push_str(&w.header);
        text.push_str("\r\n");
    }
    for l in &w.lines {
        text.push_str(l);
        text.push_str("\r\n");
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&w.path)?;
    f.write_all(text.as_bytes())?;
    f.flush()
}

pub const PENDING_NAME: &str = "Pending";

/// Hàng chờ các lần ghi thất bại (thường do file đang mở trong Excel).
/// Lưu vào kho có chữ ký (không tạo file) để không mất dữ liệu khi tắt chương trình,
/// và không ai sửa được số đo đang chờ.
#[derive(Default)]
pub struct PendingStore {
    pub items: Vec<PendingWrite>,
    pub store: Option<std::sync::Arc<crate::secure::SecureStore>>,
}

impl PendingStore {
    /// Trả kèm cảnh báo nếu dữ liệu chờ đã bị sửa (khi đó bị bỏ).
    pub fn load(store: std::sync::Arc<crate::secure::SecureStore>) -> (PendingStore, Option<String>) {
        let (items, warn) = match store.load(PENDING_NAME) {
            Ok(Some(b)) => (serde_json::from_slice(&b).unwrap_or_default(), None),
            Ok(None) => (vec![], None),
            Err(crate::secure::LoadError::Tampered) => {
                let _ = store.remove(PENDING_NAME);
                (vec![], Some("Unsaved CSV data from the last session was changed outside this program and has been discarded.".to_string()))
            }
            Err(crate::secure::LoadError::Io(e)) => (vec![], Some(format!("Could not read unsaved CSV data: {e}"))),
        };
        (PendingStore { items, store: Some(store) }, warn)
    }

    fn persist(&self) {
        if let Some(st) = &self.store {
            if self.items.is_empty() {
                let _ = st.remove(PENDING_NAME);
            } else if let Ok(b) = serde_json::to_vec(&self.items) {
                let _ = st.save(PENDING_NAME, &b);
            }
        }
    }

    /// Ghi tất cả file đích của một khối. Trả về (số file đã ghi, lỗi của các file phải chờ).
    pub fn write_block(&mut self, block: &CsvBlock, formats: &[FileFormat]) -> (Vec<PathBuf>, Vec<(PathBuf, String)>) {
        let mut ok = vec![];
        let mut failed = vec![];
        for w in block.targets(formats) {
            // Nếu file này đã có dữ liệu chờ thì xếp sau để giữ đúng thứ tự dòng
            if self.items.iter().any(|p| p.path == w.path) {
                failed.push((w.path.clone(), "đang chờ ghi bù dữ liệu trước".into()));
                self.items.push(w);
                continue;
            }
            match append(&w) {
                Ok(()) => ok.push(w.path.clone()),
                Err(e) => {
                    failed.push((w.path.clone(), e.to_string()));
                    self.items.push(w);
                }
            }
        }
        self.persist();
        (ok, failed)
    }

    /// Thử ghi lại các mục đang chờ. Trả về danh sách file vừa ghi bù thành công.
    pub fn retry(&mut self) -> Vec<PathBuf> {
        let mut done = vec![];
        let mut blocked: Vec<PathBuf> = vec![];
        let mut keep = vec![];
        for w in std::mem::take(&mut self.items) {
            if blocked.contains(&w.path) {
                keep.push(w);
                continue;
            }
            match append(&w) {
                Ok(()) => done.push(w.path.clone()),
                Err(_) => {
                    blocked.push(w.path.clone());
                    keep.push(w);
                }
            }
        }
        let changed = !done.is_empty();
        self.items = keep;
        if changed {
            self.persist();
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 17).unwrap().and_hms_opt(h, m, s).unwrap()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fpp_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn five_point_records() -> Vec<Record> {
        let raw = [17.42, 17.71, 17.70, 17.65, 16.94]; // giá trị máy đo, offset -0.4
        let names = ["Left Edge", "Left Center", "Center", "Right Center", "Right Edge"];
        let ers = ["Ohms_Val_LE", "Ohms_Val_LC", "Ohms_Val_CC", "Ohms_Val_RC", "Ohms_Val_RE"];
        (0..5)
            .map(|i| Record { point: names[i].into(), ers: ers[i].into(), value: raw[i] + -0.4, time: t(19, 46, 33 + i as u32) })
            .collect()
    }

    #[test]
    fn nagy_five_points_matches_sample_file_layout() {
        let d = tmpdir("nagy5");
        let info = RunInfo { line: "VG1".into(), run_number: "1981".into(), run_minute: "29".into(), test_name: "5 Points".into(), offset_nagy: -0.4 };
        let b = nagy_block(&d, &info, &five_point_records(), t(19, 46, 33)).unwrap();
        assert_eq!(b.file_name, "Five-Points-20261017.csv");
        let mut ps = PendingStore::default();
        let (ok, failed) = ps.write_block(&b, &FileFormat::ALL);
        assert!(failed.is_empty());
        let names: Vec<String> = ok.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Five-Points-20261017.csv", "ERS_Five-Points-20261017.csv", "Sequoia_Five-Points-20261017.csv"]);
        let expect = "ProductLine,Measurement time,RunNumber,RunMinute,Offset,LEMeasurement,LCMeasurement,CCMeasurement,RCMeasurement,REMeasurement\r\n\
                      VG1,17/10/2026 19:46:33,1981,29,-0.4,17.02,17.31,17.3,17.25,16.54\r\n";
        for p in &ok {
            assert_eq!(std::fs::read_to_string(p).unwrap(), expect);
        }
        // lượt thứ hai: không ghi lại tiêu đề
        ps.write_block(&b, &[FileFormat::Ers]);
        let s = std::fs::read_to_string(d.join("ERS_Five-Points-20261017.csv")).unwrap();
        assert_eq!(s.matches("ProductLine").count(), 1);
        assert_eq!(s.lines().count(), 3);
        assert_eq!(std::fs::read_to_string(d.join("Five-Points-20261017.csv")).unwrap().lines().count(), 2);
    }

    #[test]
    fn nagy_continuous_one_row_per_reading() {
        let d = tmpdir("nagyc");
        let info = RunInfo { line: "VG3".into(), run_number: "".into(), run_minute: "".into(), test_name: "Continuous Measure".into(), offset_nagy: 0.0 };
        let recs = vec![
            Record { point: "Sample_01".into(), ers: "Not Stored".into(), value: 12.5, time: t(8, 0, 0) },
            Record { point: "Sample_02".into(), ers: "Not Stored".into(), value: 12.6, time: t(8, 0, 5) },
        ];
        let b = nagy_block(&d, &info, &recs, t(8, 0, 0)).unwrap();
        assert_eq!(b.file_name, "Continuous-20261017.csv");
        assert!(b.header.ends_with(",Offset,Data"));
        assert_eq!(b.lines, vec!["VG3,17/10/2026 08:00:00,,,0,12.5", "VG3,17/10/2026 08:00:05,,,0,12.6"]);
    }

    #[test]
    fn fpp_storage_dir_and_prefix() {
        let save = tmpdir("fpps");
        let rec = tmpdir("fppr");
        let info = RunInfo { line: "OT1".into(), run_number: "5497".into(), run_minute: "120".into(), test_name: "3 Points".into(), offset_nagy: 0.0 };
        let recs = vec![Record { point: "Left".into(), ers: "Ohms_Val_LE".into(), value: 45.3, time: t(1, 0, 0) }];
        let b = fpp_block(&save, &rec, &info, &recs, t(1, 0, 0)).unwrap();
        assert_eq!(b.dir, save);
        assert_eq!(b.lines, vec!["OT1,5497,120,Left,Ohms_Val_LE,45.3"]);
        let tg = b.targets(&[FileFormat::Sequoia]);
        assert_eq!(tg[0].path, save.join("Sequoia_20261017.csv"));
        let gage = vec![Record { point: "Sample 1".into(), ers: "Not Stored".into(), value: 9.9, time: t(1, 0, 0) }];
        let info2 = RunInfo { test_name: "Gage R and R".into(), ..info };
        assert_eq!(fpp_block(&save, &rec, &info2, &gage, t(1, 0, 0)).unwrap().dir, rec);
    }

    #[test]
    fn pending_when_locked_then_retry() {
        let d = tmpdir("pend");
        let mem = crate::secure::MemoryStore::default();
        let store = std::sync::Arc::new(crate::secure::SecureStore::new(Box::new(mem.clone())));
        // Giả lập file bị khóa: đặt một THƯ MỤC trùng tên file đích
        let blocker = d.join("ERS_Five-Points-20261017.csv");
        std::fs::create_dir_all(&blocker).unwrap();
        let info = RunInfo { line: "VG1".into(), run_number: "1".into(), run_minute: "2".into(), test_name: "5 Points".into(), offset_nagy: 0.0 };
        let b = nagy_block(&d, &info, &five_point_records(), t(1, 0, 0)).unwrap();
        let mut ps = PendingStore::load(store.clone()).0;
        let (ok, failed) = ps.write_block(&b, &[FileFormat::Default, FileFormat::Ers]);
        assert_eq!(ok.len(), 1);
        assert_eq!(failed.len(), 1);
        assert!(mem.map.lock().unwrap().contains_key(PENDING_NAME), "hàng chờ phải được lưu");
        // lượt sau vào cùng file cũng phải xếp hàng sau
        let (_, failed2) = ps.write_block(&b, &[FileFormat::Ers]);
        assert_eq!(failed2.len(), 1);
        assert_eq!(ps.items.len(), 2);
        // mở lại chương trình: đọc được hàng chờ
        let mut ps2 = PendingStore::load(store.clone()).0;
        assert_eq!(ps2.items.len(), 2);
        assert!(ps2.retry().is_empty());
        std::fs::remove_dir(&blocker).unwrap();
        let done = ps2.retry();
        assert_eq!(done.len(), 2);
        assert!(ps2.items.is_empty());
        assert!(!mem.map.lock().unwrap().contains_key(PENDING_NAME));
        let s = std::fs::read_to_string(&blocker).unwrap();
        assert_eq!(s.matches("ProductLine").count(), 1);
        assert_eq!(s.lines().count(), 3);
    }

    #[test]
    fn pending_tampered_is_discarded() {
        let d = tmpdir("pendt");
        std::fs::create_dir_all(d.join("Five-Points-20261017.csv")).unwrap(); // khóa file đích
        let mem = crate::secure::MemoryStore::default();
        let store = std::sync::Arc::new(crate::secure::SecureStore::new(Box::new(mem.clone())));
        let info = RunInfo { line: "VG1".into(), run_number: "1".into(), run_minute: "2".into(), test_name: "5 Points".into(), offset_nagy: 0.0 };
        let b = nagy_block(&d, &info, &five_point_records(), t(1, 0, 0)).unwrap();
        PendingStore::load(store.clone()).0.write_block(&b, &[FileFormat::Default]);
        // sửa số đo đang chờ: 17.02 -> 18.02
        {
            let mut m = mem.map.lock().unwrap();
            let v = m.get_mut(PENDING_NAME).unwrap();
            *v = String::from_utf8_lossy(v).replace("17.02", "18.02").into_bytes();
        }
        let (ps, warn) = PendingStore::load(store);
        assert!(ps.items.is_empty());
        assert!(warn.unwrap().contains("discarded"));
    }
}

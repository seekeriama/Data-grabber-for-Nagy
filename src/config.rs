//! Cài đặt chương trình (thay cho My.Settings của bản gốc), gồm cả hệ số hiệu chỉnh 4PP.
//! Lưu trong kho có chữ ký (`secure.rs`), không tạo file cạnh exe.

use crate::catalog::Device;
use crate::csvout::FileFormat;
use crate::secure::{LoadError, SecureStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SETTINGS_NAME: &str = "Settings";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// DefaultSavePath của bản gốc (mặc định C:\Data\). File CSV và calibration.txt nằm ở đây.
    pub save_path: String,
    /// Thư mục ghi kết quả 4PP không lưu ERS (Continuous / Gage R&R / Calibration). Bản gốc cố định C:\Records\.
    pub records_path: String,
    /// Line chọn lần cuối (LineID).
    pub line_id: String,
    /// Thiết bị kết nối lần cuối (DeviceConnected), dùng để dò trước khi khởi động.
    pub device: Device,
    /// Offset cộng vào kết quả Nagy (OffsetNagy).
    pub offset_nagy: f64,
    /// Định dạng tên file CSV: Default / ERS / Sequoia (ít nhất 1, tối đa 3).
    pub file_formats: Vec<FileFormat>,
    /// Hệ số đa thức 4PP (trước đây ở calibration.txt)
    pub poly: Poly,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            save_path: "C:\\Data\\".into(),
            records_path: "C:\\Records\\".into(),
            line_id: String::new(),
            device: Device::FourPoint,
            offset_nagy: 0.0,
            file_formats: vec![FileFormat::Default],
            poly: Poly::default(),
        }
    }
}

impl Config {
    pub fn normalized(mut self) -> Self {
        self.file_formats = FileFormat::normalize(&self.file_formats);
        if self.save_path.trim().is_empty() {
            self.save_path = Config::default().save_path;
        }
        if self.records_path.trim().is_empty() {
            self.records_path = Config::default().records_path;
        }
        if !self.offset_nagy.is_finite() {
            self.offset_nagy = 0.0;
        }
        self
    }

    /// Đọc cài đặt. Trả kèm cảnh báo nếu dữ liệu đã bị sửa ngoài chương trình.
    pub fn load(store: &SecureStore) -> (Config, Option<String>) {
        match store.load(SETTINGS_NAME) {
            Ok(Some(b)) => match serde_json::from_slice::<Config>(&b) {
                Ok(c) => (c.normalized(), None),
                Err(_) => (Config::default(), Some("Settings could not be read and were reset to defaults.".into())),
            },
            Ok(None) => (Config::default(), None),
            Err(LoadError::Tampered) => (
                Config::default(),
                Some(
                    "The saved settings were changed outside this program and have been rejected.\n\
                     All settings were reset to defaults (Nagy offset = 0).\n\
                     Please set the save folder, CSV format and offset again (password required)."
                        .into(),
                ),
            ),
            Err(LoadError::Io(e)) => (Config::default(), Some(format!("Could not read settings: {e}"))),
        }
    }

    pub fn save(&self, store: &SecureStore) -> std::io::Result<()> {
        let b = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        store.save(SETTINGS_NAME, &b)
    }

    pub fn save_dir(&self) -> PathBuf {
        PathBuf::from(&self.save_path)
    }
}

/// Hệ số đa thức hiệu chỉnh 4PP: R' = A·R² + B·R + C khi Low ≤ R ≤ High.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Poly {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub low: f64,
    pub high: f64,
}

impl Default for Poly {
    /// Nội dung mặc định khi chưa có file: "0 1 0 0 600"
    fn default() -> Self {
        Poly { a: 0.0, b: 1.0, c: 0.0, low: 0.0, high: 600.0 }
    }
}

impl Poly {
    /// Đọc "A B C Low High". Chấp nhận BOM UTF-8 (bản VB gốc ghi file có BOM),
    /// xuống dòng / tab / ";" làm dấu phân cách, và dấu phẩy thập phân ("0,5").
    pub fn parse(s: &str) -> Option<Poly> {
        let s = s.trim_start_matches('\u{feff}');
        let v: Vec<f64> = s
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter(|x| !x.is_empty())
            .map(|x| if x.contains(',') && !x.contains('.') { x.replace(',', ".") } else { x.to_string() })
            .map(|x| x.parse::<f64>())
            .collect::<Result<_, _>>()
            .ok()?;
        if v.len() < 5 {
            return None;
        }
        Some(Poly { a: v[0], b: v[1], c: v[2], low: v[3], high: v[4] })
    }

    pub fn to_file_string(&self) -> String {
        use crate::util::fmt_num;
        format!("{} {} {} {} {}", fmt_num(self.a), fmt_num(self.b), fmt_num(self.c), fmt_num(self.low), fmt_num(self.high))
    }

    pub fn apply(&self, r: f64) -> f64 {
        if r >= self.low && r <= self.high {
            self.a * r * r + self.b * r + self.c
        } else {
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_roundtrip() {
        let p = Poly::parse("0 1 0 0 600").unwrap();
        assert_eq!(p, Poly::default());
        assert_eq!(p.apply(45.0), 45.0);
        let q = Poly { a: 0.001, b: 0.95, c: 0.2, low: 10.0, high: 100.0 };
        assert!((q.apply(50.0) - (2.5 + 47.5 + 0.2)).abs() < 1e-9);
        assert_eq!(q.apply(5.0), 5.0);
        assert_eq!(Poly::parse(&q.to_file_string()).unwrap(), q);
        // file do bản VB gốc ghi: có BOM UTF-8
        assert_eq!(Poly::parse("\u{feff}0 1 0 0 600"), Some(Poly::default()));
        assert_eq!(Poly::parse("0\r\n1\r\n0\r\n0\r\n600\r\n"), Some(Poly::default()));
        assert_eq!(Poly::parse("0,5 1 0 0 600").unwrap().a, 0.5);
    }

    #[test]
    fn config_in_secure_store_and_tamper() {
        use crate::secure::MemoryStore;
        let mem = MemoryStore::default();
        let st = SecureStore::new(Box::new(mem.clone()));
        let (c, w) = Config::load(&st);
        assert_eq!((c.clone(), w), (Config::default(), None));
        let c2 = Config { offset_nagy: -0.4, poly: Poly { a: 0.0, b: 0.98, c: 0.1, low: 0.0, high: 600.0 }, ..c };
        c2.save(&st).unwrap();
        assert_eq!(Config::load(&st), (c2.clone(), None));
        // sửa offset trực tiếp trong kho: -0.4 -> -0.9
        {
            let mut m = mem.map.lock().unwrap();
            let v = m.get_mut(SETTINGS_NAME).unwrap();
            let s = String::from_utf8_lossy(v).replace("-0.4", "-0.9");
            *v = s.into_bytes();
        }
        let (c3, w3) = Config::load(&st);
        assert_eq!(c3.offset_nagy, 0.0);
        assert!(w3.unwrap().contains("changed outside"));
    }

    #[test]
    fn config_defaults_and_formats() {
        let c: Config = serde_json::from_str(r#"{"file_formats":[]}"#).unwrap();
        assert_eq!(c.normalized().file_formats, vec![FileFormat::Default]);
        let c: Config = serde_json::from_str(r#"{"device":"Nagy","offset_nagy":-0.4,"file_formats":["Sequoia","ERS","ERS"]}"#).unwrap();
        let c = c.normalized();
        assert_eq!(c.device, Device::Nagy);
        assert_eq!(c.file_formats, vec![FileFormat::Ers, FileFormat::Sequoia]);
        assert_eq!(c.save_path, "C:\\Data\\");
    }
}

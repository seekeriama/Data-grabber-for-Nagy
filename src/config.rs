//! Cài đặt chương trình (thay cho My.Settings của bản gốc) và file calibration.txt.

use crate::catalog::Device;
use crate::csvout::FileFormat;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

    pub fn load(path: &Path) -> Config {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Config>(&s).ok())
            .unwrap_or_default()
            .normalized()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let s = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, s)
    }

    pub fn save_dir(&self) -> PathBuf {
        PathBuf::from(&self.save_path)
    }

    pub fn calibration_path(&self) -> PathBuf {
        self.save_dir().join("calibration.txt")
    }
}

/// Hệ số đa thức hiệu chỉnh 4PP: R' = A·R² + B·R + C khi Low ≤ R ≤ High.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    pub fn parse(s: &str) -> Option<Poly> {
        let v: Vec<f64> = s.split_whitespace().map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
        if v.len() < 5 {
            return None;
        }
        Some(Poly { a: v[0], b: v[1], c: v[2], low: v[3], high: v[4] })
    }

    pub fn to_file_string(&self) -> String {
        use crate::util::fmt_num;
        format!("{} {} {} {} {}", fmt_num(self.a), fmt_num(self.b), fmt_num(self.c), fmt_num(self.low), fmt_num(self.high))
    }

    /// Như StartForm_Load: chưa có file thì tạo thư mục + file "0 1 0 0 600".
    pub fn load_or_create(path: &Path) -> (Poly, Option<String>) {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, "0 1 0 0 600");
        }
        match std::fs::read_to_string(path) {
            Ok(s) => match Poly::parse(&s) {
                Some(p) => (p, None),
                None => (Poly::default(), Some(format!("calibration.txt không đúng dạng \"A B C Low High\": {}", path.display()))),
            },
            Err(e) => (Poly::default(), Some(format!("Không đọc được {}: {e}", path.display()))),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_file_string())
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

//! Danh mục Line / Product / Test / điểm đo.
//!
//! Dữ liệu lấy nguyên từ hai hàm `Load4PPDataSet` và `LoadNagyDataSet` của chương trình gốc
//! (ghi cứng trong code). File `catalog.json` được nhúng vào exe lúc build; chương trình
//! không đọc danh mục từ file bên ngoài (muốn thêm sản phẩm thì sửa catalog.json rồi build lại).

use serde::{Deserialize, Serialize};

pub const EMBEDDED_JSON: &str = include_str!("catalog.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Device {
    #[serde(rename = "4-Point Probe")]
    FourPoint,
    #[serde(rename = "Nagy")]
    Nagy,
}

impl Device {
    pub fn label(self) -> &'static str {
        match self {
            Device::FourPoint => "4-Point Probe",
            Device::Nagy => "Nagy",
        }
    }
    pub fn other(self) -> Device {
        match self {
            Device::FourPoint => Device::Nagy,
            Device::Nagy => Device::FourPoint,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Line {
    pub name: String,
    pub allow_host_data: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Product {
    pub name: String,
    pub spec_upper: f64,
    pub spec_lower: f64,
    pub spec_target: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProductTest {
    pub product: String,
    pub order: i64,
    pub test: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestDef {
    pub name: String,
    pub sample_size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestItem {
    pub test: String,
    pub order: i64,
    pub point: String,
    pub ers: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceCatalog {
    pub lines: Vec<Line>,
    pub products: Vec<Product>,
    pub product_tests: Vec<ProductTest>,
    pub tests: Vec<TestDef>,
    pub test_items: Vec<TestItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub nagy: DeviceCatalog,
    pub four_point: DeviceCatalog,
}

/// Sample Size và hệ số hiệu chỉnh của máy 4 điểm (GetNewMeas).
/// Bản gốc có 4 mục: Float Ribbon 1.0 / 4x4 0.9981 / 12x12 0.9997 / 3x3 0.9966.
/// Theo yêu cầu chỉ còn một mục "10cm x 10cm", dùng hệ số của mẫu 4x4 inch (≈ 10.2 cm).
pub const SAMPLE_SIZES: [(&str, f64); 1] = [("10cm x 10cm", 0.9981)];

/// Mọi test đều dùng mục duy nhất "10cm x 10cm".
pub fn sample_size_index(_name: &str) -> Option<usize> {
    Some(0)
}

/// Các test không cần nhập Run Number / Run Minute để bắt đầu (BeginTestCheck).
pub const NO_RUN_INFO_TESTS: [&str; 6] = [
    "Gage R and R",
    "Continuous Measure",
    "Calibration 6 Ohm Si Wafer",
    "Calibration 16 Ohm Si Wafer",
    "Calibration 60 Ohm Si Wafer",
    "Calibration 500 Ohm Si Wafer",
];

pub const CONTINUOUS: &str = "Continuous Measure";

impl Catalog {
    pub fn embedded() -> Catalog {
        serde_json::from_str(EMBEDDED_JSON).expect("catalog.json nhúng sẵn bị lỗi")
    }

    pub fn for_device(&self, d: Device) -> &DeviceCatalog {
        match d {
            Device::FourPoint => &self.four_point,
            Device::Nagy => &self.nagy,
        }
    }
}

fn same(a: &str, b: &str) -> bool {
    // DataView.RowFilter của DataSet mặc định không phân biệt hoa thường
    a.eq_ignore_ascii_case(b)
}

impl DeviceCatalog {
    pub fn line(&self, name: &str) -> Option<&Line> {
        self.lines.iter().find(|l| same(&l.name, name))
    }

    pub fn product(&self, name: &str) -> Option<&Product> {
        self.products.iter().find(|p| same(&p.name, name))
    }

    /// Test của sản phẩm, sắp theo TestOrder (ProductSelection_SelectedIndexChanged).
    pub fn tests_for_product(&self, product: &str) -> Vec<String> {
        let mut v: Vec<&ProductTest> = self.product_tests.iter().filter(|t| same(&t.product, product)).collect();
        v.sort_by_key(|t| t.order); // sort ổn định giữ thứ tự nhập khi trùng order
        v.into_iter().map(|t| t.test.clone()).collect()
    }

    pub fn test(&self, name: &str) -> Option<&TestDef> {
        self.tests.iter().find(|t| same(&t.name, name))
    }

    /// (PointName, ERSName) sắp theo PointOrder (TestTypeSelection_SelectedIndexChanged).
    pub fn items_for_test(&self, test: &str) -> Vec<(String, String)> {
        let mut v: Vec<&TestItem> = self.test_items.iter().filter(|t| same(&t.test, test)).collect();
        v.sort_by_key(|t| t.order);
        v.into_iter().map(|t| (t.point.clone(), t.ers.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_matches_original_counts() {
        let c = Catalog::embedded();
        // Số dòng đếm được từ IL của chương trình gốc
        assert_eq!(c.nagy.lines.len(), 7); // 5 của bản gốc + JB2, LK1
        assert!(c.nagy.line("JB2").is_some() && c.nagy.line("LK1").is_some());
        assert_eq!(c.nagy.products.len(), 12); // 11 của bản gốc + TZ18
        assert_eq!(c.nagy.product_tests.len(), 14);
        assert_eq!(c.nagy.tests.len(), 3);
        assert_eq!(c.nagy.test_items.len(), 9);
        assert_eq!(c.four_point.lines.len(), 9);
        assert_eq!(c.four_point.products.len(), 31);
        assert_eq!(c.four_point.product_tests.len(), 78);
        assert_eq!(c.four_point.tests.len(), 14);
        assert_eq!(c.four_point.test_items.len(), 220);
    }

    #[test]
    fn nagy_five_points() {
        let c = Catalog::embedded();
        assert_eq!(c.nagy.tests_for_product("T203"), vec!["5 Points", "3 Points"]);
        assert_eq!(c.nagy.tests_for_product("TZ18"), vec!["5 Points", "3 Points"]);
        let items = c.nagy.items_for_test("5 Points");
        let names: Vec<_> = items.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(names, ["Left Edge", "Left Center", "Center", "Right Center", "Right Edge"]);
        assert_eq!(c.nagy.test("5 Points").unwrap().sample_size, "4x4");
    }

    #[test]
    fn fpp_order_and_host_line() {
        let c = Catalog::embedded();
        // TE85: Post Heating có TestOrder 0 nên đứng trước 30 Point 5 Zone
        assert_eq!(c.four_point.tests_for_product("TE85"), vec!["Post Heating", "30 Point 5 Zone", "Continuous Measure"]);
        assert!(c.four_point.line("OT1").unwrap().allow_host_data);
        assert_eq!(sample_size_index("Float Ribbon"), Some(0));
        assert_eq!(SAMPLE_SIZES[0].0, "10cm x 10cm");
    }
}

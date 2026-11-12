//! Nơi lưu cài đặt có chống sửa đổi.
//!
//! - Không tạo file cạnh exe. Trên Windows, dữ liệu nằm trong Registry
//!   `HKEY_CURRENT_USER\Software\Pilkington\FourPointProbe` (dạng nhị phân).
//! - Mỗi giá trị kèm chữ ký HMAC-SHA256 với khóa nằm trong chương trình. Sửa trực tiếp
//!   trong Registry (ví dụ đổi Offset bằng regedit) làm chữ ký sai: chương trình phát hiện,
//!   bỏ giá trị đó và báo cho người dùng.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::io;

type HmacSha256 = Hmac<Sha256>;

/// Khóa ký. Đổi khóa này sẽ làm mọi cài đặt cũ bị coi là không hợp lệ.
const SIGN_KEY: &[u8] = b"FourPointProbe/NSG-VGI/2026/settings-integrity-v1#Q7c!pX";
const MAGIC: &[u8; 4] = b"FPP1";
const TAG_LEN: usize = 32;

#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// Dữ liệu có nhưng chữ ký sai (bị sửa ngoài chương trình hoặc hỏng)
    Tampered,
    Io(String),
}

fn tag(name: &str, payload: &[u8]) -> [u8; TAG_LEN] {
    let mut mac = HmacSha256::new_from_slice(SIGN_KEY).expect("HMAC nhận khóa mọi độ dài");
    mac.update(MAGIC);
    mac.update(name.as_bytes());
    mac.update(&[0]);
    mac.update(payload);
    mac.finalize().into_bytes().into()
}

/// MAGIC + dữ liệu + chữ ký. Chữ ký gắn với tên giá trị, nên không chép được giá trị này sang tên khác.
pub fn seal(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len() + TAG_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(payload);
    out.extend_from_slice(&tag(name, payload));
    out
}

pub fn open(name: &str, blob: &[u8]) -> Result<Vec<u8>, LoadError> {
    if blob.len() < 4 + TAG_LEN || &blob[..4] != MAGIC {
        return Err(LoadError::Tampered);
    }
    let payload = &blob[4..blob.len() - TAG_LEN];
    let got = &blob[blob.len() - TAG_LEN..];
    let mut mac = HmacSha256::new_from_slice(SIGN_KEY).expect("HMAC nhận khóa mọi độ dài");
    mac.update(MAGIC);
    mac.update(name.as_bytes());
    mac.update(&[0]);
    mac.update(payload);
    // so sánh thời gian hằng
    mac.verify_slice(got).map_err(|_| LoadError::Tampered)?;
    Ok(payload.to_vec())
}

/// Kho lưu giá trị nhị phân theo tên.
pub trait RawStore: Send + Sync {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>>;
    fn write(&self, name: &str, data: &[u8]) -> io::Result<()>;
    fn remove(&self, name: &str) -> io::Result<()>;
    fn describe(&self) -> String;
}

/// Đọc / ghi có kiểm tra chữ ký.
pub struct SecureStore {
    raw: Box<dyn RawStore>,
}

impl SecureStore {
    pub fn new(raw: Box<dyn RawStore>) -> Self {
        SecureStore { raw }
    }

    /// Ok(None): chưa có (lần chạy đầu). Err(Tampered): có nhưng bị sửa.
    pub fn load(&self, name: &str) -> Result<Option<Vec<u8>>, LoadError> {
        match self.raw.read(name) {
            Ok(None) => Ok(None),
            Ok(Some(b)) => open(name, &b).map(Some),
            Err(e) => Err(LoadError::Io(e.to_string())),
        }
    }

    pub fn save(&self, name: &str, payload: &[u8]) -> io::Result<()> {
        self.raw.write(name, &seal(name, payload))
    }

    pub fn remove(&self, name: &str) -> io::Result<()> {
        self.raw.remove(name)
    }

    pub fn describe(&self) -> String {
        self.raw.describe()
    }

    /// Kho mặc định: Registry trên Windows; nơi khác (chỉ để phát triển / chạy thử) dùng bộ nhớ.
    pub fn platform_default() -> SecureStore {
        #[cfg(windows)]
        {
            SecureStore::new(Box::new(registry::RegistryStore::new("Software\\Pilkington\\FourPointProbe")))
        }
        #[cfg(not(windows))]
        {
            SecureStore::new(Box::new(MemoryStore::default()))
        }
    }
}

/// Kho trong bộ nhớ (kiểm thử, chạy thử ngoài Windows).
#[derive(Default, Clone)]
pub struct MemoryStore {
    pub map: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>,
}

impl RawStore for MemoryStore {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.map.lock().unwrap().get(name).cloned())
    }
    fn write(&self, name: &str, data: &[u8]) -> io::Result<()> {
        self.map.lock().unwrap().insert(name.to_string(), data.to_vec());
        Ok(())
    }
    fn remove(&self, name: &str) -> io::Result<()> {
        self.map.lock().unwrap().remove(name);
        Ok(())
    }
    fn describe(&self) -> String {
        "memory".into()
    }
}

#[cfg(windows)]
pub mod registry {
    use super::RawStore;
    use std::io;
    use winreg::enums::{HKEY_CURRENT_USER, REG_BINARY};
    use winreg::{RegKey, RegValue};

    pub struct RegistryStore {
        path: String,
    }

    impl RegistryStore {
        pub fn new(path: &str) -> Self {
            RegistryStore { path: path.to_string() }
        }
    }

    impl RawStore for RegistryStore {
        fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let key = match hkcu.open_subkey(&self.path) {
                Ok(k) => k,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            match key.get_raw_value(name) {
                Ok(v) => Ok(Some(v.bytes)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        fn write(&self, name: &str, data: &[u8]) -> io::Result<()> {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let (key, _) = hkcu.create_subkey(&self.path)?;
            key.set_raw_value(name, &RegValue { bytes: data.to_vec(), vtype: REG_BINARY })
        }

        fn remove(&self, name: &str) -> io::Result<()> {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            match hkcu.open_subkey_with_flags(&self.path, winreg::enums::KEY_ALL_ACCESS) {
                Ok(k) => match k.delete_value(name) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                    _ => Ok(()),
                },
                Err(_) => Ok(()),
            }
        }

        fn describe(&self) -> String {
            format!("HKEY_CURRENT_USER\\{}", self.path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_and_tamper() {
        let b = seal("Settings", b"{\"offset_nagy\":-0.4}");
        assert_eq!(open("Settings", &b).unwrap(), b"{\"offset_nagy\":-0.4}");
        // đổi một byte (vd -0.4 -> -0.9) => phát hiện
        let mut t = b.clone();
        let i = t.iter().position(|&c| c == b'4').unwrap();
        t[i] = b'9';
        assert_eq!(open("Settings", &t), Err(LoadError::Tampered));
        // chép sang tên khác => phát hiện
        assert_eq!(open("Pending", &b), Err(LoadError::Tampered));
        assert_eq!(open("Settings", b"garbage"), Err(LoadError::Tampered));
    }

    #[test]
    fn store_roundtrip() {
        let mem = MemoryStore::default();
        let st = SecureStore::new(Box::new(mem.clone()));
        assert_eq!(st.load("Settings"), Ok(None));
        st.save("Settings", b"abc").unwrap();
        assert_eq!(st.load("Settings"), Ok(Some(b"abc".to_vec())));
        mem.map.lock().unwrap().get_mut("Settings").unwrap()[5] ^= 1;
        assert_eq!(st.load("Settings"), Err(LoadError::Tampered));
    }
}

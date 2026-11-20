//! Dò thiết bị trên các cổng COM (fStart.StartForm_Load / Start4PPConnection / StartNagyConnection).

use crate::catalog::Device;
use crate::keithley;
use crate::link::Link;
use crate::nagy;

pub struct Connected {
    pub device: Device,
    pub link: Box<dyn Link>,
    /// Nhãn Device Model (Keithley: hai phần đầu của *IDN?; Nagy: trống)
    pub model: String,
    pub port: String,
}

/// Thử một loại thiết bị trên lần lượt các cổng.
pub fn try_device(device: Device, ports: &[String]) -> Result<Connected, String> {
    let mut errors = vec![];
    for p in ports {
        match device {
            Device::Nagy => match nagy::open(p) {
                Ok(mut l) => match nagy::handshake(&mut l) {
                    Ok(true) => return Ok(Connected { device, link: Box::new(l), model: String::new(), port: p.clone() }),
                    Ok(false) => {}
                    Err(e) => errors.push(format!("{p}: {e}")),
                },
                Err(e) => errors.push(format!("{p}: {e}")),
            },
            Device::FourPoint => match keithley::open(p) {
                Ok(mut l) => match keithley::idn(&mut l) {
                    Ok(s) if s.contains("KEITHLEY") => {
                        return Ok(Connected { device, link: Box::new(l), model: keithley::model_label(&s), port: p.clone() })
                    }
                    _ => {}
                },
                Err(e) => errors.push(format!("{p}: {e}")),
            },
        }
    }
    let what = match device {
        Device::Nagy => "Could not find the Nagy Connection!",
        Device::FourPoint => "Could not find the Keithley Connection!",
    };
    if errors.is_empty() {
        Err(what.to_string())
    } else {
        Err(format!("{what}\n{}", errors.join("\n")))
    }
}

/// Thử thiết bị dùng lần trước, không được thì thử loại còn lại.
pub fn connect(preferred: Device, ports: &[String]) -> Result<Connected, String> {
    if ports.is_empty() {
        return Err("No COM ports detected,\nPlease check that a device is connected!".into());
    }
    match try_device(preferred, ports) {
        Ok(c) => Ok(c),
        Err(e1) => try_device(preferred.other(), ports).map_err(|e2| format!("{e1}\n\n{e2}")),
    }
}

/// Chế độ chạy thử không cần máy.
pub fn simulated(device: Device) -> Connected {
    use crate::sim::{FakeKeithley, FakeNagy};
    match device {
        Device::FourPoint => {
            let mut k = FakeKeithley::new(vec![45.2, 45.6, 45.9, 45.4, 45.1, 46.0]);
            k.reads_up = 4;
            k.noise = 0.0008;
            let model = keithley::model_label("KEITHLEY INSTRUMENTS INC.,MODEL 2400 (SIMULATED),0,0");
            Connected { device, link: Box::new(k), model, port: "SIM".into() }
        }
        Device::Nagy => Connected { device, link: Box::new(FakeNagy::auto(std::time::Duration::from_secs(3))), model: String::new(), port: "SIM".into() },
    }
}

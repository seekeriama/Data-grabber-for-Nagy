//! Kiểm tra nhanh một cổng COM: cargo run --example probe_port -- COM3 nagy|4pp
use fpp::catalog::Device;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let port = a.get(1).cloned().unwrap_or_default();
    let dev = if a.get(2).map(|s| s == "nagy").unwrap_or(false) { Device::Nagy } else { Device::FourPoint };
    match fpp::connect::try_device(dev, &[port]) {
        Ok(mut c) => {
            println!("OK {} model={:?} port={}", c.device.label(), c.model, c.port);
            if dev == Device::FourPoint {
                let m = fpp::keithley::read_ohms(c.link.as_mut()).unwrap();
                println!("read_ohms = {}", m.ohms);
            } else {
                let mut r = fpp::nagy::Reader::default();
                let t0 = std::time::Instant::now();
                while t0.elapsed().as_secs() < 3 {
                    if let Some(v) = r.poll(c.link.as_mut()).unwrap() {
                        println!("nagy reading = {v}");
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        Err(e) => println!("FAIL {e}"),
    }
}

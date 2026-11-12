//! Giao diện FourPointProbe (egui). Bố cục theo bản gốc: màn hình Setup, màn hình đo,
//! hộp thoại Calibration; thêm menu "CSV Format" chọn Default / ERS / Sequoia.

#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText};
use fpp::catalog::{sample_size_index, Catalog, Device, NO_RUN_INFO_TESTS, SAMPLE_SIZES};
use fpp::config::{Config, Poly};
use fpp::connect::{self, Connected};
use fpp::csvout::{fpp_block, nagy_block, FileFormat, PendingStore, Record, RunInfo};
use fpp::engine::{self, Cmd, Evt, Session, Tone};
use fpp::hostdata::{self, HostData};
use fpp::link::Link;
use fpp::secure::{MemoryStore, SecureStore};
use std::sync::Arc;
use fpp::util::{fmt_num, round2};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Mật khẩu để đổi Offset của Nagy và hệ số 3 Point Calibration của 4PP.
const OFFSET_PASSWORD: &str = "ZZCARROT";

/// Kích thước cửa sổ như bản cũ: Setup Screen nhỏ gọn, MeasureForm rộng.
const SETUP_SIZE: [f32; 2] = [830.0, 420.0];
/// MeasureForm: cột kết quả 300 px (bằng ~50% trước đây) + khung thông tin / biểu đồ 755 px (giữ nguyên).
const RESULTS_W: f32 = 300.0;
const INFO_W: f32 = 755.0;
const MEASURE_SIZE: [f32; 2] = [RESULTS_W + 12.0 + INFO_W + 16.0, 600.0];

fn tone_colors(t: Tone) -> (Color32, Color32) {
    // (chữ, nền)
    match t {
        Tone::Connected => (Color32::WHITE, Color32::from_rgb(139, 0, 0)),
        Tone::Idle => (Color32::BLACK, Color32::from_rgb(211, 211, 211)),
        Tone::Reading => (Color32::BLACK, Color32::from_rgb(144, 238, 144)),
        Tone::Measuring => (Color32::BLACK, Color32::from_rgb(255, 255, 0)),
        Tone::Done => (Color32::WHITE, Color32::from_rgb(0, 128, 0)),
        Tone::Error => (Color32::WHITE, Color32::from_rgb(220, 0, 0)),
    }
}

enum Conn {
    Connecting(Receiver<Result<Connected, String>>),
    Ready { device: Device, link: Option<Box<dyn Link>>, model: String, port: String },
    Failed(String),
}

#[derive(Default)]
struct Setup {
    line: String,
    product: String,
    host_product: String,
    tests: Vec<String>,
    test: Option<String>,
    run_number: String,
    run_minute: String,
    meas_per_sample: String,
    sample_size: Option<usize>,
    items: Vec<(String, String)>,
    upper_spec: f64,
    host_rx: Option<Receiver<Result<HostData, String>>>,
}

struct Measure {
    session: Session,
    records: Vec<Record>,
    status: (String, Tone),
    live: (String, Tone),
    sample_name: String,
    submit: bool,
    tx: Sender<Cmd>,
    rx: Receiver<Evt>,
    handle: Option<JoinHandle<Box<dyn Link>>>,
    closing: bool,
    comm_error: Option<String>,
    confirm_remeasure: Option<usize>,
    confirm_exit: bool,
}

enum CalibDlg {
    Poly { a: String, b: String, c: String, low: String, high: String, password: String },
    Nagy { offset: String, password: String },
}

struct App {
    cfg: Config,
    /// Kho cài đặt có chữ ký (Registry trên Windows), không tạo file
    store: Arc<SecureStore>,
    catalog: Catalog,
    simulate: Option<Device>,
    conn: Conn,
    setup: Setup,
    measure: Option<Measure>,
    pending: PendingStore,
    last_retry: Instant,
    message: Option<(String, String)>,
    calib: Option<CalibDlg>,
    folder_edit: Option<String>,
    save_note: Option<(String, bool)>,
    /// Màn hình đang hiển thị lần vẽ trước (để đổi kích thước / tiêu đề cửa sổ)
    shown_measure: Option<bool>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, simulate: Option<Device>) -> App {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        install_fonts(&cc.egui_ctx);
        // Chạy thử: cài đặt riêng trong bộ nhớ và ghi CSV vào thư mục tạm,
        // để không đụng tới cài đặt thật và thư mục dữ liệu thật.
        let store = Arc::new(if simulate.is_some() { SecureStore::new(Box::new(MemoryStore::default())) } else { SecureStore::platform_default() });
        let (mut cfg, cfg_warn) = Config::load(&store);
        if simulate.is_some() {
            let tmp = std::env::temp_dir().join("FourPointProbe-SIM");
            cfg.save_path = format!("{}{}", tmp.join("Data").display(), std::path::MAIN_SEPARATOR);
            cfg.records_path = format!("{}{}", tmp.join("Records").display(), std::path::MAIN_SEPARATOR);
        }
        let (pending, pend_warn) = PendingStore::load(store.clone());
        let warn: Vec<String> = [cfg_warn, pend_warn].into_iter().flatten().collect();
        let mut app = App {
            cfg,
            store,
            catalog: Catalog::embedded(),
            simulate,
            conn: Conn::Failed(String::new()),
            setup: Setup::default(),
            measure: None,
            pending,
            last_retry: Instant::now(),
            message: if warn.is_empty() { None } else { Some(("Settings".to_string(), warn.join("\n\n"))) },
            calib: None,
            folder_edit: None,
            save_note: None,
            shown_measure: None,
        };
        if !app.pending.items.is_empty() {
            app.save_note = Some((format!("{} CSV write(s) waiting from last session; retrying...", app.pending.items.len()), true));
        }
        app.start_connect(simulate.unwrap_or(app.cfg.device));
        app
    }

    fn save_cfg(&mut self) {
        if let Err(e) = self.cfg.save(&self.store) {
            self.message = Some(("Settings".into(), format!("Could not save settings to {}:\n{e}", self.store.describe())));
        }
    }

    fn device(&self) -> Option<Device> {
        match &self.conn {
            Conn::Ready { device, .. } => Some(*device),
            _ => None,
        }
    }

    fn start_connect(&mut self, preferred: Device) {
        self.conn = Conn::Failed(String::new()); // đóng cổng cũ trước khi dò lại
        let (tx, rx) = channel();
        let sim = self.simulate.map(|_| preferred);
        std::thread::spawn(move || {
            let r = match sim {
                Some(d) => Ok(connect::simulated(d)),
                None => connect::connect(preferred, &fpp::link::list_ports()),
            };
            let _ = tx.send(r);
        });
        self.conn = Conn::Connecting(rx);
    }

    // ---------------- Setup logic (fStart) ----------------

    fn reset_setup(&mut self) {
        self.setup = Setup::default();
        let line = self.cfg.line_id.clone();
        if let Some(d) = self.device() {
            if self.catalog.for_device(d).line(&line).is_some() {
                self.set_line(line);
            }
        }
    }

    fn set_line(&mut self, line: String) {
        let Some(d) = self.device() else { return };
        self.setup.line = line.clone();
        if self.cfg.line_id != line {
            self.cfg.line_id = line.clone();
            self.save_cfg();
        }
        let allow = self.catalog.for_device(d).line(&line).map(|l| l.allow_host_data).unwrap_or(false);
        if allow {
            self.fetch_host_data();
        }
    }

    fn fetch_host_data(&mut self) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = hostdata::fetch(hostdata::OT1_URL, Duration::from_secs(5)).and_then(|b| hostdata::parse(&b));
            let _ = tx.send(r);
        });
        self.setup.host_rx = Some(rx);
    }

    fn apply_host_data(&mut self, h: HostData) {
        self.setup.host_product = h.host_product;
        self.set_product(String::new());
        if let Some(p) = h.product {
            self.set_product(p);
            if let Some(r) = h.run_number {
                self.setup.run_number = r;
            }
            if let Some(m) = h.run_minute {
                self.setup.run_minute = m;
            }
        }
    }

    fn set_product(&mut self, p: String) {
        let Some(d) = self.device() else { return };
        let dc = self.catalog.for_device(d);
        self.setup.tests = dc.tests_for_product(&p);
        self.setup.upper_spec = dc.product(&p).map(|x| x.spec_upper).unwrap_or(0.0);
        self.setup.product = p;
        self.setup.test = None;
        self.setup.items.clear();
        self.setup.meas_per_sample = "0".into();
        if !self.setup.tests.is_empty() {
            let pick = if self.setup.line == "NWD" { self.setup.tests.len() - 1 } else { 0 };
            self.set_test(self.setup.tests[pick].clone());
        }
    }

    fn set_test(&mut self, t: String) {
        let Some(d) = self.device() else { return };
        let dc = self.catalog.for_device(d);
        if let Some(td) = dc.test(&t) {
            self.setup.sample_size = sample_size_index(&td.sample_size);
        }
        self.setup.items = dc.items_for_test(&t);
        self.setup.meas_per_sample = self.setup.items.len().to_string();
        self.setup.test = Some(t);
    }

    fn can_begin(&self) -> bool {
        let s = &self.setup;
        let Some(t) = &s.test else { return false };
        if s.line.is_empty() || s.product.is_empty() || s.items.is_empty() {
            return false;
        }
        NO_RUN_INFO_TESTS.contains(&t.as_str()) || (!s.run_number.trim().is_empty() && !s.run_minute.trim().is_empty())
    }

    fn begin_test(&mut self, ctx: &egui::Context) {
        if self.cfg.save_path.trim().is_empty() {
            self.message = Some(("Error".into(), "No Path saved!".into()));
            return;
        }
        let Conn::Ready { device, link, .. } = &mut self.conn else { return };
        let Some(link) = link.take() else { return };
        let device = *device;
        let s = &self.setup;
        let session = Session {
            device,
            info: RunInfo {
                line: s.line.clone(),
                run_number: s.run_number.trim().to_string(),
                run_minute: s.run_minute.trim().to_string(),
                test_name: s.test.clone().unwrap_or_default(),
                offset_nagy: self.cfg.offset_nagy,
            },
            items: s.items.clone(),
            sample_size: s.sample_size,
            upper_spec: s.upper_spec,
            poly: self.cfg.poly,
        };
        let (tx, crx) = channel();
        let (etx, rx) = channel();
        let sess = session.clone();
        let c = ctx.clone();
        let handle = std::thread::spawn(move || {
            engine::run(sess, link, crx, move |e| {
                let _ = etx.send(e);
                c.request_repaint();
            })
        });
        if device == Device::Nagy {
            self.setup.run_minute.clear(); // bản gốc xóa Run Minute khi vào đo Nagy
        }
        self.save_note = None;
        self.measure = Some(Measure {
            session,
            records: vec![],
            status: ("System Connected".into(), Tone::Connected),
            live: ("No Active Measurement".into(), Tone::Idle),
            sample_name: String::new(),
            submit: false,
            tx,
            rx,
            handle: Some(handle),
            closing: false,
            comm_error: None,
            confirm_remeasure: None,
            confirm_exit: false,
        });
    }

    fn submit(&mut self) {
        let Some(m) = &mut self.measure else { return };
        let now = chrono::Local::now().naive_local();
        let block = match m.session.device {
            Device::Nagy => nagy_block(&self.cfg.save_dir(), &m.session.info, &m.records, now),
            Device::FourPoint => fpp_block(&self.cfg.save_dir(), &PathBuf::from(&self.cfg.records_path), &m.session.info, &m.records, now),
        };
        if let Some(b) = block {
            let (ok, failed) = self.pending.write_block(&b, &self.cfg.file_formats);
            let names = |v: Vec<PathBuf>| v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
            if failed.is_empty() {
                self.save_note = Some((format!("Saved {} result(s) to:\n{}", m.records.len(), names(ok)), false));
            } else {
                let f: Vec<String> = failed.iter().map(|(p, e)| format!("{} ({e})", p.display())).collect();
                let msg = format!(
                    "Could not write (file open in Excel?):\n{}\n\nThe data is kept and will be written automatically as soon as the file is closed.",
                    f.join("\n")
                );
                self.message = Some(("CSV".into(), msg.clone()));
                self.save_note = Some((msg, true));
            }
        }
        m.records.clear();
        let _ = m.tx.send(Cmd::Stop);
        m.closing = true;
    }

    // ---------------- UI ----------------

    fn menu(&mut self, ui: &mut egui::Ui) {
        let measuring = self.measure.is_some();
        let small = |s: &str| RichText::new(s).size(13.0);
        egui::menu::bar(ui, |ui| {
            ui.add_enabled_ui(!measuring, |ui| {
                if ui.button(small("Choose Save Folder...")).clicked() {
                    self.choose_folder();
                }
                ui.label(small(&self.cfg.save_path));
                let cal_label = match self.device().unwrap_or(self.cfg.device) {
                    Device::Nagy => "Offset Calibration",
                    Device::FourPoint => "3 Point Calibration",
                };
                if ui.add_enabled(self.device().is_some(), egui::Button::new(small(cal_label))).clicked() {
                    self.calib = Some(match self.device() {
                        Some(Device::Nagy) => CalibDlg::Nagy { offset: String::new(), password: String::new() },
                        _ => CalibDlg::Poly { a: String::new(), b: String::new(), c: String::new(), low: String::new(), high: String::new(), password: String::new() },
                    });
                }
                ui.menu_button(small("CSV Format"), |ui| {
                    ui.label(small("File name format (choose 1 to 3):"));
                    for f in FileFormat::ALL {
                        let mut on = self.cfg.file_formats.contains(&f);
                        let example = format!("{}  ->  {}{}", f.label(), f.prefix(), example_name(Some(Device::Nagy)));
                        if ui.checkbox(&mut on, small(&example)).changed() {
                            let mut v: Vec<FileFormat> = self.cfg.file_formats.iter().copied().filter(|x| *x != f).collect();
                            if on {
                                v.push(f);
                            }
                            if v.is_empty() {
                                self.message = Some(("CSV Format".into(), "At least one format must be selected.".into()));
                            } else {
                                self.cfg.file_formats = FileFormat::normalize(&v);
                                self.save_cfg();
                            }
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new("Each result is written to every selected file.\nColumns are the same in all formats.").small());
                    ui.label(RichText::new("Three-Point- / Continuous- files get the same prefix.\n4-Point Probe (Keithley) keeps its original name: yyyymmdd.csv").small().weak());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let conn_label = match &self.conn {
                        Conn::Ready { device, .. } => format!("Connected To: {}", device.label()),
                        Conn::Connecting(_) => "Connecting...".into(),
                        Conn::Failed(_) => "Not Connected".into(),
                    };
                    ui.menu_button(small(&conn_label), |ui| {
                        if let Conn::Ready { model, port, .. } = &self.conn {
                            ui.label(small(&format!("{port} {model}")));
                            ui.separator();
                        }
                        for d in [Device::FourPoint, Device::Nagy] {
                            let checked = self.device() == Some(d);
                            if ui.selectable_label(checked, small(d.label())).clicked() {
                                ui.close_menu();
                                self.start_connect(d);
                            }
                        }
                    });
                });
            });
        });
    }

    fn choose_folder(&mut self) {
        #[cfg(windows)]
        {
            if let Some(p) = rfd::FileDialog::new().set_title("Choose Save Folder").set_directory(&self.cfg.save_path).pick_folder() {
                self.set_save_path(p.display().to_string());
            }
        }
        #[cfg(not(windows))]
        {
            self.folder_edit = Some(self.cfg.save_path.clone());
        }
    }

    fn set_save_path(&mut self, mut p: String) {
        if !p.ends_with('\\') && !p.ends_with('/') {
            p.push(if cfg!(windows) { '\\' } else { '/' });
        }
        self.cfg.save_path = p;
        self.save_cfg();
    }

    /// Màn hình Setup theo bố cục bản cũ: nhãn lớn phía trên ô nhập, danh sách test bên trái,
    /// ba nút bên phải.
    fn setup_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        const C1: f32 = 140.0;
        const C2: f32 = 160.0;
        const C3: f32 = 145.0;
        const C4: f32 = 300.0;
        const GAP: f32 = 16.0;
        let lbl = |s: &str| RichText::new(s).size(24.0);
        // Giá trị trong các ô chọn / ô nhập: cùng cỡ chữ với nhãn "Line Code"
        let val = |s: &str| RichText::new(if s.is_empty() { " " } else { s }).size(24.0);
        let val_font = egui::FontId::proportional(24.0);
        ui.add_space(4.0);
        match &self.conn {
            Conn::Connecting(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Searching COM ports for Keithley / Nagy...");
                });
                return;
            }
            Conn::Failed(e) => {
                let e = e.clone();
                ui.colored_label(Color32::from_rgb(180, 0, 0), if e.is_empty() { "Not connected." } else { &e });
                ui.add_space(8.0);
                if ui.button("Retry connection").clicked() {
                    self.start_connect(self.cfg.device);
                }
                return;
            }
            Conn::Ready { .. } => {}
        }
        let d = self.device().unwrap();
        let dc = self.catalog.for_device(d).clone();
        let mut new_line = None;
        let mut new_product = None;
        let mut new_test = None;
        let mut clear = false;
        let mut begin = false;
        let mut host = false;
        let allow_host = dc.line(&self.setup.line).map(|l| l.allow_host_data).unwrap_or(false);
        let can_begin = self.can_begin();

        // Hàng 1: Line Code | Run Number | Run Minute | No. of Meas per Sample
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            ui.vertical(|ui| {
                ui.set_width(C1);
                ui.label(lbl("Line Code"));
                egui::ComboBox::from_id_salt("line").width(C1 - 10.0).height(300.0).selected_text(val(&self.setup.line)).show_ui(ui, |ui| {
                    for l in &dc.lines {
                        if ui.selectable_label(self.setup.line == l.name, val(&l.name)).clicked() {
                            new_line = Some(l.name.clone());
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.set_width(C2);
                ui.label(lbl("Run Number"));
                ui.add(egui::TextEdit::singleline(&mut self.setup.run_number).desired_width(C2).font(val_font.clone()));
            });
            ui.vertical(|ui| {
                ui.set_width(C3);
                ui.label(lbl("Run Minute"));
                ui.add(egui::TextEdit::singleline(&mut self.setup.run_minute).desired_width(C3).font(val_font.clone()));
            });
            ui.vertical(|ui| {
                ui.set_width(C4);
                ui.label(lbl("No. of Meas per Sample"));
                let mut n = self.setup.meas_per_sample.clone();
                ui.add_enabled(false, egui::TextEdit::singleline(&mut n).desired_width(70.0).horizontal_align(egui::Align::Center).font(val_font.clone()));
            });
        });
        ui.add_space(8.0);

        // Hàng 2: Product Type ......... Sample Size
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            ui.vertical(|ui| {
                ui.set_width(C1 + GAP + C2 + GAP + C3);
                ui.label(lbl("Product Type"));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("product").width(C1 - 10.0).height(300.0).selected_text(val(&self.setup.product)).show_ui(ui, |ui| {
                        for p in &dc.products {
                            if ui.selectable_label(self.setup.product == p.name, val(&p.name)).clicked() {
                                new_product = Some(p.name.clone());
                            }
                        }
                    });
                    if !self.setup.host_product.is_empty() {
                        ui.label(RichText::new(&self.setup.host_product).size(13.0).color(Color32::DARK_BLUE));
                    }
                });
            });
            ui.vertical(|ui| {
                ui.set_width(C4);
                ui.label(lbl("Sample Size"));
                let cur = self.setup.sample_size.and_then(|i| SAMPLE_SIZES.get(i)).map(|x| x.0).unwrap_or("");
                egui::ComboBox::from_id_salt("ssize").width(C4 - 10.0).selected_text(val(cur)).show_ui(ui, |ui| {
                    for (i, (n, _)) in SAMPLE_SIZES.iter().enumerate() {
                        ui.selectable_value(&mut self.setup.sample_size, Some(i), val(n));
                    }
                });
            });
        });
        ui.add_space(12.0);

        // Hàng 3: danh sách test | Get Run Data / Clear Entries / Begin Test
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            let list_w = C1 + GAP + C2 + GAP + C3;
            egui::Frame::new().fill(Color32::WHITE).stroke(egui::Stroke::new(1.0_f32, Color32::from_gray(160))).inner_margin(2).show(ui, |ui| {
                ui.set_width(list_w - 4.0);
                ui.set_height(122.0);
                egui::ScrollArea::vertical().id_salt("tests").auto_shrink([false, false]).show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for t in &self.setup.tests {
                            let sel = self.setup.test.as_deref() == Some(t);
                            let text = RichText::new(t).size(24.0).color(if sel { Color32::WHITE } else { Color32::BLACK });
                            let b = egui::Button::new(text).fill(if sel { Color32::from_rgb(0, 120, 215) } else { Color32::WHITE }).stroke(egui::Stroke::NONE).corner_radius(0.0);
                            if ui.add(b).clicked() {
                                new_test = Some(t.clone());
                            }
                        }
                    });
                });
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let big = |ui: &mut egui::Ui, s: &str, on: bool| {
                    ui.add_enabled_ui(on, |ui| ui.add_sized([C4, 36.0], egui::Button::new(RichText::new(s).size(22.0)))).inner.clicked()
                };
                host = big(ui, "Get Run Data", allow_host && self.setup.host_rx.is_none());
                clear = big(ui, "Clear Entries", true);
                begin = big(ui, "Begin Test", can_begin);
            });
        });

        if host {
            self.fetch_host_data();
        }
        if let Some(l) = new_line {
            self.set_line(l);
        }
        if let Some(p) = new_product {
            self.set_product(p);
        }
        if let Some(t) = new_test {
            self.set_test(t);
        }
        if clear {
            let line = self.setup.line.clone();
            self.setup = Setup { line, ..Default::default() };
        }
        if begin {
            self.begin_test(ctx);
        }
    }

    /// Màn hình đo (MeasureForm) theo bố cục bản cũ: kết quả bên trái; thông tin, số đo trực tiếp
    /// và biểu đồ bên phải; Exit Measurement / Submit ở dưới.
    fn measure_ui(&mut self, ui: &mut egui::Ui) {
        let Some(m) = &mut self.measure else { return };
        let mut remeasure_click = None;
        let mut submit = false;
        let mut exit = false;
        let mono = |s: &str, size: f32| RichText::new(s).family(egui::FontFamily::Monospace).size(size).strong().color(Color32::BLACK);
        let avail = ui.available_size();
        let bottom_h = 48.0;
        let body_h = (avail.y - bottom_h).max(200.0);
        let left_w = RESULTS_W;
        let right_w = avail.x - left_w - 12.0; // = INFO_W ở kích thước mặc định

        ui.horizontal_top(|ui| {
            // Trái: danh sách kết quả
            ui.allocate_ui_with_layout(egui::vec2(left_w, body_h), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.set_min_size(egui::vec2(left_w, body_h));
                egui::ScrollArea::vertical().id_salt("results").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    for (i, r) in m.records.iter().enumerate() {
                        let text = format!("{} - {}", r.point, fmt_num(round2(r.value)));
                        let b = egui::Button::new(RichText::new(text).size(20.0).strong().color(Color32::BLACK))
                            .fill(Color32::from_rgb(144, 238, 144))
                            .min_size(egui::vec2(left_w - 24.0, 34.0));
                        let enabled = !m.closing && m.confirm_remeasure.is_none() && !m.confirm_exit;
                        if ui.add_enabled(enabled, b).on_hover_text("Click to remeasure").clicked() {
                            remeasure_click = Some(i);
                        }
                    }
                });
            });
            // Phải: thông tin + biểu đồ
            ui.allocate_ui_with_layout(egui::vec2(right_w, body_h), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                let (live_fg, live_bg) = match m.live.1 {
                    Tone::Idle | Tone::Connected => (Color32::BLACK, Color32::WHITE),
                    t => tone_colors(t),
                };
                egui::Frame::new().fill(live_bg).inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
                    ui.set_width(right_w - 32.0);
                    ui.horizontal(|ui| {
                        ui.label(mono(&m.session.info.test_name, 28.0));
                        if m.session.device == Device::Nagy {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(40.0);
                                ui.label(mono(&format!("Offset Value: {}", fmt_num(m.session.info.offset_nagy)), 22.0));
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(mono(&format!("Sample Minute {}", m.session.info.run_minute), 22.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (fg, bg) = tone_colors(m.status.1);
                            egui::Frame::new().fill(bg).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                                ui.set_width(300.0);
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| ui.label(RichText::new(if m.status.0.is_empty() { " " } else { &m.status.0 }).size(18.0).color(fg)));
                            });
                        });
                    });
                    ui.label(mono(&m.sample_name, 22.0));
                    ui.label(RichText::new("Live Reading").size(13.0).color(Color32::BLACK));
                    ui.vertical_centered(|ui| {
                        ui.label(mono(&m.live.0, 40.0).color(live_fg));
                    });
                    if let Some(e) = &m.comm_error {
                        ui.colored_label(Color32::RED, RichText::new(e).size(13.0));
                    }
                });
                ui.add_space(6.0);
                let pts = chart_points(&m.session, &m.records);
                let h = ui.available_height().max(120.0);
                draw_chart(ui, &pts, egui::vec2(right_w, h));
            });
        });

        // Dưới: Exit Measurement | Submit
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(left_w - 30.0);
            let btn = |ui: &mut egui::Ui, s: &str, on: bool| ui.add_enabled_ui(on, |ui| ui.add_sized([176.0, 36.0], egui::Button::new(RichText::new(s).size(20.0)))).inner.clicked();
            exit = btn(ui, "Exit Measurement", !m.closing);
            ui.add_space(80.0);
            submit = btn(ui, "Submit", m.submit && !m.closing);
            if m.closing {
                ui.spinner();
                ui.label("Closing...");
            }
        });

        if let Some(i) = remeasure_click {
            m.confirm_remeasure = Some(i);
        }
        if exit {
            if m.records.is_empty() {
                let _ = m.tx.send(Cmd::Stop);
                m.closing = true;
            } else {
                m.confirm_exit = true;
            }
        }
        if submit {
            self.submit();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Hỏi đo lại (Measurement_Click)
        if let Some(m) = &mut self.measure {
            if let Some(i) = m.confirm_remeasure {
                let name = m.records.get(i).map(|r| r.point.clone()).unwrap_or_default();
                let mut close = false;
                modal(ctx, "Remeasure Dialog Box", |ui| {
                    ui.label(format!("Choose Yes to Remeasure {name}\nChoose No to Resume Measurement"));
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            let _ = m.tx.send(Cmd::Remeasure(i));
                            close = true;
                        }
                        if ui.button("No").clicked() {
                            close = true;
                        }
                    });
                });
                if close {
                    m.confirm_remeasure = None;
                }
            }
            if m.confirm_exit {
                let mut close = false;
                modal(ctx, "Exit Measurement", |ui| {
                    ui.label(format!("{} result(s) have not been submitted.\nExit without saving?", m.records.len()));
                    ui.horizontal(|ui| {
                        if ui.button("Exit without saving").clicked() {
                            let _ = m.tx.send(Cmd::Stop);
                            m.closing = true;
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
                if close {
                    m.confirm_exit = false;
                }
            }
        }

        // Calibration
        let mut close_cal = false;
        if let Some(c) = &mut self.calib {
            match c {
                CalibDlg::Poly { a, b, c: cc, low, high, password } => {
                    let poly = self.cfg.poly;
                    let mut update = None;
                    let mut wrong_pw = false;
                    modal(ctx, "3 Point Calibration", |ui| {
                        egui::Grid::new("poly").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label("");
                            ui.label(RichText::new("Current Value").strong());
                            ui.label(RichText::new("New Value").strong());
                            ui.end_row();
                            for (lbl, cur, txt) in [("a x^2", poly.a, &mut *a), ("b x", poly.b, &mut *b), ("c", poly.c, &mut *cc), ("Lower Limit", poly.low, &mut *low), ("Upper Limit", poly.high, &mut *high)] {
                                ui.label(lbl);
                                ui.label(fmt_num(cur));
                                ui.add(egui::TextEdit::singleline(txt).desired_width(120.0));
                                ui.end_row();
                            }
                            ui.label("Password:");
                            ui.label("");
                            ui.add(egui::TextEdit::singleline(password).password(true).desired_width(120.0));
                            ui.end_row();
                        });
                        let all = [&*a, &*b, &*cc, &*low, &*high].iter().all(|s| !s.trim().is_empty()) && !password.is_empty();
                        ui.horizontal(|ui| {
                            if ui.add_enabled(all, egui::Button::new("Update")).clicked() {
                                if password.as_str() == OFFSET_PASSWORD {
                                    let v: Result<Vec<f64>, _> = [&*a, &*b, &*cc, &*low, &*high].iter().map(|s| s.trim().parse::<f64>()).collect();
                                    update = Some(v);
                                } else {
                                    wrong_pw = true;
                                    password.clear();
                                }
                            }
                            if ui.button("Exit").clicked() {
                                close_cal = true;
                            }
                        });
                    });
                    match update {
                        Some(Ok(v)) => {
                            self.cfg.poly = Poly { a: v[0], b: v[1], c: v[2], low: v[3], high: v[4] };
                            self.save_cfg();
                            close_cal = true;
                        }
                        Some(Err(_)) => self.message = Some(("Calibration".into(), "All values must be numbers (use . as decimal point).".into())),
                        None if wrong_pw => self.message = Some(("3 Point Calibration".into(), "Wrong password. The calibration was not changed.".into())),
                        None => {}
                    }
                }
                CalibDlg::Nagy { offset, password } => {
                    let cur = self.cfg.offset_nagy;
                    let mut update = None;
                    let mut wrong_pw = false;
                    modal(ctx, "Offset Calibration", |ui| {
                        egui::Grid::new("off").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label("");
                            ui.label(RichText::new("Current Value").strong());
                            ui.label(RichText::new("New Value").strong());
                            ui.end_row();
                            ui.label("Offset:");
                            ui.label(fmt_num(cur));
                            ui.add(egui::TextEdit::singleline(offset).desired_width(120.0));
                            ui.end_row();
                            ui.label("Password:");
                            ui.label("");
                            ui.add(egui::TextEdit::singleline(password).password(true).desired_width(120.0));
                            ui.end_row();
                        });
                        ui.horizontal(|ui| {
                            let ready = !offset.trim().is_empty() && !password.is_empty();
                            if ui.add_enabled(ready, egui::Button::new("Update")).clicked() {
                                if password.as_str() == OFFSET_PASSWORD {
                                    update = Some(offset.trim().parse::<f64>());
                                } else {
                                    wrong_pw = true;
                                    password.clear();
                                }
                            }
                            if ui.button("Exit").clicked() {
                                close_cal = true;
                            }
                        });
                    });
                    match update {
                        Some(Ok(v)) if v.is_finite() => {
                            self.cfg.offset_nagy = v;
                            self.save_cfg();
                            close_cal = true;
                        }
                        Some(_) => self.message = Some(("Calibration".into(), "Offset must be a number (use . as decimal point).".into())),
                        None if wrong_pw => self.message = Some(("Offset Calibration".into(), "Wrong password. The offset was not changed.".into())),
                        None => {}
                    }
                }
            }
        }
        if close_cal {
            self.calib = None;
        }

        // Nhập thư mục (khi không có hộp thoại hệ thống)
        if let Some(p) = &mut self.folder_edit {
            let mut done = None;
            modal(ctx, "Choose Save Folder", |ui| {
                ui.add(egui::TextEdit::singleline(p).desired_width(360.0));
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) => {
                    let p = self.folder_edit.take().unwrap();
                    self.set_save_path(p);
                }
                Some(false) => self.folder_edit = None,
                None => {}
            }
        }

        if let Some((t, b)) = &self.message {
            let mut close = false;
            msgbox(ctx, t, |ui| {
                ui.label(b);
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
            if close {
                self.message = None;
            }
        }
    }

    fn poll_background(&mut self) {
        // Kết quả dò thiết bị
        if let Conn::Connecting(rx) = &self.conn {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok(c) => {
                        let changed = self.cfg.device != c.device;
                        self.cfg.device = c.device;
                        if changed {
                            self.save_cfg();
                        }
                        self.conn = Conn::Ready { device: c.device, link: Some(c.link), model: c.model, port: c.port };
                        self.reset_setup();
                        self.shown_measure = None; // cập nhật tiêu đề cửa sổ theo máy
                    }
                    Err(e) => self.conn = Conn::Failed(e),
                }
            }
        }
        // Host data
        if let Some(rx) = &self.setup.host_rx {
            if let Ok(r) = rx.try_recv() {
                self.setup.host_rx = None;
                match r {
                    Ok(h) => self.apply_host_data(h),
                    Err(e) => self.message = Some(("Get Host Data".into(), format!("{}\n{e}", hostdata::OT1_URL))),
                }
            }
        }
        // Sự kiện từ luồng đo
        let mut finished = false;
        if let Some(m) = &mut self.measure {
            while let Ok(e) = m.rx.try_recv() {
                match e {
                    Evt::Status(s, t) => m.status = (s, t),
                    Evt::Live(s, t) => m.live = (s, t),
                    Evt::SampleName(n) => m.sample_name = n,
                    Evt::Record(None, r) => {
                        if !m.closing {
                            m.records.push(r)
                        }
                    }
                    Evt::Record(Some(i), r) => {
                        if i < m.records.len() {
                            m.records[i] = r;
                        }
                    }
                    Evt::SubmitEnabled(b) => m.submit = b,
                    Evt::CommError(s) => m.comm_error = Some(s),
                }
            }
            if m.handle.as_ref().map(|h| h.is_finished()).unwrap_or(false) {
                finished = true;
            }
        }
        if finished {
            let mut m = self.measure.take().unwrap();
            if m.records.is_empty() {
                self.save_note = self.save_note.take();
            }
            let link = m.handle.take().unwrap().join().ok();
            if let Conn::Ready { link: slot, .. } = &mut self.conn {
                *slot = link;
                if slot.is_none() {
                    self.conn = Conn::Failed("Measurement thread stopped unexpectedly. Please reconnect.".into());
                }
            }
        }
        // Ghi bù CSV đang chờ
        if !self.pending.items.is_empty() && self.last_retry.elapsed() >= Duration::from_secs(3) {
            self.last_retry = Instant::now();
            let done = self.pending.retry();
            if !done.is_empty() {
                let left = self.pending.items.len();
                let files: Vec<String> = done.iter().map(|p| p.display().to_string()).collect();
                self.save_note = Some((
                    if left == 0 { format!("Pending data written to:\n{}", files.join("\n")) } else { format!("Written: {}\nStill waiting: {left}", files.join(", ")) },
                    left > 0,
                ));
            }
        }
    }
}

fn example_name(d: Option<Device>) -> String {
    let date = chrono::Local::now().format("%Y%m%d");
    match d {
        Some(Device::Nagy) => format!("Five-Points-{date}.csv"),
        _ => format!("{date}.csv"),
    }
}

/// Hộp thông báo: ID riêng để không trùng với hộp thoại cùng tiêu đề đang mở.
fn msgbox(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title)
        .id(egui::Id::new("msgbox"))
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, add);
}

fn modal(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, add);
}

/// Toạ độ biểu đồ như UpdateChart của bản gốc.
fn chart_points(s: &Session, recs: &[Record]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    let mut count = 0.0;
    for r in recs {
        if !r.value.is_finite() {
            continue;
        }
        count += 1.0;
        let x = match (s.device, s.info.test_name.as_str()) {
            (Device::FourPoint, "2 Inch Ohms") => count * 2.0,
            (Device::FourPoint, "4 Inch Ohms") => 2.0 + (count - 1.0) * 4.0,
            _ => count,
        };
        out.push((x, r.value));
    }
    out
}

fn draw_chart(ui: &mut egui::Ui, pts: &[(f64, f64)], size: egui::Vec2) {
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, 2.0, Color32::WHITE);
    let plot = egui::Rect::from_min_max(rect.min + egui::vec2(52.0, 10.0), rect.max - egui::vec2(12.0, 24.0));
    let axis = egui::Stroke::new(1.0_f32, Color32::GRAY);
    painter.line_segment([plot.left_bottom(), plot.right_bottom()], axis);
    painter.line_segment([plot.left_top(), plot.left_bottom()], axis);
    // Chú thích như biểu đồ cũ
    let lg = egui::pos2(rect.right() - 110.0, rect.top() + 22.0);
    painter.line_segment([lg, lg + egui::vec2(22.0, 0.0)], egui::Stroke::new(2.0_f32, Color32::from_rgb(30, 144, 255)));
    painter.text(lg + egui::vec2(28.0, 0.0), egui::Align2::LEFT_CENTER, "Ohms/Sq", egui::FontId::proportional(12.0), Color32::BLACK);
    if pts.is_empty() {
        return;
    }
    let (mut x0, mut x1) = (f64::MAX, f64::MIN);
    let (mut y0, mut y1) = (f64::MAX, f64::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    if x1 - x0 < 1.0 {
        x0 -= 1.0;
        x1 += 1.0;
    }
    let pad = ((y1 - y0) * 0.15).max(y1.abs() * 0.01).max(0.01);
    y0 -= pad;
    y1 += pad;
    let to = |x: f64, y: f64| egui::pos2(plot.left() + ((x - x0) / (x1 - x0)) as f32 * plot.width(), plot.bottom() - ((y - y0) / (y1 - y0)) as f32 * plot.height());
    let font = egui::FontId::proportional(11.0);
    for k in 0..=4 {
        let y = y0 + (y1 - y0) * k as f64 / 4.0;
        let p = to(x0, y);
        painter.line_segment([egui::pos2(plot.left(), p.y), egui::pos2(plot.right(), p.y)], egui::Stroke::new(0.5_f32, Color32::from_gray(225)));
        painter.text(egui::pos2(plot.left() - 4.0, p.y), egui::Align2::RIGHT_CENTER, fmt_num(round2(y)), font.clone(), Color32::DARK_GRAY);
    }
    painter.text(egui::pos2(plot.left(), plot.bottom() + 4.0), egui::Align2::LEFT_TOP, fmt_num(x0), font.clone(), Color32::DARK_GRAY);
    painter.text(egui::pos2(plot.right(), plot.bottom() + 4.0), egui::Align2::RIGHT_TOP, fmt_num(x1), font, Color32::DARK_GRAY);
    let line: Vec<egui::Pos2> = pts.iter().map(|&(x, y)| to(x, y)).collect();
    let blue = Color32::from_rgb(30, 144, 255);
    for w in line.windows(2) {
        painter.line_segment([w[0], w[1]], egui::Stroke::new(2.0_f32, blue));
    }
    for p in line {
        painter.circle_filled(p, 3.5, blue);
    }
}

fn install_fonts(ctx: &egui::Context) {
    // Font của bản cũ: Microsoft Sans Serif (chữ thường) và Courier New Bold (màn hình đo)
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |key: &str, paths: &[&str], fam: egui::FontFamily| {
        for p in paths {
            if let Ok(bytes) = std::fs::read(p) {
                fonts.font_data.insert(key.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                fonts.families.entry(fam).or_default().insert(0, key.into());
                return;
            }
        }
    };
    add("sans", &["C:\\Windows\\Fonts\\micross.ttf", "C:\\Windows\\Fonts\\arial.ttf", "C:\\Windows\\Fonts\\segoeui.ttf"], egui::FontFamily::Proportional);
    add("mono", &["C:\\Windows\\Fonts\\courbd.ttf", "C:\\Windows\\Fonts\\cour.ttf"], egui::FontFamily::Monospace);
    ctx.set_fonts(fonts);
    ctx.style_mut(|st| {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        st.text_styles.insert(TextStyle::Body, FontId::new(16.0, Proportional));
        st.text_styles.insert(TextStyle::Button, FontId::new(22.0, Proportional));
        st.text_styles.insert(TextStyle::Heading, FontId::new(22.0, Proportional));
        st.text_styles.insert(TextStyle::Small, FontId::new(12.0, Proportional));
        st.spacing.button_padding = egui::vec2(6.0, 2.0);
        st.spacing.interact_size.y = 30.0;
        st.visuals.widgets.inactive.weak_bg_fill = Color32::from_gray(225);
        // viền ô nhập / nút như WinForms
        let border = egui::Stroke::new(1.0_f32, Color32::from_gray(150));
        st.visuals.widgets.inactive.bg_stroke = border;
        st.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_gray(190));
        st.visuals.extreme_bg_color = Color32::WHITE;
        st.visuals.panel_fill = Color32::from_gray(240);
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background();
        let measuring = self.measure.is_some();
        if self.shown_measure != Some(measuring) {
            self.shown_measure = Some(measuring);
            let sim = if self.simulate.is_some() { " (SIMULATION)" } else { "" };
            let (size, title) = if measuring {
                (MEASURE_SIZE, format!("MeasureForm{sim}"))
            } else {
                let t = match self.device().unwrap_or(self.cfg.device) {
                    Device::Nagy => "Nagy Setup Screen",
                    Device::FourPoint => "4-Point Probe Setup Screen",
                };
                (SETUP_SIZE, format!("{t}{sim}"))
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size[0], size[1])));
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        if !measuring {
            egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu(ui));
        }
        if self.measure.is_none() {
            if let Some((note, warn)) = &self.save_note {
                let (note, warn) = (note.clone(), *warn);
                egui::TopBottomPanel::bottom("note").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(if warn { Color32::from_rgb(180, 90, 0) } else { Color32::DARK_GREEN }, note);
                        if !self.pending.items.is_empty() {
                            ui.label(format!("({} pending)", self.pending.items.len()));
                        }
                    });
                });
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.measure.is_some() {
                self.measure_ui(ui);
            } else {
                self.setup_ui(ui, ctx);
            }
        });
        self.dialogs(ctx);
        if self.measure.is_some() || !self.pending.items.is_empty() || matches!(self.conn, Conn::Connecting(_)) || self.setup.host_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(m) = &mut self.measure {
            let _ = m.tx.send(Cmd::Stop);
            if let Some(h) = m.handle.take() {
                let t0 = Instant::now();
                while !h.is_finished() && t0.elapsed() < Duration::from_secs(3) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                if h.is_finished() {
                    let _ = h.join();
                }
            }
        }
        let _ = self.cfg.save(&self.store);
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let simulate = args.iter().position(|a| a == "--simulate").map(|i| match args.get(i + 1).map(|s| s.to_ascii_lowercase()) {
        Some(s) if s == "nagy" => Device::Nagy,
        _ => Device::FourPoint,
    });
    let title = if simulate.is_some() { "FourPointProbe (SIMULATION)" } else { "FourPointProbe" };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(SETUP_SIZE).with_min_inner_size([760.0, 380.0]).with_title(title),
        ..Default::default()
    };
    eframe::run_native(title, opts, Box::new(move |cc| Ok(Box::new(App::new(cc, simulate)))))
}

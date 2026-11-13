//! Enumerate ports without opening them, and guide unplug/replug identification.
use super::i18n::{display, t, ui_format};
use super::{INK, LINE, MUTED, TEAL, card, label};
use eframe::egui::{self, Color32, RichText, Vec2};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PortDetails {
    pub path: String,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub serial: Option<String>,
    pub usb_id: Option<(u16, u16)>,
    bluetooth: bool,
}
impl PortDetails {
    pub fn title(&self) -> String {
        self.product
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.manufacturer.clone().filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| {
                if self.usb_id.is_some() {
                    t("USB 串口设备").into()
                } else {
                    self.path.clone()
                }
            })
    }
    fn eligible(&self) -> bool {
        !self.bluetooth
            && !self.path.to_ascii_lowercase().contains("bluetooth")
            && !self.path.contains("debug-console")
    }
    fn identity(&self) -> String {
        let mut fields = Vec::new();
        if let Some(name) = &self.manufacturer {
            fields.push(format!("厂商：{name}"));
        }
        if let Some((vid, pid)) = self.usb_id {
            fields.push(format!("USB {vid:04X}:{pid:04X}"));
        }
        if let Some(serial) = &self.serial {
            fields.push(format!("序列号：{serial}"));
        }
        if fields.is_empty() {
            "系统未提供厂商或序列号，可通过插拔识别。".into()
        } else {
            fields.join(" · ")
        }
    }
}
fn normalize(ports: Vec<serialport::SerialPortInfo>) -> Vec<PortDetails> {
    let mut result = Vec::new();
    for info in ports {
        // macOS /dev/tty.* and /dev/cu.* are aliases for one device. Prefer cu for outbound use.
        if cfg!(target_os = "macos") && info.port_name.starts_with("/dev/tty.") {
            continue;
        }
        let mut port = PortDetails {
            path: info.port_name,
            product: None,
            manufacturer: None,
            serial: None,
            usb_id: None,
            bluetooth: false,
        };
        match info.port_type {
            serialport::SerialPortType::UsbPort(usb) => {
                port.product = usb.product;
                port.manufacturer = usb.manufacturer;
                port.serial = usb.serial_number;
                port.usb_id = Some((usb.vid, usb.pid));
            }
            serialport::SerialPortType::BluetoothPort => port.bluetooth = true,
            _ => {}
        }
        result.push(port);
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result.dedup_by(|a, b| a.path == b.path);
    result
}
pub(super) fn scan() -> Result<Vec<PortDetails>, serialport::Error> {
    serialport::available_ports().map(normalize)
}
pub(super) fn single_usb(ports: &[PortDetails]) -> Option<String> {
    let mut usb = ports.iter().filter(|p| p.usb_id.is_some());
    let first = usb.next()?;
    if usb.next().is_none() {
        Some(first.path.clone())
    } else {
        None
    }
}

/// The baseline is captured only after the user confirms unplugging the target.
/// Baseline removals are remembered, including delayed OS disappearance of the target.
struct Discovery {
    baseline: BTreeSet<String>,
    candidates: Vec<PortDetails>,
}
impl Discovery {
    fn new(ports: &[PortDetails]) -> Self {
        Self {
            baseline: ports.iter().map(|p| p.path.clone()).collect(),
            candidates: vec![],
        }
    }
    fn observe(&mut self, ports: &[PortDetails]) {
        let now: BTreeSet<_> = ports.iter().map(|p| p.path.clone()).collect();
        self.baseline.retain(|p| now.contains(p));
        self.candidates = ports
            .iter()
            .filter(|p| p.eligible() && !self.baseline.contains(&p.path))
            .cloned()
            .collect();
    }
    fn selected(&self, path: &str) -> Option<PortDetails> {
        self.candidates.iter().find(|p| p.path == path).cloned()
    }
}

#[derive(Clone, Copy)]
pub(super) enum Target {
    Upgrade,
    Debug,
}
#[derive(Default)]
pub(super) struct Finder {
    target: Option<Target>,
    discovery: Option<Discovery>,
    last_poll: Option<Instant>,
    selected: String,
    error: Option<String>,
}
impl Finder {
    pub fn start(&mut self, target: Target) {
        self.target = Some(target);
        self.discovery = None;
        self.last_poll = None;
        self.selected.clear();
        self.error = None;
    }
    pub fn active(&self) -> bool {
        self.target.is_some()
    }
    fn capture(&mut self) {
        match scan() {
            Ok(ports) => {
                self.discovery = Some(Discovery::new(&ports));
                self.last_poll = Some(Instant::now());
                self.selected.clear();
                self.error = None;
            }
            Err(e) => self.error = Some(format!("读取串口列表失败：{e}。请重试。")),
        }
    }
    fn refresh(&mut self) {
        self.last_poll = Some(Instant::now());
        match scan() {
            Ok(ports) => {
                if let Some(discovery) = &mut self.discovery {
                    discovery.observe(&ports);
                    if discovery.selected(&self.selected).is_none() {
                        self.selected.clear();
                    }
                    if discovery.candidates.len() == 1 {
                        self.selected = discovery.candidates[0].path.clone();
                    }
                }
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("读取设备变化失败：{e}。正在重试。"));
                if let Some(discovery) = &mut self.discovery {
                    discovery.candidates.clear();
                }
                self.selected.clear();
            }
        }
    }
    pub fn show(&mut self, ctx: &egui::Context) -> Option<(Target, PortDetails)> {
        let target = self.target?;
        if self.discovery.is_some()
            && self
                .last_poll
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
        {
            self.refresh();
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        let mut chosen = None;
        let response=egui::Modal::new(egui::Id::new("find-serial-port")).frame(card().inner_margin(26)).show(ctx,|ui| {
            ui.set_width(500.0f32.min((ctx.content_rect().width()-100.0).max(280.0)));
            ui.label(RichText::new(t("通过插拔找到串口")).size(23.0).strong().color(INK));
            ui.add_space(6.0);label(ui,t("只需拔插目标 USB 转串口，其他设备保持连接。"));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                let step=if self.discovery.is_none() {0} else if self.discovery.as_ref().is_some_and(|d|d.candidates.is_empty()) {1} else {2};
                for (i,text) in [t("1  拔下设备"),t("2  插回设备"),t("3  确认串口")].iter().enumerate() {
                    ui.label(RichText::new(*text).size(12.0).strong().color(if step==i {TEAL} else {MUTED}));
                    if i<2 {ui.label(RichText::new("  →  ").color(MUTED));}
                }
            });
            ui.add_space(20.0);
            if self.discovery.is_none() {
                ui.label(RichText::new(t("先从 Mac 拔下目标 USB 转串口")).size(16.0).strong());
                ui.add_space(8.0);
                ui.label(t("芯片主板若已独立供电，可以保持供电。拔下后稍等片刻，再点击下面的按钮记录当前设备列表。"));
                ui.add_space(16.0);
                if ui.add(egui::Button::new(RichText::new(t("已拔下，开始检测")).color(Color32::WHITE)).fill(TEAL).min_size(Vec2::new(ui.available_width(),40.0))).clicked() {self.capture();}
            } else {
                let candidates=self.discovery.as_ref().map(|d|d.candidates.clone()).unwrap_or_default();
                if candidates.is_empty() {
                    ui.horizontal(|ui| {ui.spinner();ui.label(RichText::new(t("现在插回 USB 转串口")).size(16.0).strong());});
                    ui.add_space(8.0);ui.label(t("正在自动检测新增串口。通常几秒内会出现，无需手动刷新。"));
                    ui.add_space(8.0);label(ui,t("一直没有变化？检查USB数据线和驱动，或重新记录拔下后的基准。"));
                } else {
                    ui.label(RichText::new(ui_format!("发现 {} 个新增串口", "Found {} new serial ports",candidates.len())).size(16.0).strong().color(TEAL));
                    label(ui,if candidates.len()>1 {t("请选择与你刚插回的设备对应的一项；也可重新拔插确认。")} else {t("确认下方设备信息，再应用到当前页面。")});
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical().id_salt("port-candidates").max_height(220.0).show(ui,|ui| {
                        for port in &candidates {
                            egui::Frame::new().fill(if self.selected==port.path {Color32::from_rgb(232,244,237)} else {Color32::from_rgb(247,249,247)})
                                .stroke(egui::Stroke::new(1.0,LINE)).corner_radius(10).inner_margin(12).show(ui,|ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.radio_value(&mut self.selected,port.path.clone(),RichText::new(port.title()).strong());
                                    ui.label(RichText::new(&port.path).monospace().size(12.0));
                                    label(ui,&display(&port.identity()));
                                });ui.add_space(6.0);
                        }
                    });
                    ui.add_space(12.0);
                    if ui.add_enabled(!self.selected.is_empty(),egui::Button::new(RichText::new(t("使用此串口")).color(Color32::WHITE)).fill(TEAL).min_size(Vec2::new(ui.available_width(),40.0))).clicked() {
                        // Rescan before applying: a candidate removed since the last poll must not be used.
                        let intended=self.selected.clone();
                        self.refresh();
                        if let Some(port)=self.discovery.as_ref().and_then(|d|d.selected(&intended)) {chosen=Some((target,port));}
                        else if self.error.is_none() {self.error=Some(t("刚选择的串口已移除，请重新选择。").into());}
                    }
                }
                ui.add_space(12.0);
                if ui.button(t("重新从拔下设备开始")).clicked() {self.discovery=None;self.selected.clear();self.error=None;}
            }
            if let Some(error)=&self.error {ui.add_space(8.0);ui.colored_label(Color32::from_rgb(185,64,52),display(error).as_ref());}
            ui.add_space(14.0);ui.separator();ui.add_space(8.0);
            label(ui,t("此过程只读取设备列表，不打开串口，也不向芯片发送指令。识别到的是 USB 转串口，不代表已确认后端芯片型号。"));
            if ui.button(t("取消")).clicked() {self.target=None;}
        });
        if response.should_close() || chosen.is_some() {
            self.target = None;
        }
        chosen
    }
}

pub(super) fn select_ui(ui: &mut egui::Ui, id: &str, path: &mut String, ports: &[PortDetails]) {
    let selected = ports.iter().find(|p| p.path == *path);
    let title = selected.map(|p| p.title()).unwrap_or_else(|| {
        if path.is_empty() {
            t("选择串口…").into()
        } else {
            path.clone()
        }
    });
    egui::ComboBox::from_id_salt(id)
        .width((ui.available_width() - 64.0).max(130.0))
        .wrap_mode(egui::TextWrapMode::Truncate)
        .selected_text(RichText::new(title).size(12.0))
        .show_ui(ui, |ui| {
            for port in ports {
                let response = ui.selectable_value(
                    path,
                    port.path.clone(),
                    format!("{} · {}", port.title(), port.path),
                );
                response.on_hover_text(display(&port.identity()).into_owned());
            }
        });
}
pub(super) fn details_ui(ui: &mut egui::Ui, path: &str, ports: &[PortDetails]) {
    if path.is_empty() {
        return;
    }
    ui.add(egui::Label::new(RichText::new(path).size(11.0).monospace().color(MUTED)).truncate())
        .on_hover_text(path);
    if let Some(port) = ports.iter().find(|p| p.path == path) {
        egui::CollapsingHeader::new(t("设备信息"))
            .id_salt(path)
            .show(ui, |ui| {
                label(ui, &display(&port.identity()));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn port(path: &str) -> PortDetails {
        PortDetails {
            path: path.into(),
            product: None,
            manufacturer: None,
            serial: None,
            usb_id: Some((0x1234, 0x5678)),
            bluetooth: false,
        }
    }
    #[test]
    fn baseline_never_becomes_a_candidate_without_removal() {
        let old = port("/dev/cu.usbserial-old");
        let new = port("/dev/cu.usbserial-new");
        let mut d = Discovery::new(std::slice::from_ref(&old));
        d.observe(&[old.clone(), new.clone()]);
        assert_eq!(d.candidates, [new]);
        assert!(d.selected(&old.path).is_none());
    }
    #[test]
    fn replug_with_same_path_is_detected_after_disappearance() {
        let target = port("/dev/cu.usbserial-110");
        let mut d = Discovery::new(std::slice::from_ref(&target));
        d.observe(&[]);
        d.observe(std::slice::from_ref(&target));
        assert_eq!(d.candidates, [target]);
    }
    #[test]
    fn unplugged_candidate_is_not_applicable() {
        let target = port("/dev/cu.usbserial-110");
        let mut d = Discovery::new(&[]);
        d.observe(std::slice::from_ref(&target));
        assert!(d.selected(&target.path).is_some());
        d.observe(&[]);
        assert!(d.selected(&target.path).is_none());
        d.observe(&[port("/dev/cu.usbserial-replacement")]);
        assert!(d.selected(&target.path).is_none());
    }
    #[test]
    fn multiple_candidates_are_preserved_not_guessed() {
        let a = port("/dev/cu.usbserial-a");
        let b = port("/dev/cu.usbserial-b");
        let mut d = Discovery::new(&[]);
        d.observe(&[a.clone(), b.clone()]);
        assert_eq!(d.candidates, [a, b]);
        assert!(single_usb(&d.candidates).is_none());
    }
    #[test]
    fn excludes_bluetooth_and_system_console_from_discovery() {
        let mut bluetooth = port("/dev/cu.headphones");
        bluetooth.bluetooth = true;
        let mut d = Discovery::new(&[]);
        d.observe(&[
            bluetooth,
            port("/dev/cu.Bluetooth-Incoming-Port"),
            port("/dev/cu.debug-console"),
        ]);
        assert!(d.candidates.is_empty());
    }
    #[test]
    fn unknown_usb_driver_metadata_does_not_prevent_detection() {
        let mut unknown = port("/dev/cu.custom-driver");
        unknown.usb_id = None;
        let mut d = Discovery::new(&[]);
        d.observe(std::slice::from_ref(&unknown));
        assert_eq!(d.candidates, [unknown]);
    }
    #[test]
    fn metadata_retained_and_macos_tty_alias_removed() {
        let input = vec![
            serialport::SerialPortInfo {
                port_name: "/dev/cu.usbserial-110".into(),
                port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                    vid: 0x1234,
                    pid: 0xabcd,
                    serial_number: Some("abc".into()),
                    manufacturer: Some("Vendor".into()),
                    product: Some("USB UART".into()),
                }),
            },
            serialport::SerialPortInfo {
                port_name: "/dev/tty.usbserial-110".into(),
                port_type: serialport::SerialPortType::Unknown,
            },
        ];
        let ports = normalize(input);
        assert_eq!(ports[0].title(), "USB UART");
        assert!(ports[0].identity().contains("1234:ABCD"));
        assert!(ports[0].identity().contains("abc"));
        if cfg!(target_os = "macos") {
            assert_eq!(ports.len(), 1);
        }
    }
}

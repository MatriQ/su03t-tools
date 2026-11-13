use super::i18n::{choose as tr, display, t, ui_format};
use super::{MUTED, TEAL, card, label, port_picker, section};
use eframe::egui::{self, Color32, RichText, Vec2};
use std::path::PathBuf;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use su03t_update::serial_debug::{self as serial, Config, Event, Session, TextDecoder};
use su03t_update::{
    capture::{self, RawBuffer},
    profiles::{CAR_COMMANDS, CarTracker, Profile},
};

const LOG_LIMIT: usize = 1024 * 1024;
const RECORD_LIMIT: usize = 2000;
#[derive(Clone, Copy, PartialEq)]
enum Format {
    Hex,
    Text,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_language_translates_system_records_but_preserves_device_text() {
        let mut debugger = Debugger::default();
        debugger.info("串口已打开。说出模块唤醒词和命令，观察接收数据。");
        debugger.receive_event(Event::Received {
            at: Duration::from_millis(10),
            bytes: "停止\r\n".as_bytes().to_vec(),
        });
        super::super::i18n::set(super::super::i18n::Language::En, false);
        let output = debugger.export();
        assert!(output.starts_with("SU-03T serial debug log"));
        assert!(output.contains("Serial port open."));
        assert!(output.contains("停止"));
        super::super::i18n::set(super::super::i18n::Language::Zh, false);
        assert!(debugger.export().starts_with("SU-03T 串口调试日志"));
    }
    #[test]
    fn clearing_display_preserves_session_counters_raw_capture_and_command_counts() {
        let mut debugger = Debugger::default();
        debugger.receive_event(Event::Received {
            at: Duration::from_millis(123),
            bytes: vec![0, 1, 0x80, 0xff],
        });
        debugger.receive_event(Event::Sent {
            at: Duration::from_millis(124),
            bytes: vec![2],
        });
        debugger.clear();
        assert!(debugger.records.is_empty());
        assert_eq!((debugger.rx, debugger.tx), (4, 1));
        assert_eq!(debugger.car.counts, [1, 0, 0, 0, 1]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.bin");
        debugger.raw.save(&path).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), [0, 1, 0x80, 0xff]);
        debugger.set_profile(Profile::B8);
        assert_eq!(debugger.baud, 57600);
        assert!(debugger.profile.receive_only());
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Ending {
    None,
    Lf,
    Cr,
    CrLf,
}
impl Ending {
    fn name(self) -> &'static str {
        match self {
            Self::None => t("不追加"),
            Self::Lf => "LF",
            Self::Cr => "CR",
            Self::CrLf => "CRLF",
        }
    }
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::Cr => b"\r",
            Self::CrLf => b"\r\n",
        }
    }
}
fn bits_name(bits: serialport::DataBits) -> &'static str {
    match bits {
        serialport::DataBits::Five => "5",
        serialport::DataBits::Six => "6",
        serialport::DataBits::Seven => "7",
        serialport::DataBits::Eight => "8",
    }
}
fn stop_name(bits: serialport::StopBits) -> &'static str {
    match bits {
        serialport::StopBits::One => "1",
        serialport::StopBits::Two => "2",
    }
}

struct Record {
    at: Duration,
    direction: &'static str,
    bytes: Vec<u8>,
    text: String,
}
impl Record {
    fn cost(&self) -> usize {
        self.bytes.len() + self.text.len() + 96
    }
}

pub(super) struct Debugger {
    port: String,
    ports: Vec<port_picker::PortDetails>,
    baud: u32,
    data_bits: serialport::DataBits,
    parity: serialport::Parity,
    stop_bits: serialport::StopBits,
    flow_control: serialport::FlowControl,
    profile: Profile,
    session: Option<Session>,
    connected: bool,
    closing: bool,
    status: String,
    error: Option<String>,
    records: VecDeque<Record>,
    log_cost: usize,
    removed: u64,
    rx: u64,
    tx: u64,
    errors: u64,
    elapsed: Duration,
    raw: RawBuffer,
    car: CarTracker,
    record_enabled: bool,
    record_dir: Option<PathBuf>,
    active_capture: Option<PathBuf>,
    last_scan: Instant,
    decoder: TextDecoder,
    started: Instant,
    connection_label: String,
    view: Format,
    send_format: Format,
    ending: Ending,
    input: String,
    input_error: Option<String>,
    follow: bool,
    log_offset: f32,
    evicted_pending: u64,
    copied: Option<Instant>,
    split: f32,
    jump: bool,
    unseen: u64,
}
impl Default for Debugger {
    fn default() -> Self {
        let mut me = Self {
            port: String::new(),
            ports: vec![],
            baud: 9600,
            data_bits: serialport::DataBits::Eight,
            parity: serialport::Parity::None,
            stop_bits: serialport::StopBits::One,
            flow_control: serialport::FlowControl::None,
            profile: Profile::Car,
            session: None,
            connected: false,
            closing: false,
            status: "未连接".into(),
            error: None,
            records: VecDeque::new(),
            log_cost: 0,
            removed: 0,
            rx: 0,
            tx: 0,
            errors: 0,
            elapsed: Duration::ZERO,
            raw: RawBuffer::default(),
            car: CarTracker::default(),
            record_enabled: false,
            record_dir: None,
            active_capture: None,
            last_scan: Instant::now(),
            decoder: TextDecoder::default(),
            started: Instant::now(),
            connection_label: String::new(),
            view: Format::Hex,
            send_format: Format::Hex,
            ending: Ending::None,
            input: String::new(),
            input_error: None,
            follow: true,
            log_offset: 0.0,
            evicted_pending: 0,
            copied: None,
            split: 0.53,
            jump: false,
            unseen: 0,
        };
        me.refresh();
        me
    }
}
impl Debugger {
    pub(super) fn set_profile(&mut self, profile: Profile) {
        if self.busy() {
            return;
        }
        self.profile = profile;
        self.baud = profile.baud();
        self.data_bits = serialport::DataBits::Eight;
        self.parity = serialport::Parity::None;
        self.stop_bits = serialport::StopBits::One;
        self.flow_control = serialport::FlowControl::None;
        self.view = if profile == Profile::B8 {
            Format::Text
        } else {
            Format::Hex
        };
    }
    pub(super) fn busy(&self) -> bool {
        self.session.is_some()
    }
    fn refresh(&mut self) {
        match port_picker::scan() {
            Ok(ports) => {
                if self.port.is_empty() {
                    self.port = port_picker::single_usb(&ports).unwrap_or_default();
                }
                self.ports = ports;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("无法刷新串口：{e}")),
        }
    }
    pub(super) fn select_port(&mut self, path: String) {
        if !self.busy() {
            self.port = path;
            self.refresh();
        }
    }
    fn append(&mut self, record: Record) {
        if !self.follow {
            self.unseen = self.unseen.saturating_add(1);
        }
        self.log_cost += record.cost();
        self.records.push_back(record);
        while self.records.len() > RECORD_LIMIT || self.log_cost > LOG_LIMIT {
            if let Some(old) = self.records.pop_front() {
                self.log_cost -= old.cost();
                self.removed += 1;
                if !self.follow {
                    self.evicted_pending += 1;
                }
            }
        }
    }
    fn info(&mut self, text: impl Into<String>) {
        self.append(Record {
            at: self.started.elapsed(),
            direction: "SYS",
            bytes: vec![],
            text: text.into(),
        });
    }
    fn clear(&mut self) {
        self.records.clear();
        self.log_offset = 0.0;
        self.evicted_pending = 0;
        self.log_cost = 0;
        self.removed = 0;
        self.copied = None;
        self.follow = true;
        self.jump = true;
        self.unseen = 0;
    }
    fn connect(&mut self) {
        if self.busy() || self.port.trim().is_empty() || self.baud == 0 {
            return;
        }
        self.clear();
        self.rx = 0;
        self.tx = 0;
        self.errors = 0;
        self.elapsed = Duration::ZERO;
        self.raw = RawBuffer::default();
        self.car = CarTracker::default();
        self.decoder = TextDecoder::default();
        self.error = None;
        self.input_error = None;
        self.started = Instant::now();
        self.closing = false;
        self.connected = false;
        self.status = "正在连接…".into();
        let config = Config {
            port: self.port.trim().into(),
            baud: self.baud,
            data_bits: self.data_bits,
            parity: self.parity,
            stop_bits: self.stop_bits,
            flow_control: self.flow_control,
            receive_only: self.profile.receive_only(),
        };
        self.connection_label = format!(
            "{} | {} | {}",
            capture::environment(),
            self.profile.name(),
            config.description()
        );
        self.info(format!(
            "连接 {}。接收模式，不自动发送数据。",
            self.connection_label
        ));
        self.active_capture = if self.record_enabled {
            self.record_dir.as_ref().map(|dir| {
                dir.join(format!(
                    "su03t-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                ))
            })
        } else {
            None
        };
        if let Some(prefix) = &self.active_capture {
            self.info(format!(
                "连续记录：{}.rx.bin / .log；16MiB×4文件/每流，超限滚动；清空显示不影响文件。",
                prefix.display()
            ));
        }
        self.session = Some(Session::connect_recorded(
            config,
            self.active_capture.clone().map(capture::Options::new),
        ));
    }
    fn receive_event(&mut self, event: Event) {
        match event {
            Event::Connected => {
                if !self.closing {
                    self.connected = true;
                    self.status = "已连接 · 正在监听".into();
                }
                self.info(if self.profile == Profile::B8 {
                    "B8只接收日志。请按需重启模块；实际日志取决于固件是否启用。"
                } else {
                    "串口已打开。说出模块唤醒词和命令，观察接收数据。"
                });
            }
            Event::Received { at, bytes } => {
                self.rx = self.rx.saturating_add(bytes.len() as u64);
                self.raw.push(&bytes);
                if self.profile == Profile::Car {
                    self.car.observe(&bytes);
                }
                for chunk in bytes.chunks(64) {
                    let text = self.decoder.push(chunk);
                    self.append(Record {
                        at,
                        direction: "RX",
                        bytes: chunk.to_vec(),
                        text,
                    });
                }
            }
            Event::Sent { at, bytes } => {
                self.tx = self.tx.saturating_add(bytes.len() as u64);
                let mut decoder = TextDecoder::default();
                for chunk in bytes.chunks(64) {
                    let text = decoder.push(chunk);
                    self.append(Record {
                        at,
                        direction: "TX",
                        bytes: chunk.to_vec(),
                        text,
                    });
                }
                let tail = decoder.finish();
                if !tail.is_empty() {
                    self.append(Record {
                        at,
                        direction: "TXT",
                        bytes: vec![],
                        text: tail,
                    });
                }
            }
        }
    }
    pub(super) fn poll(&mut self) {
        if self.busy() {
            self.elapsed = self.started.elapsed();
        }
        if !self.busy() && self.last_scan.elapsed() >= Duration::from_secs(1) {
            self.last_scan = Instant::now();
            if let Ok(ports) = port_picker::scan() {
                if self.port.is_empty() {
                    self.port = port_picker::single_usb(&ports).unwrap_or_default();
                }
                self.ports = ports;
            }
        }
        for _ in 0..256 {
            let event = self.session.as_ref().and_then(Session::try_event);
            if let Some(event) = event {
                self.receive_event(event);
            } else {
                break;
            }
        }
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.finished_result().is_some())
            && let Some(session) = self.session.take()
        {
            // Completion means no more events can be produced: preserve the final data.
            while let Some(event) = session.try_event() {
                self.receive_event(event);
            }
            let tail = self.decoder.finish();
            if !tail.is_empty() {
                self.info(format!("接收尾部不完整 UTF-8 字节：{tail}"));
            }
            match session.finished_result().expect("completed") {
                Ok(()) => {
                    self.status = "已断开".into();
                    self.info("连接已关闭，串口已释放。");
                }
                Err(message) => {
                    self.errors += 1;
                    self.status = "连接已结束".into();
                    self.info(&message);
                    self.error = Some(message);
                }
            }
            self.connected = false;
            self.closing = false;
        }
    }
    fn send(&mut self) {
        if !self.connected || self.closing || self.profile.receive_only() {
            return;
        }
        let payload = if self.send_format == Format::Hex {
            serial::parse_hex(&self.input)
        } else {
            serial::text_payload(&self.input, self.ending.bytes())
        };
        self.input_error = match payload {
            Ok(bytes) => self
                .session
                .as_ref()
                .and_then(|s| s.send(bytes).err())
                .map(|e| e.to_string()),
            Err(e) => Some(e.to_string()),
        };
        if self.input_error.is_some() {
            self.errors += 1;
        }
    }
    fn export(&self) -> String {
        let mut text = display(&format!(
            "SU-03T 串口调试日志\n{}\nRX={} bytes, TX={} bytes, errors={}, duration={:.3}s\n相对时间；TX仅表示主机写入；记录按串口读取分块，不代表协议帧。\n已移除{}条较早记录；导出当前显示缓冲；独立RX缓冲保留{}字节，移除{}字节。\n\n",
            self.connection_label,
            self.rx,
            self.tx,
            self.errors,
            self.elapsed.as_secs_f64(),
            self.removed,
            self.raw.len(),
            self.raw.removed
        )).into_owned();
        for r in &self.records {
            text.push_str(&format!(
                "[{:.3}s] {} HEX=[{}] TEXT={}\n",
                r.at.as_secs_f64(),
                r.direction,
                serial::hex(&r.bytes),
                if r.direction == "SYS" {
                    display(&r.text).into_owned()
                } else {
                    r.text.clone()
                }
                .replace('\r', "\\r")
                .replace('\n', "\\n")
            ));
        }
        text
    }
    fn controls(&mut self, ui: &mut egui::Ui, blocked: bool, finder: &mut port_picker::Finder) {
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(if self.profile == Profile::B8 {
                    t("B8 日志接收")
                } else {
                    t("语音串口调试")
                })
                .size(19.0)
                .strong(),
            );
            label(ui, t(self.profile.name()));
            ui.add_space(18.0);
            ui.add_enabled_ui(!self.busy() && !blocked, |ui| {
                if self.profile != Profile::B8 {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(self.profile == Profile::Car, t("小车 · 9600"))
                            .clicked()
                        {
                            self.set_profile(Profile::Car);
                        }
                        if ui
                            .selectable_label(self.profile == Profile::Custom, t("通用 · 115200"))
                            .clicked()
                        {
                            self.set_profile(Profile::Custom);
                        }
                    });
                    ui.add_space(12.0);
                }
                section(ui, "01", t("选择串口"));
                ui.horizontal(|ui| {
                    port_picker::select_ui(ui, "debug-port", &mut self.port, &self.ports);
                    if ui.button(t("刷新")).clicked() {
                        self.refresh();
                    }
                });
                port_picker::details_ui(ui, &self.port, &self.ports);
                if ui.button(t("通过插拔识别串口…")).clicked() {
                    finder.start(port_picker::Target::Debug);
                }
                egui::CollapsingHeader::new(t("手动输入串口路径")).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.port)
                            .hint_text(if cfg!(target_os = "linux") {
                                "/dev/ttyUSB0"
                            } else {
                                "/dev/cu.usbserial-…"
                            })
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(14.0);
                section(ui, "02", t("通信参数"));
                ui.horizontal(|ui| {
                    label(ui, t("波特率"));
                    egui::ComboBox::from_id_salt("debug-baud")
                        .width(135.0)
                        .selected_text(self.baud.to_string())
                        .show_ui(ui, |ui| {
                            for baud in [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600]
                            {
                                ui.selectable_value(&mut self.baud, baud, baud.to_string());
                            }
                        });
                });
                ui.horizontal(|ui| {
                    label(ui, t("自定义"));
                    ui.add(
                        egui::DragValue::new(&mut self.baud)
                            .range(1..=3_000_000)
                            .speed(100),
                    );
                });
                ui.add_space(6.0);
                egui::Grid::new("debug-format")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        label(ui, t("数据位"));
                        egui::ComboBox::from_id_salt("debug-data")
                            .width(135.0)
                            .selected_text(bits_name(self.data_bits))
                            .show_ui(ui, |ui| {
                                for v in [
                                    serialport::DataBits::Five,
                                    serialport::DataBits::Six,
                                    serialport::DataBits::Seven,
                                    serialport::DataBits::Eight,
                                ] {
                                    ui.selectable_value(&mut self.data_bits, v, bits_name(v));
                                }
                            });
                        ui.end_row();
                        label(ui, t("流控"));
                        ui.add_enabled_ui(!self.profile.receive_only(), |ui| {
                            egui::ComboBox::from_id_salt("debug-flow")
                                .width(135.0)
                                .selected_text(match self.flow_control {
                                    serialport::FlowControl::None => t("无"),
                                    serialport::FlowControl::Hardware => "RTS/CTS",
                                    serialport::FlowControl::Software => "XON/XOFF",
                                })
                                .show_ui(ui, |ui| {
                                    for (flow, name) in [
                                        (serialport::FlowControl::None, t("无")),
                                        (serialport::FlowControl::Hardware, "RTS/CTS"),
                                        (serialport::FlowControl::Software, "XON/XOFF"),
                                    ] {
                                        ui.selectable_value(&mut self.flow_control, flow, name);
                                    }
                                });
                        });
                        ui.end_row();
                        label(ui, t("校验位"));
                        egui::ComboBox::from_id_salt("debug-parity")
                            .width(135.0)
                            .selected_text(match self.parity {
                                serialport::Parity::None => t("None / 无"),
                                serialport::Parity::Even => t("Even / 偶"),
                                serialport::Parity::Odd => t("Odd / 奇"),
                            })
                            .show_ui(ui, |ui| {
                                for (v, s) in [
                                    (serialport::Parity::None, t("None / 无")),
                                    (serialport::Parity::Even, t("Even / 偶")),
                                    (serialport::Parity::Odd, t("Odd / 奇")),
                                ] {
                                    ui.selectable_value(&mut self.parity, v, s);
                                }
                            });
                        ui.end_row();
                        label(ui, t("停止位"));
                        egui::ComboBox::from_id_salt("debug-stop")
                            .width(135.0)
                            .selected_text(stop_name(self.stop_bits))
                            .show_ui(ui, |ui| {
                                for v in [serialport::StopBits::One, serialport::StopBits::Two] {
                                    ui.selectable_value(&mut self.stop_bits, v, stop_name(v));
                                }
                            });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                label(ui, t("参数仅用于当前连接，不会修改模块固件。"));
                ui.add_space(14.0);
                section(ui, "03", t("连续记录"));
                label(
                    ui,
                    t("新连接开始新会话，重置统计与内存RX缓冲；已保存文件保持不变。"),
                );
                ui.checkbox(&mut self.record_enabled, t("接收原始 BIN + 收发文本日志"));
                if self.record_enabled {
                    if ui.button(t("选择保存目录…")).clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        self.record_dir = Some(dir);
                    }
                    if let Some(dir) = &self.record_dir {
                        label(ui, &dir.display().to_string());
                    } else {
                        label(ui, t("连接前请选择目录。"));
                    }
                    label(
                        ui,
                        t("每流最多4个16MiB文件，超限覆盖本会话最早分卷；每次连接创建新文件。"),
                    );
                }
            });
            ui.add_space(18.0);
            if self.busy() {
                if ui
                    .add_enabled(
                        !self.closing,
                        egui::Button::new(if self.closing {
                            t("正在断开…")
                        } else {
                            t("断开连接")
                        })
                        .min_size(Vec2::new(ui.available_width(), 40.0)),
                    )
                    .clicked()
                {
                    if let Some(s) = &self.session {
                        s.stop();
                    }
                    self.closing = true;
                    self.status = t("正在断开…").into();
                }
            } else if ui
                .add_enabled(
                    !blocked
                        && !self.port.trim().is_empty()
                        && self.baud > 0
                        && (!self.record_enabled || self.record_dir.is_some()),
                    egui::Button::new(
                        RichText::new(t("连接并监听  →"))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(TEAL)
                    .min_size(Vec2::new(ui.available_width(), 40.0)),
                )
                .clicked()
            {
                self.connect();
            }
            ui.add_space(8.0);
            label(
                ui,
                if blocked {
                    t("固件升级正在占用串口，请先停止升级。")
                } else {
                    t("连接后不会自动发送任何指令。")
                },
            );
            if let Some(error) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(Color32::from_rgb(185, 64, 52), display(error).as_ref());
            }
        });
        ui.add_space(14.0);
        egui::Frame::new().fill(Color32::from_rgb(234,240,234)).corner_radius(12).inner_margin(16).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(t("调试接线")).strong().color(TEAL));
            ui.add_space(6.0);
            ui.label(t(self.profile.wiring()));
            label(ui,t("模块独立供电时，转接器 VCC 不接；必须共地。串口信号电平 3.3V。"));
            ui.add_space(8.0);
            label(ui, if self.profile == Profile::B8 { t("57600 / 8N1 为蜂鸟M日志预设；实际输出取决于固件。B8连接不会发送数据或触发升级。") } else { t("业务引脚按固件配置；小车版本20261107001使用B6/B7。其他固件需核对映射，板上丝印不能证明当前输出位置。") });
        });
    }
    fn overview(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let color = if self.connected && !self.closing {
                    TEAL
                } else {
                    MUTED
                };
                ui.label(
                    RichText::new(display(&self.status).as_ref())
                        .size(16.0)
                        .strong()
                        .color(color),
                );
                if self.busy() && !self.connected {
                    ui.spinner();
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("RX  {} B", self.rx))
                        .monospace()
                        .color(TEAL),
                );
                ui.add_space(18.0);
                ui.label(
                    RichText::new(format!("TX  {} B", self.tx))
                        .monospace()
                        .color(Color32::from_rgb(58, 105, 153)),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(ui_format!(
                        "错误 {}  ·  {:.1}s",
                        "Errors {} · {:.1}s",
                        self.errors,
                        self.elapsed.as_secs_f64()
                    ))
                    .monospace()
                    .color(MUTED),
                );
            });
            label(ui, t("TX 计数表示主机写入，不代表模块已确认。"));
        });
        ui.add_space(14.0);
        if self.profile == Profile::Car {
            card().inner_margin(16).show(ui, |ui| {
                ui.label(RichText::new(t("小车语音反馈")).strong());
                label(
                    ui,
                    t("唤醒词：你好小美 · 仅解释20261107001固件的单字节输出；不自动发送指令"),
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for (index, (code, name)) in CAR_COMMANDS.iter().enumerate() {
                        let name = t(name);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(format!("{name} / {code:02X}"))
                                    .monospace()
                                    .color(TEAL),
                            );
                            ui.label(
                                RichText::new(ui_format!("{} 次", "{} times", self.car.counts[index]))
                                    .size(20.0)
                                    .strong(),
                            );
                        });
                        ui.add_space(14.0);
                    }
                });
                label(
                    ui,
                    &ui_format!("最近：{} · 未知字节 {} · 次数为实际接收统计，不代表已完成实板验收", "Latest: {} · Unknown bytes: {} · Counts report received data, not hardware acceptance",
                        t(self.car.last.unwrap_or("等待语音反馈")),
                        self.car.unknown
                    ),
                );
            });
            ui.add_space(14.0);
        }
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(Color32::from_rgb(33,54,50)).corner_radius(10).inner_margin(12).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            *ui.visuals_mut() = egui::Visuals::dark(); ui.visuals_mut().override_text_color = Some(Color32::from_rgb(218,233,223));
            ui.horizontal_wrapped(|ui| {
                ui.strong(t("收发记录"));
                for (format,name) in [(Format::Hex,"HEX"),(Format::Text,"UTF-8")] {ui.selectable_value(&mut self.view,format,name);}
                ui.label(egui::RichText::new(if self.follow {tr("跟随最新","Following latest").to_owned()} else {format!("{} · +{}",tr("已暂停跟随","Follow paused"),self.unseen)}).size(10.0));
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!self.follow,egui::Button::new(tr("回到底部并跟随","Jump to latest & follow")).small()).clicked(){self.jump=true;self.follow=true;self.unseen=0;}
                if ui.add_enabled(!self.records.is_empty(),egui::Button::new(if self.copied.is_some_and(|t|t.elapsed()<Duration::from_secs(2)){t("已复制")}else{t("复制日志")}).small()).clicked(){ui.ctx().copy_text(self.export());self.copied=Some(Instant::now());}
                if ui.add_enabled(!self.records.is_empty(),egui::Button::new(t("保存日志…")).small()).clicked()&&let Some(path)=rfd::FileDialog::new().set_file_name("su03t-debug.log").add_filter(t("日志"),&["log","txt"]).save_file(){match std::fs::write(&path,self.export()){Ok(())=>self.info(format!("日志已保存：{}",path.display())),Err(e)=>self.error=Some(format!("保存日志失败：{e}"))}}
                if ui.add_enabled(!self.records.is_empty(),egui::Button::new(t("清空显示")).small()).clicked(){self.clear();}
                if ui.add_enabled(!self.raw.is_empty(),egui::Button::new(t("导出接收 BIN…")).small()).clicked()&&let Some(path)=rfd::FileDialog::new().set_file_name("su03t-rx.bin").add_filter(t("原始接收字节"),&["bin"]).save_file(){match self.raw.save(&path){Ok(())=>self.info(format!("原始接收缓冲已保存：{}（{}字节）",path.display(),self.raw.len())),Err(error)=>{self.errors+=1;self.error=Some(format!("保存原始数据失败：{error}"));}}}
            });
            ui.add_space(5.0);ui.separator();ui.add_space(5.0);
            let height=(ui.available_height()-38.0).max(40.0);
            let mut scroll=egui::ScrollArea::both().id_salt("debug-console").auto_shrink([false,false]).max_height(height).stick_to_bottom(true);
            if self.jump{scroll=scroll.vertical_scroll_offset(f32::MAX);self.jump=false;}
            else if self.evicted_pending>0 && !self.follow {scroll=scroll.vertical_scroll_offset((self.log_offset-self.evicted_pending as f32*16.0).max(0.0));}
            self.evicted_pending=0;
            let out=ui.scope(|ui|{ui.spacing_mut().item_spacing.y=0.0;
                if self.records.is_empty(){scroll.show(ui,|ui|{ui.set_min_height(height);ui.label(egui::RichText::new(t("连接后，说出唤醒词和指令，观察模块输出。")).size(11.0));})}
                else{scroll.show_rows(ui,16.0,self.records.len(),|ui,range|{for index in range{
                    let record=&self.records[index];
                    let value=if self.view==Format::Hex&&!record.bytes.is_empty(){serial::hex(&record.bytes)}else{if record.direction=="SYS"{display(&record.text).into_owned()}else{record.text.clone()}.replace('\r',"\\r").replace('\n',"\\n")};
                    let color=match record.direction{"RX"=>Color32::from_rgb(152,215,185),"TX"=>Color32::from_rgb(150,193,242),_=>Color32::from_rgb(181,208,191)};
                    ui.add(egui::Label::new(egui::RichText::new(format!("[{:8.3}s] {} {value}",record.at.as_secs_f64(),record.direction)).monospace().size(11.0).line_height(Some(16.0)).color(color)).extend().selectable(true));
                }})}
            }).inner;
            self.log_offset=out.state.offset.y;
            self.follow=out.state.offset.y+out.inner_rect.height()>=out.content_size.y-0.5;if self.follow{self.unseen=0;}
            ui.add_space(5.0);ui.label(egui::RichText::new(ui_format!("显示缓冲已移除 {} 条 · RX缓冲保留 {} B，移除 {} B · 清空显示保留计数、RX缓冲及记录文件", "Display dropped {} records · RX buffer: {} B, dropped {} B · Clearing keeps counts, RX buffer and recorded files",self.removed,self.raw.len(),self.raw.removed)).size(10.0));
        });
    }
    fn sending(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        if self.profile.receive_only() {
            card().inner_margin(16).show(ui, |ui| {
                ui.label(RichText::new(t("只接收 · 不发送")).strong().color(TEAL));
                label(
                    ui,
                    t("B8 模式在通信核心禁止发送；转接器 TXD 保持不接。请用此模式观察上电日志。"),
                );
            });
            return;
        }
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("手动发送")).strong());
                ui.add_space(10.0);
                ui.selectable_value(&mut self.send_format, Format::Hex, "HEX");
                ui.selectable_value(&mut self.send_format, Format::Text, t("文本 UTF-8"));
            });
            ui.add_space(6.0);
            let input_id = ui.make_persistent_id("debug-send-input");
            let shortcut = ui.memory(|m| m.has_focus(input_id))
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
            ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .id(input_id)
                    .char_limit(400_000)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(if self.send_format == Format::Hex {
                        t("例如 AA 01 55（仅示例，需与你的固件协议一致）")
                    } else {
                        t("输入要发送给模块的文本")
                    }),
            );
            ui.add_space(8.0);
            let mut trigger = false;
            ui.horizontal(|ui| {
                if self.send_format == Format::Text {
                    label(ui, t("追加结尾"));
                    egui::ComboBox::from_id_salt("debug-ending")
                        .width(85.0)
                        .selected_text(self.ending.name())
                        .show_ui(ui, |ui| {
                            for e in [Ending::None, Ending::Lf, Ending::Cr, Ending::CrLf] {
                                ui.selectable_value(&mut self.ending, e, e.name());
                            }
                        });
                } else {
                    label(ui, t("HEX 按原始字节发送，不追加结尾。"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.connected && !self.closing,
                            egui::Button::new(RichText::new(t("发送 →")).color(Color32::WHITE))
                                .fill(TEAL),
                        )
                        .clicked()
                    {
                        trigger = true;
                    }
                });
            });
            if shortcut {
                trigger = true;
            }
            if trigger {
                self.send();
            }
            if let Some(error) = &self.input_error {
                ui.colored_label(Color32::from_rgb(185, 64, 52), display(error).as_ref());
            }
            label(
                ui,
                if cfg!(target_os = "macos") {
                    t("⌘ Enter 发送 · 数据按输入原样发送，协议由固件定义。")
                } else {
                    t("Ctrl Enter 发送 · 数据按输入原样发送，协议由固件定义。")
                },
            );
        });
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        blocked: bool,
        finder: &mut port_picker::Finder,
    ) {
        let (left, right) = super::split_rects(ui, &mut self.split, "debug-splitter");
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            ui.set_clip_rect(left.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt("debug-workspace")
                .auto_shrink([false, false])
                .max_height(left.height())
                .show(ui, |ui| {
                    self.controls(ui, blocked, finder);
                    ui.add_space(12.0);
                    self.overview(ui);
                    ui.add_space(12.0);
                    self.sending(ui);
                    label(
                        ui,
                        t("听不到回包时，先核对固件是否启用了串口输出，再核对波特率、接线和共地。"),
                    );
                });
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            ui.set_clip_rect(right.intersect(ui.clip_rect()));
            self.log_panel(ui);
        });
    }
}

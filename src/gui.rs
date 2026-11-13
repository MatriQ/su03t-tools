//! Native desktop front end. Flashing runs in a separate CLI process so serial
//! waits never block the window, and Stop can terminate the transfer promptly.
use eframe::egui;
use egui::{Color32, RichText, Stroke, Vec2};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use su03t_update::{firmware::Image, profiles::Profile};

#[path = "debug_ui.rs"]
mod debug_ui;
#[path = "port_picker.rs"]
mod port_picker;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Upgrade,
    Debug,
    BootLog,
}

const MAX_LOG_LINES: usize = 2000;

enum Event {
    Line(String),
    Finished { success: bool, stopped: bool },
}

struct Job {
    cancel: Sender<()>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_lines(input: impl std::io::Read + Send + 'static, tx: Sender<Event>) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(input).lines() {
            match line {
                Ok(line) => {
                    if tx.send(Event::Line(line)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Event::Line(format!("读取烧录日志失败：{error}")));
                    return;
                }
            }
        }
    })
}

fn start_job(
    port: String,
    firmware: Option<PathBuf>,
    sha256: Option<String>,
    baud: u32,
    wait: u64,
    trigger: bool,
    probe: bool,
) -> std::io::Result<Job> {
    let executable = std::env::current_exe()?;
    let (cancel, cancel_rx) = mpsc::channel();
    let (event_tx, events) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut command = Command::new(executable);
        command
            .arg(if probe { "probe" } else { "flash" })
            .arg("--port")
            .arg(port)
            .arg("--baud")
            .arg(baud.to_string())
            .arg("--wait")
            .arg(wait.to_string());
        if probe {
            if trigger {
                command.arg("--trigger");
            }
        } else {
            command
                .arg("--firmware")
                .arg(firmware.expect("flash image"))
                .arg("--expected-sha256")
                .arg(sha256.expect("flash hash"));
            if !trigger {
                command.arg("--passive");
            }
        }
        let spawn = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawn {
            Ok(child) => child,
            Err(error) => {
                let _ = event_tx.send(Event::Line(format!("无法启动烧录程序：{error}")));
                let _ = event_tx.send(Event::Finished {
                    success: false,
                    stopped: false,
                });
                return;
            }
        };
        let stdout = read_lines(child.stdout.take().expect("piped stdout"), event_tx.clone());
        let stderr = read_lines(child.stderr.take().expect("piped stderr"), event_tx.clone());
        let mut stopped = false;
        let success = loop {
            if cancel_rx.try_recv().is_ok() {
                stopped = true;
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) => thread::sleep(Duration::from_millis(50)),
                Err(error) => {
                    let _ = event_tx.send(Event::Line(format!("检查烧录进程失败：{error}")));
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        // Reader completion precedes Finished, preserving the last output lines.
        let _ = stdout.join();
        let _ = stderr.join();
        let _ = event_tx.send(Event::Finished { success, stopped });
    });
    Ok(Job {
        cancel,
        events,
        worker: Some(worker),
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Opening,
    Waiting,
    Transferring,
    Verifying,
    Success,
    DiagnosticDone,
    Stopped,
    Error,
}

const INK: Color32 = Color32::from_rgb(28, 44, 46);
const MUTED: Color32 = Color32::from_rgb(109, 126, 127);
const TEAL: Color32 = Color32::from_rgb(15, 118, 110);
const LINE: Color32 = Color32::from_rgb(226, 233, 230);
const PAPER: Color32 = Color32::from_rgb(245, 247, 244);

struct Updater {
    ports: Vec<port_picker::PortDetails>,
    port: String,
    firmware: Option<PathBuf>,
    baud: u32,
    job: Option<Job>,
    stopping: bool,
    progress: f32,
    status: String,
    logs: VecDeque<String>,
    phase: Phase,
    file_error: Option<String>,
    file_size: Option<usize>,
    file_sha256: Option<String>,
    wait_seconds: u64,
    trigger: bool,
    probe_trigger: bool,
    diagnosing: bool,
    last_scan: Instant,
    acknowledged: usize,
    total: usize,
    started: Option<Instant>,
    duration: Option<Duration>,
    copied: Option<Instant>,
    mode: Mode,
    debugger: debug_ui::Debugger,
    finder: port_picker::Finder,
}

impl Default for Updater {
    fn default() -> Self {
        let mut app = Self {
            ports: Vec::new(),
            port: String::new(),
            firmware: None,
            baud: 921600,
            job: None,
            stopping: false,
            progress: 0.0,
            status: "选择串口和升级固件后开始。".into(),
            logs: VecDeque::new(),
            phase: Phase::Idle,
            file_error: None,
            file_size: None,
            file_sha256: None,
            wait_seconds: 60,
            trigger: true,
            probe_trigger: false,
            diagnosing: false,
            last_scan: Instant::now(),
            acknowledged: 0,
            total: 0,
            started: None,
            duration: None,
            copied: None,
            mode: Mode::Debug,
            debugger: debug_ui::Debugger::default(),
            finder: port_picker::Finder::default(),
        };
        app.refresh_ports();
        app
    }
}

impl Updater {
    fn log(&mut self, line: impl Into<String>) {
        if self.logs.len() == MAX_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(format!(
            "[{:8.3}s] {}",
            self.started
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0),
            line.into()
        ));
    }

    fn refresh_ports(&mut self) {
        match port_picker::scan() {
            Ok(ports) => {
                if !ports.iter().any(|p| p.path == self.port) {
                    self.port = port_picker::single_usb(&ports).unwrap_or_default();
                }
                self.ports = ports;
            }
            Err(error) => self.log(format!("枚举串口失败：{error}")),
        }
    }

    fn poll(&mut self) {
        if self.job.is_none() && self.last_scan.elapsed() >= Duration::from_secs(1) {
            self.last_scan = Instant::now();
            if let Ok(ports) = port_picker::scan() {
                if self.port.is_empty() {
                    self.port = port_picker::single_usb(&ports).unwrap_or_default();
                }
                self.ports = ports;
            }
        }
        let incoming: Vec<Event> = self
            .job
            .as_ref()
            .map(|job| job.events.try_iter().collect())
            .unwrap_or_default();
        for event in incoming {
            match event {
                Event::Line(line) => {
                    if let Some(rest) = line.strip_prefix("PROGRESS ") {
                        let mut fields = rest.split_whitespace();
                        if let (Some(done), Some(total)) = (fields.next(), fields.next())
                            && let (Ok(done), Ok(total)) =
                                (done.parse::<f32>(), total.parse::<f32>())
                            && total > 0.0
                        {
                            self.acknowledged = done as usize;
                            self.total = total as usize;
                            self.phase = if done >= total {
                                Phase::Verifying
                            } else {
                                Phase::Transferring
                            };
                            self.progress = (done / total).clamp(0.0, 1.0);
                            self.status = format!("正在传输 · {:.0}%", self.progress * 100.0);
                        }
                    } else {
                        if line.starts_with("串口已打开：") {
                            self.phase = Phase::Waiting;
                        }
                        self.log(line);
                    }
                }
                Event::Finished { success, stopped } => {
                    self.duration = self.started.map(|t| t.elapsed());
                    self.phase = if stopped {
                        Phase::Stopped
                    } else if success {
                        if self.diagnosing {
                            Phase::DiagnosticDone
                        } else {
                            Phase::Success
                        }
                    } else {
                        Phase::Error
                    };
                    self.status = if stopped {
                        "已停止。请将模块断电，重新开始后再上电。".into()
                    } else if success {
                        if self.diagnosing {
                            "启动握手已识别；未发送固件。".into()
                        } else {
                            self.progress = 1.0;
                            "设备确认传输完成；未读回校验，请重新上电验证语音。".into()
                        }
                    } else {
                        if self.diagnosing {
                            "诊断结束：未识别启动握手；本次未发送固件。".into()
                        } else {
                            "烧录未完成，请查看日志并检查接线及供电。".into()
                        }
                    };
                    self.log(self.status.clone());
                    self.job = None;
                    self.stopping = false;
                }
            }
        }
    }
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(16)
        .inner_margin(22)
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

fn section(ui: &mut egui::Ui, number: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(number).monospace().size(11.0).color(TEAL));
        ui.label(RichText::new(title).size(15.0).strong().color(INK));
    });
    ui.add_space(10.0);
}

fn chip(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(52.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 15.0, TEAL);
    let c = r.center();
    p.rect_stroke(
        egui::Rect::from_center_size(c, Vec2::splat(22.0)),
        4.0,
        Stroke::new(1.6, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    for offset in [-6.0, 0.0, 6.0] {
        for side in [-1.0, 1.0] {
            p.line_segment(
                [
                    c + Vec2::new(offset, 11.0 * side),
                    c + Vec2::new(offset, 16.0 * side),
                ],
                Stroke::new(1.4, Color32::WHITE),
            );
            p.line_segment(
                [
                    c + Vec2::new(11.0 * side, offset),
                    c + Vec2::new(16.0 * side, offset),
                ],
                Stroke::new(1.4, Color32::WHITE),
            );
        }
    }
    p.circle_filled(c, 2.5, Color32::from_rgb(167, 225, 194));
}

impl Updater {
    fn choose_file(&mut self, path: PathBuf) {
        self.phase = Phase::Idle;
        self.progress = 0.0;
        self.acknowledged = 0;
        self.total = 0;
        self.started = None;
        self.duration = None;
        self.file_sha256 = None;
        match Image::load(&path, false) {
            Ok(image) => {
                self.file_size = Some(image.bytes().len());
                self.file_sha256 = Some(image.sha256);
                self.file_error = None;
            }
            Err(error) => {
                self.file_size = None;
                let filename_ok = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with("_update.bin"));
                self.file_error = Some(if !filename_ok {
                    "这不是串口升级文件，请选择以 _update.bin 结尾的固件。".into()
                } else if error.to_string().contains("启动区") {
                    "所选文件包含启动程序，请更换为平台导出的串口升级文件。".into()
                } else {
                    format!("无法使用此固件：{error}")
                });
            }
        }
        self.firmware = Some(path);
    }

    fn begin(&mut self, probe: bool) {
        if self.debugger.busy() || self.job.is_some() || self.port.is_empty() {
            return;
        }
        if !probe && (self.file_error.is_some() || self.file_sha256.is_none()) {
            return;
        }
        self.logs.clear();
        self.progress = 0.0;
        self.acknowledged = 0;
        self.total = 0;
        self.duration = None;
        self.started = Some(Instant::now());
        self.phase = Phase::Opening;
        self.diagnosing = probe;
        self.status = "正在检查文件与打开串口…".into();
        match start_job(
            self.port.clone(),
            self.firmware.clone(),
            self.file_sha256.clone(),
            self.baud,
            self.wait_seconds,
            if probe {
                self.probe_trigger
            } else {
                self.trigger
            },
            probe,
        ) {
            Ok(job) => self.job = Some(job),
            Err(error) => {
                self.phase = Phase::Error;
                self.duration = self.started.map(|t| t.elapsed());
                self.status = "无法启动烧录程序".into();
                self.log(error.to_string());
            }
        }
    }

    fn phase_info(&self) -> (&'static str, &'static str, Color32) {
        match self.phase {
            Phase::Idle => ("准备就绪后，即可开始", "完成左侧配置，再开始升级。", TEAL),
            Phase::Opening => (
                "检查文件与打开串口",
                "确认文件内容未变化，并尝试独占打开所选串口。",
                TEAL,
            ),
            Phase::Waiting => (
                "等待启动握手",
                "请将模块断电再上电。此状态表示尚未收到 CCC，不能判断模块是否有电。",
                TEAL,
            ),
            Phase::Transferring => (
                "正在传输固件",
                "请保持连接，进度以设备确认的数据为准。",
                TEAL,
            ),
            Phase::Verifying => (
                "等待结束确认",
                "数据包已确认，正在等待设备确认传输结束。此步骤不执行读回校验。",
                TEAL,
            ),
            Phase::Success => (
                "设备确认传输完成",
                "尚未执行读回校验。请将模块重新上电，验证唤醒与语音指令。",
                TEAL,
            ),
            Phase::Stopped => (
                if self.diagnosing {
                    "诊断已停止"
                } else {
                    "升级已停止"
                },
                "将模块断电，重新开始后再上电。",
                MUTED,
            ),
            Phase::DiagnosticDone => (
                "已识别启动握手",
                "本次仅诊断启动，未传输固件；不能据此确认型号和升级兼容性。",
                TEAL,
            ),
            Phase::Error => (
                if self.diagnosing {
                    "启动诊断未建立握手"
                } else {
                    "本次升级未完成"
                },
                "查看接收字节数与HEX日志，再检查返回线路、波特率及启动程序兼容性。",
                Color32::from_rgb(185, 64, 52),
            ),
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        let running = self.job.is_some();
        let debug_busy = self.debugger.busy();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("升级配置").size(19.0).strong());
            label(ui, "按顺序选择设备和固件");
            ui.add_space(16.0);
            ui.add_enabled_ui(!running, |ui| {
                section(ui, "01", "连接设备");
                ui.horizontal(|ui| {
                    port_picker::select_ui(ui, "serial-port", &mut self.port, &self.ports);
                    if ui.button("刷新").clicked() {
                        self.refresh_ports();
                    }
                });
                port_picker::details_ui(ui, &self.port, &self.ports);
                if ui
                    .add_enabled(!debug_busy, egui::Button::new("通过插拔识别串口…"))
                    .clicked()
                {
                    self.finder.start(port_picker::Target::Upgrade);
                }
                ui.add_space(6.0);
                label(
                    ui,
                    if self.port.is_empty() {
                        "接入 USB 转串口后，点击刷新。"
                    } else {
                        "已选择串口，开始后将尝试连接。"
                    },
                );
                ui.add_space(14.0);
                section(ui, "02", "选择升级固件");
                let response = egui::Frame::new()
                    .fill(Color32::from_rgb(247, 250, 248))
                    .stroke(Stroke::new(
                        1.0,
                        if self.file_error.is_some() {
                            Color32::from_rgb(210, 120, 108)
                        } else {
                            LINE
                        },
                    ))
                    .corner_radius(10)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new("BIN  /  串口升级镜像")
                                .size(10.0)
                                .monospace()
                                .color(TEAL),
                        );
                        ui.add_space(8.0);
                        let filename = self
                            .firmware
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "拖入文件，或点击选择".into());
                        let name = ui.add(
                            egui::Label::new(RichText::new(filename).size(14.0).strong())
                                .truncate(),
                        );
                        if let Some(path) = &self.firmware {
                            name.on_hover_text(path.display().to_string());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(path.display().to_string())
                                        .size(10.0)
                                        .color(MUTED),
                                )
                                .wrap(),
                            );
                        }
                        ui.add_space(4.0);
                        label(
                            ui,
                            &self
                                .file_size
                                .map(|n| format!("{:.1} KiB  ·  已通过基础检查", n as f64 / 1024.0))
                                .unwrap_or_else(|| "请选择 *_update.bin".into()),
                        );
                        if let Some(hash) = &self.file_sha256 {
                            ui.add_space(6.0);
                            ui.label(RichText::new("SHA-256").size(10.0).monospace().color(TEAL));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{}\n{}", &hash[..32], &hash[32..]))
                                        .monospace()
                                        .size(10.0),
                                )
                                .wrap(),
                            );
                            if ui.small_button("复制SHA-256").clicked() {
                                ui.ctx().copy_text(hash.clone());
                            }
                        }
                        ui.add_space(10.0);
                        ui.button(if self.firmware.is_some() {
                            "更换固件…"
                        } else {
                            "选择文件…"
                        })
                        .clicked()
                    });
                if response.inner
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("升级固件", &["bin"])
                        .pick_file()
                {
                    self.choose_file(path);
                }
                if let Some(error) = &self.file_error {
                    ui.add_space(6.0);
                    ui.colored_label(Color32::from_rgb(185, 64, 52), error);
                }
                ui.add_space(14.0);
                section(ui, "03", "通信设置");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("baud-rate")
                        .width(135.0)
                        .selected_text(RichText::new(self.baud.to_string()).monospace())
                        .show_ui(ui, |ui| {
                            for baud in [921600, 460800, 230400, 115200] {
                                ui.selectable_value(&mut self.baud, baud, baud.to_string());
                            }
                        });
                    label(ui, "波特率");
                });
                ui.add_space(6.0);
                label(ui, "8N1 · 无流控 · 官方样本使用921600；需与启动程序一致");
                egui::CollapsingHeader::new("等待时间与启动触发").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        label(ui, "等待秒数");
                        ui.add(egui::DragValue::new(&mut self.wait_seconds).range(1..=600));
                    });
                    ui.checkbox(&mut self.trigger, "升级时重复发送启动触发32（字符2）");
                    label(
                        ui,
                        "触发信号可能改变正常启动流程；无启动语音不能证明握手成功。",
                    );
                });
                ui.add_space(14.0);
                egui::CollapsingHeader::new("启动诊断 · 不传固件").show(ui, |ui| {
                    ui.checkbox(&mut self.probe_trigger, "诊断时发送启动触发32（字符2）");
                    label(
                        ui,
                        "默认只接收；诊断使用当前升级波特率与等待时间，不需要选择固件。",
                    );
                    if ui
                        .add_enabled(
                            !debug_busy && !self.port.is_empty(),
                            egui::Button::new("开始启动诊断"),
                        )
                        .clicked()
                    {
                        self.begin(true);
                    }
                });
            });
            ui.add_space(18.0);
            let ready = !running
                && !debug_busy
                && !self.port.is_empty()
                && self.firmware.is_some()
                && self.file_error.is_none();
            if running {
                if ui
                    .add_enabled(
                        !self.stopping,
                        egui::Button::new(if self.stopping {
                            "正在停止…"
                        } else {
                            if self.diagnosing {
                                "停止诊断"
                            } else {
                                "停止升级"
                            }
                        })
                        .min_size(Vec2::new(ui.available_width(), 42.0)),
                    )
                    .clicked()
                {
                    if let Some(job) = &self.job {
                        let _ = job.cancel.send(());
                    }
                    self.stopping = true;
                    self.status = "正在停止…".into();
                }
            } else if ui
                .add_enabled(
                    ready,
                    egui::Button::new(RichText::new("开始升级  →").color(Color32::WHITE).strong())
                        .fill(TEAL)
                        .min_size(Vec2::new(ui.available_width(), 42.0)),
                )
                .clicked()
            {
                self.begin(false);
            }
            ui.add_space(8.0);
            label(
                ui,
                if debug_busy {
                    "串口调试正在连接，请先到调试页断开。"
                } else {
                    "先点击开始，再给模块上电。"
                },
            );
        });
    }

    fn activity(&mut self, ui: &mut egui::Ui) {
        let (title, subtitle, color) = self.phase_info();
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if self.diagnosing {
                        "启动诊断"
                    } else {
                        "升级进度"
                    })
                    .size(15.0)
                    .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let state = match self.phase {
                        Phase::Idle => "未开始",
                        Phase::Opening => "检查连接",
                        Phase::Waiting => "等待设备",
                        Phase::Transferring | Phase::Verifying => "升级中",
                        Phase::Success => "已完成",
                        Phase::DiagnosticDone => "诊断完成",
                        Phase::Stopped => "已停止",
                        Phase::Error => "需检查",
                    };
                    egui::Frame::new()
                        .fill(color.gamma_multiply(0.08))
                        .corner_radius(20)
                        .inner_margin(egui::Margin::symmetric(10, 4))
                        .show(ui, |ui| {
                            ui.label(RichText::new(state).size(11.0).color(color));
                        });
                });
            });
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if self.diagnosing {
                        "启动".into()
                    } else {
                        format!("{:.0}", self.progress * 100.0)
                    })
                    .size(48.0)
                    .strong()
                    .color(INK),
                );
                ui.label(
                    RichText::new(if self.diagnosing { "诊断" } else { "%" })
                        .size(22.0)
                        .color(MUTED),
                );
                if self.job.is_some() {
                    ui.add_space(10.0);
                    ui.add(egui::Spinner::new().color(TEAL).size(18.0));
                }
            });
            ui.add_space(4.0);
            if !self.diagnosing {
                ui.add(
                    egui::ProgressBar::new(self.progress)
                        .fill(color)
                        .desired_height(6.0)
                        .corner_radius(3),
                );
            }
            ui.add_space(18.0);
            ui.label(
                RichText::new(if self.stopping {
                    if self.diagnosing {
                        "正在停止诊断"
                    } else {
                        "正在停止升级"
                    }
                } else {
                    title
                })
                .size(17.0)
                .strong()
                .color(color),
            );
            ui.add_space(4.0);
            label(ui, subtitle);
            ui.add_space(18.0);
            ui.separator();
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let total = if self.diagnosing {
                    0
                } else if self.total > 0 {
                    self.total
                } else {
                    self.file_size.unwrap_or(0)
                };
                label(
                    ui,
                    &if self.diagnosing {
                        "本次不传输固件".into()
                    } else {
                        format!(
                            "已确认  {:.1} / {:.1} KiB",
                            self.acknowledged as f64 / 1024.0,
                            total as f64 / 1024.0
                        )
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let elapsed = if self.job.is_some() {
                        self.started.map(|t| t.elapsed())
                    } else {
                        self.duration
                    };
                    let seconds = elapsed.unwrap_or_default().as_secs();
                    label(
                        ui,
                        &format!("用时  {:02}:{:02}", seconds / 60, seconds % 60),
                    );
                });
            });
        });
        ui.add_space(16.0);
        let log_height = (ui.available_height() - 32.0).max(160.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(27, 41, 43))
            .corner_radius(16)
            .inner_margin(20)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("运行日志")
                            .size(14.0)
                            .strong()
                            .color(Color32::from_rgb(226, 235, 231)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.logs.is_empty(),
                                egui::Button::new("保存日志…").small(),
                            )
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .set_file_name("su03t-upgrade.log")
                                .add_filter("日志", &["log", "txt"])
                                .save_file()
                        {
                            let text = format!(
                                "{}\n{}\n",
                                su03t_update::capture::environment(),
                                self.logs.iter().cloned().collect::<Vec<_>>().join("\n")
                            );
                            if let Err(error) = std::fs::write(path, text) {
                                self.log(format!("保存日志失败：{error}"));
                            }
                        }
                        let copied = self
                            .copied
                            .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                        if ui
                            .add_enabled(
                                !self.logs.is_empty(),
                                egui::Button::new(
                                    RichText::new(if copied {
                                        "已复制"
                                    } else {
                                        "复制日志"
                                    })
                                    .size(11.0)
                                    .color(Color32::from_rgb(183, 203, 196)),
                                )
                                .fill(Color32::from_rgb(42, 58, 59))
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            ctx_copy(ui.ctx(), &self.logs);
                            self.copied = Some(Instant::now());
                            ui.ctx().request_repaint_after(Duration::from_secs(2));
                        }
                    });
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .id_salt("logs")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(log_height - 40.0)
                    .show(ui, |ui| {
                        ui.set_min_height(log_height - 40.0);
                        ui.set_min_width(ui.available_width());
                        if self.logs.is_empty() {
                            ui.add_space(18.0);
                            ui.label(
                                RichText::new("等待开始升级")
                                    .size(13.0)
                                    .color(Color32::from_rgb(157, 181, 172)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("连接信息与传输结果会显示在这里。")
                                    .size(12.0)
                                    .color(Color32::from_rgb(117, 145, 137)),
                            );
                        }
                        for line in &self.logs {
                            let error = line.contains("Error")
                                || line.contains("失败")
                                || line.contains("Caused by")
                                || line.contains("未完成");
                            ui.add(
                                egui::Label::new(RichText::new(line).monospace().size(11.0).color(
                                    if error {
                                        Color32::from_rgb(244, 162, 143)
                                    } else {
                                        Color32::from_rgb(185, 209, 197)
                                    },
                                ))
                                .wrap(),
                            );
                        }
                    });
            });
    }
}

fn ctx_copy(ctx: &egui::Context, logs: &VecDeque<String>) {
    ctx.copy_text(logs.iter().cloned().collect::<Vec<_>>().join("\n"));
}

impl eframe::App for Updater {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.debugger.poll();
        if self.job.is_some() || self.debugger.busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if self.mode == Mode::Upgrade
            && self.job.is_none()
            && let Some(path) =
                ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()))
        {
            self.choose_file(path);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PAPER).inner_margin(28))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    chip(ui);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("SU-03T").size(28.0).strong().color(INK));
                        ui.label(
                            RichText::new("离线语音模块 · 升级与串口调试")
                                .size(12.0)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} / {}  ·  {}", std::env::consts::OS, std::env::consts::ARCH, env!("CARGO_PKG_VERSION")))
                                .monospace()
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    let before = self.mode;
                    ui.add_enabled_ui(!self.debugger.busy() && self.job.is_none(), |ui| {
                        ui.selectable_value(&mut self.mode, Mode::Debug, "语音调试");
                        ui.selectable_value(&mut self.mode, Mode::BootLog, "B8 日志");
                        ui.selectable_value(&mut self.mode, Mode::Upgrade, "固件升级");
                    });
                    if self.mode != before {
                        match self.mode {
                            Mode::Debug => self.debugger.set_profile(Profile::Car),
                            Mode::BootLog => self.debugger.set_profile(Profile::B8),
                            Mode::Upgrade => {},
                        }
                    }
                    if self.debugger.busy() {
                        ui.label(RichText::new("调试串口正在使用").size(11.0).color(TEAL));
                    } else if self.job.is_some() {
                        ui.label(RichText::new(if self.diagnosing { "正在启动诊断" } else { "正在升级" }).size(11.0).color(TEAL));
                    }
                });
                ui.add_space(14.0);
                if matches!(self.mode, Mode::Debug | Mode::BootLog) {
                    self.debugger.ui(ui, self.job.is_some(), &mut self.finder);
                    return;
                }
                // Outer scrolling keeps all controls reachable at the minimum window size.
                egui::ScrollArea::vertical()
                    .id_salt("workspace")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width();
                        let left = 322.0;
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(left, 588.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    self.settings(ui);
                                },
                            );
                            ui.add_space(12.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new((width - left - 20.0).max(300.0), 588.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    self.activity(ui);
                                },
                            );
                        });
                        ui.add_space(16.0);
                        egui::Frame::new()
                            .fill(Color32::from_rgb(234, 240, 234))
                            .corner_radius(12)
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new("接线速查").size(12.0).strong().color(TEAL),
                                    );
                                    ui.add_space(12.0);
                                    ui.label(
                                        RichText::new("B7 → RXD     B6 → TXD     GND 共地")
                                            .monospace()
                                            .size(12.0)
                                            .color(INK),
                                    );
                                    ui.add_space(12.0);
                                    label(ui, "3.3V 串口电平  ·  模块独立 5V 供电");
                                });
                                ui.add_space(5.0);
                                label(
                                    ui,
                                    "实验性升级：需兼容的蜂鸟M引导程序。先开始烧录，再给模块重新上电；CCC握手后才传输固件，无读回校验。",
                                );
                            });
                    });
            });
        if self.finder.active()
            && let Some((target, port)) = self.finder.show(ctx)
        {
            match target {
                port_picker::Target::Upgrade => {
                    self.port = port.path;
                    self.refresh_ports();
                }
                port_picker::Target::Debug => self.debugger.select_port(port.path),
            }
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn configure_fonts(ctx: &egui::Context) {
    for candidate in [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(candidate) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "system-cjk".into(),
                egui::FontData::from_owned(bytes).into(),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("system-cjk".into());
            }
            ctx.set_fonts(fonts);
            break;
        }
    }
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1020.0, 860.0])
            .with_min_inner_size([850.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SU-03T 升级与调试",
        options,
        Box::new(|cc| {
            configure_fonts(&cc.egui_ctx);
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::light();
            style.visuals.override_text_color = Some(INK);
            style.visuals.panel_fill = PAPER;
            style.visuals.extreme_bg_color = Color32::from_rgb(246, 249, 246);
            style.visuals.selection.bg_fill = Color32::from_rgb(210, 233, 224);
            style.visuals.selection.stroke = Stroke::new(1.0, TEAL);
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(244, 247, 244);
            style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(244, 247, 244);
            style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(230, 241, 233);
            style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(230, 241, 233);
            style.visuals.widgets.hovered.bg_stroke =
                Stroke::new(1.0, Color32::from_rgb(135, 182, 162));
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(210, 233, 224);
            for widgets in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
            ] {
                widgets.corner_radius = egui::CornerRadius::same(7);
            }
            style.spacing.item_spacing = Vec2::new(8.0, 4.0);
            style.spacing.button_padding = Vec2::new(12.0, 7.0);
            style.spacing.interact_size.y = 30.0;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(12.0));
            cc.egui_ctx.set_style(style);
            Ok(Box::new(Updater::default()))
        }),
    )
}

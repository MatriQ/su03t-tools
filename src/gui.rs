//! Native UI. A separate CLI process owns the serial port; UI state follows its
//! typed events and cancellation can stop a blocked driver call.
use eframe::egui;
use egui::{Color32, RichText, Stroke, Vec2};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use su03t_update::{
    firmware::Image,
    flash_event::{EVENT_PREFIX, FlashEvent, RetryReason, Target},
    profiles::Profile,
    serial_debug::hex,
};
#[path = "debug_ui.rs"]
mod debug_ui;
#[path = "driver_ui.rs"]
mod driver_ui;
#[path = "ui_i18n.rs"]
mod i18n;
#[path = "port_picker.rs"]
mod port_picker;
use i18n::choose as tr;

const MAX_LOG_LINES: usize = 2000;
const INK: Color32 = Color32::from_rgb(28, 44, 46);
const MUTED: Color32 = Color32::from_rgb(101, 122, 115);
const TEAL: Color32 = Color32::from_rgb(17, 107, 95);
const AMBER: Color32 = Color32::from_rgb(153, 87, 22);
const RED: Color32 = Color32::from_rgb(178, 57, 44);
const LINE: Color32 = Color32::from_rgb(220, 229, 223);
const PAPER: Color32 = Color32::from_rgb(243, 246, 241);
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Upgrade,
    Debug,
    BootLog,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Idle,
    Opening,
    Waiting,
    Writing,
    AwaitingAck,
    Success,
    DiagnosticDone,
    Stopped,
    Error,
}
enum Event {
    Protocol(FlashEvent),
    Diagnostic(String),
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
struct JobSpec {
    port: String,
    firmware: Option<PathBuf>,
    sha256: Option<String>,
    baud: u32,
    wait: u64,
    trigger: bool,
    probe: bool,
}
fn read_lines(
    input: impl std::io::Read + Send + 'static,
    tx: Sender<Event>,
    structured: bool,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(input).lines() {
            let event = match line {
                Ok(line) if structured && line.starts_with(EVENT_PREFIX) => {
                    match serde_json::from_str(&line[EVENT_PREFIX.len()..]) {
                        Ok(e) => Event::Protocol(e),
                        Err(e) => Event::Diagnostic(format!("Invalid worker event: {e}")),
                    }
                }
                Ok(line) => Event::Diagnostic(line),
                Err(e) => Event::Diagnostic(format!("Worker log read failed: {e}")),
            };
            if tx.send(event).is_err() {
                break;
            }
        }
    })
}
fn start_job(spec: JobSpec) -> std::io::Result<Job> {
    let executable = std::env::current_exe()?;
    let (cancel, cancel_rx) = mpsc::channel();
    let (tx, events) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut command = Command::new(executable);
        command
            .arg(if spec.probe { "probe" } else { "flash" })
            .arg("--events")
            .arg("--port")
            .arg(spec.port)
            .arg("--baud")
            .arg(spec.baud.to_string())
            .arg("--wait")
            .arg(spec.wait.to_string());
        if spec.probe {
            if spec.trigger {
                command.arg("--trigger");
            }
        } else {
            command
                .arg("--firmware")
                .arg(spec.firmware.expect("flash image"))
                .arg("--expected-sha256")
                .arg(spec.sha256.expect("flash hash"));
            if !spec.trigger {
                command.arg("--passive");
            }
        }
        let mut child = match command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(error) => {
                let _ = tx.send(Event::Protocol(FlashEvent::Failed {
                    code: "worker".into(),
                    detail: error.to_string(),
                }));
                let _ = tx.send(Event::Finished {
                    success: false,
                    stopped: false,
                });
                return;
            }
        };
        let stdout = read_lines(child.stdout.take().expect("piped stdout"), tx.clone(), true);
        let stderr = read_lines(
            child.stderr.take().expect("piped stderr"),
            tx.clone(),
            false,
        );
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
                Ok(None) => thread::sleep(Duration::from_millis(30)),
                Err(error) => {
                    let _ = tx.send(Event::Diagnostic(error.to_string()));
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        let _ = stdout.join();
        let _ = stderr.join();
        let _ = tx.send(Event::Finished { success, stopped });
    });
    Ok(Job {
        cancel,
        events,
        worker: Some(worker),
    })
}
#[derive(Clone)]
struct Text {
    zh: String,
    en: String,
}
impl Text {
    fn new(zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self {
            zh: zh.into(),
            en: en.into(),
        }
    }
    fn get(&self) -> &str {
        tr(&self.zh, &self.en)
    }
}
#[derive(Clone)]
enum Message {
    Text(Text),
    Protocol(FlashEvent),
}
#[derive(Clone)]
struct LogRecord {
    at: Duration,
    message: Message,
}
struct Updater {
    ports: Vec<port_picker::PortDetails>,
    port: String,
    firmware: Option<PathBuf>,
    baud: u32,
    job: Option<Job>,
    stopping: bool,
    phase: Phase,
    logs: VecDeque<LogRecord>,
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
    handshake_at: Option<Instant>,
    stage_since: Instant,
    unconfirmed_since: Instant,
    target: Option<Target>,
    retry: usize,
    max_retries: usize,
    response_ms: u64,
    handshake_ms: u64,
    startup_rx: u64,
    completed: bool,
    failure: Option<(String, String)>,
    diagnostics: Vec<String>,
    write_warning: bool,
    follow: bool,
    log_offset: f32,
    evicted_pending: usize,
    removed: u64,
    jump: bool,
    unseen: usize,
    copied: Option<Instant>,
    cleared: Option<Instant>,
    split: f32,
    language: i18n::Language,
    system_english: bool,
    mode: Mode,
    debugger: debug_ui::Debugger,
    finder: port_picker::Finder,
    driver_check: driver_ui::Checker,
}
impl Default for Updater {
    fn default() -> Self {
        let language = i18n::load();
        let system_english = i18n::system_english();
        i18n::set(language, system_english);
        let mut app = Self {
            ports: vec![],
            port: String::new(),
            firmware: None,
            baud: 921600,
            job: None,
            stopping: false,
            phase: Phase::Idle,
            logs: VecDeque::new(),
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
            handshake_at: None,
            stage_since: Instant::now(),
            unconfirmed_since: Instant::now(),
            target: None,
            retry: 0,
            max_retries: 30,
            response_ms: 5000,
            handshake_ms: 60000,
            startup_rx: 0,
            completed: false,
            failure: None,
            diagnostics: vec![],
            write_warning: false,
            follow: true,
            log_offset: 0.0,
            evicted_pending: 0,
            removed: 0,
            jump: false,
            unseen: 0,
            copied: None,
            cleared: None,
            split: 0.53,
            language,
            system_english,
            mode: Mode::Upgrade,
            debugger: debug_ui::Debugger::default(),
            finder: port_picker::Finder::default(),
            driver_check: driver_ui::Checker::default(),
        };
        app.refresh_ports();
        #[cfg(all(debug_assertions, not(test)))]
        app.preview_from_environment();
        app
    }
}
impl Updater {
    fn push(&mut self, message: Message) {
        if self.logs.len() == MAX_LOG_LINES {
            self.logs.pop_front();
            self.removed += 1;
            if !self.follow {
                self.evicted_pending += 1;
            }
        }
        self.logs.push_back(LogRecord {
            at: self.started.map(|s| s.elapsed()).unwrap_or_default(),
            message,
        });
        if !self.follow {
            self.unseen = self.unseen.saturating_add(1);
        }
    }
    fn note(&mut self, zh: impl Into<String>, en: impl Into<String>) {
        self.push(Message::Text(Text::new(zh, en)))
    }
    fn clear_logs(&mut self) {
        self.logs.clear();
        self.log_offset = 0.0;
        self.evicted_pending = 0;
        self.removed = 0;
        self.unseen = 0;
        self.follow = true;
        self.jump = true;
        self.copied = None;
        self.cleared = Some(Instant::now());
    }
    fn refresh_ports(&mut self) {
        match port_picker::scan() {
            Ok(ports) => {
                if !ports.iter().any(|p| p.path == self.port) {
                    self.port = port_picker::single_usb(&ports).unwrap_or_default();
                }
                self.ports = ports
            }
            Err(e) => self.note(
                format!("枚举串口失败：{e}"),
                format!("Could not enumerate serial ports: {e}"),
            ),
        }
    }
    fn choose_file(&mut self, path: PathBuf) {
        self.file_error = None;
        self.file_size = None;
        self.file_sha256 = None;
        match Image::load(&path, false) {
            Ok(image) => {
                self.file_size = Some(image.bytes().len());
                self.file_sha256 = Some(image.sha256)
            }
            Err(e) => self.file_error = Some(format!("{e:#}")),
        }
        self.firmware = Some(path);
        self.phase = Phase::Idle;
        self.acknowledged = 0;
        self.total = 0;
        self.failure = None;
    }
    fn begin(&mut self, probe: bool) {
        #[cfg(debug_assertions)]
        if std::env::var_os("SU03T_UI_PREVIEW").is_some() {
            self.note(
                "模拟界面不连接物理串口。",
                "The UI preview cannot connect to a physical serial port.",
            );
            return;
        }

        if self.job.is_some()
            || self.debugger.busy()
            || self.port.is_empty()
            || (!probe && (self.file_error.is_some() || self.file_sha256.is_none()))
        {
            return;
        }
        self.stopping = false;
        self.diagnosing = probe;
        self.phase = Phase::Opening;
        self.started = Some(Instant::now());
        self.duration = None;
        self.handshake_at = None;
        self.stage_since = Instant::now();
        self.unconfirmed_since = Instant::now();
        self.target = None;
        self.retry = 0;
        self.acknowledged = 0;
        self.total = 0;
        self.startup_rx = 0;
        self.completed = false;
        self.failure = None;
        self.diagnostics.clear();
        self.write_warning = false;
        self.follow = true;
        self.jump = true;
        self.note(
            "—— 新会话：请保持模块断电，等待上电提示 ——",
            "—— New session: keep module power off until prompted ——",
        );
        let env = su03t_update::capture::environment();
        self.note(env.clone(), env);
        match start_job(JobSpec {
            port: self.port.clone(),
            firmware: self.firmware.clone(),
            sha256: self.file_sha256.clone(),
            baud: self.baud,
            wait: self.wait_seconds,
            trigger: if probe {
                self.probe_trigger
            } else {
                self.trigger
            },
            probe,
        }) {
            Ok(job) => self.job = Some(job),
            Err(e) => {
                self.phase = Phase::Error;
                self.failure = Some(("worker".into(), e.to_string()));
                self.duration = self.started.map(|s| s.elapsed());
                self.note(
                    format!("无法启动烧录进程：{e}"),
                    format!("Could not start transfer worker: {e}"),
                );
            }
        }
    }
    fn stop(&mut self) {
        if let Some(job) = &self.job {
            let _ = job.cancel.send(());
            self.stopping = true;
            self.note(
                "正在停止，请等待串口释放后再重新上电。",
                "Stopping. Wait for the serial port to be released before power-cycling.",
            );
        }
    }
    fn protocol(&mut self, event: FlashEvent) {
        let mut log = true;
        match &event {
            FlashEvent::Image { bytes, .. } => self.total = *bytes,
            FlashEvent::PortOpened { .. } => {}
            FlashEvent::HandshakeWaiting { timeout_ms } => {
                self.phase = Phase::Waiting;
                self.stage_since = Instant::now();
                self.handshake_ms = *timeout_ms;
            }
            FlashEvent::StartupRx { .. } => self.startup_rx += 1,
            FlashEvent::StartupSummary { count, .. } => self.startup_rx = *count,
            FlashEvent::HandshakeReceived => {
                self.handshake_at = Some(Instant::now());
                self.unconfirmed_since = Instant::now();
            }
            FlashEvent::Writing {
                target,
                retry,
                max_retries,
            } => {
                self.phase = Phase::Writing;
                self.target = Some(*target);
                self.retry = *retry;
                self.max_retries = *max_retries;
                self.stage_since = Instant::now();
                self.write_warning = false;
                log = matches!(target, Target::Eot | Target::Block { index: 1, .. }) || *retry > 0;
            }
            FlashEvent::WaitingAck {
                target,
                retry,
                timeout_ms,
            } => {
                self.phase = Phase::AwaitingAck;
                self.target = Some(*target);
                self.retry = *retry;
                self.response_ms = *timeout_ms;
                self.stage_since = Instant::now();
                log = matches!(target, Target::Eot | Target::Block { index: 1, .. }) || *retry > 0;
            }
            FlashEvent::Retry {
                target,
                retry,
                max_retries,
                ..
            } => {
                self.target = Some(*target);
                self.retry = *retry;
                self.max_retries = *max_retries;
            }
            FlashEvent::Acknowledged { .. } => {
                self.unconfirmed_since = Instant::now();
            }
            FlashEvent::Progress { done, total } => {
                self.acknowledged = *done;
                self.total = *total;
                log = false;
            }
            FlashEvent::Completed { .. } => self.completed = true,
            FlashEvent::Failed { code, detail } => {
                self.failure = Some((code.clone(), detail.clone()));
                self.phase = Phase::Error;
            }
            FlashEvent::Received { .. } => {}
        }
        if log {
            self.push(Message::Protocol(event));
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
        let events: Vec<_> = self
            .job
            .as_ref()
            .map(|job| job.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Protocol(event) => self.protocol(event),
                Event::Diagnostic(line) => {
                    if self.diagnostics.len() < 64 {
                        self.diagnostics.push(line)
                    }
                }
                Event::Finished { success, stopped } => {
                    self.duration = self.started.map(|s| s.elapsed());
                    self.phase = if stopped {
                        Phase::Stopped
                    } else if success && self.completed {
                        if self.diagnosing {
                            Phase::DiagnosticDone
                        } else {
                            Phase::Success
                        }
                    } else {
                        Phase::Error
                    };
                    if self.phase == Phase::Error && self.failure.is_none() {
                        self.failure = Some(("worker".into(), self.diagnostics.join("\n")));
                        self.note(
                            "烧录进程未报告有效完成状态。",
                            "The transfer worker did not report a valid completion state.",
                        );
                        for line in std::mem::take(&mut self.diagnostics) {
                            self.note(line.clone(), line)
                        }
                    }
                    self.note("串口已释放。", "Serial port released.");
                    self.job = None;
                    self.stopping = false;
                }
            }
        }
        if self.job.is_some()
            && self.phase == Phase::Writing
            && self.stage_since.elapsed() >= Duration::from_secs(5)
            && !self.write_warning
        {
            self.write_warning = true;
            self.note("串口写入已持续5秒，尚未进入设备确认等待；可停止后检查USB连接。","Serial write pending for 5 seconds. Device acknowledgment wait has not started; you can stop and check USB.");
        }
    }
    fn failure_title(&self) -> Text {
        let code = self
            .failure
            .as_ref()
            .map(|(code, _)| code.as_str())
            .unwrap_or("worker");
        match code {
            "ack_timeout" if self.target == Some(Target::Eot) => {
                Text::new("设备结束确认超时", "Final device acknowledgment timed out")
            }
            "ack_timeout" => Text::new(
                format!("{}确认超时", target_name(self.target)),
                format!("{} acknowledgment timed out", target_name_en(self.target)),
            ),
            "handshake_timeout" => Text::new("等待启动握手超时", "Startup handshake timed out"),
            "firmware_changed" => Text::new(
                "固件文件已变化，请重新选择",
                "Firmware changed — select the file again",
            ),
            "firmware" => Text::new("无法使用所选固件", "The selected firmware cannot be used"),
            "open_port" => Text::new("无法打开串口", "Could not open the serial port"),
            "device_skip" => Text::new(
                "设备返回跳过信号 NNN",
                "Device returned the NNN skip signal",
            ),
            "device_cancelled" => Text::new("设备取消了传输", "The device cancelled the transfer"),
            "transport" => Text::new("串口通信中断", "Serial communication failed"),
            _ => Text::new("本次升级未完成", "Upgrade incomplete"),
        }
    }
    fn status(&self) -> (Text, Text, Text, Color32) {
        if self.stopping {
            return (
                Text::new("正在停止升级…", "Stopping upgrade…"),
                Text::new(
                    "等待烧录进程退出并释放串口。",
                    "Waiting for the transfer worker to exit and release the port.",
                ),
                Text::new(
                    "请等待停止完成后，再给模块重新上电。",
                    "Wait until stopping completes before power-cycling the module.",
                ),
                MUTED,
            );
        }
        let keep = Text::new(
            "请保持模块供电和串口连接，传输期间不要断电或拔掉USB。",
            "Keep power and the serial connection stable. Do not disconnect power or USB during transfer.",
        );
        match self.phase {
            Phase::Idle => (
                Text::new("请先将模块断电", "Power off the module first"),
                Text::new(
                    "选择串口与升级固件，确认接线后，点击下方按钮开始连接。",
                    "Select the serial port and update firmware. Check wiring, then start the connection below.",
                ),
                Text::new(
                    "模块断电 → 开始连接 → 等待提示后上电；USB转串口保持连接电脑。",
                    "Power off → start connection → power on only when prompted. Keep the USB adapter connected.",
                ),
                AMBER,
            ),
            Phase::Opening => (
                Text::new(
                    "正在检查文件并打开串口",
                    "Checking firmware and opening the port",
                ),
                Text::new(
                    "串口就绪后，工具会明确提示上电。",
                    "The tool will prompt you to power on once the serial port is ready.",
                ),
                Text::new(
                    "请保持模块断电，USB转串口保持连接。",
                    "Keep module power off and the USB serial adapter connected.",
                ),
                TEAL,
            ),
            Phase::Waiting => (
                Text::new("现在给模块上电", "Power on the module now"),
                Text::new(
                    "正在等待启动握手。若模块已经上电，请将模块断电后再上电。",
                    "Waiting for the startup handshake. If the module is already on, power it off and then on again.",
                ),
                Text::new(
                    "只切换模块的5V电源，USB转串口保持连接电脑。",
                    "Switch only the module’s 5 V power. Keep the USB serial adapter connected to your computer.",
                ),
                AMBER,
            ),
            Phase::Writing => {
                let stalled = self.stage_since.elapsed() >= Duration::from_secs(5);
                (
                    if stalled {
                        Text::new("串口写入尚未完成", "Serial write has not completed")
                    } else {
                        Text::new(
                            format!("正在发送{}", target_name(self.target)),
                            format!("Sending {}", target_name_en(self.target)),
                        )
                    },
                    Text::new(
                        "数据正在提交串口驱动，尚未进入等待设备确认阶段。",
                        "Submitting data to the serial driver. The device acknowledgment wait has not started yet.",
                    ),
                    keep,
                    if stalled { AMBER } else { TEAL },
                )
            }
            Phase::AwaitingAck => {
                let delayed = self.retry > 0;
                let title = if self.target == Some(Target::Eot) {
                    Text::new(
                        "正在等待设备结束确认",
                        "Waiting for the final acknowledgment",
                    )
                } else if delayed {
                    Text::new(
                        format!("设备尚未确认{}", target_name(self.target)),
                        format!(
                            "Device has not acknowledged {}",
                            target_name_en(self.target)
                        ),
                    )
                } else {
                    Text::new(
                        format!("正在等待{}确认", target_name(self.target)),
                        format!("Waiting for {} acknowledgment", target_name_en(self.target)),
                    )
                };
                let desc = if self.retry >= 3
                    && self.unconfirmed_since.elapsed() >= Duration::from_secs(15)
                {
                    Text::new(
                        "已连续等待至少15秒，正在自动重试。可继续等待，或停止后检查接线和供电并重新开始。",
                        "No acknowledgment for at least 15 seconds. Automatic retries continue. You can wait, or stop to check wiring and power before restarting.",
                    )
                } else if self.target == Some(Target::Eot) {
                    Text::new(
                        "全部固件数据已确认，仍需设备确认传输结束。",
                        "All firmware data is acknowledged. Waiting for the device to confirm the end of transfer.",
                    )
                } else {
                    Text::new(
                        "进度以设备确认的数据为准。等待超时会自动重发，并显示重试次数。",
                        "Progress counts device acknowledgments. A response timeout triggers a visible retry.",
                    )
                };
                let notice = if delayed {
                    Text::new(
                        "继续等待时保持供电；如需重新上电，请先停止升级。",
                        "Keep power on while waiting. Stop the upgrade before power-cycling the module.",
                    )
                } else {
                    keep
                };
                (
                    title,
                    desc,
                    notice,
                    if delayed || self.target == Some(Target::Eot) {
                        AMBER
                    } else {
                        TEAL
                    },
                )
            }
            Phase::Success => (
                Text::new("请将模块断电，再上电", "Power the module off, then on"),
                Text::new(
                    "设备已确认传输结束。重新上电后，请验证欢迎语、唤醒词和语音指令。",
                    "The device confirmed the end of transfer. After restarting, test the greeting, wake word and voice commands.",
                ),
                Text::new(
                    "传输完成；尚未执行读回校验或语音功能验证。",
                    "Transfer complete. Readback verification and voice-function testing have not been performed.",
                ),
                TEAL,
            ),
            Phase::DiagnosticDone => (
                Text::new("已识别启动握手", "Startup handshake received"),
                Text::new(
                    "本次仅执行启动诊断，未发送固件，不能据此确认设备型号或兼容性。",
                    "This was a startup probe only. No firmware was sent; module model and compatibility remain unconfirmed.",
                ),
                Text::new(
                    "需要退出启动模式时，将模块断电再上电。",
                    "To leave startup mode, power the module off and on again.",
                ),
                TEAL,
            ),
            Phase::Stopped => (
                Text::new("重新开始前，请先断电", "Power off before starting again"),
                Text::new(
                    "本次操作已停止，完成状态未确认。串口已释放，日志已保留。",
                    "The operation stopped. Completion is unconfirmed. The serial port is released and logs are retained.",
                ),
                Text::new(
                    "模块断电 → 重新开始连接 → 等待上电提示后再上电。",
                    "Power off → start the connection again → power on only when prompted.",
                ),
                MUTED,
            ),
            Phase::Error => (
                self.failure_title(),
                Text::new(
                    "查看右侧日志了解故障阶段，再检查串口连接、模块接线和供电。",
                    "Check the log for the failure stage, then check the serial connection, module wiring and power.",
                ),
                Text::new(
                    "重试顺序：模块断电 → 重新开始 → 等待提示后上电。",
                    "To retry: power off → start again → power on only when prompted.",
                ),
                RED,
            ),
        }
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        let running = self.job.is_some();
        let blocked = self.debugger.busy();
        card().inner_margin(15).show(ui,|ui|{
            ui.set_min_width(ui.available_width());ui.horizontal(|ui|{ui.strong(tr("升级配置","Upgrade setup"));ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{label(ui,if running{tr("配置已锁定","Setup locked")}else{tr("准备升级","Prepare upgrade")});});});
            ui.add_space(8.0);ui.add_enabled_ui(!running,|ui|{
                ui.columns(2,|columns|{
                    label(&mut columns[0],tr("01 / 串口","01 / Serial port"));
                    columns[0].horizontal(|ui|{port_picker::select_ui(ui,"upgrade-port",&mut self.port,&self.ports);if ui.small_button("↻").on_hover_text(tr("刷新串口","Refresh ports")).clicked(){self.refresh_ports();}});
                    port_picker::details_ui(&mut columns[0],&self.port,&self.ports);
                    if columns[0].add_enabled(!blocked,egui::Button::new(tr("通过插拔识别…","Identify adapter…")).small()).clicked(){self.finder.start(port_picker::Target::Upgrade);}
                    label(&mut columns[1],tr("02 / 升级固件","02 / Firmware file"));
                    let name=self.firmware.as_ref().and_then(|p|p.file_name()).map(|s|s.to_string_lossy().into_owned()).unwrap_or_else(||tr("选择 *_update.bin","Select *_update.bin").into());
                    columns[1].add(egui::Label::new(RichText::new(name).monospace().size(11.0)).truncate()).on_hover_text(self.firmware.as_ref().map(|p|p.display().to_string()).unwrap_or_default());
                    if let Some(bytes)=self.file_size{label(&mut columns[1],&format!("{:.2} MiB",bytes as f64/(1024.0*1024.0)));}
                    if columns[1].small_button(if self.firmware.is_some(){tr("更换文件…","Change file…")}else{tr("选择文件…","Choose file…")}).clicked()
                        &&let Some(path)=rfd::FileDialog::new().set_title(tr("选择串口升级固件","Select update firmware")).add_filter(tr("升级固件","Firmware"),&["bin"]).pick_file(){self.choose_file(path);}
                });
                if let Some(error)=&self.file_error{ui.colored_label(RED,format!("{}: {}",tr("无法使用文件","Invalid file"),i18n::display(error)));}
                ui.add_space(8.0);ui.separator();
                label(ui,tr("模块 / 转接器：B7 → RXD · B6 ← TXD · GND ↔ GND","Module / Adapter: B7 → RXD · B6 ← TXD · GND ↔ GND"));
                label(ui,tr("串口电平3.3V · 模块独立5V供电","3.3 V UART · Separate 5 V module power"));
                egui::CollapsingHeader::new(tr("文件校验与高级设置","Checksum & advanced settings")).id_salt("upgrade-advanced").show(ui,|ui|{
                    if let Some(hash)=&self.file_sha256{ui.add(egui::Label::new(RichText::new(format!("SHA-256: {hash}")).monospace().size(10.0)).wrap());}
                    ui.horizontal(|ui|{label(ui,tr("波特率","Baud rate"));egui::ComboBox::from_id_salt("upgrade-baud").width(100.0).selected_text(self.baud.to_string()).show_ui(ui,|ui|{for baud in[921600,460800,230400,115200]{ui.selectable_value(&mut self.baud,baud,baud.to_string());}});});
                    label(ui,tr("8N1 · 无流控 · 需与启动程序波特率一致","8N1 · No flow control · Match the bootloader baud rate"));
                    ui.horizontal(|ui|{label(ui,tr("启动等待（秒）","Handshake wait (s)"));ui.add(egui::DragValue::new(&mut self.wait_seconds).range(1..=600));});
                    ui.checkbox(&mut self.trigger,tr("升级时发送启动触发32（字符2）","Send startup trigger 32 (ASCII 2) for upgrades"));
                    label(ui,tr("响应等待5秒，最多重试30次；触发信号可能改变正常启动。","Response timeout: 5 s; up to 30 retries. Startup triggers may affect normal boot."));
                    ui.separator();ui.strong(tr("启动诊断 · 不传固件","Startup probe · No firmware"));
                    ui.checkbox(&mut self.probe_trigger,tr("诊断时发送启动触发32","Send startup trigger 32 during probe"));
                    label(ui,tr("默认只接收，不需要选择固件。","Receive-only by default; no firmware file required."));
                    if ui.add_enabled(!blocked&&!self.port.is_empty(),egui::Button::new(tr("模块已断电，开始诊断","Module is off — start probe"))).clicked(){self.begin(true);}
                });
            });
        });
    }
    fn activity(&self, ui: &mut egui::Ui) {
        let stage = match self.phase {
            Phase::Idle | Phase::Opening | Phase::Stopped => 0,
            Phase::Waiting | Phase::DiagnosticDone => 1,
            Phase::Success => 4,
            _ => {
                if self.target == Some(Target::Eot) {
                    3
                } else {
                    2
                }
            }
        };
        ui.horizontal(|ui| {
            for (i, (zh, en)) in [
                ("准备", "Prepare"),
                ("上电握手", "Power on"),
                ("传输固件", "Transfer"),
                ("结束确认", "Finish"),
            ]
            .iter()
            .enumerate()
            {
                let c = if i <= stage { TEAL } else { MUTED };
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        if i < stage {
                            format!("{}.", i + 1)
                        } else {
                            (i + 1).to_string()
                        },
                        tr(zh, en)
                    ))
                    .size(11.0)
                    .color(c),
                );
                if i < 3 {
                    ui.add_space(4.0);
                }
            }
        });
        ui.add_space(8.0);
        let (title, description, notice, color) = self.status();
        card().inner_margin(18).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(if self.diagnosing {
                    tr("启动诊断", "Startup probe")
                } else {
                    tr("固件升级", "Firmware upgrade")
                })
                .size(11.0)
                .color(color),
            );
            ui.add_space(6.0);
            ui.label(RichText::new(title.get()).size(25.0).strong().color(INK));
            ui.add_space(6.0);
            ui.label(RichText::new(description.get()).size(12.0).color(MUTED));
            ui.add_space(12.0);
            egui::Frame::new()
                .fill(color.gamma_multiply(0.10))
                .corner_radius(7)
                .inner_margin(11)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new(notice.get()).size(12.0).color(color));
                });
            if self.handshake_at.is_some() && !self.diagnosing && self.total > 0 {
                ui.add_space(12.0);
                let progress = self.acknowledged as f32 / self.total as f32;
                ui.horizontal(|ui| {
                    label(
                        ui,
                        if self.acknowledged == self.total && self.phase != Phase::Success {
                            tr(
                                "数据已确认，等待结束确认",
                                "Data acknowledged; completion pending",
                            )
                        } else {
                            tr("设备已确认的数据", "Firmware data acknowledged")
                        },
                    );
                    ui.label(
                        RichText::new(format!("{:.0}%", progress * 100.0))
                            .strong()
                            .color(color),
                    );
                });
                ui.add(
                    egui::ProgressBar::new(progress)
                        .desired_height(6.0)
                        .fill(color),
                );
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            match self.phase {
                Phase::Waiting => {
                    let elapsed = self.stage_since.elapsed().as_secs_f64();
                    metric(
                        ui,
                        tr("握手等待", "Handshake wait"),
                        &format!("{elapsed:.0} s"),
                    );
                    metric(
                        ui,
                        tr("剩余时间", "Time remaining"),
                        &format!(
                            "{:.0} s",
                            (self.handshake_ms as f64 / 1000.0 - elapsed).max(0.0)
                        ),
                    );
                    metric(
                        ui,
                        tr("启动RX采样", "Startup RX sample"),
                        &format!("{} B", self.startup_rx),
                    );
                }
                Phase::Writing => {
                    metric(
                        ui,
                        tr("当前操作", "Current operation"),
                        &target_name_local(self.target),
                    );
                    metric(
                        ui,
                        tr("写入持续", "Write duration"),
                        &format!("{:.1} s", self.stage_since.elapsed().as_secs_f64()),
                    );
                }
                Phase::AwaitingAck => {
                    metric(
                        ui,
                        tr("未确认时长", "Without acknowledgment"),
                        &format!("{:.1} s", self.unconfirmed_since.elapsed().as_secs_f64()),
                    );
                    metric(
                        ui,
                        tr("本包重试", "Block retries"),
                        &format!("{} / {}", self.retry, self.max_retries),
                    );
                    metric(
                        ui,
                        tr("本轮剩余", "This attempt left"),
                        &format!(
                            "{:.1} s",
                            (self.response_ms as f64 / 1000.0
                                - self.stage_since.elapsed().as_secs_f64())
                            .max(0.0)
                        ),
                    );
                }
                _ => {}
            }
            if self.total > 0 {
                metric(
                    ui,
                    tr("已确认数据", "Acknowledged"),
                    &format!("{} / {} B", self.acknowledged, self.total),
                );
            }
            let elapsed = self
                .duration
                .or_else(|| self.started.map(|s| s.elapsed()))
                .unwrap_or_default();
            metric(
                ui,
                tr(
                    "总用时（含上电等待）",
                    "Total time (including power-on wait)",
                ),
                &format!("{:.1} s", elapsed.as_secs_f64()),
            );
            if self.phase == Phase::Error && self.acknowledged == 0 {
                label(
                    ui,
                    tr(
                        "尚无固件数据包得到确认。",
                        "No firmware data block was acknowledged.",
                    ),
                );
            }
        });
    }
    fn left(&mut self, ui: &mut egui::Ui) {
        let height = (ui.available_height() - 64.0).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("upgrade-controls")
            .auto_shrink([false, false])
            .max_height(height)
            .show(ui, |ui| {
                self.settings(ui);
                ui.add_space(12.0);
                self.activity(ui);
            });
        ui.add_space(9.0);
        let ready = self.job.is_none()
            && !self.debugger.busy()
            && !self.port.is_empty()
            && self.file_sha256.is_some()
            && self.file_error.is_none();
        let running = self.job.is_some();
        let text = if self.stopping {
            tr("正在停止…", "Stopping…")
        } else if running {
            if self.diagnosing {
                tr("停止诊断", "Stop probe")
            } else if self.retry >= 3 || self.write_warning {
                tr("停止并重新准备", "Stop and prepare again")
            } else {
                tr("停止升级", "Stop upgrade")
            }
        } else {
            tr("模块已断电，开始连接", "Module is off — start connection")
        };
        if ui
            .add_enabled(
                if running { !self.stopping } else { ready },
                egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
                    .fill(if running && self.retry > 0 {
                        AMBER
                    } else {
                        TEAL
                    })
                    .min_size(Vec2::new(ui.available_width(), 40.0)),
            )
            .clicked()
        {
            if running {
                self.stop()
            } else {
                self.begin(false)
            }
        }
        if self.debugger.busy() {
            label(
                ui,
                tr(
                    "串口调试正在连接，请先断开。",
                    "A debug session is active. Disconnect it first.",
                ),
            );
        }
    }
    fn export(&self) -> String {
        let mut out = format!(
            "{}\nlanguage={}\n",
            su03t_update::capture::environment(),
            if i18n::english() { "en" } else { "zh-CN" }
        );
        for record in &self.logs {
            out.push_str(&format!(
                "[{:8.3}s] {}\n",
                record.at.as_secs_f64(),
                message_text(&record.message)
            ));
        }
        out
    }
    fn logs(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(Color32::from_rgb(33,54,50)).corner_radius(10).inner_margin(12).show(ui,|ui|{
            ui.set_min_width(ui.available_width());*ui.visuals_mut() = egui::Visuals::dark(); ui.visuals_mut().override_text_color=Some(Color32::from_rgb(218,233,223));
            ui.horizontal_wrapped(|ui|{ui.strong(tr("运行日志","Activity log"));ui.label(RichText::new(if self.follow{tr("跟随最新","Following latest").into()}else{format!("{} · +{}",tr("已暂停跟随","Follow paused"),self.unseen)}).size(10.0).color(Color32::from_rgb(181,208,191)));});
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!self.follow,egui::Button::new(tr("回到底部并跟随","Jump to latest & follow")).small()).clicked(){self.follow=true;self.jump=true;self.unseen=0;}
                if ui.add_enabled(!self.logs.is_empty(),egui::Button::new(if self.copied.is_some_and(|t|t.elapsed()<Duration::from_secs(2)){tr("已复制","Copied")}else{tr("复制日志","Copy log")}).small()).clicked(){ui.ctx().copy_text(self.export());self.copied=Some(Instant::now());}
                if ui.add_enabled(!self.logs.is_empty(),egui::Button::new(tr("保存日志","Save log")).small()).clicked()&&let Some(path)=rfd::FileDialog::new().set_title(tr("保存升级日志","Save upgrade log")).set_file_name("su03t-upgrade.log").add_filter(tr("日志","Log"),&["log","txt"]).save_file()&&let Err(e)=std::fs::write(path,self.export()){self.note(format!("保存日志失败：{e}"),format!("Could not save log: {e}"));}
                if ui.add_enabled(!self.logs.is_empty(),egui::Button::new(tr("清除日志","Clear log")).small()).on_hover_text(tr("仅清空显示，不影响进度、计数或烧录。","Clear the display only; progress, counts and transfer are retained.")).clicked(){self.clear_logs();}
            });
            ui.add_space(5.0);ui.separator();ui.add_space(5.0);
            let height=(ui.available_height()-22.0).max(40.0);let mut scroll=egui::ScrollArea::both().id_salt("upgrade-log-view").auto_shrink([false,false]).max_height(height).stick_to_bottom(true);
            if self.jump{scroll=scroll.vertical_scroll_offset(f32::MAX);self.jump=false;}
            else if self.evicted_pending>0 && !self.follow {scroll=scroll.vertical_scroll_offset((self.log_offset-self.evicted_pending as f32*16.0).max(0.0));}
            self.evicted_pending=0;
            let out=ui.scope(|ui|{
                ui.spacing_mut().item_spacing.y=0.0;
                if self.logs.is_empty(){scroll.show(ui,|ui|{ui.set_min_height(height);ui.label(RichText::new(if self.cleared.is_some(){tr("日志已清除；新事件将继续显示。","Log cleared. New events will continue here.")}else{tr("等待开始。连接、握手与传输事件将显示在这里。","Waiting to start. Connection, handshake and transfer events appear here.")}).size(11.0).color(Color32::from_rgb(181,208,191)));})}
                else{scroll.show_rows(ui,16.0,self.logs.len(),|ui,range|{for index in range{let record=&self.logs[index];let text=message_text(&record.message);let error=matches!(&record.message,Message::Protocol(FlashEvent::Failed{..}|FlashEvent::Retry{..}));let color=if error{Color32::from_rgb(245,192,140)}else{Color32::from_rgb(194,213,200)};ui.add(egui::Label::new(RichText::new(format!("[{:8.3}s] {}",record.at.as_secs_f64(),text.replace('\n'," | "))).monospace().size(11.0).line_height(Some(16.0)).color(color)).extend().selectable(true));}})}
            }).inner;
            self.log_offset=out.state.offset.y;
            self.follow=out.state.offset.y+out.inner_rect.height()>=out.content_size.y-0.5;if self.follow{self.unseen=0;}
            ui.add_space(5.0);
            let summary=format!("{} {} · {} {} · {}",self.logs.len(),tr("行","lines"),self.removed,tr("条较早记录已移除","older records dropped"),tr("原始数据保留","Raw data retained"));
            ui.add(egui::Label::new(RichText::new(&summary).size(10.0).color(Color32::from_rgb(168,191,178))).truncate()).on_hover_text(summary);
        });
    }
}
fn metric(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(name).size(11.0).color(MUTED));
        ui.label(
            RichText::new(value)
                .monospace()
                .size(11.0)
                .strong()
                .color(INK),
        );
    });
}
fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(16)
}
fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(MUTED));
}
fn section(ui: &mut egui::Ui, index: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(index).monospace().size(11.0).color(TEAL));
        ui.label(RichText::new(title).size(14.0).strong().color(INK));
    });
    ui.add_space(7.0);
}
fn target_name(target: Option<Target>) -> String {
    match target {
        Some(Target::Block { index, .. }) => format!("第{index}包"),
        Some(Target::Eot) => "结束标记".into(),
        None => "数据包".into(),
    }
}
fn target_name_en(target: Option<Target>) -> String {
    match target {
        Some(Target::Block { index, .. }) => format!("block {index}"),
        Some(Target::Eot) => "end-of-transfer marker".into(),
        None => "block".into(),
    }
}
fn target_name_local(target: Option<Target>) -> String {
    if i18n::english() {
        target_name_en(target)
    } else {
        target_name(target)
    }
}
fn message_text(message: &Message) -> String {
    match message {
        Message::Text(t) => t.get().into(),
        Message::Protocol(e) => event_text(e).get().into(),
    }
}
fn event_text(event: &FlashEvent) -> Text {
    match event {
        FlashEvent::Image {
            path,
            bytes,
            sha256,
        } => Text::new(
            format!("固件 {path}：{bytes} B · SHA-256 {sha256}"),
            format!("Firmware {path}: {bytes} B · SHA-256 {sha256}"),
        ),
        FlashEvent::PortOpened { port, baud } => Text::new(
            format!("串口已打开：{port} · {baud} / 8N1"),
            format!("Serial port open: {port} · {baud} / 8N1"),
        ),
        FlashEvent::HandshakeWaiting { timeout_ms } => Text::new(
            format!(
                "现在给模块上电，等待CCC启动握手（{}秒）。",
                timeout_ms / 1000
            ),
            format!(
                "Power on the module now; waiting for CCC ({} s).",
                timeout_ms / 1000
            ),
        ),
        FlashEvent::StartupRx { byte, elapsed_ms } => Text::new(
            format!("启动RX +{elapsed_ms}ms：{byte:02X}"),
            format!("Startup RX +{elapsed_ms}ms: {byte:02X}"),
        ),
        FlashEvent::StartupSummary { count, sample } => Text::new(
            format!("握手阶段接收 {count} B · HEX（最多256B）：{}", hex(sample)),
            format!(
                "Handshake RX: {count} B · HEX (up to 256 B): {}",
                hex(sample)
            ),
        ),
        FlashEvent::HandshakeReceived => Text::new(
            "已收到CCC启动握手，开始传输；请保持供电。",
            "CCC startup handshake received; beginning transfer. Keep power on.",
        ),
        FlashEvent::Writing {
            target,
            retry,
            max_retries,
        } => Text::new(
            format!(
                "提交{}到串口 · 重试 {retry}/{max_retries}",
                target_name(Some(*target))
            ),
            format!(
                "Writing {} to serial port · retry {retry}/{max_retries}",
                target_name_en(Some(*target))
            ),
        ),
        FlashEvent::WaitingAck {
            target,
            retry,
            timeout_ms,
        } => Text::new(
            format!(
                "{}已提交，等待设备确认 · 重试 {retry} · 超时{timeout_ms}ms",
                target_name(Some(*target))
            ),
            format!(
                "{} submitted; waiting for acknowledgment · retry {retry} · timeout {timeout_ms}ms",
                target_name_en(Some(*target))
            ),
        ),
        FlashEvent::Received { target, byte } => Text::new(
            format!(
                "{}等待期间RX：{byte:02X}（非ACK/NAK）",
                target_name(Some(*target))
            ),
            format!(
                "RX while awaiting {}: {byte:02X} (not ACK/NAK)",
                target_name_en(Some(*target))
            ),
        ),
        FlashEvent::Retry {
            target,
            retry,
            max_retries,
            reason,
        } => Text::new(
            format!(
                "{}{}，开始第{retry}/{max_retries}次重试。",
                target_name(Some(*target)),
                if *reason == RetryReason::Timeout {
                    "等待超时"
                } else {
                    "收到NAK"
                }
            ),
            format!(
                "{} {}; starting retry {retry}/{max_retries}.",
                target_name_en(Some(*target)),
                if *reason == RetryReason::Timeout {
                    "timed out"
                } else {
                    "received NAK"
                }
            ),
        ),
        FlashEvent::Acknowledged { target } => Text::new(
            format!("{}收到设备ACK确认。", target_name(Some(*target))),
            format!(
                "{} acknowledged by the device (ACK).",
                target_name_en(Some(*target))
            ),
        ),
        FlashEvent::Progress { done, total } => Text::new(
            format!("已确认 {done}/{total} B"),
            format!("Acknowledged {done}/{total} B"),
        ),
        FlashEvent::Completed { probe: true } => Text::new(
            "诊断完成：已识别握手，未传固件。",
            "Probe complete: handshake received; no firmware sent.",
        ),
        FlashEvent::Completed { probe: false } => Text::new(
            "设备已确认传输结束；请将模块断电再上电验证语音。尚未读回校验。",
            "Device confirmed the end of transfer. Power-cycle the module and test voice functions. No readback verification performed.",
        ),
        FlashEvent::Failed { code, detail } => Text::new(
            format!("操作失败 [{code}] · 原始详情：{detail}"),
            format!("Operation failed [{code}] · Original technical detail: {detail}"),
        ),
    }
}

/// Reserve two independent panes. The native ScrollArea owns its scroll state;
/// a language change never resets its stable ID or fixed-height row positions.
fn split_rects(ui: &mut egui::Ui, ratio: &mut f32, id: &str) -> (egui::Rect, egui::Rect) {
    let rect = ui.available_rect_before_wrap();
    let width = (rect.width() - 14.0).max(1.0);
    *ratio = ratio.clamp(0.40, 0.65);
    let divider = egui::Rect::from_min_size(
        egui::pos2(rect.left() + width * *ratio + 3.0, rect.top()),
        Vec2::new(8.0, rect.height()),
    );
    let response = ui
        .interact(
            divider,
            ui.make_persistent_id(id),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        *ratio = ((pos.x - rect.left()) / width).clamp(0.40, 0.65);
        ui.ctx().request_repaint();
    }
    if response.has_focus() {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
            *ratio = (*ratio - 0.02).max(0.40)
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
            *ratio = (*ratio + 0.02).min(0.65)
        }
    }
    ui.painter().line_segment(
        [
            divider.center_top() + Vec2::new(0.0, rect.height() / 2.0 - 20.0),
            divider.center_bottom() - Vec2::new(0.0, rect.height() / 2.0 - 20.0),
        ],
        Stroke::new(
            2.0,
            if response.hovered() || response.dragged() {
                TEAL
            } else {
                LINE
            },
        ),
    );
    let left = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.left() + width * *ratio, rect.bottom()),
    );
    let right = egui::Rect::from_min_max(egui::pos2(left.right() + 14.0, rect.top()), rect.max);
    (left, right)
}
impl eframe::App for Updater {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        i18n::set(self.language, self.system_english);
        self.poll();
        self.debugger.poll();
        if cfg!(target_os = "macos") {
            self.driver_check
                .observe_ports(self.ports.iter().map(|p| p.path.clone()));
            self.driver_check
                .poll(ctx, self.job.is_some() || self.debugger.busy());
            if let Some((zh, en)) = self.driver_check.take_summary() {
                self.note(zh, en);
            }
        }
        if self.mode == Mode::Upgrade
            && self.job.is_none()
            && let Some(path) =
                ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()))
        {
            self.choose_file(path);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(PAPER).inner_margin(20)).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{
                egui::Frame::new().fill(TEAL).corner_radius(8).inner_margin(9).show(ui,|ui|{ui.label(RichText::new("S").strong().size(16.0).color(Color32::WHITE));});
                ui.vertical(|ui|{ui.label(RichText::new("SU-03T").size(22.0).strong().color(INK));label(ui,tr("离线语音模块 · 升级与调试","Firmware & serial diagnostics"));});
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    let before=self.language;
                    ui.selectable_value(&mut self.language,i18n::Language::En,"English");ui.selectable_value(&mut self.language,i18n::Language::Zh,"中文");
                    ui.selectable_value(&mut self.language,i18n::Language::System,tr("系统","System")).on_hover_text(tr("跟随系统语言","Use system language"));
                    if before!=self.language{i18n::set(self.language,self.system_english);if let Err(e)=i18n::save(self.language){self.note(format!("保存语言设置失败：{e}"),format!("Could not save language preference: {e}"));}ctx.request_repaint();}
                    label(ui,env!("CARGO_PKG_VERSION"));
                });
            });
            ui.add_space(10.0);ui.horizontal_wrapped(|ui|{
                let before=self.mode;ui.add_enabled_ui(self.job.is_none()&&!self.debugger.busy(),|ui|{ui.selectable_value(&mut self.mode,Mode::Debug,tr("语音调试","Voice debug"));ui.selectable_value(&mut self.mode,Mode::BootLog,tr("B8 日志","B8 logs"));ui.selectable_value(&mut self.mode,Mode::Upgrade,tr("固件升级","Firmware"));});
                if before!=self.mode{match self.mode{Mode::Debug=>self.debugger.set_profile(Profile::Car),Mode::BootLog=>self.debugger.set_profile(Profile::B8),Mode::Upgrade=>{}}}
                if self.job.is_some(){label(ui,tr("串口正在使用","Serial port in use"));}
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|self.driver_check.entry(ui));
            });ui.add_space(12.0);
            if self.mode!=Mode::Upgrade{
                let blocked=self.job.is_some();
                #[cfg(debug_assertions)] let blocked=blocked||std::env::var_os("SU03T_UI_PREVIEW").is_some();
                self.debugger.ui(ui,blocked,&mut self.finder);return
            }
            let footer=26.0;let available=ui.available_rect_before_wrap();let workspace=egui::Rect::from_min_max(available.min,egui::pos2(available.right(),available.bottom()-footer));
            ui.scope_builder(egui::UiBuilder::new().max_rect(workspace),|ui|{let(left,right)=split_rects(ui,&mut self.split,"upgrade-splitter");ui.scope_builder(egui::UiBuilder::new().max_rect(left),|ui|{ui.set_clip_rect(left.intersect(ui.clip_rect()));self.left(ui);});ui.scope_builder(egui::UiBuilder::new().max_rect(right),|ui|{ui.set_clip_rect(right.intersect(ui.clip_rect()));self.logs(ui);});});
            ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(egui::pos2(available.left(),available.bottom()-footer+7.0),available.max)),|ui|{label(ui,tr("实验性升级 · 工具无法检测模块供电 · 进度按设备确认更新 · 未执行读回校验","Experimental upgrade · Module power cannot be detected · Progress counts acknowledgments · No readback verification"));});
        });
        if self.finder.active()
            && let Some((target, port)) = self.finder.show(ctx)
        {
            match target {
                port_picker::Target::Upgrade => {
                    self.port = port.path;
                    self.refresh_ports()
                }
                port_picker::Target::Debug => self.debugger.select_port(port.path),
            }
        }
        self.driver_check.show(ctx);
        #[cfg(all(debug_assertions, not(test)))]
        self.capture_preview(ctx);
        ctx.request_repaint_after(if self.job.is_some() || self.debugger.busy() {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        });
    }
}
fn configure_fonts(ctx: &egui::Context) {
    for path in [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
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
    eframe::run_native(
        if cfg!(debug_assertions) && std::env::var_os("SU03T_UI_PREVIEW").is_some() {
            "SU-03T Tools · SIMULATED UI PREVIEW"
        } else {
            "SU-03T Tools"
        },
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1120.0, 840.0])
                .with_min_inner_size([850.0, 700.0])
                .with_active(
                    !(cfg!(debug_assertions) && std::env::var_os("SU03T_UI_CAPTURE").is_some()),
                ),
            ..Default::default()
        },
        Box::new(|cc| {
            configure_fonts(&cc.egui_ctx);
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::light();
            style.wrap_mode = Some(egui::TextWrapMode::Wrap);
            style.visuals.override_text_color = Some(INK);
            style.visuals.panel_fill = PAPER;
            style.visuals.selection.bg_fill = Color32::from_rgb(210, 233, 224);
            style.visuals.hyperlink_color = TEAL;
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(230, 241, 233);
            style.spacing.item_spacing = Vec2::new(7.0, 4.0);
            style.spacing.button_padding = Vec2::new(9.0, 6.0);
            style.spacing.interact_size.y = 28.0;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(11.0));
            cc.egui_ctx.set_style(style);
            let mut app = Updater::default();
            let preview = cfg!(debug_assertions) && std::env::var_os("SU03T_UI_PREVIEW").is_some();
            if cfg!(target_os = "macos") && !preview {
                app.driver_check.start(&cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(all(debug_assertions, not(test)))]
impl Updater {
    /// Capture our own rendered framebuffer, without reading another app or the desktop.
    fn capture_preview(&mut self, ctx: &egui::Context) {
        use std::io::Write;
        if std::env::var_os("SU03T_UI_PREVIEW").is_none() {
            return;
        }
        let Some(path) = std::env::var_os("SU03T_UI_CAPTURE") else {
            return;
        };
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot {
            let result = (|| -> std::io::Result<()> {
                let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
                write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
                for pixel in &image.pixels {
                    file.write_all(&[pixel.r(), pixel.g(), pixel.b()])?;
                }
                file.flush()
            })();
            if let Err(error) = result {
                eprintln!("UI capture failed: {error}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let frame = ctx.data_mut(|data| {
            let id = egui::Id::new("ui-capture-frame");
            let frame = data.get_temp::<u32>(id).unwrap_or(0) + 1;
            data.insert_temp(id, frame);
            frame
        });
        let now = ctx.input(|input| input.time);
        let ready = ctx.data_mut(|data| {
            let started_id = egui::Id::new("ui-capture-started");
            let requested_id = egui::Id::new("ui-capture-requested");
            let started = data.get_temp::<f64>(started_id).unwrap_or(now);
            data.insert_temp(started_id, started);
            let requested = data.get_temp::<bool>(requested_id).unwrap_or(false);
            if frame >= 6 && now - started >= 0.4 && !requested {
                data.insert_temp(requested_id, true);
                true
            } else {
                false
            }
        });
        if ready {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
    }

    /// Debug builds only: deterministic visual QA without touching a serial port.
    fn preview_from_environment(&mut self) {
        let Ok(scene) = std::env::var("SU03T_UI_PREVIEW") else {
            return;
        };
        if let Ok(language) = std::env::var("SU03T_UI_LANGUAGE") {
            self.language = match language.as_str() {
                "en" => i18n::Language::En,
                "zh" => i18n::Language::Zh,
                _ => self.language,
            };
            i18n::set(self.language, self.system_english);
        }
        if scene.starts_with("drivers-") {
            self.driver_check.preview(scene == "drivers-ready");
            self.note(
                "界面预览：驱动状态为模拟，未打开串口。",
                "UI PREVIEW: simulated driver status; no serial port was opened.",
            );
            return;
        }
        self.started = Some(Instant::now());
        self.total = 1_259_048;
        self.port = "/dev/SIMULATED".into();
        self.firmware = Some("SIMULATED_update.bin".into());
        self.file_size = Some(self.total);
        self.file_sha256 = Some("0".repeat(64));
        self.note(
            "界面预览：全部数据为模拟，未打开串口。",
            "UI PREVIEW: all data is simulated; no serial port was opened.",
        );
        self.protocol(FlashEvent::HandshakeWaiting { timeout_ms: 60000 });
        if scene != "power" {
            self.protocol(FlashEvent::HandshakeReceived);
            self.protocol(FlashEvent::WaitingAck {
                target: Target::Block {
                    index: 1,
                    total: 1230,
                },
                retry: 0,
                timeout_ms: 5000,
            });
        }
        if matches!(
            scene.as_str(),
            "power" | "retry" | "write" | "finish" | "transfer"
        ) {
            let (cancel, cancel_rx) = mpsc::channel();
            let (tx, events) = mpsc::channel();
            let worker = thread::spawn(move || {
                if cancel_rx.recv().is_ok() {
                    let _ = tx.send(Event::Finished {
                        success: false,
                        stopped: true,
                    });
                }
            });
            self.job = Some(Job {
                cancel,
                events,
                worker: Some(worker),
            });
        }
        match scene.as_str() {
            "retry" => {
                self.unconfirmed_since = Instant::now() - Duration::from_secs(16);
                self.protocol(FlashEvent::Retry {
                    target: Target::Block {
                        index: 1,
                        total: 1230,
                    },
                    retry: 3,
                    max_retries: 30,
                    reason: RetryReason::Timeout,
                });
            }
            "write" => {
                self.protocol(FlashEvent::Writing {
                    target: Target::Block {
                        index: 1,
                        total: 1230,
                    },
                    retry: 0,
                    max_retries: 30,
                });
                self.stage_since = Instant::now() - Duration::from_secs(8);
            }
            "finish" => {
                self.acknowledged = self.total;
                self.protocol(FlashEvent::WaitingAck {
                    target: Target::Eot,
                    retry: 0,
                    timeout_ms: 5000,
                });
            }
            "success" => {
                self.acknowledged = self.total;
                self.completed = true;
                self.phase = Phase::Success;
            }
            "error" => {
                self.protocol(FlashEvent::Failed {
                    code: "ack_timeout".into(),
                    detail: "block 1: retry limit reached without ACK".into(),
                });
            }
            "transfer" => {
                self.acknowledged = 369 * 1024;
                self.protocol(FlashEvent::WaitingAck {
                    target: Target::Block {
                        index: 370,
                        total: 1230,
                    },
                    retry: 0,
                    timeout_ms: 5000,
                });
                for index in 320..=369 {
                    self.push(Message::Protocol(FlashEvent::Acknowledged {
                        target: Target::Block { index, total: 1230 },
                    }));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(app: &mut Updater, success: bool, stopped: bool) {
        let (cancel, _) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        tx.send(Event::Finished { success, stopped }).unwrap();
        app.job = Some(Job {
            cancel,
            events,
            worker: None,
        });
        app.poll();
    }
    #[test]
    fn clear_during_retry_retains_progress_failure_context_and_new_events() {
        let mut app = Updater::default();
        let target = Target::Block {
            index: 2,
            total: 10,
        };
        app.protocol(FlashEvent::Progress {
            done: 1024,
            total: 10240,
        });
        app.protocol(FlashEvent::WaitingAck {
            target,
            retry: 3,
            timeout_ms: 5000,
        });
        app.protocol(FlashEvent::Failed {
            code: "ack_timeout".into(),
            detail: "block 2".into(),
        });
        app.clear_logs();
        assert!(app.logs.is_empty());
        assert_eq!(app.acknowledged, 1024);
        assert_eq!(app.total, 10240);
        assert_eq!(app.retry, 3);
        assert_eq!(app.target, Some(target));
        assert_eq!(app.phase, Phase::Error);
        assert!(app.failure.is_some());
        app.protocol(FlashEvent::Acknowledged { target });
        assert_eq!(app.logs.len(), 1);
    }
    #[test]
    fn worker_exit_and_data_100_percent_are_insufficient_without_completion_ack() {
        let mut app = Updater::default();
        app.protocol(FlashEvent::Progress {
            done: 1024,
            total: 1024,
        });
        app.protocol(FlashEvent::WaitingAck {
            target: Target::Eot,
            retry: 0,
            timeout_ms: 5000,
        });
        finish(&mut app, true, false);
        assert_eq!(app.phase, Phase::Error);
        app.protocol(FlashEvent::Completed { probe: false });
        finish(&mut app, true, false);
        assert_eq!(app.phase, Phase::Success);
        finish(&mut app, false, true);
        assert_eq!(app.phase, Phase::Stopped);
    }
    #[test]
    fn language_switch_preserves_state_and_structured_log_payloads() {
        let mut app = Updater::default();
        app.protocol(FlashEvent::WaitingAck {
            target: Target::Block { index: 1, total: 1 },
            retry: 3,
            timeout_ms: 5000,
        });
        app.follow = false;
        app.unseen = 7;
        app.protocol(FlashEvent::StartupRx {
            byte: 0xff,
            elapsed_ms: 9,
        });
        let record = app.logs.back().unwrap();
        i18n::set(i18n::Language::En, false);
        assert!(message_text(&record.message).contains("Startup RX"));
        assert!(message_text(&record.message).contains("FF"));
        i18n::set(i18n::Language::Zh, false);
        assert!(message_text(&record.message).contains("启动RX"));
        assert_eq!(app.phase, Phase::AwaitingAck);
        assert_eq!(app.retry, 3);
        assert!(!app.follow);
        assert_eq!(app.unseen, 8);
    }
}

#[cfg(test)]
mod log_widget_tests {
    use super::*;
    fn draw(
        app: &mut Updater,
        ctx: &egui::Context,
        time: &mut f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        *time += 0.1;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(620.0, 720.0),
                )),
                time: Some(*time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.logs(ui));
            },
        )
    }
    fn sample() -> Updater {
        let mut app = Updater {
            language: i18n::Language::En,
            ..Updater::default()
        };
        i18n::set(app.language, app.system_english);
        for n in 0..120 {
            app.note(format!("ROW {n:03}"), format!("ROW {n:03}"));
        }
        app
    }
    #[test]
    fn actual_clear_button_preserves_live_transfer_state() {
        i18n::set(i18n::Language::En, false);
        let mut app = sample();
        app.phase = Phase::AwaitingAck;
        app.acknowledged = 1024;
        app.total = 4096;
        app.retry = 3;
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let out = draw(&mut app, &ctx, &mut time, vec![]);
        let position = out
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    if t.galley.job.text == "Clear log" {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .expect("clear button text");
        for pressed in [true, false] {
            draw(
                &mut app,
                &ctx,
                &mut time,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.logs.is_empty());
        assert_eq!(app.phase, Phase::AwaitingAck);
        assert_eq!((app.acknowledged, app.total, app.retry), (1024, 4096, 3));
    }
    #[test]
    fn native_scroll_widget_pauses_on_history_and_resumes_at_bottom() {
        i18n::set(i18n::Language::En, false);
        let mut app = sample();
        let ctx = egui::Context::default();
        let mut time = 0.0;
        draw(&mut app, &ctx, &mut time, vec![]);
        assert!(app.follow);
        assert!(app.log_offset > 0.0);
        let pos = egui::pos2(300.0, 350.0);
        draw(
            &mut app,
            &ctx,
            &mut time,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, 240.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..4 {
            draw(&mut app, &ctx, &mut time, vec![]);
        }
        assert!(!app.follow);
        let offset = app.log_offset;
        app.note("追加日志", "New entry");
        draw(&mut app, &ctx, &mut time, vec![]);
        assert!((app.log_offset - offset).abs() < 1.0);
        i18n::set(i18n::Language::Zh, false);
        draw(&mut app, &ctx, &mut time, vec![]);
        assert!((app.log_offset - offset).abs() < 1.0);
        assert!(!app.follow);
        draw(
            &mut app,
            &ctx,
            &mut time,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -10000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..6 {
            draw(&mut app, &ctx, &mut time, vec![]);
        }
        assert!(app.follow);
        let bottom = app.log_offset;
        app.note("末行", "Last line");
        draw(&mut app, &ctx, &mut time, vec![]);
        assert!(app.follow);
        assert!(app.log_offset > bottom);
    }
}

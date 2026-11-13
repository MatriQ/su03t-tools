//! Passive-on-connect serial debugging. No boot trigger or firmware protocol is sent.
use crate::capture::{Options as CaptureOptions, Recorder};
use anyhow::{Context, Result, bail};
use std::io::{self, Read, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const MAX_SEND_BYTES: usize = 64 * 1024;
const EVENT_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct Config {
    pub port: String,
    pub baud: u32,
    pub data_bits: serialport::DataBits,
    pub parity: serialport::Parity,
    pub stop_bits: serialport::StopBits,
    pub flow_control: serialport::FlowControl,
    pub receive_only: bool,
}
impl Config {
    pub fn preset(port: String, profile: crate::profiles::Profile) -> Self {
        Self {
            port,
            baud: profile.baud(),
            data_bits: serialport::DataBits::Eight,
            parity: serialport::Parity::None,
            stop_bits: serialport::StopBits::One,
            flow_control: serialport::FlowControl::None,
            receive_only: profile.receive_only(),
        }
    }
    pub fn description(&self) -> String {
        format!(
            "{} / {} / {}{}{} / {:?} / {}",
            self.port,
            self.baud,
            u8::from(self.data_bits),
            match self.parity {
                serialport::Parity::None => "N",
                serialport::Parity::Odd => "O",
                serialport::Parity::Even => "E",
            },
            u8::from(self.stop_bits),
            self.flow_control,
            if self.receive_only {
                "只接收"
            } else {
                "手动收发"
            }
        )
    }
    pub fn open_serial(&self, timeout: Duration) -> Result<Box<dyn serialport::SerialPort>> {
        anyhow::ensure!(self.baud > 0, "波特率必须大于 0");
        anyhow::ensure!(
            !self.receive_only || self.flow_control == serialport::FlowControl::None,
            "B8只接收模式必须使用无流控，避免系统发送流控信号"
        );
        serialport::new(&self.port, self.baud)
            .data_bits(self.data_bits)
            .parity(self.parity)
            .stop_bits(self.stop_bits)
            .flow_control(self.flow_control)
            .timeout(timeout)
            .open()
            .map_err(|error| {
                let reason = match error.kind() {
                    serialport::ErrorKind::NoDevice => "设备不存在、已拔出或驱动不可用",
                    serialport::ErrorKind::Io(io::ErrorKind::PermissionDenied) => {
                        "访问被拒绝：检查端口占用；Linux还需核对dialout/uucp组和设备权限"
                    }
                    serialport::ErrorKind::InvalidInput => "不支持的串口参数，请核对转接器能力",
                    _ if error.to_string().to_lowercase().contains("busy") => {
                        "串口被其他程序占用，请关闭其他串口会话"
                    }
                    _ => "检查接线、驱动、设备连接和端口占用",
                };
                anyhow::anyhow!("无法打开 {}：{reason}（{error}）", self.port)
            })
    }
    fn open(&self) -> Result<Box<dyn Transport>> {
        let port = self.open_serial(Duration::from_millis(50))?;
        Ok(Box::new(port))
    }
}

pub trait Transport: Read + Write + Send {}
impl<T: Read + Write + Send> Transport for T {}

#[derive(Debug)]
pub enum Event {
    Connected,
    Received { at: Duration, bytes: Vec<u8> },
    Sent { at: Duration, bytes: Vec<u8> },
}

/// Owns the only serial handle. Drop/stop always releases it, even under event backpressure.
pub struct Session {
    commands: SyncSender<Vec<u8>>,
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    result: Arc<Mutex<Option<std::result::Result<(), String>>>>,
    worker: Option<JoinHandle<()>>,
    receive_only: bool,
}
impl Session {
    pub fn connect(config: Config) -> Self {
        Self::connect_recorded(config, None)
    }
    pub fn connect_recorded(config: Config, capture: Option<CaptureOptions>) -> Self {
        let description = config.description();
        let receive_only = config.receive_only;
        Self::start(move || config.open(), capture, description, receive_only)
    }

    /// Useful for transport-level integration tests. The transport MUST have bounded I/O timeouts.
    pub fn with_transport(transport: impl Transport + 'static) -> Self {
        Self::with_recorded_transport(transport, None, false)
    }
    pub fn with_recorded_transport(
        transport: impl Transport + 'static,
        capture: Option<CaptureOptions>,
        receive_only: bool,
    ) -> Self {
        Self::start(
            move || Ok(Box::new(transport)),
            capture,
            "test transport".into(),
            receive_only,
        )
    }
    fn start(
        open: impl FnOnce() -> Result<Box<dyn Transport>> + Send + 'static,
        capture: Option<CaptureOptions>,
        description: String,
        receive_only: bool,
    ) -> Self {
        let (commands, command_rx) = mpsc::sync_channel::<Vec<u8>>(16);
        let (event_tx, events) = mpsc::sync_channel(EVENT_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let worker_stop = stop.clone();
        let worker_result = result.clone();
        let worker = thread::spawn(move || {
            let send_event = |mut event| -> bool {
                loop {
                    if worker_stop.load(Ordering::Acquire) {
                        return false;
                    }
                    match event_tx.try_send(event) {
                        Ok(()) => return true,
                        Err(TrySendError::Full(value)) => {
                            event = value;
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(TrySendError::Disconnected(_)) => return false,
                    }
                }
            };
            let started = Instant::now();
            let mut recorder = None;
            let mut outcome = (|| -> Result<()> {
                if worker_stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                recorder = capture
                    .as_ref()
                    .map(|options| Recorder::open(options, &description))
                    .transpose()?;
                let mut port = open()?;
                if !send_event(Event::Connected) {
                    return Ok(());
                }
                let mut buffer = [0u8; 4096];
                while !worker_stop.load(Ordering::Acquire) {
                    // Bound each send burst to preserve receive/disconnect responsiveness.
                    for bytes in command_rx.try_iter().take(4) {
                        if worker_stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        port.write_all(&bytes)
                            .context("串口发送失败，部分数据可能已发送，请核对设备后重试")?;
                        if let Some(recorder) = &mut recorder {
                            recorder.event(started.elapsed(), "TX", &bytes)?;
                        }
                        // A successful write is a host-side write, not a device acknowledgement.
                        if !send_event(Event::Sent {
                            at: started.elapsed(),
                            bytes,
                        }) {
                            return Ok(());
                        }
                    }
                    match port.read(&mut buffer) {
                        Ok(0) => bail!("串口已断开"),
                        Ok(n) => {
                            if let Some(recorder) = &mut recorder {
                                recorder.event(started.elapsed(), "RX", &buffer[..n])?;
                            }
                            if !send_event(Event::Received {
                                at: started.elapsed(),
                                bytes: buffer[..n].to_vec(),
                            }) {
                                return Ok(());
                            }
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::TimedOut
                                    | io::ErrorKind::WouldBlock
                                    | io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(e).context("串口接收失败，设备可能已断开"),
                    }
                }
                Ok(())
            })(); // The serial handle is closed before the completion result becomes visible.
            if let Some(recorder) = &mut recorder {
                let result = recorder
                    .system(
                        started.elapsed(),
                        &match &outcome {
                            Ok(()) => "连接已结束；串口已释放".into(),
                            Err(error) => format!("ERROR {error:#}"),
                        },
                    )
                    .and_then(|_| recorder.flush());
                if let Err(error) = result {
                    outcome = Err(error.context("完成记录失败"));
                }
            }
            *worker_result.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(outcome.map_err(|e| format!("{e:#}")));
        });
        Self {
            commands,
            events,
            stop,
            result,
            worker: Some(worker),
            receive_only,
        }
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        anyhow::ensure!(!self.receive_only, "B8日志模式只允许接收，禁止发送");
        anyhow::ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_SEND_BYTES,
            "单次发送需为 1～65536 字节"
        );
        anyhow::ensure!(
            !self.stop.load(Ordering::Acquire) && self.finished_result().is_none(),
            "串口正在断开或已关闭"
        );
        self.commands.try_send(bytes).map_err(|e| {
            anyhow::anyhow!(match e {
                TrySendError::Full(_) => "发送队列已满，请稍后重试",
                TrySendError::Disconnected(_) => "串口已关闭",
            })
        })
    }
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn finished_result(&self) -> Option<std::result::Result<(), String>> {
        if self.worker.as_ref().is_some_and(|h| h.is_finished()) {
            Some(
                self.result
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .unwrap_or_else(|| Err("串口工作线程意外结束".into())),
            )
        } else {
            None
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn parse_hex(input: &str) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    for token in input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
    {
        let token = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        anyhow::ensure!(
            !token.is_empty() && token.len().is_multiple_of(2) && token.is_ascii(),
            "每组十六进制需为偶数位，例如 AA 01 55 或 AA0155"
        );
        anyhow::ensure!(
            result.len().saturating_add(token.len() / 2) <= MAX_SEND_BYTES,
            "单次最多发送 65536 字节"
        );
        for pair in token.as_bytes().chunks_exact(2) {
            let text = std::str::from_utf8(pair)?;
            result.push(u8::from_str_radix(text, 16).context("十六进制只能包含 0–9 和 A–F")?);
        }
        anyhow::ensure!(result.len() <= MAX_SEND_BYTES, "单次最多发送 65536 字节");
    }
    anyhow::ensure!(!result.is_empty(), "请输入要发送的十六进制数据");
    Ok(result)
}

pub fn text_payload(text: &str, ending: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(
        text.len().saturating_add(ending.len()) <= MAX_SEND_BYTES,
        "单次最多发送 65536 字节"
    );
    let mut bytes = text.as_bytes().to_vec();
    bytes.extend_from_slice(ending);
    anyhow::ensure!(!bytes.is_empty(), "请输入要发送的文本");
    Ok(bytes)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Streaming UTF-8 decoder: preserve split characters, escape invalid binary bytes.
#[derive(Default)]
pub struct TextDecoder {
    pending: Vec<u8>,
}
impl TextDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    output.push_str(
                        std::str::from_utf8(&self.pending[..valid]).expect("valid prefix"),
                    );
                    if let Some(length) = error.error_len() {
                        for b in &self.pending[valid..valid + length] {
                            output.push_str(&format!("\\x{b:02X}"));
                        }
                        self.pending.drain(..valid + length);
                    } else {
                        self.pending.drain(..valid);
                        break;
                    }
                }
            }
        }
        visible_controls(&output)
    }
    pub fn finish(&mut self) -> String {
        let s = self
            .pending
            .iter()
            .map(|b| format!("\\x{b:02X}"))
            .collect::<String>();
        self.pending.clear();
        s
    }
}
fn visible_controls(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => "\\n\n".into(),
            '\r' => "\\r".into(),
            '\t' => "\\t".into(),
            c if c.is_control() => format!("\\x{:02X}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receive_only_rejects_kernel_flow_control_before_opening_a_device() {
        let mut config = Config::preset(
            "/dev/su03t-test-must-not-open".into(),
            crate::profiles::Profile::B8,
        );
        assert!(config.description().contains("57600 / 8N1"));
        config.flow_control = serialport::FlowControl::Software;
        let error = match config.open_serial(Duration::from_millis(10)) {
            Err(error) => error,
            Ok(_) => panic!("flow control must be rejected"),
        };
        assert!(error.to_string().contains("B8只接收模式必须使用无流控"));
    }
    #[test]
    fn hex_is_exact_and_rejects_malformed_input() {
        for s in ["AA 01 55", "aa0155", "0xAA, 0X01,55"] {
            assert_eq!(parse_hex(s).unwrap(), [170, 1, 85]);
        }
        for s in ["", "  ", "A A", "0x", "ZZ", "你好", "12:34"] {
            assert!(parse_hex(s).is_err(), "{s}");
        }
        assert_eq!(parse_hex("00 FF 0A").unwrap(), [0, 255, 10]);
        assert_eq!(
            parse_hex(&"00".repeat(MAX_SEND_BYTES)).unwrap().len(),
            MAX_SEND_BYTES
        );
        assert!(parse_hex(&"00".repeat(MAX_SEND_BYTES + 1)).is_err());
    }
    #[test]
    fn utf8_spanning_reads_and_binary_are_preserved() {
        let mut decoder = TextDecoder::default();
        assert_eq!(decoder.push(&[0xe4, 0xbd]), "");
        assert_eq!(decoder.push(&[0xa0, 0xe5, 0xa5, 0xbd]), "你好");
        assert_eq!(decoder.push(&[0xff, 0, 13, 10]), "\\xFF\\x00\\r\\n\n");
        assert_eq!(decoder.push(&[0xe4]), "");
        assert_eq!(decoder.finish(), "\\xE4");
    }
    #[test]
    fn text_payload_preserves_utf8_and_explicit_endings() {
        assert_eq!(
            text_payload("你好", b"\r\n").unwrap(),
            "你好\r\n".as_bytes()
        );
        assert_eq!(text_payload("AT", b"").unwrap(), b"AT");
        assert_eq!(text_payload("", b"\n").unwrap(), b"\n");
        assert!(text_payload("", b"").is_err());
        assert!(text_payload(&"x".repeat(MAX_SEND_BYTES), b"\n").is_err());
    }
}

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use su03t_update::{
    capture,
    firmware::Image,
    profiles::{CarTracker, Profile},
    serial_debug::{self as serial, Config, Event, Session},
};
#[cfg(feature = "gui")]
mod gui;

#[derive(Parser)]
#[command(
    version,
    about = "SU-03T 调试与升级工具 · macOS / Linux；无参数打开桌面界面"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    /// 枚举系统串口，不打开设备
    Ports,
    /// 检查串口升级文件并计算SHA-256，不接触设备
    FirmwareInfo {
        #[arg(long)]
        firmware: PathBuf,
    },
    /// 实验性串口升级：仅适用于已兼容的蜂鸟M引导程序
    Flash {
        #[arg(long)]
        port: String,
        #[arg(long)]
        firmware: PathBuf,
        #[arg(long, default_value_t = 921600)]
        baud: u32,
        #[arg(long, default_value_t = 60)]
        wait: u64,
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 30)]
        retries: usize,
        /// 不发送0x32（字符2）启动触发，仅等待CCC
        #[arg(long)]
        passive: bool,
        #[arg(long)]
        allow_other_filename: bool,
        /// 文件变化时拒绝烧录；校验发生在打开串口之前
        #[arg(long)]
        expected_sha256: Option<String>,
        /// 仅限macOS PTY模拟测试
        #[arg(long, hide = true)]
        virtual_port: bool,
        /// Structured worker events for the native UI.
        #[arg(long, hide = true)]
        events: bool,
    },
    /// 只诊断启动握手，绝不传固件；默认只接收
    Probe {
        #[arg(long)]
        port: String,
        #[arg(long, default_value_t = 921600)]
        baud: u32,
        #[arg(long, default_value_t = 15)]
        wait: u64,
        /// 显式启用官方SPL分支的0x32（字符2）触发；会影响正常启动
        #[arg(long)]
        trigger: bool,
        #[arg(long, hide = true)]
        virtual_port: bool,
        #[arg(long, hide = true)]
        events: bool,
    },
    /// 串口调试：默认只监听；发送必须显式指定；Ctrl+C结束
    Monitor(DebugArgs),
}
#[derive(Clone, Copy, ValueEnum)]
enum DebugProfile {
    Car,
    B8,
    Custom,
}
impl DebugProfile {
    fn profile(self) -> Profile {
        match self {
            Self::Car => Profile::Car,
            Self::B8 => Profile::B8,
            Self::Custom => Profile::Custom,
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum Parity {
    None,
    Odd,
    Even,
}
#[derive(Clone, Copy, ValueEnum)]
enum Flow {
    None,
    Hardware,
    Software,
}
#[derive(clap::Args)]
struct DebugArgs {
    #[arg(long)]
    port: String,
    #[arg(long, value_enum, default_value = "custom")]
    profile: DebugProfile,
    #[arg(long)]
    baud: Option<u32>,
    #[arg(long, value_parser = clap::value_parser!(u8).range(5..=8))]
    data_bits: Option<u8>,
    #[arg(long, value_enum)]
    parity: Option<Parity>,
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    stop_bits: Option<u8>,
    #[arg(long, value_enum)]
    flow: Option<Flow>,
    /// 一次性发送原始HEX，不附加帧头或结尾
    #[arg(long, conflicts_with = "send_text")]
    send_hex: Option<String>,
    #[arg(long)]
    send_text: Option<String>,
    /// 同时记录 PREFIX.rx.bin 和 PREFIX.log；每流16MiB×4文件滚动保存
    #[arg(long)]
    record: Option<PathBuf>,
    /// 最大连接秒数，0表示持续监听
    #[arg(long, default_value_t = 0.0)]
    duration: f64,
    #[arg(long, hide = true)]
    virtual_port: bool,
}
#[cfg(test)]
fn load_firmware(path: &std::path::Path, allow_other_filename: bool) -> Result<Vec<u8>> {
    Ok(Image::load(path, allow_other_filename)?.bytes().to_vec())
}
fn open(port: &str, baud: u32, virtual_port: bool) -> Result<Box<dyn serialport::SerialPort>> {
    if virtual_port {
        ensure!(
            cfg!(target_os = "macos") && port.starts_with("/dev/ttys"),
            "--virtual-port is only for macOS PTY integration tests"
        );
        return serialport::new(port, 0)
            .timeout(Duration::from_millis(10))
            .open()
            .context("打开测试PTY");
    }
    let mut config = Config::preset(port.into(), Profile::Custom);
    config.baud = baud;
    config.open_serial(Duration::from_millis(10))
}
#[derive(Default)]
struct HandshakeTrace {
    count: u64,
    sample: Vec<u8>,
}
impl HandshakeTrace {
    fn byte(&mut self, byte: u8, started: Instant) {
        self.count += 1;
        if self.sample.len() < 256 {
            self.sample.push(byte);
        }
        if self.count <= 8 {
            eprintln!("启动RX +{}ms：{byte:02X}", started.elapsed().as_millis());
        }
    }
    fn finish(&self) {
        eprintln!(
            "握手阶段累计接收：{} 字节（不含固件传输响应）。",
            self.count
        );
        if self.count == 0 {
            eprintln!(
                "握手阶段未收到任何字节；请检查模块TX到转串口RX的返回线路、波特率及启动程序兼容性。"
            );
        } else {
            eprintln!("握手RX HEX（最多前256字节）：{}", serial::hex(&self.sample));
        }
    }
}
fn monitor(args: DebugArgs) -> Result<()> {
    ensure!(
        args.duration.is_finite() && args.duration >= 0.0,
        "连接时长需为非负有限数字"
    );
    let profile = args.profile.profile();
    let mut config = Config::preset(args.port, profile);
    if let Some(baud) = args.baud {
        config.baud = baud;
    }
    if let Some(bits) = args.data_bits {
        config.data_bits = match bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        };
    }
    if let Some(parity) = args.parity {
        config.parity = match parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        };
    }
    if let Some(bits) = args.stop_bits {
        config.stop_bits = if bits == 2 {
            serialport::StopBits::Two
        } else {
            serialport::StopBits::One
        };
    }
    if let Some(flow) = args.flow {
        config.flow_control = match flow {
            Flow::None => serialport::FlowControl::None,
            Flow::Hardware => serialport::FlowControl::Hardware,
            Flow::Software => serialport::FlowControl::Software,
        };
    }
    let payload = if let Some(hex) = args.send_hex {
        Some(serial::parse_hex(&hex)?)
    } else if let Some(text) = args.send_text {
        Some(serial::text_payload(&text, b"")?)
    } else {
        None
    };
    ensure!(
        !profile.receive_only() || payload.is_none(),
        "B8日志模式只允许接收，禁止发送"
    );
    eprintln!(
        "{}\n{}\n{}\n{}",
        capture::environment(),
        profile.name(),
        config.description(),
        profile.wiring()
    );
    let running = Arc::new(AtomicBool::new(true));
    let stop = running.clone();
    ctrlc::set_handler(move || {
        stop.store(false, Ordering::Release);
    })
    .context("注册退出处理")?;
    let capture = args.record.map(capture::Options::new);
    let session = if args.virtual_port {
        Session::with_recorded_transport(
            open(&config.port, config.baud, true)?,
            capture,
            config.receive_only,
        )
    } else {
        Session::connect_recorded(config, capture)
    };
    let started = Instant::now();
    let mut pending = payload;
    let (mut rx, mut tx) = (0u64, 0u64);
    let mut tracker = CarTracker::default();
    let mut closing = false;
    loop {
        let completed = session.finished_result();
        while let Some(event) = session.try_event() {
            match event {
                Event::Connected => {
                    eprintln!("[0.000s] SYS 已连接；未显式指定发送参数时只监听");
                    if let Some(bytes) = pending.take() {
                        session.send(bytes)?;
                    }
                }
                Event::Received { at, bytes } => {
                    rx += bytes.len() as u64;
                    if profile == Profile::Car {
                        tracker.observe(&bytes);
                    }
                    println!("[{:.3}s] RX HEX={}", at.as_secs_f64(), serial::hex(&bytes));
                }
                Event::Sent { at, bytes } => {
                    tx += bytes.len() as u64;
                    println!("[{:.3}s] TX HEX={}", at.as_secs_f64(), serial::hex(&bytes));
                }
            }
        }
        if let Some(result) = completed {
            eprintln!(
                "RX={rx} B TX={tx} B duration={:.3}s errors={}",
                started.elapsed().as_secs_f64(),
                u8::from(result.is_err())
            );
            if profile == Profile::Car {
                eprintln!(
                    "当前小车配置接收次数：{:?}，未知字节={}",
                    tracker.counts, tracker.unknown
                );
            }
            return result.map_err(anyhow::Error::msg);
        }
        if !closing
            && (!running.load(Ordering::Acquire)
                || (args.duration > 0.0 && started.elapsed().as_secs_f64() >= args.duration))
        {
            session.stop();
            closing = true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn emit_event(event: su03t_update::flash_event::FlashEvent) {
    println!(
        "{}{}",
        su03t_update::flash_event::EVENT_PREFIX,
        serde_json::to_string(&event).expect("serializable worker event")
    );
}

/// The default human-readable CLI remains compatible. The GUI uses this separate
/// event stream and never has to infer transport state from translated log text.
fn structured_transfer(
    port: &str,
    baud: u32,
    virtual_port: bool,
    image_request: Option<(&std::path::Path, bool, Option<&str>)>,
    options: &su03t_update::Options,
) -> Result<()> {
    use su03t_update::flash_event::FlashEvent;
    let probe = image_request.is_none();
    let result = (|| -> Result<()> {
        ensure!(
            baud > 0 && !options.handshake_timeout.is_zero() && !options.response_timeout.is_zero(),
            "baud rate and timeouts must be greater than zero"
        );
        let image = image_request
            .map(|(path, allow, expected)| -> Result<Image> {
                let image = Image::load(path, allow)?;
                image.verify_expected(expected)?;
                emit_event(FlashEvent::Image {
                    path: image.path.display().to_string(),
                    bytes: image.bytes().len(),
                    sha256: image.sha256.clone(),
                });
                Ok(image)
            })
            .transpose()?;
        let mut io = open(port, baud, virtual_port)?;
        emit_event(FlashEvent::PortOpened {
            port: port.into(),
            baud,
        });
        let started = Instant::now();
        let mut trace = HandshakeTrace::default();
        let receive = |byte| {
            trace.count += 1;
            if trace.sample.len() < 256 {
                trace.sample.push(byte);
            }
            if trace.count <= 8 {
                emit_event(FlashEvent::StartupRx {
                    byte,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        };
        let result = if let Some(image) = image {
            su03t_update::send_observed(
                io.as_mut(),
                image.bytes(),
                options,
                |done, total| emit_event(FlashEvent::Progress { done, total }),
                receive,
                emit_event,
            )
        } else {
            su03t_update::wait_for_handshake_observed(io.as_mut(), options, receive, emit_event)
        };
        emit_event(FlashEvent::StartupSummary {
            count: trace.count,
            sample: trace.sample,
        });
        result?;
        // Drop the serial handle before reporting completion.
        drop(io);
        emit_event(FlashEvent::Completed { probe });
        Ok(())
    })();
    if let Err(error) = &result {
        let detail = format!("{error:#}");
        let code = if detail.contains("waiting for CCC timed out") {
            "handshake_timeout"
        } else if detail.contains("NNN") {
            "device_skip"
        } else if detail.contains("CAN") {
            "device_cancelled"
        } else if detail.contains("SHA-256") {
            "firmware_changed"
        } else if detail.contains("retry limit") {
            "ack_timeout"
        } else if detail.contains("无法打开") || detail.contains("打开测试PTY") {
            "open_port"
        } else if detail.contains("固件") {
            "firmware"
        } else {
            "transport"
        };
        emit_event(FlashEvent::Failed {
            code: code.into(),
            detail,
        });
    }
    result
}

fn main() -> Result<()> {
    match Args::parse().command {
        None => {
            #[cfg(feature = "gui")]
            {
                gui::run().map_err(|e| anyhow::anyhow!(e.to_string()))
            }
            #[cfg(not(feature = "gui"))]
            {
                anyhow::bail!("此构建仅含CLI，请使用 --help 查看命令")
            }
        }
        Some(Commands::Ports) => {
            for port in serialport::available_ports().context("列出串口")? {
                println!("{}\t{:?}", port.port_name, port.port_type);
            }
            Ok(())
        }
        Some(Commands::FirmwareInfo { firmware }) => {
            let image = Image::load(&firmware, false)?;
            println!(
                "{}\n路径：{}\n大小：{} bytes\nSHA-256：{}\n仅通过基础检查；文件名和哈希不能证明型号、协议或兼容性。",
                capture::environment(),
                image.path.display(),
                image.bytes().len(),
                image.sha256
            );
            Ok(())
        }
        Some(Commands::Flash {
            port,
            firmware,
            baud,
            wait,
            timeout_ms,
            retries,
            passive,
            allow_other_filename,
            expected_sha256,
            virtual_port,
            events,
        }) => {
            if events {
                return structured_transfer(
                    &port,
                    baud,
                    virtual_port,
                    Some((&firmware, allow_other_filename, expected_sha256.as_deref())),
                    &su03t_update::Options {
                        handshake_timeout: Duration::from_secs(wait),
                        response_timeout: Duration::from_millis(timeout_ms),
                        retries,
                        trigger: !passive,
                    },
                );
            }
            ensure!(
                baud > 0 && wait > 0 && timeout_ms > 0,
                "波特率和超时时间必须大于0"
            );
            let image = Image::load(&firmware, allow_other_filename)?;
            image.verify_expected(expected_sha256.as_deref())?;
            eprintln!(
                "{}\n固件 {}：{} 字节\nSHA-256：{}",
                capture::environment(),
                image.path.display(),
                image.bytes().len(),
                image.sha256
            );
            eprintln!(
                "实验性升级：仅适用于兼容的蜂鸟M启动程序；未完成跨平台实板验证；无读回校验。"
            );
            let mut io = open(&port, baud, virtual_port)?;
            eprintln!(
                "串口已打开：{port}，{baud}，8N1。等待CCC，现在给模块断电再上电。B7→RX，B6→TX，共地，独立5V供电。"
            );
            eprintln!(
                "等待启动握手，不检测供电状态。{}；未收到CCC前不发送固件。",
                if passive {
                    "仅接收，不发送启动触发"
                } else {
                    "正在重复发送启动触发32（字符2）"
                }
            );
            let options = su03t_update::Options {
                handshake_timeout: Duration::from_secs(wait),
                response_timeout: Duration::from_millis(timeout_ms),
                retries,
                trigger: !passive,
            };
            let started = Instant::now();
            let mut trace = HandshakeTrace::default();
            let result = su03t_update::send_with_handshake_observer(
                io.as_mut(),
                image.bytes(),
                &options,
                |done, total| {
                    if done == 0 {
                        eprintln!("已收到CCC启动握手，开始传输固件。");
                    }
                    eprintln!("PROGRESS {done} {total}");
                },
                |byte| trace.byte(byte, started),
            );
            trace.finish();
            if let Err(error) = &result {
                eprintln!("RESULT failed：{error:#}");
            }
            result?;
            eprintln!(
                "RESULT device-confirmed：设备已确认EOT传输结束；未读回校验，请重新上电验证语音功能。"
            );
            Ok(())
        }
        Some(Commands::Probe {
            port,
            baud,
            wait,
            trigger,
            virtual_port,
            events,
        }) => {
            if events {
                return structured_transfer(
                    &port,
                    baud,
                    virtual_port,
                    None,
                    &su03t_update::Options {
                        handshake_timeout: Duration::from_secs(wait),
                        trigger,
                        ..Default::default()
                    },
                );
            }
            ensure!(baud > 0 && wait > 0, "波特率和等待时间必须大于0");
            eprintln!(
                "{}\n启动诊断：{port} / {baud} / 8N1；不传固件，不发送擦除或复位命令。",
                capture::environment()
            );
            eprintln!(
                "{}；现在给模块断电再上电。",
                if trigger {
                    "重复发送官方SPL启动触发32（字符2）"
                } else {
                    "仅接收，不发送启动触发"
                }
            );
            let mut io = open(&port, baud, virtual_port)?;
            eprintln!("串口已打开：{port}，{baud}，8N1。现在给模块断电再上电。");
            let options = su03t_update::Options {
                handshake_timeout: Duration::from_secs(wait),
                trigger,
                ..Default::default()
            };
            let started = Instant::now();
            let mut trace = HandshakeTrace::default();
            let result = su03t_update::wait_for_handshake(io.as_mut(), &options, |byte| {
                trace.byte(byte, started)
            });
            trace.finish();
            result?;
            eprintln!(
                "启动诊断完成：已识别CCC。未发送任何固件，不能据此确认设备型号或烧录兼容性。"
            );
            Ok(())
        }
        Some(Commands::Monitor(args)) => monitor(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_and_full_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_update.bin");
        std::fs::write(&path, []).unwrap();
        assert!(load_firmware(&path, false).is_err());
        std::fs::write(&path, b"unsupport xmodem-128 trans").unwrap();
        assert!(load_firmware(&path, false).is_err());
        std::fs::write(&path, b"image").unwrap();
        assert_eq!(load_firmware(&path, false).unwrap(), b"image");
        assert!(load_firmware(&dir.path().join("full.bin"), false).is_err());
    }
}

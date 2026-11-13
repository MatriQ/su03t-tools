//! Controlled no-firmware comparison with the observed vendor Windows startup.
//! Use only with the confirmed B6/B7/GND three-wire connection; DTR/RTS must not
//! be connected to board reset/power circuitry. This program never sends images.
use anyhow::{Context, Result};
use std::{
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
struct QueuedWrites<T>(T, Arc<AtomicBool>);
impl<T: Read> Read for QueuedWrites<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.1.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "operator ended observation",
            ));
        }
        self.0.read(bytes)
    }
}
impl<T: Write> Write for QueuedWrites<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    // Vendor code enqueues Output; no explicit host tcdrain in that path.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let path = args
        .get(1)
        .context("explicit B6/B7/GND serial port required")?;
    let baud: u32 = args
        .get(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(921600);
    let wait: u64 = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(180);
    anyhow::ensure!(wait > 0 && wait <= 600, "wait must be 1..600 seconds");
    let mut port = serialport::new(path, 9600)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(10))
        .open()?;
    eprintln!(
        "{}\n对照：先9600打开，切换{baud}/8N1，无流控；只发送32（字符2），不传固件。",
        su03t_update::capture::environment()
    );
    port.set_baud_rate(baud)?;
    for (name, result) in [
        ("DTR=true", port.write_data_terminal_ready(true)),
        ("RTS=false", port.write_request_to_send(false)),
    ] {
        match result {
            Ok(()) => eprintln!("控制线设置成功：{name}"),
            Err(error) => {
                eprintln!("控制线设置不受支持：{name}；{error}（本次不能完全复现Windows线状态）")
            }
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    eprintln!(
        "READY unix_ms={}：串口已稳定，现在给模块断电再上电；等待{wait}秒。此对照不执行固件传输。",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    );
    let started = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = stop.clone();
    ctrlc::set_handler(move || {
        cancel.store(true, Ordering::Release);
    })?;
    let mut io = QueuedWrites(port, stop);
    let mut count = 0u64;
    let mut sample = Vec::new();
    let result = su03t_update::wait_for_handshake(
        &mut io,
        &su03t_update::Options {
            handshake_timeout: Duration::from_secs(wait),
            ..Default::default()
        },
        |byte| {
            count += 1;
            if sample.len() < 256 {
                sample.push(byte);
            }
            if count <= 16 {
                eprintln!("RX +{}ms {byte:02X}", started.elapsed().as_millis());
            }
        },
    );
    eprintln!(
        "RESULT RX={count} HEX={}",
        su03t_update::serial_debug::hex(&sample)
    );
    match &result {
        Ok(()) => eprintln!("识别到CCC；没有发送固件。"),
        Err(error) => eprintln!("未建立握手：{error:#}"),
    }
    result
}

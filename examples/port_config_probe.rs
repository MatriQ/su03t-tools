//! Explicit, user-wired physical-loopback diagnostic; never run by cargo test.
#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    use std::{
        io::{Read, Write},
        os::fd::AsRawFd,
        time::{Duration, Instant},
    };
    let args: Vec<_> = std::env::args().collect();
    let path = args
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("explicit loopback port required"))?;
    let variant = args.get(2).map(String::as_str).unwrap_or("native");
    let mut port = serialport::new(path, 115200)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(50))
        .open_native()?;
    let fd = port.as_raw_fd();
    if variant == "shared" {
        port.set_exclusive(false)?;
    }
    let mut attrs = std::mem::MaybeUninit::<libc::termios>::uninit();
    anyhow::ensure!(
        unsafe { libc::tcgetattr(fd, attrs.as_mut_ptr()) } == 0,
        "tcgetattr failed"
    );
    let mut attrs = unsafe { attrs.assume_init() };
    println!(
        "variant={variant} fd={fd} flags={:x}/{:x}/{:x}/{:x} speed={}/{} VMIN={} VTIME={}",
        attrs.c_iflag,
        attrs.c_oflag,
        attrs.c_cflag,
        attrs.c_lflag,
        attrs.c_ispeed,
        attrs.c_ospeed,
        attrs.c_cc[libc::VMIN],
        attrs.c_cc[libc::VTIME]
    );
    if variant == "raw-reset" {
        attrs.c_iflag = 0;
        attrs.c_oflag = 0;
        attrs.c_cflag = libc::CS8 | libc::CREAD | libc::CLOCAL;
        attrs.c_lflag = 0;
        attrs.c_cc[libc::VMIN] = 0;
        attrs.c_cc[libc::VTIME] = 0;
        unsafe {
            anyhow::ensure!(
                libc::cfsetispeed(&mut attrs, libc::B115200) == 0,
                "cfsetispeed failed"
            );
            anyhow::ensure!(
                libc::cfsetospeed(&mut attrs, libc::B115200) == 0,
                "cfsetospeed failed"
            );
        }
        anyhow::ensure!(
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &attrs) } == 0,
            "tcsetattr failed"
        );
    }
    std::thread::sleep(Duration::from_millis(150));
    port.write_all(&[0x55, 0xaa, 0x01, 0x23])?;
    std::thread::sleep(Duration::from_millis(40));
    use serialport::SerialPort;
    println!("bytes_to_read={:?}", port.bytes_to_read());
    let started = Instant::now();
    let mut bytes = vec![];
    while started.elapsed() < Duration::from_secs(1) {
        let mut chunk = [0u8; 256];
        match port.read(&mut chunk) {
            Ok(n) if n > 0 => bytes.extend_from_slice(&chunk[..n]),
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error.into()),
        }
    }
    println!("RX={}", su03t_update::serial_debug::hex(&bytes));
    Ok(())
}
#[cfg(not(unix))]
fn main() {
    eprintln!("Unix serial-loopback diagnostic only");
}

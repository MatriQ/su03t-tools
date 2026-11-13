#![cfg(unix)]
use serialport::{SerialPort, TTYPort};
use std::{
    io::{Read, Write},
    thread,
    time::{Duration, Instant},
};
use su03t_update::serial_debug::{Config, Event, Session, parse_hex, text_payload};

fn event(session: &Session, predicate: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(event) = session.try_event()
            && predicate(&event)
        {
            return event;
        }
        if let Some(result) = session.finished_result() {
            panic!("session unexpectedly ended: {result:?}");
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out waiting for event");
}
fn done(session: &Session) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(result) = session.finished_result() {
            return result;
        }
        assert!(Instant::now() < deadline, "disconnect timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn passive_connect_bidirectional_bytes_and_reconnect() {
    let (mut device, mut host) = TTYPort::pair().unwrap();
    device.set_timeout(Duration::from_millis(120)).unwrap();
    host.set_timeout(Duration::from_millis(20)).unwrap();
    let path = host.name().unwrap();
    let session = Session::with_transport(host);
    event(&session, |e| matches!(e, Event::Connected));
    // A terminal connection must never send the updater's ASCII 2 trigger or any command.
    assert!(device.read(&mut [0u8; 1]).is_err());
    let incoming = b"\x00\xff\xe4\xbd\xa0\r\n\x43\x43\x43";
    device.write_all(incoming).unwrap();
    let mut received = Vec::new();
    while received.len() < incoming.len() {
        if let Event::Received { bytes, .. } =
            event(&session, |e| matches!(e, Event::Received { .. }))
        {
            received.extend(bytes);
        }
    }
    assert_eq!(received, incoming);
    let payload = parse_hex("AA 00 FF 55").unwrap();
    session.send(payload.clone()).unwrap();
    let mut wire = vec![0; payload.len()];
    device.read_exact(&mut wire).unwrap();
    assert_eq!(wire, payload);
    match event(&session, |e| matches!(e, Event::Sent { .. })) {
        Event::Sent { bytes, .. } => assert_eq!(bytes, payload),
        _ => unreachable!(),
    }
    let text = text_payload("你好", b"\r\n").unwrap();
    session.send(text.clone()).unwrap();
    let mut wire = vec![0; text.len()];
    device.read_exact(&mut wire).unwrap();
    assert_eq!(wire, text);
    event(&session, |e| matches!(e, Event::Sent { .. }));
    session.stop();
    done(&session).unwrap();
    drop(session);
    // macOS PTYs must use baud 0 to skip hardware-only speed ioctls.
    let reopened = serialport::new(&path, 0)
        .timeout(Duration::from_millis(20))
        .open()
        .unwrap();
    let next = Session::with_transport(reopened);
    event(&next, |e| matches!(e, Event::Connected));
    drop(next);
}

#[test]
fn unplug_and_open_error_finish_cleanly() {
    let (device, mut host) = TTYPort::pair().unwrap();
    host.set_timeout(Duration::from_millis(20)).unwrap();
    let session = Session::with_transport(host);
    event(&session, |e| matches!(e, Event::Connected));
    drop(device);
    assert!(done(&session).is_err());
    drop(session);
    let missing = Session::connect(Config {
        port: "/dev/su03t-test-does-not-exist".into(),
        baud: 115200,
        data_bits: serialport::DataBits::Eight,
        parity: serialport::Parity::None,
        stop_bits: serialport::StopBits::One,
        flow_control: serialport::FlowControl::None,
        receive_only: false,
    });
    assert!(done(&missing).is_err());
}

#[test]
fn disconnect_while_receive_queue_is_full_does_not_hang() {
    let (mut device, mut host) = TTYPort::pair().unwrap();
    host.set_timeout(Duration::from_millis(20)).unwrap();
    device.set_timeout(Duration::from_millis(50)).unwrap();
    // The synthetic producer must remain bounded after the receiver stops.
    // TTYPort polls before write(), but a blocking Linux PTY write can still
    // fill the peer's remaining buffer and wait indefinitely inside write().
    use std::os::fd::AsRawFd;
    let fd = device.as_raw_fd();
    // SAFETY: device owns this live descriptor; fcntl only updates its flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0);
    // SAFETY: the descriptor remains owned here and the flags contain no pointers.
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let session = Session::with_transport(host);
    event(&session, |e| matches!(e, Event::Connected));
    let writer = thread::spawn(move || {
        let end = Instant::now() + Duration::from_millis(400);
        while Instant::now() < end {
            let _ = device.write(&[0xff; 4096]);
        }
    });
    // Intentionally do not consume any data events.
    thread::sleep(Duration::from_millis(180));
    session.stop();
    done(&session).unwrap();
    drop(session);
    writer.join().unwrap();
}

#[test]
fn active_serial_handle_is_exclusive_and_released_on_disconnect() {
    let (_device, host) = TTYPort::pair().unwrap();
    let path = host.name().unwrap();
    drop(host);
    let opened = serialport::new(&path, 0)
        .timeout(Duration::from_millis(20))
        .open()
        .unwrap();
    let session = Session::with_transport(opened);
    event(&session, |e| matches!(e, Event::Connected));
    assert!(
        serialport::new(&path, 0).open().is_err(),
        "another reader/updater must not open the active port"
    );
    session.stop();
    done(&session).unwrap();
    drop(session);
    assert!(
        serialport::new(&path, 0).open().is_ok(),
        "disconnect must release the port for firmware upgrade"
    );
}

#[test]
fn b8_is_receive_only_and_disk_capture_preserves_raw_bytes() {
    let (mut device, mut host) = TTYPort::pair().unwrap();
    device.set_timeout(Duration::from_millis(120)).unwrap();
    host.set_timeout(Duration::from_millis(20)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("b8-session");
    let session = Session::with_recorded_transport(
        host,
        Some(su03t_update::capture::Options::new(prefix.clone())),
        true,
    );
    event(&session, |e| matches!(e, Event::Connected));
    assert!(
        session
            .send(vec![0x0a])
            .unwrap_err()
            .to_string()
            .contains("禁止发送")
    );
    assert!(device.read(&mut [0u8; 1]).is_err());
    let bytes = [0, 1, 0x80, 0xff];
    device.write_all(&bytes).unwrap();
    let mut observed = vec![];
    while observed.len() < bytes.len() {
        if let Event::Received { bytes, .. } =
            event(&session, |e| matches!(e, Event::Received { .. }))
        {
            observed.extend(bytes);
        }
    }
    session.stop();
    done(&session).unwrap();
    assert_eq!(observed, bytes);
    assert_eq!(
        std::fs::read(prefix.with_extension("rx.bin")).unwrap(),
        bytes
    );
    let text = std::fs::read_to_string(prefix.with_extension("log")).unwrap();
    assert!(text.contains("RX HEX=00 01 80 FF"));
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    assert!(!text.contains("TX HEX="));
}

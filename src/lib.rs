//! SU-03T updater protocol, reconstructed from the vendor's 2021 DLL.
//! See .harness/docs/protocol.md for protocol details and hardware verification limits.
pub mod capture;
pub mod console_text;
#[cfg(feature = "gui")]
pub mod driver_check;
pub mod firmware;
pub mod flash_event;
pub mod profiles;
pub mod serial_debug;
use anyhow::{Context, Result, bail};
use flash_event::{FlashEvent, RetryReason, Target};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

pub const BLOCK_SIZE: usize = 1024;
pub const ACK: u8 = 0x06;
pub const NAK: u8 = 0x15;
pub const CAN: u8 = 0x18;
pub const EOT: u8 = 0x04;
/// Vendor SPL_DOWNLOAD=1 branches directly with the already-pushed ASCII '2'.
/// See .harness/docs/protocol.md; real hardware received CCC.
pub const STARTUP_TRIGGER: u8 = 0x32;

#[derive(Debug, Clone)]
pub struct Options {
    pub handshake_timeout: Duration,
    pub response_timeout: Duration,
    pub retries: usize,
    /// Default vendor SPL_DOWNLOAD=1 sends ASCII '2' (0x32) every 10ms until CCC.
    pub trigger: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(60),
            response_timeout: Duration::from_secs(5),
            retries: 30,
            trigger: true,
        }
    }
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub fn packet(sequence: u8, payload: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(
        !payload.is_empty() && payload.len() <= BLOCK_SIZE,
        "payload must contain 1..=1024 bytes"
    );
    let mut p = vec![0x1a; BLOCK_SIZE + 5];
    p[..3].copy_from_slice(&[0x02, sequence, !sequence]);
    p[3..3 + payload.len()].copy_from_slice(payload);
    let crc = crc16(&p[3..3 + BLOCK_SIZE]).to_be_bytes();
    p[BLOCK_SIZE + 3..].copy_from_slice(&crc);
    Ok(p)
}

fn byte_until<T: Read + ?Sized>(io: &mut T, deadline: Instant) -> Result<Option<u8>> {
    let mut b = [0];
    while Instant::now() < deadline {
        match io.read(&mut b) {
            Ok(0) => bail!("serial connection closed"),
            Ok(_) => return Ok(Some(b[0])),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e).context("read serial port"),
        }
    }
    Ok(None)
}

/// Requires a transport with a bounded read timeout (CLI uses 100ms).
/// Progress counts firmware bytes acknowledged, never just bytes written.
pub fn send<T: Read + Write + ?Sized>(
    io: &mut T,
    firmware: &[u8],
    options: &Options,
    progress: impl FnMut(usize, usize),
) -> Result<()> {
    send_with_handshake_observer(io, firmware, options, progress, |_| {})
}

/// Observes only bytes received before the startup handshake completes.
/// Keep the observer fast so logging does not delay the serial receiver.
pub fn send_with_handshake_observer<T: Read + Write + ?Sized>(
    io: &mut T,
    firmware: &[u8],
    options: &Options,
    progress: impl FnMut(usize, usize),
    received: impl FnMut(u8),
) -> Result<()> {
    send_observed(io, firmware, options, progress, received, |_| {})
}

/// Events report actual transport boundaries. Writing and waiting for an ACK are
/// separate: a blocked OS write must never be presented as a device timeout.
pub fn send_observed<T: Read + Write + ?Sized>(
    io: &mut T,
    firmware: &[u8],
    options: &Options,
    mut progress: impl FnMut(usize, usize),
    received: impl FnMut(u8),
    mut event: impl FnMut(FlashEvent),
) -> Result<()> {
    anyhow::ensure!(!firmware.is_empty(), "firmware is empty");
    wait_for_handshake_observed(io, options, received, &mut event)?;
    progress(0, firmware.len());
    for (index, payload) in firmware.chunks(BLOCK_SIZE).enumerate() {
        let p = packet((index + 1) as u8, payload)?;
        exchange(
            io,
            &p,
            options,
            Target::Block {
                index: index + 1,
                total: firmware.len().div_ceil(BLOCK_SIZE),
            },
            &mut event,
        )
        .with_context(|| format!("block {} failed", index + 1))?;
        progress(
            ((index + 1) * BLOCK_SIZE).min(firmware.len()),
            firmware.len(),
        );
    }
    exchange(io, &[EOT], options, Target::Eot, &mut event)
        .context("EOT was not acknowledged; transfer completion is unconfirmed")?;
    Ok(())
}

/// Observe startup without sending firmware, EOT, or a guessed erase/reset command.
pub fn wait_for_handshake<T: Read + Write + ?Sized>(
    io: &mut T,
    options: &Options,
    received: impl FnMut(u8),
) -> Result<()> {
    wait_for_handshake_observed(io, options, received, |_| {})
}

pub fn wait_for_handshake_observed<T: Read + Write + ?Sized>(
    io: &mut T,
    options: &Options,
    mut received: impl FnMut(u8),
    mut event: impl FnMut(FlashEvent),
) -> Result<()> {
    event(FlashEvent::HandshakeWaiting {
        timeout_ms: options.handshake_timeout.as_millis().min(u64::MAX as u128) as u64,
    });
    let deadline = Instant::now() + options.handshake_timeout;
    let mut recent = [0u8; 3];
    let mut last_trigger = None::<Instant>;
    loop {
        if Instant::now() >= deadline {
            bail!(
                "waiting for CCC timed out; check B6/B7, baud rate and power-cycle the module after starting"
            );
        }
        if options.trigger && last_trigger.is_none_or(|t| t.elapsed() >= Duration::from_millis(10))
        {
            io.write_all(&[STARTUP_TRIGGER])
                .context("send bootloader trigger")?;
            io.flush()?;
            last_trigger = Some(Instant::now());
        }
        let mut b = [0];
        match io.read(&mut b) {
            Ok(0) => bail!("serial connection closed while waiting for CCC"),
            Ok(_) => {
                received(b[0]);
                recent.rotate_left(1);
                recent[2] = b[0];
                if recent == *b"NNN" {
                    bail!("device returned NNN (vendor skip signal); no firmware was sent");
                }
                if recent == *b"CCC" {
                    event(FlashEvent::HandshakeReceived);
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e).context("read bootloader handshake"),
        }
    }
    Ok(())
}

fn exchange<T: Read + Write + ?Sized>(
    io: &mut T,
    data: &[u8],
    options: &Options,
    target: Target,
    event: &mut impl FnMut(FlashEvent),
) -> Result<()> {
    for retry in 0..=options.retries {
        event(FlashEvent::Writing {
            target,
            retry,
            max_retries: options.retries,
        });
        io.write_all(data).context("write serial port")?;
        io.flush().context("flush serial port")?;
        event(FlashEvent::WaitingAck {
            target,
            retry,
            timeout_ms: options.response_timeout.as_millis().min(u64::MAX as u128) as u64,
        });
        let deadline = Instant::now() + options.response_timeout;
        let mut observed = 0;
        let reason = loop {
            match byte_until(io, deadline)? {
                Some(ACK) => {
                    event(FlashEvent::Acknowledged { target });
                    return Ok(());
                }
                Some(NAK) => break RetryReason::Nak,
                None => break RetryReason::Timeout,
                Some(CAN) => bail!("device cancelled transfer (CAN)"),
                Some(byte) => {
                    // Bound diagnostics for a noisy device; never drop protocol input.
                    if observed < 16 {
                        event(FlashEvent::Received { target, byte });
                    }
                    observed += 1;
                }
            }
        };
        if retry < options.retries {
            event(FlashEvent::Retry {
                target,
                retry: retry + 1,
                max_retries: options.retries,
                reason,
            });
        }
    }
    bail!("retry limit reached without ACK")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Mock {
        input: VecDeque<u8>,
        output: Vec<u8>,
    }
    impl Read for Mock {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            if let Some(v) = self.input.pop_front() {
                b[0] = v;
                Ok(1)
            } else {
                Err(io::ErrorKind::TimedOut.into())
            }
        }
    }
    impl Write for Mock {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn mock(input: &[u8]) -> Mock {
        Mock {
            input: input.iter().copied().collect(),
            output: vec![],
        }
    }
    fn opts() -> Options {
        Options {
            handshake_timeout: Duration::from_millis(10),
            response_timeout: Duration::from_millis(1),
            retries: 1,
            trigger: false,
        }
    }
    #[test]
    fn known_crc_vector() {
        assert_eq!(crc16(b"123456789"), 0x31c3);
    }
    #[test]
    fn packet_layout_and_padding() {
        let p = packet(1, &[0xab]).unwrap();
        assert_eq!(&p[..4], &[2, 1, 254, 0xab]);
        assert_eq!(p.len(), 1029);
        assert!(p[4..1027].iter().all(|&v| v == 0x1a));
        assert_eq!(&p[1027..], &crc16(&p[3..1027]).to_be_bytes());
        assert!(packet(1, &[]).is_err());
        assert!(packet(1, &[0; 1025]).is_err());
    }
    #[test]
    fn nak_retransmits_identical_packet_and_eot() {
        let mut m = mock(b"boot\r\nCCC\x15\x06\x15\x06");
        let mut progress = vec![];
        send(&mut m, b"firmware", &opts(), |d, t| progress.push((d, t))).unwrap();
        let p = packet(1, b"firmware").unwrap();
        assert_eq!(m.output, [p.clone(), p, vec![4, 4]].concat());
        assert_eq!(progress, vec![(0, 8), (8, 8)]);
    }
    #[test]
    fn handshake_observer_excludes_transfer_responses_and_preserves_wire_bytes() {
        let mut m = mock(b"boot\r\nCCC\x06\x06");
        let mut received = vec![];
        send_with_handshake_observer(&mut m, b"x", &opts(), |_, _| {}, |byte| received.push(byte))
            .unwrap();
        assert_eq!(received, b"boot\r\nCCC");
        assert_eq!(m.output, [packet(1, b"x").unwrap(), vec![EOT]].concat());

        let mut m = mock(b"boot\r\nNNN");
        received.clear();
        assert!(
            send_with_handshake_observer(
                &mut m,
                b"x",
                &opts(),
                |_, _| {},
                |byte| received.push(byte),
            )
            .is_err()
        );
        assert_eq!(received, b"boot\r\nNNN");
        assert!(m.output.is_empty());
    }
    #[test]
    fn sequence_wraps_and_no_extra_block_for_exact_size() {
        let mut input = b"CCC".to_vec();
        input.extend(vec![ACK; 258]);
        let mut m = mock(&input);
        send(&mut m, &vec![0x42; 257 * 1024], &opts(), |_, _| {}).unwrap();
        assert_eq!(m.output[254 * 1029 + 1], 255);
        assert_eq!(m.output[255 * 1029 + 1], 0);
        assert_eq!(m.output[256 * 1029 + 1], 1);
        assert_eq!(m.output.len(), 257 * 1029 + 1);
    }
    #[test]
    fn cancel_skip_and_timeout_fail() {
        for input in [b"CCC\x18".as_slice(), b"NNN", b"CCC", b"CC"] {
            let mut m = mock(input);
            assert!(send(&mut m, b"x", &opts(), |_, _| {}).is_err());
        }
    }
    #[test]
    fn progress_waits_for_ack() {
        let mut m = mock(b"CCC\x18");
        let mut progress = vec![];
        assert!(send(&mut m, b"x", &opts(), |d, _| progress.push(d)).is_err());
        assert_eq!(progress, vec![0]);
    }
    #[test]
    fn official_sample_crc_fixtures() {
        // Independently computed from the DLL table by the reverse-engineering pass.
        // Do not make proprietary vendor firmware a required build input.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor/Hummingbird-M-Update-Tool/uni_app_release_update.bin");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        assert_eq!(data.len(), 1_349_024);
        let first = packet(1, &data[..1024]).unwrap();
        let last = packet(0x26, &data[1317 * 1024..]).unwrap();
        assert_eq!(&first[1027..], &[0x28, 0x64]);
        assert_eq!(&last[1027..], &[0xde, 0x39]);
    }
    #[test]
    fn bootloader_responds_only_to_vendor_ascii_2_trigger() {
        // The device stays silent until the exact vendor byte arrives. Preloading
        // CCC would let an incorrect LF trigger appear to establish a handshake.
        struct Bootloader(Mock);
        impl Read for Bootloader {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.0.read(bytes)
            }
        }
        impl Write for Bootloader {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes == [0x32] {
                    self.0.input.extend(b"CCC");
                }
                self.0.write(bytes)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut device = Bootloader(mock(b""));
        let mut options = opts();
        options.trigger = true;
        wait_for_handshake(&mut device, &options, |_| {}).unwrap();
        assert_eq!(device.0.output, [0x32]);
    }
    #[test]
    fn active_trigger_stops_after_ccc_and_eot_needs_ack() {
        let mut m = mock(b"CCC\x06\x06");
        let mut options = opts();
        options.trigger = true;
        send(&mut m, b"x", &options, |_, _| {}).unwrap();
        assert_eq!(
            m.output,
            [vec![0x32], packet(1, b"x").unwrap(), vec![EOT]].concat()
        );
        let mut missing_final_ack = mock(b"CCC\x06");
        assert!(
            send(&mut missing_final_ack, b"x", &opts(), |_, _| {})
                .unwrap_err()
                .to_string()
                .contains("EOT")
        );
    }
    #[test]
    fn startup_probe_never_sends_firmware_or_eot() {
        let mut transport = mock(b"boot\r\nCCC\x06");
        let mut observed = vec![];
        wait_for_handshake(&mut transport, &opts(), |byte| observed.push(byte)).unwrap();
        assert_eq!(observed, b"boot\r\nCCC");
        assert!(transport.output.is_empty());
        let mut transport = mock(b"CCC");
        let mut options = opts();
        options.trigger = true;
        wait_for_handshake(&mut transport, &options, |_| {}).unwrap();
        assert_eq!(transport.output, b"2");
    }
    #[test]
    fn transfer_events_report_writes_ack_waits_retries_and_final_ack() {
        let mut transport = mock(b"CCC\x15\x06\x06");
        let mut events = Vec::new();
        send_observed(
            &mut transport,
            b"firmware",
            &opts(),
            |_, _| {},
            |_| {},
            |event| events.push(event),
        )
        .unwrap();
        let block = Target::Block { index: 1, total: 1 };
        assert_eq!(events[0], FlashEvent::HandshakeWaiting { timeout_ms: 10 });
        assert_eq!(events[1], FlashEvent::HandshakeReceived);
        assert_eq!(
            events[2],
            FlashEvent::Writing {
                target: block,
                retry: 0,
                max_retries: 1
            }
        );
        assert_eq!(
            events[3],
            FlashEvent::WaitingAck {
                target: block,
                retry: 0,
                timeout_ms: 1
            }
        );
        assert_eq!(
            events[4],
            FlashEvent::Retry {
                target: block,
                retry: 1,
                max_retries: 1,
                reason: RetryReason::Nak
            }
        );
        assert!(events.contains(&FlashEvent::Acknowledged { target: block }));
        assert_eq!(
            events.last(),
            Some(&FlashEvent::Acknowledged {
                target: Target::Eot
            })
        );
        let packet = packet(1, b"firmware").unwrap();
        assert_eq!(
            transport.output,
            [packet.clone(), packet, vec![EOT]].concat()
        );
    }
    #[test]
    fn write_failure_cannot_be_reported_as_waiting_for_device_ack() {
        struct Broken(Mock);
        impl Read for Broken {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                self.0.read(b)
            }
        }
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut transport = Broken(mock(b"CCC"));
        let mut events = Vec::new();
        let error = send_observed(
            &mut transport,
            b"firmware",
            &opts(),
            |_, _| {},
            |_| {},
            |event| events.push(event),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("write serial port"));
        assert!(matches!(events.last(), Some(FlashEvent::Writing { .. })));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, FlashEvent::WaitingAck { .. }))
        );
    }
    #[test]
    fn noisy_transfer_input_is_consumed_but_diagnostic_events_are_bounded() {
        let mut input = b"CCC".to_vec();
        input.extend(vec![b'C'; 512]);
        input.extend([ACK, ACK]);
        let mut transport = mock(&input);
        let mut events = Vec::new();
        send_observed(
            &mut transport,
            b"firmware",
            &opts(),
            |_, _| {},
            |_| {},
            |e| events.push(e),
        )
        .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, FlashEvent::Received { .. }))
                .count(),
            16
        );
        assert!(matches!(
            events.last(),
            Some(FlashEvent::Acknowledged {
                target: Target::Eot
            })
        ));
    }
    #[test]
    fn response_timeouts_emit_retries_for_the_same_block() {
        let mut transport = mock(b"CCC");
        let mut events = Vec::new();
        assert!(
            send_observed(
                &mut transport,
                b"firmware",
                &opts(),
                |_, _| {},
                |_| {},
                |e| events.push(e)
            )
            .is_err()
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, FlashEvent::Writing { .. }))
                .count(),
            2
        );
        assert!(events.contains(&FlashEvent::Retry {
            target: Target::Block { index: 1, total: 1 },
            retry: 1,
            max_retries: 1,
            reason: RetryReason::Timeout
        }));
        assert!(!events.iter().any(|e| matches!(
            e,
            FlashEvent::Writing {
                target: Target::Eot,
                ..
            }
        )));
    }
}

# Architecture

| Area | Responsibility |
| --- | --- |
| [main.rs](../../src/main.rs) | Clap entry: default-feature no-argument startup opens the GUI; ports, firmware-info, probe, monitor and flash expose the CLI. |
| [lib.rs](../../src/lib.rs), [flash_event.rs](../../src/flash_event.rs) | CRC, framing, handshake, retries, EOT acknowledgment and language-independent transfer events. |
| [firmware.rs](../../src/firmware.rs) | Bounds and reads the image once, screens filename/known boot-region content, hashes exact bytes and rejects an expected-hash mismatch before opening a port. |
| [gui.rs](../../src/gui.rs) | Power guidance, activity log and phases. A separate CLI process owns flash/probe serial I/O and emits `SU03T_EVENT ` JSON lines; cancellation can terminate it. |
| [serial_debug.rs](../../src/serial_debug.rs), [profiles.rs](../../src/profiles.rs) | Passive debug session, explicit TX, bounded queues and receive-only B8; application presets stay separate from flashing. |
| [debug_ui.rs](../../src/debug_ui.rs), [capture.rs](../../src/capture.rs) | Display/raw RX retention, exports, counters and independent rotating recordings. RX chunks are not protocol frames. |
| [driver_check.rs](../../src/driver_check.rs), [driver_ui.rs](../../src/driver_ui.rs), [port_picker.rs](../../src/port_picker.rs) | Port discovery and async guidance. macOS queries ioreg read-only and associates a USB node with its own serial descendants. |
| [ui_i18n.rs](../../src/ui_i18n.rs), [ui_catalog.rs](../../src/ui_catalog.rs) | Chinese/English selection and text. Device data, paths and hashes preserve their original form. |

## Windows runtime

The optional windows-launcher feature builds su03t-tools.exe, a desktop launcher for the sibling console-capable su03t-update.exe. Desktop and serial-worker launches use CREATE_NO_WINDOW; CLI output remains available when invoking the console binary. Windows uses the WGPU DirectX 12 renderer, APPDATA-based language preferences, the Windows display language and Windows CJK fonts. The MSVC target statically links its CRT. macOS/Linux retain the Glow renderer. [Launcher](../../src/windows_launcher.rs), [Windows resources](../../assets/windows.rc) and [target configuration](../../.cargo/config.toml) define these contracts.

## Firmware transfer journey

1. Desktop selection hashes the image. The CLI worker loads its fixed buffer and verifies that expected SHA before opening the serial port.
2. The host waits for three consecutive C bytes. Active flashing can repeat ASCII 2 (0x32); diagnostic probe defaults to passive. No firmware is sent before CCC; NNN stops the path.
3. Frame each payload according to [protocol](protocol.md). Writing is emitted before write_all/flush; WaitingAck follows only after they return. A blocked write must not be called an expired ACK wait.
4. ACK advances acknowledged image bytes. NAK or timeout retries the cached packet with the same sequence; CAN fails. Residual startup C and unrelated bytes are not ACK.
5. Data can reach 100% before EOT. Success requires final EOT ACK. Missing final ACK leaves completion unconfirmed. Readback, normal boot and voice acceptance remain separate.

## Diagnostic journey and state

A Session owns the debug serial handle and releases it on stop/drop, including under event backpressure. Connect is passive. TX is explicit; B8 rejects sends in the core and disallows flow control that could generate outgoing signals. Serial presets describe parameters and receive/transmit policy; use settings that match the target firmware.

Display and raw RX have separate bounds. Debug display clearing preserves counters, raw RX and recording files. Flash clear_logs clears display rows, eviction/offset bookkeeping and resumes following/jumps to latest; it preserves the live transfer phase, acknowledged/total bytes, target, retry count and failure context. New events still appear. Reading history pauses following; bottom/jump resumes it. Retain those behaviors in both UI languages. [CONTRIBUTING](../../CONTRIBUTING.md) owns the standing constraints.

USB drivers, supplied firmware, module power and bootloader identity are external. The app does not install drivers or control reset through DTR/RTS. Driver readiness does not establish exclusive access, wiring or high-baud reliability. See [testing](testing.md) and [validation](validation.md) for evidence limits.

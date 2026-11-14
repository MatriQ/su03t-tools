# SU-03T Tools

[简体中文](README.md) | English

A native **SU-03T / Hummingbird-M** firmware updater and serial diagnostic tool for macOS and Linux, built in Rust with a desktop UI and CLI.

[![CI](https://github.com/MatriQ/su03t-tools/actions/workflows/check.yml/badge.svg)](https://github.com/MatriQ/su03t-tools/actions/workflows/check.yml) [![MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

![English firmware upgrade interface](docs/images/transfer-en.png)

*Firmware flashing has passed acceptance testing on physical SU-03T hardware. See the [validation record](.harness/docs/validation.md#hardware-acceptance).*

## Features

- **Firmware transfer:** file and SHA-256 checks, startup handshake, acknowledged progress, visible retries and final acknowledgment.
- **Power-cycle guidance:** explicit instructions to power off, wait for serial readiness, power on, keep power stable and restart after completion.
- **Persistent logs:** a full-height right pane with adjustable width and compact spacing. Scroll up to pause following, return to the bottom to resume; clear, copy or save the log.
- **Serial diagnostics:** passive monitoring, HEX / UTF-8 views, explicit manual transmission, raw RX export and rotating recordings.
- **Chinese and English UI:** follow the system language or remember a manual choice. Switching languages preserves the connection, progress and log reading position.

## Installation

On macOS (Apple Silicon / Intel) or Linux (x86_64 / ARM64), run:

```bash
curl -fsSL https://github.com/MatriQ/su03t-tools/releases/latest/download/install.sh | bash
```

The installer selects the latest stable package for your platform and verifies SHA-256. The macOS destination is `~/Applications/SU03T Update.app`. Linux uses `~/.local/share/su03t-tools/vX.Y.Z` with a command link at `~/.local/bin/su03t-update`. The installer prints the launch command; quit an older running instance before opening the new app.

Choose a version or destination:

```bash
curl -fsSL https://github.com/MatriQ/su03t-tools/releases/latest/download/install.sh | bash -s -- --version v0.5.4 --prefix "$HOME/Applications"
```

On macOS, `--prefix` is the directory containing the app bundle. On Linux it is the installation root containing `bin/` and `share/`. You can also download and inspect [install.sh](install.sh) before running it. If the command directory is absent from PATH, follow the installer's output or use the full executable path.

For manual installation, download the macOS universal ZIP or your Linux architecture's `.tar.gz` from [Releases](https://github.com/MatriQ/su03t-tools/releases/latest), with the matching `SHA256SUMS.txt`. The Mac app is ad-hoc signed and not notarized; use the standard macOS approval flow after verifying the source. The installer retains OS security checks.

The macOS deployment target is 12.0. Linux packages are built on Ubuntu 24.04 and need compatible glibc and runtime libraries; build from source for other environments. Debian / Ubuntu desktop runtime packages:

```bash
sudo apt-get install libudev1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 \
  libx11-6 libxcursor1 libxi6 libxrandr2 libgl1 libegl1 fonts-noto-cjk
```

See [validation](.harness/docs/validation.md) for the tested OS, board and driver scope.

## macOS startup driver check

The app checks USB serial devices and their serial interfaces in the background at startup. The driver status entry is shared by upgrade and debug pages, and results are added to the activity log.

- **Drivers ready:** a serial interface exists. Both system and vendor drivers are recognized.
- **USB detected, serial interface missing:** the initial check opens guidance with the chip vendor's official driver link and extension approval instructions. Also check that the device is in serial mode.
- **No adapter detected:** connect the USB serial adapter and choose **Check again**; the module may remain powered off. Driver readiness is unverified until an adapter is detected.
- **Check failed:** inspect the error and retry, or continue using an existing port.

![macOS driver check and installation guidance](docs/images/drivers-missing-en.png)

Common WCH CH34x / CH91xx, Silicon Labs CP210x and FTDI products are identified by USB product IDs. For a device with custom IDs and no serial interface, consult the board manufacturer. Identical adapters are checked separately; a USB hub cannot claim a downstream device's serial port as its own.

The check reads system device information without opening a serial port. Port usage and access permissions are checked when connecting. Module identity, power and actual high-baud transfer performance remain unverified. Driver resources: [WCH's official repository](https://github.com/WCHSoftGroup/ch34xser_macos), [Silicon Labs](https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers), [FTDI](https://ftdichip.com/drivers/vcp-drivers/).

## Wiring and modes

**UART signals use 3.3 V levels. The module typically uses a separate 5 V supply. Do not confuse the two.** With independent module power, leave the adapter VCC disconnected and connect common ground. Switch only module power; keep the USB serial adapter connected to the computer. Check the actual board power requirements and firmware pin mapping.

| Mode | Default settings | Module / USB serial adapter |
| --- | --- | --- |
| Upgrade / startup probe | 921600 / 8N1 / no flow control | B7 → RXD, B6 ← TXD, GND ↔ GND |
| Serial diagnostics (custom mode) | 115200 / 8N1 / no flow control, configurable | Module TX → RXD, module RX ← TXD, GND ↔ GND; pins follow the firmware definition |
| B8 startup logs | 57600 / 8N1 / receive only | B8 → RXD, GND ↔ GND; TXD disconnected |

Match serial settings and pin mapping to the target firmware and adjust them in custom mode. The tool does not install drivers or control module reset through DTR/RTS.

## Upgrading firmware

1. Power off the module. Select its serial port and the manufacturer's `*_update.bin` file. Hover over the filename for its path; advanced settings show SHA-256.
2. Choose **Module is off — start connection**. The tool checks the file before opening the port.
3. When **Power on the module now** appears, connect module power. Keep the USB adapter connected.
4. Transfer begins after the startup handshake. Keep power stable and watch the wait, retry and acknowledgment events in the log.
5. Data progress reaching 100% still requires final device acknowledgment. After completion is confirmed, power-cycle the module and test its wake word and voice commands.

![Power-on prompt](docs/images/power-en.png)

A compatible Hummingbird-M bootloader is required. Active mode repeats `32` hex (ASCII `2`) as the startup trigger; it can be disabled in advanced settings. No firmware is sent before `CCC`. A full `release.bin` image cannot directly replace the serial update file.

### When the first block is not acknowledged

The UI shows elapsed wait time, retry count and remaining time for the current attempt. Defaults are a 5-second response timeout and 30 retries. Repeated timeouts produce a visible warning. You can keep waiting, or stop before checking B6/B7 wiring, common ground, power and the serial connection.

![First-block timeout and retry guidance](docs/images/retry-en.png)

**Clear log affects only the display.** It does not stop the transfer or reset progress, counters or failure context. Incoming logs do not interrupt reading history. Scroll to the bottom or choose **Jump to latest & follow** to resume following. Long lines can be scrolled horizontally.

The **Startup probe · No firmware** entry needs no image and is passive by default; startup triggering must be enabled explicitly. It sends no firmware, EOT, or guessed erase/reset commands.

## Serial monitoring and recording

Connections start passively. Transmission requires an explicit action and supports HEX or UTF-8. No framing or text terminator is automatically added. B8 mode blocks transmission in the communication core and is intended for startup logs.

- The debug display retains up to 2000 records / 1 MiB. The independent RX buffer retains up to 1 MiB. Clearing the display preserves session counters, raw RX and recorded files.
- Copy/save exports the retained display records. RX BIN export saves retained raw bytes. Device output, HEX, paths, filenames and checksums remain unchanged when switching languages.
- Continuous recording creates `.rx.bin` and `.log` streams, each retaining up to four rotating 16 MiB files. A new connection creates new files rather than overwriting other sessions.
- TX counts host writes, not device acknowledgments. Serial read chunks are not protocol frames.

## CLI

Running without arguments opens the desktop app. These commands use the same communication core:

```bash
su03t-update ports
su03t-update firmware-info --firmware firmware_update.bin
su03t-update monitor --port /dev/ttyUSB0 --profile custom --baud 115200
su03t-update monitor --port /dev/ttyUSB0 --profile b8 --duration 60 --record ./session-001
su03t-update probe --port /dev/ttyUSB0 --baud 921600 --wait 15
su03t-update flash --port /dev/ttyUSB0 --firmware firmware_update.bin
```

On macOS, use the enumerated `/dev/cu.*` path. `probe` listens passively unless `--trigger` is supplied. `monitor` transmits only with explicit `--send-hex` or `--send-text` arguments. See `--help` for additional options. Desktop language selection does not change the existing CLI output format.

## Building from source

Use Rust stable and native C build tools; this project was validated with Rust 1.95. Linux also needs libudev and desktop development libraries. For Debian / Ubuntu:

```bash
sudo apt-get install build-essential pkg-config libudev-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libgl1-mesa-dev libegl1-mesa-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev libxinerama-dev fonts-noto-cjk
```

```bash
cargo build --locked --release
cargo build --locked --release --no-default-features  # CLI only
bash scripts/package-macos.sh                       # Native .app
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/package-macos.sh --universal            # Universal macOS .app
bash scripts/package-linux.sh                      # Run on Linux
```

Packaging requires Python 3 and the Rust documentation component (`rustup component add rust-docs`) to include dependency and standard-library notices. The first packaging run fetches missing license files at the dependencies’ recorded upstream commits and caches them under `target/`.

Chinese text requires an available CJK font. Linux serial permissions are managed by the OS; check dialout/uucp membership or device permissions. The application does not open ports using sudo.

## Packaging and automated releases

[Packages and releases CI](https://github.com/MatriQ/su03t-tools/actions/workflows/release.yml) builds all three packages for main commits and pull requests, retaining artifacts for 14 days. A `vX.Y.Z` tag push, or publishing the corresponding GitHub Release, validates the version, runs source checks, builds and verifies the packages, then uploads them with `install.sh` and a combined SHA-256 manifest.

To release a new version, update `Cargo.toml`, `Cargo.lock` and the macOS bundle build number, commit, then push a matching tag. For example:

```bash
git tag -a v0.5.4 -m "SU-03T Tools 0.5.4"
git push origin v0.5.4
```

The Actions workflow also supports manual runs: leave tag empty for artifacts only, or supply an existing tag to publish it. Automation starts with v0.5.3. Tag/version mismatches stop publication. New releases remain drafts until all three platforms and their checksums have been uploaded and verified. Repeating a complete published release verifies its existing assets without replacing them.

## Testing and compatibility

```bash
python3 tests/test_installer.py
python3 tests/test_release_assets.py
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --no-default-features
cargo build --locked
python3 tests/client_acceptance.py target/debug/su03t-update
python3 tests/transfer_events.py target/debug/su03t-update
# The next two suites use the macOS PTY adapter
python3 tests/simulate_device.py target/debug/su03t-update
python3 tests/handshake_diagnostics.py target/debug/su03t-update
```

Tests use virtual serial ports, not attached USB devices. They cover framing, CRC, sequence wrap, retries, cancellation, changed-file rejection, missing final acknowledgment, log clearing and scroll following. Read the [protocol notes](.harness/docs/protocol.md), [validation record](.harness/docs/validation.md) and [contribution guide](CONTRIBUTING.md).

Firmware upgrading remains experimental and is not assumed compatible with other module models. `CCC` does not identify a model. EOT ACK confirms the end of transfer only: **it does not verify flash readback, normal boot or voice functions.** Use firmware and a bootloader known to match your module.

## License and references

Original code is provided under the [MIT License](LICENSE). Vendor firmware, Windows tools and drivers are not included. Third-party dependencies and optional vendor materials retain their own licenses. This is an independent project, not an official vendor client.

[SU-03T documentation](https://docs.aimachip.com/zh-cn/latest/file/offlineVoice/SU03T.html) · [Hummingbird-M FAQ](https://doc.hivoice.cn/udp/%E5%B8%B8%E8%A7%81%E9%97%AE%E9%A2%98.html)

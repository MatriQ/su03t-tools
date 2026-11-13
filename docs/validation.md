# Validation and compatibility

## v0.5.0

| Check | Result |
| --- | --- |
| Rust tests, desktop features | 45 passed |
| Rust tests, CLI-only features | 30 passed |
| Formatting and Clippy with warnings denied | Passed |
| Actual egui clear-button and scroll input tests | Passed |
| ARM macOS bundle: client, handshake, transfer and worker-event PTY suites | Passed |
| Intel macOS bundle under Rosetta: client and worker-event PTY suites | Passed |
| Universal Mach-O slices and ad-hoc signature | Verified |
| New physical-device flash after the v0.5.0 UI changes | Not performed |

The UI tests verify that clearing logs preserves progress and retry state, scrolling into history pauses following, returning to the bottom resumes it, and language changes retain the reading position. Device RX/TX text is preserved when application messages change language.

The transfer suites cover 257 packets, sequence wrap, independent CRC and padding checks, NAK/timeout retries, CAN/NNN failures, missing final acknowledgment, handshake-only probing and rejecting a changed image before opening a port. They use operating-system PTYs and never open attached USB devices.

A prior v0.4.1 session on one physical SU-03T completed the handshake, all block acknowledgments and the final EOT acknowledgment. That is limited evidence for that bootloader and setup. It is not broad board, adapter, driver, OS or voice-function acceptance for v0.5.0. An Intel binary running under Rosetta is not an Intel machine USB test.

CI runs the source test suites on macOS and Linux. Linux GUI startup can also be checked under Xvfb with the supplied container scripts. Physical USB behavior, sustained sessions and voice operation require separate hardware validation.

## Screenshots

README screenshots are rendered by the native desktop app in a debug-only simulation. They contain sample filenames and events, not a hardware flashing session. The application captures its own framebuffer; the screenshot script does not capture the desktop or other applications. Preview and capture controls are excluded from release builds.

## macOS build note

Some Rust/LLVM toolchains produce stripped build-time dylibs that macOS 27 rejects with `mis-aligned LINKEDIT string pool`. The release build override keeps debug information in build tools and disables stripping there. It does not change the firmware protocol or the release optimization level. References: [upstream Rust issue](https://github.com/rust-lang/rust/issues/157750), [Cargo build-dependency profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#build-dependencies).

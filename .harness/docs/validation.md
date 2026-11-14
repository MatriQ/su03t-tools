# Validation and compatibility

## Hardware acceptance

Firmware flashing has passed acceptance testing on physical SU-03T hardware, as confirmed by the maintainer. Automated tests and the versioned results below complement this hardware acceptance.

## v0.6.1

The Windows release adds a separate Windows-x86_64-portable ZIP. A portable.flag file beside the executables selects data/language.json there. The complete folder can be moved without losing settings; ordinary installations continue to use APPDATA. Failed portable writes are reported and never redirect preferences into the user profile.

| Check | Evidence |
| --- | --- |
| Rust desktop/CLI variants, formatting and Clippy | [Source checks passed on all three operating systems](https://github.com/MatriQ/su03t-tools/actions/runs/43589312186) |
| Four platform jobs and both Windows archives | [Package checks passed](https://github.com/MatriQ/su03t-tools/actions/runs/43589888583) |
| Native portable desktop, PowerShell 5.1 and 7 | Chinese and English windows opened and closed normally, before and after moving a private copy between Unicode paths; opposing APPDATA settings were preserved |
| Preference regression tests | Saved language survives relocation; a fresh portable copy ignores profile settings; a blocked data directory does not cause profile writes |
| Release integrity | 18 release-helper scenarios passed, including mandatory portable assets, clean staging and the unchanged v0.5.x/v0.6.0 layouts |

Both Windows ZIP checksums, archive CRCs and marker separation were verified. The portable ZIP contains no saved data directory. Its native screenshots were reviewed in both languages. Package tests use isolated temporary directories and never open a physical serial port. Windows physical-board acceptance remains pending as described below.

## v0.6.0

Windows support targets Windows 10/11 x64, with a console CLI and a separate desktop launcher. Native automated checks run on Windows Server 2022 GitHub runners; Windows 10/11 physical-board flashing has not yet been accepted. The prior SU-03T hardware acceptance above remains valid for the tested setup.

| Check | Evidence |
| --- | --- |
| Rust desktop and CLI variants, formatting and Clippy | [Source CI passed on macOS, Linux and Windows](https://github.com/MatriQ/su03t-tools/actions/runs/43551233437) |
| Windows installer, PowerShell 5.1 and 7 | 13 scenarios passed in each shell: Unicode paths, reinstall, integrity failures, archive safety, ownership, replacement rollback, locked files and uninstall |
| Four release targets | [Package CI passed](https://github.com/MatriQ/su03t-tools/actions/runs/43578174530): macOS universal, Linux x86_64/ARM64 and Windows x64 |
| Extracted Windows executables | Version/help, Unicode firmware paths, independent SHA-256, hidden-worker JSON events and refusal before opening a serial port passed |
| Native Windows desktop | Chinese and English windows launched, responded, produced complete screenshots and closed with exit status zero |

Windows installer checks use temporary directories and fixture downloads. The native desktop checks use isolated language settings and no physical serial device. The Windows executables embed their icon, use a static MSVC runtime, and enable the DirectX 12 renderer explicitly. Unix PTY transfer suites continue to run on macOS and Linux; native Windows GUI and installer passes do not establish physical Windows serial or flashing acceptance. See the [testing guide](testing.md#windows-package-verification) for reproduction and the [release guide](release.md#windows-packaging) for verification of published downloads.

## v0.5.4

Repository metadata, release tooling and canonical installation URLs use `MatriQ/su03t-tools`. The 16 installer scenarios include reinstalling a Linux version with the previous repository marker, migrating it to the new marker, and preserving directories owned by another tool. Existing installation paths allow upgrades after the repository rename.

## v0.5.3

Packaging and installation changes use 14 installer scenarios (11 portable Linux-layout scenarios plus 3 native macOS signing scenarios) and 14 release-helper scenarios. The tests cover version/architecture selection, upgrades, checksum failures, unsafe archive paths, foreign destinations, missing packages, complete manifests, interrupted drafts and immutable published assets. They use temporary prefixes and mock downloads. Release lookup covers published tags, paginated draft discovery and API errors; new drafts must be verified before publication.

The packaging workflow verifies the executable extracted from each archive with the CLI PTY suites and installs each real package into a temporary prefix through mocked downloads. Linux GUI startup is checked for eight seconds under Xvfb with software rendering. The Mac package also verifies both Mach-O slices and its ad-hoc signature after extraction. These checks cover package installation and startup; the physical flashing result is recorded under [hardware acceptance](#hardware-acceptance).

## v0.5.1

The desktop suite contains 59 Rust tests; the CLI-only suite contains 30. This adds 14 driver classification and startup-state tests to the v0.5.0 coverage below.

The macOS startup driver check uses USB device nodes and their own descendant serial interfaces. It accepts system and vendor drivers, distinguishes absent adapters from unbound adapters, and rejects stale results after a serial-device change. Registry errors remain unconfirmed checks, rather than missing-driver diagnoses.

Tests use synthetic XML and binary property lists for system drivers, unbound devices, identical adapters, nested USB devices, duplicate registry entries, custom product IDs and multiport devices. Startup prompt tests cover absent devices, scan errors and unplugging during a check. A read-only check on the development Mac detected the attached WCH `1A86:7523` with its existing serial interface and active vendor driver. No serial port was opened by that check.

Driver-check screenshots use simulated devices and do not establish missing-driver or installation acceptance on another Mac. Physical missing-driver, extension-approval and install/restart flows require separate validation.

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

The UI tests verify that clearing logs preserves progress and retry state, scrolling into history pauses following, returning to the bottom resumes it, and language changes retain the reading position. Device RX/TX text is preserved when application messages change language.

The transfer suites cover 257 packets, sequence wrap, independent CRC and padding checks, NAK/timeout retries, CAN/NNN failures, missing final acknowledgment, handshake-only probing and rejecting a changed image before opening a port. They use operating-system PTYs and never open attached USB devices.

A physical SU-03T session with v0.4.1 completed the handshake, all block acknowledgments and the final EOT acknowledgment. The maintainer has confirmed physical flashing acceptance as recorded above. The Rosetta check exercised the Intel binary on an Apple Silicon host.

CI now runs the source test suites on macOS, Linux and Windows. Linux GUI startup can also be checked under Xvfb with the supplied container scripts. Additional board, adapter and OS combinations can be recorded with their own hardware test results.

## Screenshots

README screenshots illustrate the native desktop interface. The macOS captures use the application framebuffer through the [capture workflow](https://github.com/MatriQ/su03t-tools/blob/main/CONTRIBUTING.md#native-screenshots); preview and capture controls are excluded from release builds. The Windows images are full client-area captures of the packaged application on the native CI desktop, produced by [the GUI check](../../tests/windows_gui_smoke.ps1). Screenshot capture does not operate a serial device or establish new hardware acceptance.

## macOS build note

Some Rust/LLVM toolchains produce stripped build-time dylibs that macOS 27 rejects with `mis-aligned LINKEDIT string pool`. The release build override keeps debug information in build tools and disables stripping there. It does not change the firmware protocol or the release optimization level. References: [upstream Rust issue](https://github.com/rust-lang/rust/issues/157750), [Cargo build-dependency profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#build-dependencies).

# Local development

Run commands from the selected Git root. Compilation needs no service credential, database migration or application .env. An ignored workspace.local.md can bind machine checkouts, but a fresh clone can build without it.

## Prerequisites

| Tool | Source-backed requirement |
| --- | --- |
| Rust/Cargo | CI pins 1.95.0; reproduce CI with that version. See [manifest](../../Cargo.toml), [source CI](../../.github/workflows/check.yml), [rustup](https://rustup.rs/). |
| Native compiler/libraries | macOS uses Xcode command-line tools. Windows uses Visual Studio C++ Build Tools and a Windows SDK with the Rust MSVC toolchain. Linux needs a C toolchain, pkg-config and the [native development libraries](#native-dependencies). See also the [Linux build image](../../scripts/Dockerfile.linux). |
| Python | Python 3.9+ runs stdlib-only tests/helpers (the scripts use removeprefix and modern built-in generics). |
| Node | Node 24+ runs the harness checker, not the Rust application; no npm install is needed. |
| Packaging | Python, rust-docs and platform utilities; see [release](release.md). |
| GitHub CLI | gh reads authorized Actions/Release state. [Official install](https://github.com/cli/cli#installation); reuse credential storage and use gh auth login only when the task needs a new login. |

Inspect before installing tools or changing system configuration:

~~~bash
python3 .harness/resources/probes.py tools
~~~

## Native dependencies

On Debian/Ubuntu, install the C toolchain and native development libraries when absent:

~~~bash
sudo apt-get install build-essential pkg-config libudev-dev libasound2-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libgl1-mesa-dev libegl1-mesa-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev libxinerama-dev fonts-noto-cjk
~~~

On macOS, use Xcode command-line tools. Preserve the Cargo release build-tool strip override for the documented [macOS 27 LINKEDIT issue](validation.md#macos-build-note).

## Building from source

Use CI's Rust 1.95.0 baseline for reproducible builds, with the native dependencies above. From the Git root:

~~~bash
cargo build --locked --release
cargo build --locked --release --no-default-features
~~~

The first command builds the desktop-capable executable; the second builds CLI-only. Both use target/release/su03t-update (su03t-update.exe on Windows) unless CARGO_TARGET_DIR is overridden, so build the intended variant before running it. Start the desktop build without arguments in an active desktop session, or pass --help to inspect CLI commands. Chinese rendering needs an available CJK font; Linux port access uses the system's dialout/uucp/device permissions, without running the app as root.

For native app bundles and archives, follow [local packaging](release.md#local-packaging). Packaging also requires Python 3 and rust-docs; the platform-specific steps belong to that guide. For a faster development-loop check, use the debug startup below.

## Hardware-free startup

~~~bash
cargo build --locked --no-default-features
cargo run --locked --no-default-features -- --version
cargo run --locked --no-default-features -- --help
~~~

Expected: version matches Cargo.toml; help lists CLI subcommands. These commands do not open serial ports. The first compile needs dependency registry access unless cached. Omit --no-default-features to build the desktop-capable executable. Default-feature no-argument startup requires an active desktop session and opens a window; close it normally afterward. CLI-only no-argument startup is expected to request a subcommand and fail.

## Synthetic file check

~~~bash
SU03T_SMOKE_DIR=$(mktemp -d)
trap 'rm -rf "$SU03T_SMOKE_DIR"' EXIT
printf 'harness smoke\n' > "$SU03T_SMOKE_DIR/smoke_update.bin"
cargo run --locked --no-default-features -- firmware-info --firmware "$SU03T_SMOKE_DIR/smoke_update.bin"
~~~

Expected: byte count and SHA-256 followed by the compatibility screening limit. The fixture is for this file-only command, never a board image. The EXIT trap removes it. After building, an independent machine-readable probe checks the version and fixture hash:

~~~bash
python3 .harness/resources/probes.py cli
~~~

If CARGO_TARGET_DIR is set, the probe uses its debug executable. A zero exit and accepted:true establish this CLI path only. For UI work use default-feature checks in [testing](testing.md); use [simulated screenshots](../../CONTRIBUTING.md#native-screenshots) only with an active desktop. Keep a CJK font available.

Stop task-created processes, close PTYs and remove fixtures after checks. Keep caches in target/ and packages in dist/. Firmware, vendor executables, .env and private evidence stay excluded by [.gitignore](../../.gitignore). Do not run the application with sudo.

## Firmware studio audio

The [firmware studio](firmware/studio.md) uses CPAL for explicit native capture/playback and optional external FFmpeg with libmp3lame for firmware MP3 conversion/decoding. FFmpeg is not needed to build or launch the app; the studio shows conversion errors and lets users select its executable. It is not bundled or installed automatically. Linux GUI builds additionally require libasound2-dev; the distributed GUI needs the corresponding ALSA runtime. macOS app packaging declares microphone usage in English and Simplified Chinese. OS permission/capture is requested only by the user's Start recording action. Automated checks use synthetic audio and never operate the microphone/speakers.

## Repository helper scripts

Installed applications do not depend on the repository's `scripts/` directory. Source packaging, release automation, screenshots and optional asset/reference maintenance still use these helpers:

| Helpers | Purpose / caller |
| --- | --- |
| [package-macos.sh](../../scripts/package-macos.sh), [package-linux.sh](../../scripts/package-linux.sh), [package-windows.ps1](../../scripts/package-windows.ps1) | Platform archives, invoked by package CI and local release commands |
| [collect-licenses.py](../../scripts/collect-licenses.py) | Dependency notices, invoked by all platform packagers |
| [release_assets.py](../../scripts/release_assets.py) | Release metadata, checksums, archive checks and publication; covered by release-helper tests |
| [capture-ui.py](../../scripts/capture-ui.py) | Synthetic bilingual UI screenshots, including B8 logs; invoked explicitly by the capture workflow |
| [Dockerfile.linux](../../scripts/Dockerfile.linux) | Optional Linux build/test environment with native development libraries; the installed application does not require Docker |
| [create-icon.swift](../../scripts/create-icon.swift) | Editable source of the checked-in icon PNG set; run manually only when regenerating artwork |
| [fetch-vendor.sh](../../scripts/fetch-vendor.sh), [analyze_vendor.py](../../scripts/analyze_vendor.py) | Optional acquisition/static verification of historical vendor references and CRC/image relations; never run Windows vendor code and are not build/test prerequisites |

The obsolete `verify-linux.sh` wrapper was removed: no current workflow invokes it, its hard-coded container paths require undocumented setup, and maintained source/package CI already performs its checks. Use the commands in [testing](testing.md) and [release](release.md) instead. Optional maintenance tools are retained because they preserve artwork/reference reproducibility, rather than being application runtime dependencies. No vendor downloads or artwork regeneration is triggered by ordinary compilation.

B8 README screenshots and the detailed guide use hosted repository URLs so the README copies inside release archives do not point at unbundled engineering-document assets.

## Windows development

Use Rust 1.95 with the x86_64-pc-windows-msvc target, Visual Studio 2022 C++ Build Tools and a Windows 10/11 SDK. The GUI uses WGPU's DirectX 12 backend. The target configuration statically links the CRT. Native GUI work needs an active Windows desktop; source checks alone do not exercise graphics initialization.

Build with cargo build --locked --release --target x86_64-pc-windows-msvc --features windows-launcher. Keep the resulting su03t-tools.exe desktop launcher beside su03t-update.exe in target/x86_64-pc-windows-msvc/release. Invoke su03t-update.exe for console commands. Language preferences live in APPDATA/SU03T Tools/language.json. A portable.flag file beside the executable selects data/language.json in that same directory, independently of the working directory or user profile. Portable settings move with the complete folder and save failures remain visible instead of falling back to APPDATA; Windows display language and CJK fonts provide the system defaults.

[Windows packaging](release.md#windows-packaging) includes a user-scoped installer. [Native Windows checks](testing.md#windows-package-verification) use local fixtures and isolated application settings, never an attached board. COM port names come from Device Manager; driver installation remains a vendor/OS operation.

# Local development

Run commands from the selected Git root. Compilation needs no service credential, database migration or application .env. An ignored workspace.local.md can bind machine checkouts, but a fresh clone can build without it.

## Prerequisites

| Tool | Source-backed requirement |
| --- | --- |
| Rust/Cargo | CI pins 1.95.0; reproduce CI with that version. See [manifest](../../Cargo.toml), [source CI](../../.github/workflows/check.yml), [rustup](https://rustup.rs/). |
| Native compiler/libraries | macOS uses Xcode command-line tools. Linux needs a C toolchain, pkg-config and the [native development libraries](#native-dependencies). See also the [Linux build image](../../scripts/Dockerfile.linux). |
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
sudo apt-get install build-essential pkg-config libudev-dev libxkbcommon-dev \
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

The first command builds the desktop-capable executable; the second builds CLI-only. Both use target/release/su03t-update unless CARGO_TARGET_DIR is overridden, so build the intended variant before running it. Start the desktop build without arguments in an active desktop session, or pass --help to inspect CLI commands. Chinese rendering needs an available CJK font; Linux port access uses the system's dialout/uucp/device permissions, without running the app as root.

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

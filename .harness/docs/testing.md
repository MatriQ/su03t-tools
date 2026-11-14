# Verification

Choose checks by affected behavior, from the Git root. Listed unit, PTY and installer suites create synthetic inputs and must not open attached USB hardware.

| Change | Relevant evidence |
| --- | --- |
| Harness/docs | Portable checker, links, source freshness and detached fresh-clone check. A move also needs every caller and archive destination checked. |
| Installer/release | Bash syntax, installer/publisher cases and a real archive installed into a temporary prefix when packaging changes. |
| Transfer/firmware/events | Both Rust variants, client/events PTYs, and macOS simulation/handshake suites on macOS; retain independent CRC/padding assertions. |
| UI/i18n/log/driver | Default-feature tests, fmt/Clippy, relevant PTYs, simulated screenshots for changed rendering. Driver parsing tests use synthetic property lists. |
| Debug/B8/capture/disconnect | Rust serial tests plus client PTYs; preserve raw bytes and bounded cancellation under event backpressure. |
| Packages | Extraction, version, architecture/signature, temporary installation and migrated document presence in package CI. |

The Clear-button regression tests are clear_during_retry_retains_progress_failure_context_and_new_events and actual_clear_button_preserves_live_transfer_state in [gui.rs](../../src/gui.rs); native_scroll_widget_pauses_on_history_and_resumes_at_bottom covers scroll following. Start with these default-feature tests for that behavior, then run the contribution/CI checks required for the wider change.

## Commands

When changing the installer, also run Bash syntax validation: bash -n install.sh.

~~~bash
node .harness/runtime/check.mjs --workspace .
python3 tests/test_installer.py
python3 tests/test_release_assets.py
cargo fmt --all --check
cargo test --locked
cargo test --locked --no-default-features
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
python3 tests/client_acceptance.py target/debug/su03t-update
python3 tests/transfer_events.py target/debug/su03t-update
~~~

Build the intended variant before invoking its binary: CLI-only and default builds can replace the same debug path. If CARGO_TARGET_DIR is overridden, pass that location's executable to the PTY scripts. On macOS also run:

~~~bash
python3 tests/simulate_device.py target/debug/su03t-update
python3 tests/handshake_diagnostics.py target/debug/su03t-update
~~~

The last two explicitly use the macOS-only --virtual-port adapter. Portable client/events suites select the correct adapter. It is not a workaround for an actual device.

## Coverage and compatibility

Unit and PTY suites cover packet framing, CRC, padding, sequence wrap, retries, cancellation, changed-file rejection before serial open, missing final acknowledgment, display clearing and scroll following. They use synthetic input and newly created virtual serial ports. Read the [protocol](protocol.md), [hardware acceptance and historical validation](validation.md), and [contribution guide](../../CONTRIBUTING.md) for the corresponding contracts and acceptance scope.

## CI and reporting

[Source CI](../../.github/workflows/check.yml) covers macOS/Ubuntu. [Package CI](../../.github/workflows/release.yml) covers Mac universal and Ubuntu x86_64/ARM64; it verifies real archives with PTYs and temporary installation, plus eight-second Xvfb GUI startup on Linux. The separate [harness gate](../../.github/workflows/harness-gate.yml) runs Node 24 artifact checks without this machine's local context.

PTY passes establish modeled behavior. Xvfb survival establishes startup, not a full GUI session. Final EOT ACK establishes transfer completion, not readback or voice functions. See [hardware acceptance and historical validation](validation.md#hardware-acceptance) for the physical-device results.

Report revision/working tree, OS/architecture, variant, commands, outcomes and unresolved limits. Preserve first failures and retry reasons. Machine execution reports and raw logs stay ignored locally. Do not relax tests to accept unacknowledged transfer or premature GUI exit.

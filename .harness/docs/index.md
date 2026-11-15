# Engineering guide

SU-03T Tools is one Rust package, `su03t-update`, with an eframe/egui desktop UI and a CLI sharing the communication core. Release targets are macOS universal, Linux x86_64/ARM64 and Windows x64. This harness covers the current repository; member `tools` resolves through the [local-context reader](../runtime/local-context.mjs).

## Task routes

| Task | Guide or workflow | Main implementation |
| --- | --- | --- |
| Implement a change | [Engineering](../skills/engineering/SKILL.md), [architecture](architecture.md) | [CLI](../../src/main.rs), [UI](../../src/gui.rs), [core](../../src/lib.rs) |
| Prepare or start locally | [Local development](../skills/local-dev/SKILL.md), [development](development.md) | [Cargo manifest](../../Cargo.toml) |
| Select checks | [Verify](../skills/verify/SKILL.md), [testing](testing.md) | [source CI](../../.github/workflows/check.yml), [PTY acceptance](../../tests/client_acceptance.py) |
| Prepare, publish or inspect a release | [Release workflow](../skills/release/SKILL.md), [release guide](release.md), [Homebrew distribution](release.md#homebrew-distribution), [Windows packaging](release.md#windows-packaging) | [package CI](../../.github/workflows/release.yml), [publisher](../../scripts/release_assets.py), [Unix installer](../../install.sh), [Windows installer](../../install.ps1) |
| Diagnose no ACK after CCC | [Troubleshooting](troubleshooting.md#ccc-arrives-but-block-1-is-not-acknowledged), [protocol](protocol.md) | [core](../../src/lib.rs), [events](../../src/flash_event.rs) |
| Investigate B8, captures or drivers | [Troubleshooting](troubleshooting.md), [architecture](architecture.md) | [serial session](../../src/serial_debug.rs), [driver scan](../../src/driver_check.rs) |
| Refresh the harness | [Maintenance](maintenance.md) | [specification](../harness.json), [registry](../workspace.json) |

Read the [required engineering rules](../rules/engineering.md) and [contribution guide](../../CONTRIBUTING.md) before changes. Procedures are reached through AGENTS.md and its CLAUDE.md alias; they are not registered native commands, auto-discovered Skills or custom agents.

## Task coverage

| Question | Answer and evidence route | Remaining boundary |
| --- | --- | --- |
| Does CCC identify the model? | [Protocol](protocol.md): startup handshake only. | SU-03T flashing acceptance is recorded; additional target configurations have their own acceptance. |
| Why can block 1 wait about 155 seconds? | [Troubleshooting](troubleshooting.md#ccc-arrives-but-block-1-is-not-acknowledged): 31 attempts at five seconds each. | A blocked write is a separate phase. |
| Does 100% mean success? | [Transfer journey](architecture.md#firmware-transfer-journey): acknowledged data precedes EOT ACK. | Readback, normal boot and voice behavior remain separate. |
| Which checks avoid attached hardware? | [Testing](testing.md): unit tests and PTYs create their own fixtures. | Physical commands need a selected target and task scope. |
| Can an older Linux installation be reused? | [Installation and recovery](release.md#installation-and-recovery): both repository markers are accepted. | Foreign destinations remain refused. |
| Do files prove host activation? | [Maintenance](maintenance.md#activation-and-evidence): test loading separately. | Readiness does not transfer with a clone. |

The migrated [protocol](protocol.md) owns framing, and the [validation record](validation.md#hardware-acceptance) records passed SU-03T hardware flashing acceptance and historical test results. README files and CONTRIBUTING.md remain at the root by the user's choice. New AI-authored project prose belongs here. Machine paths, credentials, readiness and raw execution reports belong in ignored local state.

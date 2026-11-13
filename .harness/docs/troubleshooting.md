# Troubleshooting

Collect version, OS/architecture, adapter, selected baud/profile and a short sanitized elapsed log with its exact failure stage. Preserve event order and original bytes. Do not request proprietary firmware, credentials or an entire private log when a sanitized excerpt/hash suffices.

## CCC arrives but block 1 is not acknowledged

CCC establishes startup handshake, not a model or reliable full-duplex high-baud connection. Inspect the last worker event:

| Last phase | Interpretation and next branch |
| --- | --- |
| HandshakeWaiting | No complete CCC. Review module power sequence, return wire, baud and bounded startup RX summary. |
| Writing | write_all/flush has not returned. Investigate serial/driver blocking and cancellation; do not call it an ACK timeout. |
| WaitingAck then Retry Timeout | Host write returned; no timely ACK. Preserve elapsed/retry context, then check compatibility, B6/B7, ground, power and adapter. |
| Received other bytes | Input is not necessarily an ACK; keep bounded HEX, including residual C. |
| Failed block 1 / retry limit | Attempts exhausted. Stop, retain failure context and verify the actual setup before reflashing. |

Default response timeout is five seconds and retries are 30 after the first attempt: approximately 155 seconds for one unacknowledged exchange, plus write/log overhead. Changing the timeout does not prove reliable transmission. Clear log changes display only. WaitingAck/Retry events distinguish this from a stalled OS write.

Follow [exchange/handshake](../../src/lib.rs), [worker events](../../src/main.rs), [schema](../../src/flash_event.rs) and [UI mapping](../../src/gui.rs). [Protocol](protocol.md) owns framing/completion. [transfer_events.py](../../tests/transfer_events.py) covers missing initial/final ACK on PTYs.

## Power, drivers and ports

Start the host while the module is off, then power the module when prompted. Keep the USB adapter connected and switch module power. Use [board-specific wiring](../../README.en.md#wiring-and-modes): UART signals are 3.3 V and a separate module supply is typically 5 V; keep common ground and do not confuse signal/supply voltage.

macOS reads ioreg and device-path metadata. USB present without a descendant serial interface differs from no adapter or an inconclusive scan. Driver readiness does not prove a free port. Open failures branch to device path/unplugging, competing programs, and Linux dialout/uucp/device permissions. Do not automatically install drivers, change groups, reset boards or open arbitrary devices during investigation.

Even passive probe opens its selected port and requires task scope. It sends no firmware/EOT/guessed erase/reset; startup triggering is explicit. Use PTYs for unattended code diagnosis. B8 remains receive-only in the core.

## Completion and captures

100% data progress precedes final EOT ACK. Missing final ACK leaves completion unconfirmed. Normal boot and voice acceptance need separate known-board checks after power cycling.

Debug display clearing retains counters, raw RX and recordings; flash clearing retains progress/retry context. History reading pauses following. Recordings are .rx.bin/.log streams, each keeping four rotating 16 MiB files; new sessions need new prefixes. Inspect [capture](../../src/capture.rs), [session](../../src/serial_debug.rs) and [debug UI](../../src/debug_ui.rs) for independent buffers.

## Build and publication

macOS 27 LINKEDIT failures follow the documented [build override](validation.md#macos-build-note). Linux loader failure requires archive architecture/runtime checks. Publication follows [release](release.md); an asset checksum mismatch must stop replacement.

Operational evidence is the local session log/capture, CLI outcome and GitHub run/release; there is no production metrics service or queue. Escalate a sanitized reproducible issue with exact identifiers and separate modeled/physical evidence. No specific on-call owner or SLA has been established.

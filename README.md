# SU-03T Tools

Native macOS and Linux firmware transfer and serial diagnostics for compatible
SU-03T / Hummingbird-M modules. This import contains the working v0.4.1 app.

Build the desktop app with `cargo build --locked --release`, or use
`--no-default-features` for the CLI. Run without arguments to open the app.
`ports`, `firmware-info`, `probe`, `flash` and `monitor` provide the CLI workflows.

Use 3.3 V UART signals, common ground and separate module power. Start the host
before powering on the module. Upgrading requires a compatible bootloader and
serial `*_update.bin`; it does not perform flash readback or voice validation.

See [protocol notes](docs/protocol.md) for packet framing and retry behavior.
Original code is licensed under [MIT](LICENSE). Vendor firmware and tools are
not included.

# Serial upgrade protocol

The Rust implementation follows the default single-image serial path of the Hummingbird-M updater. It is intended for a compatible SU-03T bootloader. The original Rust code is MIT licensed; vendor executables and firmware are not included.

## Startup

Use 921600 baud, 8 data bits, no parity, one stop bit and no flow control. Start the host before powering on the module. In active mode, the host repeats ASCII `2` (`32` hex) at approximately 10 ms intervals until it receives three consecutive `C` bytes (`43 43 43`). Passive mode only listens.

`CCC` is a startup handshake, not a model identifier or a compatibility check. `NNN` is treated as a skip/failure response; no image is sent. A bounded startup timeout prevents waiting forever.

## Data blocks

| Field | Size | Value |
| --- | --- | --- |
| Start | 1 byte | `02` / STX |
| Sequence | 1 byte | Starts at 1; wraps `FF` → `00` |
| Complement | 1 byte | Bitwise complement of sequence |
| Payload | 1024 bytes | Image bytes; final block padded with `1A` |
| CRC | 2 bytes | CRC-16/XMODEM, high byte first |

CRC uses polynomial `1021`, initial value `0000`, with the complete padded payload as input. The test vector `123456789` produces `31C3`. Each packet is 1029 bytes. The image is transmitted as supplied; the tool does not strip a boot region or repackage its contents.

- `06` / ACK accepts a block and advances acknowledged progress.
- `15` / NAK resends the same cached packet with the same sequence number.
- A response timeout also retries the same packet.
- `18` / CAN cancels the transfer.
- Residual startup `C` bytes and other data are not ACKs.

Default response timeout: 5 seconds. Default retry limit: 30 retries after the first attempt. A block without any acknowledgment can therefore take about 155 seconds to fail.

## Completion

After the final data block is acknowledged, the host sends EOT (`04`) and requires an ACK. EOT is retried on NAK or timeout. A missing final ACK leaves completion unconfirmed, even when the data progress is 100%.

An acknowledged EOT confirms the end of transfer only. This implementation does not perform flash readback, identify the running firmware, or verify voice functions. Power-cycle the module and test its application separately.

## Checks and validation

A selected file is read into a fixed memory buffer and hashed before the serial port opens. The desktop client passes the selected SHA-256 to its worker, which refuses a changed file. Filename, size and known boot-region checks are screening checks, not firmware authentication or proof of compatibility.

Unit tests check framing, CRC, padding, sequence wrap and retry identity. PTY tests execute the real CLI with an independent Python CRC reference. Tests can run without vendor artifacts; an optional vendor CRC fixture is checked only when supplied locally under the ignored `vendor/` directory.

# Firmware logging controls

The Studio imports an existing BIN and edits its authenticated **application/common runtime logger startup gate**. These changes affect exported firmware bytes and are separate from clearing/filtering the desktop log view. Six known profile families support both full and update forms. Changes remain experimental, with device boot/output acceptance pending.

## Usage and scope

Open a supported BIN, open the **Firmware logging** pane and toggle **Enable** on the application runtime row. The state is the configured On/Off value; it is not a live device observation. **Undo logging edits** restores that setting to the image-import snapshot while preserving audio replacements. Audio undo likewise preserves logging edits. Export writes one new update BIN and adjacent manifest containing both edit types. Reopening the output permits further logging/audio edits. Originals and existing output paths are never overwritten.

| Channel | State/control | Boundary |
| --- | --- | --- |
| Application/common runtime logger | Known On/Off, editable only after profile and audio validation | `runtime.log.enable` ASCII `0/1` gates startup initialization; the retained `log.set_level=2` still filters initialized output |
| Engine independent log level | Raw engine `log` value displayed; effective independent state unknown and control unsupported | The actual level setter in every admitted sample discards its argument; changing `2/6` is not an established engine switch |
| Bootloader | Unknown/unsupported | No verified bootloader logging setting is patched; full-image boot bytes remain immutable |
| ARPT and direct printf | Outside this switch | `log.arpt_enable` is separately initialized and stays unchanged; direct printf/OsalPrintf and startup diagnostics may still print |

Some engine output uses the shared `OsalLogDump` callback and therefore follows the common runtime gate. This does not establish independent engine-level control or complete UART silence. Unknown/damaged images show unknown state and cannot be modified merely because they contain a field named `log`.

File-only CLI (no serial/audio devices):

```sh
su03t-update firmware-logging --firmware input.bin
su03t-update firmware-logging --firmware input.bin --runtime on --output log_on_update.bin
su03t-update firmware-logging --firmware input.bin --runtime off --output log_off_update.bin
```

The first command prints logging-only JSON. Apply audio changes before or after these commands using `firmware-replace`; both edit types remain recognized. Command plans require compatible compiled recognition resources and block BIN export when pending.

## Authentication and export

[Shared profiles](../../../src/firmware_profile.rs) authenticate audio and runtime logging together. For each exact full/update profile, the single allowlisted runtime byte must be ASCII `0` or `1` and is restored to that profile's observed original value before comparison with its existing immutable digest. Audio blob/directory-length masking remains bounded by the original profile. Every other byte, including JSON keys/whitespace, engine log, runtime level, ARPT setting, code, models, boot bytes and directory IDs/offsets, stays authenticated. Invalid values or unrelated changes cannot be normalized away. The MP3 directory/slots must additionally parse before logging controls are granted.

Update-file runtime byte offsets (full offsets are explicitly recorded separately, not guessed for unknown images):

| Profile | `runtime.log.enable` | Original |
| --- | ---: | ---: |
| official-release | `0x13a749` | 0 |
| official-debug | `0x13a749` | 1 |
| platform-a | `0x11de39` | 0 |
| platform-b | `0x123b01` | 0 |
| latest-factory | `0x13d741` | 0 |
| latest-wake-switch | `0x1335a1` | 0 |

[Logging adapter](../../../src/firmware_logging.rs), [session/export](../../../src/firmware_studio.rs) and [native UI](../../../src/firmware_ui.rs) share this authentication. Only one digit changes for a runtime-only edit. No JSON reserialization, offsets, lengths or file sizes change. Manifest schema 2 adds `logging`, `logging_changes` (kind/path, old/new configured booleans and numeric values, source/update offsets, length) and retains exact combined changed ranges, source/output hashes and changed audio IDs. Experimental/device-unverified flags remain. Both output files use the existing non-overwriting/rollback path.

## Evidence and checks

The optional official historical SDK source `src/app/src/main.c:104–121` reads `log.enable`, skips initialization for zero, otherwise initializes the common context and applies `log.set_level`. `src/utils/log/src/uni_log.c` checks that context and threshold in `LogLevelValid`/`OsalLogDump`. Static NDS32 inspection of all six final update images corroborated this gate and shared callback; corresponding full/update application bytes were independently identical.

The SDK `osal-log.h` declares `LOG_DEBUG=2` and `LOG_NONE=6`, and the engine parser reads its top-level `log` and calls `OsalLogSetLevel`. However, the linked setter in every inspected final image is a short argument-discarding diagnostic printf stub with no level-state write. This contradicts treating the header enum alone as evidence of a working switch. Engine configuration is deliberately immutable. Static analysis is not runtime/board acceptance.

Default synthetic tests exercise value normalization and unknown-image refusal. The optional session integration test covers original states, both edit orders, independent undo, reopen/export, manifests and refusal of field/code/model/engine corruption. [File acceptance](../../../tests/firmware_logging_acceptance.py) explicitly selects private fixtures, exercises On/Off for every supplied image, checks exact permitted byte differences, retains source bytes and reopens combined audio/log output. It prints profile/count/hash summaries, never recovered phrases.

```sh
SU03T_TEST_FIRMWARE=/local/known-profile.bin cargo test --locked \
  --test firmware_studio -- --ignored
python3 tests/firmware_logging_acceptance.py target/debug/su03t-update \
  /local/known-profile.bin /local/known-profile_update.bin
```

Microphone/speaker behavior and edited firmware boot/logging/recognition/output on a selected board remain manual acceptance, separate from host software checks.

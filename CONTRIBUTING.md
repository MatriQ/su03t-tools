# Contributing

Issues and pull requests are welcome in English or Chinese. See the [English README](README.en.md) or [中文说明](README.md) for setup and wiring.

## Development

Use Rust 1.95 and the platform libraries listed in the [development guide](.harness/docs/development.md#native-dependencies). Run formatting, Clippy, the default-feature and CLI-only test suites, and the PTY acceptance scripts before submitting a change. CI checks macOS, Linux and Windows x64. Windows uses the file-only CLI and PowerShell installer suites; PTY scripts run on Unix platforms. Tests must not silently open attached USB devices or send firmware to hardware.

Keep firmware transfer behavior separate from application presets. Progress must count acknowledged bytes; 100% data progress must not imply final acknowledgment. Changes to framing, CRC, retry behavior or completion handling need wire-level regression evidence. Keep receive-only B8 mode enforced in the communication core.

UI changes should retain Chinese and English text, stable log reading positions and the distinction between a serial write and an acknowledgment wait. Clearing logs must preserve transfer state and session counters. Original device output, paths and checksums must not be translated.

## Native screenshots

With an active desktop session, run:

```bash
python3 scripts/capture-ui.py
```

The script builds a debug executable and captures the app's own rendered framebuffer in Chinese and English for transfer, power-on and retry states. It uses simulated events, cannot open physical serial ports, and leaves the saved language preference unchanged. PNGs are written to `docs/images/`. Preview/capture code is excluded from release builds. Review the images before committing them; do not use screenshots from a live session containing private paths or device data.

## Reporting a bug

Include the app version, OS, adapter model, baud rate, failure stage and a short sanitized log. Say whether the same setup works with the vendor tool and whether the device acknowledged the transfer or its voice functions were tested. Remove personal paths, serial numbers and any credentials before attaching logs. Do not upload proprietary firmware or vendor executables.

## Commits and licensing

Keep commits focused on a behavior or a fix, with a short imperative subject and a brief explanation where needed. State what was tested. Use your own Git identity; there is no need to rewrite authors or backdate work.

Contributions to the original implementation are licensed under the project's MIT license. Include provenance and license information for any third-party material.

## Releases

Packaging runs on the current standard macOS runner, Ubuntu 24.04 x86_64 and Ubuntu 24.04 ARM64. The workflow builds artifacts for main and pull requests. Tag pushes and published release events also run the source checks and automatically attach the complete package set to the release. It requires only the repository's built-in `GITHUB_TOKEN`; write permission is isolated to the publish job.

Set a stable X.Y.Z version in Cargo.toml/Cargo.lock, increase the Mac bundle build number, and push a matching annotated vX.Y.Z tag. Check the source, package and harness workflows. New releases are uploaded as drafts and published after verification; a retry can finish an incomplete draft. Publication retries up to four times, with 20 seconds between attempts, to allow a newly created draft to become visible to GitHub's read endpoints. Published assets are kept immutable. If a published file differs, use a new version instead of replacing it.

Run `python3 tests/test_installer.py` and `python3 tests/test_release_assets.py` when changing installation or release behavior. These use temporary prefixes, synthetic packages and mock downloads. They never install into the real user's Applications or .local directories. Linux archives target Ubuntu 24.04 runtime compatibility; broad distro or physical hardware acceptance requires separate tests.

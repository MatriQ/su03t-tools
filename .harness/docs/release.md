# Packaging and release

## Targets

Confirm [MatriQ/su03t-tools](https://github.com/MatriQ/su03t-tools) before a gh operation. CI source checks use macOS, Ubuntu and Windows x64; package runners are macos-latest, ubuntu-24.04, ubuntu-24.04-arm and windows-2022. The universal Mac app contains arm64/x86_64 slices, targets macOS 12.0, and is ad-hoc signed, not notarized. Linux archives need Ubuntu 24.04-compatible glibc/native libraries. Windows packages target Windows 10/11 x64, contain desktop and console executables and retain system download protections. Native CI and physical-board acceptance remain distinct.

Environments are the local checkout, GitHub-hosted CI, public Releases, and user desktops with explicitly selected adapters/boards. No backend, database, cloud region or hosted application service is configured. Hardware acceptance has its own board/firmware/power/wiring evidence.

## Pipeline

[release.yml](../../.github/workflows/release.yml) builds main/PR artifacts retained for 14 days. Matching version tags, published Release events, or manual runs with an existing tag enable publication. A manual run without a tag builds only. Automation is available for v0.5.3 and later tags. Metadata requires tag/version and checkout/tag commit equality.

1. Update Cargo.toml/Cargo.lock version and Mac CFBundleVersion in [package-macos.sh](../../scripts/package-macos.sh). Update the version-bound harness CLI probe expectation, then refresh affected source/document reviews through [maintenance](maintenance.md).
2. Run [checks](testing.md), review the diff and follow the current task's publication scope. Push an annotated matching vX.Y.Z tag. Do not retag an old release to hide a fix.
3. Source quality and all four packages must pass. Extracted binaries, signature/architecture, PTYs and temporary-prefix installs are checked before publish.
4. [release_assets.py](../../scripts/release_assets.py) verifies partial hashes, adds install.sh and a unified manifest, uploads a draft, verifies it, then publishes. Only publish has contents:write; CI uses the built-in GITHUB_TOKEN.
5. Publication retries four times with 20 seconds between attempts, allowing draft read visibility to catch up. Incomplete drafts can resume. Complete published assets are verified without replacement; changed released bytes need a new version.

From v0.6.0, assets are four platform archives, install.sh, install.ps1 and SU03T-Tools-vX.Y.Z-SHA256SUMS.txt. Platform suffixes identify archives; the repository has no OS suffix.

## Local packaging

Complete [source-build prerequisites](development.md#native-dependencies) first. Packaging needs Python 3 and the Rust documentation component to collect dependency and standard-library licenses:

~~~bash
rustup component add rust-docs
~~~

The first packaging run fetches missing license files from the dependencies' recorded upstream commits and caches them under target/.

### macOS

Build a native app bundle for the current Mac:

~~~bash
bash scripts/package-macos.sh
~~~

For a universal app, install both Rust targets and package both slices:

~~~bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/package-macos.sh --universal
~~~

The app is written to dist/SU03T Update.app; universal mode also creates the release ZIP and platform checksum file. The app uses an ad-hoc signature and is not notarized.

### Linux

Run on Linux with the native libraries available:

~~~bash
bash scripts/package-linux.sh
~~~

The script writes the native archive and platform checksum to dist/. Protocol and validation documents stay under .harness/docs/ inside the archive so their local README links resolve. README engineering links point to the repository guides, which belong with the source checkout.

## Publish a new version

After updating and committing the version/build metadata described in the pipeline above, replace X.Y.Z with that new stable Cargo version:

~~~bash
git tag -a vX.Y.Z -m "SU-03T Tools X.Y.Z"
git push origin vX.Y.Z
~~~

Expect source checks, all four platform packages and final publication to succeed. Tag/version mismatches stop publication. A manual workflow run with an existing tag follows the same guards; an empty tag produces artifacts only.

## Inspect delivery

~~~bash
gh api repos/MatriQ/su03t-tools --jq .full_name
gh run list --repo MatriQ/su03t-tools --limit 5
gh release view --repo MatriQ/su03t-tools
python3 scripts/release_assets.py metadata --check-release
~~~

A gh version proves neither login nor target access. A repository read proves that read scope, not every release action. Select the actual run/version and compare commit, packaged --version and manifest. Metadata checks assets for the current package version; it does not prove a later main commit was released. Keep actual run URLs/evidence locally.

## Installation and recovery

[install.sh](../../install.sh) freezes one tag, verifies SHA-256 and archive layout, and stages on the destination filesystem. Mac defaults to ~/Applications/SU03T Update.app and validates identity/version/signature. Linux uses ~/.local/share/su03t-tools/vX.Y.Z and ~/.local/bin/su03t-update. Both old/new repository markers are accepted; foreign destinations are refused.

Verify installation with a temporary --prefix, preserving real apps and sessions. Roll back the app with --version for a known compatible older release; Linux retains previous version directories. Firmware rollback is a separate explicit hardware operation. Published bytes remain immutable.

Fix source/package failures before publication. For publish failure inspect logs and existing draft first, then rerun failed jobs with original verified artifacts when appropriate. Resolve identity/permission errors in that exact context. See [contribution release policy](../../CONTRIBUTING.md#releases) and [validation limits](validation.md).

## Homebrew distribution

The public [MatriQ/homebrew-tap](https://github.com/MatriQ/homebrew-tap) independently maintains the macOS Cask. Users install with `brew install --cask matriq/tap/su03t-tools`; Homebrew installs the app and links the same `su03t-update` CLI. The existing shell installer remains available for macOS and Linux. Quit the application before changing versions or uninstalling.

The tap's Update Cask workflow polls the latest published stable Release hourly and supports manual dispatch. It verifies the actual Mac archive and checksum manifest against GitHub asset SHA-256 metadata, then checks that the manifest agrees with the archive. It refuses incomplete releases, prereleases, automatic downgrades and changed bytes for an existing version. Homebrew style, audit, livecheck, temporary-app-directory installation and uninstall checks run before the bot commits a new version. The tap uses its own built-in GITHUB_TOKEN and needs no cross-repository secret or source-release workflow change. Scheduled runs can be delayed by GitHub.

[Tap maintenance and checks](https://github.com/MatriQ/homebrew-tap/blob/main/.harness/docs/index.md) belong to that separate repository; this harness does not enroll it as a source member. A release is available through Homebrew after its verified Cask update reaches the tap's default branch. Remove the tap's unsigned-release caveat when notarized artifacts become the supported upstream distribution.

The current app remains ad-hoc signed and not notarized. Homebrew retains quarantine and macOS first-launch approval. Cask installation checks do not prove Gatekeeper acceptance, physical flashing or official Homebrew inclusion. Preserve existing apps and command links when switching installers; resolve conflicts explicitly instead of using force or removing quarantine.

## Windows packaging

On Windows x64 with Rust 1.95 MSVC, the Windows SDK, Python and rust-docs, run [package-windows.ps1](../../scripts/package-windows.ps1). It builds the GUI launcher and console executable with a static CRT, collects licenses and packages the readmes, images, protocol and validation records into a Windows-x86_64 ZIP. [install.ps1](../../install.ps1) installs into a user directory, verifies GitHub asset digests and the checksum manifest before replacement, validates the staged executable version, retains download-zone information and rolls back failed replacement. It supports Version, Prefix, NoShortcut and Uninstall parameters; it does not modify system PATH or install drivers. Windows package publication begins at v0.6.0; older release checks retain the three-platform layout. The release publisher requires the fourth archive and install.ps1 for those new versions. Native Windows package and GUI evidence is distinct from physical-board acceptance.

After publication, [Verify Windows installation](../../.github/workflows/verify-install.yml) downloads install.ps1 from the selected public Release, checks its GitHub asset digest, installs the actual Windows archive into a temporary Unicode path and verifies CLI behavior and uninstall under PowerShell 7 and Windows PowerShell 5.1. This proves the public download path separately from local fixture-based package checks.

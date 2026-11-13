# Packaging and release

## Targets

Confirm [MatriQ/su03t-tools](https://github.com/MatriQ/su03t-tools) before a gh operation. CI source checks use macOS/Ubuntu; package runners are macos-latest, ubuntu-24.04 and ubuntu-24.04-arm. The universal Mac app contains arm64/x86_64 slices, targets macOS 12.0, and is ad-hoc signed, not notarized. Linux archives need Ubuntu 24.04-compatible glibc/native libraries. Windows source probes do not establish a supported release.

Environments are the local checkout, GitHub-hosted CI, public Releases, and user desktops with explicitly selected adapters/boards. No backend, database, cloud region or hosted application service is configured. Hardware acceptance has its own board/firmware/power/wiring evidence.

## Pipeline

[release.yml](../../.github/workflows/release.yml) builds main/PR artifacts retained for 14 days. Matching version tags, published Release events, or manual runs with an existing tag enable publication. A manual run without a tag builds only. Metadata requires tag/version and checkout/tag commit equality.

1. Update Cargo.toml/Cargo.lock version and Mac CFBundleVersion in [package-macos.sh](../../scripts/package-macos.sh). Update the version-bound harness CLI probe expectation, then refresh affected source/document reviews through [maintenance](maintenance.md).
2. Run [checks](testing.md), review the diff and follow the current task's publication scope. Push an annotated matching vX.Y.Z tag. Do not retag an old release to hide a fix.
3. Source quality and all three packages must pass. Extracted binaries, signature/architecture, PTYs and temporary-prefix installs are checked before publish.
4. [release_assets.py](../../scripts/release_assets.py) verifies partial hashes, adds install.sh and a unified manifest, uploads a draft, verifies it, then publishes. Only publish has contents:write; CI uses the built-in GITHUB_TOKEN.
5. Publication retries four times with 20 seconds between attempts, allowing draft read visibility to catch up. Incomplete drafts can resume. Complete published assets are verified without replacement; changed released bytes need a new version.

Assets are three platform archives, install.sh and SU03T-Tools-vX.Y.Z-SHA256SUMS.txt. Platform suffixes identify archives; the repository has no OS suffix. Matching-host manual packaging commands:

~~~bash
bash scripts/package-macos.sh --universal
bash scripts/package-linux.sh
~~~

Run the first only on macOS and the second only on Linux. Packaging needs rust-docs for bundled licenses. Linux includes protocol.md and validation.md under .harness/docs/ to retain README links after extraction.

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

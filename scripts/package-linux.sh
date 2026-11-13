#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Linux ]]; then
  echo 'Run this script on Linux (or in the provided Linux build container).' >&2
  exit 1
fi
cargo build --locked --release
PACKAGE_VERSION="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
PACKAGE_DIR="${CARGO_TARGET_DIR:-target}/su03t-linux-package-v${PACKAGE_VERSION}"
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR" dist
cp "${CARGO_TARGET_DIR:-target}/release/su03t-update" README.md LICENSE "$PACKAGE_DIR/"
if [[ -f README.en.md ]]; then cp README.en.md "$PACKAGE_DIR/"; fi
mkdir -p "$PACKAGE_DIR/.harness/docs" "$PACKAGE_DIR/docs"
for PACKAGE_DOC in .harness/docs/protocol.md .harness/docs/validation.md; do
  cp "$PACKAGE_DOC" "$PACKAGE_DIR/.harness/docs/"
done
if [[ -d docs/images ]]; then cp -R docs/images "$PACKAGE_DIR/docs/"; fi
python3 scripts/collect-licenses.py --target "$(rustc -vV | sed -n 's/^host: //p')" --output "$PACKAGE_DIR"
tar -czf "dist/SU03T-Tools-v${PACKAGE_VERSION}-Linux-$(uname -m).tar.gz" -C "$PACKAGE_DIR" .

python3 scripts/release_assets.py checksums "Linux-$(uname -m)"

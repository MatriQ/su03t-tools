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
mkdir -p "$PACKAGE_DIR" dist
cp "${CARGO_TARGET_DIR:-target}/release/su03t-update" README.md LICENSE "$PACKAGE_DIR/"
if [[ -f README.en.md ]]; then cp README.en.md "$PACKAGE_DIR/"; fi
mkdir -p "$PACKAGE_DIR/docs"
for PACKAGE_DOC in docs/protocol.md docs/validation.md; do
  if [[ -f "$PACKAGE_DOC" ]]; then cp "$PACKAGE_DOC" "$PACKAGE_DIR/docs/"; fi
done
if [[ -d docs/images ]]; then cp -R docs/images "$PACKAGE_DIR/docs/"; fi
tar -czf "dist/SU03T-Tools-v${PACKAGE_VERSION}-Linux-$(uname -m).tar.gz" -C "$PACKAGE_DIR" .

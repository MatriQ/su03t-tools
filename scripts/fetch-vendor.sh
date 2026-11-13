#!/bin/bash
set -euo pipefail
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"
mkdir -p vendor
ARCHIVE=vendor/Hummingbird-M-Update-Tool.zip
if [[ -e "$ARCHIVE" ]]; then
  echo "Already present: $ARCHIVE (not overwritten)"
else
  curl --fail --location --output "$ARCHIVE.part" \
    'https://docs.aimachip.com/zh-cn/latest/_static/document/SU-03T/Hummingbird-M-Update-Tool.zip'
  mv "$ARCHIVE.part" "$ARCHIVE"
fi
shasum -a 256 "$ARCHIVE"
if [[ ! -d vendor/Hummingbird-M-Update-Tool ]]; then
  unzip -q "$ARCHIVE" -d vendor
fi
python3 scripts/analyze_vendor.py

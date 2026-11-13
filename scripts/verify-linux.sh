#!/usr/bin/env bash
set -euo pipefail
cd /work
cargo test --locked
cargo test --locked --no-default-features
cargo build --locked --release
python3 tests/client_acceptance.py "${CARGO_TARGET_DIR}/release/su03t-update"
python3 tests/transfer_events.py "${CARGO_TARGET_DIR}/release/su03t-update"
set +e
LIBGL_ALWAYS_SOFTWARE=1 timeout 8s xvfb-run -a "${CARGO_TARGET_DIR}/release/su03t-update" > /build/gui-startup.log 2>&1
GUI_STATUS=$?
set -e
if [[ "$GUI_STATUS" != 124 ]]; then
  cat /build/gui-startup.log
  echo "Linux GUI exited unexpectedly: $GUI_STATUS" >&2
  exit 1
fi
echo 'PASS: Linux GUI remained running for 8 seconds in Xvfb (startup smoke only)'
bash scripts/package-linux.sh

#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script packages the macOS build and must run on macOS." >&2
  exit 1
fi

export MACOSX_DEPLOYMENT_TARGET=12.0
PACKAGE_MODE="${1:-native}"
if [[ "$PACKAGE_MODE" == "--universal" ]]; then
  cargo build --locked --release --target aarch64-apple-darwin
  cargo build --locked --release --target x86_64-apple-darwin
  mkdir -p "$PROJECT_DIR/target/universal/release"
  lipo -create "$PROJECT_DIR/target/aarch64-apple-darwin/release/su03t-update" "$PROJECT_DIR/target/x86_64-apple-darwin/release/su03t-update" -output "$PROJECT_DIR/target/universal/release/su03t-update"
  PACKAGE_BINARY="$PROJECT_DIR/target/universal/release/su03t-update"
elif [[ "$PACKAGE_MODE" == "native" ]]; then
  cargo build --locked --release
  PACKAGE_BINARY="$PROJECT_DIR/target/release/su03t-update"
else
  echo "Usage: $0 [--universal]" >&2
  exit 2
fi

APP="$PROJECT_DIR/dist/SU03T Update.app"
# Each package starts from current sources, without stale resources from older builds.
rm -rf "$APP/Contents/Resources" "$APP/Contents/_CodeSignature"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Replace the executable atomically so a running debug session keeps its original inode.
cp "$PACKAGE_BINARY" "$APP/Contents/MacOS/.su03t-update.new"
mv -f "$APP/Contents/MacOS/.su03t-update.new" "$APP/Contents/MacOS/su03t-update"
chmod +x "$APP/Contents/MacOS/su03t-update"
cp "$PROJECT_DIR/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp "$PROJECT_DIR/LICENSE" "$APP/Contents/Resources/LICENSE.txt"
if [[ "$PACKAGE_MODE" == "--universal" ]]; then
  python3 scripts/collect-licenses.py --target aarch64-apple-darwin --target x86_64-apple-darwin --output "$APP/Contents/Resources"
else
  python3 scripts/collect-licenses.py --target "$(rustc -vV | sed -n 's/^host: //p')" --output "$APP/Contents/Resources"
fi
APP_VERSION="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>SU03T Update</string>
  <key>CFBundleDisplayName</key><string>SU-03T 升级与调试</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>zh-Hans</string></array>
  <key>CFBundleIdentifier</key><string>local.su03t.update</string>
  <key>CFBundleExecutable</key><string>su03t-update</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$APP_VERSION</string>
  <key>CFBundleVersion</key><string>12</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
mkdir -p "$APP/Contents/Resources/en.lproj" "$APP/Contents/Resources/zh-Hans.lproj"
cat > "$APP/Contents/Resources/en.lproj/InfoPlist.strings" <<'LOCALIZATION'
"CFBundleDisplayName" = "SU-03T Tools";
LOCALIZATION
cat > "$APP/Contents/Resources/zh-Hans.lproj/InfoPlist.strings" <<'LOCALIZATION'
"CFBundleDisplayName" = "SU-03T 升级与调试";
LOCALIZATION
plutil -lint "$APP/Contents/Info.plist"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
echo "Packaged: $APP"
echo "Ad-hoc signature only; this app is not notarized."

if [[ "$PACKAGE_MODE" == "--universal" ]]; then
  python3 scripts/release_assets.py macos-archive
fi

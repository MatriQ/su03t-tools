#!/usr/bin/env bash
# Install a verified SU-03T Tools release into user-owned directories.
set -euo pipefail

su03t_error() { printf 'SU-03T install: %s\n' "$*" >&2; exit 1; }
su03t_download() {
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    --retry 3 --connect-timeout 15 --max-time 300 "$1" --output "$2"
}
su03t_hash() {
  local result
  if command -v shasum >/dev/null 2>&1; then
    result=$(shasum -a 256 "$1")
  elif command -v sha256sum >/dev/null 2>&1; then
    result=$(sha256sum "$1")
  else
    su03t_error 'shasum or sha256sum is required.'
  fi
  printf '%s\n' "${result%% *}"
}
su03t_validate_paths() {
  local entry
  while IFS= read -r entry; do
    case "$entry" in
      /*|..|../*|*/../*|*/..) su03t_error 'Unsafe path in release archive.' ;;
    esac
    if [ "$SU03T_OS" = Darwin ]; then
      case "$entry" in
        'SU03T Update.app/'*) ;;
        *) su03t_error 'Unexpected macOS archive layout.' ;;
      esac
    fi
  done
}
su03t_lock() {
  local candidate="$1/.su03t-install.lock"
  mkdir "$candidate" 2>/dev/null || su03t_error "Another install is active in $1. If it was interrupted, remove $candidate before retrying."
  SU03T_INSTALL_LOCK=$candidate
}
su03t_replace() {
  # A staging directory on the destination filesystem keeps replacement atomic.
  local source=$1 target=$2 backup=$3
  if [ -e "$target" ] || [ -L "$target" ]; then mv "$target" "$backup"; fi
  if ! mv "$source" "$target"; then
    if [ -e "$backup" ] || [ -L "$backup" ]; then mv "$backup" "$target"; fi
    su03t_error "Could not install $target; the previous installation was restored."
  fi
}
su03t_cleanup() {
  if [ -n "${SU03T_INSTALL_STAGE:-}" ]; then rm -rf "$SU03T_INSTALL_STAGE"; fi
  if [ -n "${SU03T_INSTALL_WORK:-}" ]; then rm -rf "$SU03T_INSTALL_WORK"; fi
  if [ -n "${SU03T_INSTALL_LOCK:-}" ]; then rmdir "$SU03T_INSTALL_LOCK" 2>/dev/null || true; fi
}
su03t_main() {
  local requested_version=${SU03T_VERSION:-} prefix=${SU03T_INSTALL_PREFIX:-}
  local repo='https://github.com/MatriQ/su03t-tools' resolved tag platform asset manifest expected actual
  local destination version_output target old_link
  SU03T_INSTALL_WORK=''
  SU03T_INSTALL_STAGE=''
  SU03T_INSTALL_LOCK=''
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --version) [ "$#" -ge 2 ] || su03t_error '--version needs a value.'; requested_version=$2; shift 2 ;;
      --prefix) [ "$#" -ge 2 ] || su03t_error '--prefix needs a directory.'; prefix=$2; shift 2 ;;
      -h|--help)
        printf '%s\n' 'Install SU-03T Tools for macOS or Linux.' \
          'Usage: bash install.sh [--version vX.Y.Z] [--prefix DIR]' \
          'Defaults: latest stable release; ~/Applications on macOS, ~/.local on Linux.'
        return 0 ;;
      *) su03t_error "Unknown option: $1" ;;
    esac
  done
  SU03T_OS=$(uname -s)
  case "$SU03T_OS:$(uname -m)" in
    Darwin:arm64|Darwin:x86_64) platform=macOS-universal; prefix=${prefix:-"$HOME/Applications"} ;;
    Linux:x86_64|Linux:amd64) platform=Linux-x86_64; prefix=${prefix:-"$HOME/.local"} ;;
    Linux:aarch64|Linux:arm64) platform=Linux-aarch64; prefix=${prefix:-"$HOME/.local"} ;;
    *) su03t_error "Unsupported system: $SU03T_OS $(uname -m). Build from source instead." ;;
  esac
  command -v curl >/dev/null 2>&1 || su03t_error 'curl is required.'
  if [ -n "$requested_version" ]; then
    tag=v${requested_version#v}
  else
    resolved=$(curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
      --retry 3 --connect-timeout 15 --max-time 60 --output /dev/null --write-out '%{url_effective}' "$repo/releases/latest")
    case "$resolved" in "$repo/releases/tag/"*) tag=${resolved##*/} ;; *) su03t_error 'Could not resolve the latest release.' ;; esac
  fi
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || su03t_error 'Version must be vX.Y.Z.'
  asset="SU03T-Tools-$tag-$platform"
  if [ "$SU03T_OS" = Darwin ]; then asset="$asset.zip"; else asset="$asset.tar.gz"; fi
  manifest="SU03T-Tools-$tag-SHA256SUMS.txt"
  SU03T_INSTALL_WORK=$(mktemp -d "${TMPDIR:-/tmp}/su03t-install.XXXXXXXX")
  # Bash versions unwind local variables differently on errexit. Cleanup state
  # remains available to the EXIT trap after main's local scope has ended.
  trap su03t_cleanup EXIT
  printf 'Downloading SU-03T Tools %s (%s)...\n' "$tag" "$platform"
  su03t_download "$repo/releases/download/$tag/$manifest" "$SU03T_INSTALL_WORK/SHA256SUMS.txt"
  expected=$(awk -v name="$asset" '$2 == name && NF == 2 { print $1; count++ } END { if (count != 1) exit 1 }' "$SU03T_INSTALL_WORK/SHA256SUMS.txt") \
    || su03t_error "Release $tag does not contain a unique checksum for $asset."
  [[ "$expected" =~ ^[a-f0-9]{64}$ ]] || su03t_error 'Invalid SHA-256 manifest.'
  su03t_download "$repo/releases/download/$tag/$asset" "$SU03T_INSTALL_WORK/$asset"
  actual=$(su03t_hash "$SU03T_INSTALL_WORK/$asset")
  [ "$actual" = "$expected" ] || su03t_error 'SHA-256 mismatch; the existing installation was not changed.'
  mkdir -p "$SU03T_INSTALL_WORK/unpacked"
  if [ "$SU03T_OS" = Darwin ]; then
    for target in ditto codesign zipinfo; do command -v "$target" >/dev/null 2>&1 || su03t_error "$target is required."; done
    zipinfo -1 "$SU03T_INSTALL_WORK/$asset" | su03t_validate_paths
    if zipinfo -l "$SU03T_INSTALL_WORK/$asset" | awk '$1 ~ /^l/ { found = 1 } END { exit !found }'; then
      su03t_error 'Symlinks are not supported in this app archive.'
    fi
    ditto -x -k "$SU03T_INSTALL_WORK/$asset" "$SU03T_INSTALL_WORK/unpacked"
    target="$SU03T_INSTALL_WORK/unpacked/SU03T Update.app"
    [ -d "$target" ] && [ ! -L "$target" ] || su03t_error 'The app bundle is missing.'
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$target/Contents/Info.plist")" = local.su03t.update ] \
      || su03t_error 'Unexpected application identifier.'
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$target/Contents/Info.plist")" = "${tag#v}" ] \
      || su03t_error 'Application version does not match the release.'
    codesign --verify --deep --strict "$target" || su03t_error 'App signature verification failed.'
    mkdir -p "$prefix"
    prefix=$(cd "$prefix" && pwd -P)
    su03t_lock "$prefix"
    destination="$prefix/SU03T Update.app"
    if [ -e "$destination" ] || [ -L "$destination" ]; then
      [ ! -L "$destination" ] && [ -d "$destination" ] \
        && [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$destination/Contents/Info.plist" 2>/dev/null)" = local.su03t.update ] \
        || su03t_error 'Destination contains another app or a symlink; choose a different --prefix.'
    fi
    SU03T_INSTALL_STAGE=$(mktemp -d "$prefix/.su03t-install.XXXXXXXX")
    ditto "$target" "$SU03T_INSTALL_STAGE/SU03T Update.app"
    codesign --verify --deep --strict "$SU03T_INSTALL_STAGE/SU03T Update.app"
    su03t_replace "$SU03T_INSTALL_STAGE/SU03T Update.app" "$destination" "$SU03T_INSTALL_STAGE/previous.app"
    printf 'Installed: %s\nOpen with: open %q\n' "$destination" "$destination"
    printf '%s\n' 'Quit any older instance before opening the new app. First launch may require macOS approval; the app is not notarized.'
  else
    command -v tar >/dev/null 2>&1 || su03t_error 'tar is required.'
    tar -tzf "$SU03T_INSTALL_WORK/$asset" | su03t_validate_paths
    if tar -tvzf "$SU03T_INSTALL_WORK/$asset" | awk '$1 !~ /^[-d]/ { found = 1 } END { exit !found }'; then
      su03t_error 'Only regular files and directories are supported in the Linux archive.'
    fi
    tar -xzf "$SU03T_INSTALL_WORK/$asset" -C "$SU03T_INSTALL_WORK/unpacked"
    target="$SU03T_INSTALL_WORK/unpacked/su03t-update"
    [ -f "$target" ] && [ -x "$target" ] || su03t_error 'Executable is missing from the archive.'
    if ! version_output=$("$target" --version 2>"$SU03T_INSTALL_WORK/runtime-error"); then
      cat "$SU03T_INSTALL_WORK/runtime-error" >&2
      su03t_error 'The binary cannot run here. Check CPU architecture and Linux runtime libraries; see the README.'
    fi
    [ "$version_output" = "su03t-update ${tag#v}" ] || su03t_error 'Executable version does not match the release.'
    mkdir -p "$prefix/bin" "$prefix/share/su03t-tools"
    prefix=$(cd "$prefix" && pwd -P)
    su03t_lock "$prefix"
    destination="$prefix/share/su03t-tools/$tag"
    if [ -e "$destination" ] || [ -L "$destination" ]; then
      [ ! -L "$destination" ] && [ -f "$destination/.su03t-install" ] \
        || su03t_error 'Destination is not an installation managed by this script.'
      case "$(head -n 1 "$destination/.su03t-install")" in
        MatriQ/su03t-tools|MatriQ/su03t-macos) ;; # Accept installations made before the repository rename.
        *) su03t_error 'Destination is not an installation managed by this script.' ;;
      esac
    fi
    target="$prefix/bin/su03t-update"
    if [ -e "$target" ] || [ -L "$target" ]; then
      [ -L "$target" ] && [ ! -d "$target" ] || su03t_error 'An existing executable occupies the command path; choose another --prefix.'
      old_link=$(readlink "$target")
      case "$old_link" in "$prefix/share/su03t-tools/"*/su03t-update) ;; *) su03t_error 'The command path belongs to another installation.' ;; esac
    fi
    SU03T_INSTALL_STAGE=$(mktemp -d "$prefix/share/su03t-tools/.install.XXXXXXXX")
    mv "$SU03T_INSTALL_WORK/unpacked" "$SU03T_INSTALL_STAGE/package"
    printf 'MatriQ/su03t-tools\n%s\n%s\n' "$tag" "$expected" > "$SU03T_INSTALL_STAGE/package/.su03t-install"
    su03t_replace "$SU03T_INSTALL_STAGE/package" "$destination" "$SU03T_INSTALL_STAGE/previous"
    ln -s "$destination/su03t-update" "$SU03T_INSTALL_STAGE/command"
    mv -f "$SU03T_INSTALL_STAGE/command" "$prefix/bin/su03t-update"
    printf 'Installed: %s\nRun: %q\n' "$destination" "$prefix/bin/su03t-update"
    case ":$PATH:" in *":$prefix/bin:"*) ;; *) printf 'Add to PATH: export PATH=%q:"$PATH"\n' "$prefix/bin" ;; esac
  fi
  su03t_cleanup
  trap - EXIT
}
su03t_main "$@"

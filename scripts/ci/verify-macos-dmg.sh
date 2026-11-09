#!/usr/bin/env bash
set -euo pipefail

# Verify the .app inside the DMG that users actually download.
#
# The DMG is built from the signed app bundle, but mounting it catches a
# packaging regression that a check of target/.../bundle/macos would miss.
#
# Usage: ./scripts/ci/verify-macos-dmg.sh [path/to/*.dmg]

dmg="${1:-}"

if [[ -n "$dmg" && ! -f "$dmg" ]]; then
  echo "error: no DMG at $dmg" >&2
  exit 2
fi

if [[ -z "$dmg" ]]; then
  dmg="$(find target apps/launcher/src-tauri/target \
    -maxdepth 6 \
    -path '*/release/bundle/dmg/*.dmg' \
    -print -quit 2>/dev/null || true)"
fi

if [[ -z "$dmg" ]]; then
  echo "error: could not find a macOS DMG; pass its path explicitly" >&2
  exit 2
fi

mount_point="$(mktemp -d)"
cleanup() {
  hdiutil detach "$mount_point" >/dev/null 2>&1 || true
  rmdir "$mount_point" 2>/dev/null || true
}
trap cleanup EXIT

hdiutil attach -nobrowse -readonly -mountpoint "$mount_point" "$dmg" >/dev/null
app="$(find "$mount_point" -maxdepth 1 -name '*.app' -print -quit)"
if [[ -z "$app" ]]; then
  echo "error: $dmg does not contain an .app bundle" >&2
  exit 1
fi

echo "Verifying DMG: $dmg"
bash "$(dirname "$0")/verify-macos-bundle.sh" "$app"

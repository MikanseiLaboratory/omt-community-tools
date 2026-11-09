#!/usr/bin/env bash
set -euo pipefail

# Verify the app inside the Tauri updater archive is signed as a bundle.
#
# The updater archive is created after the macOS .app is signed, but that
# ordering is not obvious from the release workflow. Checking the extracted
# app keeps a future bundler change from shipping an unsigned update.
#
# Usage: ./scripts/ci/verify-macos-updater.sh [path/to/*.app.tar.gz]

tarball="${1:-}"

if [[ -n "$tarball" && ! -f "$tarball" ]]; then
  echo "error: no updater archive at $tarball" >&2
  exit 2
fi

if [[ -z "$tarball" ]]; then
  tarball="$(find target apps/launcher/src-tauri/target \
    -maxdepth 6 \
    -path '*/release/bundle/macos/*.app.tar.gz' \
    -print -quit 2>/dev/null || true)"
fi

if [[ -z "$tarball" ]]; then
  echo "error: could not find a macOS updater archive; pass its path explicitly" >&2
  exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

tar -xzf "$tarball" -C "$tmp"
app="$(find "$tmp" -maxdepth 1 -name '*.app' -print -quit)"
if [[ -z "$app" ]]; then
  echo "error: $tarball does not contain an .app bundle" >&2
  exit 1
fi

echo "Verifying updater archive: $tarball"
bash "$(dirname "$0")/verify-macos-bundle.sh" "$app"

#!/usr/bin/env bash
set -euo pipefail

# Verify that a macOS .app is signed as a bundle, not merely linker-signed.
#
# Rust's linker ad-hoc signs every arm64 Mach-O. If the surrounding .app has no
# bundle signature, Gatekeeper reports "code has no resources but signature
# indicates they must be present" and refuses to launch the app as damaged.
# This check keeps that state from reaching a release.
#
# Usage: ./scripts/ci/verify-macos-bundle.sh [path/to/App.app]

app="${1:-}"

if [[ -n "$app" && ! -d "$app" ]]; then
  echo "error: no app bundle at $app" >&2
  exit 2
fi

if [[ -z "$app" ]]; then
  # Tauri writes <target-dir>/<triple>/release/bundle/macos/<name>.app.
  app="$(find target apps/launcher/src-tauri/target \
    -maxdepth 6 \
    -path '*/release/bundle/macos/*.app' \
    -print -quit 2>/dev/null || true)"
fi

if [[ -z "$app" ]]; then
  echo "error: could not find a bundled .app; pass its path explicitly" >&2
  exit 2
fi

echo "Verifying macOS bundle: $app"

if ! output="$(codesign --verify --deep --strict --verbose=2 "$app" 2>&1)"; then
  printf '%s\n' "$output" >&2
  echo "error: code signature verification failed; Gatekeeper will reject this app." >&2
  exit 1
fi

if [[ ! -f "$app/Contents/_CodeSignature/CodeResources" ]]; then
  echo "error: $app has no Contents/_CodeSignature/CodeResources; it is not signed as a bundle." >&2
  exit 1
fi

codesign -dv --verbose=4 "$app" 2>&1 \
  | grep -E '^(Identifier|Format|CodeDirectory|Signature|TeamIdentifier|Sealed Resources)' \
  || true

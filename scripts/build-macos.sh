#!/usr/bin/env bash
set -euo pipefail

# Build a distributable macOS app with Tauri's signing step enabled.
#
# A plain `bun run tauri build` leaves the .app unsigned. The arm64 Mach-Os
# still carry a linker ad-hoc signature, so a downloaded DMG is reported by
# Gatekeeper as damaged. This wrapper builds the sidecars, stages them and
# signs the bundle. A real identity (APPLE_SIGNING_IDENTITY or
# APPLE_CERTIFICATE) is left untouched.
#
# Usage:
#   ./scripts/build-macos.sh [tauri build args...]
#   TAURI_TARGET=x86_64-apple-darwin ./scripts/build-macos.sh
#
# Pass the target through TAURI_TARGET rather than --target, so the sidecars
# and the app are built for the same architecture.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="${TAURI_TARGET:-$(rustc -vV | awk '/^host:/{print $2}')}"

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" && -z "${APPLE_CERTIFICATE:-}" ]]; then
  export APPLE_SIGNING_IDENTITY="-"
fi

cargo build --release --target "$TARGET" \
  -p omt-studio-monitor \
  -p omt-test-patterns \
  -p omt-config-manager \
  -p omt-discovery-server

bash ./scripts/prepare-sidecars.sh "$TARGET"

# Local builds do not need updater archives; requiring the release signing key
# here would make the wrapper fail after building the app. Keep updater
# artifacts when the caller has the key (or passes their own --config).
args=(build --target "$TARGET")
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  args+=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
fi

exec bun run --cwd apps/launcher tauri "${args[@]}" "$@"

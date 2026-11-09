#!/usr/bin/env bash
set -euo pipefail

# Copy release tool binaries into the Tauri sidecar folder with target-triple suffixes.
# Usage: ./scripts/prepare-sidecars.sh [target-triple]

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST="$(rustc -vV | awk '/^host:/{print $2}')"
TARGET="${1:-$HOST}"
OUT="$ROOT/apps/launcher/src-tauri/binaries"

# Only fall back to target/release for a host build without --target. For a
# cross build, target/release contains host binaries and using them would put
# the wrong architecture inside the bundle (the app starts, sidecars do not).
if [[ -d "$ROOT/target/$TARGET/release" ]]; then
  PROFILE_DIR="$ROOT/target/$TARGET/release"
elif [[ "$TARGET" == "$HOST" && -d "$ROOT/target/release" ]]; then
  PROFILE_DIR="$ROOT/target/release"
else
  echo "error: $ROOT/target/$TARGET/release not found." >&2
  echo "Build the tools first: cargo build --release --target $TARGET -p omt-studio-monitor -p omt-test-patterns -p omt-config-manager -p omt-discovery-server" >&2
  exit 1
fi
mkdir -p "$OUT"

copy_one() {
  local name="$1"
  local src
  if [[ "$TARGET" == *-windows-* ]]; then
    src="$PROFILE_DIR/${name}.exe"
    if [[ ! -f "$src" ]]; then
      echo "error: missing $src for $TARGET" >&2
      exit 1
    fi
    cp "$src" "$OUT/${name}-${TARGET}.exe"
  else
    src="$PROFILE_DIR/${name}"
    if [[ ! -f "$src" ]]; then
      echo "error: missing $src for $TARGET" >&2
      exit 1
    fi
    cp "$src" "$OUT/${name}-${TARGET}"
    chmod +x "$OUT/${name}-${TARGET}"
  fi
  echo "prepared $name for $TARGET"
}

copy_one omt-studio-monitor
copy_one omt-test-patterns
copy_one omt-config-manager
copy_one omt-discovery-server-gui
copy_one omt-discovery-server

#!/usr/bin/env bash
# Optional PGO build for OMT sidecars / monitor-bench.
# Does NOT modify Cargo.toml profiles permanently.
#
# Usage (from omt-community-tools root):
#   ./scripts/bench/pgo-build.sh
# Requires llvm-profdata on PATH.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

PROF_DIR="${PROF_DIR:-target/pgo-data}"
RAW_DIR="$PROF_DIR/raw"
mkdir -p "$RAW_DIR"
rm -rf "$RAW_DIR"/*
MERGED="$PROF_DIR/merged.profdata"

echo "==> Stage 1: instrumented build"
export RUSTFLAGS="-Cprofile-generate=$RAW_DIR"
cargo build --release -p monitor-bench -p omt-test-patterns -p omt-studio-monitor

echo "==> Stage 2: train with vmx simd_report"
(
  cd ../vmx-rs
  export RUSTFLAGS="-Cprofile-generate=$RAW_DIR"
  cargo build --release --example simd_report
  cargo run --release --example simd_report -- 1920 1080 30
)

if ! command -v llvm-profdata >/dev/null 2>&1; then
  echo "llvm-profdata not found. Install LLVM or: rustup component add llvm-tools-preview"
  exit 1
fi

echo "==> Stage 3: merge profiles -> $MERGED"
mapfile -t RAWS < <(find "$RAW_DIR" -name '*.profraw' -print)
llvm-profdata merge -o "$MERGED" "${RAWS[@]}"

echo "==> Stage 4: optimized build with profile-use"
export RUSTFLAGS="-Cprofile-use=$MERGED -Cllvm-args=-pgo-warn-missing-function"
cargo build --release -p monitor-bench -p omt-test-patterns -p omt-studio-monitor
unset RUSTFLAGS

echo "PGO build complete. Compare against scripts/bench/run-baseline.sh results."

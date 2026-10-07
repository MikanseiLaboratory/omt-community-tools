#!/usr/bin/env bash
# Build release tools and run reproducible VMX baselines.
# Usage (from omt-community-tools root):
#   ./scripts/bench/run-baseline.sh
#   NATIVE=1 ./scripts/bench/run-baseline.sh
#   ./scripts/bench/run-baseline.sh 1920 1080 20

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

WIDTH="${1:-1920}"
HEIGHT="${2:-1080}"
ITERS="${3:-20}"
OUT_DIR="${OUT_DIR:-target/bench-results}"
mkdir -p "$OUT_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
TAG="release"
if [[ "${NATIVE:-0}" == "1" ]]; then
  export RUSTFLAGS="-C target-cpu=native"
  TAG="native"
  echo "RUSTFLAGS=$RUSTFLAGS"
else
  unset RUSTFLAGS || true
fi

OUT_FILE="$OUT_DIR/baseline-$TAG-$STAMP.txt"

echo "==> Building vmx simd_report ($TAG)"
(
  cd ../vmx-rs
  cargo build --release --example simd_report
  cargo run --release --example simd_report -- "$WIDTH" "$HEIGHT" "$ITERS" | tee "$ROOT/$OUT_FILE"
)

echo "==> Building omt-community-tools monitor-bench ($TAG)"
cargo build --release -p monitor-bench -p omt-test-patterns | tee -a "$OUT_FILE"

echo "Results written to $OUT_FILE"
echo "Tip: start omt-test-patterns, then:"
echo "  cargo run --release -p monitor-bench -- --url omt://127.0.0.1:PORT/name --duration 10 --backend null"

# Optional PGO build for OMT sidecars / monitor-bench.
# Does NOT modify Cargo.toml profiles permanently.
#
# Usage (from omt-community-tools root, developer machine only):
#   powershell -File scripts/bench/pgo-build.ps1
#   powershell -File scripts/bench/pgo-build.ps1 -TrainSeconds 20
#
# Requires llvm-profdata on PATH (from LLVM or rustup llvm-tools).

param(
    [int]$TrainSeconds = 15,
    [string]$ProfDir = "target/pgo-data"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root

New-Item -ItemType Directory -Force -Path $ProfDir | Out-Null
$ProfDirAbs = (Resolve-Path $ProfDir).Path
$RawDir = Join-Path $ProfDirAbs "raw"
New-Item -ItemType Directory -Force -Path $RawDir | Out-Null
Get-ChildItem $RawDir -ErrorAction SilentlyContinue | Remove-Item -Force -Recurse -ErrorAction SilentlyContinue

Write-Host "==> Stage 1: instrumented build"
$env:RUSTFLAGS = "-Cprofile-generate=$RawDir"
cargo build --release -p monitor-bench -p omt-test-patterns -p omt-studio-monitor

Write-Host "==> Stage 2: train (vmx simd_report + short sleep placeholder for live OMT)"
Push-Location (Join-Path (Split-Path $Root -Parent) "vmx-rs")
try {
    $env:RUSTFLAGS = "-Cprofile-generate=$RawDir"
    cargo build --release --example simd_report
    cargo run --release --example simd_report -- 1920 1080 30 | Out-Host
} finally {
    Pop-Location
}

Write-Host "Optional live training: run test-patterns + monitor-bench for ~$TrainSeconds seconds while instrumented binaries are used."
Start-Sleep -Seconds ([Math]::Min($TrainSeconds, 3))

$merged = Join-Path $ProfDirAbs "merged.profdata"
Write-Host "==> Stage 3: merge profiles -> $merged"
$llvm = Get-Command llvm-profdata -ErrorAction SilentlyContinue
if (-not $llvm) {
    Write-Host "llvm-profdata not found on PATH. Install LLVM tools or:"
    Write-Host "  rustup component add llvm-tools-preview"
    Write-Host "Then re-run this script."
    exit 1
}
& llvm-profdata merge -o $merged (Get-ChildItem $RawDir -Recurse -Filter *.profraw | ForEach-Object { $_.FullName })

Write-Host "==> Stage 4: optimized build with profile-use"
$env:RUSTFLAGS = "-Cprofile-use=$merged -Cllvm-args=-pgo-warn-missing-function"
cargo build --release -p monitor-bench -p omt-test-patterns -p omt-studio-monitor

Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
Write-Host "PGO build complete. Compare against scripts/bench/run-baseline.ps1 results."

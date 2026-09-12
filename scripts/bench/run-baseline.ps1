# Build release sidecars/tools and run reproducible VMX / monitor-bench baselines.
# Usage (from omt-community-tools root):
#   powershell -File scripts/bench/run-baseline.ps1
#   powershell -File scripts/bench/run-baseline.ps1 -Native
#   powershell -File scripts/bench/run-baseline.ps1 -Width 1920 -Height 1080 -Iters 20

param(
    [switch]$Native,
    [int]$Width = 1920,
    [int]$Height = 1080,
    [int]$Iters = 20,
    [string]$OutDir = "target/bench-results"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$tag = if ($Native) { "native" } else { "release" }
$outFile = Join-Path $OutDir "baseline-$tag-$stamp.txt"

if ($Native) {
    $env:RUSTFLAGS = "-C target-cpu=native"
    Write-Host "RUSTFLAGS=$env:RUSTFLAGS"
} else {
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
}

Write-Host "==> Building vmx simd_report ($tag)"
Push-Location (Join-Path (Split-Path $Root -Parent) "vmx-rs")
try {
    cargo build --release --example simd_report
    $report = cargo run --release --example simd_report -- $Width $Height $Iters 2>&1 |
        Tee-Object -FilePath $outFile
    Write-Host $report
} finally {
    Pop-Location
}

Write-Host "==> Building omt-community-tools monitor-bench ($tag)"
cargo build --release -p monitor-bench -p omt-test-patterns 2>&1 | Tee-Object -FilePath $outFile -Append

Write-Host "Results written to $outFile"
Write-Host "Tip: start omt-test-patterns, then:"
Write-Host "  cargo run --release -p monitor-bench -- --url omt://127.0.0.1:PORT/name --duration 10 --backend null"

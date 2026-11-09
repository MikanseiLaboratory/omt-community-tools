# Copy release tool binaries into the Tauri sidecar folder with target-triple suffixes.
param(
    [string]$Target = (rustc -vV | Select-String '^host:').ToString().Split(' ')[1]
)

$Root = Split-Path -Parent $PSScriptRoot
$Out = Join-Path $Root "apps\launcher\src-tauri\binaries"
$HostTriple = (rustc -vV | Select-String '^host:').ToString().Split(' ')[1]
$TargetDir = Join-Path $Root "target\$Target\release"
$HostDir = Join-Path $Root "target\release"

# Only fall back to target\release for a host build without --target. For a
# cross build, target\release contains host binaries and using them would put
# the wrong architecture inside the bundle (the app starts, sidecars do not).
if (Test-Path $TargetDir) {
    $ProfileDir = $TargetDir
} elseif ($Target -eq $HostTriple -and (Test-Path $HostDir)) {
    $ProfileDir = $HostDir
} else {
    throw "missing $TargetDir - build with cargo build --release --target $Target first"
}
New-Item -ItemType Directory -Force -Path $Out | Out-Null

function Copy-One([string]$Name) {
    $src = Join-Path $ProfileDir "$Name.exe"
    if (-not (Test-Path $src)) { throw "missing $src - build with cargo build --release -p $Name --target $Target first" }
    $dst = Join-Path $Out "$Name-$Target.exe"
    Copy-Item $src $dst -Force
    Write-Host "prepared $Name for $Target"
}

Copy-One omt-studio-monitor
Copy-One omt-test-patterns
Copy-One omt-config-manager
Copy-One omt-discovery-server-gui
Copy-One omt-discovery-server

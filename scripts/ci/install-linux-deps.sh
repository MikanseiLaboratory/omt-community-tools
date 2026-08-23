#!/usr/bin/env bash
set -euo pipefail

# Install Debian/Ubuntu packages needed to compile OMT Tools on Linux.
# Usage: ./scripts/ci/install-linux-deps.sh [tools|tauri]
#   tools — audio / windowing libs for sidecar crates
#   tauri — tools plus GTK / WebKitGTK 4.1 / AppImage bundler deps (default)

mode="${1:-tauri}"

packages=(
  pkg-config
  libasound2-dev
  libudev-dev
  libxkbcommon-dev
  libxkbcommon-x11-dev
  libwayland-dev
  libxcb-render0-dev
  libxcb-shape0-dev
  libxcb-xfixes0-dev
)

if [[ "$mode" == "tauri" ]]; then
  packages+=(
    build-essential
    curl
    wget
    file
    libssl-dev
    libgtk-3-dev
    libwebkit2gtk-4.1-dev
    libayatana-appindicator3-dev
    librsvg2-dev
    patchelf
    libxdo-dev
    libfuse2
  )
elif [[ "$mode" != "tools" ]]; then
  echo "usage: $0 [tools|tauri]" >&2
  exit 2
fi

sudo apt-get update
sudo apt-get install -y --no-install-recommends "${packages[@]}"

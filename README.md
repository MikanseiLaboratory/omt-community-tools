# OMT Community Tools

[![CI](https://github.com/MikanseiLaboratory/omt-community-tools/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/omt-community-tools/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/MikanseiLaboratory/omt-community-tools?label=Latest%20release)](https://github.com/MikanseiLaboratory/omt-community-tools/releases/latest)

Open Media Transport production utilities made by [MikanseiLaboratory](https://github.com/MikanseiLaboratory).

<img width="907" height="703" alt="image" src="https://github.com/user-attachments/assets/e3900c36-cf5b-47fe-8dc0-174d53682840" />



## Suite contents

| Tool | Description |
|------|-------------|
| Studio Monitor | Discover and view OMT sources on the LAN |
| Test Patterns | Send SMPTE-style patterns + tone over OMT |
| Config Manager | View and edit the global OMT `settings.xml` |
| Discovery Server | GUI + CLI TCP discovery server (port 6399) |

Official vMix OMT tools for Windows (Desktop Capture, Viewer, Matrix Router, Settings Manager): [vMix Desktop Capture](https://www.vmix.com/software/vmix-desktop-capture.aspx)

## Prerequisites

- Rust **1.97+** (edition 2024)
- Bun 1.2+ (launcher frontend)

## Build Targets

- Windows x64 (`x86_64-pc-windows-msvc`)
- Windows Arm64 (`aarch64-pc-windows-msvc`)
- macOS Intel (`x86_64-apple-darwin`)
- macOS Apple Silicon (`aarch64-apple-darwin`)
- [WIP] Linux x64 (`x86_64-unknown-linux-gnu`)
- [WIP] Linux Arm64 (`aarch64-unknown-linux-gnu`)

## Linux notes

Linux builds ship as `.deb`, `.rpm`, and AppImage.

The `.deb` installs `/usr/bin/omt-launcher` and the tool binaries in `/usr/bin` (`omt-studio-monitor`, `omt-test-patterns`, `omt-config-manager`, `omt-discovery-server`, `omt-discovery-server-gui`), plus a menu entry "OMT Tools". The AppImage bundles its own WebKitGTK and runs with `GDK_BACKEND=x11` (X11/XWayland). The AppImage forces `GDK_BACKEND=x11`, so on Wayland it requires XWayland (verified: on a pure Wayland session without XWayland it fails with "Failed to initialize GTK"); on Wayland desktops such as Raspberry Pi OS (which ship XWayland) it works, and the `.deb` runs natively on Wayland.

### Runtime requirements

- Avahi (mDNS) must be running for OMT source discovery. Install it with `sudo apt install avahi-daemon` and make sure the daemon is running. Without it, discovery does not work and the OBS OMT plugin can crash.
- Studio Monitor and Test Patterns need a Vulkan driver (wgpu). On a machine without a GPU driver (a VM or a headless box), install Mesa's software Vulkan driver: `sudo apt install mesa-vulkan-drivers` (lavapipe). Without it they fail with `NoSupportedDeviceFound`. Software rendering works, but the display frame rate will be low.

### WebKitGTK

On Linux the launcher sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` and `WEBKIT_DISABLE_COMPOSITING_MODE=1` at startup. That fixes a scrambled launcher window on Raspberry Pi OS. It applies to every install method, because the workaround is in the launcher binary. The launcher sets each variable only when it is unset, so a value you set yourself wins, including `0` (for example in `/etc/environment`). Set them to `0` to opt out. Child processes started by the launcher inherit these variables.

## License

[PolyForm Shield 1.0.0](https://polyformproject.org/licenses/shield/1.0.0)

Source-available. You may use, modify, and distribute this software for any purpose except providing a product that competes with this software or with products MikanseiLaboratory provides using it. See [LICENSE](LICENSE) for the full terms.

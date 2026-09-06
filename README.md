# OMT Community Tools

[![CI](https://github.com/MikanseiLaboratory/omt-tools/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/omt-tools/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/MikanseiLaboratory/omt-tools?label=Latest%20release)](https://github.com/MikanseiLaboratory/omt-tools/releases/latest)

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

## License

[PolyForm Shield 1.0.0](https://polyformproject.org/licenses/shield/1.0.0)

Source-available. You may use, modify, and distribute this software for any purpose except providing a product that competes with this software or with products MikanseiLaboratory provides using it. See [LICENSE](LICENSE) for the full terms.

<div align="center">

<img src="assets/icon_blue.png" alt="Rewind" width="96" />

# Rewind

**A fast, local-only clipping software for Windows 11.**

[![Release](https://img.shields.io/github/v/release/BioBuch/Rewind?label=latest)](https://github.com/BioBuch/Rewind/releases)
[![Ko-fi](https://img.shields.io/badge/Support-Ko--fi-FF5E5B?logo=ko-fi&logoColor=white)](https://ko-fi.com/rewindapp)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

</div>

---

Rewind captures your gameplay in the background using a rolling RAM buffer and saves crisp, synchronized MP4 clips on a single keypress — without cloud accounts, telemetry, or heavy overlays.

## Features

### Capture & Performance
- **Zero-Lag RAM Ring Buffer** — Keeps the last `15s`–`5m` in system RAM (`64–768 MB` budget) so your SSD isn't constantly worn down by temp writes.
- **Any-GPU Hardware Encoding** — Works on **NVIDIA (`NVENC`)**, **AMD (`AMF`)**, and **Intel (`QuickSync`)** via Windows Media Foundation, supporting `HEVC`, hardware `AV1`, and `H.264` up to `4K 120fps` in `.mp4` or `.mkv` (`CQP` or `CBR`).
- **GPU CAS Sharpening** — Optional Direct3D 11 Contrast-Adaptive Sharpening compute shader (`Light` / `Medium`) applied directly on the GPU before encoding.
- **Post-Clip Reaction Buffer** — Add `+5s`, `+10s`, or `+15s` after pressing your save hotkey so you never cut off your friends' reactions.
- **Hook-Free Game Detection** — Detects active games via foreground window titles and classes (`SetWinEventHook`) only. Never opens game process handles, reads memory, or injects overlay DLLs. Supports custom/ignored game lists and manual `Desktop` capture.

### Audio, Editing & Sharing
- **Synced Game & Mic Audio** — Captures WASAPI system loopback and microphone in lockstep with device selection, independent volume sliders, a global mic-mute hotkey (`Ctrl+M`), and optional **separate audio tracks** for editing.
- **1-Click Discord `< 10 MB` Export** — Automatically calculates bitrate, resolution, and framerate to compress any clip (or trimmed range) strictly under Discord's `10 MB` free upload limit.
- **Lossless Quick Trim & `9:16` Vertical** — Hardware-decoded clip player with frame scrubbing, instant lossless IDR-keyframe MP4 trimming (non-blocking so live replay keeps recording), and a `9:16` vertical export option.

### Library, Design & Zero Bloat
- **Split Tray Daemon + Ephemeral UI** — Pure Rust + `egui` with no Electron, Chromium, Overwolf, or bundled `ffmpeg.exe`. Closing the window exits the UI completely (`0 MB` UI RAM) while the tray daemon stays active.
- **SQLite Clip Library & Auto-Cleanup** — Auto-generated thumbnails, per-game folders, search, sorting, favorites, collections, an 8-second **Undo Delete**, and automatic cleanup of oldest non-favorite clips at your storage limit.
- **Custom Looks, SteamGridDB & Discord RPC** — Set a custom wallpaper with blur/brightness and auto-extracted accent colors, pull game icons via SteamGridDB, show live status on Discord Rich Presence, and update in one click from GitHub Releases.

## How Rewind Compares

| Feature | Rewind | Medal / Outplayed | ShadowPlay / ReLive | OBS Studio |
| :--- | :---: | :---: | :---: | :---: |
| **100% Local, Free & Open Source** | ✅ GPL-3.0 | ❌ Ads / Paywalls | ❌ Proprietary | ✅ GPL-2.0 |
| **Any GPU (`NVENC` / `AMF` / `QSV`)** | ✅ | ✅ | ❌ Vendor-locked | ✅ |
| **No Electron / Overwolf Bloat** | ✅ Native Rust | ❌ | ✅ | ✅ |
| **RAM Ring Buffer (SSD-friendly)** | ✅ | ❌ Disk | ❌ Disk | ✅ |
| **1-Click Discord `< 10 MB` Export** | ✅ | ❌ Paid / Cloud | ❌ | ❌ |
| **Built-in Library & Lossless Trim** | ✅ | ✅ | ⚠️ Basic | ❌ |
| **No DLL / Overlay Injection** | ✅ | ❌ | ⚠️ Overlay | ⚠️ Game Capture |

## Install

1. Download **`Rewind-Setup-x64.exe`** from [Releases](https://github.com/BioBuch/Rewind/releases/latest).
2. Run the installer and click **Install**.
3. Answer the quick first-run setup questions and follow the short interactive tour.

> [!NOTE]
> Windows SmartScreen may show an unknown-publisher warning on `Rewind-Setup-x64.exe` because the release binary is not yet code-signed.

**Requirements:** Windows 11 (x64) and a DirectX 11 GPU with hardware video encoding (`NVIDIA`, `AMD`, or `Intel`).

## Build from Source

Requires Rust (MSVC toolchain) and Windows SDK build tools:

```powershell
git clone https://github.com/BioBuch/Rewind.git
cd Rewind
cargo build --release --bin rewind --bin rewind-daemon
cargo build --release --bin Rewind-Setup
```

Run tests with:

```powershell
cargo test --lib --bins
```

## Code Signing Policy

Free code signing is provided by [SignPath.io](https://signpath.io), using a certificate by the [SignPath Foundation](https://signpath.org).

- **Committers and Reviewers:** [BioBuch](https://github.com/BioBuch)
- **Approvers:** [BioBuch](https://github.com/BioBuch)
- **Build Provenance:** Official release binaries (`rewind.exe`, `rewind-daemon.exe`, `Rewind-Setup-x64.exe`) are built exclusively on GitHub-hosted Actions runners (`windows-latest`) directly from this repository.

## Disclaimer

Rewind only captures the desktop display using standard Windows APIs (`WGC` / `DXGI Desktop Duplication`) and never reads game memory, injects DLLs, or hooks DirectX. However, because third-party kernel anti-cheats (such as Riot Vanguard or Easy Anti-Cheat) operate at their own discretion and release binaries are currently unsigned, **use Rewind at your own risk — Rewind and its contributors are not responsible for any game or account bans.**

All product names, logos, and brands are property of their respective owners.

## AI Assistance

AI coding tools were used to assist with development. All AI-assisted code and changes are reviewed and verified by a human ([BioBuch](https://github.com/BioBuch)) before being added to the repository.

## Support & License

- Support Rewind on [Ko-fi](https://ko-fi.com/rewindapp).
- Licensed under the [GNU General Public License v3.0](LICENSE).

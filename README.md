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

## Highlights

- **Zero-lag Instant Replay** — Keeps your recent gameplay in RAM via DXGI Desktop Duplication and hardware encoding (`NVENC`, `AMF`, `QuickSync`) up to `4K 120fps`.
- **Synced Game & Mic Audio** — Captures system loopback and microphone in lockstep so audio stays aligned with video.
- **Built-in Clip Editor & Discord Export** — Scrub frame-by-frame, trim losslessly, or compress clips up to `60s` straight under `10 MB` for Discord.
- **Anti-Cheat Safe Game Detection** — Matches foreground window titles and classes only; never opens game process handles or injects hooks.
- **Clean Native UI** — Lightweight Rust + `egui` interface that closes completely when you are done and leaves only the tray daemon running.

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

## Support & License

- Support Rewind on [Ko-fi](https://ko-fi.com/rewindapp).
- Licensed under the [GNU General Public License v3.0](LICENSE).

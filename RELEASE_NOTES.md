## What's new in v1

- Lightweight Windows 11 instant-replay daemon and GPU clip viewer/editor (`Windows.Graphics.Capture` + low-priority GPU thread + pre-allocated physical RAM circular slab + DXVA native-resolution playback).
- Built-in `<10 MB` Discord export (Hardware High-Profile H.264 for short clips, Hardware HEVC 720p30 Peak-Constrained VBR for long clips) with automatic Windows clipboard copy for instant `Ctrl+V` upload.

> Note: Any Windows SmartScreen or Smart App Control warning on `Rewind-Setup-x64.exe` is a false positive because releases are not yet signed with a paid code-signing certificate.

## What's new in v1.01

- **Hardware Encoder CQP & CBR Fix** — Fixed 5 incorrect `ICodecAPI` GUIDs (`AVEncVideoEncodeQP`, `AVEncCommonQuality`, `AVEncMPVGOPSize`, `AVEncMPVDefaultBPictureCount`, `AVEncCommonQualityVsSpeed`) plus CBR `MeanBitRate` / `MaxBitRate` so CQP and bitrate settings genuinely scale hardware encode quality, while keeping GPU usage low.
- **H.264 CQP & 120fps Stability** — Fixed AMD AMF H.264 stalling in CQP mode, shortened async encoder wait budget, and disabled `120 fps` for H.264 (`120 fps` requires H.265 (HEVC) or AV1; H.264 steps down to `60 fps`).
- **Discord Rich Presence** — Built-in native Discord IPC integration (`Playing <Game> with Rewind` · `Instant Replay On` + `Download Rewind` button), configurable under **Settings → Discord Rich Presence**.

> Note: Any Windows SmartScreen or Smart App Control warning on `Rewind-Setup-x64.exe` is a false positive because releases are not yet signed with a paid code-signing certificate.

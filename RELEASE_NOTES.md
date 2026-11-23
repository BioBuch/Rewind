## What's new in v1.02

- **Performance & Memory Optimizations** — Switched release builds to `opt-level = 3` + `panic = "abort"`, switched the replay ring buffer to on-demand page allocation (cutting idle daemon RAM nearly in half), and removed redundant GPU `Flush()` calls on static frames.
- **H.264 CQP & 120fps Stability** — Fixed AMD AMF H.264 stalling in CQP mode, shortened async encoder wait budget, and locked `120 fps` to H.265 (HEVC) or AV1 (H.264 automatically steps to `60 fps`).
- **Discord Rich Presence** — Built-in native Discord IPC integration (`Playing <Game> with Rewind` · `Instant Replay On` + `Download Rewind` button), configurable under **Settings → Discord Rich Presence**.

> Note: Any Windows SmartScreen or Smart App Control warning on `Rewind-Setup-x64.exe` is a false positive because releases are not yet signed with a paid code-signing certificate.

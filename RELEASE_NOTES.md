## What's new in v1.03

- **Windows 11 Settings Redesign** — Two-pane Fluent Settings with search bar, dedicated **Clipping** section (`Clip Length`, `Post-Clip Tail`, `Auto-Start Capture`), smooth animated CQP/CBR swap, auto-hidden Accent Color when Auto-Adjust Palette is enabled, and removed locked Buffer Mode.
- **Post-Clip Tail (`5s / 10s / 15s`)** — Optionally keep recording 5–15 seconds after pressing the clip hotkey so you never miss the end of a play.
- **Memory & Color Improvements** — Early-wrap ring buffer keeps daemon RAM near `~65–85 MB`, plus BT.709 color space on D3D11 VideoProcessor for accurate colors.
- **Player, Trim & UI Polish** — Trim-bounded playback, **Copy Video** file to clipboard, hover-to-rename clip cards with pen icon, circular game badges, SteamGridDB exact-name matching, hidden scrollbars with mouse-drag bounce, `"Don't remind me again"` close prompt, and smooth rounded app icon corners.
- **Discord Rich Presence** — Clean two-line status (`Clipping <Game>` · `with Rewind`).

> Note: Any Windows SmartScreen or Smart App Control warning on `Rewind-Setup-x64.exe` is a false positive because releases are not yet signed with a paid code-signing certificate.

## What's New in v1.1.2 🚀

### 🛠️ Fixes
- 🔊 **Fixed missing desktop & game audio on certain Windows 11 audio setups** (#2): Added native `GetMixFormat()` WASAPI loopback fallback (with automatic surround-to-stereo downmix and 48 kHz resampling) plus `eConsole` default endpoint fallback for drivers/spatial audio setups that reject `AUTOCONVERTPCM`.
- 🎨 **Fixed startup theme color delay**: App now loads saved accent color and wallpaper settings immediately on frame 0 without waiting for mouse movement or overwriting custom accent colors when Auto-Adjust Palette is off.

### ✨ Improvements
- 🌈 **Circular theme reveal**: Added a smooth expanding circular transition when switching accent colors, toggling Auto-Adjust Palette, or changing/resetting your background wallpaper.
- ❌ **Cancel Discord `<10 MB` encode** (#1): Added a **Cancel** button directly on the encoding toast to abort accidental exports immediately and clean up temp files.
- 🎮 **Discord Rich Presence overhaul**:
  - Dynamically displays `<Game> With Rewind` when capturing a game.
  - Shows `Rewind` / `Idle on Desktop` when capturing Desktop, and `Idle · Ready to Clip` when idle.
  - Automatically resets the elapsed timer when switching games or returning to Desktop/Idle.
- 🎛️ **Personalization & Clipping UI polish**:
  - Converted **Post-Clip Tail** to a clean dropdown matching **Clip Length** (`Instant`, `5 seconds`, `10 seconds`, `15 seconds`).
  - Smoothly animated all toggle switches on/off.
  - Disabled **Auto-Adjust Palette from Background** until a background image is selected.

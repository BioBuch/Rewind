# 🐛 Rewind v1.1.1 — 1.1 Bugfix Update 🔧

### 🎥 Capture, Remux & Trim Fixes
- **Accurate 120fps Metadata 📊**: `probe_mp4_metadata` now calculates FPS from median sample duration (`stts`) instead of wall duration, fixing wrong FPS badges when the GPU/OS briefly stalls.
- **MKV Recording & Seeking Fix 🎞️**: `mux_packets_to_mkv` now normalizes timestamps relative to clip start, splits clusters every `20s` (fixing the `32.7s` `i16::MAX` limit), and writes atomically via `.mkv.tmp`.
- **Non-Blocking Quick Trim ✂️**: `TrimClip` now releases the daemon lock during disk remux and thumbnail extraction so live instant replay capture never stalls, and returns a clean message for MKV clips.

### 🔊 Audio, Player & UI Fixes
- **Selected Game Audio Device 🎧**: Actually uses your configured `game_audio_device` via WASAPI loopback endpoint matching (with automatic retry on device changes).
- **Smoother Player & Instant Mute 🔇**: Deduplicated per-frame decoder commands, flushed queued `waveOut` audio immediately on mute, restored `0.85` volume when unmuting from `0`, and aligned fullscreen controls to the video frame on ultrawide monitors.
- **Preserved Modal Backdrop Blur 🌫️**: Fixed `<` / `>` clip navigation clearing `modal_blur_tex` and re-capturing the open modal into the blur.
- **No Click/Scroll-Through Behind Modals 🖱️**: Disabled library scrolling and clicks underneath open modals and popovers.

### 🗂️ Library, Tray & Discord RPC
- **Safe NTFS Case Renames & Reserved Names 📝**: Case-only clip renames no longer append `_{id}`, and Windows reserved stems (`CON`, `PRN`, `AUX`, `NUL`, `COM*`, `LPT*`) are safely suffixed.
- **Tray & Mic Hotkey State Sync 🔄**: Toggling Replay from the tray clears the ring buffer cleanly, and toggling mic mute via hotkey persists `mic_enabled`.
- **Discord Rich Presence Reliability 🎮**: Validates Discord IPC frame responses to reconnect dead pipes automatically and honors custom `discord_app_id` settings.

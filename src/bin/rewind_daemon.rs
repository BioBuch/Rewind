#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rewind::discord_rpc::{build_activity_strings, DiscordRpcClient};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, IDI_APPLICATION,
    MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP, WM_CLOSE, WM_DESTROY, WM_HOTKEY,
    WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use rewind::app_icon::render_icon_rgba;
use rewind::audio::{
    enumerate_input_devices, enumerate_output_devices, play_clip_chime, WasapiTrackStream,
};
use rewind::capture::{detect_hardware, GpuCapturePipeline, HardwareInfo};
use rewind::config::{AppConfig, BufferMode, VideoCodec};
use rewind::game_detector::{
    detect_foreground_game, install_foreground_hook, is_window_alive, take_foreground_changed,
};
use rewind::ipc::{
    run_pipe_server, send_request, DaemonEvent, DaemonSnapshot, IpcRequest, IpcResponse, PIPE_NAME,
};
use rewind::library::{total_disk_usage_bytes, ClipLibrary};
use rewind::remuxer::{mux_packets_to_file, probe_mp4_metadata, trim_mp4_lossless};

fn sanitize_config_for_hardware(cfg: &mut AppConfig, hw: &HardwareInfo) {
    cfg.encoder_preference = hw.native_encoder();
    if !hw.is_codec_supported(cfg.codec) {
        cfg.codec = if hw.supports_hevc {
            VideoCodec::Hevc
        } else {
            VideoCodec::H264
        };
    }
    cfg.buffer_mode = BufferMode::SystemRam;
}

use rewind::ring_buffer::{CompressedRingBuffer, PacketTrack};
use rewind::win_ffi::{
    create_hicon_from_rgba, current_process_ram_mb, destroy_hicon, is_rewind_ui_open,
    is_start_with_windows_enabled, parse_hotkey_combo, pick_folder_dialog, qpc_now_100ns,
    reveal_in_explorer, set_start_with_windows, to_wide_null, unix_now_secs, HighResTimer,
};

const WM_TRAY_ICON: u32 = WM_APP + 1;
const WM_UPDATE_TRAY_ICON: u32 = WM_APP + 2;
const WM_REBIND_HOTKEYS: u32 = WM_APP + 3;
const HK_ID_SAVE: i32 = 1;
const HK_ID_MIC: i32 = 2;
const HK_ID_BOOKMARK: i32 = 3;
const MENU_OPEN_UI: usize = 1001;
const MENU_SAVE_CLIP: usize = 1002;
const MENU_TOGGLE_REPLAY: usize = 1003;
const MENU_EXIT_DAEMON: usize = 1004;

struct DaemonCore {
    config: AppConfig,
    hardware: HardwareInfo,
    library: ClipLibrary,
    ring_buffer: CompressedRingBuffer,
    active_game: Option<String>,
    last_detected_game: Option<String>,
    manual_source: Option<String>,
    capture_active: bool,
    audio_outputs: Vec<String>,
    audio_inputs: Vec<String>,
    recent_events: Vec<DaemonEvent>,
    pending_undo: Option<(i64, i64)>,
    clip_flash_counter: u64,
    auto_close_ui_counter: u64,
}

impl DaemonCore {
    fn new() -> Self {
        let app_dir = AppConfig::app_data_dir();
        let had_cfg = app_dir.join("config.toml").exists();
        let mut config = AppConfig::load_from_dir(&app_dir);
        if !had_cfg {
            config.start_with_windows = is_start_with_windows_enabled();
        }
        let hardware = detect_hardware();
        sanitize_config_for_hardware(&mut config, &hardware);
        let db_path = app_dir.join("clips.db");
        let library = ClipLibrary::open(&db_path).expect("Failed to open SQLite clips database");
        let _ = library.sync_folder(Path::new(&config.clips_folder));

        let ring_cap_bytes = config.ring_buffer_budget_bytes();
        let ring_buffer = CompressedRingBuffer::new(config.clip_length.seconds(), ring_cap_bytes);
        let audio_outputs = enumerate_output_devices();
        let audio_inputs = enumerate_input_devices();
        let initial_game =
            detect_foreground_game(&config.custom_games, &config.ignored_games);
        let force_desktop = std::env::args().any(|a| a == "--source=Desktop");
        let initial_manual = if config.auto_start_capture || force_desktop {
            if config.auto_start_capture {
                config.replay_on = true;
            }
            Some("Desktop".to_owned())
        } else {
            None
        };

        Self {
            config,
            hardware,
            library,
            ring_buffer,
            active_game: initial_game.as_ref().map(|g| g.display_name.clone()),
            last_detected_game: initial_game.as_ref().map(|g| g.display_name.clone()),
            manual_source: initial_manual,
            capture_active: false,
            audio_outputs,
            audio_inputs,
            recent_events: Vec::new(),
            pending_undo: None,
            clip_flash_counter: 0,
            auto_close_ui_counter: 0,
        }
    }

    fn push_event(&mut self, msg: impl Into<String>) {
        self.recent_events.insert(
            0,
            DaemonEvent {
                message: msg.into(),
                unix_secs: unix_now_secs(),
            },
        );
        self.recent_events.truncate(12);
    }

    fn snapshot(&mut self) -> DaemonSnapshot {
        let now = unix_now_secs();
        if let Some((_, deleted_at)) = self.pending_undo {
            if now - deleted_at >= 8 {
                let _ = self.library.purge_expired_deletes(now, 8);
                self.pending_undo = None;
            }
        }
        let clips = self.library.list_active_clips().unwrap_or_default();
        let used_storage_bytes = total_disk_usage_bytes(Path::new(&self.config.clips_folder));
        DaemonSnapshot {
            config: self.config.clone(),
            hardware: self.hardware.clone(),
            replay_on: self.config.replay_on,
            capture_active: self.capture_active,
            active_game: self.active_game.clone(),
            active_pid: None,
            manual_source: self.manual_source.clone(),
            ring_buffer_bytes: self.ring_buffer.total_bytes(),
            clips,
            used_storage_bytes,
            audio_outputs: self.audio_outputs.clone(),
            audio_inputs: self.audio_inputs.clone(),
            recent_events: self.recent_events.clone(),
            pending_undo_clip_id: self.pending_undo.map(|(id, _)| id),
            daemon_ram_mb: current_process_ram_mb(),
            clip_flash_counter: self.clip_flash_counter,
            auto_close_ui_counter: self.auto_close_ui_counter,
            daemon_pid: std::process::id(),
        }
    }
}

fn spawn_ui_host() {
    if let Ok(cur) = std::env::current_exe() {
        let ui_exe = cur.with_file_name("rewind.exe");
        if ui_exe.exists() {
            let _ = Command::new(ui_exe).spawn();
        }
    }
}

fn trigger_save_clip(core: &Arc<Mutex<DaemonCore>>) -> Result<String, String> {
    let post_secs = {
        let mut guard = core.lock().map_err(|_| "Mutex poisoned".to_owned())?;
        let has_capture_source =
            guard.active_game.is_some() || guard.manual_source.as_deref() == Some("Desktop");
        if !has_capture_source {
            return Err("No capture source active — launch a game or select Desktop.".to_owned());
        }
        let post_secs = if guard.config.replay_on {
            guard.config.clip_post_buffer_secs.min(15)
        } else {
            0
        };
        if post_secs > 0 {
            guard.clip_flash_counter += 1;
            guard.push_event(format!("Capturing {post_secs}s post-clip..."));
        }
        post_secs
    };

    if post_secs > 0 {
        play_clip_chime();
        let core_bg = core.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(post_secs as u64));
            if let Err(err) = save_clip_immediate(&core_bg, false) {
                if let Ok(mut g) = core_bg.lock() {
                    g.push_event(format!("Save failed: {err}"));
                }
            }
        });
        return Ok(format!("Saving clip in {post_secs}s..."));
    }

    save_clip_immediate(core, true)
}

fn save_clip_immediate(core: &Arc<Mutex<DaemonCore>>, play_feedback: bool) -> Result<String, String> {
    let (mut packets, cfg, hw, game_name) = {
        let mut guard = core.lock().map_err(|_| "Mutex poisoned".to_owned())?;
        let has_capture_source =
            guard.active_game.is_some() || guard.manual_source.as_deref() == Some("Desktop");
        if !has_capture_source {
            return Err("No capture source active — launch a game or select Desktop.".to_owned());
        }
        if play_feedback {
            guard.clip_flash_counter += 1;
        }
        let packets = guard
            .ring_buffer
            .snapshot_from_nearest_idr(guard.config.clip_length.seconds());
        let game = guard
            .active_game
            .clone()
            .or_else(|| guard.last_detected_game.clone())
            .or_else(|| guard.manual_source.clone())
            .unwrap_or_else(|| "Desktop".to_owned());
        (
            packets,
            guard.config.clone(),
            guard.hardware.clone(),
            game,
        )
    };

    if play_feedback {
        play_clip_chime();
    }

    if cfg.replay_on
        && !packets
            .iter()
            .any(|p| p.track == PacketTrack::Video && p.is_idr_keyframe)
    {
        for _ in 0..25 {
            thread::sleep(Duration::from_millis(35));
            if let Ok(guard) = core.lock() {
                packets = guard
                    .ring_buffer
                    .snapshot_from_nearest_idr(guard.config.clip_length.seconds());
                if packets
                    .iter()
                    .any(|p| p.track == PacketTrack::Video && p.is_idr_keyframe)
                {
                    break;
                }
            }
        }
    }

    let packets = if packets.iter().filter(|p| p.track == PacketTrack::Video).count() == 0 {
        let mut temp_pipe = GpuCapturePipeline::open(&cfg, &hw)
            .ok_or_else(|| "Unable to open D3D11/DXGI capture pipeline".to_owned())?;
        let mut temp_ga = WasapiTrackStream::open_loopback(&cfg.game_audio_device);
        let mut temp_ma = WasapiTrackStream::open_microphone(&cfg.mic_device);
        let mut live_pkts = Vec::new();
        for _ in 0..45 {
            live_pkts.extend(temp_pipe.capture_and_encode_frame());
            if let Some(ga) = &mut temp_ga {
                live_pkts.extend(ga.drain_packets(cfg.game_audio_volume, false));
            }
            if let Some(ma) = &mut temp_ma {
                live_pkts.extend(ma.drain_packets(cfg.mic_volume, !cfg.mic_enabled));
            }
            thread::sleep(Duration::from_millis(25));
        }
        if !live_pkts.iter().any(|p| p.track == PacketTrack::Video) {
            return Err("No game/desktop frames captured yet. Turn Replay On while a game or display is active.".to_owned());
        }
        live_pkts
    } else {
        packets
    };

    let now = unix_now_secs();
    let safe_game = rewind::library::sanitize_filename(&game_name);
    let game_dir = PathBuf::from(&cfg.clips_folder).join(&safe_game);
    let file_name = format!("{safe_game}_{now}.{}", cfg.container.ext());
    let out_path = game_dir.join(file_name);
    let (w, h) = cfg.resolution.dimensions();
    let fps = cfg.fps.as_u32();
    let codec = cfg.codec;
    let container = cfg.container;
    let sep_audio = cfg.separate_audio_tracks;
    let title = format!("{game_name} Clip {now}");

    let core_bg = core.clone();
    let out_clone = out_path.clone();
    let title_clone = title.clone();
    let game_clone = game_name.clone();

    thread::spawn(move || {
        let res = mux_packets_to_file(
            &out_clone,
            &packets,
            w,
            h,
            fps,
            codec,
            container,
            sep_audio,
        );
        let probed_dur = if res.is_ok() {
            let span_secs = match (
                packets.iter().find(|p| p.track == PacketTrack::Video),
                packets.iter().rfind(|p| p.track == PacketTrack::Video),
            ) {
                (Some(first), Some(last)) => {
                    ((last.pts_100ns + last.duration_100ns - first.pts_100ns) as f32) / 10_000_000.0
                }
                _ => 0.5,
            };
            probe_mp4_metadata(&out_clone)
                .map(|m| m.duration_secs)
                .unwrap_or(span_secs)
                .max(0.5)
        } else {
            0.5
        };
        if let Ok(mut g) = core_bg.lock() {
            match res {
                Ok(()) => {
                    let _ = g.library.register_clip(
                        &out_clone,
                        Path::new(&g.config.clips_folder),
                        &title_clone,
                        &game_clone,
                        probed_dur,
                        w,
                        h,
                        fps,
                        codec.short_name(),
                        now,
                    );
                    let max_bytes = (g.config.max_storage_gb as u64) * 1024 * 1024 * 1024;
                    let clips_root = PathBuf::from(&g.config.clips_folder);
                    let auto_del = g.config.auto_delete_oldest;
                    let _ = g.library.enforce_quota(&clips_root, max_bytes, auto_del);
                    g.push_event(format!("Saved clip: {title_clone}"));
                }
                Err(e) => {
                    g.push_event(format!("Clip save failed: {e}"));
                }
            }
        }
    });

    Ok(format!("Saving {title}..."))
}

fn handle_ipc_request(
    req: IpcRequest,
    core: &Arc<Mutex<DaemonCore>>,
    stop: &Arc<AtomicBool>,
) -> IpcResponse {
    match req {
        IpcRequest::GetSnapshot => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            IpcResponse::ok_snapshot(g.snapshot(), None)
        }
        IpcRequest::SetReplay(on) => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            g.config.replay_on = on;
            let _ = g.config.save_to_dir(&AppConfig::app_data_dir());
            if !on {
                g.ring_buffer.clear();
                g.capture_active = false;
            }
            g.push_event(if on { "Instant Replay enabled" } else { "Instant Replay paused" });
            IpcResponse::ok_snapshot(g.snapshot(), None)
        }
        IpcRequest::SetCaptureSource(src) => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            if g.manual_source == src {
                return IpcResponse::ok_snapshot(g.snapshot(), None);
            }
            g.manual_source = src;
            IpcResponse::ok_snapshot(g.snapshot(), Some("Capture source updated".to_owned()))
        }
        IpcRequest::SaveClipNow => match trigger_save_clip(core) {
            Ok(msg) => {
                thread::sleep(Duration::from_millis(80));
                let Ok(mut g) = core.lock() else {
                    return IpcResponse::err("Daemon lock failed");
                };
                IpcResponse::ok_snapshot(g.snapshot(), Some(msg))
            }
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::UpdateConfig(mut new_cfg) => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            let prev_auto = g.config.auto_start_capture;
            let prev_accent = g.config.effective_accent_color();
            let prev_start_win = g.config.start_with_windows;
            sanitize_config_for_hardware(&mut new_cfg, &g.hardware);
            let hotkeys_changed = g.config.hotkey_save_clip != new_cfg.hotkey_save_clip
                || g.config.hotkey_toggle_mic != new_cfg.hotkey_toggle_mic
                || g.config.hotkey_add_bookmark != new_cfg.hotkey_add_bookmark;

            if new_cfg.auto_start_capture && !prev_auto && g.manual_source.is_none() {
                g.manual_source = Some("Desktop".to_owned());
                new_cfg.replay_on = true;
            } else if !new_cfg.auto_start_capture
                && prev_auto
                && g.manual_source.as_deref() == Some("Desktop")
            {
                g.manual_source = None;
            }
            let folder_changed = g.config.clips_folder != new_cfg.clips_folder;
            let video_stream_changed = g.config.resolution != new_cfg.resolution
                || g.config.fps != new_cfg.fps
                || g.config.codec != new_cfg.codec;
            g.ring_buffer
                .set_budget(new_cfg.clip_length.seconds(), new_cfg.ring_buffer_budget_bytes());
            if video_stream_changed {
                g.ring_buffer.clear();
            }
            let new_accent = new_cfg.effective_accent_color();
            let new_start_win = new_cfg.start_with_windows;
            g.config = new_cfg;
            let _ = g.config.save_to_dir(&AppConfig::app_data_dir());
            if prev_start_win != new_start_win {
                if let Ok(exe) = std::env::current_exe() {
                    set_start_with_windows(new_start_win, &exe);
                }
            }
            if folder_changed {
                let _ = g.library.sync_folder(Path::new(&g.config.clips_folder));
            }
            let max_bytes = (g.config.max_storage_gb as u64) * 1024 * 1024 * 1024;
            let clips_root = PathBuf::from(&g.config.clips_folder);
            let auto_del = g.config.auto_delete_oldest;
            let _ = g.library.enforce_quota(&clips_root, max_bytes, auto_del);

            if new_accent != prev_accent {
                let hwnd_val = TRAY_HWND.load(Ordering::Acquire);
                if hwnd_val != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            HWND(hwnd_val as *mut _),
                            WM_UPDATE_TRAY_ICON,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            if hotkeys_changed {
                let hwnd_val = TRAY_HWND.load(Ordering::Acquire);
                if hwnd_val != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            HWND(hwnd_val as *mut _),
                            WM_REBIND_HOTKEYS,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            IpcResponse::ok_snapshot(g.snapshot(), Some("Settings applied".to_owned()))
        }
        IpcRequest::ToggleFavorite { id } => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            match g.library.toggle_favorite(id) {
                Ok(_) => IpcResponse::ok_snapshot(g.snapshot(), None),
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcRequest::RenameClip { id, title } => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            match g.library.rename_clip(id, &title) {
                Ok(rec) => {
                    g.push_event(format!("Renamed clip to {}", rec.title));
                    IpcResponse::ok_snapshot(g.snapshot(), Some("Clip renamed".to_owned()))
                }
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcRequest::SetCollection { id, collection } => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            match g.library.set_collection(id, collection.as_deref()) {
                Ok(()) => IpcResponse::ok_snapshot(g.snapshot(), None),
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcRequest::DeleteClip { id } => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            let now = unix_now_secs();
            if let Some((prev_id, _)) = g.pending_undo.take() {
                let _ = g.library.purge_expired_deletes(now + 100, 8);
                let _ = prev_id;
            }
            match g.library.soft_delete_clip(id, now) {
                Ok(()) => {
                    g.pending_undo = Some((id, now));
                    IpcResponse::ok_snapshot(g.snapshot(), Some("Clip deleted (Undo available)".to_owned()))
                }
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcRequest::UndoDelete { id } => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            match g.library.undo_delete_clip(id) {
                Ok(()) => {
                    g.pending_undo = None;
                    IpcResponse::ok_snapshot(g.snapshot(), Some("Clip restored".to_owned()))
                }
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcRequest::TrimClip {
            id,
            start_secs,
            end_secs,
            vertical_9_16,
        } => {
            let (src_rec, clips_folder) = {
                let Ok(g) = core.lock() else {
                    return IpcResponse::err("Daemon lock failed");
                };
                let Some(rec) = g.library.get_by_id(id).ok().flatten() else {
                    return IpcResponse::err("Source clip not found");
                };
                (rec, PathBuf::from(&g.config.clips_folder))
            };
            let src_path = PathBuf::from(&src_rec.file_path);
            if src_path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("mkv"))
                .unwrap_or(false)
            {
                return IpcResponse::err("Lossless Quick Trim requires an MP4 clip");
            }
            let stem = src_path.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
            let suffix = if vertical_9_16 { "9x16_trim" } else { "trim" };
            let created_unix = unix_now_secs();
            let dst_path = src_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{stem}_{suffix}_{created_unix}.mp4"));

            match trim_mp4_lossless(&src_path, &dst_path, start_secs, end_secs, vertical_9_16) {
                Ok(meta) => {
                    let thumb_path = rewind::library::thumb_path_for(&clips_folder, &dst_path);
                    if let Some(parent) = thumb_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                        rewind::win_ffi::hide_path(parent);
                    }
                    if !thumb_path.exists() {
                        let seek = (meta.duration_secs * 0.15).min(0.5);
                        let _ = rewind::player::extract_mp4_thumbnail_jpg(&dst_path, &thumb_path, seek);
                    }
                    let new_title = if vertical_9_16 {
                        format!("{} (9:16)", src_rec.title)
                    } else {
                        format!("{} (Trim)", src_rec.title)
                    };
                    let Ok(mut g) = core.lock() else {
                        return IpcResponse::err("Daemon lock failed");
                    };
                    let _ = g.library.register_clip(
                        &dst_path,
                        &clips_folder,
                        &new_title,
                        &src_rec.game,
                        meta.duration_secs,
                        meta.width,
                        meta.height,
                        meta.fps,
                        &meta.codec,
                        created_unix,
                    );
                    g.push_event(format!("Exported lossless trim: {new_title}"));
                    IpcResponse::ok_snapshot(g.snapshot(), Some(format!("Saved {new_title}")))
                }
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcRequest::PickFolder => {
            let initial = {
                core.lock()
                    .map(|g| g.config.clips_folder.clone())
                    .unwrap_or_default()
            };
            if let Some(picked) = pick_folder_dialog(&initial) {
                let Ok(mut g) = core.lock() else {
                    return IpcResponse::err("Daemon lock failed");
                };
                g.config.clips_folder = picked.clone();
                let _ = g.config.save_to_dir(&AppConfig::app_data_dir());
                let _ = g.library.sync_folder(Path::new(&picked));
                let mut resp = IpcResponse::ok_snapshot(g.snapshot(), Some("Clips folder updated".to_owned()));
                resp.picked_folder = Some(picked);
                resp
            } else {
                let Ok(mut g) = core.lock() else {
                    return IpcResponse::err("Daemon lock failed");
                };
                IpcResponse::ok_snapshot(g.snapshot(), None)
            }
        }
        IpcRequest::RevealClip { id } => {
            let fp = {
                core.lock()
                    .ok()
                    .and_then(|g| g.library.get_by_id(id).ok().flatten())
                    .map(|r| r.file_path)
            };
            if let Some(path) = fp {
                reveal_in_explorer(&path);
                let Ok(mut g) = core.lock() else {
                    return IpcResponse::err("Daemon lock failed");
                };
                IpcResponse::ok_snapshot(g.snapshot(), None)
            } else {
                IpcResponse::err("Clip not found")
            }
        }
        IpcRequest::SyncLibrary => {
            let Ok(mut g) = core.lock() else {
                return IpcResponse::err("Daemon lock failed");
            };
            let folder = PathBuf::from(&g.config.clips_folder);
            let _ = g.library.sync_folder(&folder);
            g.audio_outputs = enumerate_output_devices();
            g.audio_inputs = enumerate_input_devices();
            IpcResponse::ok_snapshot(g.snapshot(), None)
        }
        IpcRequest::ShutdownDaemon => {
            stop.store(true, Ordering::Release);
            let hwnd_val = TRAY_HWND.load(Ordering::Acquire);
            if hwnd_val != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(hwnd_val as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            IpcResponse {
                ok: true,
                message: Some("Daemon shutting down".to_owned()),
                picked_folder: None,
                snapshot: None,
                restarting: false,
            }
        }
    }
}

fn run_capture_loop(core: Arc<Mutex<DaemonCore>>, stop: Arc<AtomicBool>) {
    let mut pipeline: Option<GpuCapturePipeline> = None;
    let mut active_video_cfg: Option<(
        u32,
        u32,
        u32,
        VideoCodec,
        rewind::config::RateControl,
        u32,
        u32,
        u32,
        rewind::config::EncoderPreference,
        rewind::config::SharpenLevel,
    )> = None;
    let mut game_audio: Option<WasapiTrackStream> = None;
    let mut mic_audio: Option<WasapiTrackStream> = None;
    let mut active_game_audio_device = String::new();
    let mut active_mic_device = String::new();
    let mut last_audio_retry = Instant::now() - Duration::from_secs(5);
    let mut tracked_game_hwnd: Option<isize> = None;
    let mut ui_closed_for_game = false;

    let mut last_fg_poll = Instant::now();
    let mut last_cfg_refresh = Instant::now() - Duration::from_secs(1);
    let mut cached_should_capture = false;
    let mut cached_cfg: Option<AppConfig> = None;
    let mut cached_hw: Option<HardwareInfo> = None;
    let pace_timer = HighResTimer::new();
    let mut pace_origin_qpc: i64 = 0;
    let mut pace_frame_count: u64 = 0;
    while !stop.load(Ordering::Acquire) {
        if take_foreground_changed() || last_fg_poll.elapsed() >= Duration::from_millis(350) {
            last_fg_poll = Instant::now();
            if let Some(ghwnd) = tracked_game_hwnd {
                if !is_window_alive(ghwnd) {
                    tracked_game_hwnd = None;
                    if let Ok(mut g) = core.lock() {
                        g.active_game = None;
                        g.last_detected_game = None;
                    }
                    if ui_closed_for_game {
                        ui_closed_for_game = false;
                        spawn_ui_host();
                    }
                }
            }

            let (custom_games, ignored_games) = core
                .lock()
                .ok()
                .map(|g| (g.config.custom_games.clone(), g.config.ignored_games.clone()))
                .unwrap_or_default();
            let detected = detect_foreground_game(&custom_games, &ignored_games);
            if let Some(dg) = detected {
                tracked_game_hwnd = Some(dg.hwnd);
                if let Ok(mut g) = core.lock() {
                    g.active_game = Some(dg.display_name.clone());
                    g.last_detected_game = Some(dg.display_name);
                    if g.config.close_ui_on_game_launch
                        && !ui_closed_for_game
                        && is_rewind_ui_open()
                    {
                        g.auto_close_ui_counter += 1;
                        ui_closed_for_game = true;
                    }
                }
            } else if tracked_game_hwnd.is_none() {
                if let Ok(mut g) = core.lock() {
                    g.active_game = None;
                    g.last_detected_game = None;
                }
            }
        }

        if cached_cfg.is_none()
            || !cached_should_capture
            || last_cfg_refresh.elapsed() >= Duration::from_millis(200)
        {
            if let Ok(mut g) = core.lock() {
                let has_capture_source =
                    g.active_game.is_some() || g.manual_source.as_deref() == Some("Desktop");
                let active = g.config.replay_on && has_capture_source;
                g.capture_active = active;
                cached_should_capture = active;
                cached_cfg = Some(g.config.clone());
                cached_hw = Some(g.hardware.clone());
                last_cfg_refresh = Instant::now();
            } else {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        }

        let Some(cfg) = cached_cfg.as_ref() else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        let Some(hw) = cached_hw.as_ref() else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };

        if !cached_should_capture {
            if pipeline.is_some() || game_audio.is_some() || mic_audio.is_some() {
                pipeline = None;
                active_video_cfg = None;
                game_audio = None;
                mic_audio = None;
                active_game_audio_device.clear();
                active_mic_device.clear();
                pace_origin_qpc = 0;
                pace_frame_count = 0;
                if let Ok(mut g) = core.lock() {
                    g.ring_buffer.clear();
                }
            }
            thread::sleep(Duration::from_millis(120));
            continue;
        }

        let (cw, ch) = cfg.resolution.dimensions();
        let vid_cfg = (
            cw,
            ch,
            cfg.fps.as_u32(),
            cfg.codec,
            cfg.rate_control,
            cfg.cqp_qp,
            cfg.bitrate_mbps,
            cfg.gop_secs,
            cfg.encoder_preference,
            cfg.capture_sharpen,
        );
        if pipeline.is_none() || active_video_cfg != Some(vid_cfg) {
            drop(pipeline.take());
            if let Ok(mut g) = core.lock() {
                if active_video_cfg != Some(vid_cfg) {
                    g.ring_buffer.clear();
                }
                g.ring_buffer
                    .set_budget(cfg.clip_length.seconds(), cfg.ring_buffer_budget_bytes());
                g.ring_buffer.preallocate();
            }
            pipeline = GpuCapturePipeline::open(cfg, hw);
            active_video_cfg = Some(vid_cfg);
            pace_origin_qpc = 0;
            pace_frame_count = 0;
        }

        let retry_due = last_audio_retry.elapsed() >= Duration::from_secs(2);
        let ga_changed = active_game_audio_device != cfg.game_audio_device;
        let mic_changed = active_mic_device != cfg.mic_device;
        if ga_changed || (game_audio.is_none() && retry_due) {
            game_audio = WasapiTrackStream::open_loopback(&cfg.game_audio_device);
            active_game_audio_device = cfg.game_audio_device.clone();
            last_audio_retry = Instant::now();
        }
        if mic_changed || (mic_audio.is_none() && retry_due) {
            mic_audio = WasapiTrackStream::open_microphone(&cfg.mic_device);
            active_mic_device = cfg.mic_device.clone();
            last_audio_retry = Instant::now();
        }

        let mut new_packets = Vec::new();
        if let Some(pipe) = &mut pipeline {
            new_packets.extend(pipe.capture_and_encode_frame());
        }
        if let Some(ga) = &mut game_audio {
            new_packets.extend(ga.drain_packets(cfg.game_audio_volume, false));
        }
        if let Some(ma) = &mut mic_audio {
            new_packets.extend(ma.drain_packets(cfg.mic_volume, !cfg.mic_enabled));
        }

        if !new_packets.is_empty() {
            if let Ok(mut g) = core.lock() {
                for pkt in new_packets {
                    g.ring_buffer.push(pkt);
                }
            }
        }

        let fps = cfg.fps.as_u32().max(1);
        let frame_interval_100ns = (10_000_000_i64) / (fps as i64);
        let now_qpc = qpc_now_100ns();
        if pace_origin_qpc == 0 {
            pace_origin_qpc = now_qpc;
            pace_frame_count = 0;
        }
        pace_frame_count += 1;
        let mut target_qpc =
            pace_origin_qpc + (((pace_frame_count as i128) * 10_000_000) / (fps as i128)) as i64;
        if target_qpc <= now_qpc - frame_interval_100ns * 4
            || target_qpc > now_qpc + frame_interval_100ns * 4
        {
            pace_origin_qpc = now_qpc;
            pace_frame_count = 1;
            target_qpc = now_qpc + frame_interval_100ns;
        }
        let rem_100ns = target_qpc - qpc_now_100ns();
        if rem_100ns > 1_000 {
            if let Some(t) = &pace_timer {
                t.wait_100ns(rem_100ns);
            } else {
                thread::sleep(Duration::from_micros((rem_100ns / 10) as u64));
            }
        }
    }
}

unsafe fn register_all_hotkeys(hwnd: HWND, cfg: &AppConfig) {
    for id in [HK_ID_SAVE, HK_ID_MIC, HK_ID_BOOKMARK] {
        let _ = UnregisterHotKey(hwnd, id);
    }
    for (id, combo) in [
        (HK_ID_SAVE, cfg.hotkey_save_clip.as_str()),
        (HK_ID_MIC, cfg.hotkey_toggle_mic.as_str()),
        (HK_ID_BOOKMARK, cfg.hotkey_add_bookmark.as_str()),
    ] {
        if let Some((mods, vk)) = parse_hotkey_combo(combo) {
            let _ = RegisterHotKey(hwnd, id, HOT_KEY_MODIFIERS(mods), vk);
        }
    }
}

static TRAY_CORE: OnceLock<Arc<Mutex<DaemonCore>>> = OnceLock::new();
static TRAY_STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static TRAY_HICON: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_TRAY_ICON {
        let mouse_msg = lparam.0 as u32;
        if mouse_msg == WM_LBUTTONDBLCLK {
            spawn_ui_host();
            return LRESULT(0);
        } else if mouse_msg == WM_RBUTTONUP {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            if let Ok(menu) = CreatePopupMenu() {
                let open_w = to_wide_null("Open Rewind");
                let save_w = to_wide_null("Save Clip Now");
                let toggle_w = to_wide_null("Toggle Replay On/Off");
                let exit_w = to_wide_null("Exit Rewind Daemon");
                let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_UI, PCWSTR(open_w.as_ptr()));
                let _ = AppendMenuW(menu, MF_STRING, MENU_SAVE_CLIP, PCWSTR(save_w.as_ptr()));
                let _ = AppendMenuW(menu, MF_STRING, MENU_TOGGLE_REPLAY, PCWSTR(toggle_w.as_ptr()));
                let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT_DAEMON, PCWSTR(exit_w.as_ptr()));

                let _ = SetForegroundWindow(hwnd);
                let cmd = TrackPopupMenu(
                    menu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN | windows::Win32::UI::WindowsAndMessaging::TPM_RETURNCMD,
                    pt.x,
                    pt.y,
                    0,
                    hwnd,
                    None,
                );
                let _ = DestroyMenu(menu);

                match cmd.0 as usize {
                    MENU_OPEN_UI => spawn_ui_host(),
                    MENU_SAVE_CLIP => {
                        if let Some(core) = TRAY_CORE.get() {
                            let _ = trigger_save_clip(core);
                        }
                    }
                    MENU_TOGGLE_REPLAY => {
                        if let Some(core) = TRAY_CORE.get() {
                            if let Ok(mut g) = core.lock() {
                                let on = !g.config.replay_on;
                                g.config.replay_on = on;
                                let _ = g.config.save_to_dir(&AppConfig::app_data_dir());
                                if !on {
                                    g.ring_buffer.clear();
                                    g.capture_active = false;
                                }
                                g.push_event(if on {
                                    "Instant Replay enabled"
                                } else {
                                    "Instant Replay paused"
                                });
                            }
                        }
                    }
                    MENU_EXIT_DAEMON => {
                        if let Some(stop) = TRAY_STOP.get() {
                            stop.store(true, Ordering::Release);
                        }
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            return LRESULT(0);
        }
    } else if msg == WM_UPDATE_TRAY_ICON {
        if let Some(core) = TRAY_CORE.get() {
            let accent = core.lock().ok().map(|g| g.config.effective_accent_color());
            if let Some(accent) = accent {
                let rgba = render_icon_rgba(accent, 64);
                if let Some(hicon) = create_hicon_from_rgba(&rgba, 64, 64) {
                    let nid = NOTIFYICONDATAW {
                        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                        hWnd: hwnd,
                        uID: 1,
                        uFlags: NIF_ICON,
                        hIcon: hicon,
                        ..Default::default()
                    };
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
                    let prev = TRAY_HICON.swap(hicon.0 as isize, Ordering::AcqRel);
                    if prev != 0 {
                        destroy_hicon(windows::Win32::UI::WindowsAndMessaging::HICON(
                            prev as *mut _,
                        ));
                    }
                }
            }
        }
        return LRESULT(0);
    } else if msg == WM_HOTKEY {
        match wparam.0 as i32 {
            HK_ID_SAVE => {
                if let Some(core) = TRAY_CORE.get() {
                    let core_c = core.clone();
                    thread::spawn(move || {
                        let _ = trigger_save_clip(&core_c);
                    });
                }
            }
            HK_ID_MIC => {
                if let Some(core) = TRAY_CORE.get() {
                    if let Ok(mut g) = core.lock() {
                        g.config.mic_enabled = !g.config.mic_enabled;
                        let _ = g.config.save_to_dir(&AppConfig::app_data_dir());
                        let state_str = if g.config.mic_enabled { "unmuted" } else { "muted" };
                        g.push_event(format!("Microphone {state_str}"));
                    }
                }
            }
            HK_ID_BOOKMARK => {
                if let Some(core) = TRAY_CORE.get() {
                    if let Ok(mut g) = core.lock() {
                        g.push_event("Highlight bookmark added to ring buffer");
                    }
                }
            }
            _ => {}
        }
        return LRESULT(0);
    } else if msg == WM_REBIND_HOTKEYS {
        if let Some(core) = TRAY_CORE.get() {
            if let Ok(g) = core.lock() {
                register_all_hotkeys(hwnd, &g.config);
            }
        }
        return LRESULT(0);
    } else if msg == WM_CLOSE || msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn main() {
    let is_restart = std::env::args().any(|a| a == "--restart");
    let mutex_name = to_wide_null("Local\\RewindCaptureDaemonSingleton");
    let mutex_handle = unsafe {
        let mut acquired = None;
        let max_attempts = if is_restart { 100 } else { 1 };
        for attempt in 0..max_attempts {
            let h = CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()));
            if windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS {
                if let Ok(handle) = h {
                    let _ = CloseHandle(handle);
                }
                if attempt + 1 < max_attempts {
                    thread::sleep(Duration::from_millis(100));
                }
            } else {
                acquired = h.ok();
                break;
            }
        }
        let Some(h) = acquired else {
            return;
        };
        h
    };

    let core = Arc::new(Mutex::new(DaemonCore::new()));
    let stop = Arc::new(AtomicBool::new(false));

    let _ = TRAY_CORE.set(core.clone());
    let _ = TRAY_STOP.set(stop.clone());

    let _fg_hook = install_foreground_hook();

    let cap_core = core.clone();
    let cap_stop = stop.clone();
    let cap_thread = thread::spawn(move || run_capture_loop(cap_core, cap_stop));

    let ipc_core = core.clone();
    let ipc_stop = stop.clone();
    let ipc_thread = thread::spawn(move || {
        run_pipe_server(PIPE_NAME, &ipc_stop, |req| {
            handle_ipc_request(req, &ipc_core, &ipc_stop)
        });
    });

    let rpc_core = core.clone();
    let rpc_stop = stop.clone();
    let rpc_thread = thread::spawn(move || run_discord_rpc_loop(rpc_core, rpc_stop));

    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let cls_name = to_wide_null("RewindTrayHostWnd");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(cls_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
        let title_w = to_wide_null("Rewind Capture Daemon");
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(cls_name.as_ptr()),
            PCWSTR(title_w.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            None,
            None,
            hinst,
            None,
        )
        .unwrap_or_default();
        TRAY_HWND.store(hwnd.0 as isize, Ordering::Release);

        let init_cfg = core.lock().ok().map(|g| g.config.clone());
        if let Some(cfg) = &init_cfg {
            register_all_hotkeys(hwnd, cfg);
        }
        let init_accent = init_cfg
            .as_ref()
            .map(|c| c.effective_accent_color())
            .unwrap_or_else(|| rewind::config::AccentColor::Blue.color());
        let init_rgba = render_icon_rgba(init_accent, 64);
        let init_hicon = match create_hicon_from_rgba(&init_rgba, 64, 64) {
            Some(h) => {
                TRAY_HICON.store(h.0 as isize, Ordering::Release);
                h
            }
            None => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        };

        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY_ICON,
            hIcon: init_hicon,
            ..Default::default()
        };
        let tip = "Rewind Instant Replay (Running in Tray)";
        for (i, ch) in tip.encode_utf16().take(127).enumerate() {
            nid.szTip[i] = ch;
        }
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

        let mut msg = MSG::default();
        while !stop.load(Ordering::Acquire) && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        for id in [HK_ID_SAVE, HK_ID_MIC, HK_ID_BOOKMARK] {
            let _ = UnregisterHotKey(hwnd, id);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        let prev_icon = TRAY_HICON.swap(0, Ordering::AcqRel);
        if prev_icon != 0 {
            destroy_hicon(windows::Win32::UI::WindowsAndMessaging::HICON(
                prev_icon as *mut _,
            ));
        }
    }

    stop.store(true, Ordering::Release);
    let _ = send_request(&IpcRequest::GetSnapshot);
    let _ = cap_thread.join();
    let _ = ipc_thread.join();
    let _ = rpc_thread.join();
    unsafe {
        let _ = CloseHandle(mutex_handle);
    }
}

fn run_discord_rpc_loop(core: Arc<Mutex<DaemonCore>>, stop: Arc<AtomicBool>) {
    let mut client = DiscordRpcClient::new();
    let mut start_unix = unix_now_secs();
    let mut current_source: Option<String> = None;
    let mut last_key: Option<(bool, String, Option<String>, bool, String, u32)> = None;

    while !stop.load(Ordering::Acquire) {
        let snap = {
            if let Ok(g) = core.lock() {
                let app_id = if g.config.discord_app_id.trim().is_empty() {
                    rewind::config::DEFAULT_DISCORD_APP_ID.to_owned()
                } else {
                    g.config.discord_app_id.trim().to_owned()
                };
                Some((
                    g.config.discord_rich_presence,
                    app_id,
                    g.manual_source.clone().or_else(|| g.active_game.clone()),
                    g.config.replay_on,
                    g.config.resolution.label().to_owned(),
                    g.config.fps.as_u32(),
                ))
            } else {
                None
            }
        };

        if let Some((enabled, app_id, game, replay_on, res_lbl, fps)) = snap {
            if !enabled || app_id.is_empty() {
                if client.is_connected_to(&app_id) || last_key.is_some() {
                    client.clear_activity();
                    client.disconnect();
                    last_key = None;
                }
            } else {
                if game != current_source {
                    current_source = game.clone();
                    start_unix = unix_now_secs();
                }
                let key = (enabled, app_id.clone(), game.clone(), replay_on, res_lbl.clone(), fps);
                let need_send = !client.is_connected_to(&app_id) || last_key.as_ref() != Some(&key);
                if need_send {
                    let name = match game.as_deref() {
                        Some("Desktop") | None => "Rewind".to_owned(),
                        Some(g) => format!("{g} With Rewind"),
                    };
                    let (details, state) =
                        build_activity_strings(game.as_deref(), replay_on, &res_lbl, fps);
                    if client.set_activity(&app_id, &name, &details, &state, start_unix) {
                        last_key = Some(key);
                    } else {
                        last_key = None;
                    }
                }
            }
        }

        for _ in 0..5 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    client.clear_activity();
    client.disconnect();
}

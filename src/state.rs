use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use egui::{Color32, ColorImage, IconData, TextureHandle, TextureOptions, ViewportCommand};
use image::imageops::FilterType;

use rewind::app_icon::{export_ico_to_install_dir, render_icon_rgba};
use rewind::capture::HardwareInfo;
use rewind::config::AppConfig;
use rewind::game_icons::{cache_path_for_game, fetch_and_cache_game_icon, is_fetchable_game};
use rewind::ipc::{ensure_daemon_running, send_request, DaemonSnapshot, IpcRequest, IpcResponse};
use rewind::library::ClipRecord;
use rewind::player::VideoDecoder;
use rewind::updater::{check_latest_release, download_and_launch_installer, ReleaseInfo};
use rewind::win_ffi::{
    set_active_window_borderless_fullscreen, set_active_window_icons_rgba, unix_now_secs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Home,
    AllClips,
    Favorites,
    Folders,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterPill {
    AllClips,
    Favorites,
    Game(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Newest,
    Oldest,
    Longest,
}

impl SortOrder {
    pub const ALL: [SortOrder; 3] = [SortOrder::Newest, SortOrder::Oldest, SortOrder::Longest];

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::Newest => "Newest",
            SortOrder::Oldest => "Oldest",
            SortOrder::Longest => "Longest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Grid,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePopup {
    None,
    CaptureSource,
    ClipLength,
    VideoQuality,
    Notifications,
    Profile,
    SortMenu,
    SettingsClipLength,
    SettingsResolution,
    SettingsFps,
    SettingsCodec,
    SettingsEncoder,
    SettingsRateControl,
    SettingsContainer,
    SettingsGameAudio,
    SettingsMicDevice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebindTarget {
    None,
    SaveClip,
    ToggleMic,
    AddBookmark,
}

#[derive(Debug, Clone)]
pub struct GameDeckSummary {
    pub name: String,
    pub clip_count: usize,
    pub total_bytes: u64,
    pub latest_unix: i64,
    pub latest_thumb_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCategory {
    Capture,
    Clipping,
    Audio,
    Hotkeys,
    Games,
    Storage,
    Personalization,
    Discord,
    System,
    Updates,
}

impl SettingsCategory {
    pub const ALL: [Self; 10] = [
        Self::Capture,
        Self::Clipping,
        Self::Audio,
        Self::Hotkeys,
        Self::Games,
        Self::Storage,
        Self::Personalization,
        Self::Discord,
        Self::System,
        Self::Updates,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Capture => "Capture & Quality",
            Self::Clipping => "Clipping",
            Self::Audio => "Audio",
            Self::Hotkeys => "Hotkeys",
            Self::Games => "Game Detection",
            Self::Storage => "Storage",
            Self::Personalization => "Personalization",
            Self::Discord => "Discord Rich Presence",
            Self::System => "System & Startup",
            Self::Updates => "Updates",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available(ReleaseInfo),
    Downloading(ReleaseInfo),
    Error(ReleaseInfo, String),
    CheckFailed(String),
}

pub struct RewindUiState {
    pub snapshot: DaemonSnapshot,
    pub daemon_connected: bool,
    pub last_poll_secs: f64,

    pub active_tab: NavTab,
    pub history: Vec<NavTab>,
    pub history_idx: usize,

    pub search_open: bool,
    pub search_query: String,
    pub filter_pill: FilterPill,
    pub sort_order: SortOrder,
    pub view_mode: ViewMode,
    pub active_popup: ActivePopup,
    pub rebind_target: RebindTarget,
    pub custom_game_input: String,
    pub ignored_game_input: String,

    pub thumb_cache: HashMap<String, Option<TextureHandle>>,

    pub selected_clip_id: Option<i64>,
    pub modal_opened_at: f64,
    pub modal_fullscreen: bool,
    pub viewport_fullscreen_applied: bool,
    pub modal_blur_tex: Option<TextureHandle>,
    pub modal_blur_requested_at: Option<f64>,
    pub clip_context_menu: Option<(i64, egui::Pos2)>,
    pub show_close_recording_notice: bool,
    pub allow_close_now: bool,
    pub last_seen_auto_close_counter: u64,
    pub should_auto_close_now: bool,
    pub decoder: Option<VideoDecoder>,
    pub player_texture: Option<TextureHandle>,
    pub player_frame_seq: u64,
    pub player_ambient_tex: Option<TextureHandle>,
    pub player_ambient_seq: u64,
    pub last_player_frame: Option<Arc<ColorImage>>,
    pub player_playing: bool,
    pub player_pos_secs: f32,
    pub player_speed: f32,
    pub player_volume: f32,
    pub player_muted: bool,
    pub player_last_tick: f64,
    pub trim_start_secs: f32,
    pub trim_end_secs: f32,
    pub trim_vertical_9_16: bool,
    pub rename_buf: String,
    pub rename_editing: bool,
    pub editor_rename_just_opened: bool,
    pub rename_modal_clip: Option<(i64, String)>,
    pub rename_modal_just_opened: bool,
    pub modal_editor_open: bool,

    pub bg_cache_key: Option<(String, u32)>,
    pub bg_texture: Option<TextureHandle>,
    pub bg_ambient_tint: Option<Color32>,
    pub app_icon_color: Option<Color32>,
    pub app_icon_win_color: Option<Color32>,
    pub app_icon_texture: Option<TextureHandle>,
    pub game_icon_cache: HashMap<String, Option<TextureHandle>>,
    game_icon_inflight: HashSet<String>,
    game_icon_rx: Receiver<(String, Option<PathBuf>)>,
    game_icon_tx: Sender<(String, Option<PathBuf>)>,

    pub restarting_from_pid: Option<u32>,
    pub daemon_restart_started_at: f64,
    pub pending_config_update: Option<AppConfig>,
    pub pending_config_at: f64,

    pub last_seen_flash_counter: u64,
    pub clip_flash_start: Option<f64>,
    pub status_banner: Option<(String, f64)>,

    pub update_status: UpdateStatus,
    pub manual_update_check: bool,
    update_rx: Option<Receiver<Result<Option<ReleaseInfo>, String>>>,
    install_rx: Option<Receiver<Result<(), String>>>,
    pub discord_encode_progress: Option<Arc<AtomicU32>>,
    discord_encode_rx: Option<Receiver<Result<(PathBuf, u64), String>>>,
    pub onboarding_step: usize,
    pub tour_step: Option<usize>,
    pub settings_category: SettingsCategory,
    pub settings_search: String,

    ipc_tx: Sender<IpcRequest>,
    ipc_rx: Receiver<Result<IpcResponse, String>>,
}

impl RewindUiState {
    pub fn connect_and_init() -> Self {
        let connected = ensure_daemon_running();
        let initial_resp = if connected {
            send_request(&IpcRequest::GetSnapshot).ok()
        } else {
            None
        };

        let snapshot = initial_resp
            .and_then(|r| r.snapshot)
            .unwrap_or_else(empty_snapshot);
        let last_seen_flash_counter = snapshot.clip_flash_counter;
        let last_seen_auto_close_counter = snapshot.auto_close_ui_counter;

        let (req_tx, req_rx) = mpsc::channel::<IpcRequest>();
        let (res_tx, res_rx) = mpsc::channel::<Result<IpcResponse, String>>();
        let (game_icon_tx, game_icon_rx) = mpsc::channel::<(String, Option<PathBuf>)>();
        thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let res = send_request(&req);
                if res_tx.send(res).is_err() {
                    break;
                }
            }
        });

        let mut state = Self {
            snapshot,
            daemon_connected: connected,
            last_poll_secs: 0.0,

            active_tab: NavTab::Home,
            history: vec![NavTab::Home],
            history_idx: 0,

            search_open: false,
            search_query: String::new(),
            filter_pill: FilterPill::AllClips,
            sort_order: SortOrder::Newest,
            view_mode: ViewMode::Grid,
            active_popup: ActivePopup::None,
            rebind_target: RebindTarget::None,
            custom_game_input: String::new(),
            ignored_game_input: String::new(),

            thumb_cache: HashMap::new(),

            selected_clip_id: None,
            modal_opened_at: 0.0,
            modal_fullscreen: false,
            viewport_fullscreen_applied: false,
            clip_context_menu: None,
            show_close_recording_notice: false,
            allow_close_now: false,
            last_seen_auto_close_counter,
            should_auto_close_now: false,
            decoder: None,
            player_texture: None,
            player_frame_seq: 0,
            player_ambient_tex: None,
            player_ambient_seq: 0,
            last_player_frame: None,
            player_playing: false,
            player_pos_secs: 0.0,
            player_speed: 1.0,
            player_volume: 0.85,
            player_muted: false,
            player_last_tick: 0.0,
            trim_start_secs: 0.0,
            trim_end_secs: 1.0,
            trim_vertical_9_16: false,
            rename_buf: String::new(),
            rename_editing: false,
            editor_rename_just_opened: false,
            rename_modal_clip: None,
            rename_modal_just_opened: false,
            modal_editor_open: false,
            modal_blur_tex: None,
            modal_blur_requested_at: None,

            bg_cache_key: None,
            bg_texture: None,
            bg_ambient_tint: None,
            app_icon_color: None,
            app_icon_win_color: None,
            app_icon_texture: None,
            game_icon_cache: HashMap::new(),
            game_icon_inflight: HashSet::new(),
            game_icon_rx,
            game_icon_tx,

            restarting_from_pid: None,
            daemon_restart_started_at: 0.0,
            pending_config_update: None,
            pending_config_at: 0.0,

            last_seen_flash_counter,
            clip_flash_start: None,
            status_banner: None,
            update_status: UpdateStatus::Idle,
            manual_update_check: false,
            update_rx: None,
            install_rx: None,
            discord_encode_progress: None,
            discord_encode_rx: None,
            onboarding_step: 0,
            tour_step: None,
            settings_category: SettingsCategory::Capture,
            settings_search: String::new(),
            ipc_tx: req_tx,
            ipc_rx: res_rx,
        };
        state.trigger_update_check(false);
        state
    }

    fn apply_snapshot(&mut self, snap: DaemonSnapshot, now_secs: f64) {
        if snap.clip_flash_counter > self.last_seen_flash_counter {
            self.last_seen_flash_counter = snap.clip_flash_counter;
            self.clip_flash_start = Some(now_secs);
        } else if snap.clip_flash_counter < self.last_seen_flash_counter {
            self.last_seen_flash_counter = snap.clip_flash_counter;
        }
        if snap.auto_close_ui_counter > self.last_seen_auto_close_counter {
            self.last_seen_auto_close_counter = snap.auto_close_ui_counter;
            self.should_auto_close_now = true;
            self.allow_close_now = true;
        } else if snap.auto_close_ui_counter < self.last_seen_auto_close_counter {
            self.last_seen_auto_close_counter = snap.auto_close_ui_counter;
        }
        let preserve_cfg = self.pending_config_update.is_some();
        let local_cfg = if preserve_cfg {
            Some(self.snapshot.config.clone())
        } else {
            None
        };
        self.snapshot = snap;
        if let Some(cfg) = local_cfg {
            self.snapshot.config = cfg;
        }
    }

    pub fn send_async(&self, req: IpcRequest) {
        let _ = self.ipc_tx.send(req);
    }

    pub fn send_sync(&mut self, req: IpcRequest, now_secs: f64) {
        let is_save_clip = matches!(req, IpcRequest::SaveClipNow);
        match send_request(&req) {
            Ok(resp) => {
                self.daemon_connected = true;
                let was_restarting = resp.restarting;
                let old_pid = resp
                    .snapshot
                    .as_ref()
                    .map(|s| s.daemon_pid)
                    .unwrap_or(self.snapshot.daemon_pid);
                if let Some(snap) = resp.snapshot {
                    self.apply_snapshot(snap, now_secs);
                }
                if was_restarting {
                    self.restarting_from_pid = Some(old_pid);
                    self.daemon_restart_started_at = now_secs;
                    self.snapshot.replay_on = false;
                    self.snapshot.capture_active = false;
                    self.snapshot.active_game = None;
                    self.snapshot.manual_source = None;
                    self.last_poll_secs = now_secs - 1.0;
                }
                if is_save_clip {
                    self.clip_flash_start = Some(now_secs);
                }
                if let Some(msg) = resp.message {
                    self.status_banner = Some((msg, now_secs));
                }
            }
            Err(err) => {
                self.daemon_connected = false;
                if self.restarting_from_pid.is_some() {
                    if let IpcRequest::UpdateConfig(cfg) = req {
                        self.pending_config_update = Some(cfg);
                    }
                } else {
                    self.status_banner = Some((err, now_secs));
                }
            }
        }
    }

    pub fn trigger_update_check(&mut self, manual: bool) {
        self.manual_update_check = manual;
        self.update_status = UpdateStatus::Checking;
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(check_latest_release());
        });
    }

    pub fn start_update_install(&mut self, info: ReleaseInfo) {
        self.update_status = UpdateStatus::Downloading(info.clone());
        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        thread::spawn(move || {
            let res = download_and_launch_installer(&info).map(|_| ());
            let _ = tx.send(res);
        });
    }

    pub fn poll_background_ipc(&mut self, ctx: &egui::Context, now_secs: f64) {
        while let Ok((game_name, path_opt)) = self.game_icon_rx.try_recv() {
            self.game_icon_inflight.remove(&game_name);
            let tex = path_opt.and_then(|p| {
                load_image_texture(ctx, &p, &format!("game_icon_{game_name}"))
            });
            self.game_icon_cache.insert(game_name, tex);
        }

        while let Ok(res) = self.ipc_rx.try_recv() {
            match res {
                Ok(resp) => {
                    self.daemon_connected = true;
                    if let Some(snap) = resp.snapshot {
                        if let Some(old_pid) = self.restarting_from_pid {
                            if snap.daemon_pid != 0 && snap.daemon_pid != old_pid {
                                self.restarting_from_pid = None;
                                self.apply_snapshot(snap, now_secs);
                                if let Some(pending) = self.pending_config_update.take() {
                                    self.send_sync(IpcRequest::UpdateConfig(pending), now_secs);
                                } else {
                                    self.status_banner =
                                        Some(("Daemon is back online".to_owned(), now_secs));
                                }
                            } else {
                                self.snapshot.replay_on = false;
                                self.snapshot.capture_active = false;
                                self.snapshot.active_game = None;
                                self.snapshot.manual_source = None;
                            }
                        } else {
                            self.apply_snapshot(snap, now_secs);
                        }
                    }
                    if let Some(msg) = resp.message {
                        self.status_banner = Some((msg, now_secs));
                    }
                }
                Err(_) => {
                    self.daemon_connected = false;
                    if self.restarting_from_pid.is_some() {
                        self.snapshot.replay_on = false;
                        self.snapshot.capture_active = false;
                        self.snapshot.active_game = None;
                        self.snapshot.manual_source = None;
                    }
                }
            }
        }

        if self.restarting_from_pid.is_some() && (now_secs - self.daemon_restart_started_at) > 8.0 {
            self.restarting_from_pid = None;
        }
        if self.restarting_from_pid.is_none() && !ctx.input(|i| i.pointer.primary_down()) {
            if self.pending_config_update.is_some() {
                if (now_secs - self.pending_config_at) >= 0.35 {
                    if let Some(pending) = self.pending_config_update.take() {
                        self.send_sync(IpcRequest::UpdateConfig(pending), now_secs);
                    }
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                }
            }
        }
        if let Some(rx) = &self.update_rx {
            if let Ok(res) = rx.try_recv() {
                self.update_rx = None;
                match res {
                    Ok(Some(info)) => {
                        self.update_status = UpdateStatus::Available(info);
                    }
                    Ok(None) => {
                        self.update_status = if self.manual_update_check {
                            UpdateStatus::UpToDate
                        } else {
                            UpdateStatus::Idle
                        };
                    }
                    Err(e) => {
                        self.update_status = if self.manual_update_check {
                            UpdateStatus::CheckFailed(e)
                        } else {
                            UpdateStatus::Idle
                        };
                    }
                }
            }
        }
        if let Some(rx) = &self.install_rx {
            if let Ok(res) = rx.try_recv() {
                self.install_rx = None;
                match res {
                    Ok(()) => {
                        std::process::exit(0);
                    }
                    Err(e) => {
                        if let UpdateStatus::Downloading(info) = self.update_status.clone() {
                            self.update_status = UpdateStatus::Error(info, e);
                        }
                    }
                }
            }
        }
        if let Some(rx) = &self.discord_encode_rx {
            if let Ok(res) = rx.try_recv() {
                self.discord_encode_rx = None;
                self.discord_encode_progress = None;
                match res {
                    Ok((path, bytes)) => {
                        let copied =
                            rewind::win_ffi::copy_file_to_clipboard(&path.to_string_lossy());
                        let mb = (bytes as f64) / 1_000_000.0;
                        let msg = if copied {
                            format!(
                                "Discord clip ready ({mb:.2} MB) — press Ctrl+V in Discord"
                            )
                        } else {
                            format!("Discord clip saved ({mb:.2} MB): {}", path.display())
                        };
                        self.status_banner = Some((msg, now_secs + 2.5));
                    }
                    Err(e) => {
                        self.status_banner =
                            Some((format!("Discord encode failed: {e}"), now_secs + 2.0));
                    }
                }
            }
        }
        let poll_interval = if self.restarting_from_pid.is_some() { 0.25 } else { 1.2 };
        if now_secs - self.last_poll_secs >= poll_interval {
            self.last_poll_secs = now_secs;
            self.send_async(IpcRequest::GetSnapshot);
        }
    }

    pub fn start_discord_encode(
        &mut self,
        clip: &ClipRecord,
        trim_range: Option<(f32, f32)>,
        now_secs: f64,
    ) {
        if self.discord_encode_progress.is_some() {
            self.status_banner = Some((
                "Already encoding a clip for Discord...".to_owned(),
                now_secs,
            ));
            return;
        }
        let src_path = PathBuf::from(&clip.file_path);
        let out_path = rewind::transcoder::discord_export_path(&clip.title, clip.id);
        let dur_hint = clip.duration_secs;
        let prog = Arc::new(AtomicU32::new(0));
        let prog_w = prog.clone();
        let (tx, rx) = mpsc::channel();
        self.discord_encode_progress = Some(prog);
        self.discord_encode_rx = Some(rx);
        self.status_banner = Some(("Encoding for Discord (<10 MB)...".to_owned(), now_secs));
        thread::spawn(move || {
            let res = rewind::transcoder::encode_for_discord(
                &src_path,
                &out_path,
                trim_range,
                dur_hint,
                &prog_w,
            );
            let _ = tx.send(res);
        });
    }

    pub fn push_config_update(&mut self, now_secs: f64) {
        let cfg = self.snapshot.config.clone();
        if self.restarting_from_pid.is_some() {
            self.pending_config_update = Some(cfg);
            return;
        }
        self.send_sync(IpcRequest::UpdateConfig(cfg), now_secs);
    }

    pub fn navigate(&mut self, tab: NavTab) {
        self.active_popup = ActivePopup::None;
        if self.active_tab != tab {
            self.history.truncate(self.history_idx + 1);
            self.history.push(tab);
            self.history_idx = self.history.len() - 1;
            self.active_tab = tab;
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.history_idx > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_idx + 1 < self.history.len()
    }

    pub fn go_back(&mut self) {
        self.active_popup = ActivePopup::None;
        if self.can_go_back() {
            self.history_idx -= 1;
            self.active_tab = self.history[self.history_idx];
        }
    }

    pub fn go_forward(&mut self) {
        self.active_popup = ActivePopup::None;
        if self.can_go_forward() {
            self.history_idx += 1;
            self.active_tab = self.history[self.history_idx];
        }
    }

    pub fn toggle_popup(&mut self, popup: ActivePopup) {
        if self.active_popup == popup {
            self.active_popup = ActivePopup::None;
        } else {
            self.active_popup = popup;
        }
    }

    pub fn dynamic_games(&self) -> Vec<GameDeckSummary> {
        let mut by_game: BTreeMap<String, GameDeckSummary> = BTreeMap::new();
        for clip in &self.snapshot.clips {
            let entry = by_game
                .entry(clip.game.clone())
                .or_insert_with(|| GameDeckSummary {
                    name: clip.game.clone(),
                    clip_count: 0,
                    total_bytes: 0,
                    latest_unix: clip.created_at_unix,
                    latest_thumb_path: clip.thumb_path.clone(),
                });
            entry.clip_count += 1;
            entry.total_bytes += clip.size_bytes;
            if clip.created_at_unix >= entry.latest_unix {
                entry.latest_unix = clip.created_at_unix;
                if !clip.thumb_path.is_empty() {
                    entry.latest_thumb_path = clip.thumb_path.clone();
                }
            }
        }
        let mut list: Vec<GameDeckSummary> = by_game.into_values().collect();
        list.sort_by(|a, b| b.latest_unix.cmp(&a.latest_unix));
        list
    }

    pub fn filtered_clips(&self) -> Vec<&ClipRecord> {
        let query = self.search_query.trim().to_ascii_lowercase();
        let mut out: Vec<&ClipRecord> = self
            .snapshot
            .clips
            .iter()
            .filter(|clip| {
                if self.active_tab == NavTab::Favorites && !clip.favorite {
                    return false;
                }
                match &self.filter_pill {
                    FilterPill::AllClips => {}
                    FilterPill::Favorites => {
                        if !clip.favorite {
                            return false;
                        }
                    }
                    FilterPill::Game(g) => {
                        if &clip.game != g {
                            return false;
                        }
                    }
                }
                if !query.is_empty() {
                    let in_title = clip.title.to_ascii_lowercase().contains(&query);
                    let in_game = clip.game.to_ascii_lowercase().contains(&query);
                    if !in_title && !in_game {
                        return false;
                    }
                }
                true
            })
            .collect();

        match self.sort_order {
            SortOrder::Newest => out.sort_by(|a, b| {
                b.created_at_unix
                    .cmp(&a.created_at_unix)
                    .then_with(|| b.id.cmp(&a.id))
            }),
            SortOrder::Oldest => out.sort_by(|a, b| {
                a.created_at_unix
                    .cmp(&b.created_at_unix)
                    .then_with(|| a.id.cmp(&b.id))
            }),
            SortOrder::Longest => out.sort_by(|a, b| {
                b.duration_secs
                    .total_cmp(&a.duration_secs)
                    .then_with(|| b.created_at_unix.cmp(&a.created_at_unix))
            }),
        }
        out
    }

    pub fn get_or_load_thumbnail(
        &mut self,
        ctx: &egui::Context,
        thumb_path: &str,
    ) -> Option<TextureHandle> {
        if thumb_path.is_empty() {
            return None;
        }
        if let Some(cached) = self.thumb_cache.get(thumb_path) {
            return cached.clone();
        }
        let tex = load_image_texture(ctx, Path::new(thumb_path), thumb_path);
        self.thumb_cache.insert(thumb_path.to_owned(), tex.clone());
        tex
    }

    pub fn open_clip_modal(&mut self, clip: &ClipRecord, now_secs: f64) {
        if self.selected_clip_id.is_none() {
            self.modal_blur_tex = None;
            self.modal_blur_requested_at = None;
        }
        self.selected_clip_id = Some(clip.id);
        self.modal_opened_at = now_secs;
        self.player_pos_secs = 0.0;
        self.player_playing = true;
        self.player_speed = 1.0;
        self.player_last_tick = now_secs;
        self.trim_start_secs = 0.0;
        self.trim_end_secs = clip.duration_secs.max(0.5);
        self.trim_vertical_9_16 = false;
        self.rename_buf = clip.title.clone();
        self.rename_editing = false;
        self.modal_editor_open = false;
        self.modal_fullscreen = false;
        self.player_texture = None;
        self.player_frame_seq = 0;
        self.player_ambient_tex = None;
        self.player_ambient_seq = 0;
        self.last_player_frame = None;
        let dec = VideoDecoder::open(Path::new(&clip.file_path), clip.duration_secs).ok();
        if let Some(d) = &dec {
            let gain = if self.player_muted { 0.0 } else { self.player_volume };
            d.set_audio_gain(gain);
            d.set_playing(true);
        }
        if let Some(old) = self.decoder.take() {
            std::thread::spawn(move || drop(old));
        }
        self.decoder = dec;
    }

    pub fn close_clip_modal(&mut self) {
        self.selected_clip_id = None;
        self.last_player_frame = None;
        if let Some(old) = self.decoder.take() {
            std::thread::spawn(move || drop(old));
        }
        self.player_texture = None;
        self.player_frame_seq = 0;
        self.player_playing = false;
        self.modal_editor_open = false;
        self.modal_fullscreen = false;
        self.modal_blur_tex = None;
        self.modal_blur_requested_at = None;
        if self.viewport_fullscreen_applied {
            set_active_window_borderless_fullscreen(false);
            self.viewport_fullscreen_applied = false;
        }
    }

    pub fn ensure_background_loaded(&mut self, ctx: &egui::Context) -> Option<[u8; 3]> {
        let path = self.snapshot.config.background_image_path.clone();
        let blur = self.snapshot.config.background_blur.min(40);
        if path.is_empty() || !Path::new(&path).exists() {
            self.bg_cache_key = None;
            self.bg_texture = None;
            self.bg_ambient_tint = None;
            return None;
        }
        if self.bg_cache_key.as_ref() == Some(&(path.clone(), blur)) {
            return None;
        }

        let dyn_img = image::open(Path::new(&path)).ok()?;
        let sample_rgba = dyn_img.resize_exact(256, 144, FilterType::Triangle).to_rgba8();
        let (accent_rgb, ambient) = extract_vibrant_accent_and_tint(&sample_rgba);

        let display_rgba = if blur == 0 {
            if dyn_img.width() > 1600 || dyn_img.height() > 900 {
                dyn_img.resize(1600, 900, FilterType::Triangle).to_rgba8()
            } else {
                dyn_img.to_rgba8()
            }
        } else {
            let (mw, mh) = if blur <= 10 { (640, 360) } else { (420, 236) };
            let scaled = dyn_img.resize(mw, mh, FilterType::Triangle).to_rgba8();
            image::imageops::blur(&scaled, (blur as f32) * 0.45)
        };

        let size = [display_rgba.width() as usize, display_rgba.height() as usize];
        let color_img = ColorImage::from_rgba_unmultiplied(size, display_rgba.as_raw());
        let tex = ctx.load_texture("custom_user_bg", color_img, TextureOptions::LINEAR);
        self.bg_cache_key = Some((path, blur));
        self.bg_texture = Some(tex);
        self.bg_ambient_tint = Some(ambient);
        Some(accent_rgb)
    }

    pub fn ensure_app_icon_synced(&mut self, ctx: &egui::Context, accent: Color32) {
        let color_changed = self.app_icon_color != Some(accent);
        if color_changed || self.app_icon_texture.is_none() {
            let rgba_256 = render_icon_rgba(accent, 256);
            let rgba_128 = render_icon_rgba(accent, 128);
            let rgba_64 = render_icon_rgba(accent, 64);
            let color_img = ColorImage::from_rgba_unmultiplied([128, 128], &rgba_128);
            let tex = ctx.load_texture("rewind_app_icon", color_img, TextureOptions::LINEAR);
            self.app_icon_texture = Some(tex);
            self.app_icon_color = Some(accent);
            ctx.send_viewport_cmd(ViewportCommand::Icon(Some(Arc::new(IconData {
                rgba: rgba_256.clone(),
                width: 256,
                height: 256,
            }))));
            if set_active_window_icons_rgba(&rgba_64, 64, &rgba_256, 256) {
                self.app_icon_win_color = Some(accent);
            } else {
                self.app_icon_win_color = None;
            }
            if color_changed {
                thread::spawn(move || export_ico_to_install_dir(accent));
            }
        } else if self.app_icon_win_color != Some(accent) {
            let rgba_256 = render_icon_rgba(accent, 256);
            let rgba_64 = render_icon_rgba(accent, 64);
            if set_active_window_icons_rgba(&rgba_64, 64, &rgba_256, 256) {
                self.app_icon_win_color = Some(accent);
            }
        }
    }

    pub fn get_or_fetch_game_icon(
        &mut self,
        ctx: &egui::Context,
        game_name: &str,
    ) -> Option<TextureHandle> {
        if let Some(cached) = self.game_icon_cache.get(game_name) {
            return cached.clone();
        }
        if !is_fetchable_game(game_name) {
            self.game_icon_cache.insert(game_name.to_owned(), None);
            return None;
        }
        let disk_path = cache_path_for_game(game_name);
        if disk_path.exists() {
            let tex = load_image_texture(ctx, &disk_path, &format!("game_icon_{game_name}"));
            self.game_icon_cache
                .insert(game_name.to_owned(), tex.clone());
            return tex;
        }
        let api_key = self.snapshot.config.steamgriddb_api_key.trim().to_owned();
        if api_key.is_empty() || self.game_icon_inflight.contains(game_name) {
            return None;
        }
        self.game_icon_inflight.insert(game_name.to_owned());
        let name_owned = game_name.to_owned();
        let tx = self.game_icon_tx.clone();
        thread::spawn(move || {
            let res = fetch_and_cache_game_icon(&name_owned, &api_key);
            let _ = tx.send((name_owned, res));
        });
        None
    }

    pub fn clear_game_icon_cache(&mut self) {
        self.game_icon_cache.clear();
        self.game_icon_inflight.clear();
    }

    pub fn apply_rebind(&mut self, combo: String, now_secs: f64) {
        match self.rebind_target {
            RebindTarget::SaveClip => self.snapshot.config.hotkey_save_clip = combo,
            RebindTarget::ToggleMic => self.snapshot.config.hotkey_toggle_mic = combo,
            RebindTarget::AddBookmark => self.snapshot.config.hotkey_add_bookmark = combo,
            RebindTarget::None => return,
        }
        self.rebind_target = RebindTarget::None;
        self.push_config_update(now_secs);
    }

    pub fn used_storage_gb(&self) -> f32 {
        self.snapshot.used_storage_bytes as f32 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn format_relative_unix(unix_secs: i64) -> String {
        let delta = (unix_now_secs() - unix_secs).max(0);
        if delta < 60 {
            "Just now".to_owned()
        } else if delta < 3600 {
            format!("{}m ago", delta / 60)
        } else if delta < 86_400 {
            format!("{}h ago", delta / 3600)
        } else {
            format!("{}d ago", delta / 86_400)
        }
    }
}

fn load_image_texture(ctx: &egui::Context, path: &Path, name: &str) -> Option<TextureHandle> {
    if !path.exists() {
        return None;
    }
    let dyn_img = image::open(path).ok()?;
    let rgba = dyn_img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color_img = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(name, color_img, TextureOptions::LINEAR))
}

fn extract_vibrant_accent_and_tint(img: &image::RgbaImage) -> ([u8; 3], Color32) {
    let mut sum_w = 0.0_f32;
    let mut acc_r = 0.0_f32;
    let mut acc_g = 0.0_f32;
    let mut acc_b = 0.0_f32;

    let mut sat_sum = 0.0_f32;
    let mut sat_count = 0_u32;

    for px in img.pixels() {
        let [r, g, b, _] = px.0;
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        if max_c < 20 || min_c > 240 {
            continue;
        }
        let sat = ((max_c - min_c) as f32) / (max_c.max(1) as f32);
        sat_sum += sat;
        sat_count += 1;

        if sat >= 0.15 {
            let lum = (max_c as f32) / 255.0;
            let w = sat * sat * (0.35 + 0.65 * lum);
            acc_r += (r as f32) * w;
            acc_g += (g as f32) * w;
            acc_b += (b as f32) * w;
            sum_w += w;
        }
    }

    let avg_sat = sat_sum / (sat_count.max(1) as f32);
    if avg_sat < 0.11 || sum_w < 8.0 {
        return ([210, 214, 220], Color32::from_rgb(16, 16, 18));
    }

    let mut r = acc_r / sum_w;
    let mut g = acc_g / sum_w;
    let mut b = acc_b / sum_w;
    let peak = r.max(g).max(b).max(1.0);
    if peak < 175.0 {
        let scale = 200.0 / peak;
        r *= scale;
        g *= scale;
        b *= scale;
    } else if peak > 235.0 {
        let scale = 230.0 / peak;
        r *= scale;
        g *= scale;
        b *= scale;
    }
    let ambient = Color32::from_rgb(
        (r * 0.14).clamp(10.0, 48.0) as u8,
        (g * 0.14).clamp(10.0, 48.0) as u8,
        (b * 0.14).clamp(10.0, 48.0) as u8,
    );
    (
        [
            r.clamp(30.0, 245.0) as u8,
            g.clamp(30.0, 245.0) as u8,
            b.clamp(30.0, 245.0) as u8,
        ],
        ambient,
    )
}

fn empty_snapshot() -> DaemonSnapshot {
    DaemonSnapshot {
        config: AppConfig::default(),
        hardware: HardwareInfo::default(),
        replay_on: false,
        capture_active: false,
        active_game: None,
        active_pid: None,
        manual_source: None,
        ring_buffer_bytes: 0,
        clips: Vec::new(),
        used_storage_bytes: 0,
        audio_outputs: vec!["Default System Output".to_owned()],
        audio_inputs: vec!["Default Input Device".to_owned()],
        recent_events: Vec::new(),
        pending_undo_clip_id: None,
        daemon_ram_mb: 0.0,
        clip_flash_counter: 0,
        auto_close_ui_counter: 0,
        daemon_pid: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_extraction_matches_monochrome_and_colored_backgrounds() {
        let mut mono = image::RgbaImage::new(64, 64);
        for (x, _, px) in mono.enumerate_pixels_mut() {
            let v = 40 + ((x * 2) as u8);
            *px = image::Rgba([v, v, v, 255]);
        }
        let (acc_m, amb_m) = extract_vibrant_accent_and_tint(&mono);
        assert_eq!(acc_m, [210, 214, 220]);
        assert_eq!(amb_m, Color32::from_rgb(16, 16, 18));

        let mut red = image::RgbaImage::new(64, 64);
        for px in red.pixels_mut() {
            *px = image::Rgba([190, 28, 32, 255]);
        }
        let (acc_r, _) = extract_vibrant_accent_and_tint(&red);
        assert!(acc_r[0] > 180 && acc_r[0] > acc_r[2] * 2);
    }
}

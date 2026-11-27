use egui::Color32;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BufferMode {
    SystemRam,
    LocalDisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    P1080,
    P1440,
    K4,
}

impl Resolution {
    pub const ALL: [Resolution; 3] = [Resolution::P1080, Resolution::P1440, Resolution::K4];

    pub fn label(self) -> &'static str {
        match self {
            Resolution::P1080 => "1080p",
            Resolution::P1440 => "1440p",
            Resolution::K4 => "4K",
        }
    }

    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Resolution::P1080 => (1920, 1080),
            Resolution::P1440 => (2560, 1440),
            Resolution::K4 => (3840, 2160),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameRate {
    Fps30,
    Fps60,
    Fps120,
}

impl FrameRate {
    pub const ALL: [FrameRate; 3] = [FrameRate::Fps30, FrameRate::Fps60, FrameRate::Fps120];

    pub fn as_u32(self) -> u32 {
        match self {
            FrameRate::Fps30 => 30,
            FrameRate::Fps60 => 60,
            FrameRate::Fps120 => 120,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FrameRate::Fps30 => "30 fps",
            FrameRate::Fps60 => "60 fps",
            FrameRate::Fps120 => "120 fps",
        }
    }

    pub fn compact_label(self) -> &'static str {
        match self {
            FrameRate::Fps30 => "30fps",
            FrameRate::Fps60 => "60fps",
            FrameRate::Fps120 => "120fps",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoQuality {
    Fhd60,
    Fhd120,
    Qhd60,
    Uhd60,
}

impl VideoQuality {
    pub const ALL: [VideoQuality; 4] = [
        VideoQuality::Fhd60,
        VideoQuality::Fhd120,
        VideoQuality::Qhd60,
        VideoQuality::Uhd60,
    ];

    pub fn title(self) -> &'static str {
        match self {
            VideoQuality::Fhd60 => "1080p 60fps",
            VideoQuality::Fhd120 => "1080p 120fps",
            VideoQuality::Qhd60 => "1440p 60fps",
            VideoQuality::Uhd60 => "4K 60fps",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            VideoQuality::Fhd60 => "Balanced Full HD",
            VideoQuality::Fhd120 => "Ultra-smooth competitive",
            VideoQuality::Qhd60 => "Crisp QHD capture",
            VideoQuality::Uhd60 => "Maximum UHD clarity",
        }
    }

    pub fn parts(self) -> (Resolution, FrameRate) {
        match self {
            VideoQuality::Fhd60 => (Resolution::P1080, FrameRate::Fps60),
            VideoQuality::Fhd120 => (Resolution::P1080, FrameRate::Fps120),
            VideoQuality::Qhd60 => (Resolution::P1440, FrameRate::Fps60),
            VideoQuality::Uhd60 => (Resolution::K4, FrameRate::Fps60),
        }
    }

    pub fn from_parts(res: Resolution, fps: FrameRate) -> Option<Self> {
        match (res, fps) {
            (Resolution::P1080, FrameRate::Fps60) => Some(VideoQuality::Fhd60),
            (Resolution::P1080, FrameRate::Fps120) => Some(VideoQuality::Fhd120),
            (Resolution::P1440, FrameRate::Fps60) => Some(VideoQuality::Qhd60),
            (Resolution::K4, FrameRate::Fps60) => Some(VideoQuality::Uhd60),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    Hevc,
    Av1,
    H264,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 3] = [VideoCodec::Hevc, VideoCodec::Av1, VideoCodec::H264];

    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::Hevc => "HEVC / H.265",
            VideoCodec::Av1 => "AV1 (Hardware)",
            VideoCodec::H264 => "AVC / H.264",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Av1 => "AV1",
            VideoCodec::H264 => "H.264",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderPreference {
    Auto,
    Nvenc,
    Amf,
    Qsv,
}

impl EncoderPreference {
    pub const ALL: [EncoderPreference; 4] = [
        EncoderPreference::Auto,
        EncoderPreference::Nvenc,
        EncoderPreference::Amf,
        EncoderPreference::Qsv,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EncoderPreference::Auto => "Auto (GPU Vendor)",
            EncoderPreference::Nvenc => "NVIDIA NVENC",
            EncoderPreference::Amf => "AMD AMF",
            EncoderPreference::Qsv => "Intel QuickSync",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateControl {
    Cqp,
    Cbr,
}

impl RateControl {
    pub fn label(self) -> &'static str {
        match self {
            RateControl::Cqp => "CQP (Quality)",
            RateControl::Cbr => "CBR (Bitrate)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerFormat {
    Mp4,
    Mkv,
}

impl ContainerFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ContainerFormat::Mp4 => "mp4",
            ContainerFormat::Mkv => "mkv",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContainerFormat::Mp4 => "MP4 (.mp4)",
            ContainerFormat::Mkv => "MKV (.mkv)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipLength {
    Sec15,
    Sec30,
    Min1,
    Min2,
    Min5,
}

impl ClipLength {
    pub const ALL: [ClipLength; 5] = [
        ClipLength::Sec15,
        ClipLength::Sec30,
        ClipLength::Min1,
        ClipLength::Min2,
        ClipLength::Min5,
    ];

    pub fn seconds(self) -> u32 {
        match self {
            ClipLength::Sec15 => 15,
            ClipLength::Sec30 => 30,
            ClipLength::Min1 => 60,
            ClipLength::Min2 => 120,
            ClipLength::Min5 => 300,
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            ClipLength::Sec15 => "Clip 15s",
            ClipLength::Sec30 => "Clip 30s",
            ClipLength::Min1 => "Clip 1m",
            ClipLength::Min2 => "Clip 2m",
            ClipLength::Min5 => "Clip 5m",
        }
    }

    pub fn long_suffix(self) -> &'static str {
        match self {
            ClipLength::Sec15 => "(15 seconds)",
            ClipLength::Sec30 => "(30 seconds)",
            ClipLength::Min1 => "(1 minute)",
            ClipLength::Min2 => "(2 minutes)",
            ClipLength::Min5 => "(5 minutes)",
        }
    }

    pub fn settings_label(self) -> &'static str {
        match self {
            ClipLength::Sec15 => "15 seconds",
            ClipLength::Sec30 => "30 seconds",
            ClipLength::Min1 => "1 minute",
            ClipLength::Min2 => "2 minutes",
            ClipLength::Min5 => "5 minutes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccentColor {
    Blue,
    Purple,
    Emerald,
    Coral,
    Amber,
}

impl AccentColor {
    pub const ALL: [AccentColor; 5] = [
        AccentColor::Blue,
        AccentColor::Purple,
        AccentColor::Emerald,
        AccentColor::Coral,
        AccentColor::Amber,
    ];

    pub fn color(self) -> Color32 {
        match self {
            AccentColor::Blue => Color32::from_rgb(76, 141, 255),
            AccentColor::Purple => Color32::from_rgb(139, 92, 246),
            AccentColor::Emerald => Color32::from_rgb(16, 185, 129),
            AccentColor::Coral => Color32::from_rgb(244, 63, 94),
            AccentColor::Amber => Color32::from_rgb(245, 158, 11),
        }
    }

    pub fn soft_bg(self) -> Color32 {
        let c = self.color();
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 34)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SharpenLevel {
    #[default]
    Off,
    Light,
    Medium,
}

impl SharpenLevel {
    pub const ALL: [Self; 3] = [Self::Off, Self::Light, Self::Medium];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Light => "Light",
            Self::Medium => "Medium",
        }
    }

    pub fn strength(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Light => 0.30,
            Self::Medium => 0.55,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub clip_length: ClipLength,
    pub buffer_mode: BufferMode,
    pub resolution: Resolution,
    pub fps: FrameRate,
    pub codec: VideoCodec,
    pub encoder_preference: EncoderPreference,
    pub rate_control: RateControl,
    pub cqp_qp: u32,
    pub bitrate_mbps: u32,
    pub gop_secs: u32,
    pub container: ContainerFormat,

    pub game_audio_device: String,
    pub game_audio_volume: u32,
    pub mic_device: String,
    pub mic_enabled: bool,
    pub mic_volume: u32,
    pub separate_audio_tracks: bool,

    pub hotkey_save_clip: String,
    pub hotkey_toggle_mic: String,
    pub hotkey_add_bookmark: String,

    pub clips_folder: String,
    pub max_storage_gb: u32,
    pub auto_delete_oldest: bool,
    pub accent_color: AccentColor,
    pub replay_on: bool,
    pub auto_start_capture: bool,
    pub start_with_windows: bool,
    pub close_ui_on_game_launch: bool,

    pub background_image_path: String,
    pub background_blur: u32,
    pub background_brightness: u32,
    pub auto_adjust_palette: bool,
    pub custom_accent_rgb: Option<[u8; 3]>,
    pub steamgriddb_api_key: String,
    pub custom_games: Vec<String>,
    pub ignored_games: Vec<String>,
    pub onboarding_completed: bool,
    pub discord_rich_presence: bool,
    pub discord_app_id: String,
    #[serde(default)]
    pub clip_post_buffer_secs: u32,
    #[serde(default)]
    pub suppress_recording_close_prompt: bool,
    #[serde(default)]
    pub capture_sharpen: SharpenLevel,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            clip_length: ClipLength::Sec30,
            buffer_mode: BufferMode::SystemRam,
            resolution: Resolution::P1080,
            fps: FrameRate::Fps60,
            codec: VideoCodec::Hevc,
            encoder_preference: EncoderPreference::Auto,
            rate_control: RateControl::Cqp,
            cqp_qp: 19,
            bitrate_mbps: 30,
            gop_secs: 1,
            container: ContainerFormat::Mp4,
            game_audio_device: "Default System Output".to_owned(),
            game_audio_volume: 100,
            mic_device: "Default Input Device".to_owned(),
            mic_enabled: true,
            mic_volume: 85,
            separate_audio_tracks: true,
            hotkey_save_clip: "F8".to_owned(),
            hotkey_toggle_mic: "Ctrl+M".to_owned(),
            hotkey_add_bookmark: "F9".to_owned(),
            clips_folder: default_clips_folder().to_string_lossy().into_owned(),
            max_storage_gb: 100,
            auto_delete_oldest: true,
            accent_color: AccentColor::Blue,
            replay_on: true,
            auto_start_capture: false,
            start_with_windows: false,
            close_ui_on_game_launch: false,
            background_image_path: String::new(),
            background_blur: 0,
            background_brightness: 55,
            auto_adjust_palette: true,
            custom_accent_rgb: None,
            steamgriddb_api_key: String::new(),
            custom_games: Vec::new(),
            ignored_games: Vec::new(),
            onboarding_completed: false,
            discord_rich_presence: true,
            discord_app_id: DEFAULT_DISCORD_APP_ID.to_owned(),
            clip_post_buffer_secs: 0,
            suppress_recording_close_prompt: false,
            capture_sharpen: SharpenLevel::Off,
        }
    }
}

pub const DEFAULT_DISCORD_APP_ID: &str = "1574498981218746409";

fn normalize_loaded_config(mut cfg: AppConfig) -> AppConfig {
    if cfg.background_brightness == 0 {
        cfg.background_brightness = 55;
    }
    if cfg.discord_app_id.trim().is_empty() {
        cfg.discord_app_id = DEFAULT_DISCORD_APP_ID.to_owned();
    }
    if cfg.codec == VideoCodec::H264 && cfg.fps == FrameRate::Fps120 {
        cfg.fps = FrameRate::Fps60;
    }
    cfg.clip_post_buffer_secs = match cfg.clip_post_buffer_secs {
        5 | 10 | 15 => cfg.clip_post_buffer_secs,
        _ => 0,
    };
    cfg
}

impl AppConfig {
    pub fn is_portable() -> bool {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("portable.flag").exists()))
            .unwrap_or(false)
    }

    pub fn app_data_dir() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if dir.join("portable.flag").exists() {
                    return dir.join("data");
                }
            }
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            PathBuf::from(appdata).join("Rewind")
        } else {
            PathBuf::from(".").join(".rewind")
        }
    }

    pub fn load_from_dir(dir: &Path) -> Self {
        let toml_path = dir.join("config.toml");
        if let Ok(raw) = fs::read_to_string(&toml_path) {
            if let Ok(cfg) = toml::from_str::<AppConfig>(&raw) {
                return normalize_loaded_config(cfg);
            }
        }
        let json_path = dir.join("config.json");
        if let Ok(raw) = fs::read_to_string(&json_path) {
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(&raw) {
                return normalize_loaded_config(cfg);
            }
        }
        let cfg = Self::default();
        let _ = cfg.save_to_dir(dir);
        cfg
    }

    pub fn save_to_dir(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let _ = fs::create_dir_all(&self.clips_folder);
        if let Ok(toml_txt) = toml::to_string_pretty(self) {
            fs::write(dir.join("config.toml"), toml_txt)?;
        }
        if let Ok(json_txt) = serde_json::to_string_pretty(self) {
            fs::write(dir.join("config.json"), json_txt)?;
        }
        Ok(())
    }

    pub fn estimated_clip_mb(&self) -> u32 {
        let effective_mbps = match self.rate_control {
            RateControl::Cqp => {
                let res_scale = match self.resolution {
                    Resolution::P1080 => 1.0,
                    Resolution::P1440 => 1.6,
                    Resolution::K4 => 2.8,
                };
                let fps_scale = match self.fps {
                    FrameRate::Fps30 => 0.65,
                    FrameRate::Fps60 => 1.0,
                    FrameRate::Fps120 => 1.55,
                };
                let codec_scale = match self.codec {
                    VideoCodec::Hevc => 1.0,
                    VideoCodec::Av1 => 0.80,
                    VideoCodec::H264 => 1.35,
                };
                let qp = self.cqp_qp.clamp(14, 28) as f32;
                let qp_scale = 2.0_f32.powf((20.0 - qp) / 6.0);
                (18.0 * res_scale * fps_scale * codec_scale * qp_scale).clamp(4.0, 120.0)
            }
            RateControl::Cbr => self.bitrate_mbps as f32,
        };
        ((effective_mbps * self.clip_length.seconds() as f32) / 8.0).round() as u32
    }

    pub fn ring_buffer_budget_bytes(&self) -> usize {
        (((self.estimated_clip_mb() as u64) * 12) / 10).clamp(64, 768) as usize * 1024 * 1024
    }

    pub fn top_bar_quality_label(&self) -> String {
        format!("{} {}", self.resolution.label(), self.fps.compact_label())
    }

    pub fn effective_accent_color(&self) -> Color32 {
        if let Some([r, g, b]) = self.custom_accent_rgb {
            return Color32::from_rgb(r, g, b);
        }
        self.accent_color.color()
    }

    pub fn effective_soft_bg(&self) -> Color32 {
        let c = self.effective_accent_color();
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 38)
    }
}

pub fn default_clips_folder() -> PathBuf {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        PathBuf::from(profile).join("Videos").join("Rewind")
    } else {
        PathBuf::from(".").join("RewindClips")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_toml_and_json() {
        let tmp = std::env::temp_dir().join("rewind_test_cfg");
        let _ = fs::remove_dir_all(&tmp);
        let mut cfg = AppConfig::default();
        cfg.clip_length = ClipLength::Min1;
        cfg.cqp_qp = 18;
        cfg.hotkey_save_clip = "F10".to_owned();
        cfg.save_to_dir(&tmp).unwrap();

        let loaded = AppConfig::load_from_dir(&tmp);
        assert_eq!(loaded, cfg);
        let _ = fs::remove_dir_all(&tmp);
    }
}

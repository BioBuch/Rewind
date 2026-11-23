use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowTextW, IsWindow,
    EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

static FOREGROUND_CHANGED: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedGame {
    pub hwnd: isize,
    pub exe_name: String,
    pub display_name: String,
    pub window_title: String,
    pub is_fullscreen: bool,
}

pub struct ForegroundHookGuard {
    hook: HWINEVENTHOOK,
}

impl Drop for ForegroundHookGuard {
    fn drop(&mut self) {
        if !self.hook.is_invalid() {
            unsafe {
                let _ = UnhookWinEvent(self.hook);
            }
        }
    }
}

unsafe extern "system" fn foreground_win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    FOREGROUND_CHANGED.store(true, Ordering::Release);
}

pub fn install_foreground_hook() -> Option<ForegroundHookGuard> {
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.is_invalid() {
        None
    } else {
        FOREGROUND_CHANGED.store(true, Ordering::Release);
        Some(ForegroundHookGuard { hook })
    }
}

pub fn take_foreground_changed() -> bool {
    FOREGROUND_CHANGED.swap(false, Ordering::AcqRel)
}

pub fn detect_foreground_game(
    custom_games: &[String],
    ignored_games: &[String],
) -> Option<DetectedGame> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let title = read_window_title(hwnd);
        let class_name = read_window_class(hwnd);
        let is_fullscreen = is_window_fullscreen(hwnd);
        classify_game_window(
            hwnd.0 as isize,
            &class_name,
            &title,
            is_fullscreen,
            custom_games,
            ignored_games,
        )
    }
}

pub fn is_window_alive(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    unsafe { IsWindow(HWND(hwnd as *mut _)).as_bool() }
}

unsafe fn read_window_title(hwnd: HWND) -> String {
    let mut buf = [0_u16; 256];
    let len = GetWindowTextW(hwnd, &mut buf);
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

unsafe fn read_window_class(hwnd: HWND) -> String {
    let mut buf = [0_u16; 256];
    let len = GetClassNameW(hwnd, &mut buf);
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

unsafe fn is_window_fullscreen(hwnd: HWND) -> bool {
    let mut w_rect = RECT::default();
    if GetWindowRect(hwnd, &mut w_rect).is_err() {
        return false;
    }
    let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !GetMonitorInfoW(mon, &mut mi).as_bool() {
        return false;
    }
    let w = w_rect.right - w_rect.left;
    let h = w_rect.bottom - w_rect.top;
    let mw = mi.rcMonitor.right - mi.rcMonitor.left;
    let mh = mi.rcMonitor.bottom - mi.rcMonitor.top;
    w >= mw && h >= mh && mw > 0 && mh > 0
}


pub fn normalize_game_entry(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains('\\') || trimmed.contains('/') {
        if let Some(name) = std::path::Path::new(trimmed).file_name() {
            return name.to_string_lossy().trim().to_owned();
        }
    }
    trimmed.to_owned()
}

fn clean_display_name(entry: &str) -> String {
    let norm = normalize_game_entry(entry);
    let mut base = norm.clone();
    for ext in [".exe", ".EXE"] {
        if base.ends_with(ext) {
            base.truncate(base.len() - ext.len());
            break;
        }
    }
    let lower = base.to_ascii_lowercase();
    for suffix in ["-win64-shipping", "_win64_shipping", "-shipping", "_shipping", "-win64", "_win64"] {
        if lower.ends_with(suffix) {
            base.truncate(base.len() - suffix.len());
            break;
        }
    }
    let spaced: String = base
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let picked = if spaced.is_empty() { norm } else { spaced };
    picked.chars().take(36).collect()
}

fn normalize_match_token(raw: &str) -> String {
    let mut s = normalize_game_entry(raw).to_ascii_lowercase();
    if let Some(stripped) = s.strip_suffix(".exe") {
        s = stripped.to_owned();
    }
    for suffix in ["-win64-shipping", "_win64_shipping", "-shipping", "_shipping", "-win64", "_win64"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.to_owned();
            break;
        }
    }
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn entry_matches_window(entry: &str, class_name: &str, window_title: &str) -> bool {
    let tok = normalize_match_token(entry);
    if tok.len() < 2 {
        return false;
    }
    let tok_title = normalize_match_token(window_title);
    let tok_class = normalize_match_token(class_name);
    if (!tok_title.is_empty() && tok_title.contains(&tok))
        || (!tok_class.is_empty() && tok_class.contains(&tok))
    {
        return true;
    }
    for suffix in ["client", "application", "launcher", "game"] {
        if let Some(stem) = tok.strip_suffix(suffix) {
            if stem.len() >= 4
                && ((!tok_title.is_empty() && tok_title.contains(stem))
                    || (!tok_class.is_empty() && tok_class.contains(stem)))
            {
                return true;
            }
        }
    }
    false
}

pub fn classify_game_window(
    hwnd: isize,
    class_name: &str,
    window_title: &str,
    is_fullscreen: bool,
    custom_games: &[String],
    ignored_games: &[String],
) -> Option<DetectedGame> {
    let title = window_title.trim();
    let lower_title = title.to_ascii_lowercase();
    if title.is_empty() {
        return None;
    }

    const SHELL_CLASSES: [&str; 4] = [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
    ];
    if SHELL_CLASSES.contains(&class_name) || lower_title.contains("rewind") {
        return None;
    }

    if ignored_games
        .iter()
        .any(|e| entry_matches_window(e, class_name, title))
    {
        return None;
    }

    if let Some(entry) = custom_games
        .iter()
        .find(|e| entry_matches_window(e, class_name, title))
    {
        return Some(DetectedGame {
            hwnd,
            exe_name: normalize_game_entry(entry),
            display_name: clean_display_name(entry),
            window_title: window_title.to_owned(),
            is_fullscreen,
        });
    }

    const NON_GAME_CLASSES: [&str; 12] = [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "CabinetWClass",
        "ConsoleWindowClass",
        "CASCADIA_HOSTING_WINDOW_CLASS",
        "ApplicationFrameWindow",
        "Windows.UI.Core.CoreWindow",
        "Chrome_WidgetWin_1",
        "MozillaWindowClass",
        "TaskManagerWindow",
    ];
    if NON_GAME_CLASSES.contains(&class_name) {
        return None;
    }

    const NON_GAME_TITLES: [&str; 8] = [
        "rewind",
        "visual studio",
        "antigravity",
        "discord",
        "spotify",
        "steam",
        "epic games launcher",
        "riot client",
    ];
    if NON_GAME_TITLES.iter().any(|t| lower_title.contains(t)) {
        return None;
    }

    const KNOWN: [(&str, &str); 13] = [
        ("valorant", "Valorant"),
        ("fortnite", "Fortnite"),
        ("counter-strike 2", "CS2"),
        ("minecraft", "Minecraft"),
        ("spider-man", "Spider-Man"),
        ("rocket league", "Rocket League"),
        ("apex legends", "Apex Legends"),
        ("overwatch", "Overwatch 2"),
        ("league of legends", "League of Legends"),
        ("grand theft auto v", "GTA V"),
        ("gta v", "GTA V"),
        ("elden ring", "Elden Ring"),
        ("cyberpunk 2077", "Cyberpunk 2077"),
    ];
    let known = KNOWN
        .iter()
        .find(|(needle, _)| lower_title.contains(needle))
        .map(|(_, name)| (*name).to_owned());

    const ENGINE_CLASSES: [&str; 8] = [
        "UnrealWindow",
        "UnityWndClass",
        "RiotWindowClass",
        "Valve001",
        "SDL_app",
        "GLFW30",
        "CryENGINE",
        "LWJGL",
    ];
    let engine = ENGINE_CLASSES.contains(&class_name);

    let display = match known {
        Some(name) => name,
        None if engine || is_fullscreen => {
            if title.len() <= 36 {
                title.to_owned()
            } else {
                title.chars().take(36).collect()
            }
        }
        None => return None,
    };

    Some(DetectedGame {
        hwnd,
        exe_name: class_name.to_owned(),
        display_name: display,
        window_title: window_title.to_owned(),
        is_fullscreen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_and_ignores_browsers() {
        assert!(classify_game_window(
            100,
            "Chrome_WidgetWin_1",
            "YouTube - Google Chrome",
            true,
            &[],
            &[]
        )
        .is_none());
        let g =
            classify_game_window(200, "VALORANTUnrealWindow", "VALORANT", true, &[], &[])
                .unwrap();
        assert_eq!(g.display_name, "Valorant");
        assert!(
            classify_game_window(300, "Progman", "Program Manager", true, &[], &[]).is_none()
        );
    }

    #[test]
    fn custom_and_ignored_executables_override_defaults() {
        let custom = vec![
            "C:\\Games\\MyIndieGame-Win64-Shipping.exe".to_owned(),
            "osu!.exe".to_owned(),
        ];
        let g1 = classify_game_window(
            400,
            "CustomWnd",
            "My Indie Game (64-bit)",
            false,
            &custom,
            &[],
        )
        .unwrap();
        assert_eq!(g1.display_name, "MyIndieGame");

        let g2 =
            classify_game_window(401, "SDL_app", "osu!lazer", false, &custom, &[]).unwrap();
        assert_eq!(g2.display_name, "osu!");

        let ignored = vec!["Yapfast.exe".to_owned()];
        assert!(classify_game_window(
            500,
            "Chrome_WidgetWin_1",
            "Yapfast",
            true,
            &[],
            &ignored
        )
        .is_none());
        assert!(classify_game_window(501, "UnrealWindow", "Yapfast", true, &[], &ignored).is_none());
    }
}

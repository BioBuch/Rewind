use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, BOOL, FALSE, HANDLE, HWND, LPARAM, POINT, RECT, TRUE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetMonitorInfoW, MonitorFromWindow, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_FLAGS_AND_ATTRIBUTES,
    INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3,
    VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_SPACE,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, DROPFILES, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, SetWindowPos, HWND_NOTOPMOST, HWND_TOP, SWP_NOOWNERZORDER, SWP_SHOWWINDOW,
};

pub fn to_wide_null(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn from_wide_slice(slice: &[u16]) -> String {
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..end])
}

pub fn qpc_now_100ns() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    let freq = *FREQ.get_or_init(|| {
        let mut f = 0_i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
        }
        f.max(1)
    });
    let mut counter = 0_i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
    }
    ((counter as i128 * 10_000_000_i128) / freq as i128) as i64
}

pub fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[repr(C)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(
        process: isize,
        ppsmemcounters: *mut ProcessMemoryCounters,
        cb: u32,
    ) -> i32;
}

pub fn current_process_ram_mb() -> f32 {
    let mut pmc = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb)
    };
    if ok != 0 {
        pmc.working_set_size as f32 / (1024.0 * 1024.0)
    } else {
        0.0
    }
}

fn pick_path_dialog(
    pick_folder: bool,
    title_str: &str,
    exe_filter: bool,
) -> Option<String> {
    unsafe {
        let init_hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let did_init = init_hr.is_ok();

        let result = (|| -> windows::core::Result<String> {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)?;
            let mut opts = dialog.GetOptions()? | FOS_FORCEFILESYSTEM;
            if pick_folder {
                opts |= FOS_PICKFOLDERS;
            }
            dialog.SetOptions(opts)?;
            if exe_filter && !pick_folder {
                let n1 = to_wide_null("Executable Files (*.exe)");
                let p1 = to_wide_null("*.exe");
                let n2 = to_wide_null("All Files (*.*)");
                let p2 = to_wide_null("*.*");
                let specs = [
                    COMDLG_FILTERSPEC {
                        pszName: PCWSTR(n1.as_ptr()),
                        pszSpec: PCWSTR(p1.as_ptr()),
                    },
                    COMDLG_FILTERSPEC {
                        pszName: PCWSTR(n2.as_ptr()),
                        pszSpec: PCWSTR(p2.as_ptr()),
                    },
                ];
                let _ = dialog.SetFileTypes(&specs);
                let _ = dialog.SetFileTypeIndex(1);
            }
            let title = to_wide_null(title_str);
            dialog.SetTitle(PCWSTR(title.as_ptr()))?;
            dialog.Show(HWND(std::ptr::null_mut()))?;
            let item = dialog.GetResult()?;
            let pwstr: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = pwstr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pwstr.0 as *const _));
            Ok(path)
        })();

        if did_init {
            CoUninitialize();
        }
        result.ok().filter(|p| !p.is_empty())
    }
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: isize,
        dwattribute: u32,
        pvattribute: *const u32,
        cbattribute: u32,
    ) -> i32;
}

unsafe extern "system" fn enum_thread_wnd_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    if IsWindowVisible(hwnd).as_bool() {
        let out = lparam.0 as *mut isize;
        if !out.is_null() {
            *out = hwnd.0 as isize;
        }
        return FALSE;
    }
    TRUE
}

fn find_own_main_hwnd() -> Option<HWND> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::EnumThreadWindows;

    unsafe {
        let mut found: isize = 0;
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(enum_thread_wnd_cb),
            LPARAM(&mut found as *mut isize as isize),
        );
        if found != 0 {
            Some(HWND(found as *mut _))
        } else {
            None
        }
    }
}

pub fn apply_win11_rounded_corners_to_active_window() -> bool {
    unsafe {
        let Some(hwnd) = find_own_main_hwnd() else {
            return false;
        };
        let pref: u32 = 2;
        DwmSetWindowAttribute(hwnd.0 as isize, 33, &pref, 4) == 0
    }
}

static LAST_WIN_HICON_SMALL: AtomicIsize = AtomicIsize::new(0);
static LAST_WIN_HICON_BIG: AtomicIsize = AtomicIsize::new(0);

pub fn create_hicon_from_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if rgba.len() < expected || width == 0 || height == 0 {
        return None;
    }

    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let Ok(hbm_color) = CreateDIBSection(HDC::default(), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return None;
        };
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbm_color.0));
            return None;
        }

        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, expected);
        for (src_px, dst_px) in rgba[..expected].chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            dst_px[0] = src_px[2];
            dst_px[1] = src_px[1];
            dst_px[2] = src_px[0];
            dst_px[3] = src_px[3];
        }

        let mask_stride = ((width as usize + 15) / 16) * 2;
        let mask = vec![0_u8; mask_stride * height as usize];
        let hbm_mask = CreateBitmap(
            width as i32,
            height as i32,
            1,
            1,
            Some(mask.as_ptr().cast()),
        );
        if hbm_mask.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbm_color.0));
            return None;
        }

        let info = ICONINFO {
            fIcon: TRUE,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        icon
    }
}

pub fn destroy_hicon(icon: windows::Win32::UI::WindowsAndMessaging::HICON) {
    if !icon.is_invalid() {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
        }
    }
}

pub fn set_active_window_icons_rgba(
    small_rgba: &[u8],
    small_sz: u32,
    big_rgba: &[u8],
    big_sz: u32,
) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageW, SetClassLongPtrW, GCLP_HICON, GCLP_HICONSM, ICON_BIG, ICON_SMALL, WM_SETICON,
    };

    unsafe {
        let Some(hwnd) = find_own_main_hwnd() else {
            return false;
        };
        let Some(small_icon) = create_hicon_from_rgba(small_rgba, small_sz, small_sz) else {
            return false;
        };
        let Some(big_icon) = create_hicon_from_rgba(big_rgba, big_sz, big_sz) else {
            destroy_hicon(small_icon);
            return false;
        };
        let _ = SetClassLongPtrW(hwnd, GCLP_HICONSM, small_icon.0 as isize);
        let _ = SetClassLongPtrW(hwnd, GCLP_HICON, big_icon.0 as isize);
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_SMALL as usize),
            LPARAM(small_icon.0 as isize),
        );
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_BIG as usize),
            LPARAM(big_icon.0 as isize),
        );
        let prev_small = LAST_WIN_HICON_SMALL.swap(small_icon.0 as isize, Ordering::AcqRel);
        if prev_small != 0 {
            destroy_hicon(windows::Win32::UI::WindowsAndMessaging::HICON(prev_small as *mut _));
        }
        let prev_big = LAST_WIN_HICON_BIG.swap(big_icon.0 as isize, Ordering::AcqRel);
        if prev_big != 0 {
            destroy_hicon(windows::Win32::UI::WindowsAndMessaging::HICON(prev_big as *mut _));
        }
        true
    }
}

pub fn pick_folder_dialog(_initial_dir: &str) -> Option<String> {
    pick_path_dialog(true, "Select Rewind Clips Folder", false)
}

pub fn pick_install_folder_dialog() -> Option<String> {
    pick_path_dialog(true, "Select Rewind Install Folder", false)
}

pub fn pick_image_file_dialog() -> Option<String> {
    pick_path_dialog(false, "Select Background Image (PNG / JPG)", false)
}

pub fn pick_exe_file_dialog() -> Option<String> {
    pick_path_dialog(false, "Select Game Executable (.exe)", true)
}

pub fn reveal_in_explorer(file_path: &str) {
    let p = Path::new(file_path);
    if p.exists() {
        let arg = format!("/select,\"{}\"", file_path);
        let _ = Command::new("explorer.exe").raw_arg_compat(&arg).spawn();
    } else if let Some(parent) = p.parent() {
        let _ = Command::new("explorer.exe").arg(parent).spawn();
    }
}

trait CommandRawExt {
    fn raw_arg_compat(&mut self, raw: &str) -> &mut Self;
}

impl CommandRawExt for Command {
    fn raw_arg_compat(&mut self, raw: &str) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.raw_arg(raw)
    }
}

pub fn parse_hotkey_combo(combo: &str) -> Option<(u32, u32)> {
    let mut mods = MOD_NOREPEAT.0;
    let mut main_vk: Option<u16> = None;

    for part in combo.split('+').map(str::trim) {
        match part.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CONTROL.0,
            "ALT" => mods |= MOD_ALT.0,
            "SHIFT" => mods |= MOD_SHIFT.0,
            other => main_vk = parse_vk(other),
        }
    }

    main_vk.map(|vk| (mods, vk as u32))
}

pub fn is_rewind_ui_open() -> bool {
    unsafe {
        let title = to_wide_null("Rewind");
        let hwnd = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
            PCWSTR::null(),
            PCWSTR(title.as_ptr()),
        )
        .unwrap_or_default();
        !hwnd.0.is_null()
    }
}

pub fn acquire_ui_single_instance_or_focus() -> Option<isize> {
    unsafe {
        let name = to_wide_null("Local\\RewindUiSingleInstance");
        let h = windows::Win32::System::Threading::CreateMutexW(
            None,
            false,
            PCWSTR(name.as_ptr()),
        )
        .ok()?;
        let already = windows::Win32::Foundation::GetLastError()
            == windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
        if already {
            let _ = windows::Win32::Foundation::CloseHandle(h);
            let title = to_wide_null("Rewind");
            let hwnd = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
                PCWSTR::null(),
                PCWSTR(title.as_ptr()),
            )
            .unwrap_or_default();
            if !hwnd.0.is_null() {
                let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::SW_RESTORE,
                );
                let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
            }
            return None;
        }
        Some(h.0 as isize)
    }
}

const RUN_REG_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_REG_VAL: &str = "RewindCaptureDaemon";

pub fn is_start_with_windows_enabled() -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("reg")
        .args(["query", RUN_REG_KEY, "/v", RUN_REG_VAL])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn set_start_with_windows(enabled: bool, daemon_exe: &Path) {
    use std::os::windows::process::CommandExt;
    if enabled {
        let cmd_val = format!("\"{}\"", daemon_exe.to_string_lossy());
        let _ = Command::new("reg")
            .args(["add", RUN_REG_KEY, "/v", RUN_REG_VAL, "/t", "REG_SZ", "/d", &cmd_val, "/f"])
            .creation_flags(0x0800_0000)
            .output();
    } else {
        let _ = Command::new("reg")
            .args(["delete", RUN_REG_KEY, "/v", RUN_REG_VAL, "/f"])
            .creation_flags(0x0800_0000)
            .output();
    }
}

pub fn copy_file_to_clipboard(file_path: &str) -> bool {
    if !Path::new(file_path).exists() {
        return false;
    }
    let mut wide_path = to_wide_null(file_path);
    wide_path.push(0);

    let header_size = std::mem::size_of::<DROPFILES>();
    let total_bytes = header_size + wide_path.len() * 2;
    let df = DROPFILES {
        pFiles: header_size as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: FALSE,
        fWide: TRUE,
    };

    unsafe {
        if OpenClipboard(HWND::default()).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, total_bytes) else {
            let _ = CloseClipboard();
            return false;
        };
        let ptr = GlobalLock(hmem) as *mut u8;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(&df as *const DROPFILES as *const u8, ptr, header_size);
        std::ptr::copy_nonoverlapping(
            wide_path.as_ptr() as *const u8,
            ptr.add(header_size),
            wide_path.len() * 2,
        );
        let _ = GlobalUnlock(hmem);
        let ok = SetClipboardData(15, HANDLE(hmem.0)).is_ok();
        let _ = CloseClipboard();
        ok
    }
}

pub struct HighResTimer {
    handle: HANDLE,
}

impl HighResTimer {
    pub fn new() -> Option<Self> {
        unsafe {
            let h = CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
            .or_else(|_| CreateWaitableTimerExW(None, PCWSTR::null(), 0, TIMER_ALL_ACCESS.0))
            .ok()?;
            Some(Self { handle: h })
        }
    }

    pub fn wait_100ns(&self, delta_100ns: i64) {
        if delta_100ns <= 0 {
            return;
        }
        let due: i64 = -delta_100ns;
        unsafe {
            if SetWaitableTimer(self.handle, &due, 0, None, None, false).is_ok() {
                let _ = WaitForSingleObject(self.handle, 250);
            }
        }
    }
}

impl Drop for HighResTimer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

static SAVED_FS_RECT: Mutex<Option<RECT>> = Mutex::new(None);

pub fn set_active_window_borderless_fullscreen(fullscreen: bool) -> bool {
    unsafe {
        let Some(hwnd) = find_own_main_hwnd() else {
            return false;
        };
        if fullscreen {
            if let Ok(mut guard) = SAVED_FS_RECT.lock() {
                if guard.is_none() {
                    let mut cur = RECT::default();
                    if GetWindowRect(hwnd, &mut cur).is_ok() {
                        *guard = Some(cur);
                    }
                }
            }
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return false;
            }
            let rc = mi.rcMonitor;
            SetWindowPos(
                hwnd,
                HWND_TOP,
                rc.left,
                rc.top,
                rc.right - rc.left,
                (rc.bottom - rc.top) + 1,
                SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
            )
            .is_ok()
        } else {
            let saved = SAVED_FS_RECT.lock().ok().and_then(|mut g| g.take());
            if let Some(rc) = saved {
                SetWindowPos(
                    hwnd,
                    HWND_NOTOPMOST,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                )
                .is_ok()
            } else {
                true
            }
        }
    }
}

fn parse_vk(name: &str) -> Option<u16> {
    match name {
        "F1" => Some(VK_F1.0),
        "F2" => Some(VK_F2.0),
        "F3" => Some(VK_F3.0),
        "F4" => Some(VK_F4.0),
        "F5" => Some(VK_F5.0),
        "F6" => Some(VK_F6.0),
        "F7" => Some(VK_F7.0),
        "F8" => Some(VK_F8.0),
        "F9" => Some(VK_F9.0),
        "F10" => Some(VK_F10.0),
        "F11" => Some(VK_F11.0),
        "F12" => Some(VK_F12.0),
        "SPACE" => Some(VK_SPACE.0),
        s if s.len() == 1 => {
            let b = s.as_bytes()[0];
            if b.is_ascii_alphanumeric() {
                Some(b as u16)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn hide_path(path: &Path) {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let attrs = GetFileAttributesW(PCWSTR(wide.as_ptr()));
        if attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_HIDDEN.0) == 0 {
            let _ = SetFileAttributesW(
                PCWSTR(wide.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(attrs | FILE_ATTRIBUTE_HIDDEN.0),
            );
        }
    }
}

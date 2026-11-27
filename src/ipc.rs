use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::capture::HardwareInfo;
use crate::config::AppConfig;
use crate::library::ClipRecord;
use crate::win_ffi::to_wide_null;

pub const PIPE_NAME: &str = r"\\.\pipe\RewindCaptureDaemonPipe";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonEvent {
    pub message: String,
    pub unix_secs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSnapshot {
    pub config: AppConfig,
    pub hardware: HardwareInfo,
    pub replay_on: bool,
    pub capture_active: bool,
    pub active_game: Option<String>,
    pub active_pid: Option<u32>,
    pub manual_source: Option<String>,
    pub ring_buffer_bytes: usize,
    pub clips: Vec<ClipRecord>,
    pub used_storage_bytes: u64,
    pub audio_outputs: Vec<String>,
    pub audio_inputs: Vec<String>,
    pub recent_events: Vec<DaemonEvent>,
    pub pending_undo_clip_id: Option<i64>,
    pub daemon_ram_mb: f32,
    #[serde(default)]
    pub clip_flash_counter: u64,
    #[serde(default)]
    pub auto_close_ui_counter: u64,
    #[serde(default)]
    pub daemon_pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "payload")]
pub enum IpcRequest {
    GetSnapshot,
    SetReplay(bool),
    SetCaptureSource(Option<String>),
    SaveClipNow,
    UpdateConfig(AppConfig),
    ToggleFavorite { id: i64 },
    RenameClip { id: i64, title: String },
    SetCollection { id: i64, collection: Option<String> },
    DeleteClip { id: i64 },
    UndoDelete { id: i64 },
    TrimClip {
        id: i64,
        start_secs: f32,
        end_secs: f32,
        vertical_9_16: bool,
    },
    PickFolder,
    RevealClip { id: i64 },
    SyncLibrary,
    ShutdownDaemon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub message: Option<String>,
    pub picked_folder: Option<String>,
    pub snapshot: Option<DaemonSnapshot>,
    #[serde(default)]
    pub restarting: bool,
}

impl IpcResponse {
    pub fn ok_snapshot(snapshot: DaemonSnapshot, message: Option<String>) -> Self {
        Self {
            ok: true,
            message,
            picked_folder: None,
            snapshot: Some(snapshot),
            restarting: false,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(msg.into()),
            picked_folder: None,
            snapshot: None,
            restarting: false,
        }
    }
}

pub fn send_request(req: &IpcRequest) -> Result<IpcResponse, String> {
    send_request_on_pipe(PIPE_NAME, req)
}

pub fn send_request_on_pipe(pipe_name: &str, req: &IpcRequest) -> Result<IpcResponse, String> {
    let payload = serde_json::to_string(req).map_err(|e| e.to_string())?;
    let wide = to_wide_null(pipe_name);

    let mut file = None;
    for attempt in 0..8 {
        match OpenOptions::new().read(true).write(true).open(pipe_name) {
            Ok(f) => {
                file = Some(f);
                break;
            }
            Err(_) => {
                unsafe {
                    let _ = WaitNamedPipeW(PCWSTR(wide.as_ptr()), 150);
                }
                if attempt < 7 {
                    thread::sleep(Duration::from_millis(30));
                }
            }
        }
    }

    let mut pipe = file.ok_or_else(|| "Daemon named pipe is not reachable".to_owned())?;
    pipe.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
    pipe.write_all(b"\n").map_err(|e| e.to_string())?;
    pipe.flush().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("read ipc response: {e}"))?;
    serde_json::from_str::<IpcResponse>(line.trim()).map_err(|e| format!("parse ipc response: {e}"))
}

pub fn ensure_daemon_running() -> bool {
    ensure_daemon_running_with_snapshot().0
}

pub fn ensure_daemon_running_with_snapshot() -> (bool, Option<DaemonSnapshot>) {
    if let Ok(resp) = send_request(&IpcRequest::GetSnapshot) {
        return (true, resp.snapshot);
    }

    if let Ok(cur_exe) = std::env::current_exe() {
        let daemon_exe = cur_exe.with_file_name("rewind-daemon.exe");
        if Path::new(&daemon_exe).exists() {
            let _ = Command::new(daemon_exe).spawn();
        }
    }

    for _ in 0..25 {
        thread::sleep(Duration::from_millis(80));
        if let Ok(resp) = send_request(&IpcRequest::GetSnapshot) {
            return (true, resp.snapshot);
        }
    }
    (false, None)
}

pub fn run_pipe_server<F>(pipe_name: &str, stop_flag: &std::sync::atomic::AtomicBool, handler: F)
where
    F: Fn(IpcRequest) -> IpcResponse + Send + Sync,
{
    let wide = to_wide_null(pipe_name);
    while !stop_flag.load(std::sync::atomic::Ordering::Acquire) {
        let handle: HANDLE = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                256 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let connected = unsafe {
            match ConnectNamedPipe(handle, None) {
                Ok(()) => true,
                Err(_) => GetLastError() == ERROR_PIPE_CONNECTED,
            }
        };

        if connected && !stop_flag.load(std::sync::atomic::Ordering::Acquire) {
            let line = read_pipe_line(handle);
            let resp = if line.trim().is_empty() {
                IpcResponse::err("Empty IPC request")
            } else {
                match serde_json::from_str::<IpcRequest>(line.trim()) {
                    Ok(req) => handler(req),
                    Err(e) => IpcResponse::err(format!("Invalid IPC JSON: {e}")),
                }
            };

            if let Ok(mut json) = serde_json::to_string(&resp) {
                json.push('\n');
                write_pipe_all(handle, json.as_bytes());
            }
            unsafe {
                let _ = FlushFileBuffers(handle);
                let _ = DisconnectNamedPipe(handle);
            }
        }

        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

fn read_pipe_line(handle: HANDLE) -> String {
    let mut out = Vec::with_capacity(1024);
    let mut buf = [0_u8; 4096];
    loop {
        let mut read_bytes = 0_u32;
        let ok = unsafe { ReadFile(handle, Some(&mut buf), Some(&mut read_bytes), None).is_ok() };
        if !ok || read_bytes == 0 {
            break;
        }
        let chunk = &buf[..read_bytes as usize];
        if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
            out.extend_from_slice(&chunk[..pos]);
            break;
        }
        out.extend_from_slice(chunk);
        if out.len() > 2 * 1024 * 1024 {
            break;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_pipe_all(handle: HANDLE, mut data: &[u8]) {
    while !data.is_empty() {
        let mut written = 0_u32;
        let ok = unsafe { WriteFile(handle, Some(data), Some(&mut written), None).is_ok() };
        if !ok || written == 0 {
            break;
        }
        data = &data[written as usize..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn named_pipe_roundtrip_serves_and_responds() {
        let pipe = format!(r"\\.\pipe\RewindUnitTestPipe_{}", std::process::id());
        let stop = Arc::new(AtomicBool::new(false));
        let stop_srv = stop.clone();
        let pipe_srv = pipe.clone();

        let srv_handle = thread::spawn(move || {
            run_pipe_server(&pipe_srv, &stop_srv, |req| match req {
                IpcRequest::SetReplay(v) => IpcResponse {
                    ok: true,
                    message: Some(format!("replay={v}")),
                    picked_folder: None,
                    snapshot: None,
                    restarting: false,
                },
                _ => IpcResponse::err("unexpected"),
            });
        });

        thread::sleep(Duration::from_millis(60));
        let resp = send_request_on_pipe(&pipe, &IpcRequest::SetReplay(true)).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.message.as_deref(), Some("replay=true"));

        stop.store(true, Ordering::Release);
        let _ = send_request_on_pipe(&pipe, &IpcRequest::ShutdownDaemon);
        let _ = srv_handle.join();
    }
}

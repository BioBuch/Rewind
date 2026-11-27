use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use serde_json::{json, Value};

pub struct DiscordRpcClient {
    pipe: Option<File>,
    connected_client_id: String,
    req_seq: u64,
}

impl DiscordRpcClient {
    pub fn new() -> Self {
        Self {
            pipe: None,
            connected_client_id: String::new(),
            req_seq: 0,
        }
    }

    pub fn is_connected_to(&self, client_id: &str) -> bool {
        self.pipe.is_some() && self.connected_client_id == client_id
    }

    pub fn disconnect(&mut self) {
        self.pipe = None;
        self.connected_client_id.clear();
    }

    pub fn ensure_connected(&mut self, client_id: &str) -> bool {
        let trimmed = client_id.trim();
        if trimmed.is_empty() {
            self.disconnect();
            return false;
        }
        if self.is_connected_to(trimmed) {
            return true;
        }
        self.disconnect();
        for idx in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{idx}");
            if let Ok(mut f) = OpenOptions::new().read(true).write(true).open(&path) {
                let handshake = json!({ "v": 1, "client_id": trimmed }).to_string();
                if write_frame(&mut f, 0, handshake.as_bytes()).is_ok()
                    && matches!(read_frame(&mut f), Some((1, _)))
                {
                    self.pipe = Some(f);
                    self.connected_client_id = trimmed.to_owned();
                    return true;
                }
            }
        }
        false
    }

    pub fn set_activity(
        &mut self,
        client_id: &str,
        name: &str,
        details: &str,
        state: &str,
        start_unix: i64,
    ) -> bool {
        if !self.ensure_connected(client_id) {
            return false;
        }
        self.req_seq = self.req_seq.wrapping_add(1);
        let payload = build_set_activity_json(
            std::process::id(),
            self.req_seq,
            name,
            details,
            state,
            start_unix,
        );
        let Some(f) = self.pipe.as_mut() else {
            return false;
        };
        if write_frame(f, 1, payload.as_bytes()).is_err() {
            self.disconnect();
            return false;
        }
        if !matches!(read_frame(f), Some((1, _))) {
            self.disconnect();
            return false;
        }
        true
    }

    pub fn clear_activity(&mut self) {
        let Some(f) = self.pipe.as_mut() else {
            return;
        };
        self.req_seq = self.req_seq.wrapping_add(1);
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id() },
            "nonce": format!("rewind-{}", self.req_seq)
        })
        .to_string();
        if write_frame(f, 1, payload.as_bytes()).is_err() {
            self.disconnect();
            return;
        }
        let _ = read_frame(f);
    }
}

pub fn build_activity_strings(
    active_game: Option<&str>,
    replay_on: bool,
    _resolution_label: &str,
    _fps: u32,
) -> (String, String) {
    let replay_lbl = if replay_on {
        "Replay On".to_owned()
    } else {
        "Replay Paused".to_owned()
    };
    match active_game {
        Some("Desktop") => ("Idle on Desktop".to_owned(), replay_lbl),
        Some(_) => (replay_lbl, String::new()),
        None => ("Idle · Ready to Clip".to_owned(), replay_lbl),
    }
}

pub fn build_set_activity_json(
    pid: u32,
    req_id: u64,
    name: &str,
    details: &str,
    state: &str,
    start_unix: i64,
) -> String {
    let mut activity = json!({
        "name": name,
        "details": details,
        "status_display_type": 0,
        "timestamps": { "start": start_unix },
        "assets": {
            "large_image": "rewind_logo",
            "large_text": "Rewind — A clipping software"
        },
        "buttons": [
            {
                "label": "Download Rewind",
                "url": "https://github.com/BioBuch/Rewind/releases/latest"
            }
        ]
    });
    if !state.is_empty() {
        activity["state"] = Value::String(state.to_owned());
    }
    json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": activity
        },
        "nonce": format!("rewind-{req_id}")
    })
    .to_string()
}

pub fn encode_ipc_frame(opcode: u32, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn write_frame(f: &mut File, opcode: u32, payload: &[u8]) -> std::io::Result<()> {
    let frame = encode_ipc_frame(opcode, payload);
    f.write_all(&frame)?;
    f.flush()
}

fn read_frame(f: &mut File) -> Option<(u32, Value)> {
    let mut hdr = [0_u8; 8];
    f.read_exact(&mut hdr).ok()?;
    let op = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    if len > 256 * 1024 {
        return None;
    }
    let mut body = vec![0_u8; len];
    f.read_exact(&mut body).ok()?;
    let val = serde_json::from_slice(&body).ok()?;
    Some((op, val))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_discord_ipc_frame_and_activity_payload() {
        let (details, state) = build_activity_strings(Some("Fortnite"), true, "1080p", 120);
        assert_eq!(details, "Replay On");
        assert_eq!(state, "");
        let (d_desk, s_desk) = build_activity_strings(Some("Desktop"), false, "1080p", 120);
        assert_eq!(d_desk, "Idle on Desktop");
        assert_eq!(s_desk, "Replay Paused");
        let (d_idle, s_idle) = build_activity_strings(None, true, "1080p", 120);
        assert_eq!(d_idle, "Idle · Ready to Clip");
        assert_eq!(s_idle, "Replay On");

        let json_str = build_set_activity_json(1234, 7, "Fortnite With Rewind", &details, &state, 1_700_000_000);
        let frame = encode_ipc_frame(1, json_str.as_bytes());
        assert_eq!(u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]), 1);
        assert_eq!(
            u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize,
            json_str.len()
        );
        let parsed: Value = serde_json::from_slice(&frame[8..]).expect("valid json");
        assert_eq!(parsed["cmd"], "SET_ACTIVITY");
        assert_eq!(parsed["args"]["activity"]["name"], "Fortnite With Rewind");
        assert_eq!(parsed["args"]["activity"]["details"], "Replay On");
        assert!(parsed["args"]["activity"]["state"].is_null());
        assert_eq!(parsed["args"]["activity"]["status_display_type"], 0);
        assert_eq!(
            parsed["args"]["activity"]["buttons"][0]["label"],
            "Download Rewind"
        );
    }
}

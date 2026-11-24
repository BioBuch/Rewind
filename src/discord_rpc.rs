use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use serde_json::{json, Value};

pub struct DiscordRpcClient {
    pipe: Option<File>,
    connected_client_id: String,
    nonce: u64,
}

impl DiscordRpcClient {
    pub fn new() -> Self {
        Self {
            pipe: None,
            connected_client_id: String::new(),
            nonce: 0,
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
                if write_frame(&mut f, 0, handshake.as_bytes()).is_ok() {
                    let _ = read_frame(&mut f);
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
        details: &str,
        state: &str,
        start_unix: i64,
    ) -> bool {
        if !self.ensure_connected(client_id) {
            return false;
        }
        self.nonce = self.nonce.wrapping_add(1);
        let payload = build_set_activity_json(
            std::process::id(),
            self.nonce,
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
        let _ = read_frame(f);
        true
    }

    pub fn clear_activity(&mut self) {
        let Some(f) = self.pipe.as_mut() else {
            return;
        };
        self.nonce = self.nonce.wrapping_add(1);
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id() },
            "nonce": format!("rewind-{}", self.nonce)
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
    let details = match (active_game, replay_on) {
        (Some(game), _) => format!("Clipping {game}"),
        (None, true) => "Idle · Ready to Clip".to_owned(),
        (None, false) => "Idle · Ready to Clip".to_owned(),
    };
    let state = if replay_on {
        "with Rewind".to_owned()
    } else {
        "with Rewind · Paused".to_owned()
    };
    (details, state)
}

pub fn build_set_activity_json(
    pid: u32,
    nonce: u64,
    details: &str,
    state: &str,
    start_unix: i64,
) -> String {
    json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": {
                "details": details,
                "state": state,
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
            }
        },
        "nonce": format!("rewind-{nonce}")
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
        assert_eq!(details, "Clipping Fortnite");
        assert_eq!(state, "with Rewind");
        assert_eq!(
            build_activity_strings(Some("Desktop"), true, "1080p", 120).0,
            "Clipping Desktop"
        );
        assert_eq!(
            build_activity_strings(None, true, "1080p", 120).0,
            "Idle · Ready to Clip"
        );

        let json_str = build_set_activity_json(1234, 7, &details, &state, 1_700_000_000);
        let frame = encode_ipc_frame(1, json_str.as_bytes());
        assert_eq!(u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]), 1);
        assert_eq!(
            u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize,
            json_str.len()
        );
        let parsed: Value = serde_json::from_slice(&frame[8..]).expect("valid json");
        assert_eq!(parsed["cmd"], "SET_ACTIVITY");
        assert_eq!(parsed["args"]["activity"]["details"], "Clipping Fortnite");
        assert_eq!(
            parsed["args"]["activity"]["buttons"][0]["label"],
            "Download Rewind"
        );
    }
}

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use image::imageops::FilterType;

use crate::config::AppConfig;

pub const STEAMGRIDDB_API_KEY_URL: &str = "https://www.steamgriddb.com/profile/preferences/api";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn is_fetchable_game(game_name: &str) -> bool {
    let trimmed = game_name.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    !matches!(lower.as_str(), "desktop" | "unknown" | "general")
}

pub fn cache_path_for_game(game_name: &str) -> PathBuf {
    let mut slug = String::with_capacity(game_name.len());
    for ch in game_name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let trimmed = slug.trim_matches('_');
    let fname = if trimmed.is_empty() { "game" } else { trimmed };
    AppConfig::app_data_dir()
        .join("game_icons")
        .join(format!("{fname}.png"))
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.trim().as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

fn curl_get_bytes(url: &str, api_key: Option<&str>, timeout_secs: &str) -> Option<Vec<u8>> {
    let mut cmd = Command::new("curl.exe");
    cmd.args(["-sS", "-L", "--max-time", timeout_secs]);
    if let Some(key) = api_key {
        cmd.args(["-H", &format!("Authorization: Bearer {key}")]);
    }
    cmd.arg(url).creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output().ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        Some(out.stdout)
    } else {
        None
    }
}

fn pick_icon_url(json_bytes: &[u8]) -> Option<String> {
    let val: serde_json::Value = serde_json::from_slice(json_bytes).ok()?;
    let items = val.get("data")?.as_array()?;
    for item in items {
        let mime = item
            .get("mime")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let thumb = item.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
        if mime.contains("x-icon") || mime.contains("microsoft.icon") {
            if !thumb.is_empty() {
                return Some(thumb.to_owned());
            }
        } else if !url.is_empty() {
            return Some(url.to_owned());
        } else if !thumb.is_empty() {
            return Some(thumb.to_owned());
        }
    }
    None
}

fn pick_best_game_ids(search_json: &serde_json::Value, game_name: &str) -> Vec<u64> {
    let Some(items) = search_json.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let target = game_name.trim();
    let target_norm: String = target
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    let mut ranked: Vec<(u8, u64)> = items
        .iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_u64()?;
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            let norm: String = name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let rank = if name.eq_ignore_ascii_case(target) {
                0
            } else if !target_norm.is_empty() && norm == target_norm {
                1
            } else {
                2
            };
            Some((rank, id))
        })
        .collect();

    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, id)| id).take(3).collect()
}

pub fn fetch_and_cache_game_icon(game_name: &str, api_key: &str) -> Option<PathBuf> {
    if !is_fetchable_game(game_name) {
        return None;
    }
    let cache_path = cache_path_for_game(game_name);
    if cache_path.exists() {
        return Some(cache_path);
    }
    let key = api_key.trim();
    if key.is_empty() {
        return None;
    }

    let encoded = url_encode(game_name);
    let search_url = format!("https://www.steamgriddb.com/api/v2/search/autocomplete/{encoded}");
    let search_bytes = curl_get_bytes(&search_url, Some(key), "8")?;
    let search_json: serde_json::Value = serde_json::from_slice(&search_bytes).ok()?;
    let candidate_ids = pick_best_game_ids(&search_json, game_name);

    let mut img_url = None;
    for game_id in candidate_ids {
        let png_icons_url =
            format!("https://www.steamgriddb.com/api/v2/icons/game/{game_id}?mimes=image/png&types=static");
        img_url = curl_get_bytes(&png_icons_url, Some(key), "8")
            .and_then(|b| pick_icon_url(&b))
            .or_else(|| {
                let any_url =
                    format!("https://www.steamgriddb.com/api/v2/icons/game/{game_id}?types=static");
                curl_get_bytes(&any_url, Some(key), "8").and_then(|b| pick_icon_url(&b))
            })
            .or_else(|| {
                let grid_url =
                    format!("https://www.steamgriddb.com/api/v2/grids/game/{game_id}?dimensions=512x512,1024x1024&types=static");
                curl_get_bytes(&grid_url, Some(key), "8").and_then(|b| pick_icon_url(&b))
            });
        if img_url.is_some() {
            break;
        }
    }
    let img_url = img_url?;

    let img_bytes = curl_get_bytes(&img_url, None, "10")?;
    let dyn_img = image::load_from_memory(&img_bytes).ok()?;
    let rgba = dyn_img.to_rgba8();
    let (rw, rh) = rgba.dimensions();
    let side = rw.min(rh);
    let cropped = if rw != rh && side > 0 {
        image::imageops::crop_imm(&rgba, (rw - side) / 2, (rh - side) / 2, side, side).to_image()
    } else {
        rgba
    };
    let resized = image::imageops::resize(&cropped, 64, 64, FilterType::CatmullRom);

    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    resized
        .save_with_format(&cache_path, image::ImageFormat::Png)
        .ok()?;
    Some(cache_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_game_icon_helpers() {
        assert_eq!(url_encode("Counter-Strike 2"), "Counter-Strike%202");
        assert!(is_fetchable_game("Valorant"));
        assert!(!is_fetchable_game("Desktop"));
        assert!(!is_fetchable_game(""));
        let p = cache_path_for_game("Counter-Strike 2");
        assert!(p.to_string_lossy().ends_with("counter_strike_2.png"));

        let sample = serde_json::json!({
            "data": [
                { "id": 5341559, "name": "Rob & Run" },
                { "id": 5529636, "name": "ROB RUN" }
            ]
        });
        assert_eq!(pick_best_game_ids(&sample, "ROB RUN"), vec![5529636, 5341559]);
    }
}

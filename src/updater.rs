use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_API_URL: &str = "https://api.github.com/repos/BioBuch/Rewind/releases/latest";
pub const RELEASES_PAGE_URL: &str = "https://github.com/BioBuch/Rewind/releases";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub html_url: String,
    pub installer_url: Option<String>,
    pub notes: String,
}

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next().unwrap_or(trimmed);
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let second = parts.next();
    let third = parts.next();
    let (minor, patch) = match (second, third) {
        (Some(s), None) if s.len() > 1 && s.starts_with('0') => (0, s.parse::<u64>().ok()?),
        (s, p) => (
            s.unwrap_or("0").parse::<u64>().ok()?,
            p.unwrap_or("0").parse::<u64>().ok()?,
        ),
    };
    Some((major, minor, patch))
}

pub fn is_newer_version(remote_tag: &str, current: &str) -> bool {
    match (parse_semver(remote_tag), parse_semver(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

pub fn check_latest_release() -> Result<Option<ReleaseInfo>, String> {
    let output = Command::new("curl.exe")
        .args([
            "-sSL",
            "--max-time",
            "8",
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            "Rewind-Updater",
            RELEASES_API_URL,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to check updates: {e}"))?;

    if !output.status.success() {
        return Err("Unable to reach GitHub Releases".to_owned());
    }

    let body_str = String::from_utf8_lossy(&output.stdout);
    let json: Value =
        serde_json::from_str(&body_str).map_err(|e| format!("Invalid release JSON: {e}"))?;

    let Some(tag_name) = json.get("tag_name").and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    if !is_newer_version(tag_name, CURRENT_VERSION) {
        return Ok(None);
    }

    let version = tag_name.trim_start_matches(['v', 'V']).to_owned();
    let html_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or(RELEASES_PAGE_URL)
        .to_owned();
    let notes = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let mut installer_url: Option<String> = None;
    if let Some(assets) = json.get("assets").and_then(|v| v.as_array()) {
        for asset in assets {
            let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let dl = asset
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !dl.is_empty()
                && (name.eq_ignore_ascii_case("Rewind-Setup-x64.exe")
                    || name.to_ascii_lowercase().ends_with(".exe"))
            {
                installer_url = Some(dl.to_owned());
                if name.eq_ignore_ascii_case("Rewind-Setup-x64.exe") {
                    break;
                }
            }
        }
    }

    Ok(Some(ReleaseInfo {
        tag_name: tag_name.to_owned(),
        version,
        html_url,
        installer_url,
        notes,
    }))
}

pub fn download_and_launch_installer(info: &ReleaseInfo) -> Result<PathBuf, String> {
    let Some(url) = &info.installer_url else {
        open_url(&info.html_url);
        return Err("No installer .exe found on release; opened Releases page.".to_owned());
    };

    let dst = std::env::temp_dir().join(format!("Rewind-Setup-{}.exe", info.version));
    let _ = std::fs::remove_file(&dst);

    let dst_str = dst.to_string_lossy().into_owned();
    let status = Command::new("curl.exe")
        .args([
            "-fSL",
            "--max-time",
            "180",
            "-A",
            "Rewind-Updater",
            "-o",
            &dst_str,
            url,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("Failed to download installer: {e}"))?;

    if !status.success() {
        return Err("Installer download failed".to_owned());
    }

    let meta = std::fs::metadata(&dst).map_err(|e| format!("Downloaded file missing: {e}"))?;
    if meta.len() < 100_000 {
        return Err("Downloaded installer is incomplete".to_owned());
    }

    Command::new(&dst)
        .spawn()
        .map_err(|e| format!("Failed to launch installer: {e}"))?;

    Ok(dst)
}

pub fn open_url(url: &str) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_release_tags_accurately() {
        assert!(is_newer_version("v0.2.1", "0.2.0"));
        assert!(is_newer_version("v0.3.0", "0.2.0"));
        assert!(is_newer_version("v1.0.0", "0.2.9"));
        assert!(!is_newer_version("v0.2.0", "0.2.0"));
        assert!(!is_newer_version("v0.1.9", "0.2.0"));
        assert!(!is_newer_version("v1.01", "1.0.1"));
        assert!(is_newer_version("v1.02", "1.0.1"));
    }
}

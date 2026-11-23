use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use egui::Color32;
use image::{imageops::FilterType, ImageFormat, RgbaImage};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

const ICON_RED_PNG: &[u8] = include_bytes!("../assets/icon_red.png");
const ICON_GREEN_PNG: &[u8] = include_bytes!("../assets/icon_green.png");
const ICON_BLUE_PNG: &[u8] = include_bytes!("../assets/icon_blue.png");

const HUE_RED: f32 = 349.1;
const HUE_GREEN: f32 = 168.1;
const HUE_BLUE: f32 = 212.1;

#[derive(Clone, Copy)]
enum BaseIcon {
    Red,
    Green,
    Blue,
}

static BASE_RED_256: OnceLock<RgbaImage> = OnceLock::new();
static BASE_GREEN_256: OnceLock<RgbaImage> = OnceLock::new();
static BASE_BLUE_256: OnceLock<RgbaImage> = OnceLock::new();

fn png_bytes(base: BaseIcon) -> &'static [u8] {
    match base {
        BaseIcon::Red => ICON_RED_PNG,
        BaseIcon::Green => ICON_GREEN_PNG,
        BaseIcon::Blue => ICON_BLUE_PNG,
    }
}

fn decode_256(base: BaseIcon) -> &'static RgbaImage {
    let slot = match base {
        BaseIcon::Red => &BASE_RED_256,
        BaseIcon::Green => &BASE_GREEN_256,
        BaseIcon::Blue => &BASE_BLUE_256,
    };
    slot.get_or_init(|| {
        let img = image::load_from_memory(png_bytes(base)).expect("embedded icon png");
        let rgba = img.to_rgba8();
        if rgba.width() == 256 && rgba.height() == 256 {
            rgba
        } else {
            image::imageops::resize(&rgba, 256, 256, FilterType::Lanczos3)
        }
    })
}

fn anchor_hue(base: BaseIcon) -> f32 {
    match base {
        BaseIcon::Red => HUE_RED,
        BaseIcon::Green => HUE_GREEN,
        BaseIcon::Blue => HUE_BLUE,
    }
}

fn hue_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn rgb_to_hue_sat(r: u8, g: u8, b: u8) -> (f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max_c = rf.max(gf).max(bf);
    let min_c = rf.min(gf).min(bf);
    let delta = max_c - min_c;
    if max_c <= 0.0 || delta <= 1e-6 {
        return (HUE_BLUE, 0.0);
    }
    let sat = delta / max_c;
    let hue = if (max_c - rf).abs() < 1e-6 {
        60.0 * (((gf - bf) / delta).rem_euclid(6.0))
    } else if (max_c - gf).abs() < 1e-6 {
        60.0 * (((bf - rf) / delta) + 2.0)
    } else {
        60.0 * (((rf - gf) / delta) + 4.0)
    };
    (hue.rem_euclid(360.0), sat)
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max_c = rf.max(gf).max(bf);
    let min_c = rf.min(gf).min(bf);
    let l = (max_c + min_c) * 0.5;
    let delta = max_c - min_c;
    if delta <= 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = delta / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
    let h = if (max_c - rf).abs() < 1e-6 {
        60.0 * (((gf - bf) / delta).rem_euclid(6.0))
    } else if (max_c - gf).abs() < 1e-6 {
        60.0 * (((bf - rf) / delta) + 2.0)
    } else {
        60.0 * (((rf - gf) / delta) + 4.0)
    };
    (h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
}

fn hue_to_rgb_chan(p: f32, q: f32, t: f32) -> f32 {
    let mut t = t.rem_euclid(1.0);
    if t < 0.0 {
        t += 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    if s <= 1e-6 {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hk = h.rem_euclid(360.0) / 360.0;
    let r = hue_to_rgb_chan(p, q, hk + 1.0 / 3.0);
    let g = hue_to_rgb_chan(p, q, hk);
    let b = hue_to_rgb_chan(p, q, hk - 1.0 / 3.0);
    [
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn resize_premultiplied(src: &RgbaImage, size: u32) -> Vec<u8> {
    let mut premul = src.clone();
    for px in premul.pixels_mut() {
        let a = px[3] as f32 / 255.0;
        px[0] = (px[0] as f32 * a).round() as u8;
        px[1] = (px[1] as f32 * a).round() as u8;
        px[2] = (px[2] as f32 * a).round() as u8;
    }
    let mut out = image::imageops::resize(&premul, size, size, FilterType::CatmullRom).into_raw();
    for px in out.chunks_exact_mut(4) {
        if px[3] > 0 && px[3] < 255 {
            let a = px[3] as f32 / 255.0;
            px[0] = (px[0] as f32 / a).clamp(0.0, 255.0).round() as u8;
            px[1] = (px[1] as f32 / a).clamp(0.0, 255.0).round() as u8;
            px[2] = (px[2] as f32 / a).clamp(0.0, 255.0).round() as u8;
        }
    }
    out
}

fn apply_smooth_rounded_mask(rgba: &mut [u8], size: u32) {
    let s = size as f32;
    let center = (s - 1.0) * 0.5;
    let half = s * (103.5 / 256.0);
    let radius = s * (42.0 / 256.0);
    let inner = half - radius;
    let aa = (s / 256.0).max(0.75);

    for y in 0..size {
        let dy = ((y as f32 - center).abs() - inner).max(0.0);
        for x in 0..size {
            let dx = ((x as f32 - center).abs() - inner).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt() - radius;
            let idx = ((y * size + x) * 4) as usize;
            if dist >= aa {
                rgba[idx + 3] = 0;
            } else if dist > -aa {
                let cov = ((aa - dist) / (2.0 * aa)).clamp(0.0, 1.0);
                rgba[idx + 3] = ((rgba[idx + 3] as f32) * cov).round() as u8;
            }
        }
    }
}

pub fn render_icon_rgba(target: Color32, size: u32) -> Vec<u8> {
    let size = size.max(1);
    let (target_h, target_sat) = rgb_to_hue_sat(target.r(), target.g(), target.b());
    let base = if target_sat < 0.05 {
        BaseIcon::Blue
    } else {
        let candidates = [BaseIcon::Red, BaseIcon::Green, BaseIcon::Blue];
        candidates
            .into_iter()
            .min_by(|&a, &b| {
                hue_dist(target_h, anchor_hue(a))
                    .partial_cmp(&hue_dist(target_h, anchor_hue(b)))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(BaseIcon::Blue)
    };

    let delta_h = (target_h - anchor_hue(base) + 540.0).rem_euclid(360.0) - 180.0;
    let sat_scale = if target_sat >= 0.30 {
        1.0
    } else {
        (target_sat / 0.30).clamp(0.0, 1.0)
    };

    let mut rgba = if size == 256 {
        decode_256(base).as_raw().clone()
    } else {
        resize_premultiplied(decode_256(base), size)
    };
    apply_smooth_rounded_mask(&mut rgba, size);

    if delta_h.abs() < 3.0 && (sat_scale - 1.0).abs() < 0.01 {
        return rgba;
    }

    for px in rgba.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue;
        }
        let (h, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
        let h2 = (h + delta_h).rem_euclid(360.0);
        let s2 = (s * sat_scale).clamp(0.0, 1.0);
        let [nr, ng, nb] = hsl_to_rgb(h2, s2, l);
        px[0] = nr;
        px[1] = ng;
        px[2] = nb;
    }

    rgba
}

pub fn render_ico_bytes(target: Color32) -> Vec<u8> {
    const SIZES: [u32; 3] = [256, 64, 32];
    let mut frames: Vec<(u32, Vec<u8>)> = Vec::with_capacity(SIZES.len());
    for sz in SIZES {
        let rgba = render_icon_rgba(target, sz);
        if let Some(img) = RgbaImage::from_raw(sz, sz, rgba) {
            let mut png = Vec::new();
            if img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).is_ok() {
                frames.push((sz, png));
            }
        }
    }
    if frames.is_empty() {
        return Vec::new();
    }

    let count = frames.len() as u16;
    let header_len = 6 + frames.len() * 16;
    let payload_len: usize = frames.iter().map(|(_, p)| p.len()).sum();
    let mut out = Vec::with_capacity(header_len + payload_len);

    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&count.to_le_bytes());

    let mut offset = header_len as u32;
    for (sz, png) in &frames {
        let dim = if *sz >= 256 { 0_u8 } else { *sz as u8 };
        let len = png.len() as u32;
        out.push(dim);
        out.push(dim);
        out.push(0);
        out.push(0);
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&32_u16.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset = offset.saturating_add(len);
    }

    for (_, png) in frames {
        out.extend_from_slice(&png);
    }
    out
}

pub fn default_ico_bytes() -> Vec<u8> {
    render_ico_bytes(Color32::from_rgb(76, 141, 255))
}

fn write_ico_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("ico.tmp");
    if fs::write(&tmp, bytes).is_ok() {
        if fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
            let _ = fs::write(path, bytes);
        }
    } else {
        let _ = fs::write(path, bytes);
    }
}

pub fn export_ico_to_install_dir(target: Color32) {
    let bytes = render_ico_bytes(target);
    if bytes.is_empty() {
        return;
    }
    let mut updated = false;
    let mut install_ico: Option<PathBuf> = None;
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let install_dir = PathBuf::from(local_app_data).join("Programs").join("Rewind");
        if install_dir.is_dir() {
            let ico_path = install_dir.join("rewind.ico");
            write_ico_atomic(&ico_path, &bytes);
            install_ico = Some(ico_path);
            updated = true;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("rewind.ico");
            if candidate.exists() && install_ico.as_deref() != Some(candidate.as_path()) {
                write_ico_atomic(&candidate, &bytes);
                updated = true;
            }
        }
    }
    if updated {
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AccentColor;

    fn avg_vibrant_hue_and_sat(rgba: &[u8]) -> (f32, f32) {
        let mut sin_sum = 0.0_f32;
        let mut cos_sum = 0.0_f32;
        let mut w_sum = 0.0_f32;
        let mut sat_sum = 0.0_f32;
        let mut sat_cnt = 0_u32;

        for px in rgba.chunks_exact(4) {
            if px[3] < 200 {
                continue;
            }
            let (h, s) = rgb_to_hue_sat(px[0], px[1], px[2]);
            let lum = (px[0].max(px[1]).max(px[2]) as f32) / 255.0;
            sat_sum += s;
            sat_cnt += 1;
            if s > 0.35 && lum > 0.45 {
                let w = s * s;
                let rad = h.to_radians();
                sin_sum += rad.sin() * w;
                cos_sum += rad.cos() * w;
                w_sum += w;
            }
        }
        let avg_h = if w_sum > 0.0 {
            sin_sum.atan2(cos_sum).to_degrees().rem_euclid(360.0)
        } else {
            0.0
        };
        let avg_sat = sat_sum / (sat_cnt.max(1) as f32);
        (avg_h, avg_sat)
    }

    #[test]
    fn icon_dimensions_alpha_and_recoloring() {
        let blue = AccentColor::Blue.color();
        let rgba = render_icon_rgba(blue, 64);
        assert_eq!(rgba.len(), 64 * 64 * 4);
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[(63 * 64 + 63) * 4 + 3], 0);
        assert_eq!(rgba[(32 * 64 + 32) * 4 + 3], 255);

        for accent in AccentColor::ALL {
            let col = accent.color();
            let rendered = render_icon_rgba(col, 64);
            let (target_h, _) = rgb_to_hue_sat(col.r(), col.g(), col.b());
            let (got_h, _) = avg_vibrant_hue_and_sat(&rendered);
            assert!(
                hue_dist(got_h, target_h) < 25.0,
                "accent {:?}: expected hue near {}, got {}",
                accent,
                target_h,
                got_h
            );
        }

        let mono = render_icon_rgba(Color32::from_rgb(210, 214, 220), 64);
        let (_, mono_sat) = avg_vibrant_hue_and_sat(&mono);
        assert!(mono_sat < 0.18, "mono saturation too high: {}", mono_sat);
    }

    #[test]
    fn ico_bytes_valid_multi_size() {
        let ico = render_ico_bytes(AccentColor::Coral.color());
        assert!(ico.len() > 64);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 3, 0]);
        assert_eq!(ico[6], 0);
        assert_eq!(ico[22], 64);
        assert_eq!(ico[38], 32);
        let first_off = u32::from_le_bytes([ico[18], ico[19], ico[20], ico[21]]) as usize;
        assert_eq!(first_off, 54);
        assert_eq!(&ico[first_off..first_off + 4], &[0x89, b'P', b'N', b'G']);

        let def_ico = default_ico_bytes();
        assert_eq!(&def_ico[0..6], &[0, 0, 1, 0, 3, 0]);
    }
}

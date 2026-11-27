#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use egui::{
    Align, Color32, CursorIcon, FontId, Id, Layout, Margin, Pos2, Rect, Rounding, Sense, Stroke,
    Ui, Vec2, ViewportCommand, Visuals,
};
use rewind::win_ffi::{apply_win11_rounded_corners_to_active_window, pick_install_folder_dialog};

const PAYLOAD_REWIND_UI: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload_rewind.exe"));
const PAYLOAD_REWIND_DAEMON: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/payload_rewind_daemon.exe"));

const ACCENT: Color32 = Color32::from_rgb(56, 189, 248);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(245, 247, 250);
const TEXT_MUTED: Color32 = Color32::from_rgb(148, 163, 184);
const CARD_BG: Color32 = Color32::from_rgb(19, 24, 33);
const BORDER_COL: Color32 = Color32::from_rgb(38, 46, 62);

enum SetupPhase {
    Ready,
    Installing {
        progress: Arc<AtomicU32>,
        result: Arc<Mutex<Option<Result<String, String>>>>,
    },
    Done {
        done_at: f64,
    },
    Error(String),
}

struct InstallerApp {
    install_dir: String,
    portable: bool,
    phase: SetupPhase,
    corners_applied: bool,
    opened_at: Option<f64>,
}

impl InstallerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        apply_visuals(&cc.egui_ctx);
        let default_dir = std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Programs").join("Rewind"))
            .unwrap_or_else(|_| PathBuf::from(r"C:\Rewind"))
            .to_string_lossy()
            .into_owned();
        Self {
            install_dir: default_dir,
            portable: false,
            phase: SetupPhase::Ready,
            corners_applied: false,
            opened_at: None,
        }
    }

    fn start_install(&mut self) {
        let dir = self.install_dir.clone();
        let portable = self.portable;
        let progress = Arc::new(AtomicU32::new(10));
        let result = Arc::new(Mutex::new(None));
        let p_bg = Arc::clone(&progress);
        let r_bg = Arc::clone(&result);
        std::thread::spawn(move || {
            let res = perform_install(&dir, portable, &p_bg);
            if let Ok(mut guard) = r_bg.lock() {
                *guard = Some(res);
            }
        });
        self.phase = SetupPhase::Installing { progress, result };
    }
}

fn perform_install(
    install_dir: &str,
    portable: bool,
    progress: &AtomicU32,
) -> Result<String, String> {
    let trimmed = install_dir.trim();
    if trimmed.is_empty() {
        return Err("Please choose an install folder".to_owned());
    }
    let target = PathBuf::from(trimmed);
    fs::create_dir_all(&target).map_err(|e| format!("Failed to create folder: {e}"))?;
    progress.store(22, Ordering::Release);

    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "rewind.exe", "/IM", "rewind-daemon.exe"])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(250));
    progress.store(42, Ordering::Release);

    let ui_exe = target.join("rewind.exe");
    let daemon_exe = target.join("rewind-daemon.exe");

    fs::write(&ui_exe, PAYLOAD_REWIND_UI)
        .map_err(|e| format!("Failed to write rewind.exe: {e}"))?;
    progress.store(64, Ordering::Release);

    fs::write(&daemon_exe, PAYLOAD_REWIND_DAEMON)
        .map_err(|e| format!("Failed to write rewind-daemon.exe: {e}"))?;
    progress.store(78, Ordering::Release);

    let ico_path = target.join("rewind.ico");
    let _ = fs::write(&ico_path, rewind::app_icon::default_ico_bytes());

    let flag_path = target.join("portable.flag");
    let dir_str = target.to_string_lossy().into_owned();
    if portable {
        fs::write(&flag_path, b"portable\n")
            .map_err(|e| format!("Failed to write portable.flag: {e}"))?;
        let _ = fs::create_dir_all(target.join("data"));
        progress.store(96, Ordering::Release);
    } else {
        let _ = fs::remove_file(&flag_path);
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let lnk = PathBuf::from(profile).join("Desktop").join("Rewind.lnk");
            create_shortcut(&ui_exe, &lnk, &target, &ico_path);
        }

        if let Ok(appdata) = std::env::var("APPDATA") {
            let prog_dir = PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Rewind");
            let _ = fs::create_dir_all(&prog_dir);
            let lnk = prog_dir.join("Rewind.lnk");
            create_shortcut(&ui_exe, &lnk, &target, &ico_path);
        }
        progress.store(90, Ordering::Release);

        let uninstaller = target.join("Rewind-Uninstall.exe");
        if let Ok(me) = std::env::current_exe() {
            let _ = fs::copy(me, &uninstaller);
        }
        reg_add(UNINSTALL_KEY, "DisplayName", "Rewind");
        reg_add(UNINSTALL_KEY, "DisplayVersion", env!("CARGO_PKG_VERSION"));
        reg_add(UNINSTALL_KEY, "Publisher", "BioBuch");
        reg_add(UNINSTALL_KEY, "InstallLocation", &dir_str);
        reg_add(UNINSTALL_KEY, "DisplayIcon", &ico_path.to_string_lossy());
        reg_add(
            UNINSTALL_KEY,
            "UninstallString",
            &format!("\"{}\" --uninstall", uninstaller.to_string_lossy()),
        );
        reg_add(
            UNINSTALL_KEY,
            "QuietUninstallString",
            &format!("\"{}\" --uninstall --quiet", uninstaller.to_string_lossy()),
        );
        reg_add_dword(UNINSTALL_KEY, "NoModify", 1);
        reg_add_dword(UNINSTALL_KEY, "NoRepair", 1);
        progress.store(96, Ordering::Release);
    }

    let _ = Command::new(&daemon_exe).current_dir(&target).spawn();
    let _ = Command::new(&ui_exe).current_dir(&target).spawn();
    progress.store(100, Ordering::Release);

    Ok(dir_str)
}

fn create_shortcut(target_exe: &Path, lnk_path: &Path, work_dir: &Path, icon_path: &Path) {
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{}';$s.Description='Rewind';$s.Save()",
        lnk_path.to_string_lossy().replace('\'', "''"),
        target_exe.to_string_lossy().replace('\'', "''"),
        work_dir.to_string_lossy().replace('\'', "''"),
        icon_path.to_string_lossy().replace('\'', "''"),
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .output();
}

fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inv * inv * inv
}

impl eframe::App for InstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.corners_applied {
            self.corners_applied = apply_win11_rounded_corners_to_active_window();
        }

        let now = ctx.input(|i| i.time);
        let start = *self.opened_at.get_or_insert(now);
        let intro_t = ease_out_cubic(((now - start) / 0.42) as f32);
        if intro_t < 1.0 {
            ctx.request_repaint();
        }

        let finished = match &self.phase {
            SetupPhase::Installing { result, .. } => {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
                result.lock().ok().and_then(|mut g| g.take())
            }
            _ => None,
        };
        if let Some(res) = finished {
            match res {
                Ok(_) => self.phase = SetupPhase::Done { done_at: now },
                Err(e) => self.phase = SetupPhase::Error(e),
            }
        }

        if let SetupPhase::Done { done_at } = self.phase {
            if now - done_at >= 2.1 {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(10, 13, 20))
                    .inner_margin(Margin::same(0.0)),
            )
            .show(ctx, |ui| {
                let win_rect = ctx.screen_rect();
                ui.painter().rect_stroke(
                    win_rect,
                    Rounding::same(14.0),
                    Stroke::new(1.0_f32, BORDER_COL),
                );

                let title_rect =
                    Rect::from_min_size(win_rect.left_top(), Vec2::new(win_rect.width(), 44.0));
                let drag_resp = ui.interact(
                    Rect::from_min_size(win_rect.left_top(), Vec2::new(win_rect.width() - 48.0, 44.0)),
                    Id::new("setup_drag"),
                    Sense::click_and_drag(),
                );
                if drag_resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }

                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(title_rect.shrink2(Vec2::new(18.0, 8.0))),
                    |ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (cr, cresp) =
                                ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
                            let hover_t =
                                ctx.animate_bool_with_time(Id::new("setup_close_h"), cresp.hovered(), 0.14);
                            if hover_t > 0.01 {
                                ui.painter().circle_filled(
                                    cr.center(),
                                    13.0,
                                    Color32::from_rgba_unmultiplied(
                                        239,
                                        68,
                                        68,
                                        (hover_t * 210.0) as u8,
                                    ),
                                );
                            }
                            ui.painter().text(
                                cr.center(),
                                egui::Align2::CENTER_CENTER,
                                "✕",
                                FontId::proportional(11.5),
                                TEXT_MUTED.lerp_to_gamma(TEXT_PRIMARY, hover_t),
                            );
                            if cresp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                ctx.send_viewport_cmd(ViewportCommand::Close);
                            }
                        });
                    },
                );

                let y_shift = (1.0 - intro_t) * 14.0;
                let alpha = (intro_t * 255.0) as u8;
                let body_rect = Rect::from_min_max(
                    Pos2::new(win_rect.left() + 36.0, title_rect.bottom() + 10.0 + y_shift),
                    Pos2::new(win_rect.right() - 36.0, win_rect.bottom() - 28.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(18.0);
                        let pulse = ((now * 2.2).sin() * 0.5 + 0.5) as f32;
                        let is_installing = matches!(self.phase, SetupPhase::Installing { .. });
                        let is_done = matches!(self.phase, SetupPhase::Done { .. });

                        let badge_center = Pos2::new(ui.available_rect_before_wrap().center().x, ui.cursor().top() + 38.0);
                        let _ = ui.allocate_exact_size(Vec2::new(80.0, 76.0), Sense::hover());

                        let glow_r = if is_installing { 46.0 + pulse * 6.0 } else { 42.0 + pulse * 2.0 };
                        let glow_col = if is_done {
                            Color32::from_rgba_unmultiplied(52, 211, 153, (30.0 * intro_t) as u8)
                        } else {
                            Color32::from_rgba_unmultiplied(56, 189, 248, ((22.0 + pulse * 14.0) * intro_t) as u8)
                        };
                        ui.painter().circle_filled(badge_center, glow_r, glow_col);

                        let badge_rect = Rect::from_center_size(badge_center, Vec2::splat(64.0));
                        ui.painter().rect_filled(
                            badge_rect,
                            Rounding::same(18.0),
                            Color32::from_rgba_unmultiplied(20, 27, 40, alpha),
                        );
                        ui.painter().rect_stroke(
                            badge_rect,
                            Rounding::same(18.0),
                            Stroke::new(
                                1.2_f32,
                                if is_done {
                                    Color32::from_rgba_unmultiplied(52, 211, 153, alpha)
                                } else {
                                    Color32::from_rgba_unmultiplied(56, 189, 248, (intro_t * 160.0) as u8)
                                },
                            ),
                        );

                        if let SetupPhase::Done { done_at } = self.phase {
                            let chk_t = ease_out_cubic(((now - done_at) / 0.36) as f32);
                            let p1 = badge_center + Vec2::new(-12.0, 1.0);
                            let p2 = badge_center + Vec2::new(-3.0, 10.0);
                            let p3 = badge_center + Vec2::new(13.0, -9.0);
                            let col = Color32::from_rgb(52, 211, 153);
                            let s1 = (chk_t * 2.0).min(1.0);
                            if s1 > 0.0 {
                                ui.painter().line_segment([p1, p1.lerp(p2, s1)], Stroke::new(3.0_f32, col));
                            }
                            if chk_t > 0.5 {
                                let s2 = ((chk_t - 0.5) * 2.0).min(1.0);
                                ui.painter().line_segment([p2, p2.lerp(p3, s2)], Stroke::new(3.0_f32, col));
                            }
                        } else {
                            let ring_col = Color32::from_rgba_unmultiplied(56, 189, 248, alpha);
                            ui.painter().circle_stroke(badge_center, 15.0, Stroke::new(2.2_f32, ring_col));
                            let tri = vec![
                                badge_center + Vec2::new(-4.0, -7.0),
                                badge_center + Vec2::new(-4.0, 7.0),
                                badge_center + Vec2::new(8.0, 0.0),
                            ];
                            ui.painter().add(egui::Shape::convex_polygon(tri, ring_col, Stroke::NONE));
                        }

                        ui.add_space(22.0);
                        ui.label(
                            egui::RichText::new("Rewind")
                                .size(26.0)
                                .strong()
                                .color(Color32::from_rgba_unmultiplied(245, 247, 250, alpha)),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("A clipping software")
                                .size(13.5)
                                .color(Color32::from_rgba_unmultiplied(148, 163, 184, alpha)),
                        );
                        ui.add_space(20.0);

                        match &self.phase {
                            SetupPhase::Ready => {
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(16, 20, 30))
                                    .stroke(Stroke::new(1.0_f32, BORDER_COL))
                                    .rounding(Rounding::same(10.0))
                                    .inner_margin(Margin::symmetric(12.0, 10.0))
                                    .show(ui, |ui| {
                                        ui.set_width(430.0);
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(&mut self.install_dir)
                                                    .desired_width(332.0)
                                                    .frame(false)
                                                    .font(FontId::proportional(12.0)),
                                            );
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                if pill_button(ui, "Browse", false, Vec2::new(76.0, 28.0)) {
                                                    if let Some(chosen) = pick_install_folder_dialog() {
                                                        let p = PathBuf::from(&chosen);
                                                        self.install_dir = if p.file_name().and_then(|n| n.to_str()).map(|n| n.eq_ignore_ascii_case("Rewind")).unwrap_or(false) {
                                                            chosen
                                                        } else {
                                                            p.join("Rewind").to_string_lossy().into_owned()
                                                        };
                                                    }
                                                }
                                            });
                                        });
                                    });

                                ui.add_space(10.0);
                                let mode_t = ctx.animate_bool_with_time(
                                    Id::new("inst_mode_t"),
                                    self.portable,
                                    0.24,
                                );
                                let row_w = 142.0 + 8.0 + 108.0;
                                let row_rect = Rect::from_center_size(
                                    Pos2::new(
                                        ui.available_rect_before_wrap().center().x,
                                        ui.cursor().top() + 15.0,
                                    ),
                                    Vec2::new(row_w, 30.0),
                                );
                                let _ = ui.allocate_exact_size(Vec2::new(row_w, 30.0), Sense::hover());
                                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(row_rect), |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        if pill_button_t(
                                            ui,
                                            "Standard Install",
                                            1.0 - mode_t,
                                            1.0,
                                            Vec2::new(142.0, 30.0),
                                        ) {
                                            self.portable = false;
                                        }
                                        if pill_button_t(
                                            ui,
                                            "Portable",
                                            mode_t,
                                            1.0,
                                            Vec2::new(108.0, 30.0),
                                        ) {
                                            self.portable = true;
                                        }
                                    });
                                });
                                ui.add_space(6.0);
                                let text_fade = (mode_t * 2.0 - 1.0).abs();
                                let show_portable_copy = mode_t >= 0.5;
                                let mode_hint = if show_portable_copy {
                                    "Self-contained folder (data/ + portable.flag), no shortcuts or registry"
                                } else {
                                    "Creates Start Menu & Desktop shortcuts + Add/Remove Programs entry"
                                };
                                ui.label(
                                    egui::RichText::new(mode_hint)
                                        .size(11.0)
                                        .color(TEXT_MUTED.linear_multiply(text_fade)),
                                );

                                ui.add_space(16.0);
                                let btn_lbl = if show_portable_copy { "Extract Portable" } else { "Install" };
                                if pill_button_t(ui, btn_lbl, 1.0, text_fade, Vec2::new(186.0, 40.0)) {
                                    self.start_install();
                                }
                            }
                            SetupPhase::Installing { progress, .. } => {
                                let target_p = progress.load(Ordering::Acquire) as f32 / 100.0;
                                let anim_p = ctx.animate_value_with_time(
                                    Id::new("install_prog_anim"),
                                    target_p,
                                    0.24,
                                );
                                let (bar_rect, _) =
                                    ui.allocate_exact_size(Vec2::new(260.0, 8.0), Sense::hover());
                                ui.painter().rect_filled(
                                    bar_rect,
                                    Rounding::same(4.0),
                                    Color32::from_rgb(24, 30, 44),
                                );
                                let fill_w = (bar_rect.width() * anim_p).max(8.0);
                                let fill_rect =
                                    Rect::from_min_size(bar_rect.left_top(), Vec2::new(fill_w, 8.0));
                                ui.painter()
                                    .rect_filled(fill_rect, Rounding::same(4.0), ACCENT);
                                ui.add_space(12.0);
                                let status = if target_p < 0.45 {
                                    "Preparing..."
                                } else if target_p < 0.82 {
                                    "Installing Rewind..."
                                } else {
                                    "Starting Rewind..."
                                };
                                ui.label(
                                    egui::RichText::new(status)
                                        .size(12.5)
                                        .color(TEXT_MUTED),
                                );
                            }
                            SetupPhase::Done { .. } => {
                                ui.label(
                                    egui::RichText::new("Installed. Opening Rewind...")
                                        .size(13.0)
                                        .color(Color32::from_rgb(52, 211, 153)),
                                );
                                ui.add_space(14.0);
                                if pill_button(ui, "Done", true, Vec2::new(140.0, 36.0)) {
                                    ctx.send_viewport_cmd(ViewportCommand::Close);
                                }
                            }
                            SetupPhase::Error(err) => {
                                ui.label(
                                    egui::RichText::new(err)
                                        .size(12.0)
                                        .color(Color32::from_rgb(244, 63, 94)),
                                );
                                ui.add_space(12.0);
                                if pill_button(ui, "Try Again", true, Vec2::new(150.0, 36.0)) {
                                    self.start_install();
                                }
                            }
                        }
                    });
                });
            });
    }
}

fn pill_button(ui: &mut Ui, label: &str, primary: bool, size: Vec2) -> bool {
    pill_button_t(ui, label, if primary { 1.0 } else { 0.0 }, 1.0, size)
}

fn pill_button_t(ui: &mut Ui, label: &str, active_t: f32, text_alpha: f32, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover_t = ui.ctx().animate_bool_with_time(
        Id::new(("inst_pill_h", label)),
        resp.hovered(),
        0.14,
    );
    let inactive_fill = Color32::from_rgb(23, 29, 40).lerp_to_gamma(Color32::from_rgb(34, 42, 58), hover_t);
    let active_fill = ACCENT.lerp_to_gamma(Color32::from_rgb(98, 211, 253), hover_t);
    let fill = inactive_fill.lerp_to_gamma(active_fill, active_t);
    let fg = TEXT_PRIMARY
        .lerp_to_gamma(Color32::from_rgb(10, 14, 20), active_t)
        .linear_multiply(text_alpha);
    let expand = if resp.is_pointer_button_down_on() { -1.0 } else { hover_t * 1.0 };
    let draw_rect = rect.expand(expand);
    ui.painter()
        .rect_filled(draw_rect, Rounding::same(size.y * 0.5), fill);
    if active_t < 0.99 {
        ui.painter().rect_stroke(
            draw_rect,
            Rounding::same(size.y * 0.5),
            Stroke::new(1.0_f32, BORDER_COL.linear_multiply(1.0 - active_t)),
        );
    }
    ui.painter().text(
        draw_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        fg,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(data) = fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts
            .font_data
            .insert("segoe_ui".to_owned(), egui::FontData::from_owned(data));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe_ui".to_owned());
    }
    if let Ok(sym) = fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
        fonts
            .font_data
            .insert("segoe_sym".to_owned(), egui::FontData::from_owned(sym));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("segoe_sym".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn apply_visuals(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(11, 14, 20);
    visuals.window_fill = CARD_BG;
    visuals.extreme_bg_color = Color32::from_rgb(15, 19, 27);
    ctx.set_visuals(visuals);
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Rewind";

fn reg_add(key: &str, name: &str, value: &str) {
    let _ = Command::new("reg")
        .args(["add", key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
        .output();
}

fn reg_add_dword(key: &str, name: &str, value: u32) {
    let _ = Command::new("reg")
        .args(["add", key, "/v", name, "/t", "REG_DWORD", "/d", &value.to_string(), "/f"])
        .output();
}

fn run_uninstall() {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "rewind.exe", "/IM", "rewind-daemon.exe"])
        .output();
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let _ = fs::remove_file(PathBuf::from(profile).join("Desktop").join("Rewind.lnk"));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let _ = fs::remove_dir_all(
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Rewind"),
        );
    }
    let _ = Command::new("reg")
        .args(["delete", RUN_KEY, "/v", "RewindCaptureDaemon", "/f"])
        .output();
    let _ = Command::new("reg")
        .args(["delete", UNINSTALL_KEY, "/f"])
        .output();
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        let cmd = format!(
            "ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"",
            dir.to_string_lossy()
        );
        let _ = Command::new("cmd")
            .args(["/C", &cmd])
            .creation_flags(0x0800_0000)
            .spawn();
    }
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--uninstall") {
        run_uninstall();
        return Ok(());
    }
    let icon_rgba =
        rewind::app_icon::render_icon_rgba(rewind::config::AccentColor::Blue.color(), 256);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rewind")
            .with_inner_size([540.0, 480.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: icon_rgba,
                width: 256,
                height: 256,
            })),
        ..Default::default()
    };
    eframe::run_native(
        "Rewind",
        options,
        Box::new(|cc| Ok(Box::new(InstallerApp::new(cc)))),
    )
}

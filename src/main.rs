#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art;
mod state;
mod ui;

use egui::{Color32, Rounding, Stroke, Visuals};
use state::RewindUiState;

struct RewindApp {
    state: RewindUiState,
    corners_applied: bool,
}

impl RewindApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        apply_dark_fluent_visuals(&cc.egui_ctx);
        Self {
            state: RewindUiState::connect_and_init(),
            corners_applied: false,
        }
    }
}

impl eframe::App for RewindApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.corners_applied {
            self.corners_applied = rewind::win_ffi::apply_win11_rounded_corners_to_active_window();
        }
        ui::render_app(ctx, &mut self.state);
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    #[cfg(target_os = "windows")]
    {
        if let Ok(data) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
            fonts
                .font_data
                .insert("segoe_ui".to_owned(), egui::FontData::from_owned(data));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "segoe_ui".to_owned());
        }
        if let Ok(sym) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
            fonts
                .font_data
                .insert("segoe_sym".to_owned(), egui::FontData::from_owned(sym));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("segoe_sym".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

fn apply_dark_fluent_visuals(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(11, 14, 20);
    visuals.window_fill = Color32::from_rgb(19, 24, 33);
    visuals.extreme_bg_color = Color32::from_rgb(15, 19, 27);
    visuals.window_rounding = Rounding::same(10.0);
    visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(34, 41, 56));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(19, 24, 33);
    visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgb(34, 41, 56));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(23, 29, 40);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(38, 46, 62));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 40, 56);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(64, 78, 104));
    ctx.set_visuals(visuals);
}

fn main() -> eframe::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        let log_path = rewind::config::AppConfig::app_data_dir().join("ui_panic.log");
        let _ = std::fs::write(&log_path, format!("{info}\n"));
    }));
    let cfg = rewind::config::AppConfig::load_from_dir(&rewind::config::AppConfig::app_data_dir());
    let init_accent = cfg.effective_accent_color();
    let icon_rgba = rewind::app_icon::render_icon_rgba(init_accent, 256);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rewind")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_transparent(false)
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
        Box::new(|cc| Ok(Box::new(RewindApp::new(cc)))),
    )
}

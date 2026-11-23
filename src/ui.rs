use crate::art::{
    paint_empty_thumb_surface, paint_fluent_background, paint_game_badge_initial, paint_icon,
    paint_rounded_image, UiIcon,
};
use crate::state::{
    ActivePopup, FilterPill, NavTab, RebindTarget, RewindUiState, SortOrder, UpdateStatus,
    ViewMode,
};
use egui::{
    Align, Color32, CursorIcon, FontId, Id, Key, Layout, Margin, Order, Pos2, Rect, Response,
    RichText, Rounding, Sense, Shadow, Shape, Stroke, TextEdit, TextureOptions, Ui, Vec2,
    ViewportCommand,
};
use rewind::config::{
    AccentColor, AppConfig, BufferMode, ClipLength, ContainerFormat, EncoderPreference, FrameRate,
    RateControl, Resolution, VideoCodec,
};
use rewind::game_detector::normalize_game_entry;
use rewind::game_icons::STEAMGRIDDB_API_KEY_URL;
use rewind::ipc::IpcRequest;
use rewind::library::ClipRecord;
use rewind::updater::{open_url, CURRENT_VERSION, RELEASES_PAGE_URL};
use rewind::win_ffi::{
    pick_exe_file_dialog, pick_image_file_dialog, set_active_window_borderless_fullscreen,
};
use std::f32::consts::{FRAC_PI_2, TAU};

#[derive(Debug, Clone, Copy)]
struct ThemeAccent(Color32);

impl ThemeAccent {
    fn from_config(cfg: &AppConfig) -> Self {
        Self(cfg.effective_accent_color())
    }

    fn color(self) -> Color32 {
        self.0
    }

    fn soft_bg(self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.0.r(), self.0.g(), self.0.b(), 38)
    }
}

#[derive(Debug, Clone, Copy)]
struct UiPalette {
    bar_bg: Color32,
    sidebar_bg: Color32,
    card_bg: Color32,
    card_hover: Color32,
    pill_bg: Color32,
    popover_bg: Color32,
    border: Color32,
    border_bright: Color32,
    icon_box_bg: Color32,
    divider_col: Color32,
    track_bg: Color32,
    nav_hover_bg: Color32,
}

impl UiPalette {
    const fn default_dark() -> Self {
        Self {
            bar_bg: Color32::from_rgb(13, 17, 24),
            sidebar_bg: Color32::from_rgb(12, 15, 22),
            card_bg: Color32::from_rgb(19, 24, 33),
            card_hover: Color32::from_rgb(25, 31, 43),
            pill_bg: Color32::from_rgb(23, 29, 40),
            popover_bg: Color32::from_rgb(22, 27, 37),
            border: Color32::from_rgb(34, 41, 56),
            border_bright: Color32::from_rgb(54, 65, 86),
            icon_box_bg: Color32::from_rgb(26, 33, 47),
            divider_col: Color32::from_rgb(28, 34, 46),
            track_bg: Color32::from_rgb(38, 45, 59),
            nav_hover_bg: Color32::from_rgb(24, 30, 42),
        }
    }

    fn from_ambient(ambient: Color32) -> Self {
        let ar = ambient.r();
        let ag = ambient.g();
        let ab = ambient.b();
        Self {
            bar_bg: Color32::from_rgba_unmultiplied(ar / 2 + 6, ag / 2 + 6, ab / 2 + 7, 175),
            sidebar_bg: Color32::from_rgba_unmultiplied(ar / 2 + 5, ag / 2 + 5, ab / 2 + 6, 175),
            card_bg: Color32::from_rgba_unmultiplied(
                ar.saturating_add(8),
                ag.saturating_add(8),
                ab.saturating_add(9),
                148,
            ),
            card_hover: Color32::from_rgba_unmultiplied(
                ar.saturating_add(18),
                ag.saturating_add(18),
                ab.saturating_add(20),
                178,
            ),
            pill_bg: Color32::from_rgba_unmultiplied(
                ar.saturating_add(14),
                ag.saturating_add(14),
                ab.saturating_add(16),
                165,
            ),
            popover_bg: Color32::from_rgba_unmultiplied(
                ar.saturating_add(12),
                ag.saturating_add(12),
                ab.saturating_add(14),
                238,
            ),
            border: Color32::from_rgba_unmultiplied(255, 255, 255, 24),
            border_bright: Color32::from_rgba_unmultiplied(255, 255, 255, 52),
            icon_box_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 16),
            divider_col: Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            track_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 34),
            nav_hover_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 22),
        }
    }
}

thread_local! {
    static UI_PAL: std::cell::Cell<UiPalette> = const { std::cell::Cell::new(UiPalette::default_dark()) };
}

fn ui_pal() -> UiPalette {
    UI_PAL.with(|p| p.get())
}
fn bar_bg() -> Color32 { ui_pal().bar_bg }
fn sidebar_bg() -> Color32 { ui_pal().sidebar_bg }
fn card_bg() -> Color32 { ui_pal().card_bg }
fn card_hover() -> Color32 { ui_pal().card_hover }
fn pill_bg() -> Color32 { ui_pal().pill_bg }
fn popover_bg() -> Color32 { ui_pal().popover_bg }
fn border_col() -> Color32 { ui_pal().border }
fn border_bright() -> Color32 { ui_pal().border_bright }
fn icon_box_bg() -> Color32 { ui_pal().icon_box_bg }
fn divider_col() -> Color32 { ui_pal().divider_col }
fn track_bg() -> Color32 { ui_pal().track_bg }
fn nav_hover_bg() -> Color32 { ui_pal().nav_hover_bg }

fn contrasting_fg(bg: Color32) -> Color32 {
    let lum = (bg.r() as u32 * 299 + bg.g() as u32 * 587 + bg.b() as u32 * 114) / 1000;
    if lum > 175 {
        Color32::from_rgb(16, 18, 22)
    } else {
        Color32::WHITE
    }
}

const TEXT_PRIMARY: Color32 = Color32::from_rgb(238, 242, 248);
const TEXT_MUTED: Color32 = Color32::from_rgb(134, 145, 165);
const TEXT_DIM: Color32 = Color32::from_rgb(96, 107, 126);
const STAR_GOLD: Color32 = Color32::from_rgb(250, 204, 21);
const REPLAY_GREEN: Color32 = Color32::from_rgb(52, 211, 153);

struct PopoverLayout {
    anchor: Option<Rect>,
    toggled_this_frame: bool,
}

pub fn render_app(ctx: &egui::Context, state: &mut RewindUiState) {
    let now_secs = ctx.input(|i| i.time);
    state.poll_background_ipc(ctx, now_secs);
    handle_ui_key_rebind(ctx, state, now_secs);
    if state.restarting_from_pid.is_some() {
        ctx.request_repaint();
    }

    if state.should_auto_close_now {
        state.allow_close_now = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
        return;
    }
    if ctx.input(|i| i.viewport().close_requested())
        && !state.allow_close_now
        && state.daemon_connected
        && state.snapshot.replay_on
    {
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        state.show_close_recording_notice = true;
    }
    if state.selected_clip_id.is_none() && state.viewport_fullscreen_applied {
        set_active_window_borderless_fullscreen(false);
        state.viewport_fullscreen_applied = false;
    }

    if let Some(new_rgb) = state.ensure_background_loaded(ctx) {
        if state.snapshot.config.custom_accent_rgb != Some(new_rgb) {
            state.snapshot.config.custom_accent_rgb = Some(new_rgb);
            if state.snapshot.config.auto_adjust_palette {
                state.push_config_update(now_secs);
            }
        }
    }

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    state.ensure_app_icon_synced(ctx, accent.color());
    let pal = if state.bg_texture.is_some() {
        let ambient = if state.snapshot.config.auto_adjust_palette {
            state
                .bg_ambient_tint
                .unwrap_or(Color32::from_rgb(16, 16, 18))
        } else {
            let ac = accent.color();
            Color32::from_rgb(
                (ac.r() / 8).max(10),
                (ac.g() / 8).max(10),
                (ac.b() / 8).max(10),
            )
        };
        UiPalette::from_ambient(ambient)
    } else {
        UiPalette::default_dark()
    };
    UI_PAL.with(|p| p.set(pal));

    let flash_alpha = compute_clip_flash_alpha(ctx, state, now_secs);
    apply_apple_rounding(ctx);
    paint_fluent_background(
        ctx,
        accent.color(),
        state.bg_texture.as_ref(),
        state.bg_ambient_tint,
        state.snapshot.config.background_brightness,
    );

    let mut pop = PopoverLayout {
        anchor: None,
        toggled_this_frame: false,
    };

    render_top_bar(ctx, state, &mut pop, now_secs, flash_alpha);
    render_sidebar(ctx, state);

    egui::CentralPanel::default()
        .frame(
            egui::Frame::none()
                .fill(Color32::TRANSPARENT)
                .inner_margin(Margin::symmetric(24.0, 18.0)),
        )
        .show(ctx, |ui| match state.active_tab {
            NavTab::Home => render_home(ui, state, &mut pop, now_secs),
            NavTab::AllClips | NavTab::Favorites => render_library(ui, state, &mut pop, now_secs),
            NavTab::Folders => render_folders(ui, state),
            NavTab::Settings => render_settings(ui, state, &mut pop, now_secs),
        });

    let popover_rect = render_active_popover(ctx, state, pop.anchor, now_secs);

    if state.selected_clip_id.is_some() {
        render_clip_modal(ctx, state, now_secs);
    }

    render_clip_context_menu(ctx, state, now_secs);
    render_rename_clip_modal(ctx, state, now_secs);
    render_close_notice_modal(ctx, state);
    render_undo_or_status_toast(ctx, state, now_secs);
    render_mandatory_update_modal(ctx, state);
    render_onboarding_and_tour(ctx, state, now_secs);

    if !pop.toggled_this_frame && state.active_popup != ActivePopup::None {
        let clicked_outside = ctx.input(|i| {
            if !i.pointer.any_pressed() {
                return false;
            }
            if let (Some(pos), Some(r)) = (i.pointer.interact_pos(), popover_rect) {
                !r.contains(pos)
            } else {
                true
            }
        });
        if clicked_outside {
            state.active_popup = ActivePopup::None;
        }
    }
}

fn apply_apple_rounding(ctx: &egui::Context) {
    if ctx.style().visuals.window_rounding == Rounding::same(16.0) {
        return;
    }
    ctx.style_mut(|s| {
        s.visuals.window_rounding = Rounding::same(16.0);
        s.visuals.menu_rounding = Rounding::same(14.0);
        s.visuals.widgets.noninteractive.rounding = Rounding::same(12.0);
        s.visuals.widgets.inactive.rounding = Rounding::same(10.0);
        s.visuals.widgets.hovered.rounding = Rounding::same(10.0);
        s.visuals.widgets.active.rounding = Rounding::same(10.0);
        s.visuals.widgets.open.rounding = Rounding::same(10.0);
    });
}

fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inv * inv * inv
}

fn anim_open_progress(ctx: &egui::Context, id: Id, key: u64, now_secs: f64, duration: f32) -> f32 {
    let (prev_key, started_at) = ctx.data_mut(|d| {
        let entry = d.get_temp_mut_or_insert_with(id, || (key, now_secs));
        if entry.0 != key {
            *entry = (key, now_secs);
        }
        *entry
    });
    let _ = prev_key;
    let raw = ((now_secs - started_at) as f32 / duration).clamp(0.0, 1.0);
    if raw < 1.0 {
        ctx.request_repaint();
    }
    ease_out_cubic(raw)
}

#[derive(Clone, Copy)]
struct BounceScrollState {
    target_y: f32,
    current_y: f32,
    overscroll: f32,
    last_max_y: f32,
    last_time: f64,
}

fn smooth_bounce_scroll(ui: &mut Ui, id_source: &str, add_contents: impl FnOnce(&mut Ui)) {
    let id = Id::new(("smooth_bounce_scroll", id_source));
    let now = ui.input(|i| i.time);
    let avail = ui.available_rect_before_wrap();
    let hovered = ui.rect_contains_pointer(avail);

    let mut s = ui.ctx().data_mut(|d| {
        d.get_temp::<BounceScrollState>(id).unwrap_or(BounceScrollState {
            target_y: 0.0,
            current_y: 0.0,
            overscroll: 0.0,
            last_max_y: 0.0,
            last_time: now,
        })
    });

    let dt = ((now - s.last_time) as f32).clamp(0.001, 0.05);
    s.last_time = now;

    let wheel_y = if hovered {
        ui.input(|i| i.smooth_scroll_delta.y)
    } else {
        0.0
    };
    if wheel_y.abs() > 0.01 {
        ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
        s.target_y -= wheel_y * 1.15;
        if s.target_y < 0.0 {
            let excess = -s.target_y;
            s.target_y = 0.0;
            s.overscroll = (s.overscroll + excess * 0.36).min(56.0);
        } else if s.target_y > s.last_max_y {
            let excess = s.target_y - s.last_max_y;
            s.target_y = s.last_max_y;
            s.overscroll = (s.overscroll - excess * 0.36).max(-56.0);
        }
    }

    let k = 1.0 - (-16.0 * dt).exp();
    s.current_y += (s.target_y - s.current_y) * k;
    s.overscroll *= (-11.0 * dt).exp();
    if s.overscroll.abs() < 0.05 {
        s.overscroll = 0.0;
    }

    let scroll_offset = s.current_y.clamp(0.0, s.last_max_y) + (-s.overscroll).max(0.0);
    let out = egui::ScrollArea::vertical()
        .id_salt(id_source)
        .auto_shrink([false, false])
        .vertical_scroll_offset(scroll_offset)
        .show(ui, |ui| {
            if s.overscroll > 0.2 {
                ui.add_space(s.overscroll);
            }
            add_contents(ui);
            if s.overscroll < -0.2 {
                ui.add_space(-s.overscroll);
            }
        });

    let max_y = (out.content_size.y - out.inner_rect.height() - s.overscroll.abs()).max(0.0);
    s.last_max_y = max_y;
    s.target_y = s.target_y.clamp(0.0, max_y);

    if wheel_y.abs() <= 0.01 && (out.state.offset.y - scroll_offset).abs() > 1.5 {
        s.target_y = out.state.offset.y.clamp(0.0, max_y);
        s.current_y = s.target_y;
        s.overscroll = 0.0;
    }

    if (s.target_y - s.current_y).abs() > 0.25 || s.overscroll.abs() > 0.25 {
        ui.ctx().request_repaint();
    }

    ui.ctx().data_mut(|d| d.insert_temp(id, s));
}

fn handle_ui_key_rebind(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) {
    if state.rebind_target == RebindTarget::None {
        return;
    }
    let mut captured: Option<String> = None;
    let mut cancel = false;
    ctx.input(|i| {
        if i.key_pressed(Key::Escape) {
            cancel = true;
            return;
        }
        for ev in &i.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                if let Some(kname) = key_display_name(*key) {
                    let mut parts = Vec::new();
                    if modifiers.ctrl {
                        parts.push("Ctrl");
                    }
                    if modifiers.alt {
                        parts.push("Alt");
                    }
                    if modifiers.shift {
                        parts.push("Shift");
                    }
                    parts.push(kname);
                    captured = Some(parts.join("+"));
                    break;
                }
            }
        }
    });
    if cancel {
        state.rebind_target = RebindTarget::None;
    } else if let Some(combo) = captured {
        state.apply_rebind(combo, now_secs);
    }
}

fn key_display_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::Space => "Space",
        _ => return None,
    })
}

fn render_top_bar(
    ctx: &egui::Context,
    state: &mut RewindUiState,
    pop: &mut PopoverLayout,
    now_secs: f64,
    flash_alpha: f32,
) {
    let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let bar_border = lerp_color(
        border_col(),
        accent.color(),
        flash_alpha * 0.55,
    );

    egui::TopBottomPanel::top("top_command_bar")
        .exact_height(46.0)
        .frame(
            egui::Frame::none()
                .fill(bar_bg())
                .stroke(Stroke::new(1.0_f32, bar_border))
                .inner_margin(Margin::symmetric(12.0, 6.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                let (logo_rect, logo_resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                if let Some(tex) = &state.app_icon_texture {
                    ui.painter().image(
                        tex.id(),
                        logo_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                if logo_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    state.navigate(NavTab::Home);
                }

                let back_resp = icon_button(
                    ui,
                    Vec2::splat(24.0),
                    UiIcon::ChevronLeft,
                    if state.can_go_back() { TEXT_PRIMARY } else { TEXT_DIM },
                    Color32::TRANSPARENT,
                );
                if state.can_go_back() && back_resp.clicked() {
                    state.go_back();
                }

                let fwd_resp = icon_button(
                    ui,
                    Vec2::splat(24.0),
                    UiIcon::ChevronRight,
                    if state.can_go_forward() { TEXT_PRIMARY } else { TEXT_DIM },
                    Color32::TRANSPARENT,
                );
                if state.can_go_forward() && fwd_resp.clicked() {
                    state.go_forward();
                }

                let (source_text, dot_color) = if state.restarting_from_pid.is_some() {
                    (
                        "Restarting Daemon...".to_owned(),
                        Color32::from_rgb(96, 107, 126),
                    )
                } else if let Some(game) = &state.snapshot.active_game {
                    let st = if state.snapshot.replay_on { "Ready" } else { "Paused" };
                    let col = if state.snapshot.replay_on {
                        REPLAY_GREEN
                    } else {
                        Color32::from_rgb(138, 148, 166)
                    };
                    (format!("{game}  •  {st}"), col)
                } else if let Some(manual) = &state.snapshot.manual_source {
                    let st = if state.snapshot.replay_on { "Ready" } else { "Paused" };
                    let col = if state.snapshot.replay_on {
                        REPLAY_GREEN
                    } else {
                        Color32::from_rgb(138, 148, 166)
                    };
                    (format!("{manual}  •  {st}"), col)
                } else {
                    (
                        "Start Your Game To Record".to_owned(),
                        Color32::from_rgb(96, 107, 126),
                    )
                };

                let src_resp = dropdown_pill(
                    ui,
                    &source_text,
                    Some(dot_color),
                    None,
                    state.active_popup == ActivePopup::CaptureSource,
                    166.0,
                );
                if src_resp.clicked() {
                    state.toggle_popup(ActivePopup::CaptureSource);
                    pop.toggled_this_frame = true;
                }
                if state.active_popup == ActivePopup::CaptureSource {
                    pop.anchor = Some(src_resp.rect);
                }

                let clip_label = state.snapshot.config.clip_length.short_label();
                let hotkey_str = state.snapshot.config.hotkey_save_clip.clone();
                let (save_clicked, len_clicked, split_rect) = split_clip_pill(
                    ui,
                    clip_label,
                    &hotkey_str,
                    state.active_popup == ActivePopup::ClipLength,
                    accent.color(),
                    flash_alpha,
                );
                if save_clicked {
                    state.send_sync(IpcRequest::SaveClipNow, now_secs);
                }
                if len_clicked {
                    state.toggle_popup(ActivePopup::ClipLength);
                    pop.toggled_this_frame = true;
                }
                if state.active_popup == ActivePopup::ClipLength {
                    pop.anchor = Some(split_rect);
                }

                let qual_label = state.snapshot.config.top_bar_quality_label();
                let qual_resp = dropdown_pill(
                    ui,
                    &qual_label,
                    None,
                    None,
                    state.active_popup == ActivePopup::VideoQuality,
                    104.0,
                );
                if qual_resp.clicked() {
                    state.toggle_popup(ActivePopup::VideoQuality);
                    pop.toggled_this_frame = true;
                }
                if state.active_popup == ActivePopup::VideoQuality {
                    pop.anchor = Some(qual_resp.rect);
                }

                let right_cluster_w = if state.search_open { 448.0 } else { 308.0 };
                let drag_w = (ui.available_width() - right_cluster_w).max(20.0);
                let drag_resp = ui.allocate_response(Vec2::new(drag_w, 32.0), Sense::click_and_drag());
                if drag_resp.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if drag_resp.double_clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    let close_resp = window_ctrl_button(ui, UiIcon::Close, true);
                    if close_resp.clicked() {
                        if state.daemon_connected && state.snapshot.replay_on {
                            state.show_close_recording_notice = true;
                        } else {
                            state.allow_close_now = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    }
                    let max_resp = window_ctrl_button(ui, UiIcon::Maximize, false);
                    if max_resp.clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized));
                    }
                    let min_resp = window_ctrl_button(ui, UiIcon::Minimize, false);
                    if min_resp.clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }

                    ui.add_space(2.0);

                    let initials = user_initials();
                    let av_resp = avatar_button(
                        ui,
                        &initials,
                        state.active_popup == ActivePopup::Profile,
                        accent,
                    );
                    if av_resp.clicked() {
                        state.toggle_popup(ActivePopup::Profile);
                        pop.toggled_this_frame = true;
                    }
                    if state.active_popup == ActivePopup::Profile {
                        pop.anchor = Some(av_resp.rect);
                    }

                    let bell_active = state.active_popup == ActivePopup::Notifications;
                    let (bell_rect, bell_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                    let bell_bg = if bell_active || bell_resp.hovered() {
                        pill_bg()
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(bell_rect, Rounding::same(6.0), bell_bg);
                    if bell_active {
                        ui.painter().rect_stroke(
                            bell_rect,
                            Rounding::same(6.0),
                            Stroke::new(1.2_f32, Color32::from_rgb(200, 210, 228)),
                        );
                    }
                    paint_icon(
                        ui.painter(),
                        bell_rect.shrink(6.0),
                        UiIcon::Bell,
                        if bell_active { TEXT_PRIMARY } else { TEXT_MUTED },
                    );
                    if bell_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        state.toggle_popup(ActivePopup::Notifications);
                        pop.toggled_this_frame = true;
                    }
                    if bell_active {
                        pop.anchor = Some(bell_rect);
                    }

                    let replay_on = state.restarting_from_pid.is_none() && state.snapshot.replay_on;
                    let replay_text = if replay_on { "Replay On" } else { "Replay Off" };
                    let (r_rect, r_resp) = ui.allocate_exact_size(Vec2::new(96.0, 28.0), Sense::click());
                    let (r_bg, r_border, r_dot, r_fg) = if replay_on {
                        (
                            Color32::from_rgba_unmultiplied(16, 185, 129, 30),
                            Color32::from_rgb(30, 96, 72),
                            REPLAY_GREEN,
                            Color32::from_rgb(167, 243, 208),
                        )
                    } else {
                        (
                            if r_resp.hovered() { card_hover() } else { pill_bg() },
                            border_col(),
                            Color32::from_rgb(112, 122, 140),
                            TEXT_MUTED,
                        )
                    };
                    ui.painter().rect_filled(r_rect, Rounding::same(7.0), r_bg);
                    ui.painter()
                        .rect_stroke(r_rect, Rounding::same(7.0), Stroke::new(1.0_f32, r_border));
                    ui.painter()
                        .circle_filled(Pos2::new(r_rect.left() + 13.0, r_rect.center().y), 3.3, r_dot);
                    ui.painter().text(
                        Pos2::new(r_rect.left() + 23.0, r_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        replay_text,
                        FontId::proportional(12.0),
                        r_fg,
                    );
                    if r_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        state.send_sync(IpcRequest::SetReplay(!replay_on), now_secs);
                    }

                    if state.search_open {
                        let box_w = 168.0;
                        let (s_rect, _) = ui.allocate_exact_size(Vec2::new(box_w, 28.0), Sense::hover());
                        ui.painter()
                            .rect_filled(s_rect, Rounding::same(7.0), Color32::from_rgb(18, 23, 33));
                        ui.painter().rect_stroke(
                            s_rect,
                            Rounding::same(7.0),
                            Stroke::new(1.0_f32, accent.color()),
                        );
                        paint_icon(
                            ui.painter(),
                            Rect::from_center_size(
                                Pos2::new(s_rect.left() + 14.0, s_rect.center().y),
                                Vec2::splat(13.0),
                            ),
                            UiIcon::Search,
                            accent.color(),
                        );
                        let edit_rect = Rect::from_min_max(
                            Pos2::new(s_rect.left() + 26.0, s_rect.top() + 3.0),
                            Pos2::new(s_rect.right() - 24.0, s_rect.bottom() - 3.0),
                        );
                        ui.put(
                            edit_rect,
                            TextEdit::singleline(&mut state.search_query)
                                .frame(false)
                                .hint_text("Search clips or games...")
                                .text_color(TEXT_PRIMARY)
                                .font(FontId::proportional(12.0)),
                        );
                        let close_r = Rect::from_center_size(
                            Pos2::new(s_rect.right() - 13.0, s_rect.center().y),
                            Vec2::splat(18.0),
                        );
                        let close_resp = ui.interact(close_r, Id::new("search_clear"), Sense::click());
                        paint_icon(
                            ui.painter(),
                            close_r.shrink(4.0),
                            UiIcon::Close,
                            if close_resp.hovered() { TEXT_PRIMARY } else { TEXT_MUTED },
                        );
                        if close_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            state.search_query.clear();
                            state.search_open = false;
                        }
                    } else {
                        let (s_rect, s_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                        if s_resp.hovered() {
                            ui.painter().rect_filled(s_rect, Rounding::same(6.0), pill_bg());
                        }
                        paint_icon(
                            ui.painter(),
                            s_rect.shrink(7.0),
                            UiIcon::Search,
                            if s_resp.hovered() { TEXT_PRIMARY } else { TEXT_MUTED },
                        );
                        if s_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            state.search_open = true;
                        }
                    }
                });
            });
        });
}

fn render_sidebar(ctx: &egui::Context, state: &mut RewindUiState) {
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let total_clips = state.snapshot.clips.len();

    egui::SidePanel::left("nav_sidebar")
        .exact_width(54.0)
        .resizable(false)
        .frame(
            egui::Frame::none()
                .fill(sidebar_bg())
                .stroke(Stroke::new(1.0_f32, border_col()))
                .inner_margin(Margin::symmetric(6.0, 12.0)),
        )
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                let items = [
                    (NavTab::Home, UiIcon::Home, None, "Home"),
                    (
                        NavTab::AllClips,
                        UiIcon::Library,
                        if total_clips > 0 { Some(total_clips) } else { None },
                        "All Clips",
                    ),
                    (NavTab::Favorites, UiIcon::StarOutline, None, "Favorites"),
                    (NavTab::Folders, UiIcon::Folder, None, "Folders"),
                ];

                for (tab, icon, badge, tip) in items {
                    let active = state.active_tab == tab;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
                    if active {
                        ui.painter().rect_filled(rect, Rounding::same(9.0), accent.soft_bg());
                        ui.painter().rect_stroke(
                            rect,
                            Rounding::same(9.0),
                            Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(
                                    accent.color().r(),
                                    accent.color().g(),
                                    accent.color().b(),
                                    80,
                                ),
                            ),
                        );
                        let bar = Rect::from_min_max(
                            Pos2::new(rect.left() - 6.0, rect.top() + 9.0),
                            Pos2::new(rect.left() - 3.0, rect.bottom() - 9.0),
                        );
                        ui.painter().rect_filled(bar, Rounding::same(1.5), accent.color());
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, Rounding::same(9.0), nav_hover_bg());
                    }

                    let icon_col = if active { accent.color() } else { TEXT_MUTED };
                    paint_icon(ui.painter(), rect.shrink(10.0), icon, icon_col);

                    if let Some(count) = badge {
                        let b_pos = Pos2::new(rect.right() - 5.0, rect.top() + 6.0);
                        ui.painter().circle_filled(b_pos, 7.5, accent.color());
                        ui.painter().text(
                            b_pos,
                            egui::Align2::CENTER_CENTER,
                            count.to_string(),
                            FontId::proportional(9.5),
                            contrasting_fg(accent.color()),
                        );
                    }

                    if resp
                        .on_hover_text(tip)
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .clicked()
                    {
                        state.navigate(tab);
                    }
                }

                let avail = ui.available_height();
                if avail > 112.0 {
                    ui.add_space(avail - 112.0);
                }

                let (ring_rect, ring_resp) = ui.allocate_exact_size(Vec2::new(42.0, 56.0), Sense::click());
                let ring_center = Pos2::new(ring_rect.center().x, ring_rect.top() + 15.0);
                let ring_r = 11.5;
                ui.painter()
                    .circle_stroke(ring_center, ring_r, Stroke::new(2.6_f32, track_bg()));

                let used_gb = state.used_storage_gb();
                let max_gb = state.snapshot.config.max_storage_gb.max(1) as f32;
                let ratio = (used_gb / max_gb).clamp(0.0, 1.0);
                if ratio > 0.005 {
                    let segs = 28;
                    let max_i = ((segs as f32) * ratio).ceil() as usize;
                    let mut arc_pts = Vec::with_capacity(max_i + 1);
                    for i in 0..=max_i {
                        let frac = (i as f32 / segs as f32).min(ratio);
                        let ang = -FRAC_PI_2 + frac * TAU;
                        arc_pts.push(Pos2::new(
                            ring_center.x + ang.cos() * ring_r,
                            ring_center.y + ang.sin() * ring_r,
                        ));
                    }
                    if arc_pts.len() >= 2 {
                        ui.painter()
                            .add(Shape::line(arc_pts, Stroke::new(2.8_f32, accent.color())));
                    }
                }

                let used_lbl = if used_gb < 1.0 {
                    format!("{:.1} GB", used_gb)
                } else {
                    format!("{:.0} GB", used_gb)
                };
                let max_lbl = format!("{} GB", state.snapshot.config.max_storage_gb);
                ui.painter().text(
                    Pos2::new(ring_rect.center().x, ring_rect.top() + 36.0),
                    egui::Align2::CENTER_CENTER,
                    used_lbl,
                    FontId::proportional(9.5),
                    TEXT_PRIMARY,
                );
                ui.painter().text(
                    Pos2::new(ring_rect.center().x, ring_rect.top() + 47.5),
                    egui::Align2::CENTER_CENTER,
                    max_lbl,
                    FontId::proportional(8.5),
                    TEXT_DIM,
                );
                if ring_resp
                    .on_hover_text("Open Storage Settings")
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    state.navigate(NavTab::Settings);
                }

                ui.add_space(4.0);

                let is_set = state.active_tab == NavTab::Settings;
                let (set_rect, set_resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
                if is_set {
                    ui.painter()
                        .rect_filled(set_rect, Rounding::same(9.0), accent.soft_bg());
                    ui.painter().rect_stroke(
                        set_rect,
                        Rounding::same(9.0),
                        Stroke::new(1.0_f32, accent.color()),
                    );
                } else if set_resp.hovered() {
                    ui.painter()
                        .rect_filled(set_rect, Rounding::same(9.0), nav_hover_bg());
                }
                paint_icon(
                    ui.painter(),
                    set_rect.shrink(10.0),
                    UiIcon::Gear,
                    if is_set { accent.color() } else { TEXT_MUTED },
                );
                if set_resp
                    .on_hover_text("Settings")
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    state.navigate(NavTab::Settings);
                }
            });
        });
}

fn render_home(
    ui: &mut Ui,
    state: &mut RewindUiState,
    pop: &mut PopoverLayout,
    now_secs: f64,
) {
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let games = state.dynamic_games();

    smooth_bounce_scroll(ui, "home_scroll", |ui| {
            if state.snapshot.clips.is_empty() {
                render_empty_library_state(ui, state, now_secs);
                return;
            }

            if !games.is_empty() {
                ui.label(RichText::new("Your Games").size(16.5).strong().color(TEXT_PRIMARY));
                ui.add_space(12.0);

                let avail_w = ui.available_width();
                let gap = 14.0;
                let card_w = ((avail_w - gap * 3.0) / 4.0).max(160.0);
                let banner_h = card_w * 9.0 / 16.0;
                let card_h = banner_h + 52.0;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for game in games.iter().take(4) {
                        let (slot_rect, resp) =
                            ui.allocate_exact_size(Vec2::new(card_w, card_h), Sense::click());
                        let hovered = resp.hovered();
                        let active_game =
                            state.filter_pill == FilterPill::Game(game.name.clone());

                        let back2 = Rect::from_min_max(
                            Pos2::new(slot_rect.left() + 14.0, slot_rect.top() - 6.0),
                            Pos2::new(slot_rect.right() - 14.0, slot_rect.top() + 8.0),
                        );
                        let back1 = Rect::from_min_max(
                            Pos2::new(slot_rect.left() + 7.0, slot_rect.top() - 3.0),
                            Pos2::new(slot_rect.right() - 7.0, slot_rect.top() + 10.0),
                        );
                        ui.painter()
                            .rect_filled(back2, Rounding::same(8.0), nav_hover_bg());
                        ui.painter()
                            .rect_filled(back1, Rounding::same(9.0), card_hover());

                        let banner_rect = Rect::from_min_max(
                            slot_rect.left_top(),
                            Pos2::new(slot_rect.right(), slot_rect.top() + banner_h),
                        );
                        let thumb_tex =
                            state.get_or_load_thumbnail(ui.ctx(), &game.latest_thumb_path);
                        let top_round = Rounding {
                            nw: 10.0,
                            ne: 10.0,
                            sw: 0.0,
                            se: 0.0,
                        };
                        if let Some(tex) = thumb_tex {
                            paint_rounded_image(
                                ui.painter(),
                                tex.id(),
                                banner_rect,
                                top_round,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            paint_empty_thumb_surface(ui.painter(), banner_rect, top_round);
                        }

                        let footer_rect = Rect::from_min_max(
                            Pos2::new(slot_rect.left(), slot_rect.top() + banner_h),
                            slot_rect.right_bottom(),
                        );
                        ui.painter().rect_filled(
                            footer_rect,
                            Rounding {
                                nw: 0.0,
                                ne: 0.0,
                                sw: 10.0,
                                se: 10.0,
                            },
                            if hovered { card_hover() } else { card_bg() },
                        );

                        let border_col = if active_game {
                            accent.color()
                        } else if hovered {
                            border_bright()
                        } else {
                            border_col()
                        };
                        ui.painter().rect_stroke(
                            slot_rect,
                            Rounding::same(10.0),
                            Stroke::new(1.0_f32, border_col),
                        );

                        let badge_c = Pos2::new(slot_rect.right() - 17.0, slot_rect.top() + 16.0);
                        ui.painter().circle_filled(
                            badge_c,
                            9.5,
                            Color32::from_rgba_unmultiplied(10, 13, 18, 215),
                        );
                        ui.painter().text(
                            badge_c,
                            egui::Align2::CENTER_CENTER,
                            game.clip_count.to_string(),
                            FontId::proportional(10.5),
                            TEXT_PRIMARY,
                        );

                        let icon_rect = Rect::from_min_size(
                            Pos2::new(footer_rect.left() + 12.0, footer_rect.center().y - 11.0),
                            Vec2::splat(22.0),
                        );
                        paint_game_badge(ui, state, icon_rect, &game.name, accent.color());
                        ui.painter().text(
                            Pos2::new(icon_rect.right() + 10.0, footer_rect.top() + 16.0),
                            egui::Align2::LEFT_CENTER,
                            truncate_text(&game.name, 16),
                            FontId::proportional(13.0),
                            TEXT_PRIMARY,
                        );
                        let sub = format!(
                            "{} {}  •  {}",
                            game.clip_count,
                            if game.clip_count == 1 { "clip" } else { "clips" },
                            RewindUiState::format_relative_unix(game.latest_unix)
                        );
                        ui.painter().text(
                            Pos2::new(icon_rect.right() + 10.0, footer_rect.top() + 33.0),
                            egui::Align2::LEFT_CENTER,
                            sub,
                            FontId::proportional(11.0),
                            TEXT_MUTED,
                        );

                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            if state.filter_pill == FilterPill::Game(game.name.clone()) {
                                state.filter_pill = FilterPill::AllClips;
                            } else {
                                state.filter_pill = FilterPill::Game(game.name.clone());
                            }
                        }
                    }
                });

                ui.add_space(18.0);
            }

            render_filter_bar(ui, state, pop);
            ui.add_space(16.0);
            render_clips_collection(ui, state, now_secs);
        });
}

fn render_empty_library_state(ui: &mut Ui, state: &mut RewindUiState, now_secs: f64) {
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let hw = state.snapshot.hardware.clone();
    let hk = state.snapshot.config.hotkey_save_clip.clone();

    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        egui::Frame::none()
            .fill(card_bg())
            .stroke(Stroke::new(1.0_f32, border_col()))
            .rounding(Rounding::same(14.0))
            .inner_margin(Margin::symmetric(32.0, 28.0))
            .show(ui, |ui| {
                ui.set_max_width(480.0);
                ui.vertical_centered(|ui| {
                    let (ic_r, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    paint_icon(ui.painter(), ic_r, UiIcon::Library, accent.color());
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("No clips yet")
                            .size(18.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "Launch a game and press {hk} to capture an instant replay."
                        ))
                        .size(13.0)
                        .color(TEXT_MUTED),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "GPU: {}  •  {}",
                            hw.gpu_name, hw.encoder_backend
                        ))
                        .size(11.5)
                        .color(TEXT_DIM),
                    );
                    ui.add_space(16.0);

                    ui.horizontal(|ui| {
                        ui.add_space(92.0);
                        let (btn_r, btn_resp) =
                            ui.allocate_exact_size(Vec2::new(190.0, 34.0), Sense::click());
                        ui.painter().rect_filled(
                            btn_r,
                            Rounding::same(8.0),
                            if btn_resp.hovered() {
                                accent.color()
                            } else {
                                accent.soft_bg()
                            },
                        );
                        ui.painter().rect_stroke(
                            btn_r,
                            Rounding::same(8.0),
                            Stroke::new(1.0_f32, accent.color()),
                        );
                        ui.painter().text(
                            btn_r.center(),
                            egui::Align2::CENTER_CENTER,
                            "Capture Instant Clip Now",
                            FontId::proportional(12.5),
                            if btn_resp.hovered() {
                                Color32::WHITE
                            } else {
                                accent.color()
                            },
                        );
                        if btn_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            state.send_sync(IpcRequest::SaveClipNow, now_secs);
                        }
                    });
                });
            });
    });
}

fn render_library(
    ui: &mut Ui,
    state: &mut RewindUiState,
    pop: &mut PopoverLayout,
    now_secs: f64,
) {
    smooth_bounce_scroll(ui, "library_scroll", |ui| {
            let heading = if state.active_tab == NavTab::Favorites {
                "Favorites"
            } else {
                "All Clips"
            };
            let visible_count = state.filtered_clips().len();
            ui.label(RichText::new(heading).size(18.0).strong().color(TEXT_PRIMARY));
            ui.add_space(2.0);
            let sub = format!(
                "{} {}  •  {:.2} GB stored",
                visible_count,
                if visible_count == 1 { "clip" } else { "clips" },
                state.used_storage_gb()
            );
            ui.label(RichText::new(sub).size(12.0).color(TEXT_MUTED));
            ui.add_space(14.0);

            if state.snapshot.clips.is_empty() {
                render_empty_library_state(ui, state, now_secs);
                return;
            }

            render_filter_bar(ui, state, pop);
            ui.add_space(16.0);
            render_clips_collection(ui, state, now_secs);
        });
}

fn render_folders(ui: &mut Ui, state: &mut RewindUiState) {
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let games = state.dynamic_games();
    let root_folder = state.snapshot.config.clips_folder.clone();

    smooth_bounce_scroll(ui, "folders_scroll", |ui| {
            ui.label(RichText::new("Game Folders").size(18.0).strong().color(TEXT_PRIMARY));
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!("Storage Root: {root_folder}"))
                    .size(12.0)
                    .color(TEXT_MUTED),
            );
            ui.add_space(16.0);

            if games.is_empty() {
                ui.label(
                    RichText::new("No game folders yet — saved clips are organized automatically by game.")
                        .size(13.0)
                        .color(TEXT_MUTED),
                );
                return;
            }

            let gap = 14.0;
            let cols = 4;
            let card_w =
                ((ui.available_width() - gap * (cols - 1) as f32) / cols as f32).max(160.0);

            for chunk in games.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for game in chunk {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(card_w, 118.0), Sense::click());
                        ui.painter().rect_filled(
                            rect,
                            Rounding::same(10.0),
                            if resp.hovered() { card_hover() } else { card_bg() },
                        );
                        ui.painter().rect_stroke(
                            rect,
                            Rounding::same(10.0),
                            Stroke::new(
                                1.0_f32,
                                if resp.hovered() { border_bright() } else { border_col() },
                            ),
                        );

                        let icon_r = Rect::from_min_size(
                            Pos2::new(rect.left() + 14.0, rect.top() + 16.0),
                            Vec2::splat(26.0),
                        );
                        paint_game_badge(ui, state, icon_r, &game.name, accent.color());

                        ui.painter().text(
                            Pos2::new(icon_r.right() + 10.0, icon_r.center().y),
                            egui::Align2::LEFT_CENTER,
                            truncate_text(&game.name, 16),
                            FontId::proportional(14.5),
                            TEXT_PRIMARY,
                        );
                        ui.painter().text(
                            Pos2::new(rect.left() + 14.0, rect.top() + 62.0),
                            egui::Align2::LEFT_CENTER,
                            truncate_text(&format!("{root_folder}\\{}", game.name), 30),
                            FontId::proportional(11.0),
                            TEXT_DIM,
                        );
                        ui.painter().text(
                            Pos2::new(rect.left() + 14.0, rect.top() + 88.0),
                            egui::Align2::LEFT_CENTER,
                            format!(
                                "{} {}  •  {:.2} GB",
                                game.clip_count,
                                if game.clip_count == 1 { "clip" } else { "clips" },
                                game.total_bytes as f32 / (1024.0 * 1024.0 * 1024.0)
                            ),
                            FontId::proportional(12.0),
                            TEXT_MUTED,
                        );

                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            state.filter_pill = FilterPill::Game(game.name.clone());
                            state.navigate(NavTab::AllClips);
                        }
                    }
                });
                ui.add_space(12.0);
            }
        });
}

fn render_filter_bar(ui: &mut Ui, state: &mut RewindUiState, pop: &mut PopoverLayout) {
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let games = state.dynamic_games();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;

        let mut pills: Vec<(FilterPill, String, bool, bool)> = vec![
            (FilterPill::AllClips, "All Clips".to_owned(), false, false),
            (FilterPill::Favorites, "Favorites".to_owned(), true, false),
        ];
        for g in games.iter().take(6) {
            pills.push((FilterPill::Game(g.name.clone()), g.name.clone(), false, true));
        }

        for (pill, label, is_star, is_game) in pills {
            let active = state.filter_pill == pill;
            let text_w = label.len() as f32 * 6.4;
            let extra = if is_game || is_star { 20.0 } else { 0.0 };
            let w = (text_w + extra + 22.0).min(150.0);

            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
            let bg = if active {
                accent.soft_bg()
            } else if resp.hovered() {
                card_hover()
            } else {
                pill_bg()
            };
            let border = if active { accent.color() } else { border_col() };
            ui.painter().rect_filled(rect, Rounding::same(14.0), bg);
            ui.painter()
                .rect_stroke(rect, Rounding::same(14.0), Stroke::new(1.0_f32, border));

            let mut text_x = rect.left() + 12.0;
            if is_star {
                let s_rect = Rect::from_center_size(
                    Pos2::new(rect.left() + 16.0, rect.center().y),
                    Vec2::splat(11.0),
                );
                paint_icon(ui.painter(), s_rect, UiIcon::StarFilled, STAR_GOLD);
                text_x = rect.left() + 26.0;
            } else if is_game {
                let g_rect = Rect::from_center_size(
                    Pos2::new(rect.left() + 16.0, rect.center().y),
                    Vec2::splat(14.0),
                );
                paint_game_badge(ui, state, g_rect, &label, accent.color());
                text_x = rect.left() + 27.0;
            }

            let text_col = if active { accent.color() } else { TEXT_MUTED };
            ui.painter().text(
                Pos2::new(text_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                truncate_text(&label, 14),
                FontId::proportional(11.5),
                text_col,
            );

            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.filter_pill = pill;
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let (seg_rect, _) = ui.allocate_exact_size(Vec2::new(60.0, 28.0), Sense::hover());
            ui.painter().rect_filled(seg_rect, Rounding::same(7.0), pill_bg());
            ui.painter()
                .rect_stroke(seg_rect, Rounding::same(7.0), Stroke::new(1.0_f32, border_col()));

            let grid_r = Rect::from_min_max(
                seg_rect.left_top() + Vec2::splat(2.0),
                Pos2::new(seg_rect.center().x - 1.0, seg_rect.bottom() - 2.0),
            );
            let list_r = Rect::from_min_max(
                Pos2::new(seg_rect.center().x + 1.0, seg_rect.top() + 2.0),
                seg_rect.right_bottom() - Vec2::splat(2.0),
            );

            let grid_resp = ui.interact(grid_r, Id::new("view_grid_btn"), Sense::click());
            let list_resp = ui.interact(list_r, Id::new("view_list_btn"), Sense::click());

            if state.view_mode == ViewMode::Grid {
                ui.painter()
                    .rect_filled(grid_r, Rounding::same(5.0), Color32::from_rgb(38, 46, 62));
            }
            if state.view_mode == ViewMode::List {
                ui.painter()
                    .rect_filled(list_r, Rounding::same(5.0), Color32::from_rgb(38, 46, 62));
            }

            paint_icon(
                ui.painter(),
                grid_r.shrink(5.5),
                UiIcon::Grid,
                if state.view_mode == ViewMode::Grid {
                    TEXT_PRIMARY
                } else {
                    TEXT_DIM
                },
            );
            paint_icon(
                ui.painter(),
                list_r.shrink(5.5),
                UiIcon::List,
                if state.view_mode == ViewMode::List {
                    TEXT_PRIMARY
                } else {
                    TEXT_DIM
                },
            );

            if grid_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.view_mode = ViewMode::Grid;
            }
            if list_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.view_mode = ViewMode::List;
            }

            let sort_resp = dropdown_pill(
                ui,
                state.sort_order.label(),
                None,
                None,
                state.active_popup == ActivePopup::SortMenu,
                88.0,
            );
            if sort_resp.clicked() {
                state.toggle_popup(ActivePopup::SortMenu);
                pop.toggled_this_frame = true;
            }
            if state.active_popup == ActivePopup::SortMenu {
                pop.anchor = Some(sort_resp.rect);
            }
        });
    });
}

fn render_clips_collection(ui: &mut Ui, state: &mut RewindUiState, now_secs: f64) {
    let clips: Vec<ClipRecord> = state.filtered_clips().into_iter().cloned().collect();
    if clips.is_empty() {
        ui.add_space(36.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No clips match your current filter")
                    .size(15.0)
                    .color(TEXT_MUTED),
            );
        });
        return;
    }

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let now_unix = rewind::win_ffi::unix_now_secs();

    if state.view_mode == ViewMode::List {
        for (idx, clip) in clips.iter().enumerate() {
            let row_w = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(row_w, 56.0), Sense::click());
            if idx == 0 {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(Id::new("tour_first_clip_rect"), rect));
            }
            ui.painter().rect_filled(
                rect,
                Rounding::same(8.0),
                if resp.hovered() { card_hover() } else { card_bg() },
            );
            ui.painter().rect_stroke(
                rect,
                Rounding::same(8.0),
                Stroke::new(
                    1.0_f32,
                    if resp.hovered() { border_bright() } else { border_col() },
                ),
            );

            let thumb =
                Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.top() + 8.0), Vec2::new(72.0, 40.0));
            if let Some(tex) = state.get_or_load_thumbnail(ui.ctx(), &clip.thumb_path) {
                paint_rounded_image(
                    ui.painter(),
                    tex.id(),
                    thumb,
                    Rounding::same(5.0),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                paint_empty_thumb_surface(ui.painter(), thumb, Rounding::same(5.0));
            }

            ui.painter().text(
                Pos2::new(thumb.right() + 12.0, rect.top() + 19.0),
                egui::Align2::LEFT_CENTER,
                &clip.title,
                FontId::proportional(13.5),
                TEXT_PRIMARY,
            );
            let mb = clip.size_bytes as f32 / (1024.0 * 1024.0);
            ui.painter().text(
                Pos2::new(thumb.right() + 12.0, rect.top() + 37.0),
                egui::Align2::LEFT_CENTER,
                format!(
                    "{}  •  {}  •  {}  •  {:.1} MB",
                    clip.game,
                    clip.relative_time(now_unix),
                    clip.duration_str(),
                    mb
                ),
                FontId::proportional(11.5),
                TEXT_MUTED,
            );

            let star_r =
                Rect::from_center_size(Pos2::new(rect.right() - 26.0, rect.center().y), Vec2::splat(24.0));
            let star_resp = ui.interact(star_r, Id::new(("list_star", clip.id)), Sense::click());
            paint_icon(
                ui.painter(),
                star_r.shrink(5.0),
                if clip.favorite {
                    UiIcon::StarFilled
                } else {
                    UiIcon::StarOutline
                },
                if clip.favorite { STAR_GOLD } else { TEXT_DIM },
            );

            if star_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.send_sync(IpcRequest::ToggleFavorite { id: clip.id }, now_secs);
            } else if resp.secondary_clicked() {
                if let Some(pos) = ui.ctx().pointer_interact_pos() {
                    state.clip_context_menu = Some((clip.id, pos));
                }
            } else if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.clip_context_menu = None;
                state.open_clip_modal(clip, now_secs);
            }
            ui.add_space(8.0);
        }
        return;
    }

    let cols = 4;
    let gap = 14.0;
    let card_w = ((ui.available_width() - gap * (cols - 1) as f32) / cols as f32).max(160.0);
    let thumb_h = card_w * 9.0 / 16.0;
    let footer_h = 60.0;
    let card_h = thumb_h + footer_h;

    for (row_idx, row) in clips.chunks(cols).enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (col_idx, clip) in row.iter().enumerate() {
                let (card_rect, card_resp) =
                    ui.allocate_exact_size(Vec2::new(card_w, card_h), Sense::click());
                if row_idx == 0 && col_idx == 0 {
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(Id::new("tour_first_clip_rect"), card_rect));
                }
                let hovered = card_resp.hovered();

                let thumb_rect = Rect::from_min_max(
                    card_rect.left_top(),
                    Pos2::new(card_rect.right(), card_rect.top() + thumb_h),
                );
                let top_round = Rounding {
                    nw: 10.0,
                    ne: 10.0,
                    sw: 0.0,
                    se: 0.0,
                };
                if let Some(tex) = state.get_or_load_thumbnail(ui.ctx(), &clip.thumb_path) {
                    paint_rounded_image(
                        ui.painter(),
                        tex.id(),
                        thumb_rect,
                        top_round,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    paint_empty_thumb_surface(ui.painter(), thumb_rect, top_round);
                }

                let footer_rect = Rect::from_min_max(
                    Pos2::new(card_rect.left(), card_rect.top() + thumb_h),
                    card_rect.right_bottom(),
                );
                ui.painter().rect_filled(
                    footer_rect,
                    Rounding {
                        nw: 0.0,
                        ne: 0.0,
                        sw: 10.0,
                        se: 10.0,
                    },
                    if hovered { card_hover() } else { card_bg() },
                );
                ui.painter().rect_stroke(
                    card_rect,
                    Rounding::same(10.0),
                    Stroke::new(1.0_f32, if hovered { border_bright() } else { border_col() }),
                );

                let dur_r = Rect::from_min_size(
                    Pos2::new(thumb_rect.right() - 40.0, thumb_rect.bottom() - 23.0),
                    Vec2::new(32.0, 17.0),
                );
                ui.painter().rect_filled(
                    dur_r,
                    Rounding::same(4.0),
                    Color32::from_rgba_unmultiplied(10, 13, 18, 215),
                );
                ui.painter().text(
                    dur_r.center(),
                    egui::Align2::CENTER_CENTER,
                    clip.duration_str(),
                    FontId::proportional(10.0),
                    TEXT_PRIMARY,
                );

                let star_box = Rect::from_min_size(
                    Pos2::new(thumb_rect.right() - 30.0, thumb_rect.top() + 8.0),
                    Vec2::splat(22.0),
                );
                let star_resp = ui.interact(star_box, Id::new(("clip_star", clip.id)), Sense::click());
                if clip.favorite || hovered || star_resp.hovered() {
                    ui.painter().rect_filled(
                        star_box,
                        Rounding::same(6.0),
                        Color32::from_rgba_unmultiplied(14, 18, 26, 220),
                    );
                    paint_icon(
                        ui.painter(),
                        star_box.shrink(5.0),
                        if clip.favorite {
                            UiIcon::StarFilled
                        } else {
                            UiIcon::StarOutline
                        },
                        if clip.favorite { STAR_GOLD } else { TEXT_MUTED },
                    );
                }

                let title = truncate_text(&clip.title, ((card_w - 24.0) / 6.8) as usize);
                ui.painter().text(
                    Pos2::new(footer_rect.left() + 12.0, footer_rect.top() + 18.0),
                    egui::Align2::LEFT_CENTER,
                    title,
                    FontId::proportional(12.5),
                    TEXT_PRIMARY,
                );

                let mini_r = Rect::from_min_size(
                    Pos2::new(footer_rect.left() + 12.0, footer_rect.top() + 33.0),
                    Vec2::splat(14.0),
                );
                paint_game_badge(ui, state, mini_r, &clip.game, accent.color());
                ui.painter().text(
                    Pos2::new(mini_r.right() + 7.0, mini_r.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{}  •  {}", truncate_text(&clip.game, 12), clip.relative_time(now_unix)),
                    FontId::proportional(11.0),
                    TEXT_MUTED,
                );

                let res_badge = clip.resolution_badge();
                let res_w = if res_badge.len() <= 2 { 26.0 } else { 38.0 };
                let res_r = Rect::from_min_size(
                    Pos2::new(footer_rect.right() - res_w - 10.0, footer_rect.top() + 31.0),
                    Vec2::new(res_w, 18.0),
                );
                ui.painter()
                    .rect_filled(res_r, Rounding::same(4.0), Color32::from_rgb(28, 35, 48));
                ui.painter().text(
                    res_r.center(),
                    egui::Align2::CENTER_CENTER,
                    res_badge,
                    FontId::proportional(10.0),
                    TEXT_MUTED,
                );

                if star_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    state.send_sync(IpcRequest::ToggleFavorite { id: clip.id }, now_secs);
                } else if card_resp.secondary_clicked() {
                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                        state.clip_context_menu = Some((clip.id, pos));
                    }
                } else if card_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    state.clip_context_menu = None;
                    state.open_clip_modal(clip, now_secs);
                }
            }
        });
        ui.add_space(14.0);
    }
}

fn render_settings(
    ui: &mut Ui,
    state: &mut RewindUiState,
    pop: &mut PopoverLayout,
    now_secs: f64,
) {
    let mut cfg_changed = false;
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let hw = state.snapshot.hardware.clone();

    smooth_bounce_scroll(ui, "settings_scroll", |ui| {
            let max_w = 740.0_f32.min(ui.available_width());
            let side_pad = ((ui.available_width() - max_w) * 0.5).max(0.0);

            ui.horizontal(|ui| {
                ui.add_space(side_pad);
                ui.vertical(|ui| {
                    ui.set_max_width(max_w);

                    ui.label(RichText::new("Settings").size(20.0).strong().color(TEXT_PRIMARY));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!(
                            "Detected GPU: {} ({} • {} MB VRAM)  |  Daemon RAM: {:.1} MB",
                            hw.gpu_name, hw.encoder_backend, hw.vram_mb, state.snapshot.daemon_ram_mb
                        ))
                        .size(12.0)
                        .color(TEXT_MUTED),
                    );
                    ui.add_space(18.0);

                    settings_section_header(ui, "Capture & Quality", None);
                    settings_card(ui, |ui| {
                        settings_row(
                            ui,
                            UiIcon::Clock,
                            accent.color(),
                            "Clip Length",
                            "Instant replay buffer duration synced with the top command bar",
                            |ui| {
                                let resp = dropdown_pill(
                                    ui,
                                    state.snapshot.config.clip_length.settings_label(),
                                    None,
                                    None,
                                    state.active_popup == ActivePopup::SettingsClipLength,
                                    118.0,
                                );
                                if resp.clicked() {
                                    state.toggle_popup(ActivePopup::SettingsClipLength);
                                    pop.toggled_this_frame = true;
                                }
                                if state.active_popup == ActivePopup::SettingsClipLength {
                                    pop.anchor = Some(resp.rect);
                                }
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Monitor,
                            TEXT_MUTED,
                            "Resolution & Frame Rate",
                            "Synced live with the top bar quality selector",
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let res_resp = dropdown_pill(
                                        ui,
                                        state.snapshot.config.resolution.label(),
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsResolution,
                                        80.0,
                                    );
                                    if res_resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsResolution);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsResolution {
                                        pop.anchor = Some(res_resp.rect);
                                    }

                                    let fps_resp = dropdown_pill(
                                        ui,
                                        state.snapshot.config.fps.label(),
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsFps,
                                        82.0,
                                    );
                                    if fps_resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsFps);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsFps {
                                        pop.anchor = Some(fps_resp.rect);
                                    }
                                });
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Codec,
                            TEXT_MUTED,
                            "Video Codec & Container",
                            "Zero-copy D3D11 hardware MFT encoding with 1–2s IDR GOP",
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let c_resp = dropdown_pill(
                                        ui,
                                        state.snapshot.config.codec.label(),
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsCodec,
                                        124.0,
                                    );
                                    if c_resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsCodec);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsCodec {
                                        pop.anchor = Some(c_resp.rect);
                                    }

                                    let ct_resp = dropdown_pill(
                                        ui,
                                        state.snapshot.config.container.label(),
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsContainer,
                                        96.0,
                                    );
                                    if ct_resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsContainer);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsContainer {
                                        pop.anchor = Some(ct_resp.rect);
                                    }
                                });
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Bitrate,
                            TEXT_MUTED,
                            "Hardware Encoder & Rate Control",
                            "CQP (~18–20) keeps fast motion sharp and static frames small",
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let enc_label = if hw.vendor_id != 0 {
                                        hw.native_encoder().label()
                                    } else {
                                        state.snapshot.config.encoder_preference.label()
                                    };
                                    let enc_resp = dropdown_pill(
                                        ui,
                                        enc_label,
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsEncoder,
                                        136.0,
                                    );
                                    if enc_resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsEncoder);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsEncoder {
                                        pop.anchor = Some(enc_resp.rect);
                                    }

                                    let rc_resp = dropdown_pill(
                                        ui,
                                        state.snapshot.config.rate_control.label(),
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsRateControl,
                                        118.0,
                                    );
                                    if rc_resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsRateControl);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsRateControl {
                                        pop.anchor = Some(rc_resp.rect);
                                    }
                                });
                            },
                        );

                        settings_divider(ui);

                        let est_mb = state.snapshot.config.estimated_clip_mb();
                        let rc = state.snapshot.config.rate_control;
                        let right_lbl = match rc {
                            RateControl::Cqp => format!("CQP {}", state.snapshot.config.cqp_qp),
                            RateControl::Cbr => format!("{} Mbps", state.snapshot.config.bitrate_mbps),
                        };
                        settings_row(
                            ui,
                            UiIcon::Bitrate,
                            TEXT_MUTED,
                            if rc == RateControl::Cqp { "CQP Quality Target" } else { "Video Bitrate" },
                            if rc == RateControl::Cqp {
                                "Lower QP means higher visual fidelity (~18–20 recommended)"
                            } else {
                                "Constant target bitrate for CBR streams"
                            },
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    ui.label(
                                        RichText::new(right_lbl)
                                            .size(12.5)
                                            .strong()
                                            .color(TEXT_PRIMARY),
                                    );
                                    let badge_txt = if rc == RateControl::Cqp {
                                        format!("~{} MB (motion)", est_mb)
                                    } else {
                                        format!("~{} MB / clip", est_mb)
                                    };
                                    let (b_rect, b_resp) = ui.allocate_exact_size(
                                        Vec2::new(if rc == RateControl::Cqp { 112.0 } else { 88.0 }, 22.0),
                                        Sense::hover(),
                                    );
                                    ui.painter()
                                        .rect_filled(b_rect, Rounding::same(5.0), accent.soft_bg());
                                    ui.painter().rect_stroke(
                                        b_rect,
                                        Rounding::same(5.0),
                                        Stroke::new(1.0_f32, accent.color()),
                                    );
                                    ui.painter().text(
                                        b_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        badge_txt,
                                        FontId::proportional(11.0),
                                        accent.color(),
                                    );
                                    if rc == RateControl::Cqp {
                                        let _ = b_resp.on_hover_text(
                                            "Estimated for active game motion. Static desktop scenes compress much smaller under CQP.",
                                        );
                                    }
                                });
                            },
                        );
                        ui.add_space(6.0);
                        match rc {
                            RateControl::Cqp => {
                                let mut qp = state.snapshot.config.cqp_qp as f32;
                                if custom_slider(ui, "cqp_slider", &mut qp, 14.0, 28.0, accent.color()) {
                                    state.snapshot.config.cqp_qp = qp.round() as u32;
                                    cfg_changed = true;
                                }
                                ui.add_space(6.0);
                                settings_divider(ui);
                                render_disabled_rate_row(
                                    ui,
                                    "Video Bitrate (CBR)",
                                    "Disabled while CQP rate control is active",
                                    &format!("{} Mbps  •  Disabled", state.snapshot.config.bitrate_mbps),
                                );
                            }
                            RateControl::Cbr => {
                                let mut br = state.snapshot.config.bitrate_mbps as f32;
                                if custom_slider(ui, "bitrate_slider", &mut br, 10.0, 80.0, accent.color()) {
                                    state.snapshot.config.bitrate_mbps = br.round() as u32;
                                    cfg_changed = true;
                                }
                                ui.add_space(6.0);
                                settings_divider(ui);
                                render_disabled_rate_row(
                                    ui,
                                    "CQP Quality Target",
                                    "Disabled while CBR bitrate mode is active",
                                    &format!("CQP {}  •  Disabled", state.snapshot.config.cqp_qp),
                                );
                            }
                        }
                        ui.add_space(6.0);

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::BufferRam,
                            TEXT_MUTED,
                            "Buffer Mode",
                            "System RAM eliminates continuous SSD wear (Local Disk locked)",
                            |ui| {
                                let (seg_r, _) =
                                    ui.allocate_exact_size(Vec2::new(178.0, 30.0), Sense::hover());
                                ui.painter().rect_filled(
                                    seg_r,
                                    Rounding::same(7.0),
                                    bar_bg(),
                                );
                                ui.painter().rect_stroke(
                                    seg_r,
                                    Rounding::same(7.0),
                                    Stroke::new(1.0_f32, border_col()),
                                );

                                let ram_r = Rect::from_min_max(
                                    seg_r.left_top() + Vec2::splat(2.0),
                                    Pos2::new(seg_r.center().x - 1.0, seg_r.bottom() - 2.0),
                                );
                                let disk_r = Rect::from_min_max(
                                    Pos2::new(seg_r.center().x + 1.0, seg_r.top() + 2.0),
                                    seg_r.right_bottom() - Vec2::splat(2.0),
                                );
                                let ram_resp = ui.interact(ram_r, Id::new("buf_ram"), Sense::click());
                                let disk_resp = ui.interact(disk_r, Id::new("buf_disk"), Sense::hover());

                                ui.painter()
                                    .rect_filled(ram_r, Rounding::same(5.0), accent.soft_bg());
                                ui.painter().rect_stroke(
                                    ram_r,
                                    Rounding::same(5.0),
                                    Stroke::new(1.0_f32, accent.color()),
                                );
                                ui.painter().rect_filled(
                                    disk_r,
                                    Rounding::same(5.0),
                                    Color32::from_rgb(16, 20, 28),
                                );
                                ui.painter().text(
                                    ram_r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "System RAM",
                                    FontId::proportional(11.5),
                                    accent.color(),
                                );
                                ui.painter().text(
                                    disk_r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Local Disk",
                                    FontId::proportional(11.5),
                                    TEXT_DIM,
                                );
                                if ram_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    state.snapshot.config.buffer_mode = BufferMode::SystemRam;
                                    cfg_changed = true;
                                }
                                disk_resp.on_hover_text("Disabled to prevent continuous SSD wear");
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Monitor,
                            accent.color(),
                            "Auto-Start Screen Capture on Launch",
                            "Automatically start desktop capture when Rewind starts (otherwise waits for a game or Desktop)",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "autostart_cap_switch",
                                    &mut state.snapshot.config.auto_start_capture,
                                    accent.color(),
                                ) {
                                    cfg_changed = true;
                                }
                            },
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(ui, "System & Startup", None);
                    settings_card(ui, |ui| {
                        settings_row(
                            ui,
                            UiIcon::Gear,
                            accent.color(),
                            "Start with Windows",
                            "Launch Rewind capture daemon automatically when Windows starts",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "start_with_windows_switch",
                                    &mut state.snapshot.config.start_with_windows,
                                    accent.color(),
                                ) {
                                    cfg_changed = true;
                                }
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Monitor,
                            TEXT_MUTED,
                            "Close UI on Game Launch",
                            "Automatically close the Rewind window when a game starts and reopen it after you exit the game",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "close_ui_on_game_switch",
                                    &mut state.snapshot.config.close_ui_on_game_launch,
                                    accent.color(),
                                ) {
                                    cfg_changed = true;
                                }
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Play,
                            accent.color(),
                            "Welcome & Quick Tour",
                            "Replay the first-run setup questions and interactive feature tour",
                            |ui| {
                                let (br, bresp) = ui.allocate_exact_size(
                                    Vec2::new(112.0, 28.0),
                                    Sense::click(),
                                );
                                let hov = bresp.hovered();
                                ui.painter().rect_filled(
                                    br,
                                    Rounding::same(14.0),
                                    if hov { card_hover() } else { pill_bg() },
                                );
                                ui.painter().rect_stroke(
                                    br,
                                    Rounding::same(14.0),
                                    Stroke::new(1.0_f32, border_col()),
                                );
                                ui.painter().text(
                                    br.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Replay Tour",
                                    FontId::proportional(12.0),
                                    TEXT_PRIMARY,
                                );
                                if bresp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    state.onboarding_step = 0;
                                    state.tour_step = Some(0);
                                    state.active_tab = NavTab::Home;
                                }
                            },
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(ui, "Discord Rich Presence", None);
                    settings_card(ui, |ui| {
                        settings_row(
                            ui,
                            UiIcon::Codec,
                            Color32::from_rgb(88, 101, 242),
                            "Show Activity on Discord",
                            "Displays active game and Instant Replay status on your Discord profile",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "discord_rpc_switch",
                                    &mut state.snapshot.config.discord_rich_presence,
                                    accent.color(),
                                ) {
                                    cfg_changed = true;
                                }
                            },
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(
                        ui,
                        "Game Detection",
                        Some("Window-only matching — never inspects game processes or memory"),
                    );
                    settings_card(ui, |ui| {
                        let mut add_custom: Option<String> = None;
                        let mut remove_custom: Option<usize> = None;
                        if render_exe_list_block(
                            ui,
                            "Recognized Game Executables",
                            "Always treat windows matching these .exe or game names as capturable games",
                            "Game.exe or title...",
                            "Add",
                            &mut state.custom_game_input,
                            &state.snapshot.config.custom_games,
                            accent,
                            &mut add_custom,
                            &mut remove_custom,
                        ) {
                            cfg_changed = true;
                        }
                        if let Some(raw) = add_custom {
                            let entry = normalize_game_entry(&raw);
                            if !entry.is_empty() {
                                state
                                    .snapshot
                                    .config
                                    .ignored_games
                                    .retain(|x| !x.eq_ignore_ascii_case(&entry));
                                if !state
                                    .snapshot
                                    .config
                                    .custom_games
                                    .iter()
                                    .any(|x| x.eq_ignore_ascii_case(&entry))
                                {
                                    state.snapshot.config.custom_games.push(entry);
                                    cfg_changed = true;
                                }
                            }
                        }
                        if let Some(idx) = remove_custom {
                            if idx < state.snapshot.config.custom_games.len() {
                                state.snapshot.config.custom_games.remove(idx);
                                cfg_changed = true;
                            }
                        }

                        settings_divider(ui);

                        let mut add_ignored: Option<String> = None;
                        let mut remove_ignored: Option<usize> = None;
                        if render_exe_list_block(
                            ui,
                            "Ignored Executables & Apps",
                            "Exclude false-positive windows (e.g. Yapfast.exe) from Auto game detection",
                            "App.exe (e.g. Yapfast.exe)...",
                            "Ignore",
                            &mut state.ignored_game_input,
                            &state.snapshot.config.ignored_games,
                            accent,
                            &mut add_ignored,
                            &mut remove_ignored,
                        ) {
                            cfg_changed = true;
                        }
                        if let Some(raw) = add_ignored {
                            let entry = normalize_game_entry(&raw);
                            if !entry.is_empty() {
                                state
                                    .snapshot
                                    .config
                                    .custom_games
                                    .retain(|x| !x.eq_ignore_ascii_case(&entry));
                                if !state
                                    .snapshot
                                    .config
                                    .ignored_games
                                    .iter()
                                    .any(|x| x.eq_ignore_ascii_case(&entry))
                                {
                                    state.snapshot.config.ignored_games.push(entry);
                                    cfg_changed = true;
                                }
                            }
                        }
                        if let Some(idx) = remove_ignored {
                            if idx < state.snapshot.config.ignored_games.len() {
                                state.snapshot.config.ignored_games.remove(idx);
                                cfg_changed = true;
                            }
                        }
                    });

                    ui.add_space(18.0);

                    settings_section_header(ui, "Audio", None);
                    settings_card(ui, |ui| {
                        let out_dev = state.snapshot.config.game_audio_device.clone();
                        settings_row(
                            ui,
                            UiIcon::Speaker,
                            accent.color(),
                            "Game Audio (System Loopback)",
                            "Captures default system audio without touching any game process",
                            |ui| {
                                let resp = dropdown_pill(
                                    ui,
                                    &truncate_text(&out_dev, 24),
                                    None,
                                    None,
                                    state.active_popup == ActivePopup::SettingsGameAudio,
                                    198.0,
                                );
                                if resp.clicked() {
                                    state.toggle_popup(ActivePopup::SettingsGameAudio);
                                    pop.toggled_this_frame = true;
                                }
                                if state.active_popup == ActivePopup::SettingsGameAudio {
                                    pop.anchor = Some(resp.rect);
                                }
                            },
                        );
                        ui.add_space(6.0);
                        if volume_row(
                            ui,
                            "game_vol_slider",
                            &mut state.snapshot.config.game_audio_volume,
                            accent.color(),
                        ) {
                            cfg_changed = true;
                        }

                        settings_divider(ui);

                        let mic_dev = state.snapshot.config.mic_device.clone();
                        settings_row(
                            ui,
                            UiIcon::Mic,
                            TEXT_MUTED,
                            "Microphone Device",
                            "Input microphone recorded on independent track",
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    let resp = dropdown_pill(
                                        ui,
                                        &truncate_text(&mic_dev, 22),
                                        None,
                                        None,
                                        state.active_popup == ActivePopup::SettingsMicDevice,
                                        184.0,
                                    );
                                    if resp.clicked() {
                                        state.toggle_popup(ActivePopup::SettingsMicDevice);
                                        pop.toggled_this_frame = true;
                                    }
                                    if state.active_popup == ActivePopup::SettingsMicDevice {
                                        pop.anchor = Some(resp.rect);
                                    }

                                    if toggle_switch(
                                        ui,
                                        "mic_switch",
                                        &mut state.snapshot.config.mic_enabled,
                                        accent.color(),
                                    ) {
                                        cfg_changed = true;
                                    }
                                });
                            },
                        );
                        ui.add_space(6.0);
                        if volume_row(
                            ui,
                            "mic_vol_slider",
                            &mut state.snapshot.config.mic_volume,
                            accent.color(),
                        ) {
                            cfg_changed = true;
                        }

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::AudioTracks,
                            TEXT_MUTED,
                            "Separate Audio Tracks",
                            "Split game sound and microphone onto independent container tracks",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "tracks_switch",
                                    &mut state.snapshot.config.separate_audio_tracks,
                                    accent.color(),
                                ) {
                                    cfg_changed = true;
                                }
                            },
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(
                        ui,
                        "Hotkeys",
                        Some("Click a keycap to rebind  •  Esc to cancel"),
                    );
                    settings_card(ui, |ui| {
                        hotkey_row(
                            ui,
                            "Save Clip",
                            "Snapshot ring buffer to lossless MP4/MKV immediately",
                            &state.snapshot.config.hotkey_save_clip,
                            state.rebind_target == RebindTarget::SaveClip,
                            accent.color(),
                            || state.rebind_target = RebindTarget::SaveClip,
                        );
                        settings_divider(ui);
                        hotkey_row(
                            ui,
                            "Toggle Microphone",
                            "Mute or unmute microphone recording globally",
                            &state.snapshot.config.hotkey_toggle_mic,
                            state.rebind_target == RebindTarget::ToggleMic,
                            accent.color(),
                            || state.rebind_target = RebindTarget::ToggleMic,
                        );
                        settings_divider(ui);
                        hotkey_row(
                            ui,
                            "Add Bookmark",
                            "Place a highlight timestamp in active buffer",
                            &state.snapshot.config.hotkey_add_bookmark,
                            state.rebind_target == RebindTarget::AddBookmark,
                            accent.color(),
                            || state.rebind_target = RebindTarget::AddBookmark,
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(ui, "Storage", None);
                    settings_card(ui, |ui| {
                        let clips_dir = state.snapshot.config.clips_folder.clone();
                        settings_row(
                            ui,
                            UiIcon::Folder,
                            Color32::from_rgb(234, 179, 8),
                            "Clips Folder",
                            &clips_dir,
                            |ui| {
                                let (b_rect, b_resp) =
                                    ui.allocate_exact_size(Vec2::new(72.0, 28.0), Sense::click());
                                ui.painter().rect_filled(
                                    b_rect,
                                    Rounding::same(6.0),
                                    if b_resp.hovered() { card_hover() } else { pill_bg() },
                                );
                                ui.painter().rect_stroke(
                                    b_rect,
                                    Rounding::same(6.0),
                                    Stroke::new(1.0_f32, border_col()),
                                );
                                ui.painter().text(
                                    b_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Browse",
                                    FontId::proportional(11.5),
                                    TEXT_PRIMARY,
                                );
                                if b_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    state.send_sync(IpcRequest::PickFolder, now_secs);
                                }
                            },
                        );

                        settings_divider(ui);

                        let sub_storage = format!(
                            "{:.2} GB used of {} GB",
                            state.used_storage_gb(),
                            state.snapshot.config.max_storage_gb
                        );
                        settings_row(
                            ui,
                            UiIcon::Database,
                            accent.color(),
                            "Maximum Storage Limit",
                            &sub_storage,
                            |ui| {
                                ui.label(
                                    RichText::new(format!("{} GB", state.snapshot.config.max_storage_gb))
                                        .size(12.5)
                                        .strong()
                                        .color(accent.color()),
                                );
                            },
                        );
                        ui.add_space(6.0);
                        let mut limit_f = state.snapshot.config.max_storage_gb as f32;
                        if custom_slider(
                            ui,
                            "storage_limit_slider",
                            &mut limit_f,
                            50.0,
                            500.0,
                            accent.color(),
                        ) {
                            state.snapshot.config.max_storage_gb = (limit_f / 10.0).round() as u32 * 10;
                            cfg_changed = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("50 GB").size(10.5).color(TEXT_DIM));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new("500 GB").size(10.5).color(TEXT_DIM));
                            });
                        });

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Trash,
                            TEXT_MUTED,
                            "Auto-Delete Oldest Clips",
                            "Automatically free space when reaching quota (Favorites are protected)",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "autodel_switch",
                                    &mut state.snapshot.config.auto_delete_oldest,
                                    accent.color(),
                                ) {
                                    cfg_changed = true;
                                }
                            },
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(ui, "Personalization", None);
                    settings_card(ui, |ui| {
                        let bg_path = state.snapshot.config.background_image_path.clone();
                        let bg_sub = if bg_path.is_empty() {
                            "Optional custom PNG/JPG backdrop behind Fluent UI".to_owned()
                        } else {
                            truncate_text(&bg_path, 52)
                        };
                        settings_row(
                            ui,
                            UiIcon::Monitor,
                            accent.color(),
                            "Background Picture",
                            &bg_sub,
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    if !bg_path.is_empty() && ui.button("Reset").clicked() {
                                        state.snapshot.config.background_image_path.clear();
                                        state.snapshot.config.custom_accent_rgb = None;
                                        state.bg_cache_key = None;
                                        state.bg_texture = None;
                                        state.bg_ambient_tint = None;
                                        cfg_changed = true;
                                    }
                                    let (b_rect, b_resp) =
                                        ui.allocate_exact_size(Vec2::new(112.0, 28.0), Sense::click());
                                    ui.painter().rect_filled(
                                        b_rect,
                                        Rounding::same(6.0),
                                        if b_resp.hovered() { card_hover() } else { pill_bg() },
                                    );
                                    ui.painter().rect_stroke(
                                        b_rect,
                                        Rounding::same(6.0),
                                        Stroke::new(1.0_f32, border_col()),
                                    );
                                    ui.painter().text(
                                        b_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "Choose Image...",
                                        FontId::proportional(11.5),
                                        TEXT_PRIMARY,
                                    );
                                    if b_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                        if let Some(chosen) = pick_image_file_dialog() {
                                            state.snapshot.config.background_image_path = chosen;
                                            state.bg_cache_key = None;
                                            if let Some(rgb) = state.ensure_background_loaded(ui.ctx()) {
                                                state.snapshot.config.custom_accent_rgb = Some(rgb);
                                            }
                                            cfg_changed = true;
                                        }
                                    }
                                });
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Bitrate,
                            TEXT_MUTED,
                            "Background Blur",
                            "Soften custom background image behind cards and navigation",
                            |ui| {
                                ui.label(
                                    RichText::new(format!("{} px", state.snapshot.config.background_blur))
                                        .size(12.5)
                                        .strong()
                                        .color(accent.color()),
                                );
                            },
                        );
                        ui.add_space(4.0);
                        let mut blur_f = state.snapshot.config.background_blur as f32;
                        if custom_slider(ui, "bg_blur_slider", &mut blur_f, 0.0, 40.0, accent.color()) {
                            let next_blur = blur_f.round() as u32;
                            if next_blur != state.snapshot.config.background_blur {
                                state.snapshot.config.background_blur = next_blur;
                                cfg_changed = true;
                            }
                        }
                        ui.add_space(6.0);

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Monitor,
                            TEXT_MUTED,
                            "Background Brightness",
                            "Adjust wallpaper dimming for contrast behind translucent UI",
                            |ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{}%",
                                        state.snapshot.config.background_brightness
                                    ))
                                    .size(12.5)
                                    .strong()
                                    .color(accent.color()),
                                );
                            },
                        );
                        ui.add_space(4.0);
                        let mut bright_f = state.snapshot.config.background_brightness as f32;
                        if custom_slider(
                            ui,
                            "bg_brightness_slider",
                            &mut bright_f,
                            5.0,
                            100.0,
                            accent.color(),
                        ) {
                            let next_b = bright_f.round() as u32;
                            if next_b != state.snapshot.config.background_brightness {
                                state.snapshot.config.background_brightness = next_b;
                                cfg_changed = true;
                            }
                        }
                        ui.add_space(6.0);

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Palette,
                            accent.color(),
                            "Auto-Adjust Palette from Background",
                            "Automatically theme accent and gradients from your chosen image",
                            |ui| {
                                if toggle_switch(
                                    ui,
                                    "auto_palette_switch",
                                    &mut state.snapshot.config.auto_adjust_palette,
                                    accent.color(),
                                ) {
                                    if state.snapshot.config.auto_adjust_palette {
                                        if !state.snapshot.config.background_image_path.is_empty() {
                                            state.bg_cache_key = None;
                                            if let Some(rgb) = state.ensure_background_loaded(ui.ctx()) {
                                                state.snapshot.config.custom_accent_rgb = Some(rgb);
                                            }
                                        }
                                    } else {
                                        state.snapshot.config.custom_accent_rgb = None;
                                    }
                                    cfg_changed = true;
                                }
                            },
                        );

                        settings_divider(ui);

                        let palette_locked = state.snapshot.config.auto_adjust_palette
                            && !state.snapshot.config.background_image_path.is_empty();
                        settings_row(
                            ui,
                            UiIcon::Palette,
                            if palette_locked { TEXT_DIM } else { accent.color() },
                            "Windows 11 Accent Color",
                            if palette_locked {
                                "Controlled automatically by Auto-Adjust Palette"
                            } else {
                                "Themes active indicators, buttons, storage ring, and Mica glow"
                            },
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    for swatch in AccentColor::ALL {
                                        let active = !palette_locked
                                            && state.snapshot.config.custom_accent_rgb.is_none()
                                            && state.snapshot.config.accent_color == swatch;
                                        let (s_rect, s_resp) =
                                            ui.allocate_exact_size(Vec2::splat(26.0), if palette_locked { Sense::hover() } else { Sense::click() });
                                        let c = swatch.color();
                                        let draw_col = if palette_locked {
                                            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 85)
                                        } else {
                                            c
                                        };
                                        if active {
                                            ui.painter().circle_stroke(
                                                s_rect.center(),
                                                12.5,
                                                Stroke::new(1.8_f32, Color32::WHITE),
                                            );
                                        }
                                        ui.painter().circle_filled(s_rect.center(), 9.5, draw_col);
                                        if !palette_locked
                                            && s_resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
                                        {
                                            state.snapshot.config.accent_color = swatch;
                                            state.snapshot.config.custom_accent_rgb = None;
                                            cfg_changed = true;
                                        }
                                    }
                                    if !palette_locked {
                                        let mut rgb = [
                                            accent.color().r(),
                                            accent.color().g(),
                                            accent.color().b(),
                                        ];
                                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                                            state.snapshot.config.custom_accent_rgb = Some(rgb);
                                            cfg_changed = true;
                                        }
                                    }
                                });
                            },
                        );

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Library,
                            accent.color(),
                            "SteamGridDB Game Icons (Optional)",
                            "Fetches real game cover icons for library cards instead of letter badges",
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let key_resp = ui
                                        .add(
                                            TextEdit::singleline(
                                                &mut state.snapshot.config.steamgriddb_api_key,
                                            )
                                            .password(true)
                                            .desired_width(155.0)
                                            .hint_text("Paste API key..."),
                                        )
                                        .on_hover_text(
                                            "Optional: paste a free API key from steamgriddb.com/profile/preferences/api to download real game cover icons for Home, Folders, and filter pills. Leave blank for default letter badges.",
                                        );
                                    if key_resp.lost_focus() {
                                        state.clear_game_icon_cache();
                                        cfg_changed = true;
                                    }
                                    if modal_action_pill(
                                        ui,
                                        UiIcon::Link,
                                        "Get Key",
                                        TEXT_PRIMARY,
                                        pill_bg(),
                                        border_col(),
                                        TEXT_MUTED,
                                    ) {
                                        open_url(STEAMGRIDDB_API_KEY_URL);
                                    }
                                    let rf_fg = contrasting_fg(accent.color());
                                    if modal_action_pill(
                                        ui,
                                        UiIcon::Check,
                                        "Refresh",
                                        rf_fg,
                                        accent.color(),
                                        accent.color(),
                                        rf_fg,
                                    ) {
                                        state.clear_game_icon_cache();
                                        cfg_changed = true;
                                    }
                                });
                            },
                        );
                    });

                    ui.add_space(18.0);

                    settings_section_header(ui, "Updates", Some("github.com/BioBuch/Rewind/releases"));
                    settings_card(ui, |ui| {
                        let status_sub = match &state.update_status {
                            UpdateStatus::Idle => format!("Installed version: v{CURRENT_VERSION}"),
                            UpdateStatus::Checking => "Checking GitHub Releases...".to_owned(),
                            UpdateStatus::UpToDate => {
                                format!("Up to date (v{CURRENT_VERSION} is the latest release)")
                            }
                            UpdateStatus::Available(info) => {
                                format!("Update required: {} available", info.tag_name)
                            }
                            UpdateStatus::Downloading(info) => {
                                format!("Downloading {} installer...", info.tag_name)
                            }
                            UpdateStatus::Error(_, err) => format!("Update failed: {err}"),
                            UpdateStatus::CheckFailed(err) => err.clone(),
                        };
                        ui.horizontal(|ui| {
                            let (icon_box, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                            if let Some(tex) = &state.app_icon_texture {
                                ui.painter().image(
                                    tex.id(),
                                    icon_box,
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            }
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(
                                    RichText::new(format!("Rewind v{CURRENT_VERSION}"))
                                        .size(13.0)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.label(
                                    RichText::new(&status_sub)
                                        .size(11.5)
                                        .color(TEXT_MUTED),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    if modal_action_pill(
                                        ui,
                                        UiIcon::Link,
                                        "Releases",
                                        TEXT_PRIMARY,
                                        pill_bg(),
                                        border_col(),
                                        TEXT_MUTED,
                                    ) {
                                        open_url(RELEASES_PAGE_URL);
                                    }
                                    let checking = matches!(
                                        state.update_status,
                                        UpdateStatus::Checking | UpdateStatus::Downloading(_)
                                    );
                                    let chk_lbl = if checking { "Checking..." } else { "Check Now" };
                                    let chk_fg = contrasting_fg(accent.color());
                                    if modal_action_pill(
                                        ui,
                                        UiIcon::Check,
                                        chk_lbl,
                                        chk_fg,
                                        accent.color(),
                                        accent.color(),
                                        chk_fg,
                                    ) && !checking
                                    {
                                        state.trigger_update_check(true);
                                    }
                                });
                            });
                        });
                    });

                    ui.add_space(28.0);
                });
            });
        });

    if cfg_changed {
        state.push_config_update(now_secs);
    }
}

fn render_disabled_rate_row(ui: &mut Ui, title: &str, subtitle: &str, right_pill: &str) {
    settings_row(ui, UiIcon::Bitrate, TEXT_DIM, title, subtitle, |ui| {
        let (b_rect, _) = ui.allocate_exact_size(Vec2::new(146.0, 22.0), Sense::hover());
        ui.painter().rect_filled(
            b_rect,
            Rounding::same(5.0),
            Color32::from_rgb(18, 22, 31),
        );
        ui.painter().rect_stroke(
            b_rect,
            Rounding::same(5.0),
            Stroke::new(1.0_f32, Color32::from_rgb(30, 36, 48)),
        );
        ui.painter().text(
            b_rect.center(),
            egui::Align2::CENTER_CENTER,
            right_pill,
            FontId::proportional(10.5),
            TEXT_DIM,
        );
    });
    let w = ui.available_width().max(60.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 12.0), Sense::hover());
    let track_r = Rect::from_min_max(
        Pos2::new(r.left() + 6.0, r.center().y - 2.0),
        Pos2::new(r.right() - 6.0, r.center().y + 2.0),
    );
    ui.painter()
        .rect_filled(track_r, Rounding::same(2.0), Color32::from_rgb(26, 31, 41));
}

fn render_active_popover(
    ctx: &egui::Context,
    state: &mut RewindUiState,
    anchor: Option<Rect>,
    now_secs: f64,
) -> Option<Rect> {
    if state.active_popup == ActivePopup::None {
        return None;
    }
    let anchor = anchor?;
    let accent = ThemeAccent::from_config(&state.snapshot.config);

    let (width, right_align) = match state.active_popup {
        ActivePopup::CaptureSource => (220.0, false),
        ActivePopup::ClipLength => (216.0, false),
        ActivePopup::VideoQuality => (216.0, false),
        ActivePopup::Notifications => (296.0, true),
        ActivePopup::Profile => (236.0, true),
        ActivePopup::SortMenu => (132.0, true),
        ActivePopup::SettingsClipLength => (144.0, true),
        ActivePopup::SettingsResolution => (108.0, true),
        ActivePopup::SettingsFps => (188.0, true),
        ActivePopup::SettingsCodec => (160.0, true),
        ActivePopup::SettingsEncoder => (172.0, true),
        ActivePopup::SettingsRateControl => (150.0, true),
        ActivePopup::SettingsContainer => (132.0, true),
        ActivePopup::SettingsGameAudio | ActivePopup::SettingsMicDevice => (240.0, true),
        ActivePopup::None => return None,
    };

    let screen = ctx.screen_rect();
    let x = if right_align {
        (anchor.right() - width).max(8.0)
    } else {
        anchor.left().min(screen.right() - width - 8.0)
    };
    let anim_t = anim_open_progress(
        ctx,
        Id::new("popover_open_anim"),
        state.active_popup as u64,
        now_secs,
        0.16,
    );
    let y = (anchor.bottom() + 6.0).min(screen.bottom() - 220.0) - (1.0 - anim_t) * 8.0;

    let area_resp = egui::Area::new(Id::new("floating_popover"))
        .order(Order::Foreground)
        .fixed_pos(Pos2::new(x, y))
        .show(ctx, |ui| {
            ui.set_opacity(anim_t);
            egui::Frame::none()
                .fill(popover_bg())
                .stroke(Stroke::new(1.0_f32, border_bright()))
                .rounding(Rounding::same(14.0))
                .inner_margin(Margin::same(8.0))
                .shadow(Shadow {
                    offset: Vec2::new(0.0, 8.0),
                    blur: 24.0,
                    spread: 2.0,
                    color: Color32::from_rgba_unmultiplied(0, 0, 0, (150.0 * anim_t) as u8),
                })
                .show(ui, |ui| {
                    ui.set_width(width - 16.0);
                    match state.active_popup {
                        ActivePopup::CaptureSource => {
                            popover_section_title(ui, "CAPTURE SOURCE");
                            let auto_sel = state.snapshot.manual_source.is_none();
                            let auto_lbl = if let Some(g) = &state.snapshot.active_game {
                                format!("Auto ({g})")
                            } else {
                                "Auto (Active Game)".to_owned()
                            };
                            if popover_check_item(ui, &auto_lbl, None, auto_sel, accent) {
                                state.send_sync(IpcRequest::SetCaptureSource(None), now_secs);
                                state.active_popup = ActivePopup::None;
                            }
                            let desk_sel = state.snapshot.manual_source.as_deref() == Some("Desktop");
                            if popover_check_item(
                                ui,
                                "Desktop (DXGI / WGC)",
                                Some("Capture primary monitor"),
                                desk_sel,
                                accent,
                            ) {
                                state.send_sync(
                                    IpcRequest::SetCaptureSource(Some("Desktop".to_owned())),
                                    now_secs,
                                );
                                state.active_popup = ActivePopup::None;
                            }
                        }
                        ActivePopup::ClipLength => {
                            popover_section_title(ui, "CLIP LENGTH");
                            ui.label(
                                RichText::new(format!(
                                    "Current: {}",
                                    state.snapshot.config.clip_length.settings_label()
                                ))
                                .size(11.5)
                                .color(TEXT_MUTED),
                            );
                            ui.add_space(8.0);
                            if popover_settings_link_btn(ui, "Go to Recorder Settings", accent) {
                                state.navigate(NavTab::Settings);
                            }
                        }
                        ActivePopup::VideoQuality => {
                            popover_section_title(ui, "VIDEO QUALITY");
                            ui.label(
                                RichText::new(format!(
                                    "Current: {}",
                                    state.snapshot.config.top_bar_quality_label()
                                ))
                                .size(11.5)
                                .color(TEXT_MUTED),
                            );
                            ui.add_space(8.0);
                            if popover_settings_link_btn(ui, "Go to Recorder Settings", accent) {
                                state.navigate(NavTab::Settings);
                            }
                        }
                        ActivePopup::Notifications => {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Recent Events")
                                        .size(12.5)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let link = ui.add(
                                        egui::Button::new(
                                            RichText::new("View Library")
                                                .size(11.5)
                                                .color(accent.color()),
                                        )
                                        .frame(false),
                                    );
                                    if link.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                        state.navigate(NavTab::AllClips);
                                    }
                                });
                            });
                            ui.add_space(6.0);
                            if state.snapshot.recent_events.is_empty() && state.snapshot.clips.is_empty()
                            {
                                ui.label(
                                    RichText::new("No captures or daemon events yet.")
                                        .size(11.5)
                                        .color(TEXT_MUTED),
                                );
                            } else {
                                for ev in state.snapshot.recent_events.iter().take(5) {
                                    ui.label(
                                        RichText::new(format!(
                                            "• {} ({})",
                                            ev.message,
                                            RewindUiState::format_relative_unix(ev.unix_secs)
                                        ))
                                        .size(11.0)
                                        .color(TEXT_PRIMARY),
                                    );
                                    ui.add_space(3.0);
                                }
                            }
                        }
                        ActivePopup::Profile => {
                            ui.add_space(4.0);
                            let user = std::env::var("USERNAME").unwrap_or_else(|_| "Local User".to_owned());
                            ui.label(RichText::new(user).size(13.5).strong().color(TEXT_PRIMARY));
                            ui.label(
                                RichText::new(format!(
                                    "{} • Daemon {:.1} MB",
                                    state.snapshot.hardware.encoder_backend,
                                    state.snapshot.daemon_ram_mb
                                ))
                                .size(11.0)
                                .color(TEXT_MUTED),
                            );
                            ui.add_space(8.0);
                            settings_divider(ui);
                            ui.add_space(4.0);
                            if popover_simple_item(ui, "Manage Storage") {
                                state.navigate(NavTab::Settings);
                            }
                            if popover_simple_item(ui, "Recorder Settings") {
                                state.navigate(NavTab::Settings);
                            }
                            if popover_simple_item(ui, "Rescan Clips Folder") {
                                state.send_sync(IpcRequest::SyncLibrary, now_secs);
                                state.active_popup = ActivePopup::None;
                            }
                        }
                        ActivePopup::SortMenu => {
                            for ord in SortOrder::ALL {
                                if popover_check_item(
                                    ui,
                                    ord.label(),
                                    None,
                                    state.sort_order == ord,
                                    accent,
                                ) {
                                    state.sort_order = ord;
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsClipLength => {
                            for len in ClipLength::ALL {
                                if popover_check_item(
                                    ui,
                                    len.settings_label(),
                                    None,
                                    state.snapshot.config.clip_length == len,
                                    accent,
                                ) {
                                    state.snapshot.config.clip_length = len;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsResolution => {
                            for res in Resolution::ALL {
                                if popover_check_item(
                                    ui,
                                    res.label(),
                                    None,
                                    state.snapshot.config.resolution == res,
                                    accent,
                                ) {
                                    state.snapshot.config.resolution = res;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsFps => {
                            for fps in FrameRate::ALL {
                                let enabled = !(fps == FrameRate::Fps120
                                    && state.snapshot.config.codec == VideoCodec::H264);
                                let sub = (!enabled).then_some("Requires H.265 (HEVC) or AV1");
                                if popover_check_item_enabled(
                                    ui,
                                    fps.label(),
                                    sub,
                                    state.snapshot.config.fps == fps,
                                    accent,
                                    enabled,
                                ) && enabled
                                {
                                    state.snapshot.config.fps = fps;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsCodec => {
                            for codec in VideoCodec::ALL {
                                let supported = state.snapshot.hardware.is_codec_supported(codec);
                                let sub = (!supported).then_some("Unsupported on this GPU");
                                if popover_check_item_enabled(
                                    ui,
                                    codec.label(),
                                    sub,
                                    state.snapshot.config.codec == codec,
                                    accent,
                                    supported,
                                ) && supported
                                {
                                    state.snapshot.config.codec = codec;
                                    if codec == VideoCodec::H264
                                        && state.snapshot.config.fps == FrameRate::Fps120
                                    {
                                        state.snapshot.config.fps = FrameRate::Fps60;
                                    }
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsEncoder => {
                            for enc in [
                                EncoderPreference::Nvenc,
                                EncoderPreference::Amf,
                                EncoderPreference::Qsv,
                            ] {
                                let supported = state.snapshot.hardware.is_encoder_supported(enc);
                                let sub = if supported {
                                    Some("Detected on your GPU")
                                } else {
                                    Some("Not available on this GPU")
                                };
                                if popover_check_item_enabled(
                                    ui,
                                    enc.label(),
                                    sub,
                                    state.snapshot.hardware.native_encoder() == enc,
                                    accent,
                                    supported,
                                ) && supported
                                {
                                    state.snapshot.config.encoder_preference = enc;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsRateControl => {
                            for rc in [RateControl::Cqp, RateControl::Cbr] {
                                if popover_check_item(
                                    ui,
                                    rc.label(),
                                    None,
                                    state.snapshot.config.rate_control == rc,
                                    accent,
                                ) {
                                    state.snapshot.config.rate_control = rc;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsContainer => {
                            for ct in [ContainerFormat::Mp4, ContainerFormat::Mkv] {
                                if popover_check_item(
                                    ui,
                                    ct.label(),
                                    None,
                                    state.snapshot.config.container == ct,
                                    accent,
                                ) {
                                    state.snapshot.config.container = ct;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsGameAudio => {
                            let devs = state.snapshot.audio_outputs.clone();
                            for dev in devs {
                                let sel = state.snapshot.config.game_audio_device == dev;
                                if popover_check_item(ui, &dev, None, sel, accent) {
                                    state.snapshot.config.game_audio_device = dev;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::SettingsMicDevice => {
                            let devs = state.snapshot.audio_inputs.clone();
                            for dev in devs {
                                let sel = state.snapshot.config.mic_device == dev;
                                if popover_check_item(ui, &dev, None, sel, accent) {
                                    state.snapshot.config.mic_device = dev;
                                    state.push_config_update(now_secs);
                                    state.active_popup = ActivePopup::None;
                                }
                            }
                        }
                        ActivePopup::None => {}
                    }
                });
        });

    Some(area_resp.response.rect)
}

fn format_m_ss(secs: f32) -> String {
    let s = secs.max(0.0).floor() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

fn format_file_size(bytes: u64) -> String {
    let mb = (bytes as f32) / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{:.1} MB", mb)
    }
}

fn render_clip_modal(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) {
    let Some(clip_id) = state.selected_clip_id else {
        return;
    };
    let Some(clip) = state.snapshot.clips.iter().find(|c| c.id == clip_id).cloned() else {
        state.close_clip_modal();
        return;
    };

    let dur = clip.duration_secs.max(0.5);
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let ac = accent.color();

    let (prev_clip, next_clip) = {
        let filtered = state.filtered_clips();
        let cur_pos = filtered.iter().position(|c| c.id == clip_id);
        let prev = cur_pos.and_then(|i| if i > 0 { Some((*filtered[i - 1]).clone()) } else { None });
        let next = cur_pos.and_then(|i| filtered.get(i + 1).map(|c| (*c).clone()));
        (prev, next)
    };

    let wants_kb = ctx.wants_keyboard_input();
    if !wants_kb {
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if state.modal_fullscreen {
                state.modal_fullscreen = false;
            } else {
                state.close_clip_modal();
            }
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::F)) {
            state.modal_fullscreen = !state.modal_fullscreen;
        }
        if ctx.input(|i| i.key_pressed(Key::ArrowLeft)) && !state.modal_fullscreen {
            if let Some(p) = &prev_clip {
                state.open_clip_modal(p, now_secs);
                return;
            }
        }
        if ctx.input(|i| i.key_pressed(Key::ArrowRight)) && !state.modal_fullscreen {
            if let Some(n) = &next_clip {
                state.open_clip_modal(n, now_secs);
                return;
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Delete)) && !state.modal_fullscreen {
            state.close_clip_modal();
            state.send_sync(IpcRequest::DeleteClip { id: clip.id }, now_secs);
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Space)) {
            state.player_playing = !state.player_playing;
            if let Some(dec) = &state.decoder {
                dec.set_playing(state.player_playing);
            }
        }
    }

    if state.viewport_fullscreen_applied != state.modal_fullscreen {
        set_active_window_borderless_fullscreen(state.modal_fullscreen);
        state.viewport_fullscreen_applied = state.modal_fullscreen;
    }

    let gain = if state.player_muted { 0.0 } else { state.player_volume };
    let (preload_ready, preload_pct) = state
        .decoder
        .as_ref()
        .map(|d| d.preload_progress())
        .unwrap_or((true, 1.0));
    if let Some(dec) = &state.decoder {
        dec.set_audio_gain(gain);
        dec.set_playing(state.player_playing);
        if let Some(img) = dec.take_latest_frame(&mut state.player_frame_seq) {
            if let Some(prev_arc) = state.last_player_frame.take() {
                if let Ok(prev_img) = std::sync::Arc::try_unwrap(prev_arc) {
                    dec.recycle_frame_pixels(prev_img.pixels);
                }
            }
            let sz = img.size;
            let arc = std::sync::Arc::new(img);
            let img_data = egui::ImageData::Color(arc.clone());
            if let Some(tex) = &mut state.player_texture {
                if tex.size() == sz {
                    tex.set(img_data, TextureOptions::LINEAR);
                } else {
                    *tex = ctx.load_texture(
                        format!("player_frame_{clip_id}"),
                        img_data,
                        TextureOptions::LINEAR,
                    );
                }
            } else {
                state.player_texture = Some(ctx.load_texture(
                    format!("player_frame_{clip_id}"),
                    img_data,
                    TextureOptions::LINEAR,
                ));
            }
            state.last_player_frame = Some(arc);
        }
        if !preload_ready {
            ctx.request_repaint();
        } else if state.player_playing {
            state.player_pos_secs = dec.current_pos_secs().clamp(0.0, dur);
            ctx.request_repaint_after(std::time::Duration::from_millis(6));
        } else if state.player_texture.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    let screen = ctx.screen_rect();
    let modal_thumb = state.get_or_load_thumbnail(ctx, &clip.thumb_path);

    if state.modal_fullscreen {
        egui::Area::new(Id::new("clip_modal_overlay"))
            .order(Order::Foreground)
            .fixed_pos(screen.left_top())
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.set_max_size(screen.size());
                ui.painter().rect_filled(screen, Rounding::ZERO, Color32::BLACK);

                let vid_ar = if let Some(t) = &state.player_texture {
                    let [tw, th] = t.size();
                    (tw.max(1) as f32) / (th.max(1) as f32)
                } else if clip.width > 0 && clip.height > 0 {
                    (clip.width as f32) / (clip.height as f32)
                } else {
                    16.0 / 9.0
                };
                let scr_ar = screen.width().max(1.0) / screen.height().max(1.0);
                let vid_sz = if vid_ar > scr_ar {
                    Vec2::new(screen.width(), screen.width() / vid_ar)
                } else {
                    Vec2::new(screen.height() * vid_ar, screen.height())
                };
                let video_rect = Rect::from_center_size(screen.center(), vid_sz);
                if let Some(tex) = state.player_texture.as_ref().or(modal_thumb.as_ref()) {
                    ui.painter().image(
                        tex.id(),
                        video_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    paint_empty_thumb_surface(ui.painter(), video_rect, Rounding::ZERO);
                }

                if !preload_ready {
                    paint_preload_overlay(ui.painter(), video_rect, preload_pct, ac);
                }

                render_player_controls_overlay(ui, state, screen, screen, dur, ac);
            });
        return;
    }

    let modal_t = ease_out_cubic(((now_secs - state.modal_opened_at) as f32 / 0.20).clamp(0.0, 1.0));
    if modal_t < 1.0 {
        ctx.request_repaint();
    }
    let modal_w = 780.0_f32.min(screen.width() - 96.0).max(540.0);
    let modal_h = 600.0_f32.min(screen.height() - 40.0).max(460.0);
    let modal_center = screen.center() + Vec2::new(0.0, (1.0 - modal_t) * 14.0);
    let modal_rect = Rect::from_center_size(modal_center, Vec2::new(modal_w, modal_h));
    let prev_rect = Rect::from_center_size(
        Pos2::new(modal_rect.left() - 28.0, modal_rect.center().y),
        Vec2::splat(36.0),
    );
    let next_rect = Rect::from_center_size(
        Pos2::new(modal_rect.right() + 28.0, modal_rect.center().y),
        Vec2::splat(36.0),
    );

    let now_unix = rewind::win_ffi::unix_now_secs();
    let mut should_close = false;

    egui::Area::new(Id::new("clip_modal_overlay"))
        .order(Order::Foreground)
        .fixed_pos(screen.left_top())
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            ui.set_opacity(modal_t);
            ui.painter().rect_filled(
                screen,
                Rounding::ZERO,
                Color32::from_rgba_unmultiplied(6, 8, 13, (185.0 * modal_t) as u8),
            );

            if nav_chevron_button_at(ui, prev_rect, UiIcon::ChevronLeft, prev_clip.is_some()) {
                if let Some(p) = &prev_clip {
                    state.open_clip_modal(p, now_secs);
                }
            }
            if nav_chevron_button_at(ui, next_rect, UiIcon::ChevronRight, next_clip.is_some()) {
                if let Some(n) = &next_clip {
                    state.open_clip_modal(n, now_secs);
                }
            }

            ui.painter().rect_filled(
                modal_rect,
                Rounding::same(18.0),
                Color32::from_rgba_unmultiplied(15, 19, 27, 246),
            );
            ui.painter().rect_stroke(
                modal_rect,
                Rounding::same(18.0),
                Stroke::new(1.0_f32, border_bright()),
            );

            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(modal_rect), |ui| {
                ui.vertical(|ui| {
                    ui.set_width(modal_w);
                    ui.spacing_mut().item_spacing.y = 0.0;

                    let header_r = Rect::from_min_size(
                        modal_rect.left_top(),
                        Vec2::new(modal_w, 50.0),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(header_r.shrink2(Vec2::new(18.0, 10.0))), |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;

                            let g_w = (clip.game.len() as f32 * 6.8 + 18.0).clamp(54.0, 130.0);
                            let (g_rect, _) =
                                ui.allocate_exact_size(Vec2::new(g_w, 24.0), Sense::hover());
                            ui.painter().rect_filled(g_rect, Rounding::same(6.0), accent.soft_bg());
                            ui.painter().rect_stroke(
                                g_rect,
                                Rounding::same(6.0),
                                Stroke::new(1.0_f32, ac),
                            );
                            ui.painter().text(
                                g_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                &clip.game,
                                FontId::proportional(11.5),
                                ac,
                            );

                            if state.rename_editing {
                                let resp = ui.add(
                                    TextEdit::singleline(&mut state.rename_buf)
                                        .desired_width(270.0)
                                        .font(FontId::proportional(14.5)),
                                );
                                if state.editor_rename_just_opened {
                                    resp.request_focus();
                                    state.editor_rename_just_opened = false;
                                }
                                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                                let esc = ui.input(|i| i.key_pressed(Key::Escape));
                                let save_fg = contrasting_fg(ac);
                                let save_clicked = modal_action_pill(
                                    ui,
                                    UiIcon::Check,
                                    "Save",
                                    save_fg,
                                    ac,
                                    ac,
                                    save_fg,
                                );
                                if esc {
                                    state.rename_editing = false;
                                } else if save_clicked || (enter && resp.lost_focus()) || enter {
                                    state.rename_editing = false;
                                    if !state.rename_buf.trim().is_empty() {
                                        let new_name = state.rename_buf.trim().to_owned();
                                        state.send_sync(
                                            IpcRequest::RenameClip {
                                                id: clip.id,
                                                title: new_name,
                                            },
                                            now_secs,
                                        );
                                    }
                                }
                            } else {
                                let title_resp = ui.add(
                                    egui::Label::new(
                                        RichText::new(&clip.title)
                                            .size(15.0)
                                            .strong()
                                            .color(TEXT_PRIMARY),
                                    )
                                    .sense(Sense::click()),
                                );
                                if title_resp
                                    .on_hover_text("Click to rename")
                                    .on_hover_cursor(CursorIcon::Text)
                                    .clicked()
                                {
                                    state.rename_buf = clip.title.clone();
                                    state.rename_editing = true;
                                    state.editor_rename_just_opened = true;
                                }
                                if modal_action_pill(
                                    ui,
                                    UiIcon::Edit,
                                    "Edit Name",
                                    TEXT_PRIMARY,
                                    pill_bg(),
                                    border_col(),
                                    TEXT_MUTED,
                                ) {
                                    state.rename_buf = clip.title.clone();
                                    state.rename_editing = true;
                                    state.editor_rename_just_opened = true;
                                }
                            }

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let (cr, cresp) =
                                    ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                                ui.painter().rect_filled(
                                    cr,
                                    Rounding::same(7.0),
                                    if cresp.hovered() { card_hover() } else { pill_bg() },
                                );
                                ui.painter().rect_stroke(
                                    cr,
                                    Rounding::same(7.0),
                                    Stroke::new(1.0_f32, border_col()),
                                );
                                paint_icon(ui.painter(), cr.shrink(8.0), UiIcon::Close, TEXT_MUTED);
                                if cresp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    should_close = true;
                                }
                            });
                        });
                    });

                    ui.painter().line_segment(
                        [
                            Pos2::new(modal_rect.left(), header_r.bottom()),
                            Pos2::new(modal_rect.right(), header_r.bottom()),
                        ],
                        Stroke::new(1.0_f32, border_col()),
                    );

                    let body_rect = Rect::from_min_max(
                        Pos2::new(modal_rect.left() + 18.0, header_r.bottom() + 14.0),
                        Pos2::new(modal_rect.right() - 18.0, modal_rect.bottom() - 14.0),
                    );
                    let content_w = body_rect.width().max(320.0);
                    let video_h = (modal_h - 264.0).clamp(210.0, 340.0);

                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(content_w);
                            ui.spacing_mut().item_spacing.y = 12.0;

                            let (video_rect, _) = ui.allocate_exact_size(
                                Vec2::new(content_w, video_h),
                                Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                video_rect,
                                Rounding::same(12.0),
                                Color32::from_rgb(7, 9, 14),
                            );
                            if let Some(tex) =
                                state.player_texture.as_ref().or(modal_thumb.as_ref())
                            {
                                let sz = tex.size();
                                let vid_ar = if sz[1] > 0 {
                                    sz[0] as f32 / sz[1] as f32
                                } else {
                                    16.0 / 9.0
                                };
                                let box_ar = video_rect.width() / video_rect.height().max(1.0);
                                let fit_sz = if box_ar > vid_ar {
                                    Vec2::new(video_rect.height() * vid_ar, video_rect.height())
                                } else {
                                    Vec2::new(video_rect.width(), video_rect.width() / vid_ar)
                                };
                                let fit_rect = Rect::from_center_size(video_rect.center(), fit_sz);
                                paint_rounded_image(
                                    ui.painter(),
                                    tex.id(),
                                    fit_rect,
                                    Rounding::same(10.0),
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                paint_empty_thumb_surface(
                                    ui.painter(),
                                    video_rect,
                                    Rounding::same(12.0),
                                );
                            }
                            ui.painter().rect_stroke(
                                video_rect,
                                Rounding::same(12.0),
                                Stroke::new(1.0_f32, border_col()),
                            );

                            if !preload_ready {
                                paint_preload_overlay(ui.painter(), video_rect, preload_pct, ac);
                            }

                            render_player_controls_overlay(ui, state, video_rect, video_rect, dur, ac);

                            let badge_txt = format!(
                                "{}  •  {} {}fps",
                                clip.game.to_ascii_uppercase(),
                                clip.resolution_badge(),
                                clip.fps
                            );
                            let bw = (badge_txt.len() as f32 * 6.5 + 22.0).clamp(120.0, 280.0);
                            let br = Rect::from_min_size(
                                video_rect.left_top() + Vec2::new(14.0, 14.0),
                                Vec2::new(bw, 26.0),
                            );
                            ui.painter().rect_filled(
                                br,
                                Rounding::same(6.0),
                                Color32::from_rgba_unmultiplied(8, 10, 15, 210),
                            );
                            ui.painter().rect_stroke(
                                br,
                                Rounding::same(6.0),
                                Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 26),
                                ),
                            );
                            ui.painter().text(
                                br.center(),
                                egui::Align2::CENTER_CENTER,
                                badge_txt,
                                FontId::monospace(10.5),
                                TEXT_PRIMARY,
                            );

                            egui::Frame::none()
                                .fill(Color32::from_rgba_unmultiplied(20, 25, 35, 220))
                                .stroke(Stroke::new(1.0_f32, border_col()))
                                .rounding(Rounding::same(11.0))
                                .inner_margin(Margin::symmetric(14.0, 10.0))
                                .show(ui, |ui| {
                                    ui.set_width((content_w - 28.0).max(240.0));
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 8.0;

                                        ui.horizontal(|ui| {
                                            let (sc_r, _) = ui.allocate_exact_size(
                                                Vec2::splat(16.0),
                                                Sense::hover(),
                                            );
                                            paint_icon(ui.painter(), sc_r, UiIcon::Scissors, ac);
                                            ui.label(
                                                RichText::new("Quick Trim")
                                                    .size(12.5)
                                                    .strong()
                                                    .color(TEXT_PRIMARY),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "•  {} - {}",
                                                    format_m_ss(state.trim_start_secs),
                                                    format_m_ss(state.trim_end_secs)
                                                ))
                                                .size(12.0)
                                                .color(TEXT_MUTED),
                                            );

                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    let span_s = (state.trim_end_secs
                                                        - state.trim_start_secs)
                                                        .max(0.1);
                                                    let span_lbl = format!("{span_s:.1}s");
                                                    let bw =
                                                        (span_lbl.len() as f32 * 6.8 + 16.0)
                                                            .max(46.0);
                                                    let (db_r, _) = ui.allocate_exact_size(
                                                        Vec2::new(bw, 22.0),
                                                        Sense::hover(),
                                                    );
                                                    ui.painter().rect_filled(
                                                        db_r,
                                                        Rounding::same(6.0),
                                                        accent.soft_bg(),
                                                    );
                                                    ui.painter().rect_stroke(
                                                        db_r,
                                                        Rounding::same(6.0),
                                                        Stroke::new(1.0_f32, ac),
                                                    );
                                                    ui.painter().text(
                                                        db_r.center(),
                                                        egui::Align2::CENTER_CENTER,
                                                        span_lbl,
                                                        FontId::proportional(11.0),
                                                        ac,
                                                    );
                                                },
                                            );
                                        });

                                        let tl_w = ui.available_width().max(200.0);
                                        let (tl_rect, tl_resp) = ui.allocate_exact_size(
                                            Vec2::new(tl_w, 36.0),
                                            Sense::click_and_drag(),
                                        );
                                        let handle_w = 12.0_f32;
                                        let inner_left = tl_rect.left() + handle_w * 0.5;
                                        let inner_right = tl_rect.right() - handle_w * 0.5;
                                        let inner_w = (inner_right - inner_left).max(1.0);

                                        let mut start_x = inner_left
                                            + ((state.trim_start_secs / dur).clamp(0.0, 1.0))
                                                * inner_w;
                                        let mut end_x = inner_left
                                            + ((state.trim_end_secs / dur).clamp(0.0, 1.0))
                                                * inner_w;

                                        let drag_id = Id::new("trim_tl_drag_mode");
                                        if tl_resp.drag_started() {
                                            let mut mode = 3_u8;
                                            if let Some(pos) = tl_resp.interact_pointer_pos() {
                                                let d_s = (pos.x - start_x).abs();
                                                let d_e = (pos.x - end_x).abs();
                                                if d_s <= 16.0 && d_s <= d_e {
                                                    mode = 1;
                                                } else if d_e <= 16.0 {
                                                    mode = 2;
                                                }
                                            }
                                            ctx.data_mut(|d| d.insert_temp(drag_id, mode));
                                        }
                                        if tl_resp.drag_stopped() {
                                            ctx.data_mut(|d| d.insert_temp(drag_id, 0_u8));
                                        }
                                        if tl_resp.dragged() || tl_resp.clicked() {
                                            let mode = if tl_resp.dragged() {
                                                ctx.data_mut(|d| {
                                                    d.get_temp::<u8>(drag_id).unwrap_or(3)
                                                })
                                            } else if let Some(pos) = tl_resp.interact_pointer_pos()
                                            {
                                                let d_s = (pos.x - start_x).abs();
                                                let d_e = (pos.x - end_x).abs();
                                                if d_s <= 16.0 && d_s <= d_e {
                                                    1
                                                } else if d_e <= 16.0 {
                                                    2
                                                } else {
                                                    3
                                                }
                                            } else {
                                                3
                                            };

                                            if let Some(pos) = tl_resp.interact_pointer_pos() {
                                                let frac = ((pos.x - inner_left) / inner_w)
                                                    .clamp(0.0, 1.0);
                                                let t_val = frac * dur;
                                                match mode {
                                                    1 => {
                                                        state.trim_start_secs = t_val
                                                            .min((state.trim_end_secs - 0.2).max(0.0));
                                                    }
                                                    2 => {
                                                        state.trim_end_secs = t_val
                                                            .max((state.trim_start_secs + 0.2).min(dur));
                                                    }
                                                    _ => {
                                                        state.player_pos_secs = t_val;
                                                        if let Some(dec) = &state.decoder {
                                                            dec.seek(t_val);
                                                        }
                                                    }
                                                }
                                            }
                                            start_x = inner_left
                                                + ((state.trim_start_secs / dur).clamp(0.0, 1.0))
                                                    * inner_w;
                                            end_x = inner_left
                                                + ((state.trim_end_secs / dur).clamp(0.0, 1.0))
                                                    * inner_w;
                                        }

                                        ui.painter().rect_filled(
                                            tl_rect,
                                            Rounding::same(7.0),
                                            Color32::from_rgba_unmultiplied(12, 15, 22, 235),
                                        );
                                        ui.painter().rect_stroke(
                                            tl_rect,
                                            Rounding::same(7.0),
                                            Stroke::new(1.0_f32, border_col()),
                                        );

                                        let sel_rect = Rect::from_min_max(
                                            Pos2::new(start_x, tl_rect.top() + 1.0),
                                            Pos2::new(end_x, tl_rect.bottom() - 1.0),
                                        );
                                        ui.painter().rect_filled(
                                            sel_rect,
                                            Rounding::ZERO,
                                            Color32::from_rgba_unmultiplied(
                                                ac.r(),
                                                ac.g(),
                                                ac.b(),
                                                48,
                                            ),
                                        );
                                        ui.painter().line_segment(
                                            [sel_rect.left_top(), sel_rect.right_top()],
                                            Stroke::new(1.5_f32, ac),
                                        );
                                        ui.painter().line_segment(
                                            [sel_rect.left_bottom(), sel_rect.right_bottom()],
                                            Stroke::new(1.5_f32, ac),
                                        );

                                        for t_idx in 1..=7 {
                                            let tx = inner_left
                                                + (t_idx as f32 / 8.0) * inner_w;
                                            ui.painter().line_segment(
                                                [
                                                    Pos2::new(tx, tl_rect.center().y - 5.0),
                                                    Pos2::new(tx, tl_rect.center().y + 5.0),
                                                ],
                                                Stroke::new(
                                                    1.0_f32,
                                                    Color32::from_rgba_unmultiplied(
                                                        255, 255, 255, 40,
                                                    ),
                                                ),
                                            );
                                        }

                                        let playhead_x = inner_left
                                            + ((state.player_pos_secs / dur).clamp(0.0, 1.0))
                                                * inner_w;
                                        let ph_r = Rect::from_center_size(
                                            Pos2::new(playhead_x, tl_rect.center().y),
                                            Vec2::new(2.0, 26.0),
                                        );
                                        ui.painter().rect_filled(
                                            ph_r,
                                            Rounding::same(1.0),
                                            Color32::WHITE,
                                        );

                                        let grip_col = contrasting_fg(ac);
                                        for hx in [start_x, end_x] {
                                            let h_rect = Rect::from_center_size(
                                                Pos2::new(hx, tl_rect.center().y),
                                                Vec2::new(handle_w, 36.0),
                                            );
                                            ui.painter().rect_filled(
                                                h_rect,
                                                Rounding::same(5.0),
                                                ac,
                                            );
                                            ui.painter().line_segment(
                                                [
                                                    Pos2::new(hx, h_rect.top() + 10.0),
                                                    Pos2::new(hx, h_rect.bottom() - 10.0),
                                                ],
                                                Stroke::new(1.6_f32, grip_col),
                                            );
                                        }
                                        let _ = tl_resp
                                            .on_hover_cursor(CursorIcon::ResizeHorizontal);
                                    });
                                });

                            ui.label(
                                RichText::new(format!(
                                    "{} {}fps   •   {}   •   {}",
                                    clip.resolution_badge(),
                                    clip.fps,
                                    format_file_size(clip.size_bytes),
                                    clip.relative_time(now_unix)
                                ))
                                .size(11.5)
                                .color(TEXT_MUTED),
                            );

                            let (div_r, _) =
                                ui.allocate_exact_size(Vec2::new(content_w, 1.0), Sense::hover());
                            ui.painter().line_segment(
                                [div_r.left_center(), div_r.right_center()],
                                Stroke::new(1.0_f32, border_col()),
                            );

                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;

                                let fav_lbl = if clip.favorite { "Favorited" } else { "Favorite" };
                                if modal_action_pill(
                                    ui,
                                    if clip.favorite {
                                        UiIcon::StarFilled
                                    } else {
                                        UiIcon::StarOutline
                                    },
                                    fav_lbl,
                                    if clip.favorite { STAR_GOLD } else { TEXT_PRIMARY },
                                    if clip.favorite {
                                        Color32::from_rgba_unmultiplied(250, 204, 21, 24)
                                    } else {
                                        pill_bg()
                                    },
                                    if clip.favorite { STAR_GOLD } else { border_col() },
                                    if clip.favorite { STAR_GOLD } else { TEXT_MUTED },
                                ) {
                                    state.send_sync(
                                        IpcRequest::ToggleFavorite { id: clip.id },
                                        now_secs,
                                    );
                                }

                                if modal_action_pill(
                                    ui,
                                    UiIcon::Link,
                                    "Copy Link",
                                    TEXT_PRIMARY,
                                    pill_bg(),
                                    border_col(),
                                    TEXT_MUTED,
                                ) {
                                    ctx.copy_text(clip.file_path.clone());
                                    state.status_banner =
                                        Some(("Copied clip link".to_owned(), now_secs));
                                }

                                let enc_busy = state.discord_encode_progress.is_some();
                                let sel_dur = if state.trim_start_secs > 0.05
                                    || (dur - state.trim_end_secs) > 0.05
                                {
                                    (state.trim_end_secs - state.trim_start_secs).max(0.0)
                                } else {
                                    dur
                                };
                                let over_60s = sel_dur > 60.5;
                                let disc_lbl = if enc_busy { "Encoding..." } else { "Discord 10MB" };
                                let disc_fg = if enc_busy {
                                    contrasting_fg(ac)
                                } else if over_60s {
                                    TEXT_DIM
                                } else {
                                    TEXT_PRIMARY
                                };
                                let disc_fill = if enc_busy {
                                    ac
                                } else if over_60s {
                                    Color32::from_rgba_unmultiplied(24, 28, 38, 175)
                                } else {
                                    pill_bg()
                                };
                                let disc_stroke = if enc_busy { ac } else { border_col() };
                                let disc_icon = if enc_busy {
                                    contrasting_fg(ac)
                                } else if over_60s {
                                    TEXT_DIM
                                } else {
                                    ac
                                };
                                if modal_action_pill_ex(
                                    ui,
                                    UiIcon::Codec,
                                    disc_lbl,
                                    disc_fg,
                                    disc_fill,
                                    disc_stroke,
                                    disc_icon,
                                    !enc_busy && !over_60s,
                                    if over_60s {
                                        Some("Only clips up to 60s can be encoded for Discord (<10 MB) because quality would be bad. Trim to 60s or less first.")
                                    } else {
                                        None
                                    },
                                ) {
                                    let trim = if state.trim_start_secs > 0.05
                                        || (dur - state.trim_end_secs) > 0.05
                                    {
                                        Some((state.trim_start_secs, state.trim_end_secs))
                                    } else {
                                        None
                                    };
                                    state.start_discord_encode(&clip, trim, now_secs);
                                }

                                let save_fg = contrasting_fg(ac);
                                if modal_action_pill(
                                    ui,
                                    UiIcon::Check,
                                    "Save Trim",
                                    save_fg,
                                    ac,
                                    ac,
                                    save_fg,
                                ) {
                                    state.send_sync(
                                        IpcRequest::TrimClip {
                                            id: clip.id,
                                            start_secs: state.trim_start_secs,
                                            end_secs: state.trim_end_secs,
                                            vertical_9_16: false,
                                        },
                                        now_secs,
                                    );
                                }

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let del_col = Color32::from_rgb(244, 63, 94);
                                    if modal_action_pill(
                                        ui,
                                        UiIcon::Trash,
                                        "Delete",
                                        del_col,
                                        Color32::from_rgba_unmultiplied(244, 63, 94, 18),
                                        Color32::from_rgba_unmultiplied(244, 63, 94, 80),
                                        del_col,
                                    ) {
                                        should_close = true;
                                        state.send_sync(
                                            IpcRequest::DeleteClip { id: clip.id },
                                            now_secs,
                                        );
                                    }
                                });
                            });
                        });
                    });
                });
            });

            let can_backdrop_close = (now_secs - state.modal_opened_at) > 0.20;
            let clicked_outside = can_backdrop_close
                && ctx.input(|i| {
                    i.pointer.primary_clicked()
                        && i.pointer
                            .interact_pos()
                            .map(|p| {
                                !modal_rect.contains(p)
                                    && !prev_rect.contains(p)
                                    && !next_rect.contains(p)
                            })
                            .unwrap_or(false)
                });
            if clicked_outside {
                should_close = true;
            }
        });

    if should_close {
        state.close_clip_modal();
    }
}

fn render_mandatory_update_modal(ctx: &egui::Context, state: &mut RewindUiState) {
    let (info, is_downloading, err_msg) = match &state.update_status {
        UpdateStatus::Available(info) => (info.clone(), false, None),
        UpdateStatus::Downloading(info) => (info.clone(), true, None),
        UpdateStatus::Error(info, err) => (info.clone(), false, Some(err.clone())),
        _ => return,
    };

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let ac = accent.color();
    let screen = ctx.screen_rect();
    let modal_w = 480.0_f32.min(screen.width() - 48.0);
    let modal_h = 316.0_f32;
    let modal_rect = Rect::from_center_size(screen.center(), Vec2::new(modal_w, modal_h));

    egui::Area::new(Id::new("mandatory_update_overlay"))
        .order(Order::Foreground)
        .fixed_pos(screen.left_top())
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            let _ = ui.interact(screen, Id::new("mandatory_update_backdrop"), Sense::click());
            ui.painter().rect_filled(
                screen,
                Rounding::ZERO,
                Color32::from_rgba_unmultiplied(6, 8, 13, 210),
            );

            ui.painter().rect_filled(
                modal_rect,
                Rounding::same(14.0),
                Color32::from_rgba_unmultiplied(15, 19, 27, 250),
            );
            ui.painter().rect_stroke(
                modal_rect,
                Rounding::same(14.0),
                Stroke::new(1.2_f32, ac),
            );

            ui.allocate_new_ui(
                egui::UiBuilder::new().max_rect(modal_rect.shrink2(Vec2::new(24.0, 22.0))),
                |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let (ic_r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                            if let Some(tex) = &state.app_icon_texture {
                                ui.painter().image(
                                    tex.id(),
                                    ic_r,
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            }
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!("Update Required  •  {}", info.tag_name))
                                    .size(17.0)
                                    .strong()
                                    .color(TEXT_PRIMARY),
                            );
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "A newer Rewind release ({}) is available on GitHub (installed: v{CURRENT_VERSION}). Update to continue.",
                                info.tag_name
                            ))
                            .size(12.5)
                            .color(TEXT_MUTED),
                        );
                        ui.add_space(12.0);

                        egui::Frame::none()
                            .fill(Color32::from_rgba_unmultiplied(22, 28, 39, 220))
                            .stroke(Stroke::new(1.0_f32, border_col()))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::symmetric(12.0, 10.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(74.0);
                                let preview = if info.notes.trim().is_empty() {
                                    "Includes latest capture, audio/video sync, and stability fixes."
                                } else {
                                    info.notes.trim()
                                };
                                ui.label(
                                    RichText::new(truncate_text(preview, 220))
                                        .size(11.5)
                                        .color(TEXT_PRIMARY),
                                );
                            });

                        if let Some(err) = err_msg {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(err)
                                    .size(11.5)
                                    .color(Color32::from_rgb(244, 63, 94)),
                            );
                        }

                        ui.add_space(10.0);

                        ui.label(
                            RichText::new(
                                "Note: Windows SmartScreen / Smart App Control warnings are a false positive because releases are not yet signed with a paid certificate.",
                            )
                            .size(11.0)
                            .color(TEXT_DIM),
                        );

                        ui.add_space(12.0);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let upd_lbl = if is_downloading {
                                "Downloading Installer..."
                            } else {
                                "Update Now"
                            };
                            let save_fg = contrasting_fg(ac);
                            if modal_action_pill(
                                ui,
                                UiIcon::Check,
                                upd_lbl,
                                save_fg,
                                ac,
                                ac,
                                save_fg,
                            ) && !is_downloading
                            {
                                state.start_update_install(info.clone());
                            }

                            if modal_action_pill(
                                ui,
                                UiIcon::Link,
                                "Open Releases Page",
                                TEXT_PRIMARY,
                                pill_bg(),
                                border_col(),
                                TEXT_MUTED,
                            ) {
                                open_url(&info.html_url);
                            }
                        });
                    });
                },
            );
        });
}

fn modal_action_pill(
    ui: &mut Ui,
    icon: UiIcon,
    label: &str,
    fg: Color32,
    fill: Color32,
    stroke: Color32,
    icon_col: Color32,
) -> bool {
    modal_action_pill_ex(ui, icon, label, fg, fill, stroke, icon_col, true, None)
}

fn modal_action_pill_ex(
    ui: &mut Ui,
    icon: UiIcon,
    label: &str,
    fg: Color32,
    fill: Color32,
    stroke: Color32,
    icon_col: Color32,
    enabled: bool,
    disabled_tooltip: Option<&str>,
) -> bool {
    let w = (label.len() as f32 * 6.8 + 42.0).max(86.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), sense);
    let draw_fill = if enabled && resp.hovered() && fill == pill_bg() {
        card_hover()
    } else {
        fill
    };
    ui.painter().rect_filled(rect, Rounding::same(10.0), draw_fill);
    ui.painter()
        .rect_stroke(rect, Rounding::same(10.0), Stroke::new(1.0_f32, stroke));
    let ic_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 17.0, rect.center().y),
        Vec2::splat(13.0),
    );
    paint_icon(ui.painter(), ic_rect, icon, icon_col);
    ui.painter().text(
        Pos2::new(ic_rect.right() + 7.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        fg,
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    } else {
        if let Some(tip) = disabled_tooltip {
            resp.on_hover_text(tip);
        }
        false
    }
}

fn nav_chevron_button_at(ui: &mut Ui, rect: Rect, icon: UiIcon, enabled: bool) -> bool {
    let resp = ui.interact(
        rect,
        Id::new(("modal_nav", icon as u8)),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let bg = if enabled && resp.hovered() {
        card_hover()
    } else {
        Color32::from_rgba_unmultiplied(20, 25, 36, 220)
    };
    ui.painter().circle_filled(rect.center(), 17.0, bg);
    ui.painter()
        .circle_stroke(rect.center(), 17.0, Stroke::new(1.0_f32, border_col()));
    paint_icon(
        ui.painter(),
        rect.shrink(10.0),
        icon,
        if enabled { TEXT_PRIMARY } else { TEXT_DIM },
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    } else {
        false
    }
}

fn render_undo_or_status_toast(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) {
    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let undo_id = state.snapshot.pending_undo_clip_id;
    let enc_pct = state
        .discord_encode_progress
        .as_ref()
        .map(|p| p.load(std::sync::atomic::Ordering::Acquire).min(100));

    let msg_opt = if let Some(pct) = enc_pct {
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
        Some(format!("Encoding for Discord (<10 MB)... {pct}%"))
    } else if let Some((msg, t)) = &state.status_banner {
        if now_secs - *t <= 3.5 || undo_id.is_some() {
            Some(msg.clone())
        } else {
            state.status_banner = None;
            None
        }
    } else if undo_id.is_some() {
        Some("Clip moved to trash".to_owned())
    } else {
        None
    };

    let Some(msg) = msg_opt else {
        return;
    };

    let screen = ctx.screen_rect();
    let toast_t = anim_open_progress(
        ctx,
        Id::new("status_toast_anim"),
        if enc_pct.is_some() { 1 } else { 2 },
        now_secs,
        0.18,
    );
    egui::Area::new(Id::new("status_toast"))
        .order(Order::Tooltip)
        .fixed_pos(Pos2::new(
            (screen.center().x - 190.0).max(16.0),
            screen.bottom() - 58.0 + (1.0 - toast_t) * 14.0,
        ))
        .show(ctx, |ui| {
            ui.set_opacity(toast_t);
            egui::Frame::none()
                .fill(popover_bg())
                .stroke(Stroke::new(1.0_f32, accent.color()))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(14.0, 8.0))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(msg).size(12.0).color(TEXT_PRIMARY));
                            if let Some(id) = undo_id {
                                ui.add_space(10.0);
                                if ui
                                    .button(RichText::new("Undo").size(12.0).color(accent.color()))
                                    .clicked()
                                {
                                    state.send_sync(IpcRequest::UndoDelete { id }, now_secs);
                                }
                            }
                        });
                        if let Some(pct) = enc_pct {
                            ui.add_space(5.0);
                            let (bar_r, _) =
                                ui.allocate_exact_size(Vec2::new(250.0, 4.0), Sense::hover());
                            ui.painter().rect_filled(bar_r, Rounding::same(2.0), pill_bg());
                            let fill_w = (bar_r.width() * (pct as f32 / 100.0)).max(4.0);
                            let fill_r =
                                Rect::from_min_size(bar_r.left_top(), Vec2::new(fill_w, 4.0));
                            ui.painter()
                                .rect_filled(fill_r, Rounding::same(2.0), accent.color());
                        }
                    });
                });
        });
}


fn split_clip_pill(
    ui: &mut Ui,
    label: &str,
    keycap: &str,
    popup_open: bool,
    accent_color: Color32,
    flash_alpha: f32,
) -> (bool, bool, Rect) {
    let main_w = keycap.len() as f32 * 6.4 + label.len() as f32 * 6.5 + 32.0;
    let total_w = (main_w + 24.0).max(124.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total_w, 28.0), Sense::hover());

    let main_r = Rect::from_min_max(rect.left_top(), Pos2::new(rect.right() - 24.0, rect.bottom()));
    let chev_r = Rect::from_min_max(Pos2::new(rect.right() - 24.0, rect.top()), rect.right_bottom());

    let main_resp = ui.interact(main_r, Id::new("split_save_btn"), Sense::click());
    let chev_resp = ui.interact(chev_r, Id::new("split_len_btn"), Sense::click());

    let base_bg = if popup_open || main_resp.hovered() || chev_resp.hovered() {
        card_hover()
    } else {
        pill_bg()
    };
    let base_border = if popup_open { border_bright() } else { border_col() };
    let bg = lerp_color(base_bg, accent_color, flash_alpha * 0.28);
    let border = lerp_color(base_border, accent_color, flash_alpha * 0.92);

    if flash_alpha > 0.01 {
        ui.painter().rect_stroke(
            rect.expand(1.5),
            Rounding::same(8.5),
            Stroke::new(
                2.0_f32,
                Color32::from_rgba_unmultiplied(
                    accent_color.r(),
                    accent_color.g(),
                    accent_color.b(),
                    (flash_alpha * 95.0) as u8,
                ),
            ),
        );
    }
    ui.painter().rect_filled(rect, Rounding::same(7.0), bg);
    ui.painter()
        .rect_stroke(rect, Rounding::same(7.0), Stroke::new(1.0_f32, border));
    ui.painter().line_segment(
        [
            Pos2::new(chev_r.left(), rect.top() + 5.0),
            Pos2::new(chev_r.left(), rect.bottom() - 5.0),
        ],
        Stroke::new(1.0_f32, border_col()),
    );

    let kw = keycap.len() as f32 * 6.2 + 12.0;
    let k_rect = Rect::from_min_size(
        Pos2::new(rect.left() + 8.0, rect.center().y - 9.0),
        Vec2::new(kw, 18.0),
    );
    ui.painter()
        .rect_filled(k_rect, Rounding::same(4.0), Color32::from_rgb(35, 42, 56));
    ui.painter().rect_stroke(
        k_rect,
        Rounding::same(4.0),
        Stroke::new(1.0_f32, Color32::from_rgb(56, 66, 86)),
    );
    ui.painter().text(
        k_rect.center(),
        egui::Align2::CENTER_CENTER,
        keycap,
        FontId::proportional(10.0),
        TEXT_PRIMARY,
    );
    ui.painter().text(
        Pos2::new(k_rect.right() + 7.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        TEXT_PRIMARY,
    );

    paint_icon(
        ui.painter(),
        Rect::from_center_size(chev_r.center(), Vec2::splat(10.0)),
        UiIcon::ChevronDown,
        TEXT_MUTED,
    );

    (
        main_resp.on_hover_cursor(CursorIcon::PointingHand).clicked(),
        chev_resp.on_hover_cursor(CursorIcon::PointingHand).clicked(),
        rect,
    )
}

fn dropdown_pill(
    ui: &mut Ui,
    label: &str,
    dot_color: Option<Color32>,
    keycap: Option<&str>,
    active: bool,
    min_w: f32,
) -> Response {
    let text_w = label.len() as f32 * 6.4;
    let keycap_w = keycap.map(|k| k.len() as f32 * 6.4 + 18.0).unwrap_or(0.0);
    let dot_w = if dot_color.is_some() { 14.0 } else { 0.0 };
    let w = (text_w + keycap_w + dot_w + 34.0).max(min_w);

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
    let bg = if active || resp.hovered() {
        card_hover()
    } else {
        pill_bg()
    };
    let border = if active { border_bright() } else { border_col() };

    ui.painter().rect_filled(rect, Rounding::same(7.0), bg);
    ui.painter()
        .rect_stroke(rect, Rounding::same(7.0), Stroke::new(1.0_f32, border));

    let mut x = rect.left() + 10.0;
    if let Some(dot) = dot_color {
        ui.painter()
            .circle_filled(Pos2::new(x + 3.0, rect.center().y), 3.3, dot);
        x += 12.0;
    }

    ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        TEXT_PRIMARY,
    );

    let chev_r =
        Rect::from_center_size(Pos2::new(rect.right() - 12.0, rect.center().y), Vec2::splat(10.0));
    paint_icon(ui.painter(), chev_r, UiIcon::ChevronDown, TEXT_MUTED);

    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn icon_button(ui: &mut Ui, size: Vec2, icon: UiIcon, color: Color32, bg: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let draw_bg = if resp.hovered() { card_hover() } else { bg };
    if draw_bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, Rounding::same(6.0), draw_bg);
    }
    paint_icon(ui.painter(), rect.shrink(6.0), icon, color);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn window_ctrl_button(ui: &mut Ui, icon: UiIcon, is_close: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 26.0), Sense::click());
    if resp.hovered() {
        let hover_bg = if is_close {
            Color32::from_rgb(196, 43, 28)
        } else {
            card_hover()
        };
        ui.painter().rect_filled(rect, Rounding::same(5.0), hover_bg);
    }
    let fg = if resp.hovered() { TEXT_PRIMARY } else { TEXT_MUTED };
    paint_icon(ui.painter(), rect.shrink(7.0), icon, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn avatar_button(ui: &mut Ui, initials: &str, active: bool, accent: ThemeAccent) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let fill = if active || resp.hovered() {
        accent.soft_bg()
    } else {
        Color32::from_rgb(24, 34, 52)
    };
    let border = if active {
        accent.color()
    } else {
        Color32::from_rgb(48, 66, 98)
    };
    ui.painter().circle_filled(rect.center(), 12.5, fill);
    ui.painter()
        .circle_stroke(rect.center(), 12.5, Stroke::new(1.1_f32, border));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        FontId::proportional(10.5),
        accent.color(),
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn settings_section_header(ui: &mut Ui, title: &str, right_hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(13.0).strong().color(TEXT_PRIMARY));
        if let Some(hint) = right_hint {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(hint).size(11.0).color(TEXT_DIM));
            });
        }
    });
    ui.add_space(8.0);
}

fn settings_card(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::none()
        .fill(card_bg())
        .stroke(Stroke::new(1.0_f32, border_col()))
        .rounding(Rounding::same(11.0))
        .inner_margin(Margin::symmetric(16.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
}

fn settings_divider(ui: &mut Ui) {
    ui.add_space(8.0);
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), 1.0));
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0_f32, divider_col()),
    );
    ui.add_space(8.0);
}

fn settings_row(
    ui: &mut Ui,
    icon: UiIcon,
    icon_color: Color32,
    title: &str,
    subtitle: &str,
    right_ctrl: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        let (icon_box, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        ui.painter()
            .rect_filled(icon_box, Rounding::same(8.0), icon_box_bg());
        paint_icon(ui.painter(), icon_box.shrink(8.0), icon, icon_color);

        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).size(13.0).strong().color(TEXT_PRIMARY));
            ui.label(RichText::new(subtitle).size(11.5).color(TEXT_MUTED));
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            right_ctrl(ui);
        });
    });
}

fn volume_row(ui: &mut Ui, id_salt: &str, vol: &mut u32, accent: Color32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(44.0);
        ui.label(RichText::new("Volume").size(11.0).color(TEXT_MUTED));
        ui.add_space(8.0);
        let slider_w = (ui.available_width() - 48.0).max(120.0);
        ui.allocate_ui(Vec2::new(slider_w, 18.0), |ui| {
            let mut v = *vol as f32;
            if custom_slider(ui, id_salt, &mut v, 0.0, 100.0, accent) {
                *vol = v.round() as u32;
                changed = true;
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("{}%", *vol)).size(11.5).color(TEXT_PRIMARY));
        });
    });
    changed
}

fn hotkey_row(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    keycap: &str,
    rebinding: bool,
    accent: Color32,
    on_click: impl FnOnce(),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).size(13.0).strong().color(TEXT_PRIMARY));
            ui.label(RichText::new(subtitle).size(11.5).color(TEXT_MUTED));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if rebinding { "Press key..." } else { keycap };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(98.0, 30.0), Sense::click());
            let bg = if rebinding {
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 35)
            } else if resp.hovered() {
                card_hover()
            } else {
                pill_bg()
            };
            let border = if rebinding {
                accent
            } else {
                border_bright()
            };
            ui.painter().rect_filled(rect, Rounding::same(6.0), bg);
            ui.painter()
                .rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0_f32, border));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.5),
                if rebinding { accent } else { TEXT_PRIMARY },
            );
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                on_click();
            }
        });
    });
}

fn toggle_switch(ui: &mut Ui, id_salt: &str, on: &mut bool, accent: Color32) -> bool {
    ui.push_id(id_salt, |ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(38.0, 20.0), Sense::click());
        let bg = if *on { accent } else { track_bg() };
        ui.painter().rect_filled(rect, Rounding::same(10.0), bg);
        let knob_x = if *on {
            rect.right() - 10.0
        } else {
            rect.left() + 10.0
        };
        let knob_col = if *on {
            contrasting_fg(accent)
        } else {
            Color32::WHITE
        };
        ui.painter()
            .circle_filled(Pos2::new(knob_x, rect.center().y), 7.5, knob_col);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            *on = !*on;
            return true;
        }
        false
    })
    .inner
}

fn custom_slider(
    ui: &mut Ui,
    id_salt: &str,
    val: &mut f32,
    min: f32,
    max: f32,
    accent: Color32,
) -> bool {
    ui.push_id(id_salt, |ui| {
        let w = ui.available_width().max(60.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::click_and_drag());
        let track_r = Rect::from_min_max(
            Pos2::new(rect.left() + 6.0, rect.center().y - 2.0),
            Pos2::new(rect.right() - 6.0, rect.center().y + 2.0),
        );
        let mut changed = false;
        let span = (max - min).max(0.0001);
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let t = ((pos.x - track_r.left()) / track_r.width()).clamp(0.0, 1.0);
                *val = min + t * span;
                changed = true;
            }
        }
        let frac = ((*val - min) / span).clamp(0.0, 1.0);
        let thumb_x = track_r.left() + frac * track_r.width();

        ui.painter()
            .rect_filled(track_r, Rounding::same(2.0), track_bg());
        let active_r = Rect::from_min_max(track_r.left_top(), Pos2::new(thumb_x, track_r.bottom()));
        ui.painter().rect_filled(active_r, Rounding::same(2.0), accent);

        ui.painter()
            .circle_filled(Pos2::new(thumb_x, rect.center().y), 6.5, Color32::WHITE);
        ui.painter().circle_stroke(
            Pos2::new(thumb_x, rect.center().y),
            6.5,
            Stroke::new(2.2_f32, accent),
        );

        let _ = resp.on_hover_cursor(CursorIcon::PointingHand);
        changed
    })
    .inner
}

fn popover_section_title(ui: &mut Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(10.0).strong().color(TEXT_DIM));
    ui.add_space(4.0);
}

fn popover_settings_link_btn(ui: &mut Ui, label: &str, accent: ThemeAccent) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let bg = if resp.hovered() {
        accent.color()
    } else {
        accent.soft_bg()
    };
    let fg = if resp.hovered() {
        contrasting_fg(accent.color())
    } else {
        accent.color()
    };
    ui.painter().rect_filled(rect, Rounding::same(7.0), bg);
    ui.painter()
        .rect_stroke(rect, Rounding::same(7.0), Stroke::new(1.0_f32, accent.color()));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.0),
        fg,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn render_exe_list_block(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    hint: &str,
    add_label: &str,
    input: &mut String,
    entries: &[String],
    accent: ThemeAccent,
    add_out: &mut Option<String>,
    remove_out: &mut Option<usize>,
) -> bool {
    let mut interacted = false;
    settings_row(ui, UiIcon::Monitor, accent.color(), title, subtitle, |ui| {
        if modal_action_pill(
            ui,
            UiIcon::Folder,
            "Browse .exe...",
            TEXT_PRIMARY,
            pill_bg(),
            border_col(),
            TEXT_MUTED,
        ) {
            if let Some(chosen) = pick_exe_file_dialog() {
                *add_out = Some(chosen);
                interacted = true;
            }
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(40.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        let txt_resp = ui.add(
            TextEdit::singleline(input)
                .desired_width(240.0)
                .hint_text(hint),
        );
        let enter_pressed = txt_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let add_fg = contrasting_fg(accent.color());
        if (modal_action_pill(
            ui,
            UiIcon::Check,
            add_label,
            add_fg,
            accent.color(),
            accent.color(),
            add_fg,
        ) || enter_pressed)
            && !input.trim().is_empty()
        {
            *add_out = Some(input.trim().to_owned());
            input.clear();
            interacted = true;
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(40.0);
        if entries.is_empty() {
            ui.label(
                RichText::new("None configured")
                    .size(11.0)
                    .color(TEXT_DIM),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                for (idx, item) in entries.iter().enumerate() {
                    let chip_text = format!("{item}  ×");
                    let text_w = chip_text.len() as f32 * 6.8;
                    let w = (text_w + 18.0).clamp(58.0, 260.0);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        Rounding::same(12.0),
                        if resp.hovered() { card_hover() } else { pill_bg() },
                    );
                    ui.painter().rect_stroke(
                        rect,
                        Rounding::same(12.0),
                        Stroke::new(
                            1.0_f32,
                            if resp.hovered() {
                                accent.color()
                            } else {
                                border_col()
                            },
                        ),
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        chip_text,
                        FontId::proportional(11.5),
                        TEXT_PRIMARY,
                    );
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        *remove_out = Some(idx);
                        interacted = true;
                    }
                }
            });
        }
    });
    interacted
}

fn popover_check_item(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    selected: bool,
    accent: ThemeAccent,
) -> bool {
    popover_check_item_enabled(ui, title, subtitle, selected, accent, true)
}

fn popover_check_item_enabled(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    selected: bool,
    accent: ThemeAccent,
    enabled: bool,
) -> bool {
    let h = if subtitle.is_some() { 42.0 } else { 28.0 };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), sense);
    if selected {
        ui.painter().rect_filled(rect, Rounding::same(6.0), accent.soft_bg());
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, Rounding::same(6.0), card_hover());
    }

    let fg = if enabled { TEXT_PRIMARY } else { TEXT_DIM };
    let sub_fg = if enabled { TEXT_MUTED } else { TEXT_DIM };

    if let Some(sub) = subtitle {
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.top() + 13.5),
            egui::Align2::LEFT_CENTER,
            title,
            FontId::proportional(12.0),
            fg,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.top() + 29.0),
            egui::Align2::LEFT_CENTER,
            sub,
            FontId::proportional(10.5),
            sub_fg,
        );
    } else {
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            FontId::proportional(12.0),
            fg,
        );
    }

    if selected {
        let chk =
            Rect::from_center_size(Pos2::new(rect.right() - 14.0, rect.center().y), Vec2::splat(11.0));
        paint_icon(ui.painter(), chk, UiIcon::Check, accent.color());
    }
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    } else {
        false
    }
}

fn popover_simple_item(ui: &mut Ui, title: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, Rounding::same(6.0), card_hover());
    }
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.0),
        TEXT_PRIMARY,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn user_initials() -> String {
    let raw = std::env::var("USERNAME").unwrap_or_else(|_| "U".to_owned());
    let chars: Vec<char> = raw
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if chars.is_empty() {
        "U".to_owned()
    } else {
        chars.into_iter().collect()
    }
}

fn truncate_text(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars || max_chars < 4 {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max_chars - 3).collect();
        out.push_str("...");
        out
    }
}

fn compute_clip_flash_alpha(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) -> f32 {
    let Some(start) = state.clip_flash_start else {
        return 0.0;
    };
    let dt = (now_secs - start) as f32;
    if dt < 0.0 {
        return 0.0;
    }
    if dt >= 1.10 {
        state.clip_flash_start = None;
        return 0.0;
    }
    ctx.request_repaint();
    if dt < 0.18 {
        let u = (dt / 0.18).clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    } else {
        let v = ((dt - 0.18) / 0.92).clamp(0.0, 1.0);
        0.5 * (1.0 + (std::f32::consts::PI * v).cos())
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp_u8 = |x: u8, y: u8| -> u8 {
        ((x as f32) + ((y as f32) - (x as f32)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_unmultiplied(
        lerp_u8(a.r(), b.r()),
        lerp_u8(a.g(), b.g()),
        lerp_u8(a.b(), b.b()),
        lerp_u8(a.a(), b.a()),
    )
}

fn render_player_controls_overlay(
    ui: &mut Ui,
    state: &mut RewindUiState,
    click_rect: Rect,
    ctrl_bounds: Rect,
    dur: f32,
    ac: Color32,
) {
    let vig_top = (ctrl_bounds.bottom() - 88.0).max(ctrl_bounds.top());
    let mut vig = egui::Mesh::default();
    let clear = Color32::from_rgba_unmultiplied(8, 10, 15, 0);
    let dark = Color32::from_rgba_unmultiplied(8, 10, 15, 224);
    vig.colored_vertex(Pos2::new(ctrl_bounds.left(), vig_top), clear);
    vig.colored_vertex(Pos2::new(ctrl_bounds.right(), vig_top), clear);
    vig.colored_vertex(ctrl_bounds.right_bottom(), dark);
    vig.colored_vertex(ctrl_bounds.left_bottom(), dark);
    vig.add_triangle(0, 1, 2);
    vig.add_triangle(0, 2, 3);
    ui.painter().add(vig);

    let play_click_rect = Rect::from_min_max(
        click_rect.left_top(),
        Pos2::new(click_rect.right(), (ctrl_bounds.bottom() - 62.0).max(click_rect.top())),
    );
    let play_click_resp =
        ui.interact(play_click_rect, Id::new("video_surface_click"), Sense::click());
    if play_click_resp.double_clicked() {
        state.modal_fullscreen = !state.modal_fullscreen;
    } else if play_click_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        state.player_playing = !state.player_playing;
        if let Some(dec) = &state.decoder {
            dec.set_playing(state.player_playing);
        }
    }

    let scrub_rect = Rect::from_min_max(
        Pos2::new(ctrl_bounds.left() + 14.0, ctrl_bounds.bottom() - 56.0),
        Pos2::new(ctrl_bounds.right() - 14.0, ctrl_bounds.bottom() - 40.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(scrub_rect), |ui| {
        let mut pos = state.player_pos_secs.clamp(0.0, dur);
        if custom_slider(ui, "player_scrub", &mut pos, 0.0, dur, ac) {
            state.player_pos_secs = pos;
            if let Some(dec) = &state.decoder {
                dec.seek(pos);
            }
        }
    });

    let ctrl_rect = Rect::from_min_max(
        Pos2::new(ctrl_bounds.left() + 14.0, ctrl_bounds.bottom() - 36.0),
        Pos2::new(ctrl_bounds.right() - 14.0, ctrl_bounds.bottom() - 8.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(ctrl_rect), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let (pp_r, pp_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
            ui.painter().rect_filled(
                pp_r,
                Rounding::same(7.0),
                if pp_resp.hovered() {
                    Color32::from_rgba_unmultiplied(46, 54, 72, 230)
                } else {
                    Color32::from_rgba_unmultiplied(28, 34, 46, 220)
                },
            );
            ui.painter().rect_stroke(
                pp_r,
                Rounding::same(7.0),
                Stroke::new(1.0_f32, border_col()),
            );
            paint_icon(
                ui.painter(),
                pp_r.shrink(7.0),
                if state.player_playing {
                    UiIcon::Pause
                } else {
                    UiIcon::Play
                },
                TEXT_PRIMARY,
            );
            if pp_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.player_playing = !state.player_playing;
                if let Some(dec) = &state.decoder {
                    dec.set_playing(state.player_playing);
                }
            }

            let (sp_r, sp_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
            ui.painter().rect_filled(
                sp_r,
                Rounding::same(7.0),
                if sp_resp.hovered() {
                    Color32::from_rgba_unmultiplied(46, 54, 72, 230)
                } else {
                    Color32::from_rgba_unmultiplied(28, 34, 46, 220)
                },
            );
            ui.painter().rect_stroke(
                sp_r,
                Rounding::same(7.0),
                Stroke::new(1.0_f32, border_col()),
            );
            paint_icon(
                ui.painter(),
                sp_r.shrink(7.0),
                UiIcon::Speaker,
                if state.player_muted { TEXT_DIM } else { TEXT_PRIMARY },
            );
            if sp_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.player_muted = !state.player_muted;
            }

            ui.allocate_ui(Vec2::new(82.0, 18.0), |ui| {
                let mut vol = if state.player_muted {
                    0.0
                } else {
                    state.player_volume
                };
                if custom_slider(ui, "player_vol_slider", &mut vol, 0.0, 1.0, ac) {
                    state.player_volume = vol;
                    state.player_muted = vol <= 0.01;
                }
            });

            ui.label(
                RichText::new(format!(
                    "{} / {}",
                    format_m_ss(state.player_pos_secs),
                    format_m_ss(dur)
                ))
                .size(11.5)
                .color(TEXT_PRIMARY),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (fs_r, fs_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                ui.painter().rect_filled(
                    fs_r,
                    Rounding::same(7.0),
                    if fs_resp.hovered() {
                        Color32::from_rgba_unmultiplied(46, 54, 72, 230)
                    } else {
                        Color32::from_rgba_unmultiplied(28, 34, 46, 220)
                    },
                );
                ui.painter().rect_stroke(
                    fs_r,
                    Rounding::same(7.0),
                    Stroke::new(1.0_f32, border_col()),
                );
                paint_icon(
                    ui.painter(),
                    fs_r.shrink(7.0),
                    if state.modal_fullscreen {
                        UiIcon::Minimize
                    } else {
                        UiIcon::Maximize
                    },
                    TEXT_PRIMARY,
                );
                let tip = if state.modal_fullscreen {
                    "Exit Fullscreen (Esc)"
                } else {
                    "Fullscreen (F)"
                };
                if fs_resp
                    .on_hover_text(tip)
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    state.modal_fullscreen = !state.modal_fullscreen;
                }
            });
        });
    });
}

fn render_clip_context_menu(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) {
    let Some((clip_id, raw_pos)) = state.clip_context_menu else {
        return;
    };
    let Some(clip) = state.snapshot.clips.iter().find(|c| c.id == clip_id).cloned() else {
        state.clip_context_menu = None;
        return;
    };

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        state.clip_context_menu = None;
        return;
    }

    let screen = ctx.screen_rect();
    let menu_size = Vec2::new(196.0, 138.0);
    let anim_t = anim_open_progress(
        ctx,
        Id::new("ctx_menu_open_anim"),
        clip_id as u64,
        now_secs,
        0.14,
    );
    let pos = Pos2::new(
        raw_pos.x.clamp(8.0, (screen.right() - menu_size.x - 8.0).max(8.0)),
        raw_pos.y.clamp(8.0, (screen.bottom() - menu_size.y - 8.0).max(8.0)) - (1.0 - anim_t) * 6.0,
    );
    let menu_rect = Rect::from_min_size(pos, menu_size);

    egui::Area::new(Id::new("clip_context_menu"))
        .order(Order::Tooltip)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            ui.set_opacity(anim_t);
            egui::Frame::none()
                .fill(Color32::from_rgb(20, 25, 36))
                .stroke(Stroke::new(1.0_f32, border_bright()))
                .rounding(Rounding::same(14.0))
                .inner_margin(Margin::same(6.0))
                .shadow(Shadow {
                    offset: Vec2::new(0.0, 10.0),
                    blur: 24.0,
                    spread: 2.0,
                    color: Color32::from_rgba_unmultiplied(0, 0, 0, (150.0 * anim_t) as u8),
                })
                .show(ui, |ui| {
                    ui.set_min_size(menu_size - Vec2::splat(14.0));
                    ui.set_max_width(menu_size.x - 12.0);
                    ui.spacing_mut().item_spacing.y = 4.0;

                    if context_menu_item(ui, UiIcon::Edit, "Edit Clip Name", TEXT_PRIMARY) {
                        state.rename_modal_clip = Some((clip_id, clip.title.clone()));
                        state.rename_modal_just_opened = true;
                        state.clip_context_menu = None;
                    }

                    if context_menu_item(ui, UiIcon::Link, "Share Clip", TEXT_PRIMARY) {
                        let copied = rewind::win_ffi::copy_file_to_clipboard(&clip.file_path);
                        state.status_banner = Some((
                            if copied {
                                "Clip copied — press Ctrl+V to share".to_owned()
                            } else {
                                "Failed to copy clip to clipboard".to_owned()
                            },
                            now_secs,
                        ));
                        state.clip_context_menu = None;
                    }

                    let over_60s = clip.duration_secs > 60.5;
                    if context_menu_item_ex(
                        ui,
                        UiIcon::Codec,
                        "Discord (<10 MB)",
                        if over_60s { TEXT_DIM } else { TEXT_PRIMARY },
                        !over_60s,
                        if over_60s {
                            Some("Only clips up to 60s can be encoded for Discord (<10 MB) because longer clips would have bad quality. Open the clip and trim to 60s or less.")
                        } else {
                            None
                        },
                    ) {
                        state.start_discord_encode(&clip, None, now_secs);
                        state.clip_context_menu = None;
                    }

                    let del_col = Color32::from_rgb(244, 63, 94);
                    if context_menu_item(ui, UiIcon::Trash, "Delete", del_col) {
                        state.clip_context_menu = None;
                        state.send_sync(IpcRequest::DeleteClip { id: clip_id }, now_secs);
                    }
                });
        });

    let clicked_outside = ctx.input(|i| {
        (i.pointer.primary_clicked() || i.pointer.secondary_clicked())
            && i.pointer
                .interact_pos()
                .map(|p| !menu_rect.contains(p))
                .unwrap_or(true)
    });
    if clicked_outside {
        state.clip_context_menu = None;
    }
}

fn context_menu_item(ui: &mut Ui, icon: UiIcon, label: &str, color: Color32) -> bool {
    context_menu_item_ex(ui, icon, label, color, true, None)
}

fn context_menu_item_ex(
    ui: &mut Ui,
    icon: UiIcon,
    label: &str,
    color: Color32,
    enabled: bool,
    disabled_tooltip: Option<&str>,
) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), sense);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, Rounding::same(8.0), card_hover());
    }
    let ic_r = Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), Vec2::splat(13.0));
    paint_icon(ui.painter(), ic_r, icon, color);
    ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.5),
        color,
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    } else {
        if let Some(tip) = disabled_tooltip {
            resp.on_hover_text(tip);
        }
        false
    }
}

fn render_close_notice_modal(ctx: &egui::Context, state: &mut RewindUiState) {
    if !state.show_close_recording_notice {
        return;
    }

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        state.show_close_recording_notice = false;
        return;
    }

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let ac = accent.color();
    let screen = ctx.screen_rect();
    let modal_w = 430.0_f32.min(screen.width() - 48.0);
    let modal_h = 192.0_f32;
    let modal_rect = Rect::from_center_size(screen.center(), Vec2::new(modal_w, modal_h));

    egui::Area::new(Id::new("close_recording_notice"))
        .order(Order::Tooltip)
        .fixed_pos(screen.left_top())
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            let _ = ui.interact(screen, Id::new("close_notice_backdrop"), Sense::click());
            ui.painter().rect_filled(
                screen,
                Rounding::ZERO,
                Color32::from_rgba_unmultiplied(6, 8, 13, 195),
            );

            ui.painter().rect_filled(
                modal_rect,
                Rounding::same(14.0),
                Color32::from_rgba_unmultiplied(15, 19, 27, 250),
            );
            ui.painter().rect_stroke(
                modal_rect,
                Rounding::same(14.0),
                Stroke::new(1.2_f32, ac),
            );

            ui.allocate_new_ui(
                egui::UiBuilder::new().max_rect(modal_rect.shrink2(Vec2::new(22.0, 20.0))),
                |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Rewind is still recording")
                                .size(16.5)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(
                                "Closing this window keeps Rewind running in your system tray so your replay hotkey stays active. Open it anytime from the tray icon.",
                            )
                            .size(12.5)
                            .color(TEXT_MUTED),
                        );
                        ui.add_space(18.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let ok_fg = contrasting_fg(ac);
                            if modal_action_pill(
                                ui,
                                UiIcon::Check,
                                "Okay",
                                ok_fg,
                                ac,
                                ac,
                                ok_fg,
                            ) {
                                state.show_close_recording_notice = false;
                                state.allow_close_now = true;
                                ctx.send_viewport_cmd(ViewportCommand::Close);
                            }
                            if modal_action_pill(
                                ui,
                                UiIcon::Close,
                                "Cancel",
                                TEXT_PRIMARY,
                                pill_bg(),
                                border_col(),
                                TEXT_MUTED,
                            ) {
                                state.show_close_recording_notice = false;
                            }
                        });
                    });
                },
            );
        });
}

fn paint_game_badge(
    ui: &mut Ui,
    state: &mut RewindUiState,
    rect: Rect,
    game_name: &str,
    accent: Color32,
) {
    if let Some(tex) = state.get_or_fetch_game_icon(ui.ctx(), game_name) {
        paint_rounded_image(
            ui.painter(),
            tex.id(),
            rect,
            Rounding::same((rect.width() * 0.24).clamp(2.5, 5.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        paint_game_badge_initial(ui.painter(), rect, game_name, accent);
    }
}

fn paint_preload_overlay(painter: &egui::Painter, rect: Rect, pct: f32, accent: Color32) {
    let pill = Rect::from_center_size(rect.center(), Vec2::new(220.0, 56.0));
    painter.rect_filled(
        pill,
        Rounding::same(10.0),
        Color32::from_rgba_unmultiplied(12, 15, 22, 228),
    );
    painter.rect_stroke(pill, Rounding::same(10.0), Stroke::new(1.0_f32, border_bright()));
    let shown = (pct.clamp(0.0, 1.0) * 100.0).round() as u32;
    painter.text(
        Pos2::new(pill.center().x, pill.top() + 18.0),
        egui::Align2::CENTER_CENTER,
        format!("Preloading video... {shown}%"),
        FontId::proportional(12.5),
        TEXT_PRIMARY,
    );
    let track = Rect::from_min_size(
        Pos2::new(pill.left() + 18.0, pill.bottom() - 16.0),
        Vec2::new(pill.width() - 36.0, 5.0),
    );
    painter.rect_filled(track, Rounding::same(2.5), Color32::from_rgb(36, 44, 58));
    let fill_w = (track.width() * pct.clamp(0.02, 1.0)).max(4.0);
    let fill = Rect::from_min_size(track.left_top(), Vec2::new(fill_w, track.height()));
    painter.rect_filled(fill, Rounding::same(2.5), accent);
}

fn render_rename_clip_modal(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) {
    let Some((clip_id, mut buf)) = state.rename_modal_clip.take() else {
        return;
    };

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        state.rename_modal_just_opened = false;
        return;
    }

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let ac = accent.color();
    let screen = ctx.screen_rect();
    let modal_w = 410.0_f32.min(screen.width() - 48.0);
    let modal_h = 166.0_f32;
    let modal_rect = Rect::from_center_size(screen.center(), Vec2::new(modal_w, modal_h));
    let mut keep_open = true;

    egui::Area::new(Id::new("rename_clip_modal"))
        .order(Order::Tooltip)
        .fixed_pos(screen.left_top())
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            let _ = ui.interact(screen, Id::new("rename_clip_backdrop"), Sense::click());
            ui.painter().rect_filled(
                screen,
                Rounding::ZERO,
                Color32::from_rgba_unmultiplied(6, 8, 13, 195),
            );

            ui.painter().rect_filled(
                modal_rect,
                Rounding::same(14.0),
                Color32::from_rgba_unmultiplied(15, 19, 27, 250),
            );
            ui.painter().rect_stroke(
                modal_rect,
                Rounding::same(14.0),
                Stroke::new(1.2_f32, ac),
            );

            ui.allocate_new_ui(
                egui::UiBuilder::new().max_rect(modal_rect.shrink2(Vec2::new(22.0, 18.0))),
                |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Edit Clip Name")
                                .size(16.0)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        ui.add_space(12.0);
                        let resp = ui.add(
                            TextEdit::singleline(&mut buf)
                                .desired_width(ui.available_width())
                                .font(FontId::proportional(13.5)),
                        );
                        if state.rename_modal_just_opened {
                            resp.request_focus();
                            state.rename_modal_just_opened = false;
                        }
                        ui.add_space(16.0);
                        let enter = ui.input(|i| i.key_pressed(Key::Enter));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let ok_fg = contrasting_fg(ac);
                            let save_clicked = modal_action_pill(
                                ui,
                                UiIcon::Check,
                                "Save",
                                ok_fg,
                                ac,
                                ac,
                                ok_fg,
                            );
                            if (save_clicked || enter) && !buf.trim().is_empty() {
                                state.send_sync(
                                    IpcRequest::RenameClip {
                                        id: clip_id,
                                        title: buf.trim().to_owned(),
                                    },
                                    now_secs,
                                );
                                keep_open = false;
                            }
                            if modal_action_pill(
                                ui,
                                UiIcon::Close,
                                "Cancel",
                                TEXT_PRIMARY,
                                pill_bg(),
                                border_col(),
                                TEXT_MUTED,
                            ) {
                                keep_open = false;
                            }
                        });
                    });
                },
            );
        });

    if keep_open {
        state.rename_modal_clip = Some((clip_id, buf));
    }
}

fn render_onboarding_and_tour(ctx: &egui::Context, state: &mut RewindUiState, now_secs: f64) {
    if !state.daemon_connected {
        return;
    }
    if state.snapshot.config.onboarding_completed && state.tour_step.is_none() {
        return;
    }

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let ac = accent.color();
    let screen = ctx.screen_rect();

    if let Some(tour_idx) = state.tour_step {
        let t = anim_open_progress(ctx, Id::new("tour_step_anim"), tour_idx as u64, now_secs, 0.22);
        let first_clip_rect = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("tour_first_clip_rect")))
            .unwrap_or_else(|| Rect::from_center_size(screen.center(), Vec2::new(300.0, 180.0)));
        let (target_rect, title, body) = match tour_idx {
            0 => (
                Rect::from_min_max(
                    Pos2::new(screen.left() + 102.0, screen.top() + 5.0),
                    Pos2::new(
                        (screen.right() - 102.0).max(screen.left() + 260.0),
                        screen.top() + 41.0,
                    ),
                ),
                "Capture & Instant Replay",
                "Choose your capture source, replay length, and toggle Instant Replay on or off anytime.",
            ),
            1 => (
                Rect::from_min_max(
                    Pos2::new(screen.left() + 5.0, screen.top() + 54.0),
                    Pos2::new(screen.left() + 49.0, screen.top() + 236.0),
                ),
                "Games & Clip Library",
                "Browse your games on Home, filter All Clips or Favorites, and right-click any clip for quick actions.",
            ),
            2 => (
                first_clip_rect,
                "Clip Editor & Discord (<10 MB)",
                "Click any clip to trim frame-by-frame or export to Discord (<10 MB) for clips up to 60 seconds.",
            ),
            _ => (
                Rect::from_min_max(
                    Pos2::new(screen.left() + 5.0, (screen.bottom() - 54.0).max(120.0)),
                    Pos2::new(screen.left() + 49.0, (screen.bottom() - 10.0).max(164.0)),
                ),
                "Settings & Customization",
                "Configure hotkeys, 120fps quality, custom games, or replay this tour whenever you want.",
            ),
        };

        let card_w = 348.0_f32;
        let card_h = 176.0_f32;
        let card_pos = match tour_idx {
            0 => Pos2::new(
                (target_rect.center().x - card_w * 0.5).clamp(24.0, screen.right() - card_w - 24.0),
                target_rect.bottom() + 16.0 + (1.0 - t) * 10.0,
            ),
            1 => Pos2::new(
                (target_rect.right() + 18.0).min(screen.right() - card_w - 24.0),
                target_rect.top() + (1.0 - t) * 10.0,
            ),
            2 => {
                if target_rect.right() + 20.0 + card_w <= screen.right() - 16.0 {
                    Pos2::new(
                        target_rect.right() + 20.0,
                        target_rect.top().clamp(64.0, screen.bottom() - card_h - 20.0)
                            + (1.0 - t) * 10.0,
                    )
                } else {
                    Pos2::new(
                        (target_rect.center().x - card_w * 0.5)
                            .clamp(24.0, screen.right() - card_w - 24.0),
                        (target_rect.bottom() + 16.0).min(screen.bottom() - card_h - 20.0)
                            + (1.0 - t) * 10.0,
                    )
                }
            }
            _ => Pos2::new(
                (target_rect.right() + 18.0).min(screen.right() - card_w - 24.0),
                (target_rect.bottom() - card_h).max(24.0) + (1.0 - t) * 10.0,
            ),
        };
        let card_rect = Rect::from_min_size(card_pos, Vec2::new(card_w, card_h));

        egui::Area::new(Id::new("rewind_tour_overlay"))
            .order(Order::Tooltip)
            .fixed_pos(screen.left_top())
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.set_max_size(screen.size());
                let _ = ui.interact(screen, Id::new("tour_backdrop"), Sense::click());
                paint_tour_spotlight(ui.painter(), screen, target_rect, ac, t);

                ui.painter().rect_filled(
                    card_rect,
                    Rounding::same(18.0),
                    Color32::from_rgba_unmultiplied(16, 20, 30, (248.0 * t) as u8),
                );
                ui.painter().rect_stroke(
                    card_rect,
                    Rounding::same(18.0),
                    Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(255, 255, 255, (32.0 * t) as u8),
                    ),
                );

                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(card_rect.shrink(18.0)),
                    |ui| {
                        ui.set_opacity(t);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("TOUR  {} / 4", tour_idx + 1))
                                        .size(10.5)
                                        .strong()
                                        .color(ac),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let skip = ui.add(
                                        egui::Label::new(
                                            RichText::new("Skip").size(11.5).color(TEXT_MUTED),
                                        )
                                        .sense(Sense::click()),
                                    );
                                    if skip.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                        state.tour_step = None;
                                        state.active_tab = NavTab::Home;
                                        if !state.snapshot.config.onboarding_completed {
                                            state.snapshot.config.onboarding_completed = true;
                                            state.push_config_update(now_secs);
                                        }
                                    }
                                });
                            });
                            ui.add_space(6.0);
                            ui.label(RichText::new(title).size(16.0).strong().color(TEXT_PRIMARY));
                            ui.add_space(6.0);
                            ui.label(RichText::new(body).size(12.5).color(TEXT_MUTED));
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                for i in 0..4 {
                                    let col = if i == tour_idx { ac } else { border_bright() };
                                    let w = if i == tour_idx { 16.0 } else { 6.0 };
                                    let (dr, _) =
                                        ui.allocate_exact_size(Vec2::new(w, 6.0), Sense::hover());
                                    ui.painter().rect_filled(dr, Rounding::same(3.0), col);
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let next_lbl = if tour_idx == 3 { "Get Started" } else { "Next" };
                                    if onboarding_pill(ui, next_lbl, true, ac) {
                                        if tour_idx >= 3 {
                                            state.tour_step = None;
                                            state.active_tab = NavTab::Home;
                                            if !state.snapshot.config.onboarding_completed {
                                                state.snapshot.config.onboarding_completed = true;
                                                state.push_config_update(now_secs);
                                            }
                                        } else {
                                            let next = tour_idx + 1;
                                            state.tour_step = Some(next);
                                            state.active_tab = match next {
                                                1 | 2 => NavTab::AllClips,
                                                3 => NavTab::Settings,
                                                _ => NavTab::Home,
                                            };
                                        }
                                    }
                                    if tour_idx > 0 {
                                        ui.add_space(6.0);
                                        if onboarding_pill(ui, "Back", false, ac) {
                                            let prev = tour_idx - 1;
                                            state.tour_step = Some(prev);
                                            state.active_tab = match prev {
                                                1 | 2 => NavTab::AllClips,
                                                3 => NavTab::Settings,
                                                _ => NavTab::Home,
                                            };
                                        }
                                    }
                                });
                            });
                        });
                    },
                );
            });
        return;
    }

    let step = state.onboarding_step.min(2);
    let t = anim_open_progress(
        ctx,
        Id::new("onboarding_card_anim"),
        step as u64,
        now_secs,
        0.24,
    );
    let card_w = 460.0_f32.min(screen.width() - 48.0);
    let card_h = 356.0_f32;
    let card_center = screen.center() + Vec2::new(0.0, (1.0 - t) * 16.0);
    let card_rect = Rect::from_center_size(card_center, Vec2::new(card_w, card_h));

    egui::Area::new(Id::new("rewind_onboarding_modal"))
        .order(Order::Tooltip)
        .fixed_pos(screen.left_top())
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            let _ = ui.interact(screen, Id::new("onboard_backdrop"), Sense::click());
            ui.painter().rect_filled(
                screen,
                Rounding::ZERO,
                Color32::from_rgba_unmultiplied(6, 8, 13, (200.0 * t) as u8),
            );
            ui.painter().rect_filled(
                card_rect,
                Rounding::same(22.0),
                Color32::from_rgba_unmultiplied(15, 19, 28, (250.0 * t) as u8),
            );
            ui.painter().rect_stroke(
                card_rect,
                Rounding::same(22.0),
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(255, 255, 255, (30.0 * t) as u8),
                ),
            );

            ui.allocate_new_ui(
                egui::UiBuilder::new().max_rect(card_rect.shrink(28.0)),
                |ui| {
                    ui.set_opacity(t);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for i in 0..3 {
                                let col = if i == step { ac } else { border_bright() };
                                let w = if i == step { 20.0 } else { 7.0 };
                                let (dr, _) =
                                    ui.allocate_exact_size(Vec2::new(w, 6.0), Sense::hover());
                                ui.painter().rect_filled(dr, Rounding::same(3.0), col);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let skip = ui.add(
                                    egui::Label::new(
                                        RichText::new("Skip Setup").size(12.0).color(TEXT_MUTED),
                                    )
                                    .sense(Sense::click()),
                                );
                                if skip.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    state.snapshot.config.onboarding_completed = true;
                                    state.push_config_update(now_secs);
                                }
                            });
                        });
                        ui.add_space(18.0);

                        match step {
                            0 => {
                                ui.label(
                                    RichText::new("Welcome to Rewind")
                                        .size(22.0)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("A clipping software. Two quick questions to set things up:")
                                        .size(12.5)
                                        .color(TEXT_MUTED),
                                );
                                ui.add_space(20.0);
                                if onboarding_question_toggle(
                                    ui,
                                    "ob_autostart",
                                    "Start Rewind with Windows",
                                    "Keeps instant replay ready automatically when you turn on your PC.",
                                    &mut state.snapshot.config.start_with_windows,
                                    ac,
                                ) {
                                    state.push_config_update(now_secs);
                                }
                                ui.add_space(12.0);
                                if onboarding_question_toggle(
                                    ui,
                                    "ob_autohide",
                                    "Hide UI when a game launches",
                                    "Closes this window while you play and reopens it when you exit.",
                                    &mut state.snapshot.config.close_ui_on_game_launch,
                                    ac,
                                ) {
                                    state.push_config_update(now_secs);
                                }
                            }
                            1 => {
                                ui.label(
                                    RichText::new("Clip Preferences")
                                        .size(22.0)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("You can always adjust these later in Settings.")
                                        .size(12.5)
                                        .color(TEXT_MUTED),
                                );
                                ui.add_space(20.0);
                                ui.label(
                                    RichText::new("Default Replay Clip Length")
                                        .size(13.0)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    for (cl, lbl) in [
                                        (ClipLength::Sec30, "30 seconds"),
                                        (ClipLength::Min1, "1 minute"),
                                        (ClipLength::Min2, "2 minutes"),
                                    ] {
                                        let sel = state.snapshot.config.clip_length == cl;
                                        if onboarding_pill(ui, lbl, sel, ac) {
                                            state.snapshot.config.clip_length = cl;
                                            state.push_config_update(now_secs);
                                        }
                                        ui.add_space(6.0);
                                    }
                                });
                                ui.add_space(18.0);
                                ui.label(
                                    RichText::new("Save Instant Replay Hotkey")
                                        .size(13.0)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    let is_rebinding =
                                        state.rebind_target == RebindTarget::SaveClip;
                                    let hk_lbl = if is_rebinding {
                                        "Press any key...".to_owned()
                                    } else {
                                        state.snapshot.config.hotkey_save_clip.clone()
                                    };
                                    if onboarding_pill(ui, &hk_lbl, is_rebinding, ac) {
                                        state.rebind_target = if is_rebinding {
                                            RebindTarget::None
                                        } else {
                                            RebindTarget::SaveClip
                                        };
                                    }
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new("Press anytime in-game to save a clip")
                                            .size(12.0)
                                            .color(TEXT_MUTED),
                                    );
                                });
                            }
                            _ => {
                                ui.add_space(12.0);
                                ui.label(
                                    RichText::new("You're all set")
                                        .size(24.0)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new("Take a 15-second interactive tour to see where Capture, Library, Discord export, and Settings live.")
                                        .size(13.0)
                                        .color(TEXT_MUTED),
                                );
                                ui.add_space(28.0);
                            }
                        }

                        ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                            ui.horizontal(|ui| {
                                if step < 2 {
                                    if onboarding_pill(ui, "Continue", true, ac) {
                                        state.onboarding_step = step + 1;
                                    }
                                } else {
                                    if onboarding_pill(ui, "Start Quick Tour", true, ac) {
                                        state.snapshot.config.onboarding_completed = true;
                                        state.push_config_update(now_secs);
                                        state.tour_step = Some(0);
                                        state.active_tab = NavTab::Home;
                                    }
                                    ui.add_space(8.0);
                                    if onboarding_pill(ui, "Skip Tour", false, ac) {
                                        state.snapshot.config.onboarding_completed = true;
                                        state.push_config_update(now_secs);
                                    }
                                }
                                if step > 0 {
                                    ui.add_space(8.0);
                                    if onboarding_pill(ui, "Back", false, ac) {
                                        state.onboarding_step = step - 1;
                                    }
                                }
                            });
                        });
                    });
                },
            );
        });
}

fn onboarding_question_toggle(
    ui: &mut Ui,
    id: &str,
    title: &str,
    subtitle: &str,
    val: &mut bool,
    accent: Color32,
) -> bool {
    let mut changed = false;
    egui::Frame::none()
        .fill(Color32::from_rgba_unmultiplied(24, 30, 44, 210))
        .stroke(Stroke::new(1.0_f32, border_col()))
        .rounding(Rounding::same(14.0))
        .inner_margin(Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 48.0);
                    ui.label(RichText::new(title).size(13.0).strong().color(TEXT_PRIMARY));
                    ui.add_space(2.0);
                    ui.label(RichText::new(subtitle).size(11.5).color(TEXT_MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if toggle_switch(ui, id, val, accent) {
                        changed = true;
                    }
                });
            });
        });
    changed
}

fn onboarding_pill(ui: &mut Ui, label: &str, primary: bool, accent: Color32) -> bool {
    let w = (label.len() as f32 * 7.2 + 28.0).max(80.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
    let hov = resp.hovered();
    let fill = if primary {
        if hov { accent.gamma_multiply(1.12) } else { accent }
    } else if hov {
        card_hover()
    } else {
        pill_bg()
    };
    let fg = if primary { contrasting_fg(accent) } else { TEXT_PRIMARY };
    ui.painter().rect_filled(rect, Rounding::same(17.0), fill);
    if !primary {
        ui.painter()
            .rect_stroke(rect, Rounding::same(17.0), Stroke::new(1.0_f32, border_col()));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        fg,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn paint_tour_spotlight(
    painter: &egui::Painter,
    screen: Rect,
    target: Rect,
    accent: Color32,
    t: f32,
) {
    let dim = Color32::from_rgba_unmultiplied(6, 8, 13, (175.0 * t) as u8);
    let top = Rect::from_min_max(screen.left_top(), Pos2::new(screen.right(), target.top()));
    let bottom = Rect::from_min_max(Pos2::new(screen.left(), target.bottom()), screen.right_bottom());
    let left = Rect::from_min_max(
        Pos2::new(screen.left(), target.top()),
        Pos2::new(target.left(), target.bottom()),
    );
    let right = Rect::from_min_max(
        Pos2::new(target.right(), target.top()),
        Pos2::new(screen.right(), target.bottom()),
    );
    for r in [top, bottom, left, right] {
        if r.width() > 0.0 && r.height() > 0.0 {
            painter.rect_filled(r, Rounding::ZERO, dim);
        }
    }
    painter.rect_stroke(
        target.expand(3.0),
        Rounding::same(14.0),
        Stroke::new(
            2.0_f32,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (230.0 * t) as u8),
        ),
    );
}

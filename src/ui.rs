use crate::art::{
    paint_empty_thumb_surface, paint_fluent_background_reveal, paint_game_badge_initial,
    paint_icon, paint_rounded_image, UiIcon,
};
use crate::state::{
    ActivePopup, FilterPill, NavTab, RebindTarget, RewindUiState, SettingsCategory, SortOrder,
    UpdateStatus, ViewMode,
};
use egui::{
    Align, Color32, CursorIcon, FontId, Id, Key, Layout, Margin, Order, Pos2, Rect, Response,
    RichText, Rounding, Sense, Shadow, Shape, Stroke, TextEdit, TextureOptions, Ui, Vec2,
    ViewportCommand,
};
use rewind::config::{
    AccentColor, AppConfig, ClipLength, ContainerFormat, EncoderPreference, FrameRate, RateControl,
    Resolution, VideoCodec,
};
use rewind::game_detector::normalize_game_entry;
use rewind::game_icons::STEAMGRIDDB_API_KEY_URL;
use rewind::ipc::IpcRequest;
use rewind::library::ClipRecord;
use rewind::updater::{open_url, CURRENT_VERSION, RELEASES_PAGE_URL};
use rewind::win_ffi::{
    copy_file_to_clipboard, pick_exe_file_dialog, pick_image_file_dialog,
    set_active_window_borderless_fullscreen,
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

thread_local! {
    static THEME_SWEEP: std::cell::Cell<Option<(Pos2, f32, Color32, Color32)>> =
        const { std::cell::Cell::new(None) };
}

fn sweep_color(pos: Pos2, c: Color32) -> Color32 {
    THEME_SWEEP.with(|cell| {
        let Some((origin, radius, old_ac, new_ac)) = cell.get() else {
            return c;
        };
        if c != new_ac {
            return c;
        }
        let dist = pos.distance(origin);
        let blend = ((radius - dist) * 0.5).clamp(0.0, 1.0);
        lerp_color(old_ac, new_ac, blend)
    })
}

fn sweep_soft_bg(pos: Pos2, ac: Color32) -> Color32 {
    let c = sweep_color(pos, ac);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 36)
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
            card_bg: Color32::from_rgb(20, 22, 28),
            card_hover: Color32::from_rgb(28, 31, 39),
            pill_bg: Color32::from_rgb(24, 27, 34),
            popover_bg: Color32::from_rgb(20, 22, 28),
            border: Color32::from_rgba_premultiplied(20, 20, 20, 20),
            border_bright: Color32::from_rgba_premultiplied(32, 32, 32, 32),
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
            border: Color32::from_rgba_unmultiplied(
                ar.saturating_add(16),
                ag.saturating_add(16),
                ab.saturating_add(18),
                42,
            ),
            border_bright: Color32::from_rgba_unmultiplied(
                ar.saturating_add(34),
                ag.saturating_add(34),
                ab.saturating_add(38),
                72,
            ),
            icon_box_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 16),
            divider_col: Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            track_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 34),
            nav_hover_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 22),
        }
    }
}

thread_local! {
    static UI_PAL: std::cell::Cell<UiPalette> = const { std::cell::Cell::new(UiPalette::default_dark()) };
    static UI_PAL_NEW: std::cell::Cell<UiPalette> = const { std::cell::Cell::new(UiPalette::default_dark()) };
}

fn ui_pal() -> UiPalette {
    UI_PAL.with(|p| p.get())
}

fn ui_pal_new() -> UiPalette {
    UI_PAL_NEW.with(|p| p.get())
}

fn clip_poly_to_rect(poly: &[Pos2], rect: Rect, corner_r: f32) -> Vec<Pos2> {
    fn clip_edge(pts: Vec<Pos2>, inside: impl Fn(Pos2) -> bool, intersect: impl Fn(Pos2, Pos2) -> Pos2) -> Vec<Pos2> {
        let mut out = Vec::with_capacity(pts.len() + 2);
        if pts.is_empty() {
            return out;
        }
        let mut prev = *pts.last().unwrap();
        let mut prev_in = inside(prev);
        for &cur in &pts {
            let cur_in = inside(cur);
            if cur_in {
                if !prev_in {
                    out.push(intersect(prev, cur));
                }
                out.push(cur);
            } else if prev_in {
                out.push(intersect(prev, cur));
            }
            prev = cur;
            prev_in = cur_in;
        }
        out
    }
    fn clip_halfplane(pts: Vec<Pos2>, nx: f32, ny: f32, c: f32) -> Vec<Pos2> {
        clip_edge(
            pts,
            |p| nx * p.x + ny * p.y >= c,
            |a, b| {
                let da = nx * a.x + ny * a.y - c;
                let db = nx * b.x + ny * b.y - c;
                let t = da / (da - db);
                Pos2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
            },
        )
    }
    let mut pts = poly.to_vec();
    pts = clip_halfplane(pts, 1.0, 0.0, rect.left());
    pts = clip_halfplane(pts, -1.0, 0.0, -rect.right());
    pts = clip_halfplane(pts, 0.0, 1.0, rect.top());
    pts = clip_halfplane(pts, 0.0, -1.0, -rect.bottom());
    if corner_r > 0.5 {
        let cr = corner_r
            .min(rect.width() * 0.5)
            .min(rect.height() * 0.5);
        let tl = (rect.left() + cr, rect.top() + cr);
        let tr = (rect.right() - cr, rect.top() + cr);
        let bl = (rect.left() + cr, rect.bottom() - cr);
        let br = (rect.right() - cr, rect.bottom() - cr);
        for k in 1..=12 {
            let theta = (k as f32 / 13.0) * std::f32::consts::FRAC_PI_2;
            let (ct, st) = (theta.cos(), theta.sin());
            pts = clip_halfplane(pts, ct, st, tl.0 * ct + tl.1 * st - cr);
            pts = clip_halfplane(pts, -ct, st, -tr.0 * ct + tr.1 * st - cr);
            pts = clip_halfplane(pts, ct, -st, bl.0 * ct - bl.1 * st - cr);
            pts = clip_halfplane(pts, -ct, -st, -br.0 * ct - br.1 * st - cr);
        }
    }
    pts
}

fn build_swept_panel_shapes(
    rect: Rect,
    old_fill: Color32,
    new_fill: Color32,
    corner_r: f32,
) -> Vec<Shape> {
    let Some((origin, radius, _, _)) = THEME_SWEEP.with(|c| c.get()) else {
        return vec![Shape::rect_filled(rect, Rounding::same(corner_r), old_fill)];
    };
    if old_fill == new_fill {
        return vec![Shape::rect_filled(rect, Rounding::same(corner_r), old_fill)];
    }
    let r = radius.max(1.0);
    let outer = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()]
        .into_iter()
        .map(|c| origin.distance(c))
        .fold(r + 10.0, f32::max)
        + 10.0;
    let segs = 64usize;
    let mut out = Vec::new();

    let circle_pts: Vec<Pos2> = (0..segs)
        .map(|i| {
            let ang = (i as f32 / segs as f32) * std::f32::consts::TAU;
            Pos2::new(origin.x + ang.cos() * r, origin.y + ang.sin() * r)
        })
        .collect();
    let clipped_in = clip_poly_to_rect(&circle_pts, rect, corner_r);
    if clipped_in.len() >= 3 {
        let mut mesh_in = egui::Mesh::default();
        for &pt in &clipped_in {
            mesh_in.colored_vertex(pt, new_fill);
        }
        for i in 1..clipped_in.len() - 1 {
            mesh_in.add_triangle(0, i as u32, (i + 1) as u32);
        }
        out.push(Shape::mesh(mesh_in));
    }

    if outer > r {
        let mut mesh_out = egui::Mesh::default();
        for i in 0..segs {
            let a0 = (i as f32 / segs as f32) * std::f32::consts::TAU;
            let a1 = ((i + 1) as f32 / segs as f32) * std::f32::consts::TAU;
            let quad = [
                Pos2::new(origin.x + a0.cos() * r, origin.y + a0.sin() * r),
                Pos2::new(origin.x + a0.cos() * outer, origin.y + a0.sin() * outer),
                Pos2::new(origin.x + a1.cos() * outer, origin.y + a1.sin() * outer),
                Pos2::new(origin.x + a1.cos() * r, origin.y + a1.sin() * r),
            ];
            let clipped = clip_poly_to_rect(&quad, rect, corner_r);
            if clipped.len() >= 3 {
                let base = mesh_out.vertices.len() as u32;
                for &pt in &clipped {
                    mesh_out.colored_vertex(pt, old_fill);
                }
                for k in 1..clipped.len() - 1 {
                    mesh_out.add_triangle(base, base + k as u32, base + (k + 1) as u32);
                }
            }
        }
        if !mesh_out.indices.is_empty() {
            out.push(Shape::mesh(mesh_out));
        }
    }
    out
}

fn paint_swept_panel(painter: &egui::Painter, rect: Rect, old_fill: Color32, new_fill: Color32) {
    let p = painter.with_clip_rect(rect);
    for s in build_swept_panel_shapes(rect, old_fill, new_fill, 0.0) {
        p.add(s);
    }
}

fn paint_swept_rounded_rect(
    painter: &egui::Painter,
    rect: Rect,
    corner_r: f32,
    old_fill: Color32,
    new_fill: Color32,
    stroke: Stroke,
) {
    let p = painter.with_clip_rect(rect);
    for s in build_swept_panel_shapes(rect, old_fill, new_fill, corner_r) {
        p.add(s);
    }
    if stroke.width > 0.0 && stroke.color.a() > 0 {
        painter.rect_stroke(rect, Rounding::same(corner_r), stroke);
    }
}

fn paint_swept_pill_box(
    painter: &egui::Painter,
    rect: Rect,
    corner_r: f32,
    hovered: bool,
    active_border: bool,
) {
    let old_fill = if hovered { card_hover() } else { pill_bg() };
    let new_pal = ui_pal_new();
    let new_fill = if hovered { new_pal.card_hover } else { new_pal.pill_bg };
    let old_border = if active_border { border_bright() } else { border_col() };
    let new_border = if active_border { new_pal.border_bright } else { new_pal.border };
    let border = if let Some((origin, radius, _, _)) = THEME_SWEEP.with(|c| c.get()) {
        let dist = rect.center().distance(origin);
        let blend = ((radius - dist) * 0.5).clamp(0.0, 1.0);
        lerp_color(old_border, new_border, blend)
    } else {
        old_border
    };
    paint_swept_rounded_rect(painter, rect, corner_r, old_fill, new_fill, Stroke::new(1.0_f32, border));
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
        && !state.snapshot.config.suppress_recording_close_prompt
    {
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        state.show_close_recording_notice = true;
    }
    if state.selected_clip_id.is_none() && state.viewport_fullscreen_applied {
        set_active_window_borderless_fullscreen(false);
        state.viewport_fullscreen_applied = false;
    }

    if let Some(new_rgb) = state.ensure_background_loaded(ctx) {
        if state.snapshot.config.auto_adjust_palette
            && state.snapshot.config.custom_accent_rgb != Some(new_rgb)
        {
            state.snapshot.config.custom_accent_rgb = Some(new_rgb);
            state.push_config_update(now_secs);
        }
    }

    let accent = ThemeAccent::from_config(&state.snapshot.config);
    let active_reveal = state.active_theme_reveal(ctx, now_secs);
    let pal_accent = active_reveal
        .as_ref()
        .map(|(_, _, old_ac, _, _)| *old_ac)
        .unwrap_or_else(|| accent.color());
    let build_pal = |ac: Color32| {
        if state.bg_texture.is_some() {
            let ambient = if state.snapshot.config.auto_adjust_palette {
                state
                    .bg_ambient_tint
                    .unwrap_or(Color32::from_rgb(16, 16, 18))
            } else {
                Color32::from_rgb(
                    (ac.r() / 8).max(10),
                    (ac.g() / 8).max(10),
                    (ac.b() / 8).max(10),
                )
            };
            UiPalette::from_ambient(ambient)
        } else {
            UiPalette::default_dark()
        }
    };
    UI_PAL.with(|p| p.set(build_pal(pal_accent)));
    UI_PAL_NEW.with(|p| p.set(build_pal(accent.color())));

    let flash_alpha = compute_clip_flash_alpha(ctx, state, now_secs);
    apply_apple_rounding(ctx);
    THEME_SWEEP.with(|cell| {
        cell.set(
            active_reveal
                .as_ref()
                .map(|(origin, r, old_ac, _, _)| (*origin, *r, *old_ac, accent.color())),
        );
    });
    let logo_ac = sweep_color(Pos2::new(27.0, 23.0), accent.color());
    state.ensure_app_icon_synced(ctx, logo_ac);
    paint_fluent_background_reveal(
        ctx,
        accent.color(),
        state.bg_texture.as_ref(),
        state.bg_ambient_tint,
        state.snapshot.config.background_brightness,
        active_reveal
            .as_ref()
            .map(|(o, r, oa, ot, ob)| (*o, *r, *oa, ot.as_ref(), *ob)),
    );

    let mut pop = PopoverLayout {
        anchor: None,
        toggled_this_frame: false,
    };

    render_top_bar(ctx, state, &mut pop, now_secs, flash_alpha);
    render_sidebar(ctx, state);

    let modal_blocking = state.selected_clip_id.is_some()
        || state.rename_modal_clip.is_some()
        || state.show_close_recording_notice
        || !state.snapshot.config.onboarding_completed
        || state.tour_step.is_some();

    egui::CentralPanel::default()
        .frame(
            egui::Frame::none()
                .fill(Color32::TRANSPARENT)
                .inner_margin(Margin::symmetric(24.0, 18.0)),
        )
        .show(ctx, |ui| {
            if modal_blocking {
                ui.disable();
            }
            match state.active_tab {
                NavTab::Home => render_home(ui, state, &mut pop, now_secs),
                NavTab::AllClips | NavTab::Favorites => render_library(ui, state, &mut pop, now_secs),
                NavTab::Folders => render_folders(ui, state),
                NavTab::Settings => render_settings(ui, state, &mut pop, now_secs),
            }
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
    let top_layer_is_base = ui
        .input(|i| i.pointer.hover_pos())
        .and_then(|p| ui.ctx().layer_id_at(p))
        .map(|l| matches!(l.order, Order::Background | Order::Middle))
        .unwrap_or(true);
    let hovered = ui.is_enabled() && top_layer_is_base && ui.rect_contains_pointer(avail);

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

    let not_dragging_widget = ui.ctx().dragged_id().is_none();
    let (wheel_y, drag_y, pointer_down) = if hovered {
        ui.input(|i| {
            let down = i.pointer.primary_down() && not_dragging_widget;
            let dy = if down { i.pointer.delta().y } else { 0.0 };
            let wy = if i.raw_scroll_delta.y.abs() > 0.01 {
                i.raw_scroll_delta.y
            } else {
                i.smooth_scroll_delta.y
            };
            (wy, dy, down)
        })
    } else {
        (0.0, 0.0, false)
    };
    let scroll_delta = wheel_y * 1.35 + drag_y;
    if scroll_delta.abs() > 0.01 {
        if wheel_y.abs() > 0.01 {
            ui.ctx().input_mut(|i| {
                i.raw_scroll_delta.y = 0.0;
                i.smooth_scroll_delta.y = 0.0;
            });
        }
        s.target_y -= scroll_delta;
        if s.target_y < 0.0 {
            let excess = -s.target_y;
            s.target_y = 0.0;
            s.overscroll = (s.overscroll + excess * 0.35).min(48.0);
        } else if s.target_y > s.last_max_y {
            let excess = s.target_y - s.last_max_y;
            s.target_y = s.last_max_y;
            s.overscroll = (s.overscroll - excess * 0.35).max(-48.0);
        }
    }

    let k = 1.0 - (-22.0 * dt).exp();
    s.current_y += (s.target_y - s.current_y) * k;
    if !pointer_down {
        s.overscroll *= (-14.0 * dt).exp();
    }
    if s.overscroll.abs() < 0.05 {
        s.overscroll = 0.0;
    }

    let scroll_offset = s.current_y.clamp(0.0, s.last_max_y) + (-s.overscroll).max(0.0);
    let out = egui::ScrollArea::vertical()
        .id_salt(id_source)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, false])
        .vertical_scroll_offset(scroll_offset)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                if s.overscroll > 0.2 {
                    ui.add_space(s.overscroll);
                }
                add_contents(ui);
                if s.overscroll < -0.2 {
                    ui.add_space(-s.overscroll);
                }
            });
        });

    let max_y = (out.content_size.y - s.overscroll.abs() - out.inner_rect.height()).max(0.0);
    s.last_max_y = max_y;
    s.target_y = s.target_y.clamp(0.0, max_y);
    s.current_y = s.current_y.clamp(0.0, max_y);

    if (s.target_y - s.current_y).abs() > 0.2 || s.overscroll.abs() > 0.2 {
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
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0_f32, bar_border))
                .inner_margin(Margin::symmetric(12.0, 6.0)),
        )
        .show(ctx, |ui| {
            paint_swept_panel(ui.painter(), ui.clip_rect(), bar_bg(), ui_pal_new().bar_bg);
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
                        if state.daemon_connected
                            && state.snapshot.replay_on
                            && !state.snapshot.config.suppress_recording_close_prompt
                        {
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
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0_f32, border_col()))
                .inner_margin(Margin::symmetric(6.0, 12.0)),
        )
        .show(ctx, |ui| {
            paint_swept_panel(ui.painter(), ui.clip_rect(), sidebar_bg(), ui_pal_new().sidebar_bg);
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
                    let local_ac = sweep_color(rect.center(), accent.color());
                    if active {
                        ui.painter().rect_filled(
                            rect,
                            Rounding::same(9.0),
                            Color32::from_rgba_unmultiplied(local_ac.r(), local_ac.g(), local_ac.b(), 38),
                        );
                        ui.painter().rect_stroke(
                            rect,
                            Rounding::same(9.0),
                            Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(
                                    local_ac.r(),
                                    local_ac.g(),
                                    local_ac.b(),
                                    80,
                                ),
                            ),
                        );
                        let bar = Rect::from_min_max(
                            Pos2::new(rect.left() - 6.0, rect.top() + 9.0),
                            Pos2::new(rect.left() - 3.0, rect.bottom() - 9.0),
                        );
                        ui.painter().rect_filled(bar, Rounding::same(1.5), local_ac);
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, Rounding::same(9.0), nav_hover_bg());
                    }

                    let icon_col = if active { local_ac } else { TEXT_MUTED };
                    paint_icon(ui.painter(), rect.shrink(10.0), icon, icon_col);

                    if let Some(count) = badge {
                        let b_pos = Pos2::new(rect.right() - 5.0, rect.top() + 6.0);
                        ui.painter().circle_filled(b_pos, 7.5, local_ac);
                        ui.painter().text(
                            b_pos,
                            egui::Align2::CENTER_CENTER,
                            count.to_string(),
                            FontId::proportional(9.5),
                            contrasting_fg(local_ac),
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
                        let ring_ac = sweep_color(ring_center, accent.color());
                        ui.painter()
                            .add(Shape::line(arc_pts, Stroke::new(2.8_f32, ring_ac)));
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
                let gear_ac = sweep_color(set_rect.center(), accent.color());
                if is_set {
                    ui.painter()
                        .rect_filled(set_rect, Rounding::same(9.0), sweep_soft_bg(set_rect.center(), accent.color()));
                    ui.painter().rect_stroke(
                        set_rect,
                        Rounding::same(9.0),
                        Stroke::new(1.0_f32, gear_ac),
                    );
                } else if set_resp.hovered() {
                    ui.painter()
                        .rect_filled(set_rect, Rounding::same(9.0), nav_hover_bg());
                }
                paint_icon(
                    ui.painter(),
                    set_rect.shrink(10.0),
                    UiIcon::Gear,
                    if is_set { gear_ac } else { TEXT_MUTED },
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
            let extra = if is_game {
                23.0
            } else if is_star {
                20.0
            } else {
                0.0
            };
            let w = (text_w + extra + 22.0).min(156.0);

            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
            let bg = if active {
                Color32::WHITE
            } else if resp.hovered() {
                card_hover()
            } else {
                pill_bg()
            };
            ui.painter().rect_filled(rect, Rounding::same(14.0), bg);
            if !active {
                ui.painter().rect_stroke(
                    rect,
                    Rounding::same(14.0),
                    Stroke::new(1.0_f32, border_col()),
                );
            }

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
                    Vec2::splat(17.0),
                );
                paint_game_badge(ui, state, g_rect, &label, accent.color());
                text_x = rect.left() + 29.0;
            }

            let text_col = if active {
                Color32::from_rgb(14, 18, 26)
            } else {
                TEXT_MUTED
            };
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

                let title = truncate_text(&clip.title, ((card_w - 36.0) / 6.8) as usize);
                let title_galley = ui.painter().layout_no_wrap(
                    title,
                    FontId::proportional(12.5),
                    TEXT_PRIMARY,
                );
                let title_w = (title_galley.size().x + 26.0).min(card_w - 16.0);
                let title_r = Rect::from_min_size(
                    Pos2::new(footer_rect.left() + 8.0, footer_rect.top() + 7.0),
                    Vec2::new(title_w, 22.0),
                );
                let title_resp = ui.interact(title_r, Id::new(("clip_title", clip.id)), Sense::click());
                let title_hov = title_resp.hovered();
                if title_hov {
                    ui.painter().rect_filled(
                        title_r,
                        Rounding::same(5.0),
                        Color32::from_rgba_unmultiplied(255, 255, 255, 16),
                    );
                    ui.painter().rect_stroke(
                        title_r,
                        Rounding::same(5.0),
                        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 26)),
                    );
                }
                ui.painter().galley(
                    Pos2::new(
                        title_r.left() + 4.0,
                        title_r.center().y - title_galley.size().y * 0.5,
                    ),
                    title_galley,
                    TEXT_PRIMARY,
                );
                if title_hov {
                    let pen_r = Rect::from_center_size(
                        Pos2::new(title_r.right() - 10.0, title_r.center().y),
                        Vec2::splat(11.0),
                    );
                    paint_icon(ui.painter(), pen_r, UiIcon::Edit, TEXT_PRIMARY);
                }

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
                } else if title_resp
                    .on_hover_text("Click to rename")
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    state.rename_modal_clip = Some((clip.id, clip.title.clone()));
                    state.rename_modal_just_opened = true;
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

    let query = state.settings_search.trim().to_ascii_lowercase();
    let active_cat = state.settings_category;
    let show_cat = |cat: SettingsCategory, keywords: &str| {
        if query.is_empty() {
            active_cat == cat
        } else {
            cat.label().to_ascii_lowercase().contains(&query) || keywords.contains(&query)
        }
    };

    let full_h = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;

        ui.allocate_ui_with_layout(
            Vec2::new(224.0, full_h),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_min_height(full_h);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let (icon_r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                    let s_ic_col = sweep_color(icon_r.center(), accent.color());
                    let prev_tex = ui
                        .ctx()
                        .data(|d| d.get_temp::<(Color32, egui::TextureHandle)>(Id::new("set_app_ic")));
                    let s_tex = match prev_tex {
                        Some((c, t)) if c == s_ic_col => t,
                        _ => {
                            let rgba = rewind::app_icon::render_icon_rgba(s_ic_col, 128);
                            let img = egui::ColorImage::from_rgba_unmultiplied([128, 128], &rgba);
                            let t = ui.ctx().load_texture(
                                "set_app_icon_tex",
                                img,
                                egui::TextureOptions::LINEAR,
                            );
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(Id::new("set_app_ic"), (s_ic_col, t.clone())));
                            t
                        }
                    };
                    ui.painter().image(
                        s_tex.id(),
                        icon_r,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new("Rewind").size(13.5).strong().color(TEXT_PRIMARY));
                        ui.label(
                            RichText::new(format!(
                                "v{CURRENT_VERSION} • {:.0} MB",
                                state.snapshot.daemon_ram_mb
                            ))
                            .size(11.0)
                            .color(TEXT_MUTED),
                        );
                    });
                });

                ui.add_space(12.0);

                let search_bg_idx = ui.painter().add(Shape::Noop);
                let search_frame = egui::Frame::none()
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::new(1.0_f32, border_col()))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(Margin::symmetric(10.0, 5.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (sr, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                            paint_icon(ui.painter(), sr, UiIcon::Search, TEXT_MUTED);
                            ui.add_space(4.0);
                            ui.add(
                                TextEdit::singleline(&mut state.settings_search)
                                    .desired_width(166.0)
                                    .frame(false)
                                    .hint_text("Find a setting")
                                    .font(FontId::proportional(12.5)),
                            );
                        });
                    });
                let sr_rect = search_frame.response.rect;
                let sr_shapes = build_swept_panel_shapes(sr_rect, pill_bg(), ui_pal_new().pill_bg, 6.0);
                ui.painter().with_clip_rect(sr_rect).set(search_bg_idx, Shape::Vec(sr_shapes));

                ui.add_space(10.0);

                for cat in SettingsCategory::ALL {
                    let active = query.is_empty() && state.settings_category == cat;
                    let (row_r, resp) =
                        ui.allocate_exact_size(Vec2::new(224.0, 34.0), Sense::click());
                    let bg = if active {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 18)
                    } else if resp.hovered() {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 10)
                    } else {
                        Color32::TRANSPARENT
                    };
                    if bg != Color32::TRANSPARENT {
                        ui.painter().rect_filled(row_r, Rounding::same(6.0), bg);
                    }
                    let cat_ac = sweep_color(row_r.center(), accent.color());
                    if active {
                        let pill = Rect::from_min_size(
                            Pos2::new(row_r.left() + 2.0, row_r.center().y - 8.0),
                            Vec2::new(3.0, 16.0),
                        );
                        ui.painter().rect_filled(pill, Rounding::same(1.5), cat_ac);
                    }
                    let cat_icon = match cat {
                        SettingsCategory::Capture => UiIcon::Monitor,
                        SettingsCategory::Clipping => UiIcon::Clock,
                        SettingsCategory::Audio => UiIcon::Speaker,
                        SettingsCategory::Hotkeys => UiIcon::Clock,
                        SettingsCategory::Games => UiIcon::Grid,
                        SettingsCategory::Storage => UiIcon::Folder,
                        SettingsCategory::Personalization => UiIcon::Palette,
                        SettingsCategory::Discord => UiIcon::Codec,
                        SettingsCategory::System => UiIcon::Gear,
                        SettingsCategory::Updates => UiIcon::Bell,
                    };
                    let ic_r = Rect::from_center_size(
                        Pos2::new(row_r.left() + 20.0, row_r.center().y),
                        Vec2::splat(15.0),
                    );
                    let ic_col = if active { cat_ac } else { TEXT_MUTED };
                    paint_icon(ui.painter(), ic_r, cat_icon, ic_col);
                    ui.painter().text(
                        Pos2::new(row_r.left() + 38.0, row_r.center().y),
                        egui::Align2::LEFT_CENTER,
                        cat.label(),
                        FontId::proportional(13.0),
                        if active { TEXT_PRIMARY } else { TEXT_MUTED },
                    );
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        state.settings_category = cat;
                        state.settings_search.clear();
                    }
                }
            },
        );

        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), full_h),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_min_height(full_h);
                smooth_bounce_scroll(ui, "settings_scroll", |ui| {
            let max_w = 740.0_f32.min(ui.available_width());
            ui.vertical(|ui| {
                ui.set_max_width(max_w);
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new("Settings").size(22.0).strong().color(TEXT_MUTED));
                    ui.label(RichText::new(">").size(18.0).color(TEXT_DIM));
                    let page_title = if query.is_empty() {
                        state.settings_category.label().to_owned()
                    } else {
                        format!("Search: \"{}\"", state.settings_search.trim())
                    };
                    ui.label(RichText::new(page_title).size(22.0).strong().color(TEXT_PRIMARY));
                });
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!(
                        "{} ({} • {} MB VRAM)  •  Daemon RAM: {:.1} MB",
                        hw.gpu_name, hw.encoder_backend, hw.vram_mb, state.snapshot.daemon_ram_mb
                    ))
                    .size(12.0)
                    .color(TEXT_MUTED),
                );
                ui.add_space(16.0);

                if show_cat(
                    SettingsCategory::Clipping,
                    "clip length post clip tail replay buffer duration",
                ) {
                    settings_section_header(ui, "Clipping", None);
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
                            UiIcon::Clock,
                            TEXT_MUTED,
                            "Post-Clip Tail",
                            "Keep recording 5–15 seconds after pressing the clip hotkey",
                            |ui| {
                                let tail_label = match state.snapshot.config.clip_post_buffer_secs {
                                    0 => "Instant",
                                    5 => "5 seconds",
                                    10 => "10 seconds",
                                    15 => "15 seconds",
                                    _ => "Instant",
                                };
                                let resp = dropdown_pill(
                                    ui,
                                    tail_label,
                                    None,
                                    None,
                                    state.active_popup == ActivePopup::SettingsPostClipTail,
                                    118.0,
                                );
                                if resp.clicked() {
                                    state.toggle_popup(ActivePopup::SettingsPostClipTail);
                                    pop.toggled_this_frame = true;
                                }
                                if state.active_popup == ActivePopup::SettingsPostClipTail {
                                    pop.anchor = Some(resp.rect);
                                }
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
                }

                if show_cat(
                    SettingsCategory::Capture,
                    "resolution fps codec hevc av1 h264 bitrate cqp gop container encoder",
                ) {
                    settings_section_header(ui, "Capture & Quality", None);
                    settings_card(ui, |ui| {
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
                        let anim_t = ui.ctx().animate_bool_with_time(
                            Id::new("rate_ctrl_cbr_anim"),
                            rc == RateControl::Cbr,
                            0.22,
                        );
                        let show_cbr = anim_t >= 0.5;
                        let alpha = if show_cbr {
                            (anim_t - 0.5) * 2.0
                        } else {
                            (0.5 - anim_t) * 2.0
                        }
                        .clamp(0.0, 1.0);
                        let slide_y = (1.0 - alpha) * 8.0;
                        let right_lbl = if show_cbr {
                            format!("{} Mbps", state.snapshot.config.bitrate_mbps)
                        } else {
                            format!("CQP {}", state.snapshot.config.cqp_qp)
                        };

                        ui.scope(|ui| {
                            ui.multiply_opacity(alpha);
                            if slide_y > 0.1 {
                                ui.add_space(slide_y);
                            }
                            settings_row(
                                ui,
                                UiIcon::Bitrate,
                                TEXT_MUTED,
                                if show_cbr { "Video Bitrate (CBR)" } else { "CQP Quality Target" },
                                if show_cbr {
                                    "Constant target bitrate for CBR streams"
                                } else {
                                    "Lower QP means higher visual fidelity (~18–20 recommended)"
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
                                        let badge_txt = if show_cbr {
                                            format!("~{} MB / clip", est_mb)
                                        } else {
                                            format!("~{} MB (motion)", est_mb)
                                        };
                                        let (b_rect, b_resp) = ui.allocate_exact_size(
                                            Vec2::new(if show_cbr { 88.0 } else { 112.0 }, 22.0),
                                            Sense::hover(),
                                        );
                                        ui.painter().rect_filled(
                                            b_rect,
                                            Rounding::same(5.0),
                                            accent.soft_bg(),
                                        );
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
                                        if !show_cbr {
                                            let _ = b_resp.on_hover_text(
                                                "Estimated for active game motion. Static desktop scenes compress much smaller under CQP.",
                                            );
                                        }
                                    });
                                },
                            );
                            ui.add_space(6.0);
                            if show_cbr {
                                let mut br = state.snapshot.config.bitrate_mbps as f32;
                                if custom_slider(ui, "bitrate_slider", &mut br, 10.0, 80.0, accent.color()) {
                                    state.snapshot.config.bitrate_mbps = br.round() as u32;
                                    cfg_changed = true;
                                }
                            } else {
                                let mut qp = state.snapshot.config.cqp_qp as f32;
                                if custom_slider(ui, "cqp_slider", &mut qp, 14.0, 28.0, accent.color()) {
                                    state.snapshot.config.cqp_qp = qp.round() as u32;
                                    cfg_changed = true;
                                }
                            }
                        });
                        ui.add_space(6.0);

                        settings_divider(ui);

                        settings_row(
                            ui,
                            UiIcon::Codec,
                            accent.color(),
                            "Capture Sharpening (Experimental)",
                            "Lightweight GPU CAS sharpening on recorded frames (~0.1 ms)",
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    for lvl in rewind::config::SharpenLevel::ALL {
                                        let active =
                                            state.snapshot.config.capture_sharpen == lvl;
                                        let bg = if active {
                                            accent.soft_bg()
                                        } else {
                                            Color32::from_rgba_unmultiplied(255, 255, 255, 10)
                                        };
                                        let stroke_col = if active {
                                            accent.color()
                                        } else {
                                            border_col()
                                        };
                                        let txt_col = if active {
                                            accent.color()
                                        } else {
                                            TEXT_MUTED
                                        };
                                        let resp = ui.add(
                                            egui::Button::new(
                                                RichText::new(lvl.label())
                                                    .size(11.5)
                                                    .color(txt_col),
                                            )
                                            .fill(bg)
                                            .stroke(Stroke::new(1.0_f32, stroke_col))
                                            .rounding(Rounding::same(6.0))
                                            .min_size(Vec2::new(56.0, 26.0)),
                                        );
                                        if resp.clicked() && !active {
                                            state.snapshot.config.capture_sharpen = lvl;
                                            cfg_changed = true;
                                        }
                                    }
                                });
                            },
                        );
                    });

                    ui.add_space(18.0);
                }

                if show_cat(
                    SettingsCategory::System,
                    "startup windows auto capture replay close tray onboarding tour",
                ) {
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
                }

                if show_cat(
                    SettingsCategory::Discord,
                    "discord rich presence activity status profile",
                ) {
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
                }

                if show_cat(
                    SettingsCategory::Games,
                    "game detection custom ignored exe executable",
                ) {
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
                }

                if show_cat(
                    SettingsCategory::Audio,
                    "audio speaker microphone mic volume tracks device",
                ) {
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
                }

                if show_cat(
                    SettingsCategory::Hotkeys,
                    "hotkeys keybind save clip mic toggle bookmark",
                ) {
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
                }

                if show_cat(
                    SettingsCategory::Storage,
                    "storage clips folder disk quota delete",
                ) {
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
                }

                if show_cat(
                    SettingsCategory::Personalization,
                    "personalization theme accent color background blur brightness steamgriddb",
                ) {
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
                                    if !bg_path.is_empty() {
                                        let (r_rect, r_resp) =
                                            ui.allocate_exact_size(Vec2::new(56.0, 28.0), Sense::click());
                                        paint_swept_pill_box(ui.painter(), r_rect, 6.0, r_resp.hovered(), false);
                                        ui.painter().text(
                                            r_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "Reset",
                                            FontId::proportional(11.5),
                                            TEXT_PRIMARY,
                                        );
                                        if r_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                            state.start_theme_reveal(
                                                r_rect.center(),
                                                now_secs,
                                                accent.color(),
                                                state.snapshot.config.background_brightness,
                                            );
                                            state.snapshot.config.background_image_path.clear();
                                            state.snapshot.config.auto_adjust_palette = false;
                                            state.snapshot.config.custom_accent_rgb = None;
                                            state.bg_cache_key = None;
                                            state.bg_texture = None;
                                            state.bg_ambient_tint = None;
                                            cfg_changed = true;
                                        }
                                    }
                                    let (b_rect, b_resp) =
                                        ui.allocate_exact_size(Vec2::new(112.0, 28.0), Sense::click());
                                    paint_swept_pill_box(ui.painter(), b_rect, 6.0, b_resp.hovered(), false);
                                    ui.painter().text(
                                        b_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "Choose Image...",
                                        FontId::proportional(11.5),
                                        TEXT_PRIMARY,
                                    );
                                    if b_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                        if let Some(chosen) = pick_image_file_dialog() {
                                            state.start_theme_reveal(
                                                b_rect.center(),
                                                now_secs,
                                                accent.color(),
                                                state.snapshot.config.background_brightness,
                                            );
                                            state.snapshot.config.background_image_path = chosen;
                                            state.bg_cache_key = None;
                                            if let Some(rgb) = state.ensure_background_loaded(ui.ctx()) {
                                                if state.snapshot.config.auto_adjust_palette {
                                                    state.snapshot.config.custom_accent_rgb = Some(rgb);
                                                }
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
                                let blur_col = sweep_color(ui.cursor().right_top(), accent.color());
                                ui.label(
                                    RichText::new(format!("{} px", state.snapshot.config.background_blur))
                                        .size(12.5)
                                        .strong()
                                        .color(blur_col),
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
                                let br_col = sweep_color(ui.cursor().right_top(), accent.color());
                                ui.label(
                                    RichText::new(format!(
                                        "{}%",
                                        state.snapshot.config.background_brightness
                                    ))
                                    .size(12.5)
                                    .strong()
                                    .color(br_col),
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

                        let has_bg = !state.snapshot.config.background_image_path.trim().is_empty();
                        if !has_bg && state.snapshot.config.auto_adjust_palette {
                            state.snapshot.config.auto_adjust_palette = false;
                            cfg_changed = true;
                        }
                        let auto_row = ui.add_enabled_ui(has_bg, |ui| {
                            settings_row(
                                ui,
                                UiIcon::Palette,
                                if has_bg { accent.color() } else { TEXT_DIM },
                                "Auto-Adjust Palette from Background",
                                if has_bg {
                                    "Automatically theme accent and gradients from your chosen image"
                                } else {
                                    "Choose a Background Picture above to enable auto palette"
                                },
                                |ui| {
                                    if toggle_switch(
                                        ui,
                                        "auto_palette_switch",
                                        &mut state.snapshot.config.auto_adjust_palette,
                                        accent.color(),
                                    ) {
                                        state.start_theme_reveal(
                                            ui.min_rect().center(),
                                            now_secs,
                                            accent.color(),
                                            state.snapshot.config.background_brightness,
                                        );
                                        if state.snapshot.config.auto_adjust_palette {
                                            if !state.snapshot.config.background_image_path.is_empty() {
                                                state.bg_cache_key = None;
                                                if let Some(rgb) = state.ensure_background_loaded(ui.ctx()) {
                                                    if state.snapshot.config.auto_adjust_palette {
                                                        state.snapshot.config.custom_accent_rgb = Some(rgb);
                                                    }
                                                }
                                            }
                                        } else {
                                            state.snapshot.config.custom_accent_rgb = None;
                                        }
                                        cfg_changed = true;
                                    }
                                },
                            );
                        });
                        if !has_bg {
                            auto_row
                                .response
                                .on_disabled_hover_text("Choose a Background Picture first");
                        }

                        let show_accent = !state.snapshot.config.auto_adjust_palette;
                        let accent_anim = ui.ctx().animate_bool_with_time(
                            Id::new("accent_row_anim"),
                            show_accent,
                            0.18,
                        );
                        if accent_anim > 0.01 {
                            ui.scope(|ui| {
                                ui.multiply_opacity(accent_anim);
                                settings_divider(ui);

                                settings_row(
                                    ui,
                                    UiIcon::Palette,
                                    accent.color(),
                                    "Windows 11 Accent Color",
                                    "Themes active indicators, buttons, storage ring, and Mica glow",
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 8.0;
                                            for swatch in AccentColor::ALL {
                                                let active = state.snapshot.config.custom_accent_rgb.is_none()
                                                    && state.snapshot.config.accent_color == swatch;
                                                let (s_rect, s_resp) =
                                                    ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                                                if active {
                                                    ui.painter().circle_stroke(
                                                        s_rect.center(),
                                                        12.5,
                                                        Stroke::new(1.8_f32, Color32::WHITE),
                                                    );
                                                }
                                                ui.painter().circle_filled(s_rect.center(), 9.5, swatch.color());
                                                if s_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                                    state.start_theme_reveal(
                                                        s_rect.center(),
                                                        now_secs,
                                                        accent.color(),
                                                        state.snapshot.config.background_brightness,
                                                    );
                                                    state.snapshot.config.accent_color = swatch;
                                                    state.snapshot.config.custom_accent_rgb = None;
                                                    cfg_changed = true;
                                                }
                                            }
                                            let disp_ac = sweep_color(
                                                ui.cursor().right_top() - Vec2::new(18.0, -12.0),
                                                accent.color(),
                                            );
                                            let mut rgb = [
                                                disp_ac.r(),
                                                disp_ac.g(),
                                                disp_ac.b(),
                                            ];
                                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                                state.snapshot.config.custom_accent_rgb = Some(rgb);
                                                cfg_changed = true;
                                            }
                                        });
                                    },
                                );
                            });
                        }

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
                                    let key_bg_idx = ui.painter().add(Shape::Noop);
                                    let key_frame = egui::Frame::none()
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::new(1.0_f32, border_col()))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(Margin::symmetric(8.0, 5.0))
                                        .show(ui, |ui| {
                                            ui.add(
                                                TextEdit::singleline(
                                                    &mut state.snapshot.config.steamgriddb_api_key,
                                                )
                                                .password(true)
                                                .frame(false)
                                                .desired_width(145.0)
                                                .hint_text("Paste API key..."),
                                            )
                                        });
                                    let kf_rect = key_frame.response.rect;
                                    let kf_shapes =
                                        build_swept_panel_shapes(kf_rect, pill_bg(), ui_pal_new().pill_bg, 6.0);
                                    ui.painter()
                                        .with_clip_rect(kf_rect)
                                        .set(key_bg_idx, Shape::Vec(kf_shapes));
                                    let key_resp = key_frame.inner.on_hover_text(
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
                }

                if show_cat(
                    SettingsCategory::Updates,
                    "updates version github release check install",
                ) {
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
                }
            });
        });
            },
        );
    });

    if cfg_changed {
        state.pending_config_update = Some(state.snapshot.config.clone());
        state.pending_config_at = now_secs;
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(80));
    }
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
        ActivePopup::SettingsPostClipTail => (144.0, true),
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
                            if popover_settings_link_btn(ui, "Go to Clipping Settings", accent) {
                                state.settings_category = SettingsCategory::Clipping;
                                state.settings_search.clear();
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
                            if popover_settings_link_btn(ui, "Go to Capture Settings", accent) {
                                state.settings_category = SettingsCategory::Capture;
                                state.settings_search.clear();
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
                        ActivePopup::SettingsPostClipTail => {
                            for (secs, lbl) in [
                                (0_u32, "Instant"),
                                (5, "5 seconds"),
                                (10, "10 seconds"),
                                (15, "15 seconds"),
                            ] {
                                if popover_check_item(
                                    ui,
                                    lbl,
                                    None,
                                    state.snapshot.config.clip_post_buffer_secs == secs,
                                    accent,
                                ) {
                                    state.snapshot.config.clip_post_buffer_secs = secs;
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
        let mut space_toggled = false;
        if ctx.input(|i| i.key_pressed(Key::Space)) {
            state.player_playing = !state.player_playing;
            if let Some(dec) = &state.decoder {
                dec.set_playing(state.player_playing);
            }
            space_toggled = true;
        }
        if space_toggled {
            ctx.data_mut(|d| {
                d.insert_temp(Id::new(("player_hud_active", clip.id)), now_secs);
            });
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
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else if state.player_playing && dur > 0.2 {
            let t_min = state.trim_start_secs.clamp(0.0, dur);
            let t_max = state.trim_end_secs.clamp((t_min + 0.1).min(dur), dur);
            let cur = dec.current_pos_secs().clamp(0.0, dur);
            if cur < t_min - 0.08 || cur >= t_max {
                let seek_id = Id::new("trim_loop_seek_ts");
                let last_seek = ctx.data(|d| d.get_temp::<f64>(seek_id).unwrap_or(-10.0));
                if (now_secs - last_seek) > 0.35 {
                    ctx.data_mut(|d| d.insert_temp(seek_id, now_secs));
                    state.player_pos_secs = t_min;
                    dec.seek(t_min);
                }
            } else {
                state.player_pos_secs = cur;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(6));
        } else if state.player_texture.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    let screen = ctx.screen_rect();
    let modal_thumb = state.get_or_load_thumbnail(ctx, &clip.thumb_path);

    if !state.modal_fullscreen && state.modal_blur_tex.is_none() {
        let mut captured: Option<std::sync::Arc<egui::ColorImage>> = None;
        ctx.input(|i| {
            for ev in &i.raw.events {
                if let egui::Event::Screenshot { image, .. } = ev {
                    captured = Some(image.clone());
                    break;
                }
            }
        });
        if let Some(shot) = captured {
            let [sw, sh] = shot.size;
            let dw = (sw / 2).max(1);
            let dh = (sh / 2).max(1);
            let mut small = vec![0_u8; dw * dh * 4];
            for y in 0..dh {
                let sy = (y * 2).min(sh.saturating_sub(1));
                for x in 0..dw {
                    let sx = (x * 2).min(sw.saturating_sub(1));
                    let c = shot.pixels[sy * sw + sx];
                    let off = (y * dw + x) * 4;
                    small[off] = c.r();
                    small[off + 1] = c.g();
                    small[off + 2] = c.b();
                    small[off + 3] = 255;
                }
            }
            if let Some(buf) = image::RgbaImage::from_raw(dw as u32, dh as u32, small) {
                let blurred = image::imageops::fast_blur(&buf, 2.0);
                let ci = egui::ColorImage::from_rgba_unmultiplied([dw, dh], blurred.as_raw());
                state.modal_blur_tex = Some(ctx.load_texture(
                    "modal_blur_bg",
                    ci,
                    TextureOptions::LINEAR,
                ));
                state.modal_opened_at = now_secs;
            }
        } else if let Some(req_t) = state.modal_blur_requested_at {
            if now_secs - req_t < 0.12 {
                ctx.request_repaint();
                return;
            }
        } else {
            state.modal_blur_requested_at = Some(now_secs);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            ctx.request_repaint();
            return;
        }
    }

    let modal_t = ease_out_cubic(((now_secs - state.modal_opened_at) as f32 / 0.20).clamp(0.0, 1.0));
    if modal_t < 1.0 {
        ctx.request_repaint();
    }
    let modal_w = 820.0_f32.min(screen.width() - 84.0).max(560.0);
    let content_w_est = (modal_w - 28.0).max(320.0);
    let video_h_est = (content_w_est * 9.0 / 16.0).clamp(240.0, (screen.height() - 212.0).max(240.0));
    let modal_h = (video_h_est + 176.0).min(screen.height() - 36.0).max(420.0);
    let modal_center = screen.center() + Vec2::new(0.0, (1.0 - modal_t) * 14.0);
    let modal_rect = Rect::from_center_size(modal_center, Vec2::new(modal_w, modal_h));
    let hud_hit_rect = if state.modal_fullscreen {
        screen
    } else {
        Rect::from_min_size(
            Pos2::new(modal_rect.left() + 14.0, modal_rect.top() + 58.0),
            Vec2::new(content_w_est, video_h_est),
        )
    };

    let hud_id = Id::new(("player_hud_active", clip.id));
    let mut hud_last_active =
        ctx.data_mut(|d| d.get_temp::<f64>(hud_id).unwrap_or(state.modal_opened_at));
    let pointer_moved = ctx.input(|i| {
        let active = i.pointer.delta().length_sq() > 0.5
            || i.pointer.any_pressed()
            || i.pointer.any_released();
        let in_player = i
            .pointer
            .hover_pos()
            .or_else(|| i.pointer.interact_pos())
            .map(|p| hud_hit_rect.contains(p))
            .unwrap_or(false);
        active && in_player
    });
    if pointer_moved {
        hud_last_active = now_secs;
        ctx.data_mut(|d| d.insert_temp(hud_id, hud_last_active));
    }
    let hud_age = (now_secs - hud_last_active).max(0.0) as f32;
    let hud_on = hud_age < 2.0;
    let hud_alpha = ctx.animate_bool_with_time(
        Id::new(("player_hud_vis", clip.id)),
        hud_on,
        if hud_on { 0.14 } else { 0.30 },
    );
    if hud_on {
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(
            (2.0 - hud_age).max(0.016),
        ));
    }
    if hud_alpha > 0.01 && hud_alpha < 0.99 {
        ctx.request_repaint();
    }

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

                render_player_controls_overlay(ui, state, screen, video_rect, dur, ac, hud_alpha);
            });
        return;
    }

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
            if let Some(blur_tex) = &state.modal_blur_tex {
                ui.painter().image(
                    blur_tex.id(),
                    screen,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * modal_t) as u8),
                );
                ui.painter().rect_filled(
                    screen,
                    Rounding::ZERO,
                    Color32::from_rgba_unmultiplied(6, 8, 13, (150.0 * modal_t) as u8),
                );
            } else {
                ui.painter().rect_filled(
                    screen,
                    Rounding::ZERO,
                    Color32::from_rgba_unmultiplied(6, 8, 13, (218.0 * modal_t) as u8),
                );
            }

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

            let modal_shadow = Shadow {
                offset: Vec2::new(0.0, 20.0),
                blur: 48.0,
                spread: 8.0,
                color: Color32::from_rgba_unmultiplied(0, 0, 0, 175),
            };
            ui.painter()
                .add(modal_shadow.as_shape(modal_rect, Rounding::same(8.0)));
            ui.painter().rect_filled(
                modal_rect,
                Rounding::same(8.0),
                Color32::from_rgb(17, 18, 22),
            );
            ui.painter().rect_stroke(
                modal_rect,
                Rounding::same(8.0),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
            );
            ui.painter().line_segment(
                [
                    Pos2::new(modal_rect.left() + 8.0, modal_rect.top() + 1.0),
                    Pos2::new(modal_rect.right() - 8.0, modal_rect.top() + 1.0),
                ],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
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
                                ui.allocate_exact_size(Vec2::new(g_w, 22.0), Sense::hover());
                            ui.painter().rect_filled(
                                g_rect,
                                Rounding::same(8.0),
                                Color32::from_rgb(25, 27, 34),
                            );
                            ui.painter().text(
                                g_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                &clip.game,
                                FontId::proportional(11.0),
                                TEXT_MUTED,
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
                                let galley = ui.painter().layout_no_wrap(
                                    clip.title.clone(),
                                    FontId::proportional(15.0),
                                    TEXT_PRIMARY,
                                );
                                let pad_x = 8.0;
                                let pen_w = 13.0;
                                let gap = 7.0;
                                let total_w =
                                    (galley.size().x + pad_x * 2.0 + gap + pen_w).min(460.0);
                                let (t_rect, title_resp) = ui.allocate_exact_size(
                                    Vec2::new(total_w, 28.0),
                                    Sense::click(),
                                );
                                let hov = title_resp.hovered();
                                if hov {
                                    ui.painter().rect_filled(
                                        t_rect,
                                        Rounding::same(8.0),
                                        Color32::from_rgb(25, 27, 34),
                                    );
                                }
                                ui.painter().galley(
                                    Pos2::new(
                                        t_rect.left() + pad_x,
                                        t_rect.center().y - galley.size().y * 0.5,
                                    ),
                                    galley,
                                    TEXT_PRIMARY,
                                );
                                let pen_rect = Rect::from_center_size(
                                    Pos2::new(t_rect.right() - pad_x - pen_w * 0.5, t_rect.center().y),
                                    Vec2::splat(pen_w),
                                );
                                paint_icon(
                                    ui.painter(),
                                    pen_rect,
                                    UiIcon::Edit,
                                    if hov { TEXT_PRIMARY } else { TEXT_DIM },
                                );
                                if title_resp
                                    .on_hover_text("Click to rename")
                                    .on_hover_cursor(CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    state.rename_buf = clip.title.clone();
                                    state.rename_editing = true;
                                    state.editor_rename_just_opened = true;
                                }
                            }

                            ui.label(
                                RichText::new(format!(
                                    "•  {} {}fps  •  {}  •  {}",
                                    clip.resolution_badge(),
                                    clip.fps,
                                    format_file_size(clip.size_bytes),
                                    clip.relative_time(now_unix)
                                ))
                                .size(11.5)
                                .color(TEXT_DIM),
                            );

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let (cr, cresp) =
                                    ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                                ui.painter().rect_filled(
                                    cr,
                                    Rounding::same(8.0),
                                    if cresp.hovered() {
                                        Color32::from_rgb(25, 27, 34)
                                    } else {
                                        Color32::TRANSPARENT
                                    },
                                );
                                paint_icon(ui.painter(), cr.shrink(8.0), UiIcon::Close, TEXT_MUTED);
                                if cresp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    should_close = true;
                                }
                            });
                        });
                    });

                    let body_rect = Rect::from_min_max(
                        Pos2::new(modal_rect.left() + 14.0, header_r.bottom() + 8.0),
                        Pos2::new(modal_rect.right() - 14.0, modal_rect.bottom() - 14.0),
                    );
                    let content_w = body_rect.width().max(320.0);
                    let video_h = video_h_est;

                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(content_w);
                            ui.spacing_mut().item_spacing.y = 8.0;

                            let (video_rect, _) = ui.allocate_exact_size(
                                Vec2::new(content_w, video_h),
                                Sense::hover(),
                            );
                            let mut controls_bounds = video_rect;
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
                                controls_bounds = fit_rect;
                                paint_rounded_image(
                                    ui.painter(),
                                    tex.id(),
                                    fit_rect,
                                    Rounding::same(8.0),
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                paint_empty_thumb_surface(
                                    ui.painter(),
                                    video_rect,
                                    Rounding::same(8.0),
                                );
                            }

                            if !preload_ready {
                                paint_preload_overlay(ui.painter(), controls_bounds, preload_pct, ac);
                            }

                            let mut overlay_ui =
                                ui.new_child(egui::UiBuilder::new().max_rect(controls_bounds));
                            render_player_controls_overlay(
                                &mut overlay_ui,
                                state,
                                controls_bounds,
                                controls_bounds,
                                dur,
                                ac,
                                hud_alpha,
                            );

                            egui::Frame::none()
                                .inner_margin(Margin::ZERO)
                                .show(ui, |ui| {
                                    ui.set_width(content_w);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 4.0;

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
                                                        Rounding::same(8.0),
                                                        Color32::from_rgb(25, 27, 34),
                                                    );
                                                    ui.painter().text(
                                                        db_r.center(),
                                                        egui::Align2::CENTER_CENTER,
                                                        span_lbl,
                                                        FontId::monospace(11.0),
                                                        TEXT_PRIMARY,
                                                    );
                                                    ui.add_space(6.0);
                                                    let est_sz = ((clip.size_bytes as f32)
                                                        * (span_s / dur).clamp(0.0, 1.0))
                                                    .round()
                                                        as u64;
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "~{}",
                                                            format_file_size(est_sz)
                                                        ))
                                                        .size(11.0)
                                                        .color(TEXT_DIM),
                                                    );
                                                },
                                            );
                                        });

                                        let tl_w = ui.available_width().max(200.0);
                                        let (tl_rect, tl_resp) = ui.allocate_exact_size(
                                            Vec2::new(tl_w, 28.0),
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
                                                        if state.player_pos_secs < state.trim_start_secs {
                                                            state.player_pos_secs = state.trim_start_secs;
                                                            if let Some(dec) = &state.decoder {
                                                                dec.seek(state.trim_start_secs);
                                                            }
                                                        }
                                                    }
                                                    2 => {
                                                        state.trim_end_secs = t_val
                                                            .max((state.trim_start_secs + 0.2).min(dur));
                                                        if state.player_pos_secs > state.trim_end_secs {
                                                            state.player_pos_secs = state.trim_end_secs;
                                                            if let Some(dec) = &state.decoder {
                                                                dec.seek(state.trim_end_secs);
                                                            }
                                                        }
                                                    }
                                                    _ => {
                                                        let clamped = t_val.clamp(
                                                            state.trim_start_secs,
                                                            state.trim_end_secs,
                                                        );
                                                        state.player_pos_secs = clamped;
                                                        if let Some(dec) = &state.decoder {
                                                            dec.seek(clamped);
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
                                            Rounding::same(8.0),
                                            Color32::from_rgb(25, 27, 34),
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
                                                16,
                                            ),
                                        );
                                        ui.painter().line_segment(
                                            [sel_rect.left_top(), sel_rect.right_top()],
                                            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(ac.r(), ac.g(), ac.b(), 140)),
                                        );
                                        ui.painter().line_segment(
                                            [sel_rect.left_bottom(), sel_rect.right_bottom()],
                                            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(ac.r(), ac.g(), ac.b(), 140)),
                                        );

                                        let bar_count = ((inner_w / 5.5).floor() as usize).clamp(32, 140);
                                        for b_idx in 0..bar_count {
                                            let bx = inner_left
                                                + ((b_idx as f32 + 0.5) / bar_count as f32)
                                                    * inner_w;
                                            let seed = ((clip.id as u64)
                                                .wrapping_mul(6364136223846793005)
                                                ^ (b_idx as u64).wrapping_mul(1442695040888963407))
                                            .rotate_left(13);
                                            let amp = 0.25 + ((seed % 100) as f32 / 100.0) * 0.70;
                                            let half_h = (amp * 10.0).max(2.5);
                                            let in_range = bx >= start_x && bx <= end_x;
                                            let bar_col = if in_range {
                                                Color32::from_rgba_unmultiplied(
                                                    ac.r(),
                                                    ac.g(),
                                                    ac.b(),
                                                    220,
                                                )
                                            } else {
                                                Color32::from_rgba_unmultiplied(255, 255, 255, 52)
                                            };
                                            ui.painter().line_segment(
                                                [
                                                    Pos2::new(bx, tl_rect.center().y - half_h),
                                                    Pos2::new(bx, tl_rect.center().y + half_h),
                                                ],
                                                Stroke::new(2.0_f32, bar_col),
                                            );
                                        }

                                        let playhead_x = inner_left
                                            + ((state.player_pos_secs / dur).clamp(0.0, 1.0))
                                                * inner_w;
                                        ui.painter().line_segment(
                                            [
                                                Pos2::new(playhead_x, tl_rect.top() + 2.0),
                                                Pos2::new(playhead_x, tl_rect.bottom() - 2.0),
                                            ],
                                            Stroke::new(1.6_f32, ac),
                                        );
                                        ui.painter().circle_filled(
                                            Pos2::new(playhead_x, tl_rect.top() + 2.0),
                                            3.5,
                                            ac,
                                        );

                                        let grip_col = Color32::from_rgb(40, 46, 58);
                                        for hx in [start_x, end_x] {
                                            let h_rect = Rect::from_center_size(
                                                Pos2::new(hx, tl_rect.center().y),
                                                Vec2::new(handle_w, 28.0),
                                            );
                                            ui.painter().rect_filled(
                                                h_rect,
                                                Rounding::same(5.0),
                                                Color32::WHITE,
                                            );
                                            ui.painter().line_segment(
                                                [
                                                    Pos2::new(hx, h_rect.top() + 8.0),
                                                    Pos2::new(hx, h_rect.bottom() - 8.0),
                                                ],
                                                Stroke::new(1.6_f32, grip_col),
                                            );
                                        }
                                        let _ = tl_resp
                                            .on_hover_cursor(CursorIcon::ResizeHorizontal);
                                    });
                                });

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
                                        Color32::TRANSPARENT
                                    },
                                    Color32::TRANSPARENT,
                                    if clip.favorite { STAR_GOLD } else { TEXT_MUTED },
                                ) {
                                    state.send_sync(
                                        IpcRequest::ToggleFavorite { id: clip.id },
                                        now_secs,
                                    );
                                }

                                if modal_action_pill(
                                    ui,
                                    UiIcon::Library,
                                    "Copy Video",
                                    TEXT_PRIMARY,
                                    Color32::TRANSPARENT,
                                    Color32::TRANSPARENT,
                                    TEXT_MUTED,
                                ) {
                                    if copy_file_to_clipboard(&clip.file_path) {
                                        state.status_banner = Some((
                                            "Copied video file to clipboard".to_owned(),
                                            now_secs,
                                        ));
                                    } else {
                                        state.status_banner = Some((
                                            "Failed to copy video to clipboard".to_owned(),
                                            now_secs,
                                        ));
                                    }
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
                                } else {
                                    Color32::TRANSPARENT
                                };
                                let disc_stroke = Color32::TRANSPARENT;
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

                                let is_trimmed = state.trim_start_secs > 0.05
                                    || (dur - state.trim_end_secs) > 0.05;
                                let save_vis = ui.ctx().animate_bool_with_time(
                                    Id::new(("save_trim_vis", clip.id)),
                                    is_trimmed,
                                    0.18,
                                );
                                if save_vis > 0.02 {
                                    let save_fg = Color32::from_rgb(14, 18, 26);
                                    let mut clicked_save = false;
                                    ui.scope(|ui| {
                                        ui.multiply_opacity(save_vis);
                                        clicked_save = modal_action_pill_ex(
                                            ui,
                                            UiIcon::Check,
                                            "Save Trim",
                                            save_fg,
                                            Color32::WHITE,
                                            Color32::TRANSPARENT,
                                            save_fg,
                                            save_vis > 0.5,
                                            None,
                                        );
                                    });
                                    if clicked_save {
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
                                }

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let del_col = Color32::from_rgb(244, 63, 94);
                                    if modal_action_pill(
                                        ui,
                                        UiIcon::Trash,
                                        "Delete",
                                        del_col,
                                        Color32::TRANSPARENT,
                                        Color32::TRANSPARENT,
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
    let draw_fill = if enabled && resp.hovered() {
        if fill == Color32::TRANSPARENT {
            Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 20)
        } else if fill == pill_bg() {
            card_hover()
        } else {
            fill
        }
    } else {
        fill
    };
    let ic_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 17.0, rect.center().y),
        Vec2::splat(13.0),
    );
    let draw_pill = |painter: &egui::Painter, p_fill: Color32, p_stroke: Color32, p_fg: Color32, p_ic: Color32| {
        let p_rect = if p_fill == Color32::WHITE {
            rect.shrink2(Vec2::new(0.0, 1.5))
        } else {
            rect
        };
        painter.rect_filled(p_rect, Rounding::same(8.0), p_fill);
        if p_stroke.a() > 0 {
            painter.rect_stroke(p_rect, Rounding::same(8.0), Stroke::new(1.0_f32, p_stroke));
        }
        paint_icon(painter, ic_rect, icon, p_ic);
        painter.text(
            Pos2::new(ic_rect.right() + 7.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            p_fg,
        );
    };
    if let Some((origin, radius, old_ac, new_ac)) = THEME_SWEEP.with(|c| c.get()) {
        let (p_old_fill, p_new_fill, p_old_st, p_new_st, p_old_fg, p_new_fg) = if fill == new_ac {
            let old_fg = contrasting_fg(old_ac);
            let new_fg = contrasting_fg(new_ac);
            (old_ac, new_ac, old_ac, new_ac, old_fg, new_fg)
        } else if fill == pill_bg() {
            (
                draw_fill,
                if enabled && resp.hovered() { ui_pal_new().card_hover } else { ui_pal_new().pill_bg },
                stroke,
                if stroke.a() > 0 { ui_pal_new().border } else { stroke },
                fg,
                fg,
            )
        } else {
            (draw_fill, draw_fill, stroke, stroke, fg, fg)
        };
        if p_old_fill != p_new_fill {
            let p_old_ic = if fill == new_ac { p_old_fg } else { icon_col };
            let p_new_ic = if fill == new_ac { p_new_fg } else { icon_col };
            let p = ui.painter().with_clip_rect(rect);
            for s in build_swept_panel_shapes(rect, p_old_fill, p_new_fill, 8.0) {
                p.add(s);
            }
            if p_old_st.a() > 0 || p_new_st.a() > 0 {
                let st_col = if p_old_st == p_new_st {
                    p_old_st
                } else {
                    let dist = rect.center().distance(origin);
                    let blend = ((radius - dist) * 0.5).clamp(0.0, 1.0);
                    lerp_color(p_old_st, p_new_st, blend)
                };
                if st_col.a() > 0 {
                    ui.painter().rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0_f32, st_col));
                }
            }
            let ic_col = if p_old_ic == p_new_ic {
                p_old_ic
            } else {
                let dist = ic_rect.center().distance(origin);
                let blend = ((radius - dist) * 0.5).clamp(0.0, 1.0);
                lerp_color(p_old_ic, p_new_ic, blend)
            };
            let fg_col = if p_old_fg == p_new_fg {
                p_old_fg
            } else {
                let dist = rect.center().distance(origin);
                let blend = ((radius - dist) * 0.5).clamp(0.0, 1.0);
                lerp_color(p_old_fg, p_new_fg, blend)
            };
            paint_icon(ui.painter(), ic_rect, icon, ic_col);
            ui.painter().text(
                Pos2::new(ic_rect.right() + 7.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                FontId::proportional(12.0),
                fg_col,
            );
        } else {
            draw_pill(ui.painter(), draw_fill, stroke, fg, icon_col);
        }
    } else {
        draw_pill(ui.painter(), draw_fill, stroke, fg, icon_col);
    }
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
    let hov_t = ui.ctx().animate_bool_with_time(
        Id::new(("modal_nav_hov", icon as u8)),
        enabled && resp.hovered(),
        0.14,
    );
    let bg = if enabled {
        lerp_color(
            Color32::from_rgba_unmultiplied(20, 22, 28, 178),
            Color32::from_rgba_unmultiplied(35, 38, 48, 217),
            hov_t,
        )
    } else {
        Color32::from_rgba_unmultiplied(20, 22, 28, 110)
    };
    let radius = 17.0 + hov_t * 1.5;
    let border_a = if enabled {
        (20.0 + hov_t * 14.0) as u8
    } else {
        12
    };
    ui.painter().circle_filled(rect.center(), radius, bg);
    ui.painter().circle_stroke(
        rect.center(),
        radius,
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, border_a)),
    );
    let ic_col = if enabled {
        Color32::from_rgba_unmultiplied(
            TEXT_PRIMARY.r(),
            TEXT_PRIMARY.g(),
            TEXT_PRIMARY.b(),
            (178.0 + hov_t * 77.0) as u8,
        )
    } else {
        Color32::from_rgba_unmultiplied(TEXT_DIM.r(), TEXT_DIM.g(), TEXT_DIM.b(), 120)
    };
    paint_icon(
        ui.painter(),
        rect.shrink(10.0 - hov_t * 0.8),
        icon,
        ic_col,
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

    let (msg_opt, banner_age) = if let Some(pct) = enc_pct {
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
        (Some(format!("Encoding for Discord (<10 MB)... {pct}%")), None)
    } else if let Some((msg, t)) = &state.status_banner {
        let age = (now_secs - *t).max(0.0) as f32;
        (Some(msg.clone()), Some(age))
    } else if undo_id.is_some() {
        (Some("Clip moved to trash".to_owned()), None)
    } else {
        (None, None)
    };

    let spring_id = Id::new("status_toast_spring");
    let Some(msg) = msg_opt else {
        ctx.data_mut(|d| d.insert_temp(spring_id, (48.0_f32, 0.0_f32)));
        return;
    };

    let (y_offset, alpha) = if let Some(age) = banner_age {
        let exit_start = 1.25_f32;
        let exiting = undo_id.is_none() && age >= exit_start;
        let y_target = if exiting { 52.0_f32 } else { -22.0_f32 };
        let (stiffness, damping) = if exiting {
            (240.0_f32, 28.0_f32)
        } else {
            (320.0_f32, 22.0_f32)
        };
        let dt = ctx.input(|i| i.stable_dt).clamp(0.001, 0.033);
        let (mut y_pos, mut y_vel) =
            ctx.data_mut(|d| d.get_temp::<(f32, f32)>(spring_id).unwrap_or((48.0, 0.0)));
        let accel = stiffness * (y_target - y_pos) - damping * y_vel;
        y_vel += accel * dt;
        y_pos += y_vel * dt;
        if exiting && y_pos >= 44.0 {
            state.status_banner = None;
            ctx.data_mut(|d| d.insert_temp(spring_id, (48.0_f32, 0.0_f32)));
            return;
        }
        ctx.data_mut(|d| d.insert_temp(spring_id, (y_pos, y_vel)));
        let settled = !exiting && (y_pos - y_target).abs() < 0.15 && y_vel.abs() < 1.0;
        if settled {
            if undo_id.is_none() {
                ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                    (exit_start - age).max(0.008),
                ));
            }
        } else {
            ctx.request_repaint();
        }
        (y_pos, 1.0_f32)
    } else {
        let toast_t = anim_open_progress(
            ctx,
            Id::new("status_toast_anim"),
            if enc_pct.is_some() { 1 } else { 2 },
            now_secs,
            0.22,
        );
        let y_pos = -22.0 + (1.0 - toast_t) * 28.0;
        ctx.data_mut(|d| d.insert_temp(spring_id, (y_pos, 0.0_f32)));
        (y_pos, toast_t)
    };

    egui::Area::new(Id::new("status_toast"))
        .order(Order::Tooltip)
        .constrain(false)
        .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, y_offset))
        .show(ctx, |ui| {
            ui.set_opacity(alpha.clamp(0.0, 1.0));
            let toast_bg_idx = ui.painter().add(Shape::Noop);
            let toast_ac = sweep_color(
                Pos2::new(ctx.screen_rect().center().x, ctx.screen_rect().bottom() + y_offset - 18.0),
                accent.color(),
            );
            let toast_frame = egui::Frame::none()
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0_f32, toast_ac))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(14.0, 8.0))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(msg).size(12.0).color(TEXT_PRIMARY));
                            if enc_pct.is_some() {
                                ui.add_space(10.0);
                                let cancel_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Cancel").size(12.0).color(toast_ac),
                                    )
                                    .frame(false),
                                );
                                if cancel_btn
                                    .on_hover_cursor(CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    state.cancel_discord_encode(now_secs);
                                }
                            } else if let Some(id) = undo_id {
                                ui.add_space(10.0);
                                if ui
                                    .button(RichText::new("Undo").size(12.0).color(toast_ac))
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
                                .rect_filled(fill_r, Rounding::same(2.0), toast_ac);
                        }
                    });
                });
            let tf_rect = toast_frame.response.rect;
            let tf_shapes =
                build_swept_panel_shapes(tf_rect, popover_bg(), ui_pal_new().popover_bg, 8.0);
            ui.painter()
                .with_clip_rect(tf_rect)
                .set(toast_bg_idx, Shape::Vec(tf_shapes));
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

    let hovered = popup_open || main_resp.hovered() || chev_resp.hovered();
    let base_bg = if hovered { card_hover() } else { pill_bg() };
    let new_pal = ui_pal_new();
    let new_base_bg = if hovered { new_pal.card_hover } else { new_pal.pill_bg };
    let base_border = if popup_open { border_bright() } else { border_col() };
    let bg = lerp_color(base_bg, accent_color, flash_alpha * 0.28);
    let new_bg = lerp_color(new_base_bg, accent_color, flash_alpha * 0.28);
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
    paint_swept_rounded_rect(ui.painter(), rect, 7.0, bg, new_bg, Stroke::new(1.0_f32, border));
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
    paint_swept_pill_box(ui.painter(), rect, 7.0, active || resp.hovered(), active);

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
    let av_ac = sweep_color(rect.center(), accent.color());
    let fill = if active || resp.hovered() {
        Color32::from_rgba_unmultiplied(av_ac.r(), av_ac.g(), av_ac.b(), 36)
    } else {
        Color32::from_rgb(24, 34, 52)
    };
    let border = if active {
        av_ac
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
        av_ac,
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
    let old_bg = card_bg();
    let new_bg = ui_pal_new().card_bg;
    let bg_idx = ui.painter().add(Shape::Noop);
    let resp = egui::Frame::none()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0_f32, border_col()))
        .rounding(Rounding::same(7.0))
        .inner_margin(Margin::symmetric(16.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
    let card_rect = resp.response.rect;
    let shapes = build_swept_panel_shapes(card_rect, old_bg, new_bg, 7.0);
    ui.painter().with_clip_rect(card_rect).set(bg_idx, Shape::Vec(shapes));
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
        paint_icon(
            ui.painter(),
            icon_box.shrink(8.0),
            icon,
            sweep_color(icon_box.center(), icon_color),
        );

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
        let anim_t = ui
            .ctx()
            .animate_bool_with_time(Id::new(("toggle_anim", id_salt)), *on, 0.14);
        let local_ac = sweep_color(rect.center(), accent);
        let bg = lerp_color(track_bg(), local_ac, anim_t);
        ui.painter().rect_filled(rect, Rounding::same(10.0), bg);
        let x0 = rect.left() + 10.0;
        let x1 = rect.right() - 10.0;
        let knob_x = x0 + (x1 - x0) * anim_t;
        let knob_col = lerp_color(Color32::WHITE, contrasting_fg(local_ac), anim_t);
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
    custom_slider_ex(ui, id_salt, val, min, max, accent, 2.0, 6.5)
}

fn custom_slider_ex(
    ui: &mut Ui,
    id_salt: &str,
    val: &mut f32,
    min: f32,
    max: f32,
    accent: Color32,
    track_half_h: f32,
    thumb_base_r: f32,
) -> bool {
    ui.push_id(id_salt, |ui| {
        let w = ui.available_width().max(60.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::click_and_drag());
        let track_r = Rect::from_min_max(
            Pos2::new(rect.left() + 6.0, rect.center().y - track_half_h),
            Pos2::new(rect.right() - 6.0, rect.center().y + track_half_h),
        );
        let mut changed = false;
        let span = (max - min).max(0.0001);
        let mut raw_frac = None;
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let t = ((pos.x - track_r.left()) / track_r.width()).clamp(0.0, 1.0);
                *val = min + t * span;
                raw_frac = Some(t);
                changed = true;
            }
        }
        let target_frac = raw_frac.unwrap_or_else(|| ((*val - min) / span).clamp(0.0, 1.0));
        let anim_time = if resp.dragged() { 0.0 } else { 0.12 };
        let vis_frac = ui
            .ctx()
            .animate_value_with_time(Id::new(("slider_vis", id_salt)), target_frac, anim_time)
            .clamp(0.0, 1.0);
        let thumb_x = track_r.left() + vis_frac * track_r.width();
        let active_scale = ui.ctx().animate_bool_with_time(
            Id::new(("slider_hover", id_salt)),
            resp.hovered() || resp.dragged(),
            0.12,
        );
        let thumb_r = thumb_base_r + active_scale * 1.2;

        let local_ac = sweep_color(Pos2::new(thumb_x, rect.center().y), accent);
        ui.painter()
            .rect_filled(track_r, Rounding::same(track_half_h), track_bg());
        let active_r = Rect::from_min_max(track_r.left_top(), Pos2::new(thumb_x, track_r.bottom()));
        if let Some((origin, radius, old_ac, new_ac)) = THEME_SWEEP.with(|c| c.get()).filter(|(_, _, _, na)| *na == accent) {
            ui.painter()
                .rect_filled(active_r, Rounding::same(track_half_h), old_ac);
            let dy = (rect.center().y - origin.y).abs();
            if dy < radius {
                let dx = (radius * radius - dy * dy).sqrt();
                let ix0 = (origin.x - dx).max(active_r.left());
                let ix1 = (origin.x + dx).min(active_r.right());
                if ix1 > ix0 {
                    let clip_in = Rect::from_min_max(
                        Pos2::new(ix0, active_r.top() - 1.0),
                        Pos2::new(ix1, active_r.bottom() + 1.0),
                    );
                    ui.painter()
                        .with_clip_rect(clip_in)
                        .rect_filled(active_r, Rounding::same(track_half_h), new_ac);
                }
            }
        } else {
            ui.painter()
                .rect_filled(active_r, Rounding::same(track_half_h), local_ac);
        }

        if thumb_r > 0.5 {
            ui.painter()
                .circle_filled(Pos2::new(thumb_x, rect.center().y), thumb_r, Color32::WHITE);
            ui.painter().circle_stroke(
                Pos2::new(thumb_x, rect.center().y),
                thumb_r,
                Stroke::new((thumb_base_r * 0.34).max(1.4), local_ac),
            );
        }

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
    hud_alpha: f32,
) {
    if hud_alpha > 0.01 {
        let vig_top = (ctrl_bounds.bottom() - 72.0).max(ctrl_bounds.top());
        let vig_mid = (ctrl_bounds.bottom() - 32.0).max(vig_top);
        let mut vig = egui::Mesh::default();
        let clear = Color32::from_rgba_unmultiplied(0, 0, 0, 0);
        let mid = Color32::from_rgba_unmultiplied(0, 0, 0, (92.0 * hud_alpha) as u8);
        let dark = Color32::from_rgba_unmultiplied(0, 0, 0, (204.0 * hud_alpha) as u8);
        vig.colored_vertex(Pos2::new(ctrl_bounds.left(), vig_top), clear);
        vig.colored_vertex(Pos2::new(ctrl_bounds.right(), vig_top), clear);
        vig.colored_vertex(Pos2::new(ctrl_bounds.right(), vig_mid), mid);
        vig.colored_vertex(Pos2::new(ctrl_bounds.left(), vig_mid), mid);
        vig.colored_vertex(ctrl_bounds.right_bottom(), dark);
        vig.colored_vertex(ctrl_bounds.left_bottom(), dark);
        vig.add_triangle(0, 1, 2);
        vig.add_triangle(0, 2, 3);
        vig.add_triangle(3, 2, 4);
        vig.add_triangle(3, 4, 5);
        ui.painter().add(vig);
    }

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

    if hud_alpha <= 0.01 {
        return;
    }

    ui.scope(|ui| {
        ui.multiply_opacity(hud_alpha);
        let scrub_rect = Rect::from_min_max(
            Pos2::new(ctrl_bounds.left() + 14.0, ctrl_bounds.bottom() - 56.0),
            Pos2::new(ctrl_bounds.right() - 14.0, ctrl_bounds.bottom() - 40.0),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(scrub_rect), |ui| {
            ui.set_opacity(hud_alpha);
            let t_min = state.trim_start_secs.clamp(0.0, dur);
            let t_max = state.trim_end_secs.clamp((t_min + 0.1).min(dur), dur);
            let mut pos = state.player_pos_secs.clamp(t_min, t_max);
            if custom_slider_ex(ui, "player_scrub", &mut pos, 0.0, dur, ac, 1.25, 4.0) {
                let clamped = pos.clamp(t_min, t_max);
                state.player_pos_secs = clamped;
                if let Some(dec) = &state.decoder {
                    dec.seek(clamped);
                }
            }
        });

        let ctrl_rect = Rect::from_min_max(
            Pos2::new(ctrl_bounds.left() + 14.0, ctrl_bounds.bottom() - 36.0),
            Pos2::new(ctrl_bounds.right() - 14.0, ctrl_bounds.bottom() - 8.0),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(ctrl_rect), |ui| {
        ui.set_opacity(hud_alpha);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let (pp_r, pp_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
            ui.painter().rect_filled(
                pp_r,
                Rounding::same(8.0),
                if pp_resp.hovered() {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 24)
                } else {
                    Color32::TRANSPARENT
                },
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
                Rounding::same(8.0),
                if sp_resp.hovered() {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 24)
                } else {
                    Color32::TRANSPARENT
                },
            );
            paint_icon(
                ui.painter(),
                sp_r.shrink(7.0),
                UiIcon::Speaker,
                if state.player_muted { TEXT_DIM } else { TEXT_PRIMARY },
            );
            if sp_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.player_muted = !state.player_muted;
                if !state.player_muted && state.player_volume <= 0.01 {
                    state.player_volume = 0.85;
                }
            }

            ui.allocate_ui(Vec2::new(82.0, 18.0), |ui| {
                let mut vol = if state.player_muted {
                    0.0
                } else {
                    state.player_volume
                };
                if custom_slider_ex(ui, "player_vol_slider", &mut vol, 0.0, 1.0, ac, 1.25, 4.0) {
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
                    Rounding::same(8.0),
                    if fs_resp.hovered() {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 24)
                    } else {
                        Color32::TRANSPARENT
                    },
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
    let menu_size = Vec2::new(196.0, 128.0);
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
                .fill(Color32::from_rgba_unmultiplied(18, 20, 26, 232))
                .stroke(Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 20),
                ))
                .rounding(Rounding::same(9.0))
                .inner_margin(Margin::same(5.0))
                .shadow(Shadow {
                    offset: Vec2::new(0.0, 14.0),
                    blur: 32.0,
                    spread: 4.0,
                    color: Color32::from_rgba_unmultiplied(0, 0, 0, (175.0 * anim_t) as u8),
                })
                .show(ui, |ui| {
                    ui.set_min_size(menu_size - Vec2::splat(10.0));
                    ui.set_max_width(menu_size.x - 10.0);
                    ui.spacing_mut().item_spacing.y = 2.0;

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

                    let (div_r, _) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 5.0), Sense::hover());
                    ui.painter().line_segment(
                        [
                            Pos2::new(div_r.left() + 4.0, div_r.center().y),
                            Pos2::new(div_r.right() - 4.0, div_r.center().y),
                        ],
                        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
                    );

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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), sense);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(
            rect,
            Rounding::same(6.0),
            Color32::from_rgba_unmultiplied(255, 255, 255, 15),
        );
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
    let modal_h = 160.0_f32;
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
                egui::UiBuilder::new().max_rect(modal_rect.shrink2(Vec2::new(22.0, 18.0))),
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
                        ui.add_space(16.0);
                        let chk_id = Id::new("dont_remind_close_chk");
                        let mut dont_remind = ui
                            .ctx()
                            .data_mut(|d| d.get_temp::<bool>(chk_id).unwrap_or(false));
                        ui.horizontal_centered(|ui| {
                            let (c_rect, c_resp) =
                                ui.allocate_exact_size(Vec2::new(166.0, 32.0), Sense::click());
                            let box_r = Rect::from_center_size(
                                Pos2::new(c_rect.left() + 9.0, c_rect.center().y),
                                Vec2::splat(16.0),
                            );
                            ui.painter().rect_filled(
                                box_r,
                                Rounding::same(4.0),
                                if dont_remind { ac } else { pill_bg() },
                            );
                            ui.painter().rect_stroke(
                                box_r,
                                Rounding::same(4.0),
                                Stroke::new(1.0_f32, if dont_remind { ac } else { border_col() }),
                            );
                            if dont_remind {
                                paint_icon(
                                    ui.painter(),
                                    box_r.shrink(3.0),
                                    UiIcon::Check,
                                    contrasting_fg(ac),
                                );
                            }
                            ui.painter().text(
                                Pos2::new(box_r.right() + 8.0, c_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                "Don't remind me again",
                                FontId::proportional(12.0),
                                TEXT_MUTED,
                            );
                            if c_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                dont_remind = !dont_remind;
                                ui.ctx().data_mut(|d| d.insert_temp(chk_id, dont_remind));
                            }

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
                                    if dont_remind {
                                        state.snapshot.config.suppress_recording_close_prompt =
                                            true;
                                        state.push_config_update(ctx.input(|i| i.time));
                                    }
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
        let rad = rect.width().min(rect.height()) * 0.5;
        paint_rounded_image(
            ui.painter(),
            tex.id(),
            rect,
            Rounding::same(rad),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.painter().circle_stroke(
            rect.center(),
            rad,
            Stroke::new(0.8_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
        );
    } else {
        paint_game_badge_initial(ui.painter(), rect, game_name, accent);
    }
}

fn paint_preload_overlay(painter: &egui::Painter, rect: Rect, _pct: f32, accent: Color32) {
    let t = painter.ctx().input(|i| i.time);
    painter.ctx().request_repaint();

    let sb = sidebar_bg();
    painter.rect_filled(
        rect,
        Rounding::same(12.0),
        Color32::from_rgba_unmultiplied(sb.r(), sb.g(), sb.b(), 234),
    );

    let center = rect.center() - Vec2::new(0.0, 12.0);
    let r = 20.0_f32;
    painter.circle_stroke(
        center,
        r,
        Stroke::new(3.5_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
    );

    let tau = std::f32::consts::TAU;
    let base_rot = ((t / 2.0) as f32).fract() * tau;
    let phase = ((t / 1.45) as f32).fract();
    let ease = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let (arc_span, arc_off) = if phase < 0.5 {
        let u = ease(phase * 2.0);
        (0.03 + u * 0.60, u * 0.24)
    } else {
        let u = ease((phase - 0.5) * 2.0);
        (0.63 - u * 0.60, 0.24 + u * 0.76)
    };

    let start_a = base_rot + arc_off * tau;
    let sweep = arc_span * tau;
    let steps = 32_usize;
    let pts: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = start_a + (i as f32 / steps as f32) * sweep;
            Pos2::new(center.x + a.cos() * r, center.y + a.sin() * r)
        })
        .collect();
    if let (Some(&first), Some(&last)) = (pts.first(), pts.last()) {
        painter.circle_filled(first, 1.75, accent);
        painter.circle_filled(last, 1.75, accent);
    }
    painter.add(egui::Shape::line(pts, Stroke::new(3.5_f32, accent)));

    painter.text(
        center + Vec2::new(0.0, 36.0),
        egui::Align2::CENTER_CENTER,
        "Loading",
        FontId::proportional(13.0),
        TEXT_MUTED,
    );
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
                                            state.push_config_update(ctx.input(|i| i.time));
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
                                                state.push_config_update(ctx.input(|i| i.time));
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
                                            state.push_config_update(ctx.input(|i| i.time));
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
                                        state.push_config_update(ctx.input(|i| i.time));
                                        state.tour_step = Some(0);
                                        state.active_tab = NavTab::Home;
                                    }
                                    ui.add_space(8.0);
                                    if onboarding_pill(ui, "Skip Tour", false, ac) {
                                        state.snapshot.config.onboarding_completed = true;
                                        state.push_config_update(ctx.input(|i| i.time));
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

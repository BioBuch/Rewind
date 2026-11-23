use egui::{
    Color32, FontId, Mesh, Painter, Pos2, Rect, Rounding, Shape, Stroke, TextureHandle, TextureId,
    Vec2,
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

#[derive(Debug, Clone, Copy)]
pub enum UiIcon {
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Check,
    Home,
    Library,
    StarOutline,
    StarFilled,
    Folder,
    Gear,
    Search,
    Close,
    Bell,
    Minimize,
    Maximize,
    Clock,
    Monitor,
    Codec,
    Bitrate,
    Speaker,
    Mic,
    AudioTracks,
    Trash,
    Database,
    Palette,
    Grid,
    List,
    Play,
    Pause,
    Scissors,
    Link,
    Edit,
}

pub fn paint_fluent_background(
    ctx: &egui::Context,
    accent_color: Color32,
    bg_tex: Option<&TextureHandle>,
    _ambient_tint: Option<Color32>,
    brightness_pct: u32,
) {
    let rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    if let Some(tex) = bg_tex {
        painter.rect_filled(rect, Rounding::ZERO, Color32::BLACK);
        let [tw, th] = tex.size();
        let tex_ar = (tw.max(1) as f32) / (th.max(1) as f32);
        let win_ar = (rect.width().max(1.0)) / (rect.height().max(1.0));
        let uv = if tex_ar > win_ar {
            let half = (win_ar / tex_ar) * 0.5;
            Rect::from_min_max(Pos2::new(0.5 - half, 0.0), Pos2::new(0.5 + half, 1.0))
        } else {
            let half = (tex_ar / win_ar) * 0.5;
            Rect::from_min_max(Pos2::new(0.0, 0.5 - half), Pos2::new(1.0, 0.5 + half))
        };
        let b = ((brightness_pct.clamp(5, 100) as f32 / 100.0) * 255.0)
            .round()
            .clamp(12.0, 255.0) as u8;
        painter.image(tex.id(), rect, uv, Color32::from_rgb(b, b, b));

        let mut mesh = Mesh::default();
        mesh.colored_vertex(rect.left_top(), Color32::from_rgba_unmultiplied(0, 0, 0, 55));
        mesh.colored_vertex(rect.right_top(), Color32::from_rgba_unmultiplied(0, 0, 0, 40));
        mesh.colored_vertex(rect.right_bottom(), Color32::from_rgba_unmultiplied(0, 0, 0, 25));
        mesh.colored_vertex(rect.left_bottom(), Color32::from_rgba_unmultiplied(0, 0, 0, 50));
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
        return;
    }

    let base = Color32::from_rgb(11, 14, 20);
    let ac = accent_color;
    let tl = Color32::from_rgb(
        (base.r() as u16 + (ac.r() as u16 / 10)) as u8,
        (base.g() as u16 + (ac.g() as u16 / 10)) as u8,
        (base.b() as u16 + (ac.b() as u16 / 9)) as u8,
    );
    let tr = Color32::from_rgb(16, 21, 31);
    let br = Color32::from_rgb(10, 13, 19);
    let bl = Color32::from_rgb(12, 15, 22);

    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), tl);
    mesh.colored_vertex(rect.right_top(), tr);
    mesh.colored_vertex(rect.right_bottom(), br);
    mesh.colored_vertex(rect.left_bottom(), bl);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

pub fn paint_empty_thumb_surface(painter: &Painter, rect: Rect, rounding: Rounding) {
    painter.rect_filled(rect, rounding, Color32::from_rgb(20, 25, 36));
    let c = rect.center();
    let r = (rect.height() * 0.16).clamp(10.0, 20.0);
    painter.circle_stroke(c, r, Stroke::new(1.4_f32, Color32::from_rgb(68, 79, 102)));
    paint_icon(
        painter,
        Rect::from_center_size(c, Vec2::splat(r * 1.1)),
        UiIcon::Play,
        Color32::from_rgb(118, 130, 154),
    );
}

pub fn paint_game_badge_initial(painter: &Painter, rect: Rect, game_name: &str, accent: Color32) {
    let rad = rect.width().min(rect.height()) * 0.5;
    painter.circle_filled(rect.center(), rad, Color32::from_rgb(26, 34, 49));
    painter.circle_stroke(rect.center(), rad, Stroke::new(1.0_f32, Color32::from_rgb(48, 60, 84)));
    let ch = game_name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('G');
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ch.to_string(),
        FontId::proportional((rect.height() * 0.62).clamp(9.0, 14.0)),
        accent,
    );
}

pub fn paint_icon(painter: &Painter, rect: Rect, icon: UiIcon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new((s * 0.16).clamp(1.2, 2.0), color);

    match icon {
        UiIcon::ChevronLeft => {
            let d = s * 0.45;
            painter.line_segment(
                [Pos2::new(c.x + d * 0.5, c.y - d), Pos2::new(c.x - d * 0.5, c.y)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - d * 0.5, c.y), Pos2::new(c.x + d * 0.5, c.y + d)],
                stroke,
            );
        }
        UiIcon::ChevronRight => {
            let d = s * 0.45;
            painter.line_segment(
                [Pos2::new(c.x - d * 0.5, c.y - d), Pos2::new(c.x + d * 0.5, c.y)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x + d * 0.5, c.y), Pos2::new(c.x - d * 0.5, c.y + d)],
                stroke,
            );
        }
        UiIcon::ChevronDown => {
            let d = s * 0.45;
            painter.line_segment(
                [Pos2::new(c.x - d, c.y - d * 0.35), Pos2::new(c.x, c.y + d * 0.45)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x, c.y + d * 0.45), Pos2::new(c.x + d, c.y - d * 0.35)],
                stroke,
            );
        }
        UiIcon::Check => {
            let d = s * 0.6;
            painter.line_segment(
                [Pos2::new(c.x - d * 0.85, c.y + d * 0.05), Pos2::new(c.x - d * 0.15, c.y + d * 0.7)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - d * 0.15, c.y + d * 0.7), Pos2::new(c.x + d * 0.95, c.y - d * 0.6)],
                stroke,
            );
        }
        UiIcon::Home => {
            let w = s * 0.76;
            let h = s * 0.72;
            let pts = vec![
                Pos2::new(c.x, c.y - h),
                Pos2::new(c.x + w, c.y - h * 0.15),
                Pos2::new(c.x + w * 0.78, c.y - h * 0.15),
                Pos2::new(c.x + w * 0.78, c.y + h * 0.85),
                Pos2::new(c.x - w * 0.78, c.y + h * 0.85),
                Pos2::new(c.x - w * 0.78, c.y - h * 0.15),
                Pos2::new(c.x - w, c.y - h * 0.15),
            ];
            painter.add(Shape::closed_line(pts, stroke));
            let door = Rect::from_min_max(
                Pos2::new(c.x - w * 0.22, c.y + h * 0.2),
                Pos2::new(c.x + w * 0.22, c.y + h * 0.85),
            );
            painter.rect_stroke(door, Rounding::same(1.0), stroke);
        }
        UiIcon::Library => {
            let r = Rect::from_center_size(c, Vec2::new(s * 1.45, s * 1.25));
            painter.rect_stroke(r, Rounding::same(2.0), stroke);
            let y_top = r.top() + r.height() * 0.34;
            painter.line_segment([Pos2::new(r.left(), y_top), Pos2::new(r.right(), y_top)], stroke);
            painter.line_segment(
                [
                    Pos2::new(r.left() + r.width() * 0.35, r.top()),
                    Pos2::new(r.left() + r.width() * 0.2, y_top),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(r.left() + r.width() * 0.7, r.top()),
                    Pos2::new(r.left() + r.width() * 0.55, y_top),
                ],
                stroke,
            );
        }
        UiIcon::StarOutline | UiIcon::StarFilled => {
            let mut pts = Vec::with_capacity(10);
            let outer = s * 0.84;
            let inner = s * 0.38;
            for i in 0..10 {
                let rad = if i % 2 == 0 { outer } else { inner };
                let ang = -FRAC_PI_2 + (i as f32) * (PI / 5.0);
                pts.push(Pos2::new(c.x + ang.cos() * rad, c.y + ang.sin() * rad));
            }
            if matches!(icon, UiIcon::StarFilled) {
                let mut mesh = Mesh::default();
                mesh.colored_vertex(c, color);
                for p in &pts {
                    mesh.colored_vertex(*p, color);
                }
                for i in 0..10u32 {
                    let next = if i + 1 == 10 { 1 } else { i + 2 };
                    mesh.add_triangle(0, i + 1, next);
                }
                painter.add(Shape::mesh(mesh));
            } else {
                painter.add(Shape::closed_line(pts, stroke));
            }
        }
        UiIcon::Folder => {
            let w = s * 0.82;
            let h = s * 0.65;
            let pts = vec![
                Pos2::new(c.x - w, c.y - h * 0.85),
                Pos2::new(c.x - w * 0.2, c.y - h * 0.85),
                Pos2::new(c.x, c.y - h * 0.48),
                Pos2::new(c.x + w, c.y - h * 0.48),
                Pos2::new(c.x + w, c.y + h * 0.85),
                Pos2::new(c.x - w, c.y + h * 0.85),
            ];
            painter.add(Shape::closed_line(pts, stroke));
        }
        UiIcon::Gear => {
            painter.circle_stroke(c, s * 0.36, stroke);
            painter.circle_stroke(c, s * 0.68, Stroke::new(stroke.width * 0.9, color));
            for i in 0..8 {
                let ang = (i as f32) * (PI / 4.0);
                let p1 = Pos2::new(c.x + ang.cos() * s * 0.65, c.y + ang.sin() * s * 0.65);
                let p2 = Pos2::new(c.x + ang.cos() * s * 0.92, c.y + ang.sin() * s * 0.92);
                painter.line_segment([p1, p2], Stroke::new(stroke.width * 1.15, color));
            }
        }
        UiIcon::Search => {
            let center = Pos2::new(c.x - s * 0.14, c.y - s * 0.14);
            let r = s * 0.52;
            painter.circle_stroke(center, r, stroke);
            let handle_start = Pos2::new(center.x + r * 0.72, center.y + r * 0.72);
            let handle_end = Pos2::new(c.x + s * 0.82, c.y + s * 0.82);
            painter.line_segment([handle_start, handle_end], stroke);
        }
        UiIcon::Close => {
            let d = s * 0.52;
            painter.line_segment([Pos2::new(c.x - d, c.y - d), Pos2::new(c.x + d, c.y + d)], stroke);
            painter.line_segment([Pos2::new(c.x + d, c.y - d), Pos2::new(c.x - d, c.y + d)], stroke);
        }
        UiIcon::Bell => {
            let w = s * 0.65;
            let h = s * 0.68;
            let pts = vec![
                Pos2::new(c.x - w * 0.7, c.y + h * 0.45),
                Pos2::new(c.x - w * 0.52, c.y - h * 0.45),
                Pos2::new(c.x, c.y - h * 0.75),
                Pos2::new(c.x + w * 0.52, c.y - h * 0.45),
                Pos2::new(c.x + w * 0.7, c.y + h * 0.45),
            ];
            painter.add(Shape::closed_line(pts, stroke));
            painter.line_segment(
                [Pos2::new(c.x - w * 0.88, c.y + h * 0.45), Pos2::new(c.x + w * 0.88, c.y + h * 0.45)],
                stroke,
            );
            painter.circle_filled(Pos2::new(c.x, c.y + h * 0.82), 1.4, color);
        }
        UiIcon::Minimize => {
            let d = s * 0.62;
            painter.line_segment([Pos2::new(c.x - d, c.y), Pos2::new(c.x + d, c.y)], stroke);
        }
        UiIcon::Maximize => {
            let r = Rect::from_center_size(c, Vec2::splat(s * 1.05));
            painter.rect_stroke(r, Rounding::same(1.5), stroke);
        }
        UiIcon::Clock => {
            painter.circle_stroke(c, s * 0.76, stroke);
            painter.line_segment([c, Pos2::new(c.x, c.y - s * 0.42)], stroke);
            painter.line_segment([c, Pos2::new(c.x + s * 0.34, c.y + s * 0.16)], stroke);
        }
        UiIcon::Monitor => {
            let r = Rect::from_center_size(Pos2::new(c.x, c.y - s * 0.12), Vec2::new(s * 1.45, s * 1.0));
            painter.rect_stroke(r, Rounding::same(2.0), stroke);
            painter.line_segment(
                [Pos2::new(c.x - s * 0.4, c.y + s * 0.65), Pos2::new(c.x + s * 0.4, c.y + s * 0.65)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x, c.y + s * 0.38), Pos2::new(c.x, c.y + s * 0.65)],
                stroke,
            );
        }
        UiIcon::Codec => {
            let r = Rect::from_center_size(c, Vec2::splat(s * 1.25));
            painter.rect_stroke(r, Rounding::same(2.0), stroke);
            let inner = Rect::from_center_size(c, Vec2::splat(s * 0.58));
            painter.rect_stroke(inner, Rounding::same(1.0), stroke);
        }
        UiIcon::Bitrate => {
            let pts = vec![
                Pos2::new(c.x + s * 0.18, c.y - s * 0.8),
                Pos2::new(c.x - s * 0.45, c.y + s * 0.08),
                Pos2::new(c.x + s * 0.05, c.y + s * 0.08),
                Pos2::new(c.x - s * 0.18, c.y + s * 0.8),
                Pos2::new(c.x + s * 0.45, c.y - s * 0.08),
                Pos2::new(c.x - s * 0.05, c.y - s * 0.08),
            ];
            painter.add(Shape::closed_line(pts, stroke));
        }
        UiIcon::Speaker => {
            let cone = vec![
                Pos2::new(c.x - s * 0.65, c.y - s * 0.25),
                Pos2::new(c.x - s * 0.25, c.y - s * 0.25),
                Pos2::new(c.x + s * 0.22, c.y - s * 0.62),
                Pos2::new(c.x + s * 0.22, c.y + s * 0.62),
                Pos2::new(c.x - s * 0.25, c.y + s * 0.25),
                Pos2::new(c.x - s * 0.65, c.y + s * 0.25),
            ];
            painter.add(Shape::closed_line(cone, stroke));
            painter.line_segment(
                [Pos2::new(c.x + s * 0.52, c.y - s * 0.32), Pos2::new(c.x + s * 0.52, c.y + s * 0.32)],
                stroke,
            );
        }
        UiIcon::Mic => {
            let capsule = Rect::from_center_size(Pos2::new(c.x, c.y - s * 0.18), Vec2::new(s * 0.6, s * 0.92));
            painter.rect_stroke(capsule, Rounding::same(s * 0.3), stroke);
            painter.line_segment(
                [Pos2::new(c.x, c.y + s * 0.42), Pos2::new(c.x, c.y + s * 0.78)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - s * 0.38, c.y + s * 0.78), Pos2::new(c.x + s * 0.38, c.y + s * 0.78)],
                stroke,
            );
        }
        UiIcon::AudioTracks => {
            for (dy, knob_x) in [(-0.34, 0.28), (0.34, -0.28)] {
                let y = c.y + s * dy;
                painter.line_segment([Pos2::new(c.x - s * 0.72, y), Pos2::new(c.x + s * 0.72, y)], stroke);
                painter.circle_filled(Pos2::new(c.x + s * knob_x, y), s * 0.19, color);
            }
        }
        UiIcon::Trash => {
            let body = Rect::from_center_size(Pos2::new(c.x, c.y + s * 0.14), Vec2::new(s * 0.92, s * 1.05));
            painter.rect_stroke(body, Rounding::same(1.5), stroke);
            painter.line_segment(
                [Pos2::new(c.x - s * 0.65, c.y - s * 0.42), Pos2::new(c.x + s * 0.65, c.y - s * 0.42)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - s * 0.25, c.y - s * 0.65), Pos2::new(c.x + s * 0.25, c.y - s * 0.65)],
                stroke,
            );
        }
        UiIcon::Database => {
            let r1 = Rect::from_center_size(Pos2::new(c.x, c.y - s * 0.35), Vec2::new(s * 1.25, s * 0.55));
            let r2 = Rect::from_center_size(Pos2::new(c.x, c.y + s * 0.35), Vec2::new(s * 1.25, s * 0.55));
            painter.rect_stroke(r1, Rounding::same(s * 0.25), stroke);
            painter.rect_stroke(r2, Rounding::same(s * 0.25), stroke);
        }
        UiIcon::Palette => {
            painter.circle_stroke(c, s * 0.74, stroke);
            for (dx, dy) in [(-0.3, -0.22), (0.28, -0.24), (-0.24, 0.26), (0.22, 0.22)] {
                painter.circle_filled(Pos2::new(c.x + s * dx, c.y + s * dy), 1.5, color);
            }
        }
        UiIcon::Grid => {
            let q = s * 0.56;
            let sz = Vec2::splat(s * 0.46);
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let r = Rect::from_center_size(Pos2::new(c.x + dx * q * 0.6, c.y + dy * q * 0.6), sz);
                painter.rect_stroke(r, Rounding::same(1.5), stroke);
            }
        }
        UiIcon::List => {
            for dy in [-0.45, 0.0, 0.45] {
                let y = c.y + s * dy;
                painter.line_segment(
                    [Pos2::new(c.x - s * 0.68, y), Pos2::new(c.x + s * 0.68, y)],
                    stroke,
                );
            }
        }
        UiIcon::Play => {
            let tri = vec![
                Pos2::new(c.x - s * 0.45, c.y - s * 0.65),
                Pos2::new(c.x + s * 0.65, c.y),
                Pos2::new(c.x - s * 0.45, c.y + s * 0.65),
            ];
            painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
        }
        UiIcon::Pause => {
            let bar_w = (s * 0.34).max(2.2);
            let bar_h = s * 1.18;
            let l_rect = Rect::from_center_size(Pos2::new(c.x - s * 0.32, c.y), Vec2::new(bar_w, bar_h));
            let r_rect = Rect::from_center_size(Pos2::new(c.x + s * 0.32, c.y), Vec2::new(bar_w, bar_h));
            painter.rect_filled(l_rect, Rounding::same(1.0), color);
            painter.rect_filled(r_rect, Rounding::same(1.0), color);
        }
        UiIcon::Scissors => {
            let r = (s * 0.25).max(2.0);
            painter.circle_stroke(Pos2::new(c.x - s * 0.42, c.y - s * 0.36), r, stroke);
            painter.circle_stroke(Pos2::new(c.x - s * 0.42, c.y + s * 0.36), r, stroke);
            painter.line_segment(
                [Pos2::new(c.x - s * 0.2, c.y - s * 0.25), Pos2::new(c.x + s * 0.62, c.y + s * 0.36)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - s * 0.2, c.y + s * 0.25), Pos2::new(c.x + s * 0.62, c.y - s * 0.36)],
                stroke,
            );
        }
        UiIcon::Link => {
            painter.line_segment(
                [Pos2::new(c.x - s * 0.22, c.y + s * 0.22), Pos2::new(c.x + s * 0.22, c.y - s * 0.22)],
                stroke,
            );
            let r1 = Rect::from_center_size(Pos2::new(c.x - s * 0.32, c.y + s * 0.32), Vec2::splat(s * 0.62));
            let r2 = Rect::from_center_size(Pos2::new(c.x + s * 0.32, c.y - s * 0.32), Vec2::splat(s * 0.62));
            painter.rect_stroke(r1, Rounding::same(s * 0.24), stroke);
            painter.rect_stroke(r2, Rounding::same(s * 0.24), stroke);
        }
        UiIcon::Edit => {
            let tip = Pos2::new(c.x - s * 0.65, c.y + s * 0.65);
            let top = Pos2::new(c.x + s * 0.45, c.y - s * 0.45);
            painter.line_segment(
                [Pos2::new(c.x - s * 0.3, c.y + s * 0.65), Pos2::new(c.x + s * 0.7, c.y - s * 0.35)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - s * 0.65, c.y + s * 0.3), Pos2::new(c.x + s * 0.35, c.y - s * 0.7)],
                stroke,
            );
            painter.line_segment([tip, Pos2::new(c.x - s * 0.65, c.y + s * 0.3)], stroke);
            painter.line_segment([tip, Pos2::new(c.x - s * 0.3, c.y + s * 0.65)], stroke);
            painter.line_segment(
                [Pos2::new(c.x + s * 0.35, c.y - s * 0.7), Pos2::new(c.x + s * 0.7, c.y - s * 0.35)],
                stroke,
            );
            let _ = top;
        }
    }
}

pub fn paint_rounded_image(
    painter: &Painter,
    tex_id: TextureId,
    rect: Rect,
    rounding: Rounding,
    uv: Rect,
    tint: Color32,
) {
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    let max_r = (w * 0.5).min(h * 0.5);
    let corners = [
        (rounding.nw.min(max_r), Pos2::new(rect.left() + rounding.nw.min(max_r), rect.top() + rounding.nw.min(max_r)), PI, PI * 1.5),
        (rounding.ne.min(max_r), Pos2::new(rect.right() - rounding.ne.min(max_r), rect.top() + rounding.ne.min(max_r)), PI * 1.5, TAU),
        (rounding.se.min(max_r), Pos2::new(rect.right() - rounding.se.min(max_r), rect.bottom() - rounding.se.min(max_r)), 0.0, FRAC_PI_2),
        (rounding.sw.min(max_r), Pos2::new(rect.left() + rounding.sw.min(max_r), rect.bottom() - rounding.sw.min(max_r)), FRAC_PI_2, PI),
    ];

    let mut perim: Vec<Pos2> = Vec::with_capacity(64);
    let segs = 14_usize;
    for (r, center, a0, a1) in corners {
        if r <= 0.5 {
            perim.push(center);
        } else {
            for i in 0..=segs {
                let ang = a0 + (a1 - a0) * (i as f32 / segs as f32);
                perim.push(Pos2::new(center.x + ang.cos() * r, center.y + ang.sin() * r));
            }
        }
    }

    let mut mesh = Mesh::with_texture(tex_id);
    let to_uv = |p: Pos2| -> Pos2 {
        Pos2::new(
            uv.left() + ((p.x - rect.left()) / w) * uv.width(),
            uv.top() + ((p.y - rect.top()) / h) * uv.height(),
        )
    };
    let push_v = |mesh: &mut Mesh, pos: Pos2| {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: to_uv(pos),
            color: tint,
        });
    };
    push_v(&mut mesh, rect.center());
    for &p in &perim {
        push_v(&mut mesh, p);
    }
    let n = perim.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + ((i + 1) % n));
    }
    painter.add(mesh);
}

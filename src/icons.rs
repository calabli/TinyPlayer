//! Vector icons drawn with egui shapes — no icon font, no image assets.

use eframe::egui::{Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Pause,
    Prev,
    Next,
    Volume,
    Muted,
    Fullscreen,
    ExitFullscreen,
    Drop,
    Settings,
    List,
    Shuffle,
    Repeat,
    RepeatOne,
    Close,
    Stop,
    Star,
    StarFilled,
    Folder,
    PictureInPicture,
}

/// Draws `icon` centered at `c`, fitting in a `s`×`s` box.
pub fn draw(p: &Painter, icon: Icon, c: Pos2, s: f32, color: Color32) {
    let h = s / 2.0;
    let stroke = Stroke::new((s / 11.0).max(1.4), color);
    match icon {
        Icon::PictureInPicture => {
            let outer = eframe::egui::Rect::from_center_size(c, vec2(s, s * 0.76));
            p.rect_stroke(outer, 2.0, stroke, eframe::egui::StrokeKind::Middle);
            let inner = eframe::egui::Rect::from_min_max(pos2(c.x + h * 0.05, c.y + h * 0.02), pos2(c.x + h * 0.74, c.y + h * 0.5));
            p.rect_filled(inner, 1.0, color);
        }
        Icon::Play => {
            let pts = vec![
                pos2(c.x - h * 0.55, c.y - h * 0.75),
                pos2(c.x + h * 0.75, c.y),
                pos2(c.x - h * 0.55, c.y + h * 0.75),
            ];
            p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Pause => {
            let w = h * 0.28;
            for dx in [-h * 0.38, h * 0.38] {
                let r = eframe::egui::Rect::from_center_size(pos2(c.x + dx, c.y), vec2(w, h * 1.5));
                p.rect_filled(r, 1.0, color);
            }
        }
        Icon::Prev | Icon::Next => {
            let dir = if icon == Icon::Next { 1.0 } else { -1.0 };
            let tri = vec![
                pos2(c.x - dir * h * 0.6, c.y - h * 0.6),
                pos2(c.x + dir * h * 0.45, c.y),
                pos2(c.x - dir * h * 0.6, c.y + h * 0.6),
            ];
            p.add(Shape::convex_polygon(tri, color, Stroke::NONE));
            let x = c.x + dir * h * 0.6;
            p.line_segment([pos2(x, c.y - h * 0.6), pos2(x, c.y + h * 0.6)], stroke);
        }
        Icon::Volume | Icon::Muted => {
            let body = vec![
                pos2(c.x - h * 0.75, c.y - h * 0.28),
                pos2(c.x - h * 0.4, c.y - h * 0.28),
                pos2(c.x + h * 0.05, c.y - h * 0.68),
                pos2(c.x + h * 0.05, c.y + h * 0.68),
                pos2(c.x - h * 0.4, c.y + h * 0.28),
                pos2(c.x - h * 0.75, c.y + h * 0.28),
            ];
            p.add(Shape::convex_polygon(body, color, Stroke::NONE));
            if icon == Icon::Muted {
                let (a, b) = (c.x + h * 0.32, c.x + h * 0.8);
                p.line_segment([pos2(a, c.y - h * 0.24), pos2(b, c.y + h * 0.24)], stroke);
                p.line_segment([pos2(a, c.y + h * 0.24), pos2(b, c.y - h * 0.24)], stroke);
            } else {
                arc(p, c + vec2(h * 0.05, 0.0), h * 0.4, stroke);
                arc(p, c + vec2(h * 0.05, 0.0), h * 0.75, stroke);
            }
        }
        Icon::Fullscreen | Icon::ExitFullscreen => {
            let o = h * 0.75;
            let l = h * 0.4;
            let inward = icon == Icon::ExitFullscreen;
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = pos2(c.x + sx * o, c.y + sy * o);
                let (dx, dy) = if inward { (sx * l, sy * l) } else { (-sx * l, -sy * l) };
                let origin = if inward { pos2(corner.x - dx, corner.y - dy) } else { corner };
                p.line_segment([origin, pos2(origin.x + dx, origin.y)], stroke);
                p.line_segment([origin, pos2(origin.x, origin.y + dy)], stroke);
            }
        }
        Icon::Settings => {
            // Gear: thick ring with six stubby teeth touching it.
            let teeth = Stroke::new(stroke.width * 2.2, color);
            for i in 0..6 {
                let a = std::f32::consts::TAU * i as f32 / 6.0;
                let (sn, cs) = a.sin_cos();
                p.line_segment(
                    [pos2(c.x + cs * h * 0.5, c.y + sn * h * 0.5), pos2(c.x + cs * h * 0.82, c.y + sn * h * 0.82)],
                    teeth,
                );
            }
            p.circle_stroke(c, h * 0.5, Stroke::new(stroke.width * 1.6, color));
            p.circle_stroke(c, h * 0.18, stroke);
        }
        Icon::List => {
            for dy in [-0.5, 0.0, 0.5] {
                let y = c.y + dy * h * 1.2;
                p.circle_filled(pos2(c.x - h * 0.7, y), stroke.width * 0.75, color);
                p.line_segment([pos2(c.x - h * 0.35, y), pos2(c.x + h * 0.8, y)], stroke);
            }
        }
        Icon::Shuffle => {
            let (l, r, t, b) = (c.x - h * 0.8, c.x + h * 0.55, c.y - h * 0.5, c.y + h * 0.5);
            p.line_segment([pos2(l, b), pos2(r, t)], stroke);
            p.line_segment([pos2(l, t), pos2(r, b)], stroke);
            for y in [t, b] {
                let tip = pos2(c.x + h * 0.85, y);
                let pts = vec![tip, pos2(r - h * 0.05, y - h * 0.28), pos2(r - h * 0.05, y + h * 0.28)];
                p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
            }
        }
        Icon::Repeat | Icon::RepeatOne => {
            // Two half-loops with opposing arrowheads (the classic repeat glyph).
            let (l, r, t, b) = (c.x - h * 0.8, c.x + h * 0.8, c.y - h * 0.45, c.y + h * 0.45);
            p.add(Shape::line(vec![pos2(l, c.y + h * 0.1), pos2(l, t), pos2(r - h * 0.3, t)], stroke));
            p.add(Shape::line(vec![pos2(r, c.y - h * 0.1), pos2(r, b), pos2(l + h * 0.3, b)], stroke));
            let ah = h * 0.3;
            p.add(Shape::convex_polygon(
                vec![pos2(r, t), pos2(r - ah, t - ah), pos2(r - ah, t + ah)],
                color,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![pos2(l, b), pos2(l + ah, b - ah), pos2(l + ah, b + ah)],
                color,
                Stroke::NONE,
            ));
            if icon == Icon::RepeatOne {
                p.text(c, eframe::egui::Align2::CENTER_CENTER, "1", eframe::egui::FontId::monospace(s * 0.5), color);
            }
        }
        Icon::Star | Icon::StarFilled => {
            // Five-point star: alternate outer/inner radius.
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let r = if i % 2 == 0 { h * 0.95 } else { h * 0.42 };
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    pos2(c.x + r * a.cos(), c.y + r * a.sin() + h * 0.06)
                })
                .collect();
            if icon == Icon::StarFilled {
                // Non-convex: fill the inner pentagon plus five tip triangles.
                let inner: Vec<Pos2> = pts.iter().skip(1).step_by(2).copied().collect();
                p.add(Shape::convex_polygon(inner.clone(), color, Stroke::NONE));
                for k in 0..5 {
                    let tip = pts[k * 2];
                    let a = inner[(k + 4) % 5];
                    let b = inner[k];
                    p.add(Shape::convex_polygon(vec![tip, b, a], color, Stroke::NONE));
                }
            } else {
                p.add(Shape::closed_line(pts, Stroke::new(stroke.width * 0.9, color)));
            }
        }
        Icon::Folder => {
            let r = eframe::egui::Rect::from_center_size(c + vec2(0.0, h * 0.08), vec2(h * 1.7, h * 1.2));
            p.rect_stroke(r, 2.0, stroke, eframe::egui::StrokeKind::Middle);
            p.line_segment([pos2(r.left(), r.top()), pos2(r.left(), r.top() - h * 0.2)], stroke);
            p.line_segment([pos2(r.left(), r.top() - h * 0.2), pos2(r.left() + h * 0.6, r.top() - h * 0.2)], stroke);
            p.line_segment([pos2(r.left() + h * 0.6, r.top() - h * 0.2), pos2(r.left() + h * 0.8, r.top())], stroke);
        }
        Icon::Stop => {
            let r = eframe::egui::Rect::from_center_size(c, vec2(h * 1.1, h * 1.1));
            p.rect_filled(r, 2.0, color);
        }
        Icon::Close => {
            let d = h * 0.45;
            p.line_segment([pos2(c.x - d, c.y - d), pos2(c.x + d, c.y + d)], stroke);
            p.line_segment([pos2(c.x - d, c.y + d), pos2(c.x + d, c.y - d)], stroke);
        }
        Icon::Drop => {
            p.line_segment([pos2(c.x, c.y - h * 0.8), pos2(c.x, c.y + h * 0.2)], stroke);
            p.line_segment([pos2(c.x - h * 0.4, c.y - h * 0.2), pos2(c.x, c.y + h * 0.2)], stroke);
            p.line_segment([pos2(c.x + h * 0.4, c.y - h * 0.2), pos2(c.x, c.y + h * 0.2)], stroke);
            p.line_segment([pos2(c.x - h * 0.7, c.y + h * 0.75), pos2(c.x + h * 0.7, c.y + h * 0.75)], stroke);
        }
    }
}

/// Right-facing sound-wave arc.
fn arc(p: &Painter, c: Pos2, r: f32, stroke: Stroke) {
    let pts: Vec<Pos2> = (0..=8)
        .map(|i| {
            let a = -0.85 + 1.7 * i as f32 / 8.0;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    p.add(Shape::line(pts, stroke));
}

/// Four corner brackets around `rect` — the "viewfinder" frame from the design.
pub fn corner_brackets(p: &Painter, rect: eframe::egui::Rect, len: f32, stroke: Stroke) {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    for (x, y, dx, dy) in [(l, t, len, len), (r, t, -len, len), (l, b, len, -len), (r, b, -len, -len)] {
        p.line_segment([pos2(x, y), pos2(x + dx, y)], stroke);
        p.line_segment([pos2(x, y), pos2(x, y + dy)], stroke);
    }
}

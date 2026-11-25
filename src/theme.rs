//! Colors, fonts and egui style for the tech-minimal look.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Mesh, Rect, Stroke, epaint::Vertex, pos2};

const BG: Color32 = Color32::from_rgb(0x05, 0x07, 0x0B);
const PANEL: Color32 = Color32::from_rgb(0x0B, 0x10, 0x18);
const SURFACE: Color32 = Color32::from_rgb(0x12, 0x19, 0x26);
pub const ACCENT: Color32 = Color32::from_rgb(0x2F, 0xB8, 0xF5);
pub const TEXT: Color32 = Color32::from_rgb(0xEF, 0xF4, 0xFA);
pub const TEXT_2: Color32 = Color32::from_rgb(0x8A, 0x99, 0xAE);
pub const TEXT_3: Color32 = Color32::from_rgb(0x4D, 0x5B, 0x70);
const LINE: Color32 = Color32::from_rgb(0x1D, 0x26, 0x36);

const LIGHT_BG: Color32 = Color32::from_rgb(0xF2, 0xF5, 0xFA);
const LIGHT_PANEL: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const LIGHT_SURFACE: Color32 = Color32::from_rgb(0xE8, 0xED, 0xF5);
const LIGHT_TEXT: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
const LIGHT_TEXT_2: Color32 = Color32::from_rgb(0x52, 0x60, 0x75);
const LIGHT_TEXT_3: Color32 = Color32::from_rgb(0x8B, 0x9A, 0xB0);
const LIGHT_LINE: Color32 = Color32::from_rgb(0xD6, 0xDF, 0xEC);

/// Only the library, the settings window and the empty page follow the theme.
/// The player's chrome sits on top of video, so it stays dark.
static LIGHT: AtomicBool = AtomicBool::new(false);

fn pick(dark: Color32, light: Color32) -> Color32 {
    if LIGHT.load(Ordering::Relaxed) { light } else { dark }
}

pub fn bg() -> Color32 {
    pick(BG, LIGHT_BG)
}

pub fn panel() -> Color32 {
    pick(PANEL, LIGHT_PANEL)
}

pub fn surface() -> Color32 {
    pick(SURFACE, LIGHT_SURFACE)
}

pub fn text() -> Color32 {
    pick(TEXT, LIGHT_TEXT)
}

pub fn text_2() -> Color32 {
    pick(TEXT_2, LIGHT_TEXT_2)
}

pub fn text_3() -> Color32 {
    pick(TEXT_3, LIGHT_TEXT_3)
}

pub fn line() -> Color32 {
    pick(LINE, LIGHT_LINE)
}

/// A faint wash for hovered or raised surfaces.
pub fn tint(alpha: u8) -> Color32 {
    if LIGHT.load(Ordering::Relaxed) {
        Color32::from_black_alpha(alpha)
    } else {
        Color32::from_white_alpha(alpha)
    }
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// `choice` is the saved setting: "dark", "light" or "system".
pub fn wants_light(choice: &str, system: Option<egui::Theme>) -> bool {
    match choice {
        "light" => true,
        "system" => system == Some(egui::Theme::Light),
        _ => false,
    }
}

/// Semi-transparent obsidian used behind overlays.
pub fn scrim(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(5, 7, 11, alpha)
}

/// Deep translucent obsidian surface for floating HUD capsules over video.
pub fn glass(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(10, 14, 22, alpha)
}

/// Subtle 1px specular hairline border for dark floating panels.
pub fn glass_stroke() -> Stroke {
    Stroke::new(1.0, Color32::from_white_alpha(28))
}

pub fn accent_alpha(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(0x2F, 0xB8, 0xF5, alpha)
}

pub fn accent_stroke(alpha: u8) -> Stroke {
    Stroke::new(1.0, accent_alpha(alpha))
}

/// Hardware-smooth vertical linear gradient quad.
pub fn vertical_gradient(p: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    let uv = pos2(0.0, 0.0);
    mesh.vertices.push(Vertex { pos: rect.left_top(), uv, color: top });
    mesh.vertices.push(Vertex { pos: rect.right_top(), uv, color: top });
    mesh.vertices.push(Vertex { pos: rect.right_bottom(), uv, color: bottom });
    mesh.vertices.push(Vertex { pos: rect.left_bottom(), uv, color: bottom });
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    p.add(egui::Shape::mesh(mesh));
}

pub fn apply(ctx: &egui::Context, light: bool) {
    install_fonts(ctx);
    set_light(ctx, light);
}

pub fn set_light(ctx: &egui::Context, light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
    ctx.all_styles_mut(|style| {
        style.visuals = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
        style.visuals.panel_fill = bg();
        style.visuals.window_fill = panel();
        style.visuals.override_text_color = Some(text());
        style.interaction.tooltip_delay = 0.3;
    });
}

/// egui's built-in fonts have no CJK glyphs, so add a system font as a
/// fallback instead of bundling one (keeps the binary small).
fn install_fonts(ctx: &egui::Context) {
    let candidates: &[(&str, u32)] = if cfg!(target_os = "windows") {
        &[
            ("C:\\Windows\\Fonts\\msyh.ttc", 0),
            ("C:\\Windows\\Fonts\\msyh.ttf", 0),
            ("C:\\Windows\\Fonts\\simhei.ttf", 0),
        ]
    } else if cfg!(target_os = "macos") {
        &[
            ("/System/Library/Fonts/PingFang.ttc", 0),
            ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
            ("/System/Library/Fonts/STHeiti Light.ttc", 0),
        ]
    } else {
        &[("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0)]
    };

    let mut fonts = FontDefinitions::default();
    for (path, index) in candidates {
        // Memory-map instead of reading: CJK fonts are ~20 MB, and only the
        // glyph pages actually used get loaded (and they're shared, not private).
        let Ok(file) = std::fs::File::open(path) else { continue };
        // SAFETY: system font files aren't modified while we run.
        if let Ok(map) = unsafe { memmap2::Mmap::map(&file) } {
            let data = FontData::from_blob(Arc::new(map), *index);
            fonts.font_data.insert("cjk".into(), Arc::new(data));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

/// Prefix for shortcut labels. Spelled out because the bundled fallback fonts
/// have no ⌘ glyph.
pub const CMD: &str = if cfg!(target_os = "macos") { "Cmd+" } else { "Ctrl+" };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_choice_follows_the_system_only_when_asked() {
        assert!(!wants_light("dark", Some(egui::Theme::Light)));
        assert!(wants_light("light", Some(egui::Theme::Dark)));
        assert!(wants_light("system", Some(egui::Theme::Light)));
        assert!(!wants_light("system", Some(egui::Theme::Dark)));
        assert!(!wants_light("system", None));
    }
}

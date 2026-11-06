//! Colors, fonts and egui style for the tech-minimal look.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};

const BG: Color32 = Color32::from_rgb(0x07, 0x09, 0x0C);
const PANEL: Color32 = Color32::from_rgb(0x0D, 0x11, 0x17);
pub const ACCENT: Color32 = Color32::from_rgb(0x3D, 0x8E, 0xF0);
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xED, 0xF3);
pub const TEXT_2: Color32 = Color32::from_rgb(0x8B, 0x94, 0x9E);
pub const TEXT_3: Color32 = Color32::from_rgb(0x48, 0x4F, 0x58);
pub const LINE: Color32 = Color32::from_rgb(0x21, 0x26, 0x2D);

const LIGHT_BG: Color32 = Color32::from_rgb(0xF5, 0xF6, 0xF8);
const LIGHT_PANEL: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const LIGHT_TEXT: Color32 = Color32::from_rgb(0x1B, 0x1F, 0x24);
const LIGHT_TEXT_2: Color32 = Color32::from_rgb(0x57, 0x60, 0x6A);
const LIGHT_TEXT_3: Color32 = Color32::from_rgb(0x7D, 0x85, 0x8E);
const LIGHT_LINE: Color32 = Color32::from_rgb(0xD8, 0xDE, 0xE4);

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

/// `choice` is the saved setting: "dark", "light" or "system".
pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

pub fn wants_light(choice: &str, system: Option<egui::Theme>) -> bool {
    match choice {
        "light" => true,
        "system" => system == Some(egui::Theme::Light),
        _ => false,
    }
}

/// Semi-transparent black used behind overlays.
pub fn scrim(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(7, 9, 12, alpha)
}

pub fn accent_alpha(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(0x3D, 0x8E, 0xF0, alpha)
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

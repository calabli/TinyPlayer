//! Colors, fonts and egui style for the tech-minimal look.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};

pub const BG: Color32 = Color32::from_rgb(0x07, 0x09, 0x0C);
pub const PANEL: Color32 = Color32::from_rgb(0x0D, 0x11, 0x17);
pub const ACCENT: Color32 = Color32::from_rgb(0x3D, 0x8E, 0xF0);
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xED, 0xF3);
pub const TEXT_2: Color32 = Color32::from_rgb(0x8B, 0x94, 0x9E);
pub const TEXT_3: Color32 = Color32::from_rgb(0x48, 0x4F, 0x58);
pub const LINE: Color32 = Color32::from_rgb(0x21, 0x26, 0x2D);

/// Semi-transparent black used behind overlays.
pub fn scrim(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(7, 9, 12, alpha)
}

pub fn accent_alpha(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(0x3D, 0x8E, 0xF0, alpha)
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    // Same dark look regardless of the OS light/dark setting.
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.override_text_color = Some(TEXT);
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

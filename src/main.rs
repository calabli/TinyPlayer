#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod history;
mod icons;
mod mpv;
mod player;
mod session;
mod settings_panel;
mod theme;
mod video;

use eframe::egui;

fn main() -> eframe::Result {
    // Files passed on the command line (e.g. from "Open with" / file association).
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TinyPlayer")
            .with_inner_size([1100.0, 640.0])
            .with_min_inner_size([480.0, 300.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };

    eframe::run_native(
        "TinyPlayer",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, files)))),
    )
}

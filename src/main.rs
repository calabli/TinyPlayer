#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod history;
mod icons;
mod library;
mod library_ui;
mod mpv;
mod player;
mod quick_panel;
mod playlist;
mod probe;
mod scanner;
mod session;
mod settings_panel;
mod single_instance;
mod theme;
mod video;

use eframe::egui;

fn main() -> eframe::Result {
    // Files passed on the command line (e.g. from "Open with" / file association).
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();

    // Already running? Hand the files to that window and exit.
    let listener = match single_instance::acquire(&files) {
        single_instance::Role::Forwarded => return Ok(()),
        single_instance::Role::Primary(l) => l,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TinyPlayer")
            .with_inner_size([1100.0, 640.0])
            .with_min_inner_size([480.0, 300.0])
            .with_drag_and_drop(true)
            .with_icon(egui::IconData {
                rgba: include_bytes!("../assets/icon-128.rgba").to_vec(),
                width: 128,
                height: 128,
            }),
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };

    eframe::run_native(
        "TinyPlayer",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, files, listener)))),
    )
}

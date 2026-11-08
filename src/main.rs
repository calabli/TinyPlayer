#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ab_loop;
mod app;
mod audio_fx;
mod audio_output;
mod background;
mod crop_detect;
mod duplicates;
mod file_move;
mod float_window;
mod history;
mod hover_thumbs;
mod i18n;
mod icons;
mod keep_awake;
mod library;
mod library_lock;
mod library_manage;
mod library_ui;
mod mac_open;
mod marks;
mod media_keys;
mod mpv;
mod multiview;
mod player;
mod quick_panel;
mod playlist;
mod probe;
mod scanner;
mod session;
mod settings_panel;
mod single_instance;
mod split_layout;
mod theme;
mod update;
mod video;
mod view;
mod vr;

use eframe::egui;


/// Window position/size in points, remembered across runs.
#[derive(Clone, Copy, Debug)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

impl WindowPlace {
    pub fn from_rects(outer: egui::Rect, inner: egui::Rect, maximized: bool) -> Self {
        Self { x: outer.min.x, y: outer.min.y, w: inner.width(), h: inner.height(), maximized }
    }

    pub fn encode(&self) -> String {
        format!("{:.0},{:.0},{:.0},{:.0},{}", self.x, self.y, self.w, self.h, u8::from(self.maximized))
    }

    fn decode(s: &str) -> Option<Self> {
        let v: Vec<&str> = s.split(',').collect();
        let f = |i: usize| v.get(i)?.parse::<f32>().ok();
        let place = Self { x: f(0)?, y: f(1)?, w: f(2)?, h: f(3)?, maximized: v.get(4) == Some(&"1") };
        // Ignore nonsense (e.g. a window parked off-screen by a removed monitor).
        (place.w >= 480.0 && place.h >= 300.0 && place.x > -2000.0 && place.y > -200.0).then_some(place)
    }
}

fn main() -> eframe::Result {
    // Files passed on the command line (e.g. from "Open with" / file association).
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();

    mac_open::install();

    // Already running? Hand the files to that window and exit.
    let listener = match single_instance::acquire(&files) {
        single_instance::Role::Forwarded => return Ok(()),
        single_instance::Role::Primary(l) => l,
    };

    let place = history::Store::open_default()
        .ok()
        .and_then(|s| s.get_value("window"))
        .and_then(|v| WindowPlace::decode(&v));

    let viewport_place = |vp: egui::ViewportBuilder| match place {
        Some(p) => vp.with_position([p.x, p.y]).with_maximized(p.maximized),
        None => vp,
    };
    let options = eframe::NativeOptions {
        viewport: viewport_place(egui::ViewportBuilder::default()
            .with_title("TinyPlayer")
            .with_inner_size(place.map_or([1100.0, 640.0], |p| [p.w, p.h]))
            .with_min_inner_size([480.0, 300.0])
            .with_drag_and_drop(true)
            .with_icon(egui::IconData {
                rgba: include_bytes!("../assets/icon-128.rgba").to_vec(),
                width: 128,
                height: 128,
            })),
        renderer: eframe::Renderer::Glow,
        centered: place.is_none(),
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

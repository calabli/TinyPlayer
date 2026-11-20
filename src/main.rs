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
mod sleep_timer;
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

/// The window's inner size when there's no saved one: 16:9, small enough to
/// leave room around it on a laptop screen.
const DEFAULT_SIZE: [f32; 2] = [854.0, 480.0];

fn main() -> eframe::Result {
    // Files passed on the command line (e.g. from "Open with" / file association).
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();

    mac_open::install();

    let store = history::Store::open_default().ok();
    let several = store.as_ref().and_then(|s| s.get_value("multi_window")).is_some_and(|v| v == "1");

    // Already running? Hand the files to that window and exit, unless
    // several windows are allowed: then this one runs on its own.
    let listener = match single_instance::acquire(&files, several) {
        single_instance::Role::Forwarded => return Ok(()),
        single_instance::Role::Primary(l) => l,
        single_instance::Role::Extra => None,
    };

    // Off, the window opens at the default size in the middle every time.
    let remember = store.as_ref().and_then(|s| s.get_value("remember_window")).is_none_or(|v| v == "1");
    let place = store
        .as_ref()
        .filter(|_| remember)
        .and_then(|s| s.get_value("window"))
        .and_then(|v| WindowPlace::decode(&v));
    drop(store);
    // A further window opens a little below and right of the saved place, so
    // it doesn't hide the first one exactly. Maximized would hide it entirely.
    let place = if single_instance::is_extra() {
        let step = 36.0 * (1 + std::process::id() % 4) as f32;
        let p = place.unwrap_or(WindowPlace { x: 80.0, y: 80.0, w: DEFAULT_SIZE[0], h: DEFAULT_SIZE[1], maximized: false });
        Some(WindowPlace { x: p.x + step, y: p.y + step, maximized: false, ..p })
    } else {
        place
    };

    let viewport_place = |vp: egui::ViewportBuilder| match place {
        Some(p) => vp.with_position([p.x, p.y]).with_maximized(p.maximized),
        None => vp,
    };
    let options = eframe::NativeOptions {
        viewport: viewport_place(egui::ViewportBuilder::default()
            .with_title("TinyPlayer")
            .with_inner_size(place.map_or(DEFAULT_SIZE, |p| [p.w, p.h]))
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

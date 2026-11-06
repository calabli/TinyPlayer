//! Finds a file's black bars with a headless mpv on a throwaway thread, so
//! the running player's filters and decoding aren't disturbed. The sample
//! comes from 20% in, past the logos and fades a file usually starts with.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::mpv::{Event, Mpv};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// `reset=0` widens the area frame by frame, so sampling longer is safer.
const SAMPLE_TIME: Duration = Duration::from_millis(800);

pub type Area = (i64, i64, i64, i64);

pub struct CropDetect {
    tx: Sender<(String, Option<Area>)>,
    pub rx: Receiver<(String, Option<Area>)>,
}

impl CropDetect {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    pub fn start(&self, ctx: &egui::Context, path: String) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("crop-detect".into())
            .spawn(move || {
                let area = detect(&path);
                let _ = tx.send((path, area));
                ctx.request_repaint();
            })
            .ok();
    }
}

fn detect(path: &str) -> Option<Area> {
    let mpv = Mpv::new(&[
        ("vo", "null"),
        ("ao", "null"),
        ("aid", "no"),
        ("sid", "no"),
        ("hwdec", "no"),
        ("vf", "@cd:lavfi=[cropdetect=limit=24:round=2:reset=0]"),
        ("start", "20%"),
        ("keep-open", "yes"),
        ("idle", "yes"),
        ("terminal", "no"),
        ("config", "no"),
        ("osc", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("input-default-bindings", "no"),
    ])
    .ok()?;
    mpv.command(&["loadfile", path, "replace"]).ok()?;
    let deadline = Instant::now() + OPEN_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        match mpv.wait_event(left.as_secs_f64()) {
            Some(Event::FileLoaded) => break,
            Some(Event::EndFile) => return None,
            _ => {}
        }
    }
    std::thread::sleep(SAMPLE_TIME);
    let value = |k: &str| mpv.get_string(&format!("vf-metadata/cd/lavfi.cropdetect.{k}"))?.parse().ok();
    Some((value("w")?, value("h")?, value("x")?, value("y")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs libmpv next to the test binary and a letterboxed video.
    /// Run with: TP_CROP_FILE=path/to/video.mp4 cargo test crop_detect -- --ignored
    #[test]
    #[ignore]
    fn finds_bars_in_a_real_file() {
        let Some(file) = std::env::var("TP_CROP_FILE").ok() else { return };
        let (w, h, x, y) = detect(&file).expect("detected");
        eprintln!("{w}x{h}+{x}+{y}");
        assert!(w > 0 && h > 0);
    }
}

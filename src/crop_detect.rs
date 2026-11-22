//! Finds a file's black bars with a headless mpv on a throwaway thread, so
//! the running player's filters and decoding aren't disturbed. The sample
//! comes from 20% in, past the logos and fades a file usually starts with.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::mpv::{Event, Mpv};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// `reset=0` widens the area frame by frame, so sampling longer is safer.
const SAMPLE_TIME: Duration = Duration::from_millis(800);
const POLL_SLICE: Duration = Duration::from_millis(100);

pub type Area = (i64, i64, i64, i64);

pub struct CropDetect {
    tx: Sender<(String, Option<Area>)>,
    pub rx: Receiver<(String, Option<Area>)>,
    cancel: Arc<AtomicBool>,
}

impl CropDetect {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, cancel: Arc::new(AtomicBool::new(false)) }
    }

    pub fn start(&mut self, ctx: &egui::Context, path: String) {
        // Cancel any previous detection so rapid retries don't pile up decoders.
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Arc::clone(&cancel);
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("crop-detect".into())
            .spawn(move || {
                let area = detect(&path, &cancel);
                if !cancel.load(Ordering::Relaxed) {
                    let _ = tx.send((path, area));
                    ctx.request_repaint();
                }
            });
        if spawned.is_err() {
            let _ = self.tx.send((String::new(), None));
        }
    }
}

impl Drop for CropDetect {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn detect(path: &str, cancel: &AtomicBool) -> Option<Area> {
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
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        match mpv.wait_event(left.min(POLL_SLICE).as_secs_f64()) {
            Some(Event::FileLoaded) => break,
            Some(Event::EndFile | Event::Shutdown) => return None,
            _ => {}
        }
    }
    let sample_end = Instant::now() + SAMPLE_TIME;
    while Instant::now() < sample_end {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let left = sample_end.saturating_duration_since(Instant::now());
        if matches!(mpv.wait_event(left.min(POLL_SLICE).as_secs_f64()), Some(Event::Shutdown)) {
            return None;
        }
    }
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
        let (w, h, x, y) = detect(&file, &AtomicBool::new(false)).expect("detected");
        eprintln!("{w}x{h}+{x}+{y}");
        assert!(w > 0 && h > 0);
    }
}

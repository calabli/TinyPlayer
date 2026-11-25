//! Frame previews above the seek line. A headless mpv on its own thread
//! grabs the frame near the hovered time; only the newest request is served,
//! so sweeping across the bar never builds a backlog. Results are cached per
//! time bucket, and the mpv instance is freed after a short idle period.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use crate::mpv::{Event, Mpv};
use crate::probe::{clear_dir, first_jpg};

const THUMB_WIDTH: u32 = 320;
const MIN_BUCKET_SECS: f64 = 2.0;
const BUCKETS_PER_FILE: f64 = 100.0;
/// About 230 KB each, so the cache stays near 5 MB.
const MAX_CACHED: usize = 24;
const GRAB_TIMEOUT: Duration = Duration::from_secs(3);
const IDLE_RELEASE: Duration = Duration::from_secs(20);

fn bucket_secs(duration: f64) -> f64 {
    (duration / BUCKETS_PER_FILE).max(MIN_BUCKET_SECS)
}

fn bucket_of(time: f64, duration: f64) -> i64 {
    (time.max(0.0) / bucket_secs(duration)) as i64
}

/// The time the frame for a bucket is taken from: its middle, kept inside the file.
fn bucket_time(bucket: i64, duration: f64) -> f64 {
    let size = bucket_secs(duration);
    ((bucket as f64 + 0.5) * size).min((duration - 0.5).max(0.0))
}

/// Least-recently-used list keyed by bucket; the back is the newest.
struct Lru<T> {
    items: Vec<(i64, T)>,
    cap: usize,
}

impl<T> Lru<T> {
    fn new(cap: usize) -> Self {
        Self { items: Vec::new(), cap }
    }

    fn insert(&mut self, key: i64, value: T) {
        self.items.retain(|(k, _)| *k != key);
        self.items.push((key, value));
        if self.items.len() > self.cap {
            self.items.remove(0);
        }
    }

    fn touch(&mut self, key: i64) -> bool {
        let Some(i) = self.items.iter().position(|(k, _)| *k == key) else { return false };
        let item = self.items.remove(i);
        self.items.push(item);
        true
    }

    fn newest(&self) -> Option<&T> {
        self.items.last().map(|(_, v)| v)
    }

    fn clear(&mut self) {
        self.items.clear();
    }
}

/// How long a frame that couldn't be grabbed waits before another try.
const RETRY_AFTER: Duration = Duration::from_secs(10);

struct Job {
    path: String,
    bucket: i64,
    time: f64,
}

struct Grabbed {
    path: String,
    bucket: i64,
    image: Option<ColorImage>,
}

pub struct HoverThumbs {
    tx: Sender<Job>,
    rx: Receiver<Grabbed>,
    cache: Lru<TextureHandle>,
    /// Buckets whose grab failed, and when; tried again after `RETRY_AFTER`,
    /// as a failure can be a passing timeout or a restarted grabber.
    failed: HashMap<i64, Instant>,
    path: String,
    pending: Option<i64>,
}

impl HoverThumbs {
    pub fn spawn(ctx: egui::Context, work_dir: PathBuf) -> Self {
        let (tx, jobs) = mpsc::channel();
        let (done, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hover-thumbs".into())
            .spawn(move || worker(jobs, done, ctx, &work_dir.join(format!("preview-tmp{}", crate::single_instance::work_tag()))))
            .ok();
        Self { tx, rx, cache: Lru::new(MAX_CACHED), failed: HashMap::new(), path: String::new(), pending: None }
    }

    /// Turns finished frames into textures. Frames for another file are dropped.
    pub fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(g) = self.rx.try_recv() {
            if g.path == self.path {
                if let Some(image) = g.image {
                    let tex = ctx.load_texture("hover-thumb", image, TextureOptions::LINEAR);
                    self.cache.insert(g.bucket, tex);
                    self.failed.remove(&g.bucket);
                } else {
                    self.failed.insert(g.bucket, Instant::now());
                }
                if self.pending == Some(g.bucket) {
                    self.pending = None;
                }
            }
        }
    }

    /// The frame for `time`, or the nearest thing to it while that one loads.
    /// Local files only: seeking a network stream per hover would be slow.
    pub fn lookup(&mut self, path: &str, time: f64, duration: f64) -> Option<&TextureHandle> {
        if path.contains("://") {
            return None;
        }
        if path != self.path {
            self.path = path.to_string();
            self.cache.clear();
            self.failed.clear();
            self.pending = None;
        }
        let bucket = bucket_of(time, duration);
        if !self.cache.touch(bucket) && !self.failed.get(&bucket).is_some_and(|at| at.elapsed() < RETRY_AFTER) && self.pending != Some(bucket) {
            self.pending = Some(bucket);
            let job = Job { path: path.to_string(), bucket, time: bucket_time(bucket, duration) };
            let _ = self.tx.send(job);
        }
        self.cache.newest()
    }
}

fn worker(jobs: Receiver<Job>, done: Sender<Grabbed>, ctx: egui::Context, dir: &Path) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let mut mpv: Option<Mpv> = None;
    loop {
        let job = if mpv.is_some() {
            match jobs.recv_timeout(IDLE_RELEASE) {
                Ok(j) => j,
                Err(RecvTimeoutError::Timeout) => {
                    mpv = None;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            let Ok(j) = jobs.recv() else { return };
            j
        };
        let job = jobs.try_iter().last().unwrap_or(job);
        if mpv.is_none() {
            mpv = new_grabber(dir);
        }
        let image = match &mpv {
            Some(m) => {
                let (img, healthy) = grab(m, dir, &job);
                if !healthy {
                    mpv = None;
                }
                img
            }
            None => None,
        };
        let has_image = image.is_some();
        if done.send(Grabbed { path: job.path, bucket: job.bucket, image }).is_err() {
            return;
        }
        if has_image {
            ctx.request_repaint();
        }
    }
}

fn new_grabber(dir: &Path) -> Option<Mpv> {
    let out = dir.to_string_lossy().into_owned();
    let scale = format!("scale=w={THUMB_WIDTH}:h=-2");
    Mpv::new(&[
        ("vo", "image"),
        ("vo-image-format", "jpg"),
        ("vo-image-jpeg-quality", "70"),
        ("vo-image-outdir", &out),
        ("vf", &scale),
        ("ao", "null"),
        ("aid", "no"),
        ("sid", "no"),
        ("hwdec", "no"),
        // Nearest keyframe: a preview doesn't need the exact frame.
        ("hr-seek", "no"),
        ("frames", "1"),
        ("idle", "yes"),
        ("keep-open", "no"),
        ("terminal", "no"),
        ("config", "no"),
        ("osc", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("input-default-bindings", "no"),
    ])
    .ok()
}

/// Returns `(image, healthy)`. When `healthy` is false, the mpv instance may
/// have stale events in flight and should be recreated.
fn grab(m: &Mpv, dir: &Path, job: &Job) -> (Option<ColorImage>, bool) {
    clear_dir(dir);
    m.set_property_string("start", &format!("{:.3}", job.time));
    if m.command(&["loadfile", &job.path, "replace"]).is_err() {
        return (None, false);
    }
    if !wait_end(m, GRAB_TIMEOUT) {
        // Wait for the stop too, so its EndFile can't end the next grab early.
        let _ = m.command(&["stop"]);
        let drained = wait_end(m, Duration::from_secs(1));
        return (None, drained);
    }
    let image = first_jpg(dir)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok())
        .map(|img| {
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
        });
    (image, true)
}

fn wait_end(m: &Mpv, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        match m.wait_event(left.as_secs_f64()) {
            Some(Event::EndFile) => return true,
            Some(Event::Shutdown) | None => return false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_scale_with_duration_but_not_below_the_minimum() {
        assert_eq!(bucket_secs(60.0), MIN_BUCKET_SECS);
        assert_eq!(bucket_secs(7200.0), 72.0);
        assert_eq!(bucket_of(-1.0, 7200.0), 0);
        assert_eq!(bucket_of(144.0, 7200.0), 2);
    }

    #[test]
    fn bucket_time_stays_inside_the_file() {
        assert_eq!(bucket_time(0, 60.0), 1.0);
        assert_eq!(bucket_time(30, 60.0), 59.5);
        assert_eq!(bucket_time(3, 0.2), 0.0);
    }

    #[test]
    fn lru_evicts_the_oldest_untouched_item() {
        let mut lru = Lru::new(2);
        lru.insert(1, "a");
        lru.insert(2, "b");
        assert!(lru.touch(1));
        lru.insert(3, "c");
        assert!(!lru.touch(2));
        assert!(lru.touch(1));
        assert_eq!(lru.newest(), Some(&"a"));
    }

    #[test]
    fn lru_reinsert_replaces_the_value() {
        let mut lru = Lru::new(2);
        lru.insert(1, "a");
        lru.insert(1, "b");
        assert_eq!(lru.items.len(), 1);
        assert_eq!(lru.newest(), Some(&"b"));
    }
}

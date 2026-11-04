//! Reads duration/resolution and grabs a thumbnail for a media file using a
//! headless libmpv instance (`vo=image`). Runs on a background thread.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::mpv::{Event, Format, Mpv, PropValue};

/// Thumbnails are scaled to this width (height keeps the aspect ratio).
pub const THUMB_WIDTH: u32 = 480;
const PER_FILE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Default, Clone)]
pub struct Info {
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    /// Where the thumbnail was written, if the file has video.
    pub thumb: Option<PathBuf>,
}

pub struct Prober {
    mpv: Mpv,
    /// Scratch directory mpv writes frames into; emptied per file.
    out_dir: PathBuf,
}

impl Prober {
    pub fn new(work_dir: &Path) -> Result<Self, String> {
        let out_dir = work_dir.join("probe-tmp");
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let out = out_dir.to_string_lossy().into_owned();
        let scale = format!("scale=w={THUMB_WIDTH}:h=-2");
        let mpv = Mpv::new(&[
            ("vo", "image"),
            ("vo-image-format", "jpg"),
            ("vo-image-jpeg-quality", "82"),
            ("vo-image-outdir", &out),
            ("vf", &scale),
            ("ao", "null"),
            ("aid", "no"),
            ("sid", "no"),
            ("hwdec", "no"),
            ("frames", "1"),
            ("start", "10%"),
            ("idle", "yes"),
            ("keep-open", "no"),
            ("terminal", "no"),
            ("config", "no"),
            ("osc", "no"),
            ("load-scripts", "no"),
            ("ytdl", "no"),
            ("input-default-bindings", "no"),
        ])?;
        for (name, fmt) in [("duration", Format::Double), ("width", Format::Int), ("height", Format::Int)] {
            mpv.observe(name, fmt);
        }
        Ok(Self { mpv, out_dir })
    }

    /// Probes `path` and moves its thumbnail to `thumb_dest` (if any frame
    /// was produced). Returns None if the file couldn't be opened in time.
    pub fn probe(&self, path: &Path, thumb_dest: &Path) -> Option<Info> {
        clear_dir(&self.out_dir);
        let p = path.to_string_lossy();
        self.mpv.command(&["loadfile", &p, "replace"]).ok()?;

        let mut info = Info::default();
        let mut loaded = false;
        let deadline = Instant::now() + PER_FILE_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let _ = self.mpv.command(&["stop"]);
                break;
            }
            match self.mpv.wait_event(left.as_secs_f64()) {
                Some(Event::FileLoaded) => loaded = true,
                Some(Event::EndFile) => break,
                Some(Event::PropertyChange { name, value }) => match (name.as_str(), value) {
                    ("duration", PropValue::Double(d)) => info.duration = d,
                    ("width", PropValue::Int(w)) => info.width = w,
                    ("height", PropValue::Int(h)) => info.height = h,
                    _ => {}
                },
                _ => {}
            }
        }
        if !loaded {
            return None;
        }

        if let Some(frame) = first_jpg(&self.out_dir) {
            if let Some(parent) = thumb_dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(&frame, thumb_dest).is_ok() || std::fs::copy(&frame, thumb_dest).is_ok() {
                info.thumb = Some(thumb_dest.to_path_buf());
            }
        }
        Some(info)
    }
}

fn clear_dir(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn first_jpg(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs libmpv next to the test binary and a real video.
    /// Run with: TP_PROBE_FILE=path/to/video.mp4 cargo test probe -- --ignored
    #[test]
    #[ignore]
    fn probes_a_real_file() {
        let Some(file) = std::env::var_os("TP_PROBE_FILE") else { return };
        let dir = std::env::temp_dir().join("tinyplayer-probe-test");
        let prober = Prober::new(&dir).expect("libmpv");
        let dest = dir.join("thumb.jpg");
        let _ = std::fs::remove_file(&dest);
        let t = Instant::now();
        let info = prober.probe(Path::new(&file), &dest).expect("probe");
        eprintln!("{info:?} in {:?}", t.elapsed());
        assert!(info.duration > 0.0);
        assert!(info.width > 0 && info.height > 0);
        assert!(dest.exists(), "thumbnail written");
    }
}

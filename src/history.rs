//! Playback history: resume positions, play counts and history settings,
//! stored in a local SQLite database.
//!
//! Two kinds of data are kept apart on purpose:
//! - `history` (resume positions, recent list) — cleared by "clear on exit"
//! - `play_events` (play counts) — kept, only cleared by an explicit action

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::playlist::LoopMode;

/// A play counts once the viewer has watched this long (or 10% of the file,
/// whichever is shorter).
pub const COUNT_AFTER_SECS: f64 = 30.0;
/// Reopening the same file within this window doesn't count as a new play.
pub const DEDUPE_SECS: i64 = 30 * 60;
/// Progress past this fraction counts as "finished".
pub const FINISHED_FRACTION: f64 = 0.95;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResumeMode {
    Auto,
    Ask,
    Start,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub remember: bool,
    pub resume_mode: ResumeMode,
    /// Files shorter than this (seconds) don't get a resume position. 0 = off.
    pub min_duration: u32,
    pub clear_finished: bool,
    pub clear_on_exit: bool,
    // Playback
    /// Opening one file also queues the other media files in its folder.
    pub fill_folder: bool,
    pub loop_mode: LoopMode,
    pub shuffle: bool,
    // Video
    pub hwdec: bool,
    /// Aspect override: "auto", "16:9", "4:3", "2.35:1" or "stretch".
    pub aspect: String,
    // Subtitles
    /// Load subtitle files whose name matches the video.
    pub sub_auto: bool,
    /// mpv `sub-font-size` (mpv's default is 38 at 720p scale).
    pub sub_size: u32,
    /// mpv `sub-pos`: 100 = bottom edge, smaller = higher.
    pub sub_pos: u32,
    /// Preferred subtitle language: "auto", "zh" or "en".
    pub sub_lang: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            remember: true,
            resume_mode: ResumeMode::Auto,
            min_duration: 180,
            clear_finished: true,
            clear_on_exit: false,
            fill_folder: true,
            loop_mode: LoopMode::Off,
            shuffle: false,
            hwdec: true,
            aspect: "auto".into(),
            sub_auto: true,
            sub_size: 46,
            sub_pos: 100,
            sub_lang: "auto".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub title: String,
    pub position: f64,
    pub duration: f64,
}

pub struct Store {
    db: Connection,
}

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Identifies a local file by path + size, so a replaced file doesn't inherit
/// the old one's position. Returns None for URLs and missing files.
pub fn file_key(path: &str) -> Option<String> {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() {
        return None;
    }
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    Some(format!("{}|{}", canon.to_string_lossy(), meta.len()))
}

pub fn data_dir() -> PathBuf {
    // Override for development and tests, so they don't touch real history.
    if let Some(dir) = std::env::var_os("TINYPLAYER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("TinyPlayer")
}

impl Store {
    pub fn open_default() -> Result<Self, String> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Self::open(&dir.join("tinyplayer.db"))
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS history (
                 key        TEXT PRIMARY KEY,
                 path       TEXT NOT NULL,
                 title      TEXT NOT NULL,
                 position   REAL NOT NULL,
                 duration   REAL NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS play_events (
                 id           INTEGER PRIMARY KEY,
                 key          TEXT NOT NULL,
                 path         TEXT NOT NULL,
                 started_at   INTEGER NOT NULL,
                 watched_secs REAL NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS play_events_key ON play_events(key, started_at);
             CREATE TABLE IF NOT EXISTS settings (k TEXT PRIMARY KEY, v TEXT NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { db })
    }

    // ---- Settings ----

    pub fn load_settings(&self) -> Settings {
        let d = Settings::default();
        let get = |k: &str| -> Option<String> {
            self.db
                .query_row("SELECT v FROM settings WHERE k = ?1", [k], |r| r.get(0))
                .optional()
                .ok()
                .flatten()
        };
        let flag = |k: &str, def: bool| get(k).map_or(def, |v| v == "1");
        Settings {
            remember: flag("remember", d.remember),
            resume_mode: match get("resume_mode").as_deref() {
                Some("ask") => ResumeMode::Ask,
                Some("start") => ResumeMode::Start,
                Some("auto") => ResumeMode::Auto,
                _ => d.resume_mode,
            },
            min_duration: get("min_duration").and_then(|v| v.parse().ok()).unwrap_or(d.min_duration),
            clear_finished: flag("clear_finished", d.clear_finished),
            clear_on_exit: flag("clear_on_exit", d.clear_on_exit),
            fill_folder: flag("fill_folder", d.fill_folder),
            loop_mode: get("loop_mode").and_then(|v| LoopMode::parse(&v)).unwrap_or(d.loop_mode),
            shuffle: flag("shuffle", d.shuffle),
            hwdec: flag("hwdec", d.hwdec),
            aspect: get("aspect").unwrap_or(d.aspect),
            sub_auto: flag("sub_auto", d.sub_auto),
            sub_size: get("sub_size").and_then(|v| v.parse().ok()).unwrap_or(d.sub_size),
            sub_pos: get("sub_pos").and_then(|v| v.parse().ok()).unwrap_or(d.sub_pos),
            sub_lang: get("sub_lang").unwrap_or(d.sub_lang),
        }
    }

    pub fn save_settings(&self, s: &Settings) {
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        let mode = match s.resume_mode {
            ResumeMode::Auto => "auto",
            ResumeMode::Ask => "ask",
            ResumeMode::Start => "start",
        };
        for (k, v) in [
            ("remember", b(s.remember)),
            ("resume_mode", mode.to_string()),
            ("min_duration", s.min_duration.to_string()),
            ("clear_finished", b(s.clear_finished)),
            ("clear_on_exit", b(s.clear_on_exit)),
            ("fill_folder", b(s.fill_folder)),
            ("loop_mode", s.loop_mode.as_str().to_string()),
            ("shuffle", b(s.shuffle)),
            ("hwdec", b(s.hwdec)),
            ("aspect", s.aspect.clone()),
            ("sub_auto", b(s.sub_auto)),
            ("sub_size", s.sub_size.to_string()),
            ("sub_pos", s.sub_pos.to_string()),
            ("sub_lang", s.sub_lang.clone()),
        ] {
            let _ = self.db.execute(
                "INSERT INTO settings (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
                params![k, v],
            );
        }
    }

    // ---- Resume positions ----

    pub fn position(&self, key: &str) -> Option<Entry> {
        self.db
            .query_row(
                "SELECT path, title, position, duration FROM history WHERE key = ?1",
                [key],
                |r| Ok(Entry { path: r.get(0)?, title: r.get(1)?, position: r.get(2)?, duration: r.get(3)? }),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn save_position(&self, key: &str, path: &str, title: &str, position: f64, duration: f64) {
        let _ = self.db.execute(
            "INSERT INTO history (key, path, title, position, duration, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(key) DO UPDATE SET path = ?2, title = ?3, position = ?4,
                                            duration = ?5, updated_at = ?6",
            params![key, path, title, position, duration, now_secs()],
        );
    }

    pub fn forget(&self, key: &str) {
        let _ = self.db.execute("DELETE FROM history WHERE key = ?1", [key]);
    }

    /// Most recently watched files that still exist, newest first.
    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        let Ok(mut stmt) = self.db.prepare(
            "SELECT path, title, position, duration FROM history ORDER BY updated_at DESC LIMIT ?1",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map([limit as i64 * 2], |r| {
            Ok(Entry { path: r.get(0)?, title: r.get(1)?, position: r.get(2)?, duration: r.get(3)? })
        });
        rows.map(|it| it.flatten().filter(|e| Path::new(&e.path).exists()).take(limit).collect())
            .unwrap_or_default()
    }

    pub fn history_count(&self) -> i64 {
        self.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap_or(0)
    }

    /// Clears resume positions and the recent list. Play counts are kept.
    pub fn clear_history(&self) {
        let _ = self.db.execute("DELETE FROM history", []);
    }

    // ---- Play counts ----

    /// Records a play unless the same file was already counted within
    /// [`DEDUPE_SECS`]. Returns the event row to update later, or None.
    pub fn record_play(&self, key: &str, path: &str, started_at: i64) -> Option<i64> {
        let recent: Option<i64> = self
            .db
            .query_row(
                "SELECT id FROM play_events WHERE key = ?1 AND started_at > ?2
                 ORDER BY started_at DESC LIMIT 1",
                params![key, started_at - DEDUPE_SECS],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if recent.is_some() {
            return None;
        }
        self.db
            .execute(
                "INSERT INTO play_events (key, path, started_at) VALUES (?1, ?2, ?3)",
                params![key, path, started_at],
            )
            .ok()?;
        Some(self.db.last_insert_rowid())
    }

    pub fn update_watched(&self, event_id: i64, watched_secs: f64) {
        let _ = self.db.execute(
            "UPDATE play_events SET watched_secs = ?2 WHERE id = ?1",
            params![event_id, watched_secs],
        );
    }

    pub fn play_count(&self, key: &str) -> i64 {
        self.db
            .query_row("SELECT COUNT(*) FROM play_events WHERE key = ?1", [key], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn total_plays(&self) -> i64 {
        self.db.query_row("SELECT COUNT(*) FROM play_events", [], |r| r.get(0)).unwrap_or(0)
    }

    pub fn clear_play_stats(&self) {
        let _ = self.db.execute("DELETE FROM play_events", []);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Store {
        Store::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn settings_round_trip() {
        let s = mem();
        assert_eq!(s.load_settings(), Settings::default());
        let custom = Settings {
            remember: false,
            resume_mode: ResumeMode::Ask,
            min_duration: 600,
            clear_finished: false,
            clear_on_exit: true,
            fill_folder: false,
            loop_mode: LoopMode::One,
            shuffle: true,
            hwdec: false,
            aspect: "4:3".into(),
            sub_auto: false,
            sub_size: 60,
            sub_pos: 90,
            sub_lang: "en".into(),
        };
        s.save_settings(&custom);
        assert_eq!(s.load_settings(), custom);
    }

    #[test]
    fn clear_history_keeps_play_counts() {
        let s = mem();
        s.save_position("k", "/a.mp4", "a", 42.0, 600.0);
        s.record_play("k", "/a.mp4", 1_000);
        s.clear_history();
        assert!(s.position("k").is_none());
        assert_eq!(s.play_count("k"), 1);
    }

    #[test]
    fn plays_within_window_are_deduped() {
        let s = mem();
        assert!(s.record_play("k", "/a.mp4", 10_000).is_some());
        assert!(s.record_play("k", "/a.mp4", 10_000 + 60).is_none());
        assert!(s.record_play("k", "/a.mp4", 10_000 + DEDUPE_SECS + 1).is_some());
        assert_eq!(s.play_count("k"), 2);
    }
}

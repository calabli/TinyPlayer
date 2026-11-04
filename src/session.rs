//! Tracks the file being watched: saves its resume position and decides when
//! a play counts. All rules live here so the UI only forwards events.

use std::time::Instant;

use crate::history::{self, Entry, ResumeMode, Settings, Store};
use crate::player::State;

/// Don't write a resume position until the viewer has actually watched this
/// long — otherwise opening a file and closing it again would wipe the
/// position saved last time.
const MIN_WATCH_BEFORE_SAVE: f64 = 5.0;
/// Positions closer than this to the start aren't worth resuming.
const MIN_RESUME_POS: f64 = 5.0;
const SAVE_EVERY_SECS: f64 = 10.0;
/// Gaps longer than this between ticks (sleep, hung frame) aren't watch time.
const MAX_TICK: f64 = 1.0;

/// What the UI should do right after a file loads.
pub enum Resume {
    None,
    /// Already seeked; tell the viewer.
    Resumed(f64),
    /// Ask the viewer first.
    Ask(f64),
}

struct Current {
    key: Option<String>,
    path: String,
    started_at: i64,
    watched: f64,
    event_id: Option<i64>,
    counted: bool,
    since_save: f64,
    was_paused: bool,
    was_eof: bool,
    // Last known values for *this* file. mpv's state already describes the
    // next file by the time we save on a file change, so never read it then.
    title: String,
    position: f64,
    duration: f64,
    /// Play count for this file, cached so the UI doesn't query every frame.
    plays: i64,
}

pub struct Tracker {
    pub store: Option<Store>,
    pub settings: Settings,
    /// Session-only: nothing is written while this is on.
    pub incognito: bool,
    current: Option<Current>,
    last_tick: Option<Instant>,
}

impl Tracker {
    pub fn new() -> Self {
        let store = Store::open_default().ok();
        let settings = store.as_ref().map(|s| s.load_settings()).unwrap_or_default();
        let tracker = Self { store, settings, incognito: false, current: None, last_tick: None };
        // If the app crashed last time, "clear on exit" never ran — do it now.
        if tracker.settings.clear_on_exit {
            if let Some(s) = &tracker.store {
                s.clear_history();
            }
        }
        tracker
    }

    fn writes_enabled(&self) -> bool {
        !self.incognito && self.store.is_some()
    }

    pub fn save_settings(&self) {
        if let Some(s) = &self.store {
            s.save_settings(&self.settings);
        }
    }

    /// Call when mpv reports a newly loaded file. Returns the resume action;
    /// the caller performs the seek for [`Resume::Resumed`].
    pub fn file_loaded(&mut self, state: &State) -> Resume {
        self.finish();
        let key = history::file_key(&state.path);
        let plays = match (&self.store, &key) {
            (Some(s), Some(k)) => s.play_count(k),
            _ => 0,
        };
        self.current = Some(Current {
            key: key.clone(),
            path: state.path.clone(),
            started_at: history::now_secs(),
            watched: 0.0,
            event_id: None,
            counted: false,
            since_save: 0.0,
            was_paused: state.paused,
            was_eof: false,
            title: state.title.clone(),
            position: 0.0,
            duration: state.duration,
            plays,
        });
        self.last_tick = None;

        let (Some(store), Some(key)) = (&self.store, key) else { return Resume::None };
        if !self.settings.remember {
            return Resume::None;
        }
        let Some(Entry { position, .. }) = store.position(&key) else { return Resume::None };
        if position < MIN_RESUME_POS {
            return Resume::None;
        }
        match self.settings.resume_mode {
            ResumeMode::Auto => Resume::Resumed(position),
            ResumeMode::Ask => Resume::Ask(position),
            ResumeMode::Start => Resume::None,
        }
    }

    /// Call every frame / wakeup with the latest state.
    pub fn tick(&mut self, state: &State) {
        let now = Instant::now();
        let dt = self.last_tick.map_or(0.0, |t| now.duration_since(t).as_secs_f64().min(MAX_TICK));
        self.last_tick = Some(now);

        let Some(cur) = &mut self.current else { return };
        if state.path != cur.path || state.idle {
            return;
        }
        cur.title.clone_from(&state.title);
        cur.position = state.position;
        cur.duration = state.duration;
        let playing = !state.idle && !state.paused && !state.eof;
        if playing {
            // Wall-clock time: 2× speed for 10 s still counts as 10 s watched.
            cur.watched += dt;
            cur.since_save += dt;
        }

        // Count the play once enough has been watched.
        if !cur.counted && state.duration > 0.0 {
            let needed = history::COUNT_AFTER_SECS.min(state.duration * 0.1);
            if cur.watched >= needed {
                cur.counted = true;
                if !self.incognito {
                    if let (Some(store), Some(key)) = (&self.store, &cur.key) {
                        cur.event_id = store.record_play(key, &cur.path, cur.started_at);
                        if cur.event_id.is_some() {
                            cur.plays += 1;
                        }
                    }
                }
            }
        }

        let paused_now = state.paused && !cur.was_paused;
        let eof_now = state.eof && !cur.was_eof;
        cur.was_paused = state.paused;
        cur.was_eof = state.eof;
        if paused_now || eof_now || cur.since_save >= SAVE_EVERY_SECS {
            self.save();
        }
    }

    /// Writes the current position (or forgets it, per the settings).
    fn save(&mut self) {
        let writes = self.writes_enabled();
        let settings = self.settings.clone();
        let Some(cur) = &mut self.current else { return };
        cur.since_save = 0.0;
        let (Some(store), true) = (&self.store, writes) else { return };
        if let Some(id) = cur.event_id {
            store.update_watched(id, cur.watched);
        }
        let Some(key) = &cur.key else { return };
        if !settings.remember || cur.watched < MIN_WATCH_BEFORE_SAVE {
            return;
        }
        let (pos, dur) = (cur.position, cur.duration);
        let too_short = dur > 0.0 && dur < settings.min_duration as f64;
        let finished = cur.was_eof || (dur > 0.0 && pos / dur >= history::FINISHED_FRACTION);
        if too_short || (finished && settings.clear_finished) || pos < MIN_RESUME_POS {
            store.forget(key);
        } else {
            store.save_position(key, &cur.path, &cur.title, pos, dur);
        }
    }

    /// Saves and closes the current session (file change or exit).
    pub fn finish(&mut self) {
        if self.current.is_some() {
            self.save();
        }
        self.current = None;
    }

    /// Call once on shutdown.
    pub fn shutdown(&mut self) {
        self.finish();
        if self.settings.clear_on_exit {
            if let Some(s) = &self.store {
                s.clear_history();
            }
        }
    }

    pub fn recent(&self, n: usize) -> Vec<Entry> {
        self.store.as_ref().map(|s| s.recent(n)).unwrap_or_default()
    }

    /// (saved positions, total plays)
    pub fn stats(&self) -> (i64, i64) {
        self.store.as_ref().map_or((0, 0), |s| (s.history_count(), s.total_plays()))
    }

    pub fn current_play_count(&self) -> i64 {
        self.current.as_ref().map_or(0, |c| c.plays)
    }
}

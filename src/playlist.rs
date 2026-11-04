//! The play queue. Owned by the app (not mpv's internal playlist) so that
//! loop/shuffle rules, reordering and folder auto-fill stay under our control.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "flv", "wmv", "m4v", "ts", "m2ts", "mpg", "mpeg", "3gp",
    "ogv", "rmvb", "mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoopMode {
    /// Stop after the last item.
    Off,
    /// Wrap around to the first item.
    All,
    /// Repeat the current item.
    One,
}

impl LoopMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "all" => Some(Self::All),
            "one" => Some(Self::One),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: String,
    pub title: String,
}

impl Item {
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        let title = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        Self { path, title }
    }
}

#[derive(Debug)]
pub struct Playlist {
    pub items: Vec<Item>,
    pub current: Option<usize>,
    pub loop_mode: LoopMode,
    shuffle: bool,
    /// Play order when shuffling: a permutation of item indices.
    order: Vec<usize>,
    rng: u64,
    /// Bumped whenever the queue is replaced or edited, so a background
    /// folder scan that finishes late can tell its result is stale.
    generation: u64,
    /// Folder scan requested by the last `set_files`: (generation, dir, file name).
    pending_fill: Option<(u64, PathBuf, String)>,
}

pub fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA_EXTS.iter().any(|m| m.eq_ignore_ascii_case(e)))
}

/// Media files directly inside `dir`, in natural order ("第2讲" before "第10讲").
/// Uses the directory entry's own file type (no extra stat per file), which
/// matters on network drives and big folders.
pub fn scan_dir(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| is_media(p))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.sort_by(|a, b| natural_cmp(file_name(a), file_name(b)));
    files
}

fn file_name(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// Compares strings treating runs of digits as numbers, case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

impl Playlist {
    pub fn new(loop_mode: LoopMode, shuffle: bool) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9, |d| d.as_nanos() as u64)
            | 1;
        Self {
            items: Vec::new(),
            current: None,
            loop_mode,
            shuffle,
            order: Vec::new(),
            rng: seed,
            generation: 0,
            pending_fill: None,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Replaces the queue with `paths` and returns the index to play first.
    ///
    /// Never blocks on a folder scan for a single file: the queue starts with
    /// just that file so playback begins at once, and when `fill_folder` is
    /// set a scan of its folder is left in [`Self::take_pending_fill`] for the
    /// caller to run in the background and hand back to [`Self::merge_folder`].
    /// A folder path is expanded right away (there's nothing to play otherwise).
    pub fn set_files(&mut self, paths: &[String], fill_folder: bool) -> Option<usize> {
        self.generation += 1;
        self.pending_fill = None;
        let mut items = Vec::new();
        for p in paths {
            let path = Path::new(p);
            if path.is_dir() {
                items.extend(scan_dir(path));
            } else {
                items.push(p.clone());
            }
        }
        if let [single] = paths {
            let p = Path::new(single);
            if fill_folder && !p.is_dir() && is_media(p) {
                if let (Some(dir), Some(name)) = (p.parent(), p.file_name()) {
                    let name = name.to_string_lossy().into_owned();
                    self.pending_fill = Some((self.generation, dir.to_path_buf(), name));
                }
            }
        }
        self.items = items.into_iter().map(Item::new).collect();
        if self.items.is_empty() {
            self.current = None;
            return None;
        }
        self.current = Some(0);
        self.reshuffle();
        self.current
    }

    /// The folder scan requested by the last [`Self::set_files`], if any:
    /// (generation, folder, name of the playing file).
    pub fn take_pending_fill(&mut self) -> Option<(u64, PathBuf, String)> {
        self.pending_fill.take()
    }

    /// Adopts a finished folder scan: the queue becomes the folder's media
    /// files with the playing file selected. Ignored if the queue changed in
    /// the meantime (another file opened, items reordered/removed).
    /// Returns true if the queue was updated.
    pub fn merge_folder(&mut self, generation: u64, playing_name: &str, siblings: Vec<String>) -> bool {
        if generation != self.generation || self.items.len() != 1 || siblings.len() < 2 {
            return false;
        }
        let same = |s: &str| {
            let n = Path::new(s).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if cfg!(any(target_os = "windows", target_os = "macos")) {
                n.to_lowercase() == playing_name.to_lowercase()
            } else {
                n == playing_name
            }
        };
        let Some(i) = siblings.iter().position(|s| same(s)) else { return false };
        // Keep the exact path string mpv is playing for the current item.
        let current_path = self.items[0].path.clone();
        self.items = siblings.into_iter().map(Item::new).collect();
        self.items[i] = Item::new(current_path);
        self.current = Some(i);
        self.reshuffle();
        true
    }

    pub fn select(&mut self, i: usize) -> Option<usize> {
        if i < self.items.len() {
            self.current = Some(i);
        }
        self.current
    }

    /// Next item to play. `auto` is true when the current item ended on its
    /// own (as opposed to the user pressing "next").
    pub fn next(&mut self, auto: bool) -> Option<usize> {
        let cur = self.current?;
        if auto && self.loop_mode == LoopMode::One {
            return Some(cur);
        }
        let pos = self.order_pos(cur);
        let n = self.items.len();
        let next_pos = if pos + 1 < n {
            pos + 1
        } else if self.loop_mode == LoopMode::All || (!auto && n > 1 && self.loop_mode == LoopMode::One) {
            if self.shuffle {
                self.reshuffle_from_scratch();
            }
            0
        } else {
            return None;
        };
        self.current = Some(self.order_at(next_pos));
        self.current
    }

    pub fn prev(&mut self) -> Option<usize> {
        let cur = self.current?;
        let pos = self.order_pos(cur);
        let n = self.items.len();
        let prev_pos = if pos > 0 {
            pos - 1
        } else if self.loop_mode != LoopMode::Off && n > 1 {
            n - 1
        } else {
            return None;
        };
        self.current = Some(self.order_at(prev_pos));
        self.current
    }

    /// Whether [`Self::prev`] would move (used to grey out the button).
    pub fn has_prev(&self) -> bool {
        let Some(cur) = self.current else { return false };
        self.order_pos(cur) > 0 || (self.loop_mode != LoopMode::Off && self.items.len() > 1)
    }

    /// Whether a manual [`Self::next`] would move.
    pub fn has_next(&self) -> bool {
        let Some(cur) = self.current else { return false };
        self.order_pos(cur) + 1 < self.items.len() || (self.loop_mode != LoopMode::Off && self.items.len() > 1)
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        self.reshuffle();
    }

    /// Removes item `i`. Returns true if the playing item was removed.
    pub fn remove(&mut self, i: usize) -> bool {
        if i >= self.items.len() {
            return false;
        }
        self.generation += 1;
        self.items.remove(i);
        let removed_current = self.current == Some(i);
        self.current = match self.current {
            Some(c) if c == i => {
                if self.items.is_empty() { None } else { Some(i.min(self.items.len() - 1)) }
            }
            Some(c) if c > i => Some(c - 1),
            other => other,
        };
        self.reshuffle();
        removed_current
    }

    /// Moves item `from` so it ends up at index `to`.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let n = self.items.len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.generation += 1;
        let item = self.items.remove(from);
        self.items.insert(to, item);
        if let Some(c) = self.current {
            self.current = Some(if c == from {
                to
            } else if from < c && c <= to {
                c - 1
            } else if to <= c && c < from {
                c + 1
            } else {
                c
            });
        }
        self.reshuffle();
    }

    // ---- Shuffle order ----

    fn order_pos(&self, idx: usize) -> usize {
        if self.shuffle {
            self.order.iter().position(|&i| i == idx).unwrap_or(0)
        } else {
            idx
        }
    }

    fn order_at(&self, pos: usize) -> usize {
        if self.shuffle { self.order[pos] } else { pos }
    }

    /// New random order with the current item first, so the rest of the
    /// list plays exactly once before anything repeats.
    fn reshuffle(&mut self) {
        let n = self.items.len();
        let mut rest: Vec<usize> = (0..n).filter(|&i| Some(i) != self.current).collect();
        for i in (1..rest.len()).rev() {
            let j = (self.rand() % (i as u64 + 1)) as usize;
            rest.swap(i, j);
        }
        self.order = self.current.into_iter().chain(rest).collect();
    }

    fn reshuffle_from_scratch(&mut self) {
        let keep = self.current;
        self.current = None;
        self.reshuffle();
        self.current = keep;
    }

    fn rand(&mut self) -> u64 {
        // xorshift64 — plenty for shuffling a playlist.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize, mode: LoopMode) -> Playlist {
        let mut p = Playlist::new(mode, false);
        p.items = (0..n).map(|i| Item::new(format!("/v/{i}.mp4"))).collect();
        p.current = Some(0);
        p
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["第10讲.mp4", "第2讲.mp4", "第1讲.mp4", "Intro.mp4", "ep02.mkv", "EP1.mkv"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["EP1.mkv", "ep02.mkv", "Intro.mp4", "第1讲.mp4", "第2讲.mp4", "第10讲.mp4"]);
    }

    #[test]
    fn loop_off_stops_at_end() {
        let mut p = list(2, LoopMode::Off);
        assert_eq!(p.next(true), Some(1));
        assert_eq!(p.next(true), None);
        assert_eq!(p.current, Some(1));
    }

    #[test]
    fn loop_all_wraps_and_one_repeats() {
        let mut p = list(2, LoopMode::All);
        p.current = Some(1);
        assert_eq!(p.next(true), Some(0));
        p.loop_mode = LoopMode::One;
        assert_eq!(p.next(true), Some(0));
        // Pressing "next" still moves on in repeat-one.
        assert_eq!(p.next(false), Some(1));
    }

    #[test]
    fn shuffle_plays_everything_once() {
        let mut p = list(6, LoopMode::Off);
        p.set_shuffle(true);
        let mut seen = vec![p.current.unwrap()];
        while let Some(i) = p.next(true) {
            seen.push(i);
        }
        seen.sort();
        assert_eq!(seen, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn folder_fill_is_deferred_and_merged() {
        let dir = std::env::temp_dir().join(format!("tp-fill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["第1讲.mp4", "第2讲.mp4", "第10讲.mp4", "notes.txt"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let file = dir.join("第2讲.mp4").to_string_lossy().into_owned();
        let mut p = Playlist::new(LoopMode::Off, false);

        // Plays immediately with just the opened file.
        assert_eq!(p.set_files(std::slice::from_ref(&file), true), Some(0));
        assert_eq!(p.len(), 1);
        let (generation, folder, name) = p.take_pending_fill().expect("scan requested");

        // Background scan result merges in, keeping the playing file current.
        assert!(p.merge_folder(generation, &name, scan_dir(&folder)));
        let titles: Vec<_> = p.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["第1讲.mp4", "第2讲.mp4", "第10讲.mp4"]);
        assert_eq!(p.current, Some(1));

        // A stale result (another file opened since) is ignored.
        p.set_files(std::slice::from_ref(&file), true);
        let (g2, ..) = p.take_pending_fill().unwrap();
        p.set_files(std::slice::from_ref(&file), true);
        assert!(!p.merge_folder(g2, &name, scan_dir(&folder)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_and_remove_keep_current() {
        let mut p = list(4, LoopMode::Off);
        p.current = Some(2);
        p.move_item(0, 3); // [1,2,3,0] → current item "2" is now at 1
        assert_eq!(p.current, Some(1));
        assert_eq!(p.items[1].title, "2.mp4");
        assert!(!p.remove(0)); // [2,3,0]
        assert_eq!(p.current, Some(0));
        assert!(p.remove(0)); // removed playing item → next one takes its place
        assert_eq!(p.items[p.current.unwrap()].title, "3.mp4");
    }
}

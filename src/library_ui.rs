//! Library home screen, shown whenever nothing is playing:
//! - top bar: search, Library/Folders mode switch, add folder, open file, settings
//! - nav: views (all / continue / recent / most played), collections, tags, folders
//! - content: a virtualized thumbnail grid, or a folder browser with breadcrumbs

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2,
};

use crate::app::fmt_time;
use crate::i18n::{tf, tr};
use crate::icons::{self, Icon};
use crate::library::{Collection, Folder, Library, Video};
use crate::library_manage::{self, Manager};
use crate::playlist::natural_cmp;
use crate::theme::{self, ACCENT, TEXT};

const TOP_H: f32 = 56.0;
const NAV_W: f32 = 188.0;
const CARD_MIN_W: f32 = 200.0;
const GAP: f32 = 16.0;
const META_H: f32 = 54.0;
/// Thumbnails decoded per frame, so scrolling a big library stays smooth.
const THUMB_LOADS_PER_FRAME: usize = 4;
const FINISHED: f32 = 0.95;
/// Thumbnail textures kept in memory (~0.5 MB each at 480 px). The least
/// recently shown ones are dropped beyond this and reloaded on demand.
const MAX_THUMBS: usize = 160;

#[derive(Clone, Copy, PartialEq)]
enum View {
    All,
    Continue,
    Recent,
    MostPlayed,
    /// None = every favorited video.
    Favorites(Option<i64>),
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Grid,
    Folders,
    Manage,
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Added,
    Name,
    Duration,
    Played,
    Plays,
}

pub enum Action {
    None,
    Play(String),
    /// Queue these files in order and start with the first.
    PlayAll(Vec<String>),
    AddFolder,
    OpenFile,
    Settings,
    /// Back to the player's start page.
    Home,
    Lock,
}

/// What one grid cell shows.
#[derive(Clone)]
enum Cell {
    Video(i64),
    /// A sub-folder in folder mode: display name and its location.
    Folder { name: String, loc: FolderLoc },
}

/// A place in the folder browser: a watched root plus sub-directories.
#[derive(Clone, PartialEq, Eq, Hash)]
struct FolderLoc {
    root: i64,
    parts: Vec<String>,
}

/// What decides the grid's contents; the list is rebuilt when this changes.
#[derive(PartialEq)]
struct GridKey {
    revision: u64,
    view: View,
    sort: Sort,
    search: String,
    tags: Vec<String>,
    folder: Option<i64>,
}

/// Sorting and counting thousands of videos on every frame would make the
/// page sluggish, so the results are kept until the library or view changes.
#[derive(Default)]
struct Cached {
    grid: Option<(GridKey, Rc<Vec<Cell>>, f64)>,
    folder: Option<(u64, Option<FolderLoc>, Rc<Vec<Cell>>, Rc<Vec<String>>)>,
    folder_stats: HashMap<FolderLoc, (Vec<i64>, usize, f64, usize)>,
    nav: Option<(u64, [usize; 3], Vec<(String, usize)>)>,
    /// Private mode was on for `folder_stats`.
    hidden: bool,
}

/// Background work on the library, for the progress bar.
#[derive(Default)]
struct Work {
    scanned: Option<usize>,
    probing: Option<(usize, usize)>,
    /// When thumbnails started and how many were done then, for the estimate.
    started: Option<(Instant, usize)>,
}

pub struct LibraryView {
    /// Position of each video in `videos`.
    index: HashMap<i64, usize>,
    cached: Cached,
    work: Work,
    lib: Option<Library>,
    videos: Vec<Video>,
    folders: Vec<Folder>,
    collections: Vec<Collection>,
    dirty: bool,
    view: View,
    mode: Mode,
    sort: Sort,
    search: String,
    tags_sel: Vec<String>,
    folder_sel: Option<i64>,
    /// Folder browser location; None = list of watched roots.
    folder_loc: Option<FolderLoc>,
    /// id -> (thumbnail path, texture, frame it was last drawn)
    thumbs: HashMap<i64, (String, TextureHandle, u64)>,
    frame: u64,
    thumb_failed: HashSet<i64>,
    manager: Manager,
    /// Counts reloads of `videos`.
    revision: u64,
    /// A disk came or went, so the folders need scanning again.
    rescan: bool,
    /// The library has a password, so it can be locked again.
    pub lockable: bool,
    /// Private mode: leaves out what was watched and how often.
    pub hide_history: bool,
    /// (video id, text being typed, focus requested yet)
    tag_edit: Option<(i64, String, bool)>,
    tag_edit_pos: egui::Pos2,
    /// Collection picker opened from a card's star: (video id, anchor).
    fav_picker: Option<(i64, egui::Pos2)>,
    /// True on the frame the picker opened, so the opening click doesn't
    /// count as a click outside it.
    fav_picker_fresh: bool,
    /// New-collection name typed inside the picker (separate from the nav's).
    fav_new_name: String,
    /// Name typed for a new collection (nav "+" or the picker).
    new_collection: Option<String>,
}

impl LibraryView {
    pub fn new() -> Self {
        Self {
            lib: Library::open(&Library::db_path()).ok(),
            videos: Vec::new(),
            folders: Vec::new(),
            collections: Vec::new(),
            dirty: true,
            view: View::All,
            mode: Mode::Grid,
            sort: Sort::Added,
            search: String::new(),
            tags_sel: Vec::new(),
            folder_sel: None,
            folder_loc: None,
            thumbs: HashMap::new(),
            frame: 0,
            thumb_failed: HashSet::new(),
            manager: Manager::new(),
            revision: 0,
            rescan: false,
            lockable: false,
            hide_history: false,
            tag_edit: None,
            tag_edit_pos: pos2(0.0, 0.0),
            fav_picker: None,
            fav_picker_fresh: false,
            fav_new_name: String::new(),
            new_collection: None,
            index: HashMap::new(),
            cached: Cached::default(),
            work: Work::default(),
        }
    }

    pub fn scanned(&mut self, found: usize) {
        self.work.scanned = Some(found);
    }

    pub fn probed(&mut self, done: usize, total: usize) {
        self.work.scanned = None;
        self.work.probing = Some((done, total));
        self.work.started.get_or_insert((Instant::now(), done));
    }

    pub fn scan_finished(&mut self) {
        self.work = Work::default();
    }

    /// Forgets every folder, video, tag, collection and set. Files stay.
    pub fn clear(&mut self) {
        self.manager.shut_down(&egui::Context::default(), self.lib.as_ref(), &self.videos);
        if let Some(lib) = &self.lib {
            lib.clear();
        }
        let _ = std::fs::remove_dir_all(Library::thumbs_dir());
        self.manager = Manager::new();
        self.mode = Mode::Grid;
        self.search.clear();
        self.tags_sel.clear();
        self.folder_sel = None;
        self.folder_loc = None;
        self.dirty = true;
    }

    pub fn shut_down(&mut self, ctx: &egui::Context) {
        self.manager.shut_down(ctx, self.lib.as_ref(), &self.videos);
    }

    /// True once when the folders should be scanned again.
    pub fn take_rescan(&mut self) -> bool {
        std::mem::take(&mut self.rescan)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn has_folders(&mut self) -> bool {
        self.reload_if_dirty();
        !self.folders.is_empty()
    }

    pub fn add_folder(&mut self, path: &str) -> bool {
        let added = self.lib.as_ref().and_then(|l| l.add_folder(path)).is_some();
        self.dirty = true;
        added
    }

    /// Whether the file at `path` is favorited; None if it isn't in the library.
    pub fn is_favorite_path(&self, path: &str) -> Option<bool> {
        let lib = self.lib.as_ref()?;
        let id = lib.video_id_for_path(path)?;
        Some(lib.is_favorite(id))
    }

    /// Quick favorite for the player. Returns the new state, or None if the
    /// file isn't in the library.
    pub fn toggle_favorite_path(&mut self, path: &str) -> Option<bool> {
        let lib = self.lib.as_ref()?;
        let id = lib.video_id_for_path(path)?;
        let on = lib.toggle_favorite(id);
        self.dirty = true;
        Some(on)
    }

    fn reload_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.revision += 1;
        if let Some(lib) = &self.lib {
            self.videos = lib.videos();
            self.index = self.videos.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
            self.folders = lib.folders();
            self.collections = lib.collections();
        }
        // Thumbnails may have been regenerated for changed files.
        let current: HashMap<i64, Option<&String>> = self.videos.iter().map(|v| (v.id, v.thumb.as_ref())).collect();
        self.thumbs.retain(|id, (path, _, _)| current.get(id).copied().flatten() == Some(&*path));
        self.thumb_failed.retain(|id| current.contains_key(id));
        if self.folder_sel.is_some_and(|f| !self.folders.iter().any(|x| x.id == f)) {
            self.folder_sel = None;
        }
        if self.folder_loc.as_ref().is_some_and(|l| !self.folders.iter().any(|x| x.id == l.root)) {
            self.folder_loc = None;
        }
        if let View::Favorites(Some(c)) = self.view {
            if !self.collections.iter().any(|x| x.id == c) {
                self.view = View::Favorites(None);
            }
        }
    }

    fn video(&self, id: i64) -> Option<&Video> {
        self.videos.get(*self.index.get(&id)?)
    }

    fn refresh_nav_counts(&mut self) {
        if self.cached.nav.as_ref().is_some_and(|(revision, ..)| *revision == self.revision) {
            return;
        }
        let continue_n = self.videos.iter().filter(|v| v.progress().is_some_and(|x| x < FINISHED)).count();
        let week_ago = crate::history::now_secs() - 7 * 24 * 3600;
        let recent_n = self.videos.iter().filter(|v| v.added_at >= week_ago).count();
        let played_n = self.videos.iter().filter(|v| v.plays > 0).count();
        let mut tags: HashMap<String, (String, usize)> = HashMap::new();
        for t in self.videos.iter().flat_map(|v| &v.tags) {
            tags.entry(t.to_lowercase()).or_insert_with(|| (t.clone(), 0)).1 += 1;
        }
        let mut tag_counts: Vec<(String, usize)> = tags.into_values().collect();
        tag_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| natural_cmp(&a.0, &b.0)));
        self.cached.nav = Some((self.revision, [continue_n, recent_n, played_n], tag_counts));
    }

    /// The grid's cells and their total length.
    fn grid_cells(&mut self) -> (Rc<Vec<Cell>>, f64) {
        let key = GridKey {
            revision: self.revision,
            view: self.view,
            sort: self.sort,
            search: self.search.clone(),
            tags: self.tags_sel.clone(),
            folder: self.folder_sel,
        };
        if let Some((cached, cells, total)) = &self.cached.grid {
            if *cached == key {
                return (cells.clone(), *total);
            }
        }
        let videos = self.filtered();
        let total = videos.iter().map(|v| v.duration).sum();
        let cells: Rc<Vec<Cell>> = Rc::new(videos.iter().map(|v| Cell::Video(v.id)).collect());
        self.cached.grid = Some((key, cells.clone(), total));
        (cells, total)
    }

    fn filtered(&self) -> Vec<&Video> {
        let words: Vec<String> = self.search.to_lowercase().split_whitespace().map(str::to_string).collect();
        let week_ago = crate::history::now_secs() - 7 * 24 * 3600;
        let mut out: Vec<&Video> = self
            .videos
            .iter()
            .filter(|v| match self.view {
                View::All => true,
                View::Continue => v.progress().is_some_and(|p| p < FINISHED),
                View::Recent => v.added_at >= week_ago,
                View::MostPlayed => v.plays > 0,
                View::Favorites(None) => !v.collections.is_empty(),
                View::Favorites(Some(c)) => v.collections.contains(&c),
            })
            .filter(|v| self.folder_sel.is_none_or(|f| v.folder_id == f))
            .filter(|v| self.tags_sel.iter().all(|t| v.tags.iter().any(|vt| vt.eq_ignore_ascii_case(t))))
            .filter(|v| matches_search(v, &words))
            .collect();
        let sort = match self.view {
            View::Continue => Sort::Played,
            View::MostPlayed => Sort::Plays,
            _ => self.sort,
        };
        match sort {
            Sort::Played => out.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(b.added_at.cmp(&a.added_at))),
            Sort::Added => out.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(b.id.cmp(&a.id))),
            Sort::Name => out.sort_by(|a, b| natural_cmp(&a.title, &b.title)),
            Sort::Duration => out.sort_by(|a, b| b.duration.total_cmp(&a.duration)),
            Sort::Plays => out.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.last_played.cmp(&a.last_played))),
        }
        out
    }

    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect) -> Action {
        if self.manager.poll(ui.ctx(), self.lib.as_ref(), &self.videos) {
            self.dirty = true;
        }
        self.rescan |= self.manager.watch_disks(ui.ctx(), &self.folders);
        self.reload_if_dirty();
        if self.cached.hidden != self.hide_history {
            self.cached.hidden = self.hide_history;
            self.cached.folder_stats.clear();
        }
        if self.hide_history {
            if matches!(self.view, View::Continue | View::MostPlayed) {
                self.view = View::All;
            }
            if matches!(self.sort, Sort::Played | Sort::Plays) {
                self.sort = Sort::Added;
            }
        }
        let mut action = Action::None;
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, theme::bg());

        let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP_H));
        if let Some(a) = self.top_bar(ui, &p, top) {
            action = a;
        }
        let nav = Rect::from_min_max(pos2(rect.left(), top.bottom()), pos2(rect.left() + NAV_W, rect.bottom()));
        let status = self.status_bar(ui, &p, rect);
        let content = Rect::from_min_max(pos2(nav.right(), top.bottom()), pos2(rect.right(), rect.bottom() - status));
        if self.mode == Mode::Manage && !self.folders.is_empty() {
            if let Some(a) = self.manage(ui, &p, nav, content) {
                action = a;
            }
            return action;
        }
        self.nav(ui, &p, nav);

        let content_action = if self.folders.is_empty() {
            empty_library(ui, &p, content)
        } else if self.mode == Mode::Folders {
            self.folder_browser(ui, &p, content)
        } else {
            self.grid_view(ui, &p, content)
        };
        if !matches!(content_action, Action::None) {
            action = content_action;
        }
        self.tag_editor(ui);
        self.favorite_picker(ui);
        action
    }

    /// Shows what's running in the background. Returns the height it took.
    fn status_bar(&self, ui: &mut egui::Ui, p: &egui::Painter, rect: Rect) -> f32 {
        const H: f32 = 30.0;
        let (text, fraction) = match (self.work.scanned, self.work.probing) {
            (_, Some((done, total))) if done < total => {
                let left = self.work.started.and_then(|(at, first)| {
                    let (secs, finished) = (at.elapsed().as_secs_f64(), done.saturating_sub(first));
                    (secs >= 3.0 && finished >= 3).then(|| (total - done) as f64 * secs / finished as f64)
                });
                let percent = done * 100 / total.max(1);
                let text = tf("Processing videos {} / {} · {}%", &[&done, &total, &percent]);
                let text = left.map_or(text.clone(), |secs| format!("{text} · {}", time_left(secs)));
                (text, Some(done as f32 / total.max(1) as f32))
            }
            (Some(found), _) => (tf("Scanning folders, found {} videos", &[&found]), None),
            _ => return 0.0,
        };
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - H), rect.max);
        p.rect_filled(bar, 0.0, theme::panel());
        p.line_segment([bar.left_top(), bar.right_top()], Stroke::new(0.5, theme::line()));
        p.text(pos2(bar.left() + 18.0, bar.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), theme::text_2());
        let track = Rect::from_min_max(pos2(bar.right() - 260.0, bar.center().y - 2.0), pos2(bar.right() - 18.0, bar.center().y + 2.0));
        p.rect_filled(track, 2.0, theme::line());
        match fraction {
            Some(f) => {
                let fill = Rect::from_min_max(track.min, pos2(track.left() + track.width() * f, track.bottom()));
                p.rect_filled(fill, 2.0, ACCENT);
            }
            None => {
                // No total yet, so a piece of the bar slides along instead.
                let t = ui.input(|i| i.time) as f32 % 1.6 / 1.6;
                let width = track.width() * 0.25;
                let left = track.left() + (track.width() - width) * t;
                p.rect_filled(Rect::from_min_size(pos2(left, track.top()), vec2(width, track.height())), 2.0, ACCENT);
                ui.ctx().request_repaint_after_secs(0.05);
            }
        }
        H
    }

    fn manage(&mut self, ui: &mut egui::Ui, p: &egui::Painter, nav: Rect, content: Rect) -> Option<Action> {
        self.cached.folder_stats.clear();
        let mut loads = 0;
        let mut waiting = false;
        let mut thumbs = HashMap::new();
        for id in self.manager.visible() {
            waiting |= self.ensure_thumb(ui.ctx(), id, &mut loads);
            if let Some(texture) = self.use_thumb(id) {
                thumbs.insert(id, texture);
            }
        }
        self.evict_thumbs();
        if waiting {
            ui.ctx().request_repaint();
        }
        let lib = self.lib.as_ref()?;
        let data = library_manage::Data {
            lib,
            videos: &self.videos,
            folders: &self.folders,
            search: &self.search,
            revision: self.revision,
            index: &self.index,
            collections: &self.collections,
            thumbs: &thumbs,
        };
        self.manager.nav(ui, p, nav, &data);
        let out = self.manager.show(ui, p, content, &data);
        if self.manager.take_changed() {
            self.dirty = true;
        }
        if out.clear_search {
            self.search.clear();
        }
        out.play.map(Action::Play)
    }

    // ---- Top bar -------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui, p: &egui::Painter, top: Rect) -> Option<Action> {
        let mut action = None;
        p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(0.5, theme::line()));
        p.text(pos2(top.left() + 20.0, top.center().y), Align2::LEFT_CENTER, "TINYPLAYER", FontId::monospace(12.0), ACCENT);

        let search_w = (top.width() - NAV_W - 730.0).clamp(120.0, 380.0);
        let search_rect = Rect::from_min_size(pos2(top.left() + NAV_W + 16.0, top.center().y - 15.0), vec2(search_w, 30.0));
        p.rect_filled(search_rect, 15.0, theme::panel());
        p.rect_stroke(search_rect, 15.0, Stroke::new(0.5, theme::line()), StrokeKind::Inside);
        search_icon(p, pos2(search_rect.left() + 16.0, search_rect.center().y), theme::text_3());
        let edit_rect = Rect::from_min_max(search_rect.min + vec2(30.0, 5.0), search_rect.max - vec2(12.0, 5.0));
        let mut search_resp = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
            search_resp = Some(
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(tr("Search titles, tags, folders"))
                        .frame(egui::Frame::NONE)
                        .text_color(theme::text())
                        .font(FontId::proportional(13.0))
                        .desired_width(edit_rect.width()),
                ),
            );
        });
        if let Some(r) = &search_resp {
            if ui.ctx().input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
                r.request_focus();
            }
            // TextEdit drops focus on Esc itself, so check lost_focus too.
            if (r.has_focus() || r.lost_focus()) && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                self.search.clear();
                r.surrender_focus();
            }
            if r.changed() && !self.search.is_empty() && self.mode == Mode::Folders {
                // Searching is a grid thing; switch so results are visible.
                self.mode = Mode::Grid;
            }
        }

        // Mode switch: Library | Folders | Manage
        let seg = Rect::from_min_size(pos2(search_rect.right() + 16.0, top.center().y - 14.0), vec2(196.0, 28.0));
        p.rect_filled(seg, 14.0, theme::panel());
        for (i, (mode, label)) in [(Mode::Grid, tr("Library")), (Mode::Folders, tr("Folders")), (Mode::Manage, tr("Manage"))].into_iter().enumerate() {
            let r = Rect::from_min_size(seg.min + vec2(2.0 + i as f32 * 64.0, 2.0), vec2(64.0, 24.0));
            let resp = ui.interact(r, ui.scope_id().with(("mode", i)), Sense::click());
            let on = self.mode == mode;
            if on {
                p.rect_filled(r, 12.0, theme::accent_alpha(36));
            }
            let c = if on { ACCENT } else if resp.hovered() { theme::text() } else { theme::text_2() };
            p.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), c);
            if resp.clicked() {
                self.mode = mode;
            }
        }

        // Right side.
        let rx = top.right() - 20.0;
        let gear = Rect::from_center_size(pos2(rx - 12.0, top.center().y), vec2(30.0, 30.0));
        let g = ui.interact(gear, ui.scope_id().with("lib_settings"), Sense::click());
        icons::draw(p, Icon::Settings, gear.center(), 16.0, if g.hovered() { ACCENT } else { theme::text_2() });
        if g.clicked() {
            action = Some(Action::Settings);
        }
        let (open_left, open_clicked) = text_btn(ui, p, pos2(rx - 40.0, top.center().y), "lib_open", tr("Open file"), theme::text_2());
        if open_clicked {
            action = Some(Action::OpenFile);
        }
        let (add_left, add_clicked) = text_btn(ui, p, pos2(open_left - 24.0, top.center().y), "lib_add", tr("+ Add folder"), ACCENT);
        if add_clicked {
            action = Some(Action::AddFolder);
        }
        let (home_left, home_clicked) = text_btn(ui, p, pos2(add_left - 20.0, top.center().y), "lib_home", tr("Player"), theme::text_2());
        if home_clicked {
            action = Some(Action::Home);
        }
        if self.lockable {
            let (_, lock_clicked) = text_btn(ui, p, pos2(home_left - 20.0, top.center().y), "lib_lock", tr("Lock"), theme::text_2());
            if lock_clicked {
                action = Some(Action::Lock);
            }
        }
        action
    }

    // ---- Nav -----------------------------------------------------------

    fn nav(&mut self, ui: &mut egui::Ui, p: &egui::Painter, nav: Rect) {
        p.line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(0.5, theme::line()));
        let np = ui.painter_at(nav);
        let mut y = nav.top() + 12.0;
        let grid_mode = self.mode == Mode::Grid;
        self.refresh_nav_counts();
        let Some((_, [continue_n, recent_n, played_n], tag_counts)) = &self.cached.nav else { return };
        let ([continue_n, recent_n, played_n], tag_counts) = ([*continue_n, *recent_n, *played_n], tag_counts.clone());
        for (view, label, n) in [
            (View::All, tr("All"), self.videos.len()),
            (View::Continue, tr("Continue watching"), continue_n),
            (View::Recent, tr("Recently added"), recent_n),
            (View::MostPlayed, tr("Most played"), played_n),
        ] {
            if self.hide_history && matches!(view, View::Continue | View::MostPlayed) {
                continue;
            }
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 30.0));
            if nav_item(ui, &np, r, ("nav", label), label, Some(n), grid_mode && self.view == view) {
                self.view = view;
                self.mode = Mode::Grid;
            }
            y += 30.0;
        }

        // Collections
        y += 12.0;
        np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "COLLECTIONS", FontId::monospace(10.0), theme::text_3());
        let plus = Rect::from_center_size(pos2(nav.right() - 20.0, y + 8.0), vec2(22.0, 22.0));
        let pr = ui.interact(plus, ui.scope_id().with("new_collection"), Sense::click());
        np.text(plus.center(), Align2::CENTER_CENTER, "+", FontId::proportional(15.0), if pr.hovered() { ACCENT } else { theme::text_3() });
        if pr.clicked() {
            self.new_collection = Some(String::new());
        }
        y += 22.0;
        let fav_n = self.videos.iter().filter(|v| !v.collections.is_empty()).count();
        let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
        if nav_item(ui, &np, r, "fav_all", tr("All favorites"), Some(fav_n), grid_mode && self.view == View::Favorites(None)) {
            self.view = View::Favorites(None);
            self.mode = Mode::Grid;
        }
        y += 28.0;
        let mut delete = None;
        for c in &self.collections {
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
            let on = grid_mode && self.view == View::Favorites(Some(c.id));
            if nav_item_indented(ui, &np, r, ("coll", c.id), tr(&c.name), Some(c.count as usize), on, 20.0) {
                self.view = View::Favorites(Some(c.id));
                self.mode = Mode::Grid;
            }
            icons::draw(&np, Icon::Star, pos2(r.left() + 24.0, r.center().y), 11.0, if on { ACCENT } else { theme::text_3() });
            ui.interact(r, ui.scope_id().with(("coll", c.id)), Sense::click()).context_menu(|ui| {
                if ui.button(tr("Delete collection (keeps the videos)")).clicked() {
                    delete = Some(c.id);
                    ui.close();
                }
            });
            y += 28.0;
        }
        if let (Some(id), Some(lib)) = (delete, &self.lib) {
            lib.delete_collection(id);
            self.dirty = true;
        }
        // Inline "new collection" field.
        if let Some(name) = &mut self.new_collection {
            let r = Rect::from_min_size(pos2(nav.left() + 14.0, y + 2.0), vec2(NAV_W - 28.0, 24.0));
            np.rect_stroke(r, 4.0, Stroke::new(0.5, ACCENT), StrokeKind::Inside);
            let mut done = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink2(vec2(8.0, 3.0))), |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text(tr("Collection name"))
                        .frame(egui::Frame::NONE)
                        .text_color(theme::text())
                        .font(FontId::proportional(12.0)),
                );
                if !resp.has_focus() && !resp.lost_focus() {
                    resp.request_focus();
                }
                if ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                } else if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) || (resp.lost_focus() && !resp.has_focus()) {
                    done = Some(false);
                }
            });
            match done {
                Some(true) => {
                    if let Some(lib) = &self.lib {
                        lib.create_collection(name);
                    }
                    self.new_collection = None;
                    self.dirty = true;
                }
                Some(false) => self.new_collection = None,
                None => {}
            }
            y += 30.0;
        }

        // Tags
        if !tag_counts.is_empty() {
            y += 12.0;
            np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "TAGS", FontId::monospace(10.0), theme::text_3());
            y += 22.0;
            for (tag, n) in tag_counts.iter().take(10) {
                let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
                let on = self.tags_sel.iter().any(|t| t.eq_ignore_ascii_case(tag));
                if nav_item(ui, &np, r, ("tag", tag.as_str()), &format!("# {tag}"), Some(*n), on) {
                    toggle_tag(&mut self.tags_sel, tag);
                    self.mode = Mode::Grid;
                }
                y += 28.0;
            }
        }

        // Folders
        y += 12.0;
        np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "FOLDERS", FontId::monospace(10.0), theme::text_3());
        y += 22.0;
        let mut remove_folder = None;
        let mut check_folder = None;
        for f in &self.folders {
            if y > nav.bottom() - 28.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
            let name = base_name(&f.path);
            let on = match self.mode {
                Mode::Grid | Mode::Manage => self.folder_sel == Some(f.id),
                Mode::Folders => self.folder_loc.as_ref().is_some_and(|l| l.root == f.id),
            };
            let hovered = ui.rect_contains_pointer(r);
            if nav_item(ui, &np, r, ("folder", f.id), &name, if hovered { None } else { Some(f.count as usize) }, on) {
                match self.mode {
                    Mode::Grid | Mode::Manage => self.folder_sel = if on { None } else { Some(f.id) },
                    Mode::Folders => self.folder_loc = Some(FolderLoc { root: f.id, parts: Vec::new() }),
                }
            }
            ui.interact(r, ui.scope_id().with(("folder", f.id)), Sense::click()).context_menu(|ui| {
                if ui.button(tr("Check this folder for duplicates")).clicked() {
                    check_folder = Some(PathBuf::from(&f.path));
                    ui.close();
                }
            });
            if hovered {
                let x = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(22.0, 22.0));
                let xr = ui.interact(x, ui.scope_id().with(("folder_rm", f.id)), Sense::click());
                icons::draw(&np, Icon::Close, x.center(), 11.0, if xr.hovered() { ACCENT } else { theme::text_3() });
                let clicked = xr.clicked();
                xr.on_hover_text(format!("{}\n{}", tr("Remove from library (keeps the file)"), f.path));
                if clicked {
                    remove_folder = Some(f.id);
                }
            }
            y += 28.0;
        }
        if let (Some(id), Some(lib)) = (remove_folder, &self.lib) {
            lib.remove_folder(id);
            self.dirty = true;
        }
        if let Some(path) = check_folder {
            self.check_duplicates(path);
        }
    }

    /// Opens the manage page's duplicate check for one folder.
    fn check_duplicates(&mut self, path: PathBuf) {
        self.mode = Mode::Manage;
        self.manager.check_library_folder(path);
    }

    // ---- Grid view -----------------------------------------------------

    fn grid_view(&mut self, ui: &mut egui::Ui, p: &egui::Painter, content: Rect) -> Action {
        let (cells, total) = self.grid_cells();
        let header = Rect::from_min_size(content.min + vec2(24.0, 12.0), vec2(content.width() - 48.0, 30.0));
        let mut hx = header.left();

        // Title for collection/most-played views.
        let title = match self.view {
            View::Favorites(Some(c)) => self.collections.iter().find(|x| x.id == c).map(|x| tr(&x.name).to_string()),
            View::Favorites(None) => Some(tr("All favorites").to_string()),
            View::MostPlayed => Some(tr("Most played").to_string()),
            _ => None,
        };
        if let Some(t) = title {
            let g = p.layout_no_wrap(t, FontId::proportional(15.0), theme::text());
            let w = g.size().x;
            p.galley(pos2(hx, header.center().y - g.size().y / 2.0), g, theme::text());
            hx += w + 12.0;
        }
        let mut remove_tag_filter = None;
        for t in &self.tags_sel {
            let g = p.layout_no_wrap(format!("# {t}  ×"), FontId::proportional(12.0), Color32::WHITE);
            let r = Rect::from_min_size(pos2(hx, header.center().y - 11.0), g.size() + vec2(16.0, 6.0));
            let resp = ui.interact(r, ui.scope_id().with(("tagf", t.as_str())), Sense::click());
            p.rect_filled(r, 11.0, if resp.hovered() { ACCENT.gamma_multiply(0.8) } else { ACCENT });
            p.galley(r.min + vec2(8.0, 3.0), g, Color32::WHITE);
            if resp.clicked() {
                remove_tag_filter = Some(t.clone());
            }
            hx = r.right() + 8.0;
        }
        p.text(
            pos2(hx, header.center().y),
            Align2::LEFT_CENTER,
            tf("{} videos · {}", &[&cells.len(), &fmt_time(total)]),
            FontId::monospace(11.0),
            theme::text_3(),
        );
        if !matches!(self.view, View::Continue | View::MostPlayed) {
            let mut sx = header.right();
            for (sort, label) in [
                (Sort::Duration, tr("Duration")),
                (Sort::Name, tr("Name")),
                (Sort::Plays, tr("Plays")),
                (Sort::Played, tr("Recently played")),
                (Sort::Added, tr("Recently added")),
            ] {
                if self.hide_history && matches!(sort, Sort::Plays | Sort::Played) {
                    continue;
                }
                let g = p.layout_no_wrap(label.into(), FontId::proportional(12.0), theme::text());
                let r = Rect::from_min_size(pos2(sx - g.size().x - 12.0, header.center().y - 12.0), vec2(g.size().x + 12.0, 24.0));
                let resp = ui.interact(r, ui.scope_id().with(("sort", label)), Sense::click());
                let on = self.sort == sort;
                let c = if on { ACCENT } else if resp.hovered() { theme::text() } else { theme::text_2() };
                p.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), c);
                if resp.clicked() {
                    self.sort = sort;
                }
                sx = r.left() - 4.0;
            }
        }
        if let Some(t) = remove_tag_filter {
            self.tags_sel.retain(|x| x != &t);
        }

        let grid_rect = Rect::from_min_max(pos2(content.left() + 24.0, header.bottom() + 8.0), pos2(content.right() - 12.0, content.bottom()));
        if cells.is_empty() {
            let msg = match self.view {
                View::Favorites(_) => tr("No favorites yet. Hover a video and click its star"),
                View::MostPlayed => tr("Nothing played yet"),
                View::Continue => tr("No videos in progress"),
                _ => tr("No matching videos"),
            };
            p.text(grid_rect.center() - vec2(0.0, 40.0), Align2::CENTER_CENTER, msg, FontId::proportional(14.0), theme::text_2());
            return Action::None;
        }
        let ranked = self.view == View::MostPlayed;
        self.grid(ui, grid_rect, &cells, ranked, "library_grid")
    }

    // ---- Folder browser ------------------------------------------------

    fn folder_browser(&mut self, ui: &mut egui::Ui, p: &egui::Painter, content: Rect) -> Action {
        let header = Rect::from_min_size(content.min + vec2(24.0, 12.0), vec2(content.width() - 48.0, 30.0));
        let cy = header.center().y;

        // Breadcrumb: All folders / root / a / b
        let mut crumbs: Vec<(String, Option<FolderLoc>)> = vec![(tr("All folders").into(), None)];
        if let Some(loc) = &self.folder_loc {
            if let Some(root) = self.folders.iter().find(|f| f.id == loc.root) {
                crumbs.push((base_name(&root.path), Some(FolderLoc { root: loc.root, parts: Vec::new() })));
                for i in 0..loc.parts.len() {
                    crumbs.push((loc.parts[i].clone(), Some(FolderLoc { root: loc.root, parts: loc.parts[..=i].to_vec() })));
                }
            }
        }
        let mut x = header.left();
        // "Up" arrow.
        if self.folder_loc.is_some() {
            let up = Rect::from_center_size(pos2(x + 12.0, cy), vec2(24.0, 24.0));
            let ur = ui.interact(up, ui.scope_id().with("folder_up"), Sense::click());
            let c = if ur.hovered() { ACCENT } else { theme::text_2() };
            let st = Stroke::new(1.6, c);
            p.line_segment([pos2(up.center().x, cy + 6.0), pos2(up.center().x, cy - 6.0)], st);
            p.line_segment([pos2(up.center().x - 5.0, cy - 1.0), pos2(up.center().x, cy - 6.0)], st);
            p.line_segment([pos2(up.center().x + 5.0, cy - 1.0), pos2(up.center().x, cy - 6.0)], st);
            if ur.clicked() {
                self.folder_loc = match self.folder_loc.take() {
                    Some(mut l) if !l.parts.is_empty() => {
                        l.parts.pop();
                        Some(l)
                    }
                    _ => None,
                };
            }
            x += 32.0;
        }
        let n = crumbs.len();
        let mut goto = None;
        for (i, (name, loc)) in crumbs.into_iter().enumerate() {
            let last = i + 1 == n;
            let g = p.layout_no_wrap(name, FontId::proportional(13.0), theme::text());
            let r = Rect::from_min_size(pos2(x, cy - g.size().y / 2.0), g.size());
            let resp = ui.interact(r.expand(3.0), ui.scope_id().with(("crumb", i)), Sense::click());
            let c = if last { theme::text() } else if resp.hovered() { ACCENT } else { theme::text_2() };
            p.galley(r.min, g, c);
            if resp.clicked() && !last {
                goto = Some(loc);
            }
            x = r.right() + 8.0;
            if !last {
                p.text(pos2(x, cy), Align2::LEFT_CENTER, "/", FontId::proportional(13.0), theme::text_3());
                x += 14.0;
            }
        }
        if let Some(loc) = goto {
            self.folder_loc = loc;
        }

        // Cells: sub-folders first, then videos directly in this folder.
        let (cells, play_all) = self.folder_list();
        let video_n = cells.iter().filter(|c| matches!(c, Cell::Video(_))).count();
        let folder_n = cells.len() - video_n;
        let stat = tf("{} folders · {} videos", &[&folder_n, &video_n]);
        let (pa_left, pa_clicked) = text_btn(ui, p, pos2(header.right(), cy), "play_all", tr("Play all"), ACCENT);
        p.text(pos2(pa_left - 20.0, cy), Align2::RIGHT_CENTER, stat, FontId::monospace(11.0), theme::text_3());

        let grid_rect = Rect::from_min_max(pos2(content.left() + 24.0, header.bottom() + 8.0), pos2(content.right() - 12.0, content.bottom()));
        if cells.is_empty() {
            p.text(grid_rect.center() - vec2(0.0, 40.0), Align2::CENTER_CENTER, tr("No videos in this folder"), FontId::proportional(14.0), theme::text_2());
            return Action::None;
        }
        let action = self.grid(ui, grid_rect, &cells, false, "folder_grid");
        if pa_clicked && !play_all.is_empty() {
            return Action::PlayAll((*play_all).clone());
        }
        action
    }

    /// Cells for the current folder location, plus every video below it
    /// (recursive, natural order) for "Play all".
    fn folder_list(&mut self) -> (Rc<Vec<Cell>>, Rc<Vec<String>>) {
        if let Some((revision, loc, cells, play_all)) = &self.cached.folder {
            if *revision == self.revision && *loc == self.folder_loc {
                return (cells.clone(), play_all.clone());
            }
        }
        let (cells, play_all) = self.folder_cells();
        let (cells, play_all) = (Rc::new(cells), Rc::new(play_all));
        self.cached.folder = Some((self.revision, self.folder_loc.clone(), cells.clone(), play_all.clone()));
        self.cached.folder_stats.clear();
        (cells, play_all)
    }

    fn folder_cells(&self) -> (Vec<Cell>, Vec<String>) {
        let Some(loc) = &self.folder_loc else {
            // Top level: one card per watched root.
            let mut all = Vec::new();
            let cells = self
                .folders
                .iter()
                .map(|f| {
                    all.extend(self.videos_under(&FolderLoc { root: f.id, parts: Vec::new() }));
                    Cell::Folder { name: base_name(&f.path), loc: FolderLoc { root: f.id, parts: Vec::new() } }
                })
                .collect();
            return (cells, all);
        };
        let Some(dir) = self.loc_path(loc) else { return (Vec::new(), Vec::new()) };
        let mut subs: BTreeMap<String, ()> = BTreeMap::new();
        let mut here: Vec<&Video> = Vec::new();
        for v in self.videos.iter().filter(|v| v.folder_id == loc.root) {
            let Ok(rel) = Path::new(&v.path).strip_prefix(&dir) else { continue };
            let comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            match comps.len() {
                0 => {}
                1 => here.push(v),
                _ => {
                    subs.insert(comps[0].clone(), ());
                }
            }
        }
        let mut sub_names: Vec<String> = subs.into_keys().collect();
        sub_names.sort_by(|a, b| natural_cmp(a, b));
        here.sort_by(|a, b| natural_cmp(&a.title, &b.title));
        let mut cells: Vec<Cell> = sub_names
            .into_iter()
            .map(|name| {
                let mut parts = loc.parts.clone();
                parts.push(name.clone());
                Cell::Folder { name, loc: FolderLoc { root: loc.root, parts } }
            })
            .collect();
        cells.extend(here.iter().map(|v| Cell::Video(v.id)));
        (cells, self.videos_under(loc))
    }

    fn loc_path(&self, loc: &FolderLoc) -> Option<PathBuf> {
        let root = self.folders.iter().find(|f| f.id == loc.root)?;
        let mut p = PathBuf::from(&root.path);
        for part in &loc.parts {
            p.push(part);
        }
        Some(p)
    }

    /// Paths of all videos under `loc`, recursively, in natural path order.
    fn videos_under(&self, loc: &FolderLoc) -> Vec<String> {
        let Some(dir) = self.loc_path(loc) else { return Vec::new() };
        let mut v: Vec<(String, String)> = self
            .videos
            .iter()
            .filter(|v| v.folder_id == loc.root)
            .filter_map(|v| {
                let rel = Path::new(&v.path).strip_prefix(&dir).ok()?;
                Some((rel.to_string_lossy().into_owned(), v.path.clone()))
            })
            .collect();
        v.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        v.into_iter().map(|(_, p)| p).collect()
    }

    fn folder_stats(&self, loc: &FolderLoc) -> (Vec<i64>, usize, f64, usize) {
        let Some(dir) = self.loc_path(loc) else { return (Vec::new(), 0, 0.0, 0) };
        let mut inside: Vec<&Video> = self
            .videos
            .iter()
            .filter(|v| v.folder_id == loc.root && Path::new(&v.path).starts_with(&dir))
            .collect();
        inside.sort_by(|a, b| natural_cmp(&a.path, &b.path));
        let total: f64 = inside.iter().map(|v| v.duration).sum();
        let finished = |v: &&&Video| !self.hide_history && v.progress().is_some_and(|p| p >= FINISHED);
        let watched = inside.iter().filter(finished).count();
        let covers = inside.iter().take(4).map(|v| v.id).collect();
        (covers, inside.len(), total, watched)
    }

    // ---- Shared grid ---------------------------------------------------

    fn grid(&mut self, ui: &mut egui::Ui, rect: Rect, cells: &[Cell], ranked: bool, salt: &str) -> Action {
        let mut action = Action::None;
        let width = rect.width() - 12.0;
        let cols = (((width + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
        let card_w = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
        let thumb_h = card_w * 9.0 / 16.0;
        let card_h = thumb_h + META_H;
        let rows = cells.len().div_ceil(cols);
        let total_h = rows as f32 * (card_h + GAP);
        let mut loads = 0;
        let mut pending_thumbs = false;
        let mut events: Vec<CardEvent> = Vec::new();

        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show_viewport(ui, |ui, vp| {
                ui.set_min_size(vec2(width, total_h));
                let origin = ui.max_rect().min;
                let first = ((vp.top() / (card_h + GAP)).floor().max(0.0)) as usize;
                let last = (((vp.bottom() / (card_h + GAP)).ceil()) as usize).min(rows);
                let p = ui.painter().clone();
                for row in first..last {
                    for col in 0..cols {
                        let i = row * cols + col;
                        let Some(cell) = cells.get(i) else { break };
                        let pos = origin + vec2(col as f32 * (card_w + GAP), row as f32 * (card_h + GAP));
                        let card = Rect::from_min_size(pos, vec2(card_w, card_h));
                        let thumb = Rect::from_min_size(pos, vec2(card_w, thumb_h));
                        match cell {
                            Cell::Video(id) => {
                                let id = *id;
                                if self.ensure_thumb(ui.ctx(), id, &mut loads) {
                                    pending_thumbs = true;
                                }
                                let tex = self.use_thumb(id);
                                let Some(v) = self.video(id) else { continue };
                                let rank = ranked.then_some(i + 1);
                                if let Some(ev) = draw_card(ui, &p, card, thumb, v, tex, rank, self.hide_history) {
                                    events.push(ev);
                                }
                            }
                            Cell::Folder { name, loc } => {
                                let stats = match self.cached.folder_stats.get(loc) {
                                    Some(stats) => stats.clone(),
                                    None => {
                                        let stats = self.folder_stats(loc);
                                        self.cached.folder_stats.insert(loc.clone(), stats.clone());
                                        stats
                                    }
                                };
                                let (covers, count, total, watched) = stats;
                                for &id in &covers {
                                    if self.ensure_thumb(ui.ctx(), id, &mut loads) {
                                        pending_thumbs = true;
                                    }
                                }
                                let texs: Vec<Option<egui::TextureId>> =
                                    covers.iter().map(|id| self.use_thumb(*id)).collect();
                                if let Some(ev) = draw_folder_card(ui, &p, card, thumb, name, loc, &texs, count, total, watched) {
                                    events.push(ev);
                                }
                            }
                        }
                    }
                }
            });
        });
        if pending_thumbs {
            ui.ctx().request_repaint();
        }
        self.evict_thumbs();

        for ev in events {
            match ev {
                CardEvent::Play(path) => action = Action::Play(path),
                CardEvent::OpenFolder(loc) => {
                    self.folder_loc = Some(loc);
                }
                CardEvent::PlayFolder(loc) => {
                    let paths = self.videos_under(&loc);
                    if !paths.is_empty() {
                        action = Action::PlayAll(paths);
                    }
                }
                CardEvent::CheckFolder(loc) => {
                    if let Some(path) = self.loc_path(&loc) {
                        self.check_duplicates(path);
                    }
                }
                CardEvent::TagFilter(t) => {
                    toggle_tag(&mut self.tags_sel, &t);
                    self.mode = Mode::Grid;
                }
                CardEvent::EditTags(id, at) => {
                    self.tag_edit = Some((id, String::new(), false));
                    self.tag_edit_pos = at;
                }
                CardEvent::RemoveTag(id, t) => {
                    if let Some(lib) = &self.lib {
                        lib.remove_tag(id, &t);
                    }
                    self.dirty = true;
                }
                CardEvent::Favorite(id, at) => {
                    self.fav_picker = Some((id, at));
                    self.fav_picker_fresh = true;
                }
                CardEvent::Reveal(path) => reveal_in_file_manager(&path),
                CardEvent::Remove(id) => {
                    if let Some(lib) = &self.lib {
                        lib.remove_video(id);
                    }
                    self.dirty = true;
                }
            }
        }
        action
    }

    /// Texture for `id` if loaded, marking it as used this frame.
    fn use_thumb(&mut self, id: i64) -> Option<egui::TextureId> {
        let frame = self.frame;
        self.thumbs.get_mut(&id).map(|(_, t, used)| {
            *used = frame;
            t.id()
        })
    }

    /// Drops the least recently drawn textures beyond [`MAX_THUMBS`].
    /// Never drops one drawn this frame.
    fn evict_thumbs(&mut self) {
        self.frame += 1;
        if self.thumbs.len() <= MAX_THUMBS {
            return;
        }
        let mut by_age: Vec<(u64, i64)> = self.thumbs.iter().map(|(id, (_, _, used))| (*used, *id)).collect();
        by_age.sort_unstable();
        let excess = self.thumbs.len() - MAX_THUMBS;
        let current = self.frame - 1;
        for (used, id) in by_age.into_iter().take(excess) {
            if used >= current {
                break;
            }
            self.thumbs.remove(&id);
        }
    }

    /// Loads the thumbnail texture for `id` if allowed this frame.
    /// Returns true if a load had to be deferred to a later frame.
    fn ensure_thumb(&mut self, ctx: &egui::Context, id: i64, loads: &mut usize) -> bool {
        if self.thumbs.contains_key(&id) || self.thumb_failed.contains(&id) {
            return false;
        }
        let Some(path) = self.video(id).and_then(|v| v.thumb.clone()) else { return false };
        if *loads >= THUMB_LOADS_PER_FRAME {
            return true;
        }
        *loads += 1;
        match load_thumb(ctx, id, &path) {
            Some(t) => {
                self.thumbs.insert(id, (path, t, self.frame));
            }
            None => {
                self.thumb_failed.insert(id);
            }
        }
        false
    }

    // ---- Popups --------------------------------------------------------

    /// Small popup for typing a new tag.
    fn tag_editor(&mut self, ui: &mut egui::Ui) {
        let Some((id, mut text, focused)) = self.tag_edit.take() else { return };
        let mut keep = true;
        let mut commit = false;
        egui::Area::new(ui.scope_id().with("tag_editor"))
            .order(egui::Order::Foreground)
            .fixed_pos(self.tag_edit_pos)
            .show(ui.ctx(), |ui| {
                popup_frame().show(ui, |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text(tr("Type a tag, press Enter"))
                            .frame(egui::Frame::NONE)
                            .text_color(theme::text())
                            .font(FontId::proportional(13.0))
                            .desired_width(180.0),
                    );
                    if !focused {
                        r.request_focus();
                    }
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                        keep = false;
                    } else if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) || (focused && r.lost_focus()) {
                        keep = false;
                    }
                });
            });
        if commit && !text.trim().is_empty() {
            if let Some(lib) = &self.lib {
                lib.add_tag(id, &text);
            }
            self.dirty = true;
        }
        if keep {
            self.tag_edit = Some((id, text, true));
        }
    }

    /// Checklist of collections for one video, opened from its star.
    fn favorite_picker(&mut self, ui: &mut egui::Ui) {
        let Some((vid, at)) = self.fav_picker else { return };
        let Some(v) = self.video(vid) else {
            self.fav_picker = None;
            return;
        };
        let in_cols = v.collections.clone();
        let mut toggles: Vec<(i64, bool)> = Vec::new();
        let mut create: Option<String> = None;
        let mut close = false;
        let area = egui::Area::new(ui.scope_id().with("fav_picker"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                popup_frame().show(ui, |ui| {
                    ui.set_width(190.0);
                    ui.label(egui::RichText::new(tr("Add to")).size(11.0).color(theme::text_3()));
                    ui.add_space(4.0);
                    for c in &self.collections {
                        let on = in_cols.contains(&c.id);
                        let (r, resp) = ui.allocate_exact_size(vec2(190.0, 26.0), Sense::click());
                        let p = ui.painter();
                        if resp.hovered() {
                            p.rect_filled(r, 4.0, theme::tint(10));
                        }
                        let bx = Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(13.0, 13.0));
                        if on {
                            p.rect_filled(bx, 3.0, ACCENT);
                            p.line_segment([bx.center() + vec2(-3.5, 0.0), bx.center() + vec2(-1.0, 2.5)], Stroke::new(1.6, Color32::WHITE));
                            p.line_segment([bx.center() + vec2(-1.0, 2.5), bx.center() + vec2(3.5, -2.5)], Stroke::new(1.6, Color32::WHITE));
                        } else {
                            p.rect_stroke(bx, 3.0, Stroke::new(1.0, theme::text_3()), StrokeKind::Inside);
                        }
                        p.text(pos2(r.left() + 28.0, r.center().y), Align2::LEFT_CENTER, tr(&c.name), FontId::proportional(13.0), theme::text());
                        if resp.clicked() {
                            toggles.push((c.id, !on));
                        }
                    }
                    ui.add_space(4.0);
                    ui.separator();
                    let name = &mut self.fav_new_name;
                    let r = ui.add(
                        egui::TextEdit::singleline(name)
                            .hint_text(tr("+ New collection, press Enter"))
                            .frame(egui::Frame::NONE)
                            .text_color(theme::text())
                            .font(FontId::proportional(12.0))
                            .desired_width(190.0),
                    );
                    if r.has_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) && !name.trim().is_empty() {
                        create = Some(name.clone());
                    }
                });
            });
        // Close on Esc or a click outside the popup.
        let fresh = std::mem::take(&mut self.fav_picker_fresh);
        let outside = !fresh
            && ui.ctx().input(|i| i.pointer.any_click())
            && ui.ctx().input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p));
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) || (outside && toggles.is_empty()) {
            close = true;
        }
        if let Some(lib) = &self.lib {
            for (cid, on) in toggles {
                lib.set_in_collection(cid, vid, on);
                self.dirty = true;
            }
            if let Some(name) = create {
                if let Some(cid) = lib.create_collection(&name) {
                    lib.set_in_collection(cid, vid, true);
                }
                self.fav_new_name.clear();
                self.dirty = true;
            }
        }
        if close {
            self.fav_picker = None;
            self.fav_new_name.clear();
        }
    }
}

enum CardEvent {
    Play(String),
    OpenFolder(FolderLoc),
    PlayFolder(FolderLoc),
    CheckFolder(FolderLoc),
    TagFilter(String),
    EditTags(i64, egui::Pos2),
    RemoveTag(i64, String),
    Favorite(i64, egui::Pos2),
    Reveal(String),
    Remove(i64),
}

fn matches_search(v: &Video, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let hay = format!("{} {} {}", v.title, v.tags.join(" "), v.path).to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

fn draw_card(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    card: Rect,
    thumb: Rect,
    v: &Video,
    tex: Option<egui::TextureId>,
    rank: Option<usize>,
    hide_history: bool,
) -> Option<CardEvent> {
    let mut ev = None;
    let resp = ui.interact(thumb, ui.scope_id().with(("card", v.id)), Sense::click());
    let hovered = resp.hovered() || ui.rect_contains_pointer(thumb);

    match tex {
        Some(t) => {
            p.image(t, thumb, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.rect_filled(thumb, 6.0, placeholder_color(v.id));
            let label = if v.duration > 0.0 { "" } else { tr("Generating…") };
            p.text(thumb.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.0), theme::text_3());
        }
    }
    if hovered {
        p.rect_stroke(thumb, 6.0, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
        icons::draw(p, Icon::Play, thumb.center(), 30.0, Color32::from_white_alpha(230));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    // Top-left: rank (most played) or resolution.
    let (badge, badge_bg) = match rank {
        Some(r) => (format!("#{r}"), if r <= 3 { ACCENT } else { theme::scrim(190) }),
        None => (resolution_label(v.width, v.height), theme::scrim(190)),
    };
    if !badge.is_empty() {
        let font = FontId::monospace(if rank.is_some() { 11.0 } else { 9.0 });
        let g = p.layout_no_wrap(badge, font, TEXT);
        let r = Rect::from_min_size(thumb.min + vec2(6.0, 6.0), g.size() + vec2(8.0, 4.0));
        p.rect_filled(r, 3.0, badge_bg);
        p.galley(r.min + vec2(4.0, 2.0), g, Color32::WHITE);
    }

    // Top-right: favorite star (always shown once favorited).
    let fav = !v.collections.is_empty();
    let star = Rect::from_center_size(pos2(thumb.right() - 18.0, thumb.top() + 18.0), vec2(26.0, 26.0));
    if fav || hovered {
        let sr = ui.interact(star, ui.scope_id().with(("star", v.id)), Sense::click());
        p.circle_filled(star.center(), 13.0, theme::scrim(if sr.hovered() { 230 } else { 170 }));
        let icon = if fav { Icon::StarFilled } else { Icon::Star };
        icons::draw(p, icon, star.center(), 14.0, if fav || sr.hovered() { ACCENT } else { TEXT });
        sr.clone().on_hover_text(if fav { tr("Favorited · click to manage") } else { tr("Favorite") });
        if sr.clicked() {
            ev = Some(CardEvent::Favorite(v.id, pos2(star.right() - 190.0, star.bottom() + 6.0)));
        }
    }

    let progress = v.progress().filter(|_| !hide_history);
    let finished = progress.is_some_and(|x| x >= FINISHED);
    let badge = if finished { tr("Watched").to_string() } else if v.duration > 0.0 { fmt_time(v.duration) } else { String::new() };
    if !badge.is_empty() {
        let g = p.layout_no_wrap(badge, FontId::monospace(10.0), TEXT);
        let r = Rect::from_min_size(thumb.right_bottom() - g.size() - vec2(12.0, 10.0), g.size() + vec2(8.0, 4.0));
        p.rect_filled(r, 3.0, theme::scrim(200));
        p.galley(r.min + vec2(4.0, 2.0), g, TEXT);
    }
    if let Some(f) = progress.filter(|_| !finished) {
        let y = thumb.bottom() - 1.5;
        p.line_segment([pos2(thumb.left(), y), pos2(thumb.right(), y)], Stroke::new(3.0, Color32::from_white_alpha(50)));
        p.line_segment([pos2(thumb.left(), y), pos2(thumb.left() + f * thumb.width(), y)], Stroke::new(3.0, ACCENT));
    }

    // Title + plays.
    let ty = thumb.bottom() + 15.0;
    let plays = if v.plays > 0 && !hide_history { format!("▶ {}", v.plays) } else { String::new() };
    let plays_font = FontId::monospace(if rank.is_some() { 11.0 } else { 10.0 });
    let plays_w = if plays.is_empty() { 0.0 } else { p.layout_no_wrap(plays.clone(), plays_font.clone(), theme::text()).size().x + 8.0 };
    let title = elide(p, &v.title, FontId::proportional(13.0), card.width() - plays_w);
    p.text(pos2(card.left(), ty), Align2::LEFT_CENTER, title, FontId::proportional(13.0), if finished { theme::text_2() } else { theme::text() });
    if !plays.is_empty() {
        p.text(pos2(card.right(), ty), Align2::RIGHT_CENTER, plays, plays_font, if rank.is_some() { ACCENT } else { theme::text_3() });
    }

    // Tags + "+ Tag".
    let mut x = card.left();
    let cy = thumb.bottom() + 38.0;
    for t in &v.tags {
        let g = p.layout_no_wrap(format!("#{t}"), FontId::proportional(11.0), theme::text_2());
        let r = Rect::from_min_size(pos2(x, cy - 9.0), g.size() + vec2(10.0, 4.0));
        if r.right() > card.right() - 44.0 {
            break;
        }
        let tr = ui.interact(r, ui.scope_id().with(("card_tag", v.id, t.as_str())), Sense::click());
        p.rect_filled(r, 3.0, if tr.hovered() { theme::accent_alpha(40) } else { theme::tint(14) });
        p.galley(r.min + vec2(5.0, 2.0), g, if tr.hovered() { ACCENT } else { theme::text_2() });
        if tr.clicked() {
            ev = Some(CardEvent::TagFilter(t.clone()));
        }
        x = r.right() + 5.0;
    }
    let add = Rect::from_min_size(pos2(x, cy - 9.0), vec2(42.0, 18.0));
    let ar = ui.interact(add, ui.scope_id().with(("card_add_tag", v.id)), Sense::click());
    p.text(add.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, tr("+ Tag"), FontId::proportional(11.0), if ar.hovered() { ACCENT } else { theme::text_3() });
    if ar.clicked() {
        ev = Some(CardEvent::EditTags(v.id, pos2(card.left(), cy + 12.0)));
    }

    if resp.clicked() && ev.is_none() {
        ev = Some(CardEvent::Play(v.path.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button(tr("Play")).clicked() {
            ev = Some(CardEvent::Play(v.path.clone()));
            ui.close();
        }
        if ui.button(if fav { tr("Manage collections…") } else { tr("Add to collection…") }).clicked() {
            ev = Some(CardEvent::Favorite(v.id, pos2(thumb.left() + 8.0, thumb.top() + 8.0)));
            ui.close();
        }
        if ui.button(tr("Add tag…")).clicked() {
            ev = Some(CardEvent::EditTags(v.id, pos2(card.left(), cy + 12.0)));
            ui.close();
        }
        if !v.tags.is_empty() {
            ui.menu_button(tr("Remove tag"), |ui| {
                for t in &v.tags {
                    if ui.button(format!("#{t}")).clicked() {
                        ev = Some(CardEvent::RemoveTag(v.id, t.clone()));
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        let reveal = if cfg!(target_os = "macos") { tr("Show in Finder") } else { tr("Show in Explorer") };
        if ui.button(reveal).clicked() {
            ev = Some(CardEvent::Reveal(v.path.clone()));
            ui.close();
        }
        if ui.button(tr("Remove from library (keeps the file)")).clicked() {
            ev = Some(CardEvent::Remove(v.id));
            ui.close();
        }
    });
    ev
}

#[allow(clippy::too_many_arguments)]
fn draw_folder_card(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    card: Rect,
    thumb: Rect,
    name: &str,
    loc: &FolderLoc,
    covers: &[Option<egui::TextureId>],
    count: usize,
    total: f64,
    watched: usize,
) -> Option<CardEvent> {
    let mut ev = None;
    let id = (loc.root, loc.parts.join("/"));
    let resp = ui.interact(thumb, ui.scope_id().with(("folder_card", &id)), Sense::click());
    let hovered = resp.hovered();

    // 2×2 mosaic of the first videos inside.
    p.rect_filled(thumb, 6.0, theme::bg());
    let (w, h) = ((thumb.width() - 1.0) / 2.0, (thumb.height() - 1.0) / 2.0);
    for i in 0..4 {
        let cell = Rect::from_min_size(thumb.min + vec2((i % 2) as f32 * (w + 1.0), (i / 2) as f32 * (h + 1.0)), vec2(w, h));
        match covers.get(i).copied().flatten() {
            Some(t) => {
                p.image(t, cell, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(cell, 0.0, theme::panel());
            }
        }
    }
    let bg = Rect::from_min_size(thumb.right_bottom() - vec2(64.0, 26.0), vec2(56.0, 18.0));
    p.rect_filled(bg, 3.0, theme::scrim(210));
    icons::draw(p, Icon::Folder, pos2(bg.left() + 13.0, bg.center().y), 11.0, TEXT);
    p.text(pos2(bg.right() - 6.0, bg.center().y), Align2::RIGHT_CENTER, count.to_string(), FontId::monospace(10.0), TEXT);
    if count > 0 && watched > 0 {
        let f = watched as f32 / count as f32;
        let y = thumb.bottom() - 1.5;
        p.line_segment([pos2(thumb.left(), y), pos2(thumb.right(), y)], Stroke::new(3.0, Color32::from_white_alpha(50)));
        p.line_segment([pos2(thumb.left(), y), pos2(thumb.left() + f * thumb.width(), y)], Stroke::new(3.0, ACCENT));
    }
    if hovered {
        p.rect_stroke(thumb, 6.0, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    let ty = thumb.bottom() + 15.0;
    p.text(pos2(card.left(), ty), Align2::LEFT_CENTER, elide(p, name, FontId::proportional(13.0), card.width()), FontId::proportional(13.0), theme::text());
    let mut meta = fmt_time(total);
    if count > 0 && watched > 0 {
        meta.push_str(&tf(" · watched {}/{}", &[&watched, &count]));
    }
    p.text(pos2(card.left(), thumb.bottom() + 36.0), Align2::LEFT_CENTER, meta, FontId::monospace(10.0), theme::text_3());

    if resp.clicked() {
        ev = Some(CardEvent::OpenFolder(loc.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button(tr("Open")).clicked() {
            ev = Some(CardEvent::OpenFolder(loc.clone()));
            ui.close();
        }
        if ui.button(tr("Play all")).clicked() {
            ev = Some(CardEvent::PlayFolder(loc.clone()));
            ui.close();
        }
        if ui.button(tr("Check this folder for duplicates")).clicked() {
            ev = Some(CardEvent::CheckFolder(loc.clone()));
            ui.close();
        }
    });
    ev
}

fn resolution_label(w: i64, h: i64) -> String {
    if w <= 0 || h <= 0 {
        return String::new();
    }
    match h.min(w) {
        x if x >= 2160 => "4K".into(),
        x if x >= 1440 => "2K".into(),
        x if x >= 1080 => "1080P".into(),
        x if x >= 720 => "720P".into(),
        x => format!("{x}P"),
    }
}

fn empty_library(ui: &mut egui::Ui, p: &egui::Painter, content: Rect) -> Action {
    let c = content.center() - vec2(0.0, 30.0);
    let frame = Rect::from_center_size(c - vec2(0.0, 30.0), vec2(240.0, 120.0));
    icons::corner_brackets(p, frame, 12.0, Stroke::new(1.0, theme::accent_alpha(200)));
    icons::draw(p, Icon::Folder, frame.center() - vec2(0.0, 14.0), 26.0, ACCENT);
    p.text(frame.center() + vec2(0.0, 24.0), Align2::CENTER_CENTER, tr("Build your library"), FontId::proportional(14.0), theme::text());
    p.text(pos2(c.x, frame.bottom() + 22.0), Align2::CENTER_CENTER, tr("Add folders of videos for automatic thumbnails, tags and search"), FontId::proportional(12.0), theme::text_2());
    let mut action = Action::None;
    let y = frame.bottom() + 60.0;
    if text_btn(ui, p, pos2(c.x - 14.0, y), "empty_add", tr("+ Add folder"), ACCENT).1 {
        action = Action::AddFolder;
    }
    let open_w = p.layout_no_wrap(tr("Open file").into(), FontId::proportional(13.0), theme::text()).size().x;
    if text_btn(ui, p, pos2(c.x + 14.0 + open_w, y), "empty_open", tr("Open file"), theme::text_2()).1 {
        action = Action::OpenFile;
    }
    p.text(pos2(c.x, y + 40.0), Align2::CENTER_CENTER, tr("Or just drop a video into this window"), FontId::proportional(11.0), theme::text_3());
    action
}

// ---- Small widgets ---------------------------------------------------------

fn popup_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(theme::panel())
        .stroke(Stroke::new(0.5, ACCENT))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// Right-aligned text button ending at `right_center`. Returns (left edge, clicked).
fn text_btn(ui: &mut egui::Ui, p: &egui::Painter, right_center: egui::Pos2, id: &str, text: &str, color: Color32) -> (f32, bool) {
    let g = p.layout_no_wrap(text.to_string(), FontId::proportional(13.0), color);
    let r = Rect::from_min_size(pos2(right_center.x - g.size().x, right_center.y - g.size().y / 2.0), g.size());
    let resp = ui.interact(r.expand2(vec2(8.0, 6.0)), ui.scope_id().with(id), Sense::click());
    let c = if resp.hovered() { theme::text() } else { color };
    p.text(r.min, Align2::LEFT_TOP, text, FontId::proportional(13.0), c);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    (r.left(), resp.clicked())
}

fn time_left(secs: f64) -> String {
    let minutes = (secs / 60.0).ceil() as u64;
    if minutes < 1 {
        tr("Less than a minute left").to_string()
    } else if minutes < 60 {
        tf("About {} min left", &[&minutes])
    } else {
        tf("About {} h {} min left", &[&(minutes / 60), &(minutes % 60)])
    }
}

pub(crate) fn nav_item(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    r: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    count: Option<usize>,
    on: bool,
) -> bool {
    nav_item_indented(ui, p, r, id, label, count, on, 0.0)
}

#[allow(clippy::too_many_arguments)]
fn nav_item_indented(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    r: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    count: Option<usize>,
    on: bool,
    indent: f32,
) -> bool {
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    if on {
        p.rect_filled(r, 0.0, theme::accent_alpha(14));
        p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(2.0, ACCENT));
    } else if resp.hovered() {
        p.rect_filled(r, 0.0, theme::tint(6));
    }
    let text = elide(p, label, FontId::proportional(13.0), r.width() - 70.0 - indent);
    p.text(pos2(r.left() + 18.0 + indent, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), if on || resp.hovered() { theme::text() } else { theme::text_2() });
    if let Some(n) = count {
        p.text(pos2(r.right() - 16.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), FontId::monospace(10.0), theme::text_3());
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn toggle_tag(sel: &mut Vec<String>, tag: &str) {
    if let Some(i) = sel.iter().position(|t| t.eq_ignore_ascii_case(tag)) {
        sel.remove(i);
    } else {
        sel.push(tag.to_string());
    }
}

pub(crate) fn base_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn search_icon(p: &egui::Painter, c: egui::Pos2, color: Color32) {
    p.circle_stroke(c - vec2(1.5, 1.5), 5.0, Stroke::new(1.4, color));
    p.line_segment([c + vec2(2.2, 2.2), c + vec2(5.5, 5.5)], Stroke::new(1.6, color));
}

pub(crate) fn placeholder_color(id: i64) -> Color32 {
    const C: [(u8, u8, u8); 6] = [(15, 34, 51), (42, 26, 46), (19, 38, 28), (43, 33, 19), (26, 31, 51), (16, 40, 42)];
    let (r, g, b) = C[(id.unsigned_abs() % C.len() as u64) as usize];
    Color32::from_rgb(r, g, b)
}

fn load_thumb(ctx: &egui::Context, id: i64, path: &str) -> Option<TextureHandle> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(format!("thumb-{id}"), color, egui::TextureOptions::LINEAR))
}

pub(crate) fn elide(p: &egui::Painter, text: &str, font: FontId, max_w: f32) -> String {
    let fits = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), theme::text()).size().x <= max_w;
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&t) { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

pub(crate) fn reveal_in_file_manager(path: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.replace('/', "\\"))).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").args(["-R", path]).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(dir) = Path::new(path).parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

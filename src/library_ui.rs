//! Library home screen, shown whenever nothing is playing:
//! - top bar: search, 媒体库/文件夹 mode switch, add folder, open file, settings
//! - nav: views (all / continue / recent / most played), collections, tags, folders
//! - content: a virtualized thumbnail grid, or a folder browser with breadcrumbs

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2,
};

use crate::app::fmt_time;
use crate::icons::{self, Icon};
use crate::library::{Collection, Folder, Library, Video};
use crate::playlist::natural_cmp;
use crate::theme::{self, ACCENT, LINE, PANEL, TEXT, TEXT_2, TEXT_3};

const TOP_H: f32 = 56.0;
const NAV_W: f32 = 188.0;
const CARD_MIN_W: f32 = 200.0;
const GAP: f32 = 16.0;
const META_H: f32 = 54.0;
/// Thumbnails decoded per frame, so scrolling a big library stays smooth.
const THUMB_LOADS_PER_FRAME: usize = 4;
const FINISHED: f32 = 0.95;

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
}

/// What one grid cell shows.
#[derive(Clone)]
enum Cell {
    Video(i64),
    /// A sub-folder in folder mode: display name and its location.
    Folder { name: String, loc: FolderLoc },
}

/// A place in the folder browser: a watched root plus sub-directories.
#[derive(Clone, PartialEq)]
struct FolderLoc {
    root: i64,
    parts: Vec<String>,
}

pub struct LibraryView {
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
    thumbs: HashMap<i64, (String, TextureHandle)>,
    thumb_failed: HashSet<i64>,
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
    /// Thumbnail generation progress (done, total).
    pub probing: Option<(usize, usize)>,
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
            thumb_failed: HashSet::new(),
            tag_edit: None,
            tag_edit_pos: pos2(0.0, 0.0),
            fav_picker: None,
            fav_picker_fresh: false,
            fav_new_name: String::new(),
            new_collection: None,
            probing: None,
        }
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
        if let Some(lib) = &self.lib {
            self.videos = lib.videos();
            self.folders = lib.folders();
            self.collections = lib.collections();
        }
        // Thumbnails may have been regenerated for changed files.
        let current: HashMap<i64, Option<&String>> = self.videos.iter().map(|v| (v.id, v.thumb.as_ref())).collect();
        self.thumbs.retain(|id, (path, _)| current.get(id).copied().flatten() == Some(&*path));
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
        self.videos.iter().find(|v| v.id == id)
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
        self.reload_if_dirty();
        let mut action = Action::None;
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, theme::BG);

        let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP_H));
        if let Some(a) = self.top_bar(ui, &p, top) {
            action = a;
        }
        let nav = Rect::from_min_max(pos2(rect.left(), top.bottom()), pos2(rect.left() + NAV_W, rect.bottom()));
        self.nav(ui, &p, nav);

        let content = Rect::from_min_max(pos2(nav.right(), top.bottom()), rect.max);
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

    // ---- Top bar -------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui, p: &egui::Painter, top: Rect) -> Option<Action> {
        let mut action = None;
        p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(0.5, LINE));
        p.text(pos2(top.left() + 20.0, top.center().y), Align2::LEFT_CENTER, "TINYPLAYER", FontId::monospace(12.0), ACCENT);

        let search_w = (top.width() - NAV_W - 560.0).clamp(160.0, 380.0);
        let search_rect = Rect::from_min_size(pos2(top.left() + NAV_W + 16.0, top.center().y - 15.0), vec2(search_w, 30.0));
        p.rect_filled(search_rect, 15.0, PANEL);
        p.rect_stroke(search_rect, 15.0, Stroke::new(0.5, LINE), StrokeKind::Inside);
        search_icon(p, pos2(search_rect.left() + 16.0, search_rect.center().y), TEXT_3);
        let edit_rect = Rect::from_min_max(search_rect.min + vec2(30.0, 5.0), search_rect.max - vec2(12.0, 5.0));
        let mut search_resp = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
            search_resp = Some(
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索标题、标签、文件夹")
                        .frame(egui::Frame::NONE)
                        .text_color(TEXT)
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

        // Mode switch: 媒体库 | 文件夹
        let seg = Rect::from_min_size(pos2(search_rect.right() + 16.0, top.center().y - 14.0), vec2(132.0, 28.0));
        p.rect_filled(seg, 14.0, PANEL);
        for (i, (mode, label)) in [(Mode::Grid, "媒体库"), (Mode::Folders, "文件夹")].into_iter().enumerate() {
            let r = Rect::from_min_size(seg.min + vec2(2.0 + i as f32 * 64.0, 2.0), vec2(64.0, 24.0));
            let resp = ui.interact(r, ui.scope_id().with(("mode", i)), Sense::click());
            let on = self.mode == mode;
            if on {
                p.rect_filled(r, 12.0, theme::accent_alpha(36));
            }
            let c = if on { ACCENT } else if resp.hovered() { TEXT } else { TEXT_2 };
            p.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), c);
            if resp.clicked() {
                self.mode = mode;
            }
        }

        // Right side.
        let rx = top.right() - 20.0;
        let gear = Rect::from_center_size(pos2(rx - 12.0, top.center().y), vec2(30.0, 30.0));
        let g = ui.interact(gear, ui.scope_id().with("lib_settings"), Sense::click());
        icons::draw(p, Icon::Settings, gear.center(), 16.0, if g.hovered() { ACCENT } else { TEXT_2 });
        if g.clicked() {
            action = Some(Action::Settings);
        }
        let (open_left, open_clicked) = text_btn(ui, p, pos2(rx - 40.0, top.center().y), "lib_open", "打开文件", TEXT_2);
        if open_clicked {
            action = Some(Action::OpenFile);
        }
        let (add_left, add_clicked) = text_btn(ui, p, pos2(open_left - 24.0, top.center().y), "lib_add", "+ 添加文件夹", ACCENT);
        if add_clicked {
            action = Some(Action::AddFolder);
        }
        if let Some((done, total)) = self.probing {
            if add_left - 20.0 > seg.right() + 140.0 {
                p.text(pos2(add_left - 20.0, top.center().y), Align2::RIGHT_CENTER, format!("生成缩略图 {done}/{total}"), FontId::monospace(11.0), TEXT_3);
            }
        }
        action
    }

    // ---- Nav -----------------------------------------------------------

    fn nav(&mut self, ui: &mut egui::Ui, p: &egui::Painter, nav: Rect) {
        p.line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(0.5, LINE));
        let np = ui.painter_at(nav);
        let mut y = nav.top() + 12.0;
        let grid_mode = self.mode == Mode::Grid;
        let continue_n = self.videos.iter().filter(|v| v.progress().is_some_and(|x| x < FINISHED)).count();
        let week_ago = crate::history::now_secs() - 7 * 24 * 3600;
        let recent_n = self.videos.iter().filter(|v| v.added_at >= week_ago).count();
        let played_n = self.videos.iter().filter(|v| v.plays > 0).count();
        for (view, label, n) in [
            (View::All, "全部", self.videos.len()),
            (View::Continue, "继续观看", continue_n),
            (View::Recent, "最近添加", recent_n),
            (View::MostPlayed, "最常播放", played_n),
        ] {
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 30.0));
            if nav_item(ui, &np, r, ("nav", label), label, Some(n), grid_mode && self.view == view) {
                self.view = view;
                self.mode = Mode::Grid;
            }
            y += 30.0;
        }

        // Collections
        y += 12.0;
        np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "COLLECTIONS", FontId::monospace(10.0), TEXT_3);
        let plus = Rect::from_center_size(pos2(nav.right() - 20.0, y + 8.0), vec2(22.0, 22.0));
        let pr = ui.interact(plus, ui.scope_id().with("new_collection"), Sense::click());
        np.text(plus.center(), Align2::CENTER_CENTER, "+", FontId::proportional(15.0), if pr.hovered() { ACCENT } else { TEXT_3 });
        if pr.clicked() {
            self.new_collection = Some(String::new());
        }
        y += 22.0;
        let fav_n = self.videos.iter().filter(|v| !v.collections.is_empty()).count();
        let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
        if nav_item(ui, &np, r, "fav_all", "全部收藏", Some(fav_n), grid_mode && self.view == View::Favorites(None)) {
            self.view = View::Favorites(None);
            self.mode = Mode::Grid;
        }
        y += 28.0;
        let mut delete = None;
        for c in &self.collections {
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
            let on = grid_mode && self.view == View::Favorites(Some(c.id));
            if nav_item_indented(ui, &np, r, ("coll", c.id), &c.name, Some(c.count as usize), on, 20.0) {
                self.view = View::Favorites(Some(c.id));
                self.mode = Mode::Grid;
            }
            icons::draw(&np, Icon::Star, pos2(r.left() + 24.0, r.center().y), 11.0, if on { ACCENT } else { TEXT_3 });
            ui.interact(r, ui.scope_id().with(("coll", c.id)), Sense::click()).context_menu(|ui| {
                if ui.button("删除收藏夹（不删除视频）").clicked() {
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
                        .hint_text("收藏夹名称")
                        .frame(egui::Frame::NONE)
                        .text_color(TEXT)
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
        let mut tag_counts: Vec<(String, usize)> = Vec::new();
        for v in &self.videos {
            for t in &v.tags {
                match tag_counts.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(t)) {
                    Some((_, c)) => *c += 1,
                    None => tag_counts.push((t.clone(), 1)),
                }
            }
        }
        tag_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| natural_cmp(&a.0, &b.0)));
        if !tag_counts.is_empty() {
            y += 12.0;
            np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "TAGS", FontId::monospace(10.0), TEXT_3);
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
        np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "FOLDERS", FontId::monospace(10.0), TEXT_3);
        y += 22.0;
        let mut remove_folder = None;
        for f in &self.folders {
            if y > nav.bottom() - 28.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
            let name = base_name(&f.path);
            let on = match self.mode {
                Mode::Grid => self.folder_sel == Some(f.id),
                Mode::Folders => self.folder_loc.as_ref().is_some_and(|l| l.root == f.id),
            };
            let hovered = ui.rect_contains_pointer(r);
            if nav_item(ui, &np, r, ("folder", f.id), &name, if hovered { None } else { Some(f.count as usize) }, on) {
                match self.mode {
                    Mode::Grid => self.folder_sel = if on { None } else { Some(f.id) },
                    Mode::Folders => self.folder_loc = Some(FolderLoc { root: f.id, parts: Vec::new() }),
                }
            }
            if hovered {
                let x = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(22.0, 22.0));
                let xr = ui.interact(x, ui.scope_id().with(("folder_rm", f.id)), Sense::click());
                icons::draw(&np, Icon::Close, x.center(), 11.0, if xr.hovered() { ACCENT } else { TEXT_3 });
                let clicked = xr.clicked();
                xr.on_hover_text(format!("从媒体库移除（不删除文件）\n{}", f.path));
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
    }

    // ---- Grid view -----------------------------------------------------

    fn grid_view(&mut self, ui: &mut egui::Ui, p: &egui::Painter, content: Rect) -> Action {
        let ids: Vec<i64> = self.filtered().iter().map(|v| v.id).collect();
        let header = Rect::from_min_size(content.min + vec2(24.0, 12.0), vec2(content.width() - 48.0, 30.0));
        let mut hx = header.left();

        // Title for collection/most-played views.
        let title = match self.view {
            View::Favorites(Some(c)) => self.collections.iter().find(|x| x.id == c).map(|x| x.name.clone()),
            View::Favorites(None) => Some("全部收藏".to_string()),
            View::MostPlayed => Some("最常播放".to_string()),
            _ => None,
        };
        if let Some(t) = title {
            let g = p.layout_no_wrap(t, FontId::proportional(15.0), TEXT);
            let w = g.size().x;
            p.galley(pos2(hx, header.center().y - g.size().y / 2.0), g, TEXT);
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
        let total: f64 = ids.iter().filter_map(|id| self.video(*id)).map(|v| v.duration).sum();
        p.text(
            pos2(hx, header.center().y),
            Align2::LEFT_CENTER,
            format!("{} 个视频 · {}", ids.len(), fmt_time(total)),
            FontId::monospace(11.0),
            TEXT_3,
        );
        if !matches!(self.view, View::Continue | View::MostPlayed) {
            let mut sx = header.right();
            for (sort, label) in [
                (Sort::Duration, "时长"),
                (Sort::Name, "名称"),
                (Sort::Plays, "播放次数"),
                (Sort::Played, "最近播放"),
                (Sort::Added, "最近添加"),
            ] {
                let g = p.layout_no_wrap(label.into(), FontId::proportional(12.0), TEXT);
                let r = Rect::from_min_size(pos2(sx - g.size().x - 12.0, header.center().y - 12.0), vec2(g.size().x + 12.0, 24.0));
                let resp = ui.interact(r, ui.scope_id().with(("sort", label)), Sense::click());
                let on = self.sort == sort;
                let c = if on { ACCENT } else if resp.hovered() { TEXT } else { TEXT_2 };
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
        if ids.is_empty() {
            let msg = match self.view {
                View::Favorites(_) => "还没有收藏。悬停视频，点右上角的星标即可收藏",
                View::MostPlayed => "还没有播放记录",
                View::Continue => "没有看到一半的视频",
                _ => "没有匹配的视频",
            };
            p.text(grid_rect.center() - vec2(0.0, 40.0), Align2::CENTER_CENTER, msg, FontId::proportional(14.0), TEXT_2);
            return Action::None;
        }
        let ranked = self.view == View::MostPlayed;
        let cells: Vec<Cell> = ids.into_iter().map(Cell::Video).collect();
        self.grid(ui, grid_rect, &cells, ranked, "library_grid")
    }

    // ---- Folder browser ------------------------------------------------

    fn folder_browser(&mut self, ui: &mut egui::Ui, p: &egui::Painter, content: Rect) -> Action {
        let header = Rect::from_min_size(content.min + vec2(24.0, 12.0), vec2(content.width() - 48.0, 30.0));
        let cy = header.center().y;

        // Breadcrumb: 全部文件夹 / root / a / b
        let mut crumbs: Vec<(String, Option<FolderLoc>)> = vec![("全部文件夹".into(), None)];
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
            let c = if ur.hovered() { ACCENT } else { TEXT_2 };
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
            let g = p.layout_no_wrap(name, FontId::proportional(13.0), TEXT);
            let r = Rect::from_min_size(pos2(x, cy - g.size().y / 2.0), g.size());
            let resp = ui.interact(r.expand(3.0), ui.scope_id().with(("crumb", i)), Sense::click());
            let c = if last { TEXT } else if resp.hovered() { ACCENT } else { TEXT_2 };
            p.galley(r.min, g, c);
            if resp.clicked() && !last {
                goto = Some(loc);
            }
            x = r.right() + 8.0;
            if !last {
                p.text(pos2(x, cy), Align2::LEFT_CENTER, "/", FontId::proportional(13.0), TEXT_3);
                x += 14.0;
            }
        }
        if let Some(loc) = goto {
            self.folder_loc = loc;
        }

        // Cells: sub-folders first, then videos directly in this folder.
        let (cells, play_all) = self.folder_cells();
        let video_n = cells.iter().filter(|c| matches!(c, Cell::Video(_))).count();
        let folder_n = cells.len() - video_n;
        let stat = format!("{folder_n} 个文件夹 · {video_n} 个视频");
        let (pa_left, pa_clicked) = text_btn(ui, p, pos2(header.right(), cy), "play_all", "播放全部", ACCENT);
        p.text(pos2(pa_left - 20.0, cy), Align2::RIGHT_CENTER, stat, FontId::monospace(11.0), TEXT_3);

        let grid_rect = Rect::from_min_max(pos2(content.left() + 24.0, header.bottom() + 8.0), pos2(content.right() - 12.0, content.bottom()));
        if cells.is_empty() {
            p.text(grid_rect.center() - vec2(0.0, 40.0), Align2::CENTER_CENTER, "这个文件夹里没有视频", FontId::proportional(14.0), TEXT_2);
            return Action::None;
        }
        let action = self.grid(ui, grid_rect, &cells, false, "folder_grid");
        if pa_clicked && !play_all.is_empty() {
            return Action::PlayAll(play_all);
        }
        action
    }

    /// Cells for the current folder location, plus every video below it
    /// (recursive, natural order) for "全部播放".
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
        let watched = inside.iter().filter(|v| v.progress().is_some_and(|p| p >= FINISHED)).count();
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
                                let Some(v) = self.video(id) else { continue };
                                let tex = self.thumbs.get(&id).map(|(_, t)| t.id());
                                let rank = ranked.then_some(i + 1);
                                if let Some(ev) = draw_card(ui, &p, card, thumb, v, tex, rank) {
                                    events.push(ev);
                                }
                            }
                            Cell::Folder { name, loc } => {
                                let (covers, count, total, watched) = self.folder_stats(loc);
                                for &id in &covers {
                                    if self.ensure_thumb(ui.ctx(), id, &mut loads) {
                                        pending_thumbs = true;
                                    }
                                }
                                let texs: Vec<Option<egui::TextureId>> =
                                    covers.iter().map(|id| self.thumbs.get(id).map(|(_, t)| t.id())).collect();
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
                self.thumbs.insert(id, (path, t));
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
                            .hint_text("输入标签，回车添加")
                            .frame(egui::Frame::NONE)
                            .text_color(TEXT)
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
                    ui.label(egui::RichText::new("收藏到").size(11.0).color(TEXT_3));
                    ui.add_space(4.0);
                    for c in &self.collections {
                        let on = in_cols.contains(&c.id);
                        let (r, resp) = ui.allocate_exact_size(vec2(190.0, 26.0), Sense::click());
                        let p = ui.painter();
                        if resp.hovered() {
                            p.rect_filled(r, 4.0, Color32::from_white_alpha(10));
                        }
                        let bx = Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(13.0, 13.0));
                        if on {
                            p.rect_filled(bx, 3.0, ACCENT);
                            p.line_segment([bx.center() + vec2(-3.5, 0.0), bx.center() + vec2(-1.0, 2.5)], Stroke::new(1.6, Color32::WHITE));
                            p.line_segment([bx.center() + vec2(-1.0, 2.5), bx.center() + vec2(3.5, -2.5)], Stroke::new(1.6, Color32::WHITE));
                        } else {
                            p.rect_stroke(bx, 3.0, Stroke::new(1.0, TEXT_3), StrokeKind::Inside);
                        }
                        p.text(pos2(r.left() + 28.0, r.center().y), Align2::LEFT_CENTER, &c.name, FontId::proportional(13.0), TEXT);
                        if resp.clicked() {
                            toggles.push((c.id, !on));
                        }
                    }
                    ui.add_space(4.0);
                    ui.separator();
                    let name = &mut self.fav_new_name;
                    let r = ui.add(
                        egui::TextEdit::singleline(name)
                            .hint_text("+ 新建收藏夹，回车创建")
                            .frame(egui::Frame::NONE)
                            .text_color(TEXT)
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
            let label = if v.duration > 0.0 { "" } else { "生成中…" };
            p.text(thumb.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.0), TEXT_3);
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
        sr.clone().on_hover_text(if fav { "已收藏 · 点击管理收藏夹" } else { "收藏" });
        if sr.clicked() {
            ev = Some(CardEvent::Favorite(v.id, pos2(star.right() - 190.0, star.bottom() + 6.0)));
        }
    }

    let progress = v.progress();
    let finished = progress.is_some_and(|x| x >= FINISHED);
    let badge = if finished { "已看完".to_string() } else if v.duration > 0.0 { fmt_time(v.duration) } else { String::new() };
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
    let plays = if v.plays > 0 { format!("▶ {}", v.plays) } else { String::new() };
    let plays_font = FontId::monospace(if rank.is_some() { 11.0 } else { 10.0 });
    let plays_w = if plays.is_empty() { 0.0 } else { p.layout_no_wrap(plays.clone(), plays_font.clone(), TEXT).size().x + 8.0 };
    let title = elide(p, &v.title, FontId::proportional(13.0), card.width() - plays_w);
    p.text(pos2(card.left(), ty), Align2::LEFT_CENTER, title, FontId::proportional(13.0), if finished { TEXT_2 } else { TEXT });
    if !plays.is_empty() {
        p.text(pos2(card.right(), ty), Align2::RIGHT_CENTER, plays, plays_font, if rank.is_some() { ACCENT } else { TEXT_3 });
    }

    // Tags + "+ 标签".
    let mut x = card.left();
    let cy = thumb.bottom() + 38.0;
    for t in &v.tags {
        let g = p.layout_no_wrap(format!("#{t}"), FontId::proportional(11.0), TEXT_2);
        let r = Rect::from_min_size(pos2(x, cy - 9.0), g.size() + vec2(10.0, 4.0));
        if r.right() > card.right() - 44.0 {
            break;
        }
        let tr = ui.interact(r, ui.scope_id().with(("card_tag", v.id, t.as_str())), Sense::click());
        p.rect_filled(r, 3.0, if tr.hovered() { theme::accent_alpha(40) } else { Color32::from_white_alpha(14) });
        p.galley(r.min + vec2(5.0, 2.0), g, if tr.hovered() { ACCENT } else { TEXT_2 });
        if tr.clicked() {
            ev = Some(CardEvent::TagFilter(t.clone()));
        }
        x = r.right() + 5.0;
    }
    let add = Rect::from_min_size(pos2(x, cy - 9.0), vec2(42.0, 18.0));
    let ar = ui.interact(add, ui.scope_id().with(("card_add_tag", v.id)), Sense::click());
    p.text(add.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, "+ 标签", FontId::proportional(11.0), if ar.hovered() { ACCENT } else { TEXT_3 });
    if ar.clicked() {
        ev = Some(CardEvent::EditTags(v.id, pos2(card.left(), cy + 12.0)));
    }

    if resp.clicked() && ev.is_none() {
        ev = Some(CardEvent::Play(v.path.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button("播放").clicked() {
            ev = Some(CardEvent::Play(v.path.clone()));
            ui.close();
        }
        if ui.button(if fav { "管理收藏夹…" } else { "收藏到…" }).clicked() {
            ev = Some(CardEvent::Favorite(v.id, pos2(thumb.left() + 8.0, thumb.top() + 8.0)));
            ui.close();
        }
        if ui.button("添加标签…").clicked() {
            ev = Some(CardEvent::EditTags(v.id, pos2(card.left(), cy + 12.0)));
            ui.close();
        }
        if !v.tags.is_empty() {
            ui.menu_button("移除标签", |ui| {
                for t in &v.tags {
                    if ui.button(format!("#{t}")).clicked() {
                        ev = Some(CardEvent::RemoveTag(v.id, t.clone()));
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        let reveal = if cfg!(target_os = "macos") { "在访达中显示" } else { "在资源管理器中显示" };
        if ui.button(reveal).clicked() {
            ev = Some(CardEvent::Reveal(v.path.clone()));
            ui.close();
        }
        if ui.button("从媒体库移除（不删除文件）").clicked() {
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
    p.rect_filled(thumb, 6.0, theme::BG);
    let (w, h) = ((thumb.width() - 1.0) / 2.0, (thumb.height() - 1.0) / 2.0);
    for i in 0..4 {
        let cell = Rect::from_min_size(thumb.min + vec2((i % 2) as f32 * (w + 1.0), (i / 2) as f32 * (h + 1.0)), vec2(w, h));
        match covers.get(i).copied().flatten() {
            Some(t) => {
                p.image(t, cell, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(cell, 0.0, PANEL);
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
    p.text(pos2(card.left(), ty), Align2::LEFT_CENTER, elide(p, name, FontId::proportional(13.0), card.width()), FontId::proportional(13.0), TEXT);
    let mut meta = fmt_time(total);
    if count > 0 && watched > 0 {
        meta.push_str(&format!(" · 看完 {watched}/{count}"));
    }
    p.text(pos2(card.left(), thumb.bottom() + 36.0), Align2::LEFT_CENTER, meta, FontId::monospace(10.0), TEXT_3);

    if resp.clicked() {
        ev = Some(CardEvent::OpenFolder(loc.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button("打开").clicked() {
            ev = Some(CardEvent::OpenFolder(loc.clone()));
            ui.close();
        }
        if ui.button("播放全部").clicked() {
            ev = Some(CardEvent::PlayFolder(loc.clone()));
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
    p.text(frame.center() + vec2(0.0, 24.0), Align2::CENTER_CENTER, "建立你的媒体库", FontId::proportional(14.0), TEXT);
    p.text(pos2(c.x, frame.bottom() + 22.0), Align2::CENTER_CENTER, "添加视频所在的文件夹，自动生成缩略图，支持标签与搜索", FontId::proportional(12.0), TEXT_2);
    let mut action = Action::None;
    let y = frame.bottom() + 60.0;
    if text_btn(ui, p, pos2(c.x - 14.0, y), "empty_add", "+ 添加文件夹", ACCENT).1 {
        action = Action::AddFolder;
    }
    let open_w = p.layout_no_wrap("打开文件".into(), FontId::proportional(13.0), TEXT).size().x;
    if text_btn(ui, p, pos2(c.x + 14.0 + open_w, y), "empty_open", "打开文件", TEXT_2).1 {
        action = Action::OpenFile;
    }
    p.text(pos2(c.x, y + 40.0), Align2::CENTER_CENTER, "也可以直接把视频拖进窗口播放", FontId::proportional(11.0), TEXT_3);
    action
}

// ---- Small widgets ---------------------------------------------------------

fn popup_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .stroke(Stroke::new(0.5, ACCENT))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// Right-aligned text button ending at `right_center`. Returns (left edge, clicked).
fn text_btn(ui: &mut egui::Ui, p: &egui::Painter, right_center: egui::Pos2, id: &str, text: &str, color: Color32) -> (f32, bool) {
    let g = p.layout_no_wrap(text.to_string(), FontId::proportional(13.0), color);
    let r = Rect::from_min_size(pos2(right_center.x - g.size().x, right_center.y - g.size().y / 2.0), g.size());
    let resp = ui.interact(r.expand2(vec2(8.0, 6.0)), ui.scope_id().with(id), Sense::click());
    let c = if resp.hovered() { TEXT } else { color };
    p.text(r.min, Align2::LEFT_TOP, text, FontId::proportional(13.0), c);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    (r.left(), resp.clicked())
}

fn nav_item(
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
        p.rect_filled(r, 0.0, Color32::from_white_alpha(6));
    }
    let text = elide(p, label, FontId::proportional(13.0), r.width() - 70.0 - indent);
    p.text(pos2(r.left() + 18.0 + indent, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), if on || resp.hovered() { TEXT } else { TEXT_2 });
    if let Some(n) = count {
        p.text(pos2(r.right() - 16.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), FontId::monospace(10.0), TEXT_3);
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

fn base_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn search_icon(p: &egui::Painter, c: egui::Pos2, color: Color32) {
    p.circle_stroke(c - vec2(1.5, 1.5), 5.0, Stroke::new(1.4, color));
    p.line_segment([c + vec2(2.2, 2.2), c + vec2(5.5, 5.5)], Stroke::new(1.6, color));
}

fn placeholder_color(id: i64) -> Color32 {
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

fn elide(p: &egui::Painter, text: &str, font: FontId, max_w: f32) -> String {
    let fits = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), TEXT).size().x <= max_w;
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

fn reveal_in_file_manager(path: &str) {
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

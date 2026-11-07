//! The library's 管理 page: videos by folder set, search across folders and
//! videos, moving files, and checking for likely duplicates.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::app::fmt_time;
use crate::duplicates::{self, Group, Item, Level, Options};
use crate::file_move::{self, FolderJob, Job, Mover, Msg, Task};
use crate::i18n::{tf, tr};
use crate::library::{Collection, Folder, FolderSet, Library, Video};
use crate::library_ui::{base_name, elide, nav_item, placeholder_color, reveal_in_file_manager};
use crate::playlist::natural_cmp;
use crate::theme::{self, ACCENT};

const ROW_H: f32 = 44.0;
const BAR_H: f32 = 48.0;
const COL_DURATION: f32 = 76.0;
const COL_SIZE: f32 = 76.0;
const COL_DATE: f32 = 88.0;
/// Where a row's picture and text start.
const THUMB_X: f32 = 30.0;
const TEXT_X: f32 = 84.0;
const DISK_CHECK_SECS: f64 = 5.0;
const TOAST_SECS: f64 = 8.0;
/// Limits on what the folder picker lists, so a huge disk can't stall it.
const MAX_PICKER_DIRS: usize = 3000;
const MAX_PICKER_DEPTH: usize = 8;
const MAX_GROUPS_SHOWN: usize = 300;
const MARKED: Color32 = Color32::from_rgb(0xF0, 0x71, 0x67);

const LEVELS: [(Level, &str); 3] = [(Level::Strict, "严格"), (Level::Normal, "标准"), (Level::Loose, "宽松")];

pub struct Data<'a> {
    pub lib: &'a Library,
    pub videos: &'a [Video],
    pub folders: &'a [Folder],
    pub search: &'a str,
    /// Changes whenever `videos` is reloaded.
    pub revision: u64,
    pub collections: &'a [Collection],
    /// Thumbnails loaded for the videos that were on screen.
    pub thumbs: &'a HashMap<i64, egui::TextureId>,
}

#[derive(Default)]
pub struct Outcome {
    pub play: Option<String>,
    pub clear_search: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Videos,
    Duplicates,
}

#[derive(Clone, Copy, PartialEq)]
enum Col {
    Name,
    Duration,
    Size,
    Date,
}

#[derive(Clone, Copy, PartialEq)]
enum Show {
    All,
    Strong,
    Weak,
}

#[derive(Default, Clone, Copy)]
struct Stats {
    count: usize,
    duration: f64,
    size: i64,
    mtime: i64,
}

enum Row {
    Header(String),
    Folder { path: PathBuf, stats: Stats, parent: Option<String>, offline: bool, root: bool },
    Video { id: i64, with_path: bool },
}

struct Dir {
    root: i64,
    path: PathBuf,
    depth: usize,
}

enum Dialog {
    NewSet { name: String, picked: Vec<i64>, error: Option<&'static str> },
    Move { ids: Vec<i64>, folders: Vec<PathBuf>, dirs: Vec<Dir>, filter: String, pick: Option<usize>, new_name: String, error: Option<&'static str> },
    Trash { ids: Vec<i64> },
    Rename { id: i64, stem: String, error: Option<&'static str> },
    Tag { ids: Vec<i64>, name: String, error: Option<&'static str> },
    Collect { ids: Vec<i64>, picked: Vec<i64> },
}

#[derive(Default)]
struct Progress {
    name: String,
    done: u64,
    total: u64,
    verifying: bool,
    moved: usize,
    failed: usize,
    jobs: usize,
}

struct Toast {
    text: String,
    at: f64,
    undo: bool,
}

/// The watched folders that couldn't be reached at the last look.
struct Disks {
    offline: HashSet<i64>,
    /// Asked at this time; the next look isn't before [`DISK_CHECK_SECS`].
    checked_at: f64,
    looking: Option<Receiver<HashSet<i64>>>,
}

impl Default for Disks {
    fn default() -> Self {
        Self { offline: HashSet::new(), checked_at: f64::NEG_INFINITY, looking: None }
    }
}

enum Undo {
    Moves,
    Ignored(String),
}

pub struct Manager {
    page: Page,
    set: Option<i64>,
    sets: Vec<FolderSet>,
    sets_dirty: bool,
    cwd: Option<PathBuf>,
    col: Col,
    descending: bool,
    selected: HashSet<i64>,
    selected_dirs: HashSet<PathBuf>,
    /// Videos drawn last frame, whose thumbnails are wanted.
    visible: Vec<i64>,
    disks: Disks,
    dialog: Option<Dialog>,
    mover: Option<Mover>,
    progress: Progress,
    /// Each moved video's previous folder, for undo.
    origins: HashMap<i64, i64>,
    undo_moves: Vec<Task>,
    undo: Option<Undo>,
    toast: Option<Toast>,
    options: Options,
    show: Show,
    groups: Vec<Group>,
    ignored: Option<HashSet<String>>,
    marked: HashSet<i64>,
    /// What `groups` was found from; None until the first search.
    searched: Option<(Options, u64, Option<i64>, usize)>,
    searching: Option<Receiver<Vec<Group>>>,
    changed: bool,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            page: Page::Videos,
            set: None,
            sets: Vec::new(),
            sets_dirty: true,
            cwd: None,
            col: Col::Date,
            descending: true,
            selected: HashSet::new(),
            selected_dirs: HashSet::new(),
            visible: Vec::new(),
            disks: Disks::default(),
            dialog: None,
            mover: None,
            progress: Progress::default(),
            origins: HashMap::new(),
            undo_moves: Vec::new(),
            undo: None,
            toast: None,
            options: Options { duration: true, size: true, name: true, level: Level::Normal },
            show: Show::All,
            groups: Vec::new(),
            ignored: None,
            marked: HashSet::new(),
            searched: None,
            searching: None,
            changed: false,
        }
    }

    fn say(&mut self, ctx: &egui::Context, text: String, undo: Option<Undo>) {
        self.undo = undo;
        self.toast = Some(Toast { undo: self.undo.is_some(), text, at: ctx.input(|i| i.time) });
    }

    fn root_ids(&self, folders: &[Folder]) -> Vec<i64> {
        let chosen = self.set.and_then(|id| self.sets.iter().find(|s| s.id == id));
        match chosen {
            Some(s) => s.folders.iter().copied().filter(|f| folders.iter().any(|x| x.id == *f)).collect(),
            None => folders.iter().map(|f| f.id).collect(),
        }
    }

    // ---- Moving --------------------------------------------------------

    /// Looks now and then for disks coming and going, without blocking the
    /// UI on one that's gone. Returns true when one did, for a new scan.
    pub fn watch_disks(&mut self, ctx: &egui::Context, folders: &[Folder]) -> bool {
        let now = ctx.input(|i| i.time);
        let mut changed = false;
        if let Some(rx) = &self.disks.looking {
            if let Ok(offline) = rx.try_recv() {
                self.disks.looking = None;
                changed = offline != self.disks.offline;
                self.disks.offline = offline;
            }
        } else if now - self.disks.checked_at >= DISK_CHECK_SECS {
            self.disks.checked_at = now;
            let roots: Vec<(i64, PathBuf)> = folders.iter().map(|f| (f.id, PathBuf::from(&f.path))).collect();
            let (tx, rx) = mpsc::channel();
            let wake = ctx.clone();
            let started = std::thread::Builder::new().name("disk-check".into()).spawn(move || {
                let offline = roots.into_iter().filter(|(_, p)| !p.is_dir()).map(|(id, _)| id).collect();
                let _ = tx.send(offline);
                wake.request_repaint();
            });
            self.disks.looking = started.is_ok().then_some(rx);
        }
        ctx.request_repaint_after_secs(DISK_CHECK_SECS as f32);
        changed
    }

    /// Applies finished work. Returns true if the library changed.
    pub fn poll(&mut self, ctx: &egui::Context, lib: Option<&Library>, videos: &[Video]) -> bool {
        let Some(mover) = &self.mover else { return false };
        let mut changed = false;
        let mut finished = false;
        while let Ok(msg) = mover.rx.try_recv() {
            match msg {
                Msg::Started { name } => {
                    self.progress.name = name;
                    self.progress.done = 0;
                    self.progress.total = 0;
                    self.progress.verifying = false;
                }
                Msg::Copied { done, total } => (self.progress.done, self.progress.total) = (done, total),
                Msg::Verifying => self.progress.verifying = true,
                Msg::Moved { job, to } => {
                    if let Some(lib) = lib {
                        lib.record_move(job.id, &to, job.folder_id);
                    }
                    if let (Some(dir), Some(folder)) = (job.from.parent(), self.origins.get(&job.id)) {
                        self.undo_moves.push(Task::File(Job { id: job.id, from: to, to_dir: dir.to_path_buf(), folder_id: *folder }));
                    }
                    self.progress.moved += 1;
                    changed = true;
                }
                Msg::FolderMoved { job, to } => {
                    let mut origin = job.folder_id;
                    for v in videos {
                        if let Ok(rest) = Path::new(&v.path).strip_prefix(&job.from) {
                            origin = v.folder_id;
                            if let Some(lib) = lib {
                                lib.record_move(v.id, &to.join(rest), job.folder_id);
                            }
                        }
                    }
                    if let Some(dir) = job.from.parent() {
                        self.undo_moves.push(Task::Folder(FolderJob { from: to, to_dir: dir.to_path_buf(), folder_id: origin }));
                    }
                    self.progress.moved += 1;
                    changed = true;
                }
                Msg::Failed => self.progress.failed += 1,
                Msg::Finished => finished = true,
            }
        }
        if finished {
            self.mover = None;
            self.selected.clear();
            self.selected_dirs.clear();
            let (moved, failed) = (self.progress.moved, self.progress.failed);
            let text = if failed > 0 {
                tf("已剪切 {} 项，{} 项失败，失败的还在原位置", &[&moved, &failed])
            } else if moved > 0 {
                tf("已剪切 {} 项", &[&moved])
            } else {
                tr("已取消，原文件没有变化").to_string()
            };
            let undo = (moved > 0).then_some(Undo::Moves);
            self.say(ctx, text, undo);
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after_secs(0.1);
        }
        changed
    }

    fn start_moves(&mut self, ctx: &egui::Context, jobs: Vec<Task>, videos: &[Video]) {
        self.origins = videos.iter().map(|v| (v.id, v.folder_id)).collect();
        self.undo_moves.clear();
        self.progress = Progress { jobs: jobs.len(), ..Progress::default() };
        let wake = ctx.clone();
        self.mover = Some(Mover::start(jobs, move || wake.request_repaint()));
    }

    fn undo_last(&mut self, ctx: &egui::Context, d: &Data) {
        match self.undo.take() {
            Some(Undo::Moves) => {
                let mut jobs = std::mem::take(&mut self.undo_moves);
                jobs.reverse();
                self.start_moves(ctx, jobs, d.videos);
            }
            Some(Undo::Ignored(key)) => {
                d.lib.ignore_duplicate(&key, false);
                self.ignored = None;
            }
            None => {}
        }
        self.toast = None;
    }

    // ---- Navigation ----------------------------------------------------

    pub fn nav(&mut self, ui: &mut egui::Ui, p: &egui::Painter, nav: Rect, d: &Data) {
        if self.sets_dirty {
            self.sets = d.lib.folder_sets();
            self.sets_dirty = false;
        }
        p.line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(0.5, theme::line()));
        let np = ui.painter_at(nav);
        let mut y = nav.top() + 12.0;
        np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "FOLDER SETS", FontId::monospace(10.0), theme::text_3());
        let plus = Rect::from_center_size(pos2(nav.right() - 20.0, y + 8.0), vec2(22.0, 22.0));
        let pr = ui.interact(plus, ui.scope_id().with("new_set"), Sense::click());
        np.text(plus.center(), Align2::CENTER_CENTER, "+", FontId::proportional(15.0), if pr.hovered() { ACCENT } else { theme::text_3() });
        if pr.clicked() {
            self.dialog = Some(Dialog::NewSet { name: String::new(), picked: Vec::new(), error: None });
        }
        y += 22.0;

        let videos_page = self.page == Page::Videos;
        let r = Rect::from_min_size(pos2(nav.left(), y), vec2(nav.width(), 30.0));
        if nav_item(ui, &np, r, "set_all", tr("全部文件夹"), Some(d.videos.len()), videos_page && self.set.is_none()) {
            self.choose_set(None);
        }
        y += 30.0;
        let mut delete = None;
        for set in self.sets.clone() {
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(nav.width(), 30.0));
            let count = d.videos.iter().filter(|v| set.folders.contains(&v.folder_id)).count();
            if nav_item(ui, &np, r, ("set", set.id), &set.name, Some(count), videos_page && self.set == Some(set.id)) {
                self.choose_set(Some(set.id));
            }
            ui.interact(r, ui.scope_id().with(("set_menu", set.id)), Sense::click()).context_menu(|ui| {
                if ui.button(tr("删除集合")).clicked() {
                    delete = Some(set.id);
                    ui.close();
                }
            });
            y += 30.0;
        }
        if let Some(id) = delete {
            d.lib.delete_folder_set(id);
            self.sets_dirty = true;
            if self.set == Some(id) {
                self.choose_set(None);
            }
        }

        y += 12.0;
        np.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "TOOLS", FontId::monospace(10.0), theme::text_3());
        y += 22.0;
        let r = Rect::from_min_size(pos2(nav.left(), y), vec2(nav.width(), 30.0));
        if nav_item(ui, &np, r, "dups", tr("可能重复"), None, !videos_page) {
            self.page = Page::Duplicates;
            self.clear_selection();
        }
    }

    fn choose_set(&mut self, set: Option<i64>) {
        self.set = set;
        self.page = Page::Videos;
        self.cwd = None;
        self.clear_selection();
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.selected_dirs.clear();
    }

    // ---- Content -------------------------------------------------------

    pub fn show(&mut self, ui: &mut egui::Ui, p: &egui::Painter, rect: Rect, d: &Data) -> Outcome {
        let mut out = Outcome::default();
        self.visible.clear();
        match self.page {
            Page::Videos => self.videos_page(ui, p, rect, d, &mut out),
            Page::Duplicates => self.duplicates_page(ui, p, rect, d, &mut out),
        }
        self.dialogs(ui, d);
        self.progress_dialog(ui);
        self.draw_toast(ui, p, rect, d);
        out
    }

    fn rows(&self, d: &Data) -> Vec<Row> {
        let roots = self.root_ids(d.folders);
        let folder_path: HashMap<i64, &str> = d.folders.iter().map(|f| (f.id, f.path.as_str())).collect();
        let scoped: Vec<&Video> = d.videos.iter().filter(|v| roots.contains(&v.folder_id)).collect();

        let mut dirs: BTreeMap<PathBuf, Stats> = BTreeMap::new();
        for v in &scoped {
            let Some(root) = folder_path.get(&v.folder_id).map(Path::new) else { continue };
            for dir in Path::new(&v.path).ancestors().skip(1) {
                let entry = dirs.entry(dir.to_path_buf()).or_default();
                entry.count += 1;
                entry.duration += v.duration;
                entry.size += v.size;
                entry.mtime = entry.mtime.max(v.mtime);
                if dir == root {
                    break;
                }
            }
        }

        let words: Vec<String> = d.search.to_lowercase().split_whitespace().map(str::to_string).collect();
        let mut rows = Vec::new();
        if !words.is_empty() {
            let hits: Vec<Row> = dirs
                .iter()
                .filter(|(path, _)| {
                    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                    words.iter().all(|w| name.contains(w.as_str()))
                })
                .map(|(path, stats)| Row::Folder {
                    path: path.clone(),
                    stats: *stats,
                    parent: path.parent().map(|x| x.to_string_lossy().into_owned()),
                    offline: false,
                    root: d.folders.iter().any(|f| Path::new(&f.path) == path),
                })
                .collect();
            if !hits.is_empty() {
                rows.push(Row::Header(tr("文件夹").to_string()));
                rows.extend(hits);
            }
            let mut videos: Vec<&&Video> = scoped
                .iter()
                .filter(|v| {
                    let text = format!("{} {}", v.title, v.path).to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .collect();
            self.sort(&mut videos);
            if !videos.is_empty() {
                rows.push(Row::Header(tr("视频").to_string()));
                rows.extend(videos.iter().map(|v| Row::Video { id: v.id, with_path: true }));
            }
        } else if let Some(cwd) = &self.cwd {
            for (path, stats) in &dirs {
                if path.parent() == Some(cwd.as_path()) {
                    rows.push(Row::Folder { path: path.clone(), stats: *stats, parent: None, offline: false, root: false });
                }
            }
            let mut videos: Vec<&&Video> = scoped.iter().filter(|v| Path::new(&v.path).parent() == Some(cwd.as_path())).collect();
            self.sort(&mut videos);
            rows.extend(videos.iter().map(|v| Row::Video { id: v.id, with_path: false }));
        } else {
            for id in &roots {
                if let Some(path) = folder_path.get(id) {
                    let path = PathBuf::from(path);
                    let offline = self.disks.offline.contains(id);
                    let mut stats = dirs.get(&path).copied().unwrap_or_default();
                    if offline {
                        stats.count = d.folders.iter().find(|f| f.id == *id).map_or(0, |f| f.missing as usize);
                    }
                    let parent = path.parent().map(|x| x.to_string_lossy().into_owned());
                    rows.push(Row::Folder { path, stats, parent, offline, root: true });
                }
            }
        }
        rows
    }

    fn sort(&self, videos: &mut [&&Video]) {
        videos.sort_by(|a, b| {
            let order = match self.col {
                Col::Name => natural_cmp(&a.title, &b.title),
                Col::Duration => a.duration.total_cmp(&b.duration),
                Col::Size => a.size.cmp(&b.size),
                Col::Date => a.mtime.cmp(&b.mtime),
            };
            if self.descending { order.reverse() } else { order }
        });
    }

    fn videos_page(&mut self, ui: &mut egui::Ui, p: &egui::Painter, rect: Rect, d: &Data, out: &mut Outcome) {
        let rect = rect.shrink2(vec2(16.0, 0.0));
        let searching = !d.search.trim().is_empty();
        let rows = self.rows(d);
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
        if searching {
            let folders = rows.iter().filter(|r| matches!(r, Row::Folder { .. })).count();
            let videos = rows.iter().filter(|r| matches!(r, Row::Video { .. })).count();
            let text = tf("“{}”：{} 个文件夹，{} 个视频", &[&d.search.trim(), &folders, &videos]);
            p.text(pos2(head.left(), head.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), theme::text_2());
        } else {
            self.breadcrumb(ui, p, head, d);
        }

        let cols = Rect::from_min_size(pos2(rect.left(), head.bottom()), vec2(rect.width(), 24.0));
        self.column_header(ui, p, cols);

        let any_selected = !self.selected.is_empty() || !self.selected_dirs.is_empty();
        let batch_h = if any_selected { 52.0 } else { 0.0 };
        let list = Rect::from_min_max(pos2(rect.left(), cols.bottom()), pos2(rect.right(), rect.bottom() - batch_h));
        if rows.is_empty() {
            let text = if searching { tr("这个集合里没有匹配项") } else { tr("这里没有视频") };
            p.text(list.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), theme::text_3());
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical().id_salt(("manage_list", self.cwd.clone(), self.set)).auto_shrink(false).show_rows(
                    ui,
                    ROW_H,
                    rows.len(),
                    |ui, range| {
                        for row in &rows[range] {
                            self.draw_row(ui, row, d, out);
                        }
                    },
                );
            });
        }
        if any_selected {
            let bar = Rect::from_min_size(pos2(rect.left(), rect.bottom() - 46.0), vec2(rect.width(), 38.0));
            self.batch_bar(ui, p, bar, d);
        }
    }

    fn breadcrumb(&mut self, ui: &mut egui::Ui, p: &egui::Painter, head: Rect, d: &Data) {
        let set_name = self.set.and_then(|id| self.sets.iter().find(|s| s.id == id)).map_or(tr("全部文件夹"), |s| s.name.as_str()).to_string();
        let Some(cwd) = self.cwd.clone() else {
            p.text(pos2(head.left(), head.center().y), Align2::LEFT_CENTER, set_name, FontId::proportional(13.0), theme::text_2());
            return;
        };
        let root = d.folders.iter().map(|f| PathBuf::from(&f.path)).find(|r| cwd.starts_with(r));
        let Some(root) = root else { return };
        let mut stops: Vec<(String, Option<PathBuf>)> = vec![(set_name, None)];
        let mut at = root.clone();
        stops.push((base_name(&root.to_string_lossy()), Some(at.clone())));
        for part in cwd.strip_prefix(&root).into_iter().flatten() {
            at.push(part);
            stops.push((part.to_string_lossy().into_owned(), Some(at.clone())));
        }
        let mut x = head.left();
        let font = FontId::proportional(13.0);
        for (i, (label, target)) in stops.into_iter().enumerate() {
            if i > 0 {
                let sep = p.text(pos2(x, head.center().y), Align2::LEFT_CENTER, " / ", font.clone(), theme::text_3());
                x = sep.right();
            }
            let width = p.layout_no_wrap(label.clone(), font.clone(), theme::text()).size().x;
            let r = Rect::from_min_size(pos2(x, head.center().y - 12.0), vec2(width, 24.0));
            let resp = ui.interact(r, ui.scope_id().with(("crumb", i)), Sense::click());
            let color = if resp.hovered() { ACCENT } else { theme::text_2() };
            p.text(pos2(x, head.center().y), Align2::LEFT_CENTER, label, font.clone(), color);
            if resp.clicked() {
                self.cwd = target;
                self.clear_selection();
            }
            x += width;
        }
    }

    fn column_header(&mut self, ui: &mut egui::Ui, p: &egui::Painter, r: Rect) {
        p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.5, theme::line()));
        let right = r.right() - 14.0;
        let columns = [
            (Col::Name, tr("名称"), Rect::from_min_max(pos2(r.left() + TEXT_X, r.top()), pos2(r.left() + 200.0, r.bottom())), Align2::LEFT_CENTER),
            (Col::Duration, tr("时长"), column(r, right - COL_DATE - COL_SIZE, COL_DURATION), Align2::RIGHT_CENTER),
            (Col::Size, tr("大小"), column(r, right - COL_DATE, COL_SIZE), Align2::RIGHT_CENTER),
            (Col::Date, tr("修改日期"), column(r, right, COL_DATE), Align2::RIGHT_CENTER),
        ];
        for (col, label, area, align) in columns {
            let resp = ui.interact(area, ui.scope_id().with(("col", label)), Sense::click());
            let on = self.col == col;
            let arrow = if !on { "" } else if self.descending { " ↓" } else { " ↑" };
            let color = if on { ACCENT } else if resp.hovered() { theme::text() } else { theme::text_3() };
            let at = if align == Align2::LEFT_CENTER { area.left_center() } else { area.right_center() };
            p.text(at, align, format!("{label}{arrow}"), FontId::proportional(11.0), color);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() {
                if on {
                    self.descending = !self.descending;
                } else {
                    self.col = col;
                    self.descending = col != Col::Name;
                }
            }
        }
    }

    fn draw_row(&mut self, ui: &mut egui::Ui, row: &Row, d: &Data, out: &mut Outcome) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        let p = ui.painter_at(r);
        if !matches!(row, Row::Header(_)) {
            p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.5, theme::line()));
        }
        let right = r.right() - 14.0;
        match row {
            Row::Header(text) => {
                p.text(pos2(r.left() + 4.0, r.bottom() - 12.0), Align2::LEFT_CENTER, text, FontId::proportional(11.0), theme::text_3());
            }
            Row::Folder { path, stats, parent, offline, root } => {
                let on = self.selected_dirs.contains(path);
                if on {
                    p.rect_filled(r, 0.0, theme::accent_alpha(26));
                } else if resp.hovered() && !*offline {
                    p.rect_filled(r, 0.0, theme::tint(6));
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                if !*root {
                    let boxed = Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), vec2(26.0, 30.0));
                    let tick = ui.interact(boxed, ui.scope_id().with(("tick_dir", path)), Sense::click());
                    draw_check(&p, boxed.center(), on, tick.hovered());
                    if tick.clicked() {
                        self.toggle_dir(path);
                    }
                }
                let icon = Rect::from_min_size(pos2(r.left() + THUMB_X + 10.0, r.center().y - 9.0), vec2(24.0, 18.0));
                draw_folder_icon(&p, icon);
                let name = base_name(&path.to_string_lossy());
                let sub = match parent {
                    Some(_) if *offline => tf("硬盘未连接 · 保留了 {} 个视频的记录", &[&stats.count]),
                    Some(parent) => tf("{} 个视频 · {}", &[&stats.count, parent]),
                    None => tf("{} 个视频", &[&stats.count]),
                };
                self.row_text(&p, r, &name, Some(&sub));
                if !*offline {
                    self.row_columns(&p, r, right, stats.duration, stats.size, stats.mtime);
                }
                if resp.clicked() {
                    if *offline {
                        self.say(ui.ctx(), tr("硬盘未连接，接上后会自动恢复").to_string(), None);
                    } else if !*root && ui.input(|i| i.modifiers.command) {
                        self.toggle_dir(path);
                    } else {
                        self.cwd = Some(path.clone());
                        self.clear_selection();
                        out.clear_search = true;
                    }
                }
            }
            Row::Video { id, with_path } => {
                let Some(v) = d.videos.iter().find(|v| v.id == *id) else { return };
                let on = self.selected.contains(id);
                if on {
                    p.rect_filled(r, 0.0, theme::accent_alpha(26));
                } else if resp.hovered() {
                    p.rect_filled(r, 0.0, theme::tint(6));
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                let boxed = Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), vec2(26.0, 30.0));
                let tick = ui.interact(boxed, ui.scope_id().with(("tick", *id)), Sense::click());
                draw_check(&p, boxed.center(), on, tick.hovered());
                if tick.clicked() {
                    self.toggle(*id);
                }
                let thumb = Rect::from_min_size(pos2(r.left() + THUMB_X, r.center().y - 14.0), vec2(46.0, 28.0));
                self.draw_thumb(&p, thumb, *id, d);
                let parent = Path::new(&v.path).parent().map(|x| x.to_string_lossy().into_owned());
                let sub = parent.filter(|_| *with_path);
                self.row_text(&p, r, &v.title, sub.as_deref());
                self.row_columns(&p, r, right, v.duration, v.size, v.mtime);
                if resp.clicked() {
                    if ui.input(|i| i.modifiers.command) {
                        self.toggle(*id);
                    } else {
                        out.play = Some(v.path.clone());
                    }
                }
                resp.context_menu(|ui| {
                    if ui.button(tr("播放")).clicked() {
                        out.play = Some(v.path.clone());
                        ui.close();
                    }
                    if ui.button(tr("在文件管理器中显示")).clicked() {
                        reveal_in_file_manager(&v.path);
                        ui.close();
                    }
                    if ui.button(tr("重命名")).clicked() {
                        let stem = Path::new(&v.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        self.dialog = Some(Dialog::Rename { id: *id, stem, error: None });
                        ui.close();
                    }
                    if ui.button(tr("剪切到…")).clicked() {
                        self.open_move(vec![*id], Vec::new(), d);
                        ui.close();
                    }
                    if ui.button(tr("移到废纸篓")).clicked() {
                        self.dialog = Some(Dialog::Trash { ids: vec![*id] });
                        ui.close();
                    }
                });
            }
        }
    }

    fn draw_thumb(&mut self, p: &egui::Painter, r: Rect, id: i64, d: &Data) {
        self.visible.push(id);
        match d.thumbs.get(&id) {
            Some(texture) => {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                p.image(*texture, r, uv, Color32::WHITE);
            }
            None => {
                p.rect_filled(r, 3.0, placeholder_color(id));
                crate::icons::draw(p, crate::icons::Icon::Play, r.center(), 10.0, Color32::from_white_alpha(170));
            }
        }
    }

    /// The thumbnails to load for the next frame.
    pub fn visible(&self) -> Vec<i64> {
        self.visible.clone()
    }

    fn toggle_dir(&mut self, path: &Path) {
        if !self.selected_dirs.remove(path) {
            self.selected_dirs.insert(path.to_path_buf());
        }
    }

    fn toggle(&mut self, id: i64) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
    }

    fn row_text(&self, p: &egui::Painter, r: Rect, name: &str, sub: Option<&str>) {
        let width = r.width() - TEXT_X - COL_DURATION - COL_SIZE - COL_DATE - 24.0;
        let x = r.left() + TEXT_X;
        let title = elide(p, name, FontId::proportional(13.0), width);
        match sub {
            Some(sub) => {
                p.text(pos2(x, r.center().y - 8.0), Align2::LEFT_CENTER, title, FontId::proportional(13.0), theme::text());
                let text = elide(p, sub, FontId::proportional(11.0), width);
                p.text(pos2(x, r.center().y + 9.0), Align2::LEFT_CENTER, text, FontId::proportional(11.0), theme::text_2());
            }
            None => {
                p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, title, FontId::proportional(13.0), theme::text());
            }
        }
    }

    fn row_columns(&self, p: &egui::Painter, r: Rect, right: f32, duration: f64, size: i64, mtime: i64) {
        let y = r.center().y;
        let font = FontId::monospace(11.0);
        let time = if duration > 0.0 { fmt_time(duration) } else { "—".to_string() };
        p.text(pos2(right - COL_DATE - COL_SIZE, y), Align2::RIGHT_CENTER, time, font.clone(), theme::text_2());
        p.text(pos2(right - COL_DATE, y), Align2::RIGHT_CENTER, format_size(size), font.clone(), theme::text_2());
        p.text(pos2(right, y), Align2::RIGHT_CENTER, format_date(mtime), font, theme::text_2());
    }

    fn batch_bar(&mut self, ui: &mut egui::Ui, p: &egui::Painter, bar: Rect, d: &Data) {
        p.rect_filled(bar, 8.0, theme::panel());
        p.rect_stroke(bar, 8.0, Stroke::new(0.5, ACCENT), StrokeKind::Inside);
        let size: i64 = d.videos.iter().filter(|v| self.selected.contains(&v.id)).map(|v| v.size).sum();
        let dirs = self.selected_dirs.len();
        let text = if dirs > 0 {
            tf("已选 {} 个文件夹，{} 个视频", &[&dirs, &self.selected.len()])
        } else {
            tf("已选 {} 个 · {}", &[&self.selected.len(), &format_size(size)])
        };
        p.text(pos2(bar.left() + 12.0, bar.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), theme::text());
        let mut x = bar.right() - 8.0;
        if button(ui, p, &mut x, bar.center().y, "sel_clear", tr("取消选择"), false) {
            self.clear_selection();
        }
        let ids: Vec<i64> = self.selected.iter().copied().collect();
        if dirs == 0 {
            if button(ui, p, &mut x, bar.center().y, "sel_trash", tr("移到废纸篓"), false) {
                self.dialog = Some(Dialog::Trash { ids: ids.clone() });
            }
            if button(ui, p, &mut x, bar.center().y, "sel_collect", tr("加入收藏"), false) {
                self.dialog = Some(Dialog::Collect { ids: ids.clone(), picked: Vec::new() });
            }
            if button(ui, p, &mut x, bar.center().y, "sel_tag", tr("加标签"), false) {
                self.dialog = Some(Dialog::Tag { ids: ids.clone(), name: String::new(), error: None });
            }
        }
        if button(ui, p, &mut x, bar.center().y, "sel_move", tr("剪切到…"), true) {
            let folders = self.selected_dirs.iter().cloned().collect();
            self.open_move(ids, folders, d);
        }
    }

    fn open_move(&mut self, ids: Vec<i64>, folders: Vec<PathBuf>, d: &Data) {
        let mut dirs = Vec::new();
        for folder in d.folders.iter().filter(|f| !self.disks.offline.contains(&f.id)) {
            walk(folder.id, Path::new(&folder.path), 0, &mut dirs);
        }
        self.dialog = Some(Dialog::Move { ids, folders, dirs, filter: String::new(), pick: None, new_name: String::new(), error: None });
    }

    // ---- Duplicates ----------------------------------------------------

    fn refresh_groups(&mut self, ctx: &egui::Context, d: &Data) {
        if let Some(rx) = &self.searching {
            match rx.try_recv() {
                Ok(groups) => {
                    self.groups = groups;
                    self.searching = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.searching = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after_secs(0.05),
            }
        }
        let roots = self.root_ids(d.folders);
        let wanted = (self.options, d.revision, self.set, roots.len());
        if self.searched == Some(wanted) {
            return;
        }
        self.searched = Some(wanted);
        let items: Vec<Item> = d
            .videos
            .iter()
            .filter(|v| roots.contains(&v.folder_id))
            .map(|v| Item { id: v.id, name: v.title.clone(), duration: v.duration, size: v.size })
            .collect();
        let options = self.options;
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        let started = std::thread::Builder::new().name("duplicates".into()).spawn(move || {
            let _ = tx.send(duplicates::find(&items, options));
            wake.request_repaint();
        });
        self.searching = started.is_ok().then_some(rx);
    }

    fn duplicates_page(&mut self, ui: &mut egui::Ui, p: &egui::Painter, rect: Rect, d: &Data, out: &mut Outcome) {
        self.refresh_groups(ui.ctx(), d);
        if self.ignored.is_none() {
            self.ignored = Some(d.lib.ignored_duplicates());
        }
        let ignored = self.ignored.clone().unwrap_or_default();
        let groups: Vec<Group> = self.groups.iter().filter(|g| !ignored.contains(&g.key())).cloned().collect();
        let rect = rect.shrink2(vec2(16.0, 0.0));
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
        let marked: Vec<&Video> = d.videos.iter().filter(|v| self.marked.contains(&v.id)).collect();
        let size: i64 = marked.iter().map(|v| v.size).sum();
        let summary = if self.searching.is_some() {
            tr("正在比较…").to_string()
        } else {
            tf("{} 组可能重复，已标记 {} 个，可释放 {}", &[&groups.len(), &marked.len(), &format_size(size)])
        };
        p.text(pos2(head.left(), head.center().y), Align2::LEFT_CENTER, summary, FontId::proportional(13.0), theme::text_2());
        let mut x = head.right();
        if button(ui, p, &mut x, head.center().y, "dup_apply", tr("处理已标记"), true) {
            if marked.is_empty() {
                self.say(ui.ctx(), tr("先把要处理的视频标记为移到废纸篓").to_string(), None);
            } else {
                self.dialog = Some(Dialog::Trash { ids: marked.iter().map(|v| v.id).collect() });
            }
        }

        let bar = Rect::from_min_size(pos2(rect.left(), head.bottom()), vec2(rect.width(), 30.0));
        self.duplicate_filters(ui, p, bar, &groups);

        let list = Rect::from_min_max(pos2(rect.left(), bar.bottom() + 6.0), rect.max);
        let chosen = self.options.duration || self.options.size || self.options.name;
        let shown: Vec<&Group> = groups
            .iter()
            .filter(|g| match self.show {
                Show::All => true,
                Show::Strong => g.strong(),
                Show::Weak => !g.strong(),
            })
            .take(MAX_GROUPS_SHOWN)
            .collect();
        if !chosen || (shown.is_empty() && self.searching.is_none()) {
            let text = if chosen { tr("没有发现可能重复的视频") } else { tr("至少选一个比较条件") };
            p.text(list.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), theme::text_3());
            return;
        }
        let shown: Vec<Group> = shown.into_iter().cloned().collect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().id_salt("dup_list").auto_shrink(false).show(ui, |ui| {
                for group in &shown {
                    self.draw_group(ui, group, d, out);
                }
            });
        });
    }

    fn duplicate_filters(&mut self, ui: &mut egui::Ui, p: &egui::Painter, bar: Rect, groups: &[Group]) {
        let y = bar.center().y;
        let mut x = bar.left();
        p.text(pos2(x, y), Align2::LEFT_CENTER, tr("比较"), FontId::proportional(12.0), theme::text_2());
        x += 34.0;
        for (i, label) in [tr("时长"), tr("大小"), tr("名称")].into_iter().enumerate() {
            let flag = match i {
                0 => &mut self.options.duration,
                1 => &mut self.options.size,
                _ => &mut self.options.name,
            };
            if chip(ui, p, &mut x, y, ("dup_use", i), label, *flag) {
                *flag = !*flag;
            }
        }
        x += 12.0;
        for (level, label) in LEVELS {
            if chip(ui, p, &mut x, y, ("dup_level", label), tr(label), self.options.level == level) {
                self.options.level = level;
            }
        }
        x += 12.0;
        let strong = groups.iter().filter(|g| g.strong()).count();
        let filters = [
            (Show::All, tf("全部 {}", &[&groups.len()])),
            (Show::Strong, tf("很可能 {}", &[&strong])),
            (Show::Weak, tf("可能 {}", &[&(groups.len() - strong)])),
        ];
        for (show, label) in filters {
            if chip(ui, p, &mut x, y, ("dup_show", label.as_str()), &label, self.show == show) {
                self.show = show;
            }
        }
    }

    fn draw_group(&mut self, ui: &mut egui::Ui, group: &Group, d: &Data, out: &mut Outcome) {
        let width = ui.available_width();
        let mut videos: Vec<&Video> = group.ids.iter().filter_map(|id| d.videos.iter().find(|v| v.id == *id)).collect();
        videos.sort_by_key(|v| std::cmp::Reverse(v.size));
        let (head, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
        let p = ui.painter_at(head);
        p.rect_filled(head, egui::CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, theme::panel());
        let badge = if group.strong() { tr("很可能重复") } else { tr("可能重复") };
        let color = if group.strong() { Color32::from_rgb(0xF6, 0xD9, 0x8B) } else { theme::text_2() };
        let g = p.text(pos2(head.left() + 10.0, head.center().y), Align2::LEFT_CENTER, badge, FontId::proportional(12.0), color);
        let mut reasons = Vec::new();
        if group.reasons.duration {
            reasons.push(tr("时长接近"));
        }
        if group.reasons.size {
            reasons.push(tr("大小接近"));
        }
        if group.reasons.name {
            reasons.push(tr("名称相似"));
        }
        p.text(pos2(g.right() + 10.0, head.center().y), Align2::LEFT_CENTER, reasons.join(" · "), FontId::proportional(11.0), ACCENT);
        let mut x = head.right() - 8.0;
        if button(ui, &p, &mut x, head.center().y, &format!("dup_no_{}", group.key()), tr("不是重复"), false) {
            let key = group.key();
            d.lib.ignore_duplicate(&key, true);
            self.ignored = None;
            self.say(ui.ctx(), tr("已记住：这组不是重复").to_string(), Some(Undo::Ignored(key)));
        }

        for v in &videos {
            let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            let p = ui.painter_at(r);
            p.line_segment([r.left_top(), r.right_top()], Stroke::new(0.5, theme::line()));
            if resp.hovered() {
                p.rect_filled(r, 0.0, theme::tint(6));
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let thumb = Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 14.0), vec2(46.0, 28.0));
            self.draw_thumb(&p, thumb, v.id, d);
            let text_w = r.width() - 296.0;
            let title = elide(&p, &base_name(&v.path), FontId::proportional(13.0), text_w);
            p.text(pos2(r.left() + 66.0, r.center().y - 8.0), Align2::LEFT_CENTER, title, FontId::proportional(13.0), theme::text());
            let dir = Path::new(&v.path).parent().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            p.text(pos2(r.left() + 66.0, r.center().y + 9.0), Align2::LEFT_CENTER, elide(&p, &dir, FontId::proportional(11.0), text_w), FontId::proportional(11.0), theme::text_2());
            let font = FontId::monospace(11.0);
            let right = r.right() - 110.0;
            let time = if v.duration > 0.0 { fmt_time(v.duration) } else { "—".to_string() };
            p.text(pos2(right - 70.0, r.center().y), Align2::RIGHT_CENTER, time, font.clone(), theme::text_2());
            p.text(pos2(right, r.center().y), Align2::RIGHT_CENTER, format_size(v.size), font, theme::text_2());

            let on = self.marked.contains(&v.id);
            let label = if on { tr("移到废纸篓") } else { tr("保留") };
            let mut x = r.right() - 8.0;
            if toggle_button(ui, &p, &mut x, r.center().y, ("dup_mark", v.id), label, on) {
                let kept = videos.iter().filter(|o| !self.marked.contains(&o.id)).count();
                if on {
                    self.marked.remove(&v.id);
                } else if kept > 1 {
                    self.marked.insert(v.id);
                } else {
                    self.say(ui.ctx(), tr("每组至少保留一个视频").to_string(), None);
                }
            } else if resp.clicked() {
                out.play = Some(v.path.clone());
            }
        }
        ui.add_space(10.0);
    }

    // ---- Dialogs -------------------------------------------------------

    fn dialogs(&mut self, ui: &mut egui::Ui, d: &Data) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let ctx = ui.ctx().clone();
        let mut close = false;
        egui::Modal::new(ui.scope_id().with("manage_dialog")).show(&ctx, |ui| {
            ui.set_width(420.0);
            match &mut dialog {
                Dialog::NewSet { name, picked, error } => {
                    ui.label(tr("新建文件夹集合"));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(name).hint_text(tr("集合名称，比如 工作素材")).desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for f in d.folders {
                            let mut on = picked.contains(&f.id);
                            if ui.checkbox(&mut on, f.path.as_str()).changed() {
                                if on {
                                    picked.push(f.id);
                                } else {
                                    picked.retain(|x| *x != f.id);
                                }
                            }
                        }
                    });
                    if let Some(e) = error {
                        ui.colored_label(MARKED, tr(e));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        if ui.button(tr("创建集合")).clicked() {
                            if name.trim().is_empty() {
                                *error = Some("输入集合名称");
                            } else if picked.is_empty() {
                                *error = Some("至少选一个文件夹");
                            } else {
                                self.set = d.lib.create_folder_set(name.trim(), picked);
                                self.sets_dirty = true;
                                self.page = Page::Videos;
                                self.cwd = None;
                                close = true;
                            }
                        }
                    });
                }
                Dialog::Move { ids, folders, dirs, filter, pick, new_name, error } => {
                    ui.label(tf("剪切 {} 项到", &[&(ids.len() + folders.len())]));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(filter).hint_text(tr("搜索目标文件夹")).desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    let needle = filter.to_lowercase();
                    egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                        for (i, dir) in dirs.iter().enumerate() {
                            let text = dir.path.to_string_lossy();
                            if !text.to_lowercase().contains(&needle) {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.add_space(dir.depth as f32 * 14.0);
                                if ui.selectable_label(*pick == Some(i), base_name(&text)).clicked() {
                                    *pick = Some(i);
                                    *error = None;
                                }
                            });
                        }
                    });
                    let chosen: Vec<&Video> = d.videos.iter().filter(|v| ids.contains(&v.id)).collect();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(new_name).hint_text(tr("新文件夹名称")).desired_width(280.0));
                        if ui.button(tr("新建文件夹")).clicked() {
                            match pick.and_then(|i| dirs.get(i)).map(|dir| (dir.root, dir.depth, dir.path.clone())) {
                                None => *error = Some("选一个目标文件夹"),
                                Some(_) if !valid_name(new_name) => *error = Some("名称不能为空，也不能包含特殊字符"),
                                Some((root, depth, parent)) => {
                                    let path = parent.join(new_name.trim());
                                    if path.exists() {
                                        *error = Some("已有同名文件或文件夹");
                                    } else if std::fs::create_dir(&path).is_ok() {
                                        let at = pick.map_or(dirs.len(), |i| i + 1);
                                        dirs.insert(at, Dir { root, path, depth: depth + 1 });
                                        *pick = Some(at);
                                        new_name.clear();
                                        *error = None;
                                    } else {
                                        *error = Some("无法在这里新建文件夹");
                                    }
                                }
                            }
                        }
                    });
                    if let Some(dir) = pick.and_then(|i| dirs.get(i)) {
                        ui.add_space(6.0);
                        ui.label(dir.path.to_string_lossy());
                        let other_disk = chosen.iter().any(|v| !file_move::same_disk(Path::new(&v.path), &dir.path))
                            || folders.iter().any(|f| !file_move::same_disk(f, &dir.path));
                        if other_disk {
                            ui.colored_label(theme::text_2(), tr("目标在另一块磁盘，要先复制并校验，再删除原文件"));
                        }
                        if chosen.iter().any(|v| dir.path.join(base_name(&v.path)).exists() && Path::new(&v.path).parent() != Some(dir.path.as_path())) {
                            ui.colored_label(theme::text_2(), tr("目标已有同名文件，将保留两者并自动改名"));
                        }
                    }
                    if let Some(e) = error {
                        ui.colored_label(MARKED, tr(e));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        if ui.button(tr("剪切到这里")).clicked() {
                            match pick.and_then(|i| dirs.get(i)) {
                                None => *error = Some("选一个目标文件夹"),
                                Some(dir) if folders.iter().any(|f| dir.path.starts_with(f)) => {
                                    *error = Some("不能把文件夹剪切到它自己里面");
                                }
                                Some(dir) => {
                                    let mut jobs: Vec<Task> = folders
                                        .iter()
                                        .filter(|f| f.parent() != Some(dir.path.as_path()))
                                        .map(|f| Task::Folder(FolderJob { from: f.clone(), to_dir: dir.path.clone(), folder_id: dir.root }))
                                        .collect();
                                    jobs.extend(
                                        chosen
                                            .iter()
                                            .filter(|v| Path::new(&v.path).parent() != Some(dir.path.as_path()))
                                            .filter(|v| !folders.iter().any(|f| Path::new(&v.path).starts_with(f)))
                                            .map(|v| Task::File(Job { id: v.id, from: PathBuf::from(&v.path), to_dir: dir.path.clone(), folder_id: dir.root })),
                                    );
                                    if jobs.is_empty() {
                                        *error = Some("视频已经在这个文件夹里");
                                    } else {
                                        self.start_moves(&ctx, jobs, d.videos);
                                        close = true;
                                    }
                                }
                            }
                        }
                    });
                }
                Dialog::Trash { ids } => {
                    let chosen: Vec<&Video> = d.videos.iter().filter(|v| ids.contains(&v.id)).collect();
                    let size: i64 = chosen.iter().map(|v| v.size).sum();
                    ui.label(tf("把 {} 个视频移到废纸篓？", &[&chosen.len()]));
                    ui.add_space(4.0);
                    ui.colored_label(theme::text_2(), tf("共 {}。可以从系统废纸篓恢复，清空废纸篓后才释放空间。", &[&format_size(size)]));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        if ui.button(tr("移到废纸篓")).clicked() {
                            let mut failed = 0;
                            for v in &chosen {
                                if to_trash(&v.path) {
                                    d.lib.remove_video(v.id);
                                    self.marked.remove(&v.id);
                                    self.selected.remove(&v.id);
                                } else {
                                    failed += 1;
                                }
                            }
                            let moved = chosen.len() - failed;
                            let text = if failed > 0 {
                                tf("已移到废纸篓 {} 个，{} 个失败", &[&moved, &failed])
                            } else {
                                tf("已将 {} 个视频移到废纸篓", &[&moved])
                            };
                            self.say(&ctx, text, None);
                            self.changed = true;
                            close = true;
                        }
                    });
                }
                Dialog::Rename { id, stem, error } => {
                    ui.label(tr("重命名"));
                    ui.add_space(6.0);
                    let edit = ui.add(egui::TextEdit::singleline(stem).desired_width(f32::INFINITY));
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.colored_label(theme::text_2(), tr("同时修改磁盘上的文件，进度、收藏和标签都保留"));
                    if let Some(e) = error {
                        ui.colored_label(MARKED, tr(e));
                    }
                    ui.add_space(6.0);
                    let mut go = enter;
                    ui.horizontal(|ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        go |= ui.button(tr("重命名")).clicked();
                    });
                    if let (true, Some(v)) = (go, d.videos.iter().find(|v| v.id == *id)) {
                        match rename_video(Path::new(&v.path), stem) {
                            Err(e) => *error = Some(e),
                            Ok(to) => {
                                d.lib.record_move(*id, &to, v.folder_id);
                                self.changed = true;
                                close = true;
                            }
                        }
                    }
                }
                Dialog::Tag { ids, name, error } => {
                    ui.label(tf("给 {} 个视频加标签", &[&ids.len()]));
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(name).hint_text(tr("标签名称")).desired_width(f32::INFINITY));
                    if let Some(e) = error {
                        ui.colored_label(MARKED, tr(e));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        if ui.button(tr("加标签")).clicked() {
                            if name.trim().is_empty() {
                                *error = Some("输入标签名称");
                            } else {
                                for id in ids.iter() {
                                    d.lib.add_tag(*id, name.trim());
                                }
                                self.say(&ctx, tf("已给 {} 个视频加标签", &[&ids.len()]), None);
                                self.selected.clear();
                                self.changed = true;
                                close = true;
                            }
                        }
                    });
                }
                Dialog::Collect { ids, picked } => {
                    ui.label(tf("把 {} 个视频加入收藏", &[&ids.len()]));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for c in d.collections {
                            let mut on = picked.contains(&c.id);
                            if ui.checkbox(&mut on, c.name.as_str()).changed() {
                                if on {
                                    picked.push(c.id);
                                } else {
                                    picked.retain(|x| *x != c.id);
                                }
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        if ui.button(tr("加入收藏")).clicked() && !picked.is_empty() {
                            for collection in picked.iter() {
                                for id in ids.iter() {
                                    d.lib.set_in_collection(*collection, *id, true);
                                }
                            }
                            self.say(&ctx, tf("已把 {} 个视频加入收藏", &[&ids.len()]), None);
                            self.selected.clear();
                            self.changed = true;
                            close = true;
                        }
                    });
                }
            }
        });
        // Only Esc: the click that opened a dialog would count as one outside.
        if !close && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = Some(dialog);
        }
    }

    fn progress_dialog(&mut self, ui: &mut egui::Ui) {
        if self.mover.is_none() {
            return;
        }
        let ctx = ui.ctx().clone();
        let copying = self.progress.total > 0;
        let fraction = if copying { self.progress.done as f32 / self.progress.total as f32 } else { 0.0 };
        egui::Modal::new(ui.scope_id().with("manage_progress")).show(&ctx, |ui| {
            ui.set_width(380.0);
            let index = (self.progress.moved + self.progress.failed + 1).min(self.progress.jobs.max(1));
            ui.label(tf("正在剪切 {} / {}", &[&index, &self.progress.jobs]));
            ui.colored_label(theme::text_2(), self.progress.name.as_str());
            ui.add_space(6.0);
            ui.add(egui::ProgressBar::new(fraction).desired_height(6.0));
            if self.progress.verifying {
                ui.colored_label(theme::text_2(), tr("正在校验复制结果"));
            }
            ui.add_space(6.0);
            if ui.button(tr("取消")).clicked() {
                if let Some(mover) = &self.mover {
                    mover.cancel();
                }
            }
        });
    }

    fn draw_toast(&mut self, ui: &mut egui::Ui, p: &egui::Painter, rect: Rect, d: &Data) {
        let Some(toast) = &self.toast else { return };
        let now = ui.input(|i| i.time);
        if now - toast.at > TOAST_SECS {
            self.toast = None;
            self.undo = None;
            return;
        }
        ui.ctx().request_repaint_after_secs((TOAST_SECS - (now - toast.at)) as f32);
        let font = FontId::proportional(13.0);
        let undo = tr("撤销");
        let text_w = p.layout_no_wrap(toast.text.clone(), font.clone(), theme::text()).size().x;
        let undo_w = if toast.undo { p.layout_no_wrap(undo.to_string(), font.clone(), ACCENT).size().x + 14.0 } else { 0.0 };
        let r = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 74.0), vec2(text_w + undo_w + 28.0, 32.0));
        p.rect_filled(r, 7.0, theme::panel());
        p.rect_stroke(r, 7.0, Stroke::new(0.5, theme::line()), StrokeKind::Inside);
        p.text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, &toast.text, font.clone(), theme::text());
        if toast.undo {
            let area = Rect::from_min_max(pos2(r.right() - undo_w - 6.0, r.top()), r.max);
            let resp = ui.interact(area, ui.scope_id().with("toast_undo"), Sense::click());
            p.text(pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER, undo, font, ACCENT);
            if resp.clicked() {
                self.undo_last(ui.ctx(), d);
            }
        }
    }

    /// True once after a dialog changed the library.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

fn column(r: Rect, right: f32, width: f32) -> Rect {
    Rect::from_min_max(pos2(right - width, r.top()), pos2(right, r.bottom()))
}

fn walk(root: i64, dir: &Path, depth: usize, out: &mut Vec<Dir>) {
    if out.len() >= MAX_PICKER_DIRS {
        return;
    }
    out.push(Dir { root, path: dir.to_path_buf(), depth });
    if depth >= MAX_PICKER_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    subs.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    for sub in subs {
        walk(root, &sub, depth + 1, out);
    }
}

/// Names that work on both Windows and macOS.
fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.starts_with('.') && !name.chars().any(|c| "/\\:*?\"<>|".contains(c) || c.is_control())
}

/// Renames a file, keeping its extension. `stem` is the new name without it.
fn rename_video(from: &Path, stem: &str) -> Result<PathBuf, &'static str> {
    if !valid_name(stem) {
        return Err("名称不能为空，也不能包含特殊字符");
    }
    let mut name = stem.trim().to_string();
    if let Some(ext) = from.extension() {
        name = format!("{name}.{}", ext.to_string_lossy());
    }
    let to = from.with_file_name(name);
    if to == from {
        return Ok(to);
    }
    // On a case-insensitive disk, a new case of the same name exists already.
    let same_file = matches!((std::fs::canonicalize(from), std::fs::canonicalize(&to)), (Ok(a), Ok(b)) if a == b);
    if to.exists() && !same_file {
        return Err("已有同名文件或文件夹");
    }
    std::fs::rename(from, &to).map_err(|_| "无法重命名这个文件")?;
    Ok(to)
}

fn to_trash(path: &str) -> bool {
    #[allow(unused_mut)]
    let mut context = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        // The default goes through Finder, which asks for a permission first.
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        context.set_delete_method(DeleteMethod::NsFileManager);
    }
    context.delete(path).is_ok()
}

fn format_size(bytes: i64) -> String {
    const GB: f64 = 1e9;
    const MB: f64 = 1e6;
    let b = bytes as f64;
    if b >= GB { format!("{:.1} GB", b / GB) } else { format!("{:.0} MB", b / MB) }
}

/// A file time as a date, in UTC.
fn format_date(secs: i64) -> String {
    if secs <= 0 {
        return "—".to_string();
    }
    // Howard Hinnant's days-to-civil algorithm.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02}")
}

fn draw_folder_icon(p: &egui::Painter, r: Rect) {
    let color = Color32::from_rgb(0x8F, 0xC0, 0xF8);
    let body = Rect::from_min_max(r.min + vec2(3.0, 4.0), r.max - vec2(3.0, 2.0));
    let tab = Rect::from_min_size(r.min + vec2(3.0, 1.0), vec2(8.0, 4.0));
    p.rect_filled(tab, 1.0, color);
    p.rect_filled(body, 2.0, color);
}

fn draw_check(p: &egui::Painter, c: egui::Pos2, on: bool, hovered: bool) {
    let r = Rect::from_center_size(c, vec2(15.0, 15.0));
    if on {
        p.rect_filled(r, 3.0, ACCENT);
        let points = [c + vec2(-3.5, 0.0), c + vec2(-1.0, 3.0), c + vec2(4.0, -3.0)];
        p.line_segment([points[0], points[1]], Stroke::new(1.6, Color32::WHITE));
        p.line_segment([points[1], points[2]], Stroke::new(1.6, Color32::WHITE));
    } else {
        p.rect_stroke(r, 3.0, Stroke::new(1.0, if hovered { ACCENT } else { theme::text_3() }), StrokeKind::Inside);
    }
}

/// Places a button leftward from `x`, which moves past it.
fn button(ui: &mut egui::Ui, p: &egui::Painter, x: &mut f32, y: f32, id: &str, text: &str, primary: bool) -> bool {
    let width = p.layout_no_wrap(text.to_string(), FontId::proportional(12.0), theme::text()).size().x + 20.0;
    let r = Rect::from_min_size(pos2(*x - width, y - 13.0), vec2(width, 26.0));
    *x -= width + 8.0;
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    if primary {
        p.rect_filled(r, 6.0, ACCENT);
    } else {
        p.rect_stroke(r, 6.0, Stroke::new(0.5, if resp.hovered() { ACCENT } else { theme::line() }), StrokeKind::Inside);
    }
    let color = if primary { Color32::WHITE } else { theme::text() };
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.0), color);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn toggle_button(ui: &mut egui::Ui, p: &egui::Painter, x: &mut f32, y: f32, id: impl std::hash::Hash + std::fmt::Debug, text: &str, marked: bool) -> bool {
    let r = Rect::from_min_size(pos2(*x - 88.0, y - 12.0), vec2(88.0, 24.0));
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    let color = if marked { MARKED } else { ACCENT };
    p.rect_stroke(r, 5.0, Stroke::new(0.5, color), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.0), color);
    resp.clicked()
}

/// Places a pill rightward from `x`, which moves past it.
fn chip(ui: &mut egui::Ui, p: &egui::Painter, x: &mut f32, y: f32, id: impl std::hash::Hash + std::fmt::Debug, text: &str, on: bool) -> bool {
    let width = p.layout_no_wrap(text.to_string(), FontId::proportional(12.0), theme::text()).size().x + 18.0;
    let r = Rect::from_min_size(pos2(*x, y - 12.0), vec2(width, 24.0));
    *x += width + 6.0;
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    if on {
        p.rect_filled(r, 12.0, theme::accent_alpha(36));
    }
    p.rect_stroke(r, 12.0, Stroke::new(0.5, if on || resp.hovered() { ACCENT } else { theme::line() }), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.0), if on { ACCENT } else { theme::text_2() });
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_work_on_every_system() {
        assert!(valid_name("第 2 集"));
        assert!(!valid_name("  "));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("a:b"));
        assert!(!valid_name(".hidden"));
    }

    #[test]
    fn renaming_keeps_the_extension_and_refuses_a_clash() {
        let dir = std::env::temp_dir().join(format!("tinyplayer-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("a.mp4");
        std::fs::write(&from, b"x").unwrap();
        std::fs::write(dir.join("b.mp4"), b"y").unwrap();
        assert_eq!(rename_video(&from, "b"), Err("已有同名文件或文件夹"));
        assert_eq!(rename_video(&from, "a"), Ok(from.clone()));
        let to = rename_video(&from, "c").unwrap();
        assert_eq!(to, dir.join("c.mp4"));
        assert!(to.exists() && !from.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dates_read_in_utc() {
        assert_eq!(format_date(0), "—");
        assert_eq!(format_date(1), "1970-01-01");
        assert_eq!(format_date(1_709_164_800), "2024-02-29");
        assert_eq!(format_date(1_735_689_599), "2024-12-31");
    }

    #[test]
    fn sizes_pick_a_readable_unit() {
        assert_eq!(format_size(58_200_000_000), "58.2 GB");
        assert_eq!(format_size(350_000_000), "350 MB");
    }
}

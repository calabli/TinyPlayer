//! Library home screen: search, nav (views / tags / folders), and a
//! virtualized thumbnail grid. Shown whenever nothing is playing.

use std::collections::{HashMap, HashSet};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2,
};

use crate::app::fmt_time;
use crate::icons::{self, Icon};
use crate::library::{Folder, Library, Video};
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
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Added,
    Name,
    Duration,
    Played,
}

pub enum Action {
    None,
    Play(String),
    AddFolder,
    OpenFile,
    Settings,
}

pub struct LibraryView {
    lib: Option<Library>,
    videos: Vec<Video>,
    folders: Vec<Folder>,
    dirty: bool,
    view: View,
    sort: Sort,
    search: String,
    tags_sel: Vec<String>,
    folder_sel: Option<i64>,
    thumbs: HashMap<i64, (String, TextureHandle)>,
    thumb_failed: HashSet<i64>,
    /// (video id, text being typed, focus requested yet)
    tag_edit: Option<(i64, String, bool)>,
    tag_edit_pos: egui::Pos2,
    /// Thumbnail generation progress (done, total).
    pub probing: Option<(usize, usize)>,
}

impl LibraryView {
    pub fn new() -> Self {
        Self {
            lib: Library::open(&Library::db_path()).ok(),
            videos: Vec::new(),
            folders: Vec::new(),
            dirty: true,
            view: View::All,
            sort: Sort::Added,
            search: String::new(),
            tags_sel: Vec::new(),
            folder_sel: None,
            thumbs: HashMap::new(),
            thumb_failed: HashSet::new(),
            tag_edit: None,
            tag_edit_pos: pos2(0.0, 0.0),
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

    fn reload_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(lib) = &self.lib {
            self.videos = lib.videos();
            self.folders = lib.folders();
        }
        // Thumbnails may have been regenerated for changed files.
        let current: HashMap<i64, Option<&String>> = self.videos.iter().map(|v| (v.id, v.thumb.as_ref())).collect();
        self.thumbs.retain(|id, (path, _)| current.get(id).copied().flatten() == Some(&*path));
        self.thumb_failed.retain(|id| current.contains_key(id));
        if self.folder_sel.is_some_and(|f| !self.folders.iter().any(|x| x.id == f)) {
            self.folder_sel = None;
        }
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
            })
            .filter(|v| self.folder_sel.is_none_or(|f| v.folder_id == f))
            .filter(|v| self.tags_sel.iter().all(|t| v.tags.iter().any(|vt| vt.eq_ignore_ascii_case(t))))
            .filter(|v| {
                if words.is_empty() {
                    return true;
                }
                let hay = format!("{} {} {}", v.title, v.tags.join(" "), v.path).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        match (self.view, self.sort) {
            (View::Continue, _) | (_, Sort::Played) => {
                out.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(b.added_at.cmp(&a.added_at)))
            }
            (_, Sort::Added) => out.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(b.id.cmp(&a.id))),
            (_, Sort::Name) => out.sort_by(|a, b| natural_cmp(&a.title, &b.title)),
            (_, Sort::Duration) => out.sort_by(|a, b| b.duration.total_cmp(&a.duration)),
        }
        out
    }

    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect) -> Action {
        self.reload_if_dirty();
        let mut action = Action::None;
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, theme::BG);

        // ---- Top bar ----
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP_H));
        p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(0.5, LINE));
        p.text(pos2(top.left() + 20.0, top.center().y), Align2::LEFT_CENTER, "TINYPLAYER", FontId::monospace(12.0), ACCENT);

        let search_w = (top.width() - NAV_W - 380.0).clamp(160.0, 420.0);
        let search_rect = Rect::from_min_size(pos2(top.left() + NAV_W + 16.0, top.center().y - 15.0), vec2(search_w, 30.0));
        p.rect_filled(search_rect, 15.0, PANEL);
        p.rect_stroke(search_rect, 15.0, Stroke::new(0.5, LINE), StrokeKind::Inside);
        search_icon(&p, pos2(search_rect.left() + 16.0, search_rect.center().y), TEXT_3);
        let edit_rect = Rect::from_min_max(search_rect.min + vec2(30.0, 5.0), search_rect.max - vec2(12.0, 5.0));
        let mut search_resp = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("搜索标题、标签、文件夹")
                    .frame(egui::Frame::NONE)
                    .text_color(TEXT)
                    .font(FontId::proportional(13.0))
                    .desired_width(edit_rect.width()),
            );
            search_resp = Some(r);
        });
        let focus_search = ui.ctx().input(|i| i.modifiers.command && i.key_pressed(egui::Key::K));
        if let Some(r) = &search_resp {
            if focus_search {
                r.request_focus();
            }
            // TextEdit drops focus on Esc itself, so check lost_focus too.
            if (r.has_focus() || r.lost_focus()) && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                self.search.clear();
                r.surrender_focus();
            }
        }

        // Right side of the top bar.
        let mut rx = top.right() - 20.0;
        let gear = Rect::from_center_size(pos2(rx - 12.0, top.center().y), vec2(30.0, 30.0));
        let g = ui.interact(gear, ui.scope_id().with("lib_settings"), Sense::click());
        icons::draw(&p, Icon::Settings, gear.center(), 16.0, if g.hovered() { ACCENT } else { TEXT_2 });
        if g.clicked() {
            action = Action::Settings;
        }
        rx -= 40.0;
        let (open_left, open_clicked) = text_btn(ui, &p, pos2(rx, top.center().y), "lib_open", "打开文件", TEXT_2);
        if open_clicked {
            action = Action::OpenFile;
        }
        let (add_left, add_clicked) = text_btn(ui, &p, pos2(open_left - 24.0, top.center().y), "lib_add", "+ 添加文件夹", ACCENT);
        if add_clicked {
            action = Action::AddFolder;
        }
        if let Some((done, total)) = self.probing {
            p.text(
                pos2(add_left - 20.0, top.center().y),
                Align2::RIGHT_CENTER,
                format!("生成缩略图 {done}/{total}"),
                FontId::monospace(11.0),
                TEXT_3,
            );
        }

        // ---- Nav ----
        let nav = Rect::from_min_max(pos2(rect.left(), top.bottom()), pos2(rect.left() + NAV_W, rect.bottom()));
        p.line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(0.5, LINE));
        let mut y = nav.top() + 14.0;
        let continue_n = self.videos.iter().filter(|v| v.progress().is_some_and(|x| x < FINISHED)).count();
        let week_ago = crate::history::now_secs() - 7 * 24 * 3600;
        let recent_n = self.videos.iter().filter(|v| v.added_at >= week_ago).count();
        for (view, label, n) in [
            (View::All, "全部", self.videos.len()),
            (View::Continue, "继续观看", continue_n),
            (View::Recent, "最近添加", recent_n),
        ] {
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 32.0));
            if nav_item(ui, &p, r, ("nav", label), label, Some(n), self.view == view) {
                self.view = view;
            }
            y += 32.0;
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
            y += 14.0;
            p.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "TAGS", FontId::monospace(10.0), TEXT_3);
            y += 22.0;
            for (tag, n) in tag_counts.iter().take(12) {
                let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
                let on = self.tags_sel.iter().any(|t| t.eq_ignore_ascii_case(tag));
                if nav_item(ui, &p, r, ("tag", tag.as_str()), &format!("# {tag}"), Some(*n), on) {
                    toggle_tag(&mut self.tags_sel, tag);
                }
                y += 28.0;
            }
        }

        // Folders
        y += 14.0;
        p.text(pos2(nav.left() + 18.0, y + 8.0), Align2::LEFT_CENTER, "FOLDERS", FontId::monospace(10.0), TEXT_3);
        y += 22.0;
        let mut remove_folder = None;
        for f in &self.folders {
            if y > nav.bottom() - 28.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(nav.left(), y), vec2(NAV_W, 28.0));
            let name = std::path::Path::new(&f.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.clone());
            let on = self.folder_sel == Some(f.id);
            let hovered = ui.rect_contains_pointer(r);
            if nav_item(ui, &p, r, ("folder", f.id), &name, if hovered { None } else { Some(f.count as usize) }, on) {
                self.folder_sel = if on { None } else { Some(f.id) };
            }
            if hovered {
                let x = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(22.0, 22.0));
                let xr = ui.interact(x, ui.scope_id().with(("folder_rm", f.id)), Sense::click());
                icons::draw(&p, Icon::Close, x.center(), 11.0, if xr.hovered() { ACCENT } else { TEXT_3 });
                xr.clone().on_hover_text(format!("从媒体库移除（不删除文件）\n{}", f.path));
                if xr.clicked() {
                    remove_folder = Some(f.id);
                }
            }
            y += 28.0;
        }
        if let (Some(id), Some(lib)) = (remove_folder, &self.lib) {
            lib.remove_folder(id);
            self.dirty = true;
        }

        // ---- Content ----
        let content = Rect::from_min_max(pos2(nav.right(), top.bottom()), rect.max);
        if self.folders.is_empty() {
            return match empty_library(ui, &p, content) {
                Action::None => action,
                a => a,
            };
        }
        let ids: Vec<i64> = self.filtered().iter().map(|v| v.id).collect();
        let header = Rect::from_min_size(content.min + vec2(24.0, 12.0), vec2(content.width() - 48.0, 30.0));
        let mut hx = header.left();
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
        p.text(pos2(hx, header.center().y), Align2::LEFT_CENTER, format!("{} 个视频", ids.len()), FontId::monospace(11.0), TEXT_3);
        // Sort (hidden for "继续观看", which is always by last played).
        if self.view != View::Continue {
            let mut sx = header.right();
            for (sort, label) in [(Sort::Duration, "时长"), (Sort::Name, "名称"), (Sort::Played, "最近播放"), (Sort::Added, "最近添加")] {
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

        let grid_rect = Rect::from_min_max(pos2(content.left() + 24.0, header.bottom() + 8.0), pos2(content.right() - 12.0, content.bottom()));
        if ids.is_empty() {
            p.text(grid_rect.center() - vec2(0.0, 40.0), Align2::CENTER_CENTER, "没有匹配的视频", FontId::proportional(14.0), TEXT_2);
        } else {
            let grid_action = self.grid(ui, grid_rect, &ids);
            if !matches!(grid_action, Action::None) {
                action = grid_action;
            }
        }
        if let Some(t) = remove_tag_filter {
            self.tags_sel.retain(|x| x != &t);
        }
        self.tag_editor(ui);
        action
    }

    fn grid(&mut self, ui: &mut egui::Ui, rect: Rect, ids: &[i64]) -> Action {
        let mut action = Action::None;
        let width = rect.width() - 12.0;
        let cols = (((width + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
        let card_w = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
        let thumb_h = card_w * 9.0 / 16.0;
        let card_h = thumb_h + META_H;
        let rows = ids.len().div_ceil(cols);
        let total_h = rows as f32 * (card_h + GAP);
        let mut loads = 0;
        let mut pending_thumbs = false;
        let mut events: Vec<CardEvent> = Vec::new();

        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().id_salt("library_grid").auto_shrink([false, false]).show_viewport(ui, |ui, vp| {
                ui.set_min_size(vec2(width, total_h));
                let origin = ui.max_rect().min;
                let first = ((vp.top() / (card_h + GAP)).floor().max(0.0)) as usize;
                let last = (((vp.bottom() / (card_h + GAP)).ceil()) as usize).min(rows);
                let p = ui.painter().clone();
                for row in first..last {
                    for col in 0..cols {
                        let i = row * cols + col;
                        let Some(&id) = ids.get(i) else { break };
                        let Some(v) = self.videos.iter().find(|v| v.id == id) else { continue };
                        let pos = origin + vec2(col as f32 * (card_w + GAP), row as f32 * (card_h + GAP));
                        let card = Rect::from_min_size(pos, vec2(card_w, card_h));
                        let thumb = Rect::from_min_size(pos, vec2(card_w, thumb_h));

                        // Thumbnail texture (decoded lazily).
                        if !self.thumbs.contains_key(&id) && !self.thumb_failed.contains(&id) {
                            if let Some(path) = &v.thumb {
                                if loads < THUMB_LOADS_PER_FRAME {
                                    loads += 1;
                                    match load_thumb(ui.ctx(), id, path) {
                                        Some(t) => {
                                            self.thumbs.insert(id, (path.clone(), t));
                                        }
                                        None => {
                                            self.thumb_failed.insert(id);
                                        }
                                    }
                                } else {
                                    pending_thumbs = true;
                                }
                            }
                        }
                        let tex = self.thumbs.get(&id).map(|(_, t)| t.id());
                        if let Some(ev) = draw_card(ui, &p, card, thumb, v, tex) {
                            events.push(ev);
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
                CardEvent::TagFilter(t) => toggle_tag(&mut self.tags_sel, &t),
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

    /// Small popup for typing a new tag.
    fn tag_editor(&mut self, ui: &mut egui::Ui) {
        let Some((id, mut text, focused)) = self.tag_edit.take() else { return };
        let mut keep = true;
        let mut commit = false;
        egui::Area::new(ui.scope_id().with("tag_editor"))
            .order(egui::Order::Foreground)
            .fixed_pos(self.tag_edit_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(PANEL)
                    .stroke(Stroke::new(0.5, ACCENT))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
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
}

enum CardEvent {
    Play(String),
    TagFilter(String),
    EditTags(i64, egui::Pos2),
    RemoveTag(i64, String),
    Reveal(String),
    Remove(i64),
}

fn draw_card(ui: &mut egui::Ui, p: &egui::Painter, card: Rect, thumb: Rect, v: &Video, tex: Option<egui::TextureId>) -> Option<CardEvent> {
    let mut ev = None;
    let resp = ui.interact(thumb, ui.scope_id().with(("card", v.id)), Sense::click());
    let hovered = resp.hovered();

    // Thumbnail or placeholder.
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

    // Resolution badge, top-left.
    let res = match v.height.max(v.width.min(v.height)) {
        h if h >= 2160 => "4K".to_string(),
        h if h >= 1440 => "2K".to_string(),
        h if h >= 1080 => "1080P".to_string(),
        h if h >= 720 => "720P".to_string(),
        h if h > 0 => format!("{h}P"),
        _ => String::new(),
    };
    if !res.is_empty() && v.width > 0 {
        let g = p.layout_no_wrap(res, FontId::monospace(9.0), TEXT);
        let r = Rect::from_min_size(thumb.min + vec2(6.0, 6.0), g.size() + vec2(8.0, 4.0));
        p.rect_filled(r, 3.0, theme::scrim(190));
        p.galley(r.min + vec2(4.0, 2.0), g, TEXT);
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
    let plays_w = if plays.is_empty() { 0.0 } else { p.layout_no_wrap(plays.clone(), FontId::monospace(10.0), TEXT).size().x + 8.0 };
    let title = elide(p, &v.title, FontId::proportional(13.0), card.width() - plays_w);
    p.text(pos2(card.left(), ty), Align2::LEFT_CENTER, title, FontId::proportional(13.0), if finished { TEXT_2 } else { TEXT });
    if !plays.is_empty() {
        p.text(pos2(card.right(), ty), Align2::RIGHT_CENTER, plays, FontId::monospace(10.0), TEXT_3);
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

    if resp.clicked() {
        ev = Some(CardEvent::Play(v.path.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button("播放").clicked() {
            ev = Some(CardEvent::Play(v.path.clone()));
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

fn empty_library(ui: &mut egui::Ui, p: &egui::Painter, content: Rect) -> Action {
    let c = content.center() - vec2(0.0, 30.0);
    let frame = Rect::from_center_size(c - vec2(0.0, 30.0), vec2(240.0, 120.0));
    icons::corner_brackets(p, frame, 12.0, Stroke::new(1.0, theme::accent_alpha(200)));
    icons::draw(p, Icon::List, frame.center() - vec2(0.0, 14.0), 26.0, ACCENT);
    p.text(frame.center() + vec2(0.0, 24.0), Align2::CENTER_CENTER, "建立你的媒体库", FontId::proportional(14.0), TEXT);
    p.text(
        pos2(c.x, frame.bottom() + 22.0),
        Align2::CENTER_CENTER,
        "添加视频所在的文件夹，自动生成缩略图，支持标签与搜索",
        FontId::proportional(12.0),
        TEXT_2,
    );
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

/// Right-aligned text button ending at `right_center`.
/// Returns (left edge, clicked).
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

fn nav_item(ui: &mut egui::Ui, p: &egui::Painter, r: Rect, id: impl std::hash::Hash + std::fmt::Debug, label: &str, count: Option<usize>, on: bool) -> bool {
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    if on {
        p.rect_filled(r, 0.0, theme::accent_alpha(14));
        p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(2.0, ACCENT));
    } else if resp.hovered() {
        p.rect_filled(r, 0.0, Color32::from_white_alpha(6));
    }
    let text = elide(p, label, FontId::proportional(13.0), r.width() - 70.0);
    p.text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), if on || resp.hovered() { TEXT } else { TEXT_2 });
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
        if let Some(dir) = std::path::Path::new(path).parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

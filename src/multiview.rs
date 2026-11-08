//! Multi-view: several videos in one window, each with its own player,
//! sound, progress and volume. The main player keeps its playlist, history
//! and media keys; extra slots ("panes") are independent mpv instances.
//!
//! In multi-view each slot has a small control bar of its own, and keys go
//! to the focused slot (blue outline). The big control bar is hidden; turning
//! multi-view off brings back the full single-video player. Full screen is
//! per slot: that one video takes the whole screen while the others play on.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, ViewportCommand, Align2, Color32, CursorIcon, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, TextureId, pos2, vec2};
use eframe::glow;

use crate::app::fmt_time;
use crate::history::Settings;
use crate::i18n::{tf, tr};
use crate::icons::{self, Icon};
use crate::mpv::{Event, RenderContext};
use crate::player::Player;
use crate::session::{Resume, Tracker};
use crate::split_layout::{Content, Dir, Divider, Layout, MAX_SLOTS, Preset};
use crate::theme::{self, ACCENT, TEXT, TEXT_2, TEXT_3};
use crate::video::VideoTarget;

/// Height of the multi-view bar at the top of the window.
pub const TOP_H: f32 = 40.0;
const MINI_H: f32 = 30.0;
const HEADER_H: f32 = 26.0;
/// New panes start quieter: several soundtracks add up.
const PANE_VOLUME: f64 = 50.0;

type ProcAddr = Arc<dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void + Send + Sync>;

/// An extra player with its own render target.
pub struct Pane {
    // Field order matters: the render context must drop before the player.
    render: Option<RenderContext>,
    video: Option<VideoTarget>,
    pub player: Player,
    tracker: Tracker,
    error: Option<String>,
    /// Opened once the render context exists: mpv's embedded video output
    /// gives up on a file that starts before it has somewhere to draw.
    pending: Option<String>,
}

impl Pane {
    fn new(ctx: &egui::Context, settings: &Settings, incognito: bool, path: &str) -> Result<Self, String> {
        let player = Player::new()?;
        let wake = ctx.clone();
        player.mpv.set_wakeup(Box::new(move || wake.request_repaint()));
        player.apply_settings(settings);
        player.set_volume(PANE_VOLUME);
        Ok(Self {
            render: None,
            video: None,
            player,
            tracker: Tracker::for_pane(settings.clone(), incognito),
            error: None,
            pending: Some(path.to_string()),
        })
    }

    /// GL objects are made on the UI thread, where the context is current.
    fn ensure_gl(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame, gpa: &ProcAddr) {
        if self.render.is_some() || self.error.is_some() {
            return;
        }
        let Some(gl) = frame.gl().cloned() else { return };
        let repaint = ctx.clone();
        match RenderContext::new(&self.player.mpv, gpa.clone(), Box::new(move || repaint.request_repaint())) {
            Ok(r) => self.render = Some(r),
            Err(e) => {
                self.error = Some(e);
                return;
            }
        }
        match VideoTarget::new(&gl, frame) {
            Ok(v) => self.video = Some(v),
            Err(e) => self.error = Some(e),
        }
        if let Some(path) = self.pending.take() {
            self.player.open(&path, false);
        }
    }

    /// Opens a file now if the pane can draw, else as soon as it can.
    fn open(&mut self, path: &str) {
        if self.render.is_some() {
            self.player.open(path, false);
        } else {
            self.pending = Some(path.to_string());
        }
    }

    fn destroy(mut self, gl: Option<&glow::Context>) {
        self.tracker.finish();
        self.player.stop();
        if let (Some(v), Some(gl)) = (self.video.take(), gl) {
            v.destroy(gl);
        }
        self.render = None;
    }
}

enum Drag {
    Divider(Divider),
    /// Dragging a slot's title to swap it with another.
    Swap { from: u32, start: Pos2, moved: bool },
    MoveFloat { id: u32, grab: egui::Vec2 },
    ResizeFloat { id: u32 },
}

#[derive(Default)]
pub struct Actions {
    /// Pick a file for this slot.
    pub open_file_for: Option<u32>,
    /// Load this file into this slot (from the playlist menu).
    pub load: Option<(u32, String)>,
    pub back: bool,
    pub osd: Option<String>,
    /// The multi-view switch was turned off; the app decides what follows.
    pub exit: bool,
}

/// What turning multi-view off leads to.
#[derive(Debug, PartialEq)]
pub enum Exit {
    /// More than one video is playing: they'd have to be closed first.
    Blocked,
    /// The main player's video (or nothing) carries on as a single video.
    Main,
    /// The one video left is in another slot: the main player takes it over.
    Pane { path: String, position: f64, paused: bool },
}

pub struct MultiView {
    pub on: bool,
    pub layout: Layout,
    panes: HashMap<u32, Pane>,
    next_pane: u32,
    pub focus: Content,
    /// Play/pause, seeking and speed go to every slot.
    pub link: bool,
    /// Add-video menu: (slot, anchor).
    menu: Option<(u32, Pos2)>,
    menu_fresh: bool,
    drag: Option<Drag>,
    /// Panes waiting for the GL context to be released.
    dead: Vec<Pane>,
    /// The area slots were laid out in last frame.
    grid: Rect,
    /// One video blown up to fill the whole area; the others keep playing.
    pub maximized: Option<Content>,
    /// Per-slot control bars only show when the pointer is at a slot's bottom.
    pub hide_bars: bool,
    /// One video alone on the whole screen; the others keep playing.
    solo: Option<Content>,
    /// Whether the window was already full screen, to go back to it after.
    solo_was_fullscreen: bool,
    /// Full screen has taken effect, so leaving it some other way ends `solo`.
    solo_seen_fullscreen: bool,
    /// The whole area multi-view drew in last frame.
    stage: Rect,
    /// Where files being dragged in are, to mark the slot they'll replace.
    pub drop_at: Option<Pos2>,
}

impl Default for MultiView {
    fn default() -> Self {
        Self {
            on: false,
            layout: Layout::single(Content::Main),
            panes: HashMap::new(),
            next_pane: 0,
            focus: Content::Main,
            link: false,
            menu: None,
            menu_fresh: false,
            drag: None,
            dead: Vec::new(),
            grid: Rect::NOTHING,
            maximized: None,
            hide_bars: false,
            solo: None,
            solo_was_fullscreen: false,
            solo_seen_fullscreen: false,
            stage: Rect::NOTHING,
            drop_at: None,
        }
    }
}

impl MultiView {
    pub fn enable(&mut self) {
        self.on = true;
        self.layout = Layout::single(Content::Main);
        self.layout.apply_preset(Preset::LeftRight, &[Content::Main]);
        self.focus = Content::Main;
    }

    /// Back to one video: extra players stop, the main one keeps playing.
    pub fn disable(&mut self, ctx: &egui::Context) {
        self.exit_solo(ctx);
        self.on = false;
        self.dead.extend(self.panes.drain().map(|(_, p)| p));
        self.layout = Layout::single(Content::Main);
        self.focus = Content::Main;
        self.link = false;
        self.menu = None;
        self.drag = None;
        self.maximized = None;
    }

    /// What turning multi-view off would do now.
    pub fn exit_plan(&self, main: &Player) -> Exit {
        let videos: Vec<Content> = self.layout.contents().into_iter().filter(|c| !self.is_empty(*c, main)).collect();
        match videos[..] {
            [Content::Pane(id)] => match self.panes.get(&id).map(|p| &p.player.state) {
                Some(s) if !s.path.is_empty() => Exit::Pane { path: s.path.clone(), position: s.position, paused: s.paused },
                _ => Exit::Main,
            },
            [] | [_] => Exit::Main,
            _ => Exit::Blocked,
        }
    }

    /// The video filling the whole area, if any (and still in the layout).
    fn max_slot(&self) -> Option<(u32, Content)> {
        let c = self.maximized.filter(|c| *c != Content::Empty)?;
        self.layout.find(c).map(|id| (id, c))
    }

    /// The video on its own on the whole screen, if any (and still in the layout).
    fn solo_slot(&self) -> Option<(u32, Content)> {
        let c = self.solo.filter(|c| *c != Content::Empty)?;
        self.layout.find(c).map(|id| (id, c))
    }

    /// Shows one video alone, full screen, or goes back to the layout.
    fn toggle_solo(&mut self, ctx: &egui::Context, content: Content) {
        if self.solo.is_some() {
            self.exit_solo(ctx);
            return;
        }
        if content == Content::Empty {
            return;
        }
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        self.solo = Some(content);
        self.focus = content;
        self.menu = None;
        self.drag = None;
        self.solo_was_fullscreen = fullscreen;
        self.solo_seen_fullscreen = fullscreen;
        if !fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
        }
    }

    fn exit_solo(&mut self, ctx: &egui::Context) {
        if self.solo.take().is_some() && !self.solo_was_fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
    }

    fn toggle_max(&mut self, content: Content) {
        self.maximized = if self.maximized == Some(content) { None } else { Some(content) };
        self.menu = None;
        self.drag = None;
    }

    /// Where the slots go: the stage below the multi-view bar.
    pub fn grid_rect(stage: Rect) -> Rect {
        Rect::from_min_max(pos2(stage.left(), stage.top() + TOP_H), stage.max).shrink(2.0)
    }

    /// Where the main video goes (a leaf, a floating window or the whole stage).
    pub fn main_rect(&self, stage: Rect) -> Option<Rect> {
        if let Some((_, c)) = self.solo_slot() {
            return (c == Content::Main).then_some(stage);
        }
        let grid = Self::grid_rect(stage);
        if let Some((_, c)) = self.max_slot() {
            return (c == Content::Main).then_some(grid);
        }
        let (leaves, _) = self.layout.rects(grid);
        leaves
            .iter()
            .find(|(_, c, _)| *c == Content::Main)
            .map(|(_, _, r)| inner(*r))
            .or_else(|| {
                self.layout.floats.iter().find(|f| f.content == Content::Main).map(|f| Layout::float_rect(f, grid))
            })
    }

    /// The slot under a point (floating windows first).
    pub fn slot_at(&self, pos: Pos2) -> Option<(u32, Content)> {
        if let Some(slot) = self.solo_slot() {
            return self.stage.contains(pos).then_some(slot);
        }
        if let Some(slot) = self.max_slot() {
            return self.grid.contains(pos).then_some(slot);
        }
        for f in self.layout.floats.iter().rev() {
            if Layout::float_rect(f, self.grid).contains(pos) {
                return Some((f.id, f.content));
            }
        }
        let (leaves, _) = self.layout.rects(self.grid);
        leaves.into_iter().find(|(_, _, r)| r.contains(pos)).map(|(id, c, _)| (id, c))
    }

    /// Opens `path` in a slot. Returns false if the slot is the main one
    /// (the caller opens it in the main player as usual).
    pub fn load(&mut self, slot: u32, path: &str, ctx: &egui::Context, settings: &Settings, incognito: bool) -> Result<bool, String> {
        match self.layout.content(slot) {
            Some(Content::Main) | None => Ok(false),
            Some(Content::Pane(id)) => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.open(path);
                }
                self.focus = Content::Pane(id);
                Ok(true)
            }
            Some(Content::Empty) => {
                let pane = Pane::new(ctx, settings, incognito, path)?;
                self.next_pane += 1;
                let id = self.next_pane;
                self.panes.insert(id, pane);
                self.layout.set_content(slot, Content::Pane(id));
                self.focus = Content::Pane(id);
                Ok(true)
            }
        }
    }

    pub fn video_count(&self, main: &Player) -> usize {
        self.layout.contents().into_iter().filter(|c| !self.is_empty(*c, main)).count()
    }

    /// Progress tracking and events for the extra players.
    pub fn pump(&mut self) {
        for pane in self.panes.values_mut() {
            for ev in pane.player.pump() {
                match ev {
                    Event::FileLoaded => {
                        if let Resume::Resumed(pos) | Resume::Ask(pos) = pane.tracker.file_loaded(&pane.player.state) {
                            pane.player.seek_to(pos, true);
                        }
                    }
                    Event::EndFile => pane.tracker.finish(),
                    _ => {}
                }
            }
            pane.tracker.tick(&pane.player.state);
        }
    }

    fn player<'a>(&'a self, content: Content, main: &'a Player) -> Option<&'a Player> {
        match content {
            Content::Main => Some(main),
            Content::Pane(id) => self.panes.get(&id).map(|p| &p.player),
            Content::Empty => None,
        }
    }

    /// A slot showing the empty "add video" screen: no video, or the main
    /// player with nothing open (its slot keeps its place for the next one).
    fn is_empty(&self, content: Content, main: &Player) -> bool {
        match content {
            Content::Empty => true,
            Content::Main => main.state.idle,
            Content::Pane(id) => !self.panes.contains_key(&id),
        }
    }

    /// Back to the empty screen: the video stops, the slot stays.
    fn clear_slot(&mut self, slot: u32, content: Content, main: &Player) {
        match content {
            Content::Main => main.stop(),
            Content::Pane(id) => {
                if let Some(p) = self.panes.remove(&id) {
                    self.dead.push(p);
                }
                self.layout.set_content(slot, Content::Empty);
            }
            Content::Empty => {}
        }
        if self.maximized == Some(content) {
            self.maximized = None;
        }
        self.menu = None;
    }

    /// Every player currently on screen.
    fn players<'a>(&'a self, main: &'a Player) -> Vec<&'a Player> {
        self.layout.contents().into_iter().filter_map(|c| self.player(c, main)).collect()
    }

    /// Play/pause one slot, or all of them when linked.
    fn toggle_pause(&self, content: Content, main: &Player) {
        let Some(p) = self.player(content, main) else { return };
        if self.link {
            let pause = !(p.state.paused || p.state.eof);
            for q in self.players(main) {
                // A finished video starts over, as it does when unlinked.
                if !pause && q.state.eof {
                    q.seek_to(0.0, true);
                }
                q.mpv.set_flag("pause", pause);
            }
        } else {
            p.toggle_pause();
        }
    }

    fn seek_to(&self, content: Content, main: &Player, secs: f64, exact: bool) {
        if self.link {
            for q in self.players(main) {
                q.seek_to(secs.min(q.state.duration.max(0.0)), exact);
            }
        } else if let Some(p) = self.player(content, main) {
            p.seek_to(secs, exact);
        }
    }

    fn seek_by(&self, content: Content, main: &Player, delta: f64) {
        if self.link {
            if let Some(p) = self.player(content, main) {
                self.seek_to(content, main, (p.state.position + delta).max(0.0), true);
            }
        } else if let Some(p) = self.player(content, main) {
            p.seek_by(delta);
        }
    }

    /// Keys for the focused slot. Returns an OSD message, if any.
    pub fn handle_keys(&mut self, ctx: &egui::Context, keys: &[Key], main: &Player) -> Option<String> {
        let mut osd = None;
        for key in keys {
            let focus = self.focus;
            match key {
                // Full screen is the focused video's alone.
                Key::F | Key::Enter => {
                    self.toggle_solo(ctx, focus);
                    continue;
                }
                Key::Escape => {
                    if self.solo.is_some() {
                        self.exit_solo(ctx);
                    } else if self.maximized.take().is_none() && ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                    }
                    continue;
                }
                _ => {}
            }
            let Some(p) = self.player(focus, main) else { continue };
            match key {
                Key::Space | Key::K => {
                    self.toggle_pause(focus, main);
                    osd = Some(if p.state.paused { "PLAY".into() } else { "PAUSE".into() });
                }
                Key::ArrowLeft | Key::ArrowRight => {
                    let d = if *key == Key::ArrowLeft { -5.0 } else { 5.0 };
                    self.seek_by(focus, main, d);
                    osd = Some(format!("{} 5s", if d < 0.0 { "<<" } else { ">>" }));
                }
                Key::ArrowUp | Key::ArrowDown => {
                    let v = (p.state.volume + if *key == Key::ArrowUp { 5.0 } else { -5.0 }).clamp(0.0, 150.0);
                    p.set_volume(v);
                    osd = Some(format!("VOLUME  {}%", v.round()));
                }
                Key::M => {
                    p.toggle_mute();
                    osd = Some(if p.state.muted { "UNMUTE".into() } else { "MUTE".into() });
                }
                Key::Tab => {
                    // Move focus to the next slot with a video.
                    let shown: Vec<Content> = self.layout.contents().into_iter().filter(|c| !self.is_empty(*c, main)).collect();
                    if let Some(i) = shown.iter().position(|c| *c == focus) {
                        self.focus = shown[(i + 1) % shown.len()];
                    }
                }
                _ => {}
            }
        }
        osd
    }

    /// Releases panes that were closed, while the GL context is current.
    fn reap(&mut self, gl: Option<&glow::Context>) {
        for pane in self.dead.drain(..) {
            pane.destroy(gl);
        }
    }

    /// On exit: every extra player stops and frees its GL objects.
    pub fn shut_down(&mut self, gl: Option<&glow::Context>) {
        self.dead.extend(self.panes.drain().map(|(_, p)| p));
        self.reap(gl);
    }

    fn close_slot(&mut self, slot: u32) {
        if let Some(Content::Pane(id)) = self.layout.remove(slot) {
            if let Some(p) = self.panes.remove(&id) {
                self.dead.push(p);
            }
        }
        if self.layout.find(self.focus).is_none() || self.focus == Content::Empty {
            self.focus = Content::Main;
        }
    }

    fn apply_preset(&mut self, preset: Preset) {
        let mut order = vec![Content::Main];
        order.extend(self.layout.contents().into_iter().filter(|c| matches!(c, Content::Pane(_))));
        let leftover = self.layout.apply_preset(preset, &order);
        for c in leftover {
            if let Content::Pane(id) = c {
                if let Some(p) = self.panes.remove(&id) {
                    self.dead.push(p);
                }
            }
        }
        if self.layout.find(self.focus).is_none() {
            self.focus = Content::Main;
        }
    }

    // ---- Drawing -------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        gpa: Option<&ProcAddr>,
        stage: Rect,
        main: &Player,
        main_texture: Option<TextureId>,
        playlist: &[(String, String)],
        chrome: bool,
    ) -> Actions {
        let mut out = Actions::default();
        let ctx = ui.ctx().clone();
        self.reap(frame.gl().map(|g| g.as_ref()));
        let grid = Self::grid_rect(stage);
        self.grid = grid;
        self.stage = stage;
        // Leaving full screen another way (the window's own button) ends it.
        if self.solo.is_some() {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if self.solo_slot().is_none() {
                self.exit_solo(&ctx);
            } else if fullscreen {
                self.solo_seen_fullscreen = true;
            } else if self.solo_seen_fullscreen {
                self.solo = None;
            }
        }
        let p = ui.painter().clone();
        p.rect_filled(stage, 0.0, Color32::BLACK);

        if let Some(gpa) = gpa {
            for pane in self.panes.values_mut() {
                pane.ensure_gl(&ctx, frame, gpa);
            }
        }
        let ppp = ctx.pixels_per_point();
        let pointer = ctx.input(|i| i.pointer.hover_pos());

        // ---- One video alone, full screen ----
        if let Some((id, content)) = self.solo_slot() {
            self.focus = content;
            let empty = self.is_empty(content, main);
            self.draw_video(&p, frame, content, stage, main_texture, ppp, empty);
            self.slot_chrome(ui, &p, id, content, stage, false, main, chrome, pointer, &mut out);
            self.hide_cursor(&ctx, stage, main, chrome, pointer);
            self.drop_hint(&p, main);
            return out;
        }

        // ---- One video filling the area ----
        if self.maximized.is_some() && self.max_slot().is_none() {
            self.maximized = None;
        }
        if let Some((id, content)) = self.max_slot() {
            self.focus = content;
            let empty = self.is_empty(content, main);
            self.draw_video(&p, frame, content, grid, main_texture, ppp, empty);
            self.slot_chrome(ui, &p, id, content, grid, false, main, chrome, pointer, &mut out);
            self.top_bar(ui, &p, stage, main, &mut out);
            self.hide_cursor(&ctx, grid, main, chrome, pointer);
            self.drop_hint(&p, main);
            return out;
        }

        // ---- Leaves ----
        let (leaves, dividers) = self.layout.rects(grid);
        let mut chrome_jobs: Vec<(u32, Content, Rect, bool)> = Vec::new();
        for (id, content, r) in &leaves {
            let r = inner(*r);
            let empty = self.is_empty(*content, main);
            self.draw_video(&p, frame, *content, r, main_texture, ppp, empty);
            chrome_jobs.push((*id, *content, r, false));
        }
        // ---- Floats (videos) ----
        for f in &self.layout.floats.clone() {
            let r = Layout::float_rect(f, grid);
            p.rect_filled(r.expand(1.0), 4.0, Color32::from_white_alpha(70));
            let empty = self.is_empty(f.content, main);
            self.draw_video(&p, frame, f.content, r, main_texture, ppp, empty);
            chrome_jobs.push((f.id, f.content, r, true));
        }

        // Interaction: leaves first, then dividers, then floats on top.
        for (id, content, r, floating) in chrome_jobs.iter().filter(|j| !j.3) {
            self.slot_chrome(ui, &p, *id, *content, *r, *floating, main, chrome, pointer, &mut out);
        }
        for d in &dividers {
            let resp = ui.interact(d.rect, ui.scope_id().with(("mv_div", d.split)), Sense::drag());
            let hot = resp.hovered() || resp.dragged();
            if hot {
                ctx.set_cursor_icon(if d.dir == Dir::H { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
                p.rect_filled(d.rect, 0.0, theme::accent_alpha(150));
            }
            if resp.drag_started() {
                self.drag = Some(Drag::Divider(*d));
            }
        }
        for (id, content, r, floating) in chrome_jobs.iter().filter(|j| j.3) {
            self.slot_chrome(ui, &p, *id, *content, *r, *floating, main, chrome, pointer, &mut out);
        }

        self.handle_drag(&ctx, &p, grid, pointer);
        self.top_bar(ui, &p, stage, main, &mut out);
        self.add_menu(ui, playlist, &mut out);
        self.hide_cursor(&ctx, grid, main, chrome, pointer);
        self.drop_hint(&p, main);
        out
    }

    /// Outlines the slot that files being dragged in will go to.
    fn drop_hint(&self, p: &egui::Painter, main: &Player) {
        let Some((id, content)) = self.drop_at.and_then(|q| self.slot_at(q)) else { return };
        let Some(r) = self.slot_rect(id) else { return };
        p.rect_filled(r, 3.0, theme::scrim(120));
        p.rect_stroke(r, 3.0, Stroke::new(3.0, ACCENT), StrokeKind::Inside);
        let text = if self.is_empty(content, main) { tr("Drop to play here") } else { tr("Drop to replace this video") };
        p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), TEXT);
    }

    /// Where a slot is on screen now.
    fn slot_rect(&self, id: u32) -> Option<Rect> {
        if self.solo_slot().is_some_and(|(s, _)| s == id) {
            return Some(self.stage);
        }
        if self.max_slot().is_some_and(|(s, _)| s == id) {
            return Some(self.grid);
        }
        if let Some(f) = self.layout.floats.iter().find(|f| f.id == id) {
            return Some(Layout::float_rect(f, self.grid));
        }
        let (leaves, _) = self.layout.rects(self.grid);
        leaves.into_iter().find(|(s, _, _)| *s == id).map(|(_, _, r)| inner(r))
    }

    /// Hides the pointer over a playing video once the mouse rests.
    fn hide_cursor(&self, ctx: &egui::Context, area: Rect, main: &Player, chrome: bool, pointer: Option<Pos2>) {
        if chrome || self.drag.is_some() || self.menu.is_some() {
            return;
        }
        let Some(q) = pointer.filter(|q| area.contains(*q)) else { return };
        let playing = self.slot_at(q).and_then(|(_, c)| self.player(c, main)).is_some_and(|pl| !pl.state.paused && !pl.state.eof);
        if playing {
            ctx.set_cursor_icon(CursorIcon::None);
        }
    }

    fn draw_video(&mut self, p: &egui::Painter, frame: &mut eframe::Frame, content: Content, r: Rect, main_tex: Option<TextureId>, ppp: f32, empty: bool) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let content = if empty { Content::Empty } else { content };
        match content {
            Content::Main => {
                if let Some(t) = main_tex {
                    p.image(t, r, uv, Color32::WHITE);
                }
            }
            Content::Pane(id) => {
                let Some(pane) = self.panes.get_mut(&id) else { return };
                if pane.player.state.idle {
                    return;
                }
                if let (Some(video), Some(render), Some(gl)) = (&mut pane.video, &pane.render, frame.gl()) {
                    let px = [(r.width() * ppp).round() as i32, (r.height() * ppp).round() as i32];
                    if let Ok(t) = video.update(gl, render, px, None) {
                        p.image(t, r, uv, Color32::WHITE);
                    }
                }
                if let Some(e) = &pane.error {
                    p.text(r.center(), Align2::CENTER_CENTER, e, FontId::proportional(12.0), TEXT_2);
                }
            }
            Content::Empty => {
                p.rect_filled(r, 3.0, theme::panel());
                p.rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_rgb(0x30, 0x36, 0x3D)), StrokeKind::Inside);
                p.text(r.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, "+", FontId::proportional(24.0), TEXT_2);
                p.text(r.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, tr("Add a video"), FontId::proportional(13.0), TEXT_2);
                p.text(r.center() + vec2(0.0, 34.0), Align2::CENTER_CENTER, tr("Click or drop a file"), FontId::proportional(11.0), TEXT_3);
            }
        }
    }

    /// Focus outline, header (title + tools), mini control bar.
    #[allow(clippy::too_many_arguments)]
    fn slot_chrome(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        id: u32,
        content: Content,
        r: Rect,
        floating: bool,
        main: &Player,
        chrome: bool,
        pointer: Option<Pos2>,
        _out: &mut Actions,
    ) {
        let ctx = ui.ctx().clone();
        let body = ui.interact(r, ui.scope_id().with(("mv_slot", id)), Sense::click());
        let hovered = pointer.is_some_and(|q| r.contains(q));
        let empty = self.is_empty(content, main);
        if body.clicked() {
            if empty {
                if self.menu.map(|(s, _)| s) == Some(id) {
                    self.menu = None;
                } else {
                    self.menu = Some((id, body.interact_pointer_pos().unwrap_or(r.center())));
                    self.menu_fresh = true;
                }
            } else {
                // Clicking a video focuses it and toggles play/pause; a
                // double-click toggles twice (no net change) and goes full screen.
                self.focus = content;
                self.menu = None;
                self.toggle_pause(content, main);
            }
        }
        // Double-click: this video alone, full screen, or back.
        if body.double_clicked() && !empty {
            self.toggle_solo(&ctx, content);
        }
        if empty {
            // Full screen or enlarged, an empty slot has nothing to show.
            if self.solo == Some(content) {
                self.exit_solo(&ctx);
            }
            if hovered {
                p.rect_stroke(r, 3.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            // An empty slot can still be closed (if it isn't the last). The
            // main player's slot stays, to show its next video.
            if self.layout.slot_count() > 1 && hovered && content != Content::Main {
                let x = Rect::from_min_size(pos2(r.right() - 28.0, r.top() + 4.0), vec2(24.0, 20.0));
                if tool_button(ui, p, x, ("mv_close", id), Tool::Close, tr("Off")) {
                    self.close_slot(id);
                }
            }
            return;
        }
        let Some(s) = self.player(content, main).map(|pl| pl.state.clone()) else { return };
        let focused = self.focus == content;
        // Alone on the screen there's nothing to tell it apart from.
        if focused && self.solo.is_none() {
            p.rect_stroke(r, 3.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
        }

        // ---- Header: title (drag to swap / move) and tools ----
        // Like the bar, it hides while the mouse rests.
        let show_header = (hovered && chrome) || (floating && self.solo.is_none());
        if show_header {
            let head = Rect::from_min_size(r.min, vec2(r.width(), HEADER_H));
            p.rect_filled(head, 0.0, theme::scrim(150));
            let tools = self.tool_list(id, content, floating);
            let tools_w = tools.len() as f32 * 26.0 + 4.0;
            let title_w = (r.width() - tools_w - 16.0).max(20.0);
            let title = elide(p, &s.title, FontId::proportional(12.0), title_w);
            p.text(pos2(r.left() + 8.0, head.center().y), Align2::LEFT_CENTER, title, FontId::proportional(12.0), TEXT);
            let grab = Rect::from_min_size(head.min, vec2((r.width() - tools_w).max(10.0), HEADER_H));
            let gr = ui.interact(grab, ui.scope_id().with(("mv_grab", id)), Sense::click_and_drag());
            if gr.hovered() {
                ctx.set_cursor_icon(CursorIcon::Grab);
            }
            if gr.drag_started() {
                let start = gr.interact_pointer_pos().unwrap_or(r.center());
                self.drag = Some(if floating {
                    Drag::MoveFloat { id, grab: start - r.min }
                } else {
                    Drag::Swap { from: id, start, moved: false }
                });
            }
            if gr.clicked() {
                self.focus = content;
            }
            let mut x = r.right() - 4.0 - tools.len() as f32 * 26.0;
            for (tool, tip) in tools {
                let br = Rect::from_min_size(pos2(x, head.top() + 3.0), vec2(24.0, 20.0));
                if tool_button(ui, p, br, ("mv_tool", id, tool as u8), tool, tip) {
                    match tool {
                        Tool::SplitH => {
                            self.layout.split(id, Dir::H);
                        }
                        Tool::SplitV => {
                            self.layout.split(id, Dir::V);
                        }
                        Tool::Float => {
                            self.layout.float(id);
                        }
                        Tool::Dock => {
                            let beside = self.layout.find(self.focus).filter(|_| self.focus != content);
                            self.layout.dock(id, beside);
                        }
                        Tool::Close => self.clear_slot(id, content, main),
                        Tool::Max | Tool::Restore => self.toggle_max(content),
                        Tool::Fullscreen | Tool::ExitFullscreen => self.toggle_solo(&ctx, content),
                    }
                }
                x += 26.0;
            }
        }
        if floating {
            // Resize from the bottom-right corner.
            let grip = Rect::from_min_size(r.max - vec2(14.0, 14.0), vec2(14.0, 14.0));
            let g = ui.interact(grip, ui.scope_id().with(("mv_grip", id)), Sense::drag());
            p.add(egui::Shape::convex_polygon(
                vec![grip.right_top(), grip.right_bottom(), grip.left_bottom()],
                Color32::from_white_alpha(if g.hovered() { 200 } else { 110 }),
                Stroke::NONE,
            ));
            if g.hovered() || g.dragged() {
                ctx.set_cursor_icon(CursorIcon::ResizeNwSe);
            }
            if g.drag_started() {
                self.drag = Some(Drag::ResizeFloat { id });
            }
        }

        // ---- Mini control bar ----
        let at_bottom = pointer.is_some_and(|q| r.contains(q) && q.y >= r.bottom() - MINI_H - 24.0);
        // Each bar hides on its own once the mouse rests or leaves the video.
        let show_bar = if self.hide_bars { at_bottom } else { (hovered && chrome) || at_bottom || s.paused };
        if !show_bar || r.height() < MINI_H + HEADER_H {
            return;
        }
        let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - MINI_H), r.max);
        p.rect_filled(bar, 0.0, theme::scrim(170));
        let cy = bar.center().y;
        let mut x = bar.left() + 6.0;
        // Play / pause
        let pp = Rect::from_center_size(pos2(x + 11.0, cy), vec2(22.0, 22.0));
        let ppr = ui.interact(pp, ui.scope_id().with(("mv_pp", id)), Sense::click());
        icons::draw(p, if s.paused || s.eof { Icon::Play } else { Icon::Pause }, pp.center(), 13.0, if ppr.hovered() { ACCENT } else { TEXT });
        if ppr.clicked() {
            self.toggle_pause(content, main);
            self.focus = content;
        }
        x += 26.0;
        let narrow = r.width() < 300.0;
        let vol_w = if narrow { 0.0 } else { 56.0 };
        let time_txt = format!("{} / {}", fmt_time(s.position), fmt_time(s.duration));
        let time_w = if narrow { 0.0 } else { 92.0 };
        let fs_w = 26.0;
        let right_w = 22.0 + vol_w + if narrow { 0.0 } else { 34.0 } + fs_w;
        // Progress
        let track = Rect::from_min_max(pos2(x, cy - 8.0), pos2(bar.right() - right_w - time_w - 10.0, cy + 8.0));
        if track.width() > 20.0 && s.duration > 0.0 {
            let tr_ = ui.interact(track, ui.scope_id().with(("mv_seek", id)), Sense::click_and_drag());
            let f = (s.position / s.duration).clamp(0.0, 1.0) as f32;
            p.line_segment([pos2(track.left(), cy), pos2(track.right(), cy)], Stroke::new(3.0, Color32::from_white_alpha(50)));
            p.line_segment([pos2(track.left(), cy), pos2(track.left() + f * track.width(), cy)], Stroke::new(3.0, if focused { ACCENT } else { TEXT }));
            if tr_.hovered() || tr_.dragged() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            if let Some(q) = tr_.interact_pointer_pos() {
                let t = ((q.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * s.duration;
                if tr_.dragged() {
                    self.seek_to(content, main, t, false);
                }
                if tr_.drag_stopped() || tr_.clicked() {
                    self.seek_to(content, main, t, true);
                    self.focus = content;
                }
            }
        }
        if !narrow {
            p.text(pos2(track.right() + 8.0, cy), Align2::LEFT_CENTER, time_txt, FontId::monospace(10.0), TEXT);
        }
        // Mute + volume
        let mut vx = bar.right() - right_w;
        let mute = Rect::from_center_size(pos2(vx + 11.0, cy), vec2(22.0, 22.0));
        let mr = ui.interact(mute, ui.scope_id().with(("mv_mute", id)), Sense::click());
        let vicon = if s.muted || s.volume <= 0.0 { Icon::Muted } else { Icon::Volume };
        icons::draw(p, vicon, mute.center(), 13.0, if mr.hovered() { ACCENT } else { TEXT });
        if mr.clicked() {
            if let Some(pl) = self.player(content, main) {
                pl.toggle_mute();
            }
            self.focus = content;
        }
        vx += 24.0;
        if !narrow {
            let vol = Rect::from_min_size(pos2(vx, cy - 8.0), vec2(vol_w, 16.0));
            let vr = ui.interact(vol, ui.scope_id().with(("mv_vol", id)), Sense::click_and_drag());
            let frac = if s.muted { 0.0 } else { (s.volume / 100.0).clamp(0.0, 1.0) as f32 };
            p.line_segment([pos2(vol.left(), cy), pos2(vol.right(), cy)], Stroke::new(3.0, Color32::from_white_alpha(50)));
            p.line_segment([pos2(vol.left(), cy), pos2(vol.left() + frac * vol.width(), cy)], Stroke::new(3.0, TEXT));
            if let Some(q) = vr.interact_pointer_pos() {
                if vr.dragged() || vr.clicked() {
                    if let Some(pl) = self.player(content, main) {
                        pl.set_volume(((q.x - vol.left()) / vol.width()).clamp(0.0, 1.0) as f64 * 100.0);
                    }
                }
            }
            if vr.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            let pct = if s.muted { tr("Mute").to_string() } else { format!("{}%", s.volume.round()) };
            p.text(pos2(vol.right() + 6.0, cy), Align2::LEFT_CENTER, pct, FontId::monospace(10.0), TEXT_2);
        }
        // Full screen for this video alone
        let fs = Rect::from_center_size(pos2(bar.right() - fs_w / 2.0 - 2.0, cy), vec2(22.0, 22.0));
        let fr = ui.interact(fs, ui.scope_id().with(("mv_fs", id)), Sense::click());
        let solo = self.solo.is_some();
        icons::draw(p, if solo { Icon::ExitFullscreen } else { Icon::Fullscreen }, fs.center(), 13.0, if fr.hovered() { ACCENT } else { TEXT });
        if fr.hovered() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if fr.on_hover_text(if solo { tr("Exit full screen (Esc)") } else { tr("Full screen (double-click)") }).clicked() {
            self.toggle_solo(&ctx, content);
        }
    }

    fn tool_list(&self, _id: u32, content: Content, floating: bool) -> Vec<(Tool, &'static str)> {
        let mut v = Vec::new();
        if self.solo.is_some() {
            v.push((Tool::ExitFullscreen, tr("Exit full screen (Esc)")));
            v.push((Tool::Close, tr("Close video")));
            return v;
        }
        if self.maximized.is_some() {
            v.push((Tool::Restore, tr("Restore (Esc)")));
            v.push((Tool::Fullscreen, tr("Full screen (double-click)")));
            v.push((Tool::Close, tr("Close video")));
            return v;
        }
        if content != Content::Empty {
            v.push((Tool::Fullscreen, tr("Full screen (double-click)")));
            v.push((Tool::Max, tr("Enlarge in the window")));
        }
        if floating {
            v.push((Tool::Dock, tr("Put back in the layout")));
        } else {
            if self.layout.can_add() {
                v.push((Tool::SplitH, tr("Split left / right")));
                v.push((Tool::SplitV, tr("Split top / bottom")));
            }
            if content != Content::Empty {
                v.push((Tool::Float, tr("Float as a small window")));
            }
        }
        // Stops the video; the slot goes back to "add video".
        v.push((Tool::Close, tr("Close video")));
        v
    }

    fn handle_drag(&mut self, ctx: &egui::Context, p: &egui::Painter, grid: Rect, pointer: Option<Pos2>) {
        let Some(drag) = &mut self.drag else { return };
        let released = ctx.input(|i| !i.pointer.primary_down());
        let Some(q) = pointer.or_else(|| ctx.input(|i| i.pointer.interact_pos())) else {
            if released {
                self.drag = None;
            }
            return;
        };
        match drag {
            Drag::Divider(d) => {
                let ratio = match d.dir {
                    Dir::H => (q.x - d.area.left()) / d.area.width().max(1.0),
                    Dir::V => (q.y - d.area.top()) / d.area.height().max(1.0),
                };
                let split = d.split;
                self.layout.set_ratio(split, ratio);
                ctx.set_cursor_icon(if d.dir == Dir::H { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
            }
            Drag::Swap { from, start, moved } => {
                if (q - *start).length() > 6.0 {
                    *moved = true;
                }
                let from = *from;
                if *moved {
                    ctx.set_cursor_icon(CursorIcon::Grabbing);
                    let (leaves, _) = self.layout.rects(grid);
                    if let Some((_, _, r)) = leaves.iter().find(|(id, _, r)| *id != from && r.contains(q)) {
                        p.rect_stroke(inner(*r).shrink(3.0), 3.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                    }
                    if released {
                        if let Some((to, _, _)) = leaves.iter().find(|(id, _, r)| *id != from && r.contains(q)) {
                            self.layout.swap(from, *to);
                        }
                    }
                }
            }
            Drag::MoveFloat { id, grab } => {
                let (id, grab) = (*id, *grab);
                if let Some(f) = self.layout.floats.iter().find(|f| f.id == id) {
                    let size = f.rect.size();
                    let min = q - grab;
                    let fx = (min.x - grid.left()) / grid.width();
                    let fy = (min.y - grid.top()) / grid.height();
                    self.layout.set_float_rect(id, Rect::from_min_size(pos2(fx, fy), size));
                }
                ctx.set_cursor_icon(CursorIcon::Grabbing);
            }
            Drag::ResizeFloat { id } => {
                let id = *id;
                if let Some(f) = self.layout.floats.iter().find(|f| f.id == id) {
                    let min = f.rect.min;
                    let fx = (q.x - grid.left()) / grid.width();
                    let fy = (q.y - grid.top()) / grid.height();
                    self.layout.set_float_rect(id, Rect::from_min_max(min, pos2(fx, fy)));
                }
                ctx.set_cursor_icon(CursorIcon::ResizeNwSe);
            }
        }
        if released {
            self.drag = None;
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, p: &egui::Painter, stage: Rect, main: &Player, out: &mut Actions) {
        let ctx = ui.ctx().clone();
        let band = Rect::from_min_size(stage.min, vec2(stage.width(), TOP_H));
        let _blocker = ui.interact(band, ui.scope_id().with("mv_bar"), Sense::click());
        p.rect_filled(band, 0.0, theme::panel());
        p.line_segment([band.left_bottom(), band.right_bottom()], Stroke::new(1.0, theme::line()));
        let cy = band.center().y;

        // Back to the library
        let back = Rect::from_min_size(pos2(band.left() + 8.0, cy - 13.0), vec2(74.0, 26.0));
        let br = ui.interact(back, ui.scope_id().with("mv_back"), Sense::click());
        let bc = if br.hovered() { ACCENT } else { theme::text() };
        let st = Stroke::new(1.8, bc);
        let bx = back.left() + 14.0;
        p.line_segment([pos2(bx + 4.0, cy - 6.0), pos2(bx - 2.0, cy)], st);
        p.line_segment([pos2(bx - 2.0, cy), pos2(bx + 4.0, cy + 6.0)], st);
        p.text(pos2(bx + 12.0, cy), Align2::LEFT_CENTER, tr("Library"), FontId::proportional(13.0), if br.hovered() { ACCENT } else { theme::text() });
        if br.clicked() {
            out.back = true;
        }

        // Presets
        let mut x = back.right() + 16.0;
        p.text(pos2(x, cy), Align2::LEFT_CENTER, tr("Preset"), FontId::proportional(11.0), theme::text_3());
        x += 30.0;
        for preset in Preset::ALL {
            let label = preset.label();
            let w = p.layout_no_wrap(label.to_string(), FontId::proportional(12.0), TEXT).size().x + 16.0;
            let r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(w, 24.0));
            let resp = ui.interact(r, ui.scope_id().with(("mv_preset", preset as u8)), Sense::click());
            if resp.hovered() {
                p.rect_filled(r, 12.0, Color32::from_white_alpha(14));
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            p.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), if resp.hovered() { theme::text() } else { theme::text_2() });
            if resp.clicked() {
                self.apply_preset(preset);
            }
            x = r.right() + 2.0;
        }

        // Right side: multi-view switch, divider, link switch, count
        let mut rx = band.right() - 14.0;
        let (sw_left, off) = switch(ui, p, pos2(rx, cy), "mv_toggle", true, tr("Multi-view"), 1.0);
        if off {
            out.exit = true;
        }
        rx = sw_left - 14.0;
        p.line_segment([pos2(rx, cy - 8.0), pos2(rx, cy + 8.0)], Stroke::new(1.0, Color32::from_rgb(0x30, 0x36, 0x3D)));
        rx -= 14.0;
        let (link_left, flip) = switch(ui, p, pos2(rx, cy), "mv_link", self.link, tr("Linked"), 1.0);
        if flip {
            self.link = !self.link;
            out.osd = Some(if self.link { tr("Linked: play, pause and seeking apply to every video").into() } else { tr("Linked: off").into() });
        }
        rx = link_left - 16.0;
        let (bars_left, flip) = switch(ui, p, pos2(rx, cy), "mv_bars", !self.hide_bars, tr("Bars"), 1.0);
        if flip {
            self.hide_bars = !self.hide_bars;
            out.osd = Some(if self.hide_bars { tr("Bars hidden: move the pointer to a video's bottom to show them").into() } else { tr("Bars shown").into() });
        }
        rx = bars_left - 16.0;
        let count = tf("{} / {} videos", &[&self.video_count(main), &MAX_SLOTS]);
        if rx - 120.0 > x {
            p.text(pos2(rx, cy), Align2::RIGHT_CENTER, count, FontId::monospace(11.0), theme::text_3());
        }
    }

    fn add_menu(&mut self, ui: &mut egui::Ui, playlist: &[(String, String)], out: &mut Actions) {
        let Some((slot, at)) = self.menu else { return };
        let ctx = ui.ctx().clone();
        let mut close = false;
        let area = egui::Area::new(ui.scope_id().with("mv_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(&ctx, |ui| {
                egui::Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(0.5, Color32::from_white_alpha(40)))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(4, 4))
                    .show(ui, |ui| {
                        ui.set_min_width(200.0);
                        if menu_item(ui, tr("Open file…")) {
                            out.open_file_for = Some(slot);
                            close = true;
                        }
                        if !playlist.is_empty() {
                            ui.add_space(2.0);
                            ui.label(egui::RichText::new(tr("From the playlist")).size(11.0).color(TEXT_3));
                            for (title, path) in playlist.iter().take(10) {
                                if menu_item(ui, title) {
                                    out.load = Some((slot, path.clone()));
                                    close = true;
                                }
                            }
                        }
                    });
            });
        let fresh = std::mem::take(&mut self.menu_fresh);
        let outside = !fresh
            && ctx.input(|i| i.pointer.any_click())
            && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !area.response.rect.contains(q));
        if close || outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.menu = None;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    SplitH,
    SplitV,
    Float,
    Dock,
    Close,
    Max,
    Restore,
    Fullscreen,
    ExitFullscreen,
}

/// A slot's inside: leaves sit in their cell with no extra margin.
fn inner(r: Rect) -> Rect {
    r
}

fn tool_button(ui: &mut egui::Ui, p: &egui::Painter, r: Rect, id: impl std::hash::Hash + std::fmt::Debug, tool: Tool, tip: &str) -> bool {
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    let hot = resp.hovered();
    p.rect_filled(r, 3.0, if hot { ACCENT } else { Color32::from_black_alpha(150) });
    let c = r.center();
    let st = Stroke::new(1.3, Color32::WHITE);
    let box_ = Rect::from_center_size(c, vec2(12.0, 9.0));
    match tool {
        Tool::SplitH => {
            p.rect_stroke(box_, 1.0, st, StrokeKind::Middle);
            p.line_segment([pos2(c.x, box_.top()), pos2(c.x, box_.bottom())], st);
        }
        Tool::SplitV => {
            p.rect_stroke(box_, 1.0, st, StrokeKind::Middle);
            p.line_segment([pos2(box_.left(), c.y), pos2(box_.right(), c.y)], st);
        }
        Tool::Float => {
            p.rect_stroke(box_, 1.0, st, StrokeKind::Middle);
            p.rect_filled(Rect::from_min_size(pos2(c.x + 0.5, c.y - 0.5), vec2(5.0, 4.0)), 0.0, Color32::WHITE);
        }
        Tool::Dock => {
            p.rect_stroke(box_, 1.0, st, StrokeKind::Middle);
            p.line_segment([pos2(c.x + 3.0, c.y - 2.5), pos2(c.x - 2.0, c.y + 2.5)], st);
            p.line_segment([pos2(c.x - 2.0, c.y + 2.5), pos2(c.x - 2.0, c.y - 0.5)], st);
            p.line_segment([pos2(c.x - 2.0, c.y + 2.5), pos2(c.x + 1.0, c.y + 2.5)], st);
        }
        Tool::Close => icons::draw(p, Icon::Close, c, 10.0, Color32::WHITE),
        Tool::Fullscreen => icons::draw(p, Icon::Fullscreen, c, 11.0, Color32::WHITE),
        Tool::ExitFullscreen => icons::draw(p, Icon::ExitFullscreen, c, 11.0, Color32::WHITE),
        Tool::Max | Tool::Restore => {
            // Four corners pointing out (enlarge) or in (restore).
            let (o, i) = (6.0, 2.5);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = if tool == Tool::Max { c + vec2(sx * o, sy * o * 0.8) } else { c + vec2(sx * i, sy * i * 0.8) };
                let toward = if tool == Tool::Max { -1.0 } else { 1.0 };
                p.line_segment([corner, corner + vec2(toward * sx * 3.5, 0.0)], st);
                p.line_segment([corner, corner + vec2(0.0, toward * sy * 3.0)], st);
            }
        }
    }
    if hot {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.on_hover_text(tip).clicked()
}

/// A labeled switch ending at `right_center`. Returns (left edge, clicked).
pub fn switch(ui: &mut egui::Ui, p: &egui::Painter, right_center: Pos2, id: &str, on: bool, label: &str, alpha: f32) -> (f32, bool) {
    let track = Rect::from_min_size(pos2(right_center.x - 34.0, right_center.y - 9.0), vec2(34.0, 18.0));
    let lw = p.layout_no_wrap(label.to_string(), FontId::proportional(12.0), TEXT).size().x;
    let all = Rect::from_min_max(pos2(track.left() - lw - 8.0, track.top() - 4.0), pos2(track.right(), track.bottom() + 4.0));
    let resp = ui.interact(all, ui.scope_id().with(id), Sense::click());
    let a = |c: Color32| c.gamma_multiply(alpha);
    p.rect_filled(track, 9.0, a(if on { ACCENT } else { Color32::from_gray(0x55) }));
    let kx = if on { track.right() - 9.0 } else { track.left() + 9.0 };
    p.circle_filled(pos2(kx, track.center().y), 7.0, a(Color32::WHITE));
    let text = if alpha >= 1.0 { theme::text() } else { TEXT };
    p.text(pos2(track.left() - 8.0, track.center().y), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), a(if resp.hovered() { ACCENT } else { text }));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    (all.left(), resp.clicked())
}

fn menu_item(ui: &mut egui::Ui, label: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, ACCENT);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let text = elide(ui.painter(), label, FontId::proportional(12.5), r.width() - 20.0);
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), if resp.hovered() { Color32::WHITE } else { TEXT });
    resp.clicked()
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

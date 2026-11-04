//! The player window: video surface, floating controls, OSD and empty state.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Key, Rect, Sense, Stroke, StrokeKind,
    ViewportCommand, pos2, vec2,
};
use eframe::glow;

use crate::history::Entry;
use crate::icons::{self, Icon};
use crate::mpv::{Event, RenderContext};
use crate::player::Player;
use crate::session::{Resume, Tracker};
use crate::settings_panel::{self, PanelState};
use crate::theme::{self, ACCENT, LINE, TEXT, TEXT_2, TEXT_3};
use crate::video::VideoTarget;

const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "flv", "wmv", "m4v", "ts", "m2ts", "mpg", "mpeg", "3gp",
    "ogv", "rmvb", "mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma",
];
const HIDE_AFTER: f64 = 2.0;
const OSD_SECS: f64 = 1.5;
/// The "continue from…" prompt disappears after this long.
const RESUME_PROMPT_SECS: f64 = 10.0;

type ProcAddr = Arc<dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void + Send + Sync>;

pub struct App {
    // Field order matters: the render context must drop before the player.
    render: Option<RenderContext>,
    video: Option<VideoTarget>,
    player: Option<Player>,
    error: Option<String>,
    get_proc_address: Option<ProcAddr>,
    pending_open: Vec<String>,
    last_activity: f64,
    /// (text, shown at, lifetime in seconds)
    osd: Option<(String, f64, f64)>,
    seek_preview: Option<f64>,
    shown_title: String,
    tracker: Tracker,
    /// (saved position, time shown) while asking whether to resume.
    resume_prompt: Option<(f64, f64)>,
    settings_open: bool,
    panel: PanelState,
    recent: Vec<Entry>,
    recent_dirty: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<String>) -> Self {
        theme::apply(&cc.egui_ctx);
        let (player, error) = match Player::new() {
            Ok(p) => {
                let ctx = cc.egui_ctx.clone();
                p.mpv.set_wakeup(Box::new(move || ctx.request_repaint()));
                (Some(p), None)
            }
            Err(e) => (None, Some(e)),
        };
        Self {
            render: None,
            video: None,
            player,
            error,
            get_proc_address: cc.get_proc_address.clone(),
            pending_open: files,
            last_activity: 0.0,
            osd: None,
            seek_preview: None,
            shown_title: String::new(),
            tracker: Tracker::new(),
            resume_prompt: None,
            settings_open: false,
            panel: PanelState::default(),
            recent: Vec::new(),
            recent_dirty: true,
        }
    }

    /// Creates the GL objects on the first frame, when the context is current.
    fn ensure_gl(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.render.is_some() || self.error.is_some() {
            return;
        }
        let (Some(player), Some(gl), Some(gpa)) =
            (&self.player, frame.gl().cloned(), self.get_proc_address.clone())
        else {
            self.error = Some("OpenGL context unavailable".into());
            return;
        };
        let repaint = ctx.clone();
        match RenderContext::new(&player.mpv, gpa, Box::new(move || repaint.request_repaint())) {
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
    }

    fn show_osd(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.osd = Some((text.into(), now, OSD_SECS));
    }

    fn open_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("媒体文件", MEDIA_EXTS)
            .add_filter("所有文件", &["*"])
            .pick_files()
        {
            self.pending_open
                .extend(files.into_iter().map(|p| p.to_string_lossy().into_owned()));
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let Some(player) = &self.player else { return };
        let (dropped, keys, cmd, scroll, moved) = ctx.input(|i| {
            let dropped: Vec<String> = i
                .raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_string_lossy().into_owned())
                .filter(|p| !p.is_empty())
                .collect();
            let mut keys: Vec<Key> = Vec::new();
            let mut scroll = 0.0f32;
            for e in &i.events {
                match e {
                    egui::Event::Key { key, pressed: true, .. } => keys.push(*key),
                    egui::Event::MouseWheel { delta, .. } => scroll += delta.y,
                    _ => {}
                }
            }
            (dropped, keys, i.modifiers.command, scroll, i.pointer.delta() != egui::Vec2::ZERO)
        });

        let now = ctx.input(|i| i.time);
        if moved || !keys.is_empty() || scroll != 0.0 {
            self.last_activity = now;
        }
        self.pending_open.extend(dropped);

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let s = player.state.clone();
        let mut osd: Option<String> = None;
        let mut open_dialog = false;

        // The settings overlay is modal: only its own shortcuts apply.
        if keys.iter().any(|k| *k == Key::Comma) && cmd {
            self.settings_open = !self.settings_open;
            return;
        }
        if self.settings_open {
            if keys.contains(&Key::Escape) {
                self.settings_open = false;
            }
            return;
        }

        for key in keys {
            if let Some((pos, _)) = self.resume_prompt {
                match key {
                    Key::Enter => {
                        player.seek_to(pos, true);
                        self.resume_prompt = None;
                        osd = Some(format!("RESUME  {}", fmt_time(pos)));
                        continue;
                    }
                    Key::Escape => {
                        self.resume_prompt = None;
                        continue;
                    }
                    _ => {}
                }
            }
            match key {
                Key::O if cmd => open_dialog = true,
                Key::Home if !s.idle => {
                    player.seek_to(0.0, true);
                    self.resume_prompt = None;
                    osd = Some("SEEK  00:00".into());
                }
                Key::Space | Key::K => {
                    player.toggle_pause();
                    osd = Some(if s.paused || s.eof { "PLAY".into() } else { "PAUSE".into() });
                }
                Key::ArrowLeft | Key::ArrowRight if !s.idle => {
                    let step = if cmd { 60.0 } else { 5.0 };
                    let d = if key == Key::ArrowLeft { -step } else { step };
                    player.seek_by(d);
                    let target = (s.position + d).clamp(0.0, s.duration.max(0.0));
                    osd = Some(format!(
                        "{} {}s   {}",
                        if d < 0.0 { "<<" } else { ">>" },
                        step as i32,
                        fmt_time(target)
                    ));
                }
                Key::ArrowUp | Key::ArrowDown => {
                    let v = s.volume + if key == Key::ArrowUp { 5.0 } else { -5.0 };
                    player.set_volume(v);
                    osd = Some(format!("VOLUME  {}%", v.clamp(0.0, 150.0).round()));
                }
                Key::M => {
                    player.toggle_mute();
                    osd = Some(if s.muted { "UNMUTE".into() } else { "MUTE".into() });
                }
                Key::F | Key::Enter => {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
                }
                Key::Escape if fullscreen => {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                Key::OpenBracket | Key::CloseBracket => {
                    let sp = player.step_speed(if key == Key::OpenBracket { -1 } else { 1 });
                    osd = Some(format!("SPEED  {}", fmt_speed(sp)));
                }
                Key::Backspace => {
                    player.set_speed(1.0);
                    osd = Some("SPEED  1.0×".into());
                }
                Key::C => {
                    player.cycle_subtitles();
                    osd = Some("SUBTITLE  NEXT".into());
                }
                Key::PageDown | Key::N => player.next(),
                Key::PageUp | Key::P => player.prev(),
                Key::S if cmd => {
                    player.screenshot();
                    osd = Some("SCREENSHOT  → DESKTOP".into());
                }
                _ => {}
            }
        }

        if scroll != 0.0 && !s.idle {
            let v = s.volume + scroll.signum() as f64 * 5.0;
            player.set_volume(v);
            osd = Some(format!("VOLUME  {}%", v.clamp(0.0, 150.0).round()));
        }

        if let Some(t) = osd {
            self.show_osd(ctx, t);
        }
        if open_dialog {
            self.open_dialog();
        }
    }

    fn flush_open(&mut self) {
        let Some(player) = &self.player else { return };
        for (i, path) in self.pending_open.drain(..).enumerate() {
            player.open(&path, i > 0);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ensure_gl(&ctx, frame);
        self.handle_input(&ctx);
        self.flush_open();

        let full = ctx.content_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, Color32::BLACK);

        if let Some(err) = &self.error {
            draw_error(&painter, full, err);
            return;
        }
        let Some(player) = &self.player else { return };
        let s = player.state.clone();

        // Window title follows the media title.
        let title = if s.idle { "TinyPlayer".to_string() } else { format!("{} — TinyPlayer", s.title) };
        if title != self.shown_title {
            // New media: show the controls briefly so the user sees what loaded.
            self.last_activity = ctx.input(|i| i.time);
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }

        // ---- Video ----
        let surface = ui.interact(full, ui.scope_id().with("surface"), Sense::click());
        if !s.idle {
            if let (Some(video), Some(render), Some(gl)) = (&mut self.video, &self.render, frame.gl()) {
                let ppp = ctx.pixels_per_point();
                let px = [(full.width() * ppp).round() as i32, (full.height() * ppp).round() as i32];
                video.update(gl, render, px);
                painter.image(
                    video.texture_id,
                    full,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        if surface.double_clicked() {
            let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fs));
        }

        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

        if s.idle {
            match draw_empty_state(ui, full, &self.recent) {
                EmptyAction::Open => self.open_dialog(),
                EmptyAction::Recent(path) => self.pending_open.push(path),
                EmptyAction::Settings => self.settings_open = true,
                EmptyAction::None => {}
            }
        } else {
            let now = ctx.input(|i| i.time);
            let pointer_in_bar = ctx
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| p.y > full.bottom() - 70.0 || p.y < full.top() + 50.0);
            let visible = s.paused
                || now - self.last_activity < HIDE_AFTER
                || pointer_in_bar
                || self.seek_preview.is_some();
            let alpha = ctx.animate_bool_with_time(ui.scope_id().with("chrome"), visible, 0.15);
            if !visible && surface.hovered() {
                ctx.set_cursor_icon(CursorIcon::None);
            }
            if visible && !s.paused {
                ctx.request_repaint_after_secs((HIDE_AFTER - (now - self.last_activity)).max(0.05) as f32);
            }

            if s.paused && !s.eof {
                icons::draw(&painter, Icon::Play, full.center(), 40.0, Color32::from_white_alpha(220));
            }
            if alpha > 0.0 {
                draw_top_bar(&painter, full, &s, self.tracker.current_play_count(), alpha);
                self.draw_controls(ui, full, alpha);
            }
            self.draw_resume_prompt(ui, full, now);
        }

        // ---- OSD ----
        if let Some((text, at, life)) = &self.osd {
            let now = ctx.input(|i| i.time);
            let age = now - at;
            if age < *life {
                let a = ((life - age) / 0.2).min(1.0) as f32;
                draw_osd(&painter, full, text, a);
                ctx.request_repaint_after_secs(0.05);
            } else {
                self.osd = None;
            }
        }

        if hovering_files {
            let r = full.shrink(12.0);
            painter.rect_filled(full, 0.0, theme::scrim(150));
            icons::corner_brackets(&painter, r, 24.0, Stroke::new(1.5, ACCENT));
            painter.text(full.center(), Align2::CENTER_CENTER, "松开以播放", FontId::proportional(16.0), TEXT);
        }

        if self.settings_open {
            let stats = self.tracker.stats();
            let before = self.tracker.settings.clone();
            let act = settings_panel::show(
                ui,
                full,
                &mut self.panel,
                &mut self.tracker.settings,
                &mut self.tracker.incognito,
                stats,
            );
            if act.settings_changed && self.tracker.settings != before {
                self.tracker.save_settings();
            }
            if act.clear_history {
                if let Some(st) = &self.tracker.store {
                    st.clear_history();
                }
                self.recent_dirty = true;
                self.show_osd(&ctx, "HISTORY  CLEARED");
            }
            if act.clear_stats {
                if let Some(st) = &self.tracker.store {
                    st.clear_play_stats();
                }
                self.show_osd(&ctx, "PLAY STATS  CLEARED");
            }
            if act.close {
                self.settings_open = false;
            }
        }
    }

    /// Runs on every wakeup, even while the window is minimized, so progress
    /// and play counts stay accurate during background playback.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Some(player) = &mut self.player else { return };
        let now = ctx.input(|i| i.time);
        for ev in player.pump() {
            match ev {
                Event::FileLoaded => {
                    self.resume_prompt = None;
                    match self.tracker.file_loaded(&player.state) {
                        Resume::Resumed(pos) => {
                            player.seek_to(pos, true);
                            let msg = format!("已从 {} 继续 · Home 从头", fmt_time(pos));
                            self.osd = Some((msg, now, 3.5));
                        }
                        Resume::Ask(pos) => self.resume_prompt = Some((pos, now)),
                        Resume::None => {}
                    }
                }
                Event::EndFile => {
                    self.tracker.finish();
                    self.recent_dirty = true;
                }
                _ => {}
            }
        }
        self.tracker.tick(&player.state);
        if player.state.idle && self.recent_dirty {
            self.recent = self.tracker.recent(5);
            self.recent_dirty = false;
        }
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.tracker.shutdown();
        if let (Some(v), Some(gl)) = (self.video.take(), gl) {
            v.destroy(gl);
        }
        self.render = None;
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

// ---- Controls --------------------------------------------------------------

impl App {
    fn draw_controls(&mut self, ui: &mut egui::Ui, full: Rect, alpha: f32) {
        let Some(player) = &self.player else { return };
        let s = player.state.clone();
        let painter = ui.painter().clone();
        let a = |c: Color32| c.gamma_multiply(alpha);

        let bar = Rect::from_min_max(
            pos2(full.left() + 14.0, full.bottom() - 50.0),
            pos2(full.right() - 14.0, full.bottom() - 12.0),
        );
        painter.rect_filled(bar, 19.0, theme::scrim((184.0 * alpha) as u8));
        painter.rect_stroke(bar, 19.0, Stroke::new(0.5, theme::accent_alpha((46.0 * alpha) as u8)), StrokeKind::Inside);

        let cy = bar.center().y;
        let mut x = bar.left() + 8.0;
        let mut osd: Option<String> = None;

        let button = |ui: &mut egui::Ui, x: &mut f32, id: &str, icon: Icon, size: f32| -> bool {
            let rect = Rect::from_center_size(pos2(*x + 15.0, cy), vec2(30.0, 30.0));
            *x += 30.0;
            let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
            let hovered = resp.hovered();
            if hovered {
                painter.circle_filled(rect.center(), 15.0, a(theme::accent_alpha(20)));
            }
            icons::draw(&painter, icon, rect.center(), size, a(if hovered { ACCENT } else { TEXT }));
            resp.clicked()
        };

        if button(ui, &mut x, "prev", Icon::Prev, 14.0) {
            player.prev();
        }
        let pp_icon = if s.paused || s.eof { Icon::Play } else { Icon::Pause };
        if button(ui, &mut x, "pp", pp_icon, 18.0) {
            player.toggle_pause();
        }
        if button(ui, &mut x, "next", Icon::Next, 14.0) {
            player.next();
        }

        // Right side, laid out from the right edge inward.
        let mut rx = bar.right() - 8.0 - 30.0;
        let fs = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        let mut tmp = rx;
        if button(ui, &mut tmp, "fs", if fs { Icon::ExitFullscreen } else { Icon::Fullscreen }, 14.0) {
            ui.ctx().send_viewport_cmd(ViewportCommand::Fullscreen(!fs));
        }

        rx -= 30.0;
        let mut tmp = rx;
        let open_settings = button(ui, &mut tmp, "settings", Icon::Settings, 15.0);

        rx -= 30.0;
        let cc_rect = Rect::from_min_size(pos2(rx, cy - 15.0), vec2(30.0, 30.0));
        if text_button(ui, &painter, cc_rect, "cc", "CC", alpha) {
            player.cycle_subtitles();
            osd = Some("SUBTITLE  NEXT".into());
        }

        rx -= 46.0;
        let sp_rect = Rect::from_min_size(pos2(rx, cy - 15.0), vec2(46.0, 30.0));
        if text_button(ui, &painter, sp_rect, "speed", &fmt_speed(s.speed), alpha) {
            let next = crate::player::SPEEDS
                .iter()
                .copied()
                .find(|v| *v > s.speed + 0.01)
                .unwrap_or(crate::player::SPEEDS[0]);
            player.set_speed(next);
            osd = Some(format!("SPEED  {}", fmt_speed(next)));
        }

        // Volume slider
        rx -= 70.0;
        let vol_rect = Rect::from_min_size(pos2(rx, cy - 10.0), vec2(62.0, 20.0));
        let vresp = ui.interact(vol_rect, ui.scope_id().with("vol"), Sense::click_and_drag());
        let vol_frac = if s.muted { 0.0 } else { (s.volume / 100.0).clamp(0.0, 1.0) as f32 };
        let line = |from: f32, to: f32, c: Color32| {
            painter.line_segment(
                [pos2(vol_rect.left() + from * vol_rect.width(), cy), pos2(vol_rect.left() + to * vol_rect.width(), cy)],
                Stroke::new(2.0, a(c)),
            );
        };
        line(0.0, 1.0, LINE);
        line(0.0, vol_frac, if vresp.hovered() || vresp.dragged() { ACCENT } else { TEXT_2 });
        if let Some(p) = vresp.interact_pointer_pos() {
            if vresp.dragged() || vresp.clicked() {
                let v = ((p.x - vol_rect.left()) / vol_rect.width()).clamp(0.0, 1.0) as f64 * 100.0;
                player.set_volume(v);
                osd = Some(format!("VOLUME  {}%", v.round()));
            }
        }

        rx -= 30.0;
        let mut tmp = rx;
        if button(ui, &mut tmp, "mute", if s.muted || s.volume <= 0.0 { Icon::Muted } else { Icon::Volume }, 16.0) {
            player.toggle_mute();
        }

        // Times + progress track fill the space between.
        let cur = self.seek_preview.map_or(s.position, |f| f * s.duration);
        let cur_txt = fmt_time(cur);
        let dur_txt = fmt_time(s.duration);
        let mono = FontId::monospace(11.0);
        painter.text(pos2(x + 8.0, cy), Align2::LEFT_CENTER, &cur_txt, mono.clone(), a(TEXT));
        x += 8.0 + 9.0 * cur_txt.len() as f32 * 0.75 + 10.0;
        let dur_w = 9.0 * dur_txt.len() as f32 * 0.75;
        let right_end = rx - 10.0;
        painter.text(pos2(right_end, cy), Align2::RIGHT_CENTER, &dur_txt, mono.clone(), a(TEXT_3.lerp_to_gamma(TEXT_2, 0.5)));

        let track = Rect::from_min_max(pos2(x, cy - 10.0), pos2(right_end - dur_w - 10.0, cy + 10.0));
        if track.width() > 20.0 && s.duration > 0.0 {
            let tresp = ui.interact(track, ui.scope_id().with("track"), Sense::click_and_drag());
            let at = |f: f64| track.left() + (f.clamp(0.0, 1.0) as f32) * track.width();
            let seg = |from: f32, to: f32, c: Color32| {
                painter.line_segment([pos2(from, cy), pos2(to, cy)], Stroke::new(2.0, a(c)));
            };
            let frac = cur / s.duration;
            seg(track.left(), track.right(), LINE);
            seg(track.left(), at(s.cached_until / s.duration), Color32::from_rgb(0x30, 0x36, 0x3D));
            seg(track.left(), at(frac), ACCENT);
            let kx = at(frac);
            painter.line_segment([pos2(kx, cy - 5.0), pos2(kx, cy + 5.0)], Stroke::new(2.0, a(ACCENT)));

            let pointer_frac = tresp
                .hover_pos()
                .or(tresp.interact_pointer_pos())
                .map(|p| ((p.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64);
            if let Some(f) = pointer_frac {
                if tresp.hovered() || tresp.dragged() {
                    painter.text(
                        pos2(at(f), track.top() - 4.0),
                        Align2::CENTER_BOTTOM,
                        fmt_time(f * s.duration),
                        FontId::monospace(10.0),
                        a(ACCENT),
                    );
                }
                if tresp.dragged() {
                    self.seek_preview = Some(f);
                    player.seek_to(f * s.duration, false);
                }
                if tresp.drag_stopped() || tresp.clicked() {
                    self.seek_preview = None;
                    player.seek_to(f * s.duration, true);
                    osd = Some(format!("SEEK  {}", fmt_time(f * s.duration)));
                }
            }
        }

        if let Some(t) = osd {
            let ctx = ui.ctx().clone();
            self.show_osd(&ctx, t);
        }
        if open_settings {
            self.settings_open = true;
        }
    }

    /// "上次看到 12:40  继续播放 Enter  从头开始", above the control bar.
    fn draw_resume_prompt(&mut self, ui: &mut egui::Ui, full: Rect, now: f64) {
        let Some((pos, shown_at)) = self.resume_prompt else { return };
        let age = now - shown_at;
        if age > RESUME_PROMPT_SECS {
            self.resume_prompt = None;
            return;
        }
        ui.ctx().request_repaint_after_secs(0.25);
        let p = ui.painter().clone();
        let a = ((RESUME_PROMPT_SECS - age) / 0.3).min(1.0) as f32;

        let msg = format!("上次看到 {}", fmt_time(pos));
        let font = FontId::proportional(12.0);
        let mg = p.layout_no_wrap(msg, FontId::monospace(12.0), TEXT.gamma_multiply(a));
        let rg = p.layout_no_wrap("继续播放 Enter".into(), font.clone(), ACCENT);
        let sg = p.layout_no_wrap("从头开始".into(), font.clone(), TEXT_2);
        let h = 34.0;
        let w = 14.0 + mg.size().x + 18.0 + rg.size().x + 16.0 + sg.size().x + 14.0;
        let rect = Rect::from_min_size(pos2(full.left() + 16.0, full.bottom() - 64.0 - h), vec2(w, h));
        p.rect_filled(rect, 0.0, theme::scrim((210.0 * a) as u8));
        p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(2.0, ACCENT.gamma_multiply(a)));

        let cy = rect.center().y;
        let mut x = rect.left() + 14.0;
        p.galley(pos2(x, cy - mg.size().y / 2.0), mg.clone(), TEXT);
        x += mg.size().x + 18.0;

        let resume_r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(rg.size().x, 24.0));
        let rr = ui.interact(resume_r, ui.scope_id().with("resume_yes"), Sense::click());
        let rc = if rr.hovered() { TEXT } else { ACCENT };
        p.text(pos2(x, cy), Align2::LEFT_CENTER, "继续播放 Enter", font.clone(), rc.gamma_multiply(a));
        x += rg.size().x + 16.0;

        let start_r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(sg.size().x, 24.0));
        let sr = ui.interact(start_r, ui.scope_id().with("resume_no"), Sense::click());
        let sc = if sr.hovered() { TEXT } else { TEXT_2 };
        p.text(pos2(x, cy), Align2::LEFT_CENTER, "从头开始", font, sc.gamma_multiply(a));

        if rr.hovered() || sr.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if rr.clicked() {
            if let Some(player) = &self.player {
                player.seek_to(pos, true);
            }
            self.resume_prompt = None;
            let ctx = ui.ctx().clone();
            self.show_osd(&ctx, format!("RESUME  {}", fmt_time(pos)));
        } else if sr.clicked() {
            self.resume_prompt = None;
        }
    }
}

fn text_button(ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, id: &str, label: &str, alpha: f32) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
    let color = if resp.hovered() { ACCENT } else { TEXT_2 };
    painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(11.0), color.gamma_multiply(alpha));
    resp.clicked()
}

fn draw_top_bar(p: &egui::Painter, full: Rect, s: &crate::player::State, plays: i64, alpha: f32) {
    let a = |c: Color32| c.gamma_multiply(alpha);
    let band = Rect::from_min_max(full.min, pos2(full.right(), full.top() + 44.0));
    p.rect_filled(band, 0.0, theme::scrim((110.0 * alpha) as u8));
    let y = full.top() + 22.0;
    p.text(pos2(full.left() + 16.0, y), Align2::LEFT_CENTER, &s.title, FontId::proportional(13.0), a(TEXT));

    let mut tags: Vec<(String, Color32)> = Vec::new();
    if plays > 0 {
        tags.push((format!("PLAYED {plays}×"), TEXT_2));
    }
    if !s.codec.is_empty() {
        tags.push((s.codec.clone(), ACCENT));
    }
    if s.height > 0 {
        tags.push((format!("{}P", s.height), TEXT_2));
    }
    tags.push((
        if s.hwdec.is_empty() { "SW".into() } else { format!("HW · {}", s.hwdec.to_uppercase()) },
        TEXT_2,
    ));

    let mut x = full.right() - 16.0;
    let font = FontId::monospace(10.0);
    for (i, (t, c)) in tags.iter().rev().enumerate() {
        let r = p.text(pos2(x, y), Align2::RIGHT_CENTER, t, font.clone(), a(*c));
        x = r.left() - 9.0;
        if i + 1 < tags.len() {
            p.line_segment([pos2(x + 4.5, y - 5.0), pos2(x + 4.5, y + 5.0)], Stroke::new(1.0, a(Color32::from_rgb(0x30, 0x36, 0x3D))));
        }
    }
}

fn draw_osd(p: &egui::Painter, full: Rect, text: &str, alpha: f32) {
    let font = FontId::monospace(12.0);
    let galley = p.layout_no_wrap(text.to_string(), font, TEXT.gamma_multiply(alpha));
    let origin = pos2(full.left() + 16.0, full.top() + 54.0);
    let rect = Rect::from_min_size(origin, galley.size() + vec2(22.0, 12.0));
    p.rect_filled(rect, 0.0, theme::scrim((190.0 * alpha) as u8));
    p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(2.0, ACCENT.gamma_multiply(alpha)));
    p.galley(origin + vec2(12.0, 6.0), galley, TEXT);
}

enum EmptyAction {
    None,
    Open,
    Recent(String),
    Settings,
}

fn draw_empty_state(ui: &mut egui::Ui, full: Rect, recent: &[Entry]) -> EmptyAction {
    let p = ui.painter().clone();
    let list_h = if recent.is_empty() { 0.0 } else { 34.0 + 26.0 * recent.len() as f32 };
    let c = full.center() - vec2(0.0, list_h / 2.0);
    let frame = Rect::from_center_size(c - vec2(0.0, 24.0), vec2(220.0, 112.0));
    icons::corner_brackets(&p, frame, 11.0, Stroke::new(1.0, theme::accent_alpha(200)));
    icons::draw(&p, Icon::Drop, frame.center() - vec2(0.0, 14.0), 24.0, ACCENT);
    p.text(frame.center() + vec2(0.0, 22.0), Align2::CENTER_CENTER, "拖入媒体文件", FontId::proportional(13.0), TEXT);

    let open_rect = Rect::from_center_size(pos2(c.x, frame.bottom() + 26.0), vec2(150.0, 28.0));
    let resp = ui.interact(open_rect, ui.scope_id().with("open"), Sense::click());
    let color = if resp.hovered() { ACCENT } else { TEXT_2 };
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let shortcut = if cfg!(target_os = "macos") { "⌘O" } else { "Ctrl+O" };
    p.text(open_rect.center(), Align2::CENTER_CENTER, format!("打开文件  {shortcut}"), FontId::monospace(11.0), color);

    p.text(
        pos2(c.x, frame.bottom() + 64.0),
        Align2::CENTER_CENTER,
        "Space 播放/暂停 · ←/→ 快退/快进 · ↑/↓ 音量 · F 全屏",
        FontId::monospace(10.0),
        TEXT_3,
    );

    let mut action = if resp.clicked() { EmptyAction::Open } else { EmptyAction::None };

    // Settings link, top-right corner.
    let gear = Rect::from_center_size(pos2(full.right() - 24.0, full.top() + 24.0), vec2(30.0, 30.0));
    let gresp = ui.interact(gear, ui.scope_id().with("empty_settings"), Sense::click());
    icons::draw(&p, Icon::Settings, gear.center(), 15.0, if gresp.hovered() { ACCENT } else { TEXT_2 });
    if gresp.clicked() {
        action = EmptyAction::Settings;
    }

    // Continue watching.
    if !recent.is_empty() {
        let w = 360.0f32.min(full.width() - 40.0);
        let mut y = frame.bottom() + 100.0;
        p.text(pos2(c.x - w / 2.0, y), Align2::LEFT_CENTER, "CONTINUE", FontId::monospace(10.0), TEXT_3);
        y += 22.0;
        for (i, e) in recent.iter().enumerate() {
            let row = Rect::from_min_size(pos2(c.x - w / 2.0, y - 12.0), vec2(w, 24.0));
            let r = ui.interact(row, ui.scope_id().with(("recent", i)), Sense::click());
            let hovered = r.hovered();
            if hovered {
                p.rect_filled(row, 0.0, theme::accent_alpha(10));
                p.line_segment([row.left_top(), row.left_bottom()], Stroke::new(2.0, ACCENT));
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let time = format!("{} / {}", fmt_time(e.position), fmt_time(e.duration));
            let tg = p.layout_no_wrap(time, FontId::monospace(11.0), if hovered { ACCENT } else { TEXT_3 });
            let title_w = w - tg.size().x - 28.0;
            let title = elide(&p, &e.title, FontId::proportional(12.0), title_w);
            p.text(pos2(row.left() + 10.0, y), Align2::LEFT_CENTER, title, FontId::proportional(12.0), if hovered { TEXT } else { TEXT_2 });
            p.galley(pos2(row.right() - tg.size().x - 6.0, y - tg.size().y / 2.0), tg, TEXT);
            // Thin progress line under each entry.
            let frac = if e.duration > 0.0 { (e.position / e.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let ly = row.bottom() - 1.0;
            p.line_segment([pos2(row.left() + 10.0, ly), pos2(row.right() - 6.0, ly)], Stroke::new(1.0, LINE));
            p.line_segment(
                [pos2(row.left() + 10.0, ly), pos2(row.left() + 10.0 + frac * (row.width() - 16.0), ly)],
                Stroke::new(1.0, theme::accent_alpha(160)),
            );
            if r.clicked() {
                action = EmptyAction::Recent(e.path.clone());
            }
            y += 26.0;
        }
    }
    action
}

/// Truncates `text` with "…" so it fits in `max_w`.
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

fn draw_error(p: &egui::Painter, full: Rect, err: &str) {
    let c = full.center();
    p.text(c - vec2(0.0, 14.0), Align2::CENTER_CENTER, "播放内核加载失败", FontId::proportional(15.0), TEXT);
    p.text(c + vec2(0.0, 12.0), Align2::CENTER_CENTER, err, FontId::monospace(11.0), TEXT_2);
}

pub fn fmt_time(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    let (h, m, s) = (t / 3600, t % 3600 / 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

fn fmt_speed(s: f64) -> String {
    if (s * 10.0).fract().abs() < 1e-6 { format!("{s:.1}×") } else { format!("{s:.2}×") }
}

//! The player window: video surface, floating controls, OSD and empty state.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Key, Rect, Sense, Stroke, StrokeKind,
    ViewportCommand, pos2, vec2,
};
use eframe::glow;

use crate::ab_loop::AbLoop;
use crate::crop_detect::CropDetect;
use crate::history::Entry;
use crate::hover_thumbs::HoverThumbs;
use crate::icons::{self, Icon};
use crate::i18n::{tf, tr};
use crate::keep_awake::{KeepAwake, hold_for};
use crate::library_ui::{Action as LibAction, LibraryView};
use crate::marks::{self, Marks};
use crate::media_keys::{Cmd as MediaCmd, MediaKeys};
use crate::scanner::{Msg as ScanMsg, Scanner};
use crate::mpv::{Event, RenderContext};
use crate::player::Player;
use crate::quick_panel::{Crop, QuickPanel};
use crate::playlist::{self, LoopMode, Playlist};
use crate::session::{Resume, Tracker};
use crate::settings_panel::{self, PanelState};
use crate::theme::{self, ACCENT, TEXT, TEXT_2, TEXT_3};
use crate::video::VideoTarget;
use crate::view::{self, View, ZOOM_STEP};

const HIDE_AFTER: f64 = 2.0;
const OSD_SECS: f64 = 1.5;
/// The "continue from…" prompt disappears after this long.
const RESUME_PROMPT_SECS: f64 = 10.0;
const SIDEBAR_W: f32 = 280.0;
/// Height of the bottom control bar (seek line + button row).
const BAR_H: f32 = 64.0;
/// Below this width the control bar has no room for the optional buttons.
const WIDE_BAR: f32 = 820.0;
const AB_COLOR: Color32 = Color32::from_rgb(0xFF, 0xC1, 0x4D);
const BOOKMARK_COLOR: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x65);
const HOVER_THUMB_W: f32 = 200.0;
const PIP_MAX_SIDE: f32 = 400.0;
const PIP_MARGIN: f32 = 24.0;
/// How often interpolation re-checks the display's refresh rate.
const REFRESH_CHECK_SECS: f64 = 2.0;
const NORMAL_MIN_SIZE: [f32; 2] = [480.0, 300.0];
const PIP_MIN_SIZE: [f32; 2] = [200.0, 120.0];

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
    playlist: Playlist,
    sidebar_open: bool,
    /// Index of the playlist row being dragged.
    drag_from: Option<usize>,
    was_eof: bool,
    side_rect: Rect,
    side_scroll: f32,
    /// The sidebar shows chapters and bookmarks instead of the playlist.
    side_marks: bool,
    scroll_to_current: bool,
    speed_menu: bool,
    library: LibraryView,
    scanner: Scanner,
    /// Whether the playing file is favorited; None if it isn't in the library.
    current_fav: Option<bool>,
    /// Results of background folder scans: (playlist generation, playing file name, files).
    fill_tx: std::sync::mpsc::Sender<(u64, String, Vec<String>)>,
    fill_rx: std::sync::mpsc::Receiver<(u64, String, Vec<String>)>,
    quick: QuickPanel,
    /// Current mpv sub-margin-y (raised while the control bar shows).
    sub_margin: i32,
    instance_rx: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    /// Last normal (not fullscreen) window placement, saved on exit.
    window_place: Option<crate::WindowPlace>,
    ab: AbLoop,
    marks: Marks,
    view: View,
    crop_detect: CropDetect,
    last_refresh_check: f64,
    always_on_top: bool,
    pip: bool,
    /// Where the window goes back to when leaving picture-in-picture.
    pip_restore: Option<crate::WindowPlace>,
    hover_thumbs: HoverThumbs,
    keep_awake: KeepAwake,
    media_keys: Option<MediaKeys>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<String>, listener: Option<std::net::TcpListener>) -> Self {
        let (player, error) = match Player::new() {
            Ok(p) => {
                let ctx = cc.egui_ctx.clone();
                p.mpv.set_wakeup(Box::new(move || ctx.request_repaint()));
                (Some(p), None)
            }
            Err(e) => (None, Some(e)),
        };
        crate::mac_open::set_wake(cc.egui_ctx.clone());
        // Files handed over by later launches (single instance).
        let instance_rx = listener.map(|l| {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            crate::single_instance::serve(l, tx, move || ctx.request_repaint());
            rx
        });
        let tracker = Tracker::new();
        crate::i18n::set_language(&tracker.settings.language);
        let system = cc.egui_ctx.input(|i| i.raw.system_theme);
        let light = theme::wants_light(&tracker.settings.theme, system);
        theme::apply(&cc.egui_ctx, light);
        let (fill_tx, fill_rx) = std::sync::mpsc::channel();
        // Pick up new/changed files in watched folders on every start.
        let scanner = Scanner::spawn(cc.egui_ctx.clone());
        scanner.scan();
        let playlist = Playlist::new(tracker.settings.loop_mode, tracker.settings.shuffle);
        let hover_thumbs = HoverThumbs::spawn(cc.egui_ctx.clone(), crate::history::data_dir());
        let media_keys = MediaKeys::new(cc);
        if let Some(p) = &player {
            p.apply_settings(&tracker.settings);
        }
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
            tracker,
            resume_prompt: None,
            settings_open: false,
            panel: PanelState::default(),
            recent: Vec::new(),
            recent_dirty: true,
            playlist,
            sidebar_open: false,
            drag_from: None,
            was_eof: false,
            side_rect: Rect::NOTHING,
            side_scroll: 0.0,
            side_marks: false,
            scroll_to_current: false,
            speed_menu: false,
            library: LibraryView::new(),
            scanner,
            current_fav: None,
            fill_tx,
            fill_rx,
            quick: QuickPanel::default(),
            sub_margin: 22,
            instance_rx,
            window_place: None,
            ab: AbLoop::default(),
            marks: Marks::default(),
            view: View::default(),
            crop_detect: CropDetect::new(),
            last_refresh_check: 0.0,
            always_on_top: false,
            pip: false,
            pip_restore: None,
            hover_thumbs,
            keep_awake: KeepAwake::new(),
            media_keys,
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

    /// Follows the setting, and the OS while the setting says "system".
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let system = ctx.input(|i| i.raw.system_theme);
        let light = theme::wants_light(&self.tracker.settings.theme, system);
        if light != theme::is_light() {
            theme::set_light(ctx, light);
        }
    }

    fn apply_window_level(&self, ctx: &egui::Context) {
        let on_top = self.always_on_top || self.pip;
        let level = if on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
    }

    fn toggle_on_top(&mut self, ctx: &egui::Context) {
        self.always_on_top = !self.always_on_top;
        self.apply_window_level(ctx);
        self.show_osd(ctx, if self.always_on_top { tr("窗口置顶  开") } else { tr("窗口置顶  关") });
    }

    /// Shrinks the window to a borderless, always-on-top video in the corner
    /// of the screen, or puts it back where it was.
    fn toggle_pip(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
            self.show_osd(ctx, tr("请先退出全屏"));
            return;
        }
        self.pip = !self.pip;
        self.apply_window_level(ctx);
        if self.pip {
            self.pip_restore = self.window_place;
            self.sidebar_open = false;
            self.side_rect = Rect::NOTHING;
            self.speed_menu = false;
            self.quick.close();
            let (w, h) = self.player.as_ref().map_or((0, 0), |p| (p.state.width, p.state.height));
            let size = pip_size(w, h);
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(ViewportCommand::Decorations(false));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(PIP_MIN_SIZE.into()));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let corner = (monitor - size - vec2(PIP_MARGIN, PIP_MARGIN)).to_pos2();
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(corner));
            }
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(NORMAL_MIN_SIZE.into()));
            if let Some(p) = self.pip_restore.take() {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(p.w, p.h)));
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(p.x, p.y)));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(p.maximized));
            }
        }
    }

    fn press_ab(&mut self, ctx: &egui::Context) {
        let Some(player) = &self.player else { return };
        self.ab.press(player.state.position);
        player.set_ab_loop(self.ab);
        // B is set at or behind the playhead, so mpv would never reach it.
        if let (Some(a), Some(_)) = (self.ab.a, self.ab.b) {
            player.seek_to(a, true);
        }
        let text = match (self.ab.a, self.ab.b) {
            (Some(a), None) => format!("A  {}", fmt_time(a)),
            (Some(a), Some(b)) => tf("A-B 循环  {} – {}", &[&fmt_time(a), &fmt_time(b)]),
            _ => tr("A-B 循环  已清除").to_string(),
        };
        self.show_osd(ctx, text);
    }

    fn apply_view(&mut self, ctx: &egui::Context, osd: bool) {
        if let Some(p) = &self.player {
            p.set_view(self.view);
        }
        if osd {
            self.show_osd(ctx, tf("缩放  {}%", &[&self.view.percent()]));
        }
    }

    fn apply_crop(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.player else { return };
        match self.quick.crop {
            Crop::Off => p.set_crop(None),
            Crop::Ratio(r) => {
                let area = view::crop_to_ratio(p.state.width, p.state.height, r);
                p.set_crop(area.as_deref());
                if area.is_none() {
                    self.show_osd(ctx, tr("画面已经是这个比例"));
                }
            }
            Crop::Auto if p.state.path.contains("://") => self.show_osd(ctx, tr("只有本地文件可以去黑边")),
            Crop::Auto => {
                self.crop_detect.start(ctx, p.state.path.clone());
                self.show_osd(ctx, tr("正在检测黑边…"));
            }
        }
    }

    fn on_crop_detected(&mut self, ctx: &egui::Context, path: &str, area: Option<crate::crop_detect::Area>) {
        let Some(p) = &self.player else { return };
        // The viewer may have moved on while the detector ran.
        if path != p.state.path || self.quick.crop != Crop::Auto {
            return;
        }
        let frame = (p.state.width, p.state.height);
        match area.and_then(|a| view::crop_from_detected(frame, a)) {
            Some(crop) => {
                p.set_crop(Some(&crop));
                self.show_osd(ctx, tr("已去除黑边"));
            }
            None => {
                self.quick.crop = Crop::Off;
                self.show_osd(ctx, tr("没有检测到黑边"));
            }
        }
    }

    fn jump_chapter(&mut self, ctx: &egui::Context, forward: bool) {
        let Some(player) = &self.player else { return };
        if self.marks.chapters.is_empty() {
            self.show_osd(ctx, tr("这个文件没有章节"));
            return;
        }
        let pos = player.state.position;
        let target = if forward { self.marks.next_chapter(pos) } else { self.marks.prev_chapter(pos) };
        let Some(i) = target else {
            self.show_osd(ctx, if forward { tr("已是最后一章") } else { tr("已是第一章") });
            return;
        };
        let chapter = &self.marks.chapters[i];
        player.seek_to(chapter.time, true);
        let text = format!("{} / {}  {}", i + 1, self.marks.chapters.len(), chapter.title);
        self.show_osd(ctx, text);
    }

    fn add_bookmark(&mut self, ctx: &egui::Context) {
        let Some(player) = &self.player else { return };
        let (pos, path) = (player.state.position, player.state.path.clone());
        let Some(key) = self.marks.key.clone() else {
            self.show_osd(ctx, tr("只有本地文件可以添加书签"));
            return;
        };
        if self.tracker.incognito {
            self.show_osd(ctx, tr("无痕模式下不保存书签"));
            return;
        }
        if self.marks.has_bookmark_near(pos) {
            self.show_osd(ctx, tr("这里已有书签"));
            return;
        }
        let Some(store) = &self.tracker.store else { return };
        let label = self.marks.chapter_at(pos).map_or("", |i| self.marks.chapters[i].title.as_str());
        store.add_bookmark(&key, &path, pos, label);
        self.marks.bookmarks = store.bookmarks(&key);
        self.show_osd(ctx, tf("已添加书签  {}", &[&fmt_time(pos)]));
    }

    /// Opens the sidebar on the playlist or the marks, or closes it if it's
    /// already showing that.
    fn show_side(&mut self, marks: bool) {
        if !self.sidebar_open || self.side_marks == marks {
            self.toggle_sidebar();
        }
        self.side_marks = marks;
    }

    fn handle_media_key(&mut self, ctx: &egui::Context, cmd: MediaCmd) {
        match cmd {
            MediaCmd::Next => self.play_next(ctx),
            MediaCmd::Prev => self.play_prev(ctx),
            MediaCmd::Toggle | MediaCmd::Play | MediaCmd::Pause | MediaCmd::Stop => {
                let Some(player) = &self.player else { return };
                match cmd {
                    MediaCmd::Toggle => player.toggle_pause(),
                    MediaCmd::Play => player.mpv.set_flag("pause", false),
                    MediaCmd::Pause => player.mpv.set_flag("pause", true),
                    _ => player.stop(),
                }
            }
        }
    }

    fn open_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter(tr("媒体文件"), playlist::MEDIA_EXTS)
            .add_filter(tr("所有文件"), &["*"])
            .pick_files()
        {
            self.pending_open
                .extend(files.into_iter().map(|p| p.to_string_lossy().into_owned()));
        }
    }

    fn add_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title(tr("添加到媒体库")).pick_folder() {
            if self.library.add_folder(&dir.to_string_lossy()) {
                self.scanner.scan();
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let Some(player) = &self.player else { return };
        let (dropped, keys, cmd, shift, scroll, moved) = ctx.input(|i| {
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
            (dropped, keys, i.modifiers.command, i.modifiers.shift, scroll, i.pointer.delta() != egui::Vec2::ZERO)
        });

        let now = ctx.input(|i| i.time);
        if moved || !keys.is_empty() || scroll != 0.0 {
            self.last_activity = now;
        }
        self.pending_open.extend(dropped);
        if ctx.egui_wants_keyboard_input() {
            return;
        }

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let s = player.state.clone();
        let mut osd: Option<String> = None;
        let mut open_dialog = false;
        let (mut want_next, mut want_prev, mut want_side) = (false, false, false);
        let mut want_fav = false;
        let (mut want_ab, mut want_on_top, mut want_pip) = (false, false, false);
        let (mut want_bookmark, mut want_marks) = (false, false);
        let mut want_zoom: Option<f64> = None;
        let mut want_view_reset = false;
        let mut want_chapter: Option<bool> = None;

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
                Key::D if cmd && !s.idle => want_fav = true,
                Key::Home if !s.idle => {
                    player.seek_to(0.0, true);
                    self.resume_prompt = None;
                    osd = Some("SEEK  00:00".into());
                }
                Key::Space | Key::K => {
                    player.toggle_pause();
                    osd = Some(if s.paused || s.eof { "PLAY".into() } else { "PAUSE".into() });
                }
                Key::ArrowLeft | Key::ArrowRight if shift && !s.idle => want_chapter = Some(key == Key::ArrowRight),
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
                Key::Escape if self.pip => want_pip = true,
                Key::Escape if fullscreen => {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                Key::OpenBracket | Key::CloseBracket | Key::OpenCurlyBracket | Key::CloseCurlyBracket => {
                    let dir = if matches!(key, Key::OpenBracket | Key::OpenCurlyBracket) { -1 } else { 1 };
                    let sp = if shift { player.nudge_speed(dir) } else { player.step_speed(dir) };
                    osd = Some(format!("SPEED  {}", fmt_speed(sp)));
                }
                Key::Comma | Key::Period if !cmd && !s.idle => {
                    let forward = key == Key::Period;
                    player.frame_step(forward);
                    osd = Some(if forward { tr("下一帧").into() } else { tr("上一帧").into() });
                }
                Key::Equals | Key::Plus if !cmd && !s.idle => want_zoom = Some(ZOOM_STEP),
                Key::Minus if !cmd && !s.idle => want_zoom = Some(-ZOOM_STEP),
                Key::Num0 if !cmd && !s.idle => want_view_reset = true,
                Key::B if cmd && !s.idle => want_bookmark = true,
                Key::B if !s.idle => want_ab = true,
                Key::G if !cmd && !s.idle && !self.pip => want_marks = true,
                Key::T if !cmd => want_on_top = true,
                Key::I if !cmd && !s.idle => want_pip = true,
                Key::Backspace => {
                    player.set_speed(1.0);
                    osd = Some("SPEED  1.0×".into());
                }
                Key::C => {
                    player.cycle_subtitles();
                    osd = Some("SUBTITLE  NEXT".into());
                }
                Key::PageDown | Key::N => want_next = true,
                Key::PageUp | Key::P => want_prev = true,
                Key::L if !self.pip => want_side = true,
                Key::S if cmd => {
                    player.screenshot();
                    osd = Some("SCREENSHOT  → DESKTOP".into());
                }
                _ => {}
            }
        }

        // Over the sidebar the wheel scrolls the list instead of changing volume.
        let over_side = self.sidebar_open
            && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| self.side_rect.contains(p));
        if over_side {
            self.side_scroll -= scroll;
        } else if scroll != 0.0 && !s.idle && cmd {
            want_zoom = Some(scroll.signum() as f64 * ZOOM_STEP);
        } else if scroll != 0.0 && !s.idle {
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
        if want_next {
            self.play_next(ctx);
        }
        if want_prev {
            self.play_prev(ctx);
        }
        if want_side {
            self.show_side(false);
        }
        if want_marks {
            self.show_side(true);
        }
        if let Some(forward) = want_chapter {
            self.jump_chapter(ctx, forward);
        }
        if want_bookmark {
            self.add_bookmark(ctx);
        }
        if want_fav {
            self.toggle_favorite(ctx);
        }
        if want_ab {
            self.press_ab(ctx);
        }
        if let Some(d) = want_zoom {
            self.view.zoom_by(d);
            self.apply_view(ctx, true);
        }
        if want_view_reset {
            self.view = View::default();
            self.apply_view(ctx, true);
        }
        if want_on_top {
            self.toggle_on_top(ctx);
        }
        if want_pip {
            self.toggle_pip(ctx);
        }
    }

    /// Replaces the queue with whatever was opened/dropped and starts playing
    /// right away. The rest of the folder is scanned on a background thread
    /// and merged into the queue when ready, so opening never waits on disk.
    fn flush_open(&mut self, ctx: &egui::Context) {
        if self.pending_open.is_empty() {
            return;
        }
        let paths: Vec<String> = self.pending_open.drain(..).collect();
        let fill = self.tracker.settings.fill_folder;
        if let Some(i) = self.playlist.set_files(&paths, fill) {
            self.play_index(i);
        }
        if let Some((generation, dir, name)) = self.playlist.take_pending_fill() {
            let tx = self.fill_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("folder-fill".into())
                .spawn(move || {
                    let files = playlist::scan_dir(&dir);
                    let _ = tx.send((generation, name, files));
                    ctx.request_repaint();
                })
                .ok();
        }
    }

    /// Queues exactly `paths` (no folder auto-fill) and starts the first.
    fn play_all(&mut self, paths: &[String]) {
        if let Some(i) = self.playlist.set_files(paths, false) {
            self.play_index(i);
        }
    }

    /// Ctrl+D / the player's star: favorite the current file.
    fn toggle_favorite(&mut self, ctx: &egui::Context) {
        let Some(path) = self.player.as_ref().map(|p| p.state.path.clone()) else { return };
        match self.library.toggle_favorite_path(&path) {
            Some(on) => {
                self.current_fav = Some(on);
                self.show_osd(ctx, if on { tr("★ 已收藏") } else { tr("已取消收藏") });
            }
            None => self.show_osd(ctx, tr("只有媒体库里的视频可以收藏")),
        }
    }

    fn play_index(&mut self, i: usize) {
        if let (Some(player), Some(item)) = (&self.player, self.playlist.items.get(i)) {
            player.open(&item.path, false);
            self.playlist.select(i);
            self.scroll_to_current = true;
        }
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_open = !self.sidebar_open;
        self.scroll_to_current = self.sidebar_open;
        if !self.sidebar_open {
            self.side_rect = Rect::NOTHING;
        }
    }

    fn play_next(&mut self, ctx: &egui::Context) {
        match self.playlist.next(false) {
            Some(i) => self.play_index(i),
            None => self.show_osd(ctx, "LAST ITEM"),
        }
    }

    fn play_prev(&mut self, ctx: &egui::Context) {
        match self.playlist.prev() {
            Some(i) => self.play_index(i),
            None => self.show_osd(ctx, "FIRST ITEM"),
        }
    }

    /// Called when the current file reaches its end.
    fn on_eof(&mut self) {
        let cur = self.playlist.current;
        match self.playlist.next(true) {
            Some(i) if Some(i) == cur => {
                // Repeat one: rewind instead of reloading.
                if let Some(p) = &self.player {
                    p.seek_to(0.0, true);
                    p.mpv.set_flag("pause", false);
                }
            }
            Some(i) => self.play_index(i),
            None => {}
        }
    }

    /// Keeps loop/shuffle on the queue in sync with saved settings.
    fn sync_playlist_settings(&mut self) {
        let s = &self.tracker.settings;
        self.playlist.loop_mode = s.loop_mode;
        if self.playlist.shuffle() != s.shuffle {
            self.playlist.set_shuffle(s.shuffle);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_theme(&ctx);
        self.ensure_gl(&ctx, frame);
        self.handle_input(&ctx);
        self.flush_open(&ctx);

        let full = ctx.content_rect();
        // Remember where the window is (skip fullscreen/minimized states).
        ctx.input(|i| {
            let vp = i.viewport();
            if !self.pip && vp.fullscreen != Some(true) && vp.minimized != Some(true) {
                if let (Some(outer), Some(inner)) = (vp.outer_rect, vp.inner_rect) {
                    let maximized = vp.maximized == Some(true);
                    let prev = self.window_place;
                    self.window_place = Some(if maximized {
                        // Keep the restored size from before maximizing.
                        crate::WindowPlace { maximized: true, ..prev.unwrap_or(crate::WindowPlace::from_rects(outer, inner, true)) }
                    } else {
                        crate::WindowPlace::from_rects(outer, inner, false)
                    });
                }
            }
        });
        let painter = ui.painter().clone();
        let playing = self.player.as_ref().is_some_and(|p| !p.state.idle);
        painter.rect_filled(full, 0.0, if playing { Color32::BLACK } else { theme::bg() });
        // The playlist sidebar floats over the video (the video never resizes);
        // only the chrome (bars, OSD, prompts) moves aside to stay visible.
        let side_t = ctx.animate_bool_with_time(ui.scope_id().with("sidebar"), self.sidebar_open, 0.18);
        let side_w = (SIDEBAR_W * side_t).min(full.width() * 0.5).round();
        let stage = Rect::from_min_max(full.min, pos2(full.right() - side_w, full.bottom()));

        if let Some(err) = &self.error {
            draw_error(&painter, full, err);
            return;
        }
        let Some(player) = &self.player else { return };
        let s = player.state.clone();

        // The window title stays "TinyPlayer"; the file name is shown in the
        // in-app top bar instead. Still track the media title to detect a
        // newly loaded file.
        let media = if s.idle { String::new() } else { s.title.clone() };
        if media != self.shown_title {
            // New media: show the controls briefly so the user sees what loaded.
            self.last_activity = ctx.input(|i| i.time);
            self.shown_title = media;
        }

        // ---- Video ----
        // In picture-in-picture there's no title bar, so the video drags the window.
        // A zoomed picture is dragged around instead.
        let draggable = self.pip || self.view.zoomed();
        let sense = if draggable { Sense::click_and_drag() } else { Sense::click() };
        let surface = ui.interact(full, ui.scope_id().with("surface"), sense);
        if self.pip && surface.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        } else if !self.pip && self.view.zoomed() {
            if surface.dragged() {
                let fit = view::fit_size((s.width as f64, s.height as f64), (full.width() as f64, full.height() as f64));
                let d = surface.drag_delta();
                let scale = self.view.scale();
                self.view.pan_by(d.x as f64 / (fit.0 * scale), d.y as f64 / (fit.1 * scale));
                self.apply_view(&ctx, false);
            }
            if surface.hovered() {
                ctx.set_cursor_icon(if surface.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
            }
        }
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
        if self.pip && (s.idle || surface.double_clicked()) {
            self.toggle_pip(&ctx);
        } else if surface.double_clicked() && !s.idle {
            let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fs));
        }

        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

        if s.idle {
            self.quick.close();
        }
        if s.idle && self.library.has_folders() {
            match self.library.show(ui, stage) {
                LibAction::Play(path) => self.pending_open.push(path),
                LibAction::PlayAll(paths) => self.play_all(&paths),
                LibAction::AddFolder => self.add_folder_dialog(),
                LibAction::OpenFile => self.open_dialog(),
                LibAction::Settings => self.settings_open = true,
                LibAction::None => {}
            }
        } else if s.idle {
            match draw_empty_state(ui, stage, &self.recent) {
                EmptyAction::Open => self.open_dialog(),
                EmptyAction::AddFolder => self.add_folder_dialog(),
                EmptyAction::Recent(path) => self.pending_open.push(path),
                EmptyAction::Settings => self.settings_open = true,
                EmptyAction::None => {}
            }
        } else {
            let now = ctx.input(|i| i.time);
            let pointer_in_bar = ctx
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| p.y > stage.bottom() - BAR_H - 30.0 || p.y < stage.top() + 50.0);
            let visible = s.paused
                || now - self.last_activity < HIDE_AFTER
                || pointer_in_bar
                || self.seek_preview.is_some()
                || self.speed_menu
                || self.quick.open;
            let alpha = ctx.animate_bool_with_time(ui.scope_id().with("chrome"), visible, 0.15);
            // Lift subtitles above the control bar while it's showing.
            // sub-margin-y is in mpv's 720-line script units; 22 is mpv's default.
            let margin = if visible { 22 + (BAR_H / stage.height().max(1.0) * 720.0).round() as i32 } else { 22 };
            if margin != self.sub_margin {
                if let Some(p) = &self.player {
                    p.mpv.set_property_string("sub-margin-y", &margin.to_string());
                }
                self.sub_margin = margin;
            }
            if !visible && surface.hovered() {
                ctx.set_cursor_icon(CursorIcon::None);
            }
            if visible && !s.paused {
                ctx.request_repaint_after_secs((HIDE_AFTER - (now - self.last_activity)).max(0.05) as f32);
            }

            if s.paused && !s.eof && !self.pip {
                icons::draw(&painter, Icon::Play, full.center(), 40.0, Color32::from_white_alpha(220));
            }
            if self.pip {
                if alpha > 0.0 {
                    self.draw_pip_controls(ui, full, alpha);
                }
            } else if alpha > 0.0 {
                draw_top_bar(&painter, stage, &s, self.tracker.current_play_count(), alpha);
                if let Some(fav) = self.current_fav {
                    if fav_button(ui, stage, &s.title, fav, alpha) {
                        let ctx = ui.ctx().clone();
                        self.toggle_favorite(&ctx);
                    }
                }
                if back_button(ui, stage, alpha) {
                    // Stopping returns to the library; progress is saved on EndFile.
                    if let Some(p) = &self.player {
                        p.stop();
                    }
                }
                self.draw_controls(ui, stage, alpha);
            }
            self.draw_resume_prompt(ui, stage, now);

            if self.quick.open {
                if let Some(player) = &self.player {
                    let act = self.quick.show(ui, stage, BAR_H, player, &mut self.tracker.settings, self.view);
                    if act.settings_changed {
                        self.tracker.save_settings();
                        player.apply_settings(&self.tracker.settings);
                    }
                    if act.load_subtitle {
                        if let Some(file) = rfd::FileDialog::new()
                            .set_title(tr("加载字幕文件"))
                            .add_filter(tr("字幕"), &["srt", "ass", "ssa", "vtt", "sub", "sup", "idx"])
                            .pick_file()
                        {
                            let f = file.to_string_lossy().into_owned();
                            let _ = player.mpv.command(&["sub-add", &f, "select"]);
                            self.osd = Some(("SUBTITLE  LOADED".into(), now, OSD_SECS));
                        }
                    }
                    if act.more_settings {
                        self.settings_open = true;
                    }
                    if act.crop_changed {
                        self.apply_crop(&ctx);
                    }
                    if let Some(d) = act.zoom {
                        self.view.zoom_by(d);
                        self.apply_view(&ctx, false);
                    }
                    if act.view_reset {
                        self.view = View::default();
                        self.apply_view(&ctx, false);
                    }
                }
            }

            // Edge handle: "‹" while paused opens the playlist, "›" closes it.
            let show_handle = !self.pip && (self.sidebar_open || s.paused);
            let ha = ctx.animate_bool_with_time(ui.scope_id().with("edge_handle"), show_handle, 0.15);
            if ha > 0.0 && self.drag_from.is_none() {
                if draw_edge_handle(ui, stage, self.sidebar_open, ha) {
                    self.toggle_sidebar();
                }
            }
        }

        // ---- OSD ----
        if let Some((text, at, life)) = &self.osd {
            let now = ctx.input(|i| i.time);
            let age = now - at;
            if age < *life {
                let a = ((life - age) / 0.2).min(1.0) as f32;
                draw_osd(&painter, stage, text, a);
                ctx.request_repaint_after_secs(0.05);
            } else {
                self.osd = None;
            }
        }

        if side_w > 0.5 {
            let side = Rect::from_min_max(pos2(stage.right(), full.top()), full.max);
            self.draw_sidebar(ui, side, side_t);
        }

        if hovering_files {
            let r = full.shrink(12.0);
            painter.rect_filled(full, 0.0, theme::scrim(150));
            icons::corner_brackets(&painter, r, 24.0, Stroke::new(1.5, ACCENT));
            painter.text(full.center(), Align2::CENTER_CENTER, tr("松开以播放"), FontId::proportional(16.0), TEXT);
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
                crate::i18n::set_language(&self.tracker.settings.language);
                self.sync_playlist_settings();
                if let Some(p) = &self.player {
                    p.apply_settings(&self.tracker.settings);
                }
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
                    self.current_fav = self.library.is_favorite_path(&player.state.path);
                    self.ab = AbLoop::default();
                    player.set_ab_loop(self.ab);
                    self.view = View::default();
                    player.set_view(self.view);
                    self.quick.crop = Crop::Off;
                    player.set_crop(None);
                    let key = crate::history::file_key(&player.state.path);
                    let bookmarks = match (&key, &self.tracker.store) {
                        (Some(k), Some(st)) => st.bookmarks(k),
                        _ => Vec::new(),
                    };
                    self.marks = Marks { key, chapters: player.chapters(), bookmarks };
                    match self.tracker.file_loaded(&player.state) {
                        Resume::Resumed(pos) => {
                            player.seek_to(pos, true);
                            let msg = tf("已从 {} 继续 · Home 从头", &[&fmt_time(pos)]);
                            self.osd = Some((msg, now, 3.5));
                        }
                        Resume::Ask(pos) => self.resume_prompt = Some((pos, now)),
                        Resume::None => {}
                    }
                }
                Event::EndFile => {
                    self.tracker.finish();
                    self.recent_dirty = true;
                    // Progress and play counts shown on cards may have changed.
                    self.library.mark_dirty();
                }
                _ => {}
            }
        }
        let opened = crate::mac_open::take();
        if !opened.is_empty() {
            self.pending_open.extend(opened);
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        if let Some(rx) = &self.instance_rx {
            while let Ok(files) = rx.try_recv() {
                self.pending_open.extend(files);
                ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
        }
        while let Ok((generation, name, files)) = self.fill_rx.try_recv() {
            if self.playlist.merge_folder(generation, &name, files) {
                self.scroll_to_current = true;
            }
        }
        while let Ok(msg) = self.scanner.rx.try_recv() {
            match msg {
                ScanMsg::Changed => self.library.mark_dirty(),
                ScanMsg::Probing(done, total) => self.library.probing = Some((done, total)),
                ScanMsg::Idle => self.library.probing = None,
            }
        }
        self.hover_thumbs.receive(ctx);
        if now - self.last_refresh_check > REFRESH_CHECK_SECS {
            self.last_refresh_check = now;
            player.sync_refresh();
        }
        self.tracker.tick(&player.state);
        let st = &player.state;
        self.keep_awake.set(hold_for(!st.idle && !st.paused && !st.eof, st.width > 0));
        if let Some(keys) = &mut self.media_keys {
            keys.publish(st);
        }
        let eof = player.state.eof && !player.state.idle;
        let eof_edge = eof && !self.was_eof;
        self.was_eof = eof;
        let idle = player.state.idle;
        if eof_edge {
            self.on_eof();
        }
        if idle && self.recent_dirty {
            self.recent = self.tracker.recent(5);
            self.recent_dirty = false;
        }
        while let Ok((path, area)) = self.crop_detect.rx.try_recv() {
            self.on_crop_detected(ctx, &path, area);
        }
        let media: Vec<MediaCmd> = self.media_keys.as_ref().map_or(Vec::new(), |k| k.rx.try_iter().collect());
        for cmd in media {
            self.handle_media_key(ctx, cmd);
        }
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        if let (Some(place), Some(store)) = (self.window_place, &self.tracker.store) {
            store.set_value("window", &place.encode());
        }
        self.scanner.quit();
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
    /// Bottom control bar: a full-width seek line on top, then
    /// time on the left, transport + volume in the middle, tools on the right.
    fn draw_controls(&mut self, ui: &mut egui::Ui, chrome: Rect, alpha: f32) {
        let Some(player) = &self.player else { return };
        let s = player.state.clone();
        let painter = ui.painter().clone();
        let a = |c: Color32| c.gamma_multiply(alpha);
        let ctx = ui.ctx().clone();

        let bar = Rect::from_min_max(pos2(chrome.left(), chrome.bottom() - BAR_H), chrome.max);
        // Soft fade above the bar, then a solid band so text stays readable.
        let fade = Rect::from_min_max(pos2(bar.left(), bar.top() - 28.0), pos2(bar.right(), bar.top()));
        painter.rect_filled(fade, 0.0, theme::scrim((70.0 * alpha) as u8));
        painter.rect_filled(bar, 0.0, theme::scrim((215.0 * alpha) as u8));

        let mut osd: Option<String> = None;
        let mut want_prev = false;
        let mut want_next = false;
        let mut want_stop = false;
        let mut open_settings = false;
        let mut toggle_list = false;
        let mut want_pip = false;

        let full_top = chrome.top();

        // ---- Seek line ----
        let line_y = bar.top() + 3.0;
        let track = Rect::from_min_max(pos2(bar.left(), line_y - 8.0), pos2(bar.right(), line_y + 8.0));
        let cur = self.seek_preview.map_or(s.position, |f| f * s.duration);
        if s.duration > 0.0 {
            let tresp = ui.interact(track, ui.scope_id().with("track"), Sense::click_and_drag());
            let hot = tresp.hovered() || tresp.dragged();
            let thick = if hot { 5.0 } else { 3.0 };
            let at = |f: f64| track.left() + (f.clamp(0.0, 1.0) as f32) * track.width();
            let seg = |from: f32, to: f32, c: Color32| {
                if to > from {
                    painter.line_segment([pos2(from, line_y), pos2(to, line_y)], Stroke::new(thick, a(c)));
                }
            };
            let frac = cur / s.duration;
            seg(track.left(), track.right(), Color32::from_white_alpha(38));
            seg(track.left(), at(s.cached_until / s.duration), Color32::from_white_alpha(80));
            seg(track.left(), at(frac), ACCENT);
            if let Some(a_pos) = self.ab.a {
                let ax = at(a_pos / s.duration);
                if let Some(b_pos) = self.ab.b {
                    seg(ax, at(b_pos / s.duration), AB_COLOR);
                }
                for t in [Some(a_pos), self.ab.b].into_iter().flatten() {
                    let x = at(t / s.duration);
                    painter.line_segment([pos2(x, line_y - 6.0), pos2(x, line_y + 6.0)], Stroke::new(2.0, a(AB_COLOR)));
                }
            }
            knob(&painter, pos2(at(frac), line_y), if hot { 7.0 } else { 6.0 }, alpha);

            for t in self.marks.chapters.iter().map(|c| c.time).filter(|t| *t > 0.0) {
                let x = at(t / s.duration);
                painter.line_segment([pos2(x, line_y - 3.0), pos2(x, line_y + 3.0)], Stroke::new(1.5, a(Color32::BLACK)));
            }
            for b in &self.marks.bookmarks {
                painter.circle_filled(pos2(at(b.time / s.duration), line_y), 3.5, a(BOOKMARK_COLOR));
            }

            let pointer_frac = tresp
                .hover_pos()
                .or(tresp.interact_pointer_pos())
                .map(|p| ((p.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64);
            if let Some(f) = pointer_frac {
                if hot {
                    let time = fmt_time(f * s.duration);
                    let label = match self.marks.chapter_at(f * s.duration) {
                        Some(i) => format!("{}  {time}", self.marks.chapters[i].title),
                        None => time,
                    };
                    let g = painter.layout_no_wrap(label, FontId::monospace(11.0), TEXT);
                    let x = at(f).clamp(track.left() + g.size().x / 2.0 + 6.0, track.right() - g.size().x / 2.0 - 6.0);
                    let r = Rect::from_center_size(pos2(x, line_y - 20.0), g.size() + vec2(12.0, 6.0));
                    painter.rect_filled(r, 4.0, theme::scrim(225));
                    painter.galley(r.min + vec2(6.0, 3.0), g, TEXT);
                    ctx.set_cursor_icon(CursorIcon::PointingHand);
                    if let Some(tex) = self.hover_thumbs.lookup(&s.path, f * s.duration, s.duration) {
                        let size = tex.size_vec2() * (HOVER_THUMB_W / tex.size_vec2().x);
                        let half = size.x / 2.0 + 6.0;
                        let tx = at(f).clamp(track.left() + half, track.right() - half);
                        let thumb = Rect::from_center_size(pos2(tx, r.top() - 6.0 - size.y / 2.0), size);
                        painter.image(tex.id(), thumb, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        painter.rect_stroke(thumb, 3.0, Stroke::new(1.0, Color32::from_white_alpha(70)), StrokeKind::Outside);
                    }
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

        let cy = bar.top() + 37.0;
        let button = |ui: &mut egui::Ui, center: egui::Pos2, id: &str, icon: Icon, size: f32, enabled: bool| -> bool {
            let rect = Rect::from_center_size(center, vec2(34.0, 34.0));
            let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
            let hovered = enabled && resp.hovered();
            let color = if !enabled {
                TEXT_3
            } else if hovered {
                ACCENT
            } else {
                TEXT
            };
            icons::draw(&painter, icon, center, size, a(color));
            if hovered {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            enabled && resp.clicked()
        };

        // ---- Left: time ----
        let cur_g = painter.layout_no_wrap(fmt_time(cur), FontId::monospace(13.0), a(TEXT));
        let cur_w = cur_g.size().x;
        painter.galley(pos2(bar.left() + 18.0, cy - cur_g.size().y / 2.0), cur_g, TEXT);
        painter.text(
            pos2(bar.left() + 18.0 + cur_w + 6.0, cy),
            Align2::LEFT_CENTER,
            format!("/  {}", fmt_time(s.duration)),
            FontId::monospace(13.0),
            a(TEXT_2),
        );

        // ---- Middle: stop, prev, play, next, volume ----
        let group_w = 36.0 * 4.0 + 12.0 + 30.0 + 84.0;
        let mut x = bar.center().x - group_w / 2.0;
        // Keep clear of the time on narrow windows.
        x = x.max(bar.left() + 200.0);
        if button(ui, pos2(x + 18.0, cy), "stop", Icon::Stop, 15.0, true) {
            want_stop = true;
        }
        x += 36.0;
        if button(ui, pos2(x + 18.0, cy), "prev", Icon::Prev, 15.0, self.playlist.has_prev()) {
            want_prev = true;
        }
        x += 36.0;
        let pp_icon = if s.paused || s.eof { Icon::Play } else { Icon::Pause };
        if button(ui, pos2(x + 18.0, cy), "pp", pp_icon, 26.0, true) {
            player.toggle_pause();
        }
        x += 36.0;
        if button(ui, pos2(x + 18.0, cy), "next", Icon::Next, 15.0, self.playlist.has_next()) {
            want_next = true;
        }
        x += 36.0 + 12.0;
        let vol_icon = if s.muted || s.volume <= 0.0 { Icon::Muted } else { Icon::Volume };
        if button(ui, pos2(x + 15.0, cy), "mute", vol_icon, 17.0, true) {
            player.toggle_mute();
        }
        x += 32.0;
        let vol = Rect::from_min_size(pos2(x, cy - 10.0), vec2(80.0, 20.0));
        let vresp = ui.interact(vol, ui.scope_id().with("vol"), Sense::click_and_drag());
        let vol_frac = if s.muted { 0.0 } else { (s.volume / 100.0).clamp(0.0, 1.0) as f32 };
        let vx = vol.left() + vol_frac * vol.width();
        painter.line_segment([pos2(vol.left(), cy), pos2(vol.right(), cy)], Stroke::new(3.0, a(Color32::from_white_alpha(45))));
        painter.line_segment([pos2(vol.left(), cy), pos2(vx, cy)], Stroke::new(3.0, a(ACCENT)));
        knob(&painter, pos2(vx, cy), 5.0, alpha);
        if let Some(p) = vresp.interact_pointer_pos() {
            if vresp.dragged() || vresp.clicked() {
                let v = ((p.x - vol.left()) / vol.width()).clamp(0.0, 1.0) as f64 * 100.0;
                player.set_volume(v);
                osd = Some(format!("VOLUME  {}%", v.round()));
            }
        }
        if vresp.hovered() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }

        // ---- Right: CC, speed, playlist, settings, fullscreen ----
        let mut rx = bar.right() - 14.0 - 18.0;
        let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if button(ui, pos2(rx, cy), "fs", if fs { Icon::ExitFullscreen } else { Icon::Fullscreen }, 16.0, true) {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fs));
        }
        rx -= 40.0;
        if bar.width() >= WIDE_BAR {
            if button(ui, pos2(rx, cy), "pip", Icon::PictureInPicture, 17.0, true) {
                want_pip = true;
            }
            rx -= 40.0;
        }
        if button(ui, pos2(rx, cy), "settings", Icon::Settings, 17.0, true) {
            open_settings = true;
        }
        rx -= 40.0;
        if button(ui, pos2(rx, cy), "playlist", Icon::List, 17.0, true) {
            toggle_list = true;
        }

        // Speed: outlined pill that opens a small menu.
        rx -= 50.0;
        let pill = Rect::from_center_size(pos2(rx, cy), vec2(54.0, 24.0));
        let presp = ui.interact(pill, ui.scope_id().with("speed"), Sense::click());
        let pc = if presp.hovered() || self.speed_menu { ACCENT } else { TEXT_2 };
        painter.rect_stroke(pill, 12.0, Stroke::new(1.0, a(pc)), StrokeKind::Inside);
        let label = if (s.speed - 1.0).abs() < 0.01 { tr("倍速").to_string() } else { fmt_speed(s.speed) };
        painter.text(pill.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), a(if presp.hovered() { ACCENT } else { TEXT }));
        if presp.hovered() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if presp.clicked() {
            self.speed_menu = !self.speed_menu;
        }

        // Room for the English label, which is wider than the pill's gap.
        rx -= 60.0;
        let cc = Rect::from_center_size(pos2(rx, cy), vec2(52.0, 34.0));
        let ccresp = ui.interact(cc, ui.scope_id().with("cc"), Sense::click());
        painter.text(cc.center(), Align2::CENTER_CENTER, tr("字幕"), FontId::proportional(12.0), a(if ccresp.hovered() { ACCENT } else { TEXT }));
        if ccresp.clicked() {
            player.cycle_subtitles();
            osd = Some("SUBTITLE  NEXT".into());
        }

        // Speed menu, opening upward from the pill.
        if self.speed_menu {
            let speeds: Vec<f64> = crate::player::SPEEDS.iter().rev().copied().collect();
            // Fits the menu above the pill on short windows.
            let item_h = ((pill.top() - full_top - 24.0) / speeds.len() as f32).clamp(16.0, 26.0);
            let menu = Rect::from_min_size(
                pos2(pill.center().x - 36.0, pill.top() - 10.0 - item_h * speeds.len() as f32 - 8.0),
                vec2(72.0, item_h * speeds.len() as f32 + 8.0),
            );
            painter.rect_filled(menu, 8.0, Color32::from_rgba_unmultiplied(0x16, 0x1A, 0x20, 245));
            painter.rect_stroke(menu, 8.0, Stroke::new(0.5, Color32::from_white_alpha(30)), StrokeKind::Inside);
            let _blocker = ui.interact(menu, ui.scope_id().with("speed_menu"), Sense::click());
            for (i, sp) in speeds.iter().enumerate() {
                let r = Rect::from_min_size(pos2(menu.left() + 4.0, menu.top() + 4.0 + i as f32 * item_h), vec2(64.0, item_h));
                let resp = ui.interact(r, ui.scope_id().with(("speed_item", i)), Sense::click());
                let on = (sp - s.speed).abs() < 0.01;
                if resp.hovered() {
                    painter.rect_filled(r, 5.0, Color32::from_white_alpha(14));
                    ctx.set_cursor_icon(CursorIcon::PointingHand);
                }
                painter.text(r.center(), Align2::CENTER_CENTER, fmt_speed(*sp), FontId::monospace(12.0), if on { ACCENT } else { TEXT });
                if resp.clicked() {
                    player.set_speed(*sp);
                    osd = Some(format!("SPEED  {}", fmt_speed(*sp)));
                    self.speed_menu = false;
                }
            }
            // Any click elsewhere closes the menu.
            let clicked_outside = ctx.input(|i| i.pointer.any_click())
                && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !menu.contains(p) && !pill.contains(p));
            if clicked_outside {
                self.speed_menu = false;
            }
        }

        if let Some(t) = osd {
            self.show_osd(&ctx, t);
        }
        if open_settings {
            self.quick.toggle();
        }
        if toggle_list {
            self.toggle_sidebar();
        }
        if want_stop {
            if let Some(p) = &self.player {
                p.stop();
            }
        }
        if want_prev {
            self.play_prev(&ctx);
        }
        if want_next {
            self.play_next(&ctx);
        }
        if want_pip {
            self.toggle_pip(&ctx);
        }
    }

    /// Picture-in-picture only has room for play/pause and the way back.
    fn draw_pip_controls(&mut self, ui: &mut egui::Ui, full: Rect, alpha: f32) {
        let Some(player) = &self.player else { return };
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, theme::scrim((90.0 * alpha) as u8));
        let color = Color32::WHITE.gamma_multiply(alpha);

        let play = Rect::from_center_size(full.center(), vec2(48.0, 48.0));
        let resp = ui.interact(play, ui.scope_id().with("pip_play"), Sense::click());
        let paused = player.state.paused || player.state.eof;
        icons::draw(&painter, if paused { Icon::Play } else { Icon::Pause }, play.center(), 30.0, color);
        if resp.clicked() {
            player.toggle_pause();
        }

        let back = Rect::from_center_size(pos2(full.right() - 22.0, full.top() + 22.0), vec2(32.0, 32.0));
        let resp = ui.interact(back, ui.scope_id().with("pip_back"), Sense::click());
        icons::draw(&painter, Icon::PictureInPicture, back.center(), 18.0, color);
        if resp.clicked() {
            self.toggle_pip(ui.ctx());
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

        let msg = tf("上次看到 {}", &[&fmt_time(pos)]);
        let font = FontId::proportional(12.0);
        let mg = p.layout_no_wrap(msg, FontId::monospace(12.0), TEXT.gamma_multiply(a));
        let rg = p.layout_no_wrap(tr("继续播放 Enter").into(), font.clone(), ACCENT);
        let sg = p.layout_no_wrap(tr("从头开始").into(), font.clone(), TEXT_2);
        let h = 34.0;
        let w = 14.0 + mg.size().x + 18.0 + rg.size().x + 16.0 + sg.size().x + 14.0;
        let rect = Rect::from_min_size(pos2(full.left() + 16.0, full.bottom() - BAR_H - 24.0 - h), vec2(w, h));
        p.rect_filled(rect, 0.0, theme::scrim((210.0 * a) as u8));
        p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(2.0, ACCENT.gamma_multiply(a)));

        let cy = rect.center().y;
        let mut x = rect.left() + 14.0;
        p.galley(pos2(x, cy - mg.size().y / 2.0), mg.clone(), TEXT);
        x += mg.size().x + 18.0;

        let resume_r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(rg.size().x, 24.0));
        let rr = ui.interact(resume_r, ui.scope_id().with("resume_yes"), Sense::click());
        let rc = if rr.hovered() { TEXT } else { ACCENT };
        p.text(pos2(x, cy), Align2::LEFT_CENTER, tr("继续播放 Enter"), font.clone(), rc.gamma_multiply(a));
        x += rg.size().x + 16.0;

        let start_r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(sg.size().x, 24.0));
        let sr = ui.interact(start_r, ui.scope_id().with("resume_no"), Sense::click());
        let sc = if sr.hovered() { TEXT } else { TEXT_2 };
        p.text(pos2(x, cy), Align2::LEFT_CENTER, tr("从头开始"), font, sc.gamma_multiply(a));

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

/// White dot with an accent ring, as on the seek and volume sliders.
/// Picture-in-picture window size: fits the video, longest side `PIP_MAX_SIDE`.
fn pip_size(width: i64, height: i64) -> egui::Vec2 {
    let aspect = if width > 0 && height > 0 { width as f32 / height as f32 } else { 16.0 / 9.0 };
    let size = if aspect >= 1.0 {
        vec2(PIP_MAX_SIDE, PIP_MAX_SIDE / aspect)
    } else {
        vec2(PIP_MAX_SIDE * aspect, PIP_MAX_SIDE)
    };
    size.max(PIP_MIN_SIZE.into()).round()
}

fn knob(p: &egui::Painter, c: egui::Pos2, r: f32, alpha: f32) {
    p.circle_filled(c, r, Color32::WHITE.gamma_multiply(alpha));
    p.circle_stroke(c, r, Stroke::new(2.0, ACCENT.gamma_multiply(alpha)));
}

/// Star right after the title (only for files in the library).
fn fav_button(ui: &mut egui::Ui, stage: Rect, title: &str, fav: bool, alpha: f32) -> bool {
    let w = ui.painter().layout_no_wrap(title.to_string(), FontId::proportional(13.0), TEXT).size().x;
    let max_x = stage.right() - 260.0;
    let x = (stage.left() + 92.0 + w + 18.0).min(max_x);
    let r = Rect::from_center_size(pos2(x, stage.top() + 22.0), vec2(26.0, 26.0));
    let resp = ui.interact(r, ui.scope_id().with("player_fav"), Sense::click());
    let c = if fav || resp.hovered() { ACCENT } else { TEXT_2 };
    icons::draw(ui.painter(), if fav { Icon::StarFilled } else { Icon::Star }, r.center(), 15.0, c.gamma_multiply(alpha));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.on_hover_text(if fav { tr("取消收藏 (Ctrl+D)") } else { tr("收藏 (Ctrl+D)") }).clicked()
}

/// "‹ 媒体库" at the top-left of the player. Returns true when clicked.
fn back_button(ui: &mut egui::Ui, stage: Rect, alpha: f32) -> bool {
    let r = Rect::from_min_size(pos2(stage.left() + 8.0, stage.top() + 8.0), vec2(74.0, 28.0));
    let resp = ui.interact(r, ui.scope_id().with("back_to_library"), Sense::click());
    let p = ui.painter();
    let c = if resp.hovered() { ACCENT } else { TEXT };
    if resp.hovered() {
        p.rect_filled(r, 14.0, theme::accent_alpha((30.0 * alpha) as u8));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let cy = r.center().y;
    let x = r.left() + 14.0;
    let st = Stroke::new(1.8, c.gamma_multiply(alpha));
    p.line_segment([pos2(x + 4.0, cy - 6.0), pos2(x - 2.0, cy)], st);
    p.line_segment([pos2(x - 2.0, cy), pos2(x + 4.0, cy + 6.0)], st);
    p.text(pos2(x + 12.0, cy), Align2::LEFT_CENTER, tr("媒体库"), FontId::proportional(13.0), c.gamma_multiply(alpha));
    resp.clicked()
}

fn draw_top_bar(p: &egui::Painter, full: Rect, s: &crate::player::State, plays: i64, alpha: f32) {
    let a = |c: Color32| c.gamma_multiply(alpha);
    let band = Rect::from_min_max(full.min, pos2(full.right(), full.top() + 44.0));
    p.rect_filled(band, 0.0, theme::scrim((110.0 * alpha) as u8));
    let y = full.top() + 22.0;
    p.text(pos2(full.left() + 92.0, y), Align2::LEFT_CENTER, &s.title, FontId::proportional(13.0), a(TEXT));

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
    AddFolder,
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
    p.text(frame.center() + vec2(0.0, 22.0), Align2::CENTER_CENTER, tr("拖入媒体文件"), FontId::proportional(13.0), theme::text());

    let open_rect = Rect::from_center_size(pos2(c.x - 84.0, frame.bottom() + 26.0), vec2(150.0, 28.0));
    let resp = ui.interact(open_rect, ui.scope_id().with("open"), Sense::click());
    let color = if resp.hovered() { ACCENT } else { theme::text_2() };
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let shortcut = format!("{}O", theme::CMD);
    p.text(open_rect.center(), Align2::CENTER_CENTER, tf("打开文件  {}", &[&shortcut]), FontId::monospace(11.0), color);

    let lib_rect = Rect::from_center_size(pos2(c.x + 84.0, frame.bottom() + 26.0), vec2(150.0, 28.0));
    let lresp = ui.interact(lib_rect, ui.scope_id().with("add_folder"), Sense::click());
    if lresp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    p.text(
        lib_rect.center(),
        Align2::CENTER_CENTER,
        tr("+ 建立媒体库"),
        FontId::proportional(12.0),
        if lresp.hovered() { theme::text() } else { ACCENT },
    );

    p.text(
        pos2(c.x, frame.bottom() + 64.0),
        Align2::CENTER_CENTER,
        tr("Space 播放/暂停 · ←/→ 快退/快进 · ↑/↓ 音量 · F 全屏"),
        FontId::monospace(10.0),
        theme::text_3(),
    );

    let mut action = if resp.clicked() {
        EmptyAction::Open
    } else if lresp.clicked() {
        EmptyAction::AddFolder
    } else {
        EmptyAction::None
    };

    // Settings link, top-right corner.
    let gear = Rect::from_center_size(pos2(full.right() - 24.0, full.top() + 24.0), vec2(30.0, 30.0));
    let gresp = ui.interact(gear, ui.scope_id().with("empty_settings"), Sense::click());
    icons::draw(&p, Icon::Settings, gear.center(), 15.0, if gresp.hovered() { ACCENT } else { theme::text_2() });
    if gresp.clicked() {
        action = EmptyAction::Settings;
    }

    // Continue watching.
    if !recent.is_empty() {
        let w = 360.0f32.min(full.width() - 40.0);
        let mut y = frame.bottom() + 100.0;
        p.text(pos2(c.x - w / 2.0, y), Align2::LEFT_CENTER, "CONTINUE", FontId::monospace(10.0), theme::text_3());
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
            let tg = p.layout_no_wrap(time, FontId::monospace(11.0), if hovered { ACCENT } else { theme::text_3() });
            let title_w = w - tg.size().x - 28.0;
            let title = elide(&p, &e.title, FontId::proportional(12.0), title_w);
            p.text(pos2(row.left() + 10.0, y), Align2::LEFT_CENTER, title, FontId::proportional(12.0), if hovered { theme::text() } else { theme::text_2() });
            p.galley(pos2(row.right() - tg.size().x - 6.0, y - tg.size().y / 2.0), tg, theme::text());
            // Thin progress line under each entry.
            let frac = if e.duration > 0.0 { (e.position / e.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let ly = row.bottom() - 1.0;
            p.line_segment([pos2(row.left() + 10.0, ly), pos2(row.right() - 6.0, ly)], Stroke::new(1.0, theme::line()));
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
    p.text(c - vec2(0.0, 14.0), Align2::CENTER_CENTER, tr("播放内核加载失败"), FontId::proportional(15.0), theme::text());
    p.text(c + vec2(0.0, 12.0), Align2::CENTER_CENTER, err, FontId::monospace(11.0), theme::text_2());
}

pub fn fmt_time(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    let (h, m, s) = (t / 3600, t % 3600 / 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

fn fmt_speed(s: f64) -> String {
    if (s * 10.0).fract().abs() < 1e-6 { format!("{s:.1}×") } else { format!("{s:.2}×") }
}

// ---- Playlist sidebar ------------------------------------------------------

const ROW_H: f32 = 36.0;

/// A small tab on the right edge of the video, vertically centered.
/// Shows "‹" (open playlist) or "›" (close it). Returns true when clicked.
fn draw_edge_handle(ui: &mut egui::Ui, stage: Rect, open: bool, alpha: f32) -> bool {
    let rect = Rect::from_min_size(pos2(stage.right() - 22.0, stage.center().y - 30.0), vec2(22.0, 60.0));
    let resp = ui.interact(rect, ui.scope_id().with("edge_handle_btn"), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    let radius = egui::CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 };
    p.rect_filled(rect, radius, theme::scrim(((if hovered { 220.0 } else { 160.0 }) * alpha) as u8));
    p.rect_stroke(rect, radius, Stroke::new(0.5, theme::accent_alpha((60.0 * alpha) as u8)), StrokeKind::Inside);
    let c = rect.center();
    let dir = if open { 1.0 } else { -1.0 };
    let color = (if hovered { ACCENT } else { TEXT }).gamma_multiply(alpha);
    let stroke = Stroke::new(1.8, color);
    p.line_segment([pos2(c.x - dir * 3.5, c.y - 7.0), pos2(c.x + dir * 3.5, c.y)], stroke);
    p.line_segment([pos2(c.x + dir * 3.5, c.y), pos2(c.x - dir * 3.5, c.y + 7.0)], stroke);
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

enum SideAction {
    Play(usize),
    Remove(usize),
    Move(usize, usize),
    ToggleShuffle,
    CycleLoop,
}

impl App {
    fn draw_sidebar(&mut self, ui: &mut egui::Ui, side: Rect, t: f32) {
        let ctx = ui.ctx().clone();
        let p = ui.painter_at(side);
        self.side_rect = side;
        p.rect_filled(side, 0.0, Color32::from_rgba_unmultiplied(0x0B, 0x0E, 0x12, 236));
        p.line_segment([side.left_top(), side.left_bottom()], Stroke::new(0.5, Color32::from_rgb(0x16, 0x1B, 0x22)));
        let a = t.clamp(0.0, 1.0);
        let left = side.left();
        let right = left + SIDEBAR_W;
        let mut action: Option<SideAction> = None;

        // Header
        let hy = side.top() + 24.0;
        let n = self.playlist.len();
        let tabs = [(false, format!("PLAYLIST · {n:02}"), left + 16.0), (true, format!("MARKS · {:02}", self.marks.len()), left + 116.0)];
        for (marks, label, x) in tabs {
            let on = self.side_marks == marks;
            let g = p.layout_no_wrap(label, FontId::monospace(10.0), if on { TEXT } else { TEXT_3 });
            let r = Rect::from_min_size(pos2(x, hy - g.size().y / 2.0), g.size()).expand2(vec2(6.0, 8.0));
            let resp = ui.interact(r, ui.scope_id().with(("side_tab", marks)), Sense::click());
            p.galley(pos2(x, hy - g.size().y / 2.0), g, TEXT);
            if on {
                p.line_segment([pos2(r.left() + 6.0, r.bottom()), pos2(r.right() - 6.0, r.bottom())], Stroke::new(1.5, ACCENT.gamma_multiply(a)));
            }
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.side_marks = marks;
                self.side_scroll = 0.0;
            }
        }
        if self.side_marks {
            self.draw_marks(ui, side, a);
            return;
        }
        let loop_mode = self.playlist.loop_mode;
        let header_btn = |ui: &mut egui::Ui, cx: f32, id: &str, icon: Icon, on: bool| -> bool {
            let r = Rect::from_center_size(pos2(cx, hy), vec2(28.0, 28.0));
            let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
            let c = if on { ACCENT } else if resp.hovered() { TEXT } else { TEXT_3.lerp_to_gamma(TEXT_2, 0.5) };
            if resp.hovered() {
                p.circle_filled(r.center(), 14.0, theme::accent_alpha(16));
            }
            icons::draw(&p, icon, r.center(), 14.0, c.gamma_multiply(a));
            resp.clicked()
        };
        let loop_icon = if loop_mode == LoopMode::One { Icon::RepeatOne } else { Icon::Repeat };
        if header_btn(ui, right - 24.0, "pl_loop", loop_icon, loop_mode != LoopMode::Off) {
            action = Some(SideAction::CycleLoop);
        }
        if header_btn(ui, right - 54.0, "pl_shuffle", Icon::Shuffle, self.playlist.shuffle()) {
            action = Some(SideAction::ToggleShuffle);
        }

        // List
        let list = Rect::from_min_max(pos2(left, side.top() + 48.0), pos2(right, side.bottom() - 34.0));
        let max_scroll = (n as f32 * ROW_H - list.height()).max(0.0);
        if self.scroll_to_current {
            if let Some(c) = self.playlist.current {
                let top = c as f32 * ROW_H;
                if top < self.side_scroll {
                    self.side_scroll = top;
                } else if top + ROW_H > self.side_scroll + list.height() {
                    self.side_scroll = top + ROW_H - list.height();
                }
            }
            self.scroll_to_current = false;
        }
        self.side_scroll = self.side_scroll.clamp(0.0, max_scroll);
        let lp = ui.painter_at(list.intersect(side));

        if n == 0 {
            lp.text(list.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, tr("列表为空"), FontId::proportional(13.0), TEXT_2.gamma_multiply(a));
            lp.text(list.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, tr("拖入文件或文件夹即可播放"), FontId::proportional(11.0), TEXT_3.gamma_multiply(a));
        }

        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let dragging = self.drag_from.is_some();
        let drop_index = pointer.map(|p| {
            (((p.y - list.top() + self.side_scroll) / ROW_H) + 0.5).floor().clamp(0.0, n as f32) as usize
        });

        // Index column grows with the number of digits (e.g. 20001 items).
        let idx_w = (n.max(10).to_string().len() as f32 * 7.5 + 14.0).max(30.0);
        for i in 0..n {
            let y = list.top() + i as f32 * ROW_H - self.side_scroll;
            if y + ROW_H < list.top() || y > list.bottom() {
                continue;
            }
            let row = Rect::from_min_size(pos2(left, y), vec2(SIDEBAR_W, ROW_H));
            let resp = ui.interact(row.intersect(list), ui.scope_id().with(("pl_row", i)), Sense::click_and_drag());
            let current = self.playlist.current == Some(i);
            let being_dragged = self.drag_from == Some(i);

            if current {
                lp.rect_filled(row, 0.0, theme::accent_alpha(14));
                lp.line_segment([row.left_top(), row.left_bottom()], Stroke::new(2.0, ACCENT.gamma_multiply(a)));
            } else if resp.hovered() && !dragging {
                lp.rect_filled(row, 0.0, Color32::from_white_alpha(6));
            }
            let fade = if being_dragged { 0.4 } else { a };
            let cy = row.center().y;
            lp.text(
                pos2(left + 16.0, cy),
                Align2::LEFT_CENTER,
                format!("{:02}", i + 1),
                FontId::monospace(11.0),
                (if current { ACCENT } else { TEXT_3 }).gamma_multiply(fade),
            );
            let close_rect = Rect::from_center_size(pos2(right - 20.0, cy), vec2(24.0, 24.0));
            let row_hot = !dragging && (resp.hovered() || ui.rect_contains_pointer(close_rect));
            let title_w = SIDEBAR_W - 16.0 - idx_w - if row_hot { 34.0 } else { 14.0 };
            let title = elide(&lp, &self.playlist.items[i].title, FontId::proportional(12.0), title_w);
            lp.text(
                pos2(left + 16.0 + idx_w, cy),
                Align2::LEFT_CENTER,
                title,
                FontId::proportional(12.0),
                (if current { TEXT } else { TEXT_2 }).gamma_multiply(fade),
            );

            if row_hot {
                let cresp = ui.interact(close_rect, ui.scope_id().with(("pl_rm", i)), Sense::click());
                icons::draw(&lp, Icon::Close, close_rect.center(), 12.0, if cresp.hovered() { ACCENT } else { TEXT_3 });
                if cresp.clicked() {
                    action = Some(SideAction::Remove(i));
                }
            }

            if resp.drag_started() {
                self.drag_from = Some(i);
            } else if resp.clicked() && action.is_none() {
                action = Some(SideAction::Play(i));
            }
            if resp.hovered() || resp.dragged() {
                ctx.set_cursor_icon(if dragging { CursorIcon::Grabbing } else { CursorIcon::PointingHand });
            }
        }

        // Drop indicator and finishing a drag.
        let released = ctx.input(|i| !i.pointer.primary_down());
        if let (Some(from), Some(to)) = (self.drag_from, drop_index) {
            let y = list.top() + to as f32 * ROW_H - self.side_scroll;
            lp.line_segment([pos2(left + 8.0, y), pos2(right - 8.0, y)], Stroke::new(2.0, ACCENT));
            if released {
                let target = if to > from { to - 1 } else { to };
                action = Some(SideAction::Move(from, target));
                self.drag_from = None;
            }
        } else if self.drag_from.is_some() && released {
            self.drag_from = None;
        }

        p.text(
            pos2(left + 16.0, side.bottom() - 17.0),
            Align2::LEFT_CENTER,
            tr("拖动排序 · L 收起"),
            FontId::proportional(10.0),
            TEXT_3.gamma_multiply(a),
        );

        match action {
            Some(SideAction::Play(i)) => self.play_index(i),
            Some(SideAction::Move(from, to)) => self.playlist.move_item(from, to),
            Some(SideAction::Remove(i)) => {
                if self.playlist.remove(i) {
                    match self.playlist.current {
                        Some(c) => self.play_index(c),
                        None => {
                            if let Some(pl) = &self.player {
                                pl.stop();
                            }
                        }
                    }
                }
            }
            Some(SideAction::ToggleShuffle) => {
                self.tracker.settings.shuffle = !self.tracker.settings.shuffle;
                self.tracker.save_settings();
                self.sync_playlist_settings();
                let on = self.tracker.settings.shuffle;
                self.show_osd(&ctx, if on { "SHUFFLE  ON" } else { "SHUFFLE  OFF" });
            }
            Some(SideAction::CycleLoop) => {
                let next = self.tracker.settings.loop_mode.cycle();
                self.tracker.settings.loop_mode = next;
                self.tracker.save_settings();
                self.sync_playlist_settings();
                let label = match next {
                    LoopMode::Off => "LOOP  OFF",
                    LoopMode::All => "LOOP  ALL",
                    LoopMode::One => "LOOP  ONE",
                };
                self.show_osd(&ctx, label);
            }
            None => {}
        }
    }
}

impl App {
    /// The sidebar's other tab: chapters and bookmarks in time order.
    fn draw_marks(&mut self, ui: &mut egui::Ui, side: Rect, a: f32) {
        let ctx = ui.ctx().clone();
        let left = side.left();
        let right = left + SIDEBAR_W;
        let p = ui.painter_at(side);
        let list = Rect::from_min_max(pos2(left, side.top() + 48.0), pos2(right, side.bottom() - 34.0));
        let lp = ui.painter_at(list.intersect(side));
        let rows = self.marks.rows();
        self.side_scroll = self.side_scroll.clamp(0.0, (rows.len() as f32 * ROW_H - list.height()).max(0.0));
        if rows.is_empty() {
            lp.text(list.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, tr("没有章节或书签"), FontId::proportional(13.0), TEXT_2.gamma_multiply(a));
        }
        let pos = self.player.as_ref().map_or(0.0, |pl| pl.state.position);
        let current = marks::current_row(&rows, pos);
        let mut seek: Option<f64> = None;
        let mut remove: Option<i64> = None;
        for (i, row) in rows.iter().enumerate() {
            let y = list.top() + i as f32 * ROW_H - self.side_scroll;
            if y + ROW_H < list.top() || y > list.bottom() {
                continue;
            }
            let r = Rect::from_min_size(pos2(left, y), vec2(SIDEBAR_W, ROW_H));
            let resp = ui.interact(r.intersect(list), ui.scope_id().with(("mark_row", i)), Sense::click());
            let on = current == Some(i);
            if on {
                lp.rect_filled(r, 0.0, theme::accent_alpha(14));
                lp.line_segment([r.left_top(), r.left_bottom()], Stroke::new(2.0, ACCENT.gamma_multiply(a)));
            } else if resp.hovered() {
                lp.rect_filled(r, 0.0, Color32::from_white_alpha(6));
            }
            let cy = r.center().y;
            lp.text(pos2(left + 16.0, cy), Align2::LEFT_CENTER, fmt_time(row.time), FontId::monospace(11.0), (if on { ACCENT } else { TEXT_3 }).gamma_multiply(a));
            let title_x = left + 76.0;
            let mut title_w = right - title_x - 14.0;
            if row.bookmark.is_some() {
                icons::draw(&lp, Icon::StarFilled, pos2(title_x + 6.0, cy), 12.0, BOOKMARK_COLOR.gamma_multiply(a));
                title_w -= 18.0;
            }
            let close = Rect::from_center_size(pos2(right - 20.0, cy), vec2(24.0, 24.0));
            let hot = row.bookmark.is_some() && (resp.hovered() || ui.rect_contains_pointer(close));
            if hot {
                title_w -= 20.0;
            }
            let x = if row.bookmark.is_some() { title_x + 18.0 } else { title_x };
            let title = elide(&lp, &row.title, FontId::proportional(12.0), title_w);
            lp.text(pos2(x, cy), Align2::LEFT_CENTER, title, FontId::proportional(12.0), (if on { TEXT } else { TEXT_2 }).gamma_multiply(a));
            if hot {
                let cresp = ui.interact(close, ui.scope_id().with(("mark_rm", i)), Sense::click());
                icons::draw(&lp, Icon::Close, close.center(), 12.0, if cresp.hovered() { ACCENT } else { TEXT_3 });
                if cresp.clicked() {
                    remove = row.bookmark;
                }
            }
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() && remove.is_none() {
                seek = Some(row.time);
            }
        }
        let hint = tf("{}B 添加书签 · G 收起", &[&theme::CMD]);
        p.text(pos2(left + 16.0, side.bottom() - 17.0), Align2::LEFT_CENTER, hint, FontId::proportional(10.0), TEXT_3.gamma_multiply(a));

        if let (Some(t), Some(pl)) = (seek, &self.player) {
            pl.seek_to(t, true);
        }
        if let (Some(id), Some(store), Some(key)) = (remove, &self.tracker.store, &self.marks.key) {
            store.remove_bookmark(id);
            self.marks.bookmarks = store.bookmarks(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_size_keeps_the_video_shape() {
        assert_eq!(pip_size(1920, 1080), vec2(400.0, 225.0));
        assert_eq!(pip_size(1080, 1920), vec2(225.0, 400.0));
    }

    #[test]
    fn pip_size_has_a_default_and_a_floor() {
        assert_eq!(pip_size(0, 0), vec2(400.0, 225.0));
        assert_eq!(pip_size(4000, 100), vec2(400.0, 120.0));
    }
}

//! In-player quick settings, opened from the control bar's gear:
//! 字幕 / 声音 / 画面 / 色彩. Anchored above the bar at the bottom-right.
//!
//! Aspect, hardware decoding, enhancement and subtitle size/position are
//! saved settings; rotation, flips, colors and delays last for this run.

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::history::Settings;
use crate::player::{Player, Track};
use crate::theme::ACCENT;

const W: f32 = 500.0;
const H: f32 = 392.0;
const NAV_W: f32 = 112.0;

const NAV_BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
const BODY_BG: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
const TXT: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE4);
const TXT_2: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA0);
const BORDER: Color32 = Color32::from_rgb(0x6A, 0x6A, 0x6A);

#[derive(Clone, Copy, PartialEq, Default)]
enum Page {
    Subtitles,
    Audio,
    #[default]
    Video,
    Color,
}

const COLOR_PROPS: [(&str, &str); 5] =
    [("亮度", "brightness"), ("对比度", "contrast"), ("饱和度", "saturation"), ("伽马", "gamma"), ("色调", "hue")];

#[derive(Default)]
pub struct QuickPanel {
    pub open: bool,
    page: Page,
    /// Set on the frame the panel opens so the opening click isn't "outside".
    fresh: bool,
    subs: Vec<Track>,
    audio: Vec<Track>,
    last_refresh: f64,
    rotate: i64,
    hflip: bool,
    vflip: bool,
    colors: [i64; 5],
    sub_delay: f64,
    audio_delay: f64,
}

#[derive(Default)]
pub struct Actions {
    pub settings_changed: bool,
    pub load_subtitle: bool,
    pub more_settings: bool,
}

impl QuickPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.fresh = self.open;
        self.last_refresh = f64::NEG_INFINITY;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// `stage` is the video area; the panel sits above the control bar.
    pub fn show(&mut self, ui: &mut egui::Ui, stage: Rect, bar_h: f32, player: &Player, s: &mut Settings) -> Actions {
        let mut out = Actions::default();
        if !self.open {
            return out;
        }
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if now - self.last_refresh > 0.5 {
            self.subs = player.tracks("sub");
            self.audio = player.tracks("audio");
            self.last_refresh = now;
        }
        ctx.request_repaint_after_secs(0.5);

        let w = W.min(stage.width() - 32.0);
        let h = H.min(stage.height() - bar_h - 24.0);
        let panel = Rect::from_min_size(pos2(stage.right() - 16.0 - w, stage.bottom() - bar_h - 10.0 - h), vec2(w, h));
        // Blocks clicks from reaching the video/bar underneath.
        let _blocker = ui.interact(panel, ui.scope_id().with("qp_blocker"), Sense::click_and_drag());

        let p = ui.painter_at(panel.expand(2.0));
        p.rect_filled(panel, 8.0, BODY_BG);
        let nav = Rect::from_min_size(panel.min, vec2(NAV_W, panel.height()));
        p.rect_filled(nav, egui::CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 }, NAV_BG);

        // Nav
        let items = [(Page::Subtitles, "字幕"), (Page::Audio, "声音"), (Page::Video, "画面"), (Page::Color, "色彩")];
        let top = nav.center().y - items.len() as f32 * 48.0 / 2.0;
        for (i, (page, label)) in items.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(nav.left(), top + i as f32 * 48.0), vec2(NAV_W, 40.0));
            let resp = ui.interact(r, ui.scope_id().with(("qp_nav", i)), Sense::click());
            let on = self.page == page;
            if on {
                p.rect_filled(r, 0.0, ACCENT);
            } else if resp.hovered() {
                p.rect_filled(r, 0.0, Color32::from_white_alpha(10));
            }
            p.text(pos2(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(15.0), if on { Color32::WHITE } else { TXT });
            if resp.clicked() {
                self.page = page;
            }
            hover_hand(&ctx, &resp);
        }

        // Close
        let x = Rect::from_center_size(pos2(panel.right() - 18.0, panel.top() + 18.0), vec2(22.0, 22.0));
        let xr = ui.interact(x, ui.scope_id().with("qp_close"), Sense::click());
        let xc = if xr.hovered() { Color32::WHITE } else { TXT_2 };
        p.line_segment([x.center() + vec2(-6.0, -6.0), x.center() + vec2(6.0, 6.0)], Stroke::new(1.3, xc));
        p.line_segment([x.center() + vec2(-6.0, 6.0), x.center() + vec2(6.0, -6.0)], Stroke::new(1.3, xc));
        if xr.clicked() {
            self.open = false;
        }

        let body = Rect::from_min_max(pos2(nav.right() + 20.0, panel.top() + 14.0), pos2(panel.right() - 20.0, panel.bottom() - 14.0));
        let mut c = Cursor { ui, p: &p, body, y: body.top() + 8.0, ctx: ctx.clone() };
        match self.page {
            Page::Subtitles => self.subtitles(&mut c, player, s, &mut out),
            Page::Audio => self.audio(&mut c, player),
            Page::Video => self.video(&mut c, player, s, &mut out),
            Page::Color => self.color(&mut c, player),
        }

        // "更多设置" bottom-left of the body.
        let ui = c.ui;
        let more = Rect::from_min_size(pos2(body.left(), body.bottom() - 22.0), vec2(70.0, 22.0));
        let mr = ui.interact(more, ui.scope_id().with("qp_more"), Sense::click());
        p.text(more.left_center(), Align2::LEFT_CENTER, "更多设置", FontId::proportional(12.0), if mr.hovered() { ACCENT } else { TXT_2 });
        hover_hand(&ctx, &mr);
        if mr.clicked() {
            out.more_settings = true;
            self.open = false;
        }

        // Close on Esc or a click outside (except on the opening frame).
        let fresh = std::mem::take(&mut self.fresh);
        let clicked_outside = !fresh
            && ctx.input(|i| i.pointer.any_click())
            && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !panel.contains(q));
        if clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
        }
        out
    }

    // ---- Pages ---------------------------------------------------------

    fn subtitles(&mut self, c: &mut Cursor, player: &Player, s: &mut Settings, out: &mut Actions) {
        c.section("字幕轨道");
        let mut opts: Vec<(Option<i64>, String, bool)> = vec![(None, "关闭".into(), !self.subs.iter().any(|t| t.selected))];
        opts.extend(self.subs.iter().map(|t| (Some(t.id), t.label.clone(), t.selected)));
        if let Some(i) = c.radios("qp_sid", &opts.iter().map(|o| (o.1.as_str(), o.2)).collect::<Vec<_>>()) {
            let v = opts[i].0.map_or("no".to_string(), |id| id.to_string());
            player.mpv.set_property_string("sid", &v);
            self.last_refresh = f64::NEG_INFINITY;
        }
        c.gap(4.0);
        if c.buttons("qp_subbtn", &["加载字幕文件…"]) == Some(0) {
            out.load_subtitle = true;
        }
        c.gap(10.0);

        c.section("字幕大小");
        let sizes = [36u32, 46, 58, 72];
        let labels = ["小", "中", "大", "特大"];
        let cur: Vec<(&str, bool)> = labels.iter().zip(sizes).map(|(l, v)| (*l, s.sub_size == v)).collect();
        if let Some(i) = c.radios("qp_subsize", &cur) {
            s.sub_size = sizes[i];
            out.settings_changed = true;
        }
        c.gap(6.0);
        c.section("字幕位置");
        let poss = [100u32, 95, 88];
        let labels = ["底部", "稍高", "更高"];
        let cur: Vec<(&str, bool)> = labels.iter().zip(poss).map(|(l, v)| (*l, s.sub_pos == v)).collect();
        if let Some(i) = c.radios("qp_subpos", &cur) {
            s.sub_pos = poss[i];
            out.settings_changed = true;
        }
        c.gap(6.0);
        c.section("字幕延迟");
        if let Some(d) = c.stepper("qp_subdelay", self.sub_delay, 0.5) {
            self.sub_delay = d;
            player.mpv.set_property_string("sub-delay", &format!("{d:.2}"));
        }
    }

    fn audio(&mut self, c: &mut Cursor, player: &Player) {
        c.section("音轨");
        if self.audio.is_empty() {
            c.note("没有音轨");
        } else {
            let opts: Vec<(&str, bool)> = self.audio.iter().map(|t| (t.label.as_str(), t.selected)).collect();
            if let Some(i) = c.radios("qp_aid", &opts) {
                player.mpv.set_property_string("aid", &self.audio[i].id.to_string());
                self.last_refresh = f64::NEG_INFINITY;
            }
        }
        c.gap(10.0);
        c.section("音频延迟");
        if let Some(d) = c.stepper("qp_adelay", self.audio_delay, 0.1) {
            self.audio_delay = d;
            player.mpv.set_property_string("audio-delay", &format!("{d:.2}"));
        }
        c.gap(10.0);
        c.section("音量");
        let vol = player.state.volume.round();
        let labels = ["−5%", "+5%", if player.state.muted { "取消静音" } else { "静音" }];
        c.note(&format!("当前 {vol}%"));
        c.gap(2.0);
        match c.buttons("qp_vol", &labels) {
            Some(0) => player.set_volume(vol - 5.0),
            Some(1) => player.set_volume(vol + 5.0),
            Some(2) => player.toggle_mute(),
            _ => {}
        }
    }

    fn video(&mut self, c: &mut Cursor, player: &Player, s: &mut Settings, out: &mut Actions) {
        c.section("画面比例");
        let opts = [("原始比例", "auto"), ("4:3", "4:3"), ("16:9", "16:9"), ("2.35:1", "2.35:1"), ("铺满窗口", "stretch"), ("21:9", "21:9")];
        let cur: Vec<(&str, bool)> = opts.iter().map(|(l, v)| (*l, s.aspect == *v)).collect();
        if let Some(i) = c.radios_cols("qp_aspect", &cur, 4) {
            s.aspect = opts[i].1.to_string();
            out.settings_changed = true;
        }
        c.gap(10.0);
        c.section("画面旋转");
        match c.buttons("qp_rot", &["向左旋转", "向右旋转"]) {
            Some(0) => self.set_rotate(player, self.rotate - 90),
            Some(1) => self.set_rotate(player, self.rotate + 90),
            _ => {}
        }
        c.gap(10.0);
        c.section("画面翻转");
        match c.buttons("qp_flip", &["水平镜像", "垂直翻转"]) {
            Some(0) => {
                let _ = player.mpv.command(&["vf", "toggle", "hflip"]);
                self.hflip = !self.hflip;
            }
            Some(1) => {
                let _ = player.mpv.command(&["vf", "toggle", "vflip"]);
                self.vflip = !self.vflip;
            }
            _ => {}
        }
        c.gap(12.0);
        if c.toggle_row("qp_hw", "硬件加速", s.hwdec) {
            s.hwdec = !s.hwdec;
            out.settings_changed = true;
        }
        c.gap(6.0);
        if c.toggle_row("qp_enh", "画质增强", s.enhance) {
            s.enhance = !s.enhance;
            out.settings_changed = true;
        }
        if c.reset_button("qp_vreset") {
            if s.aspect != "auto" {
                s.aspect = "auto".into();
                out.settings_changed = true;
            }
            self.set_rotate(player, 0);
            if self.hflip {
                let _ = player.mpv.command(&["vf", "toggle", "hflip"]);
                self.hflip = false;
            }
            if self.vflip {
                let _ = player.mpv.command(&["vf", "toggle", "vflip"]);
                self.vflip = false;
            }
        }
    }

    fn color(&mut self, c: &mut Cursor, player: &Player) {
        for (i, (label, prop)) in COLOR_PROPS.iter().enumerate() {
            if let Some(v) = c.slider(("qp_color", i), label, self.colors[i]) {
                self.colors[i] = v;
                player.mpv.set_property_string(prop, &v.to_string());
            }
            c.gap(8.0);
        }
        if c.reset_button("qp_creset") {
            for (i, (_, prop)) in COLOR_PROPS.iter().enumerate() {
                self.colors[i] = 0;
                player.mpv.set_property_string(prop, "0");
            }
        }
    }

    fn set_rotate(&mut self, player: &Player, deg: i64) {
        self.rotate = deg.rem_euclid(360);
        player.mpv.set_property_string("video-rotate", &self.rotate.to_string());
    }
}

// ---- Layout cursor and widgets ---------------------------------------------

struct Cursor<'a> {
    ui: &'a mut egui::Ui,
    p: &'a egui::Painter,
    body: Rect,
    y: f32,
    ctx: egui::Context,
}

impl Cursor<'_> {
    fn gap(&mut self, h: f32) {
        self.y += h;
    }

    fn section(&mut self, title: &str) {
        self.p.text(pos2(self.body.left(), self.y + 8.0), Align2::LEFT_CENTER, title, FontId::proportional(13.5), Color32::WHITE);
        self.y += 26.0;
    }

    fn note(&mut self, text: &str) {
        self.p.text(pos2(self.body.left(), self.y + 9.0), Align2::LEFT_CENTER, text, FontId::proportional(12.5), TXT_2);
        self.y += 22.0;
    }

    /// Radio buttons laid out left-to-right, wrapping. Returns the clicked index.
    fn radios(&mut self, id: &str, opts: &[(&str, bool)]) -> Option<usize> {
        let mut x = self.body.left();
        let mut clicked = None;
        for (i, (label, on)) in opts.iter().enumerate() {
            let tw = self.p.layout_no_wrap(label.to_string(), FontId::proportional(13.0), TXT).size().x;
            let w = 22.0 + tw + 22.0;
            if x + w > self.body.right() && x > self.body.left() {
                x = self.body.left();
                self.y += 30.0;
            }
            if self.radio((id, i), pos2(x, self.y), label, *on) {
                clicked = Some(i);
            }
            x += w;
        }
        self.y += 32.0;
        clicked
    }

    /// Radio buttons in a fixed number of columns (as in the reference).
    fn radios_cols(&mut self, id: &str, opts: &[(&str, bool)], cols: usize) -> Option<usize> {
        let col_w = self.body.width() / cols as f32;
        let mut clicked = None;
        for (i, (label, on)) in opts.iter().enumerate() {
            let x = self.body.left() + (i % cols) as f32 * col_w;
            let y = self.y + (i / cols) as f32 * 30.0;
            if self.radio((id, i), pos2(x, y), label, *on) {
                clicked = Some(i);
            }
        }
        self.y += opts.len().div_ceil(cols) as f32 * 30.0 + 2.0;
        clicked
    }

    fn radio(&mut self, id: impl std::hash::Hash + std::fmt::Debug, at: Pos2, label: &str, on: bool) -> bool {
        let tw = self.p.layout_no_wrap(label.to_string(), FontId::proportional(13.0), TXT).size().x;
        let r = Rect::from_min_size(at, vec2(22.0 + tw, 22.0));
        let resp = self.ui.interact(r, self.ui.scope_id().with(id), Sense::click());
        let c = pos2(at.x + 8.0, r.center().y);
        if on {
            self.p.circle_stroke(c, 7.0, Stroke::new(1.5, ACCENT));
            self.p.circle_filled(c, 3.5, ACCENT);
        } else {
            self.p.circle_stroke(c, 7.0, Stroke::new(1.0, if resp.hovered() { TXT } else { BORDER }));
        }
        self.p.text(pos2(at.x + 22.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), if on || resp.hovered() { TXT } else { TXT_2 });
        hover_hand(&self.ctx, &resp);
        resp.clicked()
    }

    /// Outlined buttons in a row (fixed width, like the reference).
    fn buttons(&mut self, id: &str, labels: &[&str]) -> Option<usize> {
        let mut x = self.body.left();
        let mut clicked = None;
        for (i, label) in labels.iter().enumerate() {
            let tw = self.p.layout_no_wrap(label.to_string(), FontId::proportional(13.0), TXT).size().x;
            let w = (tw + 32.0).max(110.0);
            let r = Rect::from_min_size(pos2(x, self.y), vec2(w, 28.0));
            let resp = self.ui.interact(r, self.ui.scope_id().with((id, i)), Sense::click());
            let hot = resp.hovered();
            if hot {
                self.p.rect_filled(r, 4.0, Color32::from_white_alpha(10));
            }
            self.p.rect_stroke(r, 4.0, Stroke::new(1.0, if hot { ACCENT } else { BORDER }), StrokeKind::Inside);
            self.p.text(r.center(), Align2::CENTER_CENTER, *label, FontId::proportional(13.0), if hot { Color32::WHITE } else { TXT });
            hover_hand(&self.ctx, &resp);
            if resp.clicked() {
                clicked = Some(i);
            }
            x = r.right() + 24.0;
        }
        self.y += 34.0;
        clicked
    }

    /// "label  [toggle] 开/关". Returns true when clicked.
    fn toggle_row(&mut self, id: &str, label: &str, on: bool) -> bool {
        let cy = self.y + 12.0;
        self.p.text(pos2(self.body.left(), cy), Align2::LEFT_CENTER, label, FontId::proportional(13.0), TXT);
        let r = Rect::from_center_size(pos2(self.body.left() + 106.0, cy), vec2(40.0, 22.0));
        let resp = self.ui.interact(r.expand(4.0), self.ui.scope_id().with(id), Sense::click());
        let t = self.ctx.animate_bool_with_time(self.ui.scope_id().with((id, "anim")), on, 0.12);
        let track = Color32::from_rgb(0x55, 0x55, 0x55).lerp_to_gamma(ACCENT, t);
        self.p.rect_filled(r, 11.0, track);
        self.p.circle_filled(pos2(r.left() + 11.0 + t * (r.width() - 22.0), cy), 8.0, Color32::WHITE);
        self.p.text(pos2(r.right() + 10.0, cy), Align2::LEFT_CENTER, if on { "开" } else { "关" }, FontId::proportional(13.0), TXT);
        hover_hand(&self.ctx, &resp);
        self.y += 26.0;
        resp.clicked()
    }

    /// "− value +  重置" for a delay in seconds. Returns the new value.
    fn stepper(&mut self, id: &str, value: f64, step: f64) -> Option<f64> {
        let mut out = None;
        let cy = self.y + 14.0;
        let mut x = self.body.left();
        for (i, label) in ["−", "+"].iter().enumerate() {
            if i == 1 {
                let txt = format!("{value:+.1} 秒");
                self.p.text(pos2(x + 40.0, cy), Align2::CENTER_CENTER, txt, FontId::monospace(13.0), TXT);
                x += 80.0;
            }
            let r = Rect::from_min_size(pos2(x, cy - 14.0), vec2(36.0, 28.0));
            let resp = self.ui.interact(r, self.ui.scope_id().with((id, i)), Sense::click());
            let hot = resp.hovered();
            self.p.rect_stroke(r, 4.0, Stroke::new(1.0, if hot { ACCENT } else { BORDER }), StrokeKind::Inside);
            self.p.text(r.center(), Align2::CENTER_CENTER, *label, FontId::proportional(16.0), TXT);
            hover_hand(&self.ctx, &resp);
            if resp.clicked() {
                let v = if i == 0 { value - step } else { value + step };
                out = Some((v * 100.0).round() / 100.0);
            }
            x = r.right();
        }
        let reset = Rect::from_min_size(pos2(x + 18.0, cy - 11.0), vec2(40.0, 22.0));
        let rr = self.ui.interact(reset, self.ui.scope_id().with((id, "reset")), Sense::click());
        self.p.text(reset.left_center(), Align2::LEFT_CENTER, "重置", FontId::proportional(12.5), if rr.hovered() { ACCENT } else { TXT_2 });
        hover_hand(&self.ctx, &rr);
        if rr.clicked() {
            out = Some(0.0);
        }
        self.y += 34.0;
        out
    }

    /// Labeled slider for -100..=100. Returns the new value while dragging.
    fn slider(&mut self, id: impl std::hash::Hash + std::fmt::Debug + Copy, label: &str, value: i64) -> Option<i64> {
        let cy = self.y + 12.0;
        self.p.text(pos2(self.body.left(), cy), Align2::LEFT_CENTER, label, FontId::proportional(13.0), TXT);
        let track = Rect::from_min_max(pos2(self.body.left() + 64.0, cy - 10.0), pos2(self.body.right() - 48.0, cy + 10.0));
        let resp = self.ui.interact(track, self.ui.scope_id().with(id), Sense::click_and_drag());
        let f = (value + 100) as f32 / 200.0;
        let mid = track.center().x;
        let kx = track.left() + f * track.width();
        self.p.line_segment([pos2(track.left(), cy), pos2(track.right(), cy)], Stroke::new(3.0, Color32::from_rgb(0x55, 0x55, 0x55)));
        self.p.line_segment([pos2(mid.min(kx), cy), pos2(mid.max(kx), cy)], Stroke::new(3.0, ACCENT));
        self.p.line_segment([pos2(mid, cy - 5.0), pos2(mid, cy + 5.0)], Stroke::new(1.0, BORDER));
        self.p.circle_filled(pos2(kx, cy), 7.0, Color32::WHITE);
        self.p.circle_stroke(pos2(kx, cy), 7.0, Stroke::new(2.0, ACCENT));
        self.p.text(pos2(self.body.right(), cy), Align2::RIGHT_CENTER, format!("{value:+}"), FontId::monospace(12.0), TXT_2);
        hover_hand(&self.ctx, &resp);
        self.y += 26.0;
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                let f = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
                let v = (f * 200.0 - 100.0).round() as i64;
                // Snap to 0 near the middle so it's easy to get back to neutral.
                let v = if v.abs() <= 3 { 0 } else { v };
                return (v != value).then_some(v);
            }
        }
        None
    }

    /// "⟳ 重置" at the bottom-right. Returns true when clicked.
    fn reset_button(&mut self, id: &str) -> bool {
        let r = Rect::from_min_size(pos2(self.body.right() - 60.0, self.body.bottom() - 24.0), vec2(60.0, 24.0));
        let resp = self.ui.interact(r, self.ui.scope_id().with(id), Sense::click());
        let c = if resp.hovered() { ACCENT } else { TXT };
        let ic = pos2(r.left() + 9.0, r.center().y);
        // Circular arrow.
        let pts: Vec<Pos2> = (0..=14)
            .map(|i| {
                let a = 0.6 + i as f32 / 14.0 * 5.0;
                pos2(ic.x + 6.0 * a.cos(), ic.y + 6.0 * a.sin())
            })
            .collect();
        let end = pts[0];
        self.p.add(egui::Shape::line(pts, Stroke::new(1.5, c)));
        self.p.add(egui::Shape::convex_polygon(vec![end + vec2(3.5, -2.0), end + vec2(-2.0, -3.5), end + vec2(0.5, 3.0)], c, Stroke::NONE));
        self.p.text(pos2(r.left() + 22.0, r.center().y), Align2::LEFT_CENTER, "重置", FontId::proportional(13.0), c);
        hover_hand(&self.ctx, &resp);
        resp.clicked()
    }
}

fn hover_hand(ctx: &egui::Context, resp: &egui::Response) {
    if resp.hovered() {
        ctx.set_cursor_icon(CursorIcon::PointingHand);
    }
}

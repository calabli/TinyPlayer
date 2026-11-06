//! In-player quick settings, opened from the control bar's gear:
//! 字幕 / 字幕样式 / 声音 / 均衡器 / 画面 / 画面增强 / 色彩. Anchored above the bar at the bottom-right.
//!
//! Aspect, hardware decoding, enhancement, tone mapping, interpolation,
//! subtitle size/position/style, normalization and the equalizer are saved
//! settings; crop and zoom last for the file; rotation, flips, colors and delays last for this run.

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::audio_fx::{self, Channel};
use crate::history::Settings;
use crate::i18n::{tf, tr};
use crate::player::{Player, Track};
use crate::theme::ACCENT;
use crate::view::{View, ZOOM_STEP};

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
    SubStyle,
    Audio,
    Equalizer,
    #[default]
    Video,
    Enhance,
    Color,
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum Crop {
    #[default]
    Off,
    /// Remove the black bars the detector finds.
    Auto,
    Ratio(f64),
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
    pub crop: Crop,
}

#[derive(Default)]
pub struct Actions {
    pub settings_changed: bool,
    pub load_subtitle: bool,
    pub more_settings: bool,
    pub crop_changed: bool,
    /// Zoom change in mpv's log2 steps.
    pub zoom: Option<f64>,
    pub view_reset: bool,
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
    pub fn show(&mut self, ui: &mut egui::Ui, stage: Rect, bar_h: f32, player: &Player, s: &mut Settings, view: View) -> Actions {
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
        let items = [(Page::Subtitles, tr("字幕")), (Page::SubStyle, tr("字幕样式")), (Page::Audio, tr("声音")), (Page::Equalizer, tr("均衡器")), (Page::Video, tr("画面")), (Page::Enhance, tr("画面增强")), (Page::Color, tr("色彩"))];
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
            Page::SubStyle => self.sub_style(&mut c, player, s, &mut out),
            Page::Audio => self.audio(&mut c, player, s, &mut out),
            Page::Equalizer => self.equalizer(&mut c, s, &mut out),
            Page::Video => self.video(&mut c, player, s, &mut out),
            Page::Enhance => self.enhance(&mut c, player, s, view, &mut out),
            Page::Color => self.color(&mut c, player),
        }

        // "更多设置" bottom-left of the body.
        let ui = c.ui;
        let more = Rect::from_min_size(pos2(body.left(), body.bottom() - 22.0), vec2(70.0, 22.0));
        let mr = ui.interact(more, ui.scope_id().with("qp_more"), Sense::click());
        p.text(more.left_center(), Align2::LEFT_CENTER, tr("更多设置"), FontId::proportional(12.0), if mr.hovered() { ACCENT } else { TXT_2 });
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
        c.section(tr("字幕轨道"));
        let mut opts: Vec<(Option<i64>, String, bool)> = vec![(None, tr("关闭").into(), !self.subs.iter().any(|t| t.selected))];
        opts.extend(self.subs.iter().map(|t| (Some(t.id), t.label.clone(), t.selected)));
        if let Some(i) = c.radios("qp_sid", &opts.iter().map(|o| (o.1.as_str(), o.2)).collect::<Vec<_>>()) {
            let v = opts[i].0.map_or("no".to_string(), |id| id.to_string());
            player.mpv.set_property_string("sid", &v);
            self.last_refresh = f64::NEG_INFINITY;
        }
        c.gap(4.0);
        if c.buttons("qp_subbtn", &[tr("加载字幕文件…")]) == Some(0) {
            out.load_subtitle = true;
        }
        c.gap(10.0);

        c.section(tr("字幕大小"));
        let sizes = [36u32, 46, 58, 72];
        let labels = [tr("小"), tr("中"), tr("大"), tr("特大")];
        let cur: Vec<(&str, bool)> = labels.iter().zip(sizes).map(|(l, v)| (*l, s.sub_size == v)).collect();
        if let Some(i) = c.radios("qp_subsize", &cur) {
            s.sub_size = sizes[i];
            out.settings_changed = true;
        }
        c.gap(6.0);
        c.section(tr("字幕位置"));
        let poss = [100u32, 95, 88];
        let labels = [tr("底部"), tr("稍高"), tr("更高")];
        let cur: Vec<(&str, bool)> = labels.iter().zip(poss).map(|(l, v)| (*l, s.sub_pos == v)).collect();
        if let Some(i) = c.radios("qp_subpos", &cur) {
            s.sub_pos = poss[i];
            out.settings_changed = true;
        }
        c.gap(6.0);
        c.section(tr("字幕延迟"));
        if let Some(d) = c.stepper("qp_subdelay", self.sub_delay, 0.5) {
            self.sub_delay = d;
            player.mpv.set_property_string("sub-delay", &format!("{d:.2}"));
        }
    }

    fn sub_style(&mut self, c: &mut Cursor, player: &Player, s: &mut Settings, out: &mut Actions) {
        c.section(tr("第二字幕"));
        // mpv can't show one track twice, so the main track isn't offered.
        let tracks: Vec<&Track> = self.subs.iter().filter(|t| !t.selected).collect();
        let mut opts: Vec<(Option<i64>, &str, bool)> = vec![(None, tr("关闭"), !tracks.iter().any(|t| t.secondary))];
        opts.extend(tracks.iter().map(|t| (Some(t.id), t.label.as_str(), t.secondary)));
        if let Some(i) = c.radios("qp_sid2", &opts.iter().map(|o| (o.1, o.2)).collect::<Vec<_>>()) {
            let v = opts[i].0.map_or("no".to_string(), |id| id.to_string());
            player.mpv.set_property_string("secondary-sid", &v);
            self.last_refresh = f64::NEG_INFINITY;
        }
        c.gap(4.0);

        c.section(tr("字体"));
        if let Some(v) = c.choice("qp_font", &[(tr("默认"), "default"), (tr("黑体"), "hei"), (tr("宋体"), "song"), (tr("楷体"), "kai")], s.sub_font.as_str()) {
            s.sub_font = v.to_string();
            out.settings_changed = true;
        }
        c.section(tr("颜色"));
        let colors = [(tr("白色"), "#FFFFFF"), (tr("黄色"), "#FFE066"), (tr("青色"), "#7FE7FF"), (tr("绿色"), "#9CFF8F")];
        if let Some(v) = c.choice("qp_color", &colors, s.sub_color.as_str()) {
            s.sub_color = v.to_string();
            out.settings_changed = true;
        }
        c.section(tr("描边"));
        if let Some(v) = c.choice("qp_outline", &[(tr("细"), 1u32), (tr("中"), 2), (tr("粗"), 4)], s.sub_outline) {
            s.sub_outline = v;
            out.settings_changed = true;
        }
        if c.toggle_row("qp_shadow", tr("阴影"), s.sub_shadow) {
            s.sub_shadow = !s.sub_shadow;
            out.settings_changed = true;
        }
        c.gap(4.0);
        if c.toggle_row("qp_assforce", tr("覆盖 ASS"), s.sub_ass_force) {
            s.sub_ass_force = !s.sub_ass_force;
            out.settings_changed = true;
        }
    }

    fn audio(&mut self, c: &mut Cursor, player: &Player, s: &mut Settings, out: &mut Actions) {
        c.section(tr("音轨"));
        if self.audio.is_empty() {
            c.note(tr("没有音轨"));
        } else {
            let opts: Vec<(&str, bool)> = self.audio.iter().map(|t| (t.label.as_str(), t.selected)).collect();
            if let Some(i) = c.radios("qp_aid", &opts) {
                player.mpv.set_property_string("aid", &self.audio[i].id.to_string());
                self.last_refresh = f64::NEG_INFINITY;
            }
        }
        c.gap(4.0);
        c.section(tr("声道"));
        let channels = [(tr("立体声"), Channel::Stereo), (tr("左声道"), Channel::Left), (tr("右声道"), Channel::Right), (tr("混合"), Channel::Mixed)];
        if let Some(v) = c.choice("qp_channel", &channels, s.audio_channel) {
            s.audio_channel = v;
            out.settings_changed = true;
        }
        c.gap(4.0);
        c.section(tr("音频延迟"));
        if let Some(d) = c.stepper("qp_adelay", self.audio_delay, 0.1) {
            self.audio_delay = d;
            player.mpv.set_property_string("audio-delay", &format!("{d:.2}"));
        }
        c.gap(10.0);
        c.section(tr("音量"));
        let vol = player.state.volume.round();
        let labels = ["−5%", "+5%", if player.state.muted { tr("取消静音") } else { tr("静音") }];
        c.note(&tf("当前 {}%", &[&vol]));
        c.gap(2.0);
        match c.buttons("qp_vol", &labels) {
            Some(0) => player.set_volume(vol - 5.0),
            Some(1) => player.set_volume(vol + 5.0),
            Some(2) => player.toggle_mute(),
            _ => {}
        }
        c.gap(4.0);
        if c.toggle_row("qp_norm", tr("音量标准化"), s.audio_norm) {
            s.audio_norm = !s.audio_norm;
            out.settings_changed = true;
        }
    }

    fn equalizer(&mut self, c: &mut Cursor, s: &mut Settings, out: &mut Actions) {
        c.section(tr("预设"));
        let presets: Vec<(&str, bool)> = audio_fx::PRESETS.iter().map(|(l, g)| (tr(l), *g == s.eq)).collect();
        if let Some(i) = c.radios_cols("qp_eq_preset", &presets, audio_fx::PRESETS.len()) {
            s.eq = audio_fx::PRESETS[i].1;
            out.settings_changed = true;
        }
        c.gap(2.0);
        // Whole dB steps: each change restarts the audio filters.
        let scale = 100.0 / audio_fx::MAX_GAIN as f32;
        for (i, hz) in audio_fx::BANDS.iter().enumerate() {
            let label = if *hz >= 1000 { format!("{}k", hz / 1000) } else { hz.to_string() };
            let value = (s.eq[i] as f32 * scale).round() as i64;
            if let Some(v) = c.slider(("qp_eq", i), &label, value, format!("{:+} dB", s.eq[i])) {
                let db = (v as f32 / scale).round() as i32;
                if db != s.eq[i] {
                    s.eq[i] = db;
                    out.settings_changed = true;
                }
            }
        }
    }

    fn video(&mut self, c: &mut Cursor, player: &Player, s: &mut Settings, out: &mut Actions) {
        c.section(tr("画面比例"));
        let opts = [(tr("原始比例"), "auto"), ("4:3", "4:3"), ("16:9", "16:9"), ("2.35:1", "2.35:1"), (tr("铺满窗口"), "stretch"), ("21:9", "21:9")];
        let cur: Vec<(&str, bool)> = opts.iter().map(|(l, v)| (*l, s.aspect == *v)).collect();
        if let Some(i) = c.radios_cols("qp_aspect", &cur, 4) {
            s.aspect = opts[i].1.to_string();
            out.settings_changed = true;
        }
        c.gap(10.0);
        c.section(tr("画面旋转"));
        match c.buttons("qp_rot", &[tr("向左旋转"), tr("向右旋转")]) {
            Some(0) => self.set_rotate(player, self.rotate - 90),
            Some(1) => self.set_rotate(player, self.rotate + 90),
            _ => {}
        }
        c.gap(10.0);
        c.section(tr("画面翻转"));
        match c.buttons("qp_flip", &[tr("水平镜像"), tr("垂直翻转")]) {
            Some(0) => {
                self.hflip = !self.hflip;
                player.set_flip(self.hflip, self.vflip);
            }
            Some(1) => {
                self.vflip = !self.vflip;
                player.set_flip(self.hflip, self.vflip);
            }
            _ => {}
        }
        c.gap(12.0);
        if c.toggle_row("qp_hw", tr("硬件加速"), s.hwdec) {
            s.hwdec = !s.hwdec;
            out.settings_changed = true;
        }
        c.gap(6.0);
        if c.toggle_row("qp_enh", tr("画质增强"), s.enhance) {
            s.enhance = !s.enhance;
            out.settings_changed = true;
        }
        if c.reset_button("qp_vreset") {
            if s.aspect != "auto" {
                s.aspect = "auto".into();
                out.settings_changed = true;
            }
            self.set_rotate(player, 0);
            if self.hflip || self.vflip {
                self.hflip = false;
                self.vflip = false;
                player.set_flip(false, false);
            }
        }
    }

    fn enhance(&mut self, c: &mut Cursor, player: &Player, s: &mut Settings, view: View, out: &mut Actions) {
        c.section(tr("裁剪"));
        let crops = [
            (tr("关闭"), Crop::Off),
            (tr("去黑边"), Crop::Auto),
            ("16:9", Crop::Ratio(16.0 / 9.0)),
            ("4:3", Crop::Ratio(4.0 / 3.0)),
            ("2.35:1", Crop::Ratio(2.35)),
            ("1:1", Crop::Ratio(1.0)),
        ];
        let cur: Vec<(&str, bool)> = crops.iter().map(|(l, v)| (*l, *v == self.crop)).collect();
        if let Some(i) = c.radios_cols("qp_crop", &cur, 3) {
            self.crop = crops[i].1;
            out.crop_changed = true;
        }
        c.gap(6.0);
        c.section(&tf("缩放  {}%", &[&view.percent()]));
        match c.buttons("qp_zoom", &[tr("缩小"), tr("放大"), tr("还原")]) {
            Some(0) => out.zoom = Some(-ZOOM_STEP),
            Some(1) => out.zoom = Some(ZOOM_STEP),
            Some(2) => out.view_reset = true,
            _ => {}
        }
        c.gap(6.0);
        c.section(tr("HDR 色调映射"));
        if let Some(v) = c.choice("qp_tone", &[(tr("自动"), "auto"), ("BT.2390", "bt.2390"), ("Hable", "hable"), ("Mobius", "mobius")], s.tone_mapping.as_str()) {
            s.tone_mapping = v.to_string();
            out.settings_changed = true;
        }
        c.note(if player.is_hdr() { tr("当前视频是 HDR") } else { tr("当前视频不是 HDR，此项不起作用") });
        c.gap(6.0);
        if c.toggle_row("qp_interp", tr("插帧"), s.interpolation) {
            s.interpolation = !s.interpolation;
            out.settings_changed = true;
        }
    }

    fn color(&mut self, c: &mut Cursor, player: &Player) {
        for (i, (label, prop)) in COLOR_PROPS.iter().enumerate() {
            if let Some(v) = c.slider(("qp_color", i), tr(label), self.colors[i], format!("{:+}", self.colors[i])) {
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

    /// Radio buttons for a setting's possible values. Returns the clicked value.
    fn choice<T: Copy + PartialEq>(&mut self, id: &str, opts: &[(&str, T)], cur: T) -> Option<T> {
        let labels: Vec<(&str, bool)> = opts.iter().map(|(l, v)| (*l, *v == cur)).collect();
        self.radios(id, &labels).map(|i| opts[i].1)
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
        let gap = 24.0;
        let n = labels.len() as f32;
        let fit = (self.body.width() - gap * (n - 1.0)) / n;
        for (i, label) in labels.iter().enumerate() {
            let tw = self.p.layout_no_wrap(label.to_string(), FontId::proportional(13.0), TXT).size().x;
            let w = (tw + 32.0).max(110.0).min(fit);
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
            x = r.right() + gap;
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
        self.p.text(pos2(r.right() + 10.0, cy), Align2::LEFT_CENTER, if on { tr("开") } else { tr("关") }, FontId::proportional(13.0), TXT);
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
                let txt = tf("{} 秒", &[&format!("{value:+.1}")]);
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
        self.p.text(reset.left_center(), Align2::LEFT_CENTER, tr("重置"), FontId::proportional(12.5), if rr.hovered() { ACCENT } else { TXT_2 });
        hover_hand(&self.ctx, &rr);
        if rr.clicked() {
            out = Some(0.0);
        }
        self.y += 34.0;
        out
    }

    /// Labeled slider for -100..=100. Returns the new value while dragging.
    fn slider(&mut self, id: impl std::hash::Hash + std::fmt::Debug + Copy, label: &str, value: i64, text: String) -> Option<i64> {
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
        self.p.text(pos2(self.body.right(), cy), Align2::RIGHT_CENTER, text, FontId::monospace(12.0), TXT_2);
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
        self.p.text(pos2(r.left() + 22.0, r.center().y), Align2::LEFT_CENTER, tr("重置"), FontId::proportional(13.0), c);
        hover_hand(&self.ctx, &resp);
        resp.clicked()
    }
}

fn hover_hand(ctx: &egui::Context, resp: &egui::Response) {
    if resp.hovered() {
        ctx.set_cursor_icon(CursorIcon::PointingHand);
    }
}

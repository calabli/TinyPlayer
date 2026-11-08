//! The settings overlay (Playback / Sound / Picture / Subtitles / History / Shortcuts), drawn in the tech-minimal style.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::audio_output::DeviceRow;
use crate::history::{ResumeMode, Settings};
use crate::i18n::{tf, tr};
use crate::playlist::LoopMode;
use crate::theme::{self, ACCENT};

#[derive(Default)]
pub struct PanelState {
    /// Time until which "Clear play stats" is armed for a confirming second click.
    confirm_stats_until: f64,
    page: Page,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum Page {
    #[default]
    Playback,
    Audio,
    Video,
    Subtitles,
    Interface,
    Library,
    History,
    Shortcuts,
}

#[derive(Default)]
pub struct Actions {
    pub settings_changed: bool,
    pub clear_history: bool,
    pub clear_stats: bool,
    pub close: bool,
    pub check_update: bool,
    pub device_volume: Option<(String, f64)>,
    pub forget_device: Option<String>,
    pub toggle_lock: bool,
    pub clear_library: bool,
}

const W: f32 = 660.0;
const NAV_W: f32 = 132.0;
const ROW_H: f32 = 58.0;
const DEVICE_ROW_H: f32 = 44.0;
const VOLUME_STEP: f64 = 5.0;

pub fn show(
    ui: &mut egui::Ui,
    full: Rect,
    st: &mut PanelState,
    settings: &mut Settings,
    incognito: &mut bool,
    stats: (i64, i64),
    update_status: &str,
    devices: &[DeviceRow],
    locked: bool,
) -> Actions {
    let mut out = Actions::default();
    let p = ui.painter().clone();
    let now = ui.ctx().input(|i| i.time);

    // Backdrop: click outside closes.
    p.rect_filled(full, 0.0, theme::scrim(170));
    let height = 56.0 + ROW_H * 6.0 + 64.0;
    let panel = Rect::from_center_size(full.center(), vec2(W.min(full.width() - 32.0), height.min(full.height() - 32.0)));
    let backdrop = ui.interact(full, ui.scope_id().with("settings_backdrop"), Sense::click());
    // Swallow clicks on the panel so they don't reach the backdrop.
    let _panel_blocker = ui.interact(panel, ui.scope_id().with("settings_panel"), Sense::click());
    if backdrop.clicked() {
        out.close = true;
    }

    p.rect_filled(panel, 10.0, theme::panel());
    p.rect_stroke(panel, 10.0, Stroke::new(0.5, theme::line()), StrokeKind::Inside);

    // ---- Nav ----
    let nav = Rect::from_min_size(panel.min, vec2(NAV_W, panel.height()));
    p.line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(0.5, theme::line()));
    p.text(nav.min + vec2(16.0, 22.0), Align2::LEFT_CENTER, "SETTINGS", FontId::monospace(10.0), theme::text_3());
    let pages = [
        (Page::Playback, tr("Play")),
        (Page::Audio, tr("Audio")),
        (Page::Video, tr("Video")),
        (Page::Subtitles, tr("Subtitles")),
        (Page::Interface, tr("Interface")),
        (Page::Library, tr("Library")),
        (Page::History, tr("History")),
        (Page::Shortcuts, tr("Shortcuts")),
    ];
    for (i, (page, name)) in pages.into_iter().enumerate() {
        let item = Rect::from_min_size(nav.min + vec2(0.0, 44.0 + 34.0 * i as f32), vec2(NAV_W, 32.0));
        let resp = ui.interact(item, ui.scope_id().with(("settings_nav", i)), Sense::click());
        let on = st.page == page;
        if on {
            p.rect_filled(item, 0.0, theme::accent_alpha(12));
            p.line_segment([item.left_top(), item.left_bottom()], Stroke::new(2.0, ACCENT));
        }
        let c = if on || resp.hovered() { theme::text() } else { theme::text_2() };
        p.text(item.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(13.0), c);
        if resp.clicked() {
            st.page = page;
        }
    }

    // Close button
    let close = Rect::from_center_size(pos2(panel.right() - 22.0, panel.top() + 22.0), vec2(24.0, 24.0));
    let cresp = ui.interact(close, ui.scope_id().with("settings_close"), Sense::click());
    let cc = if cresp.hovered() { ACCENT } else { theme::text_2() };
    let d = 5.0;
    p.line_segment([close.center() + vec2(-d, -d), close.center() + vec2(d, d)], Stroke::new(1.4, cc));
    p.line_segment([close.center() + vec2(-d, d), close.center() + vec2(d, -d)], Stroke::new(1.4, cc));
    if cresp.clicked() {
        out.close = true;
    }

    // ---- Rows ----
    let left = nav.right() + 24.0;
    let right = panel.right() - 24.0;
    let mut y = panel.top() + 44.0;
    let row = |y: f32| Rect::from_min_max(pos2(left, y), pos2(right, y + ROW_H));
    let label = |r: Rect, title: &str, desc: &str, enabled: bool| {
        let a = if enabled { 1.0 } else { 0.35 };
        let ty = if desc.is_empty() { r.center().y } else { r.top() + 21.0 };
        p.text(pos2(r.left(), ty), Align2::LEFT_CENTER, title, FontId::proportional(13.0), theme::text().gamma_multiply(a));
        if !desc.is_empty() {
            p.text(pos2(r.left(), r.top() + 40.0), Align2::LEFT_CENTER, desc, FontId::proportional(11.0), theme::text_3().lerp_to_gamma(theme::text_2(), 0.4).gamma_multiply(a));
        }
        p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.5, theme::line()));
    };

    if st.page == Page::Playback {
        let r = row(y);
        label(r, tr("Queue the folder"), tr("Opening one file also queues the rest of its folder, in name order"), true);
        if toggle(ui, &p, r, "fill_folder", settings.fill_folder, true) {
            settings.fill_folder = !settings.fill_folder;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("After playback"), "", true);
        let modes = [LoopMode::Off, LoopMode::All, LoopMode::One];
        let sel = modes.iter().position(|m| *m == settings.loop_mode).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "loop_mode", &[tr("Next"), tr("Loop list"), tr("Loop one")], sel, true) {
            settings.loop_mode = modes[i];
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Shuffle"), tr("Every file plays once before any repeats"), true);
        if toggle(ui, &p, r, "shuffle", settings.shuffle, true) {
            settings.shuffle = !settings.shuffle;
            out.settings_changed = true;
        }
        return out;
    }

    if st.page == Page::Audio {
        let r = row(y);
        label(r, tr("Volume per device"), tr("Headphones, speakers and other outputs each keep their own volume"), true);
        if toggle(ui, &p, r, "device_volume", settings.device_volume, true) {
            settings.device_volume = !settings.device_volume;
            out.settings_changed = true;
        }
        y += ROW_H;
        let area = Rect::from_min_max(pos2(left, y + 8.0), pos2(right, panel.bottom() - 16.0));
        device_list(ui, area, devices, settings.device_volume, &mut out);
        return out;
    }

    if st.page == Page::Video {
        let r = row(y);
        label(r, tr("Hardware decoding"), tr("Uses the GPU to save power and CPU. Turn off if the picture looks wrong"), true);
        if toggle(ui, &p, r, "hwdec", settings.hwdec, true) {
            settings.hwdec = !settings.hwdec;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Aspect ratio"), "", true);
        let opts = ["auto", "16:9", "4:3", "2.35:1", "stretch"];
        let sel = opts.iter().position(|o| *o == settings.aspect).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "aspect", &[tr("Original"), "16:9", "4:3", "2.35:1", tr("Stretch")], sel, true) {
            settings.aspect = opts[i].to_string();
            out.settings_changed = true;
        }
        return out;
    }

    if st.page == Page::Subtitles {
        let r = row(y);
        label(r, tr("Auto-load subtitles"), tr("Loads .srt, .ass and similar files named after the video"), true);
        if toggle(ui, &p, r, "sub_auto", settings.sub_auto, true) {
            settings.sub_auto = !settings.sub_auto;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Size|subtitle"), "", true);
        let sizes = [36u32, 46, 58, 72];
        let sel = sizes.iter().position(|v| *v == settings.sub_size).unwrap_or(1);
        if let Some(i) = segment(ui, &p, r, "sub_size", &[tr("Small"), tr("Medium"), tr("Large"), tr("Huge")], sel, true) {
            settings.sub_size = sizes[i];
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Position"), "", true);
        let poss = [100u32, 95, 88];
        let sel = poss.iter().position(|v| *v == settings.sub_pos).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "sub_pos", &[tr("Bottom"), tr("Higher"), tr("Highest")], sel, true) {
            settings.sub_pos = poss[i];
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Preferred language"), tr("Picked first when a file has several tracks"), true);
        let langs = ["auto", "zh", "en"];
        let sel = langs.iter().position(|v| *v == settings.sub_lang).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "sub_lang", &[tr("Auto"), tr("Chinese"), tr("English")], sel, true) {
            settings.sub_lang = langs[i].to_string();
            out.settings_changed = true;
        }
        return out;
    }

    if st.page == Page::Interface {
        let r = row(y);
        label(r, tr("Theme"), tr("Library and settings only. Playback controls stay dark"), true);
        let themes = ["dark", "light", "system"];
        let sel = themes.iter().position(|v| *v == settings.theme).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "theme", &[tr("Dark"), tr("Light"), tr("System")], sel, true) {
            settings.theme = themes[i].to_string();
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, crate::i18n::LANGUAGE_LABEL, "", true);
        let names = crate::i18n::LANGUAGE_NAMES;
        let sel = names.iter().position(|(code, _)| *code == settings.language).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "language", &[names[0].1, names[1].1], sel, true) {
            settings.language = names[i].0.to_string();
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Check for updates"), tr("Asks GitHub at most once a day. Sends no usage data"), true);
        if toggle(ui, &p, r, "auto_update", settings.auto_update, true) {
            settings.auto_update = !settings.auto_update;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, tr("Download updates"), tr("Downloads in the background, then installs on restart or exit"), settings.auto_update);
        if toggle(ui, &p, r, "auto_download", settings.auto_download, settings.auto_update) {
            settings.auto_download = !settings.auto_download;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, &tf("Version {}", &[&crate::update::CURRENT_VERSION]), update_status, true);
        if text_link(ui, &p, r.right_center(), "check_update", tr("Check now"), ACCENT).clicked {
            out.check_update = true;
        }
        return out;
    }

    if st.page == Page::Library {
        let r = row(y);
        label(r, tr("Library password"), tr("Asks for a password before opening the library"), true);
        if toggle(ui, &p, r, "library_lock", locked, true) {
            out.toggle_lock = true;
        }
        y += ROW_H;
        let r = row(y);
        label(r, tr("Clear library"), tr("Removes all folders, tags, and sets. Video files stay."), true);
        if text_link(ui, &p, pos2(right, r.center().y), "clear_library", tr("Clear"), Color32::from_rgb(0xF8, 0x51, 0x49)).clicked {
            out.clear_library = true;
        }
        return out;
    }

    if st.page == Page::Shortcuts {
        let cmd = crate::theme::CMD;
        let keys: Vec<(String, &str)> = vec![
            ("Space / K".into(), tr("Play / pause")),
            ("← / →".into(), tr("Seek 5 s")),
            (format!("{cmd}← / →"), tr("Seek 60 s")),
            (tr("↑ / ↓ / wheel").into(), tr("Volume ±5%")),
            ("M".into(), tr("Mute")),
            ("F / Enter".into(), tr("Full screen")),
            ("Esc".into(), tr("Exit full screen")),
            ("Home".into(), tr("Back to start")),
            ("[ / ]".into(), tr("Slower / faster")),
            ("Backspace".into(), tr("Reset to 1.0×")),
            ("C".into(), tr("Next subtitle")),
            ("N / P".into(), tr("Next / previous")),
            ("L".into(), tr("Playlist")),
            ("G".into(), tr("Chapters, bookmarks")),
            ("= / - / 0".into(), tr("Zoom in / out / reset")),
            ("Shift+← / →".into(), tr("Previous / next chapter")),
            (format!("{cmd}B"), tr("Add bookmark")),
            (", / .".into(), tr("Previous / next frame")),
            ("B".into(), tr("A-B loop")),
            ("V".into(), tr("VR panorama")),
            (format!("{cmd}Shift + P"), tr("Private mode")),
            ("T".into(), tr("Always on top")),
            ("I".into(), tr("Picture-in-picture")),
            ("H".into(), tr("Frameless playback")),
            (format!("{cmd}O"), tr("Open file")),
            (format!("{cmd}D"), tr("Toggle favorite")),
            (format!("{cmd}S"), tr("Screenshot to desktop")),
            (format!("{cmd}K"), tr("Search library")),
            (format!("{cmd},"), tr("Settings")),
        ];
        let col_w = (right - left) / 2.0;
        let per_col = keys.len().div_ceil(2);
        for (i, (k, what)) in keys.iter().enumerate() {
            let col = (i / per_col) as f32;
            let rowi = (i % per_col) as f32;
            let x = left + col * col_w;
            let yy = y + 14.0 + rowi * 30.0;
            let g = p.layout_no_wrap(k.clone(), FontId::monospace(11.0), theme::text());
            let kr = Rect::from_min_size(pos2(x, yy - g.size().y / 2.0 - 3.0), g.size() + vec2(10.0, 6.0));
            p.rect_filled(kr, 4.0, theme::tint(12));
            p.galley(kr.min + vec2(5.0, 3.0), g, theme::text());
            p.text(pos2(x + 122.0, yy), Align2::LEFT_CENTER, *what, FontId::proportional(12.0), theme::text_2());
        }
        return out;
    }

    let dep = settings.remember;

    let r = row(y);
    label(r, tr("Remember position"), tr("Continue where you left off in the same file"), true);
    if toggle(ui, &p, r, "remember", settings.remember, true) {
        settings.remember = !settings.remember;
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, tr("When resuming"), "", dep);
    let modes = [ResumeMode::Auto, ResumeMode::Ask, ResumeMode::Start];
    let sel = modes.iter().position(|m| *m == settings.resume_mode).unwrap_or(0);
    if let Some(i) = segment(ui, &p, r, "resume_mode", &[tr("Auto"), tr("Ask"), tr("Restart")], sel, dep) {
        settings.resume_mode = modes[i];
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, tr("Skip short videos"), tr("No position saved for files shorter than this"), dep);
    let mins = [0u32, 180, 600];
    let sel = mins.iter().position(|m| *m == settings.min_duration).unwrap_or(1);
    if let Some(i) = segment(ui, &p, r, "min_dur", &[tr("Off|short"), tr("3 min"), tr("10 min")], sel, dep) {
        settings.min_duration = mins[i];
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, tr("Clear when finished"), tr("Past 95% counts as watched, and the position is removed"), dep);
    if toggle(ui, &p, r, "clear_finished", settings.clear_finished, dep) {
        settings.clear_finished = !settings.clear_finished;
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, tr("Clear history on exit"), tr("Clears positions and recents on exit. Play counts stay"), true);
    if toggle(ui, &p, r, "clear_on_exit", settings.clear_on_exit, true) {
        settings.clear_on_exit = !settings.clear_on_exit;
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, tr("Private mode"), tr("Records nothing and hides watch history and titles in system media controls until restart"), true);
    if toggle(ui, &p, r, "incognito", *incognito, true) {
        *incognito = !*incognito;
    }
    y += ROW_H;

    // ---- Footer ----
    let fy = y + 30.0;
    p.text(
        pos2(left, fy),
        Align2::LEFT_CENTER,
        tf("{} positions saved · {} plays", &[&stats.0, &stats.1]),
        FontId::monospace(11.0),
        theme::text_2(),
    );
    let armed = now < st.confirm_stats_until;
    let stats_label = if armed { tr("Click again to confirm") } else { tr("Clear play stats") };
    let r2 = text_link(ui, &p, pos2(right, fy), "clear_stats", stats_label, if armed { Color32::from_rgb(0xF8, 0x51, 0x49) } else { theme::text_2() });
    if r2.clicked {
        if armed {
            out.clear_stats = true;
            st.confirm_stats_until = 0.0;
        } else {
            st.confirm_stats_until = now + 3.0;
        }
    }
    if armed {
        ui.ctx().request_repaint_after_secs((st.confirm_stats_until - now) as f32);
    }
    let r1 = text_link(ui, &p, pos2(r2.left - 20.0, fy), "clear_history", tr("Clear positions now"), ACCENT);
    if r1.clicked {
        out.clear_history = true;
    }
    out
}

/// The remembered devices, each with its volume.
fn device_list(ui: &mut egui::Ui, area: Rect, devices: &[DeviceRow], enabled: bool, out: &mut Actions) {
    if devices.is_empty() {
        let c = theme::text_2();
        ui.painter().text(area.left_top() + vec2(0.0, 14.0), Align2::LEFT_CENTER, tr("No devices yet"), FontId::proportional(12.0), c);
        return;
    }
    let dim = if enabled { 1.0 } else { 0.35 };
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for device in devices {
                let (r, _) = ui.allocate_exact_size(vec2(area.width() - 8.0, DEVICE_ROW_H), Sense::hover());
                let p = ui.painter().clone();
                let cy = r.center().y;
                p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.5, theme::line()));

                // Right to left: forget, plus, value, minus.
                let forget = Rect::from_center_size(pos2(r.right() - 12.0, cy), vec2(24.0, 24.0));
                let plus = forget.translate(vec2(-38.0, 0.0));
                let value = pos2(plus.left() - 26.0, cy);
                let minus = plus.translate(vec2(-76.0, 0.0));
                let id = ui.scope_id().with(("device", &device.id));

                let text = format!("{}%", device.volume.round());
                p.text(value, Align2::CENTER_CENTER, text, FontId::monospace(12.0), theme::text().gamma_multiply(dim));
                if step_button(ui, &p, minus, id.with("minus"), false, enabled) {
                    out.device_volume = Some((device.id.clone(), device.volume - VOLUME_STEP));
                }
                if step_button(ui, &p, plus, id.with("plus"), true, enabled) {
                    out.device_volume = Some((device.id.clone(), device.volume + VOLUME_STEP));
                }

                let name_room = minus.left() - r.left() - 12.0;
                let mut job_w = name_room;
                if device.current {
                    let tag = p.layout_no_wrap(tr("In use").to_string(), FontId::proportional(11.0), ACCENT.gamma_multiply(dim));
                    job_w -= tag.size().x + 10.0;
                    p.galley(pos2(r.left() + job_w + 10.0, cy - tag.size().y / 2.0), tag, ACCENT);
                } else if forget_button(ui, &p, forget, id.with("forget")) {
                    out.forget_device = Some(device.id.clone());
                }
                let mut job = egui::text::LayoutJob::simple_singleline(
                    device.name.clone(),
                    FontId::proportional(13.0),
                    theme::text().gamma_multiply(dim),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(job_w);
                let galley = p.layout_job(job);
                p.galley(pos2(r.left(), cy - galley.size().y / 2.0), galley, theme::text());
            }
        });
    });
}

fn step_button(ui: &mut egui::Ui, p: &egui::Painter, r: Rect, id: egui::Id, plus: bool, enabled: bool) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(r, id, sense);
    let c = match (enabled, resp.hovered()) {
        (false, _) => theme::text_3(),
        (_, true) => ACCENT,
        _ => theme::text_2(),
    };
    p.rect_stroke(r, 4.0, Stroke::new(0.5, theme::line()), StrokeKind::Inside);
    let (center, d) = (r.center(), 4.5);
    p.line_segment([center - vec2(d, 0.0), center + vec2(d, 0.0)], Stroke::new(1.4, c));
    if plus {
        p.line_segment([center - vec2(0.0, d), center + vec2(0.0, d)], Stroke::new(1.4, c));
    }
    resp.clicked()
}

fn forget_button(ui: &mut egui::Ui, p: &egui::Painter, r: Rect, id: egui::Id) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let c = if resp.hovered() { ACCENT } else { theme::text_3() };
    let (center, d) = (r.center(), 4.0);
    p.line_segment([center + vec2(-d, -d), center + vec2(d, d)], Stroke::new(1.2, c));
    p.line_segment([center + vec2(-d, d), center + vec2(d, -d)], Stroke::new(1.2, c));
    resp.clicked()
}

struct LinkResp {
    clicked: bool,
    left: f32,
}

/// Right-aligned text button ending at `right_center`.
fn text_link(ui: &mut egui::Ui, p: &egui::Painter, right_center: egui::Pos2, id: &str, text: &str, color: Color32) -> LinkResp {
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let rect = Rect::from_min_size(
        pos2(right_center.x - galley.size().x, right_center.y - galley.size().y / 2.0),
        galley.size(),
    );
    let resp = ui.interact(rect.expand(4.0), ui.scope_id().with(id), Sense::click());
    let c = if resp.hovered() { ACCENT.lerp_to_gamma(Color32::WHITE, 0.3) } else { color };
    p.text(rect.min, Align2::LEFT_TOP, text, FontId::proportional(12.0), c);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    LinkResp { clicked: resp.clicked(), left: rect.left() }
}

/// Toggle switch at the right edge of `row`. Returns true when clicked.
fn toggle(ui: &mut egui::Ui, p: &egui::Painter, row: Rect, id: &str, on: bool, enabled: bool) -> bool {
    let rect = Rect::from_center_size(pos2(row.right() - 17.0, row.center().y), vec2(34.0, 18.0));
    let resp = ui.interact(rect.expand(6.0), ui.scope_id().with(id), Sense::click());
    let t = ui.ctx().animate_bool_with_time(ui.scope_id().with(id).with("anim"), on, 0.12);
    let a = if enabled { 1.0 } else { 0.35 };
    let track = theme::line().lerp_to_gamma(theme::accent_alpha(64), t);
    p.rect_filled(rect, 9.0, track.gamma_multiply(a));
    let knob_x = rect.left() + 9.0 + t * (rect.width() - 18.0);
    let knob = Color32::from_rgb(0x6E, 0x76, 0x81).lerp_to_gamma(ACCENT, t);
    p.circle_filled(pos2(knob_x, rect.center().y), 6.0, knob.gamma_multiply(a));
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    enabled && resp.clicked()
}

/// Segmented choice at the right edge of `row`. Returns the clicked index.
fn segment(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    row: Rect,
    id: &str,
    labels: &[&str],
    selected: usize,
    enabled: bool,
) -> Option<usize> {
    let font = FontId::proportional(12.0);
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| p.layout_no_wrap(l.to_string(), font.clone(), theme::text()).size().x + 18.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 2.0 * (labels.len() as f32 - 1.0);
    let mut x = row.right() - total;
    let a = if enabled { 1.0 } else { 0.35 };
    let mut clicked = None;
    for (i, (label, w)) in labels.iter().zip(&widths).enumerate() {
        let rect = Rect::from_min_size(pos2(x, row.center().y - 13.0), vec2(*w, 26.0));
        let resp = ui.interact(rect, ui.scope_id().with(id).with(i), Sense::click());
        let on = i == selected;
        if on {
            p.rect_filled(rect, 4.0, theme::accent_alpha(22).gamma_multiply(a));
        }
        let color = if on { ACCENT } else if enabled && resp.hovered() { theme::text() } else { theme::text_2() };
        p.text(rect.center(), Align2::CENTER_CENTER, *label, font.clone(), color.gamma_multiply(a));
        if enabled && resp.clicked() && !on {
            clicked = Some(i);
        }
        x += w + 2.0;
    }
    clicked
}

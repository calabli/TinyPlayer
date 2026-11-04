//! The settings overlay (播放 / 画面 / 字幕 / 播放记录 / 快捷键), drawn in the tech-minimal style.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::history::{ResumeMode, Settings};
use crate::playlist::LoopMode;
use crate::theme::{self, ACCENT, LINE, PANEL, TEXT, TEXT_2, TEXT_3};

#[derive(Default)]
pub struct PanelState {
    /// Time until which "清除播放统计" is armed for a confirming second click.
    confirm_stats_until: f64,
    page: Page,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum Page {
    #[default]
    Playback,
    Video,
    Subtitles,
    History,
    Shortcuts,
}

#[derive(Default)]
pub struct Actions {
    pub settings_changed: bool,
    pub clear_history: bool,
    pub clear_stats: bool,
    pub close: bool,
}

const W: f32 = 660.0;
const NAV_W: f32 = 132.0;
const ROW_H: f32 = 58.0;

pub fn show(
    ui: &mut egui::Ui,
    full: Rect,
    st: &mut PanelState,
    settings: &mut Settings,
    incognito: &mut bool,
    stats: (i64, i64),
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

    p.rect_filled(panel, 10.0, PANEL);
    p.rect_stroke(panel, 10.0, Stroke::new(0.5, Color32::from_rgb(0x30, 0x36, 0x3D)), StrokeKind::Inside);

    // ---- Nav ----
    let nav = Rect::from_min_size(panel.min, vec2(NAV_W, panel.height()));
    p.line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(0.5, LINE));
    p.text(nav.min + vec2(16.0, 22.0), Align2::LEFT_CENTER, "SETTINGS", FontId::monospace(10.0), TEXT_3);
    let pages = [
        (Page::Playback, "播放"),
        (Page::Video, "画面"),
        (Page::Subtitles, "字幕"),
        (Page::History, "播放记录"),
        (Page::Shortcuts, "快捷键"),
    ];
    for (i, (page, name)) in pages.into_iter().enumerate() {
        let item = Rect::from_min_size(nav.min + vec2(0.0, 44.0 + 34.0 * i as f32), vec2(NAV_W, 32.0));
        let resp = ui.interact(item, ui.scope_id().with(("settings_nav", i)), Sense::click());
        let on = st.page == page;
        if on {
            p.rect_filled(item, 0.0, theme::accent_alpha(12));
            p.line_segment([item.left_top(), item.left_bottom()], Stroke::new(2.0, ACCENT));
        }
        let c = if on || resp.hovered() { TEXT } else { TEXT_2 };
        p.text(item.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(13.0), c);
        if resp.clicked() {
            st.page = page;
        }
    }

    // Close button
    let close = Rect::from_center_size(pos2(panel.right() - 22.0, panel.top() + 22.0), vec2(24.0, 24.0));
    let cresp = ui.interact(close, ui.scope_id().with("settings_close"), Sense::click());
    let cc = if cresp.hovered() { ACCENT } else { TEXT_2 };
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
        p.text(pos2(r.left(), ty), Align2::LEFT_CENTER, title, FontId::proportional(13.0), TEXT.gamma_multiply(a));
        if !desc.is_empty() {
            p.text(pos2(r.left(), r.top() + 40.0), Align2::LEFT_CENTER, desc, FontId::proportional(11.0), TEXT_3.lerp_to_gamma(TEXT_2, 0.4).gamma_multiply(a));
        }
        p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.5, LINE));
    };

    if st.page == Page::Playback {
        let r = row(y);
        label(r, "自动加入同目录文件", "打开单个文件时，把同一文件夹的其他媒体按文件名顺序加入列表", true);
        if toggle(ui, &p, r, "fill_folder", settings.fill_folder, true) {
            settings.fill_folder = !settings.fill_folder;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, "播完后", "", true);
        let modes = [LoopMode::Off, LoopMode::All, LoopMode::One];
        let sel = modes.iter().position(|m| *m == settings.loop_mode).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "loop_mode", &["播下一个", "列表循环", "单个循环"], sel, true) {
            settings.loop_mode = modes[i];
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, "随机播放", "列表中每个文件播放一次后才会重复", true);
        if toggle(ui, &p, r, "shuffle", settings.shuffle, true) {
            settings.shuffle = !settings.shuffle;
            out.settings_changed = true;
        }
        return out;
    }

    if st.page == Page::Video {
        let r = row(y);
        label(r, "硬件解码", "用显卡解码，省电、降低 CPU 占用；画面异常时可关闭", true);
        if toggle(ui, &p, r, "hwdec", settings.hwdec, true) {
            settings.hwdec = !settings.hwdec;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, "画面比例", "", true);
        let opts = ["auto", "16:9", "4:3", "2.35:1", "stretch"];
        let sel = opts.iter().position(|o| *o == settings.aspect).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "aspect", &["原始", "16:9", "4:3", "2.35:1", "拉伸"], sel, true) {
            settings.aspect = opts[i].to_string();
            out.settings_changed = true;
        }
        return out;
    }

    if st.page == Page::Subtitles {
        let r = row(y);
        label(r, "自动加载外挂字幕", "打开视频时加载同名的 .srt / .ass 等字幕文件", true);
        if toggle(ui, &p, r, "sub_auto", settings.sub_auto, true) {
            settings.sub_auto = !settings.sub_auto;
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, "字幕大小", "", true);
        let sizes = [36u32, 46, 58, 72];
        let sel = sizes.iter().position(|v| *v == settings.sub_size).unwrap_or(1);
        if let Some(i) = segment(ui, &p, r, "sub_size", &["小", "中", "大", "特大"], sel, true) {
            settings.sub_size = sizes[i];
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, "字幕位置", "", true);
        let poss = [100u32, 95, 88];
        let sel = poss.iter().position(|v| *v == settings.sub_pos).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "sub_pos", &["底部", "稍高", "更高"], sel, true) {
            settings.sub_pos = poss[i];
            out.settings_changed = true;
        }
        y += ROW_H;

        let r = row(y);
        label(r, "首选字幕语言", "有多条字幕轨时优先选择", true);
        let langs = ["auto", "zh", "en"];
        let sel = langs.iter().position(|v| *v == settings.sub_lang).unwrap_or(0);
        if let Some(i) = segment(ui, &p, r, "sub_lang", &["自动", "中文", "英文"], sel, true) {
            settings.sub_lang = langs[i].to_string();
            out.settings_changed = true;
        }
        return out;
    }

    if st.page == Page::Shortcuts {
        let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
        let keys: Vec<(String, &str)> = vec![
            ("Space / K".into(), "播放 / 暂停"),
            ("← / →".into(), "快退 / 快进 5 秒"),
            (format!("{cmd}← / →"), "快退 / 快进 60 秒"),
            ("↑ / ↓ / 滚轮".into(), "音量 ±5%"),
            ("M".into(), "静音"),
            ("F / Enter".into(), "全屏（或双击画面）"),
            ("Esc".into(), "退出全屏"),
            ("Home".into(), "回到开头"),
            ("[ / ]".into(), "减速 / 加速"),
            ("Backspace".into(), "恢复 1.0×"),
            ("C".into(), "切换字幕"),
            ("N / P".into(), "下一个 / 上一个"),
            ("L".into(), "播放列表"),
            (format!("{cmd}O"), "打开文件"),
            (format!("{cmd}D"), "收藏 / 取消收藏"),
            (format!("{cmd}S"), "截图到桌面"),
            (format!("{cmd}K"), "媒体库搜索"),
            (format!("{cmd},"), "设置"),
        ];
        let col_w = (right - left) / 2.0;
        let per_col = keys.len().div_ceil(2);
        for (i, (k, what)) in keys.iter().enumerate() {
            let col = (i / per_col) as f32;
            let rowi = (i % per_col) as f32;
            let x = left + col * col_w;
            let yy = y + 14.0 + rowi * 30.0;
            let g = p.layout_no_wrap(k.clone(), FontId::monospace(11.0), TEXT);
            let kr = Rect::from_min_size(pos2(x, yy - g.size().y / 2.0 - 3.0), g.size() + vec2(10.0, 6.0));
            p.rect_filled(kr, 4.0, Color32::from_white_alpha(12));
            p.galley(kr.min + vec2(5.0, 3.0), g, TEXT);
            p.text(pos2(x + 122.0, yy), Align2::LEFT_CENTER, *what, FontId::proportional(12.0), TEXT_2);
        }
        return out;
    }

    let dep = settings.remember;

    let r = row(y);
    label(r, "记住播放进度", "下次打开同一文件时从上次位置继续", true);
    if toggle(ui, &p, r, "remember", settings.remember, true) {
        settings.remember = !settings.remember;
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, "续播方式", "", dep);
    let modes = [ResumeMode::Auto, ResumeMode::Ask, ResumeMode::Start];
    let sel = modes.iter().position(|m| *m == settings.resume_mode).unwrap_or(0);
    if let Some(i) = segment(ui, &p, r, "resume_mode", &["自动", "询问", "从头"], sel, dep) {
        settings.resume_mode = modes[i];
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, "不记录短视频", "时长低于此值的文件不保存进度", dep);
    let mins = [0u32, 180, 600];
    let sel = mins.iter().position(|m| *m == settings.min_duration).unwrap_or(1);
    if let Some(i) = segment(ui, &p, r, "min_dur", &["关", "3 分钟", "10 分钟"], sel, dep) {
        settings.min_duration = mins[i];
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, "播完自动清除", "播放超过 95% 视为看完，删除该条进度", dep);
    if toggle(ui, &p, r, "clear_finished", settings.clear_finished, dep) {
        settings.clear_finished = !settings.clear_finished;
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, "退出时清除播放记录", "关闭时清空进度和最近播放，播放次数保留", true);
    if toggle(ui, &p, r, "clear_on_exit", settings.clear_on_exit, true) {
        settings.clear_on_exit = !settings.clear_on_exit;
        out.settings_changed = true;
    }
    y += ROW_H;

    let r = row(y);
    label(r, "无痕模式", "本次运行不写入任何记录，重启后恢复", true);
    if toggle(ui, &p, r, "incognito", *incognito, true) {
        *incognito = !*incognito;
    }
    y += ROW_H;

    // ---- Footer ----
    let fy = y + 30.0;
    p.text(
        pos2(left, fy),
        Align2::LEFT_CENTER,
        format!("已保存 {} 条进度 · 共播放 {} 次", stats.0, stats.1),
        FontId::monospace(11.0),
        TEXT_2,
    );
    let armed = now < st.confirm_stats_until;
    let stats_label = if armed { "再点一次确认" } else { "清除播放统计" };
    let r2 = text_link(ui, &p, pos2(right, fy), "clear_stats", stats_label, if armed { Color32::from_rgb(0xF8, 0x51, 0x49) } else { TEXT_2 });
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
    let r1 = text_link(ui, &p, pos2(r2.left - 20.0, fy), "clear_history", "立即清除进度", ACCENT);
    if r1.clicked {
        out.clear_history = true;
    }
    out
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
    let track = LINE.lerp_to_gamma(theme::accent_alpha(64), t);
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
        .map(|l| p.layout_no_wrap(l.to_string(), font.clone(), TEXT).size().x + 18.0)
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
        let color = if on { ACCENT } else if enabled && resp.hovered() { TEXT } else { TEXT_2 };
        p.text(rect.center(), Align2::CENTER_CENTER, *label, font.clone(), color.gamma_multiply(a));
        if enabled && resp.clicked() && !on {
            clicked = Some(i);
        }
        x += w + 2.0;
    }
    clicked
}

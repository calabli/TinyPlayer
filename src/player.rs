//! Playback state and commands, on top of [`crate::mpv::Mpv`].

use crate::mpv::{Event, Format, Mpv, PropValue};

/// Startup volume (percent). mpv defaults to 100, which is loud on most setups.
pub const DEFAULT_VOLUME: f64 = 60.0;

pub const SPEEDS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

#[derive(Default, Clone)]
pub struct State {
    pub idle: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub cached_until: f64,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub title: String,
    pub path: String,
    pub codec: String,
    pub hwdec: String,
    pub width: i64,
    pub height: i64,
    pub eof: bool,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: i64,
    pub label: String,
    /// Showing as the main track.
    pub selected: bool,
    /// Showing as the second subtitle.
    pub secondary: bool,
}

/// mpv font name for a `Settings::sub_font` choice.
fn sub_font_name(choice: &str) -> &'static str {
    let mac = cfg!(target_os = "macos");
    match choice {
        "hei" if mac => "PingFang SC",
        "hei" => "Microsoft YaHei",
        "song" if mac => "Songti SC",
        "song" => "SimSun",
        "kai" if mac => "Kaiti SC",
        "kai" => "KaiTi",
        _ => "sans-serif",
    }
}

fn language_name(code: &str) -> String {
    match code.to_lowercase().as_str() {
        "chi" | "zho" | "zh" | "chs" | "sc" | "zh-hans" | "zh-cn" => "中文".into(),
        "cht" | "tc" | "zh-hant" | "zh-tw" | "zh-hk" => "繁体中文".into(),
        "eng" | "en" => "英文".into(),
        "jpn" | "ja" => "日文".into(),
        "kor" | "ko" => "韩文".into(),
        other => other.to_string(),
    }
}

pub struct Player {
    pub mpv: Mpv,
    pub state: State,
}

impl Player {
    pub fn new() -> Result<Self, String> {
        let mpv = Mpv::new(&[
            ("vo", "libmpv"),
            ("hwdec", "auto-safe"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("terminal", "no"),
            ("config", "no"),
            ("volume", &DEFAULT_VOLUME.to_string()),
            ("volume-max", "150"),
            // mpv's default: cache network streams, read local files directly
            // (forcing the cache on local files only cost memory).
            ("cache", "auto"),
            ("demuxer-max-bytes", "50MiB"),
            ("demuxer-max-back-bytes", "20MiB"),
            ("sub-auto", "fuzzy"),
            ("screenshot-directory", "~~desktop/"),
        ])?;

        for (name, fmt) in [
            ("idle-active", Format::Flag),
            ("pause", Format::Flag),
            ("time-pos", Format::Double),
            ("duration", Format::Double),
            ("demuxer-cache-time", Format::Double),
            ("volume", Format::Double),
            ("mute", Format::Flag),
            ("speed", Format::Double),
            ("media-title", Format::Str),
            ("path", Format::Str),
            ("video-format", Format::Str),
            ("hwdec-current", Format::Str),
            ("width", Format::Int),
            ("height", Format::Int),
            ("eof-reached", Format::Flag),
        ] {
            mpv.observe(name, fmt);
        }

        Ok(Self {
            mpv,
            state: State { idle: true, volume: DEFAULT_VOLUME, speed: 1.0, ..Default::default() },
        })
    }

    /// Applies queued property changes to [`State`] and returns the
    /// lifecycle events (file loaded / ended), in order.
    pub fn pump(&mut self) -> Vec<Event> {
        let mut lifecycle = Vec::new();
        for ev in self.mpv.poll_events() {
            match ev {
                Event::PropertyChange { name, value } => self.apply(&name, value),
                other => lifecycle.push(other),
            }
        }
        lifecycle
    }

    fn apply(&mut self, name: &str, v: PropValue) {
        let s = &mut self.state;
        match (name, v) {
            ("idle-active", PropValue::Flag(b)) => s.idle = b,
            ("pause", PropValue::Flag(b)) => s.paused = b,
            ("time-pos", PropValue::Double(d)) => s.position = d,
            ("time-pos", PropValue::None) => s.position = 0.0,
            ("duration", PropValue::Double(d)) => s.duration = d,
            ("duration", PropValue::None) => s.duration = 0.0,
            ("demuxer-cache-time", PropValue::Double(d)) => s.cached_until = d,
            ("demuxer-cache-time", PropValue::None) => s.cached_until = 0.0,
            ("volume", PropValue::Double(d)) => s.volume = d,
            ("mute", PropValue::Flag(b)) => s.muted = b,
            ("speed", PropValue::Double(d)) => s.speed = d,
            ("media-title", PropValue::Str(t)) => s.title = t,
            ("path", PropValue::Str(t)) => s.path = t,
            ("path", PropValue::None) => s.path.clear(),
            ("video-format", PropValue::Str(t)) => s.codec = t.to_uppercase(),
            ("video-format", PropValue::None) => s.codec.clear(),
            ("hwdec-current", PropValue::Str(t)) => {
                s.hwdec = if t == "no" { String::new() } else { t }
            }
            ("hwdec-current", PropValue::None) => s.hwdec.clear(),
            ("width", PropValue::Int(i)) => s.width = i,
            ("height", PropValue::Int(i)) => s.height = i,
            ("eof-reached", PropValue::Flag(b)) => s.eof = b,
            _ => {}
        }
    }

    // ---- Commands ----

    pub fn open(&self, path: &str, append: bool) {
        let mode = if append { "append-play" } else { "replace" };
        let _ = self.mpv.command(&["loadfile", path, mode]);
        if !append {
            self.mpv.set_flag("pause", false);
        }
    }

    pub fn toggle_pause(&self) {
        if self.state.eof {
            // Restart from the beginning once the file has ended.
            let _ = self.mpv.command(&["seek", "0", "absolute"]);
            self.mpv.set_flag("pause", false);
        } else {
            self.mpv.set_flag("pause", !self.state.paused);
        }
    }

    pub fn seek_to(&self, secs: f64, exact: bool) {
        let t = format!("{:.3}", secs.max(0.0));
        let flags = if exact { "absolute+exact" } else { "absolute+keyframes" };
        let _ = self.mpv.command(&["seek", &t, flags]);
    }

    pub fn seek_by(&self, delta: f64) {
        let _ = self.mpv.command(&["seek", &format!("{delta}"), "relative+exact"]);
    }

    pub fn set_volume(&self, v: f64) {
        self.mpv.set_double("volume", v.clamp(0.0, 150.0));
        if self.state.muted {
            self.mpv.set_flag("mute", false);
        }
    }

    pub fn toggle_mute(&self) {
        self.mpv.set_flag("mute", !self.state.muted);
    }

    pub fn set_speed(&self, s: f64) {
        self.mpv.set_double("speed", s);
    }

    /// Steps through [`SPEEDS`]; `dir` is +1 or -1.
    pub fn step_speed(&self, dir: i32) -> f64 {
        let cur = SPEEDS
            .iter()
            .position(|s| (s - self.state.speed).abs() < 0.01)
            .unwrap_or(2) as i32;
        let next = SPEEDS[(cur + dir).clamp(0, SPEEDS.len() as i32 - 1) as usize];
        self.set_speed(next);
        next
    }

    /// Steps one frame and leaves playback paused.
    pub fn frame_step(&self, forward: bool) {
        let _ = self.mpv.command(&[if forward { "frame-step" } else { "frame-back-step" }]);
    }

    /// Sets or clears mpv's A-B loop points.
    pub fn set_ab_loop(&self, ab: crate::ab_loop::AbLoop) {
        let value = |t: Option<f64>| t.map_or("no".to_string(), |t| format!("{t:.3}"));
        self.mpv.set_property_string("ab-loop-a", &value(ab.a));
        self.mpv.set_property_string("ab-loop-b", &value(ab.b));
    }

    pub fn cycle_subtitles(&self) {
        let _ = self.mpv.command(&["cycle", "sub"]);
    }

    /// Pushes the user's video/subtitle preferences to mpv. Safe to call at
    /// any time; most take effect immediately, sub-auto/slang on the next file.
    pub fn apply_settings(&self, s: &crate::history::Settings) {
        let m = &self.mpv;
        m.set_property_string("hwdec", if s.hwdec { "auto-safe" } else { "no" });
        match s.aspect.as_str() {
            "stretch" => {
                m.set_property_string("keepaspect", "no");
                m.set_property_string("video-aspect-override", "-1");
            }
            "auto" | "" => {
                m.set_property_string("keepaspect", "yes");
                m.set_property_string("video-aspect-override", "-1");
            }
            ratio => {
                m.set_property_string("keepaspect", "yes");
                m.set_property_string("video-aspect-override", ratio);
            }
        }
        // 画质增强: debanding plus a sharper (heavier) upscaler.
        m.set_property_string("deband", if s.enhance { "yes" } else { "no" });
        m.set_property_string("scale", if s.enhance { "ewa_lanczossharp" } else { "lanczos" });
        m.set_property_string("cscale", if s.enhance { "ewa_lanczossharp" } else { "bilinear" });
        m.set_property_string("sub-auto", if s.sub_auto { "fuzzy" } else { "no" });
        m.set_property_string("sub-font-size", &s.sub_size.to_string());
        m.set_property_string("sub-pos", &s.sub_pos.to_string());
        m.set_property_string("sub-font", sub_font_name(&s.sub_font));
        m.set_property_string("sub-color", &s.sub_color);
        m.set_property_string("sub-outline-size", &s.sub_outline.to_string());
        m.set_property_string("sub-shadow-offset", if s.sub_shadow { "2" } else { "0" });
        m.set_property_string("sub-ass-override", if s.sub_ass_force { "force" } else { "scale" });
        let slang = match s.sub_lang.as_str() {
            "zh" => "chi,zho,zh,chs,cht,sc,tc,zh-Hans,zh-Hant,zh-CN,zh-TW",
            "en" => "eng,en",
            _ => "",
        };
        m.set_property_string("slang", slang);
    }

    /// Tracks of one kind ("video", "audio" or "sub") from mpv's track list.
    pub fn tracks(&self, kind: &str) -> Vec<Track> {
        let m = &self.mpv;
        let n: usize = m.get_string("track-list/count").and_then(|c| c.parse().ok()).unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..n {
            let get = |k: &str| m.get_string(&format!("track-list/{i}/{k}"));
            if get("type").as_deref() != Some(kind) {
                continue;
            }
            let Some(id) = get("id").and_then(|v| v.parse().ok()) else { continue };
            let lang = get("lang").unwrap_or_default();
            let title = get("title").unwrap_or_default();
            let external = get("external").as_deref() == Some("yes");
            // External files get a title like "en.srt"; prefer the language.
            let mut label = if !lang.is_empty() && (external || title.is_empty()) {
                language_name(&lang)
            } else if !title.is_empty() {
                title
            } else {
                format!("轨道 {id}")
            };
            if external {
                label.push_str("（外挂）");
            }
            let selected = get("selected").as_deref() == Some("yes");
            let secondary = selected && get("main-selection").as_deref() == Some("1");
            out.push(Track { id, label, selected: selected && !secondary, secondary });
        }
        out
    }

    /// The chapters of the loaded file, in order.
    pub fn chapters(&self) -> Vec<crate::marks::Chapter> {
        let m = &self.mpv;
        let n: usize = m.get_string("chapter-list/count").and_then(|c| c.parse().ok()).unwrap_or(0);
        (0..n)
            .filter_map(|i| {
                let time = m.get_string(&format!("chapter-list/{i}/time"))?.parse().ok()?;
                let title = m.get_string(&format!("chapter-list/{i}/title")).filter(|t| !t.is_empty());
                Some(crate::marks::Chapter { time, title: title.unwrap_or_else(|| format!("第 {} 章", i + 1)) })
            })
            .collect()
    }

    pub fn stop(&self) {
        let _ = self.mpv.command(&["stop"]);
    }

    pub fn screenshot(&self) {
        let _ = self.mpv.command(&["screenshot", "video"]);
    }
}

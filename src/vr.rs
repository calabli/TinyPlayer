//! Panoramic ("VR") video on an ordinary screen: which format a file is in,
//! and where the viewer is looking. Stereo files show the left eye only.
//! The drawing itself is in `video.rs`.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::i18n::tr;

/// In degrees, measured vertically.
const DEFAULT_FOV: f64 = 70.0;
const MIN_FOV: f64 = 30.0;
const MAX_FOV: f64 = 120.0;
/// Zoom per wheel event, in the same log2 steps as `view::ZOOM_STEP`. Small
/// because a trackpad sends one every frame.
pub const WHEEL_STEP: f64 = 0.06;
/// The render size moves in steps of this fraction, so zooming doesn't
/// reallocate the frame every time.
const RENDER_STEPS: f64 = 8.0;
/// The longest side worth rendering, however large the video and window are:
/// more than this stutters on most computers.
const MAX_RENDER_SIDE: f64 = 5760.0;
/// How far a frame's shape may be from its format's and still be detected.
const ASPECT_TOLERANCE: f64 = 0.1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Span {
    Half,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Mono,
    SideBySide,
    TopBottom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Format {
    pub span: Span,
    pub layout: Layout,
}

impl Format {
    /// The shape a whole frame has in this format, assuming square pixels.
    fn frame_aspect(self) -> f64 {
        let eye = match self.span {
            Span::Half => 1.0,
            Span::Full => 2.0,
        };
        match self.layout {
            Layout::Mono => eye,
            Layout::SideBySide => eye * 2.0,
            Layout::TopBottom => eye / 2.0,
        }
    }

    /// The left eye's area of the frame as [left, top, width, height].
    fn eye(self) -> [f32; 4] {
        match self.layout {
            Layout::Mono => [0.0, 0.0, 1.0, 1.0],
            Layout::SideBySide => [0.0, 0.0, 0.5, 1.0],
            Layout::TopBottom => [0.0, 0.0, 1.0, 0.5],
        }
    }

    pub fn label(self) -> String {
        let span = match self.span {
            Span::Half => "180°",
            Span::Full => "360°",
        };
        match self.layout {
            Layout::Mono => span.to_string(),
            Layout::SideBySide => format!("{span} {}", tr("Side by side")),
            Layout::TopBottom => format!("{span} {}", tr("Top and bottom")),
        }
    }

    pub fn stored(format: Option<Self>) -> &'static str {
        match format.map(|f| (f.span, f.layout)) {
            None => "off",
            Some((Span::Half, Layout::Mono)) => "180",
            Some((Span::Half, Layout::SideBySide)) => "180-sbs",
            Some((Span::Half, Layout::TopBottom)) => "180-tb",
            Some((Span::Full, Layout::Mono)) => "360",
            Some((Span::Full, Layout::SideBySide)) => "360-sbs",
            Some((Span::Full, Layout::TopBottom)) => "360-tb",
        }
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        let (span, layout) = text.split_once('-').unwrap_or((text, ""));
        let span = match span {
            "180" => Span::Half,
            "360" => Span::Full,
            _ => return None,
        };
        let layout = match layout {
            "" => Layout::Mono,
            "sbs" => Layout::SideBySide,
            "tb" => Layout::TopBottom,
            _ => return None,
        };
        Some(Self { span, layout })
    }
}

/// The layout whose frame shape is closest to `w`×`h`.
pub fn layout_for(span: Span, w: i64, h: i64) -> Layout {
    let actual = w.max(1) as f64 / h.max(1) as f64;
    let off = |layout: &Layout| (Format { span, layout: *layout }.frame_aspect() / actual).ln().abs();
    let all = [Layout::Mono, Layout::SideBySide, Layout::TopBottom];
    all.into_iter().min_by(|a, b| off(a).total_cmp(&off(b))).unwrap_or(Layout::Mono)
}

/// The usual format for a frame of this shape, when its name doesn't say.
pub fn guess(w: i64, h: i64) -> Format {
    Format { span: Span::Full, layout: layout_for(Span::Full, w, h) }
}

/// Reads the format from a file's name. Needs the frame to have the matching
/// shape too, so `Holiday 360.mp4` at 16:9 stays an ordinary video.
pub fn from_name(path: &str, w: i64, h: i64) -> Option<Format> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_lowercase();
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
    let tokens: Vec<&str> = stem.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let has = |names: &[&str]| tokens.iter().any(|t| names.contains(t));

    // Lens images aren't equirectangular, so projecting them would be wrong.
    if has(&["fisheye", "mkx200", "mkx220", "rf52"]) {
        return None;
    }
    let named_layout = if has(&["sbs", "hsbs", "fsbs", "lr", "3dh"]) {
        Some(Layout::SideBySide)
    } else if has(&["tb", "ou", "tab", "htb", "hou", "3dv"]) {
        Some(Layout::TopBottom)
    } else {
        None
    };
    let span = if has(&["360", "vr360", "360vr"]) {
        Span::Full
    } else if has(&["180", "vr180", "180vr", "180x180"]) {
        Span::Half
    } else if has(&["vr"]) {
        if named_layout == Some(Layout::SideBySide) { Span::Half } else { Span::Full }
    } else {
        return None;
    };
    let layout = named_layout.unwrap_or_else(|| layout_for(span, w, h));
    let format = Format { span, layout };
    let actual = w as f64 / h.max(1) as f64;
    ((actual / format.frame_aspect() - 1.0).abs() <= ASPECT_TOLERANCE).then_some(format)
}

/// What the shader needs to draw one view.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Projection {
    /// Turns a view ray into a panorama direction; columns, as OpenGL takes.
    pub rotation: [f32; 9],
    /// The tangents of half the field of view, across and up.
    pub tan_half: [f32; 2],
    /// See [`Format::eye`].
    pub eye: [f32; 4],
    /// The fraction of a full turn the picture covers.
    pub span: f32,
}

/// Where the viewer is looking, in radians; the field of view is in degrees.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    yaw: f64,
    pitch: f64,
    fov: f64,
}

impl Default for Look {
    fn default() -> Self {
        Self { yaw: 0.0, pitch: 0.0, fov: DEFAULT_FOV }
    }
}

impl Look {
    pub fn fov_degrees(&self) -> i64 {
        self.fov.round() as i64
    }

    /// `delta` is in log2 steps; positive zooms in.
    pub fn zoom_by(&mut self, delta: f64) {
        self.fov = (self.fov * 2f64.powf(-delta)).clamp(MIN_FOV, MAX_FOV);
    }

    /// Drags the picture along with the pointer. `dx`, `dy` and `height` are
    /// in the same units.
    pub fn drag(&mut self, dx: f64, dy: f64, height: f64, span: Span) {
        let per_unit = self.fov.to_radians() / height.max(1.0);
        self.yaw -= dx * per_unit;
        self.pitch = (self.pitch + dy * per_unit).clamp(-FRAC_PI_2, FRAC_PI_2);
        self.yaw = match span {
            Span::Half => self.yaw.clamp(-FRAC_PI_2, FRAC_PI_2),
            Span::Full => (self.yaw + PI).rem_euclid(TAU) - PI,
        };
    }

    /// The size to render a `frame`-sized video at so that a view
    /// `view_height` pixels tall stays about as sharp as the video allows,
    /// and no larger.
    pub fn render_size(&self, format: Format, frame: [i32; 2], view_height: i32) -> [i32; 2] {
        let [w, h] = frame.map(|v| v.max(1) as f64);
        let per_radian = view_height as f64 / 2.0 / (self.fov.to_radians() / 2.0).tan();
        let turn = match format.span {
            Span::Half => PI,
            Span::Full => TAU,
        };
        let (eye_w, eye_h) = (per_radian * turn, per_radian * PI);
        let (wanted_w, wanted_h) = match format.layout {
            Layout::Mono => (eye_w, eye_h),
            Layout::SideBySide => (eye_w * 2.0, eye_h),
            Layout::TopBottom => (eye_w, eye_h * 2.0),
        };
        let wanted = (wanted_w / w).max(wanted_h / h);
        let stepped = (wanted * RENDER_STEPS).ceil() / RENDER_STEPS;
        let scale = stepped.min(1.0).min(MAX_RENDER_SIDE / w.max(h));
        // Even, as a chroma plane is half as wide.
        [w, h].map(|v| ((v * scale).round() as i32 & !1).max(2))
    }

    /// `aspect` is the view's width over its height.
    pub fn projection(&self, format: Format, aspect: f64) -> Projection {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let up = (self.fov.to_radians() / 2.0).tan();
        Projection {
            rotation: [
                cy as f32,
                0.0,
                -sy as f32,
                (-sy * sp) as f32,
                cp as f32,
                (-cy * sp) as f32,
                (sy * cp) as f32,
                sp as f32,
                (cy * cp) as f32,
            ],
            tan_half: [(up * aspect) as f32, up as f32],
            eye: format.eye(),
            span: match format.span {
                Span::Half => 0.5,
                Span::Full => 1.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO_360: Format = Format { span: Span::Full, layout: Layout::Mono };
    const SBS_180: Format = Format { span: Span::Half, layout: Layout::SideBySide };

    fn forward(look: &Look) -> [f64; 3] {
        let r = look.projection(MONO_360, 1.0).rotation;
        [r[6] as f64, r[7] as f64, r[8] as f64]
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn names_and_shapes_decide_the_format() {
        let tb = Format { span: Span::Full, layout: Layout::TopBottom };
        assert_eq!(from_name("/v/alps_360.mp4", 7680, 3840), Some(MONO_360));
        assert_eq!(from_name("C:\\v\\beach VR180 LR.mkv", 7680, 3840), Some(SBS_180));
        assert_eq!(from_name("concert.180.mp4", 7680, 3840), Some(SBS_180));
        assert_eq!(from_name("dive-360-tb.mp4", 4096, 4096), Some(tb));
        assert_eq!(from_name("dive_vr.mp4", 4096, 4096), Some(tb));
        assert_eq!(from_name("clip_vr_sbs.mp4", 7680, 3840), Some(SBS_180));
    }

    #[test]
    fn ordinary_videos_are_left_alone() {
        assert_eq!(from_name("Holiday 360.mp4", 1920, 1080), None);
        assert_eq!(from_name("movie.360p.mp4", 640, 360), None);
        assert_eq!(from_name("movie.sbs.mkv", 3840, 1080), None);
        assert_eq!(from_name("panorama.mp4", 7680, 3840), None);
        assert_eq!(from_name("cam_360.mp4", 0, 0), None);
        assert_eq!(from_name("trip_180_fisheye.mp4", 7680, 3840), None);
    }

    #[test]
    fn guesses_follow_the_shape() {
        assert_eq!(guess(3840, 1920), MONO_360);
        assert_eq!(guess(4096, 4096).layout, Layout::TopBottom);
        assert_eq!(guess(0, 0).span, Span::Full);
    }

    #[test]
    fn stored_text_round_trips() {
        for span in [Span::Half, Span::Full] {
            for layout in [Layout::Mono, Layout::SideBySide, Layout::TopBottom] {
                let f = Some(Format { span, layout });
                assert_eq!(Format::from_stored(Format::stored(f)), f);
            }
        }
        assert_eq!(Format::stored(None), "off");
        assert_eq!(Format::from_stored("off"), None);
        assert_eq!(Format::from_stored("360-x"), None);
    }

    #[test]
    fn stereo_shows_the_first_eye_only() {
        assert_eq!(SBS_180.eye(), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(Format { span: Span::Full, layout: Layout::TopBottom }.eye(), [0.0, 0.0, 1.0, 0.5]);
    }

    #[test]
    fn looking_starts_at_the_middle_of_the_picture() {
        assert!(close(forward(&Look::default()), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn dragging_moves_the_picture_with_the_pointer() {
        let mut right = Look::default();
        right.drag(-90.0, 0.0, 70.0, Span::Full);
        assert!(close(forward(&right), [1.0, 0.0, 0.0]), "{:?}", forward(&right));

        let mut up = Look::default();
        up.drag(0.0, 90.0, 70.0, Span::Full);
        assert!(close(forward(&up), [0.0, 1.0, 0.0]), "{:?}", forward(&up));
    }

    #[test]
    fn rotation_keeps_the_horizon_level() {
        let mut look = Look::default();
        look.drag(40.0, 25.0, 100.0, Span::Full);
        let r = look.projection(MONO_360, 1.0).rotation;
        assert_eq!(r[1], 0.0);
        let length = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>();
        assert!((length(&r[..3]) - 1.0).abs() < 1e-5);
        assert!((length(&r[3..6]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn looking_is_limited_by_the_picture() {
        let mut half = Look::default();
        half.drag(-1e6, -1e6, 1.0, Span::Half);
        assert_eq!((half.yaw, half.pitch), (FRAC_PI_2, -FRAC_PI_2));

        let mut full = Look::default();
        full.drag(-1e6, 0.0, 1.0, Span::Full);
        assert!(full.yaw.abs() <= PI);
    }

    #[test]
    fn render_size_follows_the_view() {
        let frame = [7680, 3840];
        let look = Look::default();
        // 771 pixels per radian is 4845 across a full turn.
        assert_eq!(look.render_size(MONO_360, frame, 1080), [5760, 2880]);
        assert_eq!(look.render_size(MONO_360, frame, 540), [2880, 1440]);
        assert_eq!(look.render_size(SBS_180, frame, 1080), [5760, 2880]);
    }

    #[test]
    fn render_size_never_exceeds_the_video_or_the_limit() {
        let look = Look::default();
        assert_eq!(look.render_size(MONO_360, [2048, 1024], 1080), [2048, 1024]);
        assert_eq!(look.render_size(MONO_360, [11520, 5760], 2160), [5760, 2880]);
        assert_eq!(look.render_size(MONO_360, [0, 0], 1080), [2, 2]);
    }

    #[test]
    fn zoom_stays_in_range() {
        let mut look = Look::default();
        look.zoom_by(1.0);
        assert_eq!(look.fov_degrees(), 35);
        look.zoom_by(9.0);
        assert_eq!(look.fov_degrees(), 30);
        look.zoom_by(-9.0);
        assert_eq!(look.fov_degrees(), 120);
    }
}

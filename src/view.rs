//! Picture geometry: zoom, pan and crop. Interpolation's refresh-rate
//! snapping lives here too, as it's the other per-window video calculation.

/// mpv's `video-zoom` is log2: 3 is 8×.
const MAX_ZOOM: f64 = 3.0;
pub const ZOOM_STEP: f64 = 0.25;
/// Bars thinner than this fraction of the frame aren't worth cropping.
const MIN_BAR_FRACTION: f64 = 0.02;
const REFRESH_RATES: [f64; 9] = [50.0, 60.0, 75.0, 90.0, 100.0, 120.0, 144.0, 165.0, 240.0];

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct View {
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
}

impl View {
    pub fn scale(&self) -> f64 {
        2f64.powf(self.zoom)
    }

    pub fn zoomed(&self) -> bool {
        self.zoom > 0.0
    }

    pub fn percent(&self) -> i64 {
        (self.scale() * 100.0).round() as i64
    }

    pub fn zoom_by(&mut self, delta: f64) {
        self.zoom = (self.zoom + delta).clamp(0.0, MAX_ZOOM);
        self.clamp_pan();
    }

    /// `dx`/`dy` are fractions of the zoomed picture.
    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.pan_x += dx;
        self.pan_y += dy;
        self.clamp_pan();
    }

    /// Stops the picture's edge at the middle of the fitted area's edge, so
    /// panning can't drag the picture out of view.
    fn clamp_pan(&mut self) {
        let limit = 0.5 - 0.5 / self.scale();
        self.pan_x = self.pan_x.clamp(-limit, limit);
        self.pan_y = self.pan_y.clamp(-limit, limit);
    }
}

/// The size a `video` picture is shown at when fitted into `window`.
pub fn fit_size(video: (f64, f64), window: (f64, f64)) -> (f64, f64) {
    if video.0 <= 0.0 || video.1 <= 0.0 {
        return window;
    }
    let scale = (window.0 / video.0).min(window.1 / video.1);
    (video.0 * scale, video.1 * scale)
}

/// The largest centered area of a `w`×`h` frame with shape `ratio`, in mpv's
/// `video-crop` form. None if the frame already has that shape.
pub fn crop_to_ratio(w: i64, h: i64, ratio: f64) -> Option<String> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let even = |v: f64| (v as i64) & !1;
    let (cw, ch) = if (w as f64 / h as f64) > ratio {
        (even(h as f64 * ratio), h)
    } else {
        (w, even(w as f64 / ratio))
    };
    if cw == w && ch == h {
        return None;
    }
    Some(format!("{cw}x{ch}+{}+{}", (w - cw) / 2, (h - ch) / 2))
}

/// `video-crop` for a detected picture area, or None if the bars are too
/// thin to matter or the result makes no sense (an all-black sample, say).
pub fn crop_from_detected(frame: (i64, i64), area: (i64, i64, i64, i64)) -> Option<String> {
    let (fw, fh) = frame;
    let (w, h, x, y) = area;
    if w <= 0 || h <= 0 || w > fw || h > fh || x < 0 || y < 0 {
        return None;
    }
    let thin = |a: i64, full: i64| ((full - a) as f64) < full as f64 * MIN_BAR_FRACTION;
    if thin(w, fw) && thin(h, fh) {
        return None;
    }
    Some(format!("{w}x{h}+{x}+{y}"))
}

/// Snaps mpv's measured display rate to the standard rate it's closest to.
pub fn nearest_refresh(measured: f64) -> f64 {
    let dist = |r: &f64| (r - measured).abs();
    REFRESH_RATES.iter().copied().min_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap_or(60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_stays_in_range_and_reports_percent() {
        let mut v = View::default();
        v.zoom_by(-1.0);
        assert_eq!(v.zoom, 0.0);
        v.zoom_by(1.0);
        assert_eq!(v.percent(), 200);
        v.zoom_by(9.0);
        assert_eq!(v.percent(), 800);
    }

    #[test]
    fn pan_is_limited_by_zoom() {
        let mut v = View::default();
        v.pan_by(0.3, 0.0);
        assert_eq!(v.pan_x, 0.0);
        v.zoom_by(1.0);
        v.pan_by(0.9, -0.9);
        assert_eq!((v.pan_x, v.pan_y), (0.25, -0.25));
    }

    #[test]
    fn zooming_out_pulls_the_pan_back() {
        let mut v = View::default();
        v.zoom_by(2.0);
        v.pan_by(0.3, 0.0);
        v.zoom_by(-2.0);
        assert_eq!(v.pan_x, 0.0);
    }

    #[test]
    fn fit_uses_the_tighter_side() {
        assert_eq!(fit_size((1920.0, 1080.0), (960.0, 960.0)), (960.0, 540.0));
        assert_eq!(fit_size((0.0, 0.0), (800.0, 600.0)), (800.0, 600.0));
    }

    #[test]
    fn crop_to_ratio_is_centered_and_even() {
        assert_eq!(crop_to_ratio(1920, 1080, 4.0 / 3.0).as_deref(), Some("1440x1080+240+0"));
        assert_eq!(crop_to_ratio(1920, 1080, 2.35).as_deref(), Some("1920x816+0+132"));
        assert_eq!(crop_to_ratio(1920, 1080, 16.0 / 9.0), None);
        assert_eq!(crop_to_ratio(0, 0, 1.5), None);
    }

    #[test]
    fn detected_crop_ignores_thin_bars_and_nonsense() {
        assert_eq!(crop_from_detected((1280, 720), (1280, 536, 0, 92)).as_deref(), Some("1280x536+0+92"));
        assert_eq!(crop_from_detected((1280, 720), (1280, 716, 0, 2)), None);
        assert_eq!(crop_from_detected((1280, 720), (-1280, 0, 0, 0)), None);
        assert_eq!(crop_from_detected((1280, 720), (2000, 720, 0, 0)), None);
    }

    #[test]
    fn refresh_snaps_to_the_closest_standard_rate() {
        assert_eq!(nearest_refresh(118.6), 120.0);
        assert_eq!(nearest_refresh(110.6), 120.0);
        assert_eq!(nearest_refresh(59.9), 60.0);
        assert_eq!(nearest_refresh(143.0), 144.0);
    }
}

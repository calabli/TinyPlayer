//! Resizing the borderless floating window. winit can't start a native resize
//! on macOS, and a native one wouldn't hold the video's shape anyway, so this
//! moves the window itself. The cursor is read from the OS: its position inside
//! the window is useless here, because the window moves under it.

use eframe::egui::{CursorIcon, Pos2, Rect, Vec2, pos2, vec2};

/// Which sides a grip moves: -1 left/top, 1 right/bottom, 0 neither.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grip {
    pub x: i8,
    pub y: i8,
}

impl Grip {
    pub const ALL: [Grip; 8] = [
        Grip { x: -1, y: 0 },
        Grip { x: 1, y: 0 },
        Grip { x: 0, y: -1 },
        Grip { x: 0, y: 1 },
        // Corners last: they overlap the edges and have to win.
        Grip { x: -1, y: -1 },
        Grip { x: 1, y: -1 },
        Grip { x: -1, y: 1 },
        Grip { x: 1, y: 1 },
    ];

    /// The grab area along the window's edge.
    pub fn rect(self, window: Rect, edge: f32, corner: f32) -> Rect {
        let span = |grip: i8, min: f32, max: f32, near: f32| match grip {
            -1 => (min, min + near),
            1 => (max - near, max),
            _ => (min, max),
        };
        let near = if self.x != 0 && self.y != 0 { corner } else { edge };
        let (l, r) = span(self.x, window.left(), window.right(), near);
        let (t, b) = span(self.y, window.top(), window.bottom(), near);
        Rect::from_min_max(pos2(l, t), pos2(r, b))
    }

    pub fn cursor(self) -> CursorIcon {
        match (self.x, self.y) {
            (0, _) => CursorIcon::ResizeVertical,
            (_, 0) => CursorIcon::ResizeHorizontal,
            (x, y) if x == y => CursorIcon::ResizeNwSe,
            _ => CursorIcon::ResizeNeSw,
        }
    }
}

/// A resize in progress.
#[derive(Clone, Copy)]
pub struct Resize {
    grip: Grip,
    start: Rect,
    cursor: Pos2,
}

impl Resize {
    /// None where the cursor can't be read; the window then just doesn't resize.
    pub fn begin(grip: Grip, window: Rect) -> Option<Self> {
        Some(Self { grip, start: window, cursor: cursor_pos()? })
    }

    pub fn grip(&self) -> Grip {
        self.grip
    }

    /// Where the window belongs now.
    pub fn target(&self, pixels_per_point: f32, aspect: f32, min: Vec2, max: Vec2) -> Option<Rect> {
        let moved = (cursor_pos()? - self.cursor) / cursor_scale(pixels_per_point);
        Some(resized(self.start, self.grip, moved, aspect, min, max))
    }
}

/// `start` after dragging `grip` by `moved`: same shape, with the opposite side
/// staying put. Dragging an edge keeps the other axis centered.
pub fn resized(start: Rect, grip: Grip, moved: Vec2, aspect: f32, min: Vec2, max: Vec2) -> Rect {
    let size = start.size();
    let scale_x = (size.x + moved.x * grip.x as f32) / size.x;
    let scale_y = (size.y + moved.y * grip.y as f32) / size.y;
    let scale = match (grip.x, grip.y) {
        (0, _) => scale_y,
        (_, 0) => scale_x,
        _ => scale_x.max(scale_y),
    };
    let shape = if aspect > 0.0 { vec2(size.y * aspect, size.y) } else { size };
    let floor = (min.x / shape.x).max(min.y / shape.y);
    let ceiling = (max.x / shape.x).min(max.y / shape.y);
    let new = (shape * scale.min(ceiling).max(floor)).round();

    let along = |grip: i8, lo: f32, hi: f32, len: f32| match grip {
        1 => lo,
        -1 => hi - len,
        _ => (lo + hi - len) / 2.0,
    };
    let left = along(grip.x, start.left(), start.right(), new.x).round();
    let top = along(grip.y, start.top(), start.bottom(), new.y).round();
    Rect::from_min_size(pos2(left, top), new)
}

/// Screen units per egui point.
fn cursor_scale(pixels_per_point: f32) -> f32 {
    if cfg!(windows) { pixels_per_point } else { 1.0 }
}

#[cfg(target_os = "macos")]
fn cursor_pos() -> Option<Pos2> {
    use std::ffi::c_void;

    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *const c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }

    // SAFETY: a null source is allowed, and the event is released only here.
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some(pos2(point.x as f32, point.y as f32))
    }
}

#[cfg(windows)]
fn cursor_pos() -> Option<Pos2> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
    }

    let mut point = Point { x: 0, y: 0 };
    // SAFETY: `point` is valid for the call.
    (unsafe { GetCursorPos(&mut point) } != 0).then(|| pos2(point.x as f32, point.y as f32))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn cursor_pos() -> Option<Pos2> {
    None
}

/// macOS keeps a window on the Desktop it was opened on. The floating one
/// follows you to the others, and stays there if the OS allows it over another
/// app's full-screen Space.
#[cfg(target_os = "macos")]
pub fn follow_spaces(on: bool) {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{MainThreadMarker, msg_send};

    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;

    if MainThreadMarker::new().is_none() {
        return;
    }
    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    // SAFETY: on the main thread, and every selector exists on these classes.
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        let windows: *mut AnyObject = msg_send![app, windows];
        let window: *mut AnyObject = msg_send![windows, firstObject];
        if !window.is_null() {
            let behavior = if on { CAN_JOIN_ALL_SPACES | FULL_SCREEN_AUXILIARY } else { 0 };
            let _: () = msg_send![window, setCollectionBehavior: behavior];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn follow_spaces(_on: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    const ASPECT: f32 = 16.0 / 9.0;
    const MIN: Vec2 = vec2(200.0, 120.0);
    const MAX: Vec2 = vec2(1920.0, 1080.0);

    fn window() -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 225.0))
    }

    fn drag(x: i8, y: i8, moved: Vec2) -> Rect {
        resized(window(), Grip { x, y }, moved, ASPECT, MIN, MAX)
    }

    #[test]
    fn the_bottom_right_corner_keeps_the_top_left_still() {
        let r = drag(1, 1, vec2(160.0, 0.0));
        assert_eq!(r.min, window().min);
        assert_eq!(r.size(), vec2(560.0, 315.0));
    }

    #[test]
    fn the_top_left_corner_keeps_the_bottom_right_still() {
        let r = drag(-1, -1, vec2(-160.0, -90.0));
        assert_eq!(r.max, window().max);
        assert_eq!(r.size(), vec2(560.0, 315.0));
    }

    #[test]
    fn an_edge_keeps_the_shape_and_stays_centered() {
        let r = drag(1, 0, vec2(160.0, 40.0));
        assert_eq!(r.size(), vec2(560.0, 315.0));
        assert_eq!(r.left(), window().left());
        assert_eq!(r.center().y, window().center().y);
    }

    #[test]
    fn it_stops_at_the_smallest_size_without_moving_the_anchor() {
        let r = drag(-1, -1, vec2(900.0, 900.0));
        assert!(r.width() >= MIN.x && r.height() >= MIN.y);
        assert_eq!(r.max, window().max);
    }

    #[test]
    fn it_stops_at_the_screen_size() {
        assert_eq!(drag(1, 1, vec2(9000.0, 9000.0)).size(), vec2(1920.0, 1080.0));
    }

    #[test]
    fn a_portrait_video_stays_portrait() {
        let start = Rect::from_min_size(pos2(0.0, 0.0), vec2(225.0, 400.0));
        let r = resized(start, Grip { x: 0, y: 1 }, vec2(0.0, 400.0), 9.0 / 16.0, MIN, MAX);
        assert_eq!(r.size(), vec2(450.0, 800.0));
    }

    #[test]
    fn grips_sit_on_the_matching_edges() {
        let w = window();
        assert_eq!(Grip { x: 1, y: 0 }.rect(w, 6.0, 14.0).left(), w.right() - 6.0);
        assert_eq!(Grip { x: 0, y: -1 }.rect(w, 6.0, 14.0).bottom(), w.top() + 6.0);
        assert_eq!(Grip { x: -1, y: 1 }.rect(w, 6.0, 14.0).size(), vec2(14.0, 14.0));
    }
}

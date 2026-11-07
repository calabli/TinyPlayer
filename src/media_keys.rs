//! System media controls: the keyboard's media keys, Control Center and the
//! Windows volume flyout. Commands arrive on an OS thread and are queued for
//! the app; the title and play state are published back to the OS.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use crate::player::State;

/// Publish the position again when it drifts this far (a seek, say); the OS
/// advances it by itself between updates.
const MAX_DRIFT_SECS: f64 = 2.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Stop,
}

fn to_cmd(e: &MediaControlEvent) -> Option<Cmd> {
    Some(match e {
        MediaControlEvent::Play => Cmd::Play,
        MediaControlEvent::Pause => Cmd::Pause,
        MediaControlEvent::Toggle => Cmd::Toggle,
        MediaControlEvent::Next => Cmd::Next,
        MediaControlEvent::Previous => Cmd::Prev,
        MediaControlEvent::Stop => Cmd::Stop,
        _ => return None,
    })
}

pub struct MediaKeys {
    controls: MediaControls,
    pub rx: Receiver<Cmd>,
    title: String,
    duration: f64,
    paused: bool,
    idle: bool,
    /// Position at the last publish, and when that was.
    anchor: (f64, Instant),
}

impl MediaKeys {
    /// None if the OS refuses; the player works without media keys.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let mut controls = MediaControls::new(PlatformConfig {
            display_name: "TinyPlayer",
            dbus_name: "tinyplayer",
            hwnd: window_handle(cc),
        })
        .ok()?;
        let (tx, rx) = mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        controls
            .attach(move |e: MediaControlEvent| {
                if let Some(cmd) = to_cmd(&e) {
                    let _ = tx.send(cmd);
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self {
            controls,
            rx,
            title: String::new(),
            duration: 0.0,
            paused: false,
            idle: true,
            anchor: (0.0, Instant::now()),
        })
    }

    /// Tells the OS what's playing, only when that actually changed.
    /// `private` keeps the file's name out of the system's media controls.
    pub fn publish(&mut self, s: &State, private: bool) {
        let title = if private { "TinyPlayer" } else { s.title.as_str() };
        let expected = self.anchor.0 + self.anchor.1.elapsed().as_secs_f64() * if self.paused { 0.0 } else { s.speed };
        let drifted = !s.idle && (s.position - expected).abs() > MAX_DRIFT_SECS;
        let meta_changed = title != self.title || s.duration != self.duration;
        if !meta_changed && !drifted && s.paused == self.paused && s.idle == self.idle {
            return;
        }
        if meta_changed {
            let _ = self.controls.set_metadata(MediaMetadata {
                title: Some(title),
                duration: (s.duration > 0.0).then(|| Duration::from_secs_f64(s.duration)),
                ..Default::default()
            });
            self.title = title.to_string();
            self.duration = s.duration;
        }
        let progress = Some(MediaPosition(Duration::from_secs_f64(s.position.max(0.0))));
        let _ = self.controls.set_playback(if s.idle {
            MediaPlayback::Stopped
        } else if s.paused {
            MediaPlayback::Paused { progress }
        } else {
            MediaPlayback::Playing { progress }
        });
        self.paused = s.paused;
        self.idle = s.idle;
        self.anchor = (s.position, Instant::now());
    }
}

#[cfg(target_os = "windows")]
fn window_handle(cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn window_handle(_cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_transport_events_and_ignores_the_rest() {
        assert_eq!(to_cmd(&MediaControlEvent::Toggle), Some(Cmd::Toggle));
        assert_eq!(to_cmd(&MediaControlEvent::Previous), Some(Cmd::Prev));
        assert_eq!(to_cmd(&MediaControlEvent::Quit), None);
    }
}

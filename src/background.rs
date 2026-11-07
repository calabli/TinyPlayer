//! Keeps heavy background work out of playback's way. Scanning, thumbnails,
//! duplicate checks and copying all wait here while a video is open, and pick
//! up where they left off afterwards.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const CHECK_EVERY: Duration = Duration::from_millis(100);

static PLAYING: AtomicBool = AtomicBool::new(false);

pub fn set_playing(playing: bool) {
    PLAYING.store(playing, Ordering::Relaxed);
}

/// Blocks while a video is open, or until `stop` is set.
pub fn wait(stop: &AtomicBool) {
    while PLAYING.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(CHECK_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_ends_the_wait() {
        wait(&AtomicBool::new(false));
        set_playing(true);
        wait(&AtomicBool::new(true));
        set_playing(false);
    }
}

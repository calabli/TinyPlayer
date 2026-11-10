//! Sleep timer: pauses playback after a while, or when the current video ends.
//! It lasts until it fires or the app quits; it isn't saved.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Mode {
    #[default]
    Off,
    Minutes(u32),
    EndOfVideo,
}

/// The choices offered, in order.
pub const MINUTES: [u32; 5] = [15, 30, 45, 60, 90];

#[derive(Default)]
pub struct SleepTimer {
    mode: Mode,
    deadline: Option<Instant>,
    warned: bool,
}

/// What the timer wants done now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Due {
    /// A minute is left: say so once.
    Warn,
    /// Time's up: pause.
    Pause,
}

impl SleepTimer {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set(&mut self, mode: Mode, now: Instant) {
        self.mode = mode;
        self.warned = false;
        self.deadline = match mode {
            Mode::Minutes(m) => Some(now + Duration::from_secs(m as u64 * 60)),
            _ => None,
        };
    }

    /// Seconds left on a timed mode.
    pub fn left(&self, now: Instant) -> Option<u64> {
        self.deadline.map(|d| d.saturating_duration_since(now).as_secs())
    }

    /// Checks the clock. `Pause` turns the timer off.
    pub fn tick(&mut self, now: Instant) -> Option<Due> {
        let deadline = self.deadline?;
        if now >= deadline {
            self.set(Mode::Off, now);
            return Some(Due::Pause);
        }
        // Short timers (tests aside, none are) get no warning at the start.
        let warn_at = deadline.checked_sub(Duration::from_secs(60))?;
        if now >= warn_at && !self.warned {
            self.warned = true;
            return Some(Due::Warn);
        }
        None
    }

    /// The current video ended. True if playback should stop here; that turns
    /// the timer off.
    pub fn video_ended(&mut self, now: Instant) -> bool {
        if self.mode == Mode::EndOfVideo {
            self.set(Mode::Off, now);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn warns_once_then_pauses_and_turns_off() {
        let t0 = Instant::now();
        let mut t = SleepTimer::default();
        t.set(Mode::Minutes(15), t0);
        assert_eq!(t.left(t0), Some(900));
        assert_eq!(t.tick(t0 + secs(10)), None);
        assert_eq!(t.tick(t0 + secs(841)), Some(Due::Warn));
        assert_eq!(t.tick(t0 + secs(850)), None);
        assert_eq!(t.tick(t0 + secs(900)), Some(Due::Pause));
        assert_eq!(t.mode(), Mode::Off);
        assert_eq!(t.tick(t0 + secs(2000)), None);
        assert_eq!(t.left(t0 + secs(2000)), None);
    }

    #[test]
    fn end_of_video_fires_once_and_only_in_that_mode() {
        let t0 = Instant::now();
        let mut t = SleepTimer::default();
        assert!(!t.video_ended(t0));
        t.set(Mode::Minutes(30), t0);
        assert!(!t.video_ended(t0));
        assert_eq!(t.mode(), Mode::Minutes(30));
        t.set(Mode::EndOfVideo, t0);
        assert_eq!(t.left(t0), None);
        assert_eq!(t.tick(t0 + secs(99999)), None);
        assert!(t.video_ended(t0));
        assert!(!t.video_ended(t0));
        assert_eq!(t.mode(), Mode::Off);
    }

    #[test]
    fn setting_again_restarts_and_off_cancels() {
        let t0 = Instant::now();
        let mut t = SleepTimer::default();
        t.set(Mode::Minutes(15), t0);
        t.set(Mode::Minutes(15), t0 + secs(600));
        assert_eq!(t.tick(t0 + secs(900)), None);
        assert_eq!(t.left(t0 + secs(900)), Some(600));
        t.set(Mode::Off, t0 + secs(900));
        assert_eq!(t.tick(t0 + secs(5000)), None);
    }
}

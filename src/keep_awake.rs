//! Stops the computer sleeping during playback. `stop-screensaver` in mpv
//! needs mpv's own window, so embedding has to ask the OS directly.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hold {
    None,
    /// Audio: the display may sleep, the computer may not.
    System,
    /// Video: keep the display on too.
    Display,
}

pub fn hold_for(playing: bool, has_video: bool) -> Hold {
    match (playing, has_video) {
        (false, _) => Hold::None,
        (true, false) => Hold::System,
        (true, true) => Hold::Display,
    }
}

pub struct KeepAwake {
    hold: Hold,
    #[cfg(target_os = "macos")]
    assertion: Option<u32>,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self {
            hold: Hold::None,
            #[cfg(target_os = "macos")]
            assertion: None,
        }
    }

    pub fn set(&mut self, hold: Hold) {
        if hold != self.hold {
            self.apply(hold);
            self.hold = hold;
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        self.set(Hold::None);
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::ffi::{c_char, c_void};

    pub type CfString = *const c_void;
    pub const UTF8: u32 = 0x0800_0100;
    pub const LEVEL_ON: u32 = 255;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CfString;
        pub fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        pub fn IOPMAssertionCreateWithName(kind: CfString, level: u32, name: CfString, id: *mut u32) -> i32;
        pub fn IOPMAssertionRelease(id: u32) -> i32;
    }
}

#[cfg(target_os = "macos")]
impl KeepAwake {
    fn apply(&mut self, hold: Hold) {
        if let Some(id) = self.assertion.take() {
            unsafe { os::IOPMAssertionRelease(id) };
        }
        let kind = match hold {
            Hold::None => return,
            Hold::System => c"PreventUserIdleSystemSleep",
            Hold::Display => c"PreventUserIdleDisplaySleep",
        };
        unsafe {
            let kind = os::CFStringCreateWithCString(std::ptr::null(), kind.as_ptr(), os::UTF8);
            let name = os::CFStringCreateWithCString(std::ptr::null(), c"TinyPlayer playback".as_ptr(), os::UTF8);
            let mut id = 0;
            if os::IOPMAssertionCreateWithName(kind, os::LEVEL_ON, name, &raw mut id) == 0 {
                self.assertion = Some(id);
            }
            os::CFRelease(kind);
            os::CFRelease(name);
        }
    }
}

#[cfg(target_os = "windows")]
impl KeepAwake {
    /// Applies to the calling thread, so it must stay on the UI thread.
    fn apply(&mut self, hold: Hold) {
        const ES_CONTINUOUS: u32 = 0x8000_0000;
        const ES_SYSTEM_REQUIRED: u32 = 0x1;
        const ES_DISPLAY_REQUIRED: u32 = 0x2;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetThreadExecutionState(flags: u32) -> u32;
        }
        let flags = match hold {
            Hold::None => 0,
            Hold::System => ES_SYSTEM_REQUIRED,
            Hold::Display => ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED,
        };
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | flags) };
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl KeepAwake {
    fn apply(&mut self, _hold: Hold) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_follows_playback_and_video() {
        assert_eq!(hold_for(false, true), Hold::None);
        assert_eq!(hold_for(true, false), Hold::System);
        assert_eq!(hold_for(true, true), Hold::Display);
    }
}

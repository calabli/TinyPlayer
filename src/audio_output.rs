//! Follows the OS default output device, and gives each device its own volume:
//! headphones stay quiet after the laptop speakers were turned up. mpv can list
//! devices but can't say which one "auto" is using, so this asks the OS.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

/// Short, because a change of device plays at the old volume until it's seen.
const POLL: Duration = Duration::from_millis(300);
/// Dragging the volume slider shouldn't write to disk on every frame.
const SAVE_AFTER_SECS: f64 = 1.0;
/// mpv answers quickly. If the volume it reports still differs after this, the
/// user changed it in the meantime.
const APPLY_TIMEOUT_SECS: f64 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub struct OutputDevice {
    /// Stable across restarts and across being unplugged.
    pub id: String,
    pub name: String,
}

/// Watches the default output device on its own thread.
pub struct OutputMonitor {
    latest: Arc<Mutex<Option<OutputDevice>>>,
}

impl OutputMonitor {
    pub fn start(ctx: egui::Context) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&latest);
        let spawned = std::thread::Builder::new().name("audio-output".into()).spawn(move || {
            let Some(mut read) = os::reader() else { return };
            let mut last = None;
            loop {
                let device = read();
                if device.is_some() && device != last {
                    last.clone_from(&device);
                    if let Ok(mut slot) = shared.lock() {
                        *slot = device;
                    }
                    ctx.request_repaint();
                }
                std::thread::sleep(POLL);
            }
        });
        if let Err(e) = spawned {
            eprintln!("audio output monitor: {e}");
        }
        Self { latest }
    }

    /// The device, if it changed since the last call.
    pub fn take_change(&self) -> Option<OutputDevice> {
        self.latest.lock().ok()?.take()
    }
}

/// The volume remembered for each device.
#[derive(Default)]
pub struct DeviceVolumes {
    saved: HashMap<String, f64>,
    current: Option<String>,
    /// A volume just sent to mpv. Until mpv reports it, the volume it does
    /// report is the old device's and must not be saved for the new one.
    applying: Option<(f64, f64)>,
    /// When the volume last changed, while that isn't on disk yet.
    unsaved_since: Option<f64>,
}

impl DeviceVolumes {
    pub fn new(saved: HashMap<String, f64>) -> Self {
        Self { saved, ..Self::default() }
    }

    /// Returns the volume to switch to. A device seen for the first time has
    /// none, and keeps what's playing now.
    pub fn switch_to(&mut self, id: &str, volume: f64, now: f64) -> Option<f64> {
        self.current = Some(id.to_string());
        self.unsaved_since = None;
        let target = self.saved.get(id).copied();
        self.applying = target.map(|t| (t, now));
        if target.is_none() {
            self.saved.insert(id.to_string(), volume);
            self.unsaved_since = Some(f64::NEG_INFINITY);
        }
        target
    }

    /// Call with the volume mpv reports.
    pub fn observe(&mut self, volume: f64, now: f64) {
        if let Some((target, since)) = self.applying {
            if (volume - target).abs() >= 0.01 && now - since < APPLY_TIMEOUT_SECS {
                return;
            }
            self.applying = None;
        }
        let Some(id) = &self.current else { return };
        if self.saved.get(id).is_none_or(|v| (v - volume).abs() >= 0.01) {
            self.saved.insert(id.clone(), volume);
            self.unsaved_since = Some(now);
        }
    }

    /// True while a change waits for its write, so an idle window wakes up.
    pub fn pending(&self) -> bool {
        self.unsaved_since.is_some()
    }

    /// What should be written now. `force` is for a device change and exit.
    pub fn take_unsaved(&mut self, now: f64, force: bool) -> Option<(String, f64)> {
        let since = self.unsaved_since?;
        if !force && now - since < SAVE_AFTER_SECS {
            return None;
        }
        self.unsaved_since = None;
        let id = self.current.clone()?;
        let volume = *self.saved.get(&id)?;
        Some((id, volume))
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::ffi::{c_char, c_void};
    use std::ptr;

    use super::OutputDevice;

    const fn code(tag: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*tag)
    }

    const SYSTEM_OBJECT: u32 = 1;
    const DEFAULT_OUTPUT: u32 = code(b"dOut");
    const DEVICE_UID: u32 = code(b"uid ");
    const OBJECT_NAME: u32 = code(b"lnam");
    const SCOPE_GLOBAL: u32 = code(b"glob");
    const UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(object: *const c_void);
    }

    fn property<T: Copy>(object: u32, selector: u32, empty: T) -> Option<T> {
        let address = Address { selector, scope: SCOPE_GLOBAL, element: 0 };
        let mut value = empty;
        let mut size = size_of::<T>() as u32;
        // SAFETY: `value` is `size` bytes, and T matches the property's type.
        let status = unsafe {
            AudioObjectGetPropertyData(object, &address, 0, ptr::null(), &mut size, (&raw mut value).cast())
        };
        (status == 0).then_some(value)
    }

    fn string(object: u32, selector: u32) -> Option<String> {
        let cf = property::<*const c_void>(object, selector, ptr::null())?;
        if cf.is_null() {
            return None;
        }
        let mut buffer = [0 as c_char; 512];
        // SAFETY: the property hands over a CFString we own, and `buffer` is
        // as long as we say.
        let ok = unsafe {
            let ok = CFStringGetCString(cf, buffer.as_mut_ptr(), buffer.len() as isize, UTF8);
            CFRelease(cf);
            ok != 0
        };
        // SAFETY: on success the buffer holds a terminated string.
        ok.then(|| unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned())
    }

    pub fn reader() -> Option<impl FnMut() -> Option<OutputDevice>> {
        Some(|| {
            let device = property(SYSTEM_OBJECT, DEFAULT_OUTPUT, 0u32).filter(|&d| d != 0)?;
            let id = string(device, DEVICE_UID)?;
            let name = string(device, OBJECT_NAME).unwrap_or_default();
            Some(OutputDevice { id, name })
        })
    }
}

#[cfg(windows)]
mod os {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
    };
    use windows::core::PWSTR;

    use super::OutputDevice;

    /// Reads a string allocated by COM, and frees it.
    fn take(text: PWSTR) -> Option<String> {
        // SAFETY: `text` is a terminated string from COM, freed once.
        unsafe {
            let out = text.to_string().ok();
            CoTaskMemFree(Some(text.0.cast()));
            out
        }
    }

    fn name(device: &IMMDevice) -> Option<String> {
        // SAFETY: plain COM calls on a live device.
        unsafe {
            let store = device.OpenPropertyStore(STGM_READ).ok()?;
            let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
            take(PropVariantToStringAlloc(&value).ok()?)
        }
    }

    pub fn reader() -> Option<impl FnMut() -> Option<OutputDevice>> {
        // SAFETY: this thread does nothing else with COM, and uses it for life.
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?
        };
        Some(move || {
            // SAFETY: plain COM calls on live objects.
            let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
            let id = take(unsafe { device.GetId() }.ok()?)?;
            let name = name(&device).unwrap_or_default();
            Some(OutputDevice { id, name })
        })
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod os {
    use super::OutputDevice;

    pub fn reader() -> Option<fn() -> Option<OutputDevice>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> HashMap<String, f64> {
        HashMap::from([("phones".to_string(), 25.0)])
    }

    #[test]
    fn a_known_device_gets_its_volume_back() {
        let mut v = DeviceVolumes::new(saved());
        assert_eq!(v.switch_to("phones", 60.0, 0.0), Some(25.0));
    }

    #[test]
    fn a_new_device_keeps_the_volume_and_is_remembered() {
        let mut v = DeviceVolumes::new(HashMap::new());
        assert_eq!(v.switch_to("speaker", 60.0, 0.0), None);
        assert_eq!(v.take_unsaved(0.0, false), Some(("speaker".to_string(), 60.0)));
    }

    #[test]
    fn the_old_volume_is_not_saved_for_the_new_device() {
        let mut v = DeviceVolumes::new(saved());
        v.switch_to("phones", 60.0, 0.0);
        v.observe(60.0, 0.1);
        assert!(!v.pending());
        v.observe(25.0, 0.2);
        assert!(!v.pending());
        v.observe(30.0, 1.2);
        assert_eq!(v.take_unsaved(9.0, false), Some(("phones".to_string(), 30.0)));
    }

    #[test]
    fn a_change_made_while_switching_is_still_saved() {
        let mut v = DeviceVolumes::new(saved());
        v.switch_to("phones", 60.0, 0.0);
        v.observe(45.0, 0.1);
        assert!(!v.pending());
        v.observe(45.0, 0.6);
        assert_eq!(v.take_unsaved(9.0, false), Some(("phones".to_string(), 45.0)));
    }

    #[test]
    fn a_change_waits_until_the_volume_settles() {
        let mut v = DeviceVolumes::new(HashMap::new());
        v.switch_to("speaker", 60.0, 0.0);
        v.take_unsaved(0.0, true);
        v.observe(65.0, 10.0);
        assert_eq!(v.take_unsaved(10.5, false), None);
        assert_eq!(v.take_unsaved(11.5, false), Some(("speaker".to_string(), 65.0)));
        assert!(!v.pending());
    }

    #[test]
    fn leaving_a_device_writes_its_change_at_once() {
        let mut v = DeviceVolumes::new(saved());
        v.switch_to("phones", 60.0, 0.0);
        v.observe(25.0, 0.0);
        v.observe(40.0, 1.0);
        assert_eq!(v.take_unsaved(1.0, true), Some(("phones".to_string(), 40.0)));
    }

    #[test]
    fn nothing_is_saved_before_a_device_is_known() {
        let mut v = DeviceVolumes::new(HashMap::new());
        v.observe(70.0, 0.0);
        assert!(!v.pending());
    }
}

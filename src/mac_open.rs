//! Files opened from Finder or "Open With". macOS doesn't put them on the
//! command line: it sends an Apple Event to the running app, and winit doesn't
//! pass that on, so this installs its own handler.

use std::sync::{Mutex, OnceLock};

use eframe::egui;

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WAKE: OnceLock<egui::Context> = OnceLock::new();

/// Files received since the last call. Some arrive before the window exists.
pub fn take() -> Vec<String> {
    PENDING.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default()
}

/// Repaints with `ctx` when files arrive, so an idle window picks them up.
pub fn set_wake(ctx: egui::Context) {
    let _ = WAKE.set(ctx);
}

#[cfg(target_os = "macos")]
pub use imp::install;

#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString};

    // Sent with msg_send!: the typed bindings need a whole extra CoreServices
    // crate just for these four-char codes.
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "TinyPlayerOpenHandler"]
        struct OpenHandler;

        impl OpenHandler {
            #[unsafe(method(willFinishLaunching:))]
            fn will_finish_launching(&self, _note: &NSNotification) {
                let manager = NSAppleEventManager::sharedAppleEventManager();
                // SAFETY: the selector matches the method below, and `install`
                // keeps this object alive for the life of the process.
                let _: () = unsafe {
                    msg_send![
                        &*manager,
                        setEventHandler: self,
                        andSelector: sel!(handleOpen:withReply:),
                        forEventClass: CORE_EVENT_CLASS,
                        andEventID: OPEN_DOCUMENTS
                    ]
                };
            }

            #[unsafe(method(handleOpen:withReply:))]
            fn handle_open(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                // SAFETY: takes a four-char code and returns a descriptor or nil.
                let list: Option<Retained<NSAppleEventDescriptor>> =
                    unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
                let Some(list) = list else { return };
                // Apple Event lists count from 1.
                let paths = (1..=list.numberOfItems()).filter_map(|i| {
                    let url = list.descriptorAtIndex(i)?.fileURLValue()?;
                    Some(url.path()?.to_string())
                });
                if let Ok(mut pending) = super::PENDING.lock() {
                    pending.extend(paths);
                }
                if let Some(ctx) = super::WAKE.get() {
                    ctx.request_repaint();
                }
            }
        }
    );

    /// Call before the event loop starts. The handler goes in while the app
    /// finishes launching: AppKit overwrites one installed earlier, and the
    /// event for the file that launched the app arrives right after.
    pub fn install() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        // SAFETY: NSObject's plain `init` is enough for this field-less class.
        let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::alloc(mtm), init] };
        let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
        // SAFETY: the selector matches the method above, and the object is
        // leaked below, so it outlives the registration.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &handler,
                sel!(willFinishLaunching:),
                Some(&name),
                None,
            );
        }
        std::mem::forget(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_empties_the_queue() {
        PENDING.lock().unwrap().push("/a.mp4".to_string());
        assert_eq!(take(), ["/a.mp4"]);
        assert!(take().is_empty());
    }
}

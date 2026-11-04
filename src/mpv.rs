//! Minimal libmpv bindings, loaded at runtime with `libloading`.
//!
//! Loading at runtime (instead of linking) means the app builds without an
//! import library and can show a clear error if libmpv is missing.

use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::ptr;

use libloading::Library;

// ---- C types -------------------------------------------------------------

#[repr(C)]
struct RawHandle {
    _p: [u8; 0],
}
#[repr(C)]
struct RawRenderCtx {
    _p: [u8; 0],
}

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawRenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawGlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct RawGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_NONE: c_int = 0;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PROPERTY_CHANGE: c_int = 22;

const RENDER_PARAM_INVALID: c_int = 0;
const RENDER_PARAM_API_TYPE: c_int = 1;
const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const RENDER_PARAM_OPENGL_FBO: c_int = 3;
const RENDER_PARAM_FLIP_Y: c_int = 4;

const RENDER_UPDATE_FRAME: u64 = 1;

type Callback = unsafe extern "C" fn(*mut c_void);

/// Function table resolved from the shared library.
struct Api {
    create: unsafe extern "C" fn() -> *mut RawHandle,
    initialize: unsafe extern "C" fn(*mut RawHandle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut RawHandle),
    set_option_string: unsafe extern "C" fn(*mut RawHandle, *const c_char, *const c_char) -> c_int,
    set_property: unsafe extern "C" fn(*mut RawHandle, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(*mut RawHandle, *mut *const c_char) -> c_int,
    observe_property: unsafe extern "C" fn(*mut RawHandle, u64, *const c_char, c_int) -> c_int,
    wait_event: unsafe extern "C" fn(*mut RawHandle, c_double) -> *mut RawEvent,
    set_wakeup_callback: unsafe extern "C" fn(*mut RawHandle, Option<Callback>, *mut c_void),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    render_create:
        unsafe extern "C" fn(*mut *mut RawRenderCtx, *mut RawHandle, *mut RawRenderParam) -> c_int,
    render_set_update_callback:
        unsafe extern "C" fn(*mut RawRenderCtx, Option<Callback>, *mut c_void),
    render_update: unsafe extern "C" fn(*mut RawRenderCtx) -> u64,
    render_render: unsafe extern "C" fn(*mut RawRenderCtx, *mut RawRenderParam) -> c_int,
    render_free: unsafe extern "C" fn(*mut RawRenderCtx),
    _lib: Library,
}

impl Api {
    fn load() -> Result<Self, String> {
        let lib = open_library()?;
        macro_rules! sym {
            ($name:literal) => {
                unsafe {
                    *lib.get(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("libmpv is missing {}: {e}", $name))?
                }
            };
        }
        Ok(Self {
            create: sym!("mpv_create"),
            initialize: sym!("mpv_initialize"),
            terminate_destroy: sym!("mpv_terminate_destroy"),
            set_option_string: sym!("mpv_set_option_string"),
            set_property: sym!("mpv_set_property"),
            command: sym!("mpv_command"),
            observe_property: sym!("mpv_observe_property"),
            wait_event: sym!("mpv_wait_event"),
            set_wakeup_callback: sym!("mpv_set_wakeup_callback"),
            error_string: sym!("mpv_error_string"),
            render_create: sym!("mpv_render_context_create"),
            render_set_update_callback: sym!("mpv_render_context_set_update_callback"),
            render_update: sym!("mpv_render_context_update"),
            render_render: sym!("mpv_render_context_render"),
            render_free: sym!("mpv_render_context_free"),
            _lib: lib,
        })
    }
}

fn library_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so"]
    }
}

fn open_library() -> Result<Library, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut candidates = Vec::new();
    for name in library_names() {
        if let Some(dir) = &exe_dir {
            candidates.push(dir.join(name));
            // Inside a macOS .app bundle: Contents/MacOS/../Frameworks
            candidates.push(dir.join("../Frameworks").join(name));
        }
        candidates.push(name.into());
    }
    let mut last_err = String::new();
    for path in candidates {
        match unsafe { Library::new(&path) } {
            Ok(lib) => return Ok(lib),
            Err(e) => last_err = format!("{}: {e}", path.display()),
        }
    }
    Err(format!("couldn't load libmpv ({last_err})"))
}

// ---- Public API ----------------------------------------------------------

/// Values reported by observed properties.
#[derive(Debug, Clone)]
pub enum PropValue {
    Double(f64),
    Flag(bool),
    Int(i64),
    Str(String),
    None,
}

#[derive(Debug, Clone)]
pub enum Event {
    PropertyChange { name: String, value: PropValue },
    FileLoaded,
    EndFile,
    Shutdown,
}

/// Property formats for [`Mpv::observe`].
#[derive(Clone, Copy)]
pub enum Format {
    Double,
    Flag,
    Int,
    Str,
}

pub struct Mpv {
    api: Api,
    handle: *mut RawHandle,
}

// The mpv client API is thread-safe.
unsafe impl Send for Mpv {}

impl Mpv {
    /// Creates and initializes an mpv instance configured for embedding.
    pub fn new(options: &[(&str, &str)]) -> Result<Self, String> {
        let api = Api::load()?;
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("mpv_create failed".into());
        }
        let mpv = Self { api, handle };
        for (k, v) in options {
            mpv.set_option(k, v)?;
        }
        mpv.check(unsafe { (mpv.api.initialize)(mpv.handle) }, "mpv_initialize")?;
        Ok(mpv)
    }

    fn check(&self, code: c_int, what: &str) -> Result<(), String> {
        if code >= 0 {
            return Ok(());
        }
        let msg = unsafe { CStr::from_ptr((self.api.error_string)(code)) };
        Err(format!("{what}: {}", msg.to_string_lossy()))
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) = (cstr(name), cstr(value));
        let code = unsafe { (self.api.set_option_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        self.check(code, &format!("option {name}={value}"))
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        let code = unsafe { (self.api.command)(self.handle, ptrs.as_mut_ptr()) };
        self.check(code, &format!("command {}", args.first().unwrap_or(&"")))
    }

    pub fn set_double(&self, name: &str, mut value: f64) {
        let n = cstr(name);
        unsafe {
            (self.api.set_property)(self.handle, n.as_ptr(), FORMAT_DOUBLE, (&raw mut value).cast());
        }
    }

    pub fn set_flag(&self, name: &str, value: bool) {
        let n = cstr(name);
        let mut v: c_int = value.into();
        unsafe {
            (self.api.set_property)(self.handle, n.as_ptr(), FORMAT_FLAG, (&raw mut v).cast());
        }
    }

    pub fn observe(&self, name: &str, format: Format) {
        let n = cstr(name);
        let f = match format {
            Format::Double => FORMAT_DOUBLE,
            Format::Flag => FORMAT_FLAG,
            Format::Int => FORMAT_INT64,
            Format::Str => FORMAT_STRING,
        };
        unsafe {
            (self.api.observe_property)(self.handle, 0, n.as_ptr(), f);
        }
    }

    /// Calls `f` from an mpv thread whenever new events are queued.
    pub fn set_wakeup(&self, f: Box<dyn Fn() + Send + Sync>) {
        let data = Box::into_raw(Box::new(f));
        unsafe { (self.api.set_wakeup_callback)(self.handle, Some(trampoline), data.cast()) };
        // Leaked on purpose: mpv may call it until the handle is destroyed,
        // and there is exactly one per player for the app's lifetime.
    }

    /// Drains all pending events without blocking.
    pub fn poll_events(&self) -> Vec<Event> {
        let mut out = Vec::new();
        loop {
            let ev = unsafe { &*(self.api.wait_event)(self.handle, 0.0) };
            match ev.event_id {
                EVENT_NONE => break,
                EVENT_SHUTDOWN => out.push(Event::Shutdown),
                EVENT_FILE_LOADED => out.push(Event::FileLoaded),
                EVENT_END_FILE => out.push(Event::EndFile),
                EVENT_PROPERTY_CHANGE if !ev.data.is_null() => {
                    let p = unsafe { &*(ev.data as *const RawEventProperty) };
                    let name = unsafe { CStr::from_ptr(p.name) }.to_string_lossy().into_owned();
                    let value = unsafe { read_value(p.format, p.data) };
                    out.push(Event::PropertyChange { name, value });
                }
                _ => {}
            }
        }
        out
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}

unsafe fn read_value(format: c_int, data: *mut c_void) -> PropValue {
    if data.is_null() {
        return PropValue::None;
    }
    unsafe {
        match format {
            FORMAT_DOUBLE => PropValue::Double(*(data as *const f64)),
            FORMAT_FLAG => PropValue::Flag(*(data as *const c_int) != 0),
            FORMAT_INT64 => PropValue::Int(*(data as *const i64)),
            FORMAT_STRING => {
                let s = *(data as *const *const c_char);
                if s.is_null() {
                    PropValue::None
                } else {
                    PropValue::Str(CStr::from_ptr(s).to_string_lossy().into_owned())
                }
            }
            _ => PropValue::None,
        }
    }
}

unsafe extern "C" fn trampoline(data: *mut c_void) {
    let f = unsafe { &*(data as *const Box<dyn Fn() + Send + Sync>) };
    f();
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

// ---- OpenGL render context -----------------------------------------------

type ProcAddr = dyn Fn(&CStr) -> *const c_void + Send + Sync;

/// Renders video frames into an OpenGL framebuffer supplied by the app.
pub struct RenderContext {
    ctx: *mut RawRenderCtx,
    render_update: unsafe extern "C" fn(*mut RawRenderCtx) -> u64,
    render_render: unsafe extern "C" fn(*mut RawRenderCtx, *mut RawRenderParam) -> c_int,
    render_free: unsafe extern "C" fn(*mut RawRenderCtx),
    _proc: Box<std::sync::Arc<ProcAddr>>,
    _update_cb: Box<Box<dyn Fn() + Send + Sync>>,
}

impl RenderContext {
    /// Must be called on the thread that owns the GL context.
    pub fn new(
        mpv: &Mpv,
        get_proc_address: std::sync::Arc<ProcAddr>,
        on_update: Box<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        let proc_box = Box::new(get_proc_address);
        let mut init = RawGlInitParams {
            get_proc_address: gl_proc_trampoline,
            get_proc_address_ctx: (&raw const *proc_box) as *mut c_void,
        };
        let api_type = c"opengl";
        let mut params = [
            RawRenderParam { kind: RENDER_PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
            RawRenderParam {
                kind: RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&raw mut init).cast(),
            },
            RawRenderParam { kind: RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        let mut ctx = ptr::null_mut();
        let code = unsafe { (mpv.api.render_create)(&mut ctx, mpv.handle, params.as_mut_ptr()) };
        mpv.check(code, "mpv_render_context_create")?;

        let update_cb = Box::new(on_update);
        unsafe {
            (mpv.api.render_set_update_callback)(
                ctx,
                Some(trampoline),
                (&raw const *update_cb) as *mut c_void,
            );
        }
        Ok(Self {
            ctx,
            render_update: mpv.api.render_update,
            render_render: mpv.api.render_render,
            render_free: mpv.api.render_free,
            _proc: proc_box,
            _update_cb: update_cb,
        })
    }

    /// Returns true if a new video frame is ready to be rendered.
    pub fn frame_ready(&self) -> bool {
        unsafe { (self.render_update)(self.ctx) & RENDER_UPDATE_FRAME != 0 }
    }

    /// Renders the current frame into `fbo` (0 = default framebuffer).
    pub fn render(&self, fbo: i32, w: i32, h: i32, flip_y: bool) {
        let mut target = RawGlFbo { fbo, w, h, internal_format: 0 };
        let mut flip: c_int = flip_y.into();
        let mut params = [
            RawRenderParam { kind: RENDER_PARAM_OPENGL_FBO, data: (&raw mut target).cast() },
            RawRenderParam { kind: RENDER_PARAM_FLIP_Y, data: (&raw mut flip).cast() },
            RawRenderParam { kind: RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        unsafe { (self.render_render)(self.ctx, params.as_mut_ptr()) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // Must happen before the mpv handle is destroyed.
        unsafe { (self.render_free)(self.ctx) };
    }
}

unsafe extern "C" fn gl_proc_trampoline(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let f = unsafe { &*(ctx as *const std::sync::Arc<ProcAddr>) };
    f(unsafe { CStr::from_ptr(name) }) as *mut c_void
}

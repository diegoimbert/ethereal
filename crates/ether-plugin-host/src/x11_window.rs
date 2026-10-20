//! Linux (X11) host windows for plugin editors that embed into a parent the host makes:
//! VST3 (`X11EmbedWindowID`) and VST2 (`effEditOpen` with the window id). CLAP editors on
//! Linux float on their own (`ether-clap`), so they don't use this.
//!
//! An [`X11Window`] is a plain top-level window on **its own** X connection (Xlib loaded at
//! runtime, `x11-dl`: no link-time dependency, so the app still starts without libX11). The
//! plugin creates its view as a child of [`X11Window::id`] on its own connection (window ids
//! are server-wide). Our connection only sees our window's events, read without blocking by
//! [`X11Window::pump`] from the format's `poll` (the plugin main thread, ~30-60 Hz):
//!
//! - `WM_DELETE_WINDOW` (the user closed it): the window is unmapped and reports not visible;
//!   the format's poll then closes the plugin view first and the window after (the same order
//!   as on Windows, where closing only hides the window too);
//! - `ConfigureNotify`: the new size, for user resizes of resizable editors.
//!
//! The window is the app main window's transient (`WM_TRANSIENT_FOR`,
//! [`crate::editor_owner::transient`]) when the app registered an X11 one, so window managers
//! keep it above the app. Wayland sessions use it through XWayland; without an X display
//! ([`available`] false) formats report no editor.

use std::cell::Cell;
use std::ffi::{CString, c_int, c_long, c_uint, c_ulong};
use std::sync::OnceLock;

use x11_dl::xlib::{self, Xlib};

use crate::editor_owner::{self, OwnerWindow};

/// Xlib, loaded once (`None`: libX11 is not installed). Shared with other X11 code of the
/// process (an extension capturing or driving editor windows).
pub fn xlib() -> Option<&'static Xlib> {
    static XLIB: OnceLock<Option<Xlib>> = OnceLock::new();
    XLIB.get_or_init(|| match Xlib::open() {
        Ok(x) => Some(x),
        Err(e) => {
            tracing::info!(%e, "libX11 not available: no embedded plugin editors");
            None
        }
    })
    .as_ref()
}

/// An Xlib error handler (`XSetErrorHandler`'s type).
type ErrorHandler = unsafe extern "C" fn(*mut xlib::Display, *mut xlib::XErrorEvent) -> c_int;

/// The handler that was installed before ours (Xlib's default, which exits the process, or
/// one an in-process plugin installed): errors outside [`trapped`] still go there.
static CHAINED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    /// `Some(first error code, 0: none)` while this thread runs [`trapped`].
    static TRAP: Cell<Option<u8>> = const { Cell::new(None) };
}

/// The one process-wide handler: records the error for the thread that is trapping (an Xlib
/// connection's errors are delivered on the thread that uses it), else chains.
unsafe extern "C" fn on_error(d: *mut xlib::Display, e: *mut xlib::XErrorEvent) -> c_int {
    let trapping = TRAP.with(|t| match t.get() {
        Some(0) => {
            // SAFETY: Xlib passes a valid event for the duration of the call.
            t.set(Some(unsafe { (*e).error_code }.max(1)));
            true
        }
        Some(_) => true,
        None => false,
    });
    if trapping {
        return 0;
    }
    match CHAINED.load(std::sync::atomic::Ordering::Relaxed) {
        0 => 0,
        // SAFETY: a handler pointer stored by `trapped` from `XSetErrorHandler`.
        f => unsafe { std::mem::transmute::<usize, ErrorHandler>(f)(d, e) },
    }
}

/// Run `f` (Xlib calls on `display`) and a round trip, with X errors recorded instead of
/// fatal (Xlib's default handler exits the process on a `BadWindow`); the first error's
/// code (0: none). One handler is installed process-wide and never removed (setting and
/// restoring it per call raced between threads: capture, injection, window lookup); the
/// trap state is per thread. Nested calls are fine. Used by every X11 caller in this crate,
/// and to be used by any other X11 code of the process.
///
/// # Safety
///
/// `display` is an open connection of `x` (it is synced after `f`).
pub unsafe fn trapped<T>(x: &Xlib, display: *mut xlib::Display, f: impl FnOnce() -> T) -> (T, u8) {
    // SAFETY: installing a valid handler; reinstalled when something replaced it (a plugin
    // in the same process), whose handler is then chained.
    unsafe {
        let previous = (x.XSetErrorHandler)(Some(on_error));
        if let Some(p) = previous
            && p as usize != on_error as ErrorHandler as usize
        {
            CHAINED.store(p as usize, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let outer = TRAP.with(|t| t.replace(Some(0)));
    let value = f();
    // SAFETY: the caller's connection: delivers `f`'s errors now, on this thread.
    unsafe { (x.XSync)(display, 0) };
    let code = TRAP.with(|t| t.replace(outer)).unwrap_or(0);
    (value, code)
}

/// Whether editor windows can be made: libX11 loads and the X display (`DISPLAY`) opens.
/// Checked once per process.
pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Some(x) = xlib() else {
            return false;
        };
        // SAFETY: plain connection test; closed right away.
        unsafe {
            let display = (x.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return false;
            }
            (x.XCloseDisplay)(display);
        }
        true
    })
}

/// A top-level X11 window hosting a plugin's view. Not `Send`: use it on the plugin main
/// thread, like the plugin's editor.
pub struct X11Window {
    x: &'static Xlib,
    display: *mut xlib::Display,
    window: c_ulong,
    wm_delete: c_ulong,
    resizable: bool,
    /// Size from the last `ConfigureNotify` (or the last resize we made).
    size: Cell<(u32, u32)>,
    /// The user closed the window (`WM_DELETE_WINDOW`).
    closed: Cell<bool>,
}

impl X11Window {
    /// Create (not yet mapped) a `width`×`height` window titled `title`. Fixed size unless
    /// `resizable`.
    pub fn open(title: &str, width: u32, height: u32, resizable: bool) -> Result<Self, String> {
        let x = xlib().ok_or("libX11 is not available")?;
        // SAFETY: Xlib calls on a connection we own; every pointer passed is valid for the call.
        unsafe {
            let display = (x.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return Err("cannot open the X display (is DISPLAY set?)".into());
            }
            let screen = (x.XDefaultScreen)(display);
            let root = (x.XDefaultRootWindow)(display);
            let black = (x.XBlackPixel)(display, screen);
            let (w, h) = (width.max(1), height.max(1));
            let window = (x.XCreateSimpleWindow)(display, root, 0, 0, w, h, 0, black, black);
            if window == 0 {
                (x.XCloseDisplay)(display);
                return Err("XCreateSimpleWindow failed".into());
            }
            let title = CString::new(title.replace('\0', " ")).unwrap_or_default();
            (x.XStoreName)(display, window, title.as_ptr());
            let mut wm_delete = (x.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            (x.XSetWMProtocols)(display, window, &mut wm_delete, 1);
            (x.XSelectInput)(display, window, xlib::StructureNotifyMask);
            if let Some(OwnerWindow::X11(owner)) = editor_owner::transient() {
                (x.XSetTransientForHint)(display, window, owner as c_ulong);
            }
            let me = Self {
                x,
                display,
                window,
                wm_delete,
                resizable,
                size: Cell::new((w, h)),
                closed: Cell::new(false),
            };
            me.set_size_hints(w, h);
            (x.XFlush)(display);
            Ok(me)
        }
    }

    /// The window id (what the plugin's view is parented to).
    pub fn id(&self) -> u64 {
        self.window
    }

    /// Fixed-size windows: min = max = the size; resizable ones: a small minimum.
    fn set_size_hints(&self, width: u32, height: u32) {
        // SAFETY: zeroed plain C struct; our window.
        unsafe {
            let mut hints: xlib::XSizeHints = std::mem::zeroed();
            hints.flags = xlib::PSize | xlib::PMinSize;
            hints.width = width as c_int;
            hints.height = height as c_int;
            if self.resizable {
                (hints.min_width, hints.min_height) = (32, 32);
            } else {
                hints.flags |= xlib::PMaxSize;
                (hints.min_width, hints.min_height) = (width as c_int, height as c_int);
                (hints.max_width, hints.max_height) = (width as c_int, height as c_int);
            }
            (self.x.XSetWMNormalHints)(self.display, self.window, &mut hints);
        }
    }

    /// Map (and raise) the window; reopens one the user closed.
    pub fn show(&self) {
        self.closed.set(false);
        // SAFETY: our live window.
        unsafe {
            (self.x.XMapRaised)(self.display, self.window);
            (self.x.XFlush)(self.display);
        }
    }

    /// Handle the window's pending events (never blocks).
    pub fn pump(&self) {
        // SAFETY: our connection; `XNextEvent` only runs when an event is queued.
        unsafe {
            while (self.x.XPending)(self.display) > 0 {
                let mut ev: xlib::XEvent = std::mem::zeroed();
                (self.x.XNextEvent)(self.display, &mut ev);
                match ev.get_type() {
                    xlib::ClientMessage => {
                        let m = ev.client_message;
                        if m.window == self.window
                            && m.data.get_long(0) as c_ulong == self.wm_delete
                        {
                            self.closed.set(true);
                            (self.x.XUnmapWindow)(self.display, self.window);
                            (self.x.XFlush)(self.display);
                        }
                    }
                    xlib::ConfigureNotify => {
                        let c = ev.configure;
                        if c.window == self.window && c.width > 0 && c.height > 0 {
                            self.size.set((c.width as u32, c.height as u32));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// False once the user closed the window (events are pumped first).
    pub fn is_visible(&self) -> bool {
        self.pump();
        !self.closed.get()
    }

    /// The window's size as last configured.
    pub fn size(&self) -> (u32, u32) {
        self.size.get()
    }

    /// Resize the window (and its fixed-size hints).
    pub fn resize(&self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        self.set_size_hints(w, h);
        // SAFETY: our live window.
        unsafe {
            (self.x.XResizeWindow)(self.display, self.window, w as c_uint, h as c_uint);
            (self.x.XFlush)(self.display);
        }
        self.size.set((w, h));
    }

    /// Test support: ask the window to close as a window manager would (a
    /// `WM_DELETE_WINDOW` client message from another connection).
    #[doc(hidden)]
    pub fn send_close_request(window: u64) -> bool {
        let Some(x) = xlib() else {
            return false;
        };
        // SAFETY: a short-lived connection sending one client message.
        unsafe {
            let display = (x.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return false;
            }
            let mut ev: xlib::XEvent = std::mem::zeroed();
            let mut m: xlib::XClientMessageEvent = std::mem::zeroed();
            m.type_ = xlib::ClientMessage;
            m.window = window as c_ulong;
            m.message_type = (x.XInternAtom)(display, c"WM_PROTOCOLS".as_ptr(), 0);
            m.format = 32;
            m.data.set_long(
                0,
                (x.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0) as c_long,
            );
            ev.client_message = m;
            let ok = (x.XSendEvent)(display, window as c_ulong, 0, 0, &mut ev) != 0;
            (x.XSync)(display, 0);
            (x.XCloseDisplay)(display);
            ok
        }
    }

    /// Test support: the window's current size and whether it is mapped, from another
    /// connection (`None`: no such window).
    #[doc(hidden)]
    pub fn query(window: u64) -> Option<(u32, u32, bool)> {
        let x = xlib()?;
        // SAFETY: a short-lived connection; plain C out-structs; a destroyed window's
        // `BadWindow` is trapped.
        unsafe {
            let display = (x.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return None;
            }
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            let (ok, code) = trapped(x, display, || {
                (x.XGetWindowAttributes)(display, window as c_ulong, &mut attrs) != 0
            });
            let ok = ok && code == 0;
            (x.XCloseDisplay)(display);
            ok.then_some((
                attrs.width as u32,
                attrs.height as u32,
                attrs.map_state == xlib::IsViewable,
            ))
        }
    }
}

impl Drop for X11Window {
    fn drop(&mut self) {
        // SAFETY: our window and connection, destroyed once.
        unsafe {
            (self.x.XDestroyWindow)(self.display, self.window);
            (self.x.XCloseDisplay)(self.display);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs an X display (`DISPLAY`, e.g. Xvfb); skipped without one.
    #[test]
    fn window_lifecycle() {
        if !available() {
            eprintln!("skipped: no X display");
            return;
        }
        let w = X11Window::open("Ether test editor", 300, 200, false).expect("open");
        let id = w.id();
        assert_ne!(id, 0);
        w.show();
        assert!(w.is_visible());
        let wait = |f: &dyn Fn() -> bool| {
            for _ in 0..200 {
                w.pump();
                if f() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        };
        assert!(
            wait(&|| X11Window::query(id).is_some_and(|q| q.2)),
            "mapped"
        );
        assert_eq!(X11Window::query(id).map(|q| (q.0, q.1)), Some((300, 200)));

        w.resize(320, 240);
        assert!(wait(
            &|| X11Window::query(id).is_some_and(|q| (q.0, q.1) == (320, 240))
        ));
        assert_eq!(w.size(), (320, 240));

        // The user closes it: unmapped, reported closed; shown again on request.
        assert!(X11Window::send_close_request(id));
        assert!(wait(&|| !w.is_visible()));
        assert!(
            wait(&|| X11Window::query(id).is_some_and(|q| !q.2)),
            "unmapped"
        );
        w.show();
        assert!(w.is_visible());

        drop(w);
        assert!(
            X11Window::query(id).is_none() || {
                // Destroyed asynchronously: give the server a moment.
                std::thread::sleep(std::time::Duration::from_millis(50));
                X11Window::query(id).is_none()
            }
        );
    }
}

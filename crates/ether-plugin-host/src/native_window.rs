//! Plugin editor windows as [`NativeWindow`]s (`PluginController::editor_window`),
//! and finding the window a plugin opened on its own (CLAP floating editors).
//!
//! Host-created editor windows (VST3, VST2, AU, embedded CLAP) know their handle; a floating
//! CLAP editor is the plugin's own window, which CLAP doesn't expose. [`NewWindows`] finds it
//! by diffing this process's top-level windows before and after the plugin opened it:
//!
//! - X11: windows up to three levels below the root (window managers reparent clients into
//!   frames), preferring new ones whose `_NET_WM_PID` is ours, then ones with a `WM_CLASS` or
//!   name (frames have neither);
//! - Windows: top-level windows of this process (`EnumWindows` + the window's process id: a
//!   plugin may create its window on a thread of its own), preferring visible captioned ones;
//! - macOS: the app's `NSWindow`s.
//!
//! Best effort: `None` when nothing new showed up (the format retries on later polls).

use ether_core::plugin::{NativeWindow, WindowHandle};

/// `handle` as a window of this process.
pub fn current(handle: WindowHandle) -> NativeWindow {
    NativeWindow {
        pid: std::process::id(),
        handle,
    }
}

/// An `NSWindow` of this process.
#[cfg(target_os = "macos")]
pub fn appkit(window: &objc2_app_kit::NSWindow) -> NativeWindow {
    current(WindowHandle::AppKit {
        window_number: window.windowNumber() as i64,
        ns_window: window as *const objc2_app_kit::NSWindow as usize,
    })
}

/// A new window seen by [`NewWindows::find`], with what makes it look like the plugin's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Candidate {
    id: u64,
    /// Owned by this process (`_NET_WM_PID`; always true where the listing is per process).
    ours: bool,
    /// Looks like an application window: X11 `WM_CLASS`/name, a Win32 caption.
    titled: bool,
    visible: bool,
}

/// The most plausible plugin window among new ones: ours, then titled, then visible; the
/// first listed on ties. A window that is neither ours nor titled is never picked (window
/// manager frames, toolkit helper windows).
fn pick(candidates: &[Candidate]) -> Option<u64> {
    let score = |c: &Candidate| u8::from(c.ours) * 4 + u8::from(c.titled) * 2 + u8::from(c.visible);
    candidates
        .iter()
        .filter(|c| c.ours || c.titled)
        .fold(None::<&Candidate>, |best, c| match best {
            Some(b) if score(b) >= score(c) => Some(b),
            _ => Some(c),
        })
        .map(|c| c.id)
}

/// This process's top-level windows at a point in time (see the module docs). Take it on
/// the plugin main thread before the plugin opens its window, then [`NewWindows::find`].
#[derive(Debug, Default)]
pub struct NewWindows {
    before: std::collections::HashSet<u64>,
}

impl NewWindows {
    pub fn snapshot() -> Self {
        Self {
            before: sys::list().into_iter().map(|c| c.id).collect(),
        }
    }

    /// The window that appeared since the snapshot most likely to be the plugin's.
    pub fn find(&self) -> Option<NativeWindow> {
        let new: Vec<Candidate> = sys::list()
            .into_iter()
            .filter(|c| !self.before.contains(&c.id))
            .collect();
        pick(&new).and_then(sys::handle).map(current)
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use std::ffi::{c_long, c_uchar, c_uint, c_ulong};

    use x11_dl::xlib;

    use super::Candidate;
    use ether_core::plugin::WindowHandle;

    /// Window-manager nesting searched below the root (client, frame, wrapper).
    const DEPTH: usize = 3;

    pub(super) fn handle(id: u64) -> Option<WindowHandle> {
        Some(WindowHandle::X11 { window: id })
    }

    pub(super) fn list() -> Vec<Candidate> {
        let Some(x) = crate::x11_window::xlib() else {
            return Vec::new();
        };
        let pid = c_ulong::from(std::process::id());
        let mut out = Vec::new();
        // SAFETY: a short-lived connection; Xlib-allocated results are freed after use;
        // windows vanish between listing and inspecting: their `BadWindow`s are trapped.
        unsafe {
            let display = (x.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return out;
            }
            crate::x11_window::trapped(x, display, || {
                let net_wm_pid = (x.XInternAtom)(display, c"_NET_WM_PID".as_ptr(), 0);
                let children = |w: c_ulong| -> Vec<c_ulong> {
                    let (mut root, mut parent) = (0, 0);
                    let mut list: *mut c_ulong = std::ptr::null_mut();
                    let mut n: c_uint = 0;
                    if (x.XQueryTree)(display, w, &mut root, &mut parent, &mut list, &mut n) == 0
                        || list.is_null()
                    {
                        return Vec::new();
                    }
                    let v = std::slice::from_raw_parts(list, n as usize).to_vec();
                    (x.XFree)(list.cast());
                    v
                };
                let owner_pid = |w: c_ulong| -> Option<c_ulong> {
                    let (mut kind, mut format) = (0, 0);
                    let (mut n, mut after) = (0, 0);
                    let mut data: *mut c_uchar = std::ptr::null_mut();
                    let ok = (x.XGetWindowProperty)(
                        display,
                        w,
                        net_wm_pid,
                        0,
                        1,
                        0,
                        xlib::XA_CARDINAL,
                        &mut kind,
                        &mut format,
                        &mut n,
                        &mut after,
                        &mut data,
                    ) == 0;
                    let value = (ok && !data.is_null() && format == 32 && n == 1)
                        // Format-32 properties come back as C longs.
                        .then(|| *(data as *const c_long) as c_ulong);
                    if !data.is_null() {
                        (x.XFree)(data.cast());
                    }
                    value
                };
                let titled = |w: c_ulong| -> bool {
                    let mut hint: xlib::XClassHint = std::mem::zeroed();
                    let class = (x.XGetClassHint)(display, w, &mut hint) != 0;
                    for p in [hint.res_name, hint.res_class] {
                        if !p.is_null() {
                            (x.XFree)(p.cast());
                        }
                    }
                    let mut name = std::ptr::null_mut();
                    let named = (x.XFetchName)(display, w, &mut name) != 0 && !name.is_null();
                    if !name.is_null() {
                        (x.XFree)(name.cast());
                    }
                    class || named
                };
                let mut level = children((x.XDefaultRootWindow)(display));
                for depth in 0..DEPTH {
                    let mut next = Vec::new();
                    for &w in &level {
                        let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
                        if (x.XGetWindowAttributes)(display, w, &mut attrs) == 0
                            || attrs.class == xlib::InputOnly
                        {
                            continue;
                        }
                        if attrs.width > 1 && attrs.height > 1 {
                            out.push(Candidate {
                                id: w,
                                ours: owner_pid(w) == Some(pid),
                                titled: titled(w),
                                visible: attrs.map_state == xlib::IsViewable,
                            });
                        }
                        if depth + 1 < DEPTH {
                            next.extend(children(w));
                        }
                    }
                    level = next;
                }
            });
            (x.XCloseDisplay)(display);
        }
        out
    }
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_STYLE, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
        IsWindowVisible, WS_CAPTION,
    };

    use super::Candidate;
    use ether_core::plugin::WindowHandle;

    pub(super) fn handle(id: u64) -> Option<WindowHandle> {
        Some(WindowHandle::Win32 { hwnd: id as isize })
    }

    unsafe extern "system" fn collect(hwnd: HWND, out: LPARAM) -> i32 {
        // SAFETY: `out` is the `Vec` passed by `list` for the duration of `EnumWindows`;
        // plain queries on an enumerated window.
        unsafe {
            let out = &mut *(out as *mut Vec<Candidate>);
            let mut process = 0u32;
            GetWindowThreadProcessId(hwnd, &mut process);
            if process != GetCurrentProcessId() {
                return 1;
            }
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) == 0 || r.right - r.left <= 1 || r.bottom - r.top <= 1 {
                return 1;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            out.push(Candidate {
                id: hwnd as usize as u64,
                ours: true,
                titled: style & WS_CAPTION == WS_CAPTION,
                visible: IsWindowVisible(hwnd) != 0,
            });
        }
        1
    }

    pub(super) fn list() -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        // SAFETY: `collect` only uses `out` during the call.
        unsafe { EnumWindows(Some(collect), &mut out as *mut Vec<Candidate> as LPARAM) };
        out
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    use super::Candidate;
    use ether_core::plugin::WindowHandle;

    /// The app's window with that number (main thread).
    pub(super) fn handle(id: u64) -> Option<WindowHandle> {
        let mtm = MainThreadMarker::new()?;
        let window = NSApplication::sharedApplication(mtm).windowWithWindowNumber(id as isize)?;
        Some(super::appkit(&window).handle)
    }

    /// The app's windows by window number (main thread only; empty elsewhere).
    pub(super) fn list() -> Vec<Candidate> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Vec::new();
        };
        NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .filter(|w| w.windowNumber() > 0)
            .map(|w| Candidate {
                id: w.windowNumber() as u64,
                ours: true,
                titled: true,
                visible: w.isVisible(),
            })
            .collect()
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod sys {
    use super::Candidate;
    use ether_core::plugin::WindowHandle;

    pub(super) fn handle(_id: u64) -> Option<WindowHandle> {
        None
    }

    pub(super) fn list() -> Vec<Candidate> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: u64, ours: bool, titled: bool, visible: bool) -> Candidate {
        Candidate {
            id,
            ours,
            titled,
            visible,
        }
    }

    #[test]
    fn pick_prefers_ours_then_titled_then_visible() {
        assert_eq!(pick(&[]), None);
        // A bare frame (neither ours nor titled) is never the plugin's window.
        assert_eq!(pick(&[c(1, false, false, true)]), None);
        assert_eq!(
            pick(&[c(1, false, false, true), c(2, false, true, false)]),
            Some(2)
        );
        assert_eq!(
            pick(&[c(1, false, true, true), c(2, true, false, false)]),
            Some(2)
        );
        assert_eq!(
            pick(&[c(1, true, true, false), c(2, true, true, true)]),
            Some(2)
        );
        // Ties keep the first listed (outermost on X11).
        assert_eq!(
            pick(&[c(3, true, true, true), c(4, true, true, true)]),
            Some(3)
        );
    }

    #[test]
    fn current_is_this_process() {
        let w = current(WindowHandle::X11 { window: 7 });
        assert_eq!(w.pid, std::process::id());
        assert_eq!(w.handle, WindowHandle::X11 { window: 7 });
    }

    /// Needs an X display (`DISPLAY`, e.g. Xvfb); skipped without one.
    #[cfg(target_os = "linux")]
    #[test]
    fn finds_a_new_x11_window() {
        use crate::x11_window::{self, X11Window};
        if !x11_window::available() {
            eprintln!("skipped: no X display");
            return;
        }
        let before = NewWindows::snapshot();
        // Named (`XStoreName`), like a plugin's floating window.
        let w = X11Window::open("Floating plugin editor", 200, 100, false).expect("open");
        w.show();
        // Seen as new and plausible (other tests may open windows concurrently, so the pick
        // itself is only checked to find something).
        assert!(
            sys::list()
                .iter()
                .any(|c| c.id == w.id() && c.titled && !before.before.contains(&c.id))
        );
        assert!(before.find().is_some());
        // A later snapshot knows it.
        assert!(NewWindows::snapshot().before.contains(&w.id()));
    }
}

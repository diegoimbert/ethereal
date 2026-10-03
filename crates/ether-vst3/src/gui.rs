//! Plugin editors: the controller's `IPlugView` attached to a host-created window (the
//! approach of `ether-clap`'s `gui.rs`). macOS: an `NSWindow` whose content
//! `NSView` is the parent passed to `IPlugView::attached`; Windows: a top-level window whose
//! `HWND` is that parent. Elsewhere `HostWindow` is
//! uninhabited and plugins report no editor.
//!
//! Resizing: the plugin asks through `IPlugFrame::resizeView` (recorded, applied on the next
//! poll: window resized, then `IPlugView::onSize`); a user resize of the window is checked
//! with `checkSizeConstraint` and applied with `onSize`.

use std::sync::Mutex;

use vst3::Steinberg::{
    IPlugFrame, IPlugFrameTrait, IPlugView, ViewRect, kInvalidArgument, kResultOk, tresult,
};
use vst3::{Class, ComWrapper};

/// Editor size in points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn from_rect(r: &ViewRect) -> Self {
        Self {
            width: (r.right - r.left).max(0) as u32,
            height: (r.bottom - r.top).max(0) as u32,
        }
    }

    pub fn to_rect(self) -> ViewRect {
        ViewRect {
            left: 0,
            top: 0,
            right: self.width.min(i32::MAX as u32) as i32,
            bottom: self.height.min(i32::MAX as u32) as i32,
        }
    }
}

/// Default editor size when the view doesn't report one.
pub(crate) const DEFAULT_SIZE: Size = Size {
    width: 640,
    height: 480,
};

/// `IPlugFrame`: records the plugin's resize requests.
#[derive(Default)]
pub(crate) struct PlugFrame {
    pub resize: Mutex<Option<Size>>,
}

impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl PlugFrame {
    pub fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self::default())
    }

    pub fn take_resize(&self) -> Option<Size> {
        self.resize.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, _view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        if new_size.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked non-null.
        let size = Size::from_rect(unsafe { &*new_size });
        *self.resize.lock().unwrap_or_else(|e| e.into_inner()) = Some(size);
        kResultOk
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::HostWindow;

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    use super::Size;

    /// A plain titled `NSWindow` whose content view hosts the plugin's view.
    pub(crate) struct HostWindow {
        window: Retained<NSWindow>,
        size: Size,
    }

    impl HostWindow {
        pub const SUPPORTED: bool = true;
        /// The platform type passed to `IPlugView::attached`.
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"NSView";

        /// Whether the current thread is the process main thread (where editors live).
        pub fn on_main_thread() -> bool {
            MainThreadMarker::new().is_some()
        }

        /// Must be called on the process main thread (AppKit requirement).
        pub fn open(title: &str, size: Size, resizable: bool) -> Result<Self, String> {
            let mtm = MainThreadMarker::new()
                .ok_or("plugin editors must be opened on the main thread (AppKit)")?;
            let mut style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable;
            if resizable {
                style |= NSWindowStyleMask::Resizable;
            }
            let rect = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(f64::from(size.width), f64::from(size.height)),
            );
            // SAFETY: plain NSWindow initializer on the main thread (checked by `mtm`).
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect,
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // We own the window through `Retained`; AppKit must not release it on close.
            // SAFETY: called on the main thread before the window is shown.
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(&NSString::from_str(title));
            window.center();
            Ok(Self { window, size })
        }

        /// The content `NSView*` the plugin view gets attached to.
        pub fn view_ptr(&self) -> Option<*mut c_void> {
            let view = self.window.contentView()?;
            Some(Retained::as_ptr(&view) as *mut c_void)
        }

        pub fn show(&self) {
            self.window.makeKeyAndOrderFront(None);
        }

        /// False once the user closed the window.
        pub fn is_visible(&self) -> bool {
            self.window.isVisible()
        }

        pub fn content_size(&self) -> Size {
            let frame = self.window.contentRectForFrameRect(self.window.frame());
            Size {
                width: frame.size.width.max(0.0) as u32,
                height: frame.size.height.max(0.0) as u32,
            }
        }

        /// Last size applied or observed.
        pub fn known_size(&self) -> Size {
            self.size
        }

        pub fn set_known_size(&mut self, size: Size) {
            self.size = size;
        }

        pub fn resize(&mut self, size: Size) {
            self.window
                .setContentSize(NSSize::new(f64::from(size.width), f64::from(size.height)));
            self.size = size;
        }

        pub fn close(self) {
            self.window.close();
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::HostWindow;

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::sync::Once;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
        GetClientRect, GetSystemMetrics, IDC_ARROW, IsIconic, IsWindowVisible, LoadCursorW,
        RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE, WNDCLASSEXW,
        WS_CAPTION, WS_CLIPCHILDREN, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
        WS_THICKFRAME,
    };

    use super::Size;

    const CLASS_NAME: &str = "EtherealVst3Editor";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Closing the window only hides it: the plugin's child view must be `removed()` before
    /// its parent is destroyed, which `close` (via the editor poll) does in that order.
    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_CLOSE {
            // SAFETY: `hwnd` is the window this procedure was called for.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
        // SAFETY: default handling of a message for our own window.
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    fn register_class() {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let name = wide(CLASS_NAME);
            // SAFETY: plain class registration; the name is copied by `RegisterClassExW`.
            unsafe {
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: GetModuleHandleW(std::ptr::null()),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    lpszClassName: name.as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassExW(&class);
            }
        });
    }

    /// A plain top-level window whose client area hosts the plugin's `HWND` view. Lives on
    /// the main thread, whose event loop (Tauri's) pumps its messages.
    pub(crate) struct HostWindow {
        hwnd: HWND,
        style: u32,
        size: Size,
    }

    impl HostWindow {
        pub const SUPPORTED: bool = true;
        /// The platform type passed to `IPlugView::attached`.
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"HWND";

        /// Whether the current thread is the process main thread (where editors live, and
        /// whose message loop serves their windows).
        pub fn on_main_thread() -> bool {
            std::thread::current().name() == Some("main")
        }

        pub fn open(title: &str, size: Size, resizable: bool) -> Result<Self, String> {
            if !Self::on_main_thread() {
                return Err("plugin editors must be opened on the main thread".into());
            }
            register_class();
            let mut style =
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
            if resizable {
                style |= WS_THICKFRAME | WS_MAXIMIZEBOX;
            }
            let (w, h) = outer_size(style, size);
            // SAFETY: plain window creation with our registered class.
            let hwnd = unsafe {
                let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
                let (x, y) = if sw > 0 && sh > 0 {
                    ((sw - w).max(0) / 2, (sh - h).max(0) / 2)
                } else {
                    (CW_USEDEFAULT, CW_USEDEFAULT)
                };
                CreateWindowExW(
                    0,
                    wide(CLASS_NAME).as_ptr(),
                    wide(title).as_ptr(),
                    style,
                    x,
                    y,
                    w,
                    h,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err(format!(
                    "CreateWindowExW failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { hwnd, style, size })
        }

        /// The window `HWND` the plugin view gets attached to (as a child).
        pub fn view_ptr(&self) -> Option<*mut c_void> {
            Some(self.hwnd)
        }

        pub fn show(&self) {
            // SAFETY: our live window.
            unsafe {
                ShowWindow(self.hwnd, SW_SHOW);
                SetForegroundWindow(self.hwnd);
            }
        }

        /// False once the user closed (hid) the window.
        pub fn is_visible(&self) -> bool {
            // SAFETY: our live window.
            unsafe { IsWindowVisible(self.hwnd) != 0 }
        }

        pub fn content_size(&self) -> Size {
            // SAFETY: our live window. A minimized window reports an empty client area:
            // keep the last size so the plugin isn't resized to nothing.
            unsafe {
                let mut r: RECT = std::mem::zeroed();
                if IsIconic(self.hwnd) != 0 || GetClientRect(self.hwnd, &mut r) == 0 {
                    return self.size;
                }
                Size {
                    width: (r.right - r.left).max(0) as u32,
                    height: (r.bottom - r.top).max(0) as u32,
                }
            }
        }

        /// Last size applied or observed.
        pub fn known_size(&self) -> Size {
            self.size
        }

        pub fn set_known_size(&mut self, size: Size) {
            self.size = size;
        }

        pub fn resize(&mut self, size: Size) {
            let (w, h) = outer_size(self.style, size);
            // SAFETY: our live window.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    w,
                    h,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            self.size = size;
        }

        pub fn close(self) {
            // SAFETY: our live window; the plugin view was removed from it already.
            unsafe { DestroyWindow(self.hwnd) };
        }
    }

    /// Window size for a client area of `size`.
    fn outer_size(style: u32, size: Size) -> (i32, i32) {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: size.width as i32,
            bottom: size.height as i32,
        };
        // SAFETY: plain computation on a local rect.
        unsafe { AdjustWindowRectEx(&mut r, style, 0, 0) };
        (r.right - r.left, r.bottom - r.top)
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) use other::HostWindow;

#[cfg(not(any(target_os = "macos", windows)))]
mod other {
    use std::ffi::c_void;

    use super::Size;

    /// Host-created editor windows are macOS-only for now (uninhabited elsewhere).
    pub(crate) enum HostWindow {}

    impl HostWindow {
        pub const SUPPORTED: bool = false;
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"";

        pub fn on_main_thread() -> bool {
            false
        }
        pub fn open(_title: &str, _size: Size, _resizable: bool) -> Result<Self, String> {
            Err("host editor windows are not supported on this OS yet".into())
        }
        pub fn view_ptr(&self) -> Option<*mut c_void> {
            match *self {}
        }
        pub fn show(&self) {
            match *self {}
        }
        pub fn is_visible(&self) -> bool {
            match *self {}
        }
        pub fn content_size(&self) -> Size {
            match *self {}
        }
        pub fn known_size(&self) -> Size {
            match *self {}
        }
        pub fn set_known_size(&mut self, _size: Size) {
            match *self {}
        }
        pub fn resize(&mut self, _size: Size) {
            match *self {}
        }
        pub fn close(self) {
            match self {}
        }
    }
}

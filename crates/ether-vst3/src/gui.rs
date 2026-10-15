//! Plugin editors: the controller's `IPlugView` attached to a host-created window (the
//! approach of `ether-clap`'s `gui.rs`). macOS: an `NSWindow` whose content
//! `NSView` is the parent passed to `IPlugView::attached`; Windows: a top-level window whose
//! `HWND` is that parent; Linux: an X11 top-level window (`ether_plugin_host::x11_window`)
//! whose id is the `X11EmbedWindowID` parent, with the `Linux::IRunLoop` plugins need served
//! by [`PlugFrame`] (see [`RunLoop`]). Elsewhere `HostWindow` is uninhabited and plugins
//! report no editor.
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

/// `IPlugFrame`: records the plugin's resize requests. On Linux also the plugin's
/// `Linux::IRunLoop` (queried from the frame by the view; see [`RunLoop`]).
#[derive(Default)]
pub(crate) struct PlugFrame {
    pub resize: Mutex<Option<Size>>,
    #[cfg(target_os = "linux")]
    pub run_loop: RunLoop,
}

#[cfg(not(target_os = "linux"))]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

#[cfg(target_os = "linux")]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame, vst3::Steinberg::Linux::IRunLoop);
}

impl PlugFrame {
    pub fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self::default())
    }

    pub fn take_resize(&self) -> Option<Size> {
        self.resize.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Run the plugin's due run-loop work (Linux: ready file descriptors, due timers; no-op
    /// elsewhere). Called from the editor poll on the plugin main thread.
    pub fn run_due(&self) {
        #[cfg(target_os = "linux")]
        self.run_loop.run_due();
    }
}

#[cfg(target_os = "linux")]
pub(crate) use run_loop::RunLoop;

/// `Linux::IRunLoop`: Linux VST3 editors have no event loop of their own; they register
/// their X connection's file descriptor (`registerEventHandler`) and timers
/// (`registerTimer`) with the host, which calls them back on the UI thread. Served from the
/// editor poll (~30-60 Hz): `poll(2)` with a zero timeout on the registered descriptors
/// (`onFDIsSet` for each ready one), then `onTimer` for every timer whose interval passed
/// (at most once per poll, so a timer faster than the poll runs at the poll's rate).
/// Handlers are held (`addRef`) until unregistered or the frame goes. Callbacks run with no
/// lock held: a plugin may (un)register from inside one.
#[cfg(target_os = "linux")]
mod run_loop {
    use std::ffi::{c_int, c_short, c_ulong};
    use std::sync::{Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use vst3::Steinberg::Linux::{
        FileDescriptor, IEventHandler, IEventHandlerTrait, IRunLoopTrait, ITimerHandler,
        ITimerHandlerTrait, TimerInterval,
    };
    use vst3::Steinberg::{kInvalidArgument, kResultOk, tresult};
    use vst3::{ComPtr, ComRef, Interface};

    use super::PlugFrame;

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: c_short,
        revents: c_short,
    }

    const POLLIN: c_short = 0x001;
    const POLLPRI: c_short = 0x002;
    const POLLERR: c_short = 0x008;
    const POLLHUP: c_short = 0x010;

    unsafe extern "C" {
        fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
    }

    struct Timer {
        handler: ComPtr<ITimerHandler>,
        interval: Duration,
        next: Instant,
    }

    #[derive(Default)]
    pub(crate) struct RunLoop {
        handlers: Mutex<Vec<(ComPtr<IEventHandler>, FileDescriptor)>>,
        timers: Mutex<Vec<Timer>>,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Our own reference to a handler the plugin passes (`None` for null).
    ///
    /// # Safety
    /// `ptr` is null or a live COM object.
    unsafe fn adopt<I: Interface>(ptr: *mut I) -> Option<ComPtr<I>> {
        // SAFETY: per the contract; `to_com_ptr` adds a reference.
        unsafe { ComRef::from_raw(ptr) }.map(|r| r.to_com_ptr())
    }

    impl RunLoop {
        /// Registered descriptors and timers (tests/diagnostics).
        pub fn counts(&self) -> (usize, usize) {
            (lock(&self.handlers).len(), lock(&self.timers).len())
        }

        pub fn run_due(&self) {
            let handlers: Vec<(ComPtr<IEventHandler>, FileDescriptor)> =
                lock(&self.handlers).clone();
            if !handlers.is_empty() {
                let mut fds: Vec<PollFd> = handlers
                    .iter()
                    .map(|(_, fd)| PollFd {
                        fd: *fd,
                        events: POLLIN | POLLPRI,
                        revents: 0,
                    })
                    .collect();
                // SAFETY: `fds` holds `len` initialized entries; zero timeout (never blocks).
                let ready = unsafe { poll(fds.as_mut_ptr(), fds.len() as c_ulong, 0) };
                if ready > 0 {
                    for ((handler, fd), p) in handlers.iter().zip(&fds) {
                        if p.revents & (POLLIN | POLLPRI | POLLERR | POLLHUP) == 0 {
                            continue;
                        }
                        // Skip a handler an earlier callback unregistered.
                        let live = lock(&self.handlers)
                            .iter()
                            .any(|(h, _)| h.as_ptr() == handler.as_ptr());
                        if live {
                            // SAFETY: a registered handler (we hold a reference).
                            unsafe { handler.onFDIsSet(*fd) };
                        }
                    }
                }
            }
            let now = Instant::now();
            let due: Vec<ComPtr<ITimerHandler>> = lock(&self.timers)
                .iter_mut()
                .filter(|t| t.next <= now)
                .map(|t| {
                    t.next = now + t.interval;
                    t.handler.clone()
                })
                .collect();
            for handler in due {
                let live = lock(&self.timers)
                    .iter()
                    .any(|t| t.handler.as_ptr() == handler.as_ptr());
                if live {
                    // SAFETY: a registered handler (we hold a reference).
                    unsafe { handler.onTimer() };
                }
            }
        }
    }

    impl IRunLoopTrait for PlugFrame {
        unsafe fn registerEventHandler(
            &self,
            handler: *mut IEventHandler,
            fd: FileDescriptor,
        ) -> tresult {
            // SAFETY: the plugin passes a live handler or null.
            let Some(handler) = (unsafe { adopt(handler) }) else {
                return kInvalidArgument;
            };
            if fd < 0 {
                return kInvalidArgument;
            }
            lock(&self.run_loop.handlers).push((handler, fd));
            kResultOk
        }

        unsafe fn unregisterEventHandler(&self, handler: *mut IEventHandler) -> tresult {
            if handler.is_null() {
                return kInvalidArgument;
            }
            // Released outside the lock (a release may run plugin code).
            let gone: Vec<_> = {
                let mut list = lock(&self.run_loop.handlers);
                let (gone, kept) = std::mem::take(&mut *list)
                    .into_iter()
                    .partition(|(h, _)| h.as_ptr() == handler);
                *list = kept;
                gone
            };
            drop(gone);
            kResultOk
        }

        unsafe fn registerTimer(
            &self,
            handler: *mut ITimerHandler,
            milliseconds: TimerInterval,
        ) -> tresult {
            // SAFETY: the plugin passes a live handler or null.
            let Some(handler) = (unsafe { adopt(handler) }) else {
                return kInvalidArgument;
            };
            let interval = Duration::from_millis(milliseconds.max(1));
            lock(&self.run_loop.timers).push(Timer {
                handler,
                interval,
                next: Instant::now() + interval,
            });
            kResultOk
        }

        unsafe fn unregisterTimer(&self, handler: *mut ITimerHandler) -> tresult {
            if handler.is_null() {
                return kInvalidArgument;
            }
            let gone: Vec<_> = {
                let mut list = lock(&self.run_loop.timers);
                let (gone, kept) = std::mem::take(&mut *list)
                    .into_iter()
                    .partition(|t| t.handler.as_ptr() == handler);
                *list = kept;
                gone
            };
            drop(gone);
            kResultOk
        }
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
    use std::cell::RefCell;
    use std::ffi::c_void;

    use ether_core::plugin::PluginNotification;

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    use ether_plugin_host::editor_keys::KeyTap;
    use ether_plugin_host::editor_owner::EditorLink;

    use super::Size;

    /// A plain titled `NSWindow` whose content view hosts the plugin's view; a child window
    /// of the app's main window once shown (`editor_owner`), so it stays above it.
    pub(crate) struct HostWindow {
        window: Retained<NSWindow>,
        keys: KeyTap,
        size: Size,
        owner: RefCell<Option<EditorLink>>,
    }

    impl HostWindow {
        /// Whether editor windows can be made in this session.
        pub fn available() -> bool {
            true
        }
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
            let keys = KeyTap::register(Retained::as_ptr(&window) as *mut c_void);
            Ok(Self {
                window,
                size,
                keys,
                owner: RefCell::new(None),
            })
        }

        /// The content `NSView*` the plugin view gets attached to.
        pub fn view_ptr(&self) -> Option<*mut c_void> {
            let view = self.window.contentView()?;
            Some(Retained::as_ptr(&view) as *mut c_void)
        }

        pub fn show(&self) {
            self.window.makeKeyAndOrderFront(None);
            // Attached once on screen (a child window added while ordered out would be
            // ordered in early); again on later shows, idempotently.
            let mut owner = self.owner.borrow_mut();
            match owner.as_mut() {
                Some(link) => link.macos_reattach(),
                None => {
                    *owner = Some(EditorLink::macos_attach(
                        Retained::as_ptr(&self.window) as *mut c_void
                    ));
                }
            }
        }

        /// False once the user closed the window (not while it is hidden with the main
        /// window, e.g. minimized with it).
        pub fn is_visible(&self) -> bool {
            self.window.isVisible()
                || self
                    .owner
                    .borrow()
                    .as_ref()
                    .is_some_and(EditorLink::hidden_with_owner)
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

        /// Keys captured in this window for the app (`ether_plugin_host::editor_keys`).
        pub fn drain_keys(&self, out: &mut Vec<PluginNotification>) {
            self.keys.drain(out);
        }

        pub fn close(self) {
            // Detach from the main window before closing.
            drop(self.owner.take());
            self.window.close();
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::HostWindow;

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    use ether_core::plugin::PluginNotification;
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

    use ether_plugin_host::editor_keys::KeyTap;
    use ether_plugin_host::editor_owner::{self, EditorLink};

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
    /// the main thread, whose event loop (Tauri's) pumps its messages. Owned by the app's
    /// main window when created on its thread (`editor_owner`), so it stays above it.
    pub(crate) struct HostWindow {
        hwnd: HWND,
        style: u32,
        size: Size,
        keys: KeyTap,
        owner: EditorLink,
    }

    impl HostWindow {
        /// Whether editor windows can be made in this session.
        pub fn available() -> bool {
            true
        }
        /// The platform type passed to `IPlugView::attached`.
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"HWND";

        /// Whether the current thread is the plugin UI thread (where editors live, and whose
        /// message loop serves their windows): the process main thread, or the desktop app's
        /// dedicated one (`ether_plugin_host::ui_thread`).
        pub fn on_main_thread() -> bool {
            ether_plugin_host::ui_thread::is_ui_thread()
        }

        pub fn open(title: &str, size: Size, resizable: bool) -> Result<Self, String> {
            if !Self::on_main_thread() {
                return Err("plugin editors must be opened on the plugin UI thread".into());
            }
            register_class();
            let mut style =
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
            if resizable {
                style |= WS_THICKFRAME | WS_MAXIMIZEBOX;
            }
            let (w, h) = outer_size(style, size);
            let plan = editor_owner::win32_plan();
            // SAFETY: plain window creation with our registered class (owned by a live window
            // of this thread, or unowned).
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
                    plan.owner_hwnd(),
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
            let keys = KeyTap::register(hwnd);
            Ok(Self {
                hwnd,
                style,
                size,
                keys,
                owner: plan.link(hwnd),
            })
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

        /// False once the user closed (hid) the window (not while it is hidden with the
        /// main window that owns it, e.g. minimized with it).
        pub fn is_visible(&self) -> bool {
            // SAFETY: our live window.
            unsafe { IsWindowVisible(self.hwnd) != 0 || self.owner.hidden_with_owner() }
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

        /// Keys captured in this window for the app (`ether_plugin_host::editor_keys`).
        pub fn drain_keys(&self, out: &mut Vec<PluginNotification>) {
            self.keys.drain(out);
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

#[cfg(target_os = "linux")]
pub(crate) use linux::HostWindow;

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::c_void;

    use ether_core::plugin::PluginNotification;
    use ether_plugin_host::x11_window::{self, X11Window};

    use super::Size;

    /// A top-level X11 window whose id is the plugin view's parent
    /// (`ether_plugin_host::x11_window`). Closing it only unmaps it: the poll removes the
    /// plugin's view before the window goes. Keys typed into the editor stay with the plugin
    /// (no key forwarding on Linux).
    pub(crate) struct HostWindow {
        window: X11Window,
        size: Size,
    }

    impl HostWindow {
        /// `kPlatformTypeX11EmbedWindowID`.
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"X11EmbedWindowID";

        /// Whether editor windows can be made: libX11 and an X display (`DISPLAY`).
        pub fn available() -> bool {
            x11_window::available()
        }

        /// X11 has no thread affinity: editors live on whichever thread runs the plugin's
        /// main-thread calls (the caller's guarantee, as for every other plugin call).
        pub fn on_main_thread() -> bool {
            true
        }

        pub fn open(title: &str, size: Size, resizable: bool) -> Result<Self, String> {
            let window = X11Window::open(title, size.width, size.height, resizable)?;
            Ok(Self { window, size })
        }

        /// The X11 window id, as the pointer-sized parent the plugin APIs take.
        pub fn view_ptr(&self) -> Option<*mut c_void> {
            Some(self.window.id() as usize as *mut c_void)
        }

        pub fn show(&self) {
            self.window.show();
        }

        /// False once the user closed the window.
        pub fn is_visible(&self) -> bool {
            self.window.is_visible()
        }

        pub fn content_size(&self) -> Size {
            let (width, height) = self.window.size();
            Size { width, height }
        }

        /// Last size applied or observed.
        pub fn known_size(&self) -> Size {
            self.size
        }

        pub fn set_known_size(&mut self, size: Size) {
            self.size = size;
        }

        pub fn resize(&mut self, size: Size) {
            self.window.resize(size.width, size.height);
            self.size = size;
        }

        pub fn drain_keys(&self, _out: &mut Vec<PluginNotification>) {}

        pub fn close(self) {
            drop(self.window);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub(crate) use other::HostWindow;

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod other {
    use std::ffi::c_void;

    use ether_core::plugin::PluginNotification;

    use super::Size;

    /// Host-created editor windows are macOS-only for now (uninhabited elsewhere).
    pub(crate) enum HostWindow {}

    impl HostWindow {
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"";

        pub fn available() -> bool {
            false
        }

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
        pub fn drain_keys(&self, _out: &mut Vec<PluginNotification>) {
            match *self {}
        }

        pub fn close(self) {
            match self {}
        }
    }
}

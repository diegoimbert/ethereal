//! Plugin editor windows (no embedding into the app UI in v0.1).
//!
//! Two ways to show a plugin GUI in its own window:
//! - **Host window** (macOS, preferred there): most macOS CLAP plugins only support embedded
//!   Cocoa views, so we create an `NSWindow` and parent the plugin view into it.
//! - **CLAP floating**: the plugin opens its own window (all OSes, when supported).
//!
//! Host windows on other OSes are compile-gated out (`HostWindow` is uninhabited there).
//!
//! Both stay above the app's main window (`ether_plugin_host::editor_owner`): a host window
//! is its child window, a floating window gets it as transient parent ([`transient_window`]).

use std::ffi::{c_ulong, c_void};

use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, PluginGui, Window as ClapWindow,
};
use clack_host::prelude::PluginMainThreadHandle;
use ether_plugin_host::editor_owner::OwnerWindow;

/// Pick the GUI configuration to use for `gui`, or `None` if we can't host it.
pub(crate) fn negotiate(
    gui: &PluginGui,
    plugin: &PluginMainThreadHandle,
) -> Option<GuiConfiguration<'static>> {
    let api_type: GuiApiType<'static> = GuiApiType::default_for_current_platform()?;
    let embedded = GuiConfiguration {
        api_type,
        is_floating: false,
    };
    let floating = GuiConfiguration {
        api_type,
        is_floating: true,
    };
    if HostWindow::SUPPORTED && gui.is_api_supported(plugin, embedded) {
        Some(embedded)
    } else if gui.is_api_supported(plugin, floating) {
        Some(floating)
    } else {
        None
    }
}

/// The app's main window as a floating editor's transient parent (`set_transient`: the
/// plugin keeps its window above it, e.g. `WM_TRANSIENT_FOR` on X11, an owned window on
/// Windows), when `owner` is usable with `api_type`.
pub(crate) fn transient_window(
    owner: OwnerWindow,
    api_type: GuiApiType<'_>,
) -> Option<ClapWindow<'static, 'static>> {
    match owner {
        // SAFETY (both): the app's main window, alive for the life of the process (longer
        // than any plugin GUI).
        OwnerWindow::Win32(hwnd) if api_type == GuiApiType::WIN32 => {
            Some(unsafe { ClapWindow::from_win32_hwnd(hwnd as *mut c_void) })
        }
        OwnerWindow::AppKit(view) if api_type == GuiApiType::COCOA => {
            Some(unsafe { ClapWindow::from_cocoa_nsview(view as *mut c_void) })
        }
        OwnerWindow::X11(window) if api_type == GuiApiType::X11 => {
            let window = c_ulong::try_from(window).ok()?;
            Some(ClapWindow::from_x11_handle(window))
        }
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::HostWindow;

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use ether_core::plugin::PluginNotification;

    use clack_extensions::gui::GuiSize;
    use ether_plugin_host::editor_keys::KeyTap;
    use ether_plugin_host::editor_owner::EditorLink;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    /// A plain titled `NSWindow` whose content view hosts the plugin's view; a child window
    /// of the app's main window once shown (`editor_owner`), so it stays above it.
    pub(crate) struct HostWindow {
        window: Retained<NSWindow>,
        keys: KeyTap,
        size: GuiSize,
        owner: RefCell<Option<EditorLink>>,
    }

    impl HostWindow {
        pub const SUPPORTED: bool = true;

        /// Must be called on the process main thread (AppKit requirement).
        pub fn open(title: &str, size: GuiSize, resizable: bool) -> Result<Self, String> {
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

        /// The content `NSView*` the plugin view gets parented into.
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

        pub fn content_size(&self) -> GuiSize {
            let frame = self.window.contentRectForFrameRect(self.window.frame());
            GuiSize {
                width: frame.size.width.max(0.0) as u32,
                height: frame.size.height.max(0.0) as u32,
            }
        }

        /// Last size applied or observed.
        pub fn known_size(&self) -> GuiSize {
            self.size
        }

        pub fn set_known_size(&mut self, size: GuiSize) {
            self.size = size;
        }

        pub fn resize(&mut self, size: GuiSize) {
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

#[cfg(not(target_os = "macos"))]
pub(crate) use other::HostWindow;

#[cfg(not(target_os = "macos"))]
mod other {
    use std::ffi::c_void;

    use ether_core::plugin::PluginNotification;

    use clack_extensions::gui::GuiSize;

    /// Host-created editor windows are macOS-only for now (uninhabited elsewhere).
    pub(crate) enum HostWindow {}

    impl HostWindow {
        pub const SUPPORTED: bool = false;

        pub fn open(_title: &str, _size: GuiSize, _resizable: bool) -> Result<Self, String> {
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
        pub fn content_size(&self) -> GuiSize {
            match *self {}
        }
        pub fn known_size(&self) -> GuiSize {
            match *self {}
        }
        pub fn set_known_size(&mut self, _size: GuiSize) {
            match *self {}
        }
        pub fn resize(&mut self, _size: GuiSize) {
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

/// Default editor size when the plugin doesn't report one.
pub(crate) const DEFAULT_SIZE: GuiSize = GuiSize {
    width: 640,
    height: 480,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_window_matches_the_api_type() {
        let x11 = transient_window(OwnerWindow::X11(42), GuiApiType::X11).unwrap();
        assert_eq!(x11.as_x11_handle(), Some(42));
        let win = transient_window(OwnerWindow::Win32(0x500), GuiApiType::WIN32).unwrap();
        assert_eq!(win.as_win32_hwnd(), Some(0x500 as *mut c_void));
        let mac = transient_window(OwnerWindow::AppKit(0x900), GuiApiType::COCOA).unwrap();
        assert_eq!(mac.as_cocoa_nsview(), Some(0x900 as *mut c_void));
        // A handle of another windowing system is never handed over.
        assert!(transient_window(OwnerWindow::X11(42), GuiApiType::WIN32).is_none());
        assert!(transient_window(OwnerWindow::Win32(1), GuiApiType::WAYLAND).is_none());
    }
}

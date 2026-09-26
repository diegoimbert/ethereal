//! Plugin editor windows (no embedding into the app UI in v0.1).
//!
//! Two ways to show a plugin GUI in its own window:
//! - **Host window** (macOS, preferred there): most macOS CLAP plugins only support embedded
//!   Cocoa views, so we create an `NSWindow` and parent the plugin view into it.
//! - **CLAP floating**: the plugin opens its own window (all OSes, when supported).
//!
//! Host windows on other OSes are compile-gated out (`HostWindow` is uninhabited there).

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGui};
use clack_host::prelude::PluginMainThreadHandle;

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

#[cfg(target_os = "macos")]
pub(crate) use macos::HostWindow;

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    use clack_extensions::gui::GuiSize;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    /// A plain titled `NSWindow` whose content view hosts the plugin's view.
    pub(crate) struct HostWindow {
        window: Retained<NSWindow>,
        size: GuiSize,
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
            Ok(Self { window, size })
        }

        /// The content `NSView*` the plugin view gets parented into.
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

        pub fn close(self) {
            self.window.close();
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) use other::HostWindow;

#[cfg(not(target_os = "macos"))]
mod other {
    use std::ffi::c_void;

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

//! Plugin editors: the controller's `IPlugView` attached to a host-created window (the
//! approach of `ether-clap`'s `gui.rs`). macOS only for now: an `NSWindow` whose content
//! `NSView` is the parent passed to `IPlugView::attached`. Elsewhere `HostWindow` is
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

#[cfg(not(target_os = "macos"))]
pub(crate) use other::HostWindow;

#[cfg(not(target_os = "macos"))]
mod other {
    use std::ffi::c_void;

    use super::Size;

    /// Host-created editor windows are macOS-only for now (uninhabited elsewhere).
    pub(crate) enum HostWindow {}

    impl HostWindow {
        pub const SUPPORTED: bool = false;
        pub const PLATFORM_TYPE: &std::ffi::CStr = c"";

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

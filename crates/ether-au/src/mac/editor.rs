//! AU editors: `-[AUAudioUnit requestViewControllerWithCompletionHandler:]` (CoreAudioKit)
//! → an `NSViewController` shown in a floating `NSWindow`, on the main thread.
//!
//! This covers v3 units and v2 units alike: for v2 components Apple's bridge wraps the
//! unit's Cocoa view (`kAudioUnitProperty_CocoaUI`) in the returned view controller. The
//! request may complete asynchronously on the main run loop, so the run loop is pumped while
//! waiting (see [`super::pump_until`]).

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{NSBackingStoreType, NSViewController, NSWindow, NSWindowStyleMask};
use objc2_audio_toolbox::AUAudioUnit;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{ASYNC_TIMEOUT, SendBox, pump_until};

// `requestViewControllerWithCompletionHandler:` is a category implemented in CoreAudioKit.
#[link(name = "CoreAudioKit", kind = "framework")]
unsafe extern "C" {}

const DEFAULT_SIZE: (f64, f64) = (640.0, 480.0);

/// Whether the unit can provide a view controller at all.
pub(crate) fn supported(au: &AUAudioUnit) -> bool {
    au.respondsToSelector(sel!(requestViewControllerWithCompletionHandler:))
}

/// Whether the unit has a custom editor: `providesUserInterface` (v3), or for a v2 unit
/// behind the bridge a `kAudioUnitProperty_CocoaUI` view (read from the bridge's
/// `audioUnit`, when the OS exposes it). Main thread, no view is created.
pub(crate) fn has_custom_view(au: &AUAudioUnit) -> bool {
    use objc2_audio_toolbox::{
        AudioUnit, AudioUnitGetPropertyInfo, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global,
    };
    if !supported(au) {
        return false;
    }
    // SAFETY: plain property getter.
    if unsafe { au.providesUserInterface() } {
        return true;
    }
    if !au.respondsToSelector(sel!(audioUnit)) {
        return false;
    }
    // SAFETY: `-[AUAudioUnitV2Bridge audioUnit]` returns the wrapped v2 instance (or null).
    let unit: AudioUnit = unsafe { msg_send![au, audioUnit] };
    if unit.is_null() {
        return false;
    }
    let mut size = 0u32;
    // SAFETY: valid instance; out pointers valid or null.
    let status = unsafe {
        AudioUnitGetPropertyInfo(
            unit,
            kAudioUnitProperty_CocoaUI,
            kAudioUnitScope_Global,
            0,
            &mut size,
            std::ptr::null_mut(),
        )
    };
    status == 0 && size > 0
}

/// Ask the unit for its view controller (main thread; pumps the run loop).
fn request_view_controller(au: &AUAudioUnit) -> Result<Retained<NSViewController>, String> {
    if !supported(au) {
        return Err("the unit cannot provide a view".into());
    }
    type Done = Option<SendBox<Retained<NSViewController>>>;
    let slot: Arc<Mutex<Option<Done>>> = Arc::new(Mutex::new(None));
    let tx = slot.clone();
    let handler = RcBlock::new(move |vc: *mut NSViewController| {
        // SAFETY: +0 reference from the callee; retain what we keep.
        let vc = unsafe { Retained::retain(vc) }.map(SendBox);
        if let Ok(mut s) = tx.lock() {
            *s = Some(vc);
        }
    });
    // SAFETY: selector checked above; the block is copied by the callee.
    let _: () = unsafe { msg_send![au, requestViewControllerWithCompletionHandler: &*handler] };
    match pump_until(&slot, ASYNC_TIMEOUT) {
        Some(Some(vc)) => Ok(vc.0),
        Some(None) => Err("the unit has no view".into()),
        None => Err("timed out waiting for the unit's view".into()),
    }
}

/// A floating window hosting the unit's view controller.
pub(crate) struct EditorWindow {
    window: Retained<NSWindow>,
    _controller: Retained<NSViewController>,
}

impl EditorWindow {
    pub fn open(au: &AUAudioUnit, title: &str) -> Result<Self, String> {
        let mtm = MainThreadMarker::new()
            .ok_or("plugin editors must be opened on the main thread (AppKit)")?;
        let vc = request_view_controller(au)?;
        let preferred = vc.preferredContentSize();
        let view = vc.view();
        let frame = view.frame();
        let (w, h) = if preferred.width > 1.0 && preferred.height > 1.0 {
            (preferred.width, preferred.height)
        } else if frame.size.width > 1.0 && frame.size.height > 1.0 {
            (frame.size.width, frame.size.height)
        } else {
            DEFAULT_SIZE
        };
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
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
        // SAFETY: main thread, before the window is shown.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.setContentViewController(Some(&vc));
        window.setContentSize(NSSize::new(w, h));
        window.center();
        window.makeKeyAndOrderFront(None);
        Ok(Self {
            window,
            _controller: vc,
        })
    }

    pub fn show(&self) {
        self.window.makeKeyAndOrderFront(None);
    }

    /// False once the user closed the window.
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    pub fn close(self) {
        self.window.setContentViewController(None);
        self.window.close();
    }
}

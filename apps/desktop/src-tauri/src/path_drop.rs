//! OS file drops carrying **paths** (`file-import`, CONTRACTS.md §12.13).
//!
//! The window keeps `dragDropEnabled: false`: Tauri's own drop handler swallows every HTML5
//! drag in the webview, which the app needs (browser → arrangement). Without it WebKit hands
//! the page `File`s without paths, so on macOS the shell forwards Finder drops itself:
//! [`install`] replaces the webview's `performDragOperation:` with one that, when the drag
//! carries file names (`NSFilenamesPboardType`), emits [`PATH_DROP_EVENT`] `{ paths }` to
//! the UI and ends the page's drag with a `dragleave` (so it never sees a drop). The page
//! still gets its `dragover`s, so drop zones show their hints and remember where the drop
//! happened (`ui/src/features/import/osDrop.ts`); the UI then sends `Import { Path }`.
//!
//! Guarded: anything unexpected (no such method, no file names, the emit failing) falls
//! through to the original implementation, i.e. an HTML5 drop with `File`s that the UI
//! uploads. Windows and Linux always take that path.

/// Event name, mirrored by `ui/src/transport/tauri/TauriTransport.ts`.
pub const PATH_DROP_EVENT: &str = "ether://path-drop";

#[derive(Clone, Debug, serde::Serialize)]
pub struct PathDrop {
    pub paths: Vec<String>,
}

#[cfg(target_os = "macos")]
pub use mac::install;

/// Other platforms: drops reach the page as files (uploaded by the UI).
#[cfg(not(target_os = "macos"))]
pub fn install(_app: &tauri::AppHandle, _window: &tauri::WebviewWindow) {}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CStr, c_char};
    use std::sync::OnceLock;

    use objc2::ffi;
    use objc2::runtime::{AnyObject, Bool, Imp, Sel};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSString;
    use tauri::{AppHandle, Emitter, WebviewWindow};

    use super::{PATH_DROP_EVENT, PathDrop};

    type PerformDrag = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool;

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static ORIGINAL: OnceLock<PerformDrag> = OnceLock::new();

    /// Replace the webview class's `performDragOperation:` (once per process).
    pub fn install(app: &AppHandle, window: &WebviewWindow) {
        if APP.set(app.clone()).is_err() {
            return;
        }
        let r = window.with_webview(|w| {
            let view = w.inner().cast::<AnyObject>();
            if view.is_null() {
                return;
            }
            // SAFETY: `view` is the live WKWebView subclass instance (main thread, inside
            // `with_webview`); the replacement has the exact signature of the original
            // (`-(BOOL)performDragOperation:(id<NSDraggingInfo>)`), taken from its method.
            unsafe {
                let cls = (*view).class();
                let s = sel!(performDragOperation:);
                let method = ffi::class_getInstanceMethod(cls, s);
                if method.is_null() {
                    return;
                }
                let Some(original) = ffi::method_getImplementation(method) else {
                    return;
                };
                let types = ffi::method_getTypeEncoding(method);
                let _ = ORIGINAL.set(std::mem::transmute::<Imp, PerformDrag>(original));
                let replacement: PerformDrag = perform_drag_operation;
                ffi::class_replaceMethod(
                    std::ptr::from_ref(cls).cast_mut(),
                    s,
                    std::mem::transmute::<PerformDrag, Imp>(replacement),
                    types,
                );
            }
            tracing::debug!("OS path drops forwarded to the UI");
        });
        if let Err(e) = r {
            tracing::warn!(%e, "OS path drops unavailable (files will be uploaded)");
        }
    }

    /// The dragged file paths (`NSFilenamesPboardType`), if any.
    ///
    /// SAFETY: `info` is the `NSDraggingInfo` AppKit passed to `performDragOperation:`.
    unsafe fn dragged_paths(info: *mut AnyObject) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let pb: *mut AnyObject = msg_send![info, draggingPasteboard];
            if pb.is_null() {
                return out;
            }
            let ty = NSString::from_str("NSFilenamesPboardType");
            let list: *mut AnyObject = msg_send![pb, propertyListForType: &*ty];
            if list.is_null() {
                return out;
            }
            let is_array: Bool = msg_send![list, isKindOfClass: objc2::class!(NSArray)];
            if !is_array.as_bool() {
                return out;
            }
            let n: usize = msg_send![list, count];
            for i in 0..n {
                let s: *mut AnyObject = msg_send![list, objectAtIndex: i];
                if s.is_null() {
                    continue;
                }
                let utf8: *const c_char = msg_send![s, UTF8String];
                if !utf8.is_null() {
                    out.push(CStr::from_ptr(utf8).to_string_lossy().into_owned());
                }
            }
        }
        out
    }

    unsafe extern "C-unwind" fn perform_drag_operation(
        this: *mut AnyObject,
        s: Sel,
        info: *mut AnyObject,
    ) -> Bool {
        let original = ORIGINAL.get().copied();
        // SAFETY: called by AppKit with a valid dragging info.
        let paths = unsafe { dragged_paths(info) };
        let sent = !paths.is_empty()
            && APP
                .get()
                .is_some_and(|app| app.emit(PATH_DROP_EVENT, PathDrop { paths }).is_ok());
        if sent {
            // End the page's drag without a drop: it gets a `dragleave`.
            // SAFETY: `this` is the webview; `draggingExited:` takes the same dragging info.
            let _: () = unsafe { msg_send![this, draggingExited: info] };
            return Bool::YES;
        }
        match original {
            // SAFETY: the method's original implementation, same receiver and arguments.
            Some(f) => unsafe { f(this, s, info) },
            None => Bool::NO,
        }
    }
}

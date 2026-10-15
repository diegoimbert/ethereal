//! Plugin editor windows stay above the app's main window (`plugin-editor-owner`).
//!
//! A plugin editor lives in a native top-level window (a format's `HostWindow`, an AU
//! editor window, or a CLAP plugin's own floating window). Without a relation to the app's
//! main window, clicking the DAW put the editor behind it. Every DAW keeps editors above its
//! main window, without making them system-wide always-on-top: they must not float over
//! other apps, and they follow the main window when it is minimized.
//!
//! The app registers its main window once with [`set_owner`] (desktop: the Tauri main
//! window's raw handle, [`OwnerWindow::from_window`]). Editors then relate to it:
//!
//! - **Windows**: the editor is an *owned* window of the main window (`hWndParent` passed to
//!   `CreateWindowExW`, [`win32_plan`]). An owned window always stays above its owner,
//!   minimizes and restores with it, and has no taskbar button. Ownership across threads
//!   attaches the two threads' input queues (`AttachThreadInput` semantics): a plugin thread
//!   that stops pumping messages would then freeze the main window's input. So ownership is
//!   only used when the editor is created on the main window's own thread (the default
//!   host). An editor made on another thread (a dedicated plugin UI thread) gets the
//!   fallback instead: [`owner_activated`] (called when the main window is activated) puts
//!   it back above the main window with an asynchronous `SetWindowPos`, which never waits
//!   on the plugin thread.
//! - **macOS**: the editor `NSWindow` is a child window of the main `NSWindow`
//!   (`addChildWindow:ordered:NSWindowAbove`, [`EditorLink::macos_attach`]): kept just
//!   above the main window, hidden with it when it is minimized, below other apps' windows.
//! - **Linux (X11)**: VST2/VST3 host editor windows (`crate::x11_window`) are transient for
//!   the main window (`WM_TRANSIENT_FOR`, [`transient`]); CLAP floating editors get the main
//!   window as their transient parent (`clap_plugin_gui::set_transient`), which the plugin
//!   turns into `WM_TRANSIENT_FOR` too. Wayland: no handle a plugin could use
//!   (CLAP's `set_transient` is X11/Win32/Cocoa only), so editors stay unrelated there.
//!
//! Sandboxed plugins run their editors in a helper process, where no owner is ever set:
//! ownership across processes would tie the main window's input to a process that may hang
//! or crash (on Windows) or is impossible (macOS child windows). Their editors keep the old
//! behavior.

use std::cell::Cell;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The app's main window, as the plugin host needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerWindow {
    /// A Win32 `HWND` (as an integer).
    Win32(isize),
    /// An AppKit `NSView*` (the main window's content view; the window is resolved from it).
    AppKit(usize),
    /// An X11 window id.
    X11(u64),
}

impl OwnerWindow {
    /// The owner for a raw window handle (`None`: a platform editors can't relate to, e.g.
    /// Wayland).
    // `c_ulong` (an Xlib window) is `u64` on Linux but `u32` on Windows.
    #[allow(clippy::useless_conversion)]
    pub fn from_raw(handle: raw_window_handle::RawWindowHandle) -> Option<Self> {
        use raw_window_handle::RawWindowHandle as R;
        match handle {
            R::Win32(h) => Some(Self::Win32(h.hwnd.get())),
            R::AppKit(h) => Some(Self::AppKit(h.ns_view.as_ptr() as usize)),
            R::Xlib(h) if h.window != 0 => Some(Self::X11(u64::from(h.window))),
            R::Xcb(h) => Some(Self::X11(u64::from(h.window.get()))),
            _ => None,
        }
    }

    /// The owner for a window (e.g. Tauri's main `WebviewWindow`).
    pub fn from_window(window: &impl raw_window_handle::HasWindowHandle) -> Option<Self> {
        Self::from_raw(window.window_handle().ok()?.as_raw())
    }
}

static OWNER: Mutex<Option<OwnerWindow>> = Mutex::new(None);

/// Register the app's main window: editors opened from now on stay above it. macOS: call
/// on the main thread (the view is retained for the life of the process).
pub fn set_owner(owner: Option<OwnerWindow>) {
    #[cfg(target_os = "macos")]
    if let Some(OwnerWindow::AppKit(view)) = owner {
        macos::retain_view(view);
    }
    *OWNER.lock().unwrap_or_else(|e| e.into_inner()) = owner;
}

/// The registered main window.
pub fn owner() -> Option<OwnerWindow> {
    *OWNER.lock().unwrap_or_else(|e| e.into_inner())
}

/// How an editor window relates to the main window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// No owner registered, or one that can't be used here: the editor is unrelated.
    None,
    /// Owned by (Windows) / a child window of (macOS) the main window.
    Owned,
    /// Raised above the main window whenever it is activated ([`owner_activated`]).
    Raised,
}

/// Windows: the relation for an editor created on thread `editor_thread` of process
/// `editor_process`, the main window belonging to `owner_thread` of `owner_process` (0: the
/// handle is stale). Ownership only within one thread (see the module docs).
pub fn win32_relation(
    owner_thread: u32,
    owner_process: u32,
    editor_thread: u32,
    editor_process: u32,
) -> Relation {
    if owner_thread == 0 || owner_process != editor_process {
        Relation::None
    } else if owner_thread == editor_thread {
        Relation::Owned
    } else {
        Relation::Raised
    }
}

/// The main window as a CLAP floating editor's transient parent (`set_transient`), when it
/// is safe to hand it to the plugin: on Windows only from the main window's own thread (the
/// plugin makes its window owned, see the module docs).
pub fn transient() -> Option<OwnerWindow> {
    let owner = owner()?;
    match owner {
        #[cfg(windows)]
        OwnerWindow::Win32(_) => (win32::plan().relation == Relation::Owned).then_some(owner),
        _ => Some(owner),
    }
}

/// Editors that are not owned but [`Relation::Raised`] (native handles as integers).
static RAISED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

fn raised() -> std::sync::MutexGuard<'static, Vec<usize>> {
    RAISED.lock().unwrap_or_else(|e| e.into_inner())
}

/// The main window was activated (desktop: Tauri's `WindowEvent::Focused(true)`, on the main
/// thread): put the [`Relation::Raised`] editors back above it. Never blocks on the editors'
/// thread. No-op where every editor is owned.
pub fn owner_activated() {
    let editors = raised().clone();
    #[cfg(windows)]
    for editor in editors {
        win32::raise(editor);
    }
    #[cfg(not(windows))]
    let _ = editors;
}

/// An editor window's relation to the main window, undone on drop (keep it alive as long
/// as the window; drop it on the window's thread, before the window is destroyed).
#[derive(Debug)]
pub struct EditorLink {
    editor: usize,
    relation: Relation,
    /// When the main window was last seen hidden ([`EditorLink::hidden_with_owner`]).
    owner_hidden_at: Cell<Option<Instant>>,
}

/// How long after the main window comes back an owned editor may still be off screen (the
/// system re-shows owned/child windows as part of the restore, possibly animated).
const OWNER_RESTORE_GRACE: Duration = Duration::from_secs(2);

/// Whether an editor is off screen only because the main window is (`owner_hidden_now`), or
/// was until less than [`OWNER_RESTORE_GRACE`] ago.
fn hidden_with_owner(
    owner_hidden_now: bool,
    last_hidden: &Cell<Option<Instant>>,
    now: Instant,
) -> bool {
    if owner_hidden_now {
        last_hidden.set(Some(now));
        return true;
    }
    last_hidden
        .get()
        .is_some_and(|t| now.saturating_duration_since(t) < OWNER_RESTORE_GRACE)
}

impl EditorLink {
    // Linux has no host editor windows (CLAP floating editors use `transient`).
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    fn new(editor: usize, relation: Relation) -> Self {
        if relation == Relation::Raised {
            raised().push(editor);
        }
        Self {
            editor,
            relation,
            owner_hidden_at: Cell::new(None),
        }
    }

    /// Whether the editor window may be off screen only because the main window is
    /// (minimized, or the app hidden): an owned/child window is hidden with its owner, which
    /// is not the user closing it. Editors check this before treating "not visible" as
    /// closed. On the editor's thread.
    pub fn hidden_with_owner(&self) -> bool {
        self.relation == Relation::Owned
            && hidden_with_owner(owner_hidden(), &self.owner_hidden_at, Instant::now())
    }

    /// The relation in effect.
    pub fn relation(&self) -> Relation {
        self.relation
    }

    /// macOS: make `ns_window` (an `NSWindow*`, main thread) a child window of the main
    /// window. Call once it is on screen; again after showing it (idempotent).
    #[cfg(target_os = "macos")]
    pub fn macos_attach(ns_window: *mut std::ffi::c_void) -> Self {
        let relation = macos::attach(ns_window as usize);
        Self::new(ns_window as usize, relation)
    }

    /// macOS: re-attach after the window was ordered out and back in.
    #[cfg(target_os = "macos")]
    pub fn macos_reattach(&self) {
        if self.relation == Relation::Owned {
            macos::attach(self.editor);
        }
    }
}

impl Drop for EditorLink {
    fn drop(&mut self) {
        match self.relation {
            Relation::None => {}
            Relation::Raised => {
                let editor = self.editor;
                raised().retain(|w| *w != editor);
            }
            Relation::Owned => {
                #[cfg(target_os = "macos")]
                macos::detach(self.editor);
            }
        }
    }
}

/// Whether the main window is off screen (minimized, hidden; macOS: or the app hidden).
fn owner_hidden() -> bool {
    #[cfg(windows)]
    return win32::owner_hidden();
    #[cfg(target_os = "macos")]
    return macos::owner_hidden();
    #[cfg(not(any(windows, target_os = "macos")))]
    false
}

/// Windows: decide the owner of an editor window about to be created on this thread.
#[cfg(windows)]
pub fn win32_plan() -> win32::Plan {
    win32::plan()
}

#[cfg(windows)]
pub mod win32 {
    //! Owned editor windows on Windows (see the module docs).

    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, HWND_TOP, IsIconic, IsWindow, IsWindowVisible,
        SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SetWindowPos,
    };

    use super::{EditorLink, OwnerWindow, Relation, owner, win32_relation};

    /// The owner of an editor window about to be created on the current thread.
    #[derive(Clone, Copy, Debug)]
    pub struct Plan {
        owner: HWND,
        pub(super) relation: Relation,
    }

    pub(super) fn plan() -> Plan {
        let none = Plan {
            owner: std::ptr::null_mut(),
            relation: Relation::None,
        };
        let Some(OwnerWindow::Win32(hwnd)) = owner() else {
            return none;
        };
        let hwnd = hwnd as HWND;
        // SAFETY: plain queries on a window handle (stale handles are rejected).
        let relation = unsafe {
            if IsWindow(hwnd) == 0 {
                return none;
            }
            let mut owner_process = 0u32;
            let owner_thread = GetWindowThreadProcessId(hwnd, &mut owner_process);
            win32_relation(
                owner_thread,
                owner_process,
                GetCurrentThreadId(),
                GetCurrentProcessId(),
            )
        };
        Plan {
            owner: if relation == Relation::Owned {
                hwnd
            } else {
                std::ptr::null_mut()
            },
            relation,
        }
    }

    impl Plan {
        /// The `hWndParent` to pass to `CreateWindowExW` (null: unowned).
        pub fn owner_hwnd(&self) -> HWND {
            self.owner
        }

        /// The relation, once the editor window `editor` exists.
        pub fn link(self, editor: *mut c_void) -> EditorLink {
            EditorLink::new(editor as usize, self.relation)
        }
    }

    /// Whether the main window is minimized or hidden (its owned windows are hidden then).
    pub(super) fn owner_hidden() -> bool {
        let Some(OwnerWindow::Win32(hwnd)) = owner() else {
            return false;
        };
        let hwnd = hwnd as HWND;
        // SAFETY: plain queries on a window handle (a stale one is not hidden).
        unsafe { IsWindow(hwnd) != 0 && (IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0) }
    }

    /// Put `editor` above the other windows of the app (no activation; asynchronous when
    /// the editor belongs to another thread, so a busy plugin thread never blocks the
    /// caller).
    pub(super) fn raise(editor: usize) {
        let hwnd = editor as HWND;
        // SAFETY: plain window calls; a destroyed window just fails.
        unsafe {
            if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
                return;
            }
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
            );
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! Child editor windows on macOS (see the module docs).

    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode,
    };

    use super::{OwnerWindow, Relation, owner};

    /// Keep the main window's view alive for the life of the process (the owner handle is a
    /// plain pointer, used until exit).
    pub(super) fn retain_view(view: usize) {
        // SAFETY: `view` is a live `NSView*` handed over by the app (raw window handle).
        if let Some(view) = unsafe { Retained::retain(view as *mut NSView) } {
            std::mem::forget(view);
        }
    }

    fn main_window() -> Option<Retained<NSWindow>> {
        let Some(OwnerWindow::AppKit(view)) = owner() else {
            return None;
        };
        MainThreadMarker::new()?;
        // SAFETY: retained by `retain_view`, so still a valid `NSView`; main thread.
        let view = unsafe { &*(view as *const NSView) };
        view.window()
    }

    /// Whether the main window is minimized or off screen, or the app hidden (child windows
    /// are ordered out with it then).
    pub(super) fn owner_hidden() -> bool {
        let Some(main) = main_window() else {
            return false;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        main.isMiniaturized()
            || !main.isVisible()
            || NSApplication::sharedApplication(mtm).isHidden()
    }

    /// Make `editor` (an `NSWindow*`) a child of the main window.
    pub(super) fn attach(editor: usize) -> Relation {
        let Some(main) = main_window() else {
            return Relation::None;
        };
        // SAFETY: the caller's live `NSWindow`, on the main thread (checked above).
        let editor = unsafe { &*(editor as *const NSWindow) };
        if std::ptr::eq(editor, &*main) {
            return Relation::None;
        }
        // An ordinary window shown while the main window is full screen opens on another
        // Space, out of sight, yet `isVisible`: never reported closed, so the UI showed the
        // editor open with nothing on screen, and reopening it did the same. Editors may
        // join the full-screen Space and move to the active one when shown.
        let behavior = editor.collectionBehavior();
        let wanted = behavior
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::MoveToActiveSpace;
        // (Exclusive with the bits already set otherwise: left as the window has it.)
        let exclusive = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenPrimary;
        if behavior != wanted && !behavior.intersects(exclusive) {
            editor.setCollectionBehavior(wanted);
        }
        let already = editor
            .parentWindow()
            .is_some_and(|p| std::ptr::eq(&*p, &*main));
        if !already {
            // SAFETY: both windows are live and on this thread.
            unsafe { main.addChildWindow_ordered(editor, NSWindowOrderingMode::Above) };
        }
        // Shown before it could join the main window's Space: bring it there.
        if editor.isVisible() && main.isVisible() && !editor.isOnActiveSpace() {
            editor.makeKeyAndOrderFront(None);
        }
        Relation::Owned
    }

    /// Undo [`attach`] (before the editor window is closed).
    pub(super) fn detach(editor: usize) {
        let Some(main) = main_window() else {
            return;
        };
        // SAFETY: the caller's live `NSWindow`, on the main thread.
        let editor = unsafe { &*(editor as *const NSWindow) };
        if editor
            .parentWindow()
            .is_some_and(|p| std::ptr::eq(&*p, &*main))
        {
            main.removeChildWindow(editor);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroIsize, NonZeroU32};
    use std::ptr::NonNull;

    use raw_window_handle::{
        AppKitWindowHandle, RawWindowHandle, WaylandWindowHandle, Win32WindowHandle,
        XcbWindowHandle, XlibWindowHandle,
    };

    use super::*;

    #[test]
    fn raw_handles_map_to_owners() {
        let win = Win32WindowHandle::new(NonZeroIsize::new(0x1234).unwrap());
        assert_eq!(
            OwnerWindow::from_raw(RawWindowHandle::Win32(win)),
            Some(OwnerWindow::Win32(0x1234))
        );
        let mut slot = 0u8;
        let view = NonNull::from(&mut slot).cast();
        let mac = AppKitWindowHandle::new(view);
        assert_eq!(
            OwnerWindow::from_raw(RawWindowHandle::AppKit(mac)),
            Some(OwnerWindow::AppKit(view.as_ptr() as usize))
        );
        assert_eq!(
            OwnerWindow::from_raw(RawWindowHandle::Xlib(XlibWindowHandle::new(77))),
            Some(OwnerWindow::X11(77))
        );
        assert_eq!(
            OwnerWindow::from_raw(RawWindowHandle::Xlib(XlibWindowHandle::new(0))),
            None
        );
        let xcb = XcbWindowHandle::new(NonZeroU32::new(9).unwrap());
        assert_eq!(
            OwnerWindow::from_raw(RawWindowHandle::Xcb(xcb)),
            Some(OwnerWindow::X11(9))
        );
        let wl = WaylandWindowHandle::new(view);
        assert_eq!(OwnerWindow::from_raw(RawWindowHandle::Wayland(wl)), None);
    }

    #[test]
    fn win32_owns_only_within_one_thread() {
        // Same thread (the default host: editors on the Tauri main thread): owned.
        assert_eq!(win32_relation(10, 1, 10, 1), Relation::Owned);
        // A plugin UI thread: no input-queue attachment, raised on activation instead.
        assert_eq!(win32_relation(10, 1, 11, 1), Relation::Raised);
        // Another process (sandbox helper) or a stale handle: unrelated.
        assert_eq!(win32_relation(10, 1, 10, 2), Relation::None);
        assert_eq!(win32_relation(0, 0, 10, 1), Relation::None);
    }

    #[test]
    fn hidden_owner_is_not_a_close_until_the_grace_ends() {
        let last = Cell::new(None);
        let t0 = Instant::now();
        // Never hidden: a hidden editor was closed by the user.
        assert!(!hidden_with_owner(false, &last, t0));
        // Main window minimized: editors are off screen with it.
        assert!(hidden_with_owner(true, &last, t0));
        // Just restored: the system may not have re-shown them yet.
        assert!(hidden_with_owner(
            false,
            &last,
            t0 + Duration::from_millis(500)
        ));
        // Later: off screen means closed again.
        assert!(!hidden_with_owner(false, &last, t0 + OWNER_RESTORE_GRACE));
        // Unowned editors never count as hidden with the owner.
        assert!(!EditorLink::new(0xC0, Relation::None).hidden_with_owner());
    }

    #[test]
    fn raised_editors_are_tracked_until_dropped() {
        let a = EditorLink::new(0xA0, Relation::Raised);
        let b = EditorLink::new(0xB0, Relation::Owned);
        assert!(raised().contains(&0xA0));
        assert!(!raised().contains(&0xB0));
        assert_eq!(a.relation(), Relation::Raised);
        drop(a);
        drop(b);
        assert!(!raised().contains(&0xA0));
        // Nothing to raise is fine on every OS.
        owner_activated();
    }
}

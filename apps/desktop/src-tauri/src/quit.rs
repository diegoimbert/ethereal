//! Quit confirmation and ⌘W routing.
//!
//! # Quit
//! ⌘Q (the app menu's Quit, [`menu`] on macOS) and closing the main window (its close
//! button, Alt+F4) don't quit right away: the shell emits [`QUIT_REQUESTED_EVENT`] to the UI,
//! which asks "Quit Ethereal?" and answers with `app_quit` (quit) or `app_quit_cancel`.
//! `app_quit` exits through the event loop (`AppHandle::exit`), so `RunEvent::Exit` still
//! stops the engine (which saves a dirty project on the way out).
//!
//! The gate ([`QuitGate`]) only asks once the UI said it listens (`app_quit_guard`), so a
//! UI that never loaded doesn't keep the app from quitting; a request left unanswered for
//! [`UNANSWERED`] (a frozen UI) quits on the next ⌘Q. Programmatic exits (an exit code:
//! `AppHandle::exit`, the updater's `request_restart`) are never asked about. The Dock's
//! Quit and logging out go through AppKit's `terminate:` and are not intercepted.
//!
//! # ⌘W ([`menu`], macOS)
//! "close window" closes the key window when it is not the main window (a plugin editor:
//! the editor's poll sees it closed, as with its title-bar button, and the UI follows), and
//! otherwise emits [`CLOSE_REQUESTED_EVENT`] (the UI closes its topmost floating window, if
//! any). It never closes the main window, which would quit the app.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Emitted to the main window: the user asked to quit (show the confirmation).
pub const QUIT_REQUESTED_EVENT: &str = "ether://quit-requested";
/// Emitted to the main window: ⌘W while the main window is the key window.
pub const CLOSE_REQUESTED_EVENT: &str = "ether://close-requested";

/// A quit request left unanswered this long lets the next one quit without asking.
pub const UNANSWERED: Duration = Duration::from_secs(10);

/// What to do with a quit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitAction {
    /// Quit now.
    Exit,
    /// Ask the UI (emit [`QUIT_REQUESTED_EVENT`]) and keep running.
    Ask,
}

#[derive(Debug, Default)]
struct GateState {
    /// The UI listens for quit requests.
    ui_ready: bool,
    /// Quitting was decided (confirmed, or allowed without asking): let every exit through.
    quitting: bool,
    /// When the unanswered request was made.
    pending_since: Option<Instant>,
}

/// Whether quitting needs the UI's confirmation (pure; see the module docs).
#[derive(Debug, Default)]
pub struct QuitGate(Mutex<GateState>);

impl QuitGate {
    fn with<R>(&self, f: impl FnOnce(&mut GateState) -> R) -> R {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }

    /// The UI started (`true`) or stopped (`false`) listening for quit requests.
    pub fn set_ui_ready(&self, ready: bool) {
        self.with(|s| {
            s.ui_ready = ready;
            s.pending_since = None;
        });
    }

    /// The user asked to quit (⌘Q, the main window's close button) at `now`.
    pub fn request(&self, now: Instant) -> QuitAction {
        self.with(|s| {
            let unanswered = s
                .pending_since
                .is_some_and(|t| now.saturating_duration_since(t) >= UNANSWERED);
            if s.quitting || !s.ui_ready || unanswered {
                s.quitting = true;
                return QuitAction::Exit;
            }
            s.pending_since.get_or_insert(now);
            QuitAction::Ask
        })
    }

    /// The UI answered: quit (`true`) or keep running.
    pub fn answer(&self, quit: bool) {
        self.with(|s| {
            s.pending_since = None;
            if quit {
                s.quitting = true;
            }
        });
    }

    /// `RunEvent::ExitRequested` with `code`: whether to prevent it. Only an exit without a
    /// code (the last window closed) is held back, and only while not quitting.
    pub fn holds_exit(&self, code: Option<i32>, now: Instant) -> bool {
        code.is_none() && self.request(now) == QuitAction::Ask
    }
}

/// Handle a quit request: quit, or show the main window and ask the UI.
pub fn request<R: Runtime>(app: &AppHandle<R>) {
    match app.state::<QuitGate>().request(Instant::now()) {
        QuitAction::Exit => app.exit(0),
        QuitAction::Ask => ask(app),
    }
}

/// Bring the main window forward and emit [`QUIT_REQUESTED_EVENT`] to it.
pub fn ask<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Err(e) = app.emit_to("main", QUIT_REQUESTED_EVENT, ()) {
        tracing::warn!(%e, "could not ask the UI about quitting");
        app.state::<QuitGate>().answer(true);
        app.exit(0);
    }
}

/// The UI listens for quit requests (`true`, once mounted) or no longer does.
#[tauri::command]
pub fn app_quit_guard(gate: tauri::State<'_, QuitGate>, ready: bool) {
    gate.set_ui_ready(ready);
}

/// The user confirmed: quit (the engine stops on `RunEvent::Exit`).
#[tauri::command]
pub fn app_quit(app: AppHandle, gate: tauri::State<'_, QuitGate>) {
    gate.answer(true);
    app.exit(0);
}

/// The user cancelled the quit confirmation.
#[tauri::command]
pub fn app_quit_cancel(gate: tauri::State<'_, QuitGate>) {
    gate.answer(false);
}

/// What ⌘W closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTarget {
    /// This native window (a plugin editor, a system panel): not the main window.
    Native(usize),
    /// The UI decides ([`CLOSE_REQUESTED_EVENT`]): the main window is key, or none is.
    App,
}

/// ⌘W's target from the key window and the main window (native handles as integers).
pub fn close_target(key: Option<usize>, main: Option<usize>) -> CloseTarget {
    match key {
        Some(k) if k != 0 && Some(k) != main => CloseTarget::Native(k),
        _ => CloseTarget::App,
    }
}

#[cfg(target_os = "macos")]
pub mod menu {
    //! The macOS menu bar: Tauri's default one, except that Quit and Close Window are the
    //! app's own items (Tauri's predefined ones call `terminate:` and a nil-targeted
    //! `performClose:`, which quit without asking or close the main window).

    use std::ffi::c_void;

    use tauri::menu::{
        AboutMetadata, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    };
    use tauri::{AppHandle, Emitter, Manager, Runtime};

    use super::{CLOSE_REQUESTED_EVENT, CloseTarget, close_target};
    use crate::app_menu;

    pub const QUIT_ID: &str = "ether-quit";
    pub const CLOSE_ID: &str = "ether-close-window";

    /// The menu bar (for `Builder::menu`, and rebuilt when the UI's language changes:
    /// `app_menu::menu_set_labels`), with the current labels.
    pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
        let l = app_menu::labels();
        let pkg = app.package_info();
        let config = app.config();
        let about = AboutMetadata {
            name: Some(pkg.name.clone()),
            version: Some(crate::APP_VERSION.to_string()),
            copyright: config.bundle.copyright.clone(),
            authors: config.bundle.publisher.clone().map(|p| vec![p]),
            ..Default::default()
        };
        let quit = MenuItem::with_id(app, QUIT_ID, &l.quit, true, Some("CmdOrCtrl+Q"))?;
        let close = MenuItem::with_id(app, CLOSE_ID, &l.close_window, true, Some("CmdOrCtrl+W"))?;
        // #573: File, Edit's undo / redo / select all and Help run the UI's own actions
        // (`app_menu`); cut / copy / paste stay AppKit's (clipboard events in the page).
        let mut file = app_menu::macos::grouped(app, app_menu::FILE_ITEMS, &l)?;
        file.push(Box::new(PredefinedMenuItem::separator(app)?));
        file.push(Box::new(close));
        let file: Vec<&dyn IsMenuItem<R>> = file.iter().map(|b| b.as_ref()).collect();
        let [undo, redo] = app_menu::EDIT_HISTORY else {
            unreachable!()
        };
        let undo = app_menu::macos::item(app, *undo, &l)?;
        let redo = app_menu::macos::item(app, *redo, &l)?;
        let select_all = app_menu::macos::item(app, app_menu::EDIT_SELECT_ALL, &l)?;
        let help: Vec<&[app_menu::ItemSpec]> = vec![app_menu::HELP_ITEMS];
        let help = app_menu::macos::grouped(app, &help, &l)?;
        let help: Vec<&dyn IsMenuItem<R>> = help.iter().map(|b| b.as_ref()).collect();
        Menu::with_items(
            app,
            &[
                &Submenu::with_items(
                    app,
                    pkg.name.clone(),
                    true,
                    &[
                        &PredefinedMenuItem::about(app, Some(l.about.as_str()), Some(about))?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::services(app, Some(l.services.as_str()))?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::hide(app, Some(l.hide.as_str()))?,
                        &PredefinedMenuItem::hide_others(app, Some(l.hide_others.as_str()))?,
                        &PredefinedMenuItem::separator(app)?,
                        &quit,
                    ],
                )?,
                &Submenu::with_items(app, &l.file, true, &file)?,
                &Submenu::with_items(
                    app,
                    &l.edit,
                    true,
                    &[
                        &undo,
                        &redo,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::cut(app, Some(l.cut.as_str()))?,
                        &PredefinedMenuItem::copy(app, Some(l.copy.as_str()))?,
                        &PredefinedMenuItem::paste(app, Some(l.paste.as_str()))?,
                        &select_all,
                    ],
                )?,
                &Submenu::with_items(
                    app,
                    &l.view,
                    true,
                    &[&PredefinedMenuItem::fullscreen(
                        app,
                        Some(l.fullscreen.as_str()),
                    )?],
                )?,
                &Submenu::with_items(
                    app,
                    &l.window,
                    true,
                    &[
                        &PredefinedMenuItem::minimize(app, Some(l.minimize.as_str()))?,
                        &PredefinedMenuItem::maximize(app, Some(l.zoom.as_str()))?,
                    ],
                )?,
                &Submenu::with_items(app, &l.help, true, &help)?,
            ],
        )
    }

    /// `Builder::on_menu_event` (main thread).
    pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
        if event.id().as_ref() == QUIT_ID {
            super::request(app);
        } else if event.id().as_ref() == CLOSE_ID {
            close_key_window(app);
        } else if let Some(action) = app_menu::menu_action(event.id().as_ref())
            && let Err(e) = app.emit_to("main", app_menu::MENU_ACTION_EVENT, action)
        {
            tracing::debug!(%e, action, "menu action not delivered");
        }
    }

    /// ⌘W: close the key window unless it is the main window (see the module docs).
    fn close_key_window<R: Runtime>(app: &AppHandle<R>) {
        let main = app
            .get_webview_window("main")
            .and_then(|w| w.ns_window().ok())
            .map(|p| p as usize);
        match close_target(key_window(), main) {
            CloseTarget::Native(window) => perform_close(window as *mut c_void),
            CloseTarget::App => {
                if let Err(e) = app.emit_to("main", CLOSE_REQUESTED_EVENT, ()) {
                    tracing::debug!(%e, "close request not delivered");
                }
            }
        }
    }

    /// `NSApp.keyWindow` (main thread).
    fn key_window() -> Option<usize> {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        objc2::MainThreadMarker::new()?;
        // SAFETY: plain AppKit getters on the main thread (checked above).
        let key: *mut AnyObject = unsafe {
            let ns_app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            if ns_app.is_null() {
                return None;
            }
            msg_send![ns_app, keyWindow]
        };
        (!key.is_null()).then_some(key as usize)
    }

    /// `[window performClose:nil]`: as a click on its close button (a window without one
    /// beeps and stays).
    fn perform_close(window: *mut c_void) {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        if objc2::MainThreadMarker::new().is_none() {
            return;
        }
        let window = window.cast::<AnyObject>();
        // SAFETY: `window` is the live key window, just read on this (main) thread.
        unsafe {
            let _: () = msg_send![window, performClose: std::ptr::null_mut::<AnyObject>()];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> QuitGate {
        let gate = QuitGate::default();
        gate.set_ui_ready(true);
        gate
    }

    #[test]
    fn quits_without_asking_until_the_ui_listens() {
        let gate = QuitGate::default();
        assert_eq!(gate.request(Instant::now()), QuitAction::Exit);
        // Then every exit goes through.
        assert!(!gate.holds_exit(None, Instant::now()));
    }

    #[test]
    fn asks_once_the_ui_listens_and_follows_its_answer() {
        let gate = ready();
        let t = Instant::now();
        assert_eq!(gate.request(t), QuitAction::Ask);
        gate.answer(false);
        assert_eq!(gate.request(t), QuitAction::Ask);
        gate.answer(true);
        assert_eq!(gate.request(t), QuitAction::Exit);
        assert!(!gate.holds_exit(None, t));
    }

    #[test]
    fn a_repeated_request_asks_again_until_it_went_unanswered() {
        let gate = ready();
        let t = Instant::now();
        assert_eq!(gate.request(t), QuitAction::Ask);
        assert_eq!(gate.request(t + Duration::from_secs(1)), QuitAction::Ask);
        assert_eq!(gate.request(t + UNANSWERED), QuitAction::Exit);
    }

    #[test]
    fn programmatic_exits_are_never_held() {
        let gate = ready();
        let t = Instant::now();
        // `AppHandle::exit(0)`, the updater's restart.
        assert!(!gate.holds_exit(Some(0), t));
        assert!(!gate.holds_exit(Some(i32::MAX), t));
        // The last window closing (no code) is asked about.
        assert!(gate.holds_exit(None, t));
    }

    #[test]
    fn the_ui_going_away_forgets_the_pending_request() {
        let gate = ready();
        let t = Instant::now();
        assert_eq!(gate.request(t), QuitAction::Ask);
        gate.set_ui_ready(true); // reloaded
        assert_eq!(gate.request(t + UNANSWERED), QuitAction::Ask);
        gate.set_ui_ready(false);
        assert_eq!(gate.request(t), QuitAction::Exit);
    }

    #[test]
    fn close_never_targets_the_main_window() {
        assert_eq!(close_target(Some(7), Some(1)), CloseTarget::Native(7));
        assert_eq!(close_target(Some(1), Some(1)), CloseTarget::App);
        assert_eq!(close_target(None, Some(1)), CloseTarget::App);
        assert_eq!(close_target(Some(0), None), CloseTarget::App);
        // No main window handle: any key window is not it.
        assert_eq!(close_target(Some(7), None), CloseTarget::Native(7));
    }
}

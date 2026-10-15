//! The macOS menu bar's app actions (#573): File, Edit's undo / redo / select all, Help.
//!
//! Each item is a keymap action or a command palette entry of the UI. Choosing it emits
//! [`MENU_ACTION_EVENT`] with that id to the main window, which runs it exactly as its
//! shortcut or palette row would (`ui/src/features/desktop-menu`). Undo / redo / select all
//! in a text field stay the text field's: the UI hands them back (`menu_native_edit`), and the
//! shell sends AppKit's `undo:` / `redo:` / `selectAll:` down the responder chain.
//!
//! The key equivalents follow the user's keymap: the UI sends each item's current chord
//! (`menu_set_shortcuts`, keymap syntax `Mod+Shift+Z`), converted by [`accelerator`].
//! Cut / copy / paste stay AppKit's predefined items: they reach the page as clipboard
//! events, which text fields and the editors both take.
//!
//! The text follows the UI's language (docs/LOCALIZATION.md): the UI sends it at start and
//! on every switch (`menu_set_labels`) and the menu is rebuilt with it, its key equivalents
//! reapplied. The defaults here, shown until the UI loads, are the English catalog's
//! (`menu.*` in `ui/src/i18n/locales/en.json`; a test checks it).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;

/// Emitted to the main window with the chosen item's action id.
pub const MENU_ACTION_EVENT: &str = "ether://menu-action";

/// Menu item ids are this prefix plus the action id.
pub const ACTION_PREFIX: &str = "ether-action:";

/// The action id of menu item `id`, if it is one of these items.
pub fn menu_action(id: &str) -> Option<&str> {
    id.strip_prefix(ACTION_PREFIX).filter(|a| !a.is_empty())
}

/// AppKit's selector for an edit action handed back from a text field (allow-list).
pub fn native_selector(action: &str) -> Option<&'static str> {
    match action {
        "edit.undo" => Some("undo:"),
        "edit.redo" => Some("redo:"),
        "edit.selectAll" => Some("selectAll:"),
        _ => None,
    }
}

/// One item: (action id, label, default key equivalent in keymap syntax).
pub type ItemSpec = (&'static str, &'static str, Option<&'static str>);

/// File menu (before Close Window).
pub const FILE_ITEMS: &[&[ItemSpec]] = &[
    &[
        ("project:new", "new project", None),
        ("project:open", "open project…", None),
    ],
    &[
        ("project.save", "save", None),
        ("project:save-as", "save as…", None),
    ],
    &[
        ("file.import", "import audio…", None),
        ("project:export-audio", "export audio…", None),
    ],
    &[("audio-settings", "settings…", None)],
];

/// Edit menu: undo / redo, then (after AppKit's cut / copy / paste) select all.
pub const EDIT_HISTORY: &[ItemSpec] = &[
    ("edit.undo", "undo", Some("Mod+Z")),
    ("edit.redo", "redo", Some("Mod+Shift+Z")),
];
pub const EDIT_SELECT_ALL: ItemSpec = ("edit.selectAll", "select all", Some("Mod+A"));

/// Help menu.
pub const HELP_ITEMS: &[ItemSpec] = &[
    ("keymap:open", "keyboard shortcuts…", None),
    ("app:news", "news…", None),
    ("app:report-issue", "report an issue…", None),
    ("app:send-diagnostics", "send diagnostics…", None),
];

/// "Check for Updates…" (Help, builds with the updater).
pub const UPDATES_ITEM: ItemSpec = ("app:check-updates", "check for updates…", None);

/// The menu bar's text in the UI's language; `actions` labels the items above by action id.
/// Missing fields keep their English default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MenuLabels {
    pub about: String,
    pub services: String,
    pub hide: String,
    pub hide_others: String,
    pub quit: String,
    pub file: String,
    pub close_window: String,
    pub edit: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub view: String,
    pub fullscreen: String,
    pub window: String,
    pub minimize: String,
    pub zoom: String,
    pub help: String,
    pub actions: HashMap<String, String>,
}

impl Default for MenuLabels {
    fn default() -> Self {
        let s = |s: &str| s.to_owned();
        Self {
            about: s("about ethereal"),
            services: s("services"),
            hide: s("hide ethereal"),
            hide_others: s("hide others"),
            quit: s("quit ethereal"),
            file: s("file"),
            close_window: s("close window"),
            edit: s("edit"),
            cut: s("cut"),
            copy: s("copy"),
            paste: s("paste"),
            view: s("view"),
            fullscreen: s("enter full screen"),
            window: s("window"),
            minimize: s("minimize"),
            zoom: s("zoom"),
            help: s("help"),
            actions: HashMap::new(),
        }
    }
}

impl MenuLabels {
    /// The label of item `(action, english, _)`: the UI's, else the English default.
    pub fn action<'a>(&'a self, (action, english, _): &ItemSpec) -> &'a str {
        self.actions
            .get(*action)
            .map(String::as_str)
            .unwrap_or(english)
    }
}

/// The labels the menu is built with (`None`: the defaults).
static LABELS: Mutex<Option<MenuLabels>> = Mutex::new(None);
/// The last key equivalents from the UI, reapplied when the menu is rebuilt.
static SHORTCUTS: Mutex<Vec<MenuShortcut>> = Mutex::new(Vec::new());

/// The current labels.
pub fn labels() -> MenuLabels {
    LABELS
        .lock()
        .map(|l| l.clone().unwrap_or_default())
        .unwrap_or_default()
}

/// Every item whose key equivalent follows the keymap.
pub fn keyed_actions() -> impl Iterator<Item = &'static str> {
    FILE_ITEMS
        .iter()
        .flat_map(|g| g.iter())
        .chain(EDIT_HISTORY.iter())
        .chain(std::iter::once(&EDIT_SELECT_ALL))
        .filter(|(_, _, key)| key.is_some())
        .map(|(id, _, _)| *id)
}

/// A keymap chord (`Mod+Shift+Z`, `Alt+Backspace`, `F9`) as a Tauri accelerator
/// (`CmdOrCtrl+Shift+Z`), or `None` for a chord a menu can't show.
pub fn accelerator(chord: &str) -> Option<String> {
    let mut parts: Vec<&str> = chord.split('+').collect();
    // "Mod++" (a plus key) isn't a keymap chord; an empty part means a malformed one.
    let key = parts.pop().filter(|k| !k.is_empty())?;
    let mut out: Vec<String> = Vec::new();
    for m in parts {
        out.push(
            match m {
                "Mod" => "CmdOrCtrl",
                "Ctrl" => "Ctrl",
                "Alt" => "Alt",
                "Shift" => "Shift",
                _ => return None,
            }
            .to_string(),
        );
    }
    let key = match key {
        "ArrowUp" => "Up".to_string(),
        "ArrowDown" => "Down".to_string(),
        "ArrowLeft" => "Left".to_string(),
        "ArrowRight" => "Right".to_string(),
        "Backspace" | "Delete" | "Space" | "Enter" | "Escape" | "Tab" | "Home" | "End"
        | "PageUp" | "PageDown" => key.to_string(),
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => k.to_ascii_uppercase(),
        k if k.len() == 1 && ",./;'[]\\-=`".contains(k) => k.to_string(),
        k if k.starts_with('F') && k[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            k.to_string()
        }
        _ => return None,
    };
    out.push(key);
    Some(out.join("+"))
}

/// One item's key equivalent from the UI (`chord`: keymap syntax; none: unbound).
#[derive(Debug, Clone, Deserialize)]
pub struct MenuShortcut {
    pub action: String,
    pub chord: Option<String>,
}

/// The UI's keymap changed: set the items' key equivalents to the user's chords.
#[tauri::command]
pub fn menu_set_shortcuts(app: tauri::AppHandle, shortcuts: Vec<MenuShortcut>) {
    #[cfg(target_os = "macos")]
    macos::set_shortcuts(&app, &shortcuts);
    #[cfg(not(target_os = "macos"))]
    let _ = &app;
    if let Ok(mut last) = SHORTCUTS.lock() {
        *last = shortcuts;
    }
}

/// The UI's language changed (or it just loaded): rebuild the menu bar with its text (macOS;
/// elsewhere there is none), keeping the key equivalents.
#[tauri::command]
pub fn menu_set_labels(app: tauri::AppHandle, labels: MenuLabels) -> Result<(), String> {
    if let Ok(mut current) = LABELS.lock() {
        if current.as_ref() == Some(&labels) {
            return Ok(());
        }
        *current = Some(labels);
    }
    #[cfg(target_os = "macos")]
    {
        let menu = crate::quit::menu::build(&app).map_err(|e| e.to_string())?;
        app.set_menu(menu).map_err(|e| e.to_string())?;
        let shortcuts = SHORTCUTS.lock().map(|s| s.clone()).unwrap_or_default();
        macos::set_shortcuts(&app, &shortcuts);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = &app;
    Ok(())
}

/// Undo / redo / select all chosen while a text field has focus: AppKit's own action, down
/// the responder chain (the web view's text editing).
#[tauri::command]
pub fn menu_native_edit(app: tauri::AppHandle, action: String) {
    let Some(selector) = native_selector(&action) else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(move || macos::send_action(selector));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, selector);
}

#[cfg(target_os = "macos")]
pub mod macos {
    use tauri::menu::{IsMenuItem, MenuItem, MenuItemKind, PredefinedMenuItem};
    use tauri::{AppHandle, Runtime};

    use super::{ACTION_PREFIX, ItemSpec, MenuLabels, MenuShortcut, accelerator};

    /// The menu item of `spec` (id `ether-action:<action>`), labelled from `labels`.
    pub fn item<R: Runtime>(
        app: &AppHandle<R>,
        spec: ItemSpec,
        labels: &MenuLabels,
    ) -> tauri::Result<MenuItem<R>> {
        let (action, _, key) = spec;
        let accel = key.and_then(accelerator);
        MenuItem::with_id(
            app,
            format!("{ACTION_PREFIX}{action}"),
            labels.action(&spec),
            true,
            accel.as_deref(),
        )
    }

    /// `groups` as items with separators between them.
    pub fn grouped<R: Runtime>(
        app: &AppHandle<R>,
        groups: &[&[ItemSpec]],
        labels: &MenuLabels,
    ) -> tauri::Result<Vec<Box<dyn IsMenuItem<R>>>> {
        let mut out: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
        for (i, g) in groups.iter().enumerate() {
            if i > 0 {
                out.push(Box::new(PredefinedMenuItem::separator(app)?));
            }
            for spec in *g {
                out.push(Box::new(item(app, *spec, labels)?));
            }
        }
        Ok(out)
    }

    pub fn set_shortcuts<R: Runtime>(app: &AppHandle<R>, shortcuts: &[MenuShortcut]) {
        let Some(menu) = app.menu() else { return };
        let Ok(top) = menu.items() else { return };
        for s in shortcuts {
            if !super::keyed_actions().any(|a| a == s.action) {
                continue;
            }
            let id = format!("{ACTION_PREFIX}{}", s.action);
            let accel = s.chord.as_deref().and_then(accelerator);
            for sub in top.iter().filter_map(MenuItemKind::as_submenu) {
                if let Some(MenuItemKind::MenuItem(item)) = sub.get(&id) {
                    if let Err(e) = item.set_accelerator(accel.as_deref()) {
                        tracing::debug!(%e, id, "menu key equivalent not set");
                    }
                }
            }
        }
    }

    /// `[NSApp sendAction:selector to:nil from:nil]` (main thread).
    pub fn send_action(selector: &'static str) {
        use objc2::msg_send;
        use objc2::runtime::{AnyObject, Sel};
        if objc2::MainThreadMarker::new().is_none() {
            return;
        }
        let sel = Sel::register(&std::ffi::CString::new(selector).expect("selector"));
        // SAFETY: AppKit on the main thread (checked above); a nil target walks the responder
        // chain from the key window's first responder, as a menu item with no target does.
        unsafe {
            let ns_app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            if ns_app.is_null() {
                return;
            }
            let null = std::ptr::null_mut::<AnyObject>();
            let _: bool = msg_send![ns_app, sendAction: sel, to: null, from: null];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_carry_the_action() {
        assert_eq!(menu_action("ether-action:edit.undo"), Some("edit.undo"));
        assert_eq!(menu_action("ether-action:keymap:open"), Some("keymap:open"));
        assert_eq!(menu_action("ether-action:"), None);
        assert_eq!(menu_action("ether-quit"), None);
    }

    #[test]
    fn only_text_edit_actions_go_back_to_appkit() {
        assert_eq!(native_selector("edit.undo"), Some("undo:"));
        assert_eq!(native_selector("edit.redo"), Some("redo:"));
        assert_eq!(native_selector("edit.selectAll"), Some("selectAll:"));
        assert_eq!(native_selector("edit.delete"), None);
        assert_eq!(native_selector("terminate"), None);
    }

    #[test]
    fn keymap_chords_become_accelerators() {
        assert_eq!(accelerator("Mod+Z").as_deref(), Some("CmdOrCtrl+Z"));
        assert_eq!(
            accelerator("Mod+Shift+Z").as_deref(),
            Some("CmdOrCtrl+Shift+Z")
        );
        assert_eq!(accelerator("Ctrl+Alt+a").as_deref(), Some("Ctrl+Alt+A"));
        assert_eq!(
            accelerator("Alt+Backspace").as_deref(),
            Some("Alt+Backspace")
        );
        assert_eq!(accelerator("Shift+ArrowUp").as_deref(), Some("Shift+Up"));
        assert_eq!(accelerator("F9").as_deref(), Some("F9"));
        assert_eq!(accelerator("Mod+,").as_deref(), Some("CmdOrCtrl+,"));
        assert_eq!(accelerator("Hyper+Z"), None);
        assert_eq!(accelerator("Mod+"), None);
        assert_eq!(accelerator("F99"), None);
    }

    /// The defaults (shown before the UI loads) are the English catalog's.
    #[test]
    fn label_defaults_are_the_english_catalog() {
        let en: HashMap<String, String> =
            serde_json::from_str(include_str!("../../../../ui/src/i18n/locales/en.json")).unwrap();
        let msg = |k: &str| en[k].replace("{app}", "ethereal");
        let d = MenuLabels::default();
        let pairs = [
            (&d.about, "menu.app.about"),
            (&d.services, "menu.app.services"),
            (&d.hide, "menu.app.hide"),
            (&d.hide_others, "menu.app.hide_others"),
            (&d.quit, "menu.app.quit"),
            (&d.file, "menu.file"),
            (&d.close_window, "menu.file.close_window"),
            (&d.edit, "menu.edit"),
            (&d.cut, "menu.edit.cut"),
            (&d.copy, "menu.edit.copy"),
            (&d.paste, "menu.edit.paste"),
            (&d.view, "menu.view"),
            (&d.fullscreen, "menu.view.fullscreen"),
            (&d.window, "menu.window"),
            (&d.minimize, "menu.window.minimize"),
            (&d.zoom, "menu.window.zoom"),
            (&d.help, "menu.help"),
        ];
        for (value, key) in pairs {
            assert_eq!(*value, msg(key), "{key}");
        }
        // Each app item's default is `menu.action.<action id, snake_case>`.
        for spec in FILE_ITEMS
            .iter()
            .flat_map(|g| g.iter())
            .chain(EDIT_HISTORY)
            .chain([&EDIT_SELECT_ALL])
            .chain(HELP_ITEMS)
            .chain([&UPDATES_ITEM])
        {
            let key = format!("menu.action.{}", catalog_id(spec.0));
            assert_eq!(spec.1, msg(&key), "{key}");
        }
        // The UI's labels replace the defaults; what it leaves out stays.
        let l: MenuLabels =
            serde_json::from_str(r#"{"file":"fichier","actions":{"edit.undo":"annuler"}}"#)
                .unwrap();
        assert_eq!(l.file, "fichier");
        assert_eq!(l.edit, "edit");
        assert_eq!(l.action(&EDIT_HISTORY[0]), "annuler");
        assert_eq!(l.action(&EDIT_HISTORY[1]), "redo");
    }

    /// `project:save-as` → `project_save_as` (catalog keys are snake_case).
    fn catalog_id(action: &str) -> String {
        let mut out = String::new();
        for c in action.chars() {
            match c {
                ':' | '.' | '-' => out.push('_'),
                c if c.is_ascii_uppercase() => {
                    out.push('_');
                    out.push(c.to_ascii_lowercase());
                }
                c => out.push(c),
            }
        }
        out
    }

    #[test]
    fn every_keyed_item_has_a_valid_default() {
        let keyed: Vec<_> = keyed_actions().collect();
        assert_eq!(keyed, ["edit.undo", "edit.redo", "edit.selectAll"]);
        for (_, _, key) in FILE_ITEMS
            .iter()
            .flat_map(|g| g.iter())
            .chain(EDIT_HISTORY)
            .chain([&EDIT_SELECT_ALL])
        {
            if let Some(k) = key {
                assert!(accelerator(k).is_some(), "{k}");
            }
        }
    }
}

//! Key forwarding from plugin editor windows to the app (CONTRACTS.md
//! §13.15).
//!
//! A plugin editor lives in a native window the host made (a format's `HostWindow`). While
//! it has keyboard focus, keys go to the plugin and never reach the app's web view, so Space
//! could not play/stop. This module intercepts the few keys the app wants (**captured**
//! codes) before the plugin sees them and queues them per window; the format's `poll` turns
//! them into [`PluginNotification::EditorKey`], which the controller emits as
//! `PluginEvent::EditorKey` (the UI then handles the key like one pressed in its own window).
//!
//! - **What is captured**: `Space`, plus the codes set with [`set_captured_codes`]
//!   (`PluginCommand::SetEditorKeyCapture`, e.g. the computer-keyboard MIDI keys), only
//!   without Ctrl / Alt / Cmd (plugin shortcuts keep working); Shift is allowed. A release is
//!   forwarded only when its press was. Every other key goes to the plugin as before.
//! - **Where**: only in windows registered with [`KeyTap::register`] (one per open editor).
//! - **How**: one process-wide hook, installed on the first registration (on the thread
//!   that owns the editor windows): macOS an `NSEvent` local monitor (key down/up, before
//!   AppKit dispatches the event to the window), Windows a `WH_GETMESSAGE` hook on that
//!   thread (the message is turned into `WM_NULL`, so neither the plugin nor
//!   `TranslateMessage` sees it). Elsewhere host windows don't exist, so nothing is hooked.
//!
//! The routing ([`route`]) and key tables are plain functions, tested on every OS.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use ether_core::plugin::PluginNotification;
use ether_core::protocol::plugins::PluginWindowKey;

/// Always captured (the transport: Space = play/stop, Shift+Space = stop to the selection).
pub const ALWAYS_CAPTURED: &str = "Space";

/// Keys queued per window between two polls (the oldest are dropped past this).
const MAX_QUEUED: usize = 256;

/// Modifier state of a key event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Default)]
struct State {
    /// Registered windows (native handle as an integer) and their pending keys.
    windows: HashMap<usize, VecDeque<PluginWindowKey>>,
    /// Captured codes besides `Space` (`set_captured_codes`).
    captured: HashSet<String>,
    /// Keys whose press was captured, per window: their release is captured too.
    held: HashSet<(usize, String)>,
    /// Bumped by every `set_captured_codes` (the sandbox host forwards changes to helpers).
    generation: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(state().get_or_insert_with(State::default))
}

/// Replace the captured codes (`KeyboardEvent.code` values) besides `Space`.
pub fn set_captured_codes<I, S>(codes: I)
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let codes: HashSet<String> = codes.into_iter().map(Into::into).collect();
    with_state(|s| {
        s.captured = codes;
        s.generation += 1;
    });
}

/// Changes on every [`set_captured_codes`] (0 before the first).
pub fn capture_generation() -> u64 {
    with_state(|s| s.generation)
}

/// The captured codes besides `Space`, sorted.
pub fn captured_codes() -> Vec<String> {
    with_state(|s| {
        let mut v: Vec<String> = s.captured.iter().cloned().collect();
        v.sort();
        v
    })
}

/// Whether a press of `code` with `mods` is captured (not given to the plugin).
pub fn captures(code: &str, mods: Mods) -> bool {
    if mods.ctrl || mods.alt || mods.meta {
        return false;
    }
    code == ALWAYS_CAPTURED || with_state(|s| s.captured.contains(code))
}

/// A key event in native window `window`: returns `true` when it is captured (queued for
/// the app; the caller must then keep it from the plugin). Called by the platform hooks.
pub fn route(window: usize, code: &str, mods: Mods, down: bool, repeat: bool) -> bool {
    let mut guard = state();
    let s = guard.get_or_insert_with(State::default);
    if !s.windows.contains_key(&window) {
        return false;
    }
    let held_key = (window, code.to_owned());
    if down {
        let captured = !(mods.ctrl || mods.alt || mods.meta)
            && (code == ALWAYS_CAPTURED || s.captured.contains(code));
        if !captured {
            return false;
        }
        s.held.insert(held_key);
    } else if !s.held.remove(&held_key) {
        return false;
    }
    let key = PluginWindowKey {
        key: key_for(code, mods.shift),
        code: code.to_owned(),
        shift: mods.shift,
        ctrl: mods.ctrl,
        alt: mods.alt,
        meta: mods.meta,
        down,
        repeat: down && repeat,
    };
    let queue = s.windows.entry(window).or_default();
    if queue.len() >= MAX_QUEUED {
        queue.pop_front();
    }
    queue.push_back(key);
    true
}

/// A registered editor window: keys captured in it are queued until [`KeyTap::drain`].
/// Unregisters on drop.
#[derive(Debug)]
pub struct KeyTap {
    window: usize,
}

impl KeyTap {
    /// Register native window `window` (`NSWindow*` on macOS, the top-level `HWND` on
    /// Windows) and install the process hook if needed. Call on the thread that owns the
    /// window (the editor main thread).
    pub fn register(window: *mut c_void) -> Self {
        let window = window as usize;
        with_state(|s| {
            s.windows.entry(window).or_default();
        });
        platform::install();
        Self { window }
    }

    /// Move the keys captured since the last call into `out` as notifications.
    pub fn drain(&self, out: &mut Vec<PluginNotification>) {
        with_state(|s| {
            if let Some(q) = s.windows.get_mut(&self.window) {
                out.extend(q.drain(..).map(PluginNotification::EditorKey));
            }
        });
    }
}

impl Drop for KeyTap {
    fn drop(&mut self) {
        let w = self.window;
        with_state(|s| {
            s.windows.remove(&w);
            s.held.retain(|(win, _)| *win != w);
        });
    }
}

/// `KeyboardEvent.key` for `code` (US layout, best effort): letters follow Shift, digits and
/// punctuation are unshifted, named keys keep their name.
pub fn key_for(code: &str, shift: bool) -> String {
    if let Some(l) = code.strip_prefix("Key") {
        return if shift {
            l.to_owned()
        } else {
            l.to_lowercase()
        };
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return d.to_owned();
    }
    let k = match code {
        "Space" => " ",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Backquote" => "`",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Backspace" => "Backspace",
        "Tab" => "Tab",
        "Enter" => "Enter",
        "Escape" => "Escape",
        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => code,
        other => other,
    };
    k.to_owned()
}

/// macOS virtual key code (`kVK_*`, a physical position) → `KeyboardEvent.code`.
pub fn mac_code(key_code: u16) -> Option<&'static str> {
    Some(match key_code {
        0x00 => "KeyA",
        0x01 => "KeyS",
        0x02 => "KeyD",
        0x03 => "KeyF",
        0x04 => "KeyH",
        0x05 => "KeyG",
        0x06 => "KeyZ",
        0x07 => "KeyX",
        0x08 => "KeyC",
        0x09 => "KeyV",
        0x0B => "KeyB",
        0x0C => "KeyQ",
        0x0D => "KeyW",
        0x0E => "KeyE",
        0x0F => "KeyR",
        0x10 => "KeyY",
        0x11 => "KeyT",
        0x12 => "Digit1",
        0x13 => "Digit2",
        0x14 => "Digit3",
        0x15 => "Digit4",
        0x16 => "Digit6",
        0x17 => "Digit5",
        0x18 => "Equal",
        0x19 => "Digit9",
        0x1A => "Digit7",
        0x1B => "Minus",
        0x1C => "Digit8",
        0x1D => "Digit0",
        0x1E => "BracketRight",
        0x1F => "KeyO",
        0x20 => "KeyU",
        0x21 => "BracketLeft",
        0x22 => "KeyI",
        0x23 => "KeyP",
        0x24 => "Enter",
        0x25 => "KeyL",
        0x26 => "KeyJ",
        0x27 => "Quote",
        0x28 => "KeyK",
        0x29 => "Semicolon",
        0x2A => "Backslash",
        0x2B => "Comma",
        0x2C => "Slash",
        0x2D => "KeyN",
        0x2E => "KeyM",
        0x2F => "Period",
        0x30 => "Tab",
        0x31 => "Space",
        0x32 => "Backquote",
        0x33 => "Backspace",
        0x35 => "Escape",
        0x7B => "ArrowLeft",
        0x7C => "ArrowRight",
        0x7D => "ArrowDown",
        0x7E => "ArrowUp",
        _ => return None,
    })
}

/// Windows set-1 scan code (a physical position; `extended`: the E0 prefix, lParam bit 24)
/// → `KeyboardEvent.code`.
pub fn win_code(scan: u32, extended: bool) -> Option<&'static str> {
    if extended {
        return Some(match scan {
            0x48 => "ArrowUp",
            0x4B => "ArrowLeft",
            0x4D => "ArrowRight",
            0x50 => "ArrowDown",
            0x1C => "NumpadEnter",
            _ => return None,
        });
    }
    Some(match scan {
        0x01 => "Escape",
        0x02 => "Digit1",
        0x03 => "Digit2",
        0x04 => "Digit3",
        0x05 => "Digit4",
        0x06 => "Digit5",
        0x07 => "Digit6",
        0x08 => "Digit7",
        0x09 => "Digit8",
        0x0A => "Digit9",
        0x0B => "Digit0",
        0x0C => "Minus",
        0x0D => "Equal",
        0x0E => "Backspace",
        0x0F => "Tab",
        0x10 => "KeyQ",
        0x11 => "KeyW",
        0x12 => "KeyE",
        0x13 => "KeyR",
        0x14 => "KeyT",
        0x15 => "KeyY",
        0x16 => "KeyU",
        0x17 => "KeyI",
        0x18 => "KeyO",
        0x19 => "KeyP",
        0x1A => "BracketLeft",
        0x1B => "BracketRight",
        0x1C => "Enter",
        0x1E => "KeyA",
        0x1F => "KeyS",
        0x20 => "KeyD",
        0x21 => "KeyF",
        0x22 => "KeyG",
        0x23 => "KeyH",
        0x24 => "KeyJ",
        0x25 => "KeyK",
        0x26 => "KeyL",
        0x27 => "Semicolon",
        0x28 => "Quote",
        0x29 => "Backquote",
        0x2B => "Backslash",
        0x2C => "KeyZ",
        0x2D => "KeyX",
        0x2E => "KeyC",
        0x2F => "KeyV",
        0x30 => "KeyB",
        0x31 => "KeyN",
        0x32 => "KeyM",
        0x33 => "Comma",
        0x34 => "Period",
        0x35 => "Slash",
        0x39 => "Space",
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ptr::NonNull;
    use std::sync::Once;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};

    use super::{Mods, mac_code, route};

    /// Install the local key monitor (once per process; AppKit: main thread only).
    pub(super) fn install() {
        static ONCE: Once = Once::new();
        if MainThreadMarker::new().is_none() {
            return;
        }
        ONCE.call_once(|| {
            let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
                // SAFETY: AppKit passes a live event for the duration of the call.
                let e = unsafe { event.as_ref() };
                if filter(e) {
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            });
            // SAFETY: the handler returns the event it was given or null (swallow).
            let monitor = unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                    NSEventMask::KeyDown | NSEventMask::KeyUp,
                    &block,
                )
            };
            // The monitor lives as long as the process.
            std::mem::forget(monitor);
        });
    }

    /// `true` when the event is captured (swallowed).
    fn filter(e: &NSEvent) -> bool {
        // Local monitors run on the main thread.
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let down = match e.r#type() {
            NSEventType::KeyDown => true,
            NSEventType::KeyUp => false,
            _ => return false,
        };
        let Some(window) = e.window(mtm) else {
            return false;
        };
        let Some(code) = mac_code(e.keyCode()) else {
            return false;
        };
        let flags = e.modifierFlags();
        let mods = Mods {
            shift: flags.contains(NSEventModifierFlags::Shift),
            ctrl: flags.contains(NSEventModifierFlags::Control),
            alt: flags.contains(NSEventModifierFlags::Option),
            meta: flags.contains(NSEventModifierFlags::Command),
        };
        let repeat = down && e.isARepeat();
        route(Retained::as_ptr(&window) as usize, code, mods, down, repeat)
    }
}

#[cfg(windows)]
mod platform {
    use std::cell::Cell;

    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GA_ROOT, GetAncestor, HC_ACTION, HHOOK, MSG, PM_REMOVE, SetWindowsHookExW,
        WH_GETMESSAGE, WM_KEYDOWN, WM_KEYUP, WM_NULL,
    };

    use super::{Mods, route, win_code};

    thread_local! {
        /// This thread's `WH_GETMESSAGE` hook (editor windows are pumped by their thread).
        static HOOK: Cell<HHOOK> = const { Cell::new(std::ptr::null_mut()) };
    }

    /// Install the message hook on the calling thread (once per thread).
    pub(super) fn install() {
        HOOK.with(|h| {
            if !h.get().is_null() {
                return;
            }
            // SAFETY: a thread-local hook with a procedure in this module.
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_GETMESSAGE,
                    Some(get_msg),
                    std::ptr::null_mut(),
                    GetCurrentThreadId(),
                )
            };
            h.set(hook);
        });
    }

    fn pressed(vk: u16) -> bool {
        // SAFETY: plain key state query for this thread.
        unsafe { GetKeyState(i32::from(vk)) < 0 }
    }

    unsafe extern "system" fn get_msg(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && wp == PM_REMOVE as usize && lp != 0 {
            // SAFETY: for WH_GETMESSAGE, lParam points to the MSG being retrieved, which the
            // hook may modify.
            let msg = unsafe { &mut *(lp as *mut MSG) };
            if msg.message == WM_KEYDOWN || msg.message == WM_KEYUP {
                let scan = ((msg.lParam >> 16) & 0xFF) as u32;
                let extended = (msg.lParam >> 24) & 1 != 0;
                if let Some(key) = win_code(scan, extended) {
                    // SAFETY: plain window query.
                    let root = unsafe { GetAncestor(msg.hwnd, GA_ROOT) };
                    let mods = Mods {
                        shift: pressed(VK_SHIFT),
                        ctrl: pressed(VK_CONTROL),
                        alt: pressed(VK_MENU),
                        meta: pressed(VK_LWIN) || pressed(VK_RWIN),
                    };
                    let down = msg.message == WM_KEYDOWN;
                    let repeat = down && (msg.lParam >> 30) & 1 != 0;
                    if route(root as usize, key, mods, down, repeat) {
                        msg.message = WM_NULL;
                    }
                }
            }
        }
        // SAFETY: pass the message on to the next hook.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wp, lp) }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    /// No host editor windows on this OS: nothing to hook.
    pub(super) fn install() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test touches the process-wide state at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::default()
        }
    }

    fn drained(tap: &KeyTap) -> Vec<PluginWindowKey> {
        let mut out = Vec::new();
        tap.drain(&mut out);
        out.into_iter()
            .map(|n| match n {
                PluginNotification::EditorKey(k) => k,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn space_is_always_captured_in_registered_windows_only() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_captured_codes(Vec::<String>::new());
        let tap = KeyTap::register(0x1000 as *mut c_void);
        assert!(route(0x1000, "Space", Mods::default(), true, false));
        assert!(route(0x1000, "Space", Mods::default(), false, false));
        assert!(route(0x1000, "Space", shift(), true, true));
        // Not a plugin window: the app's own (or a plugin popup) keeps its keys.
        assert!(!route(0x2000, "Space", Mods::default(), true, false));
        // Other keys go to the plugin.
        assert!(!route(0x1000, "KeyA", Mods::default(), true, false));
        let keys = drained(&tap);
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].key, " ");
        assert_eq!(keys[0].code, "Space");
        assert!(keys[0].down && !keys[0].repeat);
        assert!(!keys[1].down);
        assert!(keys[2].shift && keys[2].repeat);
        assert!(drained(&tap).is_empty());
    }

    #[test]
    fn modified_keys_go_to_the_plugin_and_their_release_too() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let tap = KeyTap::register(0x1100 as *mut c_void);
        let cmd = Mods {
            meta: true,
            ..Mods::default()
        };
        assert!(!route(0x1100, "Space", cmd, true, false));
        // Its release was never captured: the plugin gets it.
        assert!(!route(0x1100, "Space", Mods::default(), false, false));
        assert!(!route(
            0x1100,
            "Space",
            Mods {
                ctrl: true,
                ..Mods::default()
            },
            true,
            false
        ));
        assert!(drained(&tap).is_empty());
    }

    #[test]
    fn extra_codes_are_captured_once_set() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let tap = KeyTap::register(0x1200 as *mut c_void);
        assert!(!route(0x1200, "KeyA", Mods::default(), true, false));
        set_captured_codes(["KeyA", "KeyW"]);
        assert_eq!(captured_codes(), vec!["KeyA".to_owned(), "KeyW".to_owned()]);
        assert!(captures("KeyA", shift()));
        assert!(route(0x1200, "KeyA", shift(), true, false));
        assert!(route(0x1200, "KeyA", Mods::default(), false, false));
        let keys = drained(&tap);
        assert_eq!(keys[0].key, "A");
        assert_eq!(keys[1].key, "a");
        set_captured_codes(Vec::<String>::new());
        assert!(!captures("KeyA", Mods::default()));
    }

    #[test]
    fn dropping_the_tap_unregisters_the_window() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let tap = KeyTap::register(0x1300 as *mut c_void);
        assert!(route(0x1300, "Space", Mods::default(), true, false));
        drop(tap);
        assert!(!route(0x1300, "Space", Mods::default(), false, false));
    }

    #[test]
    fn key_tables() {
        assert_eq!(mac_code(0x31), Some("Space"));
        assert_eq!(mac_code(0x00), Some("KeyA"));
        assert_eq!(mac_code(0x7E), Some("ArrowUp"));
        assert_eq!(mac_code(0x60), None);
        assert_eq!(win_code(0x39, false), Some("Space"));
        assert_eq!(win_code(0x1E, false), Some("KeyA"));
        assert_eq!(win_code(0x48, true), Some("ArrowUp"));
        assert_eq!(win_code(0x48, false), None);
        assert_eq!(key_for("KeyQ", false), "q");
        assert_eq!(key_for("Digit7", true), "7");
        assert_eq!(key_for("Space", true), " ");
    }
}

//! Customizable keyboard shortcuts (v0.3, `keymap`; CONTRACTS.md §13.12).
//!
//! Actions and the built-in presets live in the UI (the command registry; presets
//! "Ethereal" and "Ableton-like"); the engine only stores the user's keymap in the user
//! library (`<user library>/Settings/keymap.json`), so it follows the user across projects
//! and, with a remote engine, across machines. The UI never touches files. Not undoable.
//!
//! **Chords** are strings: modifiers in this order, then one key, joined by `+`:
//! `Mod` (Cmd on macOS, Ctrl elsewhere), `Ctrl` (the real Control key on macOS only), `Alt`,
//! `Shift`, then the key: `A`..`Z`, `0`..`9`, `F1`..`F24`, `Space`, `Enter`, `Escape`,
//! `Backspace`, `Delete`, `Tab`, `ArrowUp`/`Down`/`Left`/`Right`, `Home`, `End`, `PageUp`,
//! `PageDown`, or a punctuation character as typed without Shift (`,` `.` `/` `;` `'` `[`
//! `]` `\` `-` `=` `` ` ``). Example: `Mod+Shift+D`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Most overrides a keymap holds.
pub const MAX_KEYMAP_OVERRIDES: usize = 1_024;
/// Most chords bound to one action.
pub const MAX_CHORDS_PER_ACTION: usize = 4;

/// Built-in preset the user's overrides apply on top of. Append-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum KeymapPreset {
    #[default]
    Ethereal,
    AbletonLike,
}

/// The user's bindings for one action (replaces the preset's; empty = unbound).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KeyBinding {
    /// Action id from the UI command registry (`"transport.play"`, `"edit.duplicate"`).
    pub action: String,
    pub chords: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Keymap {
    pub preset: KeymapPreset,
    /// Sorted by action, one entry per action.
    pub overrides: Vec<KeyBinding>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum KeymapCommand {
    /// Replies `Keymap` (the default keymap when none is stored).
    Get,
    /// Store the user's keymap (validated: chord syntax, limits, sorted unique actions).
    Set { keymap: Keymap },
    /// Back to the default (removes the stored file).
    Reset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum KeymapEvent {
    /// The stored keymap changed (another window or remote UI).
    Changed { keymap: Keymap },
}

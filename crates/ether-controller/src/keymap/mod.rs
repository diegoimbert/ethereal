//! User keymap storage (v0.3, owned by the `keymap` node; protocol `ether_protocol::keymap`,
//! CONTRACTS.md §13.12).
//!
//! [`EtherController::keymap_command`]: `Keymap::{Get, Set, Reset}` (from `handlers.rs`):
//! read/write `Settings/keymap.json` in the writable user library
//! (`Library::{read, write_file, remove_file, user_root}`), validating chords and limits;
//! `Set`/`Reset` emit `KeymapEvent::Changed`. Hosts without a writable library keep the
//! keymap for the session only ([`KeymapState`]). Everything else (actions, presets,
//! conflicts, the editor, the cheat sheet) is UI-side (`ui/src/features/keymap/`).
//!
//! A stored file that no longer parses or validates (hand-edited, from a newer version) is
//! ignored: `Get` answers the default keymap and the next `Set` overwrites it.

use ether_core::protocol::keymap::{
    KeyBinding, Keymap, KeymapCommand, KeymapEvent, MAX_CHORDS_PER_ACTION, MAX_KEYMAP_OVERRIDES,
};
use ether_core::protocol::{Event, ReplyValue};

use crate::handlers::{event, store_err};
use crate::store::{Library, ProjectStore, StoreError};
use crate::tx::{CmdResult, invalid};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// The keymap file inside the user library root.
pub(crate) const KEYMAP_FILE: &str = "Settings/keymap.json";

/// Longest action id accepted (ids are short dotted names like `"transport.play"`).
pub const MAX_ACTION_ID_LEN: usize = 128;

/// Runtime keymap state: the session copy for hosts without a writable user library.
#[derive(Debug, Default)]
pub(crate) struct KeymapState {
    session: Option<Keymap>,
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn keymap_command(
        &mut self,
        command: &KeymapCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            KeymapCommand::Get => Ok(ReplyValue::Keymap {
                keymap: self.keymap_load(),
            }),
            KeymapCommand::Set { keymap } => {
                validate_keymap(keymap)?;
                self.keymap_store(Some(keymap))?;
                changed(out, keymap.clone());
                Ok(ReplyValue::Unit)
            }
            KeymapCommand::Reset => {
                self.keymap_store(None)?;
                changed(out, Keymap::default());
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// The stored keymap, or the default one (none stored, or the file is unusable).
    fn keymap_load(&mut self) -> Keymap {
        let Some(root) = self.library.user_root() else {
            return self.keymap.session.clone().unwrap_or_default();
        };
        match self.library.read(&root, KEYMAP_FILE) {
            Ok(bytes) => parse_keymap(&bytes).unwrap_or_default(),
            Err(_) => Keymap::default(),
        }
    }

    /// Write (`Some`) or remove (`None`) the stored keymap.
    fn keymap_store(&mut self, keymap: Option<&Keymap>) -> CmdResult<()> {
        let Some(root) = self.library.user_root() else {
            self.keymap.session = keymap.cloned();
            return Ok(());
        };
        match keymap {
            Some(k) => {
                let json = serde_json::to_vec_pretty(k)
                    .map_err(|e| invalid(format!("keymap does not serialize: {e}")))?;
                self.library
                    .write_file(&root, KEYMAP_FILE, &json)
                    .map_err(store_err)
            }
            None => match self.library.remove_file(&root, KEYMAP_FILE) {
                Ok(()) | Err(StoreError::NotFound(_)) => Ok(()),
                Err(e) => Err(store_err(e)),
            },
        }
    }
}

fn changed(out: &mut dyn MessageSink, keymap: Keymap) {
    event(
        out,
        Event::Keymap {
            event: KeymapEvent::Changed { keymap },
        },
    );
}

/// A stored keymap file, if it parses and validates.
fn parse_keymap(bytes: &[u8]) -> Option<Keymap> {
    let keymap: Keymap = serde_json::from_slice(bytes).ok()?;
    validate_keymap(&keymap).ok()?;
    Some(keymap)
}

/// Contract checks (CONTRACTS.md §13.12): limits, sorted unique actions, chord syntax, no
/// chord twice in one binding.
pub fn validate_keymap(keymap: &Keymap) -> CmdResult<()> {
    if keymap.overrides.len() > MAX_KEYMAP_OVERRIDES {
        return Err(invalid(format!(
            "too many keymap overrides ({} > {MAX_KEYMAP_OVERRIDES})",
            keymap.overrides.len()
        )));
    }
    let mut previous: Option<&str> = None;
    for KeyBinding { action, chords } in &keymap.overrides {
        if action.is_empty()
            || action.len() > MAX_ACTION_ID_LEN
            || action.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(invalid(format!("invalid action id {action:?}")));
        }
        if let Some(p) = previous
            && p >= action.as_str()
        {
            return Err(invalid(format!(
                "keymap overrides must be sorted by action, one per action ({p:?} before {action:?})"
            )));
        }
        previous = Some(action);
        if chords.len() > MAX_CHORDS_PER_ACTION {
            return Err(invalid(format!(
                "too many chords for {action} ({} > {MAX_CHORDS_PER_ACTION})",
                chords.len()
            )));
        }
        for (i, chord) in chords.iter().enumerate() {
            if !valid_chord(chord) {
                return Err(invalid(format!("invalid chord {chord:?} for {action}")));
            }
            if chords[..i].contains(chord) {
                return Err(invalid(format!("chord {chord} twice for {action}")));
            }
        }
    }
    Ok(())
}

/// Modifiers, in the only order allowed.
const MODIFIERS: [&str; 4] = ["Mod", "Ctrl", "Alt", "Shift"];

const NAMED_KEYS: [&str; 14] = [
    "Space",
    "Enter",
    "Escape",
    "Backspace",
    "Delete",
    "Tab",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
];

const PUNCTUATION: [char; 11] = [',', '.', '/', ';', '\'', '[', ']', '\\', '-', '=', '`'];

/// Chord syntax: optional modifiers in the order `Mod`, `Ctrl`, `Alt`, `Shift` (each at most
/// once), then exactly one key, joined by `+`.
pub fn valid_chord(chord: &str) -> bool {
    let parts: Vec<&str> = chord.split('+').collect();
    let Some((key, mods)) = parts.split_last() else {
        return false;
    };
    let mut next = 0;
    for m in mods {
        match MODIFIERS[next..].iter().position(|x| x == m) {
            Some(i) => next += i + 1,
            None => return false,
        }
    }
    valid_key(key)
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.is_ascii_uppercase() || c.is_ascii_digit() || PUNCTUATION.contains(&c),
        (Some(_), Some(_)) => {
            NAMED_KEYS.contains(&key)
                || key
                    .strip_prefix('F')
                    .and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=24).contains(&n) && !key[1..].starts_with('0'))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_syntax() {
        for ok in [
            "A",
            "Mod+Shift+D",
            "Mod+Ctrl+Alt+Shift+Z",
            "Space",
            "F1",
            "F24",
            "Alt+ArrowUp",
            "Mod+,",
            "Shift+/",
            "Mod+=",
            "Mod+`",
            "9",
        ] {
            assert!(valid_chord(ok), "{ok}");
        }
        for bad in [
            "",
            "a",
            "Shift+Mod+D",
            "Mod+Mod+D",
            "Mod+",
            "Mod",
            "F0",
            "F25",
            "F01",
            "Cmd+D",
            "Mod+D+E",
            "Mod+?",
            "ArrowUP",
            " A",
        ] {
            assert!(!valid_chord(bad), "{bad}");
        }
    }
}

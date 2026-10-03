//! User keymap storage (v0.3, owned by the `keymap` node; protocol `ether_protocol::keymap`,
//! CONTRACTS.md §13.12).
//!
//! [`EtherController::keymap_command`]: `Keymap::{Get, Set, Reset}` (from `handlers.rs`):
//! read/write `Settings/keymap.json` in the writable user library
//! (`Library::{read, write_file, remove_file, user_root}`), validating chords and limits;
//! `Set`/`Reset` emit `KeymapEvent::Changed`. Hosts without a writable library keep the
//! keymap for the session only. Everything else (actions, presets, conflicts, the editor,
//! the cheat sheet) is UI-side (`ui/src/features/keymap/`).
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::keymap::KeymapCommand;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// The keymap file inside the user library root.
pub(crate) const KEYMAP_FILE: &str = "Settings/keymap.json";

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
        let _ = (command, out, KEYMAP_FILE);
        Err(unsupported(
            "custom keymaps are not implemented yet (keymap)",
        ))
    }
}

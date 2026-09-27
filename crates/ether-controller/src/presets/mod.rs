//! Device presets (v0.2, owned by the `presets` node; file format `ether_model::preset`,
//! protocol `ether_protocol::presets`, CONTRACTS.md §12.5).
//!
//! [`EtherController::preset_command`] (dispatched from `handlers.rs`): `List` merges
//! `ether_devices::factory_presets` (built-ins) with the user presets read through the
//! `Library` (`<library>/Presets/...`, `Library::{list_dir, read, write, remove, rename}`);
//! `Load` is one document transaction (`edit_with`): params reset/set, kind data, plugin
//! state (+ `EngineBridge` reload for plugins); `Save` serializes the device
//! (`EngineBridge::plugin_state` for plugins) with `ether_model::save_preset`.
//! `PresetEvent::Changed` after user-set changes. Placeholder: `Unsupported`.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::GestureId;
use ether_core::protocol::presets::PresetCommand;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn preset_command(
        &mut self,
        command: &PresetCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, gesture, now, out);
        Err(unsupported("presets are not implemented yet (presets)"))
    }
}

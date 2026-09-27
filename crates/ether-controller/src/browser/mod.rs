//! Browser v2: engine-side library index (v0.2, owned by the `browser-v2` node; protocol
//! `ether_protocol::browser`, CONTRACTS.md §12.8).
//!
//! [`EtherController::browser_command`] (dispatched from `handlers.rs`) and
//! [`EtherController::browser_tick`] (every tick: bounded indexing work, progress events).
//! The index is persisted through the `Library` (`<library>/.ethereal/index.json`); user
//! folders through `Library::{add_folder, remove_folder}` (native). `Preview` reuses
//! `media_preview` (tempo sync: a rate ratio on the preview voice, a shared touch in
//! `ether-core/src/preview.rs`). Placeholder: `Unsupported`.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::browser::BrowserCommand;

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
    pub(crate) fn browser_command(
        &mut self,
        command: &BrowserCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out);
        Err(unsupported(
            "the library index is not implemented yet (browser-v2)",
        ))
    }

    /// Called every tick.
    pub(crate) fn browser_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

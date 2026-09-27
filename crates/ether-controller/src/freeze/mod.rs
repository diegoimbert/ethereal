//! Freeze, flatten, bounce in place, consolidate (v0.2, owned by the `freeze-bounce` node;
//! protocol `ether_protocol::freeze`, model `Track::freeze`, CONTRACTS.md §12.3).
//!
//! - [`EtherController::freeze_command`] (dispatched from `handlers.rs`): render jobs on an
//!   `ether_core::offline::OfflineRenderer` stepped from [`EtherController::freeze_tick`]
//!   (reuse `export`'s job code: `crate::export` is merged; share by moving helpers into
//!   this module or calling its `pub(crate)` items), media written to the project's `media/`
//!   through `ProjectStore::write`, the document edit at `Done` as one undo step.
//! - [`frozen_desc`]: `compile.rs` calls it per track; `Some` = the track compiles to
//!   `TrackDesc::frozen` with no clips, chain, racks or modulation (its nodes are not
//!   instantiated: `engine.rs::sync_nodes` must skip devices of frozen tracks, a shared
//!   touch). Engine side: `ether_core::freeze::render_frozen`.
//! - [`check_editable`]: called before every document command (`handlers.rs`): rejects
//!   edits to a frozen track's clips, devices and device automation (`InvalidState`).
//!
//! Placeholders: commands reply `Unsupported`, nothing is frozen, everything is editable.

use ether_core::freeze::FrozenDesc;
use ether_core::protocol::freeze::FreezeCommand;
use ether_core::protocol::model::{Project, Track};
use ether_core::protocol::{Command, ReplyValue};

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
    pub(crate) fn freeze_command(
        &mut self,
        command: &FreezeCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out);
        Err(unsupported(
            "freeze and bounce are not implemented yet (freeze-bounce)",
        ))
    }

    /// Called every tick: advance the running render job.
    pub(crate) fn freeze_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

/// The frozen render of `track` for the engine (see the module docs).
pub(crate) fn frozen_desc(p: &Project, track: &Track) -> Option<FrozenDesc> {
    let _ = (p, track);
    None
}

/// Reject document commands that edit a frozen track's content.
pub(crate) fn check_editable(p: &Project, command: &Command) -> CmdResult<()> {
    let _ = (p, command);
    Ok(())
}

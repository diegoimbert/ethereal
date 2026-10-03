//! Project versions and crash recovery (v0.3, owned by the `project-versions` node;
//! protocol `ether_protocol::versions`, CONTRACTS.md §13.11).
//!
//! - [`EtherController::version_command`]: every `Version` command (from `handlers.rs`).
//!   Versions are `.ether` documents at `versions/<id>.ether` in the project folder, written
//!   and listed through `ProjectStore::{write, read, list_dir}` and deleted with
//!   `ProjectStore::remove` (defaulted, v0.3), so native disk and web OPFS both work.
//! - [`EtherController::versions_tick`]: rolling autosave versions (after the autosave
//!   check: a version when the document changed since the last one, at most every
//!   `VERSION_INTERVAL_MS`, pruning to `MAX_AUTOSAVE_VERSIONS`), and the session marker
//!   (`versions/.session`: written on open, removed on a clean close; shared touch in
//!   `project.rs` for open/close).
//! - `ListRecoverable` scans the stored projects for surviving markers whose newest version
//!   is newer than `project.ether`.
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs) and nothing is
//! written.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::versions::VersionCommand;

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
    pub(crate) fn version_command(
        &mut self,
        command: &VersionCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out);
        Err(unsupported(
            "project versions are not implemented yet (project-versions)",
        ))
    }

    /// Called every tick.
    pub(crate) fn versions_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

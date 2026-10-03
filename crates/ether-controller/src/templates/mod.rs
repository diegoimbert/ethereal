//! Project and track templates (v0.3, owned by the `templates` node; file format
//! `ether_model::template`, protocol `ether_protocol::templates`; CONTRACTS.md §13.10).
//!
//! - [`insert`]: `Template::Insert` (document command from `doc::apply`): load the template
//!   (`Library::read` of the user root), import/reference its samples, remap every entity id
//!   to `derive_id(seed, i)`, drop references to tracks outside the template, insert in
//!   order. One undo step.
//! - [`EtherController::template_command`]: the other `Template` commands (from
//!   `handlers.rs`): list/save/rename/delete/set default through the writable user library
//!   (`Library::{write_file, remove_file, rename_file, user_root}`, under
//!   `ether_model::template::TEMPLATES_DIR`), and `NewProject` (like `Project::Create`, then
//!   the template's document with a fresh `ProjectId` and the new name). Factory templates
//!   (if any) are embedded, read-only.
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::TrackId;
use ether_core::protocol::templates::{TemplateCommand, TemplateId};

use crate::doc::DocCtx;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// `Template::Insert`.
pub(crate) fn insert(
    ctx: &mut DocCtx,
    template: &TemplateId,
    seed: TrackId,
    parent: Option<TrackId>,
    before: Option<TrackId>,
) -> CmdResult<()> {
    let _ = (ctx, template, seed, parent, before);
    Err(unsupported("templates are not implemented yet (templates)"))
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Runtime `Template` commands (`Insert` goes through `doc::apply`).
    pub(crate) fn template_command(
        &mut self,
        command: &TemplateCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out);
        Err(unsupported("templates are not implemented yet (templates)"))
    }
}

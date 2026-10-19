//! `Project::Import` when no extension imports other DAWs' projects (`crate::ext`: an
//! extension that does answers the command first): `Unsupported`. `CancelImport` has
//! nothing to cancel.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::BundleSource;

use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Project::Import`: `Unsupported` in this build.
    pub(crate) fn import_command(
        &mut self,
        _new_id: ProjectId,
        _source: &BundleSource,
        _name: Option<&str>,
        _out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        Err(crate::tx::keyed_err(
            ether_core::protocol::ErrorCode::Unsupported,
            crate::text::msg("error.import.unavailable"),
        ))
    }

    /// `Project::CancelImport`: nothing runs.
    pub(crate) fn cancel_import(
        &mut self,
        _id: ProjectId,
        _out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        Ok(ReplyValue::Unit)
    }
}

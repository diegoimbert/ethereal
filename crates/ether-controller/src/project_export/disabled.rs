//! `Project::ExportTo` when no extension exports to other DAWs (`crate::ext`: an extension
//! that does answers the command first): `Unsupported`.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project_export::ProjectExportFormat;

use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices};

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Project::ExportTo`: `Unsupported` in this build.
    pub(crate) fn export_to(
        &mut self,
        _id: ProjectId,
        _format: ProjectExportFormat,
        _path: Option<&str>,
    ) -> CmdResult<ReplyValue> {
        Err(crate::tx::keyed_err(
            ether_core::protocol::ErrorCode::Unsupported,
            crate::text::msg("error.project_export.unavailable"),
        ))
    }
}

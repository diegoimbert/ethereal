//! Uploads from the UI machine (roadmap v2, owned by the `remote-engine` node; see
//! `docs/ROADMAP.md` and `ether_protocol::media` / `ether_protocol::remote`).
//!
//! `Media::{BeginUpload, UploadChunk, CancelUpload}` land here: bytes are staged in the
//! store (`ProjectStore` temp area, never in memory for large files), then
//! `Media::Import { source: Upload }` imports the completed upload like a library file.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::media::MediaCommand;

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
    /// `BeginUpload` / `UploadChunk` / `CancelUpload`.
    pub(crate) fn upload_command(
        &mut self,
        c: &MediaCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, out);
        Err(unsupported(
            "uploads are not supported yet (remote-engine node)",
        ))
    }
}

//! Browser sample preview (owned by the `media-preview` node; see `docs/ROADMAP.md`).
//!
//! `Media::{Preview, StopPreview}` land in [`EtherController::preview_command`]: resolve the
//! `MediaSource` (library file through `Library`, project media through the store), decode
//! and resample it with `ether-media` to the engine rate (bounded work per tick for long
//! files: start playing when the first chunk is ready, or cap preview length), hand it to
//! `EngineBridge::preview` with a new monotonic preview id, and emit
//! `MediaEvent::PreviewStarted`. Stop/replace emit `PreviewEnded { Stopped | Replaced }`
//! right away (the engine never reports them). [`EtherController::preview_tick`] (called
//! every tick) reads `EngineOutputs::preview_ended = Some(id)` and emits
//! `PreviewEnded { Finished }` only if `id` is still the current preview (a late report for a
//! replaced preview is ignored). Every preview gets exactly one `PreviewEnded`.
//! Keep a small cache of recently previewed decodes (auditioning the same sample twice
//! should be instant).

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
    /// `Media::Preview` / `Media::StopPreview`.
    pub(crate) fn preview_command(
        &mut self,
        c: &MediaCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, out);
        Err(unsupported("preview is not available on this host"))
    }

    /// Called every tick (after the engine outputs were polled).
    pub(crate) fn preview_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

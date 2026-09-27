//! Offline export (roadmap v2, owned by the `export` node; see `docs/ROADMAP.md` and
//! `ether_protocol::export`).
//!
//! A job builds an independent `ether_core::offline::OfflineRenderer` at the engine rate:
//! fresh device nodes (`ether_devices::create` for built-ins,
//! `EngineBridge::create_offline_plugin` for plugins; unsupported plugins are bypassed with
//! a warning), the media already decoded by the controller (`media::MediaState`) as
//! `ether_media::InMemorySource`s, and a graph from `compile::compile_graph_with` with
//! `metronome: false` (stems: one pass per track with every other source track muted).
//! Rendering is stepped from [`EtherController::export_tick`] (bounded work per tick, so
//! the controller stays responsive), then normalized/dithered/resampled and encoded
//! (WAV natively in this module; FLAC via the `flacenc` workspace dep). Native hosts write
//! `exports/<name>` through the `ProjectStore`; hosts whose store says so (web/remote) keep
//! the bytes for `Export::ReadChunk`.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::export::ExportCommand;

use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};
use crate::tx::{CmdResult, unsupported};

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Command::Export`.
    pub(crate) fn export_command(
        &mut self,
        c: &ExportCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, out);
        Err(unsupported("export is not implemented yet (export node)"))
    }

    /// Called from every tick: steps the running job, emits `Event::Export`.
    pub(crate) fn export_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

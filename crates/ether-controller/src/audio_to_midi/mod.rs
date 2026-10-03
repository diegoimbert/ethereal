//! Audio to MIDI (v0.3, owned by the `audio-to-midi` node; protocol
//! `ether_protocol::audio_to_midi`, detection DSP in `ether_media::to_midi`; CONTRACTS.md
//! §13.5).
//!
//! - [`EtherController::audio_to_midi_command`]: `AudioToMidi::{Start, Cancel}` (dispatched
//!   from `handlers.rs`). `Start` checks the clip (audio, media loaded), creates the job and
//!   replies `Unit`.
//! - [`EtherController::audio_to_midi_tick`]: runs the job in bounded slices
//!   (`ether_media::to_midi::Detector::step`, ~10 ms of work per tick), emits `Progress`,
//!   and at the end applies the result with `edit_with` (one undo step: track, clip, notes,
//!   optional instrument; ids from the command) and emits `Done`/`Failed`/`Cancelled`.
//!   A job is cancelled when its clip or project goes away.
//!
//! Until the node lands: `Start`/`Cancel` reply `Unsupported` (tests/roadmap_v4.rs).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::audio_to_midi::AudioToMidiCommand;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// The running conversion job, if any.
#[derive(Debug, Default)]
pub(crate) struct AudioToMidiState {
    _private: (),
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn audio_to_midi_command(
        &mut self,
        command: &AudioToMidiCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out, &self.audio_to_midi);
        Err(unsupported(
            "audio to MIDI is not implemented yet (audio-to-midi)",
        ))
    }

    /// Called every tick.
    pub(crate) fn audio_to_midi_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

//! MIDI capture (v0.3, owned by the `capture-midi` node; protocol
//! `ether_protocol::capture`, CONTRACTS.md §13.4).
//!
//! - [`EtherController::capture_input`]: every incoming MIDI message (called from
//!   `midi_learn_tick`, where `EngineBridge::poll_midi_input` is drained) goes into a bounded
//!   ring ([`CaptureState`], `CAPTURE_MAX_SECONDS` / `CAPTURE_MAX_EVENTS`) with its host time
//!   and the transport position (playing) or nothing (stopped).
//! - [`EtherController::capture_command`]: `Capture::{Capture, Clear, Status}` (dispatched
//!   from `handlers.rs`). `Capture` builds the clip with `edit_with` (one undo step: clip,
//!   notes, CC/bend/pressure lanes once `midi-expression` lands, and when stopped with
//!   `adopt_tempo` the tempo and loop).
//! - [`EtherController::capture_tick`]: emits `CaptureEvent::Changed` when availability
//!   changes; clears the buffer when the project changes.
//!
//! Tempo inference (stopped): inter-onset intervals → the tempo in 60..=180 bpm that best
//! fits a beat grid (ties prefer 120), phrase start on the first downbeat; the clip length
//! rounds up to whole bars.
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs) and nothing is
//! buffered.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::capture::CaptureCommand;
use ether_core::protocol::midi_map::MidiInputEvent;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// The capture ring and its availability (runtime, site-local).
#[derive(Debug, Default)]
pub(crate) struct CaptureState {
    _private: (),
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn capture_command(
        &mut self,
        command: &CaptureCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out, &self.capture);
        Err(unsupported(
            "MIDI capture is not implemented yet (capture-midi)",
        ))
    }

    /// One incoming MIDI message (all ports). Placeholder: ignored.
    pub(crate) fn capture_input(&mut self, event: &MidiInputEvent, now: u64) {
        let _ = (event, now);
    }

    /// Called every tick.
    pub(crate) fn capture_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

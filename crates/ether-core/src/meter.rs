//! Engine → controller outputs (meters, playhead, session states, diagnostics).

use ether_protocol::meters::TrackMeter;
use ether_protocol::model::TrackId;
use ether_protocol::session::ClipStateChange;

use crate::transport::PlayheadState;

/// Per-track meter reading pushed by the audio thread (once per block per track, or
/// decimated). Linear amplitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterReading {
    pub track: TrackId,
    pub peak: [f32; 2],
    pub rms: [f32; 2],
}

/// Filled by `EngineHandle::poll` (non-RT). Reuse one instance to avoid reallocations.
#[derive(Debug, Default)]
pub struct EngineOutputs {
    /// Latest playhead.
    pub playhead: Option<PlayheadState>,
    /// Max-held readings since the previous poll, ready for `MeterFrame`.
    pub meters: Vec<TrackMeter>,
    /// DSP load 0..=1 (the host measures time; the engine reports its share if known).
    pub cpu_load: f32,
    /// Session clip state transitions (from `session`).
    pub session: Vec<ClipStateChange>,
    /// Some node's event buffer overflowed since the last poll.
    pub event_overflow: bool,
    /// Audio source underruns since the last poll.
    pub underruns: u32,
}

impl EngineOutputs {
    pub fn clear(&mut self) {
        self.playhead = None;
        self.meters.clear();
        self.session.clear();
        self.event_overflow = false;
        self.underruns = 0;
    }
}

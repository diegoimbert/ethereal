//! Metronome click (roadmap v2, owned by the `tempo-metronome` node; see
//! `docs/ROADMAP.md`).
//!
//! Hook point (already wired, base-17): the engine owns one [`Metronome`] and, after the
//! master bus has been written to the hardware outputs of a sub-block, calls
//! [`Metronome::render`] once with that sub-block's `TransportInfo`, and
//! [`Metronome::reset`] on transport jumps. This node only edits this file.
//! Clicks are synthesized (no samples), sample-accurate on every beat boundary inside the
//! sub-block (accent on bar starts when `desc.accent`), while playing with
//! `RenderGraphDesc::metronome` on, and during a recording count-in: `info.recording` with
//! `info.position < desc.count_in_end` (the controller's record pre-roll, see
//! `ether_controller::recording`), whatever `metronome` says. RT rules apply: no
//! allocation after `new`.

use crate::graph::MetronomeDesc;
use crate::transport::TransportInfo;

/// Click generator state (current click phase/envelope).
#[derive(Debug, Default)]
pub struct Metronome {
    sample_rate: f32,
    /// Samples left of the click currently sounding.
    remaining: u32,
}

impl Metronome {
    /// Non-RT.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            remaining: 0,
        }
    }

    /// RT. Add the click for the sub-block described by `info` (`frames` samples) into
    /// `out[ch][offset..offset + frames]` (planar hardware outputs; channels may be shorter:
    /// clamp). `enabled` = `RenderGraphDesc::metronome`. Called by `engine.rs` once per
    /// sub-block (already wired). Placeholder: renders nothing until the `tempo-metronome`
    /// node implements it.
    pub fn render(
        &mut self,
        desc: &MetronomeDesc,
        enabled: bool,
        info: &TransportInfo,
        offset: usize,
        frames: usize,
        out: &mut [&mut [f32]],
    ) {
        let _ = (desc, enabled, info, offset, frames, out, self.sample_rate);
        self.remaining = 0;
    }

    /// RT. Silence any sounding click (transport jumps).
    pub fn reset(&mut self) {
        self.remaining = 0;
    }
}

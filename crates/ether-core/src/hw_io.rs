//! Hardware I/O inside the graph (v0.3, contracts-4; owned by the `external-instrument`
//! node; CONTRACTS.md §13.7): External Instrument (MIDI out + audio return) and External
//! Audio Effect (audio send + return).
//!
//! The controller compiles every external device of a track into a [`HwIoDesc`]
//! (`TrackDesc::hw_io`, keyed by the device's chain node). The engine (shared touch of
//! `external-instrument` in `engine.rs`):
//! 1. before the track jobs, copies each desc's `audio_return` hardware input channels
//!    (from `Engine::process`'s `inputs`, the same buffers recording/monitoring read) into
//!    a per-node return buffer ([`HwIoRt::gather_returns`]);
//! 2. the external device node outputs that buffer (instrument) or mixes it with its dry
//!    input by its `MIX` param (effect); the effect's input is copied to a per-node send
//!    buffer ([`HwIoRt::capture_send`]);
//! 3. after the jobs, adds each send buffer to its `audio_send` hardware output channels
//!    ([`HwIoRt::write_sends`]), after the master bus was written (the click and the
//!    preview are added the same way);
//! 4. MIDI from an External Instrument's input events goes to a lock-free ring of
//!    [`HwMidiEvent`]s, drained by the host (`EngineHandle`, then the native MIDI output
//!    thread, which maps `node` to the device's `midi_out` port with sample-accurate
//!    timestamps). The web has no hardware MIDI out (devices stay silent there).
//!
//! PDC: the device node reports its `LATENCY` param (ms → samples) as `Node::latency`, so the
//! returned audio is aligned with the mix. Measuring (`External::MeasureLatency`) sends a
//! click on the send/MIDI path and times its arrival on the return (`external-instrument`).
//! RT rules: buffers are allocated at compile; the audio thread only copies and adds.

use ether_protocol::model::ExternalRouting;
use serde::{Deserialize, Serialize};

use crate::node::NodeKey;

/// One external device of a track, compiled for the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HwIoDesc {
    /// The device's chain node.
    pub node: NodeKey,
    /// Hardware routing (ports and channels) as stored in the document.
    pub routing: ExternalRouting,
}

/// A MIDI message an External Instrument sends to hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwMidiEvent {
    pub node: NodeKey,
    /// Engine sample time of the message.
    pub frame: u64,
    pub data: [u8; 3],
}

/// Engine-side state (return/send buffers). Pre-wired as a stub: `external-instrument`
/// implements it.
#[derive(Debug, Default)]
pub struct HwIoRt {
    _private: (),
}

impl HwIoRt {
    /// Non-RT: allocate buffers for a new snapshot's descs.
    pub fn prepare(&mut self, descs: &[HwIoDesc], max_block: usize) {
        let _ = (descs, max_block);
    }

    /// RT: copy hardware inputs into the return buffers. Stub: nothing.
    pub fn gather_returns(&mut self, inputs: &[&[f32]], frames: usize) {
        let _ = (inputs, frames);
    }

    /// RT: an effect's input for the hardware send. Stub: nothing.
    pub fn capture_send(&mut self, node: NodeKey, input: &[&[f32]], frames: usize) {
        let _ = (node, input, frames);
    }

    /// RT: add the send buffers to the hardware outputs. Stub: nothing.
    pub fn write_sends(&mut self, outputs: &mut [&mut [f32]], frames: usize) {
        let _ = (outputs, frames);
    }
}

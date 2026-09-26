//! Processing nodes: the [`Node`] trait (anything in the graph) and [`Device`] (a
//! user-facing node with parameters and a descriptor: built-in devices, plugins).

use ether_protocol::devices::DeviceDescriptor;
use ether_protocol::model::ParamId;
use serde::{Deserialize, Serialize};

use crate::buffer::AudioBuffers;
use crate::config::PrepareConfig;
use crate::event::{EventBuffer, ProcessEvent};
use crate::transport::TransportInfo;

/// Handle to a node inserted into the engine (slot index + generation, so stale keys are
/// detected). Allocated by [`crate::EngineHandle::add_node`]; referenced from
/// [`crate::RenderGraphDesc`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeKey {
    pub index: u32,
    pub generation: u32,
}

/// Everything a node sees during one `process` call.
pub struct ProcessContext<'a> {
    pub sample_rate: f32,
    /// Number of frames in this (sub-)block, `<= max_block_size`. The scheduler splits
    /// blocks at loop points and tempo-segment boundaries, so transport info is linear
    /// within a call.
    pub frames: usize,
    pub transport: &'a TransportInfo,
    /// Input events for this node, sorted by `offset`, all `< frames`.
    pub events: &'a [ProcessEvent],
    /// Output events (note effects, plugins' MIDI out). Pre-allocated; may overflow.
    pub out_events: &'a mut EventBuffer,
}

/// Return value of [`Node::process`] (lets the engine skip silent nodes later).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Outputs contain signal.
    Continue,
    /// Outputs are silent and will stay silent until new input/events arrive.
    Silent,
}

/// A real-time audio processor.
///
/// Lifecycle: constructed + [`Node::prepare`]d on a non-RT thread, then moved to the audio
/// thread (hence `Send`); `process`/`reset` run on the audio thread and must obey the RT
/// rules (no alloc, locks, I/O, unbounded work); finally sent back and dropped on the GC
/// thread.
pub trait Node: Send {
    /// Non-RT. Allocate buffers for `config`. Called before the node's first `process` and
    /// again if the config changes (after being taken out of the graph).
    fn prepare(&mut self, config: &PrepareConfig);

    /// RT-safe. Clear internal state (voices, delay lines) e.g. on transport jumps.
    fn reset(&mut self);

    /// RT-safe. Render `ctx.frames` samples.
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus;

    /// Latency in samples introduced by this node (for plugin delay compensation). May
    /// change; the controller re-publishes the graph when it does.
    fn latency(&self) -> u32 {
        0
    }

    /// Main input / output channel counts.
    fn channels(&self) -> (u16, u16) {
        (2, 2)
    }
}

/// A node with parameters and a descriptor (built-in devices, plugins).
///
/// Parameter values are *plain* (see `ParamInfo`). During playback all changes arrive as
/// `EventKind::Param` events in `ProcessContext::events` (sample-accurate, from UI or
/// automation); [`Device::set_param`] is for initial state before insertion (non-RT) or
/// RT-safe immediate sets.
pub trait Device: Node {
    /// Non-RT. Static description incl. parameter list.
    fn descriptor(&self) -> DeviceDescriptor;

    /// Current plain value (for UI readback / state save).
    fn param(&self, id: ParamId) -> Option<f64>;

    /// RT-safe. Set immediately (no smoothing).
    fn set_param(&mut self, id: ParamId, value: f64);
}

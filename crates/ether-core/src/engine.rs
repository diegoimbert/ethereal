//! The engine and its control/GC halves.

use std::sync::Arc;

use ether_protocol::model::MediaId;

use crate::config::EngineConfig;
use crate::graph::{CompileError, RenderGraphDesc};
use crate::media::AudioSource;
use crate::meter::EngineOutputs;
use crate::node::{Node, NodeKey};
use crate::param::ParamChange;
use crate::session::SessionControl;
use crate::transport::{PlayheadState, TransportControl};

/// The three halves returned by [`create`].
pub struct EngineParts {
    pub engine: Engine,
    pub handle: EngineHandle,
    pub gc: GarbageCollector,
}

/// Create an engine. Non-RT (allocates every ring and table up front).
pub fn create(config: EngineConfig) -> EngineParts {
    let _ = config;
    todo!("core node")
}

/// Audio-thread half. Owns the node table and the current snapshot.
pub struct Engine {
    _private: (),
}

impl Engine {
    /// **RT.** Render one block. `inputs`/`outputs` are planar, each `frames` long
    /// (`frames <= max_block_size`; channel counts as configured, extra channels ignored).
    ///
    /// Per block: drain control ring (node inserts/removals, snapshot swap, transport,
    /// session), drain param ring, render (splitting at loop/tempo boundaries), write
    /// meters/playhead to the output ring, push retired objects to the GC ring. Never
    /// allocates, locks or blocks; bounded by graph size.
    pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) {
        let _ = (inputs, outputs, frames);
        todo!("core node")
    }

    pub fn config(&self) -> &EngineConfig {
        todo!("core node")
    }
}

/// Controller-thread half. All methods are non-blocking; `Err(EngineError::QueueFull)`
/// means the audio thread isn't draining (stalled or not started) and the caller should
/// retry later.
pub struct EngineHandle {
    _private: (),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("engine queue full")]
    QueueFull,
    #[error("node table full")]
    TooManyNodes,
    #[error("stale or unknown node key {0:?}")]
    UnknownNode(NodeKey),
    #[error(transparent)]
    Compile(#[from] CompileError),
}

impl EngineHandle {
    /// Prepare `node` (calls `Node::prepare` here, off the audio thread) and send it to the
    /// engine. The key is valid immediately for use in a `RenderGraphDesc`.
    pub fn add_node(&mut self, node: Box<dyn Node>) -> Result<NodeKey, EngineError> {
        let _ = node;
        todo!("core node")
    }

    /// Remove a node; it is returned through the GC ring and dropped there. Publish a graph
    /// that no longer references it first (or in the same batch).
    pub fn remove_node(&mut self, key: NodeKey) -> Result<(), EngineError> {
        let _ = key;
        todo!("core node")
    }

    /// Register (or replace) the audio source for `media` (used by audio clips/samplers).
    pub fn add_source(
        &mut self,
        media: MediaId,
        source: Arc<dyn AudioSource>,
    ) -> Result<(), EngineError> {
        let _ = (media, source);
        todo!("core node")
    }

    pub fn remove_source(&mut self, media: MediaId) -> Result<(), EngineError> {
        let _ = media;
        todo!("core node")
    }

    /// Compile `desc` (here, non-RT) and send the snapshot for an atomic swap at the next
    /// block boundary. Returns compile errors without touching the running snapshot.
    pub fn publish(&mut self, desc: RenderGraphDesc) -> Result<(), EngineError> {
        let _ = desc;
        todo!("core node")
    }

    /// Live parameter change (fader, knob). Lock-free; applied next block.
    pub fn set_param(&mut self, change: ParamChange) -> Result<(), EngineError> {
        let _ = change;
        todo!("core node")
    }

    pub fn transport(&mut self, control: TransportControl) -> Result<(), EngineError> {
        let _ = control;
        todo!("core node")
    }

    /// Session-view launch/stop (see [`crate::session`]).
    pub fn session(&mut self, control: SessionControl) -> Result<(), EngineError> {
        let _ = control;
        todo!("core node")
    }

    /// Drain engine outputs (meters, playhead, session state changes, diagnostics) into
    /// `out` (cleared first). Call at UI rate (~30-60 Hz).
    pub fn poll(&mut self, out: &mut EngineOutputs) {
        let _ = out;
        todo!("core node")
    }

    /// Latest playhead published by the audio thread.
    pub fn playhead(&self) -> PlayheadState {
        todo!("core node")
    }
}

/// Receives objects retired by the audio thread (old snapshots, removed nodes, replaced
/// sources) and drops them off the audio thread. `Send`: hosts usually move it to a
/// low-priority GC thread and call [`GarbageCollector::collect`] every ~50 ms.
pub struct GarbageCollector {
    _private: (),
}

impl GarbageCollector {
    /// Drop everything retired so far; returns the number of objects dropped.
    pub fn collect(&mut self) -> usize {
        todo!("core node")
    }
}

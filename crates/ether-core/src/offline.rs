//! Offline rendering (roadmap v2, used by the export; see CONTRACTS.md §11.1).
//!
//! An [`OfflineRenderer`] is a private engine that renders as fast as the caller pulls,
//! with no device and no threads: the caller adds nodes and sources through
//! [`OfflineRenderer::handle`] exactly as with a live engine, publishes the graph with
//! [`OfflineRenderer::publish`], then calls
//! [`OfflineRenderer::start`] and [`OfflineRenderer::render`] in a loop. Everything runs
//! on the calling thread (the controller thread / Worker), so it works on native and web.
//!
//! Nodes are fresh instances (built from the document like the live ones), never the live
//! engine's nodes, so exporting doesn't disturb playback.
//!
//! The click is never rendered: [`OfflineRenderer::publish`] forces `metronome` and any
//! count-in off (publishing through the handle directly would render it like a live engine).

use ether_protocol::model::{Beats, TrackId};

use crate::config::EngineConfig;
use crate::engine::{Engine, EngineError, EngineHandle, GarbageCollector, create};
use crate::graph::RenderGraphDesc;
use crate::meter::EngineOutputs;
use crate::transport::TransportControl;

/// Block size of offline renders (`EngineConfig::max_block_size` of the export engine).
/// Offline plugin instances are activated with it, so a host never has to know the live
/// engine's block size to build one.
pub const OFFLINE_MAX_BLOCK: usize = 512;

pub struct OfflineRenderer {
    engine: Engine,
    handle: EngineHandle,
    gc: GarbageCollector,
    /// Drained engine outputs (meters/playhead), so the output ring never fills.
    outputs: EngineOutputs,
    /// Frames rendered since `start`.
    rendered: u64,
    /// Accumulated since `start`: some node's events overflowed / source underruns.
    event_overflow: bool,
    underruns: u32,
    /// Tracks whose post-fader signal is captured ([`OfflineRenderer::capture_tracks`]).
    capture: Vec<TrackId>,
}

/// One captured track's block, see [`OfflineRenderer::captured`].
pub struct CapturedBlock<'a> {
    pub track: TrackId,
    /// The track's post-fader signal (VCA included), before the mute/solo gate and the PDC
    /// output delay, of the last [`OfflineRenderer::render`] call (`frames` long).
    pub channels: [&'a [f32]; 2],
    /// Samples this signal is late relative to the timeline (the track's own latency, at
    /// most [`OfflineRenderer::latency`]). `None`: the track is not in the graph (silence).
    pub latency: Option<u32>,
}

impl OfflineRenderer {
    /// Non-RT. A new offline engine (`config.input_channels` is ignored: no inputs).
    pub fn new(config: EngineConfig) -> Self {
        let parts = create(config);
        Self {
            engine: parts.engine,
            handle: parts.handle,
            gc: parts.gc,
            outputs: EngineOutputs::default(),
            rendered: 0,
            event_overflow: false,
            underruns: 0,
            capture: Vec::new(),
        }
    }

    /// Also capture these tracks' post-fader signal (before the mute/solo gate, so a muted
    /// track is captured too; solo is ignored), read with [`OfflineRenderer::captured`]
    /// after each [`OfflineRenderer::render`]. Call before [`OfflineRenderer::publish`].
    /// An empty list costs nothing.
    pub fn capture_tracks(&mut self, tracks: &[TrackId]) {
        self.capture = tracks.to_vec();
        self.engine.set_capture(tracks);
    }

    /// The captured tracks' signal of the last `render(frames, ..)`; valid when `frames`
    /// is at most `max_block_size` (one engine block).
    pub fn captured(&self, frames: usize) -> impl Iterator<Item = CapturedBlock<'_>> {
        self.engine.captured().iter().map(move |s| CapturedBlock {
            track: s.id,
            channels: [&s.buf[0][..frames], &s.buf[1][..frames]],
            latency: s.latency,
        })
    }

    /// Add nodes/sources here, like on a live engine. Publish with
    /// [`OfflineRenderer::publish`] (which silences the metronome), not
    /// `handle().publish`.
    pub fn handle(&mut self) -> &mut EngineHandle {
        &mut self.handle
    }

    /// Publish the graph to render, with the metronome and any count-in click forced off
    /// (exports never contain the click).
    pub fn publish(&mut self, mut desc: RenderGraphDesc) -> Result<(), EngineError> {
        desc.metronome = false;
        desc.click.count_in_end = None;
        self.handle.publish_capturing(desc, &self.capture)
    }

    pub fn sample_rate(&self) -> u32 {
        self.engine.config().sample_rate
    }

    /// Locate to `position` and start playing (applied at the next `render`).
    pub fn start(&mut self, position: Beats) -> Result<(), EngineError> {
        self.handle
            .transport(TransportControl::Locate { position })?;
        self.handle.transport(TransportControl::Play)?;
        self.rendered = 0;
        self.event_overflow = false;
        self.underruns = 0;
        Ok(())
    }

    /// Render `frames` frames into `out` (planar; every channel at least `frames` long),
    /// in engine-sized blocks. Retired objects are collected as it goes.
    pub fn render(&mut self, frames: usize, out: &mut [&mut [f32]]) {
        let block = self.engine.config().max_block_size.max(1);
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            let mut chunk: Vec<&mut [f32]> =
                out.iter_mut().map(|ch| &mut ch[done..done + n]).collect();
            self.engine.process(&[], &mut chunk, n);
            done += n;
            self.gc.collect();
            self.handle.poll(&mut self.outputs);
            self.event_overflow |= self.outputs.event_overflow;
            self.underruns += self.outputs.underruns;
        }
        self.rendered += frames as u64;
    }

    /// Total graph latency (samples) of the last published graph: the audio for timeline
    /// position `p` comes out `latency()` frames late, so an export starting at `p` renders
    /// and drops that many leading frames (and renders as many extra at the end).
    pub fn latency(&self) -> u32 {
        self.handle.latency()
    }

    /// Since the last `start`: `(some node's event buffer overflowed, source underruns)`.
    /// An export should warn when either is set.
    pub fn diagnostics(&self) -> (bool, u32) {
        (self.event_overflow, self.underruns)
    }

    /// Frames rendered since the last [`OfflineRenderer::start`].
    pub fn rendered(&self) -> u64 {
        self.rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{Node, NodeData, ProcessContext, ProcessStatus};
    use crate::{AudioBuffers, PrepareConfig};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Accepts `u32` data and publishes it.
    struct DataNode(Arc<AtomicU32>);
    impl Node for DataNode {
        fn prepare(&mut self, _: &PrepareConfig) {}
        fn reset(&mut self) {}
        fn process(
            &mut self,
            _: &mut ProcessContext<'_>,
            a: &mut AudioBuffers<'_, '_>,
        ) -> ProcessStatus {
            a.clear_outputs();
            ProcessStatus::Silent
        }
        fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
            match data.downcast::<u32>() {
                Ok(v) => {
                    self.0.store(*v, Ordering::Relaxed);
                    Some(v)
                }
                Err(other) => Some(other),
            }
        }
    }

    #[test]
    fn node_data_reaches_the_live_node() {
        let seen = Arc::new(AtomicU32::new(0));
        let mut r = OfflineRenderer::new(EngineConfig::default());
        let key = r
            .handle()
            .add_node(Box::new(DataNode(seen.clone())))
            .unwrap();
        r.handle().set_node_data(key, Box::new(7u32)).unwrap();
        let mut l = vec![0.0f32; 64];
        let mut rr = vec![0.0f32; 64];
        r.render(64, &mut [&mut l, &mut rr]);
        assert_eq!(seen.load(Ordering::Relaxed), 7);
        assert_eq!(r.latency(), 0);
        assert_eq!(r.diagnostics(), (false, 0));
    }

    #[test]
    fn renders_silence_for_an_empty_graph() {
        let mut r = OfflineRenderer::new(EngineConfig {
            max_block_size: 64,
            ..Default::default()
        });
        r.start(Beats(4.0)).unwrap();
        let mut l = vec![1.0f32; 200];
        let mut rr = vec![1.0f32; 200];
        r.render(200, &mut [&mut l, &mut rr]);
        assert!(l.iter().chain(&rr).all(|&s| s == 0.0));
        assert_eq!(r.rendered(), 200);
        assert!(r.handle().playhead().playing);
        assert!(r.handle().playhead().position.0 > 4.0);
    }
}

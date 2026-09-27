//! Offline rendering (roadmap v2, used by the `export` node; see `docs/ROADMAP.md`).
//!
//! An [`OfflineRenderer`] is a private engine that renders as fast as the caller pulls,
//! with no device and no threads: the caller adds nodes and sources and publishes a graph
//! through [`OfflineRenderer::handle`] exactly as with a live engine, then calls
//! [`OfflineRenderer::start`] and [`OfflineRenderer::render`] in a loop. Everything runs
//! on the calling thread (the controller thread / Worker), so it works on native and web.
//!
//! Nodes are fresh instances (built from the document like the live ones), never the live
//! engine's nodes, so exporting doesn't disturb playback. The metronome is never rendered
//! offline (the export desc has `metronome: false`).

use ether_protocol::model::Beats;

use crate::config::EngineConfig;
use crate::engine::{Engine, EngineError, EngineHandle, GarbageCollector, create};
use crate::transport::TransportControl;

pub struct OfflineRenderer {
    engine: Engine,
    handle: EngineHandle,
    gc: GarbageCollector,
    /// Frames rendered since `start`.
    rendered: u64,
}

impl OfflineRenderer {
    /// Non-RT. A new offline engine (`config.input_channels` is ignored: no inputs).
    pub fn new(config: EngineConfig) -> Self {
        let parts = create(config);
        Self {
            engine: parts.engine,
            handle: parts.handle,
            gc: parts.gc,
            rendered: 0,
        }
    }

    /// Add nodes/sources and publish the graph here, like on a live engine.
    pub fn handle(&mut self) -> &mut EngineHandle {
        &mut self.handle
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
        Ok(())
    }

    /// Render `frames` frames into `out` (planar; every channel at least `frames` long),
    /// in engine-sized blocks. Retired objects are collected as it goes.
    pub fn render(&mut self, frames: usize, out: &mut [&mut [f32]]) {
        let block = self.engine.config().max_block_size.max(1);
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            let mut chunk: Vec<&mut [f32]> = out
                .iter_mut()
                .map(|ch| &mut ch[done..done + n])
                .collect();
            self.engine.process(&[], &mut chunk, n);
            done += n;
            self.gc.collect();
        }
        self.rendered += frames as u64;
    }

    /// Frames rendered since the last [`OfflineRenderer::start`].
    pub fn rendered(&self) -> u64 {
        self.rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

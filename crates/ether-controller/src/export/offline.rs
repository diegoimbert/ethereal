//! Reusable pieces of an offline render job, shared by `export` and `freeze` (bounce,
//! consolidate): media loading, device-node setup on a private [`OfflineRenderer`] and the
//! block-by-block capture that drops the graph latency. Each `step` is one bounded unit of
//! work (see the module docs of `export`, "Bounded ticks").

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use ether_core::offline::{OFFLINE_MAX_BLOCK, OfflineRenderer};
use ether_core::protocol::model::*;
use ether_core::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};
use ether_core::{AudioSource, EngineConfig, NodeKey, RenderGraphDesc};
use ether_media::{DecodedAudio, InMemorySource};

use super::job::UNIT_BUILTINS;
pub(crate) use super::job::UNIT_FRAMES;
use crate::compile::{CompileContext, compile_graph_with};
use crate::engine::EngineState;
use crate::media::{IncrementalDecoder, IncrementalResampler, extension_of};
use crate::store::{Library, ProjectStore};
use crate::{BridgeError, EngineBridge};

pub(crate) fn engine_err(e: ether_core::EngineError) -> String {
    format!("offline engine: {e}")
}

/// The project's tempo map for the engine (beats ↔ seconds of a render).
pub(crate) fn tempo_rt(p: &Project) -> TempoMapRt {
    let map = p.tempo_map();
    let tempo: Vec<TempoPointDesc> = map
        .tempo
        .iter()
        .map(|t| TempoPointDesc {
            beat: t.time.0,
            bpm: t.bpm,
            curve: t.curve,
        })
        .collect();
    let sigs: Vec<TimeSignatureDesc> = map
        .signatures
        .iter()
        .map(|s| TimeSignatureDesc {
            beat: s.time.0,
            signature: s.signature,
        })
        .collect();
    TempoMapRt::compile(&tempo, &sigs)
}

enum MediaLoad {
    Decode(MediaRef, Box<IncrementalDecoder>),
    Resample(Box<IncrementalResampler>, MediaId),
}

/// Decodes and resamples media to the engine rate with the live pipeline's code
/// (bit-identical sources), one unit at a time.
pub(crate) struct MediaLoader {
    project: ProjectId,
    engine_rate: u32,
    queue: VecDeque<MediaRef>,
    load: Option<MediaLoad>,
    pub media: BTreeMap<MediaId, Arc<DecodedAudio>>,
}

impl MediaLoader {
    pub fn new(project: ProjectId, media: Vec<MediaRef>, engine_rate: u32) -> Self {
        Self {
            project,
            engine_rate,
            queue: media.into(),
            load: None,
            media: BTreeMap::new(),
        }
    }

    /// Decode/resample the next media by one unit. `Ok(true)` once everything is loaded.
    pub fn step<S: ProjectStore, L: Library>(
        &mut self,
        store: &mut S,
        library: &mut L,
        frames: &mut usize,
    ) -> Result<bool, String> {
        let load = match self.load.take() {
            Some(l) => l,
            None => {
                let Some(m) = self.queue.pop_front() else {
                    return Ok(true);
                };
                // External references resolve like live playback (`media-references`).
                let bytes = crate::media::read_media_bytes(store, library, self.project, &m)
                    .map_err(|e| format!("could not read \"{}\": {e}", m.name))?;
                let dec = IncrementalDecoder::new(bytes.into(), extension_of(&m.file))
                    .map_err(|e| format!("could not decode \"{}\": {e}", m.name))?;
                MediaLoad::Decode(m, Box::new(dec))
            }
        };
        *frames = UNIT_FRAMES;
        match load {
            MediaLoad::Decode(m, mut dec) => {
                let done = dec
                    .step(UNIT_FRAMES)
                    .map_err(|e| format!("could not decode \"{}\": {e}", m.name))?;
                if !done {
                    self.load = Some(MediaLoad::Decode(m, dec));
                    return Ok(false);
                }
                let mut audio = dec
                    .finish()
                    .map_err(|e| format!("could not decode \"{}\": {e}", m.name))?;
                // Same sizing as the live media pipeline (the document is the reference).
                if m.frames != 0 {
                    for ch in &mut audio.channels {
                        ch.resize(m.frames as usize, 0.0);
                    }
                }
                let r = IncrementalResampler::new(Arc::new(audio), self.engine_rate)
                    .map_err(|e| format!("could not resample \"{}\": {e}", m.name))?;
                self.load = Some(MediaLoad::Resample(Box::new(r), m.id));
                Ok(false)
            }
            MediaLoad::Resample(mut r, id) => {
                if r.step(UNIT_FRAMES).map_err(|e| e.to_string())? {
                    self.media.insert(id, Arc::new(r.finish()));
                } else {
                    self.load = Some(MediaLoad::Resample(r, id));
                }
                Ok(false)
            }
        }
    }
}

struct Resolver(std::collections::HashMap<MediaId, Arc<dyn AudioSource>>);

impl ether_devices::SampleResolver for Resolver {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

/// A fresh offline engine with the media registered; device nodes are created a few per
/// unit by [`NodeSetup::step`], then [`NodeSetup::finish`] compiles and publishes the graph.
pub(crate) struct NodeSetup {
    renderer: OfflineRenderer,
    sources: Resolver,
    queue: VecDeque<DeviceId>,
    nodes: BTreeMap<DeviceId, NodeKey>,
    /// Prefix of user-facing warnings ("export", "freeze", ...).
    what: &'static str,
}

impl NodeSetup {
    /// `devices`: the devices to instantiate (in creation order).
    pub fn new(
        p: &Project,
        media: &BTreeMap<MediaId, Arc<DecodedAudio>>,
        devices: Vec<DeviceId>,
        engine_rate: u32,
        what: &'static str,
    ) -> Result<Self, String> {
        let n_items = p.devices.len() + media.len();
        let mut renderer = OfflineRenderer::new(EngineConfig {
            sample_rate: engine_rate,
            max_block_size: OFFLINE_MAX_BLOCK,
            input_channels: 0,
            output_channels: 2,
            max_nodes: (n_items + 64).max(EngineConfig::default().max_nodes),
            control_queue_capacity: 2 * n_items + 256,
            ..EngineConfig::default()
        });
        let mut sources = std::collections::HashMap::new();
        for (id, audio) in media {
            let src: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio.clone()));
            renderer
                .handle()
                .add_source(*id, src.clone())
                .map_err(engine_err)?;
            sources.insert(*id, src);
        }
        Ok(Self {
            renderer,
            sources: Resolver(sources),
            queue: devices.into(),
            nodes: BTreeMap::new(),
            what,
        })
    }

    /// Create a few device nodes (up to [`UNIT_BUILTINS`] built-ins, or one plugin).
    /// `Ok(true)` once every node exists and this unit created none (publishing is its own
    /// unit).
    #[allow(clippy::too_many_arguments)]
    pub fn step<B: EngineBridge>(
        &mut self,
        p: &Project,
        bridge: &mut B,
        engine_rate: u32,
        warnings: &mut Vec<String>,
        devices: &mut usize,
        plugins: &mut usize,
    ) -> Result<bool, String> {
        while let Some(&id) = self.queue.front() {
            let Some(d) = p.devices.get(&id) else {
                self.queue.pop_front();
                continue;
            };
            let is_plugin = matches!(d.kind, DeviceKind::Plugin { .. });
            if *devices >= UNIT_BUILTINS || (is_plugin && *devices > 0) {
                return Ok(false);
            }
            self.queue.pop_front();
            let node: Box<dyn ether_core::Node> = match &d.kind {
                DeviceKind::Builtin { device } => {
                    let mut node = ether_devices::create(device, &self.sources);
                    for (id, v) in &d.params {
                        node.set_param(*id, *v);
                    }
                    node
                }
                DeviceKind::Plugin { plugin } => {
                    let state = match bridge.plugin_state(d.id) {
                        Ok(Some(live)) => Some(live),
                        Ok(None) => plugin.state.clone(),
                        Err(e) => {
                            warnings.push(format!(
                                "{}: could not read the current state of \"{}\" ({e}); \
                                 rendering it with its last saved state",
                                self.what, d.name
                            ));
                            plugin.state.clone()
                        }
                    };
                    *plugins += 1;
                    bridge
                        .create_offline_plugin(d.id, plugin, state.as_ref(), engine_rate)
                        .map_err(|e| {
                            let why = match e {
                                BridgeError::Unsupported(m) | BridgeError::Other(m) => m,
                                other => other.to_string(),
                            };
                            format!(
                                "plugin \"{}\" on this host can't be rendered offline: {why}",
                                d.name
                            )
                        })?
                }
            };
            *devices += 1;
            let key = self.renderer.handle().add_node(node).map_err(engine_err)?;
            self.nodes.insert(d.id, key);
            if is_plugin {
                return Ok(false);
            }
        }
        Ok(*devices == 0)
    }

    /// Compile the document with the created nodes, let `edit` rewrite the graph (after
    /// [`super::job::offline_overrides`]), publish it and start the transport at `start`.
    pub fn finish(
        self,
        p: &Project,
        engine: &EngineState,
        start: Beats,
        edit: impl FnOnce(&mut RenderGraphDesc),
    ) -> Result<OfflineRenderer, String> {
        let Self {
            mut renderer,
            nodes,
            ..
        } = self;
        let node_of = |d: DeviceId| nodes.get(&d).copied();
        let descriptors = |d: &Device| engine.descriptor(d);
        let mut desc = compile_graph_with(
            p,
            &CompileContext {
                nodes: &node_of,
                descriptors: &descriptors,
                armed: &|_| false,
                version: 1,
            },
        );
        super::job::offline_overrides(&mut desc);
        edit(&mut desc);
        renderer.publish(desc).map_err(engine_err)?;
        renderer.start(start).map_err(engine_err)?;
        Ok(renderer)
    }
}

/// Renders a started [`OfflineRenderer`] block by block, dropping the graph latency first.
pub(crate) struct Capture {
    renderer: OfflineRenderer,
    /// Leading frames still to drop (graph latency).
    skip: usize,
    /// Frames still to keep.
    remaining: usize,
    total: usize,
    out: Vec<Vec<f32>>,
    scratch: Vec<Vec<f32>>,
}

impl Capture {
    /// Keep `total` frames after the graph latency.
    pub fn new(renderer: OfflineRenderer, total: usize) -> Self {
        let latency = renderer.latency() as usize;
        Self {
            renderer,
            skip: latency,
            remaining: total,
            total,
            out: (0..2).map(|_| Vec::with_capacity(total)).collect(),
            scratch: vec![Vec::new(); 2],
        }
    }

    /// Render one engine block. `true` once every frame is captured.
    pub fn step(&mut self, frames: &mut usize) -> bool {
        let n = OFFLINE_MAX_BLOCK.min(self.skip + self.remaining);
        for ch in &mut self.scratch {
            ch.resize(n, 0.0);
        }
        {
            let mut outs: Vec<&mut [f32]> = self.scratch.iter_mut().map(|c| &mut c[..n]).collect();
            self.renderer.render(n, &mut outs);
        }
        *frames = n;
        let drop = self.skip.min(n);
        self.skip -= drop;
        let keep = (n - drop).min(self.remaining);
        for (o, s) in self.out.iter_mut().zip(&self.scratch) {
            o.extend_from_slice(&s[drop..drop + keep]);
        }
        self.remaining -= keep;
        self.remaining == 0 && self.skip == 0
    }

    /// Captured so far (frames after the latency).
    pub fn captured(&self) -> usize {
        self.total - self.remaining
    }

    pub fn total(&self) -> usize {
        self.total
    }

    /// Extend the capture by `frames` (tails rendered until silence).
    pub fn extend(&mut self, frames: usize) {
        self.remaining += frames;
        self.total += frames;
    }

    /// The captured channels.
    pub fn channels(&self) -> &[Vec<f32>] {
        &self.out
    }

    /// `Some(message)` when the render may be incomplete (event overflow, underruns).
    pub fn problem(&self) -> Option<&'static str> {
        let (overflow, underruns) = self.renderer.diagnostics();
        if overflow {
            Some("too many events in a block")
        } else if underruns > 0 {
            Some("audio sources could not keep up")
        } else {
            None
        }
    }

    /// The captured channels (drops the offline engine and its nodes).
    pub fn into_channels(self) -> Vec<Vec<f32>> {
        self.out
    }
}

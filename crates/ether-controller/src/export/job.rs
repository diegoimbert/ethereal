//! One export job: load media, set up each pass (mix or one stem) on a private
//! [`OfflineRenderer`] a few nodes at a time, render it block by block, then
//! resample/normalize/encode and deliver each file. Every step is a bounded unit of work.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use ether_core::graph::ResolvedTarget;
use ether_core::offline::{OFFLINE_MAX_BLOCK, OfflineRenderer};
use ether_core::protocol::export::{
    ExportDownload, ExportJobId, ExportMode, ExportRange, ExportRequest,
};
use ether_core::protocol::model::*;
use ether_core::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};
use ether_core::{AudioSource, EngineConfig, NodeKey, RenderGraphDesc};
use ether_media::{DecodedAudio, InMemorySource};

use super::encode::{self, Encoder, PeakScan, normalize_gain};
use crate::compile::{CompileContext, compile_graph_with};
use crate::engine::EngineState;
use crate::media::{IncrementalDecoder, IncrementalResampler, extension_of};
use crate::store::ProjectStore;
use crate::{BridgeError, EngineBridge};

/// Longest tail accepted (`ExportRequest::tail_seconds`).
pub(crate) const MAX_TAIL_SECONDS: f64 = 60.0;

/// A file of the job: `stem = None` is the mix (or a master "stem").
#[derive(Clone, Debug)]
pub(crate) struct Pass {
    pub stem: Option<TrackId>,
    pub file_name: String,
}

/// What a finished file became.
#[derive(Clone, Debug)]
pub(crate) enum Delivered {
    File(String),
    Download(ExportDownload, Vec<u8>),
}

enum MediaLoad {
    Decode(MediaRef, Box<IncrementalDecoder>),
    Resample(Box<IncrementalResampler>, MediaId),
}
/// Frames of rendering, resampling, peak scanning, encoding or media decoding in one unit of
/// work. The tick loops over units until its time budget (`export/mod.rs`), so this bounds
/// how far a tick can overshoot it.
pub(crate) const UNIT_FRAMES: usize = 4096;

/// Built-in device nodes created in one unit (plugins: one per unit).
pub(crate) const UNIT_BUILTINS: usize = 8;

/// Work done by the largest unit so far (tests/diagnostics).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitStats {
    /// Most audio frames processed by one unit.
    pub max_frames: usize,
    /// Most device nodes created by one unit.
    pub max_devices: usize,
    /// Most plugin instances created by one unit.
    pub max_plugins: usize,
    /// Units run.
    pub units: usize,
}

impl UnitStats {
    fn record(&mut self, frames: usize, devices: usize, plugins: usize) {
        self.max_frames = self.max_frames.max(frames);
        self.max_devices = self.max_devices.max(devices);
        self.max_plugins = self.max_plugins.max(plugins);
        self.units += 1;
    }
}

struct Resolver(std::collections::HashMap<MediaId, Arc<dyn AudioSource>>);

impl ether_devices::SampleResolver for Resolver {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

/// A pass being set up: the offline engine exists, device nodes are created a few per unit.
struct Setup {
    renderer: OfflineRenderer,
    sources: Resolver,
    queue: VecDeque<DeviceId>,
    nodes: BTreeMap<DeviceId, NodeKey>,
}

struct RenderPass {
    renderer: OfflineRenderer,
    /// Leading frames still to drop (graph latency).
    skip: usize,
    /// Frames still to keep.
    remaining: usize,
    total: usize,
    out: Vec<Vec<f32>>,
    scratch: Vec<Vec<f32>>,
}

enum Stage {
    Media,
    Setup(Box<Setup>),
    Render(Box<RenderPass>),
    Resample(Box<IncrementalResampler>),
    Peak {
        channels: Vec<Vec<f32>>,
        rate: u32,
        scan: PeakScan,
    },
    Encode {
        channels: Vec<Vec<f32>>,
        gain: f32,
        encoder: Encoder,
    },
}

/// Outcome of one unit of work.
pub(crate) enum Step {
    Working,
    Done(Vec<Delivered>),
}

pub(crate) struct Job {
    pub id: ExportJobId,
    pub project_id: ProjectId,
    project: Box<Project>,
    request: ExportRequest,
    engine_rate: u32,
    start: Beats,
    frames: usize,
    tail: usize,
    passes: Vec<Pass>,
    pass: usize,
    media_queue: VecDeque<MediaRef>,
    media_load: Option<MediaLoad>,
    media: BTreeMap<MediaId, Arc<DecodedAudio>>,
    stage: Stage,
    delivered: Vec<Delivered>,
    downloads: bool,
    /// User-facing warnings not reported yet.
    pub warnings: Vec<String>,
    pub stats: UnitStats,
}

/// `name` made safe as a single, visible file-name segment.
pub(crate) fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // No leading dots or spaces in any combination (" .x", ". .x"): never a hidden file.
    let trimmed = cleaned.trim_start_matches(|c: char| c == '.' || c.is_whitespace());
    let s: String = trimmed.chars().take(120).collect();
    let s = s.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if s.is_empty() {
        "Export".into()
    } else {
        s.to_string()
    }
}

/// Timeline range of a request, in beats (`Err` = user-facing message).
pub(crate) fn range_of(p: &Project, range: &ExportRange) -> Result<(Beats, Beats), String> {
    let (start, end) = match *range {
        ExportRange::Loop => (p.settings.loop_region.start, p.settings.loop_region.end),
        ExportRange::Custom { start, end } => (start, end),
        ExportRange::Project => {
            let clips = p.clips.values().map(|c| c.start.0 + c.length.0);
            let points = p.automation_points.values().filter_map(|pt| {
                let lane = p.automation_lanes.get(&pt.lane)?;
                matches!(lane.owner, AutomationOwner::Track { .. }).then_some(pt.time.0)
            });
            let end = clips.chain(points).fold(0.0f64, f64::max);
            (Beats(0.0), Beats(end))
        }
    };
    if !(start.0.is_finite() && end.0.is_finite() && start.0 >= 0.0) {
        return Err("invalid export range".into());
    }
    if end.0 <= start.0 + 1e-9 {
        return Err("nothing to export: the range is empty".into());
    }
    Ok((start, end))
}

fn tempo_rt(p: &Project) -> TempoMapRt {
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

/// Media the render needs (audio clips and samplers).
fn used_media(p: &Project) -> Vec<MediaRef> {
    let mut ids = BTreeSet::new();
    for c in p.clips.values() {
        if let ClipContent::Audio(a) = &c.content {
            ids.insert(a.media);
        }
    }
    for d in p.devices.values() {
        if let DeviceKind::Builtin {
            device: BuiltinDevice::Sampler {
                sample: Some(m), ..
            },
        } = &d.kind
        {
            ids.insert(*m);
        }
    }
    ids.into_iter()
        .filter_map(|m| p.media.get(&m).cloned())
        .collect()
}

/// Tracks a stem keeps intact: the stem track and everything nested in it.
fn subtree(p: &Project, root: TrackId) -> BTreeSet<TrackId> {
    p.tracks
        .values()
        .filter(|t| {
            let mut cur = Some(t.id);
            while let Some(id) = cur {
                if id == root {
                    return true;
                }
                cur = p.tracks.get(&id).and_then(|t| t.parent);
            }
            false
        })
        .map(|t| t.id)
        .collect()
}

/// The rendered graph: never the click, no looping, no inputs.
pub(crate) fn offline_overrides(desc: &mut RenderGraphDesc) {
    desc.metronome = false;
    desc.click.count_in_end = None;
    desc.loop_enabled = false;
    for t in &mut desc.tracks {
        t.monitor = false;
        t.audio_input = None;
        t.armed = false;
    }
}

/// Rewrite a mix graph into the stem of `stem` (CONTRACTS.md §11.1; semantics in the
/// module docs of `export`): that track's post-fader output straight into master, its
/// sends/returns included, every other source silent, master chain excluded (the master's
/// devices are not instantiated for stems).
pub(crate) fn stem_graph(desc: &mut RenderGraphDesc, p: &Project, stem: TrackId) {
    let Some(stem_track) = p.tracks.get(&stem) else {
        return;
    };
    if stem_track.kind == TrackKind::Master {
        return;
    }
    let master = p
        .tracks
        .values()
        .find(|t| t.kind == TrackKind::Master)
        .map(|t| t.id);
    let keep = subtree(p, stem);
    let stem_is_return = stem_track.kind == TrackKind::Return;
    for t in &mut desc.tracks {
        t.solo = false;
        if t.kind == TrackKind::Master {
            t.automation
                .retain(|a| !matches!(a.resolved, ResolvedTarget::Node { .. }));
        } else if t.id == stem {
            if t.output.is_some() {
                t.output = master;
            }
            t.group = None;
            t.mute = false;
        } else if keep.contains(&t.id) {
            // Nested in the stem (group): feeds it as usual.
        } else if stem_is_return {
            // Everyone else only feeds the stem return.
            t.output = None;
            t.sends.retain(|s| s.to == stem);
        } else if t.kind == TrackKind::Return {
            // Returns only receive the stem's sends now.
        } else {
            t.output = None;
            t.sends.clear();
            t.clips.clear();
        }
    }
}

fn engine_err(e: ether_core::EngineError) -> String {
    format!("offline engine: {e}")
}

impl Job {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ExportJobId,
        project: &Project,
        request: ExportRequest,
        engine_rate: u32,
        start: Beats,
        end: Beats,
        passes: Vec<Pass>,
    ) -> Self {
        let tempo = tempo_rt(project);
        let rate = engine_rate as f64;
        let s0 = (tempo.beats_to_seconds(start.0) * rate).round();
        let s1 = (tempo.beats_to_seconds(end.0) * rate).round();
        let frames = (s1 - s0).max(0.0) as usize;
        let tail = (request.tail_seconds.clamp(0.0, MAX_TAIL_SECONDS) * rate).round() as usize;
        Self {
            id,
            project_id: project.id,
            project: Box::new(project.clone()),
            media_queue: used_media(project).into(),
            request,
            engine_rate,
            start,
            frames,
            tail,
            passes,
            pass: 0,
            media_load: None,
            media: BTreeMap::new(),
            stage: Stage::Media,
            delivered: Vec::new(),
            downloads: false,
            warnings: Vec::new(),
            stats: UnitStats::default(),
        }
    }

    /// Overall progress, 0..=1.
    pub fn progress(&self) -> f32 {
        let n = self.passes.len().max(1) as f32;
        let within = match &self.stage {
            Stage::Media | Stage::Setup(_) => 0.0,
            Stage::Render(r) => {
                let done = r.total - r.remaining;
                0.8 * done as f32 / r.total.max(1) as f32
            }
            Stage::Resample(r) => 0.8 + 0.05 * r.progress(),
            Stage::Peak { channels, scan, .. } => {
                0.85 + 0.05 * scan.progress(channels.first().map_or(0, Vec::len))
            }
            Stage::Encode {
                channels, encoder, ..
            } => 0.9 + 0.1 * encoder.progress(channels.first().map_or(0, Vec::len)),
        };
        ((self.pass as f32 + within) / n).clamp(0.0, 1.0)
    }

    /// Do one unit of work (bounded: [`UNIT_FRAMES`] frames, [`UNIT_BUILTINS`] built-in
    /// nodes or one plugin instance).
    pub fn step<B: EngineBridge, S: ProjectStore>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        store: &mut S,
    ) -> Result<Step, String> {
        let mut frames = 0;
        let mut devices = 0;
        let mut plugins = 0;
        let r = self.unit(
            bridge,
            engine,
            store,
            &mut frames,
            &mut devices,
            &mut plugins,
        );
        self.stats.record(frames, devices, plugins);
        r
    }

    fn unit<B: EngineBridge, S: ProjectStore>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        store: &mut S,
        frames: &mut usize,
        devices: &mut usize,
        plugins: &mut usize,
    ) -> Result<Step, String> {
        match &mut self.stage {
            Stage::Media => {
                if self.step_media(store, frames)? {
                    self.stage = Stage::Setup(Box::new(self.new_setup()?));
                }
                Ok(Step::Working)
            }
            Stage::Setup(_) => {
                if let Some(pass) = self.step_setup(bridge, engine, devices, plugins)? {
                    self.stage = Stage::Render(Box::new(pass));
                }
                Ok(Step::Working)
            }
            Stage::Render(r) => {
                // One engine block per unit.
                let n = OFFLINE_MAX_BLOCK.min(r.skip + r.remaining);
                for ch in &mut r.scratch {
                    ch.resize(n, 0.0);
                }
                {
                    let mut outs: Vec<&mut [f32]> =
                        r.scratch.iter_mut().map(|c| &mut c[..n]).collect();
                    r.renderer.render(n, &mut outs);
                }
                *frames = n;
                let drop = r.skip.min(n);
                r.skip -= drop;
                let keep = (n - drop).min(r.remaining);
                for (o, s) in r.out.iter_mut().zip(&r.scratch) {
                    o.extend_from_slice(&s[drop..drop + keep]);
                }
                r.remaining -= keep;
                if r.remaining > 0 || r.skip > 0 {
                    return Ok(Step::Working);
                }
                let (overflow, underruns) = r.renderer.diagnostics();
                let channels = std::mem::take(&mut r.out);
                if overflow || underruns > 0 {
                    self.warnings.push(format!(
                        "export \"{}\": the render may be incomplete ({})",
                        self.passes[self.pass].file_name,
                        if overflow {
                            "too many events in a block"
                        } else {
                            "audio sources could not keep up"
                        }
                    ));
                }
                // Drops the offline engine and its fresh nodes (plugins included).
                self.stage = self.post_stage(channels)?;
                Ok(Step::Working)
            }
            Stage::Resample(r) => {
                *frames = UNIT_FRAMES;
                if r.step(UNIT_FRAMES)
                    .map_err(|e| format!("resampling failed: {e}"))?
                {
                    let Stage::Resample(r) = std::mem::replace(&mut self.stage, Stage::Media)
                    else {
                        unreachable!()
                    };
                    let audio = r.finish();
                    self.stage = self.level_stage(audio.channels, audio.sample_rate)?;
                }
                Ok(Step::Working)
            }
            Stage::Peak {
                channels,
                rate,
                scan,
            } => {
                *frames = UNIT_FRAMES;
                if let Some(peak) = scan.step(channels, UNIT_FRAMES) {
                    let channels = std::mem::take(channels);
                    let rate = *rate;
                    self.stage = self.encode_stage(channels, rate, normalize_gain(peak))?;
                }
                Ok(Step::Working)
            }
            Stage::Encode {
                channels,
                gain,
                encoder,
            } => {
                *frames = UNIT_FRAMES;
                let Some(bytes) = encoder.step(channels, *gain, UNIT_FRAMES)? else {
                    return Ok(Step::Working);
                };
                self.stage = Stage::Media;
                self.deliver(store, bytes)?;
                self.pass += 1;
                if self.pass >= self.passes.len() {
                    return Ok(Step::Done(std::mem::take(&mut self.delivered)));
                }
                self.stage = Stage::Setup(Box::new(self.new_setup()?));
                Ok(Step::Working)
            }
        }
    }

    /// Decode/resample the next media by one unit. `Ok(true)` once everything is loaded.
    fn step_media<S: ProjectStore>(
        &mut self,
        store: &mut S,
        frames: &mut usize,
    ) -> Result<bool, String> {
        let load = match self.media_load.take() {
            Some(l) => l,
            None => {
                let Some(m) = self.media_queue.pop_front() else {
                    return Ok(true);
                };
                let bytes = store
                    .read(self.project_id, &m.file)
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
                    self.media_load = Some(MediaLoad::Decode(m, dec));
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
                self.media_load = Some(MediaLoad::Resample(Box::new(r), m.id));
                Ok(false)
            }
            MediaLoad::Resample(mut r, id) => {
                if r.step(UNIT_FRAMES).map_err(|e| e.to_string())? {
                    self.media.insert(id, Arc::new(r.finish()));
                } else {
                    self.media_load = Some(MediaLoad::Resample(r, id));
                }
                Ok(false)
            }
        }
    }

    fn master(&self) -> Option<TrackId> {
        self.project
            .tracks
            .values()
            .find(|t| t.kind == TrackKind::Master)
            .map(|t| t.id)
    }

    fn is_stem(&self) -> bool {
        let master = self.master();
        self.passes[self.pass]
            .stem
            .is_some_and(|s| Some(s) != master)
    }

    /// A fresh offline engine for the current pass, with the media registered; its device
    /// nodes are created by [`Job::step_setup`].
    fn new_setup(&self) -> Result<Setup, String> {
        let p = &*self.project;
        let n_items = p.devices.len() + self.media.len();
        let mut renderer = OfflineRenderer::new(EngineConfig {
            sample_rate: self.engine_rate,
            max_block_size: OFFLINE_MAX_BLOCK,
            input_channels: 0,
            output_channels: 2,
            max_nodes: (n_items + 64).max(EngineConfig::default().max_nodes),
            control_queue_capacity: 2 * n_items + 256,
            ..EngineConfig::default()
        });
        let mut sources = std::collections::HashMap::new();
        for (id, audio) in &self.media {
            let src: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio.clone()));
            renderer
                .handle()
                .add_source(*id, src.clone())
                .map_err(engine_err)?;
            sources.insert(*id, src);
        }
        let master = self.master();
        let is_stem = self.is_stem();
        let queue = p
            .devices
            .values()
            // Master chain excluded from stems.
            .filter(|d| !(is_stem && Some(d.track) == master))
            // Disabled plugins are bypassed anyway: no instance needed.
            .filter(|d| d.enabled || !matches!(d.kind, DeviceKind::Plugin { .. }))
            .map(|d| d.id)
            .collect();
        Ok(Setup {
            renderer,
            sources: Resolver(sources),
            queue,
            nodes: BTreeMap::new(),
        })
    }

    /// Create a few device nodes (up to [`UNIT_BUILTINS`] built-ins, or one plugin); once
    /// all exist, publish the graph and start the transport.
    fn step_setup<B: EngineBridge>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        devices: &mut usize,
        plugins: &mut usize,
    ) -> Result<Option<RenderPass>, String> {
        let Stage::Setup(setup) = &mut self.stage else {
            unreachable!("setup stage")
        };
        let p = &*self.project;
        while let Some(&id) = setup.queue.front() {
            let Some(d) = p.devices.get(&id) else {
                setup.queue.pop_front();
                continue;
            };
            let is_plugin = matches!(d.kind, DeviceKind::Plugin { .. });
            if *devices >= UNIT_BUILTINS || (is_plugin && *devices > 0) {
                return Ok(None);
            }
            setup.queue.pop_front();
            let node: Box<dyn ether_core::Node> = match &d.kind {
                DeviceKind::Builtin { device } => {
                    let mut node = ether_devices::create(device, &setup.sources);
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
                            self.warnings.push(format!(
                                "export: could not read the current state of \"{}\" ({e}); \
                                 rendering it with its last saved state",
                                d.name
                            ));
                            plugin.state.clone()
                        }
                    };
                    *plugins += 1;
                    bridge
                        .create_offline_plugin(d.id, plugin, state.as_ref(), self.engine_rate)
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
            let key = setup.renderer.handle().add_node(node).map_err(engine_err)?;
            setup.nodes.insert(d.id, key);
            if is_plugin {
                return Ok(None);
            }
        }
        if *devices > 0 {
            // Publishing (graph compile) is its own unit.
            return Ok(None);
        }

        let Stage::Setup(setup) = std::mem::replace(&mut self.stage, Stage::Media) else {
            unreachable!()
        };
        let Setup {
            mut renderer,
            nodes,
            ..
        } = *setup;
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
        offline_overrides(&mut desc);
        if self.is_stem()
            && let Some(s) = self.passes[self.pass].stem
        {
            stem_graph(&mut desc, p, s);
        }
        renderer.publish(desc).map_err(engine_err)?;
        renderer.start(self.start).map_err(engine_err)?;
        let latency = renderer.latency() as usize;
        let total = self.frames + self.tail;
        Ok(Some(RenderPass {
            renderer,
            skip: latency,
            remaining: total,
            total,
            out: (0..2).map(|_| Vec::with_capacity(total)).collect(),
            scratch: vec![Vec::new(); 2],
        }))
    }

    /// After a pass rendered: resample to the requested rate, or go on to leveling.
    fn post_stage(&self, channels: Vec<Vec<f32>>) -> Result<Stage, String> {
        match self.request.format.sample_rate {
            Some(rate) if rate != self.engine_rate => {
                let audio = DecodedAudio {
                    sample_rate: self.engine_rate,
                    channels,
                };
                let r = IncrementalResampler::new(Arc::new(audio), rate)
                    .map_err(|e| format!("resampling failed: {e}"))?;
                Ok(Stage::Resample(Box::new(r)))
            }
            _ => self.level_stage(channels, self.engine_rate),
        }
    }

    /// Normalize: scan the peak first; otherwise encode at unity gain.
    fn level_stage(&self, channels: Vec<Vec<f32>>, rate: u32) -> Result<Stage, String> {
        if self.request.normalize {
            Ok(Stage::Peak {
                channels,
                rate,
                scan: PeakScan::default(),
            })
        } else {
            self.encode_stage(channels, rate, 1.0)
        }
    }

    fn encode_stage(&self, channels: Vec<Vec<f32>>, rate: u32, gain: f32) -> Result<Stage, String> {
        let encoder = Encoder::new(
            self.request.format.container,
            self.request.format.bit_depth,
            channels.len(),
            channels.first().map_or(0, Vec::len),
            rate,
        )?;
        Ok(Stage::Encode {
            channels,
            gain,
            encoder,
        })
    }

    fn deliver<S: ProjectStore>(&mut self, store: &mut S, bytes: Vec<u8>) -> Result<(), String> {
        let name = self.passes[self.pass].file_name.clone();
        if !self.downloads {
            match store.write_export(self.project_id, &name, &bytes) {
                Ok(path) => {
                    self.delivered.push(Delivered::File(path));
                    return Ok(());
                }
                Err(crate::store::StoreError::Unsupported(_)) => self.downloads = true,
                Err(e) => return Err(format!("could not write \"{name}\": {e}")),
            }
        }
        let (_, mime) = encode::file_type(self.request.format.container);
        self.delivered.push(Delivered::Download(
            ExportDownload {
                token: format!("{}-{}", self.id, self.pass),
                name,
                mime: mime.into(),
                size: bytes.len() as f64,
            },
            bytes,
        ));
        Ok(())
    }
}

/// File names of the passes of `request`: unique among themselves and against `existing`
/// (lower-cased names already in `exports/`), so an export never overwrites a file; a
/// collision gets a ` (2)`, ` (3)`, ... suffix.
pub(crate) fn plan_passes(
    p: &Project,
    request: &ExportRequest,
    stems: &[TrackId],
    existing: &BTreeSet<String>,
) -> Vec<Pass> {
    let base = sanitize(request.name.as_deref().unwrap_or(&p.settings.name));
    let (ext, _) = encode::file_type(request.format.container);
    let mut used = existing.clone();
    let mut unique = |stem: String| {
        let mut name = format!("{stem}.{ext}");
        let mut i = 2;
        while !used.insert(name.to_lowercase()) {
            name = format!("{stem} ({i}).{ext}");
            i += 1;
        }
        name
    };
    match &request.mode {
        ExportMode::Mix => vec![Pass {
            stem: None,
            file_name: unique(base),
        }],
        ExportMode::Stems { .. } => stems
            .iter()
            .map(|t| {
                let track = p.tracks.get(t).map_or("Track", |t| t.name.as_str());
                Pass {
                    stem: Some(*t),
                    file_name: unique(sanitize(&format!("{base} - {track}"))),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("My Song"), "My Song");
        assert_eq!(sanitize("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize("..hidden"), "hidden");
        assert_eq!(sanitize(" .x"), "x");
        assert_eq!(sanitize(". . .x"), "x");
        assert_eq!(sanitize("   "), "Export");
        assert_eq!(sanitize("x. ."), "x");
        assert_eq!(sanitize(&"n".repeat(300)).len(), 120);
    }
}

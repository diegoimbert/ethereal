//! One export job: load media, render each pass (mix or one stem) through a private
//! [`OfflineRenderer`], then resample/normalize/encode and deliver each file.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use ether_core::graph::ResolvedTarget;
use ether_core::offline::{OFFLINE_MAX_BLOCK, OfflineRenderer};
use ether_core::protocol::export::{
    AudioContainer, ExportDownload, ExportJobId, ExportMode, ExportRange, ExportRequest,
};
use ether_core::protocol::model::*;
use ether_core::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};
use ether_core::{AudioSource, EngineConfig, NodeKey, RenderGraphDesc};
use ether_media::{DecodedAudio, InMemorySource};

use super::encode::{self, FlacEncoder};
use crate::compile::{CompileContext, compile_graph_with};
use crate::engine::EngineState;
use crate::media::{IncrementalDecoder, IncrementalResampler, extension_of};
use crate::store::ProjectStore;
use crate::{BridgeError, EngineBridge};

/// Longest tail accepted (`ExportRequest::tail_seconds`).
pub(crate) const MAX_TAIL_SECONDS: f64 = 60.0;

/// Frames rendered/encoded per unit of work (the tick loops over units until its budget).
pub(crate) const CHUNK: usize = 8192;

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
    Render(Box<RenderPass>),
    Resample(Box<IncrementalResampler>),
    Encode {
        channels: Vec<Vec<f32>>,
        rate: u32,
        flac: Option<Box<FlacEncoder>>,
    },
}

/// Outcome of one unit of work.
pub(crate) enum Step {
    Working,
    /// A warning for the user (render diagnostics).
    Warning(String),
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
}

/// `name` made safe as a single file-name segment.
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
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let mut s: String = trimmed.chars().take(120).collect();
    s = s.trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() { "Export".into() } else { s }
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

/// Rewrite a mix graph into the stem of `stem` (CONTRACTS.md §11.1): that track's
/// post-fader output straight into master, its sends/returns included, every other source
/// silent, master chain excluded (the master's devices are not instantiated for stems).
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
        }
    }

    /// Overall progress, 0..=1.
    pub fn progress(&self) -> f32 {
        let n = self.passes.len().max(1) as f32;
        let within = match &self.stage {
            Stage::Media => 0.0,
            Stage::Render(r) => {
                let done = r.total - r.remaining;
                0.85 * done as f32 / r.total.max(1) as f32
            }
            Stage::Resample(r) => 0.85 + 0.05 * r.progress(),
            Stage::Encode { channels, flac, .. } => {
                let total = channels.first().map_or(0, Vec::len);
                0.9 + 0.1 * flac.as_ref().map_or(0.0, |f| f.progress(total))
            }
        };
        ((self.pass as f32 + within) / n).clamp(0.0, 1.0)
    }

    /// Do one unit of work (about [`CHUNK`] frames).
    pub fn step<B: EngineBridge, S: ProjectStore>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        store: &mut S,
    ) -> Result<Step, String> {
        if matches!(self.stage, Stage::Media) {
            if self.step_media(store)? {
                let pass = self.build_pass(bridge, engine)?;
                self.stage = Stage::Render(Box::new(pass));
            }
            return Ok(Step::Working);
        }
        match &mut self.stage {
            Stage::Media => unreachable!("handled above"),
            Stage::Render(r) => {
                let n = CHUNK.min(r.skip + r.remaining);
                for ch in &mut r.scratch {
                    ch.resize(n, 0.0);
                }
                {
                    let mut outs: Vec<&mut [f32]> =
                        r.scratch.iter_mut().map(|c| &mut c[..n]).collect();
                    r.renderer.render(n, &mut outs);
                }
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
                let warning = (overflow || underruns > 0).then(|| {
                    format!(
                        "export \"{}\": the render may be incomplete ({})",
                        self.passes[self.pass].file_name,
                        if overflow {
                            "too many events in a block"
                        } else {
                            "audio sources could not keep up"
                        }
                    )
                });
                // Drops the offline engine and its fresh nodes (plugins included).
                self.stage = self.post_stage(channels)?;
                Ok(warning.map_or(Step::Working, Step::Warning))
            }
            Stage::Resample(r) => {
                if r.step(CHUNK)
                    .map_err(|e| format!("resampling failed: {e}"))?
                {
                    let Stage::Resample(r) = std::mem::replace(&mut self.stage, Stage::Media)
                    else {
                        unreachable!()
                    };
                    let audio = r.finish();
                    self.stage = self.encode_stage(audio.channels, audio.sample_rate)?;
                }
                Ok(Step::Working)
            }
            Stage::Encode {
                channels,
                rate,
                flac,
            } => {
                let done = match flac {
                    Some(f) => f.step(channels, CHUNK)?,
                    None => true,
                };
                if !done {
                    return Ok(Step::Working);
                }
                let bytes = match flac.take() {
                    Some(f) => f.finish()?,
                    None => encode::encode_wav(channels, *rate, self.request.format.bit_depth)?,
                };
                self.deliver(store, bytes)?;
                self.pass += 1;
                if self.pass >= self.passes.len() {
                    return Ok(Step::Done(std::mem::take(&mut self.delivered)));
                }
                let pass = self.build_pass(bridge, engine)?;
                self.stage = Stage::Render(Box::new(pass));
                Ok(Step::Working)
            }
        }
    }

    /// Decode/resample the next media. `Ok(true)` once everything is loaded.
    fn step_media<S: ProjectStore>(&mut self, store: &mut S) -> Result<bool, String> {
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
        match load {
            MediaLoad::Decode(m, mut dec) => {
                let done = dec
                    .step(CHUNK * 4)
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
                if r.step(CHUNK * 4).map_err(|e| e.to_string())? {
                    self.media.insert(id, Arc::new(r.finish()));
                } else {
                    self.media_load = Some(MediaLoad::Resample(r, id));
                }
                Ok(false)
            }
        }
    }

    /// A fresh offline engine for the current pass, graph published, transport started.
    fn build_pass<B: EngineBridge>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
    ) -> Result<RenderPass, String> {
        let p = &*self.project;
        let stem = self.passes[self.pass].stem;
        let master = p
            .tracks
            .values()
            .find(|t| t.kind == TrackKind::Master)
            .map(|t| t.id);
        let is_stem = stem.is_some_and(|s| Some(s) != master);
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
        let engine_err = |e: ether_core::EngineError| format!("offline engine: {e}");

        let mut sources: std::collections::HashMap<MediaId, Arc<dyn AudioSource>> =
            Default::default();
        for (id, audio) in &self.media {
            let src: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio.clone()));
            renderer
                .handle()
                .add_source(*id, src.clone())
                .map_err(engine_err)?;
            sources.insert(*id, src);
        }
        struct Resolver<'a>(&'a std::collections::HashMap<MediaId, Arc<dyn AudioSource>>);
        impl ether_devices::SampleResolver for Resolver<'_> {
            fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
                self.0.get(&media).cloned()
            }
        }

        let mut nodes: BTreeMap<DeviceId, NodeKey> = BTreeMap::new();
        for d in p.devices.values() {
            if is_stem && Some(d.track) == master {
                continue; // master chain excluded from stems
            }
            let node: Box<dyn ether_core::Node> = match &d.kind {
                DeviceKind::Builtin { device } => {
                    let mut node = ether_devices::create(device, &Resolver(&sources));
                    for (id, v) in &d.params {
                        node.set_param(*id, *v);
                    }
                    node
                }
                DeviceKind::Plugin { plugin } => {
                    if !d.enabled {
                        continue; // bypassed anyway: no instance needed
                    }
                    let live = bridge.plugin_state(d.id).ok().flatten();
                    let state = live.or_else(|| plugin.state.clone());
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
            let key = renderer.handle().add_node(node).map_err(engine_err)?;
            nodes.insert(d.id, key);
        }

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
        if let (true, Some(s)) = (is_stem, stem) {
            stem_graph(&mut desc, p, s);
        }
        renderer.handle().publish(desc).map_err(engine_err)?;
        renderer.start(self.start).map_err(engine_err)?;
        let latency = renderer.latency() as usize;
        let total = self.frames + self.tail;
        Ok(RenderPass {
            renderer,
            skip: latency,
            remaining: total,
            total,
            out: (0..2).map(|_| Vec::with_capacity(total)).collect(),
            scratch: vec![Vec::new(); 2],
        })
    }

    /// After a pass rendered: resample to the requested rate, or go straight to encoding.
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
            _ => self.encode_stage(channels, self.engine_rate),
        }
    }

    fn encode_stage(&self, mut channels: Vec<Vec<f32>>, rate: u32) -> Result<Stage, String> {
        if self.request.normalize {
            encode::normalize(&mut channels);
        }
        let flac = match self.request.format.container {
            AudioContainer::Flac => Some(Box::new(FlacEncoder::new(
                channels.len(),
                rate,
                self.request.format.bit_depth,
            )?)),
            AudioContainer::Wav => None,
        };
        Ok(Stage::Encode {
            channels,
            rate,
            flac,
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

/// File names of the passes of `request` (unique, with extension).
pub(crate) fn plan_passes(p: &Project, request: &ExportRequest, stems: &[TrackId]) -> Vec<Pass> {
    let base = sanitize(request.name.as_deref().unwrap_or(&p.settings.name));
    let (ext, _) = encode::file_type(request.format.container);
    let mut used = BTreeSet::new();
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
        assert_eq!(sanitize("   "), "Export");
        assert_eq!(sanitize("x."), "x");
        assert_eq!(sanitize(&"n".repeat(300)).len(), 120);
    }
}

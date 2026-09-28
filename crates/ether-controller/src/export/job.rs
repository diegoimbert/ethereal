//! One export job: load media, set up each pass (mix or one stem) on a private
//! [`OfflineRenderer`] a few nodes at a time, render it block by block, then
//! resample/normalize/encode and deliver each file. Every step is a bounded unit of work.

use std::collections::BTreeSet;
use std::sync::Arc;

use ether_core::RenderGraphDesc;
use ether_core::graph::ResolvedTarget;
use ether_core::protocol::export::{
    ExportDownload, ExportJobId, ExportMode, ExportRange, ExportRequest,
};
use ether_core::protocol::model::*;
use ether_media::DecodedAudio;

use super::encode::{self, Encoder, PeakScan, normalize_gain};
use super::offline::{Capture, MediaLoader, NodeSetup, tempo_rt};
use crate::EngineBridge;
use crate::engine::EngineState;
use crate::media::IncrementalResampler;
use crate::store::ProjectStore;

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

enum Stage {
    Media,
    Setup(Box<NodeSetup>),
    Render(Box<Capture>),
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
    media: MediaLoader,
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
    // v0.2 (`freeze-bounce`): frozen tracks play their render.
    for t in p.tracks.values() {
        if let Some(f) = &t.freeze {
            ids.insert(f.media);
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
            media: MediaLoader::new(project.id, used_media(project), engine_rate),
            request,
            engine_rate,
            start,
            frames,
            tail,
            passes,
            pass: 0,
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
            Stage::Render(r) => 0.8 * r.captured() as f32 / r.total().max(1) as f32,
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
                if self.media.step(store, frames)? {
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
                if !r.step(frames) {
                    return Ok(Step::Working);
                }
                if let Some(why) = r.problem() {
                    self.warnings.push(format!(
                        "export \"{}\": the render may be incomplete ({why})",
                        self.passes[self.pass].file_name,
                    ));
                }
                let Stage::Render(r) = std::mem::replace(&mut self.stage, Stage::Media) else {
                    unreachable!()
                };
                // Drops the offline engine and its fresh nodes (plugins included).
                self.stage = self.post_stage(r.into_channels())?;
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
    fn new_setup(&self) -> Result<NodeSetup, String> {
        let p = &*self.project;
        let master = self.master();
        let is_stem = self.is_stem();
        let devices = p
            .devices
            .values()
            // Master chain excluded from stems.
            .filter(|d| !(is_stem && Some(d.track) == master))
            // Frozen tracks play their render (`freeze-bounce`): no nodes.
            .filter(|d| p.tracks.get(&d.track).is_none_or(|t| t.freeze.is_none()))
            // Disabled plugins are bypassed anyway: no instance needed.
            .filter(|d| d.enabled || !matches!(d.kind, DeviceKind::Plugin { .. }))
            .map(|d| d.id)
            .collect();
        NodeSetup::new(p, &self.media.media, devices, self.engine_rate, "export")
    }

    /// Create a few device nodes (up to [`UNIT_BUILTINS`] built-ins, or one plugin); once
    /// all exist, publish the graph and start the transport.
    fn step_setup<B: EngineBridge>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        devices: &mut usize,
        plugins: &mut usize,
    ) -> Result<Option<Capture>, String> {
        let Stage::Setup(setup) = &mut self.stage else {
            unreachable!("setup stage")
        };
        let p = &*self.project;
        if !setup.step(
            p,
            bridge,
            self.engine_rate,
            &mut self.warnings,
            devices,
            plugins,
        )? {
            return Ok(None);
        }
        let Stage::Setup(setup) = std::mem::replace(&mut self.stage, Stage::Media) else {
            unreachable!()
        };
        let stem = self
            .is_stem()
            .then_some(self.passes[self.pass].stem)
            .flatten();
        let renderer = setup.finish(p, engine, self.start, |desc| {
            if let Some(s) = stem {
                stem_graph(desc, p, s);
            }
        })?;
        Ok(Some(Capture::new(renderer, self.frames + self.tail)))
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

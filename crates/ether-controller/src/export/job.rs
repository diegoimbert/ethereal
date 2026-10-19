//! One export job: load media, set up each pass (mix or one stem) on a private
//! [`OfflineRenderer`] a few nodes at a time, then render it block by block, each block
//! streamed through every file of the render (resampled per output rate, encoded once per
//! requested format, `ExportRequest::extra_formats`) into a file the store writes as it
//! goes, or into memory (#434). Every step is a bounded unit of work.
//!
//! Normalizing needs a file's peak before its first sample is encoded: those signals are
//! held whole (at each output rate) until the render is done, then encoded at their gain.
//! A normalized export therefore still holds its render in memory (bounded by
//! [`MAX_CAPTURE_BYTES`] per render); a second render to find the peak would halve that,
//! but isn't guaranteed to give the same samples (plugins).

use std::collections::{BTreeSet, VecDeque};

use ether_core::RenderGraphDesc;
use ether_core::protocol::export::{
    AudioContainer, ExportDownload, ExportFormat, ExportJobId, ExportMode, ExportRange,
    ExportRequest,
};
use ether_core::protocol::model::*;
use ether_media::resampled_len;

use super::encode::{self, Encoder, PeakScan, Sink, normalize_gain};
use super::offline::{Capture, MediaLoader, NodeSetup, tempo_rt};
use crate::EngineBridge;
use crate::engine::EngineState;
use crate::media::StreamResampler;
use crate::store::{Library, ProjectStore, StoreError};

/// Longest tail accepted (`ExportRequest::tail_seconds`).
pub const MAX_TAIL_SECONDS: f64 = 60.0;

/// A file of the job: `stem = None` is the mix (or a master "stem").
#[derive(Clone, Debug)]
pub struct Pass {
    pub stem: Option<TrackId>,
    /// One name per format ([`formats`] order).
    pub file_names: Vec<String>,
    /// The stem is taken from the project's one full render (see [`capturable`]) instead
    /// of a render of its own.
    pub captured: bool,
}

/// One offline render of the job: the files it produces are `main` (the render's master
/// signal, or the stem of a stem-only render) and `stems` (post-fader captures of a full
/// render), as indices into the job's passes.
#[derive(Clone, Debug)]
struct Run {
    /// Render the whole project (the mix graph) instead of rewriting it into one stem.
    full: bool,
    main: Option<usize>,
    stems: Vec<(usize, TrackId)>,
}

/// Most bytes of normalized signals a render may hold at once (each is raw `f32` stereo
/// until its peak is known). Captured stems beyond it get a pass of their own.
const MAX_CAPTURE_BYTES: usize = 768 << 20;

/// What a finished file became.
#[derive(Clone, Debug)]
pub enum Delivered {
    File(String),
    Download(ExportDownload, Vec<u8>),
}

/// Frames of media decoding, or of encoding a normalized signal, in one unit of work (a
/// render unit is one engine block, or one block through one chain). The tick loops over
/// units until its time budget (`export/mod.rs`), so this bounds how far a tick can
/// overshoot it.
pub const UNIT_FRAMES: usize = 4096;

/// Built-in device nodes created in one unit (plugins: one per unit).
pub const UNIT_BUILTINS: usize = 8;

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
    /// Rendering block by block; each block goes through the run's [`Chain`]s (`Job::work`)
    /// before the next one is rendered.
    Render(Box<Capture>),
    /// The render is done (its engine dropped): the chains still take their last block,
    /// flush their resampler and finish their files.
    Finish,
    /// Normalize: encode each held signal at its gain, a unit at a time.
    Level(LevelAt),
}

/// Where the normalize stage is: a file of a held chain, and how far it is encoded.
#[derive(Clone, Copy)]
struct LevelAt {
    output: usize,
    chain: usize,
    file: usize,
    pos: usize,
}

/// The render's channels (`OfflineRenderer` outputs stereo).
const CHANNELS: usize = 2;

/// Most files a run writes at once (open file handles on native): captured stems beyond
/// it get a render of their own.
const MAX_RUN_FILES: usize = 64;

/// A rendered signal of the current run (the render's main signal, or a captured stem)
/// and the files made from it.
struct Output {
    pass: usize,
    /// The captured track (`None`: the render's main signal).
    track: Option<TrackId>,
    /// The signal's latest render block (engine rate), until every chain took it.
    block: Vec<Vec<f32>>,
    chains: Vec<Chain>,
}

/// The files of an [`Output`] at one rate: the signal goes through the resampler (when the
/// rate is not the engine's) into each file's encoder, or, to normalize, into `held`.
struct Chain {
    rate: u32,
    resampler: Option<Box<StreamResampler>>,
    /// The resampler's output for the current block.
    resampled: Vec<Vec<f32>>,
    files: Vec<FileOut>,
    /// Normalize: the whole signal at this rate and its peak, encoded once the render is
    /// done (the gain needs the peak first).
    held: Option<(Vec<Vec<f32>>, PeakScan)>,
}

struct FileOut {
    /// Index into the job's formats.
    format: usize,
    /// `None` once finished.
    encoder: Option<Encoder>,
}

/// A unit of work of a chain (`Job::work`).
#[derive(Clone, Copy)]
enum Work {
    /// Take the output's latest block.
    Feed { output: usize, chain: usize },
    /// The signal ended: flush the resampler, finish the files (unless held).
    End { output: usize, chain: usize },
}

impl Chain {
    /// Take the next block of the signal (engine rate).
    fn take(&mut self, block: &[Vec<f32>]) -> Result<(), String> {
        let block: Vec<&[f32]> = block.iter().map(Vec::as_slice).collect();
        let Some(r) = &mut self.resampler else {
            return Self::level(&mut self.held, &mut self.files, &block);
        };
        self.resampled.iter_mut().for_each(Vec::clear);
        r.push(&block, &mut self.resampled)
            .map_err(|e| format!("resampling failed: {e}"))?;
        let out: Vec<&[f32]> = self.resampled.iter().map(Vec::as_slice).collect();
        Self::level(&mut self.held, &mut self.files, &out)
    }

    /// The signal ended: the resampler's last output.
    fn end(&mut self) -> Result<(), String> {
        let Some(r) = &mut self.resampler else {
            return Ok(());
        };
        self.resampled.iter_mut().for_each(Vec::clear);
        r.finish(&mut self.resampled)
            .map_err(|e| format!("resampling failed: {e}"))?;
        self.resampler = None;
        let out: Vec<&[f32]> = self.resampled.iter().map(Vec::as_slice).collect();
        Self::level(&mut self.held, &mut self.files, &out)
    }

    /// Output-rate signal: held to normalize, else encoded right away (unity gain).
    fn level(
        held: &mut Option<(Vec<Vec<f32>>, PeakScan)>,
        files: &mut [FileOut],
        block: &[&[f32]],
    ) -> Result<(), String> {
        if let Some((signal, peak)) = held {
            peak.scan(block);
            for (s, b) in signal.iter_mut().zip(block) {
                s.extend_from_slice(b);
            }
            return Ok(());
        }
        for f in files {
            if let Some(e) = &mut f.encoder {
                e.push(block, 1.0)?;
            }
        }
        Ok(())
    }
}

/// The formats of a request: `format`, then the extra ones.
pub fn formats(request: &ExportRequest) -> Vec<ExportFormat> {
    std::iter::once(request.format)
        .chain(request.extra_formats.iter().flatten().copied())
        .collect()
}

/// The rate a format's files are written at, for a render at `engine_rate`.
fn output_rate(format: &ExportFormat, engine_rate: u32) -> u32 {
    match (format.sample_rate, format.container) {
        (Some(rate), _) => rate,
        (None, AudioContainer::Mp3) => encode::mp3_rate(engine_rate),
        (None, _) => engine_rate,
    }
}

/// Outcome of one unit of work.
pub enum Step {
    Working,
    Done(Vec<Delivered>),
}

pub struct Job {
    pub id: ExportJobId,
    pub project_id: ProjectId,
    project: Box<Project>,
    request: ExportRequest,
    formats: Vec<ExportFormat>,
    engine_rate: u32,
    start: Beats,
    frames: usize,
    tail: usize,
    passes: Vec<Pass>,
    runs: Vec<Run>,
    run: usize,
    /// The current run's signals and their files.
    outputs: Vec<Output>,
    /// Chain work queued by the render, done before it renders on.
    work: VecDeque<Work>,
    /// Files of this run finished / in total (progress).
    run_done: usize,
    run_files: usize,
    media: MediaLoader,
    stage: Stage,
    /// By (format, pass).
    delivered: Vec<((usize, usize), Delivered)>,
    downloads: bool,
    /// The store takes streamed files ([`ProjectStore::create_export`]) until it says
    /// `Unsupported`; then files are encoded in memory and written whole.
    streams: bool,
    /// User-facing warnings not reported yet.
    pub warnings: Vec<crate::text::Msg>,
    pub stats: UnitStats,
}

/// `name` made safe as a single, visible file-name segment.
pub fn sanitize(name: &str) -> String {
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
pub fn range_of(p: &Project, range: &ExportRange) -> Result<(Beats, Beats), String> {
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
pub fn offline_overrides(desc: &mut RenderGraphDesc) {
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
pub fn stem_graph(desc: &mut RenderGraphDesc, p: &Project, stem: TrackId) {
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
            t.automation.retain(|a| a.resolved.node().is_none());
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
        mut passes: Vec<Pass>,
    ) -> Self {
        let tempo = tempo_rt(project);
        let rate = engine_rate as f64;
        let s0 = (tempo.beats_to_seconds(start.0) * rate).round();
        let s1 = (tempo.beats_to_seconds(end.0) * rate).round();
        let frames = (s1 - s0).max(0.0) as usize;
        let tail = (request.tail_seconds.clamp(0.0, MAX_TAIL_SECONDS) * rate).round() as usize;
        let formats = formats(&request);
        // Streamed files hold a few blocks each; normalized ones hold their whole signal,
        // once per output rate, until the peak is known.
        let held_bytes: usize = if request.normalize {
            let mut rates: Vec<u32> = formats
                .iter()
                .map(|f| output_rate(f, engine_rate))
                .collect();
            rates.sort_unstable();
            rates.dedup();
            rates
                .iter()
                .map(|&r| resampled_len(frames + tail, engine_rate, r) * CHANNELS * 4)
                .sum()
        } else {
            0
        };
        let by_memory = match held_bytes {
            0 => usize::MAX,
            b => (MAX_CAPTURE_BYTES / b).saturating_sub(1),
        };
        let by_files = (MAX_RUN_FILES / formats.len().max(1)).saturating_sub(1);
        let runs = plan_runs(&mut passes, by_memory.min(by_files));
        Self {
            id,
            project_id: project.id,
            project: Box::new(project.clone()),
            media: MediaLoader::new(project.id, used_media(project), engine_rate),
            downloads: request.in_memory == Some(true),
            streams: true,
            formats,
            request,
            engine_rate,
            start,
            frames,
            tail,
            passes,
            runs,
            run: 0,
            outputs: Vec::new(),
            work: VecDeque::new(),
            run_done: 0,
            run_files: 1,
            stage: Stage::Media,
            delivered: Vec::new(),
            warnings: Vec::new(),
            stats: UnitStats::default(),
        }
    }

    /// Share of a run's progress that is the render (the rest: normalizing).
    fn render_share(&self) -> f32 {
        if self.request.normalize { 0.8 } else { 1.0 }
    }

    /// Overall progress, 0..=1.
    pub fn progress(&self) -> f32 {
        let n = self.runs.len().max(1) as f32;
        let render = self.render_share();
        let within = match &self.stage {
            Stage::Media | Stage::Setup(_) => 0.0,
            Stage::Render(r) => render * r.captured() as f32 / r.total().max(1) as f32,
            Stage::Finish => render,
            Stage::Level(at) => {
                let len = self.held_len(at.output, at.chain).max(1);
                let part = (at.pos as f32 / len as f32).min(1.0);
                render
                    + (1.0 - render) * (self.run_done as f32 + part) / self.run_files.max(1) as f32
            }
        };
        ((self.run as f32 + within) / n).clamp(0.0, 1.0)
    }

    fn held_len(&self, output: usize, chain: usize) -> usize {
        self.outputs
            .get(output)
            .and_then(|o| o.chains.get(chain))
            .and_then(|c| c.held.as_ref())
            .map_or(0, |(s, _)| s.first().map_or(0, Vec::len))
    }

    /// Do one unit of work (bounded: an engine block through one chain, [`UNIT_FRAMES`]
    /// frames, [`UNIT_BUILTINS`] built-in nodes or one plugin instance).
    pub fn step<B: EngineBridge, S: ProjectStore, L: Library>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        store: &mut S,
        library: &mut L,
    ) -> Result<Step, String> {
        let mut frames = 0;
        let mut devices = 0;
        let mut plugins = 0;
        let r = self.unit(
            bridge,
            engine,
            store,
            library,
            &mut frames,
            &mut devices,
            &mut plugins,
        );
        self.stats.record(frames, devices, plugins);
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn unit<B: EngineBridge, S: ProjectStore, L: Library>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        store: &mut S,
        library: &mut L,
        frames: &mut usize,
        devices: &mut usize,
        plugins: &mut usize,
    ) -> Result<Step, String> {
        // Chain work first: a rendered block goes everywhere before the next is rendered.
        if let Some(w) = self.work.pop_front() {
            self.chain_work(w, store, frames)?;
            return Ok(Step::Working);
        }
        match &mut self.stage {
            Stage::Media => {
                if self.media.step(store, library, frames)? {
                    self.stage = Stage::Setup(Box::new(self.new_setup()?));
                }
                Ok(Step::Working)
            }
            Stage::Setup(_) => {
                if let Some(capture) = self.step_setup(bridge, engine, devices, plugins)? {
                    self.open_outputs(store)?;
                    self.stage = Stage::Render(Box::new(capture));
                }
                Ok(Step::Working)
            }
            Stage::Render(r) => {
                // One engine block per unit.
                let done = r.step(frames);
                for (o, out) in self.outputs.iter_mut().enumerate() {
                    match out.track {
                        None => r.drain_main(&mut out.block),
                        Some(t) => r.drain_stem(t, &mut out.block),
                    }
                    if out.block.first().is_some_and(|c| !c.is_empty()) {
                        self.work.extend(
                            (0..out.chains.len()).map(|chain| Work::Feed { output: o, chain }),
                        );
                    }
                }
                if !done {
                    return Ok(Step::Working);
                }
                if let Some(why) = r.problem() {
                    let run = &self.runs[self.run];
                    let first = run.main.or(run.stems.first().map(|s| s.0)).unwrap_or(0);
                    self.warnings.push(
                        crate::text::msg("notice.export.incomplete")
                            .arg("file", self.passes[first].file_names[0].as_str())
                            .arg("error", why.to_string())
                            .into(),
                    );
                }
                // Drops the offline engine and its fresh nodes (plugins included).
                self.stage = Stage::Finish;
                for (o, out) in self.outputs.iter().enumerate() {
                    self.work
                        .extend((0..out.chains.len()).map(|chain| Work::End { output: o, chain }));
                }
                Ok(Step::Working)
            }
            Stage::Finish => {
                // Every chain ended; normalized signals still to encode.
                match self.next_held(0, 0) {
                    Some((output, chain)) => {
                        self.stage = Stage::Level(LevelAt {
                            output,
                            chain,
                            file: 0,
                            pos: 0,
                        })
                    }
                    None => return self.end_run(),
                }
                Ok(Step::Working)
            }
            Stage::Level(at) => {
                let at = *at;
                *frames = UNIT_FRAMES;
                self.level_unit(at, store)
            }
        }
    }

    /// Feed a chain its output's latest block, or end it.
    fn chain_work<S: ProjectStore>(
        &mut self,
        w: Work,
        store: &mut S,
        frames: &mut usize,
    ) -> Result<(), String> {
        match w {
            Work::Feed { output, chain } => {
                let Output { block, chains, .. } = &mut self.outputs[output];
                *frames = block.first().map_or(0, Vec::len);
                chains[chain].take(block)
            }
            Work::End { output, chain } => {
                let out = &mut self.outputs[output];
                let c = &mut out.chains[chain];
                c.end()?;
                if c.held.is_some() {
                    return Ok(());
                }
                let pass = out.pass;
                let files: Vec<(usize, Encoder)> = c
                    .files
                    .iter_mut()
                    .filter_map(|f| Some((f.format, f.encoder.take()?)))
                    .collect();
                for (format, encoder) in files {
                    self.deliver(store, pass, format, encoder.finish()?)?;
                    self.run_done += 1;
                }
                Ok(())
            }
        }
    }

    /// The first chain from (`output`, `chain`) on that holds a signal to normalize.
    fn next_held(&self, output: usize, chain: usize) -> Option<(usize, usize)> {
        let mut at = (output, chain);
        while at.0 < self.outputs.len() {
            let chains = &self.outputs[at.0].chains;
            if at.1 >= chains.len() {
                at = (at.0 + 1, 0);
            } else if chains[at.1].held.is_some() {
                return Some(at);
            } else {
                at.1 += 1;
            }
        }
        None
    }

    /// Normalize: encode [`UNIT_FRAMES`] more of a held signal into one of its files at the
    /// signal's gain; a finished file is delivered, a chain whose files are done drops its
    /// signal.
    fn level_unit<S: ProjectStore>(&mut self, at: LevelAt, store: &mut S) -> Result<Step, String> {
        let out = &mut self.outputs[at.output];
        let pass = out.pass;
        let c = &mut out.chains[at.chain];
        let (signal, peak) = c.held.as_ref().ok_or("export: no held signal")?;
        let len = signal.first().map_or(0, Vec::len);
        let end = (at.pos + UNIT_FRAMES).min(len);
        let file = &mut c.files[at.file];
        let encoder = file
            .encoder
            .as_mut()
            .ok_or("export: file already finished")?;
        let block: Vec<&[f32]> = signal.iter().map(|s| &s[at.pos..end]).collect();
        encoder.push(&block, normalize_gain(peak.peak()))?;
        if end < len {
            self.stage = Stage::Level(LevelAt { pos: end, ..at });
            return Ok(Step::Working);
        }
        let format = file.format;
        let encoder = file.encoder.take().expect("checked");
        let next_file = at.file + 1 < c.files.len();
        if !next_file {
            // Every file of the chain is encoded: free its signal.
            c.held = None;
        }
        self.deliver(store, pass, format, encoder.finish()?)?;
        self.run_done += 1;
        let next = if next_file {
            Some(LevelAt {
                file: at.file + 1,
                pos: 0,
                ..at
            })
        } else {
            self.next_held(at.output, at.chain + 1)
                .map(|(output, chain)| LevelAt {
                    output,
                    chain,
                    file: 0,
                    pos: 0,
                })
        };
        match next {
            Some(next) => {
                self.stage = Stage::Level(next);
                Ok(Step::Working)
            }
            None => self.end_run(),
        }
    }

    /// The run's files are all delivered: set up the next run, or end the job.
    fn end_run(&mut self) -> Result<Step, String> {
        self.outputs.clear();
        self.run += 1;
        if self.run >= self.runs.len() {
            // Format by format, request order, whichever render a file came from.
            let mut files = std::mem::take(&mut self.delivered);
            files.sort_by_key(|(key, _)| *key);
            return Ok(Step::Done(files.into_iter().map(|(_, d)| d).collect()));
        }
        self.stage = Stage::Setup(Box::new(self.new_setup()?));
        Ok(Step::Working)
    }

    fn master(&self) -> Option<TrackId> {
        self.project
            .tracks
            .values()
            .find(|t| t.kind == TrackKind::Master)
            .map(|t| t.id)
    }

    /// The current run renders one stem graph (`stem_graph`), not the project.
    fn is_stem(&self) -> bool {
        let run = &self.runs[self.run];
        let master = self.master();
        !run.full
            && run
                .main
                .and_then(|i| self.passes[i].stem)
                .is_some_and(|s| Some(s) != master)
    }

    fn run_stem(&self) -> Option<TrackId> {
        self.is_stem()
            .then(|| self.runs[self.run].main.and_then(|i| self.passes[i].stem))
            .flatten()
    }

    /// A fresh offline engine for the current pass, with the media registered; its device
    /// nodes are created by [`Job::step_setup`].
    fn new_setup(&self) -> Result<NodeSetup, String> {
        let p = &*self.project;
        let master = self.master();
        let is_stem = self.is_stem();
        let frozen = p.frozen_devices();
        let devices = p
            .devices
            .values()
            // Master chain excluded from stems.
            .filter(|d| !(is_stem && Some(d.track) == master))
            // Frozen devices: their track plays its render (`freeze-bounce`), no nodes.
            .filter(|d| !frozen.contains(&d.id))
            // Disabled plugins are bypassed anyway: no instance needed.
            .filter(|d| d.enabled || !matches!(d.kind, DeviceKind::Plugin { .. }))
            .map(|d| d.id)
            .collect();
        let stems = self.runs[self.run].stems.iter().map(|s| s.1).collect();
        Ok(
            NodeSetup::new(p, &self.media.media, devices, self.engine_rate, "export")?
                .with_capture(stems),
        )
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
        let stem = self.run_stem();
        let renderer = setup.finish(p, engine, self.start, |desc| {
            if let Some(s) = stem {
                stem_graph(desc, p, s);
            }
        })?;
        let run = &self.runs[self.run];
        // Streamed: the capture holds one block at a time.
        let capture = Capture::streamed(renderer, self.frames + self.tail);
        Ok(Some(if run.stems.is_empty() {
            capture
        } else {
            let tracks: Vec<TrackId> = run.stems.iter().map(|s| s.1).collect();
            capture.with_stems(&tracks, run.main.is_some())
        }))
    }

    /// The current run's signals (its main signal first, then the captured stems in request
    /// order), each with a chain per output rate and a file per format, its sink opened.
    fn open_outputs<S: ProjectStore>(&mut self, store: &mut S) -> Result<(), String> {
        let run = self.runs[self.run].clone();
        let total = self.frames + self.tail;
        let signals = run
            .main
            .map(|m| (m, None))
            .into_iter()
            .chain(run.stems.iter().map(|&(i, t)| (i, Some(t))));
        self.outputs.clear();
        for (pass, track) in signals {
            let mut chains: Vec<Chain> = Vec::new();
            for (format, f) in self.formats.clone().iter().enumerate() {
                let rate = output_rate(f, self.engine_rate);
                let at = match chains.iter().position(|c| c.rate == rate) {
                    Some(at) => at,
                    None => {
                        chains.push(self.new_chain(rate, total)?);
                        chains.len() - 1
                    }
                };
                let len = match &chains[at].resampler {
                    Some(r) => r.expected(),
                    None => total,
                };
                let name = self.passes[pass].file_names[format].clone();
                // In memory, a WAV's size is known: no reallocation as it grows.
                let capacity = match f.container {
                    AudioContainer::Wav => {
                        44 + len * CHANNELS * usize::from(encode::wav_bits(f.bit_depth)) / 8
                    }
                    _ => 0,
                };
                let sink = self.open_sink(store, &name, capacity)?;
                let encoder = Encoder::new(f.container, f.bit_depth, CHANNELS, rate, sink)?;
                chains[at].files.push(FileOut {
                    format,
                    encoder: Some(encoder),
                });
            }
            self.outputs.push(Output {
                pass,
                track,
                block: vec![Vec::new(); CHANNELS],
                chains,
            });
        }
        self.run_done = 0;
        self.run_files = (self.outputs.len() * self.formats.len()).max(1);
        Ok(())
    }

    fn new_chain(&self, rate: u32, total: usize) -> Result<Chain, String> {
        let resampler = if rate == self.engine_rate {
            None
        } else {
            Some(Box::new(
                StreamResampler::new(self.engine_rate, rate, CHANNELS, total)
                    .map_err(|e| format!("resampling failed: {e}"))?,
            ))
        };
        let len = resampler.as_ref().map_or(total, |r| r.expected());
        Ok(Chain {
            rate,
            resampler,
            resampled: vec![Vec::new(); CHANNELS],
            files: Vec::new(),
            held: self.request.normalize.then(|| {
                (
                    (0..CHANNELS).map(|_| Vec::with_capacity(len)).collect(),
                    PeakScan::default(),
                )
            }),
        })
    }

    /// Where a file is encoded: streamed into the store when it can, else memory.
    fn open_sink<S: ProjectStore>(
        &mut self,
        store: &mut S,
        name: &str,
        capacity: usize,
    ) -> Result<Sink, String> {
        if !self.downloads && self.streams {
            match store.create_export(self.project_id, name) {
                Ok(file) => return Ok(Sink::File(file)),
                Err(StoreError::Unsupported(_)) => self.streams = false,
                Err(e) => return Err(format!("could not write \"{name}\": {e}")),
            }
        }
        Ok(Sink::memory(capacity))
    }

    fn deliver<S: ProjectStore>(
        &mut self,
        store: &mut S,
        pass: usize,
        format: usize,
        sink: Sink,
    ) -> Result<(), String> {
        let key = (format, pass);
        let name = self.passes[pass].file_names[format].clone();
        let bytes = match sink {
            Sink::File(file) => {
                let path = store
                    .commit_export(self.project_id, &name, file)
                    .map_err(|e| format!("could not write \"{name}\": {e}"))?;
                self.delivered.push((key, Delivered::File(path)));
                return Ok(());
            }
            Sink::Memory(bytes) => bytes.into_inner(),
        };
        if !self.downloads {
            match store.write_export(self.project_id, &name, &bytes) {
                Ok(path) => {
                    self.delivered.push((key, Delivered::File(path)));
                    return Ok(());
                }
                Err(StoreError::Unsupported(_)) => self.downloads = true,
                Err(e) => return Err(format!("could not write \"{name}\": {e}")),
            }
        }
        let (_, mime) = encode::file_type(self.formats[format].container);
        let token = match format {
            0 => format!("{}-{pass}", self.id),
            f => format!("{}-{pass}-{f}", self.id),
        };
        self.delivered.push((
            key,
            Delivered::Download(
                ExportDownload {
                    token,
                    name,
                    mime: mime.into(),
                    size: bytes.len() as f64,
                },
                bytes,
            ),
        ));
        Ok(())
    }
}

/// File names of the passes of `request`, one per format: unique among themselves and
/// against `existing` (lower-cased names already in `exports/`), so an export never
/// overwrites a file; a collision gets a ` (2)`, ` (3)`, ... suffix (the same one in every
/// format). The formats have distinct containers (`validate`).
pub fn plan_passes(
    p: &Project,
    request: &ExportRequest,
    stems: &[TrackId],
    existing: &BTreeSet<String>,
) -> Vec<Pass> {
    let base = sanitize(request.name.as_deref().unwrap_or(&p.settings.name));
    let exts: Vec<&str> = formats(request)
        .iter()
        .map(|f| encode::file_type(f.container).0)
        .collect();
    let mut used = existing.clone();
    let mut unique = |stem: String| {
        let names = |s: &str| -> Vec<String> { exts.iter().map(|e| format!("{s}.{e}")).collect() };
        let mut names_now = names(&stem);
        let mut i = 2;
        while names_now.iter().any(|n| used.contains(&n.to_lowercase())) {
            names_now = names(&format!("{stem} ({i})"));
            i += 1;
        }
        for n in &names_now {
            used.insert(n.to_lowercase());
        }
        names_now
    };
    match &request.mode {
        ExportMode::Mix => vec![Pass {
            stem: None,
            file_names: unique(base),
            captured: false,
        }],
        ExportMode::Stems { .. } | ExportMode::MixAndStems { .. } => {
            let with_mix = matches!(request.mode, ExportMode::MixAndStems { .. });
            let can = capturable(p, stems);
            let mut passes = Vec::new();
            if with_mix {
                passes.push(Pass {
                    stem: None,
                    file_names: unique(base.clone()),
                    captured: false,
                });
            }
            passes.extend(stems.iter().map(|t| {
                let track = p.tracks.get(t).map_or("Track", |t| t.name.as_str());
                Pass {
                    stem: Some(*t),
                    file_names: unique(sanitize(&format!("{base} - {track}"))),
                    captured: can.contains(t),
                }
            }));
            passes
        }
    }
}

/// The stems among `stems` that one full render of the project can capture instead of a
/// pass each: the track's post-fader signal (before the mute/solo gate, so a muted track is
/// unmuted for its own stem, and solo is ignored) is then its stem exactly, because
/// nothing of the stem pass's rewrite (`stem_graph`) would change it:
/// - the project has no solo (the stem pass ignores it; the full render honors it);
/// - the master is neutral (not muted, unity fader, centered, none automated: a stem keeps
///   them, the capture sits before them) and no VCA is muted;
/// - the track and what is nested in it send nothing to returns (a stem includes the
///   returns' share) and take no other track's output as input (silent in a stem).
///
/// A return's tap is everything sent through it, which is its stem. The master is never
/// captured.
pub fn capturable(p: &Project, stems: &[TrackId]) -> BTreeSet<TrackId> {
    let Some(master) = p.tracks.values().find(|t| t.kind == TrackKind::Master) else {
        return BTreeSet::new();
    };
    let m = master.mixer;
    let master_automated = p.automation_lanes.values().any(|l| {
        l.enabled
            && matches!(l.owner, AutomationOwner::Track { .. })
            && matches!(
                l.target,
                AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track }
                    if track == master.id
            )
    });
    if m.mute
        || m.volume != Decibels::UNITY
        || m.pan.0 != 0.0
        || master_automated
        || p.tracks
            .values()
            .any(|t| t.mixer.solo || (t.kind == TrackKind::Vca && t.mixer.mute))
    {
        return BTreeSet::new();
    }
    stems
        .iter()
        .copied()
        .filter(|&s| {
            p.tracks
                .get(&s)
                .is_some_and(|t| t.kind != TrackKind::Master)
                && {
                    let keep = subtree(p, s);
                    !p.sends.values().any(|snd| keep.contains(&snd.from))
                        && !keep.iter().any(|id| {
                            p.tracks
                                .get(id)
                                .is_some_and(|t| matches!(t.input, TrackInput::Track { .. }))
                        })
                }
        })
        .collect()
}

/// Group the passes into renders, capturing at most `allowed` stems in the full render
/// (memory and open-file budgets: `Job::new`). The first run is the full render when the
/// mix or any stem is captured; every other stem has its own run. Stems without a mix only
/// share a render from two stems on.
fn plan_runs(passes: &mut [Pass], allowed: usize) -> Vec<Run> {
    let has_mix = passes.iter().any(|p| p.stem.is_none());
    let mut taken = 0;
    for p in passes.iter_mut().filter(|p| p.captured) {
        if taken < allowed {
            taken += 1;
        } else {
            p.captured = false;
        }
    }
    if !has_mix && taken < 2 {
        passes.iter_mut().for_each(|p| p.captured = false);
    }
    let stems: Vec<(usize, TrackId)> = passes
        .iter()
        .enumerate()
        .filter(|(_, p)| p.captured)
        .filter_map(|(i, p)| Some((i, p.stem?)))
        .collect();
    let mix = passes.iter().position(|p| p.stem.is_none());
    let mut runs = Vec::new();
    if mix.is_some() || !stems.is_empty() {
        runs.push(Run {
            full: true,
            main: mix,
            stems,
        });
    }
    for (i, p) in passes.iter().enumerate() {
        if p.stem.is_some() && !p.captured {
            runs.push(Run {
                full: false,
                main: Some(i),
                stems: Vec::new(),
            });
        }
    }
    runs
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

//! One render job (`Freeze`, `Bounce`, audio `Consolidate`): a list of passes, each
//! rendering one track in isolation on a private `OfflineRenderer` (the export job's
//! helpers, `crate::export::offline`) into a 32-bit float WAV in the project's `media/`.
//! Clips of a render get the engine's 64-sample anti-click ramps at their edges like any
//! clip: a freeze render starts at beat 0 and ends on padded silence, so its flattened clip
//! only differs from the frozen playback in the first 64 samples of the song.
//! Every step is a bounded unit of work, like export.

use std::collections::BTreeSet;

use ether_core::RenderGraphDesc;
use ether_core::graph::{AutomationDesc, ResolvedTarget};
use ether_core::protocol::export::{AudioContainer, BitDepth};
use ether_core::protocol::model::file::MEDIA_DIR;
use ether_core::protocol::model::*;

use crate::EngineBridge;
use crate::engine::EngineState;
use crate::export::encode::Encoder;
use crate::export::offline::{Capture, MediaLoader, NodeSetup, UNIT_FRAMES, tempo_rt};
use crate::store::{Library, ProjectStore};

/// Silence threshold of a freeze tail (-90 dBFS).
pub(crate) const TAIL_SILENCE: f32 = 3.162_277_7e-5;
/// A freeze tail grows by this much while the last chunk is not silent...
pub(crate) const TAIL_CHUNK_SECONDS: f64 = 0.5;
/// ...up to this long.
pub(crate) const MAX_TAIL_SECONDS: f64 = 30.0;
/// Silence kept after a trimmed tail, so a clip of the render (flatten) ends on silence: the
/// engine's anti-click ramp at clip edges (64 samples) then never touches the audio.
pub(crate) const TAIL_PAD_FRAMES: usize = 256;

/// What one pass renders.
#[derive(Clone, Debug)]
pub(crate) struct PassSpec {
    pub track: TrackId,
    /// First beat of the render (also the song time of media frame 0).
    pub start: Beats,
    /// Frames of the range (at the engine rate).
    pub frames: usize,
    /// Keep rendering past the range until silence (freeze).
    pub tail: bool,
    /// Post-chain (device chain included) or the clips only.
    pub chain: bool,
    /// Media id and display name of the result.
    pub media: MediaId,
    pub name: String,
}

/// A finished pass: the media written to the project store.
#[derive(Clone, Debug)]
pub(crate) struct Rendered {
    pub media: MediaRef,
}

enum Stage {
    Media,
    Setup(Box<NodeSetup>),
    Render(Box<Capture>),
    Encode {
        channels: Vec<Vec<f32>>,
        encoder: Encoder,
    },
}

pub(crate) enum Step {
    Working,
    Done,
}

pub(crate) struct RenderJob {
    pub id: String,
    pub project_id: ProjectId,
    project: Box<Project>,
    engine_rate: u32,
    passes: Vec<PassSpec>,
    pass: usize,
    media: MediaLoader,
    stage: Stage,
    pub results: Vec<Rendered>,
    /// User-facing warnings not reported yet.
    pub warnings: Vec<String>,
}

/// Tracks a pass needs besides `target`: the sources of its devices' sidechains
/// (transitively, with the tracks feeding them), so sidechains route like live playback.
/// Frozen tracks play their render (their devices are not needed).
pub(crate) fn needed_tracks(p: &Project, target: TrackId, chain: bool) -> BTreeSet<TrackId> {
    let mut set = BTreeSet::from([target]);
    if !chain {
        return set;
    }
    let mut stack = vec![target];
    while let Some(t) = stack.pop() {
        let Some(track) = p.tracks.get(&t) else {
            continue;
        };
        let mut feeds: Vec<TrackId> = Vec::new();
        if t != target {
            // A group or bus used as a sidechain source hears what feeds it.
            feeds.extend(p.tracks.values().filter_map(|c| {
                let into = c.parent == Some(t)
                    || matches!(c.output, TrackOutput::Track { track } if track == t);
                into.then_some(c.id)
            }));
        }
        if let TrackInput::Track { track, .. } = track.input {
            feeds.push(track);
        }
        if track.freeze.is_none() {
            feeds.extend(
                p.devices
                    .values()
                    .filter(|d| d.track == t)
                    .filter_map(|d| d.sidechain),
            );
        }
        for f in feeds {
            if p.tracks.contains_key(&f) && set.insert(f) {
                stack.push(f);
            }
        }
    }
    set
}

/// Devices a pass instantiates: those of the needed tracks (the target's only with its
/// chain), never of frozen tracks.
fn pass_devices(p: &Project, spec: &PassSpec, needed: &BTreeSet<TrackId>) -> Vec<DeviceId> {
    p.devices
        .values()
        .filter(|d| needed.contains(&d.track))
        .filter(|d| spec.chain || d.track != spec.track)
        .filter(|d| p.tracks.get(&d.track).is_some_and(|t| t.freeze.is_none()))
        // Disabled plugins are bypassed anyway: no instance needed.
        .filter(|d| d.enabled || !matches!(d.kind, DeviceKind::Plugin { .. }))
        .map(|d| d.id)
        .collect()
}

/// Media the passes need: audio clips and frozen renders of the needed tracks, samples of
/// their devices.
fn pass_media(p: &Project, passes: &[PassSpec]) -> Vec<MediaRef> {
    let mut ids = BTreeSet::new();
    for spec in passes {
        let needed = needed_tracks(p, spec.track, spec.chain);
        for c in p.clips.values().filter(|c| needed.contains(&c.track)) {
            if let ClipContent::Audio(a) = &c.content {
                ids.insert(a.media);
            }
        }
        for t in &needed {
            if let Some(f) = p.tracks.get(t).and_then(|t| t.freeze.as_ref()) {
                ids.insert(f.media);
            }
        }
        for d in pass_devices(p, spec, &needed) {
            if let Some(DeviceKind::Builtin { device }) = p.devices.get(&d).map(|d| &d.kind) {
                ids.extend(device.media());
            }
        }
    }
    ids.into_iter()
        .filter_map(|m| p.media.get(&m).cloned())
        .collect()
}

fn mixer_target(a: &AutomationDesc) -> bool {
    matches!(
        a.resolved,
        ResolvedTarget::TrackVolume | ResolvedTarget::TrackPan | ResolvedTarget::Send { .. }
    )
}

/// Rewrite the mix graph so the master output is `target`'s signal alone: post-chain (or
/// clips only without `chain`), **pre-fader** (unity volume, centre pan, unmuted, no sends,
/// no mixer automation), straight into a neutral master (no devices, unity). Needed tracks
/// (sidechain sources) keep playing but reach no output; every other source is silent.
pub(crate) fn isolate(
    desc: &mut RenderGraphDesc,
    target: TrackId,
    needed: &BTreeSet<TrackId>,
    chain: bool,
) {
    let master = desc
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Master)
        .map(|t| t.id);
    for t in &mut desc.tracks {
        t.solo = false;
        if t.id == target {
            t.output = master;
            t.group = None;
            t.volume = 1.0;
            t.pan = 0.0;
            t.mute = false;
            t.sends.clear();
            t.vca = None;
            t.automation.retain(|a| !mixer_target(a));
            for c in &mut t.clips {
                c.envelopes.retain(|a| !mixer_target(a));
            }
            if !chain {
                t.chain.clear();
                t.racks.clear();
                t.chain_racks.clear();
                t.modulation = Default::default();
            }
        } else if t.kind == TrackKind::Master {
            t.chain.clear();
            t.racks.clear();
            t.chain_racks.clear();
            t.modulation = Default::default();
            t.clips.clear();
            t.automation.clear();
            t.volume = 1.0;
            t.pan = 0.0;
            t.mute = false;
            t.vca = None;
            t.input_tap = None;
        } else if needed.contains(&t.id) {
            t.sends.clear();
            if t.output.is_some_and(|o| !needed.contains(&o)) {
                t.output = None;
            }
        } else {
            t.output = None;
            t.sends.clear();
            t.clips.clear();
            t.frozen = None;
            t.input_tap = None;
        }
    }
}

/// Peak of `channels[..][from..]`.
fn peak_from(channels: &[Vec<f32>], from: usize) -> f32 {
    channels
        .iter()
        .flat_map(|c| c.get(from..).unwrap_or(&[]))
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Drop trailing frames below the tail threshold, keeping at least `keep` frames (and at
/// least one frame).
pub(crate) fn trim_tail(channels: &mut [Vec<f32>], keep: usize) {
    let len = channels.first().map_or(0, Vec::len);
    let mut end = len;
    while end > keep.max(1) && channels.iter().all(|c| c[end - 1].abs() < TAIL_SILENCE) {
        end -= 1;
    }
    for c in channels {
        c.truncate(end);
    }
}

/// `name` made safe as a file-name segment of `media/<id>-<name>`.
fn file_segment(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    clean.trim_start_matches('.').to_string()
}

impl RenderJob {
    pub fn new(id: String, p: &Project, engine_rate: u32, passes: Vec<PassSpec>) -> Self {
        Self {
            id,
            project_id: p.id,
            media: MediaLoader::new(p.id, pass_media(p, &passes), engine_rate),
            project: Box::new(p.clone()),
            engine_rate,
            passes,
            pass: 0,
            stage: Stage::Media,
            results: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Frames of `[start, end)` at the engine rate (song time).
    pub fn frames_of(p: &Project, start: Beats, end: Beats, engine_rate: u32) -> usize {
        let tempo = tempo_rt(p);
        let rate = engine_rate as f64;
        let s0 = (tempo.beats_to_seconds(start.0) * rate).round();
        let s1 = (tempo.beats_to_seconds(end.0) * rate).round();
        (s1 - s0).max(0.0) as usize
    }

    /// Overall progress, 0..=1.
    pub fn progress(&self) -> f32 {
        let n = self.passes.len().max(1) as f32;
        let within = match &self.stage {
            Stage::Media | Stage::Setup(_) => 0.0,
            Stage::Render(r) => 0.9 * r.captured() as f32 / r.total().max(1) as f32,
            Stage::Encode { channels, encoder } => {
                0.9 + 0.1 * encoder.progress(channels.first().map_or(0, Vec::len))
            }
        };
        ((self.pass as f32 + within) / n).clamp(0.0, 1.0)
    }

    fn spec(&self) -> &PassSpec {
        &self.passes[self.pass]
    }

    fn tail_chunk(&self) -> usize {
        (TAIL_CHUNK_SECONDS * self.engine_rate as f64).round() as usize
    }

    /// Do one unit of work.
    pub fn step<B: EngineBridge, S: ProjectStore, L: Library>(
        &mut self,
        bridge: &mut B,
        engine: &EngineState,
        store: &mut S,
        library: &mut L,
    ) -> Result<Step, String> {
        match &mut self.stage {
            Stage::Media => {
                let mut frames = 0;
                if self.media.step(store, library, &mut frames)? {
                    self.stage = self.new_setup()?;
                }
                Ok(Step::Working)
            }
            Stage::Setup(setup) => {
                let (mut devices, mut plugins) = (0, 0);
                let p = &*self.project;
                if !setup.step(
                    p,
                    bridge,
                    self.engine_rate,
                    &mut self.warnings,
                    &mut devices,
                    &mut plugins,
                )? {
                    return Ok(Step::Working);
                }
                let Stage::Setup(setup) = std::mem::replace(&mut self.stage, Stage::Media) else {
                    unreachable!()
                };
                let spec = self.passes[self.pass].clone();
                let needed = needed_tracks(p, spec.track, spec.chain);
                let renderer = setup.finish(p, engine, spec.start, |desc| {
                    isolate(desc, spec.track, &needed, spec.chain)
                })?;
                let tail = if spec.tail { self.tail_chunk() } else { 0 };
                self.stage = Stage::Render(Box::new(Capture::new(renderer, spec.frames + tail)));
                Ok(Step::Working)
            }
            Stage::Render(r) => {
                let mut frames = 0;
                if !r.step(&mut frames) {
                    return Ok(Step::Working);
                }
                let spec = &self.passes[self.pass];
                let chunk = (TAIL_CHUNK_SECONDS * self.engine_rate as f64).round() as usize;
                let max_tail = (MAX_TAIL_SECONDS * self.engine_rate as f64).round() as usize;
                if spec.tail {
                    let rendered = r.captured();
                    let last = rendered.saturating_sub(chunk).max(spec.frames);
                    let tail_so_far = rendered - spec.frames;
                    if tail_so_far < max_tail && peak_from(r.channels(), last) >= TAIL_SILENCE {
                        r.extend(chunk.min(max_tail - tail_so_far));
                        return Ok(Step::Working);
                    }
                }
                if let Some(why) = r.problem() {
                    self.warnings.push(format!(
                        "\"{}\": the render may be incomplete ({why})",
                        spec.name
                    ));
                }
                let keep = spec.frames;
                let tail = spec.tail;
                let Stage::Render(r) = std::mem::replace(&mut self.stage, Stage::Media) else {
                    unreachable!()
                };
                // Drops the offline engine and its fresh nodes (plugins included).
                let mut channels = r.into_channels();
                if tail {
                    trim_tail(&mut channels, keep);
                    for c in &mut channels {
                        c.resize(c.len() + TAIL_PAD_FRAMES, 0.0);
                    }
                }
                let frames = channels.first().map_or(0, Vec::len);
                if frames == 0 {
                    return Err("nothing to render: the range is empty".into());
                }
                let encoder = Encoder::new(
                    AudioContainer::Wav,
                    BitDepth::Float32,
                    channels.len(),
                    frames,
                    self.engine_rate,
                )?;
                self.stage = Stage::Encode { channels, encoder };
                Ok(Step::Working)
            }
            Stage::Encode { channels, encoder } => {
                let Some(bytes) = encoder.step(channels, 1.0, UNIT_FRAMES)? else {
                    return Ok(Step::Working);
                };
                let frames = channels.first().map_or(0, Vec::len) as u64;
                let n_channels = channels.len() as u16;
                self.stage = Stage::Media;
                let spec = self.spec().clone();
                let file = format!(
                    "{MEDIA_DIR}/{}-{}.wav",
                    spec.media,
                    file_segment(&spec.name)
                );
                store
                    .write(self.project_id, &file, &bytes)
                    .map_err(|e| format!("could not write \"{file}\": {e}"))?;
                self.results.push(Rendered {
                    media: MediaRef {
                        location: Default::default(),
                        id: spec.media,
                        name: format!("{}.wav", spec.name),
                        file,
                        sample_rate: self.engine_rate,
                        channels: n_channels,
                        frames,
                        hash: None,
                    },
                });
                self.pass += 1;
                if self.pass >= self.passes.len() {
                    return Ok(Step::Done);
                }
                self.stage = self.new_setup()?;
                Ok(Step::Working)
            }
        }
    }

    fn new_setup(&self) -> Result<Stage, String> {
        let p = &*self.project;
        let spec = self.spec();
        let needed = needed_tracks(p, spec.track, spec.chain);
        let devices = pass_devices(p, spec, &needed);
        Ok(Stage::Setup(Box::new(NodeSetup::new(
            p,
            &self.media.media,
            devices,
            self.engine_rate,
            "render",
        )?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_silent_tail_but_keeps_the_range() {
        let mut ch = vec![vec![0.5, 0.0, 0.2, 0.0, 0.0, 0.0], vec![0.0; 6]];
        trim_tail(&mut ch, 2);
        assert_eq!(ch[0], vec![0.5, 0.0, 0.2]);
        let mut ch = vec![vec![0.0; 6], vec![0.0; 6]];
        trim_tail(&mut ch, 4);
        assert_eq!(ch[0].len(), 4);
        let mut ch = vec![vec![0.0; 6]];
        trim_tail(&mut ch, 0);
        assert_eq!(ch[0].len(), 1);
    }

    #[test]
    fn file_segments_are_safe() {
        assert_eq!(file_segment("Bass Freeze"), "Bass_Freeze");
        assert_eq!(file_segment("../x"), "_x");
        assert_eq!(file_segment(&"a".repeat(200)).len(), 80);
    }
}

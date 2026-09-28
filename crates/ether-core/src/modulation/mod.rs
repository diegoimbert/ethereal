//! Engine-side modulation (v0.2, owned by the `racks-modulation` node; model
//! `ether_model::modulation`, composition rule frozen in CONTRACTS.md §12.6).
//!
//! The controller compiles every track's modulators and mappings into
//! `TrackDesc::modulation` ([`ModulationDesc`]). Modulators run in the engine (not in the
//! host node): LFOs/steps/random from the transport, envelopes from the notes reaching the
//! host node, envelope followers from the audio at the host's input (or a sidechain tap).
//!
//! Hooks (wired in `engine.rs` / `crate::automation_rt`):
//! - [`ModulationRt::intercept`] / [`ModulationRt::intercept_at`]: every base-value write for
//!   a node param (live changes from the param queue, arrangement automation, clip
//!   envelopes, macro knob changes) goes through it first. `true` = the param is modulated
//!   (or is a macro): the value is stored as the **base** (at its sample offset) and the
//!   engine does not forward it; the modulation output will.
//! - [`ModulationRt::render`]: once per sub-block after automation, before the device chain:
//!   computes the sources and pushes `EventKind::Param` events with
//!   `clamp(base + Σ depth·m, 0, 1)` (mapped to plain, stepped params snapped) into the
//!   targets' event lists at the automation grid points (`automation_rt::grid`, CONTRACTS.md
//!   §12.7) and at every base/macro change, only when the plain value changes. After a
//!   snapshot swap every target is re-sent once at offset 0.
//! - [`ModulationRt::pre_node`]: before chain entry `k` processes: envelope followers read
//!   the host's input audio; envelopes/keytrack of hosts after entry 0 read the entry's
//!   notes there (hosts at entry 0 read them in `render`, sample-accurately).
//! - [`ModulationRt::write_sidechain`]: followers keyed from another track, PDC-aligned.
//! - [`readback`]: at the analysis rate after the jobs, push `AnalysisKind::Modulation`
//!   frames (normalized base/effective per modulated param) for the depth rings, at most
//!   [`READBACK_FRAMES_PER_PASS`] per pass (round-robin), so device analysis frames keep
//!   room in the ring.
//! - Live modulator param changes: `ParamTarget::Modulator` → [`ModulationRt::set_param`].
//!
//! Sources (values: bipolar `-1..=1` for LFO/steps/random, unipolar `0..=1` otherwise):
//! - **LFO**: free (Hz, phase accumulated per sample) or synced (phase locked to the beat
//!   while playing); retrigger `Note` restarts it at every note-on reaching the host,
//!   `Transport` at play; `Fade In` ramps the depth after a (re)trigger.
//! - **Envelope**: ADSR (linear attack, exponential decay/release) retriggered by every
//!   note-on reaching the host, released by the most recent note's note-off; `Velocity`
//!   scales it by the note velocity.
//! - **Envelope follower**: peak detector with attack/release and gain.
//! - **Steps**: tempo-synced step sequencer (holds while stopped), `Smooth` glides.
//! - **Random**: sample-and-hold, values derived from a hash of the period index (so
//!   renders are deterministic and block-size independent), `Smooth` glides.
//! - **Keytrack / Velocity**: the last note-on's key / velocity reaching the host, placed
//!   between the kind's Low (0) and High (1) params (held until the next note).
//! - **Macros**: the rack's macro value.
//!
//! RT rules apply (no allocation after compile); compiles on wasm32.

use ether_protocol::model::{ModulatorId, ModulatorKind, ParamId, TrackId};
use serde::{Deserialize, Serialize};

use crate::analysis::{AnalysisFrame, AnalysisKind, AnalysisRt};
use crate::automation::to_plain;
use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::drum_rack::RacksRt;
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::ParamMapping;
use crate::mixer::{ChainRt, Stereo, TrackRt};
use crate::node::NodeKey;
use crate::rack_chains::ChainRacksRt;
use crate::sched::Timing;
use crate::transport::TransportInfo;

/// Most params of a modulator kind (Steps has 19).
pub const MAX_MOD_PARAMS: usize = 24;
/// Macros per rack (`ether_model::RACK_MACROS`).
pub const MACROS: usize = 8;
/// Readback frames pushed per analysis pass (all tracks together).
pub const READBACK_FRAMES_PER_PASS: usize = 16;
/// Modulated params per readback frame (triples).
const READBACK_PARAMS: usize = crate::analysis::ANALYSIS_MAX_VALUES / 3;
/// A readback frame is re-sent at least every this many passes even when nothing changed
/// (so a device watched later gets its rings).
const READBACK_REFRESH: u32 = 15;
/// Base/macro changes remembered per target and sub-block (later ones replace the last).
const MAX_CHANGES: usize = 16;
/// Length in beats of each sync rate (`ether_devices::contract::SYNC_RATES`).
pub const SYNC_RATE_BEATS: [f64; 14] = [
    1.0 / 16.0,
    1.0 / 12.0,
    1.0 / 8.0,
    1.0 / 6.0,
    1.0 / 4.0,
    1.0 / 3.0,
    1.0 / 2.0,
    2.0 / 3.0,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
    32.0,
];

/// A track's modulation (empty for most tracks).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModulationDesc {
    pub modulators: Vec<ModulatorDesc>,
    pub mappings: Vec<ModMappingDesc>,
    /// Macro values (plain `0..=1`) of the racks whose macros are mapped.
    #[serde(default)]
    pub macros: Vec<MacroDesc>,
}

impl ModulationDesc {
    pub fn is_empty(&self) -> bool {
        self.modulators.is_empty() && self.mappings.is_empty() && self.macros.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModulatorDesc {
    pub id: ModulatorId,
    /// The host device's node (on this track's chain, a pad chain or a rack chain).
    pub host: NodeKey,
    pub kind: ModulatorKind,
    /// Plain values of every param of the kind (defaults filled in by the controller).
    pub params: Vec<(ParamId, f64)>,
    /// Envelope followers: follow this track's sidechain tap instead of the host input
    /// (ordered first and tapped by `graph.rs`, read in the consumer's job via
    /// [`ModulationRt::write_sidechain`]).
    #[serde(default)]
    pub sidechain: Option<ether_protocol::model::TrackId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModSourceDesc {
    /// Index into `ModulationDesc::modulators`.
    Modulator(u32),
    /// Macro `index` of the rack node `rack` (its param `index`, plain 0..=1).
    Macro { rack: NodeKey, index: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModMappingDesc {
    pub source: ModSourceDesc,
    /// Target node and param.
    pub node: NodeKey,
    pub param: ParamId,
    /// `-1..=1` (normalized units).
    pub depth: f64,
    /// Normalized → plain mapping of the target param.
    pub mapping: ParamMapping,
    /// Normalized base value at compile time (the document value; automation overrides it
    /// through `intercept`).
    pub base: f64,
}

/// The macro values of a rack node (plain = normalized, `0..=1`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacroDesc {
    pub rack: NodeKey,
    pub values: [f64; MACROS],
}

/// PDC inputs of envelope-follower sidechains (from `graph.rs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidechainLatency {
    /// Per sidechain source (same order as `ModulationRt::sidechain_sources`): its track id
    /// and output latency (`out_lat`).
    pub sources: Vec<(TrackId, u32)>,
    /// Input position (latency) of every track-chain entry.
    pub host_in: Vec<(NodeKey, u32)>,
}

// --- sources ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum EnvStage {
    #[default]
    Idle,
    Attack,
    Decay,
    Release,
}

#[derive(Clone, Copy, Debug)]
struct SourceState {
    /// LFO phase `0..1` (free mode), steps/random smoothing state.
    phase: f64,
    /// Samples since the last (re)trigger (LFO fade in).
    since: f64,
    /// Beat of the last note trigger (synced LFO with note retrigger).
    trigger_beat: f64,
    env_stage: EnvStage,
    env_level: f64,
    env_velocity: f64,
    /// Most recent note id (envelope release).
    env_note: Option<u32>,
    /// Follower level (linear peak, before gain).
    follower: f64,
    /// Current output.
    value: f64,
}

impl Default for SourceState {
    fn default() -> Self {
        Self {
            phase: 0.0,
            since: 0.0,
            trigger_beat: 0.0,
            env_stage: EnvStage::Idle,
            env_level: 0.0,
            env_velocity: 1.0,
            env_note: None,
            follower: 0.0,
            value: 0.0,
        }
    }
}

#[derive(Debug)]
struct Source {
    id: ModulatorId,
    host: NodeKey,
    kind: ModulatorKind,
    p: [f64; MAX_MOD_PARAMS],
    st: SourceState,
    /// Index into `sidechain_sources` (followers keyed from another track).
    sidechain: Option<usize>,
    /// Aligns the sidechain tap with the host input (PDC).
    sc_delay: DelayLine,
    sc_buf: Stereo,
}

/// Deterministic hash of a period index → `-1..=1`.
fn hash_unit(seed: u64, k: i64) -> f64 {
    let mut x = seed ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

fn lfo_shape(shape: f64, phase: f64) -> f64 {
    let x = phase - phase.floor();
    match shape.round() as i64 {
        1 => {
            // Triangle: 0 at phase 0, rising.
            let t = (x + 0.25) % 1.0;
            if t < 0.5 {
                4.0 * t - 1.0
            } else {
                3.0 - 4.0 * t
            }
        }
        2 => 2.0 * x - 1.0,
        3 => 1.0 - 2.0 * x,
        4 => {
            if x < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => (x * std::f64::consts::TAU).sin(),
    }
}

/// Smoothstep glide from `a` to `b` over the last `smooth` fraction of a period at `frac`.
fn glide(a: f64, b: f64, frac: f64, smooth: f64) -> f64 {
    if smooth <= 0.0 {
        return b;
    }
    let t = (frac / smooth).clamp(0.0, 1.0);
    a + (b - a) * t * t * (3.0 - 2.0 * t)
}

fn sync_beats(index: f64) -> f64 {
    SYNC_RATE_BEATS[(index.round().max(0.0) as usize).min(SYNC_RATE_BEATS.len() - 1)]
}

/// Transport state at a knot.
#[derive(Clone, Copy, Debug)]
struct Clock {
    playing: bool,
    beat: f64,
    sr: f64,
}

impl Source {
    fn seed(&self) -> u64 {
        (self.id.0.0 as u64) ^ ((self.id.0.0 >> 64) as u64)
    }

    /// A note-on reached the host.
    fn note_on(&mut self, note_id: u32, key: u8, velocity: f32, clock: Clock) {
        match self.kind {
            ModulatorKind::Lfo => {
                if self.p[5].round() as i64 == 1 {
                    self.st.phase = 0.0;
                    self.st.since = 0.0;
                    self.st.trigger_beat = clock.beat;
                }
            }
            ModulatorKind::Keytrack | ModulatorKind::Velocity => {
                self.st.env_velocity = velocity.clamp(0.0, 1.0) as f64;
                self.st.env_note = Some(key as u32);
            }
            ModulatorKind::Envelope => {
                self.st.env_stage = EnvStage::Attack;
                self.st.env_velocity = velocity.clamp(0.0, 1.0) as f64;
                self.st.env_note = Some(note_id);
                if self.p[0] <= 0.0 {
                    self.st.env_level = 1.0;
                    self.st.env_stage = EnvStage::Decay;
                }
            }
            _ => {}
        }
    }

    fn note_off(&mut self, note_id: Option<u32>) {
        if self.kind == ModulatorKind::Envelope
            && self.st.env_stage != EnvStage::Idle
            && (note_id.is_none() || note_id == self.st.env_note)
        {
            self.st.env_stage = EnvStage::Release;
            self.st.env_note = None;
        }
    }

    fn notes(&mut self, e: &ProcessEvent, clock: Clock) {
        match e.kind {
            EventKind::NoteOn {
                note_id,
                key,
                velocity,
                ..
            } => self.note_on(note_id, key, velocity, clock),
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                self.note_off(Some(note_id))
            }
            EventKind::AllNotesOff => self.note_off(None),
            EventKind::Midi { data } if data[0] & 0xf0 == 0x90 && data[2] > 0 => {
                self.note_on(u32::MAX, data[1], data[2] as f32 / 127.0, clock)
            }
            _ => {}
        }
    }

    fn uses_notes(&self) -> bool {
        matches!(
            self.kind,
            ModulatorKind::Envelope | ModulatorKind::Keytrack | ModulatorKind::Velocity
        ) || (self.kind == ModulatorKind::Lfo && self.p[5].round() as i64 == 1)
    }

    /// Advance by `k` samples, ending at `clock`, and update `value`.
    fn advance(&mut self, k: f64, clock: Clock) {
        let sr = clock.sr;
        match self.kind {
            ModulatorKind::Lfo => {
                let synced = self.p[2] >= 0.5;
                self.st.since += k;
                let phase = if synced && clock.playing {
                    let len = sync_beats(self.p[3]);
                    let origin = if self.p[5].round() as i64 == 1 {
                        self.st.trigger_beat
                    } else {
                        0.0
                    };
                    self.st.phase = ((clock.beat - origin) / len).rem_euclid(1.0);
                    self.st.phase
                } else {
                    self.st.phase = (self.st.phase + k * self.p[1].max(0.0) / sr).rem_euclid(1.0);
                    self.st.phase
                };
                let fade_samples = self.p[6].max(0.0) * 0.001 * sr;
                let fade = if fade_samples > 0.0 {
                    (self.st.since / fade_samples).min(1.0)
                } else {
                    1.0
                };
                self.st.value = lfo_shape(self.p[0], phase + self.p[4] / 360.0) * fade;
            }
            ModulatorKind::Envelope => {
                let mut left = k;
                let ms = |v: f64| (v.max(0.0) * 0.001 * sr).max(1.0);
                let sustain = (self.p[2] / 100.0).clamp(0.0, 1.0);
                while left > 0.0 {
                    match self.st.env_stage {
                        EnvStage::Idle => {
                            self.st.env_level = 0.0;
                            break;
                        }
                        EnvStage::Attack => {
                            let rate = 1.0 / ms(self.p[0]);
                            let need = (1.0 - self.st.env_level) / rate;
                            if need <= left {
                                left -= need;
                                self.st.env_level = 1.0;
                                self.st.env_stage = EnvStage::Decay;
                            } else {
                                self.st.env_level += rate * left;
                                left = 0.0;
                            }
                        }
                        EnvStage::Decay => {
                            // Exponential towards sustain (time constant = decay / 5).
                            let c = (-5.0 / ms(self.p[1])).exp();
                            self.st.env_level =
                                sustain + (self.st.env_level - sustain) * c.powf(left);
                            left = 0.0;
                        }
                        EnvStage::Release => {
                            let c = (-5.0 / ms(self.p[3])).exp();
                            self.st.env_level *= c.powf(left);
                            if self.st.env_level < 1e-5 {
                                self.st.env_level = 0.0;
                                self.st.env_stage = EnvStage::Idle;
                            }
                            left = 0.0;
                        }
                    }
                }
                let vel = (self.p[4] / 100.0).clamp(0.0, 1.0);
                let scale = 1.0 - vel * (1.0 - self.st.env_velocity);
                self.st.value = (self.st.env_level * scale).clamp(0.0, 1.0);
            }
            ModulatorKind::Keytrack | ModulatorKind::Velocity => {
                // Position of the last key / velocity between Low (0) and High (1).
                let Some(key) = self.st.env_note else {
                    return;
                };
                let x = if self.kind == ModulatorKind::Keytrack {
                    key as f64
                } else {
                    (self.st.env_velocity * 127.0).round()
                };
                let (lo, hi) = (self.p[0], self.p[1]);
                self.st.value = if hi == lo {
                    if x >= hi { 1.0 } else { 0.0 }
                } else {
                    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
                };
            }
            ModulatorKind::EnvelopeFollower => {
                let gain = 10f64.powf(self.p[2] / 20.0);
                self.st.value = (self.st.follower * gain).clamp(0.0, 1.0);
            }
            ModulatorKind::Steps => {
                if clock.playing {
                    let steps = self.p[0].round().clamp(1.0, 16.0) as i64;
                    let len = sync_beats(self.p[1]);
                    let pos = clock.beat / len;
                    let i = pos.floor() as i64;
                    let frac = pos - pos.floor();
                    let cur = self.p[3 + i.rem_euclid(steps) as usize].clamp(-1.0, 1.0);
                    let prev = self.p[3 + (i - 1).rem_euclid(steps) as usize].clamp(-1.0, 1.0);
                    self.st.value = glide(prev, cur, frac, self.p[2] / 100.0);
                }
            }
            ModulatorKind::Random => {
                let synced = self.p[1] >= 0.5;
                let pos = if synced {
                    if !clock.playing {
                        return;
                    }
                    clock.beat / sync_beats(self.p[2])
                } else {
                    // Free: period index from the phase accumulator (rate changes glide).
                    self.st.phase += k * self.p[0].max(0.0) / sr;
                    self.st.phase
                };
                let i = pos.floor() as i64;
                let frac = pos - pos.floor();
                let seed = self.seed();
                self.st.value = glide(
                    hash_unit(seed, i - 1),
                    hash_unit(seed, i),
                    frac,
                    self.p[3] / 100.0,
                );
            }
        }
    }

    /// Envelope follower: detect `n` samples of `l`/`r`.
    fn follow(&mut self, l: &[f32], r: &[f32], sr: f64) {
        let coef = |ms: f64| 1.0 - (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp();
        let (att, rel) = (coef(self.p[0]), coef(self.p[1]));
        let mut env = self.st.follower;
        for (a, b) in l.iter().zip(r) {
            let x = a.abs().max(b.abs()) as f64;
            let x = if x.is_finite() { x } else { 0.0 };
            env += (x - env) * if x > env { att } else { rel };
        }
        self.st.follower = if env < 1e-9 { 0.0 } else { env };
    }
}

// --- targets ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Src {
    Mod(u16),
    Macro(u16, u8),
}

#[derive(Debug)]
struct Link {
    src: Src,
    depth: f64,
}

#[derive(Debug)]
struct Target {
    node: NodeKey,
    param: ParamId,
    mapping: ParamMapping,
    /// Normalized base.
    base: f64,
    changes: [(u32, f64); MAX_CHANGES],
    n_changes: usize,
    links: Vec<Link>,
    /// Last plain value sent (NaN = send again).
    last: f64,
    /// Last normalized effective value (readback).
    effective: f64,
}

#[derive(Debug)]
struct MacroRt {
    rack: NodeKey,
    values: [f64; MACROS],
    changes: [(u32, u8, f64); MAX_CHANGES],
    n_changes: usize,
}

fn push_change<T: Copy>(list: &mut [T; MAX_CHANGES], n: &mut usize, v: T) {
    if *n < MAX_CHANGES {
        list[*n] = v;
        *n += 1;
    } else {
        list[MAX_CHANGES - 1] = v;
    }
}

/// Per-track modulation state (in `TrackRt`).
#[derive(Debug, Default)]
pub(crate) struct ModulationRt {
    /// Track indices of envelope-follower sidechain sources (distinct, sorted).
    sidechain_sources: Vec<usize>,
    sources: Vec<Source>,
    macros: Vec<MacroRt>,
    targets: Vec<Target>,
    /// Knot offsets of the current sub-block (preallocated).
    knots: Vec<u32>,
    /// Note events of entry-0 hosts, sorted (preallocated).
    notes: Vec<ProcessEvent>,
    sample_rate: f64,
    was_playing: bool,
    /// Readback: passes since the last frame per target group, and a change flag.
    readback_age: u32,
    readback_dirty: bool,
}

impl ModulationRt {
    /// Non-RT (graph compile). `sidechain_sources`: track indices of the envelope-follower
    /// sidechain sources of this track (finished before its job; their `tap` is kept).
    /// Non-RT (graph compile). `sidechain_sources`: track indices of the envelope-follower
    /// sidechain sources of this track (finished before its job; their `tap` is kept);
    /// `latency`: their ids and output latencies, and the host entries' input positions.
    pub(crate) fn compile(
        desc: &ModulationDesc,
        sidechain_sources: Vec<usize>,
        latency: SidechainLatency,
        config: &EngineConfig,
    ) -> Self {
        let frames = config.max_block_size;
        let sr = config.sample_rate.max(1) as f64;
        let host_in = |node: NodeKey| {
            latency
                .host_in
                .iter()
                .find(|(k, _)| *k == node)
                .map_or(0, |e| e.1)
        };
        let sources = desc
            .modulators
            .iter()
            .map(|m| {
                let mut p = [0.0; MAX_MOD_PARAMS];
                for &(id, v) in &m.params {
                    if let Some(slot) = p.get_mut(id.0 as usize) {
                        *slot = v;
                    }
                }
                // Sidechain source index and PDC delay (source earlier than the host: delay
                // the tap; later: the modulation lags).
                let sidechain = m
                    .sidechain
                    .filter(|_| m.kind == ModulatorKind::EnvelopeFollower)
                    .and_then(|t| latency.sources.iter().position(|(id, _)| *id == t))
                    .filter(|&i| i < sidechain_sources.len());
                let delay = sidechain
                    .and_then(|i| latency.sources.get(i))
                    .map_or(0, |&(_, l_sc)| host_in(m.host).saturating_sub(l_sc));
                Source {
                    id: m.id,
                    host: m.host,
                    kind: m.kind,
                    p,
                    st: SourceState::default(),
                    sidechain,
                    sc_delay: DelayLine::new(delay as usize),
                    sc_buf: if sidechain.is_some() {
                        crate::mixer::stereo(frames)
                    } else {
                        [Vec::new(), Vec::new()]
                    },
                }
            })
            .collect();
        let macros: Vec<MacroRt> = desc
            .macros
            .iter()
            .map(|m| MacroRt {
                rack: m.rack,
                values: m.values.map(|v| v.clamp(0.0, 1.0)),
                changes: [(0, 0, 0.0); MAX_CHANGES],
                n_changes: 0,
            })
            .collect();
        let mut targets: Vec<Target> = Vec::new();
        for m in &desc.mappings {
            let src = match m.source {
                ModSourceDesc::Modulator(i) if (i as usize) < desc.modulators.len() => {
                    Src::Mod(i as u16)
                }
                ModSourceDesc::Macro { rack, index } if (index as usize) < MACROS => {
                    match macros.iter().position(|r| r.rack == rack) {
                        Some(r) => Src::Macro(r as u16, index),
                        None => continue,
                    }
                }
                _ => continue,
            };
            let link = Link {
                src,
                depth: m.depth.clamp(-1.0, 1.0),
            };
            match targets
                .iter_mut()
                .find(|t| t.node == m.node && t.param == m.param)
            {
                Some(t) => t.links.push(link),
                None => targets.push(Target {
                    node: m.node,
                    param: m.param,
                    mapping: m.mapping,
                    base: m.base.clamp(0.0, 1.0),
                    changes: [(0, 0.0); MAX_CHANGES],
                    n_changes: 0,
                    links: vec![link],
                    last: f64::NAN,
                    effective: m.base.clamp(0.0, 1.0),
                }),
            }
        }
        targets.sort_by_key(|t| (t.node, t.param));
        Self {
            sidechain_sources,
            sources,
            macros,
            targets,
            knots: Vec::with_capacity(frames / crate::automation_rt::PARAM_GRID as usize + 2 + 64),
            notes: Vec::with_capacity(config.max_events_per_block),
            sample_rate: sr,
            was_playing: false,
            readback_age: READBACK_REFRESH,
            readback_dirty: true,
        }
    }

    /// Envelope-follower sidechain sources (track indices).
    pub(crate) fn sidechain_sources(&self) -> &[usize] {
        &self.sidechain_sources
    }

    /// RT. At the start of the consumer's job: the post-fader tap of source track `source`
    /// (latency `out_lat(source)`), for the followers keyed from it. PDC: aligned to the host
    /// device's input like a device sidechain when the source is earlier (`L_sc <= L`: delay
    /// the tap by `L - L_sc`); when it is later the modulation lags by `L_sc - L` (a control
    /// signal never delays the audio).
    pub(crate) fn write_sidechain(&mut self, source: usize, tap: &Stereo, n: usize) {
        let Some(i) = self.sidechain_sources.iter().position(|&s| s == source) else {
            return;
        };
        let sr = self.sample_rate;
        for s in self.sources.iter_mut().filter(|s| s.sidechain == Some(i)) {
            // Swap the buffer out (no allocation) to detect on the delayed copy.
            let mut buf = std::mem::take(&mut s.sc_buf);
            {
                let [l, r] = &mut buf;
                l[..n].copy_from_slice(&tap[0][..n]);
                r[..n].copy_from_slice(&tap[1][..n]);
                s.sc_delay.process(&mut l[..n], &mut r[..n]);
            }
            s.follow(&buf[0][..n], &buf[1][..n], sr);
            s.sc_buf = buf;
        }
    }

    /// RT. Carry source state (LFO phases, envelope stages), macro values and bases over a
    /// snapshot swap. Targets are re-sent once.
    pub(crate) fn inherit(&mut self, old: &mut ModulationRt) {
        self.was_playing = old.was_playing;
        for s in &mut self.sources {
            if let Some(o) = old.sources.iter_mut().find(|o| o.id == s.id) {
                if o.kind == s.kind {
                    s.st = o.st;
                }
                if o.sidechain.is_some() && s.sidechain.is_some() {
                    s.sc_delay.inherit(&mut o.sc_delay);
                }
            }
        }
        for m in &mut self.macros {
            if let Some(o) = old.macros.iter().find(|o| o.rack == m.rack) {
                m.values = o.values;
            }
        }
        for t in &mut self.targets {
            if let Some(o) = old
                .targets
                .iter()
                .find(|o| o.node == t.node && o.param == t.param)
            {
                t.base = o.base;
            }
        }
    }

    /// RT. A base value (plain) for `node`/`param`; `true` = consumed (see the module docs).
    pub(crate) fn intercept(&mut self, node: NodeKey, param: ParamId, plain: f64) -> bool {
        self.intercept_at(node, param, plain, 0)
    }

    /// RT. [`Self::intercept`] at sample `offset` of the coming sub-block (automation).
    pub(crate) fn intercept_at(
        &mut self,
        node: NodeKey,
        param: ParamId,
        plain: f64,
        offset: u32,
    ) -> bool {
        if (param.0 as usize) < MACROS
            && let Some(m) = self.macros.iter_mut().find(|m| m.rack == node)
        {
            let v = if plain.is_finite() {
                plain.clamp(0.0, 1.0)
            } else {
                0.0
            };
            push_change(&mut m.changes, &mut m.n_changes, (offset, param.0 as u8, v));
            return true;
        }
        let Ok(i) = self
            .targets
            .binary_search_by(|t| (t.node, t.param).cmp(&(node, param)))
        else {
            return false;
        };
        let t = &mut self.targets[i];
        let m = t.mapping;
        let n = if plain.is_finite() {
            ether_protocol::devices::scale_to_normalized(m.scale, m.min, m.max, plain)
        } else {
            t.base
        };
        push_change(&mut t.changes, &mut t.n_changes, (offset, n));
        true
    }

    /// RT. A live modulator param change; `true` when the modulator is on this track.
    pub(crate) fn set_param(&mut self, modulator: ModulatorId, param: ParamId, plain: f64) -> bool {
        let Some(s) = self.sources.iter_mut().find(|s| s.id == modulator) else {
            return false;
        };
        if let Some(slot) = s.p.get_mut(param.0 as usize)
            && plain.is_finite()
        {
            *slot = plain;
        }
        true
    }

    /// RT. Compute the sources for the sub-block and emit the targets' `Param` events.
    pub(crate) fn render(
        &mut self,
        timing: &Timing<'_>,
        info: &TransportInfo,
        chain: &mut [ChainRt],
        racks: &mut RacksRt,
        chain_racks: &mut ChainRacksRt,
    ) {
        if self.sources.is_empty() && self.targets.is_empty() {
            self.clear_changes();
            return;
        }
        let n = timing.frames;
        let playing = info.playing;
        let sr = self.sample_rate;
        // Transport retrigger (LFO `Transport` mode) and a fresh start for fades.
        if playing && !self.was_playing {
            for s in self.sources.iter_mut() {
                if s.kind == ModulatorKind::Lfo && s.p[5].round() as i64 == 2 {
                    s.st.phase = 0.0;
                    s.st.since = 0.0;
                }
            }
        }
        self.was_playing = playing;

        // Notes of hosts at chain entry 0 (sample-accurate triggers).
        self.notes.clear();
        let host0 = chain.first().map(|c| c.key);
        if let Some(first) = chain.first()
            && self
                .sources
                .iter()
                .any(|s| s.uses_notes() && Some(s.host) == host0)
        {
            for e in first.events.as_slice() {
                if matches!(
                    e.kind,
                    EventKind::NoteOn { .. }
                        | EventKind::NoteOff { .. }
                        | EventKind::NoteChoke { .. }
                        | EventKind::AllNotesOff
                        | EventKind::Midi { .. }
                ) && self.notes.len() < self.notes.capacity()
                {
                    self.notes.push(*e);
                }
            }
            // Stable insertion sort by offset (no allocation).
            for i in 1..self.notes.len() {
                let mut j = i;
                while j > 0 && self.notes[j - 1].offset > self.notes[j].offset {
                    self.notes.swap(j - 1, j);
                    j -= 1;
                }
            }
        }

        // Knots: grid points, base/macro changes, note triggers, and 0 when re-sending.
        self.knots.clear();
        let resend = self.targets.iter().any(|t| t.last.is_nan());
        let cap = self.knots.capacity();
        let add = |knots: &mut Vec<u32>, o: u32| {
            if knots.len() < cap {
                knots.push(o);
            }
        };
        if resend {
            add(&mut self.knots, 0);
        }
        for g in crate::automation_rt::grid(timing) {
            add(&mut self.knots, g as u32);
        }
        for t in &self.targets {
            for &(o, _) in &t.changes[..t.n_changes] {
                add(&mut self.knots, o.min(n.saturating_sub(1) as u32));
            }
        }
        for m in &self.macros {
            for &(o, _, _) in &m.changes[..m.n_changes] {
                add(&mut self.knots, o.min(n.saturating_sub(1) as u32));
            }
        }
        for e in &self.notes {
            add(&mut self.knots, e.offset);
        }
        self.knots.sort_unstable();
        self.knots.dedup();

        let clock_at = |o: f64| Clock {
            playing,
            beat: if playing {
                timing.beat_at(o)
            } else {
                timing.b0
            },
            sr,
        };
        let mut pos = 0.0f64;
        let mut next_note = 0;
        for ki in 0..self.knots.len() {
            let o = self.knots[ki];
            let of = o as f64;
            // Sources up to this knot; notes at this offset apply before it is evaluated.
            let clock = clock_at(of);
            for s in self.sources.iter_mut() {
                s.advance(of - pos, clock);
            }
            while let Some(e) = self.notes.get(next_note)
                && e.offset <= o
            {
                for s in self
                    .sources
                    .iter_mut()
                    .filter(|s| s.uses_notes() && Some(s.host) == host0)
                {
                    s.notes(e, clock);
                    s.advance(0.0, clock);
                }
                next_note += 1;
            }
            pos = of;
            // Base and macro changes up to this knot.
            for m in self.macros.iter_mut() {
                for &(co, idx, v) in &m.changes[..m.n_changes] {
                    if co <= o {
                        m.values[idx as usize] = v;
                    }
                }
            }
            for ti in 0..self.targets.len() {
                let t = &mut self.targets[ti];
                for &(co, v) in &t.changes[..t.n_changes] {
                    if co <= o {
                        t.base = v;
                    }
                }
                let mut sum = t.base;
                for l in &t.links {
                    let m = match l.src {
                        Src::Mod(i) => self.sources[i as usize].st.value,
                        Src::Macro(r, idx) => self.macros[r as usize].values[idx as usize],
                    };
                    sum += l.depth * m;
                }
                let eff = sum.clamp(0.0, 1.0);
                t.effective = eff;
                let plain = to_plain(&t.mapping, eff);
                if plain == t.last {
                    continue;
                }
                t.last = plain;
                self.readback_dirty = true;
                let (node, param) = (t.node, t.param);
                let event = ProcessEvent {
                    offset: o,
                    kind: EventKind::Param {
                        param,
                        value: plain,
                    },
                };
                let buf: Option<&mut EventBuffer> = match chain.iter_mut().find(|c| c.key == node) {
                    Some(c) => Some(&mut c.events),
                    None => match racks.events_mut(node) {
                        Some(e) => Some(e),
                        None => chain_racks.events_mut(node),
                    },
                };
                if let Some(buf) = buf {
                    buf.push(event);
                }
            }
        }
        // Finish the sub-block.
        let end = n as f64;
        if end > pos {
            let clock = clock_at(end);
            for s in self.sources.iter_mut() {
                s.advance(end - pos, clock);
            }
        }
        self.clear_changes();
    }

    fn clear_changes(&mut self) {
        for t in self.targets.iter_mut() {
            t.n_changes = 0;
        }
        for m in self.macros.iter_mut() {
            m.n_changes = 0;
        }
    }

    /// RT. Before chain entry `k` (node `key`) processes `a`.
    pub(crate) fn pre_node(
        &mut self,
        k: usize,
        key: NodeKey,
        events: &[crate::ProcessEvent],
        a: &Stereo,
        n: usize,
    ) {
        if self.sources.is_empty() {
            return;
        }
        let sr = self.sample_rate;
        let clock = Clock {
            playing: self.was_playing,
            beat: 0.0,
            sr,
        };
        for s in self.sources.iter_mut().filter(|s| s.host == key) {
            match s.kind {
                ModulatorKind::EnvelopeFollower if s.sidechain.is_none() => {
                    s.follow(&a[0][..n], &a[1][..n], sr);
                }
                // Entry-0 hosts got their notes in `render`.
                _ if k > 0 && s.uses_notes() => {
                    for e in events {
                        s.notes(e, clock);
                    }
                }
                _ => {}
            }
        }
    }

    /// RT. Push this track's readback frames (at most `budget`); returns frames pushed.
    fn readback(&mut self, analysis: &mut AnalysisRt, budget: usize) -> usize {
        if self.targets.is_empty() || budget == 0 {
            return 0;
        }
        self.readback_age += 1;
        if !self.readback_dirty && self.readback_age < READBACK_REFRESH {
            return 0;
        }
        let mut pushed = 0;
        let mut frame = AnalysisFrame::EMPTY;
        let mut i = 0;
        while i < self.targets.len() && pushed < budget {
            let node = self.targets[i].node;
            frame.node = node;
            frame.begin(AnalysisKind::Modulation);
            let mut count = 0;
            while i < self.targets.len() && self.targets[i].node == node {
                let t = &self.targets[i];
                if count < READBACK_PARAMS {
                    frame.push(f32::from_bits(t.param.0));
                    frame.push(t.base as f32);
                    frame.push(t.effective as f32);
                    count += 1;
                }
                i += 1;
            }
            analysis.push(&frame);
            pushed += 1;
        }
        if i >= self.targets.len() {
            self.readback_dirty = false;
            self.readback_age = 0;
        }
        pushed
    }
}

/// Round-robin start track of the next readback pass (audio thread only).
static READBACK_CURSOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// RT. Push modulation readback frames for every track: at most
/// [`READBACK_FRAMES_PER_PASS`] per pass, starting where the previous pass stopped.
pub(crate) fn readback(tracks: &mut [TrackRt], analysis: &mut AnalysisRt) {
    use std::sync::atomic::Ordering;
    let n = tracks.len();
    if n == 0 {
        return;
    }
    let start = READBACK_CURSOR.load(Ordering::Relaxed) % n;
    let mut budget = READBACK_FRAMES_PER_PASS;
    for step in 0..n {
        let i = (start + step) % n;
        if budget == 0 {
            READBACK_CURSOR.store(i, Ordering::Relaxed);
            return;
        }
        budget -= tracks[i].modulation.readback(analysis, budget);
    }
    READBACK_CURSOR.store((start + 1) % n, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(beat: f64) -> Clock {
        Clock {
            playing: true,
            beat,
            sr: 48_000.0,
        }
    }

    fn source(kind: ModulatorKind, params: &[(usize, f64)]) -> Source {
        let mut p = [0.0; MAX_MOD_PARAMS];
        for &(i, v) in params {
            p[i] = v;
        }
        Source {
            id: ModulatorId(ether_protocol::model::Ulid(7)),
            host: NodeKey {
                index: 0,
                generation: 0,
            },
            kind,
            p,
            st: SourceState::default(),
            sidechain: None,
            sc_delay: DelayLine::new(0),
            sc_buf: [Vec::new(), Vec::new()],
        }
    }

    #[test]
    fn lfo_shapes_are_bipolar() {
        for shape in 0..5 {
            for i in 0..100 {
                let v = lfo_shape(shape as f64, i as f64 / 100.0);
                assert!((-1.0..=1.0).contains(&v), "{shape} {v}");
            }
        }
        assert!((lfo_shape(0.0, 0.25) - 1.0).abs() < 1e-9);
        assert_eq!(lfo_shape(2.0, 0.0), -1.0);
    }

    #[test]
    fn free_lfo_advances_by_rate() {
        let mut s = source(ModulatorKind::Lfo, &[(1, 1.0)]);
        s.advance(12_000.0, clock(0.0));
        assert!((s.st.value - 1.0).abs() < 1e-9, "{}", s.st.value);
    }

    #[test]
    fn synced_lfo_follows_the_beat() {
        // 1/4 = one beat per cycle, square.
        let mut s = source(ModulatorKind::Lfo, &[(0, 4.0), (2, 1.0), (3, 8.0)]);
        s.advance(1.0, clock(10.25));
        assert_eq!(s.st.value, 1.0);
        s.advance(1.0, clock(10.75));
        assert_eq!(s.st.value, -1.0);
    }

    #[test]
    fn envelope_attacks_decays_and_releases() {
        // A 10 ms, D 100 ms, S 50 %, R 100 ms.
        let mut s = source(
            ModulatorKind::Envelope,
            &[(0, 10.0), (1, 100.0), (2, 50.0), (3, 100.0)],
        );
        s.note_on(1, 60, 1.0, clock(0.0));
        s.advance(240.0, clock(0.0));
        assert!((s.st.value - 0.5).abs() < 1e-6, "{}", s.st.value);
        s.advance(240.0 + 48_000.0, clock(0.0));
        assert!((s.st.value - 0.5).abs() < 1e-3, "{}", s.st.value);
        s.note_off(Some(2));
        assert_eq!(s.st.env_stage, EnvStage::Decay, "other notes don't release");
        s.note_off(Some(1));
        s.advance(48_000.0, clock(0.0));
        assert_eq!(s.st.value, 0.0);
    }

    #[test]
    fn random_is_deterministic_per_period() {
        let mut a = source(ModulatorKind::Random, &[(1, 1.0), (2, 8.0)]);
        let mut b = source(ModulatorKind::Random, &[(1, 1.0), (2, 8.0)]);
        a.advance(1.0, clock(3.5));
        b.advance(500.0, clock(3.2));
        assert_eq!(a.st.value, b.st.value);
        assert!((-1.0..=1.0).contains(&a.st.value));
    }

    #[test]
    fn steps_play_the_step_values() {
        let mut s = source(
            ModulatorKind::Steps,
            &[(0, 4.0), (1, 8.0), (3, -1.0), (4, 0.5), (5, 1.0), (6, 0.0)],
        );
        s.advance(1.0, clock(1.5));
        assert_eq!(s.st.value, 0.5);
        s.advance(1.0, clock(6.0));
        assert_eq!(s.st.value, 1.0);
    }

    #[test]
    fn keytrack_and_velocity_place_the_last_note() {
        let mut k = source(ModulatorKind::Keytrack, &[(0, 36.0), (1, 96.0)]);
        k.note_on(1, 66, 0.5, clock(0.0));
        k.advance(1.0, clock(0.0));
        assert!((k.st.value - 0.5).abs() < 1e-9);
        let mut v = source(ModulatorKind::Velocity, &[(0, 0.0), (1, 127.0)]);
        v.note_on(1, 66, 1.0, clock(0.0));
        v.advance(1.0, clock(0.0));
        assert_eq!(v.st.value, 1.0);
    }

    #[test]
    fn follower_tracks_the_peak() {
        let mut s = source(ModulatorKind::EnvelopeFollower, &[(0, 0.1), (1, 50.0)]);
        let l = vec![0.5f32; 4800];
        s.follow(&l, &l, 48_000.0);
        s.advance(0.0, clock(0.0));
        assert!((s.st.value - 0.5).abs() < 1e-3, "{}", s.st.value);
    }
}

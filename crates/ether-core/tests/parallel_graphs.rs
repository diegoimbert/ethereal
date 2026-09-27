//! Randomized projects for the multicore tests (`multicore` node): many tracks, nested
//! groups, returns with pre/post sends, sidechains, drum racks with choke groups, latent
//! devices (PDC), audio clips (resampled, optionally Complex-warped = pinned to the audio
//! thread), automation, mute/solo, a snapshot swap, live params and a locate.
//!
//! Shared by `tests/parallel.rs` (ether-core, test executor) and
//! `ether-native/tests/multicore*.rs` (real worker pool) through `#[path]`.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, PadDesc, ParamMapping,
    RackDesc, ResolvedTarget, SendDesc, TrackDesc, WarpDesc,
};
use ether_core::parallel::ParallelExecutor;
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, Beats, ClipId, CurveShape, DrumPadId, FadeCurve, MediaId, ParamId, SendId,
    TrackId, TrackKind, Ulid, WarpMode,
};
use ether_core::{
    AudioBuffers, AudioSource, Engine, EngineConfig, EngineHandle, EngineParts, EventKind, Node,
    NodeKey, ParamChange, ParamTarget, PrepareConfig, ProcessContext, ProcessStatus,
    RenderGraphDesc, TransportControl, create,
};

pub const SR: u32 = 48_000;
pub const MAX_BLOCK: usize = 512;
pub const MEDIA: MediaId = MediaId(Ulid(7));

pub fn config() -> EngineConfig {
    EngineConfig {
        sample_rate: SR,
        max_block_size: MAX_BLOCK,
        max_nodes: 1024,
        max_events_per_block: 256,
        ..EngineConfig::default()
    }
}

/// xorshift64* (deterministic, no dependency).
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

// ---------------------------------------------------------------------------------------
// Test nodes (allocation only in `prepare`).

/// Polyphonic naive-saw instrument; param 0 = gain (smoothed per sample).
pub struct Saw {
    voices: Vec<(u32, f32, f32, f32)>,
    gain: f32,
    target: f32,
    detune: f32,
}

impl Saw {
    pub fn new(detune: f32) -> Self {
        Self {
            voices: Vec::new(),
            gain: 0.3,
            target: 0.3,
            detune,
        }
    }
    fn apply(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note_id,
                key,
                velocity,
                ..
            } => {
                if self.voices.len() < self.voices.capacity() {
                    let f = 440.0 * 2f32.powf((key as f32 - 69.0) / 12.0) * self.detune;
                    self.voices.push((note_id, 0.0, f / SR as f32, velocity));
                }
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                self.voices.retain(|v| v.0 != note_id)
            }
            EventKind::AllNotesOff => self.voices.clear(),
            EventKind::Param { value, .. } => self.target = value as f32,
            EventKind::Midi { .. } => {}
        }
    }
}

impl Node for Saw {
    fn prepare(&mut self, _: &PrepareConfig) {
        self.voices = Vec::with_capacity(32);
    }
    fn reset(&mut self) {
        self.voices.clear();
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut ev = 0;
        for i in 0..ctx.frames {
            while ev < ctx.events.len() && ctx.events[ev].offset as usize <= i {
                self.apply(ctx.events[ev].kind);
                ev += 1;
            }
            self.gain += (self.target - self.gain) * 0.01;
            let mut s = 0.0;
            for v in self.voices.iter_mut() {
                v.1 += v.2;
                if v.1 >= 1.0 {
                    v.1 -= 1.0;
                }
                s += (v.1 * 2.0 - 1.0) * v.3;
            }
            let s = s * self.gain;
            audio.outputs[0][i] = s;
            if audio.outputs.len() > 1 {
                audio.outputs[1][i] = s * 0.9;
            }
        }
        while ev < ctx.events.len() {
            self.apply(ctx.events[ev].kind);
            ev += 1;
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// One-pole lowpass (state across blocks) with an optional latency (pure delay).
pub struct Filter {
    coef: f32,
    state: [f32; 2],
    latency: usize,
    line: [Vec<f32>; 2],
    pos: usize,
}

impl Filter {
    pub fn new(coef: f32, latency: usize) -> Self {
        Self {
            coef,
            state: [0.0; 2],
            latency,
            line: [Vec::new(), Vec::new()],
            pos: 0,
        }
    }
}

impl Node for Filter {
    fn prepare(&mut self, _: &PrepareConfig) {
        self.line = [
            vec![0.0; self.latency.max(1)],
            vec![0.0; self.latency.max(1)],
        ];
    }
    fn reset(&mut self) {
        self.state = [0.0; 2];
        for l in &mut self.line {
            l.fill(0.0);
        }
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            if let EventKind::Param { value, .. } = e.kind {
                self.coef = (value as f32).clamp(0.01, 0.99);
            }
        }
        let n_in = audio.inputs.len();
        let mut pos = self.pos;
        for ch in 0..audio.outputs.len() {
            pos = self.pos;
            for i in 0..ctx.frames {
                let x = if n_in == 0 {
                    0.0
                } else {
                    audio.inputs[ch.min(n_in - 1)][i]
                };
                self.state[ch] += (x - self.state[ch]) * self.coef;
                let y = self.state[ch];
                audio.outputs[ch][i] = if self.latency == 0 {
                    y
                } else {
                    let d = self.line[ch][pos];
                    self.line[ch][pos] = y;
                    pos = (pos + 1) % self.latency;
                    d
                };
            }
        }
        self.pos = pos;
        ProcessStatus::Continue
    }
    fn latency(&self) -> u32 {
        self.latency as u32
    }
}

/// Ducks its input by the sidechain level (`1 - depth·|sc|`), with an envelope follower.
pub struct Ducker {
    depth: f32,
    env: f32,
}

impl Node for Ducker {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.env = 0.0;
    }
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.pass_through();
        ProcessStatus::Continue
    }
    fn sidechain_inputs(&self) -> u16 {
        2
    }
    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        for i in 0..ctx.frames {
            let sc = sidechain.iter().map(|c| c[i].abs()).sum::<f32>() * 0.5;
            self.env += (sc - self.env) * 0.05;
            let g = 1.0 - self.depth * self.env.min(1.0);
            for ch in 0..audio.outputs.len() {
                audio.outputs[ch][i] = audio.inputs[ch][i] * g;
            }
        }
        ProcessStatus::Continue
    }
}

/// Soft clipper (a drum rack's own node: processes the mixed pads).
pub struct Clip(pub f32);

impl Node for Clip {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for ch in 0..audio.outputs.len() {
            for i in 0..ctx.frames {
                let x = audio.inputs[ch][i] * self.0;
                audio.outputs[ch][i] = x / (1.0 + x.abs());
            }
        }
        ProcessStatus::Continue
    }
}

/// Stereo in-memory source (a decaying chirp).
pub struct Mem(pub Vec<f32>, pub Vec<f32>);

impl AudioSource for Mem {
    fn channels(&self) -> u16 {
        2
    }
    fn frames(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let src = if channel == 0 { &self.0 } else { &self.1 };
        for (i, o) in out.iter_mut().enumerate() {
            *o = src.get(start as usize + i).copied().unwrap_or(0.0);
        }
        true
    }
}

pub fn source() -> Arc<dyn AudioSource> {
    let n = SR as usize * 2;
    let l = (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            (t * 220.0 * (1.0 + t) * std::f32::consts::TAU).sin() * 0.5
        })
        .collect();
    let r = (0..n)
        .map(|i| ((i * 7 % 1000) as f32 / 1000.0 - 0.5) * 0.3)
        .collect();
    Arc::new(Mem(l, r))
}

// ---------------------------------------------------------------------------------------
// Project generator.

pub fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

pub fn track(id: TrackId, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
    TrackDesc {
        id,
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips: vec![],
        automation: vec![],
        racks: Vec::new(),
    }
}

pub fn entry(node: NodeKey) -> ChainEntry {
    ChainEntry {
        node,
        enabled: true,
        sidechain: None,
    }
}

fn add(h: &mut EngineHandle, node: Box<dyn Node>) -> NodeKey {
    h.add_node(node).expect("node table")
}

fn effect(r: &mut Rng, h: &mut EngineHandle) -> ChainEntry {
    let lat = if r.chance(0.3) { 1 + r.below(300) } else { 0 };
    let mut e = entry(add(h, Box::new(Filter::new(0.05 + r.unit() * 0.9, lat))));
    e.enabled = !r.chance(0.1);
    e
}

pub fn midi_clip(r: &mut Rng, id: u128, keys: &[u8]) -> ClipDesc {
    let mut notes: Vec<NoteDesc> = (0..3 + r.below(12))
        .map(|_| NoteDesc {
            start: r.below(32) as f64 * 0.25,
            duration: 0.1 + r.unit() as f64 * 2.0,
            key: keys[r.below(keys.len())],
            velocity: 0.2 + r.unit() * 0.8,
            release_velocity: 0.0,
        })
        .collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    ClipDesc {
        id: ClipId(Ulid(id)),
        start: r.below(4) as f64 * 0.5,
        length: 4.0 + r.below(8) as f64,
        offset: 0.0,
        looping: r.chance(0.5).then_some((0.0, 4.0)),
        muted: r.chance(0.05),
        content: ClipContentDesc::Midi { notes },
        envelopes: vec![],
    }
}

fn audio_clip(r: &mut Rng, id: u128, complex: bool) -> ClipDesc {
    ClipDesc {
        id: ClipId(Ulid(id)),
        start: r.below(8) as f64 * 0.5,
        length: 1.0 + r.below(6) as f64,
        offset: r.below(4) as f64 * 0.25,
        looping: None,
        muted: false,
        content: ClipContentDesc::Audio {
            media: MEDIA,
            gain: 0.3 + r.unit(),
            transpose: [0.0, 3.0, -5.0][r.below(3)],
            fade_in: r.unit() as f64 * 0.5,
            fade_out: r.unit() as f64 * 0.5,
            warp: complex.then(|| WarpDesc {
                mode: WarpMode::Complex,
                markers: vec![(0.0, 0.0), (8.0, 3.0)],
            }),
            fade_in_curve: FadeCurve::default(),
            fade_out_curve: FadeCurve::default(),
            reversed: r.chance(0.1),
        },
        envelopes: vec![],
    }
}

fn volume_lane(track: TrackId, r: &mut Rng) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track },
        resolved: ResolvedTarget::TrackVolume,
        points: vec![
            (0.0, 0.3 + r.unit() as f64 * 0.5, CurveShape::Linear),
            (4.0, 0.2 + r.unit() as f64 * 0.7, CurveShape::Linear),
        ],
        mapping: ParamMapping {
            min: -70.0,
            max: 6.0,
            scale: ParamScale::Fader,
            steps: None,
        },
    }
}

fn node_lane(track: TrackId, node: NodeKey) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackPan { track },
        resolved: ResolvedTarget::Node {
            node,
            param: ParamId(0),
        },
        points: vec![
            (0.0, 0.1, CurveShape::Linear),
            (6.0, 0.9, CurveShape::Linear),
        ],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }
}

/// A generated project: what to publish, a variant for a mid-render swap, and the live
/// changes to apply.
pub struct Project {
    pub desc: RenderGraphDesc,
    pub swapped: RenderGraphDesc,
    pub live: Vec<ParamChange>,
    pub tracks: usize,
}

/// Build project `seed` on `h` (adds nodes and the source). Deterministic: the same seed
/// on a fresh engine gives the same node keys and the same desc. `complex` allows
/// Complex-warped clips (needs a stretcher factory on the handle).
pub fn build(seed: u64, h: &mut EngineHandle, complex: bool) -> Project {
    let mut r = Rng::new(seed);
    h.add_source(MEDIA, source()).unwrap();
    let master_id = tid(1);
    let mut master = track(master_id, TrackKind::Master, None);
    if r.chance(0.5) {
        master.chain.push(effect(&mut r, h));
    }
    let mut tracks = vec![];

    // Groups (possibly nested) and returns.
    let mut groups: Vec<TrackId> = vec![];
    for g in 0..r.below(4) {
        let id = tid(100 + g as u128);
        let parent = (!groups.is_empty() && r.chance(0.5)).then(|| groups[r.below(groups.len())]);
        let mut t = track(id, TrackKind::Group, Some(parent.unwrap_or(master_id)));
        t.group = parent;
        if r.chance(0.6) {
            t.chain.push(effect(&mut r, h));
        }
        t.volume = 0.5 + r.unit();
        t.mute = r.chance(0.08);
        groups.push(id);
        tracks.push(t);
    }
    let mut returns: Vec<TrackId> = vec![];
    for k in 0..r.below(3) {
        let id = tid(200 + k as u128);
        let mut t = track(id, TrackKind::Return, Some(master_id));
        t.chain.push(effect(&mut r, h));
        t.volume = 0.3 + r.unit() * 0.7;
        returns.push(id);
        tracks.push(t);
    }

    // Regular tracks.
    let n = 4 + r.below(36);
    let mut regular: Vec<TrackId> = vec![];
    let mut send_id = 1u128;
    for i in 0..n {
        let id = tid(1000 + i as u128);
        let dest = (!groups.is_empty() && r.chance(0.5)).then(|| groups[r.below(groups.len())]);
        let midi = r.chance(0.7);
        let mut t = track(
            id,
            if midi {
                TrackKind::Midi
            } else {
                TrackKind::Audio
            },
            Some(dest.unwrap_or(master_id)),
        );
        t.group = dest;
        if midi {
            if r.chance(0.25) {
                // Drum rack: pads (saw + filter), some in a choke group, into a clipper.
                let rack = add(h, Box::new(Clip(1.0 + r.unit() * 3.0)));
                let pads = (0..1 + r.below(4))
                    .map(|p| {
                        let mut chain =
                            vec![entry(add(h, Box::new(Saw::new(1.0 + p as f32 * 0.01))))];
                        if r.chance(0.5) {
                            chain.push(effect(&mut r, h));
                        }
                        PadDesc {
                            pad: DrumPadId(Ulid(10_000 + i as u128 * 16 + p as u128)),
                            note: 36 + p as u8,
                            choke_group: r.chance(0.4).then_some(1),
                            chain,
                            volume: 0.4 + r.unit(),
                            pan: r.unit() * 2.0 - 1.0,
                            mute: r.chance(0.1),
                        }
                    })
                    .collect();
                t.chain.push(entry(rack));
                t.racks.push(RackDesc { rack, pads });
                t.clips
                    .push(midi_clip(&mut r, 1 + i as u128, &[36, 37, 38, 39, 60]));
            } else {
                let saw = add(h, Box::new(Saw::new(1.0 + r.unit() * 0.02)));
                t.chain.push(entry(saw));
                t.clips
                    .push(midi_clip(&mut r, 1 + i as u128, &[48, 55, 60, 64, 67, 72]));
                if r.chance(0.3) {
                    t.automation.push(node_lane(id, saw));
                }
            }
        } else {
            for c in 0..1 + r.below(3) {
                let cx = complex && r.chance(0.2);
                t.clips
                    .push(audio_clip(&mut r, 1 + i as u128 * 8 + c as u128, cx));
            }
            t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        }
        for _ in 0..r.below(3) {
            t.chain.push(effect(&mut r, h));
        }
        // Sidechain from an earlier regular track.
        if !regular.is_empty() && r.chance(0.3) {
            let mut e = entry(add(
                h,
                Box::new(Ducker {
                    depth: 0.3 + r.unit() * 0.6,
                    env: 0.0,
                }),
            ));
            e.sidechain = Some(regular[r.below(regular.len())]);
            t.chain.push(e);
        }
        for _ in 0..r.below(3) {
            if returns.is_empty() {
                break;
            }
            t.sends.push(SendDesc {
                id: SendId(Ulid(send_id)),
                to: returns[r.below(returns.len())],
                level: r.unit(),
                pre_fader: r.chance(0.4),
            });
            send_id += 1;
        }
        t.volume = 0.2 + r.unit();
        t.pan = r.unit() * 2.0 - 1.0;
        t.mute = r.chance(0.08);
        t.solo = r.chance(0.04);
        if r.chance(0.3) {
            t.automation.push(volume_lane(id, &mut r));
        }
        regular.push(id);
        tracks.push(t);
    }
    tracks.push(master);
    // Order in the desc is irrelevant to the compiler; shuffle it to exercise that.
    for i in (1..tracks.len()).rev() {
        let j = r.below(i + 1);
        tracks.swap(i, j);
    }
    let desc = RenderGraphDesc {
        version: 1,
        loop_enabled: r.chance(0.5),
        loop_start: 0.0,
        loop_end: 6.0,
        tracks,
        ..Default::default()
    };

    // Swap: new volumes, a mute toggle, a send level change (inherit paths).
    let mut swapped = desc.clone();
    swapped.version = 2;
    for t in swapped.tracks.iter_mut() {
        if r.chance(0.3) {
            t.volume = r.unit() * 1.2;
        }
        if r.chance(0.1) {
            t.mute = !t.mute;
        }
        for s in t.sends.iter_mut() {
            s.level = r.unit();
        }
    }
    let live = regular
        .iter()
        .take(3)
        .map(|&track| ParamChange {
            target: ParamTarget::TrackVolume { track },
            value: 0.7,
        })
        .collect();
    Project {
        desc,
        swapped,
        live,
        tracks: n,
    }
}

/// Render `frames` of project `seed` in blocks of `block` with `executor` (`None` =
/// sequential): play, swap the graph at 1/3, live params + locate at 2/3. Returns the
/// interleaved-free (left ++ right) output.
pub fn render(
    seed: u64,
    block: usize,
    frames: usize,
    executor: Option<Box<dyn ParallelExecutor>>,
    stretch: Option<Arc<dyn ether_stretch::StretcherFactory>>,
    before_block: &mut dyn FnMut(&mut Engine),
) -> Vec<f32> {
    let EngineParts {
        mut engine,
        mut handle,
        mut gc,
    } = create(config());
    let complex = stretch.is_some();
    if let Some(f) = stretch {
        handle.set_stretcher_factory(f);
    }
    let project = build(seed, &mut handle, complex);
    handle.publish(project.desc.clone()).unwrap();
    if let Some(e) = executor {
        engine.set_executor(e);
    }
    handle.transport(TransportControl::Play).unwrap();
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut done = 0;
    let mut step = 0;
    while done < frames {
        if step == 0 && done >= frames / 3 {
            handle.publish(project.swapped.clone()).unwrap();
            step = 1;
        } else if step == 1 && done >= 2 * frames / 3 {
            for c in &project.live {
                handle.set_param(*c).unwrap();
            }
            handle
                .transport(TransportControl::Locate {
                    position: Beats(1.5),
                })
                .unwrap();
            step = 2;
        }
        let n = block.min(frames - done);
        before_block(&mut engine);
        {
            let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
            engine.process(&[], &mut outs, n);
        }
        left[done..done + n].copy_from_slice(&l[..n]);
        right[done..done + n].copy_from_slice(&r[..n]);
        done += n;
        gc.collect();
    }
    left.extend_from_slice(&right);
    left
}

/// First differing sample, if any (bitwise).
pub fn first_diff(a: &[f32], b: &[f32]) -> Option<usize> {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

/// A simple threaded executor for tests: scoped threads per call, dynamic claiming, so
/// the job order and thread assignment vary from run to run.
pub struct Threaded(pub usize);

impl ParallelExecutor for Threaded {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        let next = AtomicUsize::new(0);
        let run = || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= jobs {
                    break;
                }
                job(i);
            }
        };
        std::thread::scope(|s| {
            for _ in 0..self.0.min(jobs.saturating_sub(1)) {
                s.spawn(run);
            }
            run();
        });
    }

    fn workers(&self) -> usize {
        self.0
    }
}

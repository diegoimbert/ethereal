//! Shared helpers for the offline engine tests (no audio device).
#![allow(dead_code)]

use std::sync::Arc;

use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, SendDesc, TrackDesc};
use ether_core::protocol::model::{ClipId, SendId, TrackId, TrackKind, Ulid};
use ether_core::{
    AudioBuffers, AudioSource, Engine, EngineConfig, EventKind, Node, NodeKey, PrepareConfig,
    ProcessContext, ProcessStatus,
};
use rtrb::{Consumer, Producer, RingBuffer};

pub const SR: u32 = 48_000;
pub const BLOCK: usize = 512;

pub fn config() -> EngineConfig {
    EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_nodes: 64,
        max_events_per_block: 256,
        ..EngineConfig::default()
    }
}

pub fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

pub fn sid(n: u128) -> SendId {
    SendId(Ulid(n))
}

pub fn cid(n: u128) -> ClipId {
    ClipId(Ulid(n))
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

pub fn master() -> TrackDesc {
    track(tid(1), TrackKind::Master, None)
}

pub fn with_chain(mut t: TrackDesc, nodes: &[NodeKey]) -> TrackDesc {
    t.chain = nodes
        .iter()
        .map(|&node| ChainEntry {
            node,
            enabled: true, sidechain: None,
        })
        .collect();
    t
}

pub fn send(id: SendId, to: TrackId, level: f32, pre_fader: bool) -> SendDesc {
    SendDesc {
        id,
        to,
        level,
        pre_fader,
    }
}

pub fn midi_clip(id: ClipId, start: f64, length: f64, notes: &[(f64, f64, u8)]) -> ClipDesc {
    ClipDesc {
        id,
        start,
        length,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Midi {
            notes: notes
                .iter()
                .map(|&(start, duration, key)| NoteDesc {
                    start,
                    duration,
                    key,
                    velocity: 1.0,
                    release_velocity: 0.0,
                })
                .collect(),
        },
        envelopes: vec![],
    }
}

/// Render `frames` samples in blocks of `block`; returns (left, right).
pub fn render(engine: &mut Engine, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let mut done = 0;
    let mut l = vec![0.0; block];
    let mut r = vec![0.0; block];
    while done < frames {
        let n = block.min(frames - done);
        {
            let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
            engine.process(&[], &mut outs, n);
        }
        left[done..done + n].copy_from_slice(&l[..n]);
        right[done..done + n].copy_from_slice(&r[..n]);
        done += n;
    }
    (left, right)
}

/// Constant signal generator (instrument-like: no inputs).
pub struct Dc(pub f32);

impl Node for Dc {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for ch in audio.outputs.iter_mut() {
            ch.fill(self.0);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// Outputs 1.0 on the very first sample of the engine timeline (sample_time 0).
pub struct Impulse;

impl Node for Impulse {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.clear_outputs();
        if ctx.transport.sample_time == 0 {
            for ch in audio.outputs.iter_mut() {
                ch[0] = 1.0;
            }
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// Pure delay reporting its latency (a stand-in for a look-ahead plugin).
pub struct Delay {
    pub samples: usize,
    buf: [Vec<f32>; 2],
    pos: usize,
}

impl Delay {
    pub fn new(samples: usize) -> Self {
        Self {
            samples,
            buf: [vec![], vec![]],
            pos: 0,
        }
    }
}

impl Node for Delay {
    fn prepare(&mut self, _: &PrepareConfig) {
        self.buf = [vec![0.0; self.samples], vec![0.0; self.samples]];
    }
    fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
    }
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let len = self.samples;
        for ch in 0..2 {
            let mut pos = self.pos;
            for i in 0..audio.outputs[ch].len() {
                let x = audio.inputs[ch][i];
                audio.outputs[ch][i] = self.buf[ch][pos];
                self.buf[ch][pos] = x;
                pos = (pos + 1) % len;
            }
            if ch == 1 {
                self.pos = pos;
            }
        }
        ProcessStatus::Continue
    }
    fn latency(&self) -> u32 {
        self.samples as u32
    }
}

/// What a [`Recorder`] saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seen {
    /// Absolute sample time + event.
    Event(u64, EventKind),
    /// A process call: (sample_time, frames, position, bpm, beats_per_sample).
    Block(u64, usize, f64, f64, f64),
}

/// Records every event and block through an SPSC ring (no allocation in `process`), and
/// outputs silence.
pub struct Recorder {
    tx: Producer<Seen>,
}

impl Recorder {
    pub fn new() -> (Self, Consumer<Seen>) {
        let (tx, rx) = RingBuffer::new(1 << 16);
        (Self { tx }, rx)
    }
}

impl Node for Recorder {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let t = ctx.transport;
        let _ = self.tx.push(Seen::Block(
            t.sample_time,
            ctx.frames,
            t.position,
            t.bpm,
            t.beats_per_sample,
        ));
        for e in ctx.events {
            let _ = self
                .tx
                .push(Seen::Event(t.sample_time + e.offset as u64, e.kind));
        }
        audio.pass_through();
        ProcessStatus::Continue
    }
}

pub fn drain(rx: &mut Consumer<Seen>) -> Vec<Seen> {
    let mut v = vec![];
    while let Ok(s) = rx.pop() {
        v.push(s);
    }
    v
}

pub fn events(seen: &[Seen]) -> Vec<(u64, EventKind)> {
    seen.iter()
        .filter_map(|s| match s {
            Seen::Event(t, k) => Some((*t, *k)),
            _ => None,
        })
        .collect()
}

/// In-memory audio source (the `ether-media` equivalent for tests).
pub struct MemSource(pub Vec<f32>);

impl AudioSource for MemSource {
    fn channels(&self) -> u16 {
        1
    }
    fn frames(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, _channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.0.get(start as usize + i).copied().unwrap_or(0.0);
        }
        true
    }
}

pub fn mem_source(frames: usize) -> Arc<dyn AudioSource> {
    Arc::new(MemSource(
        (0..frames).map(|i| (i % 1000) as f32 / 1000.0).collect(),
    ))
}

/// Stereo in, mono out: sums its inputs into one channel.
pub struct MonoSum;

impl Node for MonoSum {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let n = audio.outputs[0].len();
        for i in 0..n {
            audio.outputs[0][i] = audio.inputs.iter().map(|c| c[i]).sum::<f32>() * 0.5;
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (2, 1)
    }
}

/// MIDI effect: emits `per_block` note-ons per call into `out_events` (for the next
/// device), passing audio through. A large `per_block` overflows the event buffers.
pub struct NoteEmitter {
    pub per_block: usize,
}

impl Node for NoteEmitter {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for i in 0..self.per_block {
            let _ = ctx.out_events.push(ether_core::ProcessEvent {
                offset: (i % ctx.frames) as u32,
                kind: EventKind::NoteOn {
                    note_id: i as u32,
                    channel: 0,
                    key: 60,
                    velocity: 1.0,
                },
            });
        }
        ctx.out_events.sort();
        audio.pass_through();
        ProcessStatus::Continue
    }
}

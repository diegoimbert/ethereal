//! Silence skipping with the real devices in the real engine: a synth note,
//! a long silence, another note, through each effect, rendered with skipping on and off.
//!
//! - tails are not cut: reverb, delay, convolution reverb, chorus, compressor release ring
//!   out the same (within -100 dBFS) and the skipped render still has its tail;
//! - after the silence the second note sounds the same (state, LFO phase caught up);
//! - the devices really are skipped (process calls counted) once their tail is over;
//! - `Engine::process` stays allocation-free on the skipping path.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use assert_no_alloc::assert_no_alloc;
use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, TrackDesc};
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, ClipId, TrackId, TrackKind, Ulid,
};
use ether_core::{
    AudioBuffers, Device, EngineConfig, Node, PrepareConfig, ProcessContext, ProcessStatus,
    RenderGraphDesc, TransportControl,
};
use ether_devices::NoSamples;

const SR: u32 = 48_000;
const BLOCK: usize = 256;
/// 120 bpm: 2 beats per second.
const SECONDS: usize = 12;

/// Counts `process` calls of the wrapped device.
struct Counted {
    inner: Box<dyn Device>,
    calls: Arc<AtomicUsize>,
}

impl Node for Counted {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.inner.prepare(config);
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.inner.process(ctx, audio)
    }
    fn latency(&self) -> u32 {
        self.inner.latency()
    }
    fn channels(&self) -> (u16, u16) {
        self.inner.channels()
    }
    fn wake_requested(&mut self) -> bool {
        self.inner.wake_requested()
    }
}

fn device(ty: BuiltinDeviceType) -> Box<dyn Device> {
    let mut d = BuiltinDevice::new(ty);
    // The short factory room (a new one starts on the 2.8 s hall, which holds the device
    // awake for most of the 9 s between the notes).
    if let BuiltinDevice::ConvolutionReverb { ir } = &mut d {
        *ir = Some(ether_core::protocol::model::IrSource::Factory { id: "room".into() });
    }
    ether_devices::create(&d, &NoSamples)
}

fn track(chain: Vec<ChainEntry>) -> TrackDesc {
    let note = |start: f64| NoteDesc {
        start,
        duration: 0.5,
        key: 57,
        velocity: 0.9,
        release_velocity: 0.0,
    };
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: TrackId(Ulid(2)),
        kind: TrackKind::Midi,
        chain,
        output: Some(TrackId(Ulid(1))),
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        midi_input: Default::default(),
        monitor: false,
        armed: false,
        clips: vec![ClipDesc {
            id: ClipId(Ulid(9)),
            start: 0.0,
            length: (SECONDS * 2) as f64,
            offset: 0.0,
            looping: None,
            muted: false,
            // A note at 0 s, another at 9 s.
            content: ClipContentDesc::Midi {
                notes: vec![note(0.0), note(18.0)],
            },
            envelopes: vec![],
        }],
        automation: vec![],
        racks: vec![],
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
        midi_sidechains: Vec::new(),
    }
}

fn master() -> TrackDesc {
    TrackDesc {
        id: TrackId(Ulid(1)),
        kind: TrackKind::Master,
        output: None,
        clips: vec![],
        chain: vec![],
        ..track(vec![])
    }
}

/// Render synth → `effect`; returns the left channel and the effect's process calls.
fn render(effect: BuiltinDeviceType, skipping: bool) -> (Vec<f32>, usize) {
    let mut p = ether_core::create(EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_nodes: 16,
        max_events_per_block: 256,
        ..EngineConfig::default()
    });
    p.engine.set_silence_skipping(skipping);
    let calls = Arc::new(AtomicUsize::new(0));
    let synth = p.handle.add_node(device(BuiltinDeviceType::Synth)).unwrap();
    let fx = p
        .handle
        .add_node(Box::new(Counted {
            inner: device(effect),
            calls: calls.clone(),
        }))
        .unwrap();
    let entry = |node| ChainEntry {
        node,
        enabled: true,
        sidechain: None,
    };
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), track(vec![entry(synth), entry(fx)])],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let frames = SECONDS * SR as usize;
    let mut out = Vec::with_capacity(frames);
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    while out.len() < frames {
        {
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            assert_no_alloc(|| p.engine.process(&[], &mut outs, BLOCK));
        }
        out.extend_from_slice(&l);
    }
    (out, calls.load(Ordering::Relaxed))
}

/// Last sample louder than -100 dBFS.
fn tail_end(x: &[f32], from: usize, to: usize) -> usize {
    (from..to)
        .rev()
        .find(|&i| x[i].abs() > 1e-5)
        .unwrap_or(from)
}

#[test]
fn tails_ring_out_and_the_next_note_matches() {
    let blocks = SECONDS * SR as usize / BLOCK;
    let second = 9 * SR as usize;
    for (effect, min_tail_s) in [
        (BuiltinDeviceType::Reverb, 1.0),
        (BuiltinDeviceType::Delay, 1.0),
        (BuiltinDeviceType::ConvolutionReverb, 0.0),
        (BuiltinDeviceType::Compressor, 0.0),
        (BuiltinDeviceType::Chorus, 0.0),
        (BuiltinDeviceType::Flanger, 0.0),
        (BuiltinDeviceType::Phaser, 0.0),
        (BuiltinDeviceType::Tremolo, 0.0),
        (BuiltinDeviceType::AutoFilter, 0.0),
        (BuiltinDeviceType::Eq, 0.0),
        (BuiltinDeviceType::Limiter, 0.0),
        (BuiltinDeviceType::Gate, 0.0),
        (BuiltinDeviceType::MultibandCompressor, 0.0),
        (BuiltinDeviceType::Saturator, 0.0),
    ] {
        let (full, all_calls) = render(effect, false);
        let (skip, calls) = render(effect, true);
        assert_eq!(
            all_calls, blocks,
            "{effect:?}: processed every block without skipping"
        );
        // The first note's tail (the synth itself stops ~0.5 s in).
        let end_full = tail_end(&full, 0, second);
        let end_skip = tail_end(&skip, 0, second);
        assert!(
            end_full as f32 >= (0.25 + min_tail_s) * SR as f32,
            "{effect:?}: tail ends at {end_full}"
        );
        assert!(
            end_skip + 64 >= end_full,
            "{effect:?}: tail cut at {end_skip} (full: {end_full})"
        );
        let diff = |range: std::ops::Range<usize>| {
            range
                .map(|i| (full[i] - skip[i]).abs())
                .fold(0.0f32, f32::max)
        };
        let (first, next) = (diff(0..second), diff(second..full.len()));
        assert!(first < 1e-5, "{effect:?}: tail differs by {first}");
        assert!(next < 1e-4, "{effect:?}: next note differs by {next}");
        assert!(
            calls < blocks * 3 / 4,
            "{effect:?}: never slept ({calls} of {blocks} blocks)"
        );
        println!(
            "{effect:?}: tail {:.2} s, slept {} of {blocks} blocks, max diff {first:.1e} / {next:.1e}",
            end_full as f32 / SR as f32,
            blocks - calls
        );
    }
}

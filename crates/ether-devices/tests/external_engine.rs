//! External devices inside the real engine (`external-instrument`, CONTRACTS.md §13.7),
//! against a simulated hardware loop (no device on the devbox / CI):
//!
//! - the effect's returned audio is aligned with the dry signal once `Latency` is the
//!   measured round trip (`EngineHandle::measure_hw_latency`);
//! - an instrument's notes reach the MIDI ring with sample-accurate engine timestamps on
//!   its channel, and measuring it times a note → audio round trip;
//! - a missing return channel is silent and resumes when the channel reappears;
//! - `Engine::process` stays allocation-free with hardware I/O.

use assert_no_alloc::assert_no_alloc;
use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, TrackDesc};
use ether_core::hw_io::{HwIoDesc, HwMidiEvent};
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, ClipId, ExternalRouting, HwChannels, TrackId, TrackKind, Ulid,
};
use ether_core::{
    AudioBuffers, Device, EngineConfig, EngineParts, Node, NodeKey, PrepareConfig, ProcessContext,
    ProcessStatus, RenderGraphDesc, TransportControl,
};
use ether_devices::NoSamples;
use ether_devices::external::{external_audio_effect as fx, external_instrument as inst};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const CHANNELS: usize = 4;

fn config() -> EngineConfig {
    EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        input_channels: CHANNELS as u16,
        output_channels: CHANNELS as u16,
        max_nodes: 64,
        max_events_per_block: 256,
        ..EngineConfig::default()
    }
}

fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

fn track(id: TrackId, kind: TrackKind, output: Option<TrackId>, chain: &[NodeKey]) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id,
        kind,
        chain: chain
            .iter()
            .map(|&node| ChainEntry {
                node,
                enabled: true,
                sidechain: None,
            })
            .collect(),
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
        racks: vec![],
        expression: Default::default(),
        hw_io: Vec::new(),
    }
}

fn graph(tracks: Vec<TrackDesc>) -> RenderGraphDesc {
    let mut all = vec![track(tid(1), TrackKind::Master, None, &[])];
    all.extend(tracks);
    RenderGraphDesc {
        tracks: all,
        ..Default::default()
    }
}

/// A test signal: a deterministic function of the engine sample time.
fn signal(t: u64) -> f32 {
    ((t as f32) * 0.0123).sin() * 0.4 + ((t as f32) * 0.0031).sin() * 0.2
}

/// Instrument-like source playing [`signal`].
struct Source;

impl Node for Source {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let t0 = ctx.transport.sample_time;
        for ch in audio.outputs.iter_mut() {
            for (i, s) in ch.iter_mut().enumerate() {
                *s = signal(t0 + i as u64);
            }
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

fn device(
    ty: BuiltinDeviceType,
    params: &[(ether_core::protocol::model::ParamId, f64)],
) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR as f32,
        max_block_size: BLOCK,
        max_events_per_block: 256,
    });
    for &(p, v) in params {
        d.set_param(p, v);
    }
    d
}

/// The simulated hardware: hardware outputs `from..from+2` come back on inputs
/// `to..to+2` `delay` samples later (an analog loopback cable), and MIDI note-ons trigger a
/// click on input `click_to` `synth_delay` samples after their timestamp (a synth).
struct Hardware {
    delay: usize,
    from: usize,
    to: usize,
    /// Pending input samples per input channel, indexed by absolute sample time.
    lines: Vec<Vec<f32>>,
    synth: Option<(usize, u64)>,
    midi: Vec<HwMidiEvent>,
    io: Option<ether_core::hw_io::HwIoIo>,
}

impl Hardware {
    fn new(delay: usize, from: usize, to: usize, frames: usize) -> Self {
        Self {
            delay,
            from,
            to,
            lines: vec![vec![0.0; frames + delay + BLOCK]; CHANNELS * 2],
            synth: None,
            midi: Vec::new(),
            io: None,
        }
    }
}

/// Run `blocks` blocks with `hw` in the loop; returns the master output (left) per sample.
fn run(
    p: &mut EngineParts,
    hw: &mut Hardware,
    start: u64,
    blocks: usize,
    inputs: usize,
) -> Vec<f32> {
    if hw.io.is_none() {
        hw.io = p.handle.take_hw_io();
    }
    let mut master = Vec::new();
    let mut outs = vec![vec![0.0f32; BLOCK]; CHANNELS];
    let mut ins = vec![vec![0.0f32; BLOCK]; inputs];
    for b in 0..blocks {
        let t = start as usize + b * BLOCK;
        for (ch, buf) in ins.iter_mut().enumerate() {
            match hw.lines.get(ch) {
                Some(line) => buf.copy_from_slice(&line[t..t + BLOCK]),
                None => buf.fill(0.0),
            }
        }
        {
            let i: Vec<&[f32]> = ins.iter().map(|c| c.as_slice()).collect();
            let mut o: Vec<&mut [f32]> = outs.iter_mut().map(|c| c.as_mut_slice()).collect();
            assert_no_alloc(|| p.engine.process(&i, &mut o, BLOCK));
        }
        master.extend_from_slice(&outs[0]);
        for k in 0..2 {
            let src = &outs[hw.from + k];
            let line = &mut hw.lines[hw.to + k];
            for (i, s) in src.iter().enumerate() {
                line[t + i + hw.delay] += s;
            }
        }
        if let Some(mut io) = hw.io.take() {
            while let Ok(m) = io.midi.pop() {
                if let Some((ch, d)) = hw.synth
                    && m.data[0] & 0xf0 == 0x90
                {
                    let at = (m.frame + d) as usize;
                    if at < hw.lines[ch].len() {
                        hw.lines[ch][at] += 0.8;
                    }
                }
                hw.midi.push(m);
            }
            hw.io = Some(io);
        }
    }
    master
}

fn effect_routing() -> ExternalRouting {
    ExternalRouting {
        midi_out: None,
        midi_channel: 1,
        audio_send: Some(HwChannels { first: 2, count: 2 }),
        audio_return: Some(HwChannels { first: 2, count: 2 }),
    }
}

/// Acceptance: measure the loop, set `Latency` to it, and the effect's returned audio is
/// aligned with the dry signal (a 50 % mix is the signal delayed, not a comb).
#[test]
fn measured_effect_return_is_aligned_with_the_dry_signal() {
    let delay = 1000usize;
    let mut p = ether_core::create(config());
    let src = p.handle.add_node(Box::new(Source)).unwrap();
    let fx_node = p
        .handle
        .add_node(device(
            BuiltinDeviceType::ExternalAudioEffect,
            &[(fx::MIX, 50.0)],
        ))
        .unwrap();
    let mut t = track(tid(2), TrackKind::Audio, Some(tid(1)), &[src, fx_node]);
    t.hw_io = vec![HwIoDesc {
        node: fx_node,
        routing: effect_routing(),
    }];
    p.handle.publish(graph(vec![t.clone()])).unwrap();
    let frames = SR as usize * 4;
    let mut hw = Hardware::new(delay, 2, 2, frames);

    // Measure (the program is muted on the send while measuring).
    p.handle.measure_hw_latency(fx_node).unwrap();
    let blocks = SR as usize / BLOCK;
    run(&mut p, &mut hw, 0, blocks, CHANNELS);
    let r = p.handle.poll_hw_latency().expect("measured");
    assert_eq!(r.node, fx_node);
    assert_eq!(r.samples, Some(delay as u32));

    // The controller sets `Latency` (one undoable edit) and republishes PDC: here, a new
    // node with the measured value.
    let ms = ether_devices::external::latency_ms(delay as u32, SR as f32);
    let fx2 = p
        .handle
        .add_node(device(
            BuiltinDeviceType::ExternalAudioEffect,
            &[(fx::MIX, 50.0), (fx::LATENCY, ms)],
        ))
        .unwrap();
    t.chain[1].node = fx2;
    t.hw_io[0].node = fx2;
    p.handle.publish(graph(vec![t])).unwrap();
    assert_eq!(p.handle.latency(), delay as u32);
    let start = (blocks * BLOCK) as u64;
    let out = run(&mut p, &mut hw, start, blocks * 2, CHANNELS);
    // Skip the switch-over and the first round trip.
    let settle = delay * 3;
    for (i, s) in out.iter().enumerate().skip(settle) {
        let at = start + i as u64;
        let want = signal(at - delay as u64);
        assert!((s - want).abs() < 1e-4, "sample {at}: {s} vs {want}");
    }
}

#[test]
fn effect_without_routing_back_is_silent_and_measuring_it_fails() {
    let mut p = ether_core::create(config());
    let src = p.handle.add_node(Box::new(Source)).unwrap();
    let fx_node = p
        .handle
        .add_node(device(BuiltinDeviceType::ExternalAudioEffect, &[]))
        .unwrap();
    let mut t = track(tid(2), TrackKind::Audio, Some(tid(1)), &[src, fx_node]);
    t.hw_io = vec![HwIoDesc {
        node: fx_node,
        routing: effect_routing(),
    }];
    p.handle.publish(graph(vec![t])).unwrap();
    // Nothing connected: the click never comes back.
    let mut hw = Hardware::new(1000, 0, 0, SR as usize * 3);
    hw.lines.iter_mut().for_each(|l| l.fill(0.0));
    hw.from = 0;
    hw.to = CHANNELS; // out 0/1 → nowhere the engine reads
    p.handle.measure_hw_latency(fx_node).unwrap();
    let out = run(&mut p, &mut hw, 0, (SR as usize * 5 / 2) / BLOCK, CHANNELS);
    assert_eq!(p.handle.poll_hw_latency().unwrap().samples, None);
    // Mix 100 % and nothing returning: silence on the master.
    assert!(out.iter().all(|s| *s == 0.0));
}

fn midi_track(inst_node: NodeKey, routing: ExternalRouting) -> TrackDesc {
    let mut t = track(tid(3), TrackKind::Midi, Some(tid(1)), &[inst_node]);
    t.clips = vec![ClipDesc {
        id: ClipId(Ulid(9)),
        start: 0.0,
        length: 8.0,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Midi {
            notes: vec![
                NoteDesc {
                    start: 1.0,
                    duration: 0.5,
                    key: 64,
                    velocity: 1.0,
                    release_velocity: 0.0,
                },
                NoteDesc {
                    start: 2.25,
                    duration: 1.0,
                    key: 67,
                    velocity: 0.5,
                    release_velocity: 0.0,
                },
            ],
        },
        envelopes: vec![],
    }];
    t.hw_io = vec![HwIoDesc {
        node: inst_node,
        routing,
    }];
    t
}

fn instrument_routing(ret: u16) -> ExternalRouting {
    ExternalRouting {
        midi_out: Some("Synth".into()),
        midi_channel: 3,
        audio_send: None,
        audio_return: Some(HwChannels {
            first: ret,
            count: 1,
        }),
    }
}

#[test]
fn instrument_notes_reach_the_midi_ring_sample_accurately() {
    let mut p = ether_core::create(config());
    let node = p
        .handle
        .add_node(device(BuiltinDeviceType::ExternalInstrument, &[]))
        .unwrap();
    p.handle
        .publish(graph(vec![midi_track(node, instrument_routing(2))]))
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let mut hw = Hardware::new(BLOCK, 0, CHANNELS, SR as usize * 3);
    run(&mut p, &mut hw, 0, SR as usize * 2 / BLOCK, CHANNELS);
    // 120 bpm: one beat = 24 000 samples. Channel 3 = status nibble 2.
    let spb = 24_000u64;
    let notes: Vec<(u64, [u8; 3])> = hw.midi.iter().map(|m| (m.frame, m.data)).collect();
    assert!(hw.midi.iter().all(|m| m.node == node));
    assert_eq!(
        notes,
        vec![
            (spb, [0x92, 64, 127]),
            (spb * 3 / 2, [0x82, 64, 0]),
            (spb * 9 / 4, [0x92, 67, 64]),
            (spb * 13 / 4, [0x82, 67, 0]),
        ]
    );
    // Stop: all notes off on the channel.
    p.handle.transport(TransportControl::Stop).unwrap();
    hw.midi.clear();
    run(
        &mut p,
        &mut hw,
        (SR as usize * 2 / BLOCK * BLOCK) as u64,
        2,
        CHANNELS,
    );
    assert!(
        hw.midi.iter().any(|m| m.data == [0xb2, 123, 0]),
        "{:?}",
        hw.midi
    );
}

#[test]
fn measuring_an_instrument_times_note_to_audio() {
    let synth_delay = 777u64;
    let mut p = ether_core::create(config());
    let node = p
        .handle
        .add_node(device(BuiltinDeviceType::ExternalInstrument, &[]))
        .unwrap();
    let mut t = midi_track(node, instrument_routing(3));
    t.clips.clear();
    p.handle.publish(graph(vec![t])).unwrap();
    let mut hw = Hardware::new(BLOCK, 0, CHANNELS, SR as usize * 2);
    hw.synth = Some((3, synth_delay));
    p.handle.measure_hw_latency(node).unwrap();
    run(&mut p, &mut hw, 0, SR as usize / BLOCK, CHANNELS);
    let r = p.handle.poll_hw_latency().expect("measured");
    assert_eq!(r.samples, Some(synth_delay as u32));
    // The measurement note was released.
    assert_eq!(hw.midi.first().map(|m| m.data), Some([0x92, 60, 127]));
    assert!(hw.midi.iter().any(|m| m.data == [0x82, 60, 0]));
    let _ = inst::GAIN;
}

/// A return channel the interface doesn't have plays silence; when the channel reappears
/// (more inputs, e.g. the interface reconnected) the device resumes.
#[test]
fn missing_return_channel_is_silent_and_resumes() {
    let mut p = ether_core::create(config());
    let node = p
        .handle
        .add_node(device(BuiltinDeviceType::ExternalInstrument, &[]))
        .unwrap();
    let mut t = midi_track(node, instrument_routing(3));
    t.clips.clear();
    p.handle.publish(graph(vec![t])).unwrap();
    let mut hw = Hardware::new(BLOCK, 0, CHANNELS, SR as usize);
    hw.lines[3].fill(0.25);
    // Only two inputs: channel 3 is missing.
    let out = run(&mut p, &mut hw, 0, 8, 2);
    assert!(out.iter().all(|s| *s == 0.0));
    // Back: the return plays.
    let out = run(&mut p, &mut hw, (8 * BLOCK) as u64, 8, CHANNELS);
    assert!(
        out.iter().all(|s| (*s - 0.25).abs() < 1e-6),
        "{:?}",
        &out[..4]
    );
}

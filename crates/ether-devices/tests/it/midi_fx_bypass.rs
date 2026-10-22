//! Every built-in MIDI effect, switched off, is MIDI thru exactly as if it were absent
//! (OSS #191), on a track chain and on (nested) instrument rack chains built from the real
//! rack devices; automating its switch while notes are held leaves no stuck note and lets
//! the notes played while it is off through untouched.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, ParamMapping, ResolvedTarget,
    TrackDesc,
};
use ether_core::protocol::devices::{DeviceCategory, ParamScale};
use ether_core::protocol::model::{
    AutomationTarget, BuiltinDevice, BuiltinDeviceType, ClipId, CurveShape, RackChainId, TrackId,
    TrackKind, Ulid,
};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, EngineConfig, EngineParts, EventKind, Node, NodeKey, PrepareConfig,
    ProcessContext, ProcessStatus, RenderGraphDesc, TransportControl,
};
use ether_devices::NoSamples;

const SR: u32 = 48_000;
/// Divides no beat.
const BLOCK: usize = 377;
const BEAT: u64 = 24_000; // 120 bpm

type Log = Arc<Mutex<Vec<(u64, EventKind)>>>;

/// Instrument stand-in: logs the note events it gets (absolute sample time), outputs silence.
struct Rec(Log);

impl Node for Rec {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let t = ctx.transport.sample_time;
        let mut log = self.0.lock().unwrap();
        for e in ctx.events {
            if matches!(
                e.kind,
                EventKind::NoteOn { .. } | EventKind::NoteOff { .. } | EventKind::AllNotesOff
            ) {
                log.push((t + e.offset as u64, e.kind));
            }
        }
        for ch in audio.outputs.iter_mut() {
            ch.fill(0.0);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// The implemented built-in MIDI effects (MIDI effect rack included).
fn midi_effects() -> Vec<BuiltinDeviceType> {
    let all: Vec<_> = BuiltinDeviceType::ALL
        .into_iter()
        .filter(|&ty| {
            let d = ether_devices::descriptor(ty);
            d.category == DeviceCategory::NoteEffect && d.implemented
        })
        .collect();
    // The `midi-fx` set at least.
    for ty in [
        BuiltinDeviceType::Arpeggiator,
        BuiltinDeviceType::Chord,
        BuiltinDeviceType::ScaleQuantize,
        BuiltinDeviceType::NoteLength,
        BuiltinDeviceType::Velocity,
        BuiltinDeviceType::Randomizer,
        BuiltinDeviceType::Pitch,
    ] {
        assert!(all.contains(&ty), "{ty:?} is a MIDI effect");
    }
    all
}

#[derive(Clone, Copy, Debug)]
enum Place {
    Track,
    Rack,
    NestedRack,
}

fn entry(node: NodeKey, enabled: bool) -> ChainEntry {
    ChainEntry {
        node,
        enabled,
        sidechain: None,
    }
}

fn rack_chain(id: u128, chain: Vec<ChainEntry>) -> RackChainDesc {
    RackChainDesc {
        id: RackChainId(Ulid(id)),
        chain,
        volume: 1.0,
        pan: 0.0,
        mute: false,
        keys: (0, 127),
        velocities: (0, 127),
        select: (0, 127),
    }
}

fn track(notes: &[(f64, f64, u8, f32)]) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: TrackId(Ulid(2)),
        kind: TrackKind::Midi,
        chain: vec![],
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
            length: 64.0,
            offset: 0.0,
            looping: None,
            muted: false,
            content: ClipContentDesc::Midi {
                notes: notes
                    .iter()
                    .map(|&(start, duration, key, velocity)| NoteDesc {
                        start,
                        duration,
                        key,
                        velocity,
                        release_velocity: 0.0,
                    })
                    .collect(),
            },
            envelopes: vec![],
        }],
        automation: vec![],
        racks: vec![],
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
    }
}

/// Renders `notes` into the logging instrument, after `fx` (`None`: no effect) at `place`
/// with its switch at `enabled` and the `automation` of its switch (if any).
fn run(
    place: Place,
    fx: Option<BuiltinDeviceType>,
    enabled: bool,
    automate: Option<&[(f64, f64)]>,
    notes: &[(f64, f64, u8, f32)],
    beats: u64,
) -> Vec<(u64, EventKind)> {
    let mut p: EngineParts = ether_core::create(EngineConfig {
        sample_rate: SR,
        max_block_size: 512,
        max_nodes: 16,
        max_events_per_block: 256,
        ..EngineConfig::default()
    });
    let log: Log = Default::default();
    let rec = p.handle.add_node(Box::new(Rec(log.clone()))).unwrap();
    let fx = fx.map(|ty| {
        p.handle
            .add_node(ether_devices::create(&BuiltinDevice::new(ty), &NoSamples))
            .unwrap()
    });
    let mut inner: Vec<ChainEntry> = fx.iter().map(|&n| entry(n, enabled)).collect();
    inner.push(entry(rec, true));
    let mut t = track(notes);
    let rack = |p: &mut EngineParts| {
        p.handle
            .add_node(ether_devices::create(
                &BuiltinDevice::new(BuiltinDeviceType::InstrumentRack),
                &NoSamples,
            ))
            .unwrap()
    };
    let desc = |rack, id, chain| ChainRackDesc {
        rack,
        kind: ChainRackKind::Instrument,
        chains: vec![rack_chain(id, chain)],
        selector: 0,
    };
    match place {
        Place::Track => t.chain = inner,
        Place::Rack => {
            let r = rack(&mut p);
            t.chain = vec![entry(r, true)];
            t.chain_racks = vec![desc(r, 1, inner)];
        }
        Place::NestedRack => {
            let outer = rack(&mut p);
            let r = rack(&mut p);
            t.chain = vec![entry(outer, true)];
            t.chain_racks = vec![desc(outer, 1, vec![entry(r, true)]), desc(r, 2, inner)];
        }
    }
    if let (Some(node), Some(points)) = (fx, automate) {
        t.automation = vec![AutomationDesc {
            target: AutomationTarget::TrackVolume { track: t.id },
            resolved: ResolvedTarget::NodeEnabled { node },
            points: points
                .iter()
                .map(|&(beat, v)| (beat, v, CurveShape::Step))
                .collect(),
            mapping: ParamMapping {
                min: 0.0,
                max: 1.0,
                scale: ParamScale::Linear,
                steps: Some(2),
            },
        }];
    }
    let master = TrackDesc {
        id: TrackId(Ulid(1)),
        kind: TrackKind::Master,
        output: None,
        clips: vec![],
        ..track(&[])
    };
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master, t],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut done = 0;
    while done < beats * BEAT {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        p.engine.process(&[], &mut outs, BLOCK);
        done += BLOCK as u64;
    }
    log.lock().unwrap().clone()
}

const NOTES: [(f64, f64, u8, f32); 4] = [
    (0.0, 1.0, 60, 0.8),
    (0.5, 1.0, 64, 0.3),
    (1.25, 0.25, 67, 1.0),
    (2.0, 0.5, 61, 0.55),
];

#[test]
fn every_midi_effect_switched_off_is_as_if_absent() {
    for place in [Place::Track, Place::Rack, Place::NestedRack] {
        let absent = run(place, None, true, None, &NOTES, 3);
        assert_eq!(
            absent
                .iter()
                .filter(|(_, k)| matches!(k, EventKind::NoteOn { .. }))
                .count(),
            NOTES.len(),
            "{place:?}"
        );
        for ty in midi_effects() {
            let off = run(place, Some(ty), false, None, &NOTES, 3);
            assert_eq!(off, absent, "{ty:?} off, {place:?}");
        }
    }
}

/// Keys still sounding at the end of `log` (`AllNotesOff` releases all).
fn stuck(log: &[(u64, EventKind)]) -> BTreeSet<(u8, u8)> {
    let mut held = BTreeSet::new();
    for (_, k) in log {
        match *k {
            EventKind::NoteOn { channel, key, .. } => {
                held.insert((channel, key));
            }
            EventKind::NoteOff { channel, key, .. } => {
                held.remove(&(channel, key));
            }
            EventKind::AllNotesOff => held.clear(),
            _ => {}
        }
    }
    held
}

#[test]
fn automating_any_midi_effects_switch_strands_and_drops_no_notes() {
    // On, off at 1 (the first note held), on at 3 (the second held).
    let switch = [(0.0, 1.0), (1.0, 0.0), (3.0, 1.0)];
    let notes = [
        (0.0, 2.0, 60, 0.8),
        (2.5, 1.0, 64, 0.3),
        (4.0, 0.5, 67, 1.0),
    ];
    for place in [Place::Track, Place::Rack, Place::NestedRack] {
        for ty in midi_effects() {
            let what = format!("{ty:?}, {place:?}");
            let log = run(place, Some(ty), true, Some(&switch), &notes, 8);
            assert!(stuck(&log).is_empty(), "{what}: stuck {:?}", stuck(&log));
            // Played while off: untouched, on time.
            let at = (2.5 * BEAT as f64) as u64;
            assert!(
                log.iter().any(|&(t, k)| t == at
                    && matches!(k, EventKind::NoteOn { key: 64, velocity, .. }
                        if (velocity - 0.3).abs() < 1e-6)),
                "{what}: the note played while off: {log:?}"
            );
            // Back on: the effect plays the last note.
            assert!(
                log.iter()
                    .any(|&(t, k)| t >= 4 * BEAT && matches!(k, EventKind::NoteOn { .. })),
                "{what}: nothing after switching back on"
            );
        }
    }
}

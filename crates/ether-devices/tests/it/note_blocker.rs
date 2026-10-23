//! Note Blocker in the real engine (`midi_fx::NoteBlocker`, issue #192): it silences the
//! instrument after it on a track chain, in an instrument rack chain, inside a MIDI effect rack
//! and in nested racks; notes held when it is added, switched on or automated on are
//! released, and switching it off (or removing it) never retriggers them. `Block Everything`
//! drops live CC / pitch bend too.

use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, ParamMapping, RenderGraphDesc,
    ResolvedTarget, TrackDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, BuiltinDevice, BuiltinDeviceType, ClipId, CurveShape, RackChainId, TrackId,
    TrackKind, Ulid,
};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::recording::{LiveMidi, UNTAGGED_PORT};
use ether_core::{
    AudioBuffers, Device, EngineConfig, EngineParts, EventKind, Node, NodeKey, PrepareConfig,
    ProcessContext, ProcessStatus, TransportControl,
};
use ether_devices::NoSamples;
use ether_devices::midi_fx::note_blocker;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
/// Samples per beat at 120 bpm.
const BEAT: usize = 24_000;

fn config() -> EngineConfig {
    EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_nodes: 64,
        max_events_per_block: 256,
        ..EngineConfig::default()
    }
}

/// Instrument: left = 1.0 while any of its voices sounds (a voice ends with its note-off or
/// choke by id, or `AllNotesOff`; no release time), right = the raw non-note MIDI messages
/// it got so far.
struct Voices {
    ids: Vec<u32>,
    midi: u32,
}

impl Node for Voices {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.ids.clear();
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            match e.kind {
                EventKind::NoteOn { note_id, .. } if self.ids.len() < self.ids.capacity() => {
                    self.ids.push(note_id)
                }
                EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                    self.ids.retain(|&id| id != note_id)
                }
                EventKind::AllNotesOff => self.ids.clear(),
                EventKind::Midi { data } if !matches!(data[0] & 0xF0, 0x80 | 0x90) => {
                    self.midi += 1
                }
                _ => {}
            }
        }
        let v = if self.ids.is_empty() { 0.0 } else { 1.0 };
        let [l, r] = audio.outputs else {
            return ProcessStatus::Continue;
        };
        l.fill(v);
        r.fill(self.midi as f32);
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

fn voices() -> Box<Voices> {
    Box::new(Voices {
        ids: Vec::with_capacity(64),
        midi: 0,
    })
}

fn device(ty: BuiltinDeviceType, block_everything: bool) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR as f32,
        max_block_size: BLOCK,
        max_events_per_block: 256,
    });
    if block_everything {
        d.set_param(note_blocker::BLOCK_EVERYTHING, 1.0);
    }
    d
}

fn blocker(p: &mut EngineParts) -> NodeKey {
    p.handle
        .add_node(device(BuiltinDeviceType::NoteBlocker, false))
        .unwrap()
}

fn entries(chain: &[(NodeKey, bool)]) -> Vec<ChainEntry> {
    chain
        .iter()
        .map(|&(node, enabled)| ChainEntry {
            node,
            enabled,
            sidechain: None,
        })
        .collect()
}

fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

fn track(id: TrackId, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
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
        midi_input: Default::default(),
        monitor: false,
        armed: false,
        clips: vec![],
        automation: vec![],
        racks: vec![],
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
        midi_sidechains: Vec::new(),
    }
}

/// A MIDI track playing `notes` (start, duration, key in beats).
fn midi_track(chain: &[(NodeKey, bool)], notes: &[(f64, f64, u8)]) -> TrackDesc {
    let mut t = track(tid(2), TrackKind::Midi, Some(tid(1)));
    t.chain = entries(chain);
    t.clips = vec![ClipDesc {
        id: ClipId(Ulid(1)),
        start: 0.0,
        length: 16.0,
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
    }];
    t
}

fn rack(rack: NodeKey, kind: ChainRackKind, chain: &[(NodeKey, bool)]) -> ChainRackDesc {
    let id = match kind {
        ChainRackKind::MidiEffect => 2,
        _ => 1,
    };
    ChainRackDesc {
        rack,
        kind,
        chains: vec![RackChainDesc {
            id: RackChainId(Ulid(id)),
            chain: entries(chain),
            volume: 1.0,
            pan: 0.0,
            mute: false,
            keys: (0, 127),
            velocities: (0, 127),
            select: (0, 127),
        }],
        selector: 0,
    }
}

fn graph(version: u64, t: TrackDesc) -> RenderGraphDesc {
    RenderGraphDesc {
        version,
        tracks: vec![track(tid(1), TrackKind::Master, None), t],
        ..Default::default()
    }
}

/// Renders `beats`; returns the last sample (left, right).
fn render(p: &mut EngineParts, beats: f64) -> (f32, f32) {
    let frames = (beats * BEAT as f64) as usize;
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut done = 0;
    let mut last = (0.0, 0.0);
    while done < frames {
        let n = BLOCK.min(frames - done);
        let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
        p.engine.process(&[], &mut outs, n);
        last = (l[n - 1], r[n - 1]);
        done += n;
    }
    last
}

fn sounds_after(p: &mut EngineParts, beats: f64) -> bool {
    render(p, beats).0 > 0.5
}

/// The same chain shapes, with the blocker `on` or off, around one instrument.
#[derive(Clone, Copy, Debug)]
enum Place {
    Track,
    /// Instrument rack chain [blocker, synth].
    InstrumentRack,
    /// MIDI effect rack chain [blocker], then the synth on the track.
    MidiEffectRack,
    /// Instrument rack chain [MIDI effect rack chain [blocker], synth].
    Nested,
}

struct Rig {
    p: EngineParts,
    blocker: NodeKey,
    synth: NodeKey,
    outer: NodeKey,
    inner: NodeKey,
    place: Place,
}

impl Rig {
    fn new(place: Place) -> Self {
        let mut p = ether_core::create(config());
        let blocker = blocker(&mut p);
        let synth = p.handle.add_node(voices()).unwrap();
        let outer = p
            .handle
            .add_node(device(BuiltinDeviceType::InstrumentRack, false))
            .unwrap();
        let inner = p
            .handle
            .add_node(device(BuiltinDeviceType::MidiEffectRack, false))
            .unwrap();
        Self {
            p,
            blocker,
            synth,
            outer,
            inner,
            place,
        }
    }

    /// The track with the blocker present (`Some(on)`) or not (`None`).
    fn track(&self, blocker: Option<bool>, notes: &[(f64, f64, u8)]) -> TrackDesc {
        let b: Vec<(NodeKey, bool)> = blocker.map(|on| (self.blocker, on)).into_iter().collect();
        let s = (self.synth, true);
        let with = |rest: &[(NodeKey, bool)]| [b.as_slice(), rest].concat();
        match self.place {
            Place::Track => midi_track(&with(&[s]), notes),
            Place::InstrumentRack => {
                let mut t = midi_track(&[(self.outer, true)], notes);
                t.chain_racks = vec![rack(self.outer, ChainRackKind::Instrument, &with(&[s]))];
                t
            }
            Place::MidiEffectRack => {
                let mut t = midi_track(&[(self.inner, true), s], notes);
                t.chain_racks = vec![rack(self.inner, ChainRackKind::MidiEffect, &b)];
                t
            }
            Place::Nested => {
                let mut t = midi_track(&[(self.outer, true)], notes);
                t.chain_racks = vec![
                    rack(
                        self.outer,
                        ChainRackKind::Instrument,
                        &[(self.inner, true), s],
                    ),
                    rack(self.inner, ChainRackKind::MidiEffect, &b),
                ];
                t
            }
        }
    }

    fn publish(&mut self, version: u64, t: TrackDesc) {
        self.p.handle.publish(graph(version, t)).unwrap();
    }
}

const PLACES: [Place; 4] = [
    Place::Track,
    Place::InstrumentRack,
    Place::MidiEffectRack,
    Place::Nested,
];

/// One note held over beats 0..4, a second one from beat 6.
const NOTES: [(f64, f64, u8); 2] = [(0.0, 4.0, 60), (6.0, 1.0, 64)];

#[test]
fn it_silences_the_instrument_wherever_it_sits() {
    for place in PLACES {
        let mut r = Rig::new(place);
        r.publish(1, r.track(None, &NOTES));
        r.p.handle.transport(TransportControl::Play).unwrap();
        assert!(sounds_after(&mut r.p, 0.5), "{place:?}: plays without it");

        let mut r = Rig::new(place);
        r.publish(1, r.track(Some(true), &NOTES));
        r.p.handle.transport(TransportControl::Play).unwrap();
        assert!(
            !sounds_after(&mut r.p, 0.5),
            "{place:?}: silent at beat 0.5"
        );
        assert!(
            !sounds_after(&mut r.p, 6.0),
            "{place:?}: silent at beat 6.5"
        );
    }
}

#[test]
fn notes_held_when_it_is_added_are_released_and_never_retriggered() {
    for place in PLACES {
        // Added mid-note, then removed mid-note: the held note ends and doesn't come back.
        let mut r = Rig::new(place);
        r.publish(1, r.track(None, &NOTES));
        r.p.handle.transport(TransportControl::Play).unwrap();
        assert!(sounds_after(&mut r.p, 1.0), "{place:?}");
        r.publish(2, r.track(Some(true), &NOTES));
        assert!(!sounds_after(&mut r.p, 0.5), "{place:?}: added: released");
        r.publish(3, r.track(None, &NOTES));
        assert!(
            !sounds_after(&mut r.p, 0.5),
            "{place:?}: removed: the held note isn't retriggered"
        );
        assert!(
            sounds_after(&mut r.p, 4.5),
            "{place:?}: the next note plays (beat 6)"
        );
    }
}

#[test]
fn notes_held_when_it_is_switched_on_are_released_and_never_retriggered() {
    for place in PLACES {
        // Switched on mid-note, then off mid-note.
        let mut r = Rig::new(place);
        r.publish(1, r.track(Some(false), &NOTES));
        r.p.handle.transport(TransportControl::Play).unwrap();
        assert!(sounds_after(&mut r.p, 1.0), "{place:?}: off = MIDI thru");
        r.publish(2, r.track(Some(true), &NOTES));
        assert!(
            !sounds_after(&mut r.p, 0.5),
            "{place:?}: switched on: released"
        );
        r.publish(3, r.track(Some(false), &NOTES));
        assert!(
            !sounds_after(&mut r.p, 0.5),
            "{place:?}: switched off: the held note isn't retriggered"
        );
        assert!(
            sounds_after(&mut r.p, 4.5),
            "{place:?}: the next note plays"
        );
    }
}

#[test]
fn notes_held_when_automation_switches_it_on_are_released() {
    for place in PLACES {
        let mut r = Rig::new(place);
        let mut t = r.track(Some(true), &NOTES);
        // Off until beat 1, on until beat 5, off again.
        t.automation = vec![AutomationDesc {
            target: AutomationTarget::TrackVolume { track: tid(2) },
            resolved: ResolvedTarget::NodeEnabled { node: r.blocker },
            points: vec![
                (0.0, 0.0, CurveShape::Step),
                (1.0, 1.0, CurveShape::Step),
                (5.0, 0.0, CurveShape::Step),
            ],
            mapping: ParamMapping {
                min: 0.0,
                max: 1.0,
                scale: ParamScale::Linear,
                steps: Some(2),
            },
        }];
        r.publish(1, t);
        r.p.handle.transport(TransportControl::Play).unwrap();
        assert!(sounds_after(&mut r.p, 0.5), "{place:?}: off before beat 1");
        assert!(
            !sounds_after(&mut r.p, 1.0),
            "{place:?}: on at beat 1: released"
        );
        assert!(
            !sounds_after(&mut r.p, 4.0),
            "{place:?}: nothing retriggered at beat 5"
        );
        assert!(
            sounds_after(&mut r.p, 1.0),
            "{place:?}: the next note plays (beat 6)"
        );
    }
}

#[test]
fn block_everything_also_drops_live_cc_and_pitch_bend() {
    for (block_everything, expected) in [(false, 2.0), (true, 0.0)] {
        let mut p = ether_core::create(config());
        let mut io = p.handle.take_recording_io().unwrap();
        let b = p
            .handle
            .add_node(device(BuiltinDeviceType::NoteBlocker, block_everything))
            .unwrap();
        let synth = p.handle.add_node(voices()).unwrap();
        let mut t = midi_track(&[(b, true), (synth, true)], &[]);
        t.monitor = true;
        p.handle.publish(graph(1, t)).unwrap();
        render(&mut p, 0.01);
        for data in [[0x90, 60, 100], [0xB0, 1, 64], [0xE0, 0, 80]] {
            io.midi_in
                .push(LiveMidi {
                    sample_time: BLOCK as u64,
                    data,
                    port: UNTAGGED_PORT,
                })
                .unwrap();
        }
        let (sounding, midi) = render(&mut p, 0.1);
        assert_eq!(sounding, 0.0, "block everything: {block_everything}");
        assert_eq!(midi, expected, "block everything: {block_everything}");
    }
}

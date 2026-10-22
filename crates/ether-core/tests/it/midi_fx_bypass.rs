//! A switched-off MIDI effect is MIDI thru, as if it were absent (OSS #191): on a track
//! chain, on an instrument rack chain (nested too), on a drum pad chain, at the end of a
//! MIDI effect rack chain, and for a whole switched-off MIDI effect rack. Toggling it while
//! notes are held (static switch or automation, the flip inside a block or on the sample a
//! note starts) leaves no stuck notes and drops no note-ons.

use crate::common;

use common::*;
use ether_core::graph::{
    AutomationDesc, ChainEntry, PadDesc, ParamMapping, RackDesc, RenderGraphDesc, ResolvedTarget,
    TrackDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, CurveShape, DrumPadId, RackChainId, TrackKind, Ulid,
};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, EngineParts, EventKind, Node, NodeKey, PrepareConfig, ProcessContext,
    ProcessStatus, TransportControl, create,
};

const BEAT: usize = 24_000; // 120 BPM @ 48 kHz
/// Divides no beat: every switch and note lands inside a block.
const ODD_BLOCK: usize = 377;

/// Instrument: outputs Σ key / 128 over its held keys (so a transposed note and a stuck
/// one both show).
struct KeyGate {
    held: [bool; 128],
}

impl KeyGate {
    fn level(&self) -> f32 {
        (0..128)
            .filter(|&k| self.held[k])
            .map(|k| k as f32 / 128.0)
            .sum()
    }
}

impl Node for KeyGate {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.held = [false; 128];
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut pos = 0;
        for e in ctx.events {
            let off = e.offset as usize;
            let v = self.level();
            for ch in audio.outputs.iter_mut() {
                ch[pos..off].fill(v);
            }
            pos = off;
            match e.kind {
                EventKind::NoteOn { key, .. } => self.held[key as usize] = true,
                EventKind::NoteOff { key, .. } => self.held[key as usize] = false,
                EventKind::AllNotesOff => self.held = [false; 128],
                _ => {}
            }
        }
        let v = self.level();
        for ch in audio.outputs.iter_mut() {
            ch[pos..].fill(v);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// MIDI effect (no audio): transposes notes by `semis`.
struct Transpose(i8);

impl Node for Transpose {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        _: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            let mut e = *e;
            match &mut e.kind {
                EventKind::NoteOn { key, .. } | EventKind::NoteOff { key, .. } => {
                    *key = (*key as i16 + self.0 as i16).clamp(0, 127) as u8;
                }
                EventKind::Param { .. } => continue,
                _ => {}
            }
            ctx.out_events.push(e);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 0)
    }
}

/// A rack node: audio through, MIDI thru (like `ether_devices::racks`).
struct RackNode;

impl Node for RackNode {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            if !matches!(e.kind, EventKind::Param { .. }) {
                ctx.out_events.push(*e);
            }
        }
        audio.pass_through();
        ProcessStatus::Continue
    }
}

/// Where the switched MIDI effect (`fx`, before the instrument) sits.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// Track chain: fx → instrument.
    Track,
    /// Instrument rack chain: fx → instrument.
    Rack,
    /// Instrument rack chain → nested instrument rack chain: fx → instrument.
    Nested,
    /// Drum pad (note 60) chain: fx → instrument.
    Pad,
    /// MIDI effect rack chain ending in fx, then the instrument on the track.
    MidiRack,
    /// Instrument rack chain: a MIDI effect rack (its chain: fx) → instrument; the switch
    /// is the MIDI effect rack's.
    MidiRackInRack,
}

const PLACES: [Place; 6] = [
    Place::Track,
    Place::Rack,
    Place::Nested,
    Place::Pad,
    Place::MidiRack,
    Place::MidiRackInRack,
];

fn entry(node: NodeKey, enabled: bool) -> ChainEntry {
    ChainEntry {
        node,
        enabled,
        sidechain: None,
    }
}

fn chain(id: u128, entries: Vec<ChainEntry>) -> RackChainDesc {
    RackChainDesc {
        id: RackChainId(Ulid(id)),
        chain: entries,
        volume: 1.0,
        pan: 0.0,
        mute: false,
        keys: (0, 127),
        velocities: (0, 127),
        select: (0, 127),
    }
}

fn rack(rack: NodeKey, kind: ChainRackKind, chains: Vec<RackChainDesc>) -> ChainRackDesc {
    ChainRackDesc {
        rack,
        kind,
        chains,
        selector: 0,
    }
}

/// The engine with a MIDI track playing `notes` through `place`, the switch of the switched
/// node (fx, or the MIDI effect rack holding it) static at `enabled`. Returns the parts, the
/// track and the switched node.
fn setup(
    place: Place,
    enabled: bool,
    notes: &[(f64, f64, u8)],
) -> (EngineParts, TrackDesc, NodeKey) {
    let mut p = create(config());
    let fx = p.handle.add_node(Box::new(Transpose(12))).unwrap();
    let inst = p
        .handle
        .add_node(Box::new(KeyGate { held: [false; 128] }))
        .unwrap();
    let mut t = track(tid(2), TrackKind::Midi, Some(tid(1)));
    t.clips = vec![midi_clip(cid(1), 0.0, 64.0, notes)];
    let mut switched = fx;
    match place {
        Place::Track => t.chain = vec![entry(fx, enabled), entry(inst, true)],
        Place::Rack => {
            let r = p.handle.add_node(Box::new(RackNode)).unwrap();
            t.chain = vec![entry(r, true)];
            t.chain_racks = vec![rack(
                r,
                ChainRackKind::Instrument,
                vec![chain(1, vec![entry(fx, enabled), entry(inst, true)])],
            )];
        }
        Place::Nested => {
            let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
            let inner = p.handle.add_node(Box::new(RackNode)).unwrap();
            t.chain = vec![entry(outer, true)];
            t.chain_racks = vec![
                rack(
                    outer,
                    ChainRackKind::Instrument,
                    vec![chain(1, vec![entry(inner, true)])],
                ),
                rack(
                    inner,
                    ChainRackKind::Instrument,
                    vec![chain(2, vec![entry(fx, enabled), entry(inst, true)])],
                ),
            ];
        }
        Place::Pad => {
            let drums = p.handle.add_node(Box::new(RackNode)).unwrap();
            t.chain = vec![entry(drums, true)];
            t.racks = vec![RackDesc {
                rack: drums,
                pads: vec![PadDesc {
                    pad: DrumPadId(Ulid(9)),
                    note: 60,
                    choke_group: None,
                    chain: vec![entry(fx, enabled), entry(inst, true)],
                    volume: 1.0,
                    pan: 0.0,
                    mute: false,
                }],
            }];
        }
        Place::MidiRack => {
            let r = p.handle.add_node(Box::new(RackNode)).unwrap();
            t.chain = vec![entry(r, true), entry(inst, true)];
            t.chain_racks = vec![rack(
                r,
                ChainRackKind::MidiEffect,
                vec![chain(1, vec![entry(fx, enabled)])],
            )];
        }
        Place::MidiRackInRack => {
            let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
            let midi = p.handle.add_node(Box::new(RackNode)).unwrap();
            t.chain = vec![entry(outer, true)];
            t.chain_racks = vec![
                rack(
                    outer,
                    ChainRackKind::Instrument,
                    vec![chain(1, vec![entry(midi, enabled), entry(inst, true)])],
                ),
                rack(
                    midi,
                    ChainRackKind::MidiEffect,
                    vec![chain(2, vec![entry(fx, true)])],
                ),
            ];
            switched = midi;
        }
    }
    (p, t, switched)
}

fn publish(p: &mut EngineParts, version: u64, t: &TrackDesc) {
    p.handle
        .publish(RenderGraphDesc {
            version,
            tracks: vec![master(), t.clone()],
            ..Default::default()
        })
        .unwrap();
}

/// The level of `key` held alone at the instrument.
fn held(key: u8) -> f32 {
    key as f32 / 128.0
}

fn assert_level(l: &[f32], beat: f64, want: f32, what: &str) {
    let got = l[(beat * BEAT as f64) as usize];
    assert!(
        (got - want).abs() < 1e-6,
        "{what} at beat {beat}: {got}, want {want}"
    );
}

#[test]
fn a_switched_off_midi_effect_passes_notes_untouched_everywhere() {
    for place in PLACES {
        for (enabled, key) in [(true, 72), (false, 60)] {
            let (mut p, t, _) = setup(place, enabled, &[(0.0, 0.5, 60)]);
            publish(&mut p, 1, &t);
            p.handle.transport(TransportControl::Play).unwrap();
            let (l, r) = render(&mut p.engine, BEAT, BLOCK);
            let what = format!("{place:?}, fx on: {enabled}");
            assert_level(&l, 0.25, held(key), &what);
            assert_level(&l, 0.75, 0.0, &what);
            assert_eq!(l, r, "{what}");
        }
    }
}

/// On from beat 0, then off at 1, on at 3, off at 6, on at 10.
fn toggles(node: NodeKey) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved: ResolvedTarget::NodeEnabled { node },
        points: vec![
            (0.0, 1.0, CurveShape::Step),
            (1.0, 0.0, CurveShape::Step),
            (3.0, 1.0, CurveShape::Step),
            (6.0, 0.0, CurveShape::Step),
            (10.0, 1.0, CurveShape::Step),
        ],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: Some(2),
        },
    }
}

#[test]
fn automating_the_switch_while_notes_are_held_drops_and_strands_nothing() {
    // All on key 60 (the drum pad's note): transposed = 72, thru = 60.
    let notes = [
        (0.0, 2.0, 60),  // held across the switch going off at 1
        (2.5, 1.0, 60),  // played while off, held across the switch coming on at 3
        (4.0, 0.5, 60),  // on
        (6.0, 0.5, 60),  // starts on the sample the switch goes off
        (7.0, 1.0, 60),  // off
        (10.0, 0.5, 60), // starts on the sample the switch comes on
    ];
    for place in PLACES {
        for block in [BLOCK, ODD_BLOCK] {
            let (mut p, mut t, switched) = setup(place, true, &notes);
            t.automation = vec![toggles(switched)];
            publish(&mut p, 1, &t);
            p.handle.transport(TransportControl::Play).unwrap();
            let (l, _) = render(&mut p.engine, 11 * BEAT, block);
            let what = format!("{place:?}, block {block}");
            assert_level(&l, 0.5, held(72), &what);
            // The switch released the note the effect started; its note-off comes thru.
            assert_level(&l, 1.5, 0.0, &what);
            assert_level(&l, 2.25, 0.0, &what);
            assert_level(&l, 2.75, held(60), &what);
            // Back on: the note that went thru is released.
            assert_level(&l, 3.25, 0.0, &what);
            assert_level(&l, 3.75, 0.0, &what);
            assert_level(&l, 4.25, held(72), &what);
            assert_level(&l, 4.75, 0.0, &what);
            // Notes starting on the switch's sample follow the new state, and sound.
            assert_level(&l, 6.25, held(60), &what);
            assert_level(&l, 6.75, 0.0, &what);
            assert_level(&l, 7.5, held(60), &what);
            assert_level(&l, 8.5, 0.0, &what);
            assert_level(&l, 10.25, held(72), &what);
            assert_level(&l, 10.75, 0.0, &what);
        }
    }
}

#[test]
fn switching_statically_while_a_note_is_held_strands_nothing() {
    for place in PLACES {
        // Off → on, then on → off, each while a note is held.
        for start in [false, true] {
            let (mut p, mut t, switched) = setup(place, start, &[(0.0, 1.0, 60), (2.0, 1.0, 60)]);
            publish(&mut p, 1, &t);
            p.handle.transport(TransportControl::Play).unwrap();
            let what = format!("{place:?}, from on: {start}");
            let (l, _) = render(&mut p.engine, BEAT / 2, ODD_BLOCK);
            assert_level(&l, 0.25, held(if start { 72 } else { 60 }), &what);
            let set = |c: &mut Vec<ChainEntry>| {
                for e in c.iter_mut().filter(|e| e.node == switched) {
                    e.enabled = !start;
                }
            };
            set(&mut t.chain);
            for r in &mut t.chain_racks {
                for c in &mut r.chains {
                    set(&mut c.chain);
                }
            }
            for r in &mut t.racks {
                for pad in &mut r.pads {
                    set(&mut pad.chain);
                }
            }
            publish(&mut p, 2, &t);
            let (l, _) = render(&mut p.engine, 3 * BEAT, ODD_BLOCK);
            // (From beat 0.5.) The held note is released, nothing sticks past its note-off,
            // and the next note plays in the new state.
            assert_level(&l, 0.25, 0.0, &what);
            assert_level(&l, 1.0, 0.0, &what);
            assert_level(&l, 1.75, held(if start { 60 } else { 72 }), &what);
            assert_level(&l, 2.75, 0.0, &what);
        }
    }
}

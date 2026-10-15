//! Stuck notes (audit #558–#560), rendered through the engine: a device switched on or off
//! (static switch or On/Off automation; on a track chain, a rack chain or a drum pad), a
//! track that stops monitoring live MIDI, and a drum pad whose note changes while it
//! sounds all release the notes they would otherwise strand.

mod common;

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
use ether_core::recording::{LiveMidi, UNTAGGED_PORT};
use ether_core::{
    AudioBuffers, EventKind, Node, NodeKey, PrepareConfig, ProcessContext, ProcessEvent,
    ProcessStatus, TransportControl, create,
};

const BEAT: usize = 24_000; // 120 BPM @ 48 kHz

/// Instrument: outputs 1.0 while any of its voices sounds. A voice ends with its note-off or
/// choke (by note id) or `AllNotesOff`, like a real poly synth with no release time.
struct Voices {
    ids: Vec<u32>,
}

fn voices() -> Box<Voices> {
    Box::new(Voices {
        ids: Vec::with_capacity(64),
    })
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
        let mut pos = 0;
        for e in ctx.events {
            let off = e.offset as usize;
            let v = if self.ids.is_empty() { 0.0 } else { 1.0 };
            for ch in audio.outputs.iter_mut() {
                ch[pos..off].fill(v);
            }
            pos = off;
            match e.kind {
                EventKind::NoteOn { note_id, .. } if self.ids.len() < self.ids.capacity() => {
                    self.ids.push(note_id)
                }
                EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                    self.ids.retain(|&id| id != note_id)
                }
                EventKind::AllNotesOff => self.ids.clear(),
                _ => {}
            }
        }
        let v = if self.ids.is_empty() { 0.0 } else { 1.0 };
        for ch in audio.outputs.iter_mut() {
            ch[pos..].fill(v);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// MIDI effect generating its own notes (like a chord or arpeggiator): every note comes out
/// an octave up under a new note id; `AllNotesOff` is forwarded.
struct Octaver;

impl Node for Octaver {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        _: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            let kind = match e.kind {
                EventKind::NoteOn {
                    note_id,
                    channel,
                    key,
                    velocity,
                } => EventKind::NoteOn {
                    note_id: note_id ^ 0x4000_0000,
                    channel,
                    key: key + 12,
                    velocity,
                },
                EventKind::NoteOff {
                    note_id,
                    channel,
                    key,
                    velocity,
                } => EventKind::NoteOff {
                    note_id: note_id ^ 0x4000_0000,
                    channel,
                    key: key + 12,
                    velocity,
                },
                EventKind::Param { .. } => continue,
                k => k,
            };
            ctx.out_events.push(ProcessEvent {
                offset: e.offset,
                kind,
            });
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

fn midi_track(chain: &[(NodeKey, bool)], notes: &[(f64, f64, u8)]) -> TrackDesc {
    let mut t = track(tid(2), TrackKind::Midi, Some(tid(1)));
    t.chain = entries(chain);
    t.clips = vec![midi_clip(cid(1), 0.0, 16.0, notes)];
    t
}

fn instrument_rack(rack: NodeKey, chain: &[(NodeKey, bool)]) -> ChainRackDesc {
    ChainRackDesc {
        rack,
        kind: ChainRackKind::Instrument,
        chains: vec![RackChainDesc {
            id: RackChainId(Ulid(1)),
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

fn pad(note: u8, chain: &[(NodeKey, bool)]) -> PadDesc {
    PadDesc {
        pad: DrumPadId(Ulid(1)),
        note,
        choke_group: None,
        chain: entries(chain),
        volume: 1.0,
        pan: 0.0,
        mute: false,
    }
}

fn desc(version: u64, t: TrackDesc) -> RenderGraphDesc {
    RenderGraphDesc {
        version,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

/// An On/Off lane: `node` is off from `off` on.
fn off_from(node: NodeKey, off: f64) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved: ResolvedTarget::NodeEnabled { node },
        points: vec![(0.0, 1.0, CurveShape::Step), (off, 0.0, CurveShape::Step)],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: Some(2),
        },
    }
}

/// Renders `beats` and returns whether the mix still sounds at the end.
fn sounds_after(p: &mut ether_core::EngineParts, beats: f64) -> bool {
    let (l, _) = render(&mut p.engine, (beats * BEAT as f64) as usize, BLOCK);
    *l.last().unwrap() > 0.5
}

#[test]
fn an_instrument_switched_off_mid_note_releases_it_when_back_on() {
    // #558 (a): synth off during the note, back on after it ended.
    let mut p = create(config());
    let synth = p.handle.add_node(voices()).unwrap();
    let notes = [(0.0, 2.0, 60)];
    p.handle
        .publish(desc(1, midi_track(&[(synth, true)], &notes)))
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    assert!(sounds_after(&mut p, 0.5));
    p.handle
        .publish(desc(2, midi_track(&[(synth, false)], &notes)))
        .unwrap();
    assert!(!sounds_after(&mut p, 2.5), "off: silent");
    p.handle
        .publish(desc(3, midi_track(&[(synth, true)], &notes)))
        .unwrap();
    assert!(
        !sounds_after(&mut p, 0.5),
        "back on: the note that ended while off is released"
    );
}

#[test]
fn a_midi_effect_switched_off_by_automation_releases_its_notes() {
    // #558 (b): Octaver → synth, its On/Off lane drops to 0 at beat 1 mid-note.
    let mut p = create(config());
    let fx = p.handle.add_node(Box::new(Octaver)).unwrap();
    let synth = p.handle.add_node(voices()).unwrap();
    let mut t = midi_track(&[(fx, true), (synth, true)], &[(0.0, 4.0, 60)]);
    t.automation = vec![off_from(fx, 1.0)];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    assert!(sounds_after(&mut p, 0.5));
    // The generated note is released at the switch; nothing is left after the note's end.
    assert!(
        !sounds_after(&mut p, 1.0),
        "switched off: generated notes released"
    );
    assert!(!sounds_after(&mut p, 3.5), "after the note: silent");
}

#[test]
fn a_midi_effect_switched_off_before_a_rack_releases_notes_inside_it() {
    // #558 (c): Octaver → instrument rack [synth] / drum rack [pad on 72: synth].
    for drums in [false, true] {
        let mut p = create(config());
        let fx = p.handle.add_node(Box::new(Octaver)).unwrap();
        let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
        let synth = p.handle.add_node(voices()).unwrap();
        let t = |fx_on: bool| {
            let mut t = midi_track(&[(fx, fx_on), (rack, true)], &[(0.0, 4.0, 60)]);
            if drums {
                t.racks = vec![RackDesc {
                    rack,
                    pads: vec![pad(72, &[(synth, true)])],
                }];
            } else {
                t.chain_racks = vec![instrument_rack(rack, &[(synth, true)])];
            }
            t
        };
        p.handle.publish(desc(1, t(true))).unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        assert!(sounds_after(&mut p, 1.0), "drums: {drums}");
        p.handle.publish(desc(2, t(false))).unwrap();
        assert!(
            !sounds_after(&mut p, 4.0),
            "drums: {drums}: the rack's synth released"
        );
    }
}

#[test]
fn devices_switched_inside_rack_chains_and_pads_release_their_notes() {
    // #558: Octaver → synth inside an instrument rack chain (static switch) and on a drum pad
    // (On/Off automation).
    for drums in [false, true] {
        let mut p = create(config());
        let fx = p.handle.add_node(Box::new(Octaver)).unwrap();
        let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
        let synth = p.handle.add_node(voices()).unwrap();
        let t = |fx_on: bool| {
            let mut t = midi_track(&[(rack, true)], &[(0.0, 4.0, 60)]);
            let inner = [(fx, fx_on), (synth, true)];
            if drums {
                t.racks = vec![RackDesc {
                    rack,
                    pads: vec![pad(60, &inner)],
                }];
                t.automation = vec![off_from(fx, 1.0)];
            } else {
                t.chain_racks = vec![instrument_rack(rack, &inner)];
            }
            t
        };
        p.handle.publish(desc(1, t(true))).unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        assert!(sounds_after(&mut p, 0.5), "drums: {drums}");
        if !drums {
            p.handle.publish(desc(2, t(false))).unwrap();
        }
        assert!(
            !sounds_after(&mut p, 4.0),
            "drums: {drums}: the generated note released"
        );
    }
}

#[test]
fn a_track_that_stops_monitoring_releases_its_live_notes() {
    // #559: a held live note, then the track is disarmed before the key is released.
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let synth = p.handle.add_node(voices()).unwrap();
    let t = |monitor: bool| {
        let mut t = midi_track(&[(synth, true)], &[]);
        t.clips.clear();
        t.monitor = monitor;
        t
    };
    p.handle.publish(desc(1, t(true))).unwrap();
    render(&mut p.engine, BLOCK, BLOCK);
    for data in [[0x90, 60, 100], [0xB0, 64, 127]] {
        io.midi_in
            .push(LiveMidi {
                sample_time: BLOCK as u64,
                data,
                port: UNTAGGED_PORT,
            })
            .unwrap();
    }
    assert!(sounds_after(&mut p, 0.1), "the live note plays");
    p.handle.publish(desc(2, t(false))).unwrap();
    assert!(!sounds_after(&mut p, 0.1), "no longer monitored: released");
}

#[test]
fn stop_while_stopped_releases_every_note() {
    // #559: Stop is a panic even while stopped (a live note held by an instrument).
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let synth = p.handle.add_node(voices()).unwrap();
    let mut t = midi_track(&[(synth, true)], &[]);
    t.monitor = true;
    p.handle.publish(desc(1, t)).unwrap();
    render(&mut p.engine, BLOCK, BLOCK);
    io.midi_in
        .push(LiveMidi {
            sample_time: BLOCK as u64,
            data: [0x90, 60, 100],
            port: UNTAGGED_PORT,
        })
        .unwrap();
    assert!(sounds_after(&mut p, 0.1));
    p.handle.transport(TransportControl::Stop).unwrap();
    assert!(!sounds_after(&mut p, 0.1));
}

#[test]
fn a_note_off_reaches_the_pad_its_note_on_went_to() {
    // #560: a pad's note changes from 36 to 37 while its note 36 sounds.
    let mut p = create(config());
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let synth = p.handle.add_node(voices()).unwrap();
    let t = |note: u8| {
        let mut t = midi_track(&[(rack, true)], &[(0.0, 2.0, 36)]);
        t.racks = vec![RackDesc {
            rack,
            pads: vec![pad(note, &[(synth, true)])],
        }];
        t
    };
    p.handle.publish(desc(1, t(36))).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    assert!(sounds_after(&mut p, 1.0));
    p.handle.publish(desc(2, t(37))).unwrap();
    assert!(!sounds_after(&mut p, 1.5), "the note-off found the pad");
}

//! Engine-side rack chains (`racks-modulation`): key/velocity/selector routing, chain mix,
//! PDC inside a rack, audio effect racks (parallel chains of the input), MIDI effect racks
//! (merged chain output replaces the rack's MIDI-thru), state across snapshot swaps, racks
//! on drum pad chains.

mod common;

use common::*;
use ether_core::graph::{ChainEntry, PadDesc, RackDesc, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{DrumPadId, ParamId, RackChainId, TrackKind, Ulid};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, EventKind, Node, NodeKey, ParamChange, ParamTarget, PrepareConfig,
    ProcessContext, ProcessStatus, TransportControl, create,
};

const BEAT: usize = 24_000; // 120 BPM @ 48 kHz

/// Instrument: outputs `level` while any note sounds (note-offs release).
struct Gate {
    level: f32,
    notes: usize,
}

impl Node for Gate {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.notes = 0;
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut pos = 0;
        for e in ctx.events {
            let off = e.offset as usize;
            let v = if self.notes > 0 { self.level } else { 0.0 };
            for ch in audio.outputs.iter_mut() {
                ch[pos..off].fill(v);
            }
            pos = off;
            match e.kind {
                EventKind::NoteOn { .. } => self.notes += 1,
                EventKind::NoteOff { .. } => self.notes = self.notes.saturating_sub(1),
                EventKind::AllNotesOff => self.notes = 0,
                _ => {}
            }
        }
        let v = if self.notes > 0 { self.level } else { 0.0 };
        for ch in audio.outputs.iter_mut() {
            ch[pos..].fill(v);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// The rack node: audio through, MIDI-thru (like `ether_devices::racks`).
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

/// MIDI effect: transposes notes by `semis`.
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

/// Multiplies by `gain`.
struct Gain(f32);

impl Node for Gain {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for ch in 0..audio.outputs.len() {
            for i in 0..audio.outputs[ch].len() {
                audio.outputs[ch][i] = audio.inputs[ch][i] * self.0;
            }
        }
        ProcessStatus::Continue
    }
}

fn entry(node: NodeKey) -> ChainEntry {
    ChainEntry {
        node,
        enabled: true,
        sidechain: None,
    }
}

fn chain(id: u128, nodes: &[NodeKey]) -> RackChainDesc {
    RackChainDesc {
        id: RackChainId(Ulid(id)),
        chain: nodes.iter().map(|&n| entry(n)).collect(),
        volume: 1.0,
        pan: 0.0,
        mute: false,
        keys: (0, 127),
        velocities: (0, 127),
        select: (0, 127),
    }
}

fn desc(version: u64, t: TrackDesc) -> RenderGraphDesc {
    RenderGraphDesc {
        version,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

fn midi_track(chain_nodes: &[NodeKey], notes: &[(f64, f64, u8)]) -> TrackDesc {
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), chain_nodes);
    t.clips = vec![midi_clip(cid(1), 0.0, 16.0, notes)];
    t
}

#[test]
fn instrument_rack_routes_by_key_zone_and_mixes_chains() {
    let mut p = create(config());
    let low = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.5,
            notes: 0,
        }))
        .unwrap();
    let high = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.25,
            notes: 0,
        }))
        .unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = midi_track(&[rack], &[(0.0, 0.5, 40), (1.0, 0.5, 80)]);
    let mut a = chain(1, &[low]);
    a.keys = (0, 59);
    let mut b = chain(2, &[high]);
    b.keys = (60, 127);
    b.volume = 0.5;
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::Instrument,
        chains: vec![a, b],
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, 2 * BEAT, BLOCK);
    // Key 40 → low chain only; key 80 → high chain at half volume.
    assert!((l[BEAT / 4] - 0.5).abs() < 1e-6, "{}", l[BEAT / 4]);
    assert!(
        (l[BEAT + BEAT / 4] - 0.125).abs() < 1e-6,
        "{}",
        l[BEAT + BEAT / 4]
    );
    assert_eq!(l[3 * BEAT / 4], 0.0);
    assert_eq!(l, r);
}

#[test]
fn selector_gates_chains_with_a_ramp() {
    let mut p = create(config());
    let a_node = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.5,
            notes: 0,
        }))
        .unwrap();
    let b_node = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.25,
            notes: 0,
        }))
        .unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = midi_track(&[rack], &[(0.0, 8.0, 60)]);
    let mut a = chain(1, &[a_node]);
    a.select = (0, 63);
    let mut b = chain(2, &[b_node]);
    b.select = (64, 127);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::Instrument,
        chains: vec![a, b],
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, BEAT / 2, BLOCK);
    assert!((l[BEAT / 4] - 0.5).abs() < 1e-6);
    // Move the selector: A fades out, B fades in (both got the note: note-ons only filter
    // at their start, so B was silent while unselected).
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rack,
                param: ParamId(8),
            },
            value: 100.0,
        })
        .unwrap();
    let (l, _) = render(&mut p.engine, BEAT / 2, BLOCK);
    assert!(l[0] > 0.45, "ramp starts at A: {}", l[0]);
    // After the 20 ms ramp: only B (which never got the note-on) → silence.
    assert!(l[BEAT / 4].abs() < 1e-6, "{}", l[BEAT / 4]);
}

#[test]
fn audio_effect_rack_runs_parallel_chains_of_its_input_aligned() {
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Impulse)).unwrap();
    let delay = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let gain = p.handle.add_node(Box::new(Gain(0.5))).unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, rack]);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::AudioEffect,
        // A delayed chain and a gain chain: aligned to the longest (100 samples).
        chains: vec![chain(1, &[delay]), chain(2, &[gain])],
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    assert!((l[100] - 1.5).abs() < 1e-6, "{}", l[100]);
    let others: f32 = l
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 100)
        .map(|(_, v)| v.abs())
        .sum();
    assert!(others < 1e-6);
    // The rack entry reports the chain latency (master output compensated).
    assert_eq!(p.handle.latency(), 100);
}

/// Renders a DC 0.5 source through an audio effect rack with `chains`.
fn audio_rack_level(chains: Vec<RackChainDesc>) -> f32 {
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, rack]);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::AudioEffect,
        chains,
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    let (l, r) = render(&mut p.engine, 1024, BLOCK);
    assert_eq!(l[600], r[600]);
    l[600]
}

#[test]
fn an_empty_audio_effect_rack_is_silent() {
    // No chain for the sound to go to: silence.
    assert_eq!(audio_rack_level(vec![]), 0.0);
    // A chain without devices passes the input through to the mix.
    assert!((audio_rack_level(vec![chain(1, &[])]) - 0.5).abs() < 1e-6);
    // Every chain muted: silence.
    let mut muted = chain(1, &[]);
    muted.mute = true;
    assert_eq!(audio_rack_level(vec![muted]), 0.0);
}

/// Note-on keys reaching the device after a MIDI effect rack with `chains` (one note, 60).
fn midi_rack_keys(chains: Vec<RackChainDesc>) -> Vec<u8> {
    let mut p = create(config());
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = midi_track(&[rack, rec], &[(0.0, 0.5, 60)]);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::MidiEffect,
        chains,
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, BEAT, BLOCK);
    let mut keys: Vec<u8> = events(&drain(&mut rx))
        .into_iter()
        .filter_map(|(_, k)| match k {
            EventKind::NoteOn { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    keys.sort_unstable();
    keys
}

#[test]
fn an_empty_midi_effect_rack_lets_no_notes_through() {
    assert_eq!(midi_rack_keys(vec![]), Vec::<u8>::new());
    // A chain without devices passes the notes in its zones.
    assert_eq!(midi_rack_keys(vec![chain(1, &[])]), vec![60]);
    let mut out_of_zone = chain(1, &[]);
    out_of_zone.keys = (0, 59);
    assert_eq!(midi_rack_keys(vec![out_of_zone]), Vec::<u8>::new());
}

#[test]
fn midi_effect_rack_merges_its_chains_output() {
    let mut p = create(config());
    let up = p.handle.add_node(Box::new(Transpose(12))).unwrap();
    let down = p.handle.add_node(Box::new(Transpose(-12))).unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = midi_track(&[rack, rec], &[(0.0, 0.5, 60)]);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::MidiEffect,
        chains: vec![chain(1, &[up]), chain(2, &[down])],
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, BEAT, BLOCK);
    let mut keys: Vec<u8> = events(&drain(&mut rx))
        .into_iter()
        .filter_map(|(_, k)| match k {
            EventKind::NoteOn { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    keys.sort_unstable();
    // The original 60 is replaced by the chains' output.
    assert_eq!(keys, vec![48, 72]);
}

#[test]
fn chain_gain_ramps_carry_over_snapshot_swaps() {
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Dc(1.0))).unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let build = |volume: f32| {
        let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, rack]);
        let mut c = chain(1, &[]);
        c.volume = volume;
        t.chain_racks = vec![ChainRackDesc {
            rack,
            kind: ChainRackKind::AudioEffect,
            chains: vec![c],
            selector: 0,
        }];
        t
    };
    p.handle.publish(desc(1, build(1.0))).unwrap();
    render(&mut p.engine, 1024, BLOCK);
    p.handle.publish(desc(2, build(0.0))).unwrap();
    let (l, _) = render(&mut p.engine, 2048, BLOCK);
    // No jump: the gain ramps down from 1 over 20 ms.
    assert!(l[0] > 0.9, "{}", l[0]);
    assert!(l[2000].abs() < 1e-6, "{}", l[2000]);
}

// ─── Nested racks ──────────────────────────────────────────────────────────────────────

#[test]
fn nested_instrument_rack_routes_notes_through_both_zone_levels() {
    let mut p = create(config());
    let low = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.5,
            notes: 0,
        }))
        .unwrap();
    let high = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.25,
            notes: 0,
        }))
        .unwrap();
    let inner = p.handle.add_node(Box::new(RackNode)).unwrap();
    let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = midi_track(&[outer], &[(0.0, 0.5, 40), (1.0, 0.5, 80), (2.0, 0.5, 100)]);
    // Outer: one chain for keys 0..=90 holding the inner rack.
    let mut o = chain(1, &[inner]);
    o.keys = (0, 90);
    // Inner: low and high halves.
    let mut a = chain(2, &[low]);
    a.keys = (0, 59);
    let mut b = chain(3, &[high]);
    b.keys = (60, 127);
    t.chain_racks = vec![
        ChainRackDesc {
            rack: outer,
            kind: ChainRackKind::Instrument,
            chains: vec![o],
            selector: 0,
        },
        ChainRackDesc {
            rack: inner,
            kind: ChainRackKind::Instrument,
            chains: vec![a, b],
            selector: 0,
        },
    ];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, 3 * BEAT, BLOCK);
    assert!((l[BEAT / 4] - 0.5).abs() < 1e-6, "{}", l[BEAT / 4]);
    assert!(
        (l[BEAT + BEAT / 4] - 0.25).abs() < 1e-6,
        "{}",
        l[BEAT + BEAT / 4]
    );
    // Key 100 is outside the outer chain's zone: nothing reaches the inner rack.
    assert_eq!(l[2 * BEAT + BEAT / 4], 0.0);
    assert_eq!(l, r);
}

#[test]
fn nested_audio_effect_rack_mixes_and_reports_its_latency() {
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Impulse)).unwrap();
    let delay = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let gain = p.handle.add_node(Box::new(Gain(0.5))).unwrap();
    let dry = p.handle.add_node(Box::new(Gain(1.0))).unwrap();
    let inner = p.handle.add_node(Box::new(RackNode)).unwrap();
    let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, outer]);
    t.chain_racks = vec![
        ChainRackDesc {
            rack: outer,
            kind: ChainRackKind::AudioEffect,
            // A chain holding the nested rack (100 samples late inside) and a dry chain,
            // aligned to it.
            chains: vec![chain(1, &[inner]), chain(2, &[dry])],
            selector: 0,
        },
        ChainRackDesc {
            rack: inner,
            kind: ChainRackKind::AudioEffect,
            chains: vec![chain(3, &[delay]), chain(4, &[gain])],
            selector: 0,
        },
    ];
    p.handle.publish(desc(1, t)).unwrap();
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    // Inner: 1 (delayed) + 0.5 (aligned) = 1.5; the outer dry chain, aligned: + 1.
    assert!((l[100] - 2.5).abs() < 1e-6, "{}", l[100]);
    let others: f32 = l
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 100)
        .map(|(_, v)| v.abs())
        .sum();
    assert!(others < 1e-6, "{others}");
    assert_eq!(p.handle.latency(), 100);
}

#[test]
fn nested_rack_chain_mute_silences_only_that_chain() {
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Dc(1.0))).unwrap();
    let half = p.handle.add_node(Box::new(Gain(0.5))).unwrap();
    let quarter = p.handle.add_node(Box::new(Gain(0.25))).unwrap();
    let inner = p.handle.add_node(Box::new(RackNode)).unwrap();
    let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, outer]);
    // Solo arrives resolved as `mute` (controller): the inner rack's second chain is muted
    // because the first is soloed.
    let mut muted = chain(4, &[quarter]);
    muted.mute = true;
    t.chain_racks = vec![
        ChainRackDesc {
            rack: outer,
            kind: ChainRackKind::AudioEffect,
            chains: vec![chain(1, &[inner])],
            selector: 0,
        },
        ChainRackDesc {
            rack: inner,
            kind: ChainRackKind::AudioEffect,
            chains: vec![chain(3, &[half]), muted],
            selector: 0,
        },
    ];
    p.handle.publish(desc(1, t)).unwrap();
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    assert!((l[600] - 0.5).abs() < 1e-6, "{}", l[600]);
}

#[test]
fn nested_midi_effect_rack_merges_into_the_outer_chain() {
    let mut p = create(config());
    let up = p.handle.add_node(Box::new(Transpose(12))).unwrap();
    let down = p.handle.add_node(Box::new(Transpose(-12))).unwrap();
    let fifth = p.handle.add_node(Box::new(Transpose(7))).unwrap();
    let inner = p.handle.add_node(Box::new(RackNode)).unwrap();
    let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = midi_track(&[outer, rec], &[(0.0, 0.5, 60)]);
    t.chain_racks = vec![
        ChainRackDesc {
            rack: outer,
            kind: ChainRackKind::MidiEffect,
            // The nested rack (an octave up and down), then a fifth up.
            chains: vec![chain(1, &[inner, fifth])],
            selector: 0,
        },
        ChainRackDesc {
            rack: inner,
            kind: ChainRackKind::MidiEffect,
            chains: vec![chain(2, &[up]), chain(3, &[down])],
            selector: 0,
        },
    ];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, BEAT, BLOCK);
    let mut keys: Vec<u8> = events(&drain(&mut rx))
        .into_iter()
        .filter_map(|(_, k)| match k {
            EventKind::NoteOn { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, vec![55, 79]);
}

#[test]
fn device_switch_lanes_reach_nested_rack_chains() {
    use ether_core::graph::{AutomationDesc, ParamMapping, ResolvedTarget};
    use ether_core::protocol::devices::ParamScale;
    use ether_core::protocol::model::{AutomationTarget, CurveShape};
    // Dc(1) → outer rack [chain: inner rack [chain: Gain(0.5)]].
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Dc(1.0))).unwrap();
    let half = p.handle.add_node(Box::new(Gain(0.5))).unwrap();
    let inner = p.handle.add_node(Box::new(RackNode)).unwrap();
    let outer = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, outer]);
    t.chain_racks = vec![
        ChainRackDesc {
            rack: outer,
            kind: ChainRackKind::AudioEffect,
            chains: vec![chain(1, &[inner])],
            selector: 0,
        },
        ChainRackDesc {
            rack: inner,
            kind: ChainRackKind::AudioEffect,
            chains: vec![chain(3, &[half])],
            selector: 0,
        },
    ];
    let switch = |node: NodeKey, off: (f64, f64)| AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved: ResolvedTarget::NodeEnabled { node },
        points: vec![
            (0.0, 1.0, CurveShape::Step),
            (off.0, 0.0, CurveShape::Step),
            (off.1, 1.0, CurveShape::Step),
        ],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: Some(2),
        },
    };
    // The Gain inside the nested chain is off during beat 1, the nested rack during beat 3.
    t.automation = vec![switch(half, (1.0, 2.0)), switch(inner, (3.0, 4.0))];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 5 * BEAT, BLOCK);
    let at = |beat: f64| l[(beat * BEAT as f64) as usize];
    assert!((at(0.5) - 0.5).abs() < 1e-6, "on: {}", at(0.5));
    assert!((at(1.5) - 1.0).abs() < 1e-6, "nested Gain off: {}", at(1.5));
    assert!((at(2.5) - 0.5).abs() < 1e-6, "on again: {}", at(2.5));
    // The nested rack off: its chains don't run, its node passes the input through.
    assert!((at(3.5) - 1.0).abs() < 1e-6, "nested rack off: {}", at(3.5));
    assert!((at(4.5) - 0.5).abs() < 1e-6, "on again: {}", at(4.5));
}

#[test]
fn racks_on_a_drum_pad_run_their_chains() {
    let mut p = create(config());
    // Pad chain: MIDI effect rack (+20 semitones) → instrument rack (a chain only for keys
    // 70..) → audio effect rack (a 100-sample delayed chain and a half-gain chain).
    let up = p.handle.add_node(Box::new(Transpose(20))).unwrap();
    let midi_rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let gate = p
        .handle
        .add_node(Box::new(Gate {
            level: 0.5,
            notes: 0,
        }))
        .unwrap();
    let inst_rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let delay = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let gain = p.handle.add_node(Box::new(Gain(0.5))).unwrap();
    let fx_rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let drums = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = midi_track(&[drums], &[(0.0, 0.5, 36)]);
    t.racks = vec![RackDesc {
        rack: drums,
        pads: vec![PadDesc {
            pad: DrumPadId(Ulid(9)),
            note: 36,
            choke_group: None,
            chain: vec![entry(midi_rack), entry(inst_rack), entry(fx_rack)],
            volume: 1.0,
            pan: 0.0,
            mute: false,
        }],
    }];
    // The pad plays its chain at key 60: only the transposed note (80) opens the gate.
    let mut high = chain(2, &[gate]);
    high.keys = (70, 127);
    t.chain_racks = vec![
        ChainRackDesc {
            rack: midi_rack,
            kind: ChainRackKind::MidiEffect,
            chains: vec![chain(1, &[up])],
            selector: 0,
        },
        ChainRackDesc {
            rack: inst_rack,
            kind: ChainRackKind::Instrument,
            chains: vec![high],
            selector: 0,
        },
        ChainRackDesc {
            rack: fx_rack,
            kind: ChainRackKind::AudioEffect,
            chains: vec![chain(3, &[delay]), chain(4, &[gain])],
            selector: 0,
        },
    ];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, BEAT, BLOCK);
    // 0.5 delayed + 0.25 aligned.
    assert!((l[BEAT / 4] - 0.75).abs() < 1e-6, "{}", l[BEAT / 4]);
    assert!((r[BEAT / 4] - 0.75).abs() < 1e-6, "{}", r[BEAT / 4]);
    // The pad's rack chains count in the drum rack entry's latency (PDC).
    assert_eq!(p.handle.latency(), 100);
}

#[test]
fn a_drum_rack_on_an_instrument_rack_chain_plays_its_pads() {
    let mut p = create(config());
    let gate = |p: &mut ether_core::EngineParts, level: f32| {
        p.handle
            .add_node(Box::new(Gate { level, notes: 0 }))
            .unwrap()
    };
    // Instrument rack: chain 1 = drum rack → half gain; chain 2 = a gate (any note).
    let kick = gate(&mut p, 1.0);
    let kick_delay = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let snare = gate(&mut p, 0.125);
    let muted = gate(&mut p, 1.0);
    let drums = p.handle.add_node(Box::new(RackNode)).unwrap();
    let half = p.handle.add_node(Box::new(Gain(0.5))).unwrap();
    let pad_gate = gate(&mut p, 0.25);
    let inst = p.handle.add_node(Box::new(RackNode)).unwrap();
    // Kick 0..0.5 beats; snare from 0.25 (same choke group: it chokes the kick); the muted
    // pad's note sounds nowhere.
    let mut t = midi_track(&[inst], &[(0.0, 0.5, 36), (0.25, 0.25, 38), (0.0, 0.5, 40)]);
    let pad = |id: u128, note: u8, chain: Vec<ChainEntry>, mute: bool| PadDesc {
        pad: DrumPadId(Ulid(id)),
        note,
        choke_group: (note != 40).then_some(1),
        chain,
        volume: 1.0,
        pan: 0.0,
        mute,
    };
    t.racks = vec![RackDesc {
        rack: drums,
        pads: vec![
            pad(11, 36, vec![entry(kick), entry(kick_delay)], false),
            pad(12, 38, vec![entry(snare)], false),
            pad(13, 40, vec![entry(muted)], true),
        ],
    }];
    t.chain_racks = vec![ChainRackDesc {
        rack: inst,
        kind: ChainRackKind::Instrument,
        chains: vec![chain(1, &[drums, half]), chain(2, &[pad_gate])],
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, BEAT, BLOCK);
    // Before the snare: kick (1.0, delayed 100) × 0.5 + the other chain's 0.25 (aligned).
    assert!((l[BEAT / 8] - 0.75).abs() < 1e-6, "{}", l[BEAT / 8]);
    assert!((r[BEAT / 8] - 0.75).abs() < 1e-6, "{}", r[BEAT / 8]);
    // Both chains are aligned: nothing before the 100-sample pad latency.
    assert_eq!(l[50], 0.0);
    // After the snare: the kick is choked; snare 0.125 × 0.5 + 0.25.
    let at = 3 * BEAT / 8;
    assert!((l[at] - 0.3125).abs() < 1e-6, "{}", l[at]);
    // The pad chain's latency counts in the instrument rack entry (PDC).
    assert_eq!(p.handle.latency(), 100);
}

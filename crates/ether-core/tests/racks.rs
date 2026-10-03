//! Engine-side rack chains (`racks-modulation`): key/velocity/selector routing, chain mix,
//! PDC inside a rack, audio effect racks (parallel chains of the input), MIDI effect racks
//! (merged chain output replaces the rack's MIDI-thru), state across snapshot swaps.

mod common;

use common::*;
use ether_core::graph::{ChainEntry, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{ParamId, RackChainId, TrackKind, Ulid};
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

#[test]
fn audio_effect_rack_without_chains_passes_through() {
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, rack]);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::AudioEffect,
        chains: vec![],
        selector: 0,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    assert!((l[600] - 0.5).abs() < 1e-6);
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

//! v0.2 `midi-fx` engine hooks (BCR-B, base-93): a bypassed MIDI effect is MIDI thru (its
//! own params stay behind), and adding/removing/(un)bypassing a MIDI effect releases the
//! track's notes (AllNotesOff to every entry, once per change), so notes the effect generated can't hang.

mod common;

use common::*;
use ether_core::graph::ChainEntry;
use ether_core::protocol::model::{ParamId, TrackKind};
use ether_core::{
    AudioBuffers, EventKind, Node, NodeKey, ParamChange, ParamTarget, PrepareConfig,
    ProcessContext, ProcessEvent, ProcessStatus, RenderGraphDesc, TransportControl, create,
};

/// A MIDI effect (no audio) that transposes notes up an octave under its own note ids.
struct Octaver;

impl Node for Octaver {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        _audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            let kind = match e.kind {
                EventKind::NoteOn {
                    note_id,
                    channel,
                    key,
                    velocity,
                } => EventKind::NoteOn {
                    note_id: 0x8000_0000 | note_id,
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
                    note_id: 0x8000_0000 | note_id,
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
        ProcessStatus::Silent
    }
    fn channels(&self) -> (u16, u16) {
        (0, 0)
    }
}

fn graph(chain: &[(NodeKey, bool)]) -> RenderGraphDesc {
    let mut t = track(tid(2), TrackKind::Midi, Some(tid(1)));
    t.chain = chain
        .iter()
        .map(|&(node, enabled)| ChainEntry {
            node,
            enabled,
            sidechain: None,
        })
        .collect();
    t.clips = vec![midi_clip(cid(1), 0.0, 8.0, &[(0.0, 4.0, 60)])];
    RenderGraphDesc {
        version: 1,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

fn note_keys(ev: &[(u64, EventKind)]) -> Vec<(u8, bool)> {
    ev.iter()
        .filter_map(|(_, k)| match k {
            EventKind::NoteOn { key, .. } => Some((*key, true)),
            EventKind::NoteOff { key, .. } => Some((*key, false)),
            _ => None,
        })
        .collect()
}

fn all_notes_off(ev: &[(u64, EventKind)]) -> usize {
    ev.iter()
        .filter(|(_, k)| matches!(k, EventKind::AllNotesOff))
        .count()
}

#[test]
fn bypassed_midi_effect_is_midi_thru_without_its_params() {
    let mut p = create(config());
    let fx = p.handle.add_node(Box::new(Octaver)).unwrap();
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle
        .publish(graph(&[(fx, false), (rec, true)]))
        .unwrap();
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: fx,
                param: ParamId(0),
            },
            value: 1.0,
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 4096, BLOCK);
    let ev = events(&drain(&mut rx));
    // The clip note reaches the instrument untransposed.
    assert_eq!(note_keys(&ev), vec![(60, true)]);
    assert!(
        !ev.iter().any(|(_, k)| matches!(k, EventKind::Param { .. })),
        "the bypassed device's params stay behind: {ev:?}"
    );
}

#[test]
fn enabled_midi_effect_transforms() {
    let mut p = create(config());
    let fx = p.handle.add_node(Box::new(Octaver)).unwrap();
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle.publish(graph(&[(fx, true), (rec, true)])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 4096, BLOCK);
    assert_eq!(note_keys(&events(&drain(&mut rx))), vec![(72, true)]);
}

#[test]
fn removing_or_bypassing_a_midi_effect_releases_notes_once() {
    let mut p = create(config());
    let fx = p.handle.add_node(Box::new(Octaver)).unwrap();
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle.publish(graph(&[(fx, true), (rec, true)])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 4096, BLOCK);
    let ev = events(&drain(&mut rx));
    assert_eq!(note_keys(&ev), vec![(72, true)]);
    assert_eq!(all_notes_off(&ev), 0);

    // Same chain republished: nothing released.
    p.handle.publish(graph(&[(fx, true), (rec, true)])).unwrap();
    render(&mut p.engine, 1024, BLOCK);
    assert_eq!(all_notes_off(&events(&drain(&mut rx))), 0);

    // Bypass: the held (generated) note is released.
    p.handle
        .publish(graph(&[(fx, false), (rec, true)]))
        .unwrap();
    render(&mut p.engine, 1024, BLOCK);
    assert!(all_notes_off(&events(&drain(&mut rx))) >= 1);

    // Un-bypass, then remove: released each time.
    p.handle.publish(graph(&[(fx, true), (rec, true)])).unwrap();
    render(&mut p.engine, 1024, BLOCK);
    assert!(all_notes_off(&events(&drain(&mut rx))) >= 1);
    p.handle.publish(graph(&[(rec, true)])).unwrap();
    render(&mut p.engine, 1024, BLOCK);
    assert!(all_notes_off(&events(&drain(&mut rx))) >= 1);

    // An audio-only chain change doesn't touch notes.
    let dc = p.handle.add_node(Box::new(Dc(0.0))).unwrap();
    p.handle.publish(graph(&[(rec, true), (dc, true)])).unwrap();
    render(&mut p.engine, 1024, BLOCK);
    assert_eq!(all_notes_off(&events(&drain(&mut rx))), 0);
}

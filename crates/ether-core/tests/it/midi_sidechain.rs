//! MIDI sidechains (`ether_core::midi_sidechain`, CONTRACTS.md §11.10): a device hears another
//! track's notes (or its own track's) sample-accurately, instead of the chain's; the notes
//! or the main signal are delayed for PDC; a track without devices still provides its
//! notes; cycles are rejected; nothing allocates.

use crate::common;

use assert_no_alloc::assert_no_alloc;
use common::*;
use ether_core::graph::{CompileError, NodeInfo, RenderGraphDesc, TrackDesc, compile_with};
use ether_core::protocol::model::TrackKind;
use ether_core::{
    AudioBuffers, EventKind, MidiPoint, MidiSidechainDesc, Node, NodeKey, PrepareConfig,
    ProcessContext, ProcessEvent, ProcessStatus, TransportControl, create,
};

/// 120 BPM at 48 kHz.
const BEAT: u64 = 24_000;

fn desc(tracks: Vec<TrackDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

fn hears(t: &mut TrackDesc, node: NodeKey, source: u128, point: MidiPoint) {
    t.midi_sidechains.push(MidiSidechainDesc {
        node,
        source: tid(source),
        point,
    });
}

/// `(sample time, key, on)` of the notes in `seen`.
fn notes(seen: &[Seen]) -> Vec<(u64, u8, bool)> {
    events(seen)
        .into_iter()
        .filter_map(|(t, k)| match k {
            EventKind::NoteOn { key, .. } => Some((t, key, true)),
            EventKind::NoteOff { key, .. } => Some((t, key, false)),
            _ => None,
        })
        .collect()
}

/// A MIDI effect (no audio): every note an octave up, under its own ids.
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
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 0)
    }
}

/// Source S (tid 2): a MIDI track with a note at beat 0.5 lasting half a beat (key 64),
/// on `chain`.
fn source(chain: &[NodeKey]) -> TrackDesc {
    let mut s = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), chain);
    s.clips = vec![midi_clip(cid(1), 0.0, 4.0, &[(0.5, 0.5, 64)])];
    s
}

fn play(p: &mut ether_core::EngineParts, tracks: Vec<TrackDesc>, frames: usize) {
    p.handle.publish(desc(tracks)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, frames, BLOCK);
}

#[test]
fn an_effect_hears_another_tracks_notes_on_their_samples() {
    // From a track without devices (its notes are scheduled for the sidechain alone) and
    // from one with an instrument (post-FX = the notes the instrument gets): the same
    // samples, across block boundaries (12 000 = 23 · 512 + 224).
    for with_instrument in [false, true] {
        let mut p = create(config());
        let (rec, mut rx) = Recorder::new();
        let rec = p.handle.add_node(Box::new(rec)).unwrap();
        let dc = p.handle.add_node(Box::new(Dc(0.1))).unwrap();
        let (s, point) = if with_instrument {
            (source(&[dc]), MidiPoint::Before(dc))
        } else {
            (source(&[]), MidiPoint::Input)
        };
        let mut t = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[rec]);
        hears(&mut t, rec, 2, point);
        // The consumer listed first: the source still renders before it.
        play(&mut p, vec![t, master(), s], 2 * BEAT as usize);
        assert_eq!(
            notes(&drain(&mut rx)),
            vec![(BEAT / 2, 64, true), (BEAT, 64, false)],
            "instrument: {with_instrument}"
        );
    }
}

#[test]
fn the_sidechain_replaces_the_notes_of_the_chain_and_keeps_the_params() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    // The consumer's own track plays key 40 into it: replaced by the source's 64.
    let mut t = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = vec![midi_clip(cid(2), 0.0, 4.0, &[(0.0, 1.0, 40)])];
    hears(&mut t, rec, 2, MidiPoint::Input);
    play(&mut p, vec![master(), source(&[]), t], 2 * BEAT as usize);
    let n = notes(&drain(&mut rx));
    assert_eq!(n, vec![(BEAT / 2, 64, true), (BEAT, 64, false)]);
}

#[test]
fn post_fx_after_midi_effects_and_the_end_of_a_midi_effect_chain() {
    // S = [octaver, instrument]: post-FX (before the instrument) is an octave up; S =
    // [octaver] only: its end.
    for end in [false, true] {
        let mut p = create(config());
        let (rec, mut rx) = Recorder::new();
        let rec = p.handle.add_node(Box::new(rec)).unwrap();
        let oct = p.handle.add_node(Box::new(Octaver)).unwrap();
        let dc = p.handle.add_node(Box::new(Dc(0.1))).unwrap();
        let (s, point) = if end {
            (source(&[oct]), MidiPoint::End)
        } else {
            (source(&[oct, dc]), MidiPoint::Before(dc))
        };
        let mut t = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[rec]);
        hears(&mut t, rec, 2, point);
        play(&mut p, vec![master(), s, t], 2 * BEAT as usize);
        assert_eq!(
            notes(&drain(&mut rx)),
            vec![(BEAT / 2, 76, true), (BEAT, 76, false)],
            "end: {end}"
        );
    }
}

#[test]
fn an_effect_after_an_instrument_hears_its_own_tracks_notes_delayed_by_the_instrument() {
    // T = [instrument (100 samples of latency), recorder] hearing T's own notes before the
    // instrument: the audio reaches the recorder 100 samples late, so do the notes.
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let inst = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let mut t = source(&[inst, rec]);
    hears(&mut t, rec, 2, MidiPoint::Before(inst));
    play(&mut p, vec![master(), t.clone()], 2 * BEAT as usize);
    assert_eq!(
        notes(&drain(&mut rx)),
        vec![(BEAT / 2 + 100, 64, true), (BEAT + 100, 64, false)]
    );
    // Without the sidechain the instrument passes no notes on.
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let inst = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    play(
        &mut p,
        vec![master(), source(&[inst, rec])],
        2 * BEAT as usize,
    );
    assert!(notes(&drain(&mut rx)).is_empty());
}

#[test]
fn pdc_delays_the_notes_or_the_main_signal() {
    // The consumer's main signal 300 samples late at the device: the notes wait 300.
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let late = p.handle.add_node(Box::new(Delay::new(300))).unwrap();
    let mut t = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[late, rec]);
    hears(&mut t, rec, 2, MidiPoint::Input);
    play(&mut p, vec![master(), source(&[]), t], 2 * BEAT as usize);
    assert_eq!(
        notes(&drain(&mut rx)),
        vec![(BEAT / 2 + 300, 64, true), (BEAT + 300, 64, false)]
    );

    // The source's point 300 samples late (after a latent device): the consumer's main
    // signal is delayed 300 before the device, which counts in its latency.
    let mut p = create(config());
    let s_delay = p.handle.add_node(Box::new(Delay::new(300))).unwrap();
    let dc = p.handle.add_node(Box::new(Dc(0.1))).unwrap();
    let (rec, _rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let s = source(&[s_delay, dc]);
    let mut t = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[rec]);
    hears(&mut t, rec, 2, MidiPoint::Before(dc));
    let lat = move |k: NodeKey| {
        Some(NodeInfo {
            latency: if k == s_delay { 300 } else { 0 },
            channels: (2, 2),
        })
    };
    let snap = compile_with(desc(vec![master(), s, t]), &config(), &lat).unwrap();
    assert_eq!(snap.track_latency(tid(3)), Some(300));
    assert_eq!(snap.track_latency(tid(2)), Some(300));
    assert_eq!(snap.output_compensation(tid(3)), Some(0));
}

#[test]
fn midi_sidechain_cycles_are_rejected() {
    let a = NodeKey {
        index: 1,
        generation: 1,
    };
    let b = NodeKey {
        index: 2,
        generation: 1,
    };
    let mut s = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[a]);
    let mut t = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[b]);
    hears(&mut s, a, 3, MidiPoint::Input);
    hears(&mut t, b, 2, MidiPoint::Input);
    let info = |_: NodeKey| {
        Some(NodeInfo {
            latency: 0,
            channels: (2, 2),
        })
    };
    assert!(matches!(
        compile_with(desc(vec![master(), s, t]), &config(), &info),
        Err(CompileError::Cycle(_))
    ));
}

#[test]
fn midi_sidechains_never_allocate() {
    let mut p = create(config());
    let (rec, _rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let (rec2, _rx2) = Recorder::new();
    let rec2 = p.handle.add_node(Box::new(rec2)).unwrap();
    let late = p.handle.add_node(Box::new(Delay::new(700))).unwrap();
    let oct = p.handle.add_node(Box::new(Octaver)).unwrap();
    let mut t = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[late, rec]);
    hears(&mut t, rec, 2, MidiPoint::Input);
    let mut u = with_chain(track(tid(4), TrackKind::Audio, Some(tid(1))), &[rec2]);
    hears(&mut u, rec2, 5, MidiPoint::End);
    let mut s2 = with_chain(track(tid(5), TrackKind::Midi, Some(tid(1))), &[oct]);
    s2.clips = vec![midi_clip(cid(3), 0.0, 4.0, &[(0.25, 0.25, 50)])];
    p.handle
        .publish(desc(vec![master(), source(&[]), t, u, s2]))
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    // Warm up (the snapshot swap), then two beats.
    render(&mut p.engine, BLOCK, BLOCK);
    for _ in 0..(2 * BEAT as usize / BLOCK) {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc(|| p.engine.process(&[], &mut outs, BLOCK));
    }
}

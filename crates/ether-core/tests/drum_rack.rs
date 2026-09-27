//! Engine-side drum racks (`drum-rack`): note routing to pads, choke groups (fade, then
//! `NoteChoke`), smoothed pad mix changes and pad state carried across snapshot swaps.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use ether_core::graph::{ChainEntry, PadDesc, RackDesc, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{DrumPadId, PAD_PLAY_NOTE, TrackKind, Ulid};
use ether_core::{
    AudioBuffers, EventKind, Node, NodeKey, PrepareConfig, ProcessContext, ProcessStatus,
    TransportControl, create,
};

type Log = Arc<Mutex<Vec<(u64, EventKind)>>>;

/// Instrument that outputs `level` while any of its notes sounds. Notes sound until
/// choked (by id, else by key/channel) or `AllNotesOff`; note-offs are ignored (one-shot).
struct Gate {
    level: f32,
    notes: Vec<(u32, u8, u8)>,
    log: Log,
}

impl Gate {
    fn new(level: f32, log: &Log) -> Box<Self> {
        Box::new(Self {
            level,
            notes: Vec::with_capacity(64),
            log: log.clone(),
        })
    }
}

impl Node for Gate {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.notes.clear();
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut log = self.log.lock().unwrap();
        let mut pos = 0;
        let sounding = |notes: &Vec<(u32, u8, u8)>| if notes.is_empty() { 0.0 } else { 1.0 };
        for e in ctx.events {
            let off = e.offset as usize;
            let v = self.level * sounding(&self.notes);
            for ch in audio.outputs.iter_mut() {
                ch[pos..off].fill(v);
            }
            pos = off;
            log.push((ctx.transport.sample_time + off as u64, e.kind));
            match e.kind {
                EventKind::NoteOn {
                    note_id,
                    channel,
                    key,
                    ..
                } => self.notes.push((note_id, channel, key)),
                EventKind::NoteChoke {
                    note_id,
                    channel,
                    key,
                } => {
                    if self.notes.iter().any(|n| n.0 == note_id) {
                        self.notes.retain(|n| n.0 != note_id);
                    } else {
                        self.notes.retain(|n| (n.1, n.2) != (channel, key));
                    }
                }
                EventKind::AllNotesOff => self.notes.clear(),
                _ => {}
            }
        }
        let v = self.level * sounding(&self.notes);
        for ch in audio.outputs.iter_mut() {
            ch[pos..].fill(v);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// Passes its input (the pads' mix) through, like the drum rack device.
struct Pass;

impl Node for Pass {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.pass_through();
        ProcessStatus::Continue
    }
}

fn pad(id: u128, note: u8, group: Option<u8>, chain: &[NodeKey]) -> PadDesc {
    PadDesc {
        pad: DrumPadId(Ulid(id)),
        note,
        choke_group: group,
        chain: chain
            .iter()
            .map(|&node| ChainEntry {
                node,
                enabled: true,
                sidechain: None,
            })
            .collect(),
        volume: 1.0,
        pan: 0.0,
        mute: false,
    }
}

fn rack_track(rack: NodeKey, pads: Vec<PadDesc>, notes: &[(f64, f64, u8)]) -> TrackDesc {
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rack]);
    t.racks = vec![RackDesc { rack, pads }];
    t.clips = vec![midi_clip(cid(1), 0.0, 16.0, notes)];
    t
}

fn desc(version: u64, t: TrackDesc) -> RenderGraphDesc {
    RenderGraphDesc {
        version,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

const BEAT: usize = 24_000; // 120 BPM @ 48 kHz
/// `CHOKE_FADE_MS` (3 ms) at 48 kHz.
const FADE: usize = 144;

fn note_ons(log: &Log) -> Vec<(u64, u8)> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|(t, k)| match k {
            EventKind::NoteOn { key, .. } => Some((*t, *key)),
            _ => None,
        })
        .collect()
}

fn chokes(log: &Log) -> Vec<u64> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|(t, k)| matches!(k, EventKind::NoteChoke { .. }).then_some(*t))
        .collect()
}

#[test]
fn notes_route_to_pads_by_key_and_choke_groups_fade_then_cut() {
    let mut p = create(config());
    let (la, lb, lc): (Log, Log, Log) = Default::default();
    let a = p.handle.add_node(Gate::new(0.5, &la)).unwrap();
    let b = p.handle.add_node(Gate::new(0.25, &lb)).unwrap();
    let c = p.handle.add_node(Gate::new(0.125, &lc)).unwrap();
    let rack = p.handle.add_node(Box::new(Pass)).unwrap();
    let t = rack_track(
        rack,
        vec![
            pad(1, 36, Some(1), &[a]),
            pad(2, 38, Some(1), &[b]),
            pad(3, 40, None, &[c]),
        ],
        // 36 at beat 0, 40 at beat 0.5, 38 at beat 1, 37 (no pad) at beat 1.5.
        &[(0.0, 0.1, 36), (0.5, 0.1, 40), (1.0, 0.1, 38), (1.5, 0.1, 37)],
    );
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 2 * BEAT, BLOCK);

    // Routing: every pad gets its own key only, transposed to PAD_PLAY_NOTE.
    assert_eq!(note_ons(&la), vec![(0, PAD_PLAY_NOTE)]);
    assert_eq!(note_ons(&lc), vec![(BEAT as u64 / 2, PAD_PLAY_NOTE)]);
    assert_eq!(note_ons(&lb), vec![(BEAT as u64, PAD_PLAY_NOTE)]);

    // Choke: B (group 1) chokes A once A's fade-out is over; C (no group) keeps sounding.
    assert_eq!(chokes(&la), vec![(BEAT + FADE - 1) as u64]);
    assert!(chokes(&lb).is_empty() && chokes(&lc).is_empty());
    let before = l[BEAT - 1];
    assert!((before - 0.625).abs() < 1e-6, "A + C: {before}");
    // A fades out linearly over FADE samples (no step), B starts at once.
    let mid = l[BEAT + FADE / 2];
    assert!((mid - (0.375 + 0.25)).abs() < 0.01, "mid-fade: {mid}");
    for k in BEAT + 1..BEAT + FADE {
        assert!(l[k] <= l[k - 1] + 1e-6, "fade is monotonic at {k}");
        assert!(l[k - 1] - l[k] < 0.01, "no step at {k}");
    }
    assert!((l[BEAT + FADE + 10] - 0.375).abs() < 1e-6, "B + C");
    assert!((l[2 * BEAT - 1] - 0.375).abs() < 1e-6);
}

#[test]
fn a_hit_on_a_fading_pad_chokes_its_old_notes_and_restores_it() {
    let mut p = create(config());
    let (la, lb): (Log, Log) = Default::default();
    let a = p.handle.add_node(Gate::new(0.5, &la)).unwrap();
    let b = p.handle.add_node(Gate::new(0.25, &lb)).unwrap();
    let rack = p.handle.add_node(Box::new(Pass)).unwrap();
    // B hits at beat 1 (A starts fading), A is hit again 1 ms later.
    let again = 1.0 + 48.0 / BEAT as f64;
    let t = rack_track(
        rack,
        vec![pad(1, 36, Some(3), &[a]), pad(2, 38, Some(3), &[b])],
        &[(0.0, 0.1, 36), (1.0, 0.1, 38), (again, 0.1, 36)],
    );
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 2 * BEAT, BLOCK);
    // A's first note is choked right before its new note; then A chokes B (fade), for good.
    let log = la.lock().unwrap().clone();
    let hit = (BEAT + 48) as u64;
    let at_hit: Vec<_> = log.iter().filter(|(t, _)| *t == hit).map(|e| e.1).collect();
    assert!(matches!(at_hit[0], EventKind::NoteChoke { .. }), "{at_hit:?}");
    assert!(matches!(at_hit[1], EventKind::NoteOn { .. }), "{at_hit:?}");
    assert_eq!(chokes(&lb), vec![hit + FADE as u64 - 1]);
    assert!((l[BEAT + 48 + FADE + 10] - 0.5).abs() < 1e-6, "A alone again");
}

#[test]
fn pad_mix_changes_are_smoothed_and_state_survives_swaps() {
    let mut p = create(config());
    let (la, lb): (Log, Log) = Default::default();
    let a = p.handle.add_node(Gate::new(0.5, &la)).unwrap();
    let slow = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let b = p.handle.add_node(Gate::new(0.25, &lb)).unwrap();
    let rack = p.handle.add_node(Box::new(Pass)).unwrap();
    // Pad A has 100 samples of latency; pad B is delayed by 100 to stay aligned.
    let pads = vec![pad(1, 36, None, &[a, slow]), pad(2, 38, None, &[b])];
    let notes = [(0.0, 0.1, 36), (0.0, 0.1, 38)];
    p.handle
        .publish(desc(1, rack_track(rack, pads.clone(), &notes)))
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, BEAT / 2, BLOCK);
    assert!((l[BEAT / 2 - 1] - 0.75).abs() < 1e-6);

    // Same graph again (e.g. an unrelated edit): the pad delay lines carry over, no gap.
    p.handle
        .publish(desc(2, rack_track(rack, pads.clone(), &notes)))
        .unwrap();
    let (l, _) = render(&mut p.engine, 4 * BLOCK, BLOCK);
    for (k, s) in l.iter().enumerate() {
        assert!((s - 0.75).abs() < 1e-6, "sample {k} after the swap: {s}");
    }

    // Muting B ramps it out (20 ms), no step.
    let mut muted = pads.clone();
    muted[1].mute = true;
    p.handle
        .publish(desc(3, rack_track(rack, muted, &notes)))
        .unwrap();
    let (l, _) = render(&mut p.engine, 4 * BLOCK, BLOCK);
    assert!(l[0] > 0.74, "starts from the old gain: {}", l[0]);
    for k in 1..l.len() {
        assert!(l[k - 1] - l[k] < 0.001, "no step at {k}");
    }
    assert!((l[l.len() - 1] - 0.5).abs() < 1e-6);
}

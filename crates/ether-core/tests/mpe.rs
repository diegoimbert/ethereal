//! MPE playback (`mpe`, CONTRACTS.md §13.3): per-note pitch / timbre reach the chain as
//! `NoteExpression`, an MPE track announces its zone in-band (MPE Configuration Message +
//! pitch-bend sensitivities) when playback starts and when its settings change, switching
//! MPE off announces an empty zone, and a non-MPE track never announces anything.

mod common;

use common::*;
use ether_core::expression::mpe::{CONFIG_MESSAGES, MpeOut, config_messages};
use ether_core::expression::{ClipExpressionDesc, NoteExpressionDesc, TrackExpressionDesc};
use ether_core::graph::{ClipDesc, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{
    CurveShape, MpeSettings, MpeZone, NoteExpressionKind, TrackKind,
};
use ether_core::{EventKind, NodeKey, TransportControl, create};

/// 120 BPM at 48 kHz.
const SPB: u64 = 24_000;

fn desc(rec: NodeKey, clips: Vec<ClipDesc>, expression: TrackExpressionDesc) -> RenderGraphDesc {
    let mut t: TrackDesc = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = clips;
    t.expression = expression;
    RenderGraphDesc {
        version: 1,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

/// One clip, a note at 0..2 bending up an octave over its first beat, timbre 0 → 1.
fn bending(mpe: Option<MpeSettings>) -> (Vec<ClipDesc>, TrackExpressionDesc) {
    let clip = midi_clip(cid(7), 0.0, 4.0, &[(0.0, 2.0, 60)]);
    let curve = |kind, points: &[(f64, f32)]| NoteExpressionDesc {
        note: 0,
        kind,
        points: points
            .iter()
            .map(|&(t, v)| (t, v, CurveShape::Linear))
            .collect(),
    };
    let x = TrackExpressionDesc {
        clips: vec![ClipExpressionDesc {
            clip: clip.id,
            lanes: vec![],
            notes: vec![
                curve(NoteExpressionKind::Pitch, &[(0.0, 0.0), (1.0, 12.0)]),
                curve(NoteExpressionKind::Timbre, &[(0.0, 0.0), (1.0, 1.0)]),
            ],
        }],
        mpe,
    };
    (vec![clip], x)
}

fn midi(seen: &[Seen]) -> Vec<(u64, [u8; 3])> {
    events(seen)
        .into_iter()
        .filter_map(|(t, k)| match k {
            EventKind::Midi { data } => Some((t, data)),
            _ => None,
        })
        .collect()
}

fn mpe() -> MpeSettings {
    MpeSettings {
        zone: MpeZone::Lower,
        member_channels: 8,
        note_pitch_range: 24.0,
        master_pitch_range: 2.0,
    }
}

#[test]
fn per_note_pitch_and_timbre_play_on_any_midi_track() {
    let (clips, x) = bending(None);
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle.publish(desc(rec, clips, x)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 2 * SPB as usize, 512);
    let seen = drain(&mut rx);
    assert!(midi(&seen).is_empty(), "no announcement without MPE");
    let pitch: Vec<(u64, f32)> = events(&seen)
        .into_iter()
        .filter_map(|(t, k)| match k {
            EventKind::NoteExpression {
                expression: NoteExpressionKind::Pitch,
                value,
                ..
            } => Some((t, value)),
            _ => None,
        })
        .collect();
    assert_eq!(pitch[0], (0, 0.0));
    assert_eq!(pitch.last().unwrap().1, 12.0);
    assert!(pitch.windows(2).all(|w| w[0].1 < w[1].1), "rising");
    // 1/128-semitone resolution: ~1536 values over the beat at most, here bounded by the
    // automation grid.
    assert!(pitch.len() > 100, "{}", pitch.len());
    let timbre = events(&seen)
        .into_iter()
        .filter(|(_, k)| {
            matches!(
                k,
                EventKind::NoteExpression {
                    expression: NoteExpressionKind::Timbre,
                    ..
                }
            )
        })
        .count();
    assert!((100..=128).contains(&timbre), "7-bit timbre: {timbre}");
}

#[test]
fn an_mpe_track_announces_its_zone_on_play_and_on_change() {
    let (clips, x) = bending(Some(mpe()));
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle
        .publish(desc(rec, clips.clone(), x.clone()))
        .unwrap();
    // Stopped: nothing is rendered, nothing announced.
    render(&mut p.engine, 1024, 512);
    assert!(midi(&drain(&mut rx)).is_empty());
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 4096, 512);
    let seen = drain(&mut rx);
    let m = midi(&seen);
    // All on the first sample played (the engine clock went on while stopped).
    let want: Vec<(u64, [u8; 3])> = config_messages(Some(&mpe()), &mpe())
        .into_iter()
        .map(|d| (1024, d))
        .collect();
    assert_eq!(m, want);
    // Before the note-on of the same sample.
    let ev = events(&seen);
    let first_note = ev
        .iter()
        .position(|(_, k)| matches!(k, EventKind::NoteOn { .. }))
        .unwrap();
    assert!(first_note >= CONFIG_MESSAGES);
    // A host learns the zone and range from it.
    let mut out = MpeOut::default();
    for (_, d) in &m {
        out.observe(*d);
    }
    assert!(out.active());
    assert_eq!(out.note_range(), 24.0);
    // Playing on: not repeated.
    render(&mut p.engine, 4096, 512);
    let again = midi(&drain(&mut rx));
    assert!(again.is_empty(), "{again:?}");
    // Settings change: announced again.
    let mut x2 = x.clone();
    x2.mpe = Some(MpeSettings {
        note_pitch_range: 48.0,
        ..mpe()
    });
    p.handle
        .publish(desc(rec, clips.clone(), x2.clone()))
        .unwrap();
    render(&mut p.engine, 1024, 512);
    let m = midi(&drain(&mut rx));
    assert_eq!(m.len(), CONFIG_MESSAGES);
    // Stop and play: announced again (a plugin may have been reset meanwhile).
    p.handle.transport(TransportControl::Stop).unwrap();
    render(&mut p.engine, 1024, 512);
    drain(&mut rx);
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 1024, 512);
    assert_eq!(midi(&drain(&mut rx)).len(), CONFIG_MESSAGES);
    // MPE off: an empty zone.
    let mut x3 = x;
    x3.mpe = None;
    p.handle.publish(desc(rec, clips, x3)).unwrap();
    render(&mut p.engine, 1024, 512);
    let m = midi(&drain(&mut rx));
    assert_eq!(m.len(), CONFIG_MESSAGES);
    assert_eq!(
        &m[..3].iter().map(|e| e.1).collect::<Vec<_>>(),
        &[[0xB0, 101, 0], [0xB0, 100, 6], [0xB0, 6, 0]]
    );
    for (_, d) in &m {
        out.observe(*d);
    }
    assert!(!out.active());
}

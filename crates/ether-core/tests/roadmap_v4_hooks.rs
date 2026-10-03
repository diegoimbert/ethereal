//! v0.3 engine prewire (contracts-4): the new `TrackDesc` fields round-trip through the web
//! codec (inside the v0.2 JSON blob), the expression/hardware stubs are RT-safe no-ops, and
//! `EventKind::NoteExpression` exists. `midi-expression` and `external-instrument` extend
//! this with real rendering tests.

mod common;

use common::*;
use ether_core::codec::{BinaryCodec, GraphCodec};
use ether_core::expression::{
    ClipExpressionDesc, ExpressionLaneDesc, ExpressionRt, NoteExpressionDesc, TrackExpressionDesc,
};
use ether_core::graph::RenderGraphDesc;
use ether_core::hw_io::{HwIoDesc, HwIoRt};
use ether_core::protocol::model::{
    CurveShape, ExpressionKind, ExternalRouting, HwChannels, MpeSettings, NoteExpressionKind,
    TrackKind,
};
use ether_core::{EventKind, NodeKey};

fn desc_with_v03_fields() -> RenderGraphDesc {
    let mut t = track(tid(1), TrackKind::Midi, None);
    t.expression = TrackExpressionDesc {
        clips: vec![ClipExpressionDesc {
            clip: cid(7),
            lanes: vec![ExpressionLaneDesc {
                kind: ExpressionKind::Cc { controller: 11 },
                points: vec![
                    (0.0, 0.25, CurveShape::Linear),
                    (1.0, 1.0, CurveShape::Step),
                ],
            }],
            notes: vec![NoteExpressionDesc {
                note: 0,
                kind: NoteExpressionKind::Pitch,
                points: vec![(0.0, -2.0, CurveShape::Curve { tension: 0.5 })],
            }],
        }],
        mpe: Some(MpeSettings::default()),
    };
    t.hw_io = vec![HwIoDesc {
        node: NodeKey {
            index: 3,
            generation: 1,
        },
        routing: ExternalRouting {
            midi_out: Some("Synth Port".into()),
            midi_channel: 3,
            audio_send: None,
            audio_return: Some(HwChannels { first: 4, count: 2 }),
        },
    }];
    RenderGraphDesc {
        tracks: vec![t, master()],
        ..Default::default()
    }
}

#[test]
fn v03_track_fields_roundtrip_through_the_codec() {
    let desc = desc_with_v03_fields();
    let mut bytes = Vec::new();
    BinaryCodec.encode(&desc, &mut bytes);
    assert_eq!(BinaryCodec.decode(&bytes).unwrap(), desc);
    // All-default tracks keep the compact encoding (no JSON blob).
    let plain = RenderGraphDesc {
        tracks: vec![master()],
        ..Default::default()
    };
    let mut a = Vec::new();
    BinaryCodec.encode(&plain, &mut a);
    assert_eq!(BinaryCodec.decode(&a).unwrap(), plain);
    assert!(a.len() < bytes.len());
}

#[test]
fn expression_and_hw_io_stubs_do_nothing() {
    let desc = desc_with_v03_fields();
    let t = &desc.tracks[0];
    // `ExpressionRt` renders inside the engine (`midi-expression`; tests in
    // `tests/expression*.rs`): here only its non-RT construction.
    let mut rt = ExpressionRt::compile(&t.expression);
    rt.prepare(&t.expression);
    rt.reset();
    let mut hw = HwIoRt::default();
    hw.prepare(&t.hw_io, 128);
    let input = [0.5f32; 128];
    hw.gather_returns(&[&input], 128);
    let mut out = [0.0f32; 128];
    hw.write_sends(&mut [&mut out], 128);
    assert!(out.iter().all(|s| *s == 0.0));
    // The per-note expression event (CLAP note expression model).
    let e = EventKind::NoteExpression {
        note_id: 1,
        channel: 0,
        key: 60,
        expression: NoteExpressionKind::Pressure,
        value: 0.5,
    };
    assert!(matches!(e, EventKind::NoteExpression { value, .. } if value == 0.5));
}

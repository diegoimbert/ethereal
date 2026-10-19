//! MPE (`mpe`, CONTRACTS.md §13.3): `Expression::SetTrackMpe` (MIDI tracks only, checked,
//! idempotent, one undo step), the compile hook (`TrackExpressionDesc::mpe`), and an
//! offline render of per-note pitch / pressure / timbre through the Poly Synth: the pitch is
//! audible (frequency), the same for blocks of 64 and 512, and an MPE track sounds the same
//! as a plain one (the in-band MPE announcement is inaudible to the synth).

mod common;

use common::*;
use ether_core::config::EngineConfig;
use ether_core::offline::OfflineRenderer;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};
use ether_core::{Device, Node, RenderGraphDesc};

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn set_mpe(track: TrackId, mpe: Option<MpeSettings>) -> Command {
    Command::Expression(ExpressionCommand::SetTrackMpe { track, mpe })
}

fn code(h: &mut Harness, c: Command) -> ErrorCode {
    err(&h.send(c)).code
}

fn mpe_of(h: &Harness, t: TrackId) -> Option<MpeSettings> {
    h.ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|d| d.id == t)
        .unwrap()
        .expression
        .mpe
}

#[test]
fn set_track_mpe_validates_undoes_and_compiles() {
    let mut h = Harness::with_project();
    let midi = track(&mut h, TrackKind::Midi);
    let audio = track(&mut h, TrackKind::Audio);
    let m = MpeSettings {
        zone: MpeZone::Upper,
        member_channels: 7,
        note_pitch_range: 24.0,
        master_pitch_range: 12.0,
    };
    assert_eq!(
        code(&mut h, set_mpe(audio, Some(m))),
        ErrorCode::InvalidArgument
    );
    for bad in [
        MpeSettings {
            member_channels: 0,
            ..m
        },
        MpeSettings {
            member_channels: 16,
            ..m
        },
        MpeSettings {
            note_pitch_range: 0.5,
            ..m
        },
        MpeSettings {
            master_pitch_range: f32::NAN,
            ..m
        },
    ] {
        assert_eq!(
            code(&mut h, set_mpe(midi, Some(bad))),
            ErrorCode::InvalidArgument,
            "{bad:?}"
        );
    }
    let missing: TrackId = h.id();
    assert_eq!(code(&mut h, set_mpe(missing, Some(m))), ErrorCode::NotFound);
    assert!(h.project().tracks[&midi].mpe.is_none());

    h.ok(set_mpe(midi, Some(m)));
    assert_eq!(h.project().tracks[&midi].mpe, Some(m));
    h.tick();
    assert_eq!(mpe_of(&h, midi), Some(m));
    // Idempotent: the same settings make no undo step.
    let out = h.send(set_mpe(midi, Some(m)));
    assert!(patches(&out).is_empty());
    h.ok(set_mpe(midi, None));
    h.tick();
    assert_eq!(mpe_of(&h, midi), None);
    // One undo step each.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tracks[&midi].mpe, Some(m));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tracks[&midi].mpe, None);
}

/// A MIDI track with a Poly Synth (sine, open filter) playing A3 for two beats with an
/// octave pitch expression and pressure / timbre curves.
fn poly_track(h: &mut Harness, mpe: Option<MpeSettings>) -> TrackId {
    let t = track(h, TrackKind::Midi);
    let dev: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: dev,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::PolySynth,
        },
        before: None,
    }));
    if mpe.is_some() {
        h.ok(set_mpe(t, mpe));
    }
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track: t,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
    let note: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: vec![NoteSpec {
            id: note,
            pitch: 57,
            velocity: 0.8,
            start: Beats(0.0),
            duration: Beats(2.0),
        }],
    }));
    let pt = |t: f64, v: f32, curve| ExpressionPoint {
        time: Beats(t),
        value: v,
        curve,
    };
    for (kind, points) in [
        (
            NoteExpressionKind::Pitch,
            vec![
                pt(0.0, 0.0, CurveShape::Step),
                pt(1.0, 12.0, CurveShape::Step),
            ],
        ),
        (
            NoteExpressionKind::Pressure,
            vec![
                pt(0.0, 0.0, CurveShape::Linear),
                pt(2.0, 1.0, CurveShape::Linear),
            ],
        ),
        (
            NoteExpressionKind::Timbre,
            vec![pt(0.0, 0.5, CurveShape::Step)],
        ),
    ] {
        let id: NoteExpressionId = h.id();
        h.ok(Command::Expression(ExpressionCommand::SetNoteExpression {
            id,
            note,
            kind,
            points,
        }));
    }
    h.tick();
    t
}

fn render(h: &mut Harness, t: TrackId, block: usize) -> Vec<f32> {
    let mut g: RenderGraphDesc = h.ctl.bridge.last_graph().clone();
    let config = EngineConfig {
        sample_rate: 48_000,
        max_block_size: block,
        max_nodes: 16,
        max_events_per_block: 1024,
        ..EngineConfig::default()
    };
    let mut r = OfflineRenderer::new(config);
    let mut synth = ether_devices::poly_synth::PolySynth::new();
    // A clean sine (no filter colour) so the pitch can be measured.
    use ether_devices::poly_synth::poly_synth as p;
    for (id, v) in [
        (p::OSC1_TYPE, 3.0),
        (p::CUTOFF, 20_000.0),
        (p::RESONANCE, 0.0),
        (p::FILTER_TYPE, 1.0),
        (p::AMP_SUSTAIN, 100.0),
    ] {
        synth.set_param(id, v);
    }
    let synth: Box<dyn Node> = Box::new(synth);
    let key = r.handle().add_node(synth).unwrap();
    for td in &mut g.tracks {
        if td.id == t {
            assert_eq!(td.chain.len(), 1);
            td.chain[0].node = key;
        } else {
            td.chain.clear();
            td.clips.clear();
        }
    }
    r.publish(g).unwrap();
    r.start(Beats(0.0)).unwrap();
    let frames = 2 * 24_000;
    let mut l = vec![0.0f32; frames];
    let mut rr = vec![0.0f32; frames];
    r.render(frames, &mut [&mut l, &mut rr]);
    l
}

/// Frequency from positive-going zero crossings.
fn frequency(x: &[f32]) -> f32 {
    let c: Vec<f32> = (1..x.len())
        .filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(c.len() > 2, "no periodic signal");
    (c.len() - 1) as f32 * 48_000.0 / (c[c.len() - 1] - c[0])
}

#[test]
fn per_note_expression_plays_through_the_poly_synth() {
    let mut h = Harness::with_project();
    let t = poly_track(&mut h, Some(MpeSettings::default()));
    let small = render(&mut h, t, 64);
    let large = render(&mut h, t, 512);
    assert_eq!(small, large, "identical for blocks of 64 and 512");
    // Beat 0..1 at 220 Hz, then an octave up (step at beat 1 = 24 000 samples).
    let f0 = frequency(&large[2_400..23_000]);
    let f1 = frequency(&large[26_000..47_000]);
    assert!((f0 / 220.0 - 1.0).abs() < 0.003, "{f0}");
    assert!((f1 / 440.0 - 1.0).abs() < 0.003, "{f1}");
    // A plain track plays the same expression the same way (the MPE announcement is
    // configuration MIDI the synth ignores).
    let mut h2 = Harness::with_project();
    let t2 = poly_track(&mut h2, None);
    assert_eq!(render(&mut h2, t2, 512), large);
}

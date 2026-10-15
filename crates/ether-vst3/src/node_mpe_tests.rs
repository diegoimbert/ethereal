//! v0.3 (`mpe`): VST3 note expression values and the MPE MIDI fallback.

use super::*;
use ether_core::expression::mpe::config_messages;
use ether_core::protocol::model::MpeSettings;

#[test]
fn tuning_is_normalized_over_plus_minus_120_semitones() {
    assert_eq!(tuning_normalized(0.0), 0.5);
    assert_eq!(tuning_normalized(12.0), 0.55);
    assert_eq!(tuning_normalized(-120.0), 0.0);
    assert_eq!(tuning_normalized(96.0), 0.9);
    assert_eq!(tuning_normalized(500.0), 1.0);
    // Round trip through the VST3 plain formula.
    for s in [-48.0f32, -0.5, 7.25, 24.0] {
        let plain = 240.0 * (tuning_normalized(s) - 0.5);
        assert!((plain - f64::from(s)).abs() < 1e-9);
    }
}

#[test]
fn mpe_midi_reaches_member_channel_mappings() {
    // Without note expression support, MpeOut moves a note and its pitch to a member
    // channel; `vst3_midi` reads that channel's bend as `kPitchBend` (per-channel
    // `IMidiMapping`, how MPE-capable VST3 plugins take MPE).
    let m = MpeSettings::default();
    let mut out = MpeOut::default();
    for d in config_messages(Some(&m), &m) {
        out.observe(d);
    }
    let mut got = Vec::new();
    out.translate(
        &EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 1.0,
        },
        |e| got.push(e),
    );
    out.translate(
        &EventKind::NoteExpression {
            note_id: 1,
            channel: 0,
            key: 60,
            expression: NoteExpressionKind::Pitch,
            value: 48.0,
        },
        |e| got.push(e),
    );
    let EventKind::Midi { data } = got[1] else {
        panic!("{got:?}")
    };
    assert_eq!(
        vst3_midi(data),
        Some(Vst3Midi::Controller {
            channel: 1,
            ctrl: kPitchBend as usize,
            value: 1.0
        })
    );
}

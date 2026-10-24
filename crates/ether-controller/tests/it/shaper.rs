//! Shaper curves (`shaper`): `Device::SetShaperCurve` replaces one lane in one undo step,
//! sanitized, only on a shaper; factory presets carry their three curves.

use crate::common;

use common::*;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::presets::{PresetCommand, PresetRef, PresetSource};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::*;

fn insert(h: &mut Harness, track: TrackId, device: BuiltinDevice) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin { device },
        before: None,
    }));
    id
}

fn setup() -> (Harness, TrackId, DeviceId) {
    let mut h = Harness::new();
    h.create_project("Shapes");
    let track: TrackId = h.id();
    h.ok(Command::Track(
        ether_core::protocol::tracks::TrackCommand::Create {
            id: track,
            kind: TrackKind::Audio,
            name: None,
            color: None,
            parent: None,
            before: None,
        },
    ));
    let d = insert(&mut h, track, BuiltinDevice::new(BuiltinDeviceType::Shaper));
    (h, track, d)
}

fn curves_of(h: &Harness, d: DeviceId) -> ShaperCurves {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::Shaper { curves },
        } => curves.clone(),
        other => panic!("not a shaper: {other:?}"),
    }
}

fn set(device: DeviceId, lane: ShaperLane, points: Vec<LfoPoint>) -> Command {
    Command::Device(DeviceCommand::SetShaperCurve {
        device,
        lane,
        curve: LfoCurve { points },
    })
}

#[test]
fn set_curve_replaces_one_lane_in_one_undo_step() {
    let (mut h, _, d) = setup();
    let start = curves_of(&h, d);
    assert_eq!(start, ShaperCurves::default());
    h.ok(set(d, ShaperLane::Pan, vec![LfoPoint::linear(0.0, 1.0)]));
    // Out of range: clamped.
    h.ok(set(
        d,
        ShaperLane::Cutoff,
        vec![LfoPoint::linear(0.5, 3.0), LfoPoint::linear(-1.0, 0.25)],
    ));
    let c = curves_of(&h, d);
    assert_eq!(c.pan.points, vec![LfoPoint::linear(0.0, 1.0)]);
    assert_eq!(
        c.cutoff.points,
        vec![LfoPoint::linear(0.0, 0.25), LfoPoint::linear(0.5, 1.0)]
    );
    assert_eq!(c.volume, start.volume);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(curves_of(&h, d).cutoff, start.cutoff);
    assert_eq!(
        curves_of(&h, d).pan.points,
        vec![LfoPoint::linear(0.0, 1.0)]
    );
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(curves_of(&h, d), start);
}

#[test]
fn set_curve_only_targets_a_shaper() {
    let (mut h, track, _) = setup();
    let lfo = insert(
        &mut h,
        track,
        BuiltinDevice::new(BuiltinDeviceType::VolumeLfo),
    );
    let out = h.send(set(
        lfo,
        ShaperLane::Volume,
        vec![LfoPoint::linear(0.0, 0.5)],
    ));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn factory_presets_carry_the_curves() {
    let (mut h, _, d) = setup();
    for fp in ether_devices::factory_presets(BuiltinDeviceType::Shaper) {
        let preset = load_preset(fp.json).unwrap_or_else(|e| panic!("{}: {e}", fp.id));
        let Some(BuiltinDevice::Shaper { curves }) = preset.kind.clone() else {
            panic!("{}: no curves", fp.id);
        };
        assert!(curves.check().is_ok(), "{}", fp.id);
        h.ok(Command::Preset(PresetCommand::Load {
            device: d,
            preset: PresetRef {
                source: PresetSource::Factory,
                id: fp.id.into(),
            },
            seed: None,
        }));
        assert_eq!(curves_of(&h, d), curves, "{}", fp.id);
    }
}

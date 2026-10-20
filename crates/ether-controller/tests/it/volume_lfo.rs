//! Volume LFO curves (`fx-modulation`): `Device::SetLfoCurve` is one undo step, sanitizes
//! the curve and only targets a Volume LFO; curve edits reach the engine; presets (factory
//! and saved) carry the curve.

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
    // A user library, for saving presets.
    let lib = ether_controller::memory::MemoryLibrary::new().with_user_root("user");
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Pump");
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
    let d = insert(
        &mut h,
        track,
        BuiltinDevice::new(BuiltinDeviceType::VolumeLfo),
    );
    (h, track, d)
}

fn curve_of(h: &Harness, d: DeviceId) -> LfoCurve {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::VolumeLfo { curve },
        } => curve.clone(),
        other => panic!("not a volume lfo: {other:?}"),
    }
}

fn set(device: DeviceId, points: Vec<LfoPoint>) -> Command {
    Command::Device(DeviceCommand::SetLfoCurve {
        device,
        curve: LfoCurve { points },
    })
}

#[test]
fn set_curve_is_one_undo_step_and_sanitized() {
    let (mut h, _, d) = setup();
    assert_eq!(curve_of(&h, d), LfoCurve::default());
    let square = vec![
        LfoPoint {
            x: 0.0,
            y: 1.0,
            curve: CurveShape::Step,
        },
        LfoPoint {
            x: 0.5,
            y: 0.0,
            curve: CurveShape::Step,
        },
    ];
    h.ok(set(d, square.clone()));
    assert_eq!(curve_of(&h, d).points, square);
    // Out of order, out of range: stored sorted and clamped.
    h.ok(set(
        d,
        vec![LfoPoint::linear(0.7, 3.0), LfoPoint::curved(-0.5, 0.2, 9.0)],
    ));
    assert_eq!(
        curve_of(&h, d).points,
        vec![LfoPoint::curved(0.0, 0.2, 1.0), LfoPoint::linear(0.7, 1.0)]
    );
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(curve_of(&h, d).points, square);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(curve_of(&h, d), LfoCurve::default());
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(curve_of(&h, d).points, square);
}

#[test]
fn set_curve_only_targets_a_volume_lfo() {
    let (mut h, track, _) = setup();
    let comp = insert(&mut h, track, BuiltinDevice::Compressor);
    let out = h.send(set(comp, vec![LfoPoint::linear(0.0, 0.5)]));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let missing: DeviceId = h.id();
    let out = h.send(set(missing, vec![LfoPoint::linear(0.0, 0.5)]));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

#[test]
fn curve_edits_reach_the_engine() {
    let (mut h, _, d) = setup();
    h.tick();
    let created = |h: &Harness| {
        h.ctl
            .bridge
            .calls
            .iter()
            .filter(|c| matches!(c, Call::CreateBuiltin(id, _) if *id == d))
            .count()
    };
    let n0 = created(&h);
    h.ok(set(d, vec![LfoPoint::linear(0.0, 0.25)]));
    h.tick();
    // The fake bridge has no in-place update: the node is rebuilt with the new curve.
    assert_eq!(created(&h), n0 + 1);
    // The same curve again: nothing to do.
    h.ok(set(d, vec![LfoPoint::linear(0.0, 0.25)]));
    h.tick();
    assert_eq!(created(&h), n0 + 1);
}

#[test]
fn presets_carry_the_curve() {
    let (mut h, track, d) = setup();
    // Every factory preset loads (curve and params) in one undo step.
    for fp in ether_devices::factory_presets(BuiltinDeviceType::VolumeLfo) {
        let preset = load_preset(fp.json).unwrap_or_else(|e| panic!("{}: {e}", fp.id));
        let Some(BuiltinDevice::VolumeLfo { curve }) = preset.kind.clone() else {
            panic!("{}: no curve", fp.id);
        };
        assert!(curve.check().is_ok(), "{}", fp.id);
        h.ok(Command::Preset(PresetCommand::Load {
            device: d,
            preset: PresetRef {
                source: PresetSource::Factory,
                id: fp.id.into(),
            },
            seed: None,
        }));
        assert_eq!(curve_of(&h, d), curve, "{}", fp.id);
        for (id, v) in &preset.params {
            assert_eq!(h.project().devices[&d].params.get(id), Some(v), "{}", fp.id);
        }
    }
    // A saved preset keeps the drawn curve.
    let drawn = vec![LfoPoint::linear(0.0, 0.0), LfoPoint::curved(0.3, 1.0, 0.5)];
    h.ok(set(d, drawn.clone()));
    let saved = match h.ok(Command::Preset(PresetCommand::Save {
        device: d,
        name: "My Pump".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    })) {
        ReplyValue::Preset { preset } => preset,
        other => panic!("{other:?}"),
    };
    let other = insert(
        &mut h,
        track,
        BuiltinDevice::new(BuiltinDeviceType::VolumeLfo),
    );
    h.ok(Command::Preset(PresetCommand::Load {
        device: other,
        preset: saved.preset,
        seed: None,
    }));
    assert_eq!(curve_of(&h, other).points, drawn);
}

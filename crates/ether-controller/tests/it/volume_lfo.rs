//! Volume LFO curves (`fx-modulation`): `Device::SetLfoCurve` is one undo step, sanitizes
//! the curve and only targets a Volume LFO (and an existing band); band curves fill in
//! flat; curve edits reach the engine; presets (factory and saved) carry the curves.

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

fn curves_of(h: &Harness, d: DeviceId) -> (LfoCurve, Vec<LfoCurve>) {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::VolumeLfo { curve, bands },
        } => (curve.clone(), bands.clone()),
        other => panic!("not a volume lfo: {other:?}"),
    }
}

fn curve_of(h: &Harness, d: DeviceId) -> LfoCurve {
    curves_of(h, d).0
}

fn set(device: DeviceId, points: Vec<LfoPoint>) -> Command {
    set_band(device, 0, points)
}

fn set_band(device: DeviceId, band: u8, points: Vec<LfoPoint>) -> Command {
    Command::Device(DeviceCommand::SetLfoCurve {
        device,
        band,
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
fn band_curves_are_undo_steps_and_fill_in_flat() {
    let (mut h, _, d) = setup();
    assert_eq!(curves_of(&h, d).1, Vec::<LfoCurve>::new());
    // Band 3 first: band 2 gets a flat curve.
    let chop = vec![
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
    h.ok(set_band(d, 2, chop.clone()));
    let (curve, bands) = curves_of(&h, d);
    assert_eq!(curve, LfoCurve::default());
    assert_eq!(
        bands,
        vec![
            LfoCurve::flat(),
            LfoCurve {
                points: chop.clone()
            }
        ]
    );
    // Band 2, sanitized; band 1 and 3 untouched.
    h.ok(set_band(d, 1, vec![LfoPoint::linear(0.4, 2.0)]));
    let (curve, bands) = curves_of(&h, d);
    assert_eq!(curve, LfoCurve::default());
    assert_eq!(bands[0].points, vec![LfoPoint::linear(0.4, 1.0)]);
    assert_eq!(bands[1].points, chop);
    // Band 1 keeps the others.
    h.ok(set(d, vec![LfoPoint::linear(0.0, 0.5)]));
    assert_eq!(curves_of(&h, d).1.len(), 2);
    // Out of range.
    let out = h.send(set_band(d, 3, vec![LfoPoint::linear(0.0, 0.5)]));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Each edit was one undo step.
    for _ in 0..3 {
        h.ok(Command::Edit(EditCommand::Undo));
    }
    assert_eq!(curves_of(&h, d), (LfoCurve::default(), Vec::new()));
    // An agent's JSON without `band` edits band 1; the old project form (no `bands`) loads.
    let cmd: DeviceCommand = serde_json::from_value(serde_json::json!({
        "type": "SetLfoCurve",
        "device": d,
        "curve": {"points": [{"x": 0.0, "y": 0.3, "curve": {"type": "Linear"}}]},
    }))
    .unwrap();
    assert!(matches!(cmd, DeviceCommand::SetLfoCurve { band: 0, .. }));
    let old: BuiltinDevice = serde_json::from_value(serde_json::json!({
        "type": "VolumeLfo",
        "curve": {"points": [{"x": 0.0, "y": 0.3, "curve": {"type": "Linear"}}]},
    }))
    .unwrap();
    assert!(matches!(old, BuiltinDevice::VolumeLfo { ref bands, .. } if bands.is_empty()));
    // ... and saves without it.
    let json = serde_json::to_value(&old).unwrap();
    assert!(json.get("bands").is_none());
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
        let Some(BuiltinDevice::VolumeLfo { curve, bands }) = preset.kind.clone() else {
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
        assert_eq!(curves_of(&h, d), (curve, bands), "{}", fp.id);
        for (id, v) in &preset.params {
            assert_eq!(h.project().devices[&d].params.get(id), Some(v), "{}", fp.id);
        }
        // An older (one-band) preset puts the band params back to their defaults.
        if !preset
            .params
            .contains_key(&ether_devices::fx_modulation::volume_lfo::BANDS)
        {
            let bands = ether_devices::fx_modulation::volume_lfo::BANDS;
            assert_eq!(
                h.project().devices[&d]
                    .params
                    .get(&bands)
                    .copied()
                    .unwrap_or(0.0),
                0.0,
                "{}",
                fp.id
            );
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

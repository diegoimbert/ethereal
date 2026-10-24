//! Old documents' Auto Filter `Type (legacy)` becomes `Type` + `Slope` on load.

use ether_controller::device_upgrade::upgrade_project;
use ether_core::protocol::model::*;
use ether_devices::fx_color::auto_filter as af;

fn auto_filter(ids: &mut IdGen, now: u64, track: TrackId, params: &[(ParamId, f64)]) -> Device {
    Device {
        sidechain_tap: None,
        midi_sidechain: None,
        chain: None,
        id: ids.next(now),
        track,
        order: OrderKey::between(None, None),
        name: "Auto Filter".into(),
        enabled: true,
        kind: DeviceKind::Builtin {
            device: BuiltinDevice::AutoFilter,
        },
        params: params.iter().copied().collect(),
        sidechain: None,
        pad: None,
    }
}

#[test]
fn legacy_auto_filter_type_becomes_type_and_slope() {
    let now = 1_700_000_000_000u64;
    let mut ids = IdGen::new(7);
    let mut p = Project::new(&mut ids, now);
    let t: TrackId = ids.next(now);
    // High-pass 24 (legacy 3), band-pass (4), the default low-pass 12 and an already new one.
    let hp24 = auto_filter(&mut ids, now, t, &[(af::TYPE, 3.0), (af::CUTOFF, 500.0)]);
    let bp = auto_filter(&mut ids, now, t, &[(af::TYPE, 4.0)]);
    let lp12 = auto_filter(&mut ids, now, t, &[]);
    let new = auto_filter(&mut ids, now, t, &[(af::MODE, 1.0), (af::SLOPE, 2.0)]);
    let (hp24_id, bp_id, lp12_id, new_id) = (hp24.id, bp.id, lp12.id, new.id);
    for d in [hp24, bp, lp12, new] {
        p.devices.insert(d.id, d);
    }
    // A lane on the legacy type of the default one: notch (5 of 0..6) → notch (3 of 0..4).
    let lane: AutomationLaneId = ids.next(now);
    p.automation_lanes.insert(
        lane,
        AutomationLane {
            id: lane,
            owner: AutomationOwner::Track { track: t },
            target: AutomationTarget::DeviceParam {
                device: lp12_id,
                param: af::TYPE,
            },
            enabled: true,
        },
    );
    let point: AutomationPointId = ids.next(now);
    p.automation_points.insert(
        point,
        AutomationPoint {
            id: point,
            lane,
            time: Beats::ZERO,
            value: 5.0 / 6.0,
            curve: CurveShape::Step,
        },
    );
    assert!(upgrade_project(&mut p));
    let params = |id: DeviceId| {
        let d = &p.devices[&id];
        [af::TYPE, af::MODE, af::SLOPE].map(|k| d.params.get(&k).copied().unwrap_or(0.0))
    };
    assert_eq!(params(hp24_id), [0.0, 1.0, 1.0]);
    assert_eq!(p.devices[&hp24_id].params[&af::CUTOFF], 500.0);
    assert_eq!(params(bp_id), [0.0, 2.0, 0.0]);
    assert_eq!(params(lp12_id), [0.0, 0.0, 0.0]);
    assert_eq!(params(new_id), [0.0, 1.0, 2.0]);
    assert_eq!(
        p.automation_lanes[&lane].target,
        AutomationTarget::DeviceParam {
            device: lp12_id,
            param: af::MODE,
        }
    );
    assert!((p.automation_points[&point].value - 0.75).abs() < 1e-9);
    // Idempotent.
    assert!(!upgrade_project(&mut p));
}

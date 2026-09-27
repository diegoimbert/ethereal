//! Contract alignment: the mixer automation targets' `ParamInfo` (the single source of
//! truth, `compile::track_param_info`) matches the shared vectors that the UI checks its
//! mixer faders and automation lanes against (`ui/src/app/contracts.test.ts`).

use ether_controller::compile::{
    PAN_MAPPING, SEND_LEVEL_MAPPING, TRACK_VOLUME_MAPPING, track_param_info,
};
use ether_controller::{MAX_SEND_DB, MAX_VOLUME_DB, SILENCE_DB};
use ether_core::protocol::devices::{ParamScale, ParamUnit};
use ether_core::protocol::model::{AutomationTarget, SendId, TrackId, Ulid};
use serde_json::Value;

fn mixer_targets() -> Value {
    let v: Value = serde_json::from_str(include_str!(
        "../../ether-protocol/tests/param_scale_vectors.json"
    ))
    .unwrap();
    v["mixer_targets"].clone()
}

#[test]
fn mixer_automation_param_info_matches_the_shared_vectors() {
    let expected = mixer_targets();
    let track = TrackId(Ulid(1));
    for (name, target) in [
        ("TrackVolume", AutomationTarget::TrackVolume { track }),
        (
            "SendLevel",
            AutomationTarget::SendLevel {
                send: SendId(Ulid(2)),
            },
        ),
        ("TrackPan", AutomationTarget::TrackPan { track }),
    ] {
        let info = track_param_info(&target).unwrap();
        let e = &expected[name];
        assert_eq!(info.min, e["min"].as_f64().unwrap(), "{name} min");
        assert_eq!(info.max, e["max"].as_f64().unwrap(), "{name} max");
        assert_eq!(
            info.default,
            e["default"].as_f64().unwrap(),
            "{name} default"
        );
        let scale: ParamScale = serde_json::from_value(e["scale"].clone()).unwrap();
        assert_eq!(info.scale, scale, "{name} scale");
        let unit: ParamUnit = serde_json::from_value(e["unit"].clone()).unwrap();
        assert_eq!(info.unit, unit, "{name} unit");
    }
    // The mappings the engine evaluates agree with the ParamInfo the UI reads.
    assert_eq!(TRACK_VOLUME_MAPPING.min, SILENCE_DB as f64);
    assert_eq!(TRACK_VOLUME_MAPPING.max, MAX_VOLUME_DB as f64);
    assert_eq!(SEND_LEVEL_MAPPING.max, MAX_SEND_DB as f64);
    assert_eq!(MAX_SEND_DB, MAX_VOLUME_DB);
    assert_eq!(PAN_MAPPING.scale, ParamScale::Linear);
}

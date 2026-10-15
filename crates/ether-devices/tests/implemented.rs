//! The `implemented` flag (#438): the registry ([`ether_devices::UNIMPLEMENTED`]) is the single
//! source of `DeviceDescriptor::implemented`; type and instance descriptors agree with it, a
//! contract placeholder can't pass for a working device, and the wire format keeps old
//! descriptors (no field) implemented.

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_devices::contract::{Placeholder, PlaceholderMode};
use ether_devices::{NoSamples, UNIMPLEMENTED};

#[test]
fn type_and_instance_descriptors_carry_the_registry_flag() {
    for t in BuiltinDeviceType::ALL {
        let expected = !UNIMPLEMENTED.contains(&t);
        assert_eq!(ether_devices::implemented(t), expected, "{t:?}");
        assert_eq!(ether_devices::descriptor(t).implemented, expected, "{t:?}");
        // A node that is still a placeholder reports `implemented: false`: a type missing from
        // UNIMPLEMENTED (or one left there after its node landed) fails here.
        let node = ether_devices::create(&BuiltinDevice::new(t), &NoSamples);
        assert_eq!(
            node.descriptor().implemented,
            expected,
            "{t:?}: instance vs registry"
        );
    }
}

#[test]
fn offered_descriptors_are_the_implemented_ones() {
    let all = ether_devices::all_descriptors();
    assert_eq!(
        all.len(),
        BuiltinDeviceType::ALL.len(),
        "the full list keeps every type"
    );
    let offered = ether_devices::offered_descriptors();
    let expected: Vec<DeviceDescriptor> = all.into_iter().filter(|d| d.implemented).collect();
    assert_eq!(offered, expected);
    for d in &offered {
        let DeviceTypeRef::Builtin { device } = d.device_type else {
            panic!("{d:?}");
        };
        assert!(!UNIMPLEMENTED.contains(&device), "{device:?}");
    }
}

#[test]
fn a_placeholder_is_never_implemented() {
    let d = ether_devices::descriptor(BuiltinDeviceType::Chorus);
    assert!(d.implemented);
    let p = Placeholder::new(d, PlaceholderMode::PassThrough);
    assert!(!p.descriptor().implemented);
}

#[test]
fn json_omits_the_flag_when_implemented_and_defaults_it_to_true() {
    let mut d = ether_devices::descriptor(BuiltinDeviceType::Synth);
    assert_eq!(d.category, DeviceCategory::Instrument);
    let v = serde_json::to_value(&d).unwrap();
    assert!(v.get("implemented").is_none(), "omitted when true: {v}");
    // Descriptors serialized before the flag existed (no field) are implemented.
    let back: DeviceDescriptor = serde_json::from_value(v).unwrap();
    assert!(back.implemented);

    d.implemented = false;
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["implemented"], serde_json::json!(false));
    let back: DeviceDescriptor = serde_json::from_value(v).unwrap();
    assert!(!back.implemented);
}

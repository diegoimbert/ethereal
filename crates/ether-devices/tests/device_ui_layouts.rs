//! Declarative layouts of the v0.1 built-ins (device-ui, CONTRACTS.md §12.4.2): every
//! built-in ships a layout, and the MockTransport's v0.1 descriptors are generated from the
//! Rust ones (`ui/src/transport/mock/devices/v01Layouts.json`), so the shared renderer shows
//! the same panels on the mock and on the real engine. Regenerate with
//! `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test device_ui_layouts`.

use std::collections::BTreeMap;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::layout::Widget;
use ether_core::protocol::model::BuiltinDeviceType;

/// The v0.1 built-ins whose mock descriptors live in `v01Layouts.json` (the EQ has its own
/// generated `eq.json`).
const V01: &[BuiltinDeviceType] = &[
    BuiltinDeviceType::Synth,
    BuiltinDeviceType::Sampler,
    BuiltinDeviceType::Compressor,
    BuiltinDeviceType::Delay,
    BuiltinDeviceType::Reverb,
    BuiltinDeviceType::Limiter,
    BuiltinDeviceType::Utility,
    BuiltinDeviceType::DrumRack,
];

fn mock_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/src/transport/mock/devices/v01Layouts.json")
}

#[test]
fn v01_mock_descriptors_match_rust() {
    let map: BTreeMap<String, DeviceDescriptor> = V01
        .iter()
        .map(|t| (format!("{t:?}"), ether_devices::descriptor(*t)))
        .collect();
    let expected = serde_json::to_value(map).unwrap();
    let path = mock_path();
    if std::env::var_os("UPDATE_MOCK_DESCRIPTORS").is_some() {
        let text = format!("{}\n", serde_json::to_string_pretty(&expected).unwrap());
        std::fs::write(&path, text).unwrap();
        return;
    }
    let found = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (run with UPDATE_MOCK_DESCRIPTORS=1)",
            path.display()
        )
    });
    let found: serde_json::Value = serde_json::from_str(&found).unwrap();
    assert_eq!(
        found,
        expected,
        "{} is stale: run `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test device_ui_layouts`",
        path.display()
    );
}

#[test]
fn every_v01_builtin_has_a_layout() {
    for t in V01.iter().copied().chain([BuiltinDeviceType::Eq]) {
        let d = ether_devices::descriptor(t);
        assert!(d.layout.is_some(), "{t:?} has no layout");
    }
}

/// Every visible param of a v0.1 device is on its panel (nothing hidden behind "More").
#[test]
fn v01_layouts_show_every_visible_param() {
    for &t in V01 {
        let d = ether_devices::descriptor(t);
        let layout = d.layout.as_ref().unwrap();
        let mut shown = Vec::new();
        for s in &layout.sections {
            for it in &s.items {
                shown.extend(bound(&it.widget));
            }
        }
        for p in d.params.iter().filter(|p| !p.hidden) {
            assert!(
                shown.contains(&p.id.0),
                "{t:?}: param {} ({:?}) is not in the layout",
                p.name,
                p.id
            );
        }
    }
}

fn bound(w: &Widget) -> Vec<u32> {
    use Widget as W;
    match w {
        W::Knob { param }
        | W::Slider { param, .. }
        | W::Toggle { param }
        | W::Choice { param }
        | W::Number { param } => vec![param.0],
        W::Envelope {
            attack,
            decay,
            sustain,
            release,
            delay,
            hold,
        } => [
            Some(*attack),
            Some(*decay),
            Some(*sustain),
            Some(*release),
            *delay,
            *hold,
        ]
        .into_iter()
        .flatten()
        .map(|p| p.0)
        .collect(),
        W::FilterCurve {
            cutoff,
            resonance,
            mode,
            drive,
            gain,
        } => [Some(*cutoff), Some(*resonance), *mode, *drive, *gain]
            .into_iter()
            .flatten()
            .map(|p| p.0)
            .collect(),
        W::Oscillator { shape, position } => [Some(*shape), *position]
            .into_iter()
            .flatten()
            .map(|p| p.0)
            .collect(),
        W::SampleWaveform { start, end } => {
            [*start, *end].into_iter().flatten().map(|p| p.0).collect()
        }
        W::XyPad { x, y } => vec![x.0, y.0],
        W::TransferCurve { drive, curve, bias } => [Some(*drive), *curve, *bias]
            .into_iter()
            .flatten()
            .map(|p| p.0)
            .collect(),
        W::Lfo {
            shape,
            rate,
            amount,
        } => [Some(*shape), Some(*rate), *amount]
            .into_iter()
            .flatten()
            .map(|p| p.0)
            .collect(),
        W::StepEditor { first, count } => (0..u32::from(*count)).map(|i| first.0 + i).collect(),
        W::Crossover { frequencies } => frequencies.iter().map(|p| p.0).collect(),
        _ => vec![],
    }
}

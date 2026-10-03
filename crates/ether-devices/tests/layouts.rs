//! Shared validator of declarative layouts (contracts-3, CONTRACTS.md §12.4.2): every
//! built-in device and modulator layout must reference existing params, fit its grid and use
//! unique section ids, so the device nodes can't drift from the renderer's assumptions.

use std::collections::BTreeSet;

use ether_core::protocol::devices::ParamInfo;
use ether_core::protocol::layout::{DeviceLayout, Widget};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};

/// Params a widget binds (all must exist).
fn bound(w: &Widget) -> Vec<ParamId> {
    use Widget as W;
    match w {
        W::Knob { param }
        | W::Slider { param, .. }
        | W::Toggle { param }
        | W::Choice { param }
        | W::Number { param } => vec![*param],
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
            .collect(),
        W::TransferCurve { drive, curve, bias } => [Some(*drive), *curve, *bias]
            .into_iter()
            .flatten()
            .collect(),
        W::Oscillator { shape, position } => {
            [Some(*shape), *position].into_iter().flatten().collect()
        }
        W::Lfo {
            shape,
            rate,
            amount,
        } => [Some(*shape), Some(*rate), *amount]
            .into_iter()
            .flatten()
            .collect(),
        W::StepEditor { first, count } => (0..u32::from(*count))
            .map(|i| ParamId(first.0 + i))
            .collect(),
        W::XyPad { x, y } => vec![*x, *y],
        W::Crossover { frequencies } => frequencies.clone(),
        W::SampleWaveform { start, end } => [*start, *end].into_iter().flatten().collect(),
        W::EqCurve {
            bands, crossovers, ..
        } => bands
            .iter()
            .flat_map(|b| [b.on, b.kind, Some(b.freq), b.gain, b.q])
            .flatten()
            .chain(crossovers.iter().copied())
            .collect(),
        W::ZoneMap
        | W::HardwareRouting
        | W::Spectrum
        | W::Tuner
        | W::Meter { .. }
        | W::RackChains
        | W::Macros => {
            vec![]
        }
    }
}

fn check(name: &str, layout: &DeviceLayout, params: &[ParamInfo]) {
    let mut ids = BTreeSet::new();
    assert!(!layout.sections.is_empty(), "{name}: empty layout");
    for s in &layout.sections {
        assert!(
            ids.insert(s.id.clone()),
            "{name}: duplicate section id {}",
            s.id
        );
        assert!((1..=4).contains(&s.span), "{name}/{}: span 1..=4", s.id);
        assert!(
            (1..=8).contains(&s.columns),
            "{name}/{}: columns 1..=8",
            s.id
        );
        for item in &s.items {
            assert!(
                (1..=s.columns).contains(&item.colspan),
                "{name}/{}: colspan {} > columns {}",
                s.id,
                item.colspan,
                s.columns
            );
            for p in bound(&item.widget) {
                assert!(
                    params.iter().any(|i| i.id == p),
                    "{name}/{}: unknown param {p:?} in {:?}",
                    s.id,
                    item.widget
                );
            }
            if let Widget::EqCurve { bands, .. } = &item.widget {
                for b in bands {
                    assert!(!b.shapes.is_empty(), "{name}: band without shapes");
                    if let Some(kind) = b.kind {
                        let labels = params
                            .iter()
                            .find(|i| i.id == kind)
                            .and_then(|i| i.labels.as_ref())
                            .unwrap_or_else(|| panic!("{name}: band kind {kind:?} is not an enum"));
                        assert_eq!(
                            labels.len(),
                            b.shapes.len(),
                            "{name}: one shape per kind label"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn every_builtin_layout_is_valid() {
    let mut with_layout = 0;
    for t in BuiltinDeviceType::ALL {
        let d = ether_devices::descriptor(t);
        if let Some(l) = &d.layout {
            check(&format!("{t:?}"), l, &d.params);
            with_layout += 1;
        }
    }
    // The EQ ships one (graphical-eq); every v0.2 device node adds its own.
    assert!(with_layout >= 1);
}

#[test]
fn every_modulator_layout_is_valid() {
    for m in ether_devices::modulators::all() {
        let l = m.layout.as_ref().expect("modulators ship a layout");
        check(&m.name, l, &m.params);
    }
}

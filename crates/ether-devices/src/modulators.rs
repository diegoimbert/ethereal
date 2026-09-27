//! Modulator param tables (v0.2, owned by the `racks-modulation` node; model
//! `ether_model::modulation`). The engine runs modulators (`ether_core::modulation`); this
//! module only describes them (`Modulation::ListModulatorKinds`). **Param ids are stable and
//! append-only.**
//!
//! | kind | id | name | range |
//! |------|----|------|-------|
//! | Lfo | 0 | `Shape` | Sine / Triangle / Saw Up / Saw Down / Square |
//! | Lfo | 1 | `Rate` | 0.01 ..= 40 Hz (log), 1 |
//! | Lfo | 2 | `Sync` | toggle, off |
//! | Lfo | 3 | `Sync Rate` | `SYNC_RATES`, 1/4 |
//! | Lfo | 4 | `Phase` | 0 ..= 360°, 0 |
//! | Lfo | 5 | `Retrigger` | Free / Note / Transport |
//! | Lfo | 6 | `Fade In` | 0 ..= 5000 ms, 0 |
//! | Envelope | 0..=3 | `Attack` `Decay` `Sustain` `Release` | ms / ms / % / ms |
//! | Envelope | 4 | `Velocity` | 0 ..= 100 %, 0 |
//! | EnvelopeFollower | 0 | `Attack` | 0.1 ..= 500 ms (log), 10 |
//! | EnvelopeFollower | 1 | `Release` | 1 ..= 2000 ms (log), 150 |
//! | EnvelopeFollower | 2 | `Gain` | -24 ..= 24 dB, 0 |
//! | Steps | 0 | `Steps` | 1 ..= 16, 8 |
//! | Steps | 1 | `Rate` | `SYNC_RATES`, 1/16 |
//! | Steps | 2 | `Smooth` | 0 ..= 100 %, 0 |
//! | Steps | 3..=18 | `Step 1`..`Step 16` | -1 ..= 1, 0 |
//! | Random | 0 | `Rate` | 0.1 ..= 40 Hz (log), 4 |
//! | Random | 1 | `Sync` | toggle, off |
//! | Random | 2 | `Sync Rate` | `SYNC_RATES`, 1/8 |
//! | Random | 3 | `Smooth` | 0 ..= 100 %, 0 |

use ether_core::protocol::devices::{ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::ModulatorKind;
use ether_core::protocol::racks::ModulatorDescriptor;

use crate::contract::{SYNC_EIGHTH, SYNC_RATES, SYNC_SIXTEENTH, choice, param, stepped, toggle};

fn params(kind: ModulatorKind) -> Vec<ParamInfo> {
    let ms = |id, name, (min, max, def): (f64, f64, f64)| {
        param(
            id,
            name,
            "Envelope",
            ParamUnit::Milliseconds,
            (min, max, def),
            ParamScale::Power { exponent: 3.0 },
        )
    };
    match kind {
        ModulatorKind::Lfo => vec![
            choice(
                0,
                "Shape",
                "LFO",
                &["Sine", "Triangle", "Saw Up", "Saw Down", "Square"],
                0,
            ),
            param(
                1,
                "Rate",
                "LFO",
                ParamUnit::Hertz,
                (0.01, 40.0, 1.0),
                ParamScale::Log,
            ),
            toggle(2, "Sync", "LFO", false),
            choice(3, "Sync Rate", "LFO", &SYNC_RATES, 8),
            param(
                4,
                "Phase",
                "LFO",
                ParamUnit::None,
                (0.0, 360.0, 0.0),
                ParamScale::Linear,
            ),
            choice(5, "Retrigger", "LFO", &["Free", "Note", "Transport"], 0),
            param(
                6,
                "Fade In",
                "LFO",
                ParamUnit::Milliseconds,
                (0.0, 5000.0, 0.0),
                ParamScale::Power { exponent: 2.0 },
            ),
        ],
        ModulatorKind::Envelope => vec![
            ms(0, "Attack", (0.0, 10000.0, 5.0)),
            ms(1, "Decay", (1.0, 10000.0, 300.0)),
            param(
                2,
                "Sustain",
                "Envelope",
                ParamUnit::Percent,
                (0.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
            ms(3, "Release", (1.0, 20000.0, 300.0)),
            param(
                4,
                "Velocity",
                "Envelope",
                ParamUnit::Percent,
                (0.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
        ],
        ModulatorKind::EnvelopeFollower => vec![
            param(
                0,
                "Attack",
                "Follower",
                ParamUnit::Milliseconds,
                (0.1, 500.0, 10.0),
                ParamScale::Log,
            ),
            param(
                1,
                "Release",
                "Follower",
                ParamUnit::Milliseconds,
                (1.0, 2000.0, 150.0),
                ParamScale::Log,
            ),
            param(
                2,
                "Gain",
                "Follower",
                ParamUnit::Decibels,
                (-24.0, 24.0, 0.0),
                ParamScale::Linear,
            ),
        ],
        ModulatorKind::Steps => {
            let mut v = vec![
                stepped(0, "Steps", "Steps", ParamUnit::None, 1, 16, 8),
                choice(1, "Rate", "Steps", &SYNC_RATES, SYNC_SIXTEENTH),
                param(
                    2,
                    "Smooth",
                    "Steps",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
            ];
            let names: Vec<String> = (1..=16).map(|i| format!("Step {i}")).collect();
            for (i, name) in names.iter().enumerate() {
                v.push(param(
                    3 + i as u32,
                    name,
                    "Steps",
                    ParamUnit::None,
                    (-1.0, 1.0, 0.0),
                    ParamScale::Linear,
                ));
            }
            v
        }
        ModulatorKind::Random => vec![
            param(
                0,
                "Rate",
                "Random",
                ParamUnit::Hertz,
                (0.1, 40.0, 4.0),
                ParamScale::Log,
            ),
            toggle(1, "Sync", "Random", false),
            choice(2, "Sync Rate", "Random", &SYNC_RATES, SYNC_EIGHTH),
            param(
                3,
                "Smooth",
                "Random",
                ParamUnit::Percent,
                (0.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
        ],
    }
}

/// Descriptor of a modulator kind.
pub fn descriptor(kind: ModulatorKind) -> ModulatorDescriptor {
    let name = match kind {
        ModulatorKind::Lfo => "LFO",
        ModulatorKind::Envelope => "Envelope",
        ModulatorKind::EnvelopeFollower => "Envelope Follower",
        ModulatorKind::Steps => "Steps",
        ModulatorKind::Random => "Random",
    };
    let params = params(kind);
    ModulatorDescriptor {
        kind,
        name: name.to_owned(),
        bipolar: kind.bipolar(),
        layout: Some(layout(kind, &params)),
        params,
    }
}

/// Declarative panels: a knob row, typed widgets where they exist.
fn layout(kind: ModulatorKind, params: &[ParamInfo]) -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{Widget, WidgetSize};
    use ether_core::protocol::model::ParamId;
    let p = ParamId;
    let items = match kind {
        ModulatorKind::Lfo => vec![
            item(
                Widget::Lfo {
                    shape: p(0),
                    rate: p(1),
                    amount: None,
                },
                WidgetSize::Medium,
            ),
            item(Widget::Toggle { param: p(2) }, WidgetSize::Small),
            item(Widget::Choice { param: p(3) }, WidgetSize::Small),
            knob(p(4), WidgetSize::Small),
            item(Widget::Choice { param: p(5) }, WidgetSize::Small),
            knob(p(6), WidgetSize::Small),
        ],
        ModulatorKind::Envelope => vec![
            item(
                Widget::Envelope {
                    attack: p(0),
                    decay: p(1),
                    sustain: p(2),
                    release: p(3),
                    delay: None,
                    hold: None,
                },
                WidgetSize::Medium,
            ),
            knob(p(4), WidgetSize::Small),
        ],
        ModulatorKind::Steps => {
            let mut steps = item(
                Widget::StepEditor {
                    first: p(3),
                    count: 16,
                },
                WidgetSize::Large,
            );
            steps.colspan = 4;
            vec![
                steps,
                item(Widget::Number { param: p(0) }, WidgetSize::Small),
                item(Widget::Choice { param: p(1) }, WidgetSize::Small),
                knob(p(2), WidgetSize::Small),
            ]
        }
        ModulatorKind::EnvelopeFollower | ModulatorKind::Random => params
            .iter()
            .map(|i| match &i.labels {
                Some(_) => item(Widget::Choice { param: i.id }, WidgetSize::Small),
                None => knob(i.id, WidgetSize::Small),
            })
            .collect(),
    };
    layout(vec![section("main", None, 1, 4, items)])
}

/// Every kind, in `ModulatorKind::ALL` order.
pub fn all() -> Vec<ModulatorDescriptor> {
    ModulatorKind::ALL.into_iter().map(descriptor).collect()
}

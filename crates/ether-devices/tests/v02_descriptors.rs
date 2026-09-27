//! v0.2 device contracts (contracts-3): descriptor ↔ mock parity, placeholder behaviour, RT
//! safety and factory presets. Device nodes keep these passing (update the mock JSON with
//! `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v02_descriptors`, never
//! by hand) and add their own behaviour tests next to them.

use std::collections::BTreeMap;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, PresetDevice, load_preset};
use ether_core::{
    AudioBuffers, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Mock descriptor file per group (`ui/src/transport/mock/devices/<file>.json`), owned by
/// the group's node.
const GROUPS: &[(&str, &[BuiltinDeviceType])] = &[
    ("polySynth", &[BuiltinDeviceType::PolySynth]),
    ("multisampler", &[BuiltinDeviceType::MultiSampler]),
    (
        "fxColor",
        &[
            BuiltinDeviceType::Saturator,
            BuiltinDeviceType::Bitcrusher,
            BuiltinDeviceType::AutoFilter,
        ],
    ),
    (
        "fxModulation",
        &[
            BuiltinDeviceType::Chorus,
            BuiltinDeviceType::Phaser,
            BuiltinDeviceType::Flanger,
            BuiltinDeviceType::Tremolo,
        ],
    ),
    (
        "fxDynamics",
        &[
            BuiltinDeviceType::Gate,
            BuiltinDeviceType::MultibandCompressor,
            BuiltinDeviceType::TransientShaper,
        ],
    ),
    (
        "fxAnalysis",
        &[
            BuiltinDeviceType::SpectrumAnalyzer,
            BuiltinDeviceType::Tuner,
        ],
    ),
    (
        "midiFx",
        &[
            BuiltinDeviceType::Arpeggiator,
            BuiltinDeviceType::Chord,
            BuiltinDeviceType::ScaleQuantize,
            BuiltinDeviceType::NoteLength,
            BuiltinDeviceType::Velocity,
            BuiltinDeviceType::Randomizer,
        ],
    ),
    (
        "racks",
        &[
            BuiltinDeviceType::InstrumentRack,
            BuiltinDeviceType::AudioEffectRack,
            BuiltinDeviceType::MidiEffectRack,
        ],
    ),
];

fn mock_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/transport/mock/devices")
}

fn check_file(name: &str, expected: &serde_json::Value) {
    let path = mock_dir().join(format!("{name}.json"));
    let text = format!("{}\n", serde_json::to_string_pretty(expected).unwrap());
    if std::env::var_os("UPDATE_MOCK_DESCRIPTORS").is_some() {
        std::fs::create_dir_all(mock_dir()).unwrap();
        std::fs::write(&path, &text).unwrap();
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
        &found,
        expected,
        "{} is stale: run `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v02_descriptors`",
        path.display()
    );
}

#[test]
fn mock_descriptors_match_rust() {
    let mut covered = 0;
    for (file, types) in GROUPS {
        let map: BTreeMap<String, DeviceDescriptor> = types
            .iter()
            .map(|t| (format!("{t:?}"), ether_devices::descriptor(*t)))
            .collect();
        covered += types.len();
        check_file(file, &serde_json::to_value(map).unwrap());
    }
    // Every v0.2 type belongs to exactly one group (the 9 earlier ones are in
    // `builtinDevices.ts`).
    assert_eq!(covered, BuiltinDeviceType::ALL.len() - 9);
    check_file(
        "modulators",
        &serde_json::to_value(ether_devices::modulators::all()).unwrap(),
    );
}

fn prepared(node: &mut dyn Node) {
    node.prepare(&PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: 256,
        max_events_per_block: 64,
    });
}

#[test]
fn placeholders_follow_their_category_without_allocating() {
    for (_, types) in GROUPS {
        for &t in *types {
            let desc = ether_devices::descriptor(t);
            let mut node = ether_devices::create(&BuiltinDevice::new(t), &NoSamples);
            prepared(node.as_mut());
            let (n_in, n_out) = node.channels();
            let input = [vec![0.5f32; 256], vec![-0.25f32; 256]];
            let mut out = [vec![0.0f32; 256], vec![0.0f32; 256]];
            let mut out_events = EventBuffer::with_capacity(64);
            let events = [
                ProcessEvent {
                    offset: 0,
                    kind: EventKind::Param {
                        param: desc.params[0].id,
                        value: desc.params[0].max,
                    },
                },
                ProcessEvent {
                    offset: 3,
                    kind: EventKind::NoteOn {
                        note_id: 1,
                        channel: 0,
                        key: 60,
                        velocity: 0.8,
                    },
                },
            ];
            let transport = TransportInfo {
                playing: true,
                ..TransportInfo::STOPPED
            };
            {
                let ins: [&[f32]; 2] = [&input[0], &input[1]];
                let (l, r) = out.split_at_mut(1);
                let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
                let mut ctx = ProcessContext {
                    sample_rate: 48_000.0,
                    frames: 256,
                    transport: &transport,
                    events: &events,
                    out_events: &mut out_events,
                };
                let mut buffers = AudioBuffers {
                    inputs: &ins[..n_in as usize],
                    outputs: &mut outs[..n_out as usize],
                };
                assert_no_alloc(|| {
                    node.process(&mut ctx, &mut buffers);
                });
            }
            assert_eq!(
                node.param(desc.params[0].id),
                Some(desc.params[0].max),
                "{t:?}"
            );
            match desc.category {
                DeviceCategory::NoteEffect => {
                    assert_eq!(
                        (n_in, n_out),
                        (0, 0),
                        "{t:?}: MIDI effects leave audio alone"
                    );
                    assert_eq!((desc.audio_inputs, desc.audio_outputs), (0, 0), "{t:?}");
                    // MIDI-thru: the note goes on, the param does not.
                    assert_eq!(out_events.as_slice(), &events[1..], "{t:?}");
                }
                DeviceCategory::AudioEffect => {
                    assert_eq!(out[0][10], 0.5, "{t:?}: pass-through");
                    assert!(out_events.is_empty());
                }
                DeviceCategory::Instrument => {
                    assert!(desc.midi_input, "{t:?}");
                }
            }
        }
    }
}

#[test]
fn factory_presets_parse_and_match_their_type() {
    for t in BuiltinDeviceType::ALL {
        for p in ether_devices::factory_presets(t) {
            let preset = load_preset(p.json).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert_eq!(
                preset.device,
                PresetDevice::Builtin { device: t },
                "{}",
                p.id
            );
            assert!(
                p.id.starts_with(&format!("{}/", t.key())),
                "{}: id must be \"{}/<slug>\"",
                p.id,
                t.key()
            );
            let desc = ether_devices::descriptor(t);
            for id in preset.params.keys() {
                assert!(
                    desc.params.iter().any(|q| q.id == *id),
                    "{}: unknown param {id:?}",
                    p.id
                );
            }
        }
    }
}

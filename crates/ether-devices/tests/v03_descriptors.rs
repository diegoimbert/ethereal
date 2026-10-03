//! v0.3 device contracts (contracts-4): descriptor ↔ mock parity, placeholder behaviour and
//! RT safety of the convolution reverb (`fx-space`) and the external devices
//! (`external-instrument`). Device nodes keep these passing (update the mock JSON with
//! `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v03_descriptors`, never by
//! hand) and add their own behaviour tests (`tests/fx_space*.rs`, `tests/external*.rs`).

use std::collections::BTreeMap;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{AudioBuffers, EventBuffer, EventKind, PrepareConfig, ProcessContext};
use ether_core::{ProcessEvent, TransportInfo};
use ether_devices::NoSamples;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// v0.3 device types whose real DSP has landed (the placeholder category checks are
/// skipped for them). APPEND-ONLY: each device node adds one line per type it implements.
const IMPLEMENTED: &[BuiltinDeviceType] = &[
    // (device nodes append here)
];

/// Mock descriptor file per group (`ui/src/transport/mock/devices/<file>.json`), owned by
/// the group's node.
const GROUPS: &[(&str, &[BuiltinDeviceType])] = &[
    ("fxSpace", &[BuiltinDeviceType::ConvolutionReverb]),
    (
        "external",
        &[
            BuiltinDeviceType::ExternalInstrument,
            BuiltinDeviceType::ExternalAudioEffect,
        ],
    ),
];

fn check_file(name: &str, expected: &serde_json::Value) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/src/transport/mock/devices");
    let path = dir.join(format!("{name}.json"));
    let text = format!("{}\n", serde_json::to_string_pretty(expected).unwrap());
    if std::env::var_os("UPDATE_MOCK_DESCRIPTORS").is_some() {
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
        "{} is stale: run `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v03_descriptors`",
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
    // The v0.3 types are the last three of `BuiltinDeviceType::ALL`.
    assert_eq!(covered, 3);
    let last: Vec<_> = BuiltinDeviceType::ALL[BuiltinDeviceType::ALL.len() - 3..].to_vec();
    assert_eq!(
        last,
        vec![
            BuiltinDeviceType::ConvolutionReverb,
            BuiltinDeviceType::ExternalInstrument,
            BuiltinDeviceType::ExternalAudioEffect,
        ]
    );
}

#[test]
fn param_tables_are_dense_and_frozen() {
    use ether_devices::{external, fx_space};
    let counts = [
        (
            BuiltinDeviceType::ConvolutionReverb,
            fx_space::convolution_reverb::COUNT,
        ),
        (
            BuiltinDeviceType::ExternalInstrument,
            external::external_instrument::COUNT,
        ),
        (
            BuiltinDeviceType::ExternalAudioEffect,
            external::external_audio_effect::COUNT,
        ),
    ];
    for (t, count) in counts {
        let d = ether_devices::descriptor(t);
        assert_eq!(d.params.len(), count, "{t:?}");
        for (i, p) in d.params.iter().enumerate() {
            assert_eq!(p.id.0 as usize, i, "{t:?}: dense ids");
        }
    }
    // Factory IR ids are frozen (append-only).
    let ids: Vec<&str> = fx_space::FACTORY_IRS.iter().map(|s| s.id).collect();
    assert_eq!(&ids[..4], &["room", "chamber", "plate", "hall"]);
    assert_eq!(fx_space::factory_irs().len(), fx_space::FACTORY_IRS.len());
}

#[test]
fn placeholders_follow_their_category_without_allocating() {
    for (_, types) in GROUPS {
        for &t in *types {
            let desc = ether_devices::descriptor(t);
            let mut node = ether_devices::create(&BuiltinDevice::new(t), &NoSamples);
            node.prepare(&PrepareConfig {
                sample_rate: 48_000.0,
                max_block_size: 256,
                max_events_per_block: 64,
            });
            let (n_in, n_out) = node.channels();
            let input = [vec![0.5f32; 256], vec![-0.25f32; 256]];
            let mut out = [vec![0.0f32; 256], vec![0.0f32; 256]];
            let mut out_events = EventBuffer::with_capacity(64);
            let events = [ProcessEvent {
                offset: 0,
                kind: EventKind::Param {
                    param: desc.params[0].id,
                    value: desc.params[0].max,
                },
            }];
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
            if IMPLEMENTED.contains(&t) {
                continue;
            }
            match desc.category {
                DeviceCategory::AudioEffect => assert_eq!(out[0][10], 0.5, "{t:?}: pass-through"),
                DeviceCategory::Instrument => {
                    assert!(desc.midi_input, "{t:?}");
                    assert!(out[0].iter().all(|s| *s == 0.0), "{t:?}: silent");
                }
                DeviceCategory::NoteEffect => unreachable!("{t:?}"),
            }
        }
    }
}

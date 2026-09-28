//! Rack device nodes (`racks-modulation`): audio through for instrument/audio effect
//! racks, MIDI-thru (never params) for MIDI effect racks, macro/selector values clamped and
//! read back, no allocation; layouts carry the macro bank, the chain list and the selector.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::layout::Widget;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const RACKS: [BuiltinDeviceType; 3] = [
    BuiltinDeviceType::InstrumentRack,
    BuiltinDeviceType::AudioEffectRack,
    BuiltinDeviceType::MidiEffectRack,
];

#[test]
fn rack_nodes_pass_through_and_keep_macro_values() {
    for ty in RACKS {
        let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
        d.prepare(&PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 64,
            max_events_per_block: 16,
        });
        let input = [vec![0.25f32; 64], vec![-0.5f32; 64]];
        let mut out = [vec![0.0f32; 64], vec![0.0f32; 64]];
        let mut out_events = EventBuffer::with_capacity(16);
        let events = [
            ProcessEvent {
                offset: 0,
                kind: EventKind::Param {
                    param: ParamId(3),
                    value: 2.0,
                },
            },
            ProcessEvent {
                offset: 1,
                kind: EventKind::Param {
                    param: ParamId(8),
                    value: 41.6,
                },
            },
            ProcessEvent {
                offset: 2,
                kind: EventKind::NoteOn {
                    note_id: 1,
                    channel: 0,
                    key: 60,
                    velocity: 1.0,
                },
            },
        ];
        let transport = TransportInfo::STOPPED;
        let (n_in, n_out) = d.channels();
        {
            let ins: [&[f32]; 2] = [&input[0], &input[1]];
            let (l, r) = out.split_at_mut(1);
            let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
            let mut ctx = ProcessContext {
                sample_rate: 48_000.0,
                frames: 64,
                transport: &transport,
                events: &events,
                out_events: &mut out_events,
            };
            let mut buffers = AudioBuffers {
                inputs: &ins[..n_in as usize],
                outputs: &mut outs[..n_out as usize],
            };
            assert_no_alloc(|| {
                d.process(&mut ctx, &mut buffers);
            });
        }
        assert_eq!(
            d.param(ParamId(3)),
            Some(1.0),
            "{ty:?}: macros clamp to 0..=1"
        );
        assert_eq!(d.param(ParamId(8)), Some(42.0), "{ty:?}: selector snaps");
        if ty == BuiltinDeviceType::MidiEffectRack {
            assert_eq!((n_in, n_out), (0, 0));
            assert_eq!(out_events.as_slice(), &events[2..]);
        } else {
            assert_eq!((n_in, n_out), (2, 2));
            assert_eq!(out[0][10], 0.25);
            assert_eq!(out[1][10], -0.5);
            assert!(out_events.is_empty());
        }
        let desc = d.descriptor();
        assert_eq!(desc.device_type, ether_devices::descriptor(ty).device_type);
        let layout = desc.layout.expect("racks ship a layout");
        let widgets: Vec<&Widget> = layout
            .sections
            .iter()
            .flat_map(|s| s.items.iter().map(|i| &i.widget))
            .collect();
        assert!(widgets.contains(&&Widget::Macros));
        assert!(widgets.contains(&&Widget::RackChains));
        assert!(widgets.contains(&&Widget::Knob { param: ParamId(8) }));
    }
}

#[test]
fn every_rack_type_has_presets() {
    for ty in RACKS {
        assert!(!ether_devices::factory_presets(ty).is_empty(), "{ty:?}");
    }
}

//! Stuck notes on the worklet path: the built-in synth, switched off while its note
//! sounds and back on after the note ended, releases the voice (the `EngineHost` render
//! loop over the shared `ether-core` engine).

use ether_controller::EngineBridge;
use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, TrackDesc};
use ether_core::protocol::model::{BuiltinDevice, ClipId, DeviceId, TrackId, TrackKind, Ulid};
use ether_core::{NodeKey, RenderGraphDesc, TransportControl};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::ring::HeapMemory;
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};

const SR: usize = 48_000;

fn graph(version: u64, synth: NodeKey, enabled: bool) -> RenderGraphDesc {
    let master = TrackId(Ulid(1));
    let midi = TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: TrackId(Ulid(2)),
        kind: TrackKind::Midi,
        chain: vec![ChainEntry {
            node: synth,
            enabled,
            sidechain: None,
        }],
        output: Some(master),
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        midi_input: Default::default(),
        monitor: false,
        armed: false,
        // One note, beats 0..2 (0..1 s at 120 BPM).
        clips: vec![ClipDesc {
            id: ClipId(Ulid(3)),
            start: 0.0,
            length: 16.0,
            offset: 0.0,
            looping: None,
            muted: false,
            content: ClipContentDesc::Midi {
                notes: vec![NoteDesc {
                    start: 0.0,
                    duration: 2.0,
                    key: 60,
                    velocity: 1.0,
                    release_velocity: 0.5,
                }],
            },
            envelopes: vec![],
        }],
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
        midi_sidechains: Vec::new(),
    };
    let mut master_desc = midi.clone();
    master_desc.id = master;
    master_desc.kind = TrackKind::Master;
    master_desc.chain.clear();
    master_desc.clips.clear();
    master_desc.output = None;
    RenderGraphDesc {
        version,
        tracks: vec![master_desc, midi],
        ..Default::default()
    }
}

/// Renders `seconds` and returns the output peak over the last 100 ms.
fn render(engine: &mut EngineHost<HeapMemory>, seconds: f64) -> f32 {
    let quanta = (seconds * SR as f64) as usize / RENDER_QUANTUM;
    let tail = SR / 10 / RENDER_QUANTUM;
    let mut peak = 0.0f32;
    for q in 0..quanta {
        engine.render(RENDER_QUANTUM);
        if q + tail >= quanta {
            peak = peak.max(engine.output(0).iter().fold(0.0, |m, s| m.max(s.abs())));
        }
    }
    peak
}

#[test]
fn a_synth_switched_off_mid_note_releases_it_when_back_on() {
    let control = HeapMemory::new(1 << 16);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut bridge = WebBridge::new(shared.clone());
    let mut engine = EngineHost::new(SR as u32, control, reports);
    let synth = bridge
        .create_builtin(DeviceId(Ulid(5)), &BuiltinDevice::Synth, &[])
        .unwrap();
    bridge.publish(graph(1, synth, true)).unwrap();
    bridge.transport(TransportControl::Play).unwrap();
    assert!(render(&mut engine, 0.5) > 0.01, "the note plays");
    // Off during the note; the note ends (1 s) while the synth is off.
    bridge.publish(graph(2, synth, false)).unwrap();
    assert!(render(&mut engine, 1.0) < 1e-6, "off: silent");
    // Back on: the voice that missed its note-off is released and decays to silence.
    bridge.publish(graph(3, synth, true)).unwrap();
    let peak = render(&mut engine, 3.0);
    assert!(peak < 1e-4, "back on: released, peak {peak}");
    assert!(
        shared.borrow().errors.is_empty(),
        "{:?}",
        shared.borrow().errors
    );
}

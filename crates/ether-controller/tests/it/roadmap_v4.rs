//! v0.3 routing: the new commands reach their modules (or reply `Unsupported`
//! without changing the document when the host can't serve them), new built-ins insert and
//! compile, new desc fields compile empty and the cascades of the new entities are done.

use crate::common;

use common::*;
use ether_core::protocol::audio_to_midi::{
    AudioToMidiCommand, AudioToMidiMode, AudioToMidiOptions,
};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::external::ExternalCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ReplyValue};

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn insert(h: &mut Harness, track: TrackId, device: BuiltinDevice) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin { device },
        before: None,
    }));
    id
}

/// A MIDI track with one clip holding one note.
fn midi_clip(h: &mut Harness) -> (TrackId, ClipId, NoteId) {
    let t = track(h, TrackKind::Midi);
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track: t,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
    let note: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: vec![NoteSpec {
            id: note,
            pitch: 60,
            velocity: 0.8,
            start: Beats(0.0),
            duration: Beats(1.0),
        }],
    }));
    (t, clip, note)
}

// ─── engine + editing ───────────────────────────────────────────────────────────────────

#[test]
fn audio_streaming_policy_streams_long_media() {
    let mut h = Harness::with_project();
    // Ten minutes of stereo: long enough to stream (the pipeline tests are in
    // tests/media_stream.rs).
    let media = MediaRef {
        id: h.id(),
        name: "long.wav".into(),
        file: "media/long.wav".into(),
        sample_rate: 48_000,
        channels: 2,
        frames: 48_000 * 600,
        hash: None,
        location: MediaLocation::Project,
    };
    assert!(media.frames as f64 / 48_000.0 > ether_controller::media_stream::STREAM_MIN_SECONDS);
    assert!(ether_controller::media_stream::should_stream(
        &media, 48_000
    ));
    // Short media and media of unknown length are decoded whole.
    let short = MediaRef {
        frames: 48_000 * 10,
        ..media.clone()
    };
    assert!(!ether_controller::media_stream::should_stream(
        &short, 48_000
    ));
    let unknown = MediaRef { frames: 0, ..media };
    assert!(!ether_controller::media_stream::should_stream(
        &unknown, 48_000
    ));
}

/// Implemented (`audio-to-midi`; behaviour in `tests/audio_to_midi.rs`): a MIDI clip is
/// refused without touching the document, cancelling an unknown job is a no-op.
#[test]
fn audio_to_midi_is_implemented() {
    let mut h = Harness::with_project();
    let (_, clip, _) = midi_clip(&mut h);
    let (track, new_clip, seed_notes) = (h.id(), h.id(), h.id());
    let before = h.project().clone();
    let out = h.send(Command::AudioToMidi(AudioToMidiCommand::Start {
        job: "job-1".into(),
        clip,
        mode: AudioToMidiMode::Melody,
        options: AudioToMidiOptions::default(),
        track,
        new_clip,
        seed_notes,
        instrument: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project(), &before);
    h.ok(Command::AudioToMidi(AudioToMidiCommand::Cancel {
        job: "job-1".into(),
    }));
}

#[test]
fn fx_space_reverb_is_implemented() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let d = insert(
        &mut h,
        t,
        BuiltinDevice::new(BuiltinDeviceType::ConvolutionReverb),
    );
    assert_eq!(
        h.project().devices[&d].params.len(),
        ether_devices::fx_space::convolution_reverb::COUNT
    );
    h.tick();
    let g = h.ctl.bridge.last_graph();
    assert_eq!(g.tracks.iter().find(|x| x.id == t).unwrap().chain.len(), 1);
    h.ok(Command::Device(DeviceCommand::SetIr {
        device: d,
        ir: Some(IrSource::Factory { id: "hall".into() }),
    }));
    assert!(matches!(
        h.ok(Command::Device(DeviceCommand::ListFactoryIrs)),
        ReplyValue::FactoryIrs { .. }
    ));
}

/// `external-instrument` landed (behaviour tests in `tests/external.rs`): routing is a
/// document command compiled into `TrackDesc::hw_io`; a host without hardware I/O (the
/// fake bridge, like the web) replies `Unsupported` for ports and measurement.
#[test]
fn external_instrument_routes_and_hostless_ports_are_unsupported() {
    let mut h = Harness::with_project();
    let midi = track(&mut h, TrackKind::Midi);
    let audio = track(&mut h, TrackKind::Audio);
    let inst = insert(
        &mut h,
        midi,
        BuiltinDevice::new(BuiltinDeviceType::ExternalInstrument),
    );
    let fx = insert(
        &mut h,
        audio,
        BuiltinDevice::new(BuiltinDeviceType::ExternalAudioEffect),
    );
    h.ok(Command::External(ExternalCommand::SetRouting {
        device: inst,
        routing: ExternalRouting {
            midi_out: Some("port".into()),
            ..ExternalRouting::default()
        },
    }));
    h.tick();
    let g = h.ctl.bridge.last_graph();
    for t in [midi, audio] {
        let desc = g.tracks.iter().find(|x| x.id == t).unwrap();
        assert_eq!(desc.chain.len(), 1);
        assert_eq!(desc.hw_io.len(), 1);
        assert_eq!(desc.hw_io[0].node, desc.chain[0].node);
    }
    assert_unsupported(&mut h, Command::External(ExternalCommand::ListPorts));
    let fx_routing = ExternalRouting {
        audio_send: Some(HwChannels { first: 0, count: 2 }),
        audio_return: Some(HwChannels { first: 0, count: 2 }),
        ..ExternalRouting::default()
    };
    h.ok(Command::External(ExternalCommand::SetRouting {
        device: fx,
        routing: fx_routing,
    }));
    assert_unsupported(
        &mut h,
        Command::External(ExternalCommand::MeasureLatency { device: fx }),
    );
}

// ─── workflow ───────────────────────────────────────────────────────────────────────────

// undo-history: implemented (tests/undo_history.rs).

// templates: implemented (tests/templates.rs).

// Implemented (tests/versions.rs).

// keymap: implemented (tests/keymap.rs).

// Web-only (the worklet's latency report, `ether-wasm/src/latency.rs`); its
// prewire test is `crates/ether-wasm/tests/latency_prewire.rs`. The format is
// pinned in `ether-model/tests/roadmap_v4.rs` (`rack_presets_store_chains`). The UX follow-ups have no
// engine side.

//! v0.3 routing (contracts-4): every new command reaches its node's module and, until the
//! node implements it, replies `Unsupported` without changing the document; new built-ins
//! insert and compile as placeholders; new desc fields compile empty; the cascades of the
//! new entities are done.
//!
//! **One test per node**: each node deletes (or rewrites into real behaviour tests) only
//! its own function, so parallel nodes never conflict here. Keep `assert_unsupported`.

mod common;

use common::*;
use ether_controller::store::ProjectStore;
use ether_core::protocol::audio_to_midi::{
    AudioToMidiCommand, AudioToMidiMode, AudioToMidiOptions,
};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::external::ExternalCommand;
use ether_core::protocol::keymap::{Keymap, KeymapCommand};
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::templates::TemplateCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::undo_history::HistoryCommand;
use ether_core::protocol::{Command, ErrorCode};

/// Sends `c` and asserts it replies `Unsupported` without changing the document.
fn assert_unsupported(h: &mut Harness, c: Command) {
    let before = h.project().clone();
    let out = h.send(c.clone());
    assert_eq!(err(&out).code, ErrorCode::Unsupported, "{c:?}");
    assert!(patches(&out).is_empty(), "{c:?}");
    assert_eq!(h.project(), &before, "{c:?}");
}

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

#[allow(dead_code)] // midi-expression's tests moved to tests/expression.rs
fn point(time: f64, value: f32) -> ExpressionPoint {
    ExpressionPoint {
        time: Beats(time),
        value,
        curve: CurveShape::Linear,
    }
}

/// Store `p` (edited outside the controller) and reopen it.
#[allow(dead_code)] // midi-expression's tests moved to tests/expression.rs
fn reopen(h: &mut Harness, p: &Project) {
    // Save first: opening autosaves a dirty project over the stored file.
    h.ok(Command::Project(ProjectCommand::Save));
    let json = file::save(p, "0.3.0").unwrap();
    h.ctl.store.save(p.id, &json).unwrap();
    h.ok(Command::Project(ProjectCommand::Open { id: p.id }));
}

// ─── engine + editing ───────────────────────────────────────────────────────────────────

#[test]
fn audio_streaming_is_off_until_the_node_lands() {
    let mut h = Harness::with_project();
    // Ten minutes of stereo: long enough to stream, but the policy says no yet.
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
    assert!(!ether_controller::media_stream::should_stream(
        &media, 48_000
    ));
}

#[test]
fn mpe_replies_unsupported() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    assert_unsupported(
        &mut h,
        Command::Expression(ExpressionCommand::SetTrackMpe {
            track: t,
            mpe: Some(MpeSettings::default()),
        }),
    );
}

#[test]
fn audio_to_midi_replies_unsupported() {
    let mut h = Harness::with_project();
    let (_, clip, _) = midi_clip(&mut h);
    let (track, new_clip, seed_notes) = (h.id(), h.id(), h.id());
    assert_unsupported(
        &mut h,
        Command::AudioToMidi(AudioToMidiCommand::Start {
            job: "job-1".into(),
            clip,
            mode: AudioToMidiMode::Melody,
            options: AudioToMidiOptions::default(),
            track,
            new_clip,
            seed_notes,
            instrument: None,
        }),
    );
    assert_unsupported(
        &mut h,
        Command::AudioToMidi(AudioToMidiCommand::Cancel {
            job: "job-1".into(),
        }),
    );
}

#[test]
fn fx_space_reverb_is_a_placeholder() {
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
    assert_unsupported(
        &mut h,
        Command::Device(DeviceCommand::SetIr {
            device: d,
            ir: Some(IrSource::Factory { id: "hall".into() }),
        }),
    );
    assert_unsupported(&mut h, Command::Device(DeviceCommand::ListFactoryIrs));
}

#[test]
fn external_instrument_devices_are_placeholders() {
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
    h.tick();
    let g = h.ctl.bridge.last_graph();
    for t in [midi, audio] {
        let desc = g.tracks.iter().find(|x| x.id == t).unwrap();
        assert_eq!(desc.chain.len(), 1);
        assert!(desc.hw_io.is_empty(), "compiled once the node lands");
    }
    assert_unsupported(
        &mut h,
        Command::External(ExternalCommand::SetRouting {
            device: inst,
            routing: ExternalRouting {
                midi_out: Some("port".into()),
                ..ExternalRouting::default()
            },
        }),
    );
    assert_unsupported(&mut h, Command::External(ExternalCommand::ListPorts));
    assert_unsupported(
        &mut h,
        Command::External(ExternalCommand::MeasureLatency { device: fx }),
    );
}

// ─── workflow ───────────────────────────────────────────────────────────────────────────

#[test]
fn undo_history_replies_unsupported() {
    let mut h = Harness::with_project();
    track(&mut h, TrackKind::Audio);
    assert_unsupported(&mut h, Command::History(HistoryCommand::List));
    assert_unsupported(
        &mut h,
        Command::History(HistoryCommand::JumpTo { step: None }),
    );
    assert_unsupported(
        &mut h,
        Command::History(HistoryCommand::SetCheckpoint {
            step: 0,
            name: Some("Before mix".into()),
        }),
    );
}

#[test]
fn templates_reply_unsupported() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    assert_unsupported(
        &mut h,
        Command::Template(TemplateCommand::List { kind: None }),
    );
    assert_unsupported(
        &mut h,
        Command::Template(TemplateCommand::SaveTracks {
            tracks: vec![t],
            name: "Vocal".into(),
            meta: PresetMeta::default(),
            overwrite: false,
        }),
    );
    let seed = h.id();
    assert_unsupported(
        &mut h,
        Command::Template(TemplateCommand::Insert {
            template: "tracks/Vocal".into(),
            seed,
            parent: None,
            before: None,
        }),
    );
    let id = h.project_id();
    assert_unsupported(
        &mut h,
        Command::Template(TemplateCommand::NewProject {
            id,
            name: "Song".into(),
            template: None,
        }),
    );
}

// project-versions: implemented (tests/versions.rs).

#[test]
fn keymap_replies_unsupported() {
    let mut h = Harness::with_project();
    assert_unsupported(&mut h, Command::Keymap(KeymapCommand::Get));
    assert_unsupported(
        &mut h,
        Command::Keymap(KeymapCommand::Set {
            keymap: Keymap::default(),
        }),
    );
}

// web-latency: web-only (the worklet's latency report, `ether-wasm/src/latency.rs`); its
// prewire test is `crates/ether-wasm/tests/latency_prewire.rs`. rack-presets: the format is
// pinned in `ether-model/tests/roadmap_v4.rs` (`rack_presets_store_chains`). ux-followups and
// keymap's UI parts have no engine side.

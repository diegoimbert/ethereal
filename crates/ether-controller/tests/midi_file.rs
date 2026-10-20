//! Standard MIDI Files: `Media::ReadMidiFile` reads a library file into notes in
//! beats without touching the project; `Media::WriteMidiFile` writes MIDI clips as they play
//! (offset, loop, length) into a type 1 file, from the earliest clip or the project start.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::media::{
    BrowseLocation, MediaCommand, MediaSource, MidiExportRequest, MidiFileContents, MidiFileNote,
    MidiFileTempo,
};
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ReplyValue};
use ether_media::smf::{self, WriteTrack};

const LIB: &str = "lib";

fn note(pitch: u8, start: f64, duration: f64) -> MidiFileNote {
    MidiFileNote {
        pitch,
        velocity: 0.8,
        release_velocity: 0.5,
        start: Beats(start),
        duration: Beats(duration),
    }
}

fn harness() -> Harness {
    let mut lib = MemoryLibrary::new();
    lib.add_root(LIB, "Samples");
    let song = smf::write(
        &[
            WriteTrack {
                name: "keys".into(),
                channel: 0,
                notes: vec![note(60, 0.0, 1.0), note(64, 2.0, 0.5)],
            },
            WriteTrack {
                name: "bass".into(),
                channel: 1,
                notes: vec![note(36, 0.0, 4.0)],
            },
        ],
        &[MidiFileTempo {
            time: Beats(0.0),
            bpm: 96.0,
        }],
        &[],
    );
    lib.add_file(LIB, "Midi/song.mid", song);
    lib.add_file(LIB, "Midi/broken.mid", b"MThd nope".to_vec());
    lib.add_file(
        LIB,
        "kick.wav",
        wav(44_100, &[sine(44_100, 220.0, 441, 0.5)]),
    );
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Test");
    h
}

fn read(
    h: &mut Harness,
    path: &str,
) -> Result<MidiFileContents, ether_core::protocol::CommandError> {
    let out = h.send(Command::Media(MediaCommand::ReadMidiFile {
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: LIB.into() },
            path: path.into(),
        },
    }));
    if out.iter().any(|m| matches!(m, ether_core::protocol::ServerMessage::Reply(r) if matches!(r.result, ether_core::protocol::ReplyResult::Err { .. }))) {
        return Err(err(&out));
    }
    match ok(&out) {
        ReplyValue::MidiFile { file } => Ok(file),
        other => panic!("{other:?}"),
    }
}

#[test]
fn reads_a_library_midi_file_without_editing_the_project() {
    let mut h = harness();
    let before = h.project().clone();
    let file = read(&mut h, "Midi/song.mid").unwrap();
    assert_eq!(file.name, "song");
    let names: Vec<_> = file
        .tracks
        .iter()
        .map(|t| t.name.clone().unwrap())
        .collect();
    assert_eq!(names, ["keys", "bass"]);
    assert_eq!(file.tracks[0].notes[1].start, Beats(2.0));
    assert!((file.tempos[0].bpm - 96.0).abs() < 0.01);
    assert_eq!(file.length, Beats(4.0));
    assert_eq!(h.project(), &before);

    assert_eq!(
        read(&mut h, "Midi/broken.mid").unwrap_err().code,
        ErrorCode::Decode
    );
    assert_eq!(
        read(&mut h, "kick.wav").unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        read(&mut h, "Midi/none.mid").unwrap_err().code,
        ErrorCode::NotFound
    );
    let out = h.send(Command::Media(MediaCommand::ReadMidiFile {
        source: MediaSource::Path {
            path: "relative.mid".into(),
        },
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

/// A MIDI track with a 2-beat looped clip at beat 4, 7 beats long (C4 at 0, E4 at 1.5).
fn looped_clip(h: &mut Harness) -> (TrackId, ClipId) {
    let track: TrackId = h.id();
    let clip: ClipId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: Some("lead".into()),
        color: None,
        parent: None,
        before: None,
    }));
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track,
        start: Beats(4.0),
        length: Beats(7.0),
        name: None,
    }));
    let (a, b) = (h.id(), h.id());
    h.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: vec![
            NoteSpec {
                id: a,
                pitch: 60,
                velocity: 1.0,
                start: Beats(0.0),
                duration: Beats(1.0),
            },
            NoteSpec {
                id: b,
                pitch: 64,
                velocity: 0.5,
                start: Beats(1.5),
                duration: Beats(1.0),
            },
        ],
    }));
    h.ok(Command::Clip(ClipCommand::SetLoop {
        id: clip,
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(2.0),
        },
    }));
    (track, clip)
}

fn export(
    h: &mut Harness,
    clips: Vec<ClipId>,
    from_project_start: bool,
) -> (String, MidiFileContents) {
    match h.ok(Command::Media(MediaCommand::WriteMidiFile {
        request: MidiExportRequest {
            clips,
            from_project_start,
            name: "my: riff".into(),
            path: None,
        },
    })) {
        ReplyValue::MidiFileWritten { result } => {
            assert_eq!(result.path, None);
            let bytes = result.data.expect("bytes").0;
            (
                result.name.clone(),
                smf::read(&bytes, &result.name).unwrap(),
            )
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn writes_clips_as_they_play() {
    let mut h = harness();
    let (_, clip) = looped_clip(&mut h);
    let (name, file) = export(&mut h, vec![clip], false);
    assert_eq!(name, "my_ riff.mid");
    assert_eq!(file.tracks.len(), 1);
    assert_eq!(file.tracks[0].name.as_deref(), Some("lead"));
    // The loop unrolled over 7 beats; E4 is cut at the clip end on the last pass.
    let notes: Vec<(u8, f64, f64)> = file.tracks[0]
        .notes
        .iter()
        .map(|n| (n.pitch, n.start.0, n.duration.0))
        .collect();
    assert_eq!(
        notes,
        [
            (60, 0.0, 1.0),
            (64, 1.5, 1.0),
            (60, 2.0, 1.0),
            (64, 3.5, 1.0),
            (60, 4.0, 1.0),
            (64, 5.5, 1.0),
            (60, 6.0, 1.0)
        ]
    );
    assert!((file.tempos[0].bpm - 120.0).abs() < 0.01);

    // From the project start: the clip keeps its place (beat 4).
    let (_, file) = export(&mut h, vec![clip], true);
    assert_eq!(file.tracks[0].notes[0].start, Beats(4.0));
}

#[test]
fn refuses_audio_clips_missing_clips_and_paths_on_hosts_without_files() {
    let mut h = harness();
    let (_, clip) = looped_clip(&mut h);
    let out = h.send(Command::Media(MediaCommand::WriteMidiFile {
        request: MidiExportRequest {
            clips: vec![],
            from_project_start: false,
            name: "x".into(),
            path: None,
        },
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let missing: ClipId = h.id();
    let out = h.send(Command::Media(MediaCommand::WriteMidiFile {
        request: MidiExportRequest {
            clips: vec![missing],
            from_project_start: false,
            name: "x".into(),
            path: None,
        },
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    let out = h.send(Command::Media(MediaCommand::WriteMidiFile {
        request: MidiExportRequest {
            clips: vec![clip],
            from_project_start: false,
            name: "x".into(),
            path: Some("/tmp/x.mid".into()),
        },
    }));
    assert_eq!(err(&out).code, ErrorCode::Unsupported);
}

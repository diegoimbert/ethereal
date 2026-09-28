//! `tap-recording`: an armed audio track whose input is another track records that track's
//! tap; loop passes become take lanes exactly like hardware takes (`comping`).

use super::*;

/// An audio track taking `source`'s signal post-fader, armed (not exclusive).
fn tapped(h: &mut H, source: TrackId) -> TrackId {
    let t = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::SetInput {
        track: t,
        input: TrackInput::Track {
            track: source,
            tap: InputTap::PostFader,
        },
    });
    h.rec(RecordingCommand::Arm {
        track: t,
        armed: true,
        exclusive: false,
    });
    t
}

fn stereo_take(track: TrackId, start: f64, file: &str) -> AudioTake {
    AudioTake {
        track,
        file: file.into(),
        start,
        frames: 48_000,
        channels: 2,
        sample_rate: 48_000,
    }
}

#[test]
fn the_session_lists_armed_tap_tracks_apart_from_hardware_inputs() {
    let mut h = H::new(true);
    let source = h.track(TrackKind::Audio);
    let hw = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::SetInput {
        track: hw,
        input: TrackInput::Audio { first: 0, count: 2 },
    });
    h.rec(RecordingCommand::Arm {
        track: hw,
        armed: true,
        exclusive: true,
    });
    let rec = tapped(&mut h, source);
    // Armed but tapping nothing: neither hardware nor tap.
    let idle = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::SetInput {
        track: idle,
        input: TrackInput::None,
    });
    h.rec(RecordingCommand::Arm {
        track: idle,
        armed: true,
        exclusive: false,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    let s = h.capture().sessions.last().cloned().unwrap();
    assert_eq!(s.taps, vec![rec]);
    assert_eq!(
        s.audio,
        vec![AudioTarget {
            track: hw,
            first: 0,
            count: 2
        }]
    );
    assert!(!s.midi);
    // The engine sees the tap consumer armed with its tap.
    let desc = h.ctl.bridge.graphs.last().unwrap();
    let t = desc.tracks.iter().find(|t| t.id == rec).unwrap();
    assert!(t.armed);
    assert_eq!(t.input_tap.map(|tap| tap.track), Some(source));
    h.rec(RecordingCommand::SetRecording { enabled: false });
}

#[test]
fn a_tap_take_becomes_a_clip_on_the_tapping_track() {
    let mut h = H::new(true);
    let source = h.track(TrackKind::Audio);
    let rec = tapped(&mut h, source);
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result = RecordedTakes {
        audio: vec![stereo_take(rec, 0.0, "media/rec-x-1.wav")],
        ..Default::default()
    };
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    let p = h.project();
    assert!(p.take_lanes.is_empty());
    let clip = &p.clips[&clips[0]];
    assert_eq!((clip.track, clip.start, clip.lane), (rec, Beats(0.0), None));
    let ClipContent::Audio(a) = &clip.content else {
        panic!("{clip:?}");
    };
    assert_eq!(p.media[&a.media].channels, 2);
    assert!(
        p.arrangement_clips_of(source).is_empty(),
        "the source is untouched"
    );
}

#[test]
fn loop_recording_from_a_tap_makes_one_take_per_pass() {
    let mut h = H::new(true);
    let source = h.track(TrackKind::Audio);
    let rec = tapped(&mut h, source);
    h.ok(Command::Transport(TransportCommand::SetLoopRegion {
        region: BeatRange {
            start: Beats(4.0),
            end: Beats(6.0),
        },
    }));
    h.ok(Command::Transport(TransportCommand::SetLoopEnabled {
        enabled: true,
    }));
    h.ctl.transport.position = Beats(4.0);
    let clips_before = h.project().clips.len();
    h.rec(RecordingCommand::SetRecording { enabled: true });
    // Three passes over the 2-beat loop (1 s at 120 bpm), as the host splits them.
    h.capture().result = RecordedTakes {
        audio: vec![
            stereo_take(rec, 4.0, "media/rec-x-1.wav"),
            stereo_take(rec, 4.0, "media/rec-x-2.wav"),
            AudioTake {
                frames: 24_000,
                ..stereo_take(rec, 4.0, "media/rec-x-3.wav")
            },
        ],
        ..Default::default()
    };
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(clips.len(), 3, "one take clip per pass");
    let p = h.project();
    let lanes = p.lanes_of(rec);
    assert_eq!(lanes.len(), 3);
    for (i, c) in clips.iter().enumerate() {
        let clip = &p.clips[c];
        assert_eq!(clip.track, rec);
        assert_eq!(clip.lane, Some(lanes[i].id));
        assert_eq!(clip.start, Beats(4.0));
    }
    // The newest pass plays where it has audio (the partial third pass), the second after.
    assert_eq!(
        comp_of(p, rec),
        vec![(lanes[2].id, 4.0, 5.0), (lanes[1].id, 5.0, 6.0)]
    );
    assert!(p.lanes_of(source).is_empty());
    // One undo step removes the takes.
    h.ok(Command::Edit(EditCommand::Undo));
    let p = h.project();
    assert!(p.take_lanes.is_empty() && p.comp_regions.is_empty());
    assert_eq!(p.clips.len(), clips_before);
}

#[test]
fn without_host_capture_a_tapped_track_records_nothing() {
    let mut h = H::new(false);
    let source = h.track(TrackKind::Audio);
    tapped(&mut h, source);
    let clips_before = h.project().clips.len();
    h.rec(RecordingCommand::SetRecording { enabled: true });
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    assert_eq!(
        recording_events(&out),
        vec![RecordingEvent::Stopped { clips: vec![] }]
    );
    assert_eq!(h.project().clips.len(), clips_before);
}

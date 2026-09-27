//! Native recording with device-less backends: the loopback input simulates a round trip
//! (output → "speaker → microphone" → input) with a known latency.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_controller::{AudioTarget, RecordSession, RecordedTakes};
use ether_core::graph::{ClipContentDesc, ClipDesc, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{Beats, ClipId, MediaId, ProjectId, TrackId, TrackKind, Ulid};
use ether_core::{EngineConfig, EngineHandle, TransportControl};
use ether_media::{DecodedAudio, InMemorySource};

use super::*;
use crate::audio::{AudioBackendKind, AudioOutput, AudioSettings};
use crate::test_util::TempDir;

const SR: u32 = 48_000;
const BLOCK: usize = 256;
/// Samples per beat at the default 120 bpm.
const SPB: f64 = SR as f64 / 2.0;

fn track(id: u128, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
    TrackDesc {
        id: TrackId(Ulid(id)),
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips: vec![],
        automation: vec![],
        racks: Vec::new(),
    }
}

struct Rig {
    handle: EngineHandle,
    shared: Arc<AudioShared>,
    out: Option<AudioOutput>,
    _gc: std::thread::JoinHandle<()>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(o) = self.out.take() {
            drop(o.stop());
        }
    }
}

fn rig(backend: AudioBackendKind, input: Option<&str>, root: &std::path::Path) -> Rig {
    let parts = ether_core::create(EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_nodes: 16,
        ..Default::default()
    });
    let mut handle = parts.handle;
    let shared = Arc::new(AudioShared::default());
    shared.recording.set_projects_root(root.to_path_buf());
    let settings = AudioSettings {
        backend,
        input_device: input.map(str::to_string),
        max_block_size: BLOCK,
        ..Default::default()
    };
    let out = AudioOutput::start(Box::new(parts.engine), &settings, shared.clone())
        .map_err(|(e, _)| e)
        .expect("backend starts");
    // Like `NativeHost::start`: the stream runs before the bridge attaches the rings.
    attach(&mut handle, &shared);
    let mut gc = parts.gc;
    let gc = std::thread::spawn(move || {
        for _ in 0..400 {
            gc.collect();
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    Rig {
        handle,
        shared,
        out: Some(out),
        _gc: gc,
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn read_wav(path: &std::path::Path) -> (u16, u32, Vec<f32>) {
    let bytes = std::fs::read(path).expect("take file");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[36..40], b"data");
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
    assert_eq!(len, bytes.len() - 44, "data size patched");
    let samples = bytes[44..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    (channels, rate, samples)
}

fn session(project: ProjectId, audio: Vec<AudioTarget>, midi: bool) -> RecordSession {
    RecordSession {
        project,
        tag: "t".into(),
        audio,
        midi,
        keep_from: 0.0,
        keep_until: None,
    }
}

fn record(rig: &mut Rig, s: &RecordSession, until_beats: f64) -> RecordedTakes {
    start(&rig.shared, s).unwrap();
    for c in [
        TransportControl::Locate {
            position: Beats(0.0),
        },
        TransportControl::SetRecording { enabled: true },
        TransportControl::Play,
    ] {
        rig.handle.transport(c).unwrap();
    }
    wait_for("playback", || {
        rig.handle.playhead().position.0 > until_beats
    });
    rig.handle
        .transport(TransportControl::SetRecording { enabled: false })
        .unwrap();
    rig.handle.transport(TransportControl::Stop).unwrap();
    stop(&rig.shared, &rig.handle).unwrap()
}

#[test]
fn loopback_click_lands_on_its_timeline_position() {
    let tmp = TempDir::new("rec-loopback");
    let mut rig = rig(AudioBackendKind::Offline, Some("loopback:1000"), tmp.path());
    assert_eq!(rig.shared.recording.round_trip_latency(), 1000);

    // Track 2 plays a click at beat 2 + 100 samples; track 3 records the loopback input.
    let media = MediaId(Ulid(9));
    let mut click = vec![0.0f32; 2_000];
    click[100] = 0.9;
    rig.handle
        .add_source(
            media,
            Arc::new(InMemorySource::new(Arc::new(DecodedAudio {
                sample_rate: SR,
                channels: vec![click],
            }))),
        )
        .unwrap();
    let master = track(1, TrackKind::Master, None);
    let mut player = track(2, TrackKind::Audio, Some(master.id));
    player.clips = vec![ClipDesc {
        id: ClipId(Ulid(1)),
        start: 2.0,
        length: 1.0,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Audio {
            media,
            gain: 1.0,
            transpose: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp: None,
        },
        envelopes: vec![],
    }];
    let mut rec = track(3, TrackKind::Audio, Some(master.id));
    rec.armed = true;
    rec.audio_input = Some((0, 1));
    let rec_id = rec.id;
    rig.handle
        .publish(RenderGraphDesc {
            tracks: vec![master, player, rec],
            ..Default::default()
        })
        .unwrap();

    let project = ProjectId::v7(1_750_000_000_000, [7; 10]);
    let takes = record(
        &mut rig,
        &session(
            project,
            vec![AudioTarget {
                track: rec_id,
                first: 0,
                count: 1,
            }],
            false,
        ),
        3.0,
    );
    assert_eq!(takes.latency, 1000);
    assert_eq!(takes.audio.len(), 1, "{takes:?}");
    let take = &takes.audio[0];
    assert_eq!(take.track, rec_id);
    assert!(
        take.start.abs() < 1e-9,
        "take starts at the record position: {takes:?}"
    );
    assert_eq!((take.channels, take.sample_rate), (1, SR));
    assert!(take.file.starts_with("media/rec-t-"));

    let path = tmp.path().join(project.to_string()).join(&take.file);
    let (channels, rate, samples) = read_wav(&path);
    assert_eq!((channels, rate), (1, SR));
    assert_eq!(samples.len() as u64, take.frames);
    let (peak, value) = samples
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    assert!(*value > 0.5, "the click was recorded ({value})");
    // Compensated: the click sits exactly where it plays on the timeline.
    let position = take.start + peak as f64 / SPB;
    let expected = 2.0 + 100.0 / SPB;
    assert!(
        (position - expected).abs() * SPB < 0.5,
        "click at {position} beats, expected {expected}"
    );
}

#[test]
fn midi_is_monitored_and_recorded_with_compensation() {
    let tmp = TempDir::new("rec-midi");
    let mut rig = rig(AudioBackendKind::Null, None, tmp.path());
    let master = track(1, TrackKind::Master, None);
    let mut keys = track(2, TrackKind::Midi, Some(master.id));
    keys.armed = true;
    keys.monitor = true;
    rig.handle
        .publish(RenderGraphDesc {
            tracks: vec![master, keys],
            ..Default::default()
        })
        .unwrap();
    // The stream started before `attach`: the engine clock still gets published.
    wait_for("engine clock", || rig.shared.recording.read_clock().1 > 0);
    let (_, sample) = rig.shared.recording.read_clock();
    let engine = rig.handle.playhead().sample_time;
    assert!(
        sample.abs_diff(engine) < 48_000,
        "clock {sample} tracks the engine {engine}"
    );
    let project = ProjectId::v7(1_750_000_000_000, [7; 10]);
    let s = session(project, vec![], true);
    start(&rig.shared, &s).unwrap();
    for c in [
        TransportControl::SetRecording { enabled: true },
        TransportControl::Play,
    ] {
        rig.handle.transport(c).unwrap();
    }
    wait_for("playback", || rig.handle.playhead().position.0 > 0.5);
    assert!(inject_midi(&rig.shared, [0x90, 60, 100]));
    let on_at = rig.handle.playhead().position.0;
    std::thread::sleep(Duration::from_millis(250));
    assert!(inject_midi(&rig.shared, [0x80, 60, 0]));
    let off_at = rig.handle.playhead().position.0;
    // The note-off plays one block later: let the engine get past it.
    wait_for("note-off", || {
        rig.handle.playhead().position.0 > off_at + 0.1
    });
    rig.handle
        .transport(TransportControl::SetRecording { enabled: false })
        .unwrap();
    let takes = stop(&rig.shared, &rig.handle).unwrap();
    assert!(takes.audio.is_empty());
    let data: Vec<[u8; 3]> = takes.midi.iter().map(|m| m.data).collect();
    assert_eq!(data, vec![[0x90, 60, 100], [0x80, 60, 0]]);
    // Placed where the key was pressed (±80 ms of scheduling jitter on a loaded machine).
    assert!(
        (takes.midi[0].position - on_at).abs() < 0.16,
        "{} vs {on_at}",
        takes.midi[0].position
    );
    let held = takes.midi[1].position - takes.midi[0].position;
    assert!((0.15..0.9).contains(&held), "held {held} beats (~0.5)");
}

#[test]
fn loopback_names() {
    assert_eq!(parse_loopback("loopback"), Some(DEFAULT_LOOPBACK));
    assert_eq!(parse_loopback("loopback:512"), Some(512));
    assert_eq!(parse_loopback("loopbackX"), None);
    assert_eq!(parse_loopback("Built-in Microphone"), None);
    assert_eq!(midi::short_message(&[0x92, 1, 2]), Some([0x92, 1, 2]));
    assert_eq!(midi::short_message(&[0xc0, 5]), Some([0xc0, 5, 0]));
    assert_eq!(midi::short_message(&[0xf8]), None);
}

#[test]
fn wav_header_layout() {
    let h = writer::wav_header(2, 44_100, 10);
    assert_eq!(&h[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 36 + 80);
    assert_eq!(u16::from_le_bytes([h[20], h[21]]), 3);
    assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 80);
}

#[test]
fn unavailable_input_fails_the_audio_session_gracefully() {
    let tmp = TempDir::new("rec-denied");
    let rig = rig(AudioBackendKind::Null, None, tmp.path());
    // What `open_input` records when the device is missing or permission is denied.
    rig.shared
        .recording
        .set_input_error("Audio input unavailable: permission denied".into());
    let project = ProjectId::v7(1_750_000_000_000, [9; 10]);
    let audio = vec![AudioTarget {
        track: TrackId(Ulid(5)),
        first: 0,
        count: 1,
    }];
    let err = start(&rig.shared, &session(project, audio, false)).unwrap_err();
    assert!(err.to_string().contains("permission denied"), "{err}");
    // MIDI-only sessions don't need the audio input.
    start(&rig.shared, &session(project, vec![], true)).unwrap();
    assert_eq!(
        stop(&rig.shared, &rig.handle).unwrap(),
        RecordedTakes::default()
    );
    // Reconfiguring the input clears the error.
    configure_input(&rig.shared, &AudioSettings::default());
    assert_eq!(rig.shared.recording.input_error(), None);
}

#[test]
fn a_silent_input_take_warns() {
    let tmp = TempDir::new("rec-silent");
    let mut rig = rig(AudioBackendKind::Offline, Some("loopback:512"), tmp.path());
    let master = track(1, TrackKind::Master, None);
    let mut rec = track(3, TrackKind::Audio, Some(master.id));
    rec.armed = true;
    rec.audio_input = Some((0, 1));
    let rec_id = rec.id;
    rig.handle
        .publish(RenderGraphDesc {
            tracks: vec![master, rec],
            ..Default::default()
        })
        .unwrap();
    let project = ProjectId::v7(1_750_000_000_000, [4; 10]);
    let target = AudioTarget {
        track: rec_id,
        first: 0,
        count: 1,
    };
    // Nothing plays: the loopback delivers digital silence, like a denied microphone.
    let takes = record(&mut rig, &session(project, vec![target], false), 3.0);
    assert_eq!(takes.audio.len(), 1);
    assert_eq!(takes.warnings, vec![writer::SILENT_INPUT.to_string()]);
}

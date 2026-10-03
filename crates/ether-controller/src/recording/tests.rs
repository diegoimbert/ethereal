//! Controller recording: session setup (count-in, punch), commit as one undo step.

use std::sync::Arc;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::recording::{
    InputList, LiveAudioChunk, LiveMidiNote, MidiPort, RecordingCommand, RecordingEvent,
};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{
    ClientMessage, Command, ErrorCode, Event, ReplyResult, ReplyValue, ServerMessage,
};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};

use super::*;
use crate::memory::{MemoryLibrary, MemoryStore};
use crate::{BridgeError, Controller, EngineBridge, EtherController, HostServices};

#[derive(Default)]
struct Bridge {
    transport: Vec<TransportControl>,
    graphs: Vec<RenderGraphDesc>,
    /// `None`: no host capture (web).
    capture: Option<Capture>,
    next: u32,
}

#[derive(Default)]
struct Capture {
    sessions: Vec<RecordSession>,
    active: bool,
    result: RecordedTakes,
    stops: u32,
    /// Live data returned by the next `poll_recording` calls.
    live: std::collections::VecDeque<(Vec<LiveAudioChunk>, Vec<LiveMidiNote>)>,
    polls: u32,
}

impl EngineBridge for Bridge {
    fn create_builtin(
        &mut self,
        _: DeviceId,
        _: &BuiltinDevice,
        _: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.next += 1;
        Ok(NodeKey {
            index: self.next,
            generation: 1,
        })
    }
    fn create_plugin(
        &mut self,
        _: DeviceId,
        _: &PluginInstance,
        _: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        Err(BridgeError::Unsupported("no plugins".into()))
    }
    fn destroy_node(&mut self, _: NodeKey) -> Result<(), BridgeError> {
        Ok(())
    }
    fn load_media(
        &mut self,
        _: &MediaRef,
        _: Arc<ether_media::DecodedAudio>,
    ) -> Result<(), BridgeError> {
        Ok(())
    }
    fn unload_media(&mut self, _: MediaId) -> Result<(), BridgeError> {
        Ok(())
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.graphs.push(graph);
        Ok(())
    }
    fn set_param(&mut self, _: ParamChange) -> Result<(), BridgeError> {
        Ok(())
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.transport.push(control);
        Ok(())
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        out.clear();
    }
    fn descriptor(&mut self, _: DeviceId) -> Option<DeviceDescriptor> {
        None
    }
    fn list_inputs(&mut self) -> Result<InputList, BridgeError> {
        match &self.capture {
            Some(_) => Ok(InputList {
                audio: vec![],
                midi: vec![MidiPort {
                    id: "k".into(),
                    name: "Keys".into(),
                }],
            }),
            None => Err(BridgeError::Unsupported("no inputs".into())),
        }
    }
    fn start_recording(&mut self, session: &RecordSession) -> Result<(), BridgeError> {
        let c = self
            .capture
            .as_mut()
            .ok_or_else(|| BridgeError::Unsupported("no capture".into()))?;
        assert!(!c.active, "one session at a time");
        c.active = true;
        c.sessions.push(session.clone());
        Ok(())
    }
    fn poll_recording(&mut self, audio: &mut Vec<LiveAudioChunk>, midi: &mut Vec<LiveMidiNote>) {
        let c = self.capture.as_mut().expect("capture");
        c.polls += 1;
        if let Some((a, m)) = c.live.pop_front() {
            audio.extend(a);
            midi.extend(m);
        }
    }
    fn stop_recording(&mut self) -> Result<RecordedTakes, BridgeError> {
        let c = self.capture.as_mut().expect("capture");
        assert!(c.active, "stop without start");
        // The engine was told to stop recording first.
        assert_eq!(
            self.transport.last(),
            Some(&TransportControl::SetRecording { enabled: false })
        );
        c.active = false;
        c.stops += 1;
        Ok(std::mem::take(&mut c.result))
    }
}

struct Clock;

impl HostServices for Clock {
    fn now_ms(&self) -> u64 {
        1_750_000_000_000
    }
    fn random_seed(&mut self) -> u64 {
        7
    }
}

type Ctl = EtherController<Bridge, Clock, MemoryStore, MemoryLibrary>;

struct H {
    ctl: Ctl,
    ids: IdGen,
    req: u32,
}

impl H {
    fn new(capture: bool) -> Self {
        let bridge = Bridge {
            capture: capture.then(Capture::default),
            ..Default::default()
        };
        let mut h = Self {
            ctl: EtherController::new(bridge, Clock, MemoryStore::new(), MemoryLibrary::new()),
            ids: IdGen::new(3),
            req: 0,
        };
        let id = h.ids.next_project_id(1);
        h.ok(Command::Project(ProjectCommand::Create {
            id,
            name: "Rec".into(),
        }));
        h
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        self.req += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id: self.req,
                gesture: None,
                command,
            },
            &mut out,
        );
        out
    }

    fn reply(out: &[ServerMessage]) -> &ReplyResult {
        match out.last() {
            Some(ServerMessage::Reply(r)) => &r.result,
            other => panic!("no reply: {other:?}"),
        }
    }

    fn ok(&mut self, command: Command) -> Vec<ServerMessage> {
        let out = self.send(command);
        assert!(
            matches!(Self::reply(&out), ReplyResult::Ok { .. }),
            "{out:#?}"
        );
        out
    }

    fn track(&mut self, kind: TrackKind) -> TrackId {
        let id: TrackId = self.ids.next(1);
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        id
    }

    fn rec(&mut self, c: RecordingCommand) -> Vec<ServerMessage> {
        self.ok(Command::Recording(c))
    }

    fn project(&self) -> &Project {
        self.ctl.project().unwrap()
    }

    fn capture(&mut self) -> &mut Capture {
        self.ctl.bridge.capture.as_mut().unwrap()
    }
}

fn recording_events(out: &[ServerMessage]) -> Vec<RecordingEvent> {
    out.iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Recording { event }) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn notes_pair_on_off_and_close_held_notes() {
    let ev = |position, data| RecordedMidi {
        position,
        data,
        pass: 0,
    };
    let notes = notes_from_midi(
        &[
            ev(4.5, [0x90, 60, 127]),
            ev(4.75, [0x91, 62, 64]),
            ev(5.0, [0x80, 60, 0]),
            ev(5.25, [0xb0, 1, 3]), // CC: ignored
            ev(5.5, [0x91, 64, 0]), // velocity-0 note on = off (no matching on: ignored)
        ],
        4.0,
        6.0,
    );
    assert_eq!(notes.len(), 2);
    assert_eq!(
        (notes[0].pitch, notes[0].start, notes[0].duration),
        (60, 0.5, 0.5)
    );
    assert_eq!(notes[0].velocity, 1.0);
    // Still held at the end.
    assert_eq!(
        (notes[1].pitch, notes[1].start, notes[1].duration),
        (62, 0.75, 1.25)
    );
}

#[test]
fn list_inputs_goes_to_the_host() {
    let mut web = H::new(false);
    let out = web.send(Command::Recording(RecordingCommand::ListInputs));
    assert!(matches!(
        H::reply(&out),
        ReplyResult::Err { error } if error.code == ErrorCode::Unsupported
    ));
    let mut native = H::new(true);
    let out = native.rec(RecordingCommand::ListInputs);
    assert!(matches!(
        H::reply(&out),
        ReplyResult::Ok { value: ReplyValue::Inputs { inputs } } if inputs.midi.len() == 1
    ));
}

#[test]
fn record_with_count_in_commits_takes_as_one_undo_step() {
    let mut h = H::new(true);
    let audio = h.track(TrackKind::Audio);
    let midi = h.track(TrackKind::Midi);
    let other = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::SetInput {
        track: audio,
        input: TrackInput::Audio { first: 1, count: 1 },
    });
    h.rec(RecordingCommand::SetCountIn { bars: 1 });
    h.ok(Command::Transport(TransportCommand::Locate {
        position: Beats(8.0),
    }));
    for t in [audio, midi] {
        h.rec(RecordingCommand::Arm {
            track: t,
            armed: true,
            exclusive: false,
        });
    }
    // Armed tracks monitor (Auto) and are marked armed in the render graph.
    h.ctl.tick(1, &mut Vec::new());
    let g = h.ctl.bridge.graphs.last().unwrap();
    let td = g.tracks.iter().find(|t| t.id == audio).unwrap();
    assert!(td.armed && td.monitor);
    assert_eq!(td.audio_input, Some((1, 1)), "(first, count)");
    assert!(!g.tracks.iter().find(|t| t.id == other).unwrap().armed);

    let history_before = h.project().clips.len();
    h.ctl.bridge.transport.clear();
    let out = h.rec(RecordingCommand::SetRecording { enabled: true });
    assert_eq!(
        recording_events(&out),
        vec![RecordingEvent::Started {
            tracks: {
                let mut v = vec![audio, midi];
                v.sort();
                v
            }
        }]
    );
    // One 4/4 bar of pre-roll before the record position.
    assert_eq!(
        h.ctl.bridge.transport,
        vec![
            TransportControl::SetRecording { enabled: true },
            TransportControl::Locate {
                position: Beats(4.0)
            },
            TransportControl::Play,
        ]
    );
    let session = h.capture().sessions[0].clone();
    assert_eq!(
        session.audio,
        vec![AudioTarget {
            track: audio,
            first: 1,
            count: 1
        }]
    );
    assert!(session.midi);
    assert_eq!((session.keep_from, session.keep_until), (8.0, None));
    // Recording twice is a no-op.
    h.rec(RecordingCommand::SetRecording { enabled: true });
    assert_eq!(h.capture().sessions.len(), 1);

    h.capture().result = RecordedTakes {
        audio: vec![AudioTake {
            track: audio,
            file: "media/rec-a.wav".into(),
            start: 8.0,
            frames: 48_000,
            channels: 1,
            sample_rate: 48_000,
        }],
        midi: vec![
            RecordedMidi {
                position: 7.5, // during the count-in: dropped
                data: [0x90, 50, 100],
                pass: 0,
            },
            RecordedMidi {
                position: 8.5,
                data: [0x90, 60, 100],
                pass: 0,
            },
            RecordedMidi {
                position: 9.0,
                data: [0x80, 60, 0],
                pass: 0,
            },
        ],
        latency: 256,
        ..Default::default()
    };
    let out = h.ok(Command::Transport(TransportCommand::Stop));
    assert_eq!(h.capture().stops, 1);
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(clips.len(), 2);
    let p = h.project();
    let audio_clip = &p.clips[&clips[0]];
    assert_eq!((audio_clip.track, audio_clip.start), (audio, Beats(8.0)));
    // 1 s at 120 bpm = 2 beats.
    assert!((audio_clip.length.0 - 2.0).abs() < 1e-9);
    let ClipContent::Audio(a) = &audio_clip.content else {
        panic!()
    };
    assert!(!a.warp.enabled, "takes play back unwarped");
    assert_eq!(a.warp.source_bpm, Some(120.0));
    let media = &p.media[&a.media];
    assert_eq!(
        (media.file.as_str(), media.frames, media.channels),
        ("media/rec-a.wav", 48_000, 1)
    );
    let midi_clip = &p.clips[&clips[1]];
    assert_eq!((midi_clip.track, midi_clip.start), (midi, Beats(8.0)));
    let notes = p.notes_of(midi_clip.id);
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].pitch, notes[0].start, notes[0].duration),
        (60, Beats(0.5), Beats(0.5))
    );
    assert!(!h.ctl.transport.recording);

    // One undo step removes everything the take added.
    h.ok(Command::Edit(EditCommand::Undo));
    let p = h.project();
    assert_eq!(p.clips.len(), history_before);
    assert!(p.media.is_empty());
    assert!(p.notes.is_empty());
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(h.project().clips.len(), history_before + 2);
}

#[test]
fn punch_keeps_the_loop_region_only() {
    let mut h = H::new(true);
    let midi = h.track(TrackKind::Midi);
    let out = h.rec(RecordingCommand::SetPunch { enabled: true });
    assert_eq!(
        recording_events(&out),
        vec![RecordingEvent::PunchChanged { enabled: true }]
    );
    // Default loop region: 0..16.
    h.ok(Command::Transport(TransportCommand::SetLoopRegion {
        region: BeatRange {
            start: Beats(4.0),
            end: Beats(8.0),
        },
    }));
    h.rec(RecordingCommand::Arm {
        track: midi,
        armed: true,
        exclusive: true,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    let s = h.capture().sessions[0].clone();
    assert_eq!((s.keep_from, s.keep_until), (4.0, Some(8.0)));
    h.capture().result.midi = vec![
        RecordedMidi {
            position: 3.0,
            data: [0x90, 40, 100],
            pass: 0,
        },
        RecordedMidi {
            position: 5.0,
            data: [0x90, 60, 100],
            pass: 0,
        },
        RecordedMidi {
            position: 9.0,
            data: [0x90, 70, 100],
            pass: 0,
        },
    ];
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    let p = h.project();
    let c = &p.clips[&clips[0]];
    assert_eq!((c.start, c.length), (Beats(4.0), Beats(4.0)));
    let notes = p.notes_of(c.id);
    assert_eq!(notes.len(), 1);
    // Held to the punch-out.
    assert_eq!(
        (notes[0].pitch, notes[0].start, notes[0].duration),
        (60, Beats(1.0), Beats(3.0))
    );
    // A punch pass is a take, selected over the punch range.
    let lane = c.lane.expect("take lane");
    let region = p.comp_of(midi);
    assert_eq!(region.len(), 1);
    assert_eq!(
        (region[0].lane, region[0].start, region[0].end),
        (lane, Beats(4.0), Beats(8.0))
    );
}

#[test]
fn without_host_capture_recording_only_drives_the_transport() {
    let mut h = H::new(false);
    let t = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::Arm {
        track: t,
        armed: true,
        exclusive: false,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    assert!(h.ctl.transport.recording && h.ctl.transport.playing);
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    assert_eq!(
        recording_events(&out),
        vec![RecordingEvent::Stopped { clips: vec![] }]
    );
    assert!(h.project().clips.is_empty());
    // Still playing: only recording stopped.
    assert!(h.ctl.transport.playing && !h.ctl.transport.recording);
}

#[test]
fn toggle_play_stop_finishes_the_recording() {
    let mut h = H::new(true);
    let t = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::Arm {
        track: t,
        armed: true,
        exclusive: false,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result.audio = vec![AudioTake {
        track: t,
        file: "media/rec-b.wav".into(),
        start: 0.0,
        frames: 4_800,
        channels: 2,
        sample_rate: 48_000,
    }];
    // Space bar = TogglePlay while playing: stops and commits the take.
    let out = h.ok(Command::Transport(TransportCommand::TogglePlay));
    assert_eq!(h.capture().stops, 1);
    assert!(!h.ctl.transport.recording && !h.ctl.transport.playing);
    let events = recording_events(&out);
    assert!(
        matches!(events.as_slice(), [RecordingEvent::Stopped { clips }] if clips.len() == 1),
        "{events:?}"
    );
    // The next play does not reopen the session.
    h.ok(Command::Transport(TransportCommand::TogglePlay));
    assert_eq!(h.capture().sessions.len(), 1);
    assert!(!h.ctl.transport.recording);
}

#[test]
fn host_warnings_become_notifications() {
    let mut h = H::new(true);
    let t = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::Arm {
        track: t,
        armed: true,
        exclusive: false,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result.warnings = vec!["silent".into()];
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    assert!(out.iter().any(|m| matches!(m,
        ServerMessage::Event(Event::Notification { message, .. }) if message == "silent")));
}

fn chunk(track: TrackId, take: u32, first_peak: u64, n: usize) -> LiveAudioChunk {
    LiveAudioChunk {
        track,
        take,
        start: 8.0,
        sample_rate: 48_000,
        frames_per_peak: 256,
        first_peak,
        min: (0..n)
            .map(|i| -((first_peak as usize + i) as f32) / 100.0)
            .collect(),
        max: (0..n)
            .map(|i| (first_peak as usize + i) as f32 / 100.0)
            .collect(),
    }
}

fn tick(h: &mut H, now: u64) -> Vec<RecordingEvent> {
    let mut out = Vec::new();
    h.ctl.tick(now, &mut out);
    recording_events(&out)
}

#[test]
fn live_progress_is_throttled_merged_and_stops_with_the_recording() {
    let mut h = H::new(true);
    let audio = h.track(TrackKind::Audio);
    let midi = h.track(TrackKind::Midi);
    h.rec(RecordingCommand::SetInput {
        track: audio,
        input: TrackInput::Audio { first: 0, count: 1 },
    });
    for t in [audio, midi] {
        h.rec(RecordingCommand::Arm {
            track: t,
            armed: true,
            exclusive: false,
        });
    }
    // Not recording: the host is not polled.
    assert!(tick(&mut h, 900).is_empty());
    assert_eq!(h.capture().polls, 0);
    h.rec(RecordingCommand::SetRecording { enabled: true });
    let revision = h.ctl.revision;
    let history = h.ctl.doc.as_ref().unwrap().history.state();

    // First data: emitted right away.
    h.capture()
        .live
        .push_back((vec![chunk(audio, 1, 0, 2)], vec![]));
    assert_eq!(
        tick(&mut h, 1000),
        vec![RecordingEvent::Progress {
            audio: vec![chunk(audio, 1, 0, 2)],
            midi: vec![],
        }]
    );
    // Within 50 ms: buffered, then merged into one chunk per take.
    let on = LiveMidiNote {
        track: TrackId::NIL,
        pitch: 60,
        velocity: 100,
        start: 8.5,
        length: None,
    };
    h.capture()
        .live
        .push_back((vec![chunk(audio, 1, 2, 3)], vec![on.clone()]));
    assert!(tick(&mut h, 1016).is_empty());
    h.capture()
        .live
        .push_back((vec![chunk(audio, 1, 5, 1), chunk(audio, 2, 0, 1)], vec![]));
    assert!(tick(&mut h, 1032).is_empty());
    assert!(
        tick(&mut h, 1040).is_empty(),
        "nothing new, still throttled"
    );
    assert_eq!(
        tick(&mut h, 1050),
        vec![RecordingEvent::Progress {
            audio: vec![chunk(audio, 1, 2, 4), chunk(audio, 2, 0, 1)],
            // Host notes have no track: one per armed MIDI track.
            midi: vec![LiveMidiNote { track: midi, ..on }],
        }]
    );
    assert!(tick(&mut h, 1200).is_empty(), "nothing new: no event");
    // Runtime only: no document change, no history step.
    assert_eq!(h.ctl.revision, revision);
    assert_eq!(h.ctl.doc.as_ref().unwrap().history.state(), history);

    // Stop: what the host flushed while closing the takes is sent at once (not
    // throttled), before the real clips; `Stopped` is the last recording event.
    h.capture()
        .live
        .push_back((vec![chunk(audio, 2, 1, 5)], vec![]));
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    assert_eq!(
        events.first(),
        Some(&RecordingEvent::Progress {
            audio: vec![chunk(audio, 2, 1, 5)],
            midi: vec![],
        })
    );
    assert!(
        matches!(events.as_slice(), [_, RecordingEvent::Stopped { .. }]),
        "{events:?}"
    );
    let polls = h.capture().polls;
    assert!(tick(&mut h, 2000).is_empty());
    assert_eq!(h.capture().polls, polls, "not polled after the stop");
}

#[test]
fn hosts_without_capture_emit_no_progress() {
    let mut h = H::new(false);
    let midi = h.track(TrackKind::Midi);
    h.rec(RecordingCommand::Arm {
        track: midi,
        armed: true,
        exclusive: false,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    assert!(tick(&mut h, 1000).is_empty());
}

// ─── comping: loop / punch passes become takes ──────────────────────────────────────────

fn midi_ev(position: f64, on: bool, key: u8, pass: u32) -> RecordedMidi {
    RecordedMidi {
        position,
        data: if on { [0x90, key, 100] } else { [0x80, key, 0] },
        pass,
    }
}

fn comp_of(p: &Project, track: TrackId) -> Vec<(TakeLaneId, f64, f64)> {
    p.comp_of(track)
        .iter()
        .map(|r| (r.lane, r.start.0, r.end.0))
        .collect()
}

#[test]
fn midi_loop_passes_become_takes_over_the_old_material() {
    let mut h = H::new(true);
    let midi = h.track(TrackKind::Midi);
    h.ok(Command::Transport(TransportCommand::SetLoopRegion {
        region: BeatRange {
            start: Beats(4.0),
            end: Beats(8.0),
        },
    }));
    h.ok(Command::Transport(TransportCommand::SetLoopEnabled {
        enabled: true,
    }));
    // Existing material crossing the recorded range's end.
    let old: ClipId = h.ids.next(1);
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: old,
        track: midi,
        start: Beats(6.0),
        length: Beats(4.0),
        name: None,
    }));
    let clips_before = h.project().clips.len();
    h.rec(RecordingCommand::Arm {
        track: midi,
        armed: true,
        exclusive: true,
    });
    h.ctl.transport.position = Beats(4.0);
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result.midi = vec![
        midi_ev(5.0, true, 60, 0),
        midi_ev(5.5, false, 60, 0),
        midi_ev(4.5, true, 62, 1),
        midi_ev(5.0, false, 62, 1),
        midi_ev(6.0, true, 64, 2),
        midi_ev(7.0, false, 64, 2),
    ];
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(clips.len(), 3, "one take clip per pass");
    let p = h.project();
    let lanes = p.lanes_of(midi);
    let names: Vec<&str> = lanes.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Take 1", "Take 2", "Take 3", "Take 4"]);
    // The old clip's part inside [4, 8) is the first take; the rest stays on the main lane.
    let first = p.lane_clips_of(lanes[0].id);
    assert_eq!(first.len(), 1);
    assert_eq!(
        (first[0].start, first[0].length, first[0].offset),
        (Beats(6.0), Beats(2.0), Beats(0.0))
    );
    let main = p.arrangement_clips_of(midi);
    assert_eq!(main.len(), 1);
    assert_eq!(
        (main[0].start, main[0].length, main[0].offset),
        (Beats(8.0), Beats(2.0), Beats(2.0))
    );
    // Each pass on its own lane, spanning the loop.
    for (i, c) in clips.iter().enumerate() {
        let clip = &p.clips[c];
        assert_eq!(clip.lane, Some(lanes[i + 1].id));
        assert_eq!((clip.start, clip.length), (Beats(4.0), Beats(4.0)));
        assert_eq!(p.notes_of(*c).len(), 1);
    }
    // The newest take is what plays.
    assert_eq!(comp_of(p, midi), vec![(lanes[3].id, 4.0, 8.0)]);
    // One undo step restores the old clip on the main lane.
    h.ok(Command::Edit(EditCommand::Undo));
    let p = h.project();
    assert!(p.take_lanes.is_empty() && p.comp_regions.is_empty());
    assert_eq!(p.clips.len(), clips_before);
    assert_eq!(p.clips[&old].lane, None);
    assert_eq!(p.clips[&old].length, Beats(4.0));
}

#[test]
fn audio_loop_passes_become_takes_and_plain_recordings_stay_clips() {
    let mut h = H::new(true);
    let audio = h.track(TrackKind::Audio);
    h.rec(RecordingCommand::SetInput {
        track: audio,
        input: TrackInput::Audio { first: 0, count: 1 },
    });
    h.rec(RecordingCommand::Arm {
        track: audio,
        armed: true,
        exclusive: true,
    });
    let take = |start: f64, file: &str| AudioTake {
        track: audio,
        file: file.into(),
        start,
        frames: 48_000,
        channels: 1,
        sample_rate: 48_000,
    };
    // A plain recording over nothing: a main-lane clip (v0.1).
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result = RecordedTakes {
        audio: vec![take(0.0, "media/a.wav")],
        ..Default::default()
    };
    h.rec(RecordingCommand::SetRecording { enabled: false });
    assert!(h.project().take_lanes.is_empty());
    assert_eq!(h.project().arrangement_clips_of(audio).len(), 1);

    // Two loop passes elsewhere: two takes, the newest selected.
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result = RecordedTakes {
        audio: vec![take(4.0, "media/b.wav"), take(4.0, "media/c.wav")],
        ..Default::default()
    };
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    let p = h.project();
    let lanes = p.lanes_of(audio);
    assert_eq!(lanes.len(), 2);
    assert_eq!(p.clips[&clips[1]].lane, Some(lanes[1].id));
    // 1 s at 120 bpm = 2 beats.
    assert_eq!(comp_of(p, audio), vec![(lanes[1].id, 4.0, 6.0)]);
    assert_eq!(
        p.arrangement_clips_of(audio).len(),
        1,
        "the first recording stays"
    );
}

// ─── midi-expression: CC / bend / pressure become lanes and note expressions ────────────

#[test]
fn recorded_expression_becomes_lanes_and_note_pressure() {
    let mut h = H::new(true);
    let midi = h.track(TrackKind::Midi);
    h.rec(RecordingCommand::Arm {
        track: midi,
        armed: true,
        exclusive: true,
    });
    h.ctl.transport.position = Beats(4.0);
    h.rec(RecordingCommand::SetRecording { enabled: true });
    let ev = |position, data| RecordedMidi {
        position,
        data,
        pass: 0,
    };
    h.capture().result.midi = vec![
        ev(4.0, [0xB0, 1, 0]),
        ev(4.5, [0x90, 60, 100]),
        ev(4.6, [0xA0, 60, 64]),
        ev(4.75, [0xE0, 0x7F, 0x7F]),
        ev(5.0, [0xB0, 1, 127]),
        ev(5.0, [0xA0, 60, 127]),
        ev(5.5, [0x80, 60, 0]),
        ev(5.6, [0xD0, 30, 0]),
    ];
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    let p = h.project();
    let lanes = p.expression_lanes_of(clips[0]);
    let kinds: Vec<ExpressionKind> = lanes.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ExpressionKind::Cc { controller: 1 },
            ExpressionKind::PitchBend,
            ExpressionKind::ChannelPressure
        ]
    );
    let start = p.clips[&clips[0]].start.0;
    let cc: Vec<(f64, f32)> = lanes[0]
        .points
        .iter()
        .map(|q| (q.time.0 + start, q.value))
        .collect();
    assert_eq!(cc, vec![(4.0, 0.0), (5.0, 1.0)]);
    assert_eq!(lanes[1].points[0].value, 1.0);
    // Poly pressure on the recorded note, from its start.
    let note = p.notes_of(clips[0])[0].id;
    let pressure = p.note_expressions_of(note);
    assert_eq!(pressure.len(), 1);
    assert_eq!(pressure[0].kind, NoteExpressionKind::Pressure);
    let pts: Vec<(f64, f32)> = pressure[0]
        .points
        .iter()
        .map(|q| (q.time.0, q.value))
        .collect();
    assert!((pts[0].0 - 0.1).abs() < 1e-9 && (pts[0].1 - 64.0 / 127.0).abs() < 1e-6);
    assert_eq!(pts[1], (0.5, 1.0));
    // One undo step removes the clip with its expression.
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(h.project().expression_lanes.is_empty());
    assert!(h.project().note_expressions.is_empty());
}

#[test]
fn a_pass_with_only_controller_moves_still_records_a_clip() {
    let mut h = H::new(true);
    let midi = h.track(TrackKind::Midi);
    h.rec(RecordingCommand::Arm {
        track: midi,
        armed: true,
        exclusive: true,
    });
    h.rec(RecordingCommand::SetRecording { enabled: true });
    h.capture().result.midi = vec![RecordedMidi {
        position: 0.5,
        data: [0xB0, 64, 127],
        pass: 0,
    }];
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(clips.len(), 1);
    let p = h.project();
    assert!(p.notes_of(clips[0]).is_empty());
    assert_eq!(p.expression_lanes_of(clips[0]).len(), 1);
}

// ─── mpe: member channels become per-note pitch / pressure / timbre ─────────────────────

#[test]
fn an_mpe_track_records_member_channels_as_note_expressions() {
    let mut h = H::new(true);
    let mpe = h.track(TrackKind::Midi);
    let plain = h.track(TrackKind::Midi);
    h.ok(Command::Expression(
        ether_core::protocol::expression::ExpressionCommand::SetTrackMpe {
            track: mpe,
            mpe: Some(MpeSettings::default()),
        },
    ));
    for t in [mpe, plain] {
        h.rec(RecordingCommand::Arm {
            track: t,
            armed: true,
            exclusive: false,
        });
    }
    h.rec(RecordingCommand::SetRecording { enabled: true });
    let ev = |position, data| RecordedMidi {
        position,
        data,
        pass: 0,
    };
    h.capture().result.midi = vec![
        ev(0.0, [0xE1, 0, 0x40]),
        ev(0.0, [0x91, 60, 100]),
        ev(0.0, [0x92, 64, 100]),
        ev(0.25, [0xE1, 0x7F, 0x7F]),
        ev(0.5, [0xD2, 127, 0]),
        ev(0.75, [0xB2, 74, 0]),
        ev(0.8, [0xB0, 1, 64]), // master channel: a lane on both tracks
        ev(1.0, [0x81, 60, 0]),
        ev(1.0, [0x82, 64, 0]),
    ];
    let out = h.rec(RecordingCommand::SetRecording { enabled: false });
    let events = recording_events(&out);
    let [RecordingEvent::Stopped { clips }] = events.as_slice() else {
        panic!("{events:?}");
    };
    let p = h.project();
    let on = |t: TrackId| *clips.iter().find(|c| p.clips[c].track == t).unwrap();
    let (cm, cp) = (on(mpe), on(plain));
    // MPE track: one lane (CC 1 from the master channel), per-note curves.
    let lanes: Vec<ExpressionKind> = p.expression_lanes_of(cm).iter().map(|l| l.kind).collect();
    assert_eq!(lanes, vec![ExpressionKind::Cc { controller: 1 }]);
    let notes = p.notes_of(cm);
    let of = |pitch: u8| {
        let n = notes.iter().find(|n| n.pitch == pitch).unwrap().id;
        let mut x: Vec<(NoteExpressionKind, Vec<(f64, f32)>)> = p
            .note_expressions_of(n)
            .iter()
            .map(|e| {
                (
                    e.kind,
                    e.points.iter().map(|q| (q.time.0, q.value)).collect(),
                )
            })
            .collect();
        x.sort_by_key(|e| e.0);
        x
    };
    assert_eq!(
        of(60),
        vec![(NoteExpressionKind::Pitch, vec![(0.0, 0.0), (0.25, 48.0)])]
    );
    assert_eq!(
        of(64),
        vec![
            (NoteExpressionKind::Pressure, vec![(0.5, 1.0)]),
            (NoteExpressionKind::Timbre, vec![(0.75, 0.0)]),
        ]
    );
    // The plain track reads the same MIDI as channel messages.
    let plain_lanes: Vec<ExpressionKind> =
        p.expression_lanes_of(cp).iter().map(|l| l.kind).collect();
    assert!(plain_lanes.contains(&ExpressionKind::PitchBend));
    assert!(plain_lanes.contains(&ExpressionKind::Cc { controller: 74 }));
    assert!(
        p.notes_of(cp)
            .iter()
            .all(|n| p.note_expressions_of(n.id).is_empty())
    );
    // One undo step.
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(h.project().note_expressions.is_empty());
}

/// `tap-recording` (uses this module's bridge and helpers).
#[path = "tap_tests.rs"]
mod tap;

//! Controller recording: session setup (count-in, punch), commit as one undo step.

use std::sync::Arc;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::recording::{InputList, MidiPort, RecordingCommand, RecordingEvent};
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
    let ev = |position, data| RecordedMidi { position, data };
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
            },
            RecordedMidi {
                position: 8.5,
                data: [0x90, 60, 100],
            },
            RecordedMidi {
                position: 9.0,
                data: [0x80, 60, 0],
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
        },
        RecordedMidi {
            position: 5.0,
            data: [0x90, 60, 100],
        },
        RecordedMidi {
            position: 9.0,
            data: [0x90, 70, 100],
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

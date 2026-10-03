//! MIDI capture (`capture-midi`, CONTRACTS.md §13.4): the always-on buffer fed by
//! `EngineBridge::poll_midi_input`, `Capture` while stopped (phrase at the playhead, tempo and
//! loop adopted) and while playing (song positions), input filters, expression lanes, one
//! undo step, `Status` / `Clear`, availability events and clearing on project change.

mod common;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::protocol::capture::{CaptureCommand, CaptureEvent, CaptureResult, CaptureStatus};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::midi_map::MidiInputEvent;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{
    EngineOutputs, NodeKey, ParamChange, PlayheadState, RenderGraphDesc, TransportControl,
};

// ─── A FakeBridge with a MIDI input queue ──────────────────────────────────────────────

#[derive(Default)]
struct MidiBridge {
    inner: FakeBridge,
    midi: Vec<MidiInputEvent>,
}

impl EngineBridge for MidiBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_builtin(device, kind, params)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.inner.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: std::sync::Arc<ether_media::DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.inner.load_media(media, audio)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.inner.unload_media(media)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.inner.publish(graph)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.inner.set_param(change)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.inner.transport(control)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.inner.poll(out)
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn poll_midi_input(&mut self, out: &mut Vec<MidiInputEvent>) {
        out.append(&mut self.midi);
    }
}

type CaptureCtl = EtherController<MidiBridge, FakeHost, MemoryStore, MemoryLibrary>;

const ON: u8 = 0x90;
const OFF: u8 = 0x80;
const PORT: &str = "keys";

struct H {
    ctl: CaptureCtl,
    ids: IdGen,
    next_request: u32,
}

impl H {
    fn new() -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let mut h = Self {
            ctl: EtherController::with_config(
                MidiBridge::default(),
                FakeHost { now: T0 },
                store,
                MemoryLibrary::new(),
                ControllerConfig::default(),
            ),
            ids: IdGen::new(11),
            next_request: 1,
        };
        h.new_project();
        h
    }

    fn new_project(&mut self) -> ProjectId {
        let id = self.ids.next_project_id(T0);
        self.ok(Command::Project(ProjectCommand::Create {
            id,
            name: "Capture".into(),
        }));
        id
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let id = self.next_request;
        self.next_request += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture: None,
                command,
            },
            &mut out,
        );
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        ok(&self.send(command))
    }

    fn now(&self) -> u64 {
        self.ctl.host.now
    }

    /// Advance the clock to `T0 + ms` (and the engine playhead with it while playing).
    fn at(&mut self, ms: u64) {
        let target = T0 + ms;
        assert!(target >= self.ctl.host.now);
        let dt = (target - self.ctl.host.now) as f64 / 1000.0;
        self.ctl.host.now = target;
        self.ctl.store.now_ms = target;
        if let Some(ph) = self.ctl.bridge.inner.playhead.as_mut()
            && ph.playing
        {
            ph.position = Beats(ph.position.0 + dt * ph.bpm / 60.0);
            ph.seconds += dt;
        }
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    /// Messages received at the current time, then one tick.
    fn midi(&mut self, messages: &[[u8; 3]]) -> Vec<ServerMessage> {
        self.midi_from(PORT, messages)
    }

    fn midi_from(&mut self, port: &str, messages: &[[u8; 3]]) -> Vec<ServerMessage> {
        let time_ms = self.now() as f64;
        for data in messages {
            self.ctl.bridge.midi.push(MidiInputEvent {
                port: port.into(),
                data: *data,
                time_ms,
            });
        }
        self.tick()
    }

    /// `(start_ms, duration_ms, key)` notes (sorted by start) played on channel 1.
    fn play(&mut self, notes: &[(u64, u64, u8)]) {
        let mut evs: Vec<(u64, [u8; 3])> = Vec::new();
        for &(s, d, k) in notes {
            evs.push((s, [ON, k, 100]));
            evs.push((s + d, [OFF, k, 0]));
        }
        evs.sort_by_key(|e| e.0);
        for (t, data) in evs {
            self.at(t);
            self.midi(&[data]);
        }
    }

    fn project(&self) -> &Project {
        self.ctl.project().expect("open project")
    }

    fn midi_track(&mut self) -> TrackId {
        let id = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind: TrackKind::Midi,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        id
    }

    fn capture(
        &mut self,
        track: TrackId,
        adopt_tempo: bool,
    ) -> (Vec<ServerMessage>, ClipId, NoteId) {
        let (clip, seed_notes) = (self.id(), self.id());
        let out = self.send(Command::Capture(CaptureCommand::Capture {
            track,
            clip,
            seed_notes,
            adopt_tempo,
        }));
        (out, clip, seed_notes)
    }

    fn captured(&mut self, track: TrackId, adopt_tempo: bool) -> (CaptureResult, ClipId, NoteId) {
        let (out, clip, seed) = self.capture(track, adopt_tempo);
        match ok(&out) {
            ReplyValue::Captured { capture } => (capture, clip, seed),
            other => panic!("expected Captured, got {other:?}"),
        }
    }

    fn status(&mut self) -> CaptureStatus {
        match self.ok(Command::Capture(CaptureCommand::Status)) {
            ReplyValue::CaptureStatus { status } => status,
            other => panic!("expected CaptureStatus, got {other:?}"),
        }
    }

    fn notes_of(&self, clip: ClipId) -> Vec<&Note> {
        let mut n: Vec<&Note> = self
            .project()
            .notes
            .values()
            .filter(|n| n.clip == clip)
            .collect();
        n.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
        n
    }

    fn start_playing(&mut self, position: f64, bpm: f64) {
        self.ok(Command::Transport(TransportCommand::Play));
        self.ctl.bridge.inner.playhead = Some(PlayheadState {
            playing: true,
            recording: false,
            position: Beats(position),
            seconds: position * 60.0 / bpm,
            bpm,
            sample_time: 0,
        });
        self.tick();
    }

    fn stop_playing(&mut self) {
        self.ok(Command::Transport(TransportCommand::Stop));
        if let Some(ph) = self.ctl.bridge.inner.playhead.as_mut() {
            ph.playing = false;
        }
        self.tick();
    }
}

fn capture_events(out: &[ServerMessage]) -> Vec<CaptureStatus> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Capture {
                event: CaptureEvent::Changed { status },
            } => Some(status),
            _ => None,
        })
        .collect()
}

fn approx(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

// ─── Stopped ───────────────────────────────────────────────────────────────────────────

#[test]
fn stopped_capture_adopts_tempo_and_loop_as_one_undo_step() {
    let mut h = H::new();
    let track = h.midi_track();
    h.tick();
    let undo_before = h.project().clone();
    // 100 bpm: 8 quarter notes (0.6 s), C major arpeggio, starting 2 s in.
    let keys = [60u8, 64, 67, 72, 67, 64, 60, 64];
    let notes: Vec<(u64, u64, u8)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (2_000 + i as u64 * 600, 300, *k))
        .collect();
    h.play(&notes);
    h.at(8_000);
    h.tick();
    let (r, clip, seed) = h.captured(track, true);
    let bpm = r.bpm.expect("tempo adopted");
    assert!(approx(bpm, 100.0, 0.05), "{bpm}");
    assert_eq!(r.clip, clip);
    assert_eq!(r.notes, 8);
    assert_eq!(r.start, Beats(0.0));
    assert_eq!(r.length, Beats(8.0), "two bars of 4/4");
    let p = h.project();
    assert!(approx(p.tempo_map().bpm_at(Beats(0.0)), bpm, 1e-9));
    assert!(p.settings.loop_enabled);
    assert_eq!(
        p.settings.loop_region,
        BeatRange {
            start: Beats(0.0),
            end: Beats(8.0)
        }
    );
    let c = &p.clips[&clip];
    assert_eq!(
        (c.track, c.start, c.length),
        (track, Beats(0.0), Beats(8.0))
    );
    let n = h.notes_of(clip);
    for (i, note) in n.iter().enumerate() {
        assert_eq!(note.id, derive_id::<NoteId, NoteId>(seed, i as u32));
        assert_eq!(note.pitch, keys[i]);
        assert!(
            approx(note.start.0, i as f64, 0.01),
            "{i}: {}",
            note.start.0
        );
        assert!(approx(note.duration.0, 0.5, 0.01));
        assert!(approx(f64::from(note.velocity), 100.0 / 127.0, 1e-6));
    }
    // One undo step restores tempo, loop and removes the clip and notes.
    h.ok(Command::Edit(EditCommand::Undo));
    let p = h.project();
    assert!(!p.clips.contains_key(&clip));
    assert!(p.notes.values().all(|n| n.clip != clip));
    assert_eq!(p.settings.loop_enabled, undo_before.settings.loop_enabled);
    assert_eq!(p.settings.loop_region, undo_before.settings.loop_region);
    assert!(approx(p.tempo_map().bpm_at(Beats(0.0)), 120.0, 1e-9));
}

#[test]
fn stopped_capture_without_adopt_tempo_keeps_the_tempo_and_lands_at_the_playhead() {
    let mut h = H::new();
    let track = h.midi_track();
    h.ok(Command::Transport(TransportCommand::Locate {
        position: Beats(16.0),
    }));
    // 0.5 s apart at the project's 120 bpm = one beat apart.
    h.play(&[(1_000, 250, 60), (1_500, 250, 62), (2_000, 250, 64)]);
    let (r, clip, _) = h.captured(track, false);
    assert_eq!(r.bpm, None);
    assert_eq!(r.start, Beats(16.0));
    assert_eq!(r.length, Beats(4.0));
    let p = h.project();
    assert!(!p.settings.loop_enabled);
    assert!(approx(p.tempo_map().bpm_at(Beats(0.0)), 120.0, 1e-9));
    let starts: Vec<f64> = h.notes_of(clip).iter().map(|n| n.start.0).collect();
    assert!(
        approx(starts[0], 0.0, 1e-9)
            && approx(starts[1], 1.0, 1e-6)
            && approx(starts[2], 2.0, 1e-6),
        "{starts:?}"
    );
}

#[test]
fn stopped_capture_takes_the_last_phrase() {
    let mut h = H::new();
    let track = h.midi_track();
    h.play(&[(0, 200, 40), (500, 200, 41)]);
    // A long silence, then the phrase that counts.
    h.play(&[
        (20_000, 200, 60),
        (20_500, 200, 62),
        (21_000, 200, 64),
        (21_500, 200, 65),
    ]);
    let (r, clip, _) = h.captured(track, true);
    assert_eq!(r.notes, 4);
    let pitches: Vec<u8> = h.notes_of(clip).iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 62, 64, 65]);
}

#[test]
fn a_note_still_held_ends_at_capture_time() {
    let mut h = H::new();
    let track = h.midi_track();
    h.at(0);
    h.midi(&[[ON, 60, 90]]);
    h.at(500);
    h.midi(&[[ON, 64, 90]]);
    h.at(1_500);
    let (r, clip, _) = h.captured(track, false);
    assert_eq!(r.notes, 2);
    let n = h.notes_of(clip);
    assert!(approx(n[0].duration.0, 3.0, 1e-6), "{}", n[0].duration.0);
    assert!(approx(n[1].duration.0, 2.0, 1e-6));
}

// ─── Playing ───────────────────────────────────────────────────────────────────────────

#[test]
fn playing_capture_keeps_song_positions() {
    let mut h = H::new();
    let track = h.midi_track();
    h.at(0);
    h.start_playing(8.0, 120.0); // bar 3
    // Beat 9.0 (+0.5 s), 10.0 (+1 s), 11.5 (+1.75 s): notes land where they were played.
    h.play(&[(500, 250, 60), (1_000, 250, 62), (1_750, 250, 64)]);
    h.at(3_000);
    h.tick();
    let (r, clip, _) = h.captured(track, true);
    assert_eq!(r.bpm, None, "no tempo change while playing");
    assert_eq!(r.start, Beats(8.0), "the bar of the first note");
    assert_eq!(r.length, Beats(4.0));
    let n = h.notes_of(clip);
    let starts: Vec<f64> = n.iter().map(|n| n.start.0).collect();
    assert!(
        approx(starts[0], 1.0, 1e-6)
            && approx(starts[1], 2.0, 1e-6)
            && approx(starts[2], 3.5, 1e-6),
        "{starts:?}"
    );
    assert!(approx(n[0].duration.0, 0.5, 1e-6));
    assert!(!h.project().settings.loop_enabled, "the loop is untouched");
}

#[test]
fn input_received_late_in_a_tick_is_placed_at_its_receive_time() {
    let mut h = H::new();
    let track = h.midi_track();
    h.at(0);
    h.start_playing(0.0, 120.0);
    // Received at +1000 ms, drained by a tick at +1100 ms (playhead at beat 2.2).
    h.at(1_000);
    h.ctl.bridge.midi.push(MidiInputEvent {
        port: PORT.into(),
        data: [ON, 60, 100],
        time_ms: (T0 + 1_000) as f64,
    });
    h.at(1_100);
    h.tick();
    h.at(1_250);
    h.midi(&[[OFF, 60, 0]]);
    let (_, clip, _) = h.captured(track, false);
    let n = h.notes_of(clip);
    assert!(approx(n[0].start.0, 2.0, 1e-6), "{}", n[0].start.0);
}

#[test]
fn playing_capture_across_a_loop_wrap_merges_the_passes() {
    let mut h = H::new();
    let track = h.midi_track();
    h.ok(Command::Transport(TransportCommand::SetLoopRegion {
        region: BeatRange {
            start: Beats(0.0),
            end: Beats(4.0),
        },
    }));
    h.ok(Command::Transport(TransportCommand::SetLoopEnabled {
        enabled: true,
    }));
    h.at(0);
    h.start_playing(3.0, 120.0);
    // Note at beat 3.5, held over the wrap.
    h.at(250);
    h.midi(&[[ON, 60, 100]]);
    // The engine wrapped: beat 0.25.
    h.at(625);
    h.ctl.bridge.inner.playhead.as_mut().unwrap().position = Beats(0.25);
    h.midi(&[[OFF, 60, 0], [ON, 62, 100]]);
    h.at(875);
    h.midi(&[[OFF, 62, 0]]);
    let (r, clip, _) = h.captured(track, false);
    assert_eq!((r.start, r.length), (Beats(0.0), Beats(4.0)));
    let n = h.notes_of(clip);
    assert_eq!(n.len(), 2);
    assert_eq!(n[0].pitch, 62);
    assert!(approx(n[0].start.0, 0.25, 1e-6));
    assert_eq!(n[1].pitch, 60);
    assert!(approx(n[1].start.0, 3.5, 1e-6));
    assert!(approx(n[1].duration.0, 0.5, 1e-6), "ends at the loop end");
}

#[test]
fn the_latest_take_with_notes_is_captured() {
    let mut h = H::new();
    let track = h.midi_track();
    h.play(&[(0, 200, 40), (500, 200, 41)]);
    h.at(1_000);
    h.start_playing(0.0, 120.0);
    h.play(&[(1_500, 200, 60), (2_000, 200, 62)]);
    h.at(2_500);
    h.stop_playing();
    // Stopped since: nothing new; the played take counts (song positions).
    let (r, clip, _) = h.captured(track, true);
    assert_eq!(r.bpm, None);
    let pitches: Vec<u8> = h.notes_of(clip).iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 62]);
}

// ─── Filters, expression, buffer ───────────────────────────────────────────────────────

#[test]
fn only_messages_reaching_the_track_input_count() {
    let mut h = H::new();
    let track = h.midi_track();
    h.ok(Command::Recording(RecordingCommand::SetInput {
        track,
        input: TrackInput::Midi {
            port: Some(PORT.into()),
            channel: Some(1),
        },
    }));
    h.at(0);
    h.midi(&[[ON, 60, 100]]); // channel 1 (0-based 0): filtered out
    h.midi_from("pads", &[[ON | 1, 61, 100]]); // other port
    h.at(200);
    h.midi(&[[OFF, 60, 0]]);
    let (out, _, _) = h.capture(track, false);
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    h.at(500);
    h.midi(&[[ON | 1, 64, 100]]);
    h.at(800);
    h.midi(&[[OFF | 1, 64, 0]]);
    let (r, clip, _) = h.captured(track, false);
    assert_eq!(r.notes, 1);
    assert_eq!(h.notes_of(clip)[0].pitch, 64);
}

#[test]
fn cc_bend_and_pressure_become_expression_lanes() {
    let mut h = H::new();
    let track = h.midi_track();
    h.at(0);
    h.midi(&[[0xb0, 64, 127], [ON, 60, 100]]); // sustain down with the note
    h.at(500);
    h.midi(&[[0xe0, 0x7f, 0x7f], [0xd0, 64, 0]]);
    h.at(1_000);
    h.midi(&[[OFF, 60, 0], [0xb0, 64, 0], [0xe0, 0, 64]]);
    h.midi(&[[0xb0, 123, 0]]); // channel mode: ignored
    let (_, clip, _) = h.captured(track, false);
    let p = h.project();
    let mut lanes: Vec<&ExpressionLane> = p
        .expression_lanes
        .values()
        .filter(|l| l.clip == clip)
        .collect();
    lanes.sort_by_key(|l| l.kind);
    let kinds: Vec<ExpressionKind> = lanes.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ExpressionKind::Cc { controller: 64 },
            ExpressionKind::PitchBend,
            ExpressionKind::ChannelPressure
        ]
    );
    let pts = |l: &ExpressionLane| -> Vec<(f64, f32)> {
        l.points.iter().map(|p| (p.time.0, p.value)).collect()
    };
    assert_eq!(pts(lanes[0]), vec![(0.0, 1.0), (2.0, 0.0)]);
    assert_eq!(pts(lanes[1]), vec![(1.0, 1.0), (2.0, 0.0)]);
    assert!((pts(lanes[2])[0].1 - 64.0 / 127.0).abs() < 1e-6);
    // Undo removes the lanes with the clip.
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(h.project().expression_lanes.is_empty());
}

#[test]
fn status_clear_and_availability_events() {
    let mut h = H::new();
    let track = h.midi_track();
    assert_eq!(h.status(), CaptureStatus::default());
    h.at(0);
    let out = h.midi(&[[ON, 60, 100]]);
    let changed = capture_events(&out);
    assert_eq!(changed.len(), 1);
    assert!(changed[0].available);
    // More notes: no event (availability unchanged).
    h.at(1_500);
    let out = h.midi(&[[OFF, 60, 0], [ON, 62, 100], [OFF, 62, 0]]);
    assert!(capture_events(&out).is_empty());
    let s = h.status();
    assert!(s.available);
    assert_eq!(s.notes, 2);
    assert!(approx(s.seconds, 1.5, 1e-9));
    // Clear → unavailable, announced once.
    let out = h.send(Command::Capture(CaptureCommand::Clear));
    let changed = capture_events(&out);
    assert_eq!(changed.len(), 1);
    assert!(!changed[0].available);
    assert!(capture_events(&h.tick()).is_empty());
    let (out, _, _) = h.capture(track, true);
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    // Capturing empties the buffer.
    h.at(2_000);
    h.midi(&[[ON, 60, 100]]);
    h.at(2_200);
    h.midi(&[[OFF, 60, 0]]);
    let (out, _, _) = h.capture(track, true);
    ok(&out);
    assert!(!capture_events(&out)[0].available);
    assert!(!h.status().available);
    let (out, _, _) = h.capture(track, true);
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
}

#[test]
fn messages_reach_the_buffer_armed_or_not_with_no_track_at_all() {
    // Nothing armed, no MIDI track yet: still buffered (site-local, all ports).
    let mut h = H::new();
    h.at(0);
    h.midi(&[[ON, 60, 100]]);
    h.at(400);
    h.midi(&[[OFF, 60, 0]]);
    let track = h.midi_track();
    let (r, _, _) = h.captured(track, false);
    assert_eq!(r.notes, 1);
}

#[test]
fn the_buffer_is_cleared_on_project_change() {
    let mut h = H::new();
    h.at(0);
    h.midi(&[[ON, 60, 100], [OFF, 60, 0]]);
    assert!(h.status().available);
    h.new_project();
    assert_eq!(h.status().notes, 0);
    let out = h.tick();
    // Availability changed: announced.
    assert_eq!(capture_events(&out).len(), 1);
}

#[test]
fn capture_rejects_bad_targets() {
    let mut h = H::new();
    h.at(0);
    h.midi(&[[ON, 60, 100], [OFF, 60, 0]]);
    let audio: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: audio,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let (out, _, _) = h.capture(audio, false);
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let missing: TrackId = h.id();
    let (out, _, _) = h.capture(missing, false);
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    // A failed capture keeps the buffer.
    assert!(h.status().available);
}

#[test]
fn the_buffer_is_bounded() {
    use ether_core::protocol::capture::CAPTURE_MAX_EVENTS;
    let mut h = H::new();
    h.at(0);
    let burst: Vec<[u8; 3]> = (0..CAPTURE_MAX_EVENTS + 100)
        .map(|i| {
            if i % 2 == 0 {
                [ON, 60, 100]
            } else {
                [OFF, 60, 0]
            }
        })
        .collect();
    h.midi(&burst);
    let s = h.status();
    assert_eq!(s.notes as usize, CAPTURE_MAX_EVENTS / 2);
}

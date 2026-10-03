//! MIDI capture (v0.3, `capture-midi`; protocol `ether_protocol::capture`, CONTRACTS.md
//! §13.4): Ableton-style "Capture". The controller always keeps the MIDI played into it.
//!
//! - [`EtherController::capture_input`]: every incoming MIDI channel message (called from
//!   `midi_learn_tick`, where `EngineBridge::poll_midi_input` is drained) goes into a bounded
//!   ring ([`CaptureState`]; `CAPTURE_MAX_SECONDS` / `CAPTURE_MAX_EVENTS`, newest kept) with its
//!   port, host time and, while playing, its song position (the playhead polled this tick,
//!   minus the message's age, folded back into the loop across a wrap). Every play/stop
//!   transition starts a new *take*.
//! - [`EtherController::capture_command`]: `Capture::{Capture, Clear, Status}`. `Capture`
//!   uses the messages that reach the track's input filter, from the latest take with notes:
//!   - played while the transport ran: notes keep their song positions; the clip spans the
//!     played bars (merged loop passes stay in the loop);
//!   - played while stopped: the last phrase (after a silence of
//!     [`build::PHRASE_GAP_SECONDS`]) goes at the playhead; with `adopt_tempo` the tempo is
//!     inferred ([`tempo::infer_bpm`]: first note = first downbeat) and the loop is set to
//!     the clip. The clip length rounds up to whole bars.
//!
//!   One undo step ("Capture"): tempo and loop, the clip, its notes (`derive_id(seed_notes,
//!   i)` in (start, pitch) order) and its CC / pitch-bend / channel-pressure lanes (step
//!   curves). The buffer is then emptied (Capture greys out until more is played).
//! - [`EtherController::capture_tick`]: a play/stop transition starts a new take; the
//!   buffer is cleared when the project changes; `CaptureEvent::Changed` on availability
//!   changes only.
//!
//! The buffer is runtime and site-local: never saved, undone or replicated.

mod build;
mod tempo;

use std::collections::VecDeque;

use ether_core::protocol::capture::{
    CAPTURE_MAX_EVENTS, CAPTURE_MAX_SECONDS, CaptureCommand, CaptureEvent, CaptureResult,
    CaptureStatus,
};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::midi_map::MidiInputEvent;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{Command, Event, ReplyValue};

use crate::doc;
use crate::handlers::{event, no_project};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// A message older than this when it is drained is not placed earlier than the playhead
/// minus this (a stalled tick does not smear a phrase backwards).
const MAX_INPUT_LAG_SECONDS: f64 = 0.5;
/// A held note's end tolerance when rounding a stopped phrase up to whole bars (beats).
const BAR_TOLERANCE: f64 = 0.25;

/// One buffered message.
#[derive(Clone, Debug, PartialEq)]
struct Captured {
    port: String,
    data: [u8; 3],
    /// Host receive time, ms (`MidiInputEvent::time_ms`).
    time_ms: f64,
    /// Song position (beats) when played while the transport ran.
    song: Option<f64>,
    /// Take: bumped on every play/stop transition.
    take: u32,
}

/// The capture ring and its availability (runtime, site-local).
#[derive(Debug, Default)]
pub(crate) struct CaptureState {
    ring: VecDeque<Captured>,
    /// Note-ons in `ring`.
    note_ons: u32,
    take: u32,
    playing: bool,
    project: Option<ProjectId>,
    /// Availability last announced with `CaptureEvent::Changed`.
    announced: bool,
}

impl CaptureState {
    fn clear(&mut self) {
        self.ring.clear();
        self.note_ons = 0;
    }

    fn push(&mut self, m: Captured) {
        if build::is_note_on(m.data) {
            self.note_ons += 1;
        }
        let newest = m.time_ms;
        self.ring.push_back(m);
        let oldest = newest - CAPTURE_MAX_SECONDS * 1000.0;
        while self
            .ring
            .front()
            .is_some_and(|f| self.ring.len() > CAPTURE_MAX_EVENTS || f.time_ms < oldest)
        {
            let f = self.ring.pop_front().expect("non-empty");
            if build::is_note_on(f.data) {
                self.note_ons -= 1;
            }
        }
    }

    fn status(&self) -> CaptureStatus {
        let seconds = match (self.ring.front(), self.ring.back()) {
            (Some(a), Some(b)) => ((b.time_ms - a.time_ms) / 1000.0).max(0.0),
            _ => 0.0,
        };
        CaptureStatus {
            available: self.note_ons > 0,
            notes: self.note_ons,
            seconds,
        }
    }
}

/// Does a message from `port` reach a track whose input is `input`?
fn reaches(input: &TrackInput, port: &str, data: [u8; 3]) -> bool {
    match input {
        TrackInput::Midi {
            port: want,
            channel,
        } => {
            want.as_deref().is_none_or(|p| p == port) && channel.is_none_or(|c| c == data[0] & 0x0f)
        }
        // No MIDI filter: all ports, all channels.
        _ => true,
    }
}

/// Start of the bar containing `b`.
fn bar_floor(map: &TempoMap, b: f64) -> f64 {
    let bb = map.bar_beat(Beats(b));
    let sig = map.signature_at(Beats(b));
    let unit = 4.0 / f64::from(sig.denominator.max(1));
    (b - (f64::from(bb.beat.max(1) - 1) + bb.fraction) * unit).max(0.0)
}

/// Length of the bar starting at `b` (beats).
fn bar_len(map: &TempoMap, b: f64) -> f64 {
    let sig = map.signature_at(Beats(b));
    (f64::from(sig.numerator.max(1)) * 4.0 / f64::from(sig.denominator.max(1))).max(Beats::EPSILON)
}

/// The first bar line at or after `end`, counting from the bar line `from` (whole bars).
fn bar_ceil(map: &TempoMap, from: f64, end: f64) -> f64 {
    let mut b = from;
    loop {
        let next = b + bar_len(map, b);
        if b + Beats::EPSILON >= end || next <= b {
            return b.max(from + bar_len(map, from));
        }
        b = next;
    }
}

/// What `Capture` builds (before ids).
struct Plan {
    start: f64,
    length: f64,
    notes: Vec<build::Note>,
    lanes: Vec<(ExpressionKind, Vec<ExpressionPoint>)>,
    bpm: Option<f64>,
    set_loop: bool,
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn capture_command(
        &mut self,
        command: &CaptureCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.capture_sync(now);
        match command {
            CaptureCommand::Status => Ok(ReplyValue::CaptureStatus {
                status: self.capture.status(),
            }),
            CaptureCommand::Clear => {
                self.capture.clear();
                self.capture_announce(out);
                Ok(ReplyValue::Unit)
            }
            CaptureCommand::Capture {
                track,
                clip,
                seed_notes,
                adopt_tempo,
            } => {
                let capture = self.capture(*track, *clip, *seed_notes, *adopt_tempo, now, out)?;
                self.capture.clear();
                self.capture_announce(out);
                Ok(ReplyValue::Captured { capture })
            }
        }
    }

    /// One incoming MIDI message (all ports).
    pub(crate) fn capture_input(&mut self, event: &MidiInputEvent, now: u64) {
        // Channel voice messages only (no system / realtime bytes).
        if !(0x80..0xf0).contains(&event.data[0]) || !event.time_ms.is_finite() {
            return;
        }
        self.capture_sync(now);
        let song = self.capture_song_position(event.time_ms, now);
        let take = self.capture.take;
        self.capture.push(Captured {
            port: event.port.clone(),
            data: event.data,
            time_ms: event.time_ms,
            song,
            take,
        });
    }

    /// Called every tick.
    pub(crate) fn capture_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        self.capture_sync(now);
        self.capture_announce(out);
    }

    /// Project change → clear; play/stop transition → new take.
    fn capture_sync(&mut self, _now: u64) {
        let project = self.doc.as_ref().map(|d| d.project.id);
        if project != self.capture.project {
            self.capture.project = project;
            self.capture.clear();
        }
        if self.transport.playing != self.capture.playing {
            self.capture.playing = self.transport.playing;
            self.capture.take = self.capture.take.wrapping_add(1);
        }
    }

    fn capture_announce(&mut self, out: &mut dyn MessageSink) {
        let status = self.capture.status();
        if status.available != self.capture.announced {
            self.capture.announced = status.available;
            event(
                out,
                Event::Capture {
                    event: CaptureEvent::Changed { status },
                },
            );
        }
    }

    /// Song position (beats) of a message received at `time_ms`, or `None` when stopped.
    fn capture_song_position(&self, time_ms: f64, now: u64) -> Option<f64> {
        if !self.transport.playing {
            return None;
        }
        let p = &self.doc.as_ref()?.project;
        let map = p.tempo_map();
        let pos = self.transport.position.0;
        let lag = ((now as f64 - time_ms) / 1000.0).clamp(0.0, MAX_INPUT_LAG_SECONDS);
        let secs = map.beats_to_seconds(Beats(pos)).0 - lag;
        let mut b = map.seconds_to_beats(Seconds(secs)).0;
        let s = &p.settings;
        let (ls, le) = (s.loop_region.start.0, s.loop_region.end.0);
        if s.loop_enabled && le > ls && pos >= ls && pos <= le && b < ls {
            b += le - ls; // played just before the wrap
        }
        Some(b.max(0.0))
    }

    fn capture(
        &mut self,
        track: TrackId,
        clip: ClipId,
        seed_notes: NoteId,
        adopt_tempo: bool,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<CaptureResult> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let p = &doc.project;
        let t = p
            .tracks
            .get(&track)
            .ok_or_else(|| not_found(format!("track {track}")))?;
        if t.kind != TrackKind::Midi {
            return Err(invalid("Capture needs a MIDI track"));
        }
        if p.clips.contains_key(&clip) {
            return Err(invalid(format!("clip {clip} already exists")));
        }
        let input = &t.input;
        let heard: Vec<&Captured> = self
            .capture
            .ring
            .iter()
            .filter(|m| reaches(input, &m.port, m.data))
            .collect();
        let Some(take) = heard
            .iter()
            .rev()
            .find(|m| build::is_note_on(m.data))
            .map(|m| m.take)
        else {
            return Err(invalid_state("nothing was played to capture"));
        };
        let msgs: Vec<&Captured> = heard.into_iter().filter(|m| m.take == take).collect();
        let played_while_running = msgs.iter().any(|m| m.song.is_some());
        let plan = if played_while_running {
            self.capture_plan_playing(&msgs, take, now)
        } else {
            self.capture_plan_stopped(&msgs, adopt_tempo, now)
        }
        .ok_or_else(|| invalid_state("nothing was played to capture"))?;

        let name = t.name.clone();
        let Plan {
            start,
            length,
            notes,
            lanes,
            bpm,
            set_loop,
        } = plan;
        self.edit_with("Capture", None, now, out, |ctx| {
            if let Some(bpm) = bpm {
                doc::apply(ctx, &Command::Transport(TransportCommand::SetTempo { bpm }))?;
            }
            if set_loop {
                let region = BeatRange {
                    start: Beats(start),
                    end: Beats(start + length),
                };
                doc::apply(
                    ctx,
                    &Command::Transport(TransportCommand::SetLoopRegion { region }),
                )?;
                doc::apply(
                    ctx,
                    &Command::Transport(TransportCommand::SetLoopEnabled { enabled: true }),
                )?;
            }
            doc::apply(
                ctx,
                &Command::Clip(ClipCommand::CreateMidi {
                    id: clip,
                    track,
                    start: Beats(start),
                    length: Beats(length),
                    name: Some(name),
                }),
            )?;
            let specs: Vec<NoteSpec> = notes
                .iter()
                .enumerate()
                .map(|(i, n)| NoteSpec {
                    id: derive_id(seed_notes, i as u32),
                    pitch: n.pitch,
                    velocity: n.velocity,
                    start: Beats(n.start.max(0.0)),
                    duration: Beats(n.duration.max(build::MIN_NOTE)),
                })
                .collect();
            doc::apply(ctx, &Command::Note(NoteCommand::Add { clip, notes: specs }))?;
            for (kind, points) in &lanes {
                if points.is_empty() {
                    continue;
                }
                let id: ExpressionLaneId = ctx.ids.next(ctx.now);
                ctx.tx.insert(Entity::ExpressionLane(ExpressionLane {
                    id,
                    clip,
                    kind: *kind,
                    points: points.clone(),
                }))?;
            }
            Ok(())
        })?;
        Ok(CaptureResult {
            clip,
            start: Beats(start),
            length: Beats(length),
            notes: notes.len() as u32,
            bpm,
        })
    }

    /// Played while the transport ran: song positions, the clip spans the played bars.
    fn capture_plan_playing(&self, msgs: &[&Captured], take: u32, now: u64) -> Option<Plan> {
        let p = &self.doc.as_ref()?.project;
        let map = p.tempo_map();
        let s = &p.settings;
        let loop_end = (s.loop_enabled && s.loop_region.end.0 > s.loop_region.start.0)
            .then_some(s.loop_region.end.0);
        // Messages played while stopped in this take cannot be (there is one per state).
        let seq: Vec<build::Msg> = msgs
            .iter()
            .filter_map(|m| m.song.map(|at| build::Msg { data: m.data, at }))
            .collect();
        let last = seq.last()?.at;
        // Notes still held end now (if this take is still running) or at the last message.
        let end = if take == self.capture.take {
            self.capture_song_position(now as f64, now).unwrap_or(last)
        } else {
            last
        };
        let notes = build::notes(&seq, end.max(last), loop_end);
        let first = notes.first()?;
        let start = bar_floor(&map, first.start);
        let note_end = notes
            .iter()
            .map(|n| n.start + n.duration.max(build::MIN_NOTE))
            .fold(start, f64::max);
        let stop = bar_ceil(&map, start, note_end);
        let notes = notes
            .into_iter()
            .map(|n| build::Note {
                start: n.start - start,
                ..n
            })
            .collect();
        let lanes = build::lanes(&seq, |b| b - start);
        Some(Plan {
            start,
            length: stop - start,
            notes,
            lanes,
            bpm: None,
            set_loop: false,
        })
    }

    /// Played while stopped: the last phrase at the playhead (tempo inferred with
    /// `adopt_tempo`).
    fn capture_plan_stopped(
        &self,
        msgs: &[&Captured],
        adopt_tempo: bool,
        now: u64,
    ) -> Option<Plan> {
        let p = &self.doc.as_ref()?.project;
        let map = p.tempo_map();
        let all: Vec<build::Msg> = msgs
            .iter()
            .map(|m| build::Msg {
                data: m.data,
                at: m.time_ms / 1000.0,
            })
            .collect();
        let from = build::last_phrase_start(&all, build::PHRASE_GAP_SECONDS)?;
        let phrase = &all[from..];
        let t0 = phrase.iter().find(|m| build::is_note_on(m.data))?.at;
        let rel: Vec<build::Msg> = phrase
            .iter()
            .map(|m| build::Msg {
                data: m.data,
                at: m.at - t0,
            })
            .collect();
        let end = (now as f64 / 1000.0 - t0).max(rel.last()?.at);
        let notes_s = build::notes(&rel, end, None);
        let at = self.transport.position.0.max(0.0);
        let bpm = if adopt_tempo {
            let onsets: Vec<f64> = notes_s.iter().map(|n| n.start).collect();
            tempo::infer_bpm(&onsets)
        } else {
            None
        };
        // Seconds from the first note → beats from the playhead.
        let at_s = map.beats_to_seconds(Beats(at)).0;
        let to_beats = |t: f64| match bpm {
            Some(bpm) => t * bpm / 60.0,
            None => map.seconds_to_beats(Seconds(at_s + t)).0 - at,
        };
        let notes: Vec<build::Note> = notes_s
            .iter()
            .map(|n| {
                let s = to_beats(n.start);
                build::Note {
                    start: s,
                    duration: to_beats(n.start + n.duration) - s,
                    ..*n
                }
            })
            .collect();
        let note_end = notes
            .iter()
            .map(|n| n.start + n.duration.max(build::MIN_NOTE))
            .fold(0.0, f64::max);
        let stop = bar_ceil(&map, at, at + (note_end - BAR_TOLERANCE).max(0.0));
        let lanes = build::lanes(&rel, to_beats);
        Some(Plan {
            start: at,
            length: stop - at,
            notes,
            lanes,
            bpm,
            set_loop: adopt_tempo,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_is_bounded_by_count_and_age() {
        let mut s = CaptureState::default();
        let msg = |t: f64, data: [u8; 3]| Captured {
            port: "p".into(),
            data,
            time_ms: t,
            song: None,
            take: 0,
        };
        for i in 0..(CAPTURE_MAX_EVENTS + 10) {
            s.push(msg(i as f64, [0x90, 60, 1]));
        }
        assert_eq!(s.ring.len(), CAPTURE_MAX_EVENTS);
        assert_eq!(s.note_ons as usize, CAPTURE_MAX_EVENTS);
        assert_eq!(s.ring.front().unwrap().time_ms, 10.0);
        // Ten minutes later everything older is gone.
        let last = s.ring.back().unwrap().time_ms;
        s.push(msg(
            last + CAPTURE_MAX_SECONDS * 1000.0 + 1.0,
            [0x80, 60, 0],
        ));
        assert_eq!(s.ring.len(), 1);
        assert_eq!(s.note_ons, 0);
        assert!(!s.status().available);
    }

    #[test]
    fn input_filters_follow_the_track_input() {
        let all = TrackInput::Midi {
            port: None,
            channel: None,
        };
        let port_ch = TrackInput::Midi {
            port: Some("keys".into()),
            channel: Some(2),
        };
        assert!(reaches(&all, "x", [0x95, 1, 1]));
        assert!(reaches(&TrackInput::None, "x", [0x95, 1, 1]));
        assert!(reaches(&port_ch, "keys", [0x92, 1, 1]));
        assert!(!reaches(&port_ch, "keys", [0x93, 1, 1]));
        assert!(!reaches(&port_ch, "pads", [0x92, 1, 1]));
    }
}

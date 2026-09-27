//! Controller-side recording (owned by the `recording` wave-3 node; see `docs/WAVE3.md`).
//!
//! - **Arm** (`RecordingCommand::Arm`): runtime state in `EtherController::armed`, not in
//!   the document and not undoable; reported with `RecordingEvent::ArmChanged`. Arm state
//!   and each track's `monitor` mode resolve `TrackDesc::{armed, monitor}` in `compile.rs`.
//! - **Punch** (`SetPunch`): runtime flag; while on, only the loop region is kept.
//! - **Record** (`SetRecording { enabled: true }`): captures a session description
//!   ([`RecordSession`]: armed audio tracks with their input channels, whether MIDI is
//!   recorded, the kept timeline range), asks the host to start capturing
//!   ([`crate::EngineBridge::start_recording`]), enables engine recording and, when stopped,
//!   starts playback after the project's count-in (a pre-roll of `count_in_bars` bars
//!   before the record position; nothing before the record position is kept).
//! - **Stop** (`SetRecording { enabled: false }` or `Transport::Stop`): disables engine
//!   recording, collects the host's takes ([`crate::EngineBridge::stop_recording`]) and
//!   commits them as **one undo step**: a `MediaRef` + audio clip per audio take (the file
//!   is already in the project's `media/` folder; the media pipeline then loads it), and a
//!   MIDI clip with the recorded notes per armed MIDI track. Clips are placed where the
//!   host's latency compensation put them (see `ether-native`'s recording module for the
//!   formula). `RecordingEvent::Stopped` lists the new clips.
//! - **Inputs** (`ListInputs`): [`crate::EngineBridge::list_inputs`] (native: cpal input
//!   channels + MIDI ports; web: `Unsupported`).
//!
//! Hosts without capture (web, tests) reply `Unsupported` from the bridge: recording then
//! runs the transport but creates no clips.

use ether_core::TransportControl;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::recording::{RecordingCommand, RecordingEvent};
use ether_core::protocol::warp::WarpCommand;
use ether_core::protocol::{Command, Event, NotificationLevel, ReplyValue};

use crate::engine::bridge_err;
use crate::handlers::{event, no_project, notify};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, internal, not_found};
use crate::{BridgeError, EngineBridge, EtherController, HostServices, MessageSink, doc};

/// An armed audio track and the hardware input channels it records.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioTarget {
    pub track: TrackId,
    /// First hardware input channel.
    pub first: u16,
    /// 1 = mono, 2 = stereo.
    pub count: u16,
}

/// What the host should capture (built by the controller when recording starts).
#[derive(Clone, Debug, PartialEq)]
pub struct RecordSession {
    pub project: ProjectId,
    /// Unique per session: hosts name take files `media/rec-<tag>-<track>-<take>.wav`.
    pub tag: String,
    pub audio: Vec<AudioTarget>,
    /// An armed MIDI track exists: collect MIDI input.
    pub midi: bool,
    /// Keep only what was played at timeline positions `[keep_from, keep_until)` (beats,
    /// after latency compensation): the record position (count-in) or the punch region.
    pub keep_from: f64,
    pub keep_until: Option<f64>,
}

/// One recorded audio take of one track, already written to the project folder.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioTake {
    pub track: TrackId,
    /// Project-relative path (`media/...`).
    pub file: String,
    /// Timeline position (beats) of the first frame, latency-compensated.
    pub start: f64,
    pub frames: u64,
    pub channels: u16,
    pub sample_rate: u32,
}

/// A MIDI message played while recording, at its latency-compensated timeline position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedMidi {
    pub position: f64,
    pub data: [u8; 3],
}

/// What the host captured during a session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordedTakes {
    pub audio: Vec<AudioTake>,
    /// Sorted by position (within `keep_from..keep_until`).
    pub midi: Vec<RecordedMidi>,
    /// Round-trip latency that was compensated, in samples (diagnostics).
    pub latency: u32,
}

/// Recording runtime state of the controller.
#[derive(Debug, Default)]
pub(crate) struct RecordingState {
    punch: bool,
    session: Option<Active>,
    sessions: u32,
}

#[derive(Debug)]
struct Active {
    session: RecordSession,
    /// The host is capturing (`start_recording` succeeded).
    host: bool,
    /// Armed MIDI tracks (each gets a clip with the recorded notes).
    midi_tracks: Vec<TrackId>,
}

/// Longest count-in (bars).
pub const MAX_COUNT_IN_BARS: u32 = 16;
/// Shortest MIDI clip / note (beats).
const MIN_LENGTH: f64 = 1.0 / 64.0;

/// Notes from recorded MIDI (note on/off pairs; notes still held end at `end`). Positions
/// are relative to `start`.
pub(crate) fn notes_from_midi(events: &[RecordedMidi], start: f64, end: f64) -> Vec<NoteSpecDraft> {
    let mut held: Vec<(u8, u8, f64, f32)> = Vec::new();
    let mut notes = Vec::new();
    for e in events {
        let status = e.data[0] & 0xf0;
        let channel = e.data[0] & 0x0f;
        let key = e.data[1] & 0x7f;
        let vel = e.data[2] & 0x7f;
        let on = status == 0x90 && vel > 0;
        let off = status == 0x80 || (status == 0x90 && vel == 0);
        if !(on || off) {
            continue;
        }
        if let Some(i) = held.iter().position(|h| h.0 == channel && h.1 == key) {
            let (_, _, t0, v) = held.remove(i);
            notes.push(NoteSpecDraft {
                pitch: key,
                velocity: v,
                start: t0 - start,
                duration: (e.position - t0).max(MIN_LENGTH),
            });
        }
        if on {
            held.push((channel, key, e.position, f32::from(vel) / 127.0));
        }
    }
    for (_, key, t0, v) in held {
        notes.push(NoteSpecDraft {
            pitch: key,
            velocity: v,
            start: t0 - start,
            duration: (end - t0).max(MIN_LENGTH),
        });
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    notes
}

/// A recorded note before it gets an id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NoteSpecDraft {
    pub pitch: u8,
    pub velocity: f32,
    pub start: f64,
    pub duration: f64,
}

fn unsupported_host(e: &BridgeError) -> bool {
    matches!(e, BridgeError::Unsupported(_))
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Command::Recording` other than the document commands (monitor, input, count-in).
    pub(crate) fn recording_command(
        &mut self,
        c: &RecordingCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            RecordingCommand::Arm {
                track,
                armed,
                exclusive,
            } => {
                let doc = self.doc.as_ref().ok_or_else(no_project)?;
                if !doc.project.tracks.contains_key(track) {
                    return Err(not_found(format!("track {track}")));
                }
                let before = self.armed.clone();
                if *armed {
                    if *exclusive {
                        self.armed.clear();
                    }
                    self.armed.insert(*track);
                } else {
                    self.armed.remove(track);
                }
                if self.armed != before {
                    self.engine.graph_dirty = true;
                    self.emit_armed(out);
                }
                Ok(ReplyValue::Unit)
            }
            RecordingCommand::SetRecording { enabled: true } => {
                self.start_recording(now, out)?;
                Ok(ReplyValue::Unit)
            }
            RecordingCommand::SetRecording { enabled: false } => {
                self.finish_recording(now, out);
                Ok(ReplyValue::Unit)
            }
            RecordingCommand::SetPunch { enabled } => {
                if self.recording.punch != *enabled {
                    self.recording.punch = *enabled;
                    event(
                        out,
                        Event::Recording {
                            event: RecordingEvent::PunchChanged { enabled: *enabled },
                        },
                    );
                }
                Ok(ReplyValue::Unit)
            }
            RecordingCommand::ListInputs => self
                .bridge
                .list_inputs()
                .map(|inputs| ReplyValue::Inputs { inputs })
                .map_err(bridge_err),
            other => Err(internal(format!("unhandled recording command {other:?}"))),
        }
    }

    /// `Transport::Stop`: stop (or return to the start position) and finish a recording.
    pub(crate) fn transport_stop(&mut self, now: u64, out: &mut dyn MessageSink) -> CmdResult<()> {
        self.stop()?;
        self.finish_recording(now, out);
        Ok(())
    }

    fn start_recording(&mut self, now: u64, out: &mut dyn MessageSink) -> CmdResult<()> {
        if self.recording.session.is_some() {
            return Ok(());
        }
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let p = &doc.project;
        let start = self.transport.position.0.max(0.0);
        let (keep_from, keep_until) = if self.recording.punch {
            let r = p.settings.loop_region;
            (r.start.0, Some(r.end.0))
        } else {
            (start, None)
        };
        let armed: Vec<&Track> = self.armed.iter().filter_map(|t| p.tracks.get(t)).collect();
        let audio = armed
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .filter_map(|t| match t.input {
                TrackInput::Audio { first, count } => Some(AudioTarget {
                    track: t.id,
                    first,
                    count: count.clamp(1, 2),
                }),
                _ => None,
            })
            .collect();
        let midi_tracks: Vec<TrackId> = armed
            .iter()
            .filter(|t| t.kind == TrackKind::Midi)
            .map(|t| t.id)
            .collect();
        let tracks: Vec<TrackId> = armed.iter().map(|t| t.id).collect();
        self.recording.sessions += 1;
        let session = RecordSession {
            project: p.id,
            tag: format!("{}-{}", now, self.recording.sessions),
            audio,
            midi: !midi_tracks.is_empty(),
            keep_from,
            keep_until,
        };
        // Count-in: pre-roll whole bars before the record position.
        let pre_roll = if self.transport.playing {
            0.0
        } else {
            let map = p.tempo_map();
            let sig = map.signature_at(Beats(start));
            let bar = f64::from(sig.numerator) * 4.0 / f64::from(sig.denominator.max(1));
            f64::from(p.settings.count_in_bars.min(MAX_COUNT_IN_BARS)) * bar
        };

        // The engine must see the current arm state before it starts capturing.
        self.publish_if_due(now, true, out);
        let host = match self.bridge.start_recording(&session) {
            Ok(()) => true,
            Err(e) if unsupported_host(&e) => false,
            Err(e) => {
                notify(
                    out,
                    NotificationLevel::Warning,
                    format!("recording input unavailable: {e}"),
                );
                false
            }
        };
        let engine = (|| -> Result<(), BridgeError> {
            self.bridge
                .transport(TransportControl::SetRecording { enabled: true })?;
            if !self.transport.playing {
                if pre_roll > 0.0 {
                    self.bridge.transport(TransportControl::Locate {
                        position: Beats(start - pre_roll),
                    })?;
                }
                self.bridge.transport(TransportControl::Play)?;
            }
            Ok(())
        })();
        if let Err(e) = engine {
            if host {
                let _ = self.bridge.stop_recording();
            }
            let _ = self
                .bridge
                .transport(TransportControl::SetRecording { enabled: false });
            return Err(bridge_err(e));
        }
        if !self.transport.playing {
            self.transport.playing = true;
            self.transport.start_position = Beats(start);
            self.transport.position = Beats(start - pre_roll);
        }
        self.transport.recording = true;
        self.recording.session = Some(Active {
            session,
            host,
            midi_tracks,
        });
        event(
            out,
            Event::Recording {
                event: RecordingEvent::Started { tracks },
            },
        );
        Ok(())
    }

    /// Disable recording and commit what was captured (no-op when not recording).
    pub(crate) fn finish_recording(&mut self, now: u64, out: &mut dyn MessageSink) {
        if self.transport.recording || self.recording.session.is_some() {
            let _ = self
                .bridge
                .transport(TransportControl::SetRecording { enabled: false });
            self.transport.recording = false;
        }
        let Some(active) = self.recording.session.take() else {
            return;
        };
        let takes = if active.host {
            match self.bridge.stop_recording() {
                Ok(t) => t,
                Err(e) => {
                    notify(
                        out,
                        NotificationLevel::Error,
                        format!("recording failed: {e}"),
                    );
                    RecordedTakes::default()
                }
            }
        } else {
            RecordedTakes::default()
        };
        let stop_at = self.transport.position.0;
        match self.commit_takes(&active, &takes, stop_at, now, out) {
            Ok(clips) => event(
                out,
                Event::Recording {
                    event: RecordingEvent::Stopped { clips },
                },
            ),
            Err(e) => notify(
                out,
                NotificationLevel::Error,
                format!("could not add the recording: {}", e.message),
            ),
        }
    }

    /// Commit the takes as one undo step; returns the new clips.
    fn commit_takes(
        &mut self,
        active: &Active,
        takes: &RecordedTakes,
        stop_at: f64,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<Vec<ClipId>> {
        let Some(doc) = self.doc.as_ref() else {
            return Ok(Vec::new());
        };
        if doc.project.id != active.session.project {
            // The project was switched while recording: the files stay in the old one.
            return Ok(Vec::new());
        }
        let s = &active.session;
        let midi_start = s.keep_from;
        let mut midi_end = match s.keep_until {
            Some(until) => until,
            None => stop_at.max(midi_start),
        };
        let midi: Vec<RecordedMidi> = takes
            .midi
            .iter()
            .filter(|e| e.position >= s.keep_from && s.keep_until.is_none_or(|u| e.position < u))
            .copied()
            .collect();
        let notes = notes_from_midi(&midi, midi_start, midi_end);
        if let Some(last) = notes
            .iter()
            .map(|n| midi_start + n.start + n.duration)
            .reduce(f64::max)
        {
            midi_end = midi_end.max(last);
        }
        let mut clips = Vec::new();
        let takes_audio: Vec<&AudioTake> = takes
            .audio
            .iter()
            .filter(|t| t.frames > 0 && t.start.is_finite())
            .collect();
        if takes_audio.is_empty() && notes.is_empty() {
            return Ok(clips);
        }
        let midi_tracks = active.midi_tracks.clone();
        self.edit_with("Record", None, now, out, |ctx| {
            let mut n_take = 0;
            for take in &takes_audio {
                let Some(track) = ctx.p().tracks.get(&take.track).cloned() else {
                    continue;
                };
                n_take += 1;
                let media = MediaRef {
                    id: ctx.ids.next(ctx.now),
                    name: format!("{} Rec {n_take}.wav", track.name),
                    file: take.file.clone(),
                    sample_rate: take.sample_rate,
                    channels: take.channels,
                    frames: take.frames,
                    hash: None,
                };
                let media_id = media.id;
                ctx.tx.insert(Entity::Media(media))?;
                let id: ClipId = ctx.ids.next(ctx.now);
                let start = Beats(take.start.max(0.0));
                doc::apply(
                    ctx,
                    &Command::Clip(ClipCommand::CreateAudio {
                        id,
                        track: track.id,
                        start,
                        media: media_id,
                    }),
                )?;
                // A take plays back exactly as recorded (warping stays one click away).
                let warp = WarpSettings {
                    enabled: false,
                    mode: WarpMode::Complex,
                    source_bpm: Some(ctx.p().tempo_map().bpm_at(start)),
                };
                doc::apply(ctx, &Command::Warp(WarpCommand::SetWarp { clip: id, warp }))?;
                clips.push(id);
            }
            if !notes.is_empty() {
                for track in &midi_tracks {
                    let Some(t) = ctx.p().tracks.get(track).cloned() else {
                        continue;
                    };
                    let id: ClipId = ctx.ids.next(ctx.now);
                    doc::apply(
                        ctx,
                        &Command::Clip(ClipCommand::CreateMidi {
                            id,
                            track: t.id,
                            start: Beats(midi_start.max(0.0)),
                            length: Beats((midi_end - midi_start).max(MIN_LENGTH)),
                            name: Some(t.name.clone()),
                        }),
                    )?;
                    let specs = notes
                        .iter()
                        .map(|n| NoteSpec {
                            id: ctx.ids.next(ctx.now),
                            pitch: n.pitch,
                            velocity: n.velocity,
                            start: Beats(n.start.max(0.0)),
                            duration: Beats(n.duration),
                        })
                        .collect();
                    doc::apply(
                        ctx,
                        &Command::Note(NoteCommand::Add {
                            clip: id,
                            notes: specs,
                        }),
                    )?;
                    clips.push(id);
                }
            }
            Ok(())
        })?;
        Ok(clips)
    }
}

#[cfg(test)]
mod tests;

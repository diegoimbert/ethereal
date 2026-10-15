//! Standard MIDI Files (#717), via `midly`: [`read`] turns a file into notes in beats
//! (`Media::ReadMidiFile`), [`write`] makes a type 1 file from notes (`Media::WriteMidiFile`).
//!
//! Times are beats (quarter notes): ticks / PPQ for metrical files, so positions follow
//! the file's tempo map musically and land on the project's grid whatever its tempo.
//! SMPTE-timed files (rare) are converted through 120 BPM.

use std::collections::{BTreeMap, VecDeque};

use ether_core::protocol::media::{
    MidiFileContents, MidiFileNote, MidiFileTempo, MidiFileTimeSignature, MidiFileTrack,
};
use ether_core::protocol::model::{Beats, TimeSignature};
use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

/// Ticks per quarter note of written files.
pub const WRITE_PPQ: u16 = 960;

/// Why a file couldn't be read.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("not a readable MIDI file: {0}")]
pub struct SmfError(String);

/// Read a Standard MIDI File (type 0, 1 or 2). `name` is the file name (its extension is
/// dropped). Type 0 files are split by channel; every other track with notes is one track.
pub fn read(bytes: &[u8], name: &str) -> Result<MidiFileContents, SmfError> {
    let smf = Smf::parse(bytes).map_err(|e| SmfError(e.to_string()))?;
    let to_beats: Box<dyn Fn(u64) -> f64> = match smf.header.timing {
        Timing::Metrical(ppq) => {
            let ppq = f64::from(ppq.as_int().max(1));
            Box::new(move |t| t as f64 / ppq)
        }
        Timing::Timecode(fps, sub) => {
            let per_second = f64::from(fps.as_f32()) * f64::from(sub.max(1));
            Box::new(move |t| t as f64 / per_second * 2.0)
        }
    };
    let split_channels = smf.header.format == Format::SingleTrack;
    let mut tracks = Vec::new();
    let mut tempos: BTreeMap<u64, f64> = BTreeMap::new();
    let mut signatures: BTreeMap<u64, TimeSignature> = BTreeMap::new();
    let mut length = 0.0f64;
    for events in &smf.tracks {
        let mut track_name: Option<String> = None;
        let mut instrument: Option<String> = None;
        // Per channel: its notes, and the open notes by key (first in, first out).
        let mut notes: BTreeMap<u8, Vec<MidiFileNote>> = BTreeMap::new();
        let mut first_channel: Option<u8> = None;
        let mut open: BTreeMap<(u8, u8), VecDeque<(u64, f32)>> = BTreeMap::new();
        let mut tick = 0u64;
        let close = |notes: &mut BTreeMap<u8, Vec<MidiFileNote>>,
                     ch: u8,
                     key: u8,
                     (on, vel): (u64, f32),
                     off: u64,
                     release: f32| {
            let start = to_beats(on);
            let end = to_beats(off);
            notes.entry(ch).or_default().push(MidiFileNote {
                pitch: key,
                velocity: vel,
                release_velocity: release,
                start: Beats(start),
                // A note on and off on the same tick still sounds briefly.
                duration: Beats((end - start).max(1.0 / 64.0)),
            });
        };
        for ev in events {
            tick += u64::from(ev.delta.as_int());
            match ev.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            first_channel.get_or_insert(ch);
                            open.entry((ch, key.as_int()))
                                .or_default()
                                .push_back((tick, f32::from(vel.as_int()) / 127.0));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            let release = match message {
                                MidiMessage::NoteOff { vel, .. } => f32::from(vel.as_int()) / 127.0,
                                _ => 0.5,
                            };
                            let k = key.as_int();
                            if let Some(on) = open.get_mut(&(ch, k)).and_then(VecDeque::pop_front) {
                                close(&mut notes, ch, k, on, tick, release);
                            }
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(MetaMessage::Tempo(us)) if us.as_int() > 0 => {
                    tempos.insert(tick, 60_000_000.0 / f64::from(us.as_int()));
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, pow, _, _))
                    if num > 0 && pow <= 5 =>
                {
                    signatures.insert(
                        tick,
                        TimeSignature {
                            numerator: num,
                            denominator: 1 << pow,
                        },
                    );
                }
                TrackEventKind::Meta(MetaMessage::TrackName(t)) if track_name.is_none() => {
                    track_name = text(t);
                }
                TrackEventKind::Meta(MetaMessage::InstrumentName(t)) if instrument.is_none() => {
                    instrument = text(t);
                }
                _ => {}
            }
        }
        // Notes never released end with the track.
        for ((ch, key), ons) in open {
            for on in ons {
                close(&mut notes, ch, key, on, tick.max(on.0), 0.5);
            }
        }
        let name = track_name.or(instrument);
        let mut push = |name: Option<String>, channel: u8, mut n: Vec<MidiFileNote>| {
            n.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
            for x in &n {
                length = length.max(x.start.0 + x.duration.0);
            }
            tracks.push(MidiFileTrack {
                name,
                channel,
                notes: n,
            });
        };
        if split_channels {
            let single = notes.len() == 1;
            for (ch, n) in notes {
                push(if single { name.clone() } else { None }, ch, n);
            }
        } else if !notes.is_empty() {
            let all: Vec<MidiFileNote> = notes.into_values().flatten().collect();
            push(name, first_channel.unwrap_or(0), all);
        }
    }
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    Ok(MidiFileContents {
        name: if stem.is_empty() {
            name.to_string()
        } else {
            stem.to_string()
        },
        tracks,
        tempos: tempos
            .into_iter()
            .map(|(t, bpm)| MidiFileTempo {
                time: Beats(to_beats(t)),
                bpm,
            })
            .collect(),
        time_signatures: signatures
            .into_iter()
            .map(|(t, signature)| MidiFileTimeSignature {
                time: Beats(to_beats(t)),
                signature,
            })
            .collect(),
        length: Beats(length),
    })
}

fn text(bytes: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(bytes)
        .trim()
        .trim_matches('\0')
        .trim()
        .to_string();
    (!s.is_empty()).then_some(s)
}

/// Events at absolute ticks `(tick, order, event)` to a track: sorted (on a tick, order 0,
/// note offs, before order 1, note ons), delta-timed, closed with an end of track.
fn finish<'a>(mut evs: Vec<(u64, u8, TrackEventKind<'a>)>) -> Vec<TrackEvent<'a>> {
    evs.sort_by_key(|e| (e.0, e.1));
    let max = u64::from(u28::max_value().as_int());
    let mut last = 0u64;
    let mut out: Vec<TrackEvent<'a>> = evs
        .into_iter()
        .map(|(t, _, kind)| {
            let delta = u28::new((t - last).min(max) as u32);
            last = t;
            TrackEvent { delta, kind }
        })
        .collect();
    out.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}

/// One track of a written file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteTrack {
    pub name: String,
    /// MIDI channel 0..=15.
    pub channel: u8,
    /// Notes (beats from the file start; negative starts are dropped).
    pub notes: Vec<MidiFileNote>,
}

/// A type 1 Standard MIDI File at [`WRITE_PPQ`]: a tempo track (`tempos` and
/// `signatures`, beats from the file start; one at 0 is recommended), then `tracks`.
pub fn write(
    tracks: &[WriteTrack],
    tempos: &[MidiFileTempo],
    signatures: &[MidiFileTimeSignature],
) -> Vec<u8> {
    let ppq = f64::from(WRITE_PPQ);
    let tick = |b: f64| (b.max(0.0) * ppq).round() as u64;
    let names: Vec<Vec<u8>> = tracks.iter().map(|t| t.name.clone().into_bytes()).collect();
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(WRITE_PPQ)),
    ));
    let mut conductor = Vec::new();
    for t in tempos.iter().filter(|t| t.bpm > 0.0) {
        let us = (60_000_000.0 / t.bpm)
            .round()
            .clamp(1.0, f64::from(u24::max_value().as_int()));
        conductor.push((
            tick(t.time.0),
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us as u32))),
        ));
    }
    for s in signatures {
        let pow = s.signature.denominator.max(1).trailing_zeros() as u8;
        conductor.push((
            tick(s.time.0),
            0,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                s.signature.numerator.max(1),
                pow,
                24,
                8,
            )),
        ));
    }
    smf.tracks.push(finish(conductor));
    for (t, name) in tracks.iter().zip(&names) {
        let channel = u4::new(t.channel.min(15));
        let mut evs = vec![(0, 0, TrackEventKind::Meta(MetaMessage::TrackName(name)))];
        for n in t.notes.iter().filter(|n| n.start.0 >= 0.0) {
            let key = u7::new(n.pitch.min(127));
            let vel = u7::new(((n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8).max(1));
            let rel = u7::new((n.release_velocity.clamp(0.0, 1.0) * 127.0).round() as u8);
            let on = tick(n.start.0);
            let off = tick(n.start.0 + n.duration.0).max(on + 1);
            evs.push((
                on,
                1,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn { key, vel },
                },
            ));
            evs.push((
                off,
                0,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff { key, vel: rel },
                },
            ));
        }
        smf.tracks.push(finish(evs));
    }
    let mut out = Vec::new();
    smf.write_std(&mut out)
        .expect("writing to a Vec never fails");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, start: f64, duration: f64) -> MidiFileNote {
        MidiFileNote {
            pitch,
            velocity: 100.0 / 127.0,
            release_velocity: 64.0 / 127.0,
            start: Beats(start),
            duration: Beats(duration),
        }
    }

    #[test]
    fn write_then_read_round_trips_notes_tempo_and_signature() {
        let tracks = [
            WriteTrack {
                name: "lead".into(),
                channel: 0,
                notes: vec![note(60, 0.0, 1.0), note(64, 1.5, 0.25), note(60, 1.0, 0.5)],
            },
            WriteTrack {
                name: "bass".into(),
                channel: 1,
                notes: vec![note(36, 4.0, 2.0)],
            },
        ];
        let tempos = [
            MidiFileTempo {
                time: Beats(0.0),
                bpm: 90.0,
            },
            MidiFileTempo {
                time: Beats(8.0),
                bpm: 140.0,
            },
        ];
        let sigs = [MidiFileTimeSignature {
            time: Beats(0.0),
            signature: TimeSignature {
                numerator: 3,
                denominator: 4,
            },
        }];
        let bytes = write(&tracks, &tempos, &sigs);
        let file = read(&bytes, "song.mid").unwrap();
        assert_eq!(file.name, "song");
        assert_eq!(file.tracks.len(), 2);
        assert_eq!(file.tracks[0].name.as_deref(), Some("lead"));
        let notes: Vec<(u8, f64, f64)> = file.tracks[0]
            .notes
            .iter()
            .map(|n| (n.pitch, n.start.0, n.duration.0))
            .collect();
        assert_eq!(notes, [(60, 0.0, 1.0), (60, 1.0, 0.5), (64, 1.5, 0.25)]);
        assert!((file.tracks[0].notes[0].velocity - 100.0 / 127.0).abs() < 1e-6);
        assert_eq!(file.tracks[1].channel, 1);
        assert_eq!(file.tempos.len(), 2);
        assert!((file.tempos[0].bpm - 90.0).abs() < 0.01);
        assert!((file.tempos[1].bpm - 140.0).abs() < 0.01);
        assert_eq!(file.tempos[1].time, Beats(8.0));
        assert_eq!(
            file.time_signatures[0].signature,
            TimeSignature {
                numerator: 3,
                denominator: 4
            }
        );
        assert_eq!(file.length, Beats(6.0));
    }

    /// A hand-made type 0 file: two channels in one track, PPQ 96, a note on with velocity 0
    /// as the note off, and a note never released.
    #[test]
    fn type_0_splits_by_channel() {
        let mut b = b"MThd\0\0\0\x06\0\0\0\x01\0\x60".to_vec();
        let events: &[u8] = &[
            0x00, 0xFF, 0x03, 0x04, b'j', b'a', b'm', b'!', // track name
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // 120 bpm
            0x00, 0x90, 60, 100, // ch 0 C4 on
            0x00, 0x99, 36, 127, // ch 9 kick on
            0x30, 0x89, 36, 0, // kick off after half a beat
            0x30, 0x90, 60, 0, // C4 off (note on, velocity 0) at beat 1
            0x00, 0x90, 62, 80, // D4 on, never released
            0x60, 0xFF, 0x2F, 0x00, // end of track at beat 2
        ];
        b.extend_from_slice(b"MTrk");
        b.extend_from_slice(&(events.len() as u32).to_be_bytes());
        b.extend_from_slice(events);
        let file = read(&b, "jam.MID").unwrap();
        assert_eq!(file.name, "jam");
        assert_eq!(file.tracks.len(), 2);
        assert_eq!((file.tracks[0].channel, file.tracks[1].channel), (0, 9));
        // Two channels: no shared name.
        assert_eq!(file.tracks[0].name, None);
        let c: Vec<(u8, f64, f64)> = file.tracks[0]
            .notes
            .iter()
            .map(|n| (n.pitch, n.start.0, n.duration.0))
            .collect();
        assert_eq!(c, [(60, 0.0, 1.0), (62, 1.0, 1.0)]);
        assert_eq!(file.tracks[1].notes[0].duration, Beats(0.5));
        assert!((file.tempos[0].bpm - 120.0).abs() < 1e-9);
        assert_eq!(file.length, Beats(2.0));
    }

    #[test]
    fn rejects_other_bytes() {
        assert!(read(b"RIFF....WAVE", "a.mid").is_err());
    }
}

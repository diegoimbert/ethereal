//! Where detected notes land: source seconds → the clip's content beats (inverting the
//! engine's content → source mapping: warp markers or the reference tempo, repitch
//! transpose, reversal) → clip-relative beats through the clip's window (offset, loop).
//! Then the document edit of a finished job (one undo step).

use ether_core::graph::WarpDesc;
use ether_core::protocol::Command;
use ether_core::protocol::audio_to_midi::AudioToMidiMode;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::tracks::TrackCommand;
use ether_media::to_midi::DetectedNote;
use ether_model::derive_id;

use crate::doc::{self, DocCtx, clip_start};
use crate::tx::{CmdResult, invalid, not_found};

/// Shortest loop the engine plays (`ether_core::sched::MIN_LOOP`).
const MIN_LOOP: f64 = 1.0 / 256.0;
/// Shortest note kept after mapping (beats).
const MIN_NOTE_BEATS: f64 = 1e-4;

/// A note placed in the new clip (clip-relative beats).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    pub start: f64,
    pub duration: f64,
    pub pitch: u8,
    pub velocity: f32,
}

/// The engine's content beat → source seconds mapping of an audio clip, inverted.
struct SourceMap {
    warp: Option<WarpDesc>,
    ref_bpm: f64,
    /// Repitch: source seconds advance `rate` times faster than the map from `pivot` on.
    rate: f64,
    pivot: f64,
}

impl SourceMap {
    fn new(p: &Project, clip: &Clip, a: &AudioContent) -> Self {
        let warp = crate::warp::warp_desc(p, clip, a);
        let ref_bpm = p.tempo_map().bpm_at(clip_start(clip)).max(1e-6);
        let repitch = warp.as_ref().is_none_or(|w| w.mode == WarpMode::Repitch);
        let rate = if repitch && a.transpose != 0.0 {
            (a.transpose as f64 / 12.0).exp2()
        } else {
            1.0
        };
        let mut m = Self {
            warp,
            ref_bpm,
            rate: 1.0,
            pivot: 0.0,
        };
        m.pivot = m.forward(clip.offset.0);
        m.rate = rate;
        m
    }

    /// Source seconds before repitch at content beat `c` (`clip_editing::source_seconds`).
    fn forward(&self, c: f64) -> f64 {
        match &self.warp {
            Some(w) if w.markers.len() >= 2 => {
                let m = &w.markers;
                let i = m.partition_point(|&(b, _)| b <= c).clamp(1, m.len() - 1);
                let (b0, s0) = m[i - 1];
                let (b1, s1) = m[i];
                if b1 - b0 <= 0.0 {
                    s0
                } else {
                    s0 + (c - b0) * (s1 - s0) / (b1 - b0)
                }
            }
            _ => c * 60.0 / self.ref_bpm,
        }
    }

    /// Content beat at which source second `s` plays.
    fn content_beat(&self, s: f64) -> f64 {
        let s = self.pivot + (s - self.pivot) / self.rate;
        match &self.warp {
            Some(w) if w.markers.len() >= 2 => {
                let m = &w.markers;
                let i = m.partition_point(|&(_, src)| src <= s).clamp(1, m.len() - 1);
                let (b0, s0) = m[i - 1];
                let (b1, s1) = m[i];
                if (s1 - s0).abs() <= 1e-12 {
                    b0
                } else {
                    b0 + (s - s0) * (b1 - b0) / (s1 - s0)
                }
            }
            _ => s * self.ref_bpm / 60.0,
        }
    }
}

/// The linear pieces of a clip's window: `(clip-relative start, content start, content
/// end)` (the engine's `sched::for_each_piece`).
fn pieces(clip: &Clip) -> Vec<(f64, f64, f64)> {
    let length = clip.length.0.max(0.0);
    let offset = clip.offset.0;
    let (ls, le) = (clip.looping.start.0, clip.looping.end.0);
    if !(clip.looping.enabled && le - ls >= MIN_LOOP && offset < le) {
        return vec![(0.0, offset, offset + length)];
    }
    let mut out = Vec::new();
    let first = le - offset;
    out.push((0.0, offset, offset + first.min(length)));
    let len = le - ls;
    let mut at = first;
    while at < length - 1e-9 && out.len() < 100_000 {
        out.push((at, ls, ls + len.min(length - at)));
        at += len;
    }
    out
}

/// Map detected notes (source seconds of `media_seconds` of media) into clip-relative
/// beats, sorted by (start, pitch).
pub(crate) fn place(
    p: &Project,
    clip: &Clip,
    a: &AudioContent,
    media_seconds: f64,
    notes: &[DetectedNote],
) -> Vec<Placed> {
    let map = SourceMap::new(p, clip, a);
    let shift = a.transpose.round() as i32;
    let pieces = pieces(clip);
    let mut out = Vec::new();
    for n in notes {
        let (mut s0, mut s1) = (n.start, n.start + n.duration);
        if a.reversed {
            (s0, s1) = (media_seconds - s1, media_seconds - s0);
        }
        let (c0, c1) = (map.content_beat(s0), map.content_beat(s1));
        if !(c0.is_finite() && c1.is_finite()) || c1 <= c0 {
            continue;
        }
        let pitch = (n.pitch as i32 + shift).clamp(0, 127) as u8;
        for &(at, from, to) in &pieces {
            if c0 < from - 1e-9 || c0 >= to - 1e-9 {
                continue;
            }
            let duration = c1.min(to) - c0;
            if duration < MIN_NOTE_BEATS {
                continue;
            }
            out.push(Placed {
                start: at + (c0 - from),
                duration,
                pitch,
                velocity: n.velocity.clamp(0.0, 1.0),
            });
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    out.dedup_by(|b, a| a.pitch == b.pitch && (a.start - b.start).abs() < 1e-9);
    out
}

/// What the finished job writes.
pub(crate) struct Result<'a> {
    pub clip: ClipId,
    pub mode: AudioToMidiMode,
    pub track: TrackId,
    pub new_clip: ClipId,
    pub seed_notes: NoteId,
    pub instrument: Option<DeviceId>,
    pub notes: &'a [DetectedNote],
    pub media_seconds: f64,
}

/// Siblings order: the track right after `t` (for "below").
fn next_sibling(p: &Project, t: &Track) -> Option<TrackId> {
    let mut sibs: Vec<&Track> = p.tracks.values().filter(|x| x.parent == t.parent).collect();
    sibs.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
    let i = sibs.iter().position(|x| x.id == t.id)?;
    sibs.get(i + 1).map(|x| x.id)
}

/// The finished job's edit: a MIDI track right below the source track, a clip at the
/// source clip's position and length holding the notes, an optional default instrument.
/// Returns the number of notes written.
pub(crate) fn apply(ctx: &mut DocCtx, r: &Result<'_>) -> CmdResult<u32> {
    let clip = ctx
        .p()
        .clips
        .get(&r.clip)
        .cloned()
        .ok_or_else(|| not_found(format!("clip {}", r.clip)))?;
    let ClipContent::Audio(a) = &clip.content else {
        return Err(invalid(format!("clip {} is not an audio clip", clip.id)));
    };
    let source = ctx.track(clip.track)?;
    let placed = place(ctx.p(), &clip, a, r.media_seconds, r.notes);
    let before = next_sibling(ctx.p(), &source);
    let name = if clip.name.trim().is_empty() {
        source.name.clone()
    } else {
        clip.name.clone()
    };
    doc::apply(
        ctx,
        &Command::Track(TrackCommand::Create {
            id: r.track,
            kind: TrackKind::Midi,
            name: Some(format!("{name} MIDI")),
            color: Some(source.color),
            parent: source.parent,
            before,
        }),
    )?;
    doc::apply(
        ctx,
        &Command::Clip(ClipCommand::CreateMidi {
            id: r.new_clip,
            track: r.track,
            start: clip_start(&clip),
            length: clip.length,
            name: Some(name),
        }),
    )?;
    if !placed.is_empty() {
        let notes = placed
            .iter()
            .enumerate()
            .map(|(i, n)| NoteSpec {
                id: derive_id(r.seed_notes, i as u32),
                pitch: n.pitch,
                velocity: n.velocity,
                start: Beats(n.start),
                duration: Beats(n.duration),
            })
            .collect();
        doc::apply(
            ctx,
            &Command::Note(NoteCommand::Add {
                clip: r.new_clip,
                notes,
            }),
        )?;
    }
    if let Some(id) = r.instrument {
        let ty = match r.mode {
            AudioToMidiMode::Drums => BuiltinDeviceType::DrumRack,
            AudioToMidiMode::Melody | AudioToMidiMode::Harmony => BuiltinDeviceType::PolySynth,
        };
        doc::apply(
            ctx,
            &Command::Device(DeviceCommand::Insert {
                id,
                track: r.track,
                device: DeviceSpec::Builtin {
                    device: BuiltinDevice::new(ty),
                },
                before: None,
            }),
        )?;
    }
    Ok(placed.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(offset: f64, length: f64, looping: Option<(f64, f64)>) -> Clip {
        Clip {
            id: ClipId::NIL,
            track: TrackId::NIL,
            start: Beats(4.0),
            name: "c".into(),
            color: None,
            muted: false,
            length: Beats(length),
            offset: Beats(offset),
            looping: looping.map_or(ClipLoop::default(), |(s, e)| ClipLoop {
                enabled: true,
                start: Beats(s),
                end: Beats(e),
            }),
            content: ClipContent::Midi,
            lane: None,
        }
    }

    #[test]
    fn pieces_follow_the_engine() {
        assert_eq!(pieces(&clip(1.0, 4.0, None)), vec![(0.0, 1.0, 5.0)]);
        // Intro from the offset to the loop end, then the loop region.
        assert_eq!(
            pieces(&clip(1.0, 8.0, Some((0.0, 4.0)))),
            vec![(0.0, 1.0, 4.0), (3.0, 0.0, 4.0), (7.0, 0.0, 1.0)]
        );
        // Offset past the loop end: plays straight.
        assert_eq!(pieces(&clip(5.0, 2.0, Some((0.0, 4.0)))), vec![(0.0, 5.0, 7.0)]);
    }
}

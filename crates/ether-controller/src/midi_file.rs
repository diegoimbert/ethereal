//! Standard MIDI Files: `Media::ReadMidiFile` (a library file, an upload or a desktop
//! path → notes in beats; the UI makes the tracks and clips with ordinary commands, one undo
//! step) and `Media::WriteMidiFile` (MIDI clips → a type 1 file, written to a desktop path
//! or returned as bytes). The SMF codec is `ether_media::smf` (`midly`).

use ether_core::protocol::ErrorCode;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::media::{
    BrowseLocation, FileKind, MediaSource, MidiExportRequest, MidiExportResult, MidiFileNote,
    MidiFileTempo, MidiFileTimeSignature,
};
use ether_core::protocol::model::{
    Base64Bytes, Beats, Clip, ClipContent, LOOP_JOIN_EPS, Project, TrackId,
};
use ether_media::smf::{self, WriteTrack};

use crate::handlers::store_err;
use crate::store::{Library, ProjectStore, check_relative_path, file_kind};
use crate::tx::{CmdResult, cmd_err, invalid, not_found};
use crate::{EngineBridge, EtherController, HostServices};

/// Largest MIDI file read (real ones are kilobytes).
pub const MAX_MIDI_FILE_BYTES: usize = 16 << 20;

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A file name for `name` (no separators, control or reserved characters) with `.mid`.
pub fn midi_file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_start_matches('.').trim();
    format!("{}.mid", if clean.is_empty() { "midi" } else { clean })
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Media::ReadMidiFile`.
    pub fn read_midi_file(&mut self, source: &MediaSource) -> CmdResult<ReplyValue> {
        let (bytes, name) = match source {
            MediaSource::Location { location, path } => {
                check_relative_path(path).map_err(store_err)?;
                if path.is_empty() {
                    return Err(invalid("path must name a file"));
                }
                let bytes = match location {
                    BrowseLocation::Library { id } => self.library.read(id, path),
                    BrowseLocation::ProjectMedia => {
                        let pid = self
                            .doc
                            .as_ref()
                            .ok_or_else(|| invalid("no project is open"))?
                            .project
                            .id;
                        self.store.read(
                            pid,
                            &format!("{}/{path}", ether_core::protocol::model::file::MEDIA_DIR),
                        )
                    }
                }
                .map_err(store_err)?;
                (bytes, basename(path).to_string())
            }
            MediaSource::Project { .. } => {
                return Err(invalid("project media are audio, not MIDI files"));
            }
            MediaSource::Upload { upload } => {
                crate::upload::take_upload(&mut self.uploads, &mut self.store, upload)?
            }
            MediaSource::Path { path } => {
                crate::file_import::read_path_of(&mut self.library, path, FileKind::Midi)?
            }
        };
        if file_kind(&name) != FileKind::Midi {
            return Err(invalid(format!("not a MIDI file: {name}")));
        }
        if bytes.len() > MAX_MIDI_FILE_BYTES {
            return Err(invalid(format!(
                "{name} is larger than {} MiB",
                MAX_MIDI_FILE_BYTES >> 20
            )));
        }
        let file =
            smf::read(&bytes, &name).map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
        Ok(ReplyValue::MidiFile { file })
    }

    /// `Media::WriteMidiFile`.
    pub fn write_midi_file(&mut self, r: &MidiExportRequest) -> CmdResult<ReplyValue> {
        let p = &self
            .doc
            .as_ref()
            .ok_or_else(|| invalid("no project is open"))?
            .project;
        let bytes = midi_file_bytes(p, r)?;
        let name = midi_file_name(&r.name);
        if let Some(path) = &r.path {
            self.store
                .write_midi_file(path, &bytes)
                .map_err(store_err)?;
            return Ok(ReplyValue::MidiFileWritten {
                result: MidiExportResult {
                    name,
                    path: Some(path.clone()),
                    data: None,
                },
            });
        }
        Ok(ReplyValue::MidiFileWritten {
            result: MidiExportResult {
                name,
                path: None,
                data: Some(Base64Bytes(bytes)),
            },
        })
    }
}

/// Tracks in display order (depth first, groups before their children).
fn display_order(p: &Project) -> Vec<TrackId> {
    fn walk(p: &Project, ids: Vec<TrackId>, out: &mut Vec<TrackId>) {
        for id in ids {
            out.push(id);
            walk(p, p.child_tracks(id).iter().map(|t| t.id).collect(), out);
        }
    }
    let mut out = Vec::new();
    walk(
        p,
        p.tracks_ordered().iter().map(|t| t.id).collect(),
        &mut out,
    );
    out
}

/// The notes `clip` plays, at arrangement beats: its content through its offset and loop
/// (unrolled like the engine, `Clip::content_at`), cut at the loop wraps and the clip's end.
/// A note held across a wrap joins the loop's start note of the same pitch like in the
/// engine (`Clip::loop_join`: one longer note, the partner's release velocity). Muted notes
/// are left out.
fn played_notes(p: &Project, clip: &Clip) -> Vec<MidiFileNote> {
    let notes = p.notes_of(clip.id);
    let end = clip.length.0;
    let looping = clip.played_loop();
    // Pieces `(d, content start, content end)`: clip-relative time `d` plays the content
    // from `content start`.
    let mut pieces = Vec::new();
    if let Some((ls, le)) = looping {
        let mut d = 0.0;
        let mut from = clip.offset.0;
        // Bounded like the other loop unrollings (`freeze::edit::clip_pieces`).
        while d < end && pieces.len() < 100_000 {
            pieces.push((d, from, le));
            d += le - from;
            from = ls;
        }
    } else {
        pieces.push((0.0, clip.offset.0, clip.offset.0 + end));
    }
    let mut out = Vec::new();
    for (d, a, b) in pieces {
        for n in notes
            .iter()
            .filter(|n| !n.muted && n.start.0 >= a && n.start.0 < b)
        {
            let at = d + (n.start.0 - a);
            if at >= end {
                continue;
            }
            let mut duration = n.duration.0.min(end - at);
            let mut release_velocity = n.release_velocity;
            let wrap = d + (b - a);
            if looping.is_some() && n.start.0 + n.duration.0 > b + LOOP_JOIN_EPS && wrap < end {
                duration = wrap - at;
                if let Some((partner, after)) = clip.loop_join(notes.iter().copied(), n.pitch, wrap)
                {
                    duration += after;
                    release_velocity = partner.release_velocity;
                }
            }
            out.push(MidiFileNote {
                pitch: n.pitch,
                velocity: n.velocity,
                release_velocity,
                start: Beats(clip.start.0 + at),
                duration: Beats(duration),
            });
        }
    }
    out
}

/// The SMF bytes of `r` (see `Media::WriteMidiFile`).
pub fn midi_file_bytes(p: &Project, r: &MidiExportRequest) -> CmdResult<Vec<u8>> {
    if r.clips.is_empty() {
        return Err(invalid("no clips to export"));
    }
    let mut clips = Vec::new();
    for id in &r.clips {
        let c = p
            .clips
            .get(id)
            .ok_or_else(|| not_found(format!("clip {id}")))?;
        if !matches!(c.content, ClipContent::Midi) {
            return Err(invalid(format!("clip {id} is an audio clip")));
        }
        clips.push(c);
    }
    let origin = if r.from_project_start {
        0.0
    } else {
        clips
            .iter()
            .map(|c| c.start.0)
            .fold(f64::INFINITY, f64::min)
    };
    let mut tracks = Vec::new();
    for track in display_order(p) {
        let mut notes: Vec<MidiFileNote> = clips
            .iter()
            .filter(|c| c.track == track)
            .flat_map(|c| played_notes(p, c))
            .collect();
        if !clips.iter().any(|c| c.track == track) {
            continue;
        }
        for n in &mut notes {
            n.start = Beats(n.start.0 - origin);
        }
        notes.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
        let name = p
            .tracks
            .get(&track)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        // One channel per track, skipping channel 10 (index 9: drums in General MIDI).
        let i = (tracks.len() % 15) as u8;
        let channel = if i >= 9 { i + 1 } else { i };
        tracks.push(WriteTrack {
            name,
            channel,
            notes,
        });
    }
    // The tempo map from the origin: what is in effect there at 0, then the changes after.
    let map = p.tempo_map();
    let mut tempos = vec![MidiFileTempo {
        time: Beats(0.0),
        bpm: map.bpm_at(Beats(origin)),
    }];
    let mut points: Vec<_> = p
        .tempo_points
        .values()
        .filter(|t| t.time.0 > origin)
        .collect();
    points.sort_by(|a, b| a.time.0.total_cmp(&b.time.0));
    for t in points {
        tempos.push(MidiFileTempo {
            time: Beats(t.time.0 - origin),
            bpm: t.bpm,
        });
    }
    let mut signatures = vec![MidiFileTimeSignature {
        time: Beats(0.0),
        signature: map.signature_at(Beats(origin)),
    }];
    let mut sigs: Vec<_> = p
        .time_signatures
        .values()
        .filter(|s| s.time.0 > origin)
        .collect();
    sigs.sort_by(|a, b| a.time.0.total_cmp(&b.time.0));
    for s in sigs {
        signatures.push(MidiFileTimeSignature {
            time: Beats(s.time.0 - origin),
            signature: s.signature,
        });
    }
    Ok(smf::write(&tracks, &tempos, &signatures))
}

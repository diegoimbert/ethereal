//! Settings-like document commands: transport settings (loop, tempo, signature,
//! metronome), recording settings (monitor, input, count-in), warp, project rename.

use ether_core::protocol::model::*;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::warp::WarpCommand;

use super::DocCtx;
use crate::tx::{CmdResult, invalid, invalid_state, not_found, unsupported};

pub(crate) const MIN_BPM: f64 = 20.0;
pub(crate) const MAX_BPM: f64 = 999.0;

/// The tempo point in effect at `at`.
pub(crate) fn tempo_point_at(p: &Project, at: Beats) -> Option<TempoPoint> {
    p.tempo_map()
        .tempo
        .into_iter()
        .rev()
        .find(|t| t.time.0 <= at.0 + Beats::EPSILON)
        .or_else(|| p.tempo_map().tempo.into_iter().next())
}

fn signature_point_at(p: &Project, at: Beats) -> Option<TimeSignaturePoint> {
    p.tempo_map()
        .signatures
        .into_iter()
        .rev()
        .find(|t| t.time.0 <= at.0 + Beats::EPSILON)
        .or_else(|| p.tempo_map().signatures.into_iter().next())
}

pub(crate) fn check_signature(s: TimeSignature) -> CmdResult<()> {
    if s.numerator == 0 || s.numerator > 99 || !matches!(s.denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(invalid(format!(
            "invalid time signature {}/{}",
            s.numerator, s.denominator
        )));
    }
    Ok(())
}

pub(crate) fn transport(ctx: &mut DocCtx, c: &TransportCommand) -> CmdResult<()> {
    match c {
        TransportCommand::SetLoopEnabled { enabled } => {
            ctx.tx.settings(SettingsChange::LoopEnabled(*enabled))
        }
        TransportCommand::SetLoopRegion { region } => {
            if !(region.start.0.is_finite()
                && region.end.0.is_finite()
                && region.start.0 >= 0.0
                && region.end.0 > region.start.0)
            {
                return Err(invalid("invalid loop region"));
            }
            ctx.tx.settings(SettingsChange::LoopRegion(*region))
        }
        TransportCommand::SetTempo { bpm } => {
            if !bpm.is_finite() {
                return Err(invalid("bpm must be finite"));
            }
            let p = tempo_point_at(ctx.p(), ctx.position)
                .ok_or_else(|| invalid_state("no tempo point"))?;
            ctx.tx.update(EntityUpdate::TempoPoint {
                id: p.id,
                change: TempoPointChange::Bpm(bpm.clamp(MIN_BPM, MAX_BPM)),
            })
        }
        TransportCommand::SetTimeSignature { signature } => {
            check_signature(*signature)?;
            let p = signature_point_at(ctx.p(), ctx.position)
                .ok_or_else(|| invalid_state("no time signature point"))?;
            ctx.tx.update(EntityUpdate::TimeSignature {
                id: p.id,
                change: TimeSignatureChange::Signature(*signature),
            })
        }
        TransportCommand::SetMetronome { enabled } => {
            ctx.tx.settings(SettingsChange::Metronome(*enabled))
        }
        _ => Err(unsupported("not a document command")),
    }
}

pub(crate) fn recording(ctx: &mut DocCtx, c: &RecordingCommand) -> CmdResult<()> {
    match c {
        RecordingCommand::SetMonitor { track, monitor } => {
            ctx.track(*track)?;
            ctx.set_track(*track, TrackChange::Monitor(*monitor))
        }
        RecordingCommand::SetInput { track, input } => {
            ctx.track(*track)?;
            if let TrackInput::Track { track: src } = input {
                ctx.track(*src)?;
            }
            ctx.set_track(*track, TrackChange::Input(input.clone()))
        }
        RecordingCommand::SetCountIn { bars } => {
            ctx.tx.settings(SettingsChange::CountInBars((*bars).min(16)))
        }
        _ => Err(unsupported("not a document command")),
    }
}

fn check_audio(clip: &Clip) -> CmdResult<()> {
    match clip.content {
        ClipContent::Audio(_) => Ok(()),
        ClipContent::Midi => Err(invalid(format!("clip {} is not an audio clip", clip.id))),
    }
}

fn marker_values(beat: Beats, source: Seconds) -> CmdResult<(Beats, Seconds)> {
    if !(beat.0.is_finite() && source.0.is_finite()) {
        return Err(invalid("warp marker times must be finite"));
    }
    Ok((beat, Seconds(source.0.max(0.0))))
}

pub(crate) fn warp(ctx: &mut DocCtx, c: &WarpCommand) -> CmdResult<()> {
    match c {
        WarpCommand::SetWarp { clip, warp } => {
            check_audio(&ctx.clip(*clip)?)?;
            ctx.set_clip(*clip, ClipChange::Warp(*warp))
        }
        WarpCommand::AddMarker {
            id,
            clip,
            beat,
            source,
        } => {
            if ctx.p().warp_markers.contains_key(id) {
                return Ok(());
            }
            check_audio(&ctx.clip(*clip)?)?;
            let (beat, source) = marker_values(*beat, *source)?;
            ctx.tx.insert(Entity::WarpMarker(WarpMarker {
                id: *id,
                clip: *clip,
                beat,
                source,
            }))
        }
        WarpCommand::MoveMarker { id, beat, source } => {
            if !ctx.p().warp_markers.contains_key(id) {
                return Err(not_found(format!("warp marker {id}")));
            }
            let (beat, source) = marker_values(*beat, *source)?;
            ctx.tx.update(EntityUpdate::WarpMarker {
                id: *id,
                change: WarpMarkerChange::Beat(beat),
            })?;
            ctx.tx.update(EntityUpdate::WarpMarker {
                id: *id,
                change: WarpMarkerChange::Source(source),
            })
        }
        WarpCommand::RemoveMarker { id } => {
            if !ctx.p().warp_markers.contains_key(id) {
                return Err(not_found(format!("warp marker {id}")));
            }
            ctx.tx.remove(EntityKey::WarpMarker(*id))
        }
        WarpCommand::DetectTempo { .. } => Err(unsupported("not a document command")),
    }
}

pub(crate) fn rename_project(ctx: &mut DocCtx, name: &str) -> CmdResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("project name must not be empty"));
    }
    ctx.tx.settings(SettingsChange::Name(name.to_string()))
}

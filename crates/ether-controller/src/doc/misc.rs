//! Settings-like document commands: transport settings (loop, tempo, signature,
//! metronome), recording settings (monitor, input, count-in), warp, project rename.

use ether_core::protocol::model::*;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::warp::WarpCommand;

use super::DocCtx;
use crate::tx::{CmdResult, invalid, unsupported};

pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 999.0;

/// The tempo point in effect at `at`.
pub fn tempo_point_at(p: &Project, at: Beats) -> Option<TempoPoint> {
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

pub fn check_signature(s: TimeSignature) -> CmdResult<()> {
    if s.numerator == 0 || s.numerator > 99 || !matches!(s.denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(invalid(format!(
            "invalid time signature {}/{}",
            s.numerator, s.denominator
        )));
    }
    Ok(())
}

pub fn transport(ctx: &mut DocCtx, c: &TransportCommand) -> CmdResult<()> {
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
            let p = tempo_point_at(ctx.p(), ctx.position).ok_or_else(|| {
                crate::tx::keyed_err(
                    ether_core::protocol::ErrorCode::InvalidState,
                    crate::text::msg("error.doc.no_tempo_point"),
                )
            })?;
            ctx.tx.update(EntityUpdate::TempoPoint {
                id: p.id,
                change: TempoPointChange::Bpm(bpm.clamp(MIN_BPM, MAX_BPM)),
            })
        }
        TransportCommand::SetTimeSignature { signature } => {
            check_signature(*signature)?;
            let p = signature_point_at(ctx.p(), ctx.position).ok_or_else(|| {
                crate::tx::keyed_err(
                    ether_core::protocol::ErrorCode::InvalidState,
                    crate::text::msg("error.doc.no_time_signature_point"),
                )
            })?;
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

pub fn recording(ctx: &mut DocCtx, c: &RecordingCommand) -> CmdResult<()> {
    match c {
        RecordingCommand::SetMonitor { track, monitor } => {
            ctx.track(*track)?;
            ctx.set_track(*track, TrackChange::Monitor(*monitor))
        }
        RecordingCommand::SetInput { track, input } => {
            ctx.track(*track)?;
            if let TrackInput::Track { track: src, .. } = input {
                ctx.track(*src)?;
            }
            ctx.set_track(*track, TrackChange::Input(input.clone()))
        }
        RecordingCommand::SetCountIn { bars } => ctx
            .tx
            .settings(SettingsChange::CountInBars((*bars).min(16))),
        _ => Err(unsupported("not a document command")),
    }
}

pub fn warp(ctx: &mut DocCtx, c: &WarpCommand) -> CmdResult<()> {
    crate::warp::command(ctx, c)
}

pub fn rename_project(ctx: &mut DocCtx, name: &str) -> CmdResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("project name must not be empty"));
    }
    ctx.tx.settings(SettingsChange::Name(name.to_string()))
}

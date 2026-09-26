//! `MixerCommand`s.

use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;

use super::DocCtx;
use crate::tx::{CmdResult, invalid};
use crate::{MAX_SEND_DB, MAX_VOLUME_DB, SILENCE_DB};

fn db(v: Decibels, max: f32) -> CmdResult<Decibels> {
    if v.0.is_nan() {
        return Err(invalid("level must be a number"));
    }
    Ok(Decibels(v.0.clamp(SILENCE_DB, max)))
}

fn set_send(ctx: &mut DocCtx, id: SendId, change: SendChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::Send { id, change })
}

pub(super) fn apply(ctx: &mut DocCtx, c: &MixerCommand) -> CmdResult<()> {
    match c {
        MixerCommand::SetVolume { track, volume } => {
            ctx.track(*track)?;
            let v = db(*volume, MAX_VOLUME_DB)?;
            ctx.set_track(*track, TrackChange::Volume(v))
        }
        MixerCommand::SetPan { track, pan } => {
            ctx.track(*track)?;
            if pan.0.is_nan() {
                return Err(invalid("pan must be a number"));
            }
            ctx.set_track(*track, TrackChange::Pan(Pan(pan.0.clamp(-1.0, 1.0))))
        }
        MixerCommand::SetMute { track, mute } => {
            ctx.track(*track)?;
            ctx.set_track(*track, TrackChange::Mute(*mute))
        }
        MixerCommand::SetSolo {
            track,
            solo,
            exclusive,
        } => {
            ctx.track(*track)?;
            if *exclusive {
                let others: Vec<TrackId> = ctx
                    .p()
                    .tracks
                    .values()
                    .filter(|t| t.id != *track && t.mixer.solo)
                    .map(|t| t.id)
                    .collect();
                for o in others {
                    ctx.set_track(o, TrackChange::Solo(false))?;
                }
            }
            ctx.set_track(*track, TrackChange::Solo(*solo))
        }
        MixerCommand::SetOutput { track, output } => {
            let t = ctx.track(*track)?;
            if let TrackOutput::Track { track: dest } = output {
                let d = ctx.track(*dest)?;
                if d.id == t.id {
                    return Err(invalid("a track cannot output to itself"));
                }
                if !matches!(d.kind, TrackKind::Group | TrackKind::Return) {
                    return Err(invalid("outputs can only target group or return tracks"));
                }
            }
            ctx.set_track(t.id, TrackChange::Output(output.clone()))
        }
        MixerCommand::CreateSend {
            id,
            from,
            to,
            level,
            pre_fader,
        } => {
            if ctx.p().sends.contains_key(id) {
                return Ok(());
            }
            let f = ctx.track(*from)?;
            let t = ctx.track(*to)?;
            if t.kind != TrackKind::Return {
                return Err(invalid("sends must target a return track"));
            }
            if f.id == t.id || f.kind == TrackKind::Master {
                return Err(invalid("invalid send source"));
            }
            // A second send on the same route (double-sent keystroke): no-op.
            if ctx
                .p()
                .sends
                .values()
                .any(|s| s.from == f.id && s.to == t.id)
            {
                return Ok(());
            }
            ctx.tx.insert(Entity::Send(TrackSend {
                id: *id,
                from: f.id,
                to: t.id,
                level: db(*level, MAX_SEND_DB)?,
                pre_fader: *pre_fader,
            }))
        }
        MixerCommand::SetSendLevel { send, level } => {
            ctx.send(*send)?;
            let v = db(*level, MAX_SEND_DB)?;
            set_send(ctx, *send, SendChange::Level(v))
        }
        MixerCommand::SetSendPreFader { send, pre_fader } => {
            ctx.send(*send)?;
            set_send(ctx, *send, SendChange::PreFader(*pre_fader))
        }
        MixerCommand::DeleteSend { send } => {
            ctx.send(*send)?;
            ctx.delete_send(*send)
        }
    }
}

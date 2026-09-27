//! Tempo map editing and metronome settings (roadmap v2, owned by the `tempo-metronome`
//! node; see `docs/ROADMAP.md` and `ether_protocol::tempo`).
//!
//! `TempoCommand` is a document command (dispatched from `doc::apply`), so every command is
//! one undo step (a drag sends `EditTempoPoint`s under one gesture). The click itself is
//! rendered by `ether_core::metronome`; [`metronome_desc`] compiles the settings into the
//! render graph, and the record session sets `MetronomeDesc::count_in_end` on the
//! published desc during its pre-roll (`EngineState::count_in_end`).
//!
//! Validation (on top of the model's invariants):
//! - times are finite and `>= 0`; the points at beat 0 can be edited but not moved or
//!   removed; two tempo points (or two time signatures) never share a position;
//! - BPM is clamped to `MIN_BPM..=MAX_BPM`; metronome volume to -144..=+6 dB;
//! - a time-signature change must fall on a bar line of the signature in effect before it
//!   (bars restart at every change, as in the engine). The rule holds after every edit:
//!   changing, moving, adding or removing a signature is **rejected** (`InvalidArgument`)
//!   if a later change would end up off its bar line; later changes are never moved
//!   implicitly (move or remove them first).

use ether_core::graph::MetronomeDesc;
use ether_core::protocol::model::*;
use ether_core::protocol::tempo::TempoCommand;

use crate::doc::{DocCtx, MAX_BPM, MIN_BPM};
use crate::tx::{CmdResult, invalid, not_found};

/// Metronome volume range (dB).
pub(crate) const MIN_METRONOME_DB: f32 = -144.0;
pub(crate) const MAX_METRONOME_DB: f32 = 6.0;

pub(crate) fn apply(ctx: &mut DocCtx, c: &TempoCommand) -> CmdResult<()> {
    match c {
        TempoCommand::AddTempoPoint {
            id,
            time,
            bpm,
            curve,
        } => {
            if ctx.p().tempo_points.contains_key(id) {
                return Ok(());
            }
            check_time("tempo point", *time)?;
            let bpm = check_bpm(*bpm)?;
            check_free_tempo_time(ctx.p(), *time, None)?;
            ctx.tx.insert(Entity::TempoPoint(TempoPoint {
                id: *id,
                time: *time,
                bpm,
                curve: *curve,
            }))
        }
        TempoCommand::EditTempoPoint {
            id,
            time,
            bpm,
            curve,
        } => {
            let p = ctx
                .p()
                .tempo_points
                .get(id)
                .cloned()
                .ok_or_else(|| not_found(format!("tempo point {id}")))?;
            if let Some(t) = *time
                && !t.approx_eq(p.time)
            {
                check_time("tempo point", t)?;
                if p.time.approx_eq(Beats::ZERO) {
                    return Err(invalid("the tempo point at beat 0 cannot be moved"));
                }
                check_free_tempo_time(ctx.p(), t, Some(*id))?;
                ctx.tx.update(EntityUpdate::TempoPoint {
                    id: *id,
                    change: TempoPointChange::Time(t),
                })?;
            }
            if let Some(b) = *bpm {
                let b = check_bpm(b)?;
                if b != p.bpm {
                    ctx.tx.update(EntityUpdate::TempoPoint {
                        id: *id,
                        change: TempoPointChange::Bpm(b),
                    })?;
                }
            }
            if let Some(c) = *curve
                && c != p.curve
            {
                ctx.tx.update(EntityUpdate::TempoPoint {
                    id: *id,
                    change: TempoPointChange::Curve(c),
                })?;
            }
            Ok(())
        }
        TempoCommand::RemoveTempoPoints { ids } => {
            for id in ids {
                let p = ctx
                    .p()
                    .tempo_points
                    .get(id)
                    .ok_or_else(|| not_found(format!("tempo point {id}")))?;
                if p.time.approx_eq(Beats::ZERO) {
                    return Err(invalid("the tempo point at beat 0 cannot be removed"));
                }
                ctx.tx.remove(EntityKey::TempoPoint(*id))?;
            }
            Ok(())
        }
        TempoCommand::AddTimeSignature {
            id,
            time,
            signature,
        } => {
            if ctx.p().time_signatures.contains_key(id) {
                return Ok(());
            }
            check_time("time signature", *time)?;
            check_signature(*signature)?;
            check_signature_time(ctx.p(), *time, None)?;
            ctx.tx.insert(Entity::TimeSignature(TimeSignaturePoint {
                id: *id,
                time: *time,
                signature: *signature,
            }))?;
            check_later_bar_lines(ctx.p(), *time)
        }
        TempoCommand::EditTimeSignature {
            id,
            time,
            signature,
        } => {
            let p = ctx
                .p()
                .time_signatures
                .get(id)
                .cloned()
                .ok_or_else(|| not_found(format!("time signature {id}")))?;
            if let Some(s) = *signature {
                check_signature(s)?;
            }
            if let Some(t) = *time
                && !t.approx_eq(p.time)
            {
                check_time("time signature", t)?;
                if p.time.approx_eq(Beats::ZERO) {
                    return Err(invalid("the time signature at beat 0 cannot be moved"));
                }
                check_signature_time(ctx.p(), t, Some(*id))?;
                ctx.tx.update(EntityUpdate::TimeSignature {
                    id: *id,
                    change: TimeSignatureChange::Time(t),
                })?;
            }
            if let Some(s) = *signature
                && s != p.signature
            {
                ctx.tx.update(EntityUpdate::TimeSignature {
                    id: *id,
                    change: TimeSignatureChange::Signature(s),
                })?;
            }
            check_later_bar_lines(ctx.p(), Beats(p.time.0.min(time.unwrap_or(p.time).0)))
        }
        TempoCommand::RemoveTimeSignatures { ids } => {
            let mut from = f64::INFINITY;
            for id in ids {
                let p = ctx
                    .p()
                    .time_signatures
                    .get(id)
                    .ok_or_else(|| not_found(format!("time signature {id}")))?;
                if p.time.approx_eq(Beats::ZERO) {
                    return Err(invalid("the time signature at beat 0 cannot be removed"));
                }
                from = from.min(p.time.0);
                ctx.tx.remove(EntityKey::TimeSignature(*id))?;
            }
            check_later_bar_lines(ctx.p(), Beats(from))
        }
        TempoCommand::SetMetronomeSettings {
            volume,
            accent,
            sound,
        } => {
            let s = ctx.p().settings.clone();
            if let Some(v) = *volume {
                if !v.0.is_finite() {
                    return Err(invalid("metronome volume must be finite"));
                }
                let v = Decibels(v.0.clamp(MIN_METRONOME_DB, MAX_METRONOME_DB));
                if v != s.metronome_volume {
                    ctx.tx.settings(SettingsChange::MetronomeVolume(v))?;
                }
            }
            if let Some(a) = *accent
                && a != s.metronome_accent
            {
                ctx.tx.settings(SettingsChange::MetronomeAccent(a))?;
            }
            if let Some(snd) = *sound
                && snd != s.metronome_sound
            {
                ctx.tx.settings(SettingsChange::MetronomeSound(snd))?;
            }
            Ok(())
        }
    }
}

fn check_time(what: &str, t: Beats) -> CmdResult<()> {
    if !t.0.is_finite() || t.0 < 0.0 {
        return Err(invalid(format!("{what} time must be finite and >= 0")));
    }
    Ok(())
}

fn check_bpm(bpm: f64) -> CmdResult<f64> {
    if !bpm.is_finite() {
        return Err(invalid("bpm must be finite"));
    }
    Ok(bpm.clamp(MIN_BPM, MAX_BPM))
}

fn check_signature(s: TimeSignature) -> CmdResult<()> {
    if s.numerator == 0 || s.numerator > 99 || !matches!(s.denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(invalid(format!(
            "invalid time signature {}/{}",
            s.numerator, s.denominator
        )));
    }
    Ok(())
}

/// No other tempo point (than `except`) sits at `t`.
fn check_free_tempo_time(p: &Project, t: Beats, except: Option<TempoPointId>) -> CmdResult<()> {
    if p.tempo_points
        .values()
        .any(|q| Some(q.id) != except && q.time.approx_eq(t))
    {
        return Err(invalid(format!(
            "there is already a tempo point at beat {}",
            t.0
        )));
    }
    Ok(())
}

/// `t` is free (no other change there) and on a bar line of the signature in effect just
/// before it, ignoring `except` (the point being moved).
fn check_signature_time(p: &Project, t: Beats, except: Option<TimeSignatureId>) -> CmdResult<()> {
    let mut sigs: Vec<&TimeSignaturePoint> = p
        .time_signatures
        .values()
        .filter(|s| Some(s.id) != except)
        .collect();
    if sigs.iter().any(|s| s.time.approx_eq(t)) {
        return Err(invalid(format!(
            "there is already a time signature at beat {}",
            t.0
        )));
    }
    sigs.sort_by(|a, b| a.time.0.total_cmp(&b.time.0));
    let Some(prev) = sigs.iter().rev().find(|s| s.time.0 < t.0) else {
        return Ok(());
    };
    if !on_bar_line(prev, t) {
        return Err(invalid(format!(
            "a time signature change must fall on a bar line ({}/{} from beat {})",
            prev.signature.numerator, prev.signature.denominator, prev.time.0
        )));
    }
    Ok(())
}

/// Bar length in quarter-note beats.
pub(crate) fn bar_length(s: TimeSignature) -> f64 {
    f64::from(s.numerator.max(1)) * 4.0 / f64::from(s.denominator.max(1))
}

/// Every time-signature change at or after `from` still falls on a bar line of the one
/// before it. Edits that would break this (changing, moving, adding or removing an earlier
/// signature) are rejected rather than moving later changes. Changes before `from`
/// (untouched by the edit, e.g. from an older document) are not checked.
fn check_later_bar_lines(p: &Project, from: Beats) -> CmdResult<()> {
    let mut sigs: Vec<&TimeSignaturePoint> = p.time_signatures.values().collect();
    sigs.sort_by(|a, b| a.time.0.total_cmp(&b.time.0));
    for w in sigs.windows(2) {
        let (prev, s) = (w[0], w[1]);
        if s.time.0 + Beats::EPSILON < from.0 {
            continue;
        }
        if !on_bar_line(prev, s.time) {
            return Err(invalid(format!(
                "the time signature change at beat {} would no longer fall on a bar line \
                 ({}/{} from beat {}); move or remove it first",
                s.time.0, prev.signature.numerator, prev.signature.denominator, prev.time.0
            )));
        }
    }
    Ok(())
}

fn on_bar_line(prev: &TimeSignaturePoint, t: Beats) -> bool {
    let bar = bar_length(prev.signature);
    let bars = (t.0 - prev.time.0) / bar;
    ((bars - bars.round()) * bar).abs() <= Beats::EPSILON
}

/// Click settings for the render graph (`count_in_end` is set on the published desc by
/// the controller while a record count-in runs: `EngineState::count_in_end`).
pub(crate) fn metronome_desc(s: &ProjectSettings) -> MetronomeDesc {
    MetronomeDesc {
        volume: s.metronome_volume.to_linear(),
        accent: s.metronome_accent,
        sound: s.metronome_sound,
        count_in_end: None,
    }
}

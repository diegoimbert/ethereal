//! Groove: humanize, quantize swing and the project playback swing (roadmap v2, owned by
//! the `groove` node; see `docs/ROADMAP.md` and `ether_protocol::groove`).
//!
//! - `GrooveCommand` is a document command ([`apply`], from `doc::apply`).
//! - `NoteCommand::Quantize { swing }` is handled in `doc/notes.rs` (shared touch), with
//!   [`swing_delay`].
//! - [`swing_notes`] is called by `compile.rs` for every MIDI clip: it applies the project
//!   swing (`ProjectSettings::swing`/`swing_grid`) to the compiled notes.
//!
//! The MockTransport mirrors all of this (`ui/src/transport/mock`): same PRNG (mulberry32),
//! same note order (by id), same swing rule, so a UI preview matches the engine exactly.

use ether_core::graph::NoteDesc;
use ether_core::protocol::groove::GrooveCommand;
use ether_core::protocol::model::*;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid};

pub(crate) fn apply(ctx: &mut DocCtx, c: &GrooveCommand) -> CmdResult<()> {
    match c {
        GrooveCommand::Humanize {
            clip,
            notes,
            timing,
            velocity,
            seed,
        } => humanize(ctx, *clip, notes.as_deref(), *timing, *velocity, *seed),
        GrooveCommand::SetSwing { amount, grid } => {
            if !amount.is_finite() {
                return Err(invalid("swing amount must be finite"));
            }
            if !(grid.0.is_finite() && grid.0 > 0.0) {
                return Err(invalid("swing grid must be > 0"));
            }
            let amount = amount.clamp(0.0, 1.0);
            let s = &ctx.p().settings;
            let (old_amount, old_grid) = (s.swing, s.swing_grid);
            if amount != old_amount {
                ctx.tx.settings(SettingsChange::Swing(amount))?;
            }
            if !grid.approx_eq(old_grid) {
                ctx.tx.settings(SettingsChange::SwingGrid(*grid))?;
            }
            Ok(())
        }
    }
}

fn humanize(
    ctx: &mut DocCtx,
    clip: ClipId,
    ids: Option<&[NoteId]>,
    timing: Beats,
    velocity: f32,
    seed: u32,
) -> CmdResult<()> {
    ctx.clip(clip)?;
    if !timing.0.is_finite() || !velocity.is_finite() {
        return Err(invalid("humanize amounts must be finite"));
    }
    let timing = timing.0.max(0.0);
    let velocity = f64::from(velocity).max(0.0);
    // Deterministic order (by id), independent of positions, so the same seed on the same
    // notes always gives the same result.
    let mut targets: Vec<Note> = ctx
        .p()
        .notes_of(clip)
        .into_iter()
        .filter(|n| ids.is_none_or(|ids| ids.contains(&n.id)))
        .cloned()
        .collect();
    targets.sort_by_key(|n| n.id);
    let mut rand = Mulberry32::new(seed);
    for n in targets {
        let dt = (rand.next() * 2.0 - 1.0) * timing;
        let dv = (rand.next() * 2.0 - 1.0) * velocity;
        let start = Beats((n.start.0 + dt).max(0.0));
        let vel = (f64::from(n.velocity) + dv).clamp(0.0, 1.0) as f32;
        if !start.approx_eq(n.start) {
            ctx.tx.update(EntityUpdate::Note {
                id: n.id,
                change: NoteChange::Start(start),
            })?;
        }
        if vel != n.velocity {
            ctx.tx.update(EntityUpdate::Note {
                id: n.id,
                change: NoteChange::Velocity(vel),
            })?;
        }
    }
    Ok(())
}

/// Small deterministic PRNG, bit-identical to `mulberry32` in `ui/src/transport/mock/random.ts`.
pub(crate) struct Mulberry32(u32);

impl Mulberry32 {
    pub(crate) fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// Next float in `[0, 1)`.
    pub(crate) fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// Swing delay for a position `t` on `grid`: `swing · grid / 3` when `t` sits (within
/// `Beats::EPSILON`) on an odd multiple of `grid`, else 0. `swing` is clamped to 0..=1
/// (non-finite = 0).
pub(crate) fn swing_delay(t: f64, grid: f64, swing: f32) -> f64 {
    if !(swing.is_finite() && swing > 0.0 && grid.is_finite() && grid > 0.0) {
        return 0.0;
    }
    let k = (t / grid).round();
    if (t - k * grid).abs() > Beats::EPSILON || k.rem_euclid(2.0) != 1.0 {
        return 0.0;
    }
    f64::from(swing.min(1.0)) * grid / 3.0
}

/// Apply the project playback swing to a clip's notes (content-relative beats; `offset`
/// = the clip's content offset, so the grid is aligned to the arrangement when the clip
/// starts on a grid line). Keeps `notes` sorted by start.
pub(crate) fn swing_notes(settings: &ProjectSettings, offset: f64, notes: &mut [NoteDesc]) {
    let grid = settings.swing_grid.0;
    let active = settings.swing > 0.0 && grid > 0.0;
    if !active {
        return;
    }
    let mut moved = false;
    for n in notes.iter_mut() {
        let d = swing_delay(n.start - offset, grid, settings.swing);
        if d > 0.0 {
            n.start += d;
            moved = true;
        }
    }
    if moved {
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulberry32_matches_the_js_reference() {
        // Values from `mulberry32(42)` in ui/src/transport/mock/random.ts.
        let mut r = Mulberry32::new(42);
        let got: Vec<f64> = (0..3).map(|_| r.next()).collect();
        assert_eq!(got, JS_MULBERRY32_42);
    }

    const JS_MULBERRY32_42: [f64; 3] =
        [0.6011037519201636, 0.44829055899754167, 0.8524657934904099];

    #[test]
    fn swing_delay_hits_odd_positions_only() {
        assert_eq!(swing_delay(0.0, 0.5, 1.0), 0.0);
        assert!((swing_delay(0.5, 0.5, 1.0) - 0.5 / 3.0).abs() < 1e-12);
        assert_eq!(swing_delay(1.0, 0.5, 1.0), 0.0);
        assert!((swing_delay(1.5 + 1e-9, 0.5, 0.6) - 0.1).abs() < 1e-6);
        assert_eq!(swing_delay(0.6, 0.5, 1.0), 0.0);
        assert!((swing_delay(-0.5, 0.5, 1.0) - 0.5 / 3.0).abs() < 1e-12);
        assert!((swing_delay(0.5, 0.5, 7.0) - 0.5 / 3.0).abs() < 1e-12);
        assert_eq!(swing_delay(0.5, 0.5, f32::NAN), 0.0);
    }
}

//! Arrangement automation and clip envelopes applied per sub-block (v0.2: extracted from
//! `engine.rs` by contracts-3, owned by the `sample-accurate-automation` node; CONTRACTS.md
//! §12.7).
//!
//! Current behaviour (v0.1, moved verbatim): mixer targets get their smoother target at the
//! sub-block start; node params get `Param` events every [`AUTOMATION_STEP`] samples from the
//! **sub-block start**, so results depend on the block size and on where sub-blocks split.
//! `sample-accurate-automation` replaces this with the frozen contract: events on an absolute
//! grid (`sample_time` multiples of [`PARAM_GRID`]) plus one at every breakpoint, values at
//! the exact beat of each event (exact tempo-ramp integration through `Timing`), and mixer
//! targets ramped per sample, so offline renders are identical for every block size.

use crate::automation::{gain_from_plain, to_plain};
use crate::event::{EventKind, ProcessEvent};
use crate::graph::{AutomationDesc, ResolvedTarget};
use crate::sched::Timing;

/// Node-param automation is re-evaluated every this many samples within a sub-block (v0.1).
pub(crate) const AUTOMATION_STEP: usize = 64;
/// The frozen v0.2 automation grid (samples of the absolute engine clock).
pub const PARAM_GRID: u64 = 32;

/// Apply one automation lane for the current sub-block. Mixer targets get their smoother
/// target set from the lane at the sub-block start (every sub-block, so a manual move never
/// sticks while a lane is enabled); node params get `Param` events every
/// [`AUTOMATION_STEP`] samples while playing when the value differs from the last one
/// sent (`last` is reset to NaN to force a re-send).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_automation(
    lane: &AutomationDesc,
    value_at: impl Fn(f64) -> Option<f64>,
    timing: &Timing<'_>,
    playing: bool,
    last: &mut f64,
    volume: &mut crate::param::Smoother,
    pan: &mut crate::param::Smoother,
    sends: &mut [crate::mixer::SendRt],
    chain: &mut [crate::mixer::ChainRt],
    racks: &mut crate::drum_rack::RacksRt,
    chain_racks: &mut crate::rack_chains::ChainRacksRt,
    modulation: &mut crate::modulation::ModulationRt,
) {
    match lane.resolved {
        ResolvedTarget::TrackVolume | ResolvedTarget::TrackPan | ResolvedTarget::Send { .. } => {
            let Some(v) = value_at(timing.b0) else {
                return;
            };
            let plain = to_plain(&lane.mapping, v);
            let drive = |s: &mut crate::param::Smoother, target: f32| {
                if s.target() != target {
                    s.set_target(target);
                }
            };
            match lane.resolved {
                ResolvedTarget::TrackVolume => {
                    drive(volume, gain_from_plain(&lane.mapping, plain));
                }
                ResolvedTarget::TrackPan => drive(pan, plain.clamp(-1.0, 1.0) as f32),
                ResolvedTarget::Send { send } => {
                    if let Some(s) = sends.iter_mut().find(|s| s.id == send) {
                        drive(&mut s.level, gain_from_plain(&lane.mapping, plain));
                    }
                }
                ResolvedTarget::Node { .. } => {}
            }
        }
        ResolvedTarget::Node { node, param } => {
            // Track-chain node, a device on a drum pad or on a rack chain.
            let events = match chain.iter_mut().find(|c| c.key == node) {
                Some(entry) => &mut entry.events,
                None => match racks.events_mut(node) {
                    Some(events) => events,
                    None => match chain_racks.events_mut(node) {
                        Some(events) => events,
                        None => return,
                    },
                },
            };
            let steps = if playing { timing.frames } else { 1 };
            let mut o = 0;
            while o < steps {
                if let Some(v) = value_at(timing.beat_at(o as f64))
                    && v != *last
                {
                    *last = v;
                    let plain = to_plain(&lane.mapping, v);
                    // Modulated params: automation sets the base (`crate::modulation`).
                    if !modulation.intercept(node, param, plain) {
                        events.push(ProcessEvent {
                            offset: o as u32,
                            kind: EventKind::Param {
                                param,
                                value: plain,
                            },
                        });
                    }
                }
                o += AUTOMATION_STEP;
            }
        }
    }
}

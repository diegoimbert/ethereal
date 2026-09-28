//! VCA faders (v0.2, owned by the `groups-buses` node; CONTRACTS.md §12.10).
//!
//! VCA tracks (`TrackKind::Vca`) carry no audio: the controller compiles them into
//! `RenderGraphDesc::vcas` (never into `tracks`), and every assigned track's desc names its VCA
//! (`TrackDesc::vca`).
//!
//! # Contract
//! - Gain: an assigned track's effective fader gain is its own fader × the VCA gain × the
//!   VCA's own VCA gain ... (dB add up the VCA chain). Applied in the track's job right after
//!   the fader ([`TrackVcaRt::apply`], pre-wired in `engine.rs`), smoothed like faders.
//!   Post-fader sends and taps see it; pre-fader sends don't.
//! - Mute: a muted VCA (or muted VCA ancestor) mutes its tracks (their mute/solo gate, see
//!   `SnapshotRt::update_gates`, pre-wired to consult [`TrackVcaRt::muted`]). Live VCA fader
//!   and mute changes arrive as `ParamTarget::{TrackVolume, TrackMute} { track: vca }`;
//!   [`VcaRt::set_param`] handles them (pre-wired as the fallback when the id is not a track).
//! - Solo: the controller folds VCA solo into the assigned tracks' `solo` when compiling
//!   (solo changes republish the graph anyway).
//! - Automation of a VCA's volume: `VcaDesc::automation`, evaluated by [`VcaRt::update`]
//!   before the track jobs. An enabled lane drives the VCA fader (like a track's volume
//!   lane); without automation the fader value is the live one. While playing it is
//!   sample-accurate like track volume lanes (CONTRACTS.md §12.7,
//!   `crate::automation_rt`): each assigned track's VCA gain ramps per sample to the
//!   effective gain at every automation grid point ([`TrackVcaRt::knots`]), so renders don't
//!   depend on the block size. Stopped, the lane is evaluated at the position.
//!
//! Everything on the audio thread is allocation-free: the per-VCA and per-track tables are
//! sized at compile time.

use ether_protocol::model::TrackId;
use serde::{Deserialize, Serialize};

use crate::automation::{evaluate, gain_from_plain, to_plain};
use crate::config::EngineConfig;
use crate::graph::{AutomationDesc, TrackDesc};
use crate::mixer::{MIX_RAMP_MS, Stereo, TrackRt};
use crate::param::Smoother;
use crate::sched::Timing;

/// A VCA fader.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VcaDesc {
    pub id: TrackId,
    /// Linear gain of the VCA fader.
    pub volume: f32,
    pub mute: bool,
    /// The VCA this VCA is assigned to.
    pub parent: Option<TrackId>,
    /// Volume automation of the VCA (`ResolvedTarget::TrackVolume`).
    pub automation: Vec<AutomationDesc>,
}

/// Runtime state of one VCA.
#[derive(Debug)]
struct VcaState {
    /// Current fader value (linear; live changes and automation land here).
    volume: f32,
    mute: bool,
    /// Index of the parent VCA.
    parent: Option<usize>,
    automation: Vec<AutomationDesc>,
}

/// All VCAs of a snapshot (in `SnapshotRt`).
#[derive(Debug, Default)]
pub(crate) struct VcaRt {
    vcas: Vec<VcaState>,
    /// Sorted `(id, index into vcas)`.
    index: Vec<(TrackId, usize)>,
    /// `track_vca[i]` = the VCA (index into `vcas`) of track `i`.
    track_vca: Vec<Option<usize>>,
}

/// Effective (gain, muted) of VCA `v` in `vcas` (product of the chain; cycles are cut).
fn effective(vcas: &[VcaState], v: usize) -> (f32, bool) {
    let mut gain = 1.0f32;
    let mut muted = false;
    let mut cur = Some(v);
    let mut steps = 0;
    while let Some(i) = cur {
        gain *= vcas[i].volume.max(0.0);
        muted |= vcas[i].mute;
        cur = vcas[i].parent;
        steps += 1;
        if steps > vcas.len() {
            break;
        }
    }
    (gain, muted)
}

/// Linear gain of a VCA's volume lane at `beat` (the last lane wins: the controller compiles
/// at most one volume lane per VCA).
fn lane_gain(automation: &[AutomationDesc], beat: f64) -> Option<f32> {
    let mut out = None;
    for lane in automation {
        if let Some(x) = evaluate(&lane.points, beat) {
            out = Some(gain_from_plain(&lane.mapping, to_plain(&lane.mapping, x)));
        }
    }
    out
}

/// Some VCA in the chain of `v` is automated.
fn chain_automated(vcas: &[VcaState], v: usize) -> bool {
    let mut cur = Some(v);
    let mut steps = 0;
    while let Some(i) = cur {
        if !vcas[i].automation.is_empty() {
            return true;
        }
        cur = vcas[i].parent;
        steps += 1;
        if steps > vcas.len() {
            break;
        }
    }
    false
}

/// Effective gain of VCA `v` at `beat`: automated VCAs of the chain at their lane's value.
fn effective_at(vcas: &[VcaState], v: usize, beat: f64) -> f32 {
    let mut gain = 1.0f32;
    let mut cur = Some(v);
    let mut steps = 0;
    while let Some(i) = cur {
        let volume = lane_gain(&vcas[i].automation, beat).unwrap_or(vcas[i].volume);
        gain *= volume.max(0.0);
        cur = vcas[i].parent;
        steps += 1;
        if steps > vcas.len() {
            break;
        }
    }
    gain
}

/// Effective (gain, muted) of `vca` in a compiled desc (non-RT; unknown ids are ignored).
pub(crate) fn effective_of_desc(vcas: &[VcaDesc], vca: Option<TrackId>) -> (f32, bool) {
    let mut gain = 1.0f32;
    let mut muted = false;
    let mut cur = vca;
    let mut steps = 0;
    while let Some(id) = cur {
        let Some(v) = vcas.iter().find(|v| v.id == id) else {
            break;
        };
        gain *= v.volume.max(0.0);
        muted |= v.mute;
        cur = v.parent;
        steps += 1;
        if steps > vcas.len() {
            break;
        }
    }
    (gain, muted)
}

impl VcaRt {
    /// Non-RT (graph compile). `tracks[i].vca` = the VCA id of track `i`.
    pub(crate) fn compile(vcas: &[VcaDesc], tracks: &[TrackDesc], config: &EngineConfig) -> Self {
        let _ = config;
        let mut index: Vec<(TrackId, usize)> =
            vcas.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
        index.sort();
        let find = |id: TrackId| {
            index
                .binary_search_by(|e| e.0.cmp(&id))
                .ok()
                .map(|k| index[k].1)
        };
        let states = vcas
            .iter()
            .map(|v| VcaState {
                volume: v.volume.max(0.0),
                mute: v.mute,
                parent: v.parent.and_then(find),
                automation: v
                    .automation
                    .iter()
                    .filter(|a| !a.points.is_empty())
                    .cloned()
                    .collect(),
            })
            .collect();
        let track_vca = tracks.iter().map(|t| t.vca.and_then(find)).collect();
        Self {
            vcas: states,
            index,
            track_vca,
        }
    }

    fn lookup(&self, id: TrackId) -> Option<usize> {
        self.index
            .binary_search_by(|e| e.0.cmp(&id))
            .ok()
            .map(|k| self.index[k].1)
    }

    /// RT. Push the effective gain and mute of every VCA into its assigned tracks.
    /// Returns `true` if any track's VCA mute changed.
    fn push(&self, tracks: &mut [TrackRt]) -> bool {
        let mut mute_changed = false;
        for (i, v) in self.track_vca.iter().enumerate() {
            let Some(v) = *v else { continue };
            let Some(t) = tracks.get_mut(i) else { continue };
            let (gain, muted) = effective(&self.vcas, v);
            if t.vca.gain.target() != gain {
                t.vca.gain.set_target(gain);
            }
            if t.vca.muted != muted {
                t.vca.muted = muted;
                mute_changed = true;
            }
        }
        mute_changed
    }

    /// RT. A live `TrackVolume`/`TrackMute` change for `id`; `Some(gates_dirty)` when `id`
    /// is a VCA.
    pub(crate) fn set_param(
        &mut self,
        id: TrackId,
        mute: Option<bool>,
        volume: Option<f32>,
        tracks: &mut [TrackRt],
    ) -> Option<bool> {
        let v = self.lookup(id)?;
        if let Some(m) = mute {
            self.vcas[v].mute = m;
        }
        if let Some(vol) = volume {
            self.vcas[v].volume = vol.max(0.0);
        }
        Some(self.push(tracks))
    }

    /// RT. Once per sub-block before the track jobs: evaluate VCA automation and push the
    /// resulting gains into the assigned tracks' [`TrackVcaRt`].
    pub(crate) fn update(&mut self, tracks: &mut [TrackRt], timing: &Timing<'_>, playing: bool) {
        if self.vcas.is_empty() {
            return;
        }
        for t in tracks.iter_mut() {
            t.vca.knots.clear();
        }
        let automated = self.vcas.iter().any(|v| !v.automation.is_empty());
        if playing && automated {
            // Sample-accurate: the effective gain of each track's VCA chain at every grid
            // knot; the track's job ramps to them ([`TrackVcaRt::apply`]).
            for (i, v) in self.track_vca.iter().enumerate() {
                let (Some(v), Some(t)) = (*v, tracks.get_mut(i)) else {
                    continue;
                };
                if !chain_automated(&self.vcas, v) {
                    continue;
                }
                for (start, end, knot, beat) in crate::automation_rt::chunks(timing) {
                    let gain = effective_at(&self.vcas, v, beat);
                    if t.vca.knots.len() < t.vca.knots.capacity() {
                        t.vca
                            .knots
                            .push((start as u32, end as u32, (knot - start) as u32, gain));
                    }
                }
            }
            // The fader values follow the automation (live changes and stops start there).
            let end = timing.beat_at(timing.frames as f64);
            for v in self.vcas.iter_mut() {
                if let Some(g) = lane_gain(&v.automation, end) {
                    v.volume = g;
                }
            }
        } else {
            for v in self.vcas.iter_mut() {
                if let Some(g) = lane_gain(&v.automation, timing.b0) {
                    v.volume = g;
                }
            }
        }
        // Mute never changes here (no mute automation): gates stay as they are. Tracks with
        // knots take their gain from them, `push` only sets the smoother target.
        let _ = self.push(tracks);
    }

    /// RT. Live fader/mute values survive a snapshot swap (the new desc carries the
    /// document's values, which the live queue already applied to the old snapshot).
    pub(crate) fn inherit(&mut self, old: &mut VcaRt) {
        // Nothing runs across a swap: the fader smoothing lives in the tracks
        // ([`TrackVcaRt::inherit`]).
        let _ = old;
    }
}

/// Per assigned track (in `TrackRt`).
#[derive(Debug)]
pub(crate) struct TrackVcaRt {
    /// Muted through a VCA.
    pub muted: bool,
    /// Effective VCA gain (linear), smoothed like faders.
    pub gain: Smoother,
    /// Assigned to a VCA (else `apply` is a no-op).
    pub assigned: bool,
    /// This sub-block's automation ramps (sample-accurate, set by [`VcaRt::update`]):
    /// `(start, end, ramp length, gain)` = ramp to `gain` over `ramp length` samples from
    /// `start`, applied on `start..end`. Empty = plain smoothing. Capacity fixed at compile.
    pub knots: Vec<(u32, u32, u32, f32)>,
}

impl Default for TrackVcaRt {
    fn default() -> Self {
        Self {
            muted: false,
            gain: Smoother::new(1.0, MIX_RAMP_MS, 48_000.0),
            assigned: false,
            knots: Vec::new(),
        }
    }
}

impl TrackVcaRt {
    /// Non-RT (graph compile): the initial effective gain and mute of track `vca`.
    pub(crate) fn compile(vca: Option<TrackId>, vcas: &[VcaDesc], config: &EngineConfig) -> Self {
        let (gain, muted) = effective_of_desc(vcas, vca);
        let assigned = vca.is_some_and(|id| vcas.iter().any(|v| v.id == id));
        Self {
            muted: assigned && muted,
            gain: Smoother::new(
                if assigned { gain } else { 1.0 },
                MIX_RAMP_MS,
                config.sample_rate as f32,
            ),
            assigned,
            // One chunk per grid interval of the largest block, plus the partial ends.
            knots: if assigned {
                Vec::with_capacity(
                    config.max_block_size / crate::automation_rt::PARAM_GRID as usize + 2,
                )
            } else {
                Vec::new()
            },
        }
    }

    /// RT. Apply the VCA gain to the post-fader signal.
    pub(crate) fn apply(&mut self, a: &mut Stereo, frames: usize) {
        if !self.assigned && !self.gain.is_smoothing() && self.gain.current() == 1.0 {
            return;
        }
        let [l, r] = a;
        if self.knots.is_empty() {
            for (sl, sr) in l[..frames].iter_mut().zip(r[..frames].iter_mut()) {
                let g = self.gain.tick();
                *sl *= g;
                *sr *= g;
            }
            return;
        }
        for &(start, end, len, gain) in &self.knots {
            self.gain.ramp_to(gain, len);
            let (start, end) = (start as usize, (end as usize).min(frames));
            for (sl, sr) in l[start..end].iter_mut().zip(r[start..end].iter_mut()) {
                let g = self.gain.tick();
                *sl *= g;
                *sr *= g;
            }
        }
    }

    /// RT. Ramp from where the old snapshot's gain was to the new target.
    pub(crate) fn inherit(&mut self, old: &mut TrackVcaRt) {
        let target = self.gain.target();
        self.gain = old.gain;
        if self.gain.current() != target || self.gain.target() != target {
            self.gain.set_target(target);
        }
    }
}

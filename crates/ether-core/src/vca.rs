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
//! - Automation of a VCA's volume: `VcaDesc::automation`, evaluated once per sub-block by
//!   [`VcaRt::update`] before the track jobs.

use ether_protocol::model::TrackId;
use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::graph::AutomationDesc;
use crate::mixer::{Stereo, TrackRt};
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

/// All VCAs of a snapshot (in `SnapshotRt`).
#[derive(Debug, Default)]
pub(crate) struct VcaRt {}

impl VcaRt {
    /// Non-RT (graph compile). `track_vca[i]` = the VCA id of track `i`.
    pub(crate) fn compile(vcas: &[VcaDesc], config: &EngineConfig) -> Self {
        let _ = (vcas, config);
        Self::default()
    }

    /// RT. A live `TrackVolume`/`TrackMute` change for `id`; `Some(gates_dirty)` when `id`
    /// is a VCA (placeholder: never).
    pub(crate) fn set_param(
        &mut self,
        id: TrackId,
        mute: Option<bool>,
        volume: Option<f32>,
        tracks: &mut [TrackRt],
    ) -> Option<bool> {
        let _ = (id, mute, volume, tracks);
        None
    }

    /// RT. Once per sub-block before the track jobs: evaluate VCA automation and push the
    /// resulting gains into the assigned tracks' [`TrackVcaRt`] (placeholder: no-op).
    pub(crate) fn update(&mut self, tracks: &mut [TrackRt], timing: &Timing<'_>, playing: bool) {
        let _ = (tracks, timing, playing);
    }

    pub(crate) fn inherit(&mut self, old: &mut VcaRt) {
        let _ = old;
    }
}

/// Per assigned track (in `TrackRt`).
#[derive(Debug, Default)]
pub(crate) struct TrackVcaRt {
    /// Muted through a VCA.
    pub muted: bool,
}

impl TrackVcaRt {
    /// RT. Apply the VCA gain to the post-fader signal (placeholder: unity).
    pub(crate) fn apply(&mut self, a: &mut Stereo, frames: usize) {
        let _ = (a, frames);
    }

    pub(crate) fn inherit(&mut self, old: &mut TrackVcaRt) {
        let _ = old;
    }
}

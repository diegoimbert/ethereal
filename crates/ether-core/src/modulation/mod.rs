//! Engine-side modulation (v0.2, owned by the `racks-modulation` node; model
//! `ether_model::modulation`, composition rule frozen in CONTRACTS.md §12.6).
//!
//! The controller compiles every track's modulators and mappings into
//! `TrackDesc::modulation` ([`ModulationDesc`]). Modulators run in the engine (not in the
//! host node): LFOs/steps/random from the transport, envelopes from the notes reaching the
//! host node, envelope followers from the audio at the host's input.
//!
//! Hooks (pre-wired in `engine.rs` / `crate::automation_rt`; placeholders until the node
//! lands):
//! - [`ModulationRt::intercept`]: every base-value write for a node param (live changes from
//!   the param queue, arrangement automation, clip envelopes, macro knob changes) goes
//!   through it first. `true` = the param is modulated (or is a macro): the value was stored
//!   as the **base** and the engine does not forward it; the modulation output will.
//! - [`ModulationRt::render`]: once per sub-block after automation, before the device chain:
//!   compute the sources and push `EventKind::Param` events with
//!   `clamp(base + Σ depth·m, 0, 1)` (mapped to plain) into the targets' event lists on the
//!   automation grid (CONTRACTS.md §12.7), only when the value changes.
//! - [`ModulationRt::pre_node`]: before chain entry `k` processes (envelope followers read
//!   the host's input audio; envelopes read the entry's note events).
//! - [`readback`]: at the analysis rate after the jobs, push `AnalysisKind::Modulation`
//!   frames (normalized base/effective per modulated param) for the depth rings.
//! - Live modulator param changes: `ParamTarget::Modulator` → [`ModulationRt::set_param`].

use ether_protocol::model::{ModulatorId, ModulatorKind, ParamId};
use serde::{Deserialize, Serialize};

use crate::analysis::AnalysisRt;
use crate::config::EngineConfig;
use crate::drum_rack::RacksRt;
use crate::graph::ParamMapping;
use crate::mixer::{ChainRt, Stereo, TrackRt};
use crate::node::NodeKey;
use crate::rack_chains::ChainRacksRt;
use crate::sched::Timing;
use crate::transport::TransportInfo;

/// A track's modulation (empty for most tracks).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModulationDesc {
    pub modulators: Vec<ModulatorDesc>,
    pub mappings: Vec<ModMappingDesc>,
}

impl ModulationDesc {
    pub fn is_empty(&self) -> bool {
        self.modulators.is_empty() && self.mappings.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModulatorDesc {
    pub id: ModulatorId,
    /// The host device's node (on this track's chain, a pad chain or a rack chain).
    pub host: NodeKey,
    pub kind: ModulatorKind,
    /// Plain values of every param of the kind (defaults filled in by the controller).
    pub params: Vec<(ParamId, f64)>,
    /// Envelope followers: follow this track's sidechain tap instead of the host input
    /// (ordered first and tapped by `graph.rs`, read in the consumer's job via
    /// [`ModulationRt::write_sidechain`]).
    #[serde(default)]
    pub sidechain: Option<ether_protocol::model::TrackId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModSourceDesc {
    /// Index into `ModulationDesc::modulators`.
    Modulator(u32),
    /// Macro `index` of the rack node `rack` (its param `index`, plain 0..=1).
    Macro { rack: NodeKey, index: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModMappingDesc {
    pub source: ModSourceDesc,
    /// Target node and param.
    pub node: NodeKey,
    pub param: ParamId,
    /// `-1..=1` (normalized units).
    pub depth: f64,
    /// Normalized → plain mapping of the target param.
    pub mapping: ParamMapping,
    /// Normalized base value at compile time (the document value; automation overrides it
    /// through `intercept`).
    pub base: f64,
}

/// Per-track modulation state (in `TrackRt`).
#[derive(Debug, Default)]
pub(crate) struct ModulationRt {
    /// Track indices of envelope-follower sidechain sources (distinct, sorted).
    sidechain_sources: Vec<usize>,
}

impl ModulationRt {
    /// Non-RT (graph compile). `sidechain_sources`: track indices of the envelope-follower
    /// sidechain sources of this track (finished before its job; their `tap` is kept).
    pub(crate) fn compile(
        desc: &ModulationDesc,
        sidechain_sources: Vec<usize>,
        config: &EngineConfig,
    ) -> Self {
        let _ = (desc, config);
        Self { sidechain_sources }
    }

    /// Envelope-follower sidechain sources (track indices).
    pub(crate) fn sidechain_sources(&self) -> &[usize] {
        &self.sidechain_sources
    }

    /// RT. At the start of the consumer's job: the post-fader tap of source track `source`
    /// (latency `out_lat(source)`), for the followers keyed from it. PDC: aligned to the host
    /// device's input like a device sidechain when the source is earlier (`L_sc <= L`: delay
    /// the tap by `L - L_sc`); when it is later the modulation lags by `L_sc - L` (a control
    /// signal never delays the audio). Placeholder: ignored.
    pub(crate) fn write_sidechain(&mut self, source: usize, tap: &Stereo, n: usize) {
        let _ = (source, tap, n);
    }

    /// RT. Carry source state (LFO phases, envelope stages) over a snapshot swap.
    pub(crate) fn inherit(&mut self, old: &mut ModulationRt) {
        let _ = old;
    }

    /// RT. A base value (plain) for `node`/`param`; `true` = consumed (see the module docs).
    pub(crate) fn intercept(&mut self, node: NodeKey, param: ParamId, plain: f64) -> bool {
        let _ = (node, param, plain);
        false
    }

    /// RT. A live modulator param change; `true` when the modulator is on this track.
    pub(crate) fn set_param(&mut self, modulator: ModulatorId, param: ParamId, plain: f64) -> bool {
        let _ = (modulator, param, plain);
        false
    }

    /// RT. Compute the sources for the sub-block and emit the targets' `Param` events.
    pub(crate) fn render(
        &mut self,
        timing: &Timing<'_>,
        info: &TransportInfo,
        chain: &mut [ChainRt],
        racks: &mut RacksRt,
        chain_racks: &mut ChainRacksRt,
    ) {
        let _ = (timing, info, chain, racks, chain_racks);
    }

    /// RT. Before chain entry `k` (node `key`) processes `a`.
    pub(crate) fn pre_node(
        &mut self,
        k: usize,
        key: NodeKey,
        events: &[crate::ProcessEvent],
        a: &Stereo,
        n: usize,
    ) {
        let _ = (k, key, events, a, n);
    }
}

/// RT. Push modulation readback frames for every track (placeholder: none).
pub(crate) fn readback(tracks: &mut [TrackRt], analysis: &mut AnalysisRt) {
    let _ = (tracks, analysis);
}

//! Frozen-track playback (v0.2, owned by the `freeze-bounce` node; CONTRACTS.md §12.3).
//!
//! A frozen track's desc carries [`FrozenDesc`] (`TrackDesc::frozen`); the controller then
//! compiles it **without clips and without device chain** (its nodes are not instantiated).
//! The engine calls [`render_frozen`] where clips would render (pre-wired in `engine.rs`,
//! once per sub-block, playing only): it adds the render at song time into the track's
//! buffer, before the (empty) chain, fader, sends and meters.
//!
//! Mapping: output sample `o` of the sub-block (`TransportInfo::seconds` = song time of
//! sample 0) reads media frame `round((seconds + o / sample_rate - start_seconds) ·
//! media_rate)`; the media is resampled to the engine rate by the controller when loaded
//! (like every source), so at equal rates this is an integer offset. Outside the media:
//! silence. Loop wraps and locates need no state (position-addressed reads).

use std::sync::Arc;

use ether_protocol::model::MediaId;
use serde::{Deserialize, Serialize};

use crate::media::AudioSource;
use crate::mixer::Stereo;
use crate::transport::TransportInfo;

/// A frozen track's render (`Track::freeze`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrozenDesc {
    /// The render (registered with `EngineHandle::add_source` like clip media).
    pub media: MediaId,
    /// Song time of the media's first frame (seconds).
    pub start_seconds: f64,
}

/// RT. Add the frozen render for the sub-block into `out[..][..frames]`. Returns `false` on a
/// source underrun (counted like clip underruns). Placeholder until `freeze-bounce` lands:
/// silent.
pub(crate) fn render_frozen(
    desc: &FrozenDesc,
    sources: &[(MediaId, Arc<dyn AudioSource>)],
    info: &TransportInfo,
    sample_rate: f64,
    out: &mut Stereo,
    frames: usize,
) -> bool {
    let _ = (desc, sources, info, sample_rate, out, frames);
    true
}

//! Render-graph snapshot codec (roadmap v2, owned by the `web-perf` node; see
//! `docs/ROADMAP.md`).
//!
//! The web host ships every published [`RenderGraphDesc`] from the controller Worker to the
//! AudioWorklet (today as JSON, `ether-wasm/src/proto.rs`). `web-perf` replaces that with a
//! compact binary encoding behind this trait; both ends of a connection must use the same
//! codec. Contract:
//!
//! - `decode(encode(d)) == d` for every desc (exact `f64`s, ids, ordering).
//! - `encode` runs on the controller side (may allocate); `decode` runs off the audio
//!   thread (the worklet decodes before compiling, like today).
//! - Encoded data starts with a version byte; `decode` rejects unknown versions with
//!   [`CodecError::Version`] instead of misreading.

use crate::graph::RenderGraphDesc;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CodecError {
    #[error("unsupported snapshot encoding version {0}")]
    Version(u8),
    #[error("malformed snapshot: {0}")]
    Malformed(String),
}

/// Encodes/decodes render-graph snapshots for transfer between threads/instances.
pub trait GraphCodec {
    /// Append the encoding of `desc` to `out`.
    fn encode(&self, desc: &RenderGraphDesc, out: &mut Vec<u8>);
    fn decode(&self, bytes: &[u8]) -> Result<RenderGraphDesc, CodecError>;
}

//! Warped audio-clip playback (owned by the `warp` wave-3 node; see `docs/WAVE3.md`).
//!
//! Today `sched::render_audio` plays warped clips ([`crate::graph::WarpDesc`]) by
//! resampling (Repitch semantics) even in `WarpMode::Complex`. This module is where the
//! pitch-preserving path goes: per-clip [`crate::Stretcher`] (`ether-stretch`) state,
//! preallocated off the audio thread, driven from `sched` with a one-line call. RT rules
//! apply (no alloc/locks/logging in `process`). Must still compile on wasm32 (web falls
//! back to unwarped/repitched playback if the stretcher is unavailable).

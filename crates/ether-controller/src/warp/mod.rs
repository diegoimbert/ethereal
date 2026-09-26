//! Controller-side warp logic (owned by the `warp` wave-3 node; see `docs/WAVE3.md`).
//!
//! Warp-marker editing helpers beyond the basic `WarpCommand` handling in
//! `doc/misc.rs`, BPM detection (stub) on imported audio, and anything the compile step
//! (`compile.rs::warp_desc` → `ether_core::graph::WarpDesc`) needs to delegate here.

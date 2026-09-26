//! Engine-side recording (owned by the `recording` wave-3 node; see `docs/WAVE3.md`).
//!
//! Capture of the hardware input already passed to `Engine::process(inputs, ..)` for
//! armed tracks (`TrackDesc::{audio_input, monitor, armed}`) into preallocated,
//! lock-free buffers drained by the host (disk writer natively), plus recorded MIDI
//! events, with the timeline position and round-trip latency needed for
//! latency-compensated placement. RT rules apply; must compile on wasm32.

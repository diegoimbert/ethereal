//! Engine-side drum racks (roadmap v2, owned by the `drum-rack` node; see
//! `docs/ROADMAP.md`).
//!
//! Runs the pad chains described by `TrackDesc::racks` (`graph::RackDesc`): per-pad event
//! routing by key (transposed to `PAD_PLAY_NOTE`), choke groups (`NoteChoke` to the other
//! pads of the group), pad chain processing with PDC alignment to the longest pad chain,
//! and the pad mix (volume/pan/mute, smoothed) into the rack node's input. Per-pad runtime
//! state (buffers, smoothers, event buffers) is preallocated in `graph::compile_with` and
//! inherited across snapshot swaps like `TrackRt`. The mixer calls into this module when it
//! reaches a chain entry whose node is a rack (one-line hook in `mixer.rs`/`engine.rs`).
//! RT rules apply; must compile on wasm32.

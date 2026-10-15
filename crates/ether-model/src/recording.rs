//! Recording data types (owned by the `recording` wave-3 node; see `docs/WAVE3.md`).
//!
//! Home for document-level recording types that don't fit the existing entities, e.g.
//! take metadata or a record-session description turned into ops when a take is committed
//! (audio clip + `MediaRef` for recorded audio, MIDI clip + notes for recorded MIDI).
//! Per-track settings (`Track::input`, monitor mode, count-in) already live in [`crate::track`]
//! / [`crate::project`]; changing those frozen types goes through a BCR.

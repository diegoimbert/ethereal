//! Controller-side recording (owned by the `recording` wave-3 node; see `docs/WAVE3.md`).
//!
//! Record sessions: `RecordingCommand` handling beyond arm/transport (currently in
//! `handlers.rs::recording_command`; the controller holds the armed set in
//! `EtherController::armed`), committing takes as one undo step (audio clip + media,
//! MIDI clip + notes) with latency-compensated placement, and `RecordingEvent`s.
//! Host input access goes through a bridge/host trait added via BCR.

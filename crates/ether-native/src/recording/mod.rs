//! Native audio/MIDI input capture (owned by the `recording` wave-3 node; see
//! `docs/WAVE3.md`).
//!
//! cpal input streams feeding `Engine::process(inputs, ..)` (see [`crate::audio`], which
//! currently opens output only and reports `inputs: Vec::new()`), MIDI input via `midir`,
//! input listing for `RecordingCommand::ListInputs`, and the disk writer for recorded
//! takes.

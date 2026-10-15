//! Plugin-related model helpers (owned by the `plugins` wave-3 node; see `docs/WAVE3.md`).
//!
//! The frozen plugin types (`PluginInstance`, incl. `sandboxed` and the saved `state`
//! blob) live in [`crate::device`]. This module is for pure helpers around them, e.g.
//! plugin-state/PDC bookkeeping or validation used by the controller when inserting a
//! plugin on a device chain. Changing the frozen types goes through a BCR.

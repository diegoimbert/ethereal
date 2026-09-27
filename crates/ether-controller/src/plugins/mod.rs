//! Controller-side plugin logic (owned by the `plugins` wave-3 node; see `docs/WAVE3.md`).
//!
//! Inserting plugins on device chains, the per-plugin sandbox toggle
//! (`PluginCommand::SetSandboxed`, currently in `handlers.rs`), plugin state capture into
//! `PluginInstance.state` via `EngineBridge::plugin_state`, and PDC-related republishes.
//! Dispatch from `handlers.rs` should be a one-line call into this module.

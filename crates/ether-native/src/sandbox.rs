//! Out-of-process plugin hosting (owned by the `plugins` wave-3 node; see `docs/WAVE3.md`).
//!
//! Wires `ether-sandbox` into `NativeBridge::create_plugin` (`bridge.rs`), which today
//! logs a warning and loads `PluginInstance { sandboxed: true, .. }` in-process. The
//! sandboxed node reports +1 block latency (PDC) and bypasses itself on helper crash.

//! Browser build: wasm-bindgen entry points.
//!
//! Two instances of this module run in the browser:
//! - [`WasmController`] in a Web Worker: `ether-controller` with an OPFS-backed
//!   `ProjectStore` + `Library` ([`store`]; engine-side, the UI thread never touches OPFS)
//!   and a [`bridge::WebBridge`] that serializes engine calls into a SharedArrayBuffer ring;
//! - [`WasmEngine`] in the AudioWorklet: owns `ether_core::Engine` + `EngineHandle` + GC
//!   ([`worklet::EngineHost`]), drains the ring between blocks, and renders in `process()`.
//!
//! The two wasm instances share no memory: they exchange serialized messages over two SAB
//! rings ([`ring`], [`proto`]): control (graph snapshots, params, transport, nodes, decoded
//! media) Worker → Worklet, and reports (playhead, meters, diagnostics) Worklet → Worker.
//! The Worker turns reports into `Playhead`/`Meters` messages for the UI in `tick`.
//! Messages to/from the UI are JSON-encoded `ClientMessage`/`ServerMessage`. Requires
//! COOP/COEP (`apps/web` sets them). No plugins on the web. Owned by the `wasm-host` node.

pub mod bridge;
pub mod latency;
pub mod media_stream;
pub mod perf;
pub mod proto;
pub mod ring;
pub mod store;
pub mod web;
pub mod worklet;

pub use web::{WasmController, WasmEngine};

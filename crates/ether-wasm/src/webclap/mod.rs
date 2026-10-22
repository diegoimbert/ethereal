//! WebCLAP hosting (web version only; `docs/PLUGIN-FORMATS.md` "WebCLAP").
//!
//! A WebCLAP is a CLAP plugin compiled to wasm32. The browser runs each plugin instance as
//! its own WebAssembly instance (its own memory) next to the engine, in the AudioWorklet;
//! the JS host glue (`apps/web/src/engine/webclap/`) implements the CLAP ABI. This module
//! is the Rust side:
//! - [`catalog`]: the installed plugins (scanned by the page) → descriptors;
//! - [`node`]: [`node::WebclapNode`], the engine node, which hands each block to the
//!   realm's [`guest::Guest`] (the JS host, registered by `WasmEngine`/`WasmController`);
//! - [`proto`]: the ring messages; [`worklet`] and [`bridge`]: the Worklet and controller
//!   Worker sides.

pub mod bridge;
pub mod catalog;
pub mod guest;
pub mod js;
pub mod node;
pub mod proto;
pub mod worklet;

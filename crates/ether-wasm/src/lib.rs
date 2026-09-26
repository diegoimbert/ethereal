//! Browser build: wasm-bindgen entry points.
//!
//! Two instances of this module run in the browser:
//! - [`WasmController`] in a Web Worker: `ether-controller` with a bridge that serializes
//!   engine calls into a SharedArrayBuffer ring;
//! - [`WasmEngine`] in the AudioWorklet: owns `ether_core::Engine` + `EngineHandle`,
//!   drains the ring between blocks, and renders in `process()`.
//!
//! Messages to/from the UI are JSON-encoded `ClientMessage`/`ServerMessage`. Meters and
//! playhead go through a second SAB ring read by the UI at rAF rate. Requires COOP/COEP
//! (`apps/web` sets them). Owned by the `wasm-host` node.

use wasm_bindgen::prelude::*;

/// Controller side (Web Worker).
#[wasm_bindgen]
pub struct WasmController {
    _private: (),
}

#[wasm_bindgen]
impl WasmController {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u64) -> WasmController {
        let _ = seed;
        todo!("wasm-host node")
    }

    /// Handle one JSON `ClientMessage`; returns a JSON array of `ServerMessage`s.
    pub fn handle(&mut self, message_json: &str) -> String {
        let _ = message_json;
        todo!("wasm-host node")
    }

    /// Periodic work; returns a JSON array of `ServerMessage`s.
    pub fn tick(&mut self, now_ms: f64) -> String {
        let _ = now_ms;
        todo!("wasm-host node")
    }
}

/// Engine side (AudioWorkletProcessor).
#[wasm_bindgen]
pub struct WasmEngine {
    _private: (),
}

#[wasm_bindgen]
impl WasmEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: u32, max_block_size: usize) -> WasmEngine {
        let _ = (sample_rate, max_block_size);
        todo!("wasm-host node")
    }

    /// Render one block into planar output (`channels * frames` floats).
    pub fn process(&mut self, output: &mut [f32], channels: usize, frames: usize) {
        let _ = (output, channels, frames);
        todo!("wasm-host node")
    }
}

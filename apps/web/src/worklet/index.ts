// Placeholder. OWNERSHIP: the `wasm-host` node owns apps/web/** (including this folder).
//
// This folder will hold the AudioWorklet processor that runs `ether-core` compiled to WASM
// (`crates/ether-wasm`), plus the Worker hosting the controller. Communication uses
// SharedArrayBuffer rings, which require the COOP/COEP headers set in vite.config.ts.
export {};

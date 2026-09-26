# Ethereal — Architecture

Open-source (GPL-3.0-or-later) Ableton-style DAW. Rust engine, React/TS UI.
The UI runs unchanged in Tauri (desktop) and in a browser; the engine core compiles natively (macOS/Windows/Linux) and to `wasm32`.

## Locked decisions

| Area | Decision |
|---|---|
| Engine core | `ether-core`: pure DSP. No threads, filesystem, clock, device, or allocation on the audio path. Compiles native + `wasm32-unknown-unknown`. Single-threaded by design; hosts may parallelize by partitions. |
| Native host | `cpal` device I/O, disk streaming thread, CLAP hosting (`clack`), optional per-plugin out-of-process sandbox. |
| Web host | AudioWorklet running `ether-core` as WASM. `SharedArrayBuffer` rings (requires COOP/COEP headers). No plugins. |
| Document | Canonical model in Rust (`ether-model`), op-based, stable IDs (ULID) everywhere, never indices. Ops = undo = future collab sync (Loro/Yrs later). UI holds a mirror updated by patches. |
| File format | `.ether` = versioned JSON (`serde_json`), `version` field + migrations from day one. |
| Protocol | All UI↔engine traffic is serializable commands/events in `ether-protocol`. TS types generated from Rust (`specta`/`ts-rs`), never hand-written. |
| UI | React + TS. `EngineTransport` interface with `TauriTransport`, `WasmTransport`, `MockTransport`. SVG for automation, `<canvas>` per clip for waveforms (peak mipmaps computed in Rust). Visual polish/perf later. |
| Plugins | CLAP only. Floating plugin windows (no embedding). Scanner always out-of-process. |
| Built-in devices | Sampler, basic-shape synth, compressor, delay. Minimal. |
| Time-stretch | Signalsmith Stretch behind a `Stretcher` trait (native first; web later via its JS/WASM build). |
| Workflow | Ableton-style Session view + Arrangement view. |
| License | GPL-3.0-or-later. Dependencies must be GPL-3-compatible (MIT/Apache/BSD/GPL fine). |
| Collaboration | Not in v0.1, but the model must stay CRDT-ready (stable IDs, op log). |

## Real-time rules (audio thread)

No allocation/free, no locks shared with non-RT threads, no syscalls/I/O/logging, bounded runtime.
UI→audio: immutable render snapshots published via atomic swap / SPSC queue; parameter changes via lock-free queues with smoothing.
Audio→UI: meters/playhead/scopes via lock-free rings at visual rate. Old snapshots dropped on a GC thread.
Debug builds wrap the callback in `assert_no_alloc`.

## Workspace layout

```
crates/
  ether-protocol        commands/events, IDs, TS type export               (native + wasm)
  ether-model           document, ops, undo, .ether serde, migrations       (native + wasm)
  ether-core            graph, scheduler, tempo map, transport, mixer, PDC  (native + wasm)
  ether-devices         synth, sampler, compressor, delay                   (native + wasm)
  ether-media           decode (symphonia), resample (rubato), peaks        (native + wasm)
  ether-controller      commands → model → patches; model → render snapshot (native + wasm)
  ether-stretch         Stretcher trait + Signalsmith impl                  (native; trait wasm)
  ether-clap            CLAP hosting (clack), PluginNode in-process         (native)
  ether-plugin-scanner  scanner binary                                      (native)
  ether-sandbox         out-of-process PluginNode + helper binary           (native)
  ether-native          cpal host, RT thread, disk streaming, GC thread     (native)
  ether-wasm            wasm-bindgen: controller worker + worklet engine    (wasm)
apps/
  desktop               Tauri v2 shell (thin glue over ether-native)
  web                   Vite host for the browser build (COOP/COEP headers)
ui/                     React + TS app
  src/transport/        EngineTransport + mock/tauri/wasm implementations
  src/kit/              shared primitives (theme, Button, Knob, Fader, Panel)
  src/timeline/         time↔pixel mapping, zoom/scroll, selection, grid
  src/features/<name>/  one folder per feature (arrangement, piano-roll, ...)
```

## Key crates and ecosystem

`cpal`, `clack`, `symphonia`, `rubato`, `midir`, `rtrb`, `triple_buffer`, `basedrop`, `assert_no_alloc`, `serde`/`serde_json`, `specta` or `ts-rs`, `ulid`, `wasm-bindgen`, Tauri v2.

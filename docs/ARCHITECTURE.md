# Ethereal — Architecture

Open-source (GPL-3.0-or-later) Ableton-style DAW. Rust engine, React/TS UI.
The UI runs unchanged in Tauri (desktop) and in a browser; the engine core compiles natively (macOS/Windows/Linux) and to `wasm32`.

## Locked decisions

| Area | Decision |
|---|---|
| Engine core | `ether-core`: pure DSP. No threads, filesystem, clock, device, or allocation on the audio path. Compiles native + `wasm32-unknown-unknown`. Single-threaded by design; hosts may parallelize by partitions. |
| Native host | `cpal` device I/O, disk streaming thread, CLAP hosting (`clack`), optional per-plugin out-of-process sandbox. |
| Web host | Controller (`ether-controller`) in a Web Worker, engine (`ether-core`) in an AudioWorklet, separate WASM instances exchanging **serialized** messages over `SharedArrayBuffer` rings (requires COOP/COEP headers; no shared wasm memory). No plugins. |
| Document | Canonical model in Rust (`ether-model`), op-based, stable IDs everywhere (entities: ULID; projects: UUIDv7), never indices. Normalized entity tables, field-level ops. Ops = undo = future collab sync (Loro/Yrs later). UI holds a mirror updated by whole-entity patches. Musical time = `f64` beats, compared/quantized only via epsilon/snap helpers. Group tracks (nesting via parent id) are in v0.1. Record-arm is runtime state, not in the document. |
| File format | `.ether` = versioned JSON (`serde_json`), `version` field + migrations from day one. |
| Storage | **All file handling is engine-side**; the UI may run on another machine and never reads/writes files or sends paths. A `ProjectStore` (native: folders on disk; web: OPFS inside the engine Worker) keeps `<projects_root>/<project-uuid>/{project.ether, media/, cache/}`; imported audio is copied into `media/` (self-contained projects); the display name lives in the file. `projects_root` from engine config: `~/Documents/Ethereal/Projects`, or `<data_dir>/ethereal-dev/<instance>/projects` in dev. The sample browser lists engine-visible locations only (library folders + project media). |
| Protocol | All UI↔engine traffic is serializable commands/events in `ether-protocol`. TS types generated from Rust (`ts-rs`), never hand-written. |
| UI | React + TS. `EngineTransport` interface with `TauriTransport`, `WasmTransport`, `MockTransport`. SVG for automation, `<canvas>` per clip for waveforms (peak mipmaps computed in Rust). Visual polish/perf later. |
| Plugins | CLAP only. Floating plugin windows (no embedding). Scanner always out-of-process. Plugin state blob is authoritative on load; params are mirrored in the document for UI/automation. |
| Built-in devices | Sampler, basic-shape synth, compressor, delay. Minimal. |
| Time-stretch | Signalsmith Stretch behind a `Stretcher` trait (native first; web later via its JS/WASM build). Warp modes: Repitch + Complex (Signalsmith) only. |
| Automation | Enabled lanes always drive their target; no "manual move overrides automation / re-enable" in v0.1. |
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

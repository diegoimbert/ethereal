# Ethereal — Architecture

A DAW with a single arrangement view. Rust engine, React/TS UI. This repository holds the core; some features
(other-DAW import/export, collab and live sharing, the plugin catalog, telemetry/news and
the self-updater) are added on top by the private repository (`ethereal-private`), built over this one through its extension points (docs/EXTENSIONS.md).
The UI runs unchanged in Tauri (desktop) and in a browser; the engine core compiles natively (macOS/Windows/Linux) and to `wasm32`.

## Locked decisions

| Area | Decision |
|---|---|
| Engine core | `ether-core`: pure DSP. No threads, filesystem, clock, device, or allocation on the audio path. Compiles native + `wasm32-unknown-unknown`. Single-threaded by design; hosts may parallelize by partitions. |
| Native host | `cpal` device I/O, disk streaming thread, CLAP hosting (`clack`), optional per-plugin out-of-process sandbox. |
| Web host | Controller (`ether-controller`) in a Web Worker, engine (`ether-core`) in an AudioWorklet, separate WASM instances exchanging **serialized** messages over `SharedArrayBuffer` rings (requires COOP/COEP headers; no shared wasm memory). No plugins. |
| Document | Canonical model in Rust (`ether-model`), op-based, stable IDs everywhere (entities: ULID; projects: UUIDv7), never indices. Normalized entity tables, field-level ops. Ops = undo = future collab sync (Loro/Yrs later). UI holds a mirror updated by whole-entity patches. Musical time = `f64` beats, compared/quantized only via epsilon/snap helpers. Group tracks (nesting via parent id) are in v0.1. Record-arm is runtime state, not in the document. |
| File format | `.ether` = versioned JSON (`serde_json`), `format_version` field (format 1 = public-release baseline) + up/down migrations (CONTRACTS.md §2). |
| Storage | **All file handling is engine-side**; the UI may run on another machine and never reads/writes files or sends paths. A `ProjectStore` (native: folders on disk; web: OPFS inside the engine Worker) keeps `<projects_root>/<project-uuid>/{project.ether, media/, cache/}`; imported audio is copied into `media/` (self-contained projects); the display name lives in the file. `projects_root` from engine config: `~/Documents/Ethereal/Projects`, or `<data_dir>/ethereal-dev/<instance>/projects` in dev. The sample browser lists engine-visible locations only (library folders + project media). |
| Protocol | All UI↔engine traffic is serializable commands/events in `ether-protocol`. TS types generated from Rust (`ts-rs`), never hand-written. |
| UI | React + TS. `EngineTransport` interface with `TauriTransport`, `WasmTransport`, `MockTransport`. SVG for automation, `<canvas>` per clip for waveforms (peak mipmaps computed in Rust). Visual polish/perf later. |
| Plugins | CLAP, VST3 and AU (macOS), each behind the format-agnostic `PluginFormatHost` trait (`ether-plugin-host`; see docs/PLUGIN-FORMATS.md). Floating plugin windows (no embedding). Scanner always out-of-process (one target per child, a bounded pool of children, incremental via a per-target scan cache; system + user plugin folders). Plugin state blob is authoritative on load; params are mirrored in the document for UI/automation. VST2 is not in this build (the private repository registers it). |
| Built-in devices | Sampler, basic-shape synth, compressor, delay. Minimal. |
| Time-stretch | Signalsmith Stretch behind a `Stretcher` trait: the vendored C++ natively, a pure-Rust port of it on the web (`ether-stretch` `rust-stretch`; cheaper `Web` preset, at most 12 voices in the AudioWorklet). Warp modes: Repitch + Complex (Signalsmith) only. |
| Automation | Enabled lanes always drive their target; no "manual move overrides automation / re-enable" in v0.1. |
| Workflow | Arrangement view only (linear timeline). No Session view / clip launching. |
| Collaboration | Not in v0.1, but the model must stay CRDT-ready (stable IDs, op log). |

## Real-time rules (audio thread)

No allocation/free, no locks shared with non-RT threads, no syscalls/I/O/logging, bounded runtime.
UI→audio: immutable render snapshots published via atomic swap / SPSC queue; parameter changes via lock-free queues with smoothing.
Audio→UI: meters/playhead/scopes via lock-free rings at visual rate. Old snapshots dropped on a GC thread.
Debug builds wrap the callback in `assert_no_alloc`.
Work scales with what sounds, not with what exists: a node that returns `ProcessStatus::Silent` is skipped while its input stays digital silence and no event reaches it (built-in effects say so once their tail is under -120 dBFS; CLAP `Sleep` maps to it). Tracks run on the worker pool whenever any device works, playing or stopped.

## Multicore scheduling

Code: `ether-core/src/parallel.rs` (contract, `JobGraph`), `graph.rs` (builds it), `engine.rs` (dispatch), `ether-native/src/workers.rs` (the pool). Contract: CONTRACTS.md §11.7.

- **One job per track, scheduled by dependency.** Each snapshot compiles a `JobGraph` off the audio thread: an edge from every source (output, send, input tap, device or modulator sidechain) to the track it feeds, a per-job count of sources, and everything the run needs (counters, ready bitset) preallocated. Per sub-block the engine calls `ParallelExecutor::execute_graph` once. A finished job decrements its successors' counters; the one reaching zero makes that successor ready. No barrier between levels: a sidechain from a light track onto a heavy one only waits for the light track, and master starts as soon as its last input is done.
- **Rack chains fork.** A track job is one job, but at an audio rack (instrument or audio effect, two or more chains) whose chains are independent (no sidechain read or tapped on that track, no nested rack or drum rack in a chain), the job hands its chains to idle threads as sub-jobs (`Fork::join`; `execute_graph_forking`) and runs some of them itself. Idle threads take sub-jobs before ready graph jobs: the forking job was the most urgent when it started. The rack then mixes the chains in chain order (bit-identical). A heavy rack (several instruments in parallel chains) then costs its longest chain, not the sum. In the static cost, such a rack counts its longest chain.
- **Critical path first.** Cost per track: its measured job time on native engines (the engine times each job, smoothed, and reports it every 0.5 s; the handle ranks each publish with it, and re-ranks the running graph at most every 2 s of engine time when a track's time moved by more than 1.3x, by swapping in a new `JobGraph` built off the audio thread), else the static estimate 1 + its devices (chain, drum-rack pads, rack chains; a forking rack counts its longest chain), scaled like the measured tracks; wasm keeps the static costs. Priority = cost + the highest priority among the jobs it feeds. Jobs are numbered by priority, so the ready queue is a bitset of ranks, and taking the most urgent job means clearing its lowest set bit (`fetch_and`). Rank order is topological, and it is the sequential order (0 workers, wasm, idle engine).
- **Bit-identical.** Each job collects its inputs at its start, in an order fixed at compile time, so the output does not depend on the schedule or the worker count (`tests/it/parallel.rs`, `ether-native/tests/it/multicore.rs`).
- **Pool.** Epochs: the audio thread publishes the work, wakes at most `width - 1` parked workers (once per sub-block; a pure chain runs on the audio thread with no wake-up), works itself, and returns once the graph finished and every worker left the epoch. Idle workers spin briefly, then leave. A completion that readies several jobs unparks that many workers minus one. A missed wake-up only costs parallelism, because the audio thread never parks.
- **Benchmark.** `cargo test -p ether-native --release --test it sched_bench:: -- --ignored --nocapture` (the OSS #196 shape: p50/p99/worst buffer time against the deadline for 4/8/16 workers); `multicore_bench::` for throughput.
- **Anticipative processing** (`anticipation.rs`, CONTRACTS.md §11.7). Tracks not listened to live render one block ahead: per host block, the real-time side (master, live tracks and what feeds them) renders the block the anticipated tracks rendered during the previous one, reading their held copies, while those render the next block, in one pass of a job graph without the edges between the two sides. A heavy master chain then overlaps the tracks instead of following them. The transport plans each sub-block once, on the anticipated side, and the real-time side replays the queued plans, so the output is bit-identical to rendering without anticipation; controls reach anticipated tracks one block later.
- **Thread priorities** (Windows MMCSS, Linux `SCHED_FIFO`, macOS time-constraint) are in `workers.rs`.

## Workspace layout

```
crates/
  ether-protocol        commands/events, IDs, TS type export               (native + wasm)
  ether-model           document, ops, undo, .ether serde, migrations       (native + wasm)
  ether-core            graph, scheduler, tempo map, transport, mixer, PDC  (native + wasm)
  ether-devices         synth, sampler, compressor, delay                   (native + wasm)
  ether-media           decode (symphonia), resample (rubato), peaks        (native + wasm)
  ether-controller      commands → model → patches; model → render snapshot (native + wasm)
  ether-stretch         Stretcher trait, Signalsmith impls, Complex+        (native + wasm)
  ether-clap            CLAP hosting (clack), PluginNode in-process         (native)
  ether-plugin-host     PluginFormatHost trait, format registry, scan runner (native)
  ether-vst3            VST3 hosting                                        (native)
  ether-au              Audio Unit hosting               (native; functional on macOS only)
  ether-plugin-scanner  scanner binary (all formats)                        (native)
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

## Cargo features

Large features that the engine and the UI don't need sit behind Cargo features, so dev
builds compile less. **The default feature set is the fast dev core.** `full` turns on
everything, and the shipped apps always build it: the release desktop build runs
`tauri build --features full`, the web build runs `build-wasm.mjs` (which uses `full`), and
the gates run `just check-all` and `just test-all` with `--all-features`. With `full`, a
build behaves exactly as it did before the features existed. When a feature is off, its code is
compiled out and its commands take the paths that hosts without that capability already
take (usually the web host's): an `Unsupported` reply, or a link that closes with "not available in this
build". Protocol types never depend on features, so `just gen-types` is the same for
every feature set. No feature changes code on the audio thread. A feature either adds or
removes a whole node (a plugin format or a stretcher) or a bridge hook.

| Crate | Feature | Compiles in | When off |
|---|---|---|---|
| ether-stretch | `signalsmith` (crate default) | Signalsmith Stretch (C++) | trait only |
| ether-stretch | `rust-stretch` (crate default) | the pure-Rust Signalsmith Stretch port and Complex+ (`rustfft`, `realfft`; build for wasm32) | no `RustStretcher` / `PlusStretcher` |
| ether-native | `plugins-clap`, `plugins-vst3`, `plugins-au` | that format's host crate | the format is not registered, so loading it is `Unsupported` ("not supported by this build") |
| ether-native | `sandbox` | `ether-sandbox` (out-of-process instances) | a sandboxed instance fails `Unsupported` |
| ether-native | `plugins` | all four formats + `sandbox` | |
| ether-native | `warp-complex` | `ether-stretch/signalsmith` | no stretcher: Complex-warped clips play Repitch |
| ether-desktop | `mcp` | `ether-server` agent bridge for `ether-mcp` | enabling "Allow AI agents (MCP)" fails "not available in this build" |
| ether-desktop, ether-server | `plugins`, `warp-complex` | pass-throughs to ether-native | |
| ether-wasm | `warp-complex` | `ether-stretch/rust-stretch` (the Worklet's stretcher factory) | no stretcher: Complex-warped clips play Repitch |
| ether-wasm | `bench` | graph snapshot benchmark (`perf`, the large fixture, the `bench_graph_snapshot` export); not in `full`, `just bench-wasm` builds it | no benchmark export (release builds) |
| all of the above apps/hosts | `full` | every feature of the crate and its dependencies | |

The features listed above as absent (other-DAW import/export, collab and live sharing, the
"explore plugins" catalog, telemetry/news and the self-updater) are **not in this
repository**: they live in the private repository (`ethereal-private`), which builds on top
of this tree.

Leaf crates (`ether-collab`, `ether-stretch`) enable their feature by default, so
`cargo test -p <leaf>` covers it. The workspace dependency on each of them sets
`default-features = false`, and the integration crates opt in through their own features.
Integration tests of gated code declare `required-features` (`[[test]]` in the crate's
`Cargo.toml`). Unit tests use `#[cfg(feature = ...)]`.

Which builds use what:

| Build | Features |
|---|---|
| `just dev-desktop`, `just dev-server`, `just mcp`, `cargo test -p <crate>` | dev core (defaults) |
| `just dev-desktop --features plugins` (or any feature) | the dev core plus that feature |
| `just dev-desktop-full`, `just dev-server-full` | `full` |
| `just check-all` (`check-features`, `check-wasm`), `just test-all` | `--all-features` and, for clippy and wasm, also `--no-default-features` |
| release desktop (`release.yml`), web build (`build-wasm.mjs`) | `full` |

## Content-Security-Policy

Both hosts ship one (hardening). Scripts are strict: same-origin files only, plus
`'wasm-unsafe-eval'` on the web (the engine) and index.html's inline theme script by its
hash. Styles allow `'unsafe-inline'` (React style attributes, injected `<style>`).
`connect-src` stays wide (`https:`, `wss:`, and on the desktop `http:`/`ws:` too): the
signaling server, relay and remote engine are user-configurable addresses, and so are the
AI providers on the web. On the desktop the AI chat fetches its providers from Rust
(`tauri-plugin-http`, scoped; docs/MCP.md), outside the webview's `connect-src`.

- Web: `scripts/release/web/_headers` (the source; `vite preview` serves it too,
  `apps/web/csp.ts`). Editing index.html's inline script means updating its hash
  (`apps/web/src/csp.test.ts` checks). `ETHER_E2E_PREVIEW=1` e2e runs collect violation
  reports and fail on any (`e2e/global-teardown.ts`); `ETHER_CSP_REPORT_ONLY=1` serves it
  report-only. `vite dev` serves none (the React refresh preamble is inline).
- Desktop: `app.security.csp` in `apps/desktop/src-tauri/tauri.conf.json`. Tauri adds its
  IPC scripts' nonces and the inline script hashes to `script-src`; it leaves `style-src`
  alone (`dangerousDisableAssetCspModification`), as a nonce there would disable
  `'unsafe-inline'`. `just dev-desktop` loads the Vite dev server, without the policy.

## Key crates and ecosystem

`cpal`, `clack`, `symphonia`, `rubato`, `midir`, `rtrb`, `triple_buffer`, `basedrop`, `assert_no_alloc`, `serde`/`serde_json`, `specta` or `ts-rs`, `ulid`, `wasm-bindgen`, Tauri v2.

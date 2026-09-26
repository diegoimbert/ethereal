# Contracts: review guide

This is what every parallel node builds against. Review this page (and skim the linked
files) before fan-out. Types are frozen after approval; changes go through a BCR
(ORCHESTRATION.md §6.1). Bodies marked `todo!("<node> node")` say which node implements
them.

## 1. Big picture

```
 UI (React)                           controller thread / Web Worker          audio thread / AudioWorklet
 ──────────                           ──────────────────────────────          ───────────────────────────
 EngineTransport ── ClientMessage ──▶ Controller (ether-controller)
   (Mock | Tauri | Wasm)                ops ─▶ Project (ether-model) + History
 project mirror ◀─ Event::Patch ──────  patches
 (zustand)       ◀─ Reply ────────────  compile_graph ─▶ RenderGraphDesc ──▶ EngineHandle.publish ─▶ Engine
 playhead/meters ◀─ Playhead/Meters ──  EngineHandle.poll ◀──────────── lock-free rings ◀──────── Engine.process
```

- One writer: the controller owns the document. The UI only sends commands and mirrors
  patches.
- The engine never sees the document. It sees a plain-data `RenderGraphDesc`, compiled
  off the audio thread into a `RenderSnapshot` and swapped in at a block boundary.
- Stateful processors (devices, plugins) are inserted once as `Node`s and referenced by
  `NodeKey`, so they keep their state across snapshot swaps.

### Crate dependency direction

`ether-model` ← `ether-protocol` ← `ether-core` ← {`ether-devices`, `ether-media`,
`ether-controller`, `ether-clap`, `ether-sandbox`} ← {`ether-native`, `ether-wasm`}.
`ether-core` also depends on `ether-stretch` (trait only).

## 2. Document model: `crates/ether-model`

Files: `ids.rs`, `value.rs`, `project.rs`, `track.rs`, `clip.rs`, `note.rs`, `automation.rs`,
`device.rs`, `mixer.rs`, `session.rs`, `tempo.rs`, `warp.rs`, `media.rs`, `entity.rs`,
`op.rs`, `history.rs`, `patch.rs`, `file.rs`.

- **IDs.** Typed ULID newtypes (`TrackId`, `ClipId`, `NoteId`, `DeviceId`, `SendId`,
  `SceneId`, `AutomationLaneId`, `AutomationPointId`, `TempoPointId`, `TimeSignatureId`,
  `WarpMarkerId`, `MediaId`, `ProjectId`). On the wire and in TS they are strings.
  **Whoever creates an entity picks its ID**: usually the UI, with `newId()`. That makes
  commands idempotent and the UI optimistic-ready, and it is how CRDTs work.
  `IdGen` is deterministic (seed + clock injected), so no `getrandom` is needed on wasm.
- **Normalized `Project`.** Every entity type has one flat `BTreeMap<Id, Entity>` table.
  Children point to parents by ID (`Clip.track`, `Note.clip`, `Device.track`,
  `AutomationPoint.lane`, `WarpMarker.clip`, ...). Nothing is nested. Singletons live in
  `ProjectSettings` (name, loop, metronome, launch quantization, count-in).
- **No indices.** Sibling order (tracks, devices, scenes) uses `OrderKey`, a
  fractional-index string. Moving an item rewrites only that item's key.
- **Values.** `Beats(f64)` (quarter notes) for all musical time; `Seconds(f64)` for source
  media time; `Decibels(f32)` (-144 = silence); `Pan(-1..1)`; `Color(0xRRGGBB)`;
  `ParamId(u32)` within a device.
  - **Device params** are stored as plain values (Hz, dB, ...).
  - **Automation values** are normalized 0..1. `ParamInfo.scale` maps between the two.
- **Clips.** `ClipLocation::Arrangement { start } | Session { scene }`. Arrangement and
  session clips are separate objects, as in Ableton. A session slot is `(track, scene)` and
  is derived, not stored (`ClipSlot` is a view). Each clip has its own content timeline:
  `offset`, `length` and `looping`, all in content beats. Notes, clip envelopes and warp
  markers are relative to that content timeline.
- **Ops (`op.rs`).** There are four kinds:
  - `Insert { entity }`
  - `Remove { key }`, which only succeeds when the entity has no children. The controller
    removes children first, so there are no hidden cascades.
  - `Update { EntityUpdate }`, which changes **one field** (`TrackChange::Volume(..)`, ...).
    Each field is a last-writer-wins register, so concurrent edits of different fields
    never conflict.
  - `Settings { change }`

  Every op has a trivial inverse. Undo applies inverses. The same op stream is what a
  future Loro/Yrs layer would sync.
- **History.** `History::commit(project, Transaction, Option<GestureId>)`. Transactions
  that share a gesture merge into one undo step, so a fader drag is one undo.
  `HistoryState` feeds the undo/redo buttons.
- **Patches.** A `Patch` holds `revision` plus `changes`: whole-entity `Upsert` / `Remove`
  (plus `Settings`), derived from the applied ops. The UI mirror applies them with no
  domain logic (`table[id] = value`). A gap in `revision` means the UI re-requests the
  project.
- **`.ether` file.** Format: `{ format: "ethereal-project", version: 1, app_version, project }`.
  Loading parses to `serde_json::Value`, runs `Migration`s up to `CURRENT_VERSION`,
  deserializes, then calls `validate()`. Media is referenced project-relative
  (`Samples/...`), absolute, or by OPFS path (web).

## 3. Wire protocol: `crates/ether-protocol`

There is one file per domain: `transport`, `project` (also `EditCommand`), `tracks`,
`clips`, `notes`, `automation`, `devices`, `mixer`, `session`, `plugins`, `recording`,
`warp`, `media` (import, peaks, browser), `meters`, `engine` (audio config/status).
`message.rs` wraps them:

- **`ClientMessage { id, gesture?, command }`**, where
  `Command = { domain: "Mixer", command: { type: "SetVolume", ... } }`.
- **`ServerMessage`** is one of `Reply | Event | Playhead | Meters`, as
  `{ kind, body }`.
  - `Reply { id, result: Ok { value: ReplyValue } | Err { error: { code, message } } }`.
    Every client message gets exactly one reply. **The patches a command causes are
    emitted before its reply.**
  - `Event`: `ProjectLoaded`, `Patch`, `Transport`, `Session`, `Plugin`, `Recording`,
    `Media`, `Engine`, `Notification`.
  - `Playhead` (~60 Hz) and `Meters` (~30 Hz) are high-rate streams. Hosts may deliver them
    on a separate channel (a Tauri `Channel`, or a SAB ring).
- **Undoable vs. not undoable.** All document edits are undoable, including loop region,
  tempo and time signature. Transport play/stop/locate, clip launching, plugin editor
  windows and engine config are not.
- **Continuous controls.** Volume, pan, send level and device params are undoable
  document edits and are also pushed to the engine param queue immediately, with no graph
  republish. The UI sends them with a `gesture` and closes the gesture with
  `Edit::EndGesture`.
- **`Edit::Batch { label, commands }`** applies several commands as one undo step. For
  example, create a MIDI track and insert a synth.
- **Plugin scanner wire format:** `ScanRequest` / `ScanResponse` (JSON over stdin/stdout,
  one bundle per process).
- **JSON conventions.** Field names are Rust snake_case, with no renaming. Enums are
  tagged `type` (unit-only enums are plain strings), except the adjacently tagged
  `Command` (`domain`/`command`), `ServerMessage` (`kind`/`body`), `Entity`
  (`type`/`value`), `EntityKey` (`type`/`id`) and field updates (`field`/`value`).
  `tests/serde_shapes.rs` pins these shapes.
- **TS generation.** `just gen-types` runs `cargo run -p ether-protocol --example gen-ts`,
  which writes one `.ts` per type plus an `index.ts` barrel into `ui/src/generated/`.
  **Never hand-edit these files.** CI job "Generated TS types fresh" fails if they are
  stale.

## 4. Engine: `crates/ether-core`

- **`create(EngineConfig) -> EngineParts { engine, handle, gc }`** splits the engine into
  three halves on three threads:
  - `Engine::process(inputs, outputs, frames)` runs on the audio thread and is RT-safe.
    Buffers are planar `f32`.
  - `EngineHandle` runs on the controller thread and is non-blocking:
    - `add_node(Box<dyn Node>) -> NodeKey` (it calls `prepare` off-thread), `remove_node`
    - `add_source(MediaId, Arc<dyn AudioSource>)`
    - `publish(RenderGraphDesc)` (compiles, then swaps)
    - `set_param(ParamChange)`, `transport(TransportControl)`, `session(SessionControl)`
    - `poll(&mut EngineOutputs)` for playhead, meters, session state and overflow flags
  - `GarbageCollector::collect()` runs on the GC thread and drops retired snapshots and
    nodes. The audio thread never frees.
- **`Node`** (`Send`): `prepare` (non-RT, may allocate), `reset`,
  `process(&mut ProcessContext, &mut AudioBuffers) -> ProcessStatus`, `latency`,
  `channels`.
  `ProcessContext` carries:
  - `frames` and `sample_rate`
  - `transport: &TransportInfo` (position, bpm, beats per sample, time signature, bar
    start, loop)
  - sorted sample-accurate `events: &[ProcessEvent]` (`NoteOn/Off/Choke`, `AllNotesOff`,
    `Param { param, value }` with plain values, raw `Midi`)
  - a pre-allocated `out_events`

  The scheduler splits blocks at loop points and tempo boundaries, so transport
  information is linear within one call.
- **`Device: Node`** adds `descriptor()`, `param(id)` and `set_param(id, v)`. At runtime,
  params arrive as events. Nodes smooth continuous params with `Smoother`.
- **`RenderGraphDesc`** is plain `serde` data. It contains:
  - tempo points and time signatures, loop, metronome
  - tracks, each with: `chain` of `NodeKey`s, routing (`output`, `sends`),
    volume/pan/mute/solo, input and monitoring, arrangement `clips`, `session_clips`, and
    `automation` resolved to engine targets with a `ParamMapping`

  Clip content is either MIDI `notes` or audio `media: MediaId` with gain, transpose,
  fades and an optional `WarpDesc`. The desc is serializable so the web host can ship it
  Worker → Worklet. `compile()` does the topological sort, PDC and cycle rejection
  (`core` node).
- **`ParamChange { target: TrackVolume | TrackPan | TrackMute | SendLevel | Node { node, param }, value }`**
  goes through a lock-free queue and is applied at the next block.
- **`AudioSource`**: `read(channel, start, out) -> bool` must be RT-safe. It returns
  silence plus `false` on a streaming underrun. Hosts implement streaming;
  `ether-media::InMemorySource` is the simple case.
- **`TempoMapRt`** is the RT-side tempo map. It must agree with `ether_model::TempoMap`
  (shared test vectors).
- **`session/`** holds `SessionControl` and `SessionState`. It is pre-created and owned by
  `core-session`.
- **Plugins (`plugin.rs`).** Two halves, mirroring CLAP threading:
  - `PluginController` (main thread, not `Send`): params, `activate() -> Box<dyn PluginNode>`,
    state save/load, floating editor, `poll()` → `PluginNotification`s.
  - `PluginNode: Device` (audio thread): `is_faulted()`.

  In-process (`ether-clap`) and sandboxed (`ether-sandbox`: shared memory plus
  semaphores, +1 block latency reported for PDC, crash → faulted → bypass) implement the
  same traits. **IPC naming rule:** every global OS object is named with
  `ipc_name(instance, pid, purpose)`.
- **`Stretcher`** (`ether-stretch`): `configure`, `reset`, latencies,
  `set_transpose_semitones`, `seek`,
  `process(input, in_frames, output, out_frames)`, plus a `StretcherFactory`. The
  Signalsmith implementation is behind the native-only `signalsmith` feature. The web
  falls back to unwarped playback.

## 5. Controller: `crates/ether-controller`

- `Controller` has three methods:
  - `handle(ClientMessage, &mut dyn MessageSink)`: patches first, then exactly one reply.
  - `tick(now_ms, sink)`: engine polling becomes `Playhead`/`Meters`/`Session`; also
    plugin notifications and autosave.
  - `project()`.
- `EngineBridge` is how the controller reaches the engine. Every argument is plain data,
  so the bridge can be:
  - native: direct over `EngineHandle`, building nodes with `ether-devices`;
  - web: serialized to the worklet.

  Its methods are `create_builtin`, `create_plugin`, `destroy_node`,
  `load_media`/`unload_media`, `publish`, `set_param`, `transport`, `session`, `poll`
  and `descriptor`.
- `HostServices` provides file read/write (path or OPFS), the clock, entropy and
  `data_dir`.
- `compile_graph(project, node_lookup, version) -> RenderGraphDesc` is a pure function
  that can be unit-tested without an engine.

## 6. UI transport: `ui/src/transport`, `ui/src/state`

- **`EngineTransport`** members:
  - `kind`
  - `connect(): Promise<Project>`
  - `send(command, { gesture }): Promise<ReplyValue>`: rejects with `CommandFailedError`,
    and resolves after that command's patches have been delivered.
  - `onEvent(listener)`
  - `subscribePlayhead(listener)`, `subscribeMeters(listener)`
  - `dispose()`

  Helpers: `cmd("Mixer", { type: "SetVolume", ... })` (typed per domain), `newId()`,
  `nextGestureId()`.
- **Implementations.**
  - `MockTransport` is in-memory. It implements most document commands with patches,
    undo/redo and gestures, plus a fake playhead and meters and a demo project. UI nodes
    build real features against it.
  - `tauri/` and `wasm/` are stubs owned by `native-host` and `wasm-host`.
  - `createDefaultTransport()` picks the implementation (mock until the real hosts land).
- **State.**
  - `useProjectStore` (zustand) is the normalized mirror. It applies patches by revision
    and holds history state, transport state and session clip states.
  - Selectors live in `state/selectors.ts`.
  - High-rate playhead and meter data live in a separate external store
    (`usePlayhead`, `useTrackMeter`), so they don't re-render the whole tree.
- **Features.** `ui/src/app/App.tsx` is the only place features are registered. Each
  feature owns `ui/src/features/<name>/` and reads state through the hooks above.

## 7. Repo mechanics

- `.github/ownership.toml` maps each node to its globs. `scripts/check-ownership.py` runs
  in CI for `node/*` branches. `Cargo.lock`, `pnpm-lock.yaml` and `ui/src/generated/**`
  are open to everyone. `foundation`, `alpha`, `v0.1` and `base-*` are unrestricted.
- `[workspace.dependencies]` pre-declares everything anticipated: cpal, clack (git-pinned),
  symphonia, rubato, midir, rtrb, triple_buffer, basedrop, assert_no_alloc, signalsmith,
  shared_memory, raw_sync, interprocess, wasm-bindgen, js-sys, web-sys, tauri + plugins,
  proptest, insta, and others. Nodes opt in from their own `Cargo.toml`. `ui/package.json`
  is likewise pre-populated.
- **Parallel dev instances** (README "Running multiple dev instances"):
  - `ETHER_INSTANCE` defaults to the worktree directory name.
  - The base port is a hash of it in 20000–29999 (`ETHER_DEV_PORT` overrides), with fixed
    offsets per server. Vite uses `strictPort`.
  - The Tauri identifier, `devUrl` and app data dir are per instance
    (`<data_dir>/ethereal-dev/<instance>/`). There is no single-instance behavior.
  - `ETHER_AUDIO=null|offline` gives headless audio.
  - IPC names include instance + pid.
  - Each worktree keeps its own cargo `target/`.

## 8. Decisions that extend or deviate from ARCHITECTURE.md

1. **TS generation: ts-rs, not specta.** ts-rs is a mature, derive-only crate with no
   runtime. It emits one file per type, honors serde attributes, and needs no Tauri
   coupling. specta v2 is still an RC and mostly used through tauri-specta, while we also
   need the web host.
2. **IDs live in `ether-model`, re-exported as `ether_protocol::ids`.** The protocol must
   carry document types (`Project`, `Patch`), so protocol depends on model. Putting the
   IDs in the protocol crate would create a dependency cycle.
3. **Patches are whole-entity upserts, not ops.** This keeps the TS mirror free of domain
   logic. Ops remain the undo and future-sync unit on the Rust side.
4. **Fully normalized document** with fractional `OrderKey`s, and field-granular `Update`
   ops. This goes further than "stable IDs" in order to be CRDT-ready.
5. **Engine = node table + serializable graph desc + three halves (engine/handle/GC).**
   It uses `rtrb` rings throughout; `basedrop` and `triple_buffer` stay available to
   hosts. Sources are registered separately as `AudioSource`s, never by pointer inside
   the desc.
6. **`ulid` is used without its `serde` feature.** Version 1.2 with no-std serde doesn't
   compile, so the IDs have hand-written string serde.
7. **Apps.** `apps/desktop/src-tauri` is a workspace member (`ether-desktop`). Its
   `build.rs` writes a placeholder `ui/dist` so `cargo check` works before a UI build.
8. **TypeScript is pinned to `~6.0`.** typescript-eslint doesn't support TS 7 yet.

## 9. Open questions for you

1. **Web threading model.** In the current design the controller runs in a Worker and the
   engine in the AudioWorklet. They don't share Rust memory: the desc and commands are
   serialized over SAB rings, and the worklet builds nodes itself. The alternative is one
   shared wasm memory (atomics), which needs nightly `build-std` today. Is the
   serialized approach OK?
2. **Time representation.** Is `Beats(f64)` OK, or do you prefer integer ticks (e.g. 960
   PPQ) for exact editing? `f64` is simpler and exact for binary grid values; triplets
   aren't exact.
3. **Undo scope.** Tempo, time signature and loop region edits are undoable, as are
   fader/knob moves (merged per gesture). Mute, solo and arm are undoable too (they are
   document fields). Should arm/solo be excluded, as in some DAWs?
4. **Automation override.** While a lane is enabled, it overrides manual moves (Ableton).
   A manual move during playback would disable the lane, the Ableton "re-enable
   automation" button. Is that wanted in v0.1?
5. **Group tracks.** `TrackKind::Group` and `parent` are in the model. Are groups in v0.1
   scope, or should the UI hide them?
6. **Warp modes.** Only `Repitch` and `Complex` (Signalsmith) are included. Should other
   Ableton modes be added as aliases?
7. **Web saving.** `Save { target: Json }` returns the `.ether` JSON and the UI
   downloads it or writes it to OPFS. Media on the web lives in OPFS. Is that OK for v0.1?
8. **Plugin params in the document.** They are mirrored in `Device.params` for UI and
   automation, but the plugin state blob is authoritative on load. Is that OK?

## 10. Stubs and ownership

| Node | Implements |
|---|---|
| `model` | every `todo!("model node")` in ether-model (`OrderKey::between`, `Project::apply/...`, `History`, `patch::changes_for`, `file::load/save`, `TempoMap`) |
| `core` | `engine.rs`, `graph::compile`, `tempo.rs` |
| `core-session` | `ether-core/src/session/**` |
| `devices` | ether-devices |
| `media` | ether-media |
| `controller` | ether-controller |
| `clap` / `sandbox` / `stretch` | their crates (+ scanner) |
| `native-host` / `wasm-host` | ether-native + apps/desktop + `ui/src/transport/tauri` / ether-wasm + apps/web + `ui/src/transport/wasm` |
| ui-* | their feature folders / `ui/src/timeline` |

`ether-protocol` has no owner after foundation, so everything in it is implemented now.
That includes the normalized↔plain param mapping (`devices::scale_to_plain` /
`scale_to_normalized`, with test vectors the UI mirrors).

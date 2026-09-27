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
`device.rs`, `mixer.rs`, `tempo.rs`, `warp.rs`, `media.rs`, `entity.rs`,
`op.rs`, `history.rs`, `patch.rs`, `file.rs`.

- **IDs.** Entity IDs are typed ULID newtypes (`TrackId`, `ClipId`, `NoteId`, `DeviceId`,
  `SendId`, `AutomationLaneId`, `AutomationPointId`, `TempoPointId`,
  `TimeSignatureId`, `WarpMarkerId`, `MediaId`). **`ProjectId` is a UUIDv7** (hyphenated
  string); it names the project folder in the store. I chose this mix over UUIDv7
  everywhere: ULIDs are shorter on the wire, and only project IDs are user-visible or
  used as folder names. On the wire and in TS all IDs are strings.
  **Whoever creates an entity picks its ID**: usually the UI, with `newId()` or
  `newProjectId()`. That makes commands idempotent and the UI optimistic-ready, and it is
  how CRDTs work. `IdGen` is deterministic (seed + clock injected; `next_project_id` mints
  UUIDv7), so no `getrandom` is needed on wasm.
- **Normalized `Project`.** Every entity type has one flat `BTreeMap<Id, Entity>` table.
  Children point to parents by ID (`Clip.track`, `Note.clip`, `Device.track`,
  `AutomationPoint.lane`, `WarpMarker.clip`, ...). Nothing is nested. Singletons live in
  `ProjectSettings` (name, loop, metronome, count-in).
- **No indices.** Sibling order (tracks, devices) uses `OrderKey`, a
  fractional-index string. Moving an item rewrites only that item's key.
- **Values.** `Beats(f64)` (quarter notes) for all musical time; `Seconds(f64)` for source
  media time; `Decibels(f32)` (-144 = silence); `Pan(-1..1)`; `Color(0xRRGGBB)`;
  `ParamId(u32)` within a device.
  - **Device params** are stored as plain values (Hz, dB, ...).
  - **Automation values** are normalized 0..1. `ParamInfo.scale` maps between the two.
- **Beats conventions (decided).** Musical time is `f64` beats, serialized as plain JSON
  numbers.
  - Never compare beats with `==`: use `Beats::approx_eq` (`EPSILON = 1e-6`).
  - Quantize, snap and bar math go through `Beats::snap`, `floor_to` and `ceil_to`, which
    treat values within epsilon of a grid line as on it.
  - The UI mirrors these helpers exactly in `ui/src/state/beats.ts`, with the same test
    vectors.
- **Tracks and groups.** Group tracks are in v0.1. Nesting uses `Track.parent` (groups
  may nest). `TrackOutput::Default` means "parent group bus if nested, else master". The
  render graph gets fully resolved `output` plus `group`, and mute/solo propagate from a
  group to its children.
- **Record-arm is not in the document.** It is momentary performance state and not
  undoable. The controller holds it, `RecordingCommand::Arm` changes it, and
  `RecordingEvent::ArmChanged { armed }` reports it. Mute and solo are document fields and
  are undoable.
- **Clips.** Every clip lives on the arrangement timeline; there is no Session view.
  `Clip { id, track, start, name, color, muted, length, offset, looping: ClipLoop, content: ClipContent }`,
  where `start: Beats` is the clip's arrangement position (changed with
  `ClipChange::Start(Beats)`). Each clip has its own content timeline: `offset`, `length`
  and `looping`, all in content beats. Notes, clip envelopes and warp markers are
  relative to that content timeline.
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
- **`.ether` file.** Format: `{ format: "ethereal-project", version, app_version, project }`,
  with `CURRENT_VERSION = 3`. Loading parses to `serde_json::Value`, runs `Migration`s up
  to `CURRENT_VERSION`, deserializes, then calls `validate()`. `MediaRef.file` is always
  project-relative (`media/...`).
  - **v1 → v2 (`V1RemoveSession`)** removes the Session view data. Arrangement clips'
    `location: { type: "Arrangement", start }` becomes a flat `start`, and `launch` is
    dropped. Session clips (`location.type == "Session"`) are dropped together with their
    notes, warp markers and clip-envelope automation lanes (and those lanes' points).
    `project.scenes` and `settings.launch_quantization` are removed.
  - **v2 → v3 (`V2RoadmapDefaults`, contracts-2)** adds the roadmap v2 tables and fields
    with neutral defaults (§11.13); a migrated project sounds and behaves as before.

## 2b. Storage: engine-side only (decided)

**The UI never reads or writes files, in any build.** The UI may run on a different
machine than the engine, so the protocol never carries file-system paths.

- **Store traits.** `ether_controller::store::ProjectStore` and `store::Library` are used
  only by the controller.
  - Native: folders on disk (`ether-native::DiskStore`).
  - Web: OPFS, accessed from the engine's Worker, which is still engine-side.
- **Layout.** Each project lives in `<projects_root>/<project-uuid>/`:
  - `project.ether`, which holds the display name, so a rename never moves the folder
  - `media/`: imported audio is copied in, so projects are self-contained
  - `cache/`: peaks and decoded audio
- **`projects_root`** comes from engine/host config (`ether_native::default_projects_root`):
  - `~/Documents/Ethereal/Projects` by default;
  - `<data_dir>/ethereal-dev/<instance>/projects` in dev builds, so each instance is
    isolated.
- **Project protocol** (`ProjectCommand`):
  - `List`, `Create { id, name }`, `Open { id }`, `Save`, `SaveAs { new_id, name }`,
    `Duplicate { id, new_id, name }`, `Rename { id, name }`, `Delete { id }`, `Get`.
  - `Event::Project` carries `ListChanged`, `Saved` or `DirtyChanged`.
  - `ProjectSummary` is `{ id, name, modified_ms }`.
- **Sample browser** (`MediaCommand`):
  - `ListLocations` returns engine-visible roots: configured library folders
    (`BrowseLocation::Library { id }`) plus the current project's media (`ProjectMedia`).
  - `ListDirectory { location, path }` uses relative paths only; the engine rejects `..`
    and absolute paths.
  - `Import { id, source: Location { location, path } }` copies the file into the project.
- **Upload from the UI machine** is not in v0.1. `MediaCommand::BeginUpload` and
  `MediaSource::Upload` reserve its place in the protocol, and hosts reply `Unsupported`.

## 3. Wire protocol: `crates/ether-protocol`

There is one file per domain: `transport`, `project` (also `EditCommand`), `tracks`,
`clips`, `notes`, `automation`, `devices`, `mixer`, `plugins`, `recording`,
`warp`, `media` (import, peaks, browser), `meters`, `engine` (audio config/status). Roadmap
v2 adds `export`, `tempo`, `markers`, `midi_map`, `groove`, `drum_rack` (with slices),
`collab` and `remote` (§11).
`message.rs` wraps them:

- **`ClientMessage { id, gesture?, command }`**, where
  `Command = { domain: "Mixer", command: { type: "SetVolume", ... } }`.
- **`ServerMessage`** is one of `Reply | Event | Playhead | Meters`, as
  `{ kind, body }`.
  - `Reply { id, result: Ok { value: ReplyValue } | Err { error: { code, message } } }`.
    Every client message gets exactly one reply. **The patches a command causes are
    emitted before its reply.**
  - `Event`: `ProjectLoaded`, `Project` (store list/saved/dirty), `Patch`, `Transport`,
    `Plugin`, `Recording`, `Media`, `Engine`, `Notification`.
  - `Playhead` (~60 Hz, `PlayheadFrame { transport }`) and `Meters` (~30 Hz) are
    high-rate streams. Hosts may deliver them
    on a separate channel (a Tauri `Channel`, or a SAB ring).
- **Undoable vs. not undoable.**
  - Undoable: all document edits, including loop region, tempo, time signature, mute
    and solo.
  - Not undoable: record-arm (not in the document), transport play/stop/locate, plugin
    editor windows, engine config, and project store operations
    (list/open/delete/duplicate).
- **Clip commands.** `ClipCommand::CreateMidi`, `CreateAudio` and `SetBounds` take
  `start: Beats` (arrangement position). `ClipCommand::Duplicate` takes
  `start: Option<Beats>`; `None` places the copy right after the original on the same
  track. A `ClipMove` is `{ id, track, start }`.
- **Automation.** An enabled lane always drives its target. v0.1 has no "manual move
  overrides automation / re-enable automation". `AutomationLane.enabled` is only an
  explicit user toggle.
  - **Precedence (engine).** The engine resolves each target once per processed
    sub-block:
    - A clip envelope of an unmuted clip that overlaps the sub-block wins over the
      arrangement lane for the same target. The lane is skipped for that sub-block.
    - When no active clip envelope covers the target any more, the lane takes over
      immediately (in the next sub-block, at its current value).
    - Handover is at sub-block granularity: at most one host block.
    - Overlapping clips on one track with envelopes for the same target are undefined.
  - **While stopped.** Lanes and clip envelopes both apply at the stopped position (one
    evaluation), with the same precedence. An envelope only applies when the position
    is inside its clip.
  - **Timing.** Mixer targets (volume, pan, send level) take the value at the sub-block
    start, smoothed over 10 ms. Device params get `Param` events every 64 samples while
    playing.
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
    - `set_param(ParamChange)`, `transport(TransportControl)`
    - `poll(&mut EngineOutputs)` for playhead, meters and overflow flags
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
    volume/pan/mute/solo, input and monitoring, arrangement `clips`, and
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
- **Plugins (`plugin.rs`).** Two halves, mirroring CLAP threading:
  - `PluginController` (main thread, not `Send`): params, `activate() -> Box<dyn PluginNode>`,
    state save/load, floating editor, `poll()` → `PluginNotification`s.
  - `PluginNode: Device` (audio thread): `is_faulted()`.

  `PluginController::set_param_value` (defaulted) sets a param while inactive;
  `PluginError::Unsupported` marks a format/platform feature that isn't available.

  In-process (`ether-clap`) and sandboxed (`ether-sandbox`: shared memory plus
  semaphores, +1 block latency reported for PDC, crash → faulted → bypass) implement the
  same traits. **IPC naming rule:** every global OS object is named with
  `ipc_name(instance, pid, purpose)`.
- **Plugin formats (formats-base).** `PluginFormat { Clap, Vst3, Au }` (serde tags
  `"Clap"`/`"Vst3"`/`"Au"`, stable, additive). Each format implements
  `ether_plugin_host::PluginFormatHost` (`scan` in the scanner process only, `instantiate`
  on the plugin main thread) in its own crate (`ether-clap`, `ether-vst3`, `ether-au`); the
  scanner and sandbox helper (`--format`) dispatch through a `Formats` registry. **Id
  convention** (`PluginInstance.plugin_id` = `PluginDescriptor.id`): CLAP = reverse-DNS
  plugin id; VST3 = class id as 32 uppercase hex in canonical `FUID::toString` order (same
  on every OS); AU = `type:subtype:manufacturer` four-char codes (`aufx:dely:appl`, non-
  printable bytes as `\xHH`). The document stores no plugin location: hosts resolve
  `(format, plugin_id)` through their scanned catalog (`PluginDescriptor.path` = bundle
  path; for AU the id). `DeviceSpec::Plugin.format` and `ScanRequest.format` are optional
  (omitted = CLAP / inferred from the path). Details: `docs/PLUGIN-FORMATS.md`.
- **`Stretcher`** (`ether-stretch`): `configure`, `reset`, latencies,
  `set_transpose_semitones`, `seek`,
  `process(input, in_frames, output, out_frames)`, plus a `StretcherFactory`. The
  Signalsmith implementation is behind the native-only `signalsmith` feature. The web
  falls back to unwarped playback.

## 5. Controller: `crates/ether-controller`

- `Controller` has three methods:
  - `handle(ClientMessage, &mut dyn MessageSink)`: patches first, then exactly one reply.
  - `tick(now_ms, sink)`: engine polling becomes `Playhead`/`Meters`; also
    plugin notifications and autosave.
  - `project()`.
- `EngineBridge` is how the controller reaches the engine. Every argument is plain data,
  so the bridge can be:
  - native: direct over `EngineHandle`, building nodes with `ether-devices`;
  - web: serialized to the worklet.

  Its methods are `create_builtin`, `create_plugin`, `destroy_node`,
  `load_media(&MediaRef, Arc<DecodedAudio>)`/`unload_media`, `publish`, `set_param`,
  `transport`, `poll` and `descriptor`.
- `HostServices` provides the clock and entropy. All file access goes through
  `ProjectStore` and `Library` (§2b), which are passed to
  `EtherController::new(bridge, host, store, library)`. The controller decodes media with
  `ether-media`. `project()` returns `None` until a project is created or opened.
- `compile_graph(project, node_lookup, version) -> RenderGraphDesc` is a pure function
  that can be unit-tested without an engine.

**Host-handled commands (base-6).** `Command::Engine(*)` (audio device list/config/status) and `Command::Plugin(Rescan | List | OpenEditor | CloseEditor)` are intercepted by the **native host** on the controller thread before `Controller::handle`, which replies itself (same ordering: one reply per message). The controller replies `Unsupported` if it ever receives them (web host). `EngineBridge::poll_plugins` (drained from the controller tick) and `EngineBridge::plugin_state` (read for every plugin device before serializing) are defaulted, so non-plugin hosts ignore them. So is `EngineBridge::plugin_param_values` (current plain values of a plugin's params, read after (re)instantiating from a state blob to mirror them into `Device.params`).

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
  `newProjectId()` (UUIDv7), `nextGestureId()`.
- **Implementations.**
  - `MockTransport` is in-memory. It implements most document commands with patches,
    undo/redo and gestures, plus a fake playhead and meters. UI nodes build real features
    against it. It also simulates the engine-side project store (three demo projects,
    dirty flag) and a fake sample library. There are no file pickers and no browser file
    I/O.
  - `tauri/` and `wasm/` are stubs owned by `native-host` and `wasm-host`.
  - `createDefaultTransport()` picks the implementation (mock until the real hosts land).
- **State.**
  - `useProjectStore` (zustand) is the normalized mirror. It applies patches by revision
    and holds history state and transport state.
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
9. **`TrackOutput::Master` is renamed `Default`.** The default output is the parent group
   bus if the track is nested, otherwise master.

## 9. Decisions log (user answers to the review questions)

1. **Web threading.** Accepted as proposed: the controller runs in a Worker and the engine
   in the Worklet, as separate wasm instances exchanging serialized messages over SAB
   rings.
2. **Time.** `f64` beats, with the epsilon and snap conventions in §2.
3. **Undo.** Arm is not undoable and has been removed from the document. Mute and solo
   are undoable.
4. **Automation override/re-enable.** Not in v0.1; no flag or command exists for it.
5. **Group tracks.** In v0.1, via `parent` in the model and group bus routing in the
   render graph (`TrackDesc.output`/`group`).
6. **Warp modes.** Repitch and Complex (Signalsmith) only.
7. **Storage.** My original proposal (UI-side JSON save/OPFS) was rejected. All file
   handling is engine-side, as specified in §2b: a `ProjectStore` of UUIDv7-named project
   folders, a `Library` for the browser, and upload reserved for later.
8. **Plugin state.** Accepted as proposed: the state blob is authoritative on load and
   params are mirrored in the document.

## 10. Stubs and ownership

| Node | Implements |
|---|---|
| `model` | every `todo!("model node")` in ether-model (`OrderKey::between`, `Project::apply/...`, `History`, `patch::changes_for`, `file::load/save`, `TempoMap`) |
| `core` | `engine.rs`, `graph::compile`, `tempo.rs` |
| `devices` | ether-devices |
| `media` | ether-media |
| `controller` | ether-controller |
| `clap` / `sandbox` / `stretch` | their crates (+ scanner) |
| `native-host` / `wasm-host` | ether-native + apps/desktop + `ui/src/transport/tauri` / ether-wasm + apps/web + `ui/src/transport/wasm` |
| ui-* | their feature folders / `ui/src/timeline` |

`ether-protocol` has no owner after foundation, so everything in it is implemented now.
That includes the normalized↔plain param mapping (`devices::scale_to_plain` /
`scale_to_normalized`, with test vectors the UI mirrors).

## 11. Roadmap v2 contracts (contracts-2)

Frozen for the 12 roadmap feature nodes; per-node files and hook points are in
[ROADMAP.md](ROADMAP.md). Everything is additive: v0.1 behaviour is unchanged until a node
implements its part (its commands reply `Unsupported`, new devices are pass-through
placeholders, new graph fields are empty or neutral). The MockTransport simulates all of it.

Conventions kept: one protocol domain file per feature (`Command::{Export, Tempo, Marker,
MidiMap, Groove, DrumRack, Slice, Collab}`), document edits are ops on normalized entity
tables with field-level `Update`s, client-chosen ids, patches before replies.

### 11.1 Export
- `Export::Render { job, request }` replies `ExportStarted`; progress, done, failed and
  cancelled arrive as `Event::Export`. One job at a time. `ExportRequest { range: Loop |
  Project | Custom, format { Wav | Flac, Int16 | Int24 | Float32 (not FLAC), sample_rate },
  mode: Mix | Stems { tracks }, normalize, tail_seconds, name }`.
- Rendering uses `ether_core::offline::OfflineRenderer` (a private engine on the controller
  thread) with fresh nodes, never the live ones. The metronome is never rendered. Stems are
  one pass per track: that track's post-fader output into master, sends included, master
  chain excluded.
- No UI paths: `ProjectStore::write_export` writes `<project>/exports/<file>` natively
  (`ExportResult::Files`, project relative); where it returns `Unsupported` (web, remote)
  the result is `Download` tokens read with `Export::ReadChunk` →
  `ReplyValue::Bytes` (base64, or binary frames over WebSocket) and dropped by `Release`.
- Plugins offline: `EngineBridge::create_offline_plugin` (defaulted `Unsupported`, which
  bypasses the plugin with a warning).

### 11.2 New built-in devices
`BuiltinDevice::{Eq, Reverb, Limiter, Utility, DrumRack}`. `BuiltinDeviceType::ALL` is the
`ListBuiltin` order and `BuiltinDevice::new(ty)` builds defaults. Param lists belong to the
owning node (append-only ids). `DeviceDescriptor::sidechain_inputs` (0 = none) is new on
every descriptor.

### 11.3 Tempo map and metronome
- `TempoCommand`: tempo-point and time-signature CRUD (the points at beat 0 can't be
  removed or moved), `SetMetronomeSettings { volume, accent, sound }` (partial). Undoable.
- Settings: `metronome_volume` (dB), `metronome_accent`, `metronome_sound: Classic | Wood |
  Beep`. The on/off switch stays `metronome` / `Transport::SetMetronome`.
- Engine: `RenderGraphDesc::click: MetronomeDesc { volume (linear), accent, sound,
  count_in_end }`. The click goes to the hardware output after master (not metered, never
  exported) while playing with `metronome` on, and during a recording count-in (`recording`
  and position `< count_in_end`) regardless of `metronome`. Hook:
  `ether_core::metronome::Metronome::render`, one call per sub-block in `engine.rs`. It
  receives the graph's total output latency: the click for beat `b` must be emitted
  `latency` samples after the timeline crosses `b`, so it lines up with the (PDC-delayed)
  music. Offline renders never contain it (`OfflineRenderer::publish` forces it off).

### 11.4 Clip editing
- `AudioContent::{fade_in_curve, fade_out_curve}: FadeCurve { Linear | EqualPower |
  Curve { tension } }` and `AudioContent::reversed`. The fade law is
  `ether_core::fades::fade_gain` (fade-outs mirror it in time), mirrored in
  `ui/src/features/clip-editing/fades.ts`. Fades stay audio-only and clip gain stays
  `AudioContent::gain`.
- **Crossfades.** Edits keep clips on a track from overlapping, except a crossfade overlap:
  the earlier clip ends inside the later one by at most both `A.fade_out` and `B.fade_in`.
  The engine sums clips, so a crossfade is two overlapping fades; `ClipCommand::Crossfade`
  creates one. Overlaps are not a model invariant.
- **Reverse.** The clip behaves as if its media were reversed; offset, loop and warp markers
  are expressed on that reversed timeline.
- `Marker { id, position, name, color }` + `MarkerCommand`; jump with `Transport::Locate`.

### 11.5 Remote engine (WebSocket)
`ether_protocol::remote`: the first text frame is `ClientHello { protocol_version, token,
client }`, answered by `ServerHello::Welcome { server: ServerInfo, session }` or
`Rejected { reason, message }` (then close 4001 auth / 4002 version / 4003 busy; an idle client is later closed with 4004). After that, text frames are
`ClientMessage`/`ServerMessage` JSON, and binary frames
`[kind u8][header_len u32 LE][header JSON][payload]` carry bulk bytes (`Bytes`: the
message's base64 `data` field travels raw; `Peaks`: f32 min/max arrays). The JSON forms stay
valid on every transport. Token auth (no token only on loopback). Uploads: `BeginUpload` →
ordered `UploadChunk`s → `Import { source: Upload }`, plus `CancelUpload` and
`MediaEvent::UploadProgress`, staged through the defaulted
`ProjectStore::{begin, append, read, discard}_upload`. Dev port offset `remote: 4`
(`scripts/dev-env.mjs`). `ether-server` (headless native host) and
`ui/src/transport/ws/WsTransport` are stubs; the frame codec is implemented on both sides.

### 11.6 Collaboration (reserved)
Model: `SiteId` (u64 as a decimal string), `ActorId`, `OpOrigin { site, actor, seq }`,
`StampedTransaction`. Protocol: `CollabCommand` (Join, Leave, SetPresence), `CollabEvent`
(Session, Presence), `Presence`/`PresenceState`, and the engine-to-engine `CollabMessage`
(Hello, Transaction, Update, SyncRequest, Snapshot, Presence, Leave), and
`Patch::origin: Option<OpOrigin>` (omitted when `None`). Everything replies
`Unsupported`; the `collab` node refines it through BCRs. Remote edits will reach UIs as
ordinary patches, and undo stays per site.

### 11.7 Multicore partition contract
`EngineConfig::worker_threads` (0 = everything on the audio thread, the default; ignored on
wasm32). The core never spawns threads; hosts supply an RT-safe `parallel::ParallelExecutor`
through `Engine::set_executor`. Jobs get disjoint `&mut` access to their tracks through
raw pointers on the engine side, sound only because the executor runs every index exactly
once and returns after all jobs finished (see `parallel.rs`). The snapshot is
partitioned into DAG levels (routing, sends, resampling inputs, sidechains). Tracks of one
level run as independent jobs (clips → automation → chain → fader → meters, into their own
buffers). Bus mixing happens after each level on the audio thread in a fixed order, so the
output is bit-identical to sequential processing. Each node belongs to one chain, so jobs
need no locks.

### 11.8 Web perf: graph codec
`ether_core::codec::GraphCodec { encode, decode }` for the Worker → Worklet snapshot: exact
round trip, version byte first, unknown versions rejected (`CodecError::Version`). On the
web, `decode` runs in the AudioWorklet (no other thread there) and allocates, like today's
JSON path: the documented RT exception on the web, to keep cheap and bounded.

### 11.9 MIDI learn
`MidiMapping { id, source: MidiSource { port?, channel?, control: Cc | Note | PitchBend },
target: Param { AutomationTarget } | TrackMute | TrackSolo | TrackArm | Transport { action },
min, max (normalized; min > max inverts), mode: Absolute | Relative { encoding } | Toggle }`.
One mapping per source, saved in the document. `MidiMapCommand::{Map, Edit, Unmap}` are
undoable; `Learn { target? }` (runtime, `MidiMapEvent::LearnChanged`/`Learned`) and `List`
(`ReplyValue::MidiMappings`) are not. Host input: `MidiInputEvent { port, data: [u8; 3],
time_ms }` through `EngineBridge::poll_midi_input`, fed natively from the recording node's
MIDI input path (its `LiveMidi` callback, extended with the port id). Mapped messages become
ordinary edits at tick rate (one gesture per control) and still reach monitored tracks.

### 11.10 Sidechain
`Device::sidechain: Option<TrackId>`, `DeviceCommand::SetSidechain`, `ChainEntry::sidechain`,
`Node::{sidechain_inputs, process_sidechain}` (defaulted).
- Tap: the source track's **post-fader** output **before** its PDC output delay, so its
  latency is exactly `out_lat(source)` (final when the consumer is compiled, since sources
  come first).
- Order: a sidechain is a routing edge `source → consumer track`. The model rejects cycles
  and the compiler orders the source first. It is not a bus connection (no effect on
  `in_lat` of anything).
- PDC (base-24, implemented in `ether-core/src/sidechain.rs`, tested in
  `ether-core/tests/base24_hooks.rs`): the sidechain must reach device *k* of track T
  aligned with T's main signal there. Walk T's chain with `L = in_lat(T)`; at a sidechained
  entry with `L_sc = out_lat(source)`: if `L_sc > L`, the **main signal** is delayed by
  `L_sc − L` just before entry *k* (a delay line counted into T's chain latency, applied
  whether T plays clips, receives buses or both, and whether the entry is bypassed or not);
  otherwise the **sidechain** is delayed by `L − L_sc`. Then `L += latency(entry k)`. T's
  output latency includes the main delays and downstream PDC absorbs it.
- The tap is post-fader *and* post mute/solo gate: a muted (or solo-silenced) source gives
  a silent sidechain.
- Devices on drum pads cannot have a sidechain (the model rejects `sidechain` on a device
  with `pad`); pad chains never carry one.

### 11.11 Groove
`NoteCommand::Quantize::swing` (0..=1, destructive: odd grid positions are delayed by
`swing·grid/3`). `GrooveCommand::Humanize { clip, notes?, timing, velocity, seed }` is
deterministic from `seed`. `GrooveCommand::SetSwing` sets `ProjectSettings::{swing,
swing_grid}`, a non-destructive **playback** groove the controller applies when compiling
MIDI clips (`groove::swing_notes`).

### 11.12 Drum rack and slicing
- `BuiltinDevice::DrumRack` sits on a track chain. `DrumPad { id, rack, note (unique per
  rack), name, color, choke_group (1..=16), volume, pan, mute }`. Pad chains are devices
  with `Device::pad = Some(pad)` on the rack's track (no nested racks).
  `Project::devices_of` excludes them and `pad_devices_of` lists them. Removal order: pad
  devices, pad, rack (the controller cascades).
- Engine: `TrackDesc::racks: Vec<RackDesc { rack: NodeKey, pads: Vec<PadDesc> }>`. At the
  rack's chain entry the engine routes notes by key to pads (transposed to
  `PAD_PLAY_NOTE` = 60), applies choke groups, runs the pad chains (PDC-aligned to the
  longest), mixes them (volume/pan/mute) and runs the rack node.
- Slicing: `BuiltinDevice::Sampler { sample, slices: SliceSettings { enabled, base_note,
  markers } }`. The data travels with the node; edits reach a live sampler in place through
  `EngineBridge::update_builtin` → `EngineHandle::set_node_data` → `Node::set_data`
  (falling back to re-creating the node). `SliceCommand` edits markers by sorted index (a
  single LWW register, like other device kind data); `ToDrumRack` turns slices into sampler
  pads with client-chosen ids (`SlicePadIds`), so concurrent sites can't mint different ids.
- Pad-chain devices are first-class engine nodes (base-24): `SnapshotRt::pad_index` routes
  live params and automation to them, their latency is refreshed like chain nodes, and the
  rack entry's latency includes the longest pad chain (all pads aligned to it; the chain
  audio entering the rack is delayed the same). A pad is delayed by
  `longest − Σ latency of its enabled entries` (bypassed pad devices are skipped when
  processing but still count in the rack's latency, like bypassed track devices). Basic
  `run_pads` is implemented: notes route by key (the pad's key becomes `PAD_PLAY_NOTE`,
  other keys are dropped); raw MIDI note-on/off and poly aftertouch route the same way,
  channel-wide raw messages (CC, pitch bend, channel pressure) reach every pad;
  `AllNotesOff` reaches each pad chain once; pad chains, alignment and pad gain. Choke
  groups and pad mix smoothing are left to `drum-rack`. Pad devices cannot have a
  sidechain.
- Structure rules: `Device::Move` rejects pad devices (`DrumRack::MoveDevice` moves them) and
  racks with pads across tracks; device and track duplication copy pads and pad chains; the
  model checks pad devices from both sides (pad device and rack).

### 11.13 `.ether` v3 defaults
Tables `markers`, `midi_mappings` and `drum_pads` start empty. Settings: `metronome_volume`
-6 dB, `metronome_accent` true, `metronome_sound` Classic, `swing` 0, `swing_grid` 0.25.
Devices: `sidechain` and `pad` null; samplers get `slices` off (base note 36). Audio clips:
fade curves Linear, `reversed` false. Tested on a realistic v2 fixture
(`ether-model/tests/fixtures/v2_full.ether`).

### 11.14 Choices worth reviewing
1. **Fade curves are separate fields** next to the existing `fade_in`/`fade_out` lengths
   instead of a nested `{ length, curve }`, and gain/fades stay on `AudioContent`. This is
   purely additive for the engine (`ClipContentDesc` keeps its fields) and for merged code.
2. **Slices live in the sampler's device kind**, not as entities, so they reach the engine
   with the node on both hosts. Edits address markers by index.
3. **Pad chains reuse `Device`** with a `pad` parent pointer instead of a second device
   table, so params, automation, plugins and MIDI mapping work on pad devices unchanged.
4. **`sidechain_inputs` is on every descriptor**, touching every descriptor literal once now
   so feature nodes don't have to.
5. **Project swing is a playback groove** compiled by the controller; `Quantize::swing` is
   the destructive variant.
6. **MIDI mappings are document entities** (saved and undoable, like Ableton); learn mode is
   runtime state.
7. `PatchChange` allows `clippy::large_enum_variant`: entities got larger, and boxing them
   would only add allocations.

### 11.15 Media preview (base-24, `media-preview`)
`Media::Preview { source }` / `StopPreview` → one engine preview voice
(`ether_core::preview`), mixed into the hardware outputs after master (not metered,
recorded or exported; plays while the transport is stopped), fed through
`EngineHandle::preview(PreviewControl::{Play { id, source, gain }, Stop})` with an ordinary
`AudioSource` (decoded and resampled engine-side; replaced/stopped/finished sources are
retired to the GC). Events: `MediaEvent::PreviewStarted { source }`, then exactly one
`MediaEvent::PreviewEnded { source, reason: Finished | Stopped | Replaced | Failed }` per
preview. **Preview ids (frozen):**
- the controller gives every `Play` a new monotonic `u64` id (`EngineBridge::preview(id,
  audio, gain)`, `audio: None` = stop);
- the engine reports **natural ends only**: `EngineOutputs::preview_ended = Some(id)` (the
  latest if several ended between two polls); stop and replace are never reported;
- the controller emits `Stopped`/`Replaced` itself when it sends them, and `Finished` only
  when the reported id is still its current preview (a late end of a replaced preview is
  ignored).
Controller hook: `EngineBridge::preview` (defaulted `Unsupported`), `media_preview` module
(`preview_command`, `preview_tick`). The MockTransport (`MockPreview`) follows the same
event rules.

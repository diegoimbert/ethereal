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
  thread) with fresh nodes, never the live ones, stepped a bounded unit of work at a time
  from the controller tick. The metronome and count-in are never rendered
  (`OfflineRenderer::publish` forces them off), looping and inputs are off, and the graph
  latency is dropped at the start. The result equals a live render of the same project
  sample-exactly when both engines use the same block size; automation and tempo ramps are
  evaluated per block, so other block sizes differ slightly.
- Stems are one pass per listed track: that track's post-fader output (chain, fader, pan)
  goes straight into master, and its sends feed the returns (return processing included).
  Every other source is silent. The master chain is excluded (master fader and pan are
  kept). Solo is ignored, and a muted track is unmuted for its own stem. A child of a group
  bypasses the group's processing in its own stem, while a group's stem includes its
  children through the group. A return's stem is everything sent to it. Selecting a track
  together with its group, or with a return it sends to, puts that audio in both files.
  Sidechain sources on other tracks are silent. Stems therefore sum to the mix only with
  neutral master devices, neutral groups, linear returns and no sidechains.
- Files: a name already in `exports/` gets a ` (2)`, ` (3)`, ... suffix (never overwritten).
- No UI paths: `ProjectStore::write_export` writes `<project>/exports/<file>` natively
  (`ExportResult::Files`, project relative); where it returns `Unsupported` (web, remote)
  the result is `Download` tokens read with `Export::ReadChunk` →
  `ReplyValue::Bytes` (base64, or binary frames over WebSocket) and dropped by `Release`.
- Plugins offline: `EngineBridge::create_offline_plugin` (a fresh instance with the current
  state; defaulted `Unsupported`). If a plugin can't be instantiated offline the export
  fails with an error naming it; plugins are never skipped. Disabled plugin devices are not
  instantiated. Notifications from offline instances never reach the document.

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
buffers). Bus mixing gathers each destination's inputs (at the start of that destination's
job) in the same fixed order as sequential processing, independent of worker count, so the
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

### 11.16 Presence v2 and listen-on-peer (base-53)
Design: docs/COLLAB.md §8-§11. All additive and append-only in `ether_protocol::collab`:
- `PresenceState` gains `viewport`, `activity`, `following`, and the controller-owned
  `listening_to`, `can_host` (all `#[serde(default)]`, omitted when unset). `cursor` keeps
  its meaning (edit cursor). The live pointer is a separate channel:
  `CollabCommand::SetPointer` → `CollabMessage::Pointer { site, pointer }` →
  `CollabEvent::Pointer`, `ArrangerPointer { beats, track, y }` in song coordinates
  (≤ 30 Hz, `POINTER_MAX_HZ`).
- Site-to-site messages `CollabMessage::{Signal, Listen, Unlisten, TransportRequest,
  StreamClock}` carry `from`/`to` (`CollabMessage::route`): the relay checks `from`, delivers
  to `to` only (same session, synced). `IceServers` is relay → site only
  (`Relay::set_ice_provider`, refreshed every `RelayConfig::ice_refresh_ms`).
- Commands `Listen { host }`, `StopListening`, `SendSignal`, `SendStreamClock`, `SetHosting`,
  `SetIceServers`; events `Signal`, `ListenStatus`, `StreamClock`, `IceServers`. Until their
  node lands, `SetPointer` (presence-v2), `Listen`/`StopListening` (stream-listen),
  `SetHosting`/`SendStreamClock` (stream-host) reply `Unsupported`; `SendSignal` and
  `SetIceServers` work (`ether-controller/tests/collab_prewire.rs`).
- Stream clock: `StreamClock { rtp, position, ..., count_in_end }` = "the sample with RTP
  timestamp `rtp` in this listener's stream is the timeline at `position`" (48 kHz,
  wrapping u32), every 100 ms and at every discontinuity. `rtp` = the RTP time of the
  rendered sample **plus the graph latency** (when it is heard), for jumps too
  (COLLAB.md §9.4, worked example); count-in anchors carry the pre-roll position.
- Engine: `ether_core::stream_tap` (`EngineHandle::set_stream_tap`): a copy of the render
  after master + metronome/count-in and before the preview voice, into pre-allocated `rtrb`
  rings (block headers with `jump` on play/locate/loop wrap, + interleaved stereo),
  RT-safe.
- `EngineBridge` (defaulted): `stream_capabilities`, `start/stop_stream_capture`,
  `stream_open/stream_signal/stream_close`, `poll_stream` (`streaming::StreamOutput`), and
  plugin GUI mirrors `create/destroy_plugin_mirror`, `set_plugin_mirror_param`.
- Relay limits (`relay::limits`): presence 20 Hz, pointer 40 Hz (clears always pass),
  site-to-site 200/s (dropped, not disconnected).

## 12. v0.2 contracts (contracts-3)

Frozen for the v0.2 nodes; per-node files, hook points and shared touches are in
[ROADMAP.md "v0.2"](ROADMAP.md#v02-contracts-3). Everything is additive and append-only:
v0.1 behaviour is unchanged until a node implements its part (its commands reply
`Unsupported`, pinned by `ether-controller/tests/roadmap_v3.rs`; new devices are
placeholders; new desc fields are empty). The MockTransport routes every new command to
one file per node (`ui/src/transport/mock/roadmap/`). The repo owner decided four UX
questions before the freeze: Bitwig-style modulation, take lanes with swipe comping,
declarative device layouts with one shared renderer, and samples referenced in place.

New domains: `Command::{Take, Freeze, TimeEdit, Preset, Browser, Analysis, Rack, Modulation,
MediaRef}`; `Event::{Freeze, Preset, Browser, Analysis, MediaRef}`; replies
`RenderStarted`, `Presets`, `Preset`, `BrowserPage`, `BrowserRoots`, `ModulatorKinds`,
`MissingMedia`. New variants on existing domains: `Track::{GroupSelected, Ungroup,
SetVca}`, `Device::SetZones`, `MediaSource::Path` (file-import).

### 12.1 Ids, entities, `.ether` v4
- New tables (`#[serde(default)]`): `take_lanes`, `comp_regions`, `rack_chains`,
  `modulators`, `mod_mappings`. New optional fields (omitted from JSON when `None`, TS
  optional): `Clip::lane`, `Device::chain`, `Track::{freeze, vca}`. New defaulted fields:
  `MediaRef::location`, `TrackInput::Track::tap`. New variants: `TrackKind::Vca`, 23
  `BuiltinDevice`s, `EntityKey`/`Entity`/`EntityUpdate` rows, `TrackChange::{Freeze, Vca}`,
  `ClipChange::Lane`, `DeviceChange::Chain`, `MediaChange::{Location, Hash}`.
- `.ether` v4: `V3ContractsV3Defaults` adds the empty tables and `location: Project` on
  media (idempotent, tested). The bump makes a v0.1 app refuse v4 files (`TooNew`) instead
  of silently dropping takes, racks and modulation when re-saving.
- **Collab-safe ids** (`ether_model::derive_id(seed, i)`): a command that creates a number of
  entities unknown to the client takes one client-chosen seed id and derives the rest in a
  documented order (`Take::Flatten`, `Freeze::{Flatten, Consolidate}`, every `TimeEdit`
  that creates entities). Replays on another site mint the same ids.
- `Project::entities()` order: … markers, take lanes, track-chain devices, drum pads, rack
  chains, pad/rack-chain devices, modulators, sends, clips, comp regions, notes, … MIDI
  mappings, modulation mappings.

### 12.2 Takes and comping (`comping`)
Model (`ether_model::take`): `TakeLane { track, order, name, color }` (audio/MIDI tracks);
take clips are ordinary clips with `Clip::lane = Some(lane)` (so notes, fades, warp,
envelopes and clip commands work unchanged); `CompRegion { track, lane, start, end,
crossfade }` selects what plays. Regions of a track never overlap (model invariant;
half-open ranges). `arrangement_clips_of` lists main-lane clips only.
- Playback (controller compile, `comping::comp_clips`): take-lane clips are never compiled
  directly; each region plays its lane's clips trimmed to the region. Adjacent regions
  overlap by the later region's `crossfade` seconds (default 5 ms, max 0.5 s) centred on
  the boundary with equal-power fades; outer edges get `crossfade / 2`. MIDI ignores
  crossfades (notes cut at the region end). Main-lane clips still play.
- Swipe comping is **one** undoable `Take::SetComp { id, split_id, track, lane, start, end }`:
  the range becomes a region of `lane`; existing regions are trimmed, split (right part =
  `split_id`) or removed; adjacent same-lane regions are not merged. `ClearComp`,
  `SetCrossfade`, `Flatten` (bake the comp into main-lane clips, ids from the seeds).
- Recording: each loop/punch pass becomes a lane with its clip and a region spanning the pass
  that selects the newest lane. A pass over existing main-lane clips first moves their parts
  inside the recorded range onto a new lane, the **first take** (edge-crossing clips are split),
  so only the comp sounds there; one undo step with the recording.

### 12.3 Freeze, flatten, bounce, consolidate, time edits (`freeze-bounce`, `time-edits`)
- `Track::freeze: Some(TrackFreeze { media, start })`: the track plays its render (post-chain,
  pre-fader, latency removed) at **song time** (frame `f` at song second `start + f/rate`,
  whatever the tempo map). Clips and devices stay in the document but are neither compiled
  nor instantiated (CPU saved). Fader, pan, mute/solo, sends and routing stay live. Only
  audio/MIDI tracks; the media is project media. The controller rejects edits to a frozen
  track's clips, devices and device automation (`freeze::check_editable`, `InvalidState`).
- Engine: `TrackDesc::frozen: Option<FrozenDesc>` (+ empty clips/chain), rendered by
  `ether_core::freeze::render_frozen` where clips render (pre-wired).
- Render jobs (`Freeze`, `Bounce`, audio `Consolidate`) reply `RenderStarted { job }` and run
  on an `OfflineRenderer` from the controller tick (like export; one job at a time,
  independent of export), then `FreezeEvent::{Progress, Done, Failed, Cancelled}`. The
  document changes once, at `Done` (media insert + edit, one undo step). Renders always land
  in the project's `media/`.
- `Flatten` makes a freeze permanent (audio track: clips and devices replaced by one clip of
  the render; MIDI track: replaced by a new audio track at its position, sends copied with
  derived ids). The clip is warped to the tempo map so it plays exactly the render.
- `Bounce { track, start, end, include_chain, target }`: `NewTrack` (a new audio track below,
  source muted) or `InPlace` (audio tracks, pre-chain only). `Consolidate` joins clips per
  track over a range (MIDI instantly; audio rendered pre-chain).
- `TimeEdit` (`ether_protocol::time_edit`): `Split` across tracks, `Copy`/`Cut`/`Paste`
  (controller-side time clipboard, runtime only), `DeleteTime`, `InsertSilence`,
  `DuplicateTime`. Selections are a beat range + tracks (empty = all audio/MIDI/group);
  time shifts move clips, take clips, comp regions and track automation after the range,
  and with `global` also markers, tempo points and time signatures. One undo step each.

### 12.4 Devices
#### 12.4.1 New built-ins and frozen param tables
`BuiltinDevice` gains, in `BuiltinDeviceType::ALL` order: `PolySynth` (synth-2),
`MultiSampler { zones }` (multisampler), `Saturator`, `Bitcrusher`, `AutoFilter`
(fx-color), `Chorus`, `Phaser`, `Flanger`, `Tremolo` (fx-modulation), `Gate`,
`MultibandCompressor`, `TransientShaper` (fx-dynamics), `SpectrumAnalyzer`, `Tuner`
(fx-analysis), `Arpeggiator`, `Chord`, `ScaleQuantize`, `NoteLength`, `Velocity`,
`Randomizer` (midi-fx), `InstrumentRack`, `AudioEffectRack`, `MidiEffectRack`
(racks-modulation). The poly synth's `Wavetable` oscillator type plays a table chosen by
`OSC1_TABLE`/`OSC2_TABLE` from a small built-in set shipped with the device
(`poly_synth/tables/`; user wavetables later, as appended kind data). Devices with a detector take a sidechain (`sidechain_inputs = 2`,
keyed in `Node::process_sidechain` like the compressor/limiter): `Gate`,
`MultibandCompressor` (the key drives all bands), `AutoFilter` (envelope follower). Each group
module in `ether-devices` (`poly_synth`, `multisampler`,
`fx_color`, `fx_modulation`, `fx_dynamics`, `fx_analysis`, `midi_fx`, `racks`) defines the
**final descriptor** (dense param ids, documented in the module's table, append-only, named
constants in `<group>::<device>::*`) and starts as a `contract::Placeholder` (audio effects
and racks pass through, instruments are silent, MIDI effects forward events). Descriptor ↔
mock parity is enforced (`ether-devices/tests/v02_descriptors.rs` against the generated
`ui/src/transport/mock/devices/*.json`). `ParamInfo::step` (v0.2, `#[serde(default)]`, TS optional): the plain step of integer and
stepped params (1 for every enum/toggle, transpose, root key, voice or count param of every
built-in; `None` = continuous); knobs, automation lanes, MIDI learn and the renderer snap to
it (`ParamInfo::snap`), the scale helpers don't. `BuiltinDeviceType::key()` (kebab case) names preset
folders. Multisampler zones (`ether_model::multisampler::SampleZone`: key/velocity zones,
root, tune, round-robin group, start/end, loop, gain, pan) live in the device kind and are
edited with `Device::SetZones`; zone media may be external references.

#### 12.4.2 Declarative layouts (`device-ui`)
`DeviceDescriptor::layout: Option<DeviceLayout>` (`ether_protocol::layout`, generated to
TS): sections (`id`, `title`, `span` 1..=4, `columns` 1..=8) of items (`widget`, `size`
Small/Medium/Large, `colspan`, `label`). Widget catalog (append-only, BCR to extend): param
widgets `Knob`, `Slider`, `Toggle`, `Choice`, `Number`; typed widgets `Envelope`,
`FilterCurve`, `TransferCurve`, `Oscillator`, `Lfo`, `StepEditor`, `XyPad`, `Crossover`;
data widgets `SampleWaveform`, `ZoneMap`, `Spectrum`, `Tuner`, `Meter`, `RackChains`,
`Macros`, and `EqCurve` (§12.15, the EQ only for now). **One shared renderer** (`ui/src/features/devices/layout/`, kit components and
tokens only) renders every built-in; devices without a layout (plugins, v0.1 devices until
they add one) get the generic layout grouped by `ParamInfo::group`. Device nodes write
specs and widget data only, never bespoke panels. `ModulatorDescriptor::layout` uses the same
catalog (the Steps editor is `StepEditor`). A shared validator
(`ether-devices/tests/layouts.rs`) checks every built-in and modulator layout: referenced
params exist (typed widgets included), `colspan <= columns`, spans/columns in range, unique
section ids, one `EqCurve` shape per `kind` label. The renderer adds MIDI-learn targets
(`midiTarget()`), modulation drop targets and depth rings to every param widget.

#### 12.4.3 Analysis channel (implemented)
Node opt-in `Node::has_analysis()` (queried once when added) and RT `Node::analysis(&mut
AnalysisSink)`: the node copies the latest results computed in `process` into up to
`ANALYSIS_FRAMES_PER_PASS` (4) pre-allocated frames per pass (`sink.frame(kind)`), e.g. the
EQ's pre **and** post spectrum in the same pass (no alternating). The engine collects from
the watched live nodes after all track jobs, at most `ANALYSIS_HZ` (30) times per second,
into a fixed ring of `Copy` frames (`ANALYSIS_RING` = 128, `ANALYSIS_MAX_VALUES` = 1024; a
node is only asked when the ring has room for a whole pass; no allocation). Modulation
readback is pushed first each pass, so device frames can't starve it. `EngineHandle::poll_analysis` →
`EngineBridge::poll_analysis` (native done; web: forward from the worklet, `fx-analysis`)
→ the controller keeps the latest frame per (device, kind) per tick and emits
`Event::Analysis { Frame { device, data } }` only for devices watched with
`Analysis::Watch`. Watches are refcounted in the controller and owned per connection: the
remote router releases a disconnecting client's watches (`Unwatch` per watch held; an
`Unwatch` a client doesn't hold is a no-op). Project loads don't touch them (watches of
devices that no longer exist are inert, and the router's per-client counts stay exact);
single-connection hosts call `EtherController::reset_analysis_watches` when their UI
reconnects. An engine watch whose queue push fails is retried on the next tick. Only watched
nodes are collected (`EngineHandle::watch_analysis`, synced by the controller each tick,
node re-creation included), round-robin so a full ring never starves the same nodes; a
node re-added into a reused slot index replaces the stale entry. Kinds and encodings:
`Spectrum` (`[min_hz, max_hz, bins_db…]`, ≤ 256 log-spaced bins), `Tuner` (`[hz|0, note|-1,
cents, confidence, level_db]`), `Levels` (device-defined meters, e.g. gain reduction),
`Modulation` (`[param bits, base, effective]` triples, racks-modulation readback).

#### 12.4.4 MIDI effects (`midi-fx`)
Category `NoteEffect` (= MIDI effect). The engine already feeds clip notes to chain entry 0
and each entry's `out_events` to the next entry. Contract: a MIDI effect has
`channels() == (0, 0)` (audio untouched) and `audio_inputs == audio_outputs == 0`, consumes
`ctx.events` and writes the transformed note/MIDI stream to `ctx.out_events`, forwarding what
it doesn't transform and every `AllNotesOff`, never `Param` events; generated notes use ids
`0x8000_0000 | n`; it only delays (never earlier than its input) and reports no latency.
Chain rule: MIDI effects precede the first instrument (`midi_fx::check_chain_order` after
`Device::{Insert, Move}`, `InvalidArgument`). Scale Quantize receives the resolved
`MusicalScale` (track scale, else project) through `Node::set_data` on creation and on
scale changes. Plugins with MIDI output keep working as before (their events feed the next
device).

### 12.5 Presets (`presets` + each device node)
File format `ether_model::preset` (`.etherpreset`, JSON `{ format: "ethereal-preset",
version: 1, app_version, preset: { name, device: Builtin { device } | Plugin { format,
plugin_id, name, vendor }, meta { tags, author, description }, params, kind, samples,
state } }`; unknown params ignored on load, missing ones reset to defaults; kind must match
the device type). Factory presets are embedded per device type
(`ether_devices::factory_presets(ty)`, ids `"<device-key>/<slug>"`, files under
`crates/ether-devices/presets/<device-key>/`, parsed and checked by
`v02_descriptors.rs`). User presets live engine-side under
`<user library>/Presets/<device-key>/…` (plugins: `Presets/plugins/<format>/<id>/…`) through
the defaulted `Library::{write_file, remove_file, rename_file, user_root}`; the UI never
touches files. `Preset::Load` is one undo step; `List/Save/Rename/Delete/SetMeta` are
runtime; `PresetEvent::Changed` after user-set changes. Sample-based presets carry
`samples` (library location + hash) that loading references/imports first.

### 12.6 Racks, macros and modulation (`racks-modulation`, Bitwig-style)
- Racks (`ether_model::rack`): `RackChain { rack, order, name, color, volume, pan, mute, solo,
  keys, velocities, select }`; chain devices are ordinary devices with `Device::chain`
  (on the rack's track, exclusive with `pad`). No nesting in v0.2 (no racks or drum racks
  inside chains or pads). Every rack's params: macros `0..8` (plain 0..1) and the chain
  selector (`8`, 0..=127). Engine: `TrackDesc::chain_racks` (`ether_core::rack_chains`),
  run at the rack entry before the rack node (pre-wired like drum-rack pads: params,
  automation and latency already reach chain nodes).
- Modulators live **inside any track-chain device** (not on drum-pad or rack-chain devices in
  v0.2, so every host is an entry the engine's `pre_node` hook sees; a rack's modulators reach
  its chain devices) (`Modulator { device, order, name, kind, params, sidechain }`,
  kinds `Lfo`, `Envelope`, `EnvelopeFollower`, `Steps`, `Random`; param tables frozen in
  `ether_devices::modulators`). `ModMapping { source: Modulator | Macro { rack, index },
  device, param, depth -1..=1 }`, one per (source, target). Scope: a modulator targets its
  host and, for a rack host, devices on the rack's chains, never the rack's own macros (no
  modulation of modulation in v0.2); a macro targets devices on its rack's chains and the
  rack's own non-macro params.
- **Envelope-follower sidechain:** `Modulator::sidechain: Option<TrackId>` (followers only,
  `Modulation::SetSidechain`) follows that track's sidechain tap (post-fader, before its PDC
  delay, like device sidechains §11.10) instead of the host's input. It reuses the sidechain
  machinery: a routing edge `source → host track` in the model's cycle check, ordered first
  and tapped by `graph.rs`, gathered in the consumer's job (`ModulationRt::write_sidechain`,
  pre-wired). PDC: when the source is earlier than the host's input position the tap is
  delayed to align; when it is later the modulation lags by the difference (a control signal
  never delays the audio). Deleting the source track cuts it (`doc/mod.rs`, done).
- **Composition (frozen):** `effective = clamp(base + Σ depth_i · m_i, 0, 1)` in normalized
  units, then the param's scale (stepped params snap after the sum). `base` = the enabled
  automation (arrangement lane or clip envelope, v0.1 precedence) else the document value
  (knob, `SetParam`, MIDI learn, presets); modulation never writes the document. Sources:
  LFO/Steps/Random bipolar, envelopes/followers/macros unipolar. Engine hooks
  (`ether_core::modulation`, pre-wired): `intercept` (every base write from the param
  queue and automation goes through it; `true` = stored as base), `render` once per
  sub-block after automation (emits `Param` events on the automation grid), `pre_node`
  (follower/envelope inputs), `readback` (depth rings via the analysis channel);
  `ParamTarget::Modulator` for live modulator params.
- **Plugins:** host-side modulation sends the effective value as ordinary parameter changes
  (CLAP param value events, VST3 `IParameterChanges`, AU scheduled params). Caveats: the
  plugin GUI shows the modulated value, the plugin may mark itself dirty, resolution is the
  automation grid; the document keeps the base and the controller ignores `ParamEdited`
  echoes of modulated params. CLAP `PARAM_MOD` may be used later.
- UI: the knob shows the base, a ring shows the effective value (`AnalysisData::Modulation`
  for watched devices, ≤ 30 Hz).

### 12.7 Sample-accurate automation (`sample-accurate-automation`)
The node parameter-event API is `EventKind::Param { param, value }` at a sample `offset`
(already delivered to every node, sorted). Built-ins apply them at their offset
(`split_at_events`); hosts forward offsets: CLAP `clap_event_param_value.header.time`, VST3
`IParamValueQueue::addPoint(sampleOffset)`, AU `AudioUnitScheduleParameters` with
`eventSampleTime`. A node that ignores offsets applies changes at block start (the
backwards-compatible default). What changes (engine only, `ether_core::automation_rt`,
extracted verbatim from `engine.rs` by contracts-3):
- node params: events on an **absolute grid** (`sample_time` multiples of `PARAM_GRID` =
  32) plus one at every breakpoint, each evaluated at the exact beat of its sample;
- tempo ramps integrated exactly per sample for scheduling and automation (`Timing`);
- mixer targets (volume, pan, sends) ramped per sample to the value at each grid point;
- acceptance: an offline render of the same project is identical (±1e-6) with block sizes
  64 and 512, with automation and tempo ramps; with neither, bit-identical as today.
Modulation uses the same grid.

### 12.8 Browser v2 (`browser-v2`)
Engine-side index (controller `browser/`, persisted at `<user library>/.ethereal/
index.json`): `LibraryItem { id: "<root>/<path>", kind: Audio | Midi | Preset | Project,
name, root, path, source, preset, tags, favourite, meta { duration, rate, channels, bpm,
key, pack, modified, size } }`. `Browser::Query { text, kinds, tags, favourites_only, roots,
folder, device, sort: Name | Recent | Duration | Bpm | Relevance, offset, limit ≤ 200 }` →
`BrowserPage { items, total, offset }`. `ListRoots` (library, packs, user folders, factory
presets), `SetFavourite`, `SetTags`, `AddFolder { path }` (native only; the desktop shell
picks it), `RemoveFolder`, `Rescan`; background indexing with `BrowserEvent::{IndexProgress,
IndexChanged}`. `Preview { item, sync }` uses the media-preview voice; `sync` repitches by
project bpm / item bpm and starts on the next beat while playing. Presets appear as items
(kind `Preset`).

### 12.9 Media references (`media-references`)
`MediaRef::location: Project | External { path }` (default `Project`). v0.2 imports from a
library location **reference the file in place** (`External`, content hash required, nothing
copied) where the host can (`Library::external_path`); web/remote copy as before.
Recordings, bounces, freezes and uploads from a remote UI stay project media. Resolution on
every site: external path whose hash matches → the project copy at `MediaRef::file` →
missing (`MediaEvent::Missing`, silence). `MediaRef` commands: `ListMissing`, `Search`
(library roots and user folders, by hash then name; unambiguous hash matches relinked
automatically, else `Candidates`), `Relink { media, source }` (undoable; hash updated with a
warning when content differs), `CollectAll` (copy every external reference into `media/`,
switch to `Project`, save). Collab/remote: peers receive media by hash and store it at
`MediaRef::file` in their own project (the document is shared; availability is per site).
The collab push by hash (`collab/mod.rs`) belongs to `file-import` (§12.13), including
external-path media; `media-references` doesn't touch it.
Migration: v0.1 media are `Project`; nothing moves.

### 12.10 Groups, buses, input taps, VCAs (`groups-buses`, priority 1)
- `Track::GroupSelected { ids, group, name }`: same parent required, nesting allowed, the
  group takes the first track's position, children keep their order; one undo step, no
  other ids. `Ungroup { group, force }`: children move to the group's parent at its place;
  the group's devices, automation, sends and explicit outputs pointing at it are lost
  (outputs reset to `Default`), so without `force` it replies `InvalidState` if any exist.
  Moving tracks in/out of groups is the existing `Track::Move { parent }`.
- Mute/solo (compile rules, `graph.rs`): muting a group mutes everything inside (and a
  muted VCA its tracks); while anything is soloed a track is audible if it is soloed, inside
  a soloed group, a group/bus on the output path of a soloed track (solo in place through the
  bus chain), a return, or master; soloing a VCA solos its tracks (folded at compile).
- `TrackInput::Track { track, tap: PreFx | PostFx | PostFader }` (default `PostFader`): a
  routing edge (no cycles, model and compiler). PDC: tap latency = `in_lat(source)` (PreFx)
  or `out_lat(source)`; the consumer's `in_lat` includes it and the tap is delayed by the
  difference (pre-wired in `graph.rs`, `ether_core::bus_tap`), so the consumer hears and
  records it aligned. It is heard when the consumer monitors.
- VCAs: `TrackKind::Vca` (no audio, clips, devices, sends or input; top-level), assignment
  `Track::vca` (VCAs can nest, no cycles, master excluded). Effective gain = own fader +
  Σ VCA faders in dB up the chain, applied after the fader (`ether_core::vca`, pre-wired);
  VCA volume is automatable and its fader/mute live (`ParamTarget::{TrackVolume, TrackMute}`
  with the VCA id). VCA tracks are compiled into `RenderGraphDesc::vcas`, never `tracks`.

### 12.11 Engine pre-wiring (contracts-3, hot files touched once)
`engine.rs`: analysis collection after the jobs (watched nodes only); `freeze::render_frozen` in the clip stage;
`bus_tap` gather/mix/write at PreFx/PostFx/PostFader; `vca.update` before the jobs and
`vca.apply` after the fader; rack-chain `run` at the rack entry plus param/automation
routing and latency refresh for chain nodes; `modulation::{intercept, render, pre_node,
readback}`; `ParamTarget::Modulator`; `automation_rt::apply_automation` (moved).
`graph.rs`: `TrackDesc::{frozen, chain_racks, modulation, input_tap, vca}`,
`RenderGraphDesc::vcas`, tap ordering and PDC, rack-chain index/latency, VCA mute in gates.
`mixer.rs`: the per-track hook state. `codec.rs` (v2): the v0.2 fields as a tagged JSON blob
(`0` = all default, no allocation).

### 12.12 Choices worth reviewing
1. **Take clips and rack-chain devices reuse `Clip` and `Device`** (a `lane` / `chain`
   parent pointer), like drum-pad devices, so every existing command, patch and UI path
   works on them.
2. **Modulators are entities inside a device**, not devices in the chain (the owner's
   Bitwig choice); macros are modulation sources rather than direct param owners.
3. **Freeze plays at song time**, not as a warped clip: exact whatever the tempo map, and
   the engine needs no warp for it. Flatten warps the clip to the tempo map.
4. **Device param tables were frozen by contracts-3**, not by the device nodes, so the mock,
   presets and layouts could be written in parallel. Nodes append params; they never
   renumber.
5. **The codec carries v0.2 track fields as JSON** to keep the binary layout stable while
   nodes refine their desc types; a node that needs speed moves its field into the binary
   layout (bumping the version).
6. **Library write access is new** (`Library::write_file` & co.) and presets/index share
   one writable user root.

### 12.13 Importing audio from the user's computer (`file-import`, priority 1)
- UI: an "Import audio…" command (toolbar/menu, ⌘I, following the owner's patterns), OS
  drag-drop onto the browser panel and onto arrangement lanes (a clip at the drop position
  on that track; below the tracks, a new track of the right kind). Several files at once;
  progress (`MediaEvent::{UploadProgress, ImportProgress}`), errors (unsupported formats:
  `Decode`), cancel (`CancelUpload`). One undo step per import-with-clip gesture (a `Batch`
  or one gesture id).
- Desktop: the Tauri file dialog and dropped-file paths → `Media::Import { source:
  MediaSource::Path { path } }`. This is the one explicit OS-file handoff: the UI passes the
  **path**, never the bytes, and never reads the file. The engine validates the path
  (absolute, regular readable file, audio extension) and reads it through
  `Library::read_external`; the media becomes an external reference in place once
  `media-references` lands (copied into the project before that).
- Web (local wasm controller) and remote: the bytes go through the frozen upload staging
  (`BeginUpload` → `UploadChunk`s → `Import { source: Upload }`), which `file-import`
  implements for the web OPFS store too; uploaded media is always copied into the project.
  `Path` replies `Unsupported` there.
- Collab: imported media replicates through the existing media push; an external-path
  media pushes its bytes (read with `read_external`) and peers store it at `MediaRef::file`.

### 12.14 Plugin sidechain (`plugin-sidechain`, priority 2)
Plugins with an aux/sidechain input bus take the device's sidechain source (`Device::
sidechain`, `Device::SetSidechain`) exactly like built-ins: the engine already provides the
latency-aligned sidechain buffers (base-24, §11.10) through `Node::process_sidechain`.
- Discovery: `PluginDescriptor::sidechain_inputs` (scan, catalog; `#[serde(default)]` = 0) and
  the instance's `DeviceDescriptor::sidechain_inputs` (authoritative, from the bus layout at
  instantiation; the UI shows the sidechain selector when > 0). Only the first aux input bus
  is used; mono aux buses get the left channel, wider ones the first two.
- Hosts pass the buffers as their second input bus: CLAP (clack) the second input audio port
  (`CLAP_PORT_IS_MAIN` unset), VST3 the first `kAux` input bus (activated with
  `activateBus` when a source is set, silent when not), AU input bus 1 (render callback
  supplying the sidechain). Without a source the aux bus gets silence.
- Sandboxed plugins: the shared-memory block gains the aux input channels (`sidechain_inputs ×
  max_block` floats after the main inputs) and the shm `VERSION` is bumped by one; the helper
  forwards them to the plugin the same way. Serialized with `sample-accurate-automation`
  (which also touches the shm event layout): whichever lands second rebases on the other and
  bumps the version again.
- Offline renders (export, freeze, bounce) route sidechains like live playback.

### 12.15 Graphical EQ (`graphical-eq`, priority 2)
- Widget `EqCurve { bands: [EqBandBinding { on?, kind?, shapes, freq, gain?, q? }], crossovers,
  spectrum: None | Post | PrePost }` (§12.4.2 catalog). It draws the combined magnitude
  response of the bands (the sum of each enabled band's dB) on a log-frequency (20 Hz–20 kHz) /
  dB grid, with one handle per band: drag = freq (x) + gain (y; x only without `gain`), wheel or
  Alt-drag = Q, double-click = toggle `on`, context menu = `kind` (labels of the kind param,
  `shapes[i]` per value). `crossovers` are vertical handles (drag = frequency). Each drag is
  one gesture (one undo step). Canvas colours come from tokens (`readToken`).
- Response math is shared: `ether_protocol::eq_response::{EqShape, svf_coefs, svf_magnitude,
  magnitude_db}`, exactly the EQ's TPT SVF (`y = m0·x + m1·band + m2·low`, magnitude at
  `s = j·tan(π·f/fs)/g`; `*24` shapes squared). The UI mirror
  (`ui/src/features/devices/layout/eq/eqResponse.ts`) must match
  `crates/ether-protocol/tests/fixtures/eq_response_vectors.json` within 1e-6 dB (regenerate
  with `UPDATE_EQ_VECTORS=1 cargo test -p ether-protocol --test eq_response`). The widget uses
  the engine sample rate when known (`EngineStatus`), else 48 kHz.
- The EQ ships the layout (`eq::layout`: the curve over 8 bands with `PrePost` spectrum, then
  the band controls). It publishes `AnalysisKind::SpectrumPre` (input) and `Spectrum` (output)
  frames while watched (≤ 30 Hz, RT-safe: accumulate in `process`, copy in `analysis`); they
  arrive as `AnalysisData::Spectrum { stage: Pre | Post }`.
- `EqCurve` is for the EQ only in v0.2: the auto filter and the multiband compressor use
  standard controls (`FilterCurve`, `Crossover`, knobs). Other filters may adopt it later with
  an explicit param mapping (a BCR on this section).

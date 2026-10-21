# Contracts: review guide

The contracts every crate and the UI build against: the protocol, the model, the engine
traits and the rules around them, with the decisions behind them. Protocol and model types
are stable: change them on purpose, in a pull request that says so (wire compatibility,
`.ether` versions, the generated TS types).

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
  `offset` (the content position at the clip start) may be **negative**: content
  positions before 0 are empty, so a negative offset is a leading gap of `-offset` beats
  (no notes; silence before the audio's first sample; never reads before sample 0) and the
  content starts at clip-relative `-offset`. Fades still apply at the clip edges. Repitch
  transposition scales around `max(offset, 0)`. Loop bounds stay `>= 0`; a looped clip
  with a negative offset plays the gap, then its content up to the loop end, then the loop.
  The UI produces negative offsets only on unlooped clips (dragging the left edge past the
  content start); on looped clips it wraps the offset into the loop region instead. Older
  documents are unchanged (their offsets are all `>= 0`); `ClipCommand::SetBounds` and
  `ClipChange::Offset` accept any finite offset.
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
  - **Field-level device params (additive).** A device touched
    only by `DeviceChange::Param` ops in a transaction yields one
    `DeviceParam { id, param, value }` per param instead of an `Upsert` (receiver:
    `devices[id].params[param] = value`, `null` = remove the entry, unknown device =
    ignore). A knob drag on a 500-param plugin patches ~100 B instead of the whole device.
  - **No plugin state on the wire.** Patches never carry `PluginInstance.state`: the
    controller sends it as `null` (`strip_plugin_states`). The UI never reads it; it is
    authoritative only in saved files and `Project::Get`. Mirrors must not treat a patched
    `null` as "fresh instance".
  - **Compatibility.** Embedded UIs (desktop, web/wasm, the mock) ship with their engine
    and always get `DeviceParam`. A remote client announces it with
    `ClientHello.features: ["device_param_patch"]` (§11.5); while a client without it is
    connected, the controller sends whole-device upserts instead (`PatchCompat`,
    `upsert_device_params`). Collab peers exchange ops, not patches, so mixed versions are
    unaffected.
- **`.ether` file.** Format: `{ format: "ethereal-project", format_version, app_version,
  project }`, with `CURRENT_VERSION = 1` (`ether_model::file`). **Format 1 is the
  public-release baseline**: documents of pre-release builds (`"version": 1..=5`, no
  `format_version`) are refused (`FileError::PreRelease`), not migrated. Loading parses to
  `serde_json::Value`, runs the `up` migrations to `CURRENT_VERSION`, deserializes, then
  calls `validate()`; a newer document is refused (`TooNew`: "made with a newer version of
  Ethereal", `ErrorCode::Unsupported`). Each `Migration` has `up` (N → N+1) and `down`
  (N+1 → N) on the untyped document; `down` is tested and available through
  `file::convert` (writing for an older reader) but no app path uses it yet. Every schema
  change an older reader cannot round-trip bumps the version, **additive ones included**
  (older readers ignore unknown fields and would drop them on re-save). Frozen fixtures per
  version (`ether-model/tests/fixtures/v<N>_*.ether`) must load forever; those at the
  current version must re-save byte for byte (`tests/format_fixtures.rs`). One code path
  for desktop, web, bundles, versions, collab snapshots and static shares. `MediaRef.file`
  is always project-relative (`media/...`). (Pre-release history: v1 → v2 removed the
  Session view, v2 → v5 added the roadmap v2, v0.2 and v0.3 tables; those migrations are
  gone.)

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
  - `Save { autosave: true }` is the UI's autosave: only plugins that reported a state
    change are re-read and the file is written in the background (a failure comes back as
    an error notification and the project turns dirty again). Plain `Save` (Ctrl+S) reads
    every plugin's live state and is on disk when it replies. A plugin's `StateDirty` is
    not a document edit (no `DirtyChanged`): it is saved by the next save, or by the
    controller on its own at most every 30 s; quitting and switching projects save it.
  - `OpenSafe { id }` opens a project in safe mode (every plugin device is a
    bypassed placeholder: no instance, state and params untouched, nothing written);
    `LoadPlugins` leaves it (instantiates them). `Plugin::Reload` loads one placeholder.
  - `Event::Project` carries `ListChanged`, `Saved`, `DirtyChanged` or
    `SafeMode { active, devices }`.
  - **Launch: no host opens a project.** The controller starts with none,
    `connect()` resolves with `null`, and the UI shows the project screen. The UI setting
    "Reopen last project on launch" (default off) reopens the last one only when the
    previous session closed cleanly (`Version::SessionStatus`, §13.11).
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
    - An audio clip's own **gain** and **transposition** envelopes
      (`AutomationTarget::ClipGain` / `ClipTranspose`, clip-owned lanes of audio clips
      only) override nothing: they modulate the clip's value (dB added to its gain,
      semitones added to its transpose, total clamped to ±48) wherever the clip plays.
      They are carried by the clip's content desc, not by a track target.
  - **While stopped.** Lanes and clip envelopes both apply at the stopped position (one
    evaluation), with the same precedence. An envelope only applies when the position
    is inside its clip.
  - **Timing.** Mixer targets (volume, pan, send level) take the value at the sub-block
    start, smoothed over 10 ms. Device params get `Param` events every 64 samples while
    playing.
- **Continuous controls.** Volume, pan, send level and device params are undoable
  document edits and are also pushed to the engine param queue immediately, with no graph
  republish. The UI sends them with a `gesture` and closes the gesture with
  `Edit::EndGesture`; during a drag it sends at most one `Device::SetParam` per
  (device, param) per animation frame.
  - **Plugin-GUI edits.** The controller drains plugin
    notifications once per tick and coalesces `ParamEdited` per (device, param), last value
    wins, without reordering them across gesture begin/end. Edits inside a plugin gesture
    merge into that gesture's undo step ("set cutoff"); gesture-less edits of a device are
    one transaction per tick, and gesture-less edits less than 250 ms apart share one
    "plugin edit" undo step (so a plugin preset switch is one step, not one per param).
- **`Edit::Batch { label, commands }`** applies several commands as one undo step. For
  example, create a MIDI track and insert a synth.
- **Undo labels** (history panel, "Undo …"): short, lowercase, verb first, acronyms
  uppercase, counts when several ("move 3 clips", "set volume", "add reverb", "set
  cutoff"). The controller labels every command (`doc/labels.rs`, one exhaustive match);
  a `Batch` keeps its caller's `label`, which clients write in the same style. Agent tool
  edits are `AI: <label>` ("AI: add notes").
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
  `channels`, `wake_requested`. `ProcessStatus::Silent` means the outputs are silent and
  stay so until input or events arrive: the engine then skips the node (silent output)
  while its input is digital silence, no event reaches it, its analysis isn't watched and
  `wake_requested` is false (`Engine::set_silence_skipping`). Nodes that
  make sound or events out of nothing (LFO-driven sources, arpeggiators, dither) never
  return it.
  `ProcessContext` carries:
  - `frames` and `sample_rate`
  - `transport: &TransportInfo` (position, bpm, beats per sample, tempo increment per
    sample, time signature, bar start and 0-based bar number as the bar/beat display
    counts it, loop)
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
  (`ether-core`).
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
  semaphores, +1 device period of latency reported for PDC, crash → faulted → bypass)
  implement the same traits. Sandbox timing: the native audio
  callback publishes the device period and one wait deadline per device buffer
  (`ether_core::period`, half a period from the callback's start); every sandboxed node on
  a device thread waits for its helper until that shared deadline at most. The sandbox
  delay starts at `max_block_size` and drops to the device period on the first device-thread
  call (rises again if a longer call comes); offline renders keep `max_block_size`. The
  helper's audio thread runs with FTZ/DAZ and real-time priority (best effort). **IPC naming rule:** every global OS object is named with
  `ipc_name(instance, pid, purpose)`.
- **Plugin formats.** `PluginFormat { Clap, Vst3, Au, Vst2 }` (serde tags
  `"Clap"`/`"Vst3"`/`"Au"`/`"Vst2"`, stable, additive; CLI names `clap`/`vst3`/`au`/`vst2`).
  `Vst2` is a gap in the open-source build: the tag stays for `.ether` compatibility, but the
  host (`ether-vst2`) is in the private repository, so this build does not register it. Each
  hosted format implements
  `ether_plugin_host::PluginFormatHost` (`scan` in the scanner process only, `instantiate`
  on the plugin main thread) in its own crate (`ether-clap`, `ether-vst3`, `ether-au`); the
  scanner and sandbox helper (`--format`) dispatch through a `Formats` registry. **Id
  convention** (`PluginInstance.plugin_id` = `PluginDescriptor.id`): CLAP = reverse-DNS
  plugin id; VST3 = class id as 32 uppercase hex in canonical `FUID::toString` order (same
  on every OS); AU = `type:subtype:manufacturer` four-char codes (`aufx:dely:appl`, non-
  printable bytes as `\xHH`); VST2 = `AEffect::uniqueID` as 8 uppercase hex digits
  (`'EtG2'` → `45744732`; shell sub-plugins: their own uniqueID, path = the shell library). The document stores no plugin location: hosts resolve
  `(format, plugin_id)` through their scanned catalog (`PluginDescriptor.path` = bundle
  path; for AU the id). `DeviceSpec::Plugin.format` and `ScanRequest.format` are optional
  (omitted = CLAP / inferred from the path). Details: `docs/PLUGIN-FORMATS.md`.
- **`Stretcher`** (`ether-stretch`): `configure`, `reset`, latencies,
  `set_transpose_semitones`, `seek`,
  `process(input, in_frames, output, out_frames)`, plus a `StretcherFactory`. The
  Signalsmith implementation is behind the native-only `signalsmith` feature; its
  pure-Rust port (`RustStretcher`, feature `rust-stretch`, same algorithm and output) is
  what the web Worklet uses (`ether-wasm` `warp-complex`, `RustPreset::Web`, voices capped
  by `EngineHandle::set_stretch_voice_limit`).

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

**Host-handled commands.** `Command::Engine(*)` (audio device list/config/status) and `Command::Plugin(Rescan | List | OpenEditor | CloseEditor | SetEditorKeyCapture | *Folder* | ParamTexts)` are intercepted by the **native host** on the controller thread before `Controller::handle`, which replies itself (same ordering: one reply per message). The controller replies `Unsupported` if it ever receives them (web host). `EngineBridge::poll_plugins` (drained from the controller tick) and `EngineBridge::plugin_state` (read for every plugin device before serializing) are defaulted, so non-plugin hosts ignore them. So is `EngineBridge::plugin_param_values` (current plain values of a plugin's params, read after (re)instantiating from a state blob to mirror them into `Device.params`). So is `EngineBridge::node_latency(key)` (native: `EngineHandle::node_latency`; web: `None`), which the controller tick compares with the latencies of the last publish to republish PDC, at most every 50 ms, when a built-in's latency changes.

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
    undo/redo and gestures, plus a fake playhead and meters. UI work builds real features
    against it. It also simulates the engine-side project store (three demo projects,
    dirty flag) and a fake sample library. There are no file pickers and no browser file
    I/O.
  - `tauri/` and `wasm/` are stubs for the desktop and web hosts.
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

- `[workspace.dependencies]` pre-declares everything anticipated: cpal, clack (git-pinned),
  symphonia, rubato, midir, rtrb, triple_buffer, basedrop, assert_no_alloc, signalsmith,
  shared_memory, raw_sync, interprocess, wasm-bindgen, js-sys, web-sys, tauri + plugins,
  proptest, insta, and others. Crates opt in from their own `Cargo.toml`. `ui/package.json`
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
- **Cargo features** (ARCHITECTURE.md "Cargo features"): the default
  build of ether-controller, ether-native, ether-wasm, ether-server, ether-mcp and
  ether-desktop is the dev core. `full` (or `--all-features`) is what releases and gates
  build. A disabled feature never changes the protocol (types, `gen-types` output) and
  takes an existing "not available" path: `Project::Import` replies `Unsupported`
  (`import`); a plugin format or sandboxed instance fails `PluginError::Unsupported`
  (`plugins-*`, `sandbox`); the native stream hooks keep their `Unsupported` defaults and
  `stream_capabilities` reports no native sender; collab connections and
  share signaling close at once (`fatal`, "not available in this build") (`net`);
  Complex warp plays Repitch (`warp-complex`); the desktop agent bridge does not start
  (`mcp`). New gated code follows the same rule. Its tests declare `required-features`.

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

## 11. Roadmap v2 contracts

The contracts of the 12 roadmap v2 features. Everything is additive: v0.1 behaviour is
unchanged where a feature is not implemented (its commands reply `Unsupported`, new devices
are pass-through placeholders, new graph fields are empty or neutral). The MockTransport
simulates all of it.

Conventions kept: one protocol domain file per feature (`Command::{Export, Tempo, Marker,
MidiMap, Groove, DrumRack, Slice, Collab}`), document edits are ops on normalized entity
tables with field-level `Update`s, client-chosen ids, patches before replies.

### 11.1 Export
- `Export::Render { job, request }` replies `ExportStarted`; progress, done, failed and
  cancelled arrive as `Event::Export`. One job at a time. `ExportRequest { range: Loop |
  Project | Custom, format { Wav | Flac, Int16 | Int24 | Float32 (not FLAC), sample_rate },
  mode: Mix | Stems { tracks } | MixAndStems { tracks }, normalize, tail_seconds, name }`.
- Rendering uses `ether_core::offline::OfflineRenderer` (a private engine on the controller
  thread) with fresh nodes, never the live ones, stepped a bounded unit of work at a time
  from the controller tick. The metronome and count-in are never rendered
  (`OfflineRenderer::publish` forces them off), looping and inputs are off, and the graph
  latency is dropped at the start. The result equals a live render of the same project
  sample-exactly when both engines use the same block size; automation and tempo ramps are
  evaluated per block, so other block sizes differ slightly.
- Stems (one render shared with the mix when the track's stem is exactly its captured
  post-fader output, else one pass per track; same result, see `export` module docs for
  which): that track's post-fader output (chain, fader, pan)
  goes straight into master, and its sends feed the returns (return processing included).
  Every other source is silent. The master chain is excluded (master fader and pan are
  kept). Solo is ignored, and a muted track is unmuted for its own stem. A child of a group
  bypasses the group's processing in its own stem, while a group's stem includes its
  children through the group. A return's stem is everything sent to it. Selecting a track
  together with its group, or with a return it sends to, puts that audio in both files.
  In a pass of its own, sidechain sources on other tracks are silent (in a shared render a
  sidechained device is keyed as in the mix). Stems therefore sum to the mix only with
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
`ListBuiltin` order and `BuiltinDevice::new(ty)` builds defaults. Param lists belong to each
device (append-only ids). `DeviceDescriptor::sidechain_inputs` (0 = none) is new on
every descriptor.

### 11.3 Tempo map and metronome
- `TempoCommand`: tempo-point and time-signature CRUD (the points at beat 0 can't be
  removed or moved), `SetMetronomeSettings { volume, accent, sound }` (partial). Undoable.
- Time signatures sit **anywhere** on the timeline (Ableton-style). Rules: time
  finite and `>= 0`, no two changes at the same position, numerator `1..=99`, denominator
  `1|2|4|8|16|32`; no bar-line rule. Bars restart at every change: a change inside a bar
  ends that bar early (a **partial bar**, still counted as one bar) and bar 1 of the new
  signature starts at the change (4/4 with 7/8 at beat 2.5: bar 1 = beats 0..2.5, bar 2 =
  2.5..6, bar 3 from 6). Maps whose changes all sit on bar lines (every older project)
  number exactly as before. Single source of the math: `ether_model::TempoMap::bar_beat`,
  mirrored by `ether_core::tempo::TempoMapRt::signature_at` (metronome downbeats, plugin
  bar start) and the UI's `timeline/tempoMap.ts` (ruler, grid lines, snapping, bar-based
  commands), kept in lockstep by `crates/ether-model/tests/tempo_vectors.json`. Editors
  place/move a change on the view's grid, at least a beat of the signature in effect
  (finer if the grid is finer); Alt disables snapping.
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
- **Crossfade gestures** (`crossfades`). `ClipCommand::SetCrossfade { first, second,
  at, length, curve: Option<FadeCurve> }` sets the crossfade of `first` into `second` to
  `length` beats centred on `at`, with the same extension rules as `Crossfade` (each clip
  extends into the other as far as its source allows; the other side makes up the rest).
  `length = 0` removes it: the clips are trimmed to meet at `at` (clamped to their overlap)
  and `first.fade_out` / `second.fade_in` reset to 0. `curve: None` keeps the clips' curves.
  The UI's fade-handle drags send it with the boundary captured at the gesture start, so a
  drag never drifts the centre; all of a gesture's commands are one undo step. Mirrored by
  the MockTransport (`transport/mock/roadmap/clipEditing.ts`). Rendering needs nothing new:
  the engine sums the overlapping clips with their fades (`ClipEnvelope`), so freeze,
  bounce, consolidate and export include crossfades. `Linear` keeps identical material
  (a split take) at constant level; `EqualPower` keeps uncorrelated material at constant
  power (and sums identical material to +3 dB at the midpoint).
- **Import** (`ether_import::model::Track::crossfades`, additive): formats that keep
  crossfaded clips touching (Live) list `Crossfade { first, second, length }`; the mapper
  turns each into the overlap model above (media permitting; else plain fades, reported).
- **Reverse.** The clip behaves as if its media were reversed; offset, loop and warp markers
  are expressed on that reversed timeline. `SetReversed` (when it changes the flag) mirrors
  them, and the clip's envelopes, in the same transaction (`ether_model::reverse`): a
  clip playing media frames `a ..= b` then plays `b ..= a`, the same material backwards;
  fades stay at the clip edges; reversing twice restores the clip. Exports to formats
  without reverse (DAWproject, Live) write that material forwards (reported); FL Studio
  keeps the reversed sample.
- `Marker { id, position, name, color }` + `MarkerCommand`; jump with `Transport::Locate`.

### 11.5 Remote engine (WebSocket)
`ether_protocol::remote`: the first text frame is `ClientHello { protocol_version, token,
client, features? }` (`features`: additive capability names, e.g.
`FEATURE_DEVICE_PARAM_PATCH` = `"device_param_patch"`, §2 Patches; missing = none), answered by `ServerHello::Welcome { server: ServerInfo, session }` or
`Rejected { reason, message }` (then close 4001 auth / 4002 version / 4003 busy; an idle client is later closed with 4004). After that, text frames are
`ClientMessage`/`ServerMessage` JSON, and binary frames
`[kind u8][header_len u32 LE][header JSON][payload]` carry bulk bytes (`Bytes`: the
message's base64 `data` field travels raw; `Peaks`: f32 min/max arrays). The JSON forms stay
valid on every transport. Token auth (no token only on loopback). Uploads: `BeginUpload` →
ordered `UploadChunk`s → `Import { source: Upload }`, plus `CancelUpload` and
`MediaEvent::UploadProgress`, staged through the defaulted
`ProjectStore::{begin, append, read, discard}_upload`. Upload ids are per client: the
server namespaces them, so a client can't touch another's upload, and `UploadProgress` goes
only to the uploader. On SIGTERM/SIGINT the server closes clients with 1001 and saves a
dirty project before exiting. Dev port offset `remote: 4`
(`scripts/dev-env.mjs`). `ether-server` (headless native host) and
`ui/src/transport/ws/WsTransport` are stubs; the frame codec is implemented on both sides.

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
time_ms }` through `EngineBridge::poll_midi_input`, fed natively from the recording
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
- PDC (implemented in `ether-core/src/sidechain.rs`, tested in
  `ether-core/tests/base24_hooks.rs`): the sidechain must reach device *k* of track T
  aligned with T's main signal there. Walk T's chain with `L = in_lat(T)`; at a sidechained
  entry with `L_sc = out_lat(source)`: if `L_sc > L`, the **main signal** is delayed by
  `L_sc − L` just before entry *k* (a delay line counted into T's chain latency, applied
  whether T plays clips, receives buses or both, and whether the entry is bypassed or not);
  otherwise the **sidechain** is delayed by `L − L_sc`. Then `L += latency(entry k)`. T's
  output latency includes the main delays and downstream PDC absorbs it.
- The tap is post-fader (volume, pan, VCA gain) but **pre** mute/solo gate: mute and solo
  only silence what a track contributes to the mix (its output, sends and post-fader
  input taps), never its sidechain feed. Soloing the consumer keeps it ducking under a
  soloed-out source, and a muted "ghost" source (e.g. a muted kick used only as a key)
  keeps keying, like Live, whose Track Activator sits after its sidechain taps. Envelope
  followers keyed from a track read the same tap.
- Devices on drum pads cannot have a sidechain (the model rejects `sidechain` on a device
  with `pad`); pad chains never carry one.
- Chains of one rack (v0.4, `ether_model::SidechainTap`): a device inside a rack chain may
  key another chain of a rack it is in (any depth). That chain runs first: a rack's chains
  run in `Project::chain_order` (keyed chains before the chains keying them, else their own
  order; the mix is a sum), the model rejects loops, and a chain of the rack holding the
  device is tapped before the rack's alignment (`ScPoint::ChainRaw` for post-FX; pre-FX
  is its input; no post-fader: the aligned and mixed outputs come after every chain).

### 11.11 Groove
`NoteCommand::Quantize::swing` (0..=1, destructive: odd grid positions are delayed by
`swing·grid/3`). `GrooveCommand::Humanize { clip, notes?, timing, velocity, seed }` is
deterministic from `seed`. `GrooveCommand::SetSwing` sets `ProjectSettings::{swing,
swing_grid}`, a non-destructive **playback** groove the controller applies when compiling
MIDI clips (`groove::swing_notes`).

### 11.12 Drum rack and slicing
- `BuiltinDevice::DrumRack` sits on a track chain or, as the chain's instrument, on an
  **instrument rack's chain** (notes reaching that chain play its pads; its output goes
  through the chain's following devices and mix; it is listed in `TrackDesc::racks` too and
  its pads run at its rack-chain entry, its longest pad chain counting in that chain's
  latency). `DrumPad { id, rack, note (unique per
  rack), name, color, choke_group (1..=16), volume, pan, mute }`. Pad chains are devices
  with `Device::pad = Some(pad)` on the rack's track (no drum rack under a drum pad at any
  depth; instrument / audio effect / MIDI effect racks may sit on a pad, their chains
  compiled into `TrackDesc::chain_racks` and run at the pad entry).
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
  pads with client-chosen ids (`SlicePadIds`), so concurrent sites can't mint different ids;
  `SampleToPads` slices a media straight onto an existing rack's pads (same ids rule).
- Pad-chain devices are first-class engine nodes: `SnapshotRt::pad_index` routes
  live params and automation to them, their latency is refreshed like chain nodes, and the
  rack entry's latency includes the longest pad chain (all pads aligned to it; the chain
  audio entering the rack is delayed the same). A pad is delayed by
  `longest − Σ latency of its enabled entries` (bypassed pad devices are skipped when
  processing but still count in the rack's latency, like bypassed track devices). Basic
  `run_pads` is implemented: notes route by key (the pad's key becomes `PAD_PLAY_NOTE`,
  other keys are dropped); raw MIDI note-on/off and poly aftertouch route the same way,
  channel-wide raw messages (CC, pitch bend, channel pressure) reach every pad; a
  note-off or choke goes to the pad its note-on went to even if the pad's note changed
  since; `AllNotesOff` reaches each pad chain; pad chains, alignment and pad gain. Choke
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
   so each feature doesn't have to.
5. **Project swing is a playback groove** compiled by the controller; `Quantize::swing` is
   the destructive variant.
6. **MIDI mappings are document entities** (saved and undoable, like Ableton); learn mode is
   runtime state.
7. `PatchChange` allows `clippy::large_enum_variant`: entities got larger, and boxing them
   would only add allocations.

### 11.15 Media preview (`media-preview`)
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

## 12. v0.2 contracts

The contracts of the v0.2 features. Everything is additive and append-only: v0.1
behaviour is unchanged where a feature is not implemented (its commands reply
`Unsupported`, pinned by `ether-controller/tests/roadmap_v3.rs`; new devices are
placeholders; new desc fields are empty). The MockTransport routes every new command to
one file per feature (`ui/src/transport/mock/roadmap/`). The repo owner decided four UX
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
- `Audition { track, lane }` is runtime and site-local (like `DrumRack::SetPadSolo`: no ops):
  the track plays that lane's clips instead of its comp until `lane: None`.
- Recording: each loop/punch pass becomes a lane with its clip and a region spanning the pass
  that selects the newest lane. A pass over existing main-lane clips first moves their parts
  inside the recorded range onto a new lane, the **first take** (edge-crossing clips are split),
  so only the comp sounds there; one undo step with the recording.

### 12.3 Freeze, flatten, bounce, consolidate, time edits (`time-edits`)
- `Track::freeze: Some(TrackFreeze { media, start })`: the track plays its render (post-chain,
  pre-fader, latency removed) at **song time** (frame `f` at song second `start + f/rate`,
  whatever the tempo map). Clips and devices stay in the document but are neither compiled
  nor instantiated (CPU saved). Fader, pan, mute/solo, sends and routing stay live. Only
  audio/MIDI tracks; the media is project media. The controller rejects edits to a frozen
  track's clips, devices and device automation (`freeze::check_editable`, `InvalidState`).
- **Freeze Group**: a group track can be frozen too (`Freeze { track: group }`, same
  `Track::freeze`). The render is the group's post-chain, pre-fader signal: its descendants
  (nested groups included) through their faders, then the group's devices, with automation
  (solo ignored, like track freeze). While frozen the descendants are *inert*
  (`Project::frozen_ancestor`; `freeze::apply_inert` in both compilers): no clips, devices
  (no nodes), sends, solo/mute, input, taps or output, and the controller rejects content
  edits, deletes, moves, duplicates and creating or moving tracks into the group
  (`InvalidState`). Their sends to returns stop, and sidechains they feed elsewhere hear
  silence (a warning lists both when the freeze starts); taps of non-frozen tracks keep the
  "unaffected by mute/solo" rule. Rejected when a descendant routes outside the group.
  Unfreezing a nested frozen group inside a frozen group is rejected. `Flatten` of a group
  replaces the group and all its descendants by one audio track `new_track` (name, color,
  parent, mixer, output, VCA, sends of the group) holding one clip of the render; the
  group's own fader/pan automation lanes are dropped. One undo step.
- **Freeze up to a device** (#25, additive): `Freeze { up_to: Some(device) }` renders the
  source and the chain up to and including that track-chain device into the media, stored as
  `TrackFreeze::up_to` (omitted = the whole chain, so older files load unchanged; the last
  device of the chain is stored as `None`, a plain freeze). Not on MIDI effects nor before a
  MIDI track's instrument (`InvalidArgument`). Only the frozen devices
  (`Project::device_frozen`: up to the freeze point, nested ones with their rack) lose their
  nodes and are locked; the devices after it (the live tail) compile into the chain after
  the render and stay editable (inserting or moving devices into the tail too). Clips stay
  locked; the track counts as frozen everywhere else. `Flatten` keeps the live tail (on the
  audio track, or moved to the new audio track).
- Engine: `TrackDesc::frozen: Option<FrozenDesc>` (+ empty clips/chain, or the live tail in
  `chain`), rendered by `ether_core::freeze::render_frozen` where clips render (pre-wired).
- Render jobs (`Freeze`, `Bounce`, audio `Consolidate`) reply `RenderStarted { job }` and run
  on an `OfflineRenderer` from the controller tick (like export; one job at a time,
  independent of export), then `FreezeEvent::{Progress, Done, Failed, Cancelled}`. The
  document changes once, at `Done` (media insert + edit, one undo step). Renders always land
  in the project's `media/`.
- `Flatten` makes a freeze permanent (audio track: clips and devices replaced by one clip of
  the render; MIDI track: replaced by a new audio track at its position, sends copied with
  derived ids). The clip is warped to the tempo map so it plays exactly the render.
- `Bounce { track, start, end, include_chain, target }`: `NewTrack` (a new audio track below,
  source muted) or `InPlace` (audio tracks, pre-chain only). The track's content is cut at
  `end` for the render; `NewTrack` with the chain keeps rendering past `end` until the
  output decays below -90 dBFS (release, reverb/delay tails; 0.5 s checks, at most 30 s,
  trailing silence trimmed), so its clip is the range plus the tail. `BounceRange { tracks,
  start, end, seed_tracks, seed_clips, seed_media }` (appended): bounce a time selection,
  each listed audio/MIDI track like `Bounce { include_chain: true, target: NewTrack }` (clips
  cut by the range rendered trimmed to it), as one job and one undo step; track `i` gets
  `derive_id(seed_*, i)` ids; tracks with no unmuted main-lane clip in the range are skipped
  (none: `InvalidArgument`). `Consolidate` joins clips per
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
`Randomizer`, `Pitch` (midi-fx; `Pitch` appended later, after `Randomizer`), `InstrumentRack`,
`AudioEffectRack`, `MidiEffectRack`. The poly synth's `Wavetable` oscillator type plays a table chosen by
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
The v0.1 `Compressor` appends `Detector` (`ParamId(6)`): a choice
`Peak | RMS | Expand`, default `Peak` (the original detector, bit-identical, so older
documents and renders don't change). RMS averages the louder channel's square with a 10 ms
one-pole window; Expand is a downward expander below the threshold (`ratio - 1` dB per dB
under it, at most 60 dB; attack = opening, release = closing). RT-safe, no allocation.
The effects pass appends params to the built-in effects (ids after the existing ones;
every default renders bit-identically to the device before it, `ether-devices/tests/
effects_defaults.rs`): Compressor `Knee` (7, dB, default 6), `Mix` (8, %), `Lookahead` (9, ms,
reported as `Node::latency`), `Auto Makeup` (10); Delay `Low Cut` / `High Cut` (6, 7, feedback
path, off at 20 Hz / 20 kHz), `Mod Rate` / `Mod Depth` (8, 9), `Freeze` (10); Reverb `Low Cut`
/ `High Cut` (6, 7, input), `Diffusion` (8, default 60 %), `Freeze` (9); later `Output` (10, wet trim, -36..+12 dB, default 0: level-matches imported Ableton reverbs); Utility `Channel` (6:
Stereo / Left / Right / Swap), `Width Mode` (7: Width / Mid/Side), `Mid/Side` (8, -100..100 %),
`Bass Mono` + `Bass Mono Freq` (9, 10), `DC Filter` (11), `Mute` (12); EQ `Channel` (41: Stereo
/ Left / Right / Mid / Side); Chorus `Sync` + `Sync Rate` (10, 11); Phaser and Flanger `LFO
Shape` (10); Arpeggiator `Distance` (9, semitones per repeat, default 12). Levels analysis
(`AnalysisKind::Levels`) for the window meters: Compressor and Limiter `[gain reduction dB
(<= 0), detector dBFS]`, Gate `[gain dB, detector dBFS]`, Utility `[left dBFS, right dBFS]`.

The instruments pass: `ParamUnit` appends `Cents` (hundredths of a semitone; the UI
shows "12 ct"). The `PolySynth` appends `Unison Voices` (`ParamId(61)`, 1..=16, step 1),
`Unison Detune` (62, 0..=100 cents of the outermost copies, power-2 knob), `Unison Blend`
(63, 0..=100 %, level of the side copies against the centre one(s); 100 % = all equal) and
`Drift` (64, 0..=100 %, slow independent pitch wander of each voice's oscillators, ±15
cents at 100 %), all defaulting to the sound before. The first unison controls (18 `Unison`
1..=8 and 19 `Detune` in percent) are renamed `Unison (legacy)` / `Detune (legacy)` and
**hidden** (`ParamInfo::hidden`): they keep working so documents and automation written
before render identically (the engine uses them while `Unison Voices` is 1), but panels,
the generic list and the AI tools only show the new ones. A param whose range or unit must
change follows this pattern (append the new one, hide the old) rather than editing the old
range, because automation, MIDI learn and modulation store normalized values.

The instruments pass (2): the v0.1 `Synth` appends `Fine` (9, cents ±100), `Glide`
(10, ms), `Velocity` (11, sensitivity %, default 100 = the level is the velocity, as
before), `Env Amount` (12, the amp envelope sweeps the cutoff ±6 octaves at ±100 %) and
`Key Tracking` (13, %). The `Sampler` appends `Decay` (8), `Sustain` (9), `Fine` (10,
cents), `Velocity` (11), `Reverse` (12), `Loop` (13: `Off | Forward | Ping-Pong`, pitched
mode outside slice mode only; loops through the release), `Loop Start` / `Loop End` (14/15,
% of the sample, clamped into the played range), `Crossfade` (16, % of the loop, forward
loops: blends the loop's tail into the material before the loop start), `Filter` (17:
`Off | Low-pass | High-pass | Band-pass`, 12 dB state-variable, per voice), `Cutoff` (18),
`Resonance` (19) and `Voices` (20, 1..=32, default 8 = the polyphony before). Every
default renders bit-identically to before (`ether-devices/tests/it/instruments_golden.rs`,
fingerprints recorded on `main`). The import model's `Sampler` gains `loop_range`,
`ping_pong`, `reverse`, `decay_ms`, `sustain`; Ableton's Simpler maps its sustain loop,
reverse and ADSR onto them.

Voice mode and global pitch on every native synth: the `Synth` appends `Mode` (14), the
`FmSynth` appends `Mode` (76), both `Poly | Mono | Legato` (default `Poly`, group `Voice`),
as the `PolySynth`'s existing `Mode` (22). `Mono`: one voice, every note retriggers the
envelopes and glides from the last note when `Glide` > 0. `Legato`: one voice; a note that
overlaps a held one takes the voice over without retriggering (and glides), a detached note
retriggers without gliding. Both keep a held-key stack (last-note priority: releasing the
newest key returns to the most recent one still held). The `PolySynth` appends a global
`Transpose` (65, -48..=48 semitones, step 1) and `Fine` (66, ±100 cents), group `Pitch`,
added to both oscillators' own tuning; the `Synth` (`Transpose` 1, ±24, and `Fine` 9) and
the `FmSynth` (53/54) already had theirs (the `Synth`'s ±24 range stays: widening it would
move stored normalized automation).

#### 12.4.2 Declarative layouts
`DeviceDescriptor::layout: Option<DeviceLayout>` (`ether_protocol::layout`, generated to
TS): sections (`id`, `title`, `span` 1..=4, `columns` 1..=8) of items (`widget`, `size`
Small/Medium/Large, `colspan`, `label`). Widget catalog (append-only: a new widget is a protocol change): param
widgets `Knob`, `Slider`, `Toggle`, `Choice`, `Number`; typed widgets `Envelope`,
`FilterCurve`, `TransferCurve`, `Oscillator`, `Lfo`, `StepEditor`, `XyPad`, `Crossover`;
data widgets `SampleWaveform`, `ZoneMap`, `Spectrum`, `Tuner`, `Meter`, `RackChains`,
`Macros`, and `EqCurve` (§12.15, the EQ only for now). Appended by `DynamicsCurve` (compressor / gate / limiter transfer curve with a live detector-level dot from `Levels`), `DelayTaps` (delay echoes), `ReverbDecay` (reverb decay envelope) and `VelocityCurve` (MIDI velocity mapping). Appended by `fm-synth`: `FmAlgorithm` (an FM synth's algorithm picker: one routing diagram per choice of its `algorithm` param, from `FmRouting` modulator / carrier bit masks). Appended by `fx-modulation`: `LfoCurve` (the Volume LFO's curve editor: the kind's `LfoCurve` on a grid of the cycle's sixteenths, edited with `Device::SetLfoCurve`, with a playhead; binds sync, sync rate, rate, phase and depth). **One shared renderer** (`ui/src/features/devices/layout/`, kit components and
tokens only) renders every built-in; devices without a layout (plugins, v0.1 devices until
they add one) get the generic layout grouped by `ParamInfo::group`. Devices write
specs and widget data only, never bespoke panels. `ModulatorDescriptor::layout` uses the same
catalog (the Steps editor is `StepEditor`). A shared validator
(`ether-devices/tests/it/layouts.rs`) checks every built-in and modulator layout: referenced
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
`Modulation` (`[param bits, base, effective]` triples, racks and modulation readback).

#### 12.4.4 MIDI effects (`midi-fx`)
Category `NoteEffect` (= MIDI effect). The engine already feeds clip notes to chain entry 0
and each entry's `out_events` to the next entry. Contract: a MIDI effect has
`channels() == (0, 0)` (audio untouched) and `audio_inputs == audio_outputs == 0`, consumes
`ctx.events` and writes the transformed note/MIDI stream to `ctx.out_events`, forwarding what
it doesn't transform and every `AllNotesOff`, never `Param` events; generated notes use ids
`0x8000_0000 | n`; it only delays (never earlier than its input) and reports no latency.
Stuck notes: an off device is MIDI thru, so switching any device on or off (switch or
On/Off automation; track chain, rack chain or pad) sends `AllNotesOff` to the next device,
and to the device itself when it comes back on; adding or removing a MIDI effect sends it to
every entry of that chain; an `AllNotesOff` reaching a rack entry reaches all its chains or
pads. A track that stops monitoring releases the live notes and sustain pedals it holds, and
Stop releases every node's notes even while stopped.
Chain rule: MIDI effects precede the first instrument (`midi_fx::check_chain_order` after
`Device::{Insert, Move}`, `InvalidArgument`). Scale Quantize receives the resolved
`MusicalScale` (track scale, else project) through `Node::set_data` on creation and on
scale changes. Plugins with MIDI output keep working as before (their events feed the next
device).

### 12.5 Presets (`presets` + each device)
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

### 12.6 Racks, macros and modulation (Bitwig-style)
- Racks (`ether_model::rack`): `RackChain { rack, order, name, color, volume, pan, mute, solo,
  keys, velocities, select }`; chain devices are ordinary devices with `Device::chain`
  (on the rack's track, exclusive with `pad`). Racks nest in chains up to `MAX_RACK_DEPTH`
  (every rack on the way down counts, drum racks included) and may sit on drum pads, never
  inside themselves. A drum rack may sit on an **instrument rack's** chain (not on audio /
  MIDI effect rack chains), never under a drum pad (directly or in a pad rack's chain).
  **Empty = nothing goes through:** a rack with no chains outputs silence (audio effect
  rack) / no notes (MIDI effect rack) / adds nothing (instrument rack); every chain muted
  is silence. A chain with no devices passes what reaches it (the rack's input, or its
  zone-filtered notes). A freshly inserted audio or MIDI effect rack (`Device::Insert`,
  `Rack::InsertDevice`, `DrumRack::InsertDevice`) starts with one empty chain "chain 1"
  (id `derive_id(rack, 0)`) in the same undo step; instrument racks start empty, and
  duplicates, presets and loaded documents keep exactly their chains. New built-in
  devices get the lowercase descriptor name on every insert path. Every rack's params: macros `0..8` (plain 0..1) and the chain
  selector (`8`, 0..=127). Engine: `TrackDesc::chain_racks` (`ether_core::rack_chains`),
  run at the rack entry before the rack node (pre-wired like drum-rack pads: params,
  automation and latency already reach chain nodes).
- Modulators live **inside any track-chain device** (not on drum-pad or rack-chain devices in
  v0.2, so every host is an entry the engine's `pre_node` hook sees; a rack's modulators reach
  its chain devices) (`Modulator { device, order, name, kind, params, sidechain }`,
  kinds `Lfo`, `Envelope`, `EnvelopeFollower`, `Steps`, `Random`, `Keytrack`, `Velocity` (appended in unipolar); param tables frozen in
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

### 12.7 Sample-accurate automation
The node parameter-event API is `EventKind::Param { param, value }` at a sample `offset`
(already delivered to every node, sorted). Built-ins apply them at their offset
(`split_at_events`); hosts forward offsets: CLAP `clap_event_param_value.header.time`, VST3
`IParamValueQueue::addPoint(sampleOffset)`, AU `AudioUnitScheduleParameters` with
`eventSampleTime`, VST2 (no timed params) splits the block at param events (sub-blocks of at
least 16 samples) and sends MIDI with `deltaFrames`. A node that ignores offsets applies changes at block start (the
backwards-compatible default). What changes (engine only, `ether_core::automation_rt`,
extracted verbatim from `engine.rs` by the v0.2 contracts):
- node params: events on an **absolute grid** (`sample_time` multiples of `PARAM_GRID` =
  32) plus one at every breakpoint, each evaluated at the exact beat of its sample;
- tempo ramps integrated exactly per sample for scheduling and automation (`Timing`);
- mixer targets (volume, pan, sends) ramped per sample to the value at each grid point;
- acceptance: an offline render of the same project is identical (±1e-6) with block sizes
  64 and 512, with automation and tempo ramps; with neither, bit-identical as today.
Modulation uses the same grid.

### 12.8 Browser v2
Engine-side index (controller `browser/`, persisted at `<user library>/.ethereal/
index.json`): `LibraryItem { id: "<root>/<path>", kind: Audio | Midi | Preset | Project,
name, root, path, source, preset, tags, favourite, meta { duration, rate, channels, bpm,
key, pack, modified, size } }`. `Browser::Query { text, kinds, tags, favourites_only, roots,
folder, device, sort: Name | Recent | Duration | Bpm | Relevance, offset, limit ≤ 200 }` →
`BrowserPage { items, total, offset }`. `ListRoots` (library, packs, user folders, factory
presets), `SetFavourite`, `SetTags`, `AddFolder { path }` (native only; the desktop shell
picks it), `RemoveFolder`, `Rescan`; folders from the UI machine (web, remote):
`ImportFolder { name }` (an engine-owned copy) + `ImportFile { root, path, upload }` per
uploaded audio file, then `Rescan`; removing it deletes the copy; `RenameFolder { root,
name }` (display only, any user folder); background indexing with `BrowserEvent::{IndexProgress,
IndexChanged}`. `Preview { item, sync }` uses the media-preview voice; `sync` repitches by
project bpm / item bpm and starts on the next beat while playing. Presets appear as items
(kind `Preset`).
File filters (additive): `FileFilter { whitelist, blacklist }`, regex rules
(Rust `regex`, case-insensitive, unanchored) on file names. A file shows when it matches a
whitelist rule (empty whitelist = every file) and no blacklist rule (blacklist wins); folders
are hidden by the blacklist only, hiding everything below; dotfiles never show. Applied
engine-side to `Media::ListDirectory` (every location), the index scan, and folder imports
(the UI asks `CheckFileFilter { paths }` before copying). `GetFileFilter` / `SetFileFilter
{ filter }` (≤ 64 rules per list, ≤ 256 bytes each, every rule must compile, else
`InvalidArgument` "whitelist rule N: …") / `ResetFileFilter` reply `BrowserFileFilter
{ filter, defaults }` and emit `BrowserEvent::FileFilterChanged { filter }` (+ `IndexChanged`;
the index drops what it now hides and rescans). `CheckFileFilter { filter: Option, paths }`
→ `BrowserFileFilterCheck { errors: [{ list, index, message }], visible: [bool], library:
Option<{ shown, total }> }` (counts over the file names the last scans saw). Stored at
`<user library>/Settings/library-filters.json` (session-only without a writable library).
Defaults: whitelist = decodable audio (`wav|wave|aif|aiff|aifc|flac|mp3|ogg|oga|m4a|aac`), MIDI
(`mid|midi`), `.etherpreset`/`.ethertemplate`; blacklist = `^\.`, `^Thumbs\.db$`,
`^desktop\.ini$`, `\.asd$`, `\.reapeaks$`. DAW projects (`.als`, `.flp`, `.rpp`,
`.dawproject`) are hidden (they open with "Import project…").

### 12.9 Media references
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
external-path media; media references doesn't touch it.
Migration: v0.1 media are `Project`; nothing moves.

### 12.10 Groups, buses, input taps, VCAs
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

### 12.11 Engine pre-wiring (hot files touched once)
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
4. **Device param tables were frozen by the v0.2 contracts**, not by each device, so the mock,
   presets and layouts could be written in parallel. Devices append params; they never
   renumber.
5. **The codec carries v0.2 track fields as JSON** to keep the binary layout stable while
   features refine their desc types; one that needs speed moves its field into the binary
   layout (bumping the version).
6. **Library write access is new** (`Library::write_file` & co.) and presets/index share
   one writable user root.

### 12.13 Importing audio from the user's computer
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
  media references lands (copied into the project before that).
- Web (local wasm controller) and remote: the bytes go through the frozen upload staging
  (`BeginUpload` → `UploadChunk`s → `Import { source: Upload }`), which `file-import`
  implements for the web OPFS store too; uploaded media is always copied into the project.
  `Path` replies `Unsupported` there.
- Collab: imported media replicates through the existing media push; an external-path
  media pushes its bytes (read with `read_external`) and peers store it at `MediaRef::file`.

### 12.14 Plugin sidechain
Plugins with an aux/sidechain input bus take the device's sidechain source (`Device::
sidechain`, `Device::SetSidechain`) exactly like built-ins: the engine already provides the
latency-aligned sidechain buffers (§11.10) through `Node::process_sidechain`.
- Discovery: `PluginDescriptor::sidechain_inputs` (scan, catalog; `#[serde(default)]` = 0) and
  the instance's `DeviceDescriptor::sidechain_inputs` (authoritative, from the bus layout at
  instantiation; the UI shows the sidechain selector when > 0). Only the first aux input bus
  is used; mono aux buses get the left channel, wider ones the first two.
- Hosts pass the buffers as their second input bus: CLAP (clack) the second input audio port
  (`CLAP_PORT_IS_MAIN` unset), VST3 the first `kAux` input bus (activated with
  `activateBus` when a source is set, silent when not), AU input bus 1 (render callback
  supplying the sidechain). Without a source the aux bus gets silence. VST2 has no buses:
  by the usual host convention an effect with exactly 4 inputs and 2 outputs takes inputs 3/4
  as a stereo sidechain (`sidechain_inputs = 2`); any other VST2 plugin has none (extra
  plugin inputs get silence).
- Sandboxed plugins: the shared-memory block gains the aux input channels (`sidechain_inputs ×
  max_block` floats after the main inputs) and the shm `VERSION` is bumped by one; the helper
  forwards them to the plugin the same way. Serialized with
  (which also touches the shm event layout): whichever lands second rebases on the other and
  bumps the version again.
- Offline renders (export, freeze, bounce) route sidechains like live playback.

### 12.15 Graphical EQ
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
  an explicit param mapping (a change to this section).

## 13. v0.3 contracts

The contracts of the v0.3 features. As in §12, everything is additive and append-only:
v0.2 behaviour is unchanged where a feature is not implemented. Its commands reply
`Unsupported` (pinned by `ether-controller/tests/roadmap_v4.rs`, one test per feature), new
devices are placeholders, new desc fields compile empty and the new engine hooks are stubs
(`ether-core/tests/roadmap_v4_hooks.rs`). The MockTransport routes every new command to one
file per feature (`ui/src/transport/mock/roadmap/`, each pinned by its own `*.test.ts`).

New domains (one protocol file each): `Command::{Expression, Capture, AudioToMidi,
External, History, Template, Version, Keymap}`; `Event::{Capture, AudioToMidi, External,
History, Template, Version, Keymap}`; replies `Captured`, `CaptureStatus`, `FactoryIrs`,
`HardwarePorts`, `History`, `Templates`, `Template`, `Versions`, `Version`, `VersionDiff`,
`Recoverable`, `Keymap`. New variants on existing domains: `Device::{SetIr,
ListFactoryIrs}`; new optional field `Preset::Load::seed` (§13.9).

Document commands (undoable, allowed in a `Batch`): every `Expression::*`,
`External::SetRouting`, `Template::Insert`, `Device::SetIr`. Commands that make one undoable
edit themselves but are not document commands: `Capture::Capture`, the end of an
`AudioToMidi` job, `External::MeasureLatency`'s result. Runtime only: everything else.

### 13.0 Ids, entities, `.ether` v5
- New ids: `ExpressionLaneId`, `NoteExpressionId`. New tables (`#[serde(default)]`):
  `expression_lanes`, `note_expressions` (`EntityKey`/`Entity`/`EntityUpdate` rows; ops
  `Op::{ExpressionLane, NoteExpression}` with `ExpressionLaneChange::Points` /
  `NoteExpressionChange::Points`). New optional field `Track::mpe` (omitted when `None`, TS
  optional; `TrackChange::Mpe`). New `BuiltinDevice`s, in `BuiltinDeviceType::ALL` order after
  `MidiEffectRack`: `ConvolutionReverb { ir: Option<IrSource> }`, `ExternalInstrument {
  routing }`, `ExternalAudioEffect { routing }` (`ExternalRouting`). `BuiltinDevice::media()`
  lists a `Media` IR. After them (`fm-synth`): `FmSynth`, the four-operator FM instrument
  (`ether_devices::fm_synth`, no kind data), then `Vibrato`, the pitch-wobble audio effect
  (`ether_devices::fx_modulation::vibrato`, no kind data; it reports its centre delay as
  latency), then `VolumeLfo { curve: LfoCurve }`, the drawn volume LFO (`fx-modulation`,
  `ether_model::lfo_curve`: 1..=64 `LfoPoint { x, y, curve: CurveShape }` sorted by `x`,
  one wrapping cycle; `curve` omitted in JSON = the default pump). `Device::SetLfoCurve
  { device, curve }` replaces it (sanitized; one `DeviceChange::Kind`, one undo step); live
  nodes take it through `EngineBridge::update_builtin` → `Node::set_data`. Its presets
  carry the curve in `Preset::kind`.
- `Project::entities()` order: … pinned notes, expression lanes, note expressions (each after
  its clip / note). Cascades are done: deleting a clip removes its lanes, deleting a note its
  expressions; `DocCtx::copy_clip` copies both.
- Validation (`ether-model/src/apply.rs`, tests `ether-model/tests/roadmap_v4.rs`): a lane's
  clip is a MIDI clip, one lane per `(clip, kind)`, `Cc` 0..=119; one note expression per
  `(note, kind)`; curves via `expression::check_points` (sorted, finite, in range, ≤
  `MAX_EXPRESSION_POINTS` = 16 384); `Track::mpe` on MIDI tracks only (`check_mpe`);
  external routings via `external::check_routing`; a `Media` IR exists.
- `.ether` v5: `V4ContractsV4Defaults` adds the two empty tables (idempotent, neutral). The
  bump makes a v0.2 app refuse v5 files (`TooNew`) instead of dropping expression and the v0.3
  devices on re-save.

### 13.1 Audio streaming
Media at least `STREAM_MIN_SECONDS` (30 s) long stream from disk on hosts that can
(`EngineBridge::stream_media(&StreamSource) -> Ok(true)`; the default `Ok(false)` keeps the
v0.2 whole-file decode). The controller still runs one decode pass for peaks and the content
hash (samples dropped), then registers the stream. The host's streaming `AudioSource` serves
`read` from a bounded lock-free chunk cache (`ether_media::stream::StreamCache`, chunks of
`CHUNK_FRAMES` frames at the engine rate) filled by a reader thread (native: one shared disk
thread, `ether-native/src/disk_stream`, `READ_AHEAD_CHUNKS` = 12; web: the engine Worker over
OPFS sync access handles, chunks shipped over the SAB ring, 8 ahead). The audio thread never
blocks, allocates or does I/O: a missing chunk is an underrun (silence, `read` returns
`false`, counted and reported). `AudioSource::prefetch_hint` and the controller's loop/locate
knowledge drive the read-ahead; warp/stretch read through the same source. Offline renders
read the file synchronously. Policy hook: `media_stream::should_stream` (placeholder: never).

### 13.2 MIDI expression
Model `ether_model::expression` (value table in its module docs): `ExpressionLane { clip,
kind: Cc { controller } | PitchBend | ChannelPressure, points }` (content-relative beats,
loops and moves with the clip); `NoteExpression { note, kind: Pitch | Pressure | Timbre,
points }` (beats from the note start; points past the note end are kept, not played). A
curve is **one value** (the unit of editing and of collab last-writer-wins). Values: CC and
pressure 0..=1, bend -1..=1, note pitch ±96 semitones. Curves interpolate with the point's
`CurveShape`, hold the first/last value outside their points.
- Commands: `Expression::{CreateLane, RemoveLane, SetPoints, ReplaceRange,
  SetNoteExpression, ClearNoteExpressions}` (and `SetTrackMpe`, §13.3), one undo step each.
- Engine (`ether_core::expression`): the controller compiles lanes and note expressions into
  `TrackDesc::expression` (`TrackExpressionDesc`, note indices into the clip's sorted note
  list; empty = v0.2). `ExpressionRt::render` (stub, pre-built; MIDI expression adds its
  call in the clip stage of `engine.rs`) emits lanes as raw MIDI (`EventKind::Midi`) and note
  expressions as `EventKind::NoteExpression { note_id, channel, key, expression, value }` for
  the sounding note, on the automation grid (`PARAM_GRID` + breakpoints), only on 7/14-bit
  changes, re-sent after a loop jump or locate; bend reset to centre when a bend lane's clip
  stops. RT: no allocation.
- Plugins: channel expression is plain MIDI; `Pressure` without MPE goes out as poly
  aftertouch; CLAP/VST3 note expressions and MPE MIDI are §13.3. The sandbox shm already
  encodes `NoteExpression`.
- Codec: `TrackDesc::{expression, hw_io}` travel in the v2 JSON blob (absent = default, no
  `VERSION` bump); a later change may move them to the binary layout (bump `VERSION`).

### 13.3 MPE (`mpe`)
`Track::mpe: Option<MpeSettings { zone: Lower | Upper, member_channels 1..=15,
note_pitch_range 1..=96 (default 48), master_pitch_range 0..=96 (default 2) }>`, set with
`Expression::SetTrackMpe` (MIDI tracks). With `Some`, the track's MIDI input is read as MPE
(each member channel's bend / channel pressure / CC 74 become the note's `Pitch` / `Pressure`
/ `Timbre` when recording, and live as `NoteExpression` when monitoring), and plugin output
uses, in order: CLAP note expressions (`TUNING`, `PRESSURE`, `BRIGHTNESS`), VST3 note
expression, else MPE MIDI (one member channel per note). The Poly Synth responds to all three.
CLAP note expressions are the reference model.

### 13.4 MIDI capture (`capture-midi`)
The controller keeps every incoming MIDI message (all ports, armed or not, playing or
stopped) in a bounded ring (`CAPTURE_MAX_SECONDS` = 600, `CAPTURE_MAX_EVENTS` = 65 536,
newest kept; runtime, site-local, never saved, cleared on project change), fed from
`midi_learn_tick` (`capture_input`). `Capture::Capture { track, clip, seed_notes,
adopt_tempo }` makes one undoable edit (clip, notes `derive_id(seed_notes, i)` in (start,
pitch) order, expression lanes once §13.2 lands): playing → notes keep their song positions;
stopped → the phrase goes at the playhead and, with `adopt_tempo`, the tempo (60..=180 bpm
best fit, ties prefer 120) and loop are set in the same step. Only messages that reach the
track's input filter count. Replies `Captured { clip, start, length, notes, bpm }`;
`InvalidState` when nothing was played. `Status` → `CaptureStatus`; `CaptureEvent::Changed`
on availability changes only.

### 13.5 Audio to MIDI (`audio-to-midi`)
`AudioToMidi::Start { job, clip, mode: Melody | Harmony | Drums, options, track, new_clip,
seed_notes, instrument }` replies `Unit`; the controller runs `ether_media::to_midi::Detector`
in bounded slices from its tick (`Progress` events), then applies one undo step: a MIDI track
`track` right below the source track, clip `new_clip` at the source clip's position and
length, notes `derive_id(seed_notes, i)` in (start, pitch) order, optionally a default
instrument with id `instrument` (`PolySynth`, or for drums a `DrumRack` holding a factory kit:
kick / snare / closed hat one-shot sampler pads on `kick_key` / `snare_key` / `hihat_key`, their
samples synthesized into the project's media, ids derived from `instrument`); then `Done`. Drum
keys ignore the clip's transpose.
`Failed` / `Cancel` → `Cancelled`; a job is cancelled when its clip or project goes away. One
job at a time (`InvalidState`). Detection is on the source; notes map through the clip's warp
and window. Options default to neutral values (GM drum keys 36/38/42).

### 13.6 Convolution reverb (`fx-space`)
`BuiltinDevice::ConvolutionReverb { ir: Option<IrSource> }`, `IrSource::Factory { id }`
(`ether_devices::fx_space::FACTORY_IRS`, ids append-only) or `Media { media }` (any project
audio media, referenced in place like samples: resolution, missing and relink work
unchanged). Param table frozen in `fx_space/mod.rs` (Mix, Pre-delay, Decay, Size, Low/High
Cut, Width, Gain, Reverse). `Device::SetIr { device, ir }` is a document command
(`DeviceChange::Kind`; the live node swaps IRs through `EngineBridge::update_builtin` →
`Node::set_data`, crossfading); `Device::ListFactoryIrs` → `FactoryIrs { irs: [FactoryIr {
id, name, category, length, channels }] }`. The IR is read and FFT-partitioned off the audio
thread; latency (if any) is reported for PDC.

### 13.7 External instrument and audio effect (`external-instrument`)
`ExternalRouting { midi_out: Option<port id>, midi_channel 1..=16, audio_send:
Option<HwChannels>, audio_return: Option<HwChannels> }` (`HwChannels { first, count 1|2 }`),
stored in the document like Ableton (a project opened elsewhere keeps the choices; unresolved
ports = silent until they reappear; collab peers use their own hardware). Params frozen in
`ether_devices::external` (instrument: Gain, Latency; effect: Send Gain, Return Gain, Mix,
Latency, Invert Phase). `Latency` (ms) is the node's `Node::latency` (PDC).
`External::SetRouting` is a document command; `ListPorts` → `HardwarePorts`;
`MeasureLatency` replies `Unit`, then `ExternalEvent::LatencyMeasured` (and sets `Latency`
as one undoable edit) or `MeasureFailed` (nothing back within 2 s). Hosts without hardware I/O
(web) reply `Unsupported` (`EngineBridge::list_hardware_ports` default). Engine
(`ether_core::hw_io`, stubs): the controller compiles `TrackDesc::hw_io` (`HwIoDesc` per
external device); `HwIoRt::{prepare, gather_returns, capture_send, write_sends}` copy
hardware inputs into return buffers before the jobs and add send buffers to the hardware
outputs after master; an instrument's MIDI goes to a lock-free ring of `HwMidiEvent`s
drained by the host's MIDI out thread (sample-accurate timestamps).

### 13.8 Undo history (`undo-history`)
`History::List` → `History { history: HistoryList { steps, current, truncated } }`
(`HistoryStep { id, label, time_ms, undone, checkpoint }`; chronological: applied steps oldest first,
then undone steps in redo order). `JumpTo { step }` undoes/redoes through the same path as
`Edit::{Undo, Redo}` (one batch of patches; `None` = before the oldest kept step; `NotFound`
for dropped steps). `SetCheckpoint { step, name }` names a step for the session.
`HistoryEvent::Changed` at most every 100 ms while a client listed the history. `History::State`
→ `HistoryState { history }` (what `Patch::history` carries; the UI reads it on connect, so
switching engines keeps undo enabled; it doesn't subscribe to `Changed`). Runtime only:
never saved, not undoable. Step ids are monotonic per session (merged gesture commits keep
their step's id; additive accessors in `ether_model::History`).

### 13.9 Rack presets
Preset file version 2: `Preset::rack: Option<PresetRack { chains: [PresetChain { name, color,
volume, pan, mute, solo, keys, velocities, select, devices: [PresetChainDevice] }],
modulators, mappings: [PresetModMapping { source: Macro { index } | Modulator { index },
target: Rack | ChainDevice { chain, device }, param, depth }] }>` (rack devices only; indices
into the preset's lists; checked on load). v1 files load unchanged (no `rack`: macros and
params only). `Preset::Load { device, preset, seed }`: loading a preset with `rack` replaces
the rack's chains, chain devices, the rack's modulators and the mappings inside it as one undo
step; new entities get `derive_id(seed, i)` in preset order (chains, then each chain's devices
in order, then modulators, then mappings); `seed` is required then (`InvalidArgument`) and
ignored otherwise (omitted from JSON when `None`, TS optional). No nested racks in v0.3.
Drum racks (still version 2, additive): a `DrumRack` preset's `rack` has no chains and
`pads: [PresetPad { note, name, color, choke_group, volume, pan, mute, devices:
[PresetChainDevice] }]` sorted by note (omitted when empty; unique notes, no macro sources),
with the rack's modulators and their mappings onto the rack; pad samples are stored like a
sampler preset's. Loading replaces the rack's pads (with their chains) in the same undo step;
pads take their ids after the chain devices (pads, then each pad's devices), before the
modulators. A drum rack inside a rack chain loads the pads and skips the modulators. Drum
rack presets without `rack` set params only and keep the pads.

### 13.10 Templates (`templates`)
File format `ether_model::template` (`.ethertemplate`, JSON `{ format: "ethereal-template",
format_version: 1, project_format_version, app_version, template: { name, meta, body } }`;
`project_format_version` is the `.ether` format of the body): `body` is
`Project { project, samples }` or `Tracks { entities, samples }` (one or more tracks and
everything hanging off them except clips, in `Project::entities` order, parents first;
`samples` by library location like sample-based presets). Templates live engine-side under
`<user library>/Templates/{Projects,Tracks}/`, the default project template id in
`Templates/default.json`; the UI never touches files. Ids: `"projects/<name>"`,
`"tracks/<name>"`, `"factory/<slug>"` (read-only). `Template::Insert { template, seed,
parent, before }` is a document command: every entity gets `derive_id(seed, i)` (`i` = its
index in `entities`), references between template entities are remapped, sends to tracks
outside are dropped and outputs reset to `Default`. `NewProject { id, name, template }`
behaves like `Project::Create` (replies `Project`). `List`, `SaveProject`, `SaveTracks`,
`Rename`, `Delete`, `SetDefault` manage the library (`TemplateEvent::Changed`). A body
from an older `.ether` format runs the document migrations (`template::migrate_body`: a
tracks body's entities go through a minimal document's tables and come back in order); a
newer one is `TooNew`; pre-release templates (`version`/`ether_version`) are refused.

### 13.11 Project versions and crash recovery
Versions are `.ether` documents at `<project>/versions/<id>.ether` (id `"<created ms>-<kind>"`,
kinds `Autosave | Manual | BeforeRestore`), written through `ProjectStore::{write, read,
list_dir, remove}` (`remove` is new and defaulted: `Unsupported`), so native disk, web OPFS
and the memory store all work (`versions/` is the project's backup folder). Autosave
versions: when the document changed since the last one, at most every `VERSION_INTERVAL_MS`
(1 min), then the folder is pruned (`versions::prune_versions`; also after a restore's
`BeforeRestore` snapshot). Unnamed autosave versions are thinned by the pure
`versions::retain(versions, now)` to at most `MAX_AUTOSAVE_VERSIONS` (30) in tiers:
**recent**, the newest and then each one ≥ 1 min older than the last picked, up to 5;
**hourly**, of the rest, the oldest of each clock hour for the 5 newest hours; **spread**, of
the rest, the oldest in each of 20 geometric age buckets (boundaries `6 h · rⁱ`, i = 1..19,
`r = (1 year / 6 h)^(1/20)` ≈ 1.44; first bucket open to 0, last to ∞); **fill**, while fewer
than 30 and more remain, the one farthest in `ln(age + 1 min)` from every kept one (older on
a tie). Oldest-per-bin keeps tier picks stable across prunings and never prunes the oldest
autosave; a version dated after `now` (clock went back) counts as written at `now`, behind
the one really written then, which is never pruned; a clock gone backwards makes the next
version due. Unnamed `BeforeRestore` versions: newest `MAX_BEFORE_RESTORE_VERSIONS` (10).
Named versions (`Manual`, or any renamed one) are never pruned and count toward no limit.
The UI's "save snapshot…" (palette, `Mod+Alt+S`, project menu) is `Create { name }`.
`Restore` snapshots a `BeforeRestore` version first, replaces the document (emits
`ProjectLoaded`, dirty). `Compare { version, against }` → `VersionDiff` (per-table counts,
settings changed, up to 50 named tracks/clips). Crash recovery: opening a project writes
`versions/.session`, a clean close removes it; `ListRecoverable` → projects whose marker
survived and whose newest version is newer than `project.ether`; `Recover` opens it (dirty);
`DiscardRecovery` removes the marker. Not undoable. The marker is written before the
project's devices and plugins are instantiated, so a plugin crashing while a project opens
counts as a crash. `SessionStatus` → `SessionStatus { unclean }`, the projects
whose marker another session left (crashed or killed), with or without work to recover
(the UI's crash dialog offers them, with "Open without plugins"; the reopen setting
requires none). In the browser the Worker never closes cleanly, so the page also records
whether it was unloaded cleanly (`ui/src/features/versions/session.ts`).

### 13.12 Keymap (`keymap`)
Actions and the built-in presets (`Ethereal`, `AbletonLike`) live in the UI command registry;
the engine stores only the user's `Keymap { preset, overrides: [KeyBinding { action, chords
}] }` at `<user library>/Settings/keymap.json` (`Keymap::{Get, Set, Reset}`,
`KeymapEvent::Changed`; hosts without a writable library keep it for the session). Chords:
modifiers in the order `Mod` (Cmd on macOS, Ctrl elsewhere), `Ctrl` (real Control, macOS),
`Alt`, `Shift`, then one key (`A`-`Z`, `0`-`9`, `F1`-`F24`, named keys, unshifted
punctuation), joined by `+` (`Mod+Shift+D`). Limits: `MAX_KEYMAP_OVERRIDES` = 1024,
`MAX_CHORDS_PER_ACTION` = 4; overrides sorted by action, unique. Not undoable.

### 13.13 Web node latency
The worklet sends a `LatencyReport` (latencies of live nodes whose value changed) at most
every `REPORT_INTERVAL_MS` (50 ms); the Worker bridge answers `EngineBridge::node_latency`
from the latest values so `EngineState::check_latencies` republishes PDC like natively. The
worklet fills a pre-sized buffer (no allocation).

### 13.15 Plugin editor keys (reused by `keyboard-midi`)
Plugin editors are native windows the host made (`HostWindow` of each format, AU's
`EditorWindow`, also inside sandbox helpers), so keys pressed there go to the plugin, never to
the web view. `ether_plugin_host::editor_keys` intercepts a few **captured** keys first:
`Space` always (the transport), plus the codes of `PluginCommand::SetEditorKeyCapture { codes
}` (`KeyboardEvent.code` values, replaces the previous set; native host only, the web replies
`Unsupported`; sandbox helpers get it before their next poll / editor open). A press is captured
only without Ctrl / Alt / Cmd (Shift allowed); its release is captured iff its press was;
everything else goes to the plugin unchanged. Hooks: macOS one `NSEvent` local key monitor
(swallows the event), Windows a thread `WH_GETMESSAGE` hook (the message becomes `WM_NULL`),
both acting only on registered editor windows (`KeyTap::register`, dropped with the window).
Captured keys are queued per window and drained by the format's `poll` as
`PluginNotification::EditorKey(PluginWindowKey)`; the controller emits
`Event::Plugin(PluginEvent::EditorKey { device, key: PluginWindowKey { key, code, shift, ctrl,
alt, meta, down, repeat } })`. UI side: `ui/src/features/plugin-keys` (`onPluginWindowKey`,
`setEditorKeyCapture(owner, codes)` which unions every owner's codes into one command,
`asKeyboardEvent` for `matchesAction`). Latency: one plugin poll (the host main-thread tick).

### 13.14 Choices worth reviewing
1. **Expression curves are one value per lane / note expression**, not one entity per point
   (thousands of points per performance). Collab merges whole curves last-writer-wins;
   `ReplaceRange` keeps range edits small on the wire.
2. **External device routing is in the document** (port ids and channels), like Ableton,
   rather than per-machine settings; unresolved ports are silent, not errors.
3. **Templates, keymaps and versions are engine-side files** (user library / project store),
   never touched by the UI, so a remote engine and the web (OPFS) behave the same.
4. **Undo history stays runtime** (not saved with the project); named checkpoints become
   durable only through.
5. **`engine.rs` was not pre-wired** (other v0.2 work was changing it): MIDI expression and
   `external-instrument` each add one call site for their (pre-built) RT state.

### 13.16 Computer MIDI keyboard
`RecordingCommand::LiveMidi { data: [u8; 3] }` (runtime, not undoable, not replicated,
allowed on a listen link) plays one channel voice message as if it came from a hardware
MIDI port: `EngineBridge::live_midi` (default `Unsupported`) queues it to the engine's live
MIDI ring (`ether_core::recording::LiveMidi`): natively `recording::inject_midi` (stamped
like midir input), on the web a binary `EngineMsg::LiveMidi` frame (tag `b'M'`, no JSON on
the audio thread) the Worklet pushes stamped 0 (next block). So every monitored MIDI track
plays it and armed MIDI tracks record it, through the same path as hardware input. Its port
is `COMPUTER_KEYBOARD_PORT` there too (`LiveMidi::port` = `recording::COMPUTER_KEYBOARD_KEY`):
a track whose input is filtered to another port (`TrackInput::Midi { port: Some(..) }`,
the engine's `TrackDesc::midi_input`) ignores it. The
controller also feeds MIDI capture with it (port `COMPUTER_KEYBOARD_PORT` = "Computer
Keyboard"); it never comes back through `poll_midi_input` (no MIDI learn / mappings).
`InvalidArgument` for system bytes or data bytes > 0x7f. The UI (`ui/src/features/
keyboard-midi`) maps keys by `KeyboardEvent.code` (Ableton's layout), sends note-offs on key
up / blur / hide / toggle off, auto-arms the selected MIDI track when no MIDI track would
hear the notes, and takes keys forwarded from native plugin windows
while it is on.

### 13.17 Piano-roll note preview
`RecordingCommand::PreviewNote { track, data: [u8; 3] }` (runtime, not undoable, not
replicated, allowed on a listen link) auditions a note on one track's instrument:
`EngineBridge::preview_note` (default `Unsupported`) → `EngineHandle::preview_note` →
`Control::PreviewNote`, queued in `RecordingRt` (fixed capacity, no allocation) and
delivered at offset 0 of the next sub-block to that MIDI track's first device (through
`ExpressionRt::live_input`, so MPE tracks read it like live input), armed / monitored or
not and whether the transport plays or not. Unlike `LiveMidi` it is never recorded or
captured and other tracks don't hear it. Web: a binary `EngineMsg::PreviewNote` frame (tag
`b'N'`, the track ulid then the 3 bytes). `InvalidArgument` for non channel-voice bytes,
`NotFound` for an unknown track. The UI (`ui/src/features/piano-roll/notePreview.ts`)
sends `[0x90, pitch, velocity]` on a note press (and at each new pitch of a vertical drag,
after the previous note's off) and the note off on release, pointer cancel, window blur and
editor unmount; the piano-roll toolbar's headphones toggle (persisted, default on) turns it
off. Drum pad audition uses the same command (`holdNote`, not gated by that toggle):
pressing a pad (pointer, Space / Enter, or its play button) holds `[0x90, pad note, 100]` on
the rack's track until release, so the note goes through the track chain, the rack and the
pad's own chain (sampler slice, effects, choke group, pad mix) exactly like playback,
instead of a `Media::Preview` of the whole file.
## 13. Agent API (docs/MCP.md)

- Protocol (`ether_protocol::agent`, exported to TS): `Command::Agent(AgentCommand)`.
  - `ListTools` replies `ReplyValue::AgentTools { tools: [AgentToolSpec { name, description,
    input_schema }] }`. `input_schema` is a JSON Schema `object`, as JSON text.
  - `CallTool { name, input }` (`input` is a JSON object, as text) replies
    `ReplyValue::AgentToolResult { content, is_error }`. `content` is JSON text, or plain text
    for errors. An unknown tool, invalid input (schema-checked) or rejected edit gives
    `is_error: true`, never a `CommandError`. With no project open, every tool is an
    `is_error` result.
- Registry: `ether-controller/src/agent/**`, the same on native, `ether-server` and WASM.
  An editing call is ONE undo step labelled `AI: <action>`, built from ordinary document
  commands. It is replicated in collab, and its `Event::Patch`es come before the reply.
  Read tools never edit.
- Tool shapes (pinned with `ai-chat`): `create_track {kind: midi|audio|group|return, name?}`
  → `{track_id}`; `create_midi_clip {track_id, start_beats, length_beats, name?}` →
  `{clip_id}`; `add_notes {clip_id, notes: [{pitch 0-127, start_beats (clip-relative),
  duration_beats, velocity 1-127}]}` → `{note_ids}`; `rename_track {track_id, name}`;
  `delete_track {track_id}`; `set_track_mix {track_id, volume_db?, pan? (-1..1), mute?,
  solo?}`; `set_tempo {bpm}`; `get_project_overview {}`; `get_clip_notes {clip_id}`;
  `undo {}`; `redo {}`. Tools may add optional inputs and result fields, but never remove
  or rename them. Every tool follows the same conventions: snake_case names, units in
  field names (`*_beats` in quarter notes from the song or clip start, `*_db`), string
  document ids, and JSON object results. Positions never depend on bar-line or
  time-signature rules.
- Desktop agent bridge (`apps/desktop/src-tauri/src/agent_bridge.rs` over
  `ether_server::agent_bridge`):
  - Opt-in, default off, persisted in `<app data dir>/config/agent.json`.
  - Tauri commands: `agent_bridge_status() -> { enabled, port, connected_clients }` and
    `agent_bridge_set_enabled(enabled) -> same`.
  - While enabled: a remote-engine listener (§11.5) on `127.0.0.1:<random port>`, with a
    fresh token, against the UI's engine. Bridge request ids start at `0x8000_0000` and
    gesture ids at `0x4000_0000`. The UI never receives bridge replies, and gets every
    event.
  - Runtime file: `<app data dir>/agent-bridge.json` = `{port, token, pid, version}`,
    mode 0600, deleted on disable and on quit.
- `ether-mcp` (stdio MCP server, `rmcp`): forwards `tools/list` and `tools/call` to
  `ListTools` and `CallTool`, and exposes the resource `ethereal://project/overview`.
  Modes: desktop (runtime file), `--server URL --token T`, and `--project PATH` (embedded
  headless engine).

## 14. Resolving missing plugins and audio files

- Protocol (`ether_protocol::resolver`, additive): `Command::Resolve(ResolveCommand)`,
  `ReplyValue::ResolveProposal { proposal }`, `Event::Resolve { event: ResolveEvent }`.
  - `Propose { installed: Vec<PluginDescriptor>, params?: Vec<PluginParams>, plugins?: bool }`
    replies `ResolveProposal { plugins: Vec<MissingPlugin>, media: Vec<MissingMedia>,
    folder_search }`. The controller has no plugin catalog: the UI passes the installed
    plugins (`Plugin::List`); `plugins: false` (hosts without plugins) lists audio files only.
    A `MissingPlugin` groups every device of one `(format, plugin_id)` not installed, with its
    saved params (`SavedParam { id, value, name? }`) and ranked `PluginCandidate { format, id,
    name, vendor, version, rule: MatchRule, confidence 0..1, reason, plan: ParamPlan, lost }`.
    `devices: Vec<MissingDeviceGroup { devices, source_id, description, origin, role, params,
    candidates: Vec<DeviceCandidate { replacement: Replacement, name, confidence, reason }> }>`
    groups the missing devices (below) by `source_id`; candidates are built-ins of the same
    role suggested by the description's words (`resolver/devices.rs`), never applied unasked.
  - `SearchMedia { media?, folder? }` replies `Unit`; `ResolveEvent::MediaProgress {
    scanned }`, one `MediaCandidates { media, candidates: Vec<MediaCandidate { source:
    MediaSource, display, confidence, reason }> }` per searched file, then `MediaSearchDone`.
    Searched: `folder` (desktop), the imported project's folder and the `Samples/` of it and
    up to three parents, the library roots. `CancelSearch` stops it.
  - `Apply { plugins: Vec<PluginResolution>, media: Vec<MediaResolution>, devices?:
    Vec<DeviceResolution { devices, replacement: Replacement (Builtin { device } | Plugin {
    plugin: ReplacementPlugin }) }> }`: one undo step
    "Resolve Missing Items". A plugin device keeps its id and is re-created with the new
    plugin (the model keeps a device's plugin id), with the state carried only for
    `ParamPlan::KeepState` (same format), parameter values, automation lanes, MIDI and
    modulation mappings moved by `ParamPlan::Map { map }` (unmapped ones dropped), or the
    deterministic choice for `ParamPlan::Auto`. A media gets the new location (copied into
    the project when the host can't reference the file) and content hash; a project that
    knew the file's content requires the same sample rate and channels (like Relink).
    A missing device is re-created (same id) as the replacement, its values, lanes and
    mappings moved by normalized parameter name (`replace::match_by_name`, shared with
    `Replace`; values through each range) and the rest listed in a warning notification; an
    instrument can't replace a device on an audio track, nor a rack or drum rack any device.
    `Replace { device, plugin? | builtin?, retain, preview? }` (one undo step "Replace Device")
    takes any device but a rack (built-in, plugin or missing) to a built-in or plugin of the
    same role (audio effect, instrument, MIDI effect: checked when both are known); racks and
    drum racks are never replaced nor replacements. The same plugin (or another version) or
    built-in type keeps its state and parameter ids; the sidechain source goes (reported) when
    the new device has no sidechain input.
    `Apply` items that fail are skipped with a warning notification; nothing valid = error.
- Missing devices (model, additive): `DeviceKind::Missing { missing: MissingDevice {
  description, source_id, origin, role: MissingRole (Instrument | AudioEffect | NoteEffect),
  params: Vec<MissingParam { id, name, min, max }> } }`. The import maps a source device with
  no Ethereal equivalent to one (`Device::params` hold its plain values, automation lanes
  normalized linearly over each range); it has no engine node (audio passes through, an
  instrument placeholder is silent; offline renders skip it), `GetDescriptor` replies its
  parameters (`DeviceTypeRef::Missing { source_id }`, linear), presets refuse it, exports to
  other DAWs report it.
- UI: the dialog shows on project open (and when the import report closes) when anything is
  missing, unless turned off ("Show when a project opens", per user); "Resolve later" closes
  it. Missing audio files found while the engine loads them within 20 s of the open are added
  to it. The top bar's "N missing" badge and the project menu's "Missing items…" reopen it.
- Deterministic matching (`ether-controller/src/resolver/plugins.rs`, unit-tested):
  `SameId` 0.95 (a VST3 class id derived from the VST2 id, `565354` + id; an AU subtype =
  the VST2 id), `SameName` 0.9 (name without arch/format suffixes, other format),
  `SameFamily` 0.85 / 0.8 (name without versions, same vendor), `Similar` 0.45..0.7
  (Jaro-Winkler ≥ 0.88); a different vendor never matches; an unknown vendor costs 0.1.
  Vendor names fold company suffixes and aliases (`NI` = `Native Instruments`).
- `MatchRule::Ai` and `Manual` are UI-only: AI suggestions come from the AI chat's provider
  (`ui/src/features/import-resolver/ai.ts`), after a one-time consent showing the payload,
  validated strictly and never applied without the user switching them on.
- Hosts: no host code (the commands go to the controller). Mock:
  `ui/src/transport/mock/roadmap/resolver.ts`.

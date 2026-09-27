# Roadmap features: hook points (contracts v2)

`contracts-2` froze the cross-boundary contracts of the 12 roadmap features and pre-created
one module per feature and layer, already registered with one line in its parent, so the
feature nodes can run in parallel. Each node owns exactly the files listed for it in
[`.github/ownership.toml`](../.github/ownership.toml) (block "Roadmap features"). Every
pre-created module starts with a doc comment saying what goes there. The contract details
(semantics, invariants, PDC, partition rules) are in [CONTRACTS.md §11](CONTRACTS.md).

Ground rules (same as wave 3):
- Protocol and model *types* are frozen (`crates/ether-protocol/**`, `crates/ether-model/src/*`
  except the node's own helpers): changes go through a BCR. Model validation for the new
  entities is already implemented (`ether-model/src/apply.rs`) and tested
  (`ether-model/tests/roadmap_v2.rs`).
- Until a node lands, its commands reply `Unsupported` (`ether-controller/tests/roadmap_v2.rs`
  pins this; replace those assertions with real behaviour tests), and its built-in devices
  are pass-through placeholders.
- **MockTransport already simulates every new command**, one file per node under
  `ui/src/transport/mock/roadmap/<feature>.ts` (+ its test), dispatched from
  `roadmap/index.ts` (base-17 split), so UI work can start before the Rust side. Keep your
  mock file in sync if a behaviour detail changes.
- UI placeholders render the standard `eth-feature-placeholder` markup (or nothing, for
  top-bar/inline slots); no styling (the `design-system` node owns styling).
- "Shared touch" = a minimal, additive edit in a shared file. **Hot files**
  (`ether-core/src/{engine.rs, graph.rs, mixer.rs}`) were pre-wired in base-17: the
  metronome call, sidechain ordering/PDC/taps (`ether-core/src/sidechain.rs`), the drum-rack
  pad hook (`ether-core/src/drum_rack`), and `Engine::set_executor`. `tempo-metronome`,
  `sidechain` and `drum-rack` work only in their own core module; only `multicore` (which
  lands last) edits the hot files. Need another hook there? Send a BCR.
- Frozen store hooks (base-17): `ProjectStore::write_export` (implemented natively and in
  `MemoryStore`; the web default means "download") and upload staging
  (`begin/append/read/discard_upload`; native impl delegates to
  `ether-native/src/uploads.rs`).

## `export`

Owns: `ui/src/features/export/**`, `crates/ether-controller/src/export/**`,
`crates/ether-core/src/offline.rs`.

- Protocol: `ExportCommand`/`ExportEvent`/`ExportResult`/`ByteChunk`
  (`crates/ether-protocol/src/export.rs`), `ReplyValue::{ExportStarted, Bytes}`,
  `Event::Export`.
- Core: `ether_core::offline::OfflineRenderer` is implemented (fresh engine, no device,
  renders on the caller's thread, drains engine outputs; `latency()` = frames to drop at the
  start, `diagnostics()` = overflow/underrun flags; tested).
- Controller: `EtherController::export_command` (dispatched from `handlers.rs`) and
  `export_tick` (called every tick) in `export/mod.rs`. Plugins offline:
  `EngineBridge::create_offline_plugin` (defaulted `Unsupported`; implement it in
  `ether-native/src/bridge.rs`). Files: `ProjectStore::write_export` (native writes
  `<project>/exports/`, already implemented); `Err(Unsupported)` (web) = keep the bytes for
  `Export::ReadChunk`. Deps `hound`/`flacenc` are pre-declared in
  the workspace (versions are verified on first use).
- UI: `ExportDialog` in the top bar (`data-slot="export"` in `App.tsx`).

## `devices-2`

Owns: `crates/ether-devices/src/{eq,reverb,limiter,utility}.rs`.

- Model: `BuiltinDevice::{Eq, Reverb, Limiter, Utility}` (+ `BuiltinDeviceType`,
  `BuiltinDeviceType::ALL`, `BuiltinDevice::new`). Param ids are yours to define in each
  module's `descriptor()`; append-only once released.
- `ether-devices/src/lib.rs` already dispatches `descriptor()`/`create()` to the modules; they
  currently return `placeholder::Placeholder`. Limiter lookahead is reported via
  `Node::latency` (PDC). Update the mock descriptors (`ui/src/transport/mock/builtinDevices.ts`).
- The limiter may take a sidechain later (coordinate with `sidechain`: `sidechain_inputs`).

## `tempo-metronome`

Owns: `ui/src/features/tempo/**`, `crates/ether-controller/src/tempo/**`,
`crates/ether-core/src/metronome.rs`.

- Protocol: `TempoCommand` (`tempo.rs`): tempo-point/time-signature CRUD and
  `SetMetronomeSettings`. Document command (`doc::apply` → `tempo::apply`).
- Model: `ProjectSettings::{metronome_volume, metronome_accent, metronome_sound}`,
  `SettingsChange::Metronome*`.
- Core: `RenderGraphDesc::click: MetronomeDesc` (compiled by `tempo::metronome_desc`), and
  `Metronome::render` (placeholder, silent), already called by `engine.rs` once per
  sub-block after master reaches the hardware outputs (and `reset` on jumps), with the
  graph's output latency: delay each click by it so it lines up with the PDC-delayed
  music. Implement it in `metronome.rs` only.
- Count-in (recording): the controller's record session pre-rolls `count_in_bars`
  (`ether-controller/src/recording/mod.rs`). Set `MetronomeDesc::count_in_end` to the record
  start on the published desc for the duration of the pre-roll; the click then sounds even
  with the metronome off.
- UI: `TempoEditor` (detail tab "tempo"), `MetronomeSettings` (top bar `data-slot="metronome"`).

## `clip-editing`

Owns: `ui/src/features/clip-editing/**`, `crates/ether-controller/src/clip_editing/**`,
`crates/ether-core/src/fades.rs`.

- Model: `AudioContent::{fade_in_curve, fade_out_curve, reversed}`, `FadeCurve`,
  `ClipChange::{FadeInCurve, FadeOutCurve, Reversed}`; `Marker` entity + `MarkerChange`.
  Crossfade/overlap rules: `ether_model::clip` module docs.
- Protocol: `ClipCommand::{SetFadeCurves, SetReversed, Crossfade}` (routed to
  `clip_editing::clip_command` from `doc/clips.rs`), `MarkerCommand` (`markers.rs`).
- Core: `fades::fade_gain` is the fade law (implemented and mirrored in
  `ui/src/features/clip-editing/fades.ts`); switch `sched::render_audio` to it and
  implement reverse playback there (`ClipContentDesc::Audio::{fade_in_curve, fade_out_curve,
  reversed}` are already compiled).
- UI: `MarkerLane` (`data-slot="markers"` above the arrangement); fade handles in
  `ClipView.tsx`/`clipDraw.ts` (shared touch).

## `remote-engine`

Owns: `crates/ether-server/**`, `ui/src/transport/ws/**`, `ui/src/features/remote/**`,
`crates/ether-controller/src/upload/**`.

- Protocol: `remote.rs` (handshake `ClientHello`/`ServerHello`, `ServerInfo`, binary frame
  codec implemented in Rust and `ui/src/transport/ws/binaryFrame.ts`), media upload
  (`MediaCommand::{BeginUpload, UploadChunk, CancelUpload}`, `MediaSource::Upload`,
  `MediaEvent::UploadProgress`).
- `ether-server`: stub lib (`ServerConfig`, `serve`) + binary. Add `ether-native` and
  `tungstenite` (pre-declared) as deps. Dev port: `PORT_OFFSETS.remote` = base `+4`
  (`scripts/dev-env.mjs`; `+3` is reserved for the collab server).
- Controller: `EtherController::upload_command` (`upload/mod.rs`) over the frozen
  `ProjectStore` upload staging methods; native staging in `ether-native/src/uploads.rs`.
- UI: `WsTransport` stub (`kind: "remote"`), `ConnectDialog` (top bar `data-slot="remote"`);
  runtime transport switching needs `TransportProvider`/`createDefaultTransport` (shared touch).

## `collab` (reserved envelope)

Owns: `crates/ether-collab/**`, `crates/ether-controller/src/collab/**`,
`ui/src/features/collab/**`.

- Model: `SiteId` (u64 as decimal string), `ActorId`, `OpOrigin`, `StampedTransaction`;
  `Patch::origin` (optional, omitted when `None`) marks patches caused by another site.
  Per-site undo goes in `ether-model/src/history.rs` (granted).
- Protocol: `CollabCommand`/`CollabEvent`/`Presence`/`PresenceState`/`CollabMessage`
  (`collab.rs`). All `CollabCommand`s reply `Unsupported`.
- Refine through BCRs (CRDT library choice, extra variants). UI: `PresenceBar` (top bar
  `data-slot="collab"`).

## `multicore`

Owns: `crates/ether-core/src/parallel.rs`, `crates/ether-native/src/workers.rs` (new).

- `EngineConfig::worker_threads` (default 0 = v0.1 behaviour; ignored on wasm32),
  `ParallelExecutor` trait + `SequentialExecutor`, injected with `Engine::set_executor`
  (stored, unused until you dispatch through it). Partition contract and the unsafe-sharing
  argument: CONTRACTS.md §11.7 and `parallel.rs` docs.
- You own the hot files: `graph.rs` (levels), `engine.rs`/`mixer.rs` (per-level dispatch),
  plus native host wiring. Land last. No protocol changes.

## `web-perf`

Owns: `crates/ether-core/src/codec.rs`, `crates/ether-wasm/src/**`, `apps/web/src/engine/**`.

- `GraphCodec` trait (+ `CodecError`) for a binary `RenderGraphDesc` encoding between the
  controller Worker and the AudioWorklet (today JSON in `ether-wasm/src/proto.rs`).
  Round-trip exactness and versioning rules are in the trait docs. On the web `decode`
  (and the compile after it) runs in the AudioWorklet and allocates, like today's JSON path:
  make it cheap and bounded. `ether-wasm/src/{store,bridge}.rs` are excluded from your
  glob (shared with export/drum-rack).

## `midi-learn`

Owns: `ui/src/features/midi-learn/**`, `crates/ether-controller/src/midi_learn/**`.

- Model: `MidiMapping` entity (`midi_map.rs`: `MidiSource`, `MidiControl`, `MidiMapTarget`,
  `TransportAction`, `MidiMapMode`, `RelativeEncoding`), one mapping per source; deleting a
  device/track/send cascades its mappings (`doc/mod.rs`, done).
- Protocol: `MidiMapCommand` (`Map`/`Edit`/`Unmap` are document commands via
  `midi_learn::apply`; `Learn`/`List` via `EtherController::midi_map_command`),
  `MidiMapEvent`, `ReplyValue::MidiMappings`, host input `MidiInputEvent`.
- Input: `EngineBridge::poll_midi_input` (defaulted; drained in `midi_learn_tick`). Natively,
  extend the recording MIDI path (`ether-native/src/recording/midi.rs`, `MidiInputs::refresh`
  callback → add the port id) to also queue `MidiInputEvent`s for the bridge.
- UI: `MidiLearnPanel` (sidebar tab "midi"). Mock: `MockTransport.simulateMidiInput`.

## `sidechain`

Owns: `ui/src/features/sidechain/**`, `crates/ether-controller/src/sidechain/**`.

- Model: `Device::sidechain` (+ `DeviceChange::Sidechain`); sidechain edges are routing
  edges (no cycles; the source track can't be removed while referenced; deleting it cuts
  the sidechain, done in `doc/mod.rs`).
- Protocol: `DeviceCommand::SetSidechain` (→ `sidechain::set_sidechain`),
  `DeviceDescriptor::sidechain_inputs` (0 everywhere for now).
- Core: done in base-24 (`ether-core/src/sidechain.rs`: ordering, tap before the output
  PDC delay, main/sidechain alignment delays, tested in `tests/base24_hooks.rs`,
  CONTRACTS.md §11.10). `Node::sidechain_inputs` / `Node::process_sidechain` are
  defaulted: give the compressor (and the limiter, with devices-2's params) a sidechain
  input and use the signal.
- UI: `SidechainSelector` in every device header (`features/devices/DeviceView.tsx`, one
  line, renders nothing when `sidechain_inputs == 0`).

## `groove`

Owns: `ui/src/features/groove/**`, `crates/ether-controller/src/groove/**`.

- Model: `ProjectSettings::{swing, swing_grid}` (playback swing; see the field docs).
- Protocol: `GrooveCommand::{Humanize, SetSwing}` (`groove.rs`), `NoteCommand::Quantize::swing`
  (handle it in `doc/notes.rs`, currently ignored).
- Controller: `groove::swing_notes` is already called by `compile.rs` for every MIDI clip
  (no-op until implemented).
- UI: `GroovePanel` (detail tab "groove"); piano-roll controls are shared touches.

## `drum-rack`

Owns: `ui/src/features/drum-rack/**`, `crates/ether-controller/src/drum_rack/**`,
`crates/ether-core/src/drum_rack/**`, `crates/ether-devices/src/drum_rack.rs`.

- Model: `BuiltinDevice::DrumRack`, `DrumPad` entity (`drum_rack.rs`), `Device::pad` (pad
  chains), `Project::{pads_of, pad_devices_of}`; `devices_of(track)` excludes pad devices.
  Sampler slicing: `BuiltinDevice::Sampler::slices: SliceSettings`. Deleting a rack device
  cascades its pads and pad devices (`doc/mod.rs`, done).
- Protocol: `DrumRackCommand`, `SliceCommand`, `AutoSlice` (`drum_rack.rs`), both document
  commands (→ `drum_rack::{rack_command, slice_command}`).
- Core: `TrackDesc::racks: Vec<RackDesc>` (compiled by the controller's
  `drum_rack::racks_desc`, empty until implemented), `PadDesc`. The engine side is wired
  and basic (base-24, `ether-core/src/drum_rack/`, tested in `tests/base24_hooks.rs`):
  pad-chain nodes get live params, automation, latency refresh and PDC; `run_pads` routes
  notes by key (→ `PAD_PLAY_NOTE`), runs pad chains aligned to the longest one and mixes
  them with pad gain into the rack node's input. Left for you: choke groups, smoothing pad
  mix changes, carrying pad state across snapshot swaps (`RacksRt::inherit`). Pad devices
  already get engine nodes (every document device does).
- In-place slice edits: `EngineBridge::update_builtin` → `EngineHandle::set_node_data` →
  `Node::set_data` (implemented plumbing; the sampler and bridges implement it), so slice
  edits don't re-create the node and cut notes. `SliceCommand::ToDrumRack` takes
  client-chosen ids.
- Structure rules (done, base-17): `Device::Move` rejects pad devices (use
  `DrumRack::MoveDevice`) and racks with pads across tracks; device/track duplication copies
  pads and pad chains; the model re-checks pad devices when their rack changes.
- Devices: `drum_rack.rs` (rack node, placeholder), slice mode in `sampler.rs` (shared touch).
- UI: `DrumRackView` (detail tab "drum-rack").

## `media-preview` (base-24)

Owns: `crates/ether-core/src/preview.rs`, `crates/ether-controller/src/media_preview/**`,
`ui/src/transport/mock/roadmap/mediaPreview.*`, `ui/src/features/browser/preview*` (new
files), its tests/e2e.

- Protocol: `Media::{Preview, StopPreview}` (existing), `MediaEvent::{PreviewStarted,
  PreviewEnded { reason }}`, `PreviewEndReason` (CONTRACTS.md §11.15).
- Core (implemented, tested in `tests/base24_hooks.rs` and `tests/no_alloc.rs`):
  `EngineHandle::preview(PreviewControl)`, one voice mixed after master once per
  sub-block, auto-stop. Ids are frozen (CONTRACTS.md §11.15): every `Play` carries a
  controller-chosen monotonic id; `EngineOutputs::preview_ended = Some(id)` reports natural
  ends only; the controller emits `Stopped`/`Replaced` itself and `Finished` only for its
  current id. Left: a short fade on stop/replace.
- Controller: `media_preview::{preview_command, preview_tick}` (dispatched from
  `handlers.rs`, currently `Unsupported`): resolve the source (library or project media),
  decode + resample with `ether-media` (bounded per tick), `EngineBridge::preview`
  (defaulted `Unsupported`; implement in `ether-native/src/bridge.rs` with an in-memory
  source and in the web bridge/worklet like `load_media`), emit the events.
- UI: the browser already sends `Preview`/`StopPreview` (`features/browser/index.tsx`,
  shared touch): reset the previewing row on `PreviewEnded`. The mock (`MockPreview`)
  already emits the events.
- **Web (overlap with `web-perf`).** `ether-wasm/src/{proto,worklet}.rs` belong to
  web-perf; your shared touch there is limited to: reusing the existing `LoadMedia`
  chunk path under a dedicated preview `MediaId`; one additive
  `JsonMsg::Preview { media, gain, id }` (with `media: None` = stop); and one extra field in
  the worklet's engine report (`preview_ended`). Add **no new binary frame tags**, and merge
  `origin/dev` after web-perf lands before touching these files.

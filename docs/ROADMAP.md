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

## `live-record` (base-43)

Owns: `crates/ether-native/src/recording/**`, `crates/ether-controller/src/recording/**`,
`ui/src/features/recording/**`, its tests; shared touches in the native bridge, the
arrangement `TrackRow.tsx`/`clipDraw.ts` (additive overlay) and `MockTransport.ts`.

- Protocol (frozen): `RecordingEvent::Progress { audio: Vec<LiveAudioChunk>, midi:
  Vec<LiveMidiNote> }`: only new data since the previous event, ~20 Hz, runtime only (never
  in the document or collab). `LiveAudioChunk` carries merged-channel min/max peaks at a fixed
  `frames_per_peak` with `first_peak` indexing and the take's latency-compensated `start`.
- Host: the native recording writer thread computes peaks as it drains the capture ring
  (off the audio thread); `EngineBridge::poll_recording` (defaulted no-op) returns them.
- Controller: poll in the tick while recording, emit `Progress`.
- UI: a live clip per armed track growing to the playhead (waveform / notes), replaced by
  the committed clip on `Stopped`; punch/count-in show only the kept range; loop takes.

## Collab v2: presence and listen-on-peer (base-53)

Design and frozen contract: [COLLAB.md §8-§11](COLLAB.md), CONTRACTS.md §11.16. base-53
landed the protocol types, the relay routing/limits/ICE advertisement (implemented and
tested), the engine stream tap, the defaulted `EngineBridge` hooks, and one controller module
per node, already dispatched from `collab/mod.rs`. Until a node lands, its commands reply
`Unsupported` (pinned in `ether-controller/tests/collab_prewire.rs`; each node removes ONLY
its own assertions there). The three first nodes run in parallel; `plugin-mirror` comes
later. Shared touches are additive; in `collab/mod.rs` they are limited to dispatch lines.
The repo owner's UX passes on dev are authoritative (reuse their patterns, kit components,
tokens).

### `presence-v2`

Owns: `crates/ether-controller/src/collab/presence.rs`,
`crates/ether-controller/tests/collab_presence*.rs`, `ui/src/features/collab/presence/**`
(new), `apps/web/e2e/collab-presence.spec.ts`.

- Controller: `SetPointer` throttled to 30 Hz (latest wins, flushed on the next tick,
  clears never throttled) → `CollabMessage::Pointer`; peers' pointers →
  `CollabEvent::Pointer` (own dropped), a clear on a peer's `Leave`. Viewport, activity and
  following already travel in `PresenceState` (nothing to do in the controller).
- UI: pointer publishing from the arranger (song coordinates, rAF-coalesced, clear on leave /
  blur / unmount), the pointer overlay (interpolated, name label, peer colour), activity
  hints (`activity` set/cleared around arrangement gestures), follow mode (chip click,
  viewport publishing and applying, stop on local scroll/zoom/Escape), the listening badge
  from `listening_to`.
- Shared touches: `ui/src/features/collab/{store.ts, PresenceBar.tsx, PresenceBar.test.tsx,
  index.tsx, collab.css}` (handle `Pointer` in the store, chip menu entries),
  `ui/src/features/arrangement/{ArrangementView.tsx, arrangement.css, clipDrag.ts,
  trackDrag.ts}` (mount the overlay, publish pointer/viewport, set activity),
  `ui/src/transport/mock/roadmap/collab.*` (simulate peer pointers),
  `crates/ether-controller/src/collab/mod.rs` (dispatch lines only),
  `crates/ether-controller/tests/collab_prewire.rs` (own assertions only).

### `stream-host`

Owns: `crates/ether-native/src/stream/**` (new: tap reader, resampler, Opus encoder,
str0m sender thread), `crates/ether-collab/src/relay/ice/**` (new: STUN responder, TURN
server behind the `turn` feature, TURN-REST credentials), `crates/ether-controller/src/
collab/stream_host.rs`, `ui/src/features/collab/host/**` (new: the web sender),
tests `crates/ether-native/tests/stream*.rs`, `crates/ether-collab/tests/ice*.rs`,
`crates/ether-controller/tests/collab_host*.rs`, `apps/web/e2e/collab-host.spec.ts`.

- Native: `EngineBridge::{stream_capabilities, start/stop_stream_capture, stream_open,
  stream_signal, stream_close, poll_stream}` in `ether-native` (install the tap with
  `EngineHandle::set_stream_tap`, sender thread: ring → 48 kHz → Opus 20 ms → one str0m
  `Rtc` per listener on one UDP socket, srflx via one STUN Binding, anchors from the tap
  headers, bitrate from str0m's estimate).
- Controller: hosting policy, accept/refuse `Listen` (≤ 8), endpoint choice (Engine/Ui),
  signal routing, `poll_stream` drain, `TransportRequest` application (§9.5), `can_host`,
  host part of `ListenStatus`, cleanup on Unlisten/Bye/Leave.
- Relay: STUN on the relay's UDP port, optional TURN (`turn` feature), per-site credentials
  through `Relay::set_ice_provider`, flags `--no-stun`, `--turn`, `--public-host`,
  `--public-ip`, `--turn-ports`, `--turn-allow-private`.
- Web sender: second worklet output (the tap: master + metronome, minus preview) →
  `MediaStreamAudioDestinationNode` → one `RTCPeerConnection` per `ListenerLink { endpoint:
  Ui }`, Opus SDP parameters, RTP↔position anchors through a read-only sender encoded
  transform (`SendStreamClock`), `SetHosting { ui_sender: true }` when supported.
- Shared touches: `crates/ether-native/src/bridge.rs` (delegations), `ether-native/src/lib.rs`
  (`mod stream;`), `ether-collab/src/relay/mod.rs` (`mod ice;`),
  `crates/ether-native/Cargo.toml`, `crates/ether-collab/Cargo.toml`, root `Cargo.toml`
  (workspace deps `str0m`, `opus`, `stun`, `turn`), `crates/ether-collab/src/relay/
  server.rs` + `src/bin/relay.rs` (bind UDP, install the ICE provider),
  `crates/ether-wasm/src/worklet.rs` (copy the tap into worklet output 1; web-perf's file,
  that change only), `apps/web/src/engine/{endpoint.ts, engine.worklet.ts}` (second
  output), `ui/src/transport/wasm/WasmTransport.ts` (expose the stream `MediaStream`),
  `ui/src/features/collab/{store.ts, PresenceBar.tsx, index.tsx}` (hosting toggle, listener
  list), `ui/src/transport/mock/roadmap/collab.*`, `crates/ether-controller/src/collab/
  mod.rs` (dispatch lines only), `crates/ether-controller/tests/collab_prewire.rs` (own
  assertions only), `THIRD_PARTY_NOTICES.txt`.

### `stream-listen`

Owns: `crates/ether-controller/src/collab/listen.rs`,
`crates/ether-controller/tests/collab_listen*.rs`, `ui/src/features/collab/listen/**` (new:
the receiver, the stream clock mapping and its tests, the listen UX),
`apps/web/e2e/collab-listen.spec.ts`.

- Controller: `Listen`/`StopListening`, holding the local transport stopped, the transport
  intercept (`collab_transport_intercept`, called first in `handlers.rs::transport_command`:
  forward Play/Stop/TogglePlay/Locate/loop, refuse recording), `Event::Transport` from the
  host's anchors, signal/clock relay to the UI, `ListenStatus` (listener part +
  `collab_host_listeners()`), end cases (§9.2), `listening_to`, restore the local transport
  at the last heard position.
- UI: "Listen on <name>'s computer" in the peer chip menu / collab dialog, **disabled with a
  reason** when `RTCPeerConnection` is missing (WebKitGTK); the receiver
  (`RTCPeerConnection`, ICE servers from `CollabEvent::IceServers`, `<audio>`/AudioContext
  started in the click gesture), the playhead mapping of §9.4 at rAF, status and errors.
- Shared touches: `ui/src/state/playhead.ts` (an override source while listening),
  `ui/src/transport/TransportProvider.tsx` (suspend the local playhead feed while
  listening), `ui/src/features/collab/{store.ts, PresenceBar.tsx, PresenceBar.test.tsx,
  index.tsx, collab.css}`, `ui/src/transport/mock/roadmap/collab.*`,
  `crates/ether-controller/src/collab/mod.rs` (dispatch lines only),
  `crates/ether-controller/tests/collab_prewire.rs` (own assertions only).
- Integration with `stream-host`: the e2e (two browser contexts, web host) needs both; until
  then test against a scripted host (controller tests with the in-memory hub and a fake
  bridge; UI tests with a fake `RTCPeerConnection`).

### `plugin-mirror` (later)

Owns: `crates/ether-controller/src/collab/mirror.rs` (new; one `mod` line in
`collab/mod.rs`), `crates/ether-native/src/plugin_mirror.rs` (new; one `mod` line in
`ether-native/src/lib.rs`),
`crates/ether-controller/tests/collab_mirror*.rs`.

- `EngineBridge::{create_plugin_mirror, destroy_plugin_mirror, set_plugin_mirror_param}` in
  `ether-native` (GUI-only instance, never in a graph; edits come back as `ParamEdited`),
  `OpenEditor` falls back to the mirror, the controller pushes document param changes into
  mirrors and (setting) swaps live instances for mirrors while listening.
- Shared touches: `crates/ether-native/src/bridge.rs`, `crates/ether-native/src/plugins.rs`
  (editor routing), `crates/ether-native/src/host.rs` (`OpenEditor`),
  `crates/ether-controller/src/collab/mod.rs` (one mod line + dispatch).

# v0.2 (contracts-3)

`contracts-3` froze the v0.2 contracts ([CONTRACTS.md §12](CONTRACTS.md)) and pre-created
one module per node and layer, registered in its parent with one line, so the v0.2 nodes
run in parallel with disjoint files. Each node owns exactly its block in
[`.github/ownership.toml`](../.github/ownership.toml) ("v0.2"); the lists below summarize
them. Every pre-created module starts with a doc comment saying what goes there.

Ground rules (as for the roadmap v2 nodes above):
- Protocol and model *types* are frozen (changes via BCR). Model validation of the new
  entities is implemented (`ether-model/src/apply.rs`) and tested
  (`ether-model/tests/roadmap_v3.rs`); `.ether` is v4.
- Until a node lands its commands reply `Unsupported`, pinned by **one test per node** in
  `crates/ether-controller/tests/roadmap_v3.rs`: each node rewrites or deletes only its own
  test function. New devices are placeholders (`ether_devices::contract::Placeholder`).
- The MockTransport routes every new command to one file per node under
  `ui/src/transport/mock/roadmap/` (+ its test); device descriptors are generated JSON in
  `ui/src/transport/mock/devices/` (one file per device node).
- **Hot files** (`ether-core/src/{engine.rs, graph.rs, mixer.rs, codec.rs}`) were pre-wired
  once by contracts-3 (CONTRACTS.md §12.11). Nodes work in their own core module; the few
  node-specific shared touches there are listed in their blocks. Need another hook: BCR.
- The repo owner's UX passes on dev are authoritative (kit components, tokens, `midiTarget()`
  on new controls, existing context menus). **Every PR with a UI-visible change includes
  screenshots** uploaded with `.orchestra/pr-screenshot.sh <node-id> <file.png> "<caption>"`
  and embedded under `## Screenshots` in the PR body.
- **Order.** `groups-buses` and `file-import` are priority 1 (land first; the desktop path
  half of `file-import` needs `media-references`, its web/remote upload half does not).
  `device-ui` lands early: device
  nodes target its renderer (until then they test their layouts with the generic view and
  the JSON parity test). Everything else runs in parallel.

## Device agent guide (synth-2, multisampler, fx-color, fx-modulation, fx-dynamics, fx-analysis, midi-fx, racks-modulation)

1. **Your module.** `crates/ether-devices/src/<group>/` (split into files freely). The param
   table in `descriptor()` is frozen by contracts-3: ids are dense and **append-only** (never
   renumber, rename freely only before a release). Use the named ids in
   `<group>::<device>::*`. Replace `Placeholder` in `create()` with your node.
2. **Real-time rules** (docs/ARCHITECTURE.md): no allocation, locks, I/O or unbounded work
   in `process`/`reset`/`set_data`/`analysis`; allocate in `new`/`prepare`. Apply
   `EventKind::Param` at its sample offset (`util::split_at_events`) and smooth continuous
   params. Write `assert_no_alloc` tests like `tests/devices2.rs`
   (`tests/<group>*.rs`, global `AllocDisabler`), plus render tests (impulse/sine, known
   outputs, extreme params, NaN-free).
3. **Latency** (lookahead, oversampling): report it with `Node::latency` (PDC).
3b. **If your device has a detector, support sidechain**: declare `sidechain_inputs = 2`
   (already set for `Gate`, `MultibandCompressor` (external key drives all bands) and
   `AutoFilter` (its envelope follower follows the sidechain)) and key the detector from
   `Node::process_sidechain` like `compressor.rs`/`limiter.rs`: the sidechain arrives
   latency-aligned (CONTRACTS.md §11.10); without a source, `process` keys from the input.
4. **Descriptor ↔ mock parity.** After any descriptor change run
   `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v02_descriptors` and commit
   your JSON (`ui/src/transport/mock/devices/<group>.json`). Never edit it by hand.
5. **Layout.** Ship a `DeviceLayout` for every device in `descriptor().layout`
   (`ether_protocol::layout`; builders in `contract::{layout, section, item, knob}`): hero
   controls `Large`, typed widgets where they help (envelopes, filter curve, transfer curve,
   zone map, spectrum/tuner, step editor). Only specs and widget data: no bespoke panels,
   no styling. The shared renderer (`device-ui`) draws it with kit components and tokens and
   makes every param MIDI-learnable (`midiTarget()`) and a modulation target. Screenshot the
   panel rendered by the shared renderer in the dark theme for your PR.
6. **Analysis / meters.** Implement `Node::{has_analysis, analysis}` (copy only; compute in
   `process`) with the encodings of CONTRACTS.md §12.4.3.
7. **Factory presets.** A few good ones per device under
   `crates/ether-devices/presets/<device-key>/<slug>.etherpreset` (`ether_model::preset`
   format, `BuiltinDeviceType::key()` folder), registered in your module's
   `factory_presets()` with `include_str!`; `v02_descriptors.rs` parses them.
8. **MIDI effects** follow CONTRACTS.md §12.4.4 (MIDI-thru of what you don't transform,
   `AllNotesOff` forwarded, own note ids, delays only).
9. **Tests and e2e.** Your `tests/<group>*.rs`, your function in `roadmap_v3.rs`, an e2e
   (`apps/web/e2e/<device>*.spec.ts`: insert the device, turn a knob, hear/see the effect).

## `groups-buses` (priority 1)

Owns: `crates/ether-core/src/{bus_tap,vca}.rs`, `crates/ether-controller/src/groups/**`,
`ui/src/features/groups/**`, mock `roadmap/groupsBuses.*`, its tests/e2e.

- Protocol: `Track::{GroupSelected, Ungroup, SetVca}`; `TrackInput::Track { track, tap }`
  (`Recording::SetInput`); `TrackKind::Vca`, `Track::vca` (CONTRACTS.md §12.10).
- Controller: `groups::track_command` (from `doc/tracks.rs`), `groups::vca_descs`
  (`RenderGraphDesc::vcas`; VCA solo folded into `TrackDesc::solo`). Compile already skips
  VCA tracks and fills `TrackDesc::{input_tap, vca}`; deleting a VCA unassigns (done).
- Engine: implement `bus_tap` (tap buffers, aligned delay, monitoring) and `vca` (gains,
  automation, live fader/mute); ordering/PDC for taps and the gate hook are pre-wired.
- Shared touches: `graph.rs` (solo/mute rules only), recording from a tap
  (`ether-core/src/recording/mod.rs`, `ether-controller/src/recording/**`), mixer/inspector
  routing picker, sends, VCA assignment, "new bus from selection", Cmd+G, drag into/out of
  groups (`ui/src/features/mixer/**`, arrangement `trackDrag.ts`, `TrackRow.tsx`,
  `ArrangementView.tsx`, `arrangement.css`), the mock track reducer hook.

## `file-import` (priority 1)

Owns: `ui/src/features/import/**`, `crates/ether-controller/src/file_import/**`
(`read_path`), upload staging for the web OPFS store and the local wasm controller
(`ether-wasm/src/{store,bridge}.rs`, `apps/web/src/engine/**`, `ui/src/transport/wasm/**`),
its tests/e2e. Contract: CONTRACTS.md §12.13 (`MediaSource::Path`, the OS-file handoff).
Shared touches: `ether-native/src/store.rs` (`Library::read_external`), the desktop shell
(`apps/desktop/src-tauri/**`: dialog plugin + dropped paths; `ui/src/transport/tauri/**`),
drops on the browser panel and arrangement lanes and the "Import audio…" command
(`features/browser/index.tsx`, `features/remote/{uploadDrop,upload}.ts`,
`features/arrangement/{ArrangementView,TrackRow}.tsx`, `App.tsx`), the collab push of
external-path media (`collab/mod.rs`, those lines only), the mock import/upload paths.
The web/remote upload half can land before `media-references`; the desktop path half
depends on it for referencing in place (copy until then).

## `device-ui` (early)

Owns: `ui/src/features/devices/layout/**` (the renderer and every widget of the catalog),
its e2e. Shared touches: `DeviceView.tsx`, `DeviceChain.tsx`, `devices.css`, `descriptors.ts`
in `ui/src/features/devices/` (mount the renderer; generic layout fallback).

- Render `DeviceDescriptor::layout` (CONTRACTS.md §12.4.2) with kit components and tokens
  only; generic layout for devices without one. Data widgets read `useAnalysis(device)`
  from `fx-analysis` (`ui/src/features/devices/analysis/`; stub it until then) and the
  device kind (zones, sample). Depth rings/drop targets come from `racks-modulation`
  (`ui/src/features/modulation/`): leave a slot.
- The repo owner styles the renderer once; keep the structure token-driven.

## `synth-2`

Owns: `crates/ether-devices/src/poly_synth/**`, `crates/ether-devices/presets/poly-synth/**`,
`crates/ether-devices/tests/poly_synth*.rs`, `ui/src/transport/mock/devices/polySynth.json`.
No shared touches. `PolySynth` (59 params: 2 oscillators with VA shapes + wavetable
position, sub, noise, multimode filter + drive, amp/filter/mod envelopes, 2 LFOs, unison,
glide, voice modes). Follow the device agent guide.

## `multisampler`

Owns: `crates/ether-devices/src/multisampler/**`, `.../presets/multisampler/**`,
`crates/ether-controller/src/multisampler/**` (`Device::SetZones`), mock
`roadmap/multisampler.*` + `devices/multisampler.json`, its tests.
Shared touches: `handlers.rs` (rebuild/update multisamplers when zone media loads, next to
samplers), zone sources in `ether-native/src/bridge.rs` / `ether-wasm/src/bridge.rs`.
Zones: `ether_model::multisampler` (selection, round robin, loops); external media allowed.

## `fx-color`, `fx-modulation`, `fx-dynamics`

Own their group module (`fx_color`, `fx_modulation`, `fx_dynamics`), presets folders
(`saturator`, `bitcrusher`, `auto-filter` / `chorus`, `phaser`, `flanger`, `tremolo` /
`gate`, `multiband-compressor`, `transient-shaper`), tests and mock JSON. No shared
touches. Sidechain inputs (2 channels, keyed in `Node::process_sidechain`): `AutoFilter`
(envelope follower), `Gate`, `MultibandCompressor` (all bands).
`fx-dynamics` publishes gain reduction as `AnalysisKind::Levels` for layout meters.

## `fx-analysis`

Owns: `crates/ether-devices/src/fx_analysis/**`, presets `spectrum-analyzer`, `tuner`,
`ui/src/features/devices/analysis/**` (the UI side of the analysis channel: Watch/Unwatch
on visibility, `Event::Analysis` store/hook used by the renderer widgets), mock
`roadmap/analysis.*` (simulate frames) + `devices/fxAnalysis.json`, its tests.
The channel itself is implemented (engine, native bridge, controller). Shared touch (web):
forward `AnalysisFrame`s from the worklet to the Worker's bridge
(`ether-wasm/src/{worklet,proto,bridge}.rs`, that change only; merge dev after web-perf).

## `midi-fx`

Owns: `crates/ether-devices/src/midi_fx/**`, its presets folders,
`crates/ether-controller/src/midi_fx/**` (`check_chain_order`, scale data), tests, mock JSON.
Shared touch: `ether-controller/src/engine.rs` (push the resolved `MusicalScale` to Scale
Quantize nodes on creation and scale changes). Contract: CONTRACTS.md §12.4.4.

## `presets`

Owns: `crates/ether-controller/src/presets/**`, `crates/ether-devices/src/factory.rs` +
presets folders of the v0.1/v2 devices, `ui/src/features/presets/**`, mock
`roadmap/presets.*`, tests. Shared touches: the writable user library
(`Library::{write_file, remove_file, rename_file, user_root}` in `ether-native/src/store.rs`,
`ether-wasm/src/store.rs`, `ether-controller/src/memory.rs`), the preset menu in the device
header (`DeviceView.tsx`). Device nodes ship their own factory presets.

## `racks-modulation`

Owns: `crates/ether-core/src/{rack_chains,modulation}/**`, `crates/ether-devices/src/racks/**`
+ `modulators.rs`, `crates/ether-controller/src/racks/**`, `ui/src/features/{racks,
modulation}/**`, mock `roadmap/racksModulation.*` + `devices/{racks,modulators}.json`, tests.
- Engine: implement `ChainRacksRt::run` (+ `chain_latency`, `inherit`) and `ModulationRt`
  (sources, `intercept`/`render`/`pre_node`/`set_param`, `readback`). Routing of params,
  automation and latency to chain nodes is already wired.
- Envelope-follower sidechains (`Modulator::sidechain`, `Modulation::SetSidechain`):
  ordering, tap and the per-job gather are wired; implement `write_sidechain` alignment and
  the command (same validation as `sidechain::set_sidechain`).
- Controller: `rack_command`, `modulation_command`, `chain_racks_desc`, `modulation_desc`;
  cascades are done. Shared touches: `doc/devices.rs` (live modulator params),
  `plugins/**` (ignore echoes of modulated values), `DeviceView.tsx` (drop targets, rings).

## `comping`

Owns: `crates/ether-controller/src/comping/**` (`take_command`, `comp_clips`),
`ui/src/features/comping/**`, mock `roadmap/comping.*`, tests/e2e. Shared touches: take lanes
from loop/punch recording (`ether-controller/src/recording/**`, mock `liveRecord.*`), the
expandable lanes under tracks (arrangement `TrackRow.tsx`, `ArrangementView.tsx`,
`arrangement.css`, `layout*.ts`). Compile already skips lane clips and merges
`comp_clips`; cascades are done.

## `freeze-bounce`

Owns: `crates/ether-core/src/freeze.rs` (`render_frozen`), `crates/ether-controller/src/
freeze/**` (`freeze_command`, `freeze_tick`, `frozen_desc`, `check_editable`),
`ui/src/features/freeze/**`, mock `roadmap/freezeBounce.*`, tests. Shared touches:
`ether-controller/src/engine.rs` (don't instantiate frozen tracks' devices),
`ether-controller/src/export/**` (extract reusable offline-job helpers, additive), track and
clip context menus (`TrackRow.tsx`, `ClipView.tsx`).

## `time-edits`

Owns: `crates/ether-controller/src/time_edit/**`, `ui/src/features/time-edits/**`, mock
`roadmap/timeEdits.*`, tests. Shared touch: time-selection shortcuts and menu entries in
`ArrangementView.tsx`.

## `sample-accurate-automation`

Owns: `crates/ether-core/src/automation_rt.rs` (v0.1 behaviour moved verbatim),
`automation.rs`, `crates/ether-core/tests/sample_accurate*.rs`. Shared touches: `sched.rs`
(`Timing`), `tempo.rs`, `engine.rs` (sub-block timing lines only), `mixer.rs` (per-sample
ramps), `param.rs`, the plugin hosts' param-event offsets (`ether-clap/src/node.rs`,
`ether-vst3/src/node.rs`, `ether-au/src/mac/node.rs`, `ether-sandbox/src/{node,shm}.rs`),
`ether-devices/src/util.rs`. Acceptance: CONTRACTS.md §12.7 (block-size independence).

## `browser-v2`

Owns: `crates/ether-controller/src/browser/**`, `ui/src/features/browser/v2/**`, mock
`roadmap/browserV2.*`, tests. Shared touches: mount in `features/browser/index.tsx`, the
tempo-synced preview (`ether-core/src/preview.rs`, `ether-controller/src/media_preview/**`),
user folders (`ether-native/src/store.rs`; native picker in `apps/desktop/src/**`).

## `media-references`

Owns: `crates/ether-controller/src/media_refs/**`, `ui/src/features/media-refs/**`, mock
`roadmap/mediaReferences.*`, tests. Shared touches: import as reference and resolution in
`ether-controller/src/media/**`, `handlers.rs` (import), `project.rs` (missing check on
open), `ether-native/src/store.rs` (`Library::{external_path, read_external}`),
`collab/mod.rs` (media transfer by hash, those lines only), the mock import/library
(`MockTransport.ts`, `library.ts`). Behaviour change and migration: CONTRACTS.md §12.9.

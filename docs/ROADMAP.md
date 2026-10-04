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

## `collab-social` (base-62): chat, pinned notes, peer playheads, "hide users and notes"

Design and frozen contract: [COLLAB.md §12](COLLAB.md), CONTRACTS.md §11.17. base-62
landed the model entities (`ChatMessage`, `PinnedNote` in `ether_model::social`, tables
`Project::{chat, pinned_notes}`, shipped with `.ether` v4 and its migration), their
validation (text caps, author, position ranges), the chat `seq` assignment in
`Project::apply` and the **History exemption for chat** (implemented and tested in
`ether-model/tests/social.rs`), the commands `Chat::Send` and `PinnedNote::{Add, Edit,
Delete}`, `PresenceState::transport` (`PeerTransport`), `CollabEvent::ChatReceived`, and
stubs replying `Unsupported` (engine `ether-controller/src/social/mod.rs`, pinned in
`tests/social_prewire.rs`; mock `ui/src/transport/mock/roadmap/social.ts`). The owner's UX
is authoritative: reuse their sidebar (rail tab), dialog and context-menu patterns, kit
components and tokens.

Owns: `crates/ether-controller/src/social/**`, `crates/ether-controller/tests/social*.rs`,
`ui/src/features/collab/social/**` (new: chat panel, toasts host, notes overlay, playhead
overlay, hide-others selector), `ui/src/kit/Toast.tsx` + `ui/src/kit/toast.css` (new kit
component), `ui/src/transport/mock/roadmap/social.*`, `apps/web/e2e/collab-social.spec.ts`.

- Controller: `Chat::Send` (validate, author snapshot from the session with its own relay
  colour, `sent_at` from the host clock, `Insert { seq: 0 }` + overflow `Remove`s in one
  `edit_with` transaction; `InvalidState` outside a session), `PinnedNote::*` (undoable
  document edits like `MarkerCommand`, author from the session or `author_name`),
  `social_presence` (controller-owned `transport`: position, playing, loop region;
  refreshed every `PEER_TRANSPORT_REFRESH_MS` while playing and at once on
  play/stop/locate/loop/tempo changes; `None` while listening), `ChatReceived` for peers'
  live messages after the join catch-up, own colour learned from the relay.
- Relay: send each site its own stamped default presence once synced (so it learns its
  colour).
- UI:
  - **Chat section** in the left sidebar: a rail tab (`LeftTab` `"chat"`) shown only in a
    session; ordered list (`seq`), author colours from the snapshot, input with the 2000
    char cap. Hidden solo; messages still load with the project.
  - **Toasts** (top right) for `ChatReceived` while the chat section is closed; the kit has
    no toast, so a new kit component `Toast` (own path, tokens only, one export line in
    `kit/index.ts`).
  - **Shortcut** `Mod+Shift+M`: open the chat section and focus its input (Enter sends);
    a palette entry "Chat: Focus input".
  - **Notes overlays** wherever cursors are tracked: in the arrangement (the presence-v2
    `PresenceLayer` pattern and `coords.ts` mapping) and in the piano roll
    (`NotePosition::editor`, the `EditorPresence` mapping): "Leave a note" in the arranger
    and note-grid context menus, dots in the author's colour, expandable text,
    edit/resolve/"Discard note", drag to move (one gesture).
  - Optional: peers' playheads in the piano roll too, and follow-mode parity there.
  - **Peer playheads** overlay: one line + ruler cap per peer in its colour, distinct from
    ours, extrapolated with the replicated tempo map and wrapped in the loop (§12.3).
  - **"Hide users and notes"** toggle in the collab dialog: local setting (localStorage),
    never replicated; `useHideOthers()` honoured by every presence/notes renderer
    (pointers, editor pointers, selection outlines, the clip "peer editing" ring via
    `useClipEditors`, playheads, notes, the "Leave a note" entries).
- Shared touches: `ui/src/app/shell/{shellStore.ts, tabs.tsx, LeftRail.tsx}` (chat tab,
  visible in a session only), `ui/src/app/shell/commands.ts` (palette entry),
  `ui/src/kit/index.ts` (export line), `ui/src/features/arrangement/{ArrangementView.tsx,
  TrackRow.tsx, arrangement.css}` (mount the overlays, the "Leave a note" menu entry),
  `ui/src/features/piano-roll/{NoteGrid.tsx, PianoRoll.tsx, pianoRoll.css}` (the piano-roll
  notes overlay and menu entry), `ui/src/features/collab/presence/editors.ts` (the "peer
  editing" clip ring honours the hide toggle),
  `ui/src/features/collab/{store.ts, PresenceBar.tsx, PresenceBar.test.tsx, index.tsx,
  collab.css}` (`ChatReceived`, the hide toggle in the dialog, `PeerHighlights` honours it),
  `ui/src/features/collab/presence/{PresenceLayer.tsx, EditorPresence.tsx}` (honour hide
  only), `ui/src/transport/mock/roadmap/collab.*` (simulate a peer's chat and transport),
  `crates/ether-controller/src/collab/mod.rs` (dispatch lines only: own colour,
  `ChatReceived`, transport refresh in the tick), `crates/ether-controller/src/handlers.rs`
  (one hook after transport commands to publish the transport at once),
  `crates/ether-collab/src/relay/{mod.rs, tests.rs}` (own presence to self), `docs/COLLAB.md`.

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
1b. **Steps.** Integer or stepped params carry `ParamInfo::step` (plain step; 1 for enums,
   toggles, transposes in semitones, keys, voices, counts; `contract::{choice, toggle,
   stepped}` set it, continuous `param(..)` + `.with_step(1.0)` otherwise). Knobs, automation
   lanes, MIDI learn and the renderer snap to it; `tests/v02_descriptors.rs` fails when a
   semitone/count param ships without one.
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
5. **Layout.** Ship a `DeviceLayout` for every device in `descriptor().layout` (checked by
   `tests/layouts.rs`; `EqCurve` is for the EQ only for now: filters use `FilterCurve`,
   multiband crossovers `Crossover`)
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
- **Acceptance:** move `TrackDesc::{input_tap, vca}` and `RenderGraphDesc::vcas` out of the
  codec's JSON blob into the binary layout (`ether-core/src/codec.rs`, shared touch; bump
  `BinaryCodec::VERSION`, update the fixtures and proptests).
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
  cascades are done.
- **Acceptance:** move `TrackDesc::{chain_racks, modulation}` out of the codec's JSON blob
  into the binary layout (`ether-core/src/codec.rs`, shared touch; bump the version, update
  fixtures and proptests). Shared touches: `doc/devices.rs` (live modulator params),
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
the mock import/library (`MockTransport.ts`, `library.ts`). The collab push by hash belongs to
`file-import`. Behaviour change and migration: CONTRACTS.md §12.9.

## `plugin-sidechain` (priority 2)

Owns: `crates/ether-controller/tests/plugin_sidechain*.rs`, `crates/ether-native/tests/
plugin_sidechain*.rs`. Contract: CONTRACTS.md §12.14. Shared touches (format files): the
aux bus in `ether-clap/src/{node,plugin,scan}.rs`, `ether-vst3/src/{node,plugin,scan}.rs`,
`ether-au/src/mac/{node,plugin,mod}.rs` (`Node::{sidechain_inputs, process_sidechain}` +
`sidechain_inputs` in the descriptors), the sandbox shm layout and version (bump it by one;
serialized after `sample-accurate-automation`, which also changes the shm layout)
(`ether-sandbox/src/{shm,node,helper,host}.rs`), the plugin catalog
(`ether-plugin-host/src/**`, `PluginDescriptor::sidechain_inputs`). No controller or engine
changes are needed: `Device::SetSidechain` already accepts any device whose descriptor has
`sidechain_inputs > 0`, and the engine feeds `process_sidechain`.

## `graphical-eq` (priority 2, after `device-ui`)

Owns: `ui/src/features/devices/layout/eq/**` (the `EqCurve` widget, `eqResponse.ts`),
`crates/ether-devices/tests/eq_analysis*.rs`, its e2e. Contract: CONTRACTS.md §12.15.
Shared touches: the EQ's analysis producer (`ether-devices/src/eq.rs`: pre/post spectrum,
`Node::{has_analysis, analysis}`), widget registration in the shared renderer
(`layout/index.ts`, `layout/Widget.tsx`), `mock/devices/eq.json` (regenerated).
Tests: TS response parity with the Rust vectors, gesture = one undo step per drag, the EQ's
analysis under `assert_no_alloc`. Screenshots of the EQ panel in the dark theme.

## `turn-hardening` (follow-up to stream-host)

Owns: `crates/ether-collab/src/relay/ice/turn.rs` (+ `turn/**` if split),
`crates/ether-collab/tests/turn*.rs`; shared touch: `docs/COLLAB.md` §10 (TURN notice).
Fixes the known limitations listed in COLLAB.md §10 so TURN can stop being experimental:
- a **per-username** verified-request cap (a replayed valid request from spoofed sources must
  not push the global count to the hard budget and force rotations);
- count **real nonce-map inserts** (not admitted requests), so the soft budget reflects the
  crate's actual memory and `admitted` decays; new clients are not crowded out of the
  unverified trickle;
- deterministic **rotation-decision tests** (soft/hard budget, live allocations, replay).
Only when all three land, lift "Experimental: do not expose it publicly yet" (docs, `--help`,
startup warning) — decision logged for the owner.

## `tap-recording` (follow-up to groups-buses)

Record a take from a track-to-track input tap (resampling), native first. The engine's capture
(`RecordingRt::process`) runs before the track jobs and captures only hardware channels: add a
post-jobs capture hook in `engine.rs` pushing each armed tap consumer's aligned tap
(`InputTapRt::signal`, `bus_tap.rs`) as extra capture channels with the consumer's `in_lat`;
the native writer (`ether-native/src/recording/{mod,writer}.rs`) maps `TrackInput::Track`
tracks to those channels; loop/punch passes become take lanes like hardware takes (comping).
Web has no capture yet: disable arming a tapped track on web with a clear reason. RT rules apply
(no allocation on the audio thread; tests with assert_no_alloc).

# v0.3 (contracts-4)

`contracts-4` froze the v0.3 contracts ([CONTRACTS.md §13](CONTRACTS.md)) and pre-created
one module per node and layer, registered in its parent with one line, so the v0.3 nodes
run in parallel with disjoint files. Each node owns exactly its block in
[`.github/ownership.toml`](../.github/ownership.toml) ("v0.3"); the lists below summarize
them. Every pre-created module starts with a doc comment saying what goes there. Session
view stays out of scope (owner). Scripting / user devices and localization / full
screen-reader scope are T1 questions for the owner, not v0.3 nodes.

Ground rules (as for v0.2):
- Protocol and model *types* are frozen (changes via BCR). Model validation of the new
  entities is implemented (`ether-model/src/apply.rs`) and tested
  (`ether-model/tests/roadmap_v4.rs`); `.ether` is v5 (`V4ContractsV4Defaults`, neutral).
- Until a node lands its commands reply `Unsupported`, pinned by **one test per node** in
  `crates/ether-controller/tests/roadmap_v4.rs`: each node rewrites or deletes only its own
  test function. New devices are placeholders (`contract::Placeholder`: the reverb and the
  effect pass through, the instrument is silent). New desc fields compile empty and new
  engine hooks are stubs (`ether-core/tests/roadmap_v4_hooks.rs`).
- The MockTransport routes every new command to one file per node under
  `ui/src/transport/mock/roadmap/` (table in its `index.ts`), each pinned by its own
  `*.test.ts` (`expectUnsupported` in `testUtils.ts`); device descriptors are generated JSON
  in `ui/src/transport/mock/devices/{fxSpace,external}.json`
  (`UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v03_descriptors`).
- **Hot file.** contracts-4 did not touch `ether-core/src/engine.rs` (in-flight v0.2 nodes
  own it). The two nodes with engine hooks (`midi-expression`: `ExpressionRt`;
  `external-instrument`: `HwIoRt`) each add **one call site** there, as listed in their
  blocks; everything else lives in their own core module. Need another hook: BCR.
- The owner's UX passes on dev are authoritative (kit components, tokens, `midiTarget()` on
  new controls, existing context menus). Every PR with a UI-visible change includes
  screenshots (`.orchestra/pr-screenshot.sh <node-id> <file.png> "<caption>"`, embedded
  under `## Screenshots`).
- **Order.** Everything runs in parallel except `mpe`, which starts after `midi-expression`
  lands (same files: `ether-core/src/expression.rs`, the piano roll). `keymap` touches many
  key handlers with lookup lines only: it merges dev often and lands late in the wave.
  `ux-followups` and `web-latency` are small and can land any time. `release-0.3` is the
  human checkpoint after the wave.

## `audio-streaming`

Owns: `ether-controller/src/media_stream/**` (`should_stream`, `StreamSource`,
`STREAM_MIN_SECONDS` = 30 s, `STREAM_READ_AHEAD_SECONDS` = 4 s), `ether-media/src/stream/**`
(`ChunkDecoder`, lock-free `StreamCache`, `CHUNK_FRAMES`), `ether-native/src/disk_stream/**`
(one shared reader thread), `ether-wasm/src/media_stream.rs` (Worker reads OPFS, ships chunks
over the SAB ring), tests. Shared touches: the `media/` pipeline (call `should_stream`, keep
one decode pass for peaks + hash, then `EngineBridge::stream_media`), `handlers.rs`
(loop/locate hints), `NativeBridge`/`WasmBridge::stream_media` + worker/worklet wiring.
Acceptance (CONTRACTS.md §13.1): a 10-minute stereo file plays with bounded memory (cache
only) on native and web; warp/stretch, loops, locate and scrubbing don't underrun in the
common case (underruns counted and reported, never a block on the audio thread);
`assert_no_alloc` on the streaming `AudioSource::read`; offline renders (export, freeze,
bounce, audio-to-MIDI) still read the whole file synchronously and are bit-identical to
today.

## `midi-expression`

Owns: `ether-controller/src/expression/**` (every `Expression::*` command except
`SetTrackMpe`, the `track_expression` compile hook), `ether-core/src/expression.rs`
(`TrackExpressionDesc`, `ExpressionRt`), `ui/src/features/expression/**`, mock
`roadmap/expression.*`, tests. Shared touches: one `ExpressionRt` call site in the clip stage
of `engine.rs` (+ `prepare` on snapshot, `reset` on loop/locate); recording CC / bend /
channel and poly pressure into lanes and note expressions (`ether-controller/src/recording/**`,
`ether-native/src/recording/{midi,live}.rs`, mock `liveRecord.*`); copying expression with
notes (`doc/notes.rs` duplicate, `doc/clips.rs` split, `time_edit/**` paste, `freeze/**`
consolidate/flatten, `comping/**` flatten; the clip and note cascades and `DocCtx::copy_clip`
are done); the plugin hosts forward channel MIDI and poly pressure; lanes under the piano
roll (`ui/src/features/piano-roll/**`: lane picker, draw/edit curves, one undo step per
gesture). Optional: move `TrackDesc::expression` out of the codec JSON blob into the binary
layout (`codec.rs`, bump `VERSION`) if profiling asks for it.
Acceptance: record a CC/bend performance, see and edit it under the piano roll, play it back
sample-accurately (offline render identical with block sizes 64 and 512) to built-ins and
plugins; poly pressure plays to plugins as poly aftertouch.

## `mpe` (after `midi-expression`)

Owns: `ether-controller/src/mpe/**` (`Expression::SetTrackMpe`), `ui/src/features/mpe/**`,
mock `roadmap/mpe.*`, tests. Shared touches: per-note pitch/timbre and MPE output in
`ether-core/src/expression.rs` (+ `EventKind::NoteExpression` in `event.rs`, frozen
shape); MPE input while recording and monitoring (`recording/**`, native MIDI input); the
plugin hosts translate `NoteExpression` (CLAP `clap_event_note_expression`, VST3
`NoteExpressionValueEvent`, else MPE MIDI with `TrackExpressionDesc::mpe`; the sandbox shm
already encodes it); the Poly Synth responds to per-note pitch/pressure/timbre
(`poly_synth/**`); MPE settings in the inspector; per-note curves in the piano roll.
Acceptance: an MPE controller (or the mock's MPE input) records per-note pitch/pressure/
timbre, the curves are editable per note, the Poly Synth and a CLAP plugin with note
expressions play them, a plugin without them receives MPE MIDI.

## `capture-midi`

Owns: `ether-controller/src/capture/**` (`CaptureState`, `capture_input`, `capture_command`,
`capture_tick`), `ui/src/features/capture/**`, mock `roadmap/capture.*`, tests. Shared
touches: one `capture_input` line where MIDI input is drained (`midi_learn/mod.rs`), the
Capture button in the transport bar + palette entry, mock MIDI input simulation in
`MockTransport.ts`. Acceptance (CONTRACTS.md §13.4): play while stopped → Capture creates a
clip at the playhead with the inferred tempo (and loop) as one undo step; play while playing
→ notes keep their song positions; the buffer is bounded (`CAPTURE_MAX_SECONDS`,
`CAPTURE_MAX_EVENTS`), site-local and cleared on project change.

## `audio-to-midi`

Owns: `ether-controller/src/audio_to_midi/**` (job state, `audio_to_midi_tick`),
`ether-media/src/to_midi/**` (incremental `Detector`: melody, harmony, drums),
`ui/src/features/audio-to-midi/**`, mock `roadmap/audioToMidi.*`, tests. Shared touch: the
clip context menu entry ("Convert to MIDI…", `ClipView.tsx`). Acceptance (CONTRACTS.md
§13.5): synthetic test signals (sine melody, chords, kick/snare/hat loop) convert with the
expected notes (pitch exact, onsets within 20 ms); bounded work per tick (no stall), progress
events, cancel, one undo step at `Done` with ids from the command; warped clips land through
the warp map.

## `fx-space`

Owns: `ether-devices/src/fx_space/**` (Convolution Reverb, frozen param table,
`FACTORY_IRS`), presets `convolution-reverb`, `ether-controller/src/fx_space/**`
(`Device::{SetIr, ListFactoryIrs}`), an IR widget under `ui/src/features/devices/layout/ir/**`,
mock `roadmap/fxSpace.*` + `devices/fxSpace.json`, tests. Shared touches: reverbs rebuilt or
updated when IR media loads (`handlers.rs`, next to samplers), IR resolution in the bridges'
`create` / `update_builtin`, the widget registration in the shared renderer (BCR if a new
widget kind is needed). Follow the v0.2 device agent guide. Acceptance: partitioned
convolution with a zero-latency head (or the partition latency reported for PDC), IR swap
without a click (crossfade), `assert_no_alloc` in `process`, IR media referenced in place
(missing/relink like samples), a few factory IRs.

## `external-instrument`

Owns: `ether-devices/src/external/**` (External Instrument, External Audio Effect, frozen
param tables), `ether-controller/src/external/**` (`set_routing`, `external_command`,
`hw_io_descs`), `ether-core/src/hw_io.rs` (`HwIoDesc`, `HwIoRt`, `HwMidiEvent`),
`ui/src/features/external/**`, mock `roadmap/external.*` + `devices/external.json`, tests.
Shared touches: one `HwIoRt` call site per stage in `engine.rs` (`prepare`,
`gather_returns` before the jobs, `capture_send`, `write_sends` after master), optional
codec move of `TrackDesc::hw_io`, `EngineBridge::list_hardware_ports` + the hardware MIDI
out thread (`ether-native/src/{bridge,audio,rt}.rs`, `recording/midi.rs`), a routing widget
in the shared renderer. Acceptance (CONTRACTS.md §13.7): with a loopback (or the null audio
backend's test loopback) the effect's returned audio is aligned with the dry signal after
`MeasureLatency`; the instrument sends notes with sample-accurate timestamps; missing ports
keep the device silent and resume when they reappear; web replies `Unsupported` for ports and
measurement.

## `undo-history`

Owns: `ether-controller/src/undo_history/**`, `ui/src/features/undo-history/**`, mock
`roadmap/undoHistory.*`, tests. Shared touches: step ids and first-commit times in
`ether-model/src/history.rs` (additive accessors), `JumpTo` through the `Edit::{Undo,
Redo}` path (`handlers.rs`, per-site in a session via `collab/mod.rs`), the mock undo stack,
the History tab (left rail / palette). Acceptance (CONTRACTS.md §13.8): the list reads as
the edit timeline, jumping is equivalent to N undos/redos (one patch batch), checkpoints
named, in a collab session only own steps are listed and peers' later edits survive a jump.

## `templates`

Owns: `ether-controller/src/templates/**`, `ether-model/src/template.rs` (file format,
frozen fields), `ui/src/features/templates/**`, mock `roadmap/templates.*`, tests. Shared
touches: "Save as template" in the track context menu, "New from template" in the project
screen and palette, `NewProject` next to `Project::Create` (`project.rs`). Acceptance
(CONTRACTS.md §13.10): save tracks (with devices, racks, modulation, sends between them,
automation) and insert them elsewhere as one undo step with derived ids (collab replay
mints the same ids); "New project" uses the default project template; templates live in
the user library on native and web.

## `project-versions`

Owns: `ether-controller/src/versions/**`, `ui/src/features/versions/**`, mock
`roadmap/versions.*`, tests. Shared touches: session marker on open/close (`project.rs`),
`ProjectStore::remove` in the native, OPFS and memory stores, the recovery dialog at startup
and a versions entry in the project screen, the mock project store. Acceptance (CONTRACTS.md
§13.11): autosave versions roll (interval, pruning), restore/compare work on native and web
(OPFS), a killed session offers recovery on the next start and restores the newest version.

## `keymap`

Owns: `ether-controller/src/keymap/**` (storage), `ui/src/features/keymap/**` (action
registry, presets "Ethereal" and "Ableton-like", conflicts, the editor, the printable cheat
sheet), mock `roadmap/keymap.*`, tests. Shared touches: lookup lines in the key handlers
(`App.tsx`, `ArrangementView.tsx`, `arrangement/actions.ts`, `PianoRoll.tsx`,
`transport-bar/index.tsx`, `useArrangementTimeEdits.ts`, `comping/actions.ts`, the palette's
shortcut hints). Acceptance (CONTRACTS.md §13.12): every existing shortcut is an action in
the registry with its current chord in the "Ethereal" preset (no behaviour change by
default); rebinding shows conflicts; the keymap persists in the user library; the cheat sheet
prints.

## `web-latency`

Owns: `ether-wasm/src/latency.rs` (`LatencyReport`, `REPORT_INTERVAL_MS`), tests. Shared
touches: the report message in `proto.rs`, the worklet's periodic report (pre-sized, no
allocation) in `worklet.rs`, the Worker bridge's `node_latency` answering from it
(`bridge.rs`). Acceptance (CONTRACTS.md §13.13): a built-in whose latency changes on web
republishes PDC exactly like natively (`latency-republish` test ported to the wasm fake
pipeline).

## `ux-followups`

Small owner-visible fixes from the UX digest: hide the Devices section on VCA tracks; the
large-knob value text clipping; peer avatar contrast in the light theme; peer playheads in
the piano roll; the `Browser.test` focus flake; drop `EqCurvePlaceholder`. Owns only the
files listed in its block. Screenshots for each visible fix.

## `rack-presets`

Owns: `ether-controller/src/presets/**`, `ether-model/src/preset.rs` (format v2,
`Preset::rack: Option<PresetRack>`, frozen), factory rack presets, `ui/src/features/presets/**`,
mock `roadmap/presets.*`, tests. Shared touches: rebuilding chains on load through the racks
code (`ether-controller/src/racks/**`), factory rack preset registration. Contract
(CONTRACTS.md §13.9): saving a rack device stores its chains, chain devices, the rack's
modulators and the mappings inside it; `Preset::Load { seed }` replaces them as one undo step
with `derive_id(seed, i)` ids; v1 rack presets (macros and params only) load unchanged.
## Sharing: P2P host hub, invite links (base-115)

Design and frozen contract: [SHARING.md](SHARING.md) (T1, owner review), CONTRACTS.md §11.18.
base-115 landed:
- the protocol (`ether_protocol::share`), with `Command::Share` / `Event::Share` and
  `ProjectSummary::share`;
- the controller stub (`crates/ether-controller/src/share/`, every command but `Get` replies
  `Unsupported`, pinned in `tests/share_prewire.rs`; each node removes ONLY its own
  assertions);
- `ether-collab::share` (invite format and data-channel fragmentation implemented,
  `share.json` shape, and the seams `PeerLink`/`SignalLink`/`PeerEndpoint`/`ShareServices`);
- the `services/signal` skeleton;
- `MockShare`;
- the TS invite parser `ui/src/domain/invite.ts`.

The first six nodes run in parallel. Each builds against fakes (the signal adapter, the
in-memory `PeerEndpoint`, `MockShare`). `share-integration` joins them. The owner's UX on dev
is authoritative.

### `signal-service`

Owns `services/signal/**`, `scripts/release/web/functions/**`.
- `RoomCore` per SHARING.md §3.3:
  - claim (TOFU host token), `HostWelcome` with ICE servers (STUN from `STUN_URLS`, TURN
    credentials when secrets exist);
  - doors (`SetDoors`), `JoinHello` → `JoinWelcome` + `PeerArrived`, or `HostOffline` and a
    later welcome when the host connects;
  - signal routing with `peer` stamping, `EndPeer`, `PeerLeft`, `CloseRoom`, the TTL alarm;
  - every limit in `LIMITS` (per-socket token bucket, per-IP bad doors and claims, joiners per
    room, signals per pairing, hello timeout, offline wait).
- A Node `ws` adapter (`services/signal/test/server.ts`) running `RoomCore` for the e2e of
  other nodes. The Pages Function proxy (`functions/signal/[[path]].ts`, a DO binding).
- Acceptance:
  - unit tests for each transition and limit;
  - `wrangler dev` smoke steps in the README;
  - no deployment (owner).

### `p2p-transport`

Owns `crates/ether-collab/src/share/{native,web,signal}/**`, `ui/src/features/share/endpoint/**`;
shared touches listed in `.github/ownership.toml`.
- `SignalLink`: native over tungstenite with rustls (`wss://`), wasm over the Worker's
  `WebSocket`. `PeerEndpoint` native: one `ether-share` thread, one UDP socket, a str0m
  `Rtc` per pairing with one data channel (`DC_LABEL`), host + srflx candidates (the
  `ether-native/src/stream/stun.rs` approach), trickle ICE, fingerprints from the SDP.
  `PeerEndpoint` web: the share `MessagePort` between the UI and the controller Worker
  (transferable `ArrayBuffer`s), with the UI agent creating `RTCPeerConnection`s on
  `ShareEvent::PeerEndpoint`. Backpressure through `buffered()`. `default_services()` returns
  the real services.
- Acceptance:
  - two native endpoints pair over loopback through an in-memory `SignalLink` and exchange
    20 MiB of fragmented frames in order, with backpressure;
  - ICE failure → `PeerOutput::Failed` within 30 s;
  - web: vitest with a fake RTC, and a Playwright two-context data-channel echo through the
    port;
  - `just check-wasm` stays green.

### `share-engine`

Owns `crates/ether-controller/src/share/**`,
`crates/ether-collab/src/share/{hub,handshake,keys,fake}.rs`, tests `share*.rs`, the mock
`share.ts`; shared touches in `collab/mod.rs` (connector per session, `left_sites` seed,
sync signals), `relay/mod.rs` (`set_snapshot_source`, `set_color`), `handlers.rs` (view-only
refusal), `lib.rs`, `project.rs` (SaveAs/Duplicate skip `share.json`, resume/reconnect on
Open).
- The hub: drives `Relay` over `PeerLink`s plus a loopback, with the role filter of SHARING.md
  §2.3, pinned names and colours, and ICE servers from the signaling service.
- Key derivations and proofs (HMAC-SHA256) with **frozen test vectors** written into
  SHARING.md §4.2.
- Every `ShareCommand`, `ShareState`/`ShareNotice` and `share.json` per §4.5 and §7.
- Acceptance:
  - with the fake signal and in-memory peers, a host and 2 joiners converge (the collab
    property test runs through the hub);
  - listen role: refused edits, chat allowed;
  - reset link keeps members; remove; stop; role change;
  - host restart: new epoch, pending resent, members dedupe via `sites`;
  - joiner reconnect with backoff; `HostOffline` → back;
  - rejoin of an offline copy with offline work → "(local copy)";
  - `SaveAs`/`Duplicate` without `share.json`.

### `share-ui`

Owns `ui/src/features/share/**` (except `endpoint/`, `join/`); shared touches: top-bar slot
in `App.tsx`, `features/collab/{PresenceBar,index,store,collab.css}`, `features/audio-settings/**`,
`features/remote/**`.
- SHARING.md §8.1 (Share button / session pill), §8.2 (Share popover, host and joiner),
  §8.4 (view only, offline banner), §8.6 (Settings dialog with Audio | Sharing | Advanced
  tabs; Remote engine and the relay join form move to Advanced), §8.7 (toasts).
- Acceptance:
  - RTL tests against `MockShare` (`simulateJoin`, `simulateLeave`,
    `simulateHostOnline`);
  - exactly one session element in the top bar;
  - screenshots light and dark;
  - coordinate with base-114 if it already moved Remote engine.

### `join-flow`

Owns `ui/src/features/share/join/**`, `apps/web/src/join/**`, `apps/web/src/main.tsx`,
`apps/desktop/src-tauri/**`, `ui/src/domain/invite.*`.
- SHARING.md §5 and §8.3:
  - `tauri-plugin-deep-link` (`ethereal` scheme) and `tauri-plugin-single-instance`
    (`deep-link`), forwarding the URL to the webview (cold and warm start);
  - the web `/join/` landing before engine boot ("Open in the app" / "Continue in browser",
    remembered), stripping the key from the URL;
  - `JoinScreen` for every `JoinStage`;
  - "Join with a link…" in the popover and the palette.
- Acceptance:
  - Playwright `/join/...` → landing → Continue → Ready → Join (mock);
  - an invalid link shows the right message;
  - desktop deep link checked manually on the owner's laptop (listed in the PR).

### `recents-shared`

Owns `ui/src/features/project/**`, the stores (`ether-native`/`ether-wasm` `store.rs`,
`ether-controller/src/{memory,store}.rs`).
- `ProjectSummary.share` from `share.json` (role, host name, ≤ 8 participants, `active`,
  `last_synced_ms`; never keys). `SaveAs`/`Duplicate` never copy `share.json`, and deleting a
  project deletes it.
- Recents: badge, avatar stack and menu entries per SHARING.md §8.5.
- Acceptance: store tests with fixture files (native and wasm), RTL tests of the badges and
  menus.

### `share-integration` (after all of the above)

Owns `apps/web/e2e/share*.spec.ts`, `crates/ether-native/tests/share*.rs`, SHARING.md and
COLLAB.md updates.
- Two browser contexts through the Node signal adapter:
  1. share, copy the link, open it in the other context, Join;
  2. edits both ways, chat, listen;
  3. the host closes (the joiner keeps an offline copy) and reopens (the joiner
     reconnects);
  4. Stop sharing ends it.
- Native↔web on the devbox (loopback).
- The PR lists the owner's laptop checks (desktop deep link, macOS).

Later, optional: `share-handover` (SHARING.md §7.4), `offline-merge` (decision 9).

### `native-turn` (after the first wave)

A TURN client for the native share endpoint (SHARING.md §6.1, §11): relay candidates from
the advertised `turn:`/`turns:` servers (UDP, then TLS, then TCP), routed through the
allocation on the share thread, refreshed and released; "Hide my IP (relay only)" works on
desktop and `ether-server`. Tests: codec and state machine unit tests, native↔native
through the relay's TURN server with no host candidates (UDP, TLS, TCP), native↔browser
relay-only e2e (`p2p-turn.spec.ts`).

## `vst2` (owner request: "we need support for VST2")

Done: `crates/ether-vst2` hosts VST 2.4 plugins through `PluginFormatHost` like the other
formats (`PluginFormat::Vst2`), on its own ABI bindings written from the GPL clean-room
headers of FST and VeSTige (never the Steinberg SDK). Scanner pool, sandbox helper
(`--format vst2`), native host, plugin browser, Settings > Plugins folders and the device
header all list it. Details in [PLUGIN-FORMATS.md](PLUGIN-FORMATS.md) ("VST2").
Follow-ups, if wanted: Linux X11 editors (with VST3's), a sidechain convention for 4-input
VST2 effects, per-format dedupe of plugins shipped as VST2 + VST3, the plugin's display text
(`effGetParamDisplay`) in the generic device UI, and the Windows `VSTPluginsPath` registry
key as an extra default folder.

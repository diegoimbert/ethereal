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
`Project::{chat, pinned_notes}` with `#[serde(default)]`, no `.ether` bump), their
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

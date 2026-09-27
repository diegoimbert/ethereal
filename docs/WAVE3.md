# Wave 3 hook points

`alpha` pre-created one module per feature and layer, already registered with one line in
its parent, so `plugins`, `warp` and `recording` can run in parallel. Each node owns exactly
the files listed below (`.github/ownership.toml`). Each module starts with a doc comment
describing what goes there. Everything else, such as dispatch calls in shared files,
`Cargo.toml` deps, protocol enums (`crates/ether-protocol/**`, BCR only) and frozen model
types, needs a minimal edit coordinated through the manager (ORCHESTRATION.md §6.1).

## `plugins`

Owns: `ui/src/features/plugins/**`, `crates/ether-model/src/plugins.rs`,
`crates/ether-controller/src/plugins/mod.rs`, `crates/ether-native/src/plugins.rs`
(existing CLAP host: `PluginHost::instantiate`, editors, main thread),
`crates/ether-native/src/sandbox.rs` (new).

Registered: `pub mod plugins;` (ether-model `lib.rs`), `mod plugins;` (ether-controller
`lib.rs`), `pub mod plugins;` + `pub mod sandbox;` (ether-native `lib.rs`); the UI
`PluginBrowser` is the "plugins" sidebar tab in `ui/src/app/App.tsx`.

Plugs into:
- `EngineBridge::create_plugin` / `plugin_state` / `poll_plugins` (`crates/ether-controller/src/lib.rs`).
- `NativeBridge::create_plugin` in `crates/ether-native/src/bridge.rs`: when `plugin.sandboxed` is set, it
  currently logs a warning and loads the plugin in-process. Route this case to `sandbox.rs`, which uses
  `ether_sandbox::spawn` (`crates/ether-sandbox/src/lib.rs`).
- `PluginCommand` (`crates/ether-protocol/src/plugins.rs`): `SetSandboxed`/`Reload` are handled in
  `crates/ether-controller/src/handlers.rs`; `List`/`Rescan`/`OpenEditor`/`CloseEditor` are handled by
  the host in `crates/ether-native/src/host.rs` (`plugin_command`).
- Model: `PluginInstance { sandboxed, state }` in `crates/ether-model/src/device.rs` (frozen).

Shared touches (`ether-sandbox` is already a dependency of ether-native): the
`bridge.rs` sandboxed branch, the `handlers.rs` dispatch line.

## `warp`

Owns: `ui/src/features/warp/**`, `crates/ether-model/src/warp.rs` (existing, real model code:
`WarpSettings`, `WarpMarker`), `crates/ether-core/src/warp/mod.rs`,
`crates/ether-controller/src/warp/mod.rs`.

Registered: `mod warp;` (ether-core and ether-controller `lib.rs`); ether-model already has
`pub mod warp;`. The UI `WarpEditor` is the "warp" detail tab in `App.tsx`, opened when an
audio clip is double-clicked.

Plugs into:
- Core: `ether_core::graph::WarpDesc` (`ClipContentDesc::Audio { warp }`, `crates/ether-core/src/graph.rs`).
  `sched::render_audio` (`crates/ether-core/src/sched.rs`, called from `engine.rs`) plays warped clips
  by resampling. `WarpMode::Complex` falls back to the same path; this is where a per-clip
  `ether_stretch::Stretcher` (re-exported as `ether_core::Stretcher`, `crates/ether-stretch/src/lib.rs`)
  plugs in. Web: keep the unwarped/repitch fallback.
- Controller: `compile.rs::warp_desc` compiles markers into `WarpDesc`; `WarpCommand` is handled in
  `crates/ether-controller/src/doc/misc.rs::warp`.

Shared touches: one call in `sched.rs`/`engine.rs`, the compile/`doc/misc.rs` delegation lines.

## `recording`

Owns: `ui/src/features/recording/**`, `crates/ether-model/src/recording.rs`,
`crates/ether-core/src/recording/mod.rs`, `crates/ether-controller/src/recording/mod.rs`,
`crates/ether-native/src/recording/mod.rs`.

Registered: `pub mod recording;` (ether-model and ether-native `lib.rs`), `mod recording;`
(ether-core and ether-controller `lib.rs`); the UI `RecordingControls` component is mounted
in the `data-slot="recording"` slot of `App.tsx`.

Plugs into:
- Protocol: `RecordingCommand` and `RecordingEvent` (`crates/ether-protocol/src/recording.rs`).
- Controller: `handlers.rs::recording_command` (`Arm`, `SetRecording`). `ListInputs` currently
  returns `Unsupported`. `EtherController::armed` holds the armed tracks (runtime, not undoable).
  `compile.rs` resolves `TrackDesc.audio_input/monitor/armed` (`crates/ether-core/src/graph.rs`).
- Core: `Engine::process(inputs, ..)` already receives hardware input (`engine.rs`).
- Native: `crates/ether-native/src/audio.rs` opens only an output stream (`input_device: None`,
  `inputs: Vec::new()`). Add cpal input capture and `midir` MIDI input (`midir` is already in
  `[workspace.dependencies]`).

Shared touches (`midir` is already a dependency of ether-native): the `audio.rs`
stream setup, the `handlers.rs` dispatch line, and a host input trait on the controller (BCR).

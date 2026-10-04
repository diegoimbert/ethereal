# Plugin formats: CLAP, VST3, VST2, AU

All four formats are hosted end to end: scanning, the native host (in-process and
sandboxed), save/reopen and the plugin browser. The base came from the `formats-base` node.
`vst3` (`crates/ether-vst3/**`) and `au` (`crates/ether-au/**`) implemented the formats, and
`formats-integration` wired them into the app (see "Integration (native host)" below).
`vst2` (`crates/ether-vst2/**`) added VST2 later (see "VST2" below).

## Architecture

```
ether-core            PluginController / PluginNode / PluginError   (contracts, wasm-safe)
ether-plugin-host     PluginFormatHost trait, Formats registry, ScanRunner, bundle walker
  ├─ ether-clap       ClapFormat  (adapter over the existing CLAP host)
  ├─ ether-vst3       Vst3Format
  ├─ ether-vst2       Vst2Format  (clean-room ABI bindings, no Steinberg SDK)
  └─ ether-au         AuFormat    (macOS only; Unsupported elsewhere)
users: ether-plugin-scanner (protocol + --scan-all), ether-sandbox helper (--format),
       ether-native (formats-integration)
```

`PluginFormatHost` (`crates/ether-plugin-host/src/lib.rs`) has these methods:

| Method | Runs where | Loads plugin code? |
|---|---|---|
| `format()` | anywhere | no |
| `default_search_paths()` | host | no |
| `discover(paths)`: scan targets under paths | host | no |
| `discover_registry()`: AU components, default none | host | no |
| `claims(target)`: shape check, used to infer a request's format | anywhere | no |
| `scan(target) -> Vec<PluginDescriptor>` | **`ether-plugin-scanner` process only** | yes |
| `instantiate(path, plugin_id) -> Box<dyn PluginController>` | plugin main thread (host `MainThread` executor, or sandbox helper main) | yes |

The formats the `Formats` registry holds depend on who builds it:

- The scanner, the sandbox helper and the native host (`ether_native::plugins::formats`)
  all build a registry with all four formats.

Scanning stays out-of-process and crash-safe:

- One scan target per child process. A `ScanRequest` carries an optional `format`; without one, the scanner infers the format from the path.
- A timeout kills hung scans.
- The pipe drain is bounded (`ScanRunner::drain_timeout`), so a daemon left behind by a plugin can't hang a scan.
- Children run in a bounded pool (`ScanRunner::jobs`, base-129): `min(cores / 2, 8)`, at least 1, overridable with `ETHER_SCAN_JOBS`. Each child still scans one target with its own timeout. The report lists results in input order, whatever order the children finish in. Every format runs in parallel, AU included: each AU is loaded in its own scanner process, so there is no shared in-process registry state to lock. If a format ever needs serializing, give it its own one-job pass.
- Scans are incremental (`ScanCache`, `<data>/plugin-db/scan-cache.json`, next to `plugins.json`). Each target's result is cached, failures included, keyed by format + canonical path. It is validated by a fingerprint: the newest mtime, total size and entry count of the bundle tree. The whole cache is dropped when the crate version or the scanner binary changes. A rescan runs the scanner only on new or changed targets, and drops removed ones. `Plugin::Rescan { full: true }` ("Full rescan") ignores the cache and retries failures. If the scanner fails to start, that failure isn't cached. AU component ids have nothing on disk to fingerprint, so an updated AU keeps its old descriptor until a full rescan. New and removed AUs are still picked up.
- What is scanned (`PluginFolderSettings`, `<data>/plugin-db/folders.json`): the OS default folders when "System folders" is on (the default; it also covers the AU registry), plus the user's folders, each limited to one format or to any. Overlapping folders are walked once, deduplicated by canonical path. `Plugin::{ListFolders, AddFolder, RemoveFolder, SetIncludeDefaults}` edit these settings and start an incremental rescan. The UI is in Settings > Plugins.

The sandbox helper takes `--format <clap|vst3|vst2|au>` (default `clap`) through `SandboxOptions.format`. It loads through `Formats::instantiate`, so sandboxing works for every format as soon as its host does.

Other contract points:

- **`PluginController::set_param_value`** (defaulted to `Unsupported`) sets a param while the plugin is inactive. The sandbox helper relies on it.
- **`PluginError::Unsupported`** is the error every stub returns.

## Ids (`PluginInstance.plugin_id` / `PluginDescriptor.id`)

Documented on `ether_model::PluginFormat`. `.ether` tags are `"Clap"`, `"Vst3"`, `"Au"` and `"Vst2"`, and they are stable.

| Format | id | `PluginDescriptor.path` |
|---|---|---|
| CLAP | CLAP plugin id, reverse-DNS | `.clap` bundle |
| VST3 | class id in canonical `FUID::toString` form: 32 uppercase hex, words l1..l4 (the `CID` in `moduleinfo.json`). Same on every OS; convert with `ether_vst3::class_id_to_string` / `parse_class_id` (COM byte layout on Windows) | `.vst3` bundle |
| VST2 | `AEffect::uniqueID` as 8 uppercase hex digits of its 32-bit value (`'EtG2'` → `45744732`); shell sub-plugins use their own uniqueID. Convert with `ether_vst2::plugin_id` / `parse_plugin_id` | the `.dll` / `.so` / `.vst` (for a shell sub-plugin: the shell library) |
| AU | `type:subtype:manufacturer` four-char codes, e.g. `aufx:dely:appl`. Printable ASCII is kept verbatim; other bytes, `:` and `\` become `\xHH`. Use `ether_au::AuComponentId` | the id itself (AUs load from the component registry) |

**Decision: `PluginInstance` has no location field.** Documents store `(format, plugin_id)` only. The host resolves the bundle from its scanned catalog (`PluginCatalog`), so a project opens on any machine that has the plugin installed. Because nothing in the `.ether` format changed, no migration was needed. `crates/ether-model/tests/plugin_formats.rs` has a fixture test showing that a pre-VST3 file still loads, and that every format round-trips.

`DeviceSpec::Plugin` gained an optional `format`. When it is omitted, the format is CLAP.

## What each node must do

### `vst3` (`crates/ether-vst3/**` only)

- Add these deps to the crate's own `Cargo.toml` from `[workspace.dependencies]`:
  - `vst3` 0.3: COM bindings for the host and plugin side, pre-generated, no SDK needed. MIT OR Apache-2.0.
  - `libloading`.
  - On macOS: `objc2-core-foundation` (CFBundle for `bundleEntry`) and `objc2-app-kit` (editors).
- Implement the following:
  - **Module loading.** macOS `bundleEntry(CFBundleRef)`, Linux `ModuleEntry(handle)`, Windows `InitDll`, with the matching exit call on unload. The binary lives at `Contents/<arch>/…` inside the bundle.
  - **Scanning.** Read `IPluginFactory(2/3)` and keep the `kVstAudioEffectClass` classes. Map sub-categories to `features` and `category`: `Instrument` → Instrument, `Fx` → AudioEffect.
  - **Instantiation.**
    - Create the `IComponent` and the `IEditController`, either as a single component or as a separate controller, connected via `IConnectionPoint`.
    - Provide `IHostApplication` and `IComponentHandler`, the latter producing `ParamEdited` and `Gesture*` notifications.
    - Params: normalized↔plain via `IEditController::normalizedParamToPlain`, `ParamScale` as appropriate.
    - `IAudioProcessor` setup/activation goes in `PluginNode`: parameter changes as `IParameterChanges` (sample-accurate), notes as `IEventList`, and `ProcessContext` from `TransportInfo`. Keep this RT-safe: preallocate everything, no allocation in `process`.
    - State is the component state plus the controller state, both packed into one blob with your own versioned framing.
    - Editor: `IPlugView` in a floating window (reuse the approach in `ether-clap/src/gui.rs`).
- **Plain values.** This is what goes into `Device.params` (`.ether`), `ProcessEvent::Param` and automation. `IEditController::normalizedParamToPlain` is main-thread-only, so it can't be used on the audio thread. Define plain values without it:
  - Continuous params (`ParameterInfo.stepCount == 0`): plain = the normalized value, 0..1. Expose `ParamInfo { min: 0, max: 1, scale: Linear }`, with `default` = `defaultNormalizedValue`.
  - Discrete params (`stepCount = n > 0`): plain = the step index, 0..=n. Expose `ParamInfo { min: 0, max: n, steps: n, scale: Linear }`, with `default` = `round(defaultNormalizedValue * n)`.
  - Conversion on any thread: normalized = plain / n, and plain = round(normalized * n); continuous values pass through unchanged. The node converts to normalized when filling `IParameterChanges`; `ParamEdited` from `IComponentHandler::performEdit` converts back.
  - `ParamId` = `ParamID`, a u32 that VST3 already guarantees stable.
  - Display strings (`getParamStringByValue`) are a later, main-thread UI feature.
- **Test strategy.** Build a minimal test plugin in-test with the `vst3` crate's plugin side: a `ComWrapper` factory with a gain processor and one param. See the `vst3` crate's `examples/gain.rs`, and mirror `ether-clap/examples/ether_test_plugin.rs` + `testing.rs`: a cdylib example wrapped into a `.vst3` bundle. Cover:
  - scan through the real scanner binary;
  - instantiate, process (under `assert_no_alloc`), state round-trip and params;
  - the sandbox helper with `--format vst3`;
  - crash/hang fixtures, as with CLAP.

### `au` (`crates/ether-au/**` only)

- Add these macOS target deps from `[workspace.dependencies]` (all Zlib OR Apache-2.0 OR MIT; `block2` MIT):
  - `objc2-audio-toolbox`, `objc2-core-audio`, `objc2-core-audio-types`;
  - `objc2-avf-audio` (AUv3/`AUAudioUnit`), `objc2-core-foundation`, `block2`.
- Implement the following:
  - **`discover_registry`:** `AudioComponentFindNext` over `aufx`/`aumu`/`aumf`/`aumi`, returning one target per component id. Reading metadata doesn't instantiate anything.
  - **`scan(id)`:** name, manufacturer and version from the registry, and optionally a validation instantiation. This runs in the scanner process.
  - **`instantiate`:** `AudioComponentInstanceNew`, or `AUAudioUnit` for v3. Render via `AudioUnitRender` with a host-provided input callback, reading from pre-allocated buffers. Params via `AudioUnitParameter*`. State via `kAudioUnitProperty_ClassInfo` (plist → bytes). Editor via `kAudioUnitProperty_CocoaUI` or `AUAudioUnit.requestViewController` in a floating window. Everything off macOS stays `Unsupported`.
- **Param ids.** `AUParameterAddress` is 64-bit, but `ParamId` is `u32` and ids must be stable across sessions, because they are stored in `.ether` and automation.
  - AUv2: take `kAudioUnitScope_Global` params as-is when the id fits in u32, which it always does for v2 `AudioUnitParameterID`. Params in other scopes (Input/Output/Part/Group, per element) need a deterministic packed id, for example `scope << 24 | element << 16 | (id & 0xFFFF)` with a reserved high bit, or a table of every param you expose, recorded in the saved state. Document the choice in the crate.
  - AUv3 (`AUParameterTree`): addresses up to `u32::MAX` map directly. Larger addresses need a stable hash (e.g. FNV-1a of the address with collision probing in tree order), and the address↔id table goes into the state blob so reload is exact.
  - Plain values: AU params are already plain (`minValue`..`maxValue`, unit), so use `ParamScale` Linear, or Log for `kAudioUnitParameterFlag_DisplayLogarithmic`.
- **Asynchronous instantiation.** AUv3 components, and v2 components bridged out-of-process, instantiate asynchronously (`AudioComponentInstantiate` / `AUAudioUnit.instantiateWithComponentDescription:options:completionHandler:`). `instantiate` runs on the plugin main thread, so blocking it on a channel can deadlock when the completion is delivered on the main run loop. Instead, pump the run loop while waiting, both in the host's main thread and in the sandbox helper: `CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, true)` in a loop with a timeout. The same applies to `requestViewController`.
- **Test strategy.** Use Apple's built-in AUs, which every Mac has:
  - AUDelay `aufx:dely:appl`;
  - AULowpass `aufx:lpas:appl`;
  - DLSMusicDevice `aumu:dls :appl`.

  Cover:
  - registry discovery finds them;
  - scan through the scanner;
  - process a buffer (the delay/lowpass changes the signal), under `assert_no_alloc`;
  - state round-trip;
  - the sandbox helper with `--format au`.

  Gate the tests with `#[cfg(target_os = "macos")]`.

### `formats-integration`

The paths this node owns are in `.github/ownership.toml`.

1. **Native instantiation.** `ether-native/src/plugins.rs` + `bridge.rs`:
   - Make `Instantiate` format-aware: `Fn(PluginFormat, &Path, &str)`.
   - Build it from a `Formats` holding Clap + Vst3 + Au.
   - `create_plugin` looks the plugin up by `(plugin.format, plugin.plugin_id)` in the catalog: add `PluginCatalog::find_format`.
   - AU has no bundle: pass `desc.path`, which is the id.
2. **Sandbox.** `ether-native/src/sandbox.rs`: pass `SandboxOptions { format, .. }`.
3. **Rescan.** `host.rs` `spawn_scan` (one line): `ether_clap::find_bundles(..)` + `scan_all` become `formats.discover(None)` + `runner.scan_targets(..)`.
4. **UI.** `ui/src/features/plugins`:
   - send `format: plugin.format` in `DeviceSpec::Plugin`;
   - show a format badge and filter in the browser.
5. **End-to-end tests.** Add the VST3 fixture and AU built-ins to the native e2e: insert, save, reopen, sandbox toggle.

### `vst2` (`crates/ether-vst2/**`)

Owner request: "we need support for VST2. Add it. Use open source SDK implementations".

- **ABI.** `ether_vst2::abi` is a minimal set of hand-written bindings (`AEffect`,
  `audioMasterCallback`, effect/host opcodes, `VstEvents`/`VstMidiEvent`, `VstTimeInfo`,
  `ERect`, flags, plug categories) written from two GPL clean-room headers: FST `fst/fst.h`
  (GPL-3.0-or-later, IEM) and VeSTige `aeffectx.h` (GPL-2.0-or-later, LMMS/Ardour). The
  Steinberg VST2 SDK is neither used nor vendored. Layouts are unit-tested (`size_of`/offsets
  for 64- and 32-bit). Cited in the module header and `THIRD_PARTY_NOTICES.txt`. No new
  dependency (the MIT `vst` crate was the alternative; it is unmaintained and its host side
  would still need the same glue, so the ~300 lines of bindings were the better fit).
- **Shapes and folders.** Windows `.dll`, Linux `.so` (files; the walker never descends into
  `.vst3`/`.clap`/`.component`/`.lv2`/app bundles), macOS `.vst` bundles. Defaults, after
  `VST_PATH`: Windows `%ProgramFiles%\VSTPlugins`, `%ProgramFiles%\Steinberg\VSTPlugins`,
  `%CommonProgramFiles%\VST2`, `%CommonProgramFiles%\Steinberg\VST2`; macOS
  `/Library/Audio/Plug-Ins/VST`, `~/Library/Audio/Plug-Ins/VST`; Linux `~/.vst`,
  `/usr/lib/vst`, `/usr/local/lib/vst`. Entry points: `VSTPluginMain`, `main_macho`, `main`.
  A library without any of them scans as "no plugins" (VST folders often hold helper DLLs),
  not as a failure.
- **Scan** (scanner process, same pool, timeout, kill and cache rules): open the plugin,
  name (`effGetEffectName` → `effGetProductString` → file name), vendor, version
  (`effGetVendorVersion`), category (`effFlagsIsSynth` or synth/generator category →
  Instrument), features from `effGetPlugCategory`. **Shells** (`kPlugCategShell`, Waves-style)
  list every sub-plugin from `effShellGetNextPlugin`; each is opened through
  `audioMasterCurrentId` for its category and vendor, within a 12 s budget (past it, the rest
  are listed by name as effects so a huge shell stays inside the 20 s scan timeout).
- **Instance.** `Vst2Plugin` (main thread) owns the shared `AEffect` (`effClose` when the
  controller and node are both gone). `activate`: `effSetSampleRate`, `effSetBlockSize`,
  `effSetProcessPrecision`, `effMainsChanged(1)`, `effStartProcess`, latency =
  `initialDelay`. `Vst2Node` (audio thread, nothing allocated): `processReplacing` (f32), or
  `processDoubleReplacing` with f32↔f64 copies for 64-bit-only plugins, or the old
  accumulating `process`; MIDI (notes, raw MIDI, MPE as MIDI, all-notes-off) through
  `effProcessEvents` with `deltaFrames`; blocks split at param events (≥ 16-sample
  sub-blocks) since `setParameter` is untimed; `audioMasterGetTime` answered from the
  engine transport per sub-block; plugin MIDI out (`audioMasterProcessEvents`) becomes node
  output events. Main ins/outs are the first ≤ 2 plugin channels; extra inputs get silence,
  no sidechain.
- **Params.** `ParamId` = index, plain = the plugin's 0..1 value, default = value at open,
  all automatable (`effCanBeAutomated` is unimplemented in most plugins). Names from
  `effGetParamName`; `Vst2Plugin::param_text` gives `effGetParamDisplay` + `effGetParamLabel`
  (not in the generic UI yet). GUI `audioMasterAutomate`/`BeginEdit`/`EndEdit` →
  `ParamEdited`/gestures; host-initiated sets and state loads are not echoed.
  `set_param_value` works active or inactive (one object).
- **State.** `effGetChunk`/`effSetChunk` (bank) for `effFlagsProgramChunks`, else every
  param value; plus the current program. Framing in the crate docs (`EthVST2\0`, v1).
- **Host callbacks.** Per-instance state through `AEffect::resvd2` (reserved for hosts; JUCE
  does the same); audio-thread callbacks through a thread-local scope set by the node (no
  locks). `audioMasterIOChanged` re-reads latency/channels (`LatencyChanged`,
  `RestartRequested` if channels changed while active), `audioMasterUpdateDisplay` →
  `StateDirty` (+ `ParamsChanged` if names changed), `audioMasterSizeWindow` resizes the
  editor window.
- **Editor.** `effEditOpen` with the host window's `NSView`/`HWND` (the host-window code is a
  copy of `ether-vst3`'s, as VST3 copied CLAP's), `effEditIdle` from `poll`,
  `effEditGetRect` for the size (re-read after opening), `effEditClose` before the window
  goes. Not user-resizable (VST2 has no host → plugin resize). Linux: no editor (like VST3).
- **Tests.** `examples/ether_vst2_test_plugin.rs`, written with the crate's own bindings, is
  built twice: a gain effect (chunks, latency, f32 + f64) and a shell (64-bit-only gain + a
  synth with MIDI out). `tests/host.rs` covers scan (plain and shell), params and text,
  sample-accurate params and MIDI, MIDI out, transport, f64, automation, latency changes,
  state (chunk and params), editor-less, no allocation; the scanner (crash/hang/CLI), the
  sandbox helper and the native e2e (`formats_e2e.rs`: insert, save/reopen, sandbox toggle)
  run it too.
- **Same plugin as VST2 and VST3.** Both are listed, each with its format chip (browser,
  device header); no dedupe yet (owner decision).

## Integration (native host)

Done by the `formats-integration` node:

- **Loading by format.** `ether_native::plugins::Instantiate` is `Fn(PluginFormat, &Path, &str)`,
  built from the `Formats` registry (`instantiate_any`). `NativeBridge::create_plugin` looks
  the plugin up by `(format, plugin_id)` (`PluginCatalog::find_format`; ids are only unique
  per format) and passes `PluginDescriptor.path`, which is the component id for AUs.
- **Missing plugins.** A plugin that isn't in the catalog fails with
  `<FORMAT> plugin <name> (<id>) is not installed (rescan plugins)`. The controller reports it
  as an error notification and keeps the device in the document without an engine node, so
  the track plays the dry signal (bypassed). A rescan followed by a reload (or reopening the
  project) loads it with its saved state. The UI shows such a device as "missing · bypassed",
  checked against the scanned list by `(format, id)`.
- **Rescan.** `spawn_scan` scans `PluginFolderSettings::discover` (the default search paths
  and AU registry if enabled, plus the user folders) with `ScanRunner::scan_targets_cached`
  (parallel, incremental; see above). A rescan asked for during a scan runs right after it.
- **Sandbox.** `SandboxOptions.format` is passed to the helper (`--format`), so the per-plugin
  sandbox toggle works for every format. **AUv3 decision:** AUv3 extensions already run out of
  process (Apple's XPC bridge), but sandboxing one is *allowed*: the helper then hosts the
  `AUAudioUnit` proxy. It is harmless, keeps the toggle the same for every plugin, and still
  isolates in-process v2 units and our AU host code from the app. It costs the usual +1 block
  of latency.
- **UI.** The plugin browser shows a CLAP/VST3/AU badge per plugin and a format filter, and
  inserts with `DeviceSpec::Plugin { format }`. Crashed and missing states work the same for
  every format.
- **Tests.** `crates/ether-native/tests/formats_e2e.rs` scans (real scanner), inserts,
  processes, saves/reopens and toggles the sandbox for the CLAP and VST3 fixtures and Apple's
  AUDelay (macOS), plus the missing-plugin path.

### Re-entrancy (AU nested run loops)

AU `instantiate` (async units: v3, or v2 bridged out of process) and `open_editor`
(`requestViewController`) wait for their completion by running a **nested**
`CFRunLoopRunInMode` on the plugin main thread. In the desktop app that thread is the process
main thread, and its run loop also delivers Tauri's `run_on_main_thread` jobs. So other plugin
registry work (polls, state saves, nodes returning from the engine, other devices' calls) can
run *inside* those calls.

The native host therefore never holds a borrow of its plugin registry (a thread-local
`RefCell`) across a call into a plugin. Each call **checks out** the controller (its registry
slot is left empty), calls the plugin with no borrow held, and checks it back in:

- a nested call that needs the same controller sees it busy: `poll` skips it, other calls
  fail with an `Ipc` "busy" error;
- a node that returns from the engine meanwhile is parked on the instance and deactivated at
  check-in; a retired instance is dropped at check-in;
- `instantiate` builds, restores and activates the new controller with no borrow held, and
  only then inserts it (retiring any instance of the device created meanwhile).

Other notes from the format nodes that hosts must respect:

- `load_state` on an *active* controller leaves the live node's links stale. The host only
  restores state before `activate` (a state change re-instantiates the device).
- `set_param_value` on an active VST3 (like CLAP) returns a `State` error. While active, params
  go to the node as `ProcessEvent::Param`.
- The first `save_state` of a VST3 after setting params while inactive briefly activates it.
- VST3 bundle paths on Linux/Windows have no local build target; release CI is their first
  compile.

## Licenses

Everything is GPL-3.0-or-later compatible:

| Dependency | License |
|---|---|
| VST2 ABI (`ether-vst2`, own code) | GPL-3.0-or-later; written from FST (GPL-3.0-or-later) and VeSTige (GPL-2.0-or-later) headers, no SDK |
| `vst3` crate | MIT OR Apache-2.0. Its bindings derive from the VST3 SDK headers, which Steinberg has published under MIT since SDK 3.8. |
| objc2 family | Zlib OR Apache-2.0 OR MIT |
| `block2` | MIT |
| `libloading` | ISC |

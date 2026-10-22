# Plugin formats: CLAP, VST3, AU, WebCLAP

CLAP, VST3 and AU are hosted end to end: scanning, the native host (in-process and
sandboxed), save/reopen and the plugin browser. `crates/ether-vst3` and `crates/ether-au`
implement the formats, and the native host wires them into the app (see "Integration (native host)" below). VST2 is
**not in this build** (it lives in the private repository; see "`vst2`" below).

## Architecture

```
ether-core            PluginController / PluginNode / PluginError   (contracts, wasm-safe)
ether-plugin-host     PluginFormatHost trait, Formats registry, ScanRunner, bundle walker
  ├─ ether-clap       ClapFormat  (adapter over the existing CLAP host)
  ├─ ether-vst3       Vst3Format
  └─ ether-au         AuFormat    (macOS only; Unsupported elsewhere)
users: ether-plugin-scanner (protocol + --scan-all), ether-sandbox helper (--format),
       ether-native
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
- Children run in a bounded pool (`ScanRunner::jobs`): `min(cores / 2, 8)`, at least 1, overridable with `ETHER_SCAN_JOBS`. Each child still scans one target with its own timeout. The report lists results in input order, whatever order the children finish in. Every format runs in parallel, AU included: each AU is loaded in its own scanner process, so there is no shared in-process registry state to lock. If a format ever needs serializing, give it its own one-job pass.
- Scans are incremental (`ScanCache`, `<data>/plugin-db/scan-cache.json`, next to `plugins.json`). Each target's result is cached, failures included, keyed by format + canonical path. It is validated by a fingerprint: the newest mtime, total size and entry count of the bundle tree. The whole cache is dropped when the crate version or the scanner binary changes. A rescan runs the scanner only on new or changed targets, and drops removed ones. `Plugin::Rescan { full: true }` ("Full rescan") ignores the cache and retries failures. A timed-out target is cached (so it stays listed as failed) but every rescan tries it again, since a slow plugin may just have lost a race with a busy machine. If the scanner fails to start, that failure isn't cached.
- Param display text: `Plugin::ParamTexts { device, values }` (host-handled) replies the plugin's own text for each plain value (`PluginController::param_text`: CLAP `value_to_text`; VST3 `getParamStringByValue` plus the param's `units`; AU none, its params carry real units), from the live instance, else the GUI mirror, through the sandbox too. The generic device UI's knobs (`ParamControl`, `features/devices/pluginParamText.ts`) show it: batched per device, throttled while dragging, cached per value, asked again once when missing, and the UI's own formatting when there is none (web, AU).
- Failed targets: `Plugin::ListScanFailures` lists them (path, format, reason), known after a restart from the scan cache. `Plugin::RetryScan { path, format, timeout_ms }` scans one again in its own child, off the controller thread (90 s by default, clamped to 1 s to 10 min); it replies `ScanRetried` when done, records the result in the scan cache and `plugins.json`, and emits `ScanFinished` with the updated list. The UI is Settings > Plugins > "Failed plugins": right-click a row, then Retry. AU component ids have nothing on disk to fingerprint, so an updated AU keeps its old descriptor until a full rescan. New and removed AUs are still picked up.
- What is scanned (`PluginFolderSettings`, `<data>/plugin-db/folders.json`): the OS default folders when "System folders" is on (the default; it also covers the AU registry), plus the user's folders, each limited to one format or to any. Overlapping folders are walked once, deduplicated by canonical path. `Plugin::{ListFolders, AddFolder, RemoveFolder, SetIncludeDefaults}` edit these settings and start an incremental rescan. The UI is in Settings > Plugins.

The sandbox helper takes `--format <clap|vst3|au>` (default `clap`) through `SandboxOptions.format`. It loads through `Formats::instantiate`, so sandboxing works for every format as soon as its host does.

Platform limits: the sandbox helper runs on macOS and Linux only (`ether_native::sandbox::spawn` returns an error on Windows, where plugins always run in-process). CLAP editors use the plugin's own floating window everywhere, and a host-made window (for plugins that only embed) only on macOS (`ether-clap/src/gui.rs`, `HostWindow::SUPPORTED`): elsewhere an embed-only CLAP plugin has no editor. VST3 editors open on macOS, Windows and Linux (X11).

Other contract points:

- **`PluginController::set_param_value`** (defaulted to `Unsupported`) sets a param while the plugin is inactive. The sandbox helper relies on it.
- **`PluginError::Unsupported`** is the error every stub returns.

## Ids (`PluginInstance.plugin_id` / `PluginDescriptor.id`)

Documented on `ether_model::PluginFormat`. `.ether` tags are `"Clap"`, `"Vst3"`, `"Au"`, `"Vst2"` and `"Webclap"`, and they are stable.

| Format | id | `PluginDescriptor.path` |
|---|---|---|
| CLAP | CLAP plugin id, reverse-DNS | `.clap` bundle |
| VST3 | class id in canonical `FUID::toString` form: 32 uppercase hex, words l1..l4 (the `CID` in `moduleinfo.json`). Same on every OS; convert with `ether_vst3::class_id_to_string` / `parse_class_id` (COM byte layout on Windows) | `.vst3` bundle |
| VST2 (model only) | `AEffect::uniqueID` as 8 uppercase hex digits of its 32-bit value (a shell sub-plugin has its own). The tag is kept so a project that references one still parses; the host is not in this build | — |
| AU | `type:subtype:manufacturer` four-char codes, e.g. `aufx:dely:appl`. Printable ASCII is kept verbatim; other bytes, `:` and `\` become `\xHH`. Use `ether_au::AuComponentId` | the id itself (AUs load from the component registry) |
| WebCLAP (web version only) | CLAP plugin id, reverse-DNS | `webclap:<module id>` (the installed module's content hash). Native hosts refuse it: `WebCLAP plugin <name> (<id>) is not available on this platform (web version only)` |

**Decision: `PluginInstance` has no location field.** Documents store `(format, plugin_id)` only. The host resolves the bundle from its scanned catalog (`PluginCatalog`), so a project opens on any machine that has the plugin installed. Because nothing in the `.ether` format changed, no migration was needed. `crates/ether-model/tests/it/plugin_formats.rs` has a fixture test showing that a pre-VST3 file still loads, and that every format round-trips.

`DeviceSpec::Plugin` gained an optional `format`. When it is omitted, the format is CLAP.

## What each format implements

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
    - Editor: `IPlugView` in a floating window (reuse the approach in `ether-clap/src/gui.rs`). Linux: an X11 host window (`ether_plugin_host::x11_window`, Xlib loaded at runtime, own connection, transient for the app's window) whose id is the `X11EmbedWindowID` parent; the `IPlugFrame` also implements `Linux::IRunLoop` (the plugin's X connection fd and timers, served from the editor poll: `poll(2)` with no timeout, then due timers). Without an X display, no editor.
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

### Integration

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

### `vst2` (not in the open-source build)

VST2 hosting lives in the private repository (`ethereal-private`, registered with
`ether_plugin_host::set_extra_formats`): the open-source build does not
register the format, so loading a VST2 plugin is `Unsupported`. `PluginFormat::Vst2` stays in
the model so a project that references one still parses.

## Integration (native host)

How the native host uses the formats:

- **Loading by format.** `ether_native::plugins::Instantiate` is `Fn(PluginFormat, &Path, &str)`,
  built from the `Formats` registry (`instantiate_any`). `NativeBridge::create_plugin` looks
  the plugin up by `(format, plugin_id)`, then the name (for the rare plugins whose id
  collides)
  (`PluginCatalog::find_plugin`; ids are only unique per format) and passes
  `PluginDescriptor.path`, which is the component id for AUs.
- **Missing plugins.** A plugin that isn't in the catalog fails with
  `<FORMAT> plugin <name> (<id>) is not installed (rescan plugins)`. The controller reports it
  as an error notification and keeps the device in the document without an engine node, so
  the track plays the dry signal (bypassed). A rescan followed by a reload (or reopening the
  project) loads it with its saved state. The UI shows such a device as "missing" (and bypassed),
  checked by `(format, id)` against the plugins of the computer whose engine plays the project
  (`pluginStore` `engineCatalog`): this computer's scan, or, while another computer's engine
  plays it, that computer's list (`setRemote`; unknown, so nothing is shown missing, until it
  comes: `setRemotePending`). Device headers, the missing count, the resolver and the load
  notice all read it, so a plugin only missing on this computer is never shown missing.
- **Rescan.** `spawn_scan` scans `PluginFolderSettings::discover` (the default search paths
  and AU registry if enabled, plus the user folders) with `ScanRunner::scan_targets_cached`
  (parallel, incremental; see above). A rescan asked for during a scan runs right after it.
- **Sandbox.** `SandboxOptions.format` is passed to the helper (`--format`), so the per-plugin
  sandbox toggle works for every format. **AUv3 decision:** AUv3 extensions already run out of
  process (Apple's XPC bridge), but sandboxing one is *allowed*: the helper then hosts the
  `AUAudioUnit` proxy. It is harmless, keeps the toggle the same for every plugin, and still
  isolates in-process v2 units and our AU host code from the app. It costs the usual +1 block
  of latency.
- **UI.** The plugin browser shows a format chip per plugin and the format toggles (see
  "Same plugin in several formats" above), and inserts with `DeviceSpec::Plugin { format }`. Crashed and missing states work the same for
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

Other notes from the format hosts that callers must respect:

- `load_state` on an *active* controller leaves the live node's links stale. The host only
  restores state before `activate` (a state change re-instantiates the device).
- `set_param_value` on an active VST3 (like CLAP) returns a `State` error. While active, params
  go to the node as `ProcessEvent::Param`.
- The first `save_state` of a VST3 after setting params while inactive briefly activates it.
- VST3 bundle paths on Linux/Windows have no local build target; release CI is their first
  compile.

## WebCLAP (web version)

[WebCLAP](https://github.com/WebCLAP) (WCLAP) is the CLAP ABI compiled to WebAssembly: a
module exports `clap_entry`, a growable function table and `malloc` (or `cabi_realloc`),
and imports or exports its memory. The web version hosts these plugins; the desktop app
keeps its native formats and shows a WebCLAP device as "web version only" (bypassed, see
"Ids" for the error). Contract summary: CONTRACTS.md §13.21.

### Installing

Settings › plugins (web) installs from an `https` address (fetched without credentials;
the server must send CORS headers) or a file, either a self-contained module (`.wasm`,
`.wclap.wasm`) or a bundle (`.wclap.tar.gz`: `module.wasm` plus resources, e.g. a web UI).
`apps/web/src/engine/webclap/manager.ts`:

- The module is compiled on the page and scanned in a Worker of its own (`scan.worker.ts`,
  killed after 20 s): each plugin of the factory is instantiated once and its descriptor,
  params (labels of stepped ranges), audio and note ports and web UI presence are read.
  Plugins that fail are listed as such.
- The file, its scan and its content hash (the module id) go to IndexedDB
  (`ether-webclap`). At every engine start they are read back, compiled and sent to both
  engine realms; the scans become the controller's catalog
  (`crates/ether-wasm/src/webclap/catalog.rs`, mapped like `ether-clap/src/params.rs`).
- Removing a module removes its plugins from the catalog; projects that use them show them
  missing (bypassed) until it is installed again.

The device library lists the installed plugins (format `webclap`) next to the built-ins,
and inserts them like any plugin (`DeviceSpec::Plugin { format: Webclap }`).

### Hosting

The engine runs in the AudioWorklet (wasm instance #2, `crates/ether-wasm`). Each plugin
instance is **its own WebAssembly instance with its own memory**, next to it in the
Worklet, driven by the JS host glue (`apps/web/src/engine/webclap/`):

- `instance.ts` implements the host side of the CLAP ABI for wasm32 (`abi.ts` has the
  struct layouts): host structures and buffers live in the plugin's memory (reserved through
  its `malloc`); host callbacks are JS functions re-exported by a small generated module
  (`wasm.ts` `trampolines`) and added to the plugin's function table.
- Host extensions: `clap.params` (rescan, clear, request_flush), `clap.state` (mark_dirty),
  `clap.latency` (changed → restart), `clap.tail`, `clap.log`, `clap.thread-check`,
  `clap.audio-ports`, `clap.note-ports` (CLAP and MIDI dialects), `clap.gui` (webview
  sizing calls answered "no"), `clap.webview/3`. The plugin's `clap.params`, `clap.state`,
  `clap.audio-ports`, `clap.note-ports`, `clap.latency`, `clap.tail`, `clap.gui` and
  `clap.webview/3` are used.
- Threads: the Worklet is one thread. Audio-thread calls (`process`, `start_processing`,
  `reset`, `flush` while active) run in an "audio" phase, the rest between blocks in a
  "main" phase; `clap.thread-check` answers by phase. Plugins that start threads
  (wasi-threads) are not supported.
- `wasi.ts`: `wasi_snapshot_preview1` with stdout/stderr to the log, clocks and random only:
  no preopened directories (opening a file fails), no environment, no sockets.
- Each block (`WebclapNode`, `crates/ether-wasm/src/webclap/node.rs`): the node writes an
  `Exchange` (the engine's channel pointers, its events, the transport) and calls the host,
  which copies the main input (a mono input feeds every channel) and the sidechain into the
  plugin's ports, writes CLAP events (sample-accurate: param values in plain units, notes,
  choke, wildcard note-off + CC 123 for all-notes-off, MIDI, note expressions when the first
  note port takes the CLAP dialect, else pressure as poly aftertouch) and the transport, calls
  `process`, and copies the main output back. Nothing is allocated per block (views are
  re-made only when a memory grows; copies are loops).
- The plugin's output events: param values become `ParamEdited` (undoable edits, automation
  recording) with their gestures; notes and MIDI go to the next device (note effects).
- Sleep on silence as native CLAP (§13.19): `SLEEP` sleeps, `CONTINUE_IF_NOT_QUIET` once
  quiet, `request_process` and `request_flush` wake it; "always process" applies.
- Latency: read at activation; `latency.changed` restarts the plugin between blocks
  (deactivate, activate) and the engine republishes PDC from the Worklet's latency reports.
- State: `Plugin` state blobs are the plugin's own `clap.state` bytes. The controller asks
  the Worklet (`WebBridge::plugin_state`, a blocking round trip of a block or two; the last
  state known when the audio isn't running); a new instance loads it before activation.
- Params, automation, modulation, macros, MIDI learn: the same paths as built-ins
  (`ParamTarget::Node` events), descriptors from the catalog; values are mirrored after
  instantiation (`plugin_param_values`).
- Offline renders (export, freeze): the controller Worker has its own host and modules and
  creates fresh instances there (`create_offline_plugin`).
- A trap (or `proc_exit`) faults the instance: silent output, "crashed" in the device header,
  reload to re-create it.

### Web UIs

A plugin with `clap.webview/3` gets its UI in its device window (others show the generic
parameter panel). The page asks the Worklet for `get_uri`; `file:` URIs (under the path the
plugin got in `clap_entry.init`, `/plugin`) are read from the bundle, absolute paths from
the bundle or `get_resource`, `data:text/html` is decoded; remote (`http(s)`) UIs are not
supported. The page collects the start page and the files it references (`uiPage.ts`, found
textually: the page never parses the plugin's HTML as a document) and sends them to the
frame, which inlines them (scripts and styles as elements, images as `data:` URLs) and
writes the result as its document. Messages follow the webview extension: the UI posts `ArrayBuffer`s
to `window.parent`, the page forwards them to the plugin (`receive`, main phase), and the
plugin's `send` reaches the UI the other way. The UI talks to its plugin only; param changes
it makes come back as the plugin's output events (above).

### Security

Plugins are untrusted code. What they can reach:

- **Plugin code (wasm)** runs in the AudioWorklet (and, for offline renders, the controller
  Worker), but a WebAssembly instance can only call what it imports: the CLAP host callbacks
  and the WASI functions above. It has no access to JS globals, the DOM, storage, network or
  the engine's memory (its memory is its own; the host copies samples and events in and out
  and never hands it a pointer outside it). The AudioWorklet scope itself has no network,
  storage or DOM APIs.
- **Denial of service**: a plugin that loops forever in `process` stalls the audio thread
  (the audio stops until the page is reloaded and the device removed or the project opened
  in safe mode). The install scan runs in
  a Worker that is killed after 20 s. Memory: an imported memory gets the plugin's declared
  maximum (2 GiB without one), its own memory is separate from the engine's.
- **Plugin UIs (HTML/JS)** run in `public/webclap-ui.html`, an `<iframe sandbox="allow-scripts">`:
  an opaque origin, so no access to the app's DOM, `localStorage`, IndexedDB (the installed
  plugins, the projects in OPFS) or cookies, no popups, no top navigation, no forms. The
  frame's own Content-Security-Policy (`scripts/release/web/_headers`, rule
  `/webclap-ui.html`) allows inline scripts, styles and `data:`/`blob:` media, and nothing
  else: no network (`connect-src 'none'`), no frames, no workers, only embedded by the app
  (`frame-ancestors 'self'`). The app's own policy gained only `frame-src 'self'` (to embed
  it). The page sends the frame only the plugin's own messages, and accepts from it only
  `ArrayBuffer`s, forwarded to that plugin. A host that serves the app without the
  frame's policy (DEPLOY.txt) fails closed: the app's policy blocks the UI's inline scripts.
- **Installing** fetches without credentials or referrer; the module's bytes are compiled
  under the app's existing `'wasm-unsafe-eval'`. No third-party JavaScript runs on the page
  or in the Worklet: a plugin's JavaScript only exists inside its UI frame.
- Not covered: a plugin can still process (or ruin) the audio it is given, and spin the
  CPU. Users are told to install the plugins they trust.

### Limits

No threads (wasi-threads), no file system for plugins, no remote UIs, CSS `url()` references
of UIs are not inlined, 64-bit modules (`wasm64`) are refused, presets (`clap.preset-load`)
and remote controls aren't read, param display texts use the UI's own formatting
(`Plugin::ParamTexts` is not answered on the web), and sandboxing (`PluginInstance.sandboxed`)
has no meaning on the web (every instance is isolated already).

### Tests

- `crates/ether-wasm/tests/it/webclap.rs`: the controller and the Worklet's engine over the
  rings with a fake guest: processing and params, state through the Worklet and reopening,
  plugin edits becoming document edits (undoable), PDC, sleep, missing and native plugins,
  offline instances.
- `apps/web/src/engine/webclap/host.test.ts`: the JS host against real wasm plugins: the
  repository's test plugin (`crates/ether-webclap-fixture`: a gain with a web UI, a delay, a
  synth; built by `apps/web/scripts/build-webclap-fixture.mjs`), Clack's gain and Signalsmith
  Basics (a bundle with shared memory and web UIs; `apps/web/test-fixtures/webclap/examples`,
  MIT / Apache-2.0).
- `apps/web/e2e/webclap.spec.ts`: install from a file and an address, play through the
  real engine, edit from the sandboxed UI, offline render with automation, reopen.

## Licenses

Every plugin-format dependency is permissive. The full dependency tree and the license texts
that ship in the releases are in [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md)
(regenerated by `just notices`, gated by `deny.toml`); the only dependency that is not
permissive in the strict sense is `symphonia` (MPL-2.0, file-level copyleft).

| Dependency | License |
|---|---|
| `vst3` crate | MIT OR Apache-2.0. Its bindings derive from the VST3 SDK headers, which Steinberg has published under MIT since SDK 3.8. |
| objc2 family | Zlib OR Apache-2.0 OR MIT |
| `block2` | MIT |
| `libloading` | ISC |

# Plugin formats: CLAP, VST3, AU

The base is in place (the `formats-base` node). Three nodes build on it:

- `vst3` (`crates/ether-vst3/**`) implements VST3.
- `au` (`crates/ether-au/**`) implements AU.
- `formats-integration` wires both into the native host once they land.

`vst3` and `au` touch no shared files, so they can run in parallel.

## Architecture

```
ether-core            PluginController / PluginNode / PluginError   (contracts, wasm-safe)
ether-plugin-host     PluginFormatHost trait, Formats registry, ScanRunner, bundle walker
  ├─ ether-clap       ClapFormat  (adapter over the existing CLAP host)
  ├─ ether-vst3       Vst3Format  (stub: discovery + ids real, scan/load Unsupported)
  └─ ether-au         AuFormat    (stub: ids real, registry/scan/load Unsupported; macOS only)
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

- The scanner and the sandbox helper build a registry with all three formats.
- The native host still uses CLAP only (`ether_clap::instantiate`, `ether_clap::find_bundles`). This keeps users with VST3s installed from seeing stub failures until the implementations land.

Scanning stays out-of-process and crash-safe:

- One scan target per child process. A `ScanRequest` carries an optional `format`; without one, the scanner infers the format from the path.
- A timeout kills hung scans.
- The pipe drain is bounded (`ScanRunner::drain_timeout`), so a daemon left behind by a plugin can't hang a scan.

The sandbox helper takes `--format <clap|vst3|au>` (default `clap`) through `SandboxOptions.format`. It loads through `Formats::instantiate`, so sandboxing works for every format as soon as its host does.

Other contract points:

- **`PluginController::set_param_value`** (defaulted to `Unsupported`) sets a param while the plugin is inactive. The sandbox helper relies on it.
- **`PluginError::Unsupported`** is the error every stub returns.

## Ids (`PluginInstance.plugin_id` / `PluginDescriptor.id`)

Documented on `ether_model::PluginFormat`. `.ether` tags are `"Clap"`, `"Vst3"` and `"Au"`, and they are stable.

| Format | id | `PluginDescriptor.path` |
|---|---|---|
| CLAP | CLAP plugin id, reverse-DNS | `.clap` bundle |
| VST3 | class id in canonical `FUID::toString` form: 32 uppercase hex, words l1..l4 (the `CID` in `moduleinfo.json`). Same on every OS; convert with `ether_vst3::class_id_to_string` / `parse_class_id` (COM byte layout on Windows) | `.vst3` bundle |
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

## Licenses

Everything is GPL-3.0-or-later compatible:

| Dependency | License |
|---|---|
| `vst3` crate | MIT OR Apache-2.0. Its bindings derive from the VST3 SDK headers, which Steinberg has published under MIT since SDK 3.8. |
| objc2 family | Zlib OR Apache-2.0 OR MIT |
| `block2` | MIT |
| `libloading` | ISC |

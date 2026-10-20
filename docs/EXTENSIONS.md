# Extension points

Ethereal can be extended without editing its files. An extension is a separate package that
depends on this repository: its entry module registers contributions, then starts the app. The
app reads the contributions where they are used. When nothing is contributed, as in this
repository's own build, the app shows only what it can do by itself. It never shows a menu item,
tab or command that would answer "unsupported".

New features that live outside this repository should go through these points. If no point fits,
add one here: a small, typed hook named after the place in the app, not after the feature.

## UI (`ui/src/ext`)

```ts
import { contribute, contributions, useContributions, emit } from "@/ext";

contribute("palette.commands", (ctx) => [{ id: "x:hello", group: "Help", label: "hello", run }]);
contribute("shell.root", { name: "my feature", Component: MyRoot }, /* order */ 10);
```

- `contribute(point, value, order = 0)` returns a function that takes the contribution back.
  Contributions are sorted by `order`, then in the order they were made.
- `contributions(point)` reads them, and is stable between changes. `useContributions(point)` is
  the React form.
- `emit(point, ...args)` calls every listener of a listener point. One that throws is logged and
  does not stop the others.
- Points and their value types are the `ExtensionPoints` interface (`ui/src/ext/points.ts`). An
  extension may augment that interface with points of its own.

### The points

| area | point | what it adds |
| --- | --- | --- |
| app | `app.init` | code run once the app mounts (may return a cleanup) |
| app | `shell.root` | a component mounted in a shell slot (`overlay`, `top-bar`, `status-bar`, `device-windows`), in an error boundary named after it |
| app | `shell.leftTab` | a left rail tab and its pane (`isVisible` / `useVisible` to show it only sometimes) |
| app | `app.event` | a listener for app events (device inserted, project opened, export done, ...) |
| app | `app.crash`, `app.crashActions` | a listener for caught crashes, and buttons on the crash cards |
| app | `app.toasts` | toasts in the app's stack, after its notices |
| app | `history.hint` | a note in the history panel |
| commands | `palette.commands` | commands in the command palette |
| commands | `project.commands`, `project.menu` | project commands, and where they show in the project menu (a section, an item or a submenu) |
| commands | `desktopMenu.labels` | labels for desktop menu items the extension added (see below) |
| projects | `project.leaveGuard` | asks before leaving the open project (may cancel) |
| projects | `project.badges`, `project.rowActions` | badges and actions on the project list rows |
| projects | `project.welcomeActions`, `project.getStarted` | actions on the welcome screen, and get started articles |
| projects | `project.launchHeld`, `project.foreignRoute` | the launch waits for the extension, and the URLs that aren't project URLs |
| projects | `import.isProjectFile` | file names the import treats as projects |
| settings | `settings.tab`, `settings.section` | a settings tab, or a section of one of the app's tabs |
| devices | `plugins.actions` | actions in the plugin library and the plugin folders |
| devices | `devices.headerBadge`, `devices.window` | a badge on a device header, and another way to open a device window |
| editing | `surface.menu`, `arrangement.rulerMenu` | context menu entries on the edit surfaces and the ruler |
| editing | `edit.activity` | a listener for what the user is dragging or editing |
| editing | `arrangement.layer`, `arrangement.glLayer`, `arrangement.visibleClips` | a DOM layer and a GL layer over the arranger, and the clips in view |
| editing | `clips.badge`, `clips.pending` | clip badges, and clips whose content is still on its way |
| editing | `pianoRoll.layer`, `pianoRoll.header`, `pianoRoll.follow` | a piano roll layer and header, and a view that follows something else |
| transport | `transport.blocked` | why play is unavailable right now |
| web | `web.route` | a page for a URL, instead of the editor |
| web | `web.boot`, `web.transport`, `web.debug` | at boot: a listener, a transport wrapper, and debug handles |
| web | `web.downloadUrl` | the desktop download page the web band offers |
| web | `web.engine` | the web engine: a worklet and controller worker of its own, init fields, messages and methods (`WasmTransport.endpointMethod`) |
| tests | `mock.domain`, `mock.docDomain`, `e2e.arrangement` | command domains of the mock engine, and extra arranger test handles |

i18n: an extension ships its own catalogs, merged at build time (`i18nCatalogPlugin({ extraDirs })`).
Its keys type-check through the `ExtensionMessages` interface (`ui/src/i18n/types.ts`). The engine
sends some messages itself (undo labels, notices, errors): a build that sets
`ETHER_EXTRA_LOCALES` to a directory of `<code>.json` catalogs merges them into the engine's
(`crates/ether-controller/build.rs`).

## Desktop (`apps/desktop/src-tauri`)

The macOS Help menu shows only items the app handles. An extension adds its groups with
`app_menu::set_extra_help_items(groups)` before the menu is built, and labels them in the UI with
`desktopMenu.labels`. Their actions reach the UI like the app's own.

A product with extensions is a desktop app of its own (its binary, `tauri.conf.json`,
`build.rs`) over this crate's library: its `main` calls
`ether_desktop_lib::run_with(tauri::generate_context!(), ext)`, where `ext: DesktopExt` holds
its hooks: `startup` (first thing: process-wide points), `log_filter`, `builder` (Tauri
plugins, managed state), `logs_dir`, `before_engine` (in setup, before the engine starts),
`setup` (end of setup), `commands` + `invoke` (its IPC commands, by name, and their
`tauri::generate_handler!`), `exited` (after the orderly `shutdown`). The log file takes a
line filter (`log_file::set_line_filter`, e.g. to redact secrets).

## The engine protocol (`crates/ether-protocol/src/ext.rs`)

A command domain, event or reply value this repository doesn't declare is an extension's. On
the wire it looks like the app's own (`{ "domain": "<name>", "command": … }`, `{ "type": … }`);
the protocol keeps it as JSON in the `Ext` variant (`Command::Ext`, `Event::Ext`,
`ReplyValue::Ext`), and the extension reads it with its own types (`ExtCommand::parse`,
`ExtValue::parse`). Without an extension the engine replies `Unsupported`. The generated
TypeScript types leave `Ext` out: an extension generates its own and joins them to the app's.

In the controller (`crates/ether-controller`):

- `set_ext_labels(f)`: the undo labels of an extension's commands (catalog messages).
- `set_agent_selection(selection)`: the selection the agent tools read, for an extension that
  learns it otherwise (the UI sends `Agent::SetSelection`).
- `ETHER_EXTRA_LOCALES` (build time, see i18n above): the extension's undo labels, notices and
  errors.

## The controller (`crates/ether-controller/src/ext.rs`)

An extension implements `ControllerExt` and is added with `EtherController::add_extension`.
It keeps its own state and gets the controller with every hook:

- `command`: every command, before the app handles it. Returning `Some` answers it: an
  extension's own domain (`Command::Ext`), or an app command the app leaves to extensions
  (`Project::Import`, `CancelImport` and `ExportTo` reply `Unsupported` without one).
- `tick`: every controller tick (background jobs).
- The app's seams: `before_command`, `refuse_edit`, `committed` and `applied` (document edits),
  `undo_redo` and `session_active` (an extension that keeps a shared history), `view_only`,
  `transport_state` / `transport_changed`, `save_refused` / `before_save`, `plugin_notes` /
  `plugin_reloaded`, `project_loaded`, `engine_changed` / `engine_status` /
  `engine_off_error` (an extension that turns the local engine off). Each defaults to
  the app's single-user behavior.
- Saves: `save_sidecar` (a file written right after the document, in the same job), `saved`,
  `project_copied` (a `SaveAs` / `Duplicate` copy is a private project).
- Renders (freeze, bounce, consolidate): `offload_render` runs one elsewhere (the result comes
  back through `finish_render`), `cancel_render` / `render_busy`; `freeze_for(site, ...)` runs
  one here for another site, reported through `render_update` (`render_requester_present`
  drops it when that site is gone).
- Document commands of an extension (one undo step, like the app's): `set_ext_doc_commands`.
- Files of its own in a project folder: `store::set_store_ext(StoreExt { private_files,
  sidecar_files, summary })`. Every store leaves `private_files` out of a `Duplicate` /
  `SaveAs` copy, waits for a queued save before reading or replacing a `sidecar_files` file
  (written with the document, see `save_sidecar`), and lists `summary`'s value as
  `ProjectSummary::ext` (e.g. a badge on the project list).

An extension that needs state of its own inside the controller gives it as the controller's
fifth type parameter: `EtherController<B, H, S, L, X>` (`X = ()` by default) built with
`with_state(..., x)`, readable as `ctl.ext`.

It may use the controller's public API: its `pub` fields (bridge, host, store, library) and
methods (`Controller::project`, `list_projects`, `load_project`, `take_upload`, `add_download`, ...),
and the helpers of `ether_controller::ext::api`.

The hosts add a process's extensions to every controller they create:
`ether_native::host::set_controller_setup(f)` (desktop app, engine server) and
`ether_wasm::web::set_controller_setup(f)` (web), called once at startup. An extension with
state of its own builds the controller itself instead: `set_controller_factory(f)` (both
hosts), `f` returning a controller made with `with_state`.

## The native bridge (`crates/ether-native/src/bridge.rs`)

- `bridge::set_bridge_ext(f)` (once, at startup): `f` makes a `BridgeExt` for every
  `NativeBridge` (the main engine's and each dedicated engine's). It sees the graphs and
  transport controls the bridge sends (`observe_graph`, `observe_transport`) and the
  dedicated engines spawned from it (`spawned`), and is dropped before the bridge's plugin
  instances. The extension reaches it with `NativeBridge::ext_mut` / `ext_ref`, and the
  engine handle for taps of its own with `ext_and_handle`.
- `NativeBridge::spawn_dedicated()`: a second engine of this app on the paced `null` backend
  (no audio device), for a copy of the open project played for someone else. Its plugins are
  the same host's, registered under ids of their own (`dedicated_device_id`,
  `is_main_instance_id`); their notifications come back from its own `poll_plugins`.
  Dropping the `DedicatedEngine` stops it.
- Feature `test-support`: fake plugins (`plugins::fake`) and `test_util::fake_bridge` for
  another crate's tests.
- `health::set_watchdog_observer(f)` (once, at startup): `f` is also told every stall event
  the engine watchdog reports (from the watchdog thread).

## Plugin formats (`crates/ether-plugin-host`)

`ether_plugin_host::set_extra_formats(f)` (once, at start): `f`'s `PluginFormatHost`s are
hosted after the app's (CLAP, VST3, AU) by every `Formats::with_extras` registry of the
process: the native host (`ether_native::plugins::formats`), the scanner
(`ether_plugin_scanner::formats`) and the sandbox helper (`ether_sandbox::helper::formats`).
The scanner and the helper are processes of their own: a product with extra formats builds
its own `ether-plugin-scanner` and `ether-sandbox-helper` binaries, which register them and
then run `ether_plugin_scanner::main()` / `ether_sandbox::helper::main()`.

## The web engine module (`crates/ether-wasm`)

`ether_wasm::web::set_controller_factory(f)` / `set_controller_setup(f)` give the web host's
controllers an extension's state and extensions. They run in a module of the product's own:
a `cdylib` crate over `ether-wasm` (whose `#[wasm_bindgen]` exports it re-exports by linking
it), with a `#[wasm_bindgen(start)]` function that sets them, and exports of its own.
`apps/web/scripts/build-wasm.mjs` builds it with `--package <its package>` (and
`ETHER_WASM_WORKSPACE=<its workspace>` when it lives in another one); the output keeps the
app's name (`ether_wasm.js`), so the UI loads it the same way.

## The engine server (`crates/ether-server`)

The `ether-server` process is `ether_server::app::main(hooks)`: its binary passes the default
`Hooks`. A product with extensions builds an `ether-server` binary of its own whose `Hooks`
set them up: `startup` (first thing: controller factory, bridge extension, plugin formats)
and `started(data_dir, dev)` (before the engine starts; the closure it returns runs at
shutdown).

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

/**
 * The UI extension points (`contribute(point, value)`, `./index.ts`; docs/EXTENSIONS.md).
 * Each entry names where the app reads it. Types only: this file imports no runtime code, so
 * any module may contribute or read without import cycles.
 */
import type { ComponentType, ReactNode, RefObject } from "react";
import type { Clip, ClipId, Device, Project, ProjectSummary, TrackId } from "@/generated";
import type { DeviceDescriptor } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import type { EngineTransport } from "@/transport";
import type { TourDef } from "@/features/tutorials/types";

/** An edit surface: the arranger, or a clip's editor (the piano roll). */
export type EditSurface = { kind: "arranger" } | { kind: "editor"; clip: ClipId };

/** What the user is doing on the arrangement (`edit.activity`): a drag or resize of `target`. */
export interface EditActivity {
  kind: "Dragging" | "Resizing";
  target: { type: "Selection" } | { type: "Clip"; clip: ClipId } | { type: "Track"; track: TrackId };
}

/** The arranger's layers' geometry (`arrangement.layer`). */
export interface ArrangementLayerProps {
  rootRef: RefObject<HTMLDivElement | null>;
  scrollRef: RefObject<HTMLDivElement | null>;
  rows: ReadonlyArray<unknown>;
  masterRow: unknown;
}

/** The piano roll grid: px ⇄ the focused clip's content beats and pitch (the key plus how far up it). */
export interface EditorCursorMapping {
  fromScreen(x: number, y: number): { beats: number; pitch: number };
  toScreen(beats: number, pitch: number): { x: number; y: number };
}

/** The piano roll's layers' geometry (`pianoRoll.layer`). */
export interface PianoRollLayerProps {
  clip: ClipId;
  gridRef: RefObject<HTMLDivElement | null>;
  mapping: RefObject<EditorCursorMapping>;
  /** Changes whenever the grid's mapping does (zoom, scroll, rows). */
  layoutKey: string;
}

/** A badge on clips' titles in the arranger (`clips.badge`), e.g. "loading". */
export interface ClipBadgeProvider {
  /** The badge `clip` shows now, or `null`. `hatch`: a soft hatch over the clip too. */
  badge(clip: Clip): { id: string; text: string; hatch?: boolean; hidesMissing?: boolean } | null;
  /** Calls `changed` when badges may have changed; returns the unsubscribe. */
  subscribe(changed: () => void): () => void;
}

/** Clips whose content isn't here yet (`clips.pending`): their editor waits. */
export interface ClipPending {
  /** A hook: which clips' content is still on its way now. */
  usePending(): (clip: Clip) => boolean;
  /** The editor's text meanwhile. */
  text(): string;
}

/** The piano roll's view follows someone else's (`pianoRoll.follow`). */
export interface PianoRollFollow {
  /** Rendered in the editor (nothing visible): publishes its view and applies a followed one. */
  Follow: ComponentType<{
    clip: ClipId;
    layout: RefObject<{ clip: ClipId; shift: number; rows: ReadonlyArray<number>; keyH: number }>;
    view: unknown;
    body: RefObject<HTMLDivElement | null>;
  }>;
  /** Whether the view of `clip` follows someone's now (the editor doesn't fit it on open). */
  following(clip: ClipId): boolean;
}

/** A device window's control as an extension decides it (`devices.window`). */
export interface ExtDeviceWindow {
  open: boolean;
  available: boolean;
  /** The button's tooltip instead of the app's. */
  title?: string;
  /** `aria-disabled` instead of `disabled` (the tooltip still shows why). */
  softDisabled?: boolean;
  /** Extra `data-*` attributes of the button (tests). */
  data?: Record<string, string>;
  toggle(): void;
  show(): void;
}

/** A component mounted once by a shell, inside its own error boundary. */
export interface ShellRoot {
  /** Its error boundary's name ("<name> crashed"; `PanelBoundary` labels it if it knows it). */
  name: string;
  /** Its name in a sentence ("<label> crashed"). */
  label?(): string;
  Component: ComponentType;
  /** The shells that mount it (default: the editor only). */
  shells?: ReadonlyArray<"editor" | "mobile" | "startup">;
  /**
   * Where: `overlay` (dialogs and runtimes, after the app's; default), `device-windows` (with
   * the floating device windows), `top-bar` (the top bar's end, before settings),
   * `status-bar` (items at the end of the status bar below the workspace: a `StatusButton`,
   * with any details in a popover above it; the UI has no floating cards but toasts).
   */
  slot?: "overlay" | "device-windows" | "top-bar" | "status-bar";
  /** `top-bar`: its wrapper's `data-slot` (default: `name`). */
  dataSlot?: string;
}

/** A command palette entry (`app/shell/commands.ts` `PaletteCommand`). */
export interface ExtPaletteCommand {
  id: string;
  label: string;
  group: string;
  shortcut?: string;
  action?: string;
  bindable?: boolean;
  keywords?: string;
  userText?: string;
  run(): void;
}

export interface PaletteContext {
  /** `null`: no engine (the commands that need one are left out). */
  transport: EngineTransport | null;
  project: Project | null;
  devices: ReadonlyArray<DeviceDescriptor>;
}

/** A project command (palette "Project" group, the project menu; `features/project/projectCommands.ts`). */
export interface ExtProjectCommand {
  id: string;
  label: string;
  /** The label inside a menu submenu ("export to another daw" ▸ "ableton live set (.als)…"). */
  short?: string;
  group: string;
  keywords?: string;
  title?: string;
  action?: string;
  run(): void;
}

export interface ProjectCommandContext {
  transport: EngineTransport;
  /** The open project, or `null` (app-level commands only). */
  project: { id: string; name: string } | null;
  /** Run an action whose failure shows as an error toast. */
  attempt(action: () => Promise<unknown>): void;
  /** Ask before switching projects when something would be lost (`project.leaveGuard`). */
  guardLeave(action: () => void | Promise<unknown>, confirm?: string): Promise<boolean>;
}

/**
 * Entries of the project menu (`ProjectMenuPanel`): command ids (from `project.commands` or the
 * app's), or a submenu of them, placed in a section. `when` hides it (default: shown).
 */
export interface ProjectMenuEntry {
  /**
   * `start`: after "open project…"; `import` / `export`: after "import project…" / "export
   * project…" (with a project open); `app`: the menu's end.
   */
  section: "start" | "import" | "export" | "app";
  item: { command: string } | { submenu: string; tour?: string; commands: ReadonlyArray<string> };
  when?(): boolean;
}

/** An entry of a stored project's menu in the picker (`ProjectScreen`). */
export interface ProjectRowAction {
  label: string;
  danger?: boolean;
  /** Asked in place first (the row shows `text` with cancel and `button`). */
  confirm?: { text: ReactNode; title: string; button: string; buttonLabel: string };
  /** Open the project first (asking `project.leaveGuard`) when it isn't the open one. */
  openFirst?: boolean;
  /** Shown as a toast once it ran. */
  done?: string;
  /** Close the picker once it ran. */
  close?: boolean;
  /** The action; a failure shows on the picker's error line. */
  run(transport: EngineTransport): Promise<unknown>;
}

/** A settings tab of its own (`AudioSettingsDialog`). */
export interface SettingsTabContribution {
  id: string;
  label(): string;
  /** Shown without an engine (else the "no engine connected" note replaces it). */
  noEngine?: boolean;
  /** Placed before this tab (default: before "advanced"). */
  before?: string;
  Component: ComponentType<{ transport: EngineTransport | null }>;
}

/** A section added to a settings tab (built-in or contributed). */
export interface SettingsSectionContribution {
  tab: string;
  /** `start` or `end` (default) of the tab. */
  at?: "start" | "end";
  /** Shown without an engine too (default: only with one). */
  noEngine?: boolean;
  Component: ComponentType<{ transport: EngineTransport | null }>;
}

/** A left rail tab (`app/shell/LeftRail.tsx`). */
export interface LeftTabContribution {
  id: string;
  label(): string;
  icon: ReactNode;
  /** Placed before this tab (default: before "ai"). */
  before?: string;
  /** Whether the tab shows now (its pane closes when it stops showing; default: always). */
  isVisible?(): boolean;
  /** `isVisible` for a component (a hook that re-renders when it changes). */
  useVisible?(): boolean;
  Component: ComponentType;
}

/** A "get started" card of the welcome screen (`features/project/GetStarted.tsx`). */
export interface GetStartedContribution {
  title(): string;
  blurb(): string;
  icon: ReactNode;
  tour: TourDef;
  /** Placed after the app's card with this tour id (default: last). */
  after?: string;
}

/** An app-level event for an extension's analytics (none are recorded in this build). */
export type AppEvent =
  { type: "project_opened" } | { type: "device_inserted"; category: string; source: string } | { type: "export"; format: string; outcome: "ok" | "failed" };

/** A crash an error boundary caught (`app/PanelBoundary.tsx`). */
export interface CaughtCrash {
  panel: string;
  error: Error;
  componentStack: string | null;
}

/** A web route of its own (`apps/web/src/main.tsx`), served without the editor or before it. */
export interface WebRoute {
  /** Whether this route owns the page at `location`. */
  match(location: Location): boolean;
  /**
   * Render it into the page. `startApp(invite)` boots the editor instead (an invite link to
   * hand to the `web.boot` contributions, or `null`).
   */
  mount(root: HTMLElement, startApp: (arg: string | null) => void): void | Promise<void>;
}

/** The live handles of the web engine (`apps/web/src/engine/endpoint.ts`). */
export interface WebEngineHandles {
  context: AudioContext;
  node: AudioWorkletNode;
  controller: Worker;
  fs: Worker;
}

/** An extension of the web engine (`apps/web/src/engine/endpoint.ts`). */
export interface WebEngineExtension {
  /** Its worklet module (registers `ether-engine`: the app's processor, extended). */
  workletUrl?: string;
  /** Worklet outputs of its own, after the speakers' (each stereo). */
  extraOutputs?: number;
  /** Fields added to the worklet processor's options. */
  processorOptions?(): Record<string, unknown>;
  /** Its controller Worker (a module that imports the app's and extends it). */
  controllerWorker?(): Worker;
  /** Before the controller starts: fields and transferables added to its init message. */
  init?(handles: WebEngineHandles): { fields?: Record<string, unknown>; transfer?: Transferable[]; cleanup?(): void } | void;
  /** A message of its own from the controller Worker (`ext:*`). */
  onControllerMessage?(msg: { type: string; [field: string]: unknown }, handles: WebEngineHandles): void;
  /** Methods added to the endpoint (`WasmEndpoint`; `handles` is `null` before start). */
  methods?(handles: () => WebEngineHandles | null): Record<string, unknown>;
}

export interface ExtMockDomain {
  /** The `Command` domain (`command.domain`) it answers (`Project`: the project commands the mock doesn't know). */
  domain: string;
  /** The reply, or `undefined` to leave it to the mock (`MockExtensionHost`: what it may use). */
  handle(command: unknown, mock: import("@/transport/mock/MockTransport").MockExtensionHost): unknown;
  /** Called for each new mock (e2e handles on it). */
  attach?(mock: import("@/transport/mock/MockTransport").MockExtensionHost): void;
}

/** A document command domain of the mock (`transport/mock/roadmap`): undoable, in batches. */
export interface ExtMockDocDomain {
  domain: string;
  /** Apply it in the mock's reducer (`ctx`: the reducer context). */
  apply(ctx: unknown, command: unknown): void;
}

export interface ExtensionPoints {
  /** Called once when the app shell first renders; may return a cleanup. */
  "app.init": () => void | (() => void);
  /** Components the shells mount (`app/App.tsx`, `features/mobile/MobileShell.tsx`, startup screens). */
  "shell.root": ShellRoot;
  /** Command palette entries (`app/shell/commands.ts`), after the app's. */
  "palette.commands": (ctx: PaletteContext) => ReadonlyArray<ExtPaletteCommand>;
  /** Project commands (palette and project menu). */
  "project.commands": (ctx: ProjectCommandContext) => ReadonlyArray<ExtProjectCommand>;
  /** Project menu entries for contributed (or app) commands. */
  "project.menu": ProjectMenuEntry;
  /**
   * Switching, creating or deleting a project: the first contribution that returns a promise
   * decides (it may ask first); none, or all `null`: the action runs at once.
   */
  "project.leaveGuard": (action: () => void | Promise<unknown>, confirm: string | undefined) => Promise<boolean> | null;
  /** Badges after a project's name in the project picker (`ProjectScreen`). */
  "project.badges": ComponentType<{ project: ProjectSummary }>;
  /** Buttons on the project picker's welcome row (before "new project"; `onDone` closes the picker). */
  "project.welcomeActions": ComponentType<{ busy: boolean; ready: boolean; onDone(): void }>;
  /** Entries of a stored project's menu in the picker (before "delete"). */
  "project.rowActions": (project: ProjectSummary) => ReadonlyArray<ProjectRowAction>;
  /** Whether launch must not show the project picker now (another screen takes its place). */
  "project.launchHeld": () => boolean;
  /** Paths whose address the project URL sync leaves alone (`features/project/projectUrl.ts`). */
  "project.foreignRoute": (pathname: string) => boolean;
  /** Files the audio import hands to another importer instead (`features/import`). */
  "import.isProjectFile": (name: string) => boolean;
  /** Settings tabs of their own. */
  "settings.tab": SettingsTabContribution;
  /** Sections added to settings tabs. */
  "settings.section": SettingsSectionContribution;
  /** Left rail tabs. */
  "shell.leftTab": LeftTabContribution;
  /** Welcome screen cards. */
  "project.getStarted": GetStartedContribution;
  /** App events (analytics). */
  "app.event": (event: AppEvent) => void;
  /** Crashes the error boundaries caught. */
  "app.crash": (crash: CaughtCrash) => void;
  /** Extra actions of a crash card or the engine error screen ("report an issue"). */
  "app.crashActions": ComponentType<{ fullWindow: boolean }>;
  /** Labels of desktop menu items an extension adds (`features/desktop-menu`): action id → label. */
  "desktopMenu.labels": () => Readonly<Record<string, string>>;
  /** Buttons in the device library's footer and the plugin folders panel ("explore plugins"). */
  "plugins.actions": ComponentType<{ place: "library" | "folders" | "folders-web"; onClose?(): void }>;
  /**
   * Toasts in the app's one stack, after its notices (`app/App.tsx`): an extension's toasts
   * (chat messages, ...) share it, so they never overlap the app's.
   */
  "app.toasts": ComponentType;
  /** A hint under the undo history (`features/undo-history`). */
  "history.hint": ComponentType;
  /** Web routes (`apps/web`). */
  "web.route": WebRoute;
  /** The web editor starts (before its engine), with the invite a `web.route` started it for. */
  "web.boot": (invite: string | null) => void;
  /** Wrap the web editor's transport (e2e fakes; may load the wrapper first); `void`: keep it. */
  "web.transport": (transport: EngineTransport) => EngineTransport | void | Promise<EngineTransport | void>;
  /** Entries added to the web build's read-only debug handle (`window.__ether`). */
  "web.debug": (transport: EngineTransport) => Record<string, unknown>;
  /**
   * Why the transport bar can't play or record now (a hook; `null`: it can). Its tooltip says
   * so and its keys do nothing (`features/transport-bar`).
   */
  "transport.blocked": () => string | null;
  /** The desktop app's download page, offered by the web's "open on desktop" band when the app didn't open. */
  "web.downloadUrl": () => string;
  /** Entries of an edit surface's context menu (the arranger's tracks and lanes, the piano roll's grid). */
  "surface.menu": (surface: EditSurface, at: { clientX: number; clientY: number }) => ReadonlyArray<ContextMenuEntry>;
  /** Entries of the arranger ruler's context menu at `beats`. */
  "arrangement.rulerMenu": (beats: number) => ReadonlyArray<ContextMenuEntry>;
  /** What the user drags or resizes on the arrangement now (`null`: done). */
  "edit.activity": (activity: EditActivity | null) => void;
  /** Layers over the arranger's tracks (DOM, in the arrangement's scroll content). */
  "arrangement.layer": ComponentType<ArrangementLayerProps>;
  /** Layers of the WebGL arranger (`gl/layers/overlays.ts`): mounted with the renderer. */
  "arrangement.glLayer": (renderer: unknown) => { onTheme(): void; remove(): void };
  /** The clips the arranger shows (the edited one first), every quarter second while contributed. */
  "arrangement.visibleClips": (clips: ReadonlyArray<ClipId>, transport: EngineTransport) => void;
  /** Badges on the arranger's clip titles. */
  "clips.badge": ClipBadgeProvider;
  /** Clips whose content isn't here yet. */
  "clips.pending": ClipPending;
  /** Layers over the piano roll's grid. */
  "pianoRoll.layer": ComponentType<PianoRollLayerProps>;
  /** Shown in the piano roll's header (e.g. who else edits this clip). */
  "pianoRoll.header": ComponentType<{ clip: ClipId }>;
  /** The piano roll's view follows someone else's. */
  "pianoRoll.follow": PianoRollFollow;
  /** Badges in a device's header. */
  "devices.headerBadge": ComponentType<{ device: Device }>;
  /**
   * A device window the app can't open itself (a plugin missing here; a hook): the control,
   * or `null` to leave it to the app.
   */
  "devices.window": (device: Device, ctx: { missing: boolean; noPluginHost: boolean }) => ExtDeviceWindow | null;
  /** Extra entries of the web build's arranger test API (`features/arrangement/testApi.ts`; e2e). */
  "e2e.arrangement": () => Record<string, unknown>;
  /** The web engine (`apps/web/src/engine/endpoint.ts`). */
  "web.engine": WebEngineExtension;
  /**
   * Loads an extension's mock domains (it contributes them when loaded): awaited before the app
   * creates its mock engine (`loadMockTransport`), which it loads only when it runs against it.
   */
  "mock.load": () => Promise<unknown>;
  /** Mock engine domains (`transport/mock/MockTransport.ts`). */
  "mock.domain": ExtMockDomain;
  /** Mock document command domains (`transport/mock/roadmap`). */
  "mock.docDomain": ExtMockDocDomain;
}

/** Re-exported for contributors (the welcome cards' tour list, clips in a contributed badge, …). */
export type { Clip, TrackId };

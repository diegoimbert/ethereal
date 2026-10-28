/**
 * App shell: top bar slots, the workspace and the footer. Features export their entry from
 * `ui/src/features/<name>/index.tsx`; the panes and drawer tabs mount theirs in
 * `app/shell/` (`LeftRail`, `Inspector`, `tabs.tsx`).
 *
 * Layout:
 *   ┌ project menu │ transport capsule │ metronome │ recording │ export │ remote │ share ┐
 *   │rail│ markers + arrangement (fills the workspace)                                   │
 *   │    │  floating panes over it (or pinned beside it): browser (left, from the rail),   │
 *   │    │  inspector (right, with the selection), editor drawer (bottom)                 │
 *   └────┴───────────────────────────────────────────────────────────────────────────────┘
 *
 * Engine access: entries (`ui/src/main.tsx`, `apps/web/src/main.tsx`) wrap `<App />` in
 * `<TransportProvider transport={await createDefaultTransport()}>` from `@/transport`; features
 * use `useTransport()` and read the document from `@/state`.
 */
import { lazy, memo, Suspense, useContext, useEffect, useRef } from "react";
import { useT } from "@/i18n";
import { ConfirmDialog, ContextMenuHost, ReadOnlySurface, TitleTooltipHost, ToastStack } from "@/kit";
import { ConvertToSamplerDialog } from "@/features/sampler-convert";
import { dismissOnRelease, useEditorStore, useProjectStore, useReadOnly } from "@/state";
import { size } from "@/theme";
import { ArrangementView, ClipColorPicker, useKeepTrackHeights } from "@/features/arrangement";
import { AudioSettingsDialog } from "@/features/audio-settings";
import { MarkerLane } from "@/features/clip-editing";
import { DebugPanelsRoot } from "@/features/debug";
import { DeviceWindowsRoot } from "@/features/devices/window";
import { ExportDialog } from "@/features/export";
import { ImportRoot } from "@/features/import";
import { KeymapRoot, matchesAction } from "@/features/keymap";
import { EngineLanguage } from "./EngineLanguage";
import { MediaRefsRoot } from "@/features/media-refs";
import { MidiLearnRoot } from "@/features/midi-learn";
import { NotificationLog, NotificationToastItems } from "@/features/notifications";
import { ImportResolverRoot } from "@/features/import-resolver";
import { ProjectMenu } from "@/features/project";
import { RecordingControls, useKeepRecordingSetup } from "@/features/recording";
import { ConnectDialog } from "@/features/remote";
import { TransportBar, useCompactTopBar } from "@/features/transport-bar";
import { ValueEditHost } from "@/features/automation/valueMenu";
import { QuitRoot } from "@/features/quit";
import { DesktopMenuRoot } from "@/features/desktop-menu";
import { TourRoot } from "@/features/tutorials";
import { DocsRoot } from "@/features/docs";
import { NewsRoot } from "@/features/news";
import { useDocumentTitle } from "@/features/document-title";
import { VersionsRoot } from "@/features/versions";
import { CommandPalette } from "./shell/CommandPalette";
import { QuickSearch } from "./shell/QuickSearch";
import { DrawerFullscreenButton, DrawerShortcut, DrawerTabs, EditorDrawer, useDrawerFullscreen } from "./shell/EditorDrawer";
import { FloatingPane } from "./shell/FloatingPane";
import { Inspector } from "./shell/Inspector";
import { useInspectorTarget } from "./shell/inspectorTarget";
import { LeftPanel, LeftRail } from "./shell/LeftRail";
import { useClickFocusesPanelSearch, useFocusPanelSearch } from "./shell/useFocusPanelSearch";
import { bindSelectionHistory } from "@/features/time-edits/selectionHistory";
import { railTabs } from "./shell/tabs";
import { paneLayout } from "./shell/paneLayout";
import { closeInspector, useShellStore } from "@/state/shell";
import { useMobileUi } from "@/features/mobile/mode";
import { EngineErrorScreen } from "@/features/startup";
import { TransportContext } from "@/transport";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { PanelBoundary } from "./PanelBoundary";
import { StatusBar } from "@/features/status-bar";
import { ExtensionRoots, useExtensionInit } from "./ExtensionRoots";
import { useContributions } from "@/ext";
import { useAgentSelectionSync } from "@/features/ai-chat/selectionSync";
import "./App.css";

/** Sizes in px, from the design tokens (`size.*` in ui/src/theme/tokens.ts). */
const px = (token: string) => parseFloat(token);
const GAP = px(size.floatGap);
const MIN = px(size.floatMinSize);
const MAIN_MIN = px(size.mainMinSize);
/** The shell's height: the window's, less anything above it (the web's "Open on desktop" band). */
const shellHeight = () => document.querySelector<HTMLElement>(".eth-shell")?.clientHeight || window.innerHeight;
/** A pane's largest size: the arrangement keeps at least `MAIN_MIN` beside / above it. */
const sideMax = () => window.innerWidth - MAIN_MIN;
const bottomMax = () => shellHeight() - MAIN_MIN - px(size.topBarHeight) - px(size.statusBarHeight);

// The workspace re-renders whenever a pane opens, closes, resizes or (un)pins; its content
// doesn't depend on that, so it is memoized (re-rendering the arrangement or the piano roll
// on every pane change made the pane animations stutter).
const Arrangement = memo(ArrangementView);
const Markers = memo(MarkerLane);
const Rail = memo(LeftRail);
const LeftContent = memo(LeftPanel);
const InspectorContent = memo(Inspector);
const Drawer = memo(EditorDrawer);

/**
 * Presses in these never close the drawer: itself, the tracks (per-clip logic), floating layers,
 * and the tools used while editing a clip: the rail and side panes (open the browser and drag a
 * sample onto the drum rack, edit in the inspector) and the top bar (play while shaping an envelope, open collaboration or settings).
 */
const DRAWER_KEEPS =
  '[data-pane], .eth-rail, .eth-shell__top, .eth-arr__scroll, .eth-arr__master, .eth-popover, .eth-dialog-backdrop, [role="menu"], [role="dialog"], [role="listbox"], [data-radix-popper-content-wrapper]';

/**
 * The workspace: the arrangement fills it; the icon rail sits on its left edge; the
 * browser (left), inspector (right) and editor drawer (bottom) float over it as cards, or,
 * pinned, also take their space (the arrangement shrinks, animated).
 */
function Workspace() {
  const t = useT();
  // A shared project's viewer (`/s/<id>`) has no browser or side panels: nothing to add.
  const readOnly = useReadOnly();
  const storedLeft = useShellStore((s) => s.left);
  const left = readOnly ? { ...storedLeft, open: false } : storedLeft;
  const right = useShellStore((s) => s.right);
  const bottom = useShellStore((s) => s.bottom);
  const fullscreen = useDrawerFullscreen();
  const shell = useShellStore.getState;
  const target = useInspectorTarget();
  const rightOpen = target !== null;

  // Undo / redo bring back the sections (time selection, piano-roll section) of their step.
  useEffect(() => bindSelectionHistory(), []);

  // The inspector opens with a selection and stays until closed (Esc), following the selection
  // (the store knows, for layout and tests); expanded or a bar: a track header click expands
  // it, its × or a double-click on a clip collapses it (TrackRow, below).
  useEffect(() => {
    if (shell().right.open !== rightOpen) shell().setOpen("right", rightOpen);
  }, [rightOpen, shell]);

  // Showing a clip (a click on one, a double-click, "open in editor") opens the drawer on its
  // editor; a click shows it collapsed unless the drawer is expanded (ArrangementView).
  useEffect(
    () =>
      useEditorStore.subscribe((s, prev) => {
        // Clicked away from the drawer (and off any clip): an unpinned drawer collapses to its
        // bar, or closes when it has no clip to show; Escape closes it.
        if (s.dismissed !== prev.dismissed) {
          const { bottom } = shell();
          if (!bottom.open || bottom.pinned) return;
          const clip = s.clip ? useProjectStore.getState().project?.clips[s.clip] : undefined;
          if (s.dismissCloses || !clip) shell().setOpen("bottom", false);
          else shell().setCollapsed("bottom", true);
          return;
        }
        if (s.request === prev.request || !s.clip) return;
        const clip = useProjectStore.getState().project?.clips[s.clip];
        if (!clip) return;
        shell().openDrawer(clip.content.type === "Midi" ? "piano-roll" : "warp");
        // A double-click always shows the editor: the drawer expands at its last size, clamped
        // like a resize so the arrangement keeps its minimum size; the inspector collapses to
        // its bar (unless pinned), leaving the room to the editor.
        if (s.revealed !== prev.revealed) {
          shell().expand("bottom", MIN, bottomMax());
          if (!shell().right.pinned) shell().setCollapsed("right", true);
        }
      }),
    [shell],
  );

  // Escape that nothing else used (not in a text field, dialog or menu) closes the inspector
  // (in the arrangement, its deselect does: features/arrangement/actions.ts).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!matchesAction("edit.deselect", e) || e.defaultPrevented) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.('input, textarea, [contenteditable="true"], [role="dialog"], [role="menu"], [role="listbox"]')) return;
      closeInspector();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // A press anywhere outside the drawer collapses it, unless pinned. Not the arrangement's
  // tracks (they decide per clip: a clip opens/switches the drawer), nor the drawer itself
  // (its resize handle included), nor floating layers (popovers, menus, dialogs) opened
  // from it, nor the side panes and the top bar (see DRAWER_KEEPS).
  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if (e.button !== 0 || !(e.target instanceof Element)) return;
      if (e.target.closest(DRAWER_KEEPS)) return;
      const { bottom } = shell();
      if (bottom.open && !bottom.pinned) dismissOnRelease();
    };
    document.addEventListener("pointerdown", onDown, true);
    return () => document.removeEventListener("pointerdown", onDown, true);
  }, [shell]);

  // A collapsed pane is a floating bar: it reserves no space and the drawer need not make room.
  const style = paneLayout(
    left,
    { ...right, open: rightOpen && !right.collapsed },
    { ...bottom, open: bottom.open && !bottom.collapsed },
    GAP,
  ) as React.CSSProperties;
  // A press anywhere in the arrangement (tracks, ruler, headers, markers) collapses an
  // unpinned left pane; the press still does its own job (capture, never stopped).
  const collapseLeft = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const { left } = shell();
    if (left.open && !left.pinned) shell().setOpen("left", false);
  };
  // Opening a panel puts the caret in its search field.
  const sidebar = useRef<HTMLDivElement>(null);
  useFocusPanelSearch(sidebar, left.open ? left.tab : null);
  useClickFocusesPanelSearch(sidebar, left.open ? left.tab : null);
  const leftTab = railTabs().find((tab) => tab.id === left.tab);
  const leftLabel = leftTab ? leftTab.label() : t("shell.tab.browser");

  return (
    <div className="eth-workspace" style={style} data-read-only={readOnly || undefined}>
      {!readOnly && (
        <PanelBoundary name="rail">
          <Rail />
        </PanelBoundary>
      )}
      <main className="eth-workspace__main" data-slot="main" onPointerDownCapture={collapseLeft}>
        <div className="eth-workspace__stage">
          <PanelBoundary name="arrangement">
            <div data-slot="markers">
              <ReadOnlySurface>
                <Markers />
              </ReadOnlySurface>
            </div>
            <Arrangement />
          </PanelBoundary>
        </div>
      </main>

      {!readOnly && (
        <FloatingPane
          side="left"
          label={leftLabel}
          header={<span className="eth-float__heading">{leftLabel}</span>}
          open={left.open}
          pinned={left.pinned}
          size={left.size}
          minSize={MIN}
          maxSize={sideMax}
          onResize={(v) => shell().setSize("left", v)}
          onPinnedChange={(v) => shell().setPinned("left", v)}
          onClose={() => shell().setOpen("left", false)}
        >
          <div ref={sidebar} data-slot="sidebar" className="eth-float__fill">
            <PanelBoundary name={leftTab?.id ?? "library"}>
              <LeftContent tab={left.tab} />
            </PanelBoundary>
          </div>
        </FloatingPane>
      )}

      <FloatingPane
        side="right"
        tour="inspector"
        label={t("shell.pane.inspector")}
        header={<span className="eth-float__heading">{t("shell.pane.inspector")}</span>}
        open={rightOpen}
        pinned={right.pinned}
        size={right.size}
        minSize={MIN}
        maxSize={sideMax}
        onResize={(v) => shell().setSize("right", v)}
        onPinnedChange={(v) => shell().setPinned("right", v)}
        // The × collapses it to a bar (remembered); Esc closes it. It follows the selection and
        // stays on the last one when a click elsewhere clears it (inspectorTarget).
        collapsed={right.collapsed}
        onCollapsedChange={(v) => shell().setCollapsed("right", v)}
      >
        <ReadOnlySurface>
          <PanelBoundary name="inspector">
            <InspectorContent target={target} />
          </PanelBoundary>
        </ReadOnlySurface>
      </FloatingPane>

      <FloatingPane
        side="bottom"
        label={t("shell.pane.editor")}
        collapsed={bottom.collapsed}
        onCollapsedChange={(v) => shell().setCollapsed("bottom", v)}
        header={<DrawerTabs />}
        actions={<DrawerFullscreenButton />}
        fullscreen={fullscreen}
        open={bottom.open}
        pinned={bottom.pinned}
        size={bottom.size}
        minSize={MIN}
        maxSize={bottomMax}
        onResize={(v) => shell().setSize("bottom", v)}
        onPinnedChange={(v) => shell().setPinned("bottom", v)}
        onClose={() => shell().setOpen("bottom", false)}
      >
        <ReadOnlySurface>
          <PanelBoundary name="editor">
            <Drawer />
          </PanelBoundary>
        </ReadOnlySurface>
      </FloatingPane>
    </div>
  );
}

/** The mobile shell, its own chunk: desktops never load it. */
const MobileShell = lazy(() => import("@/features/mobile/MobileShell").then((m) => ({ default: m.MobileShell })));
/** The read-only shell on a touch screen, from the same chunk. */
const MobileViewerShell = lazy(() => import("@/features/mobile/MobileShell").then((m) => ({ default: m.MobileViewerShell })));

export function App() {
  const toasts = useContributions("app.toasts");
  return (
    <PanelBoundary name="app" variant="app">
      <Shell />
      {/* The app's notices (saved, exported, duplicated, warnings), over every shell, then
          extensions' toasts (`@/ext` "app.toasts"). */}
      <ToastStack>
        <NotificationToastItems />
        {toasts.map((Toasts, i) => (
          <Toasts key={i} />
        ))}
      </ToastStack>
      <NotificationLog />
    </PanelBoundary>
  );
}

function Shell() {
  const mobile = useMobileUi((s) => s.enabled);
  const connection = useContext(TransportContext);
  const usingRemote = useTransportSwitch()?.remote != null;
  // The tab (and desktop window) title: `ethereal - <project>`.
  useDocumentTitle();
  // Extensions (`@/ext` "app.init").
  useExtensionInit();
  // The selection the agent tools read (AI chat, MCP).
  useAgentSelectionSync(connection?.transport ?? null, connection?.connection.status === "connected");
  // A read-only view (`state/viewer.ts`, an extension's): in the touch layout on phones and tablets.
  if (useReadOnly())
    return mobile ? (
      <PanelBoundary name="workspace" variant="app">
        <Suspense fallback={null}>
          <MobileViewerShell />
        </Suspense>
      </PanelBoundary>
    ) : (
      <ViewerShell />
    );
  // The local engine didn't start: say why (the editor would stay blank), with retry.
  if (connection?.connection.status === "error" && !usingRemote) return <EngineErrorScreen error={connection.connection.error} onRetry={connection.retry} />;
  if (mobile)
    return (
      <PanelBoundary name="mobile editor" variant="app">
        <Suspense fallback={null}>
          <MobileShell />
        </Suspense>
      </PanelBoundary>
    );
  return <EditorShell />;
}

/**
 * The read-only shell (`state/viewer.ts`: a view an extension shows, e.g. a shared project): the same
 * workspace (arrangement, inspector, editor drawer, device windows) and transport, and nothing
 * that changes the project or the app. No context menus or command palette.
 */
function ViewerShell() {
  const compact = useCompactTopBar();
  return (
    <div className="eth-shell eth-shell--read-only">
      <header className="eth-shell__top" data-slot="top" data-compact={compact || undefined}>
        <div className="eth-shell__transport" data-slot="transport-bar">
          <PanelBoundary name="transport bar">
            <PanelBoundary name="transport bar">
              <TransportBar />
            </PanelBoundary>
          </PanelBoundary>
        </div>
      </header>
      <PanelBoundary name="workspace">
        <Workspace />
      </PanelBoundary>
      <PanelBoundary name="device windows" variant="overlay">
        <DeviceWindowsRoot />
      </PanelBoundary>
      <DrawerShortcut />
      <TitleTooltipHost />
      <ValueEditHost />
    </div>
  );
}

function EditorShell() {
  // Arms and punch come back when a project opens again.
  useKeepRecordingSetup();
  // So do the track heights (the vertical zoom), per project on this computer.
  useKeepTrackHeights();
  const compact = useCompactTopBar();
  return (
    <div className="eth-shell">
      <header className="eth-shell__top" data-slot="top" data-compact={compact || undefined}>
        <div data-slot="project">
          <PanelBoundary name="project menu">
            <ProjectMenu />
          </PanelBoundary>
        </div>
        <div className="eth-shell__transport" data-slot="transport-bar">
          <TransportBar />
        </div>
        <div data-slot="recording">
          <PanelBoundary name="recording controls">
            <RecordingControls />
          </PanelBoundary>
        </div>
        <div data-slot="export">
          <PanelBoundary name="export">
            <ExportDialog />
          </PanelBoundary>
        </div>
        <div data-slot="remote">
          <PanelBoundary name="engine connection">
            <ConnectDialog />
          </PanelBoundary>
        </div>
        <ExtensionRoots shell="editor" slot="top-bar" />
      </header>
      <PanelBoundary name="workspace">
        <Workspace />
      </PanelBoundary>
      <StatusBar />
      {/* factory device windows, floating above the workspace. */}
      <PanelBoundary name="device windows" variant="overlay">
        <DeviceWindowsRoot />
        <ExtensionRoots shell="editor" slot="device-windows" bare />
      </PanelBoundary>
      {/* the open debug panels (features/debug), floating windows. */}
      <PanelBoundary name="debug panels" variant="overlay">
        <DebugPanelsRoot />
      </PanelBoundary>
      <PanelBoundary name="drawer shortcut" variant="overlay">
        <DrawerShortcut />
      </PanelBoundary>
      <PanelBoundary name="command palette" variant="overlay">
        <CommandPalette />
      </PanelBoundary>
      <PanelBoundary name="quick search" variant="overlay">
        <QuickSearch />
      </PanelBoundary>
      <PanelBoundary name="settings" variant="overlay">
        <AudioSettingsDialog />
      </PanelBoundary>
      <PanelBoundary name="import" variant="overlay">
        <ImportRoot />
      </PanelBoundary>
      <PanelBoundary name="media" variant="overlay">
        <MediaRefsRoot />
      </PanelBoundary>
      <PanelBoundary name="import resolver" variant="overlay">
        <ImportResolverRoot />
      </PanelBoundary>
      <PanelBoundary name="versions" variant="overlay">
        <VersionsRoot />
      </PanelBoundary>
      <PanelBoundary name="keymap" variant="overlay">
        <KeymapRoot />
      </PanelBoundary>
      <EngineLanguage />
      {/* midi-learn: learn events and MIDI mode, with or without the MIDI tab open. */}
      <PanelBoundary name="midi learn" variant="overlay">
        <MidiLearnRoot />
      </PanelBoundary>
      <PanelBoundary name="quit" variant="overlay">
        <QuitRoot />
        <DesktopMenuRoot />
      </PanelBoundary>
      {/* Guided tours from the welcome screen's "get started" (docs/TUTORIALS.md). */}
      <PanelBoundary name="tour" variant="overlay">
        <TourRoot />
      </PanelBoundary>
      {/* news: the maintainers' announcements, once after launch (docs/NEWS.md). */}
      <PanelBoundary name="news" variant="overlay">
        <NewsRoot />
      </PanelBoundary>
      {/* docs: the user documentation over the app (desktop; the web opens /docs). */}
      <PanelBoundary name="documentation" variant="overlay">
        <DocsRoot />
      </PanelBoundary>
      {/* Extensions' dialogs and runtimes (`@/ext` "shell.root"). */}
      <ExtensionRoots shell="editor" slot="overlay" />
      {/* Destructive actions (freeze, comping, expression, project delete) confirm here. */}
      <ConfirmDialog />
      <ConvertToSamplerDialog />
      <ClipColorPicker />
      <ContextMenuHost />
      <TitleTooltipHost />
      <ValueEditHost />
    </div>
  );
}

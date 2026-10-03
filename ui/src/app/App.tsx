/**
 * App shell: the ONLY place features are registered/mounted. Feature nodes never edit
 * this file; they edit their own `ui/src/features/<name>/` folder, whose `index.tsx`
 * export is mounted into a slot here. Owned by `foundation` (later wiring by `alpha`).
 *
 * Layout:
 *   ┌ project menu │ transport capsule │ metronome │ recording │ export │ remote │ share ┐
 *   │rail│ markers + arrangement (fills the workspace)                                   │
 *   │    │  floating panes over it (or pinned beside it): browser (left, from the rail),   │
 *   │    │  inspector (right, with the selection), editor drawer (bottom)                 │
 *   └────┴───────────────────────────────────────────────────────────────────────────────┘
 * Roadmap v2 slots (contracts-2, docs/ROADMAP.md) are pre-mounted with placeholders.
 *
 * Engine access: entries (`ui/src/main.tsx`, `apps/web/src/main.tsx`) wrap `<App />` in
 * `<TransportProvider transport={createDefaultTransport()}>` from `@/transport`; features
 * use `useTransport()` and read the document from `@/state`.
 */
import { memo, useEffect } from "react";
import { Moon, Settings, Sun } from "lucide-react";
import { ContextMenuHost, IconButton } from "@/kit";
import { useEditorStore, useProjectStore } from "@/state";
import { size, useTheme } from "@/theme";
import { ArrangementView } from "@/features/arrangement";
import { AudioSettingsDialog, openSettings } from "@/features/audio-settings";
import { MarkerLane } from "@/features/clip-editing";
import { ExportDialog } from "@/features/export";
import { ImportRoot } from "@/features/import";
import { MediaRefsRoot } from "@/features/media-refs";
import { ProjectMenu } from "@/features/project";
import { RecordingControls } from "@/features/recording";
import { ConnectDialog } from "@/features/remote";
import { ShareControl } from "@/features/share";
import { MetronomeSettings } from "@/features/tempo";
import { TransportBar } from "@/features/transport-bar";
import { CommandPalette } from "./shell/CommandPalette";
import { DrawerShortcut, DrawerTabs, EditorDrawer } from "./shell/EditorDrawer";
import { FloatingPane } from "./shell/FloatingPane";
import { Inspector } from "./shell/Inspector";
import { useInspectorTarget } from "./shell/inspectorTarget";
import { LeftPanel, LeftRail } from "./shell/LeftRail";
import { LEFT_TABS } from "./shell/tabs";
import { useShellStore } from "./shell/shellStore";
import "./App.css";

/** Sizes in px, from the design tokens (`size.*` in ui/src/theme/tokens.ts). */
const px = (token: string) => parseFloat(token);
const GAP = px(size.floatGap);
const MIN = px(size.floatMinSize);
const MAIN_MIN = px(size.mainMinSize);

// The workspace re-renders whenever a pane opens, closes, resizes or (un)pins; its content
// doesn't depend on that, so it is memoized (re-rendering the arrangement or the piano roll
// on every pane change made the pane animations stutter).
const Arrangement = memo(ArrangementView);
const Markers = memo(MarkerLane);
const Rail = memo(LeftRail);
const LeftContent = memo(LeftPanel);
const InspectorContent = memo(Inspector);
const Drawer = memo(EditorDrawer);

/** Dark / light theme switch (remembered by `@/theme`). */
function ThemeToggle() {
  const [theme, setTheme] = useTheme();
  const dark = theme === "dark";
  return (
    <IconButton
      size="sm"
      tone="ghost"
      className="eth-shell__theme"
      label={dark ? "Switch to light theme" : "Switch to dark theme"}
      icon={dark ? <Sun /> : <Moon />}
      onClick={() => setTheme(dark ? "light" : "dark")}
    />
  );
}

/**
 * The workspace: the arrangement fills it; the icon rail sits on its left edge; the
 * browser (left), inspector (right) and editor drawer (bottom) float over it as cards, or,
 * pinned, also take their space (the arrangement shrinks, animated).
 */
function Workspace() {
  const left = useShellStore((s) => s.left);
  const right = useShellStore((s) => s.right);
  const bottom = useShellStore((s) => s.bottom);
  const shell = useShellStore.getState;
  const target = useInspectorTarget();
  const rightOpen = target !== null;

  // The inspector opens with the selection (the store knows, for layout and tests).
  useEffect(() => {
    if (shell().right.open !== rightOpen) shell().setOpen("right", rightOpen);
  }, [rightOpen, shell]);

  // Opening a clip (a click on a MIDI clip, a double-click on an audio clip) shows its
  // editor in the drawer.
  useEffect(
    () =>
      useEditorStore.subscribe((s, prev) => {
        // Clicked away from MIDI clips in the arrangement: an unpinned piano roll closes.
        if (s.dismissed !== prev.dismissed) {
          const { bottom } = shell();
          if (bottom.open && !bottom.pinned && bottom.tab === "piano-roll") shell().setOpen("bottom", false);
          return;
        }
        if (s.request === prev.request || !s.clip) return;
        const clip = useProjectStore.getState().project?.clips[s.clip];
        if (clip) shell().openDrawer(clip.content.type === "Midi" ? "piano-roll" : "warp");
      }),
    [shell],
  );

  // A pinned pane reserves its size plus the gap on both of its sides (it floats inset).
  const reserved = (p: { open: boolean; pinned: boolean; size: number }) => (p.open && p.pinned ? p.size + 2 * GAP : 0);
  const style = {
    "--pane-left-size": `${left.size}px`,
    "--pane-right-size": `${right.size}px`,
    "--pane-bottom-size": `${bottom.size}px`,
    "--pane-left-reserved": `${reserved(left)}px`,
    "--pane-right-reserved": `${reserved({ ...right, open: rightOpen })}px`,
    "--pane-bottom-reserved": `${reserved(bottom)}px`,
    // Room taken by open side panes, pinned or not: the drawer sits between them.
    "--pane-left-occupied": `${left.open ? left.size + GAP : 0}px`,
    "--pane-right-occupied": `${rightOpen ? right.size + GAP : 0}px`,
  } as React.CSSProperties;
  const leftLabel = LEFT_TABS.find((t) => t.id === left.tab)?.label ?? "Browser";

  return (
    <div className="eth-workspace" style={style}>
      <Rail />
      <main className="eth-workspace__main" data-slot="main">
        <div className="eth-workspace__stage">
          <div data-slot="markers">
            <Markers />
          </div>
          <Arrangement />
        </div>
      </main>

      <FloatingPane
        side="left"
        label={leftLabel}
        header={<span className="eth-float__heading">{leftLabel}</span>}
        open={left.open}
        pinned={left.pinned}
        size={left.size}
        minSize={MIN}
        maxSize={() => window.innerWidth - MAIN_MIN}
        onResize={(v) => shell().setSize("left", v)}
        onPinnedChange={(v) => shell().setPinned("left", v)}
        onClose={() => shell().setOpen("left", false)}
      >
        <div data-slot="sidebar" className="eth-float__fill">
          <LeftContent tab={left.tab} />
        </div>
      </FloatingPane>

      <FloatingPane
        side="right"
        label="Inspector"
        header={<span className="eth-float__heading">Inspector</span>}
        open={rightOpen}
        pinned={right.pinned}
        size={right.size}
        minSize={MIN}
        maxSize={() => window.innerWidth - MAIN_MIN}
        onResize={(v) => shell().setSize("right", v)}
        onPinnedChange={(v) => shell().setPinned("right", v)}
      >
        <InspectorContent target={target} />
      </FloatingPane>

      <FloatingPane
        side="bottom"
        label="Editor"
        header={<DrawerTabs />}
        open={bottom.open}
        pinned={bottom.pinned}
        size={bottom.size}
        minSize={MIN}
        maxSize={() => window.innerHeight - MAIN_MIN - px(size.topBarHeight)}
        onResize={(v) => shell().setSize("bottom", v)}
        onPinnedChange={(v) => shell().setPinned("bottom", v)}
        onClose={() => shell().setOpen("bottom", false)}
      >
        <Drawer />
      </FloatingPane>
    </div>
  );
}

export function App() {
  return (
    <div className="eth-shell">
      <header className="eth-shell__top" data-slot="top">
        <div data-slot="project">
          <ProjectMenu />
        </div>
        <div className="eth-shell__transport" data-slot="transport-bar">
          <TransportBar />
        </div>
        <div data-slot="metronome">
          <MetronomeSettings />
        </div>
        <div data-slot="recording">
          <RecordingControls />
        </div>
        <div data-slot="export">
          <ExportDialog />
        </div>
        <div data-slot="remote">
          <ConnectDialog />
        </div>
        {/* base-115: the Share button / session pill (it also hosts the collab runtime). */}
        <div data-slot="collab">
          <ShareControl />
        </div>
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-shell__theme"
          label="Settings"
          icon={<Settings />}
          onClick={() => openSettings()}
        />
        <ThemeToggle />
      </header>
      <Workspace />
      <DrawerShortcut />
      <CommandPalette />
      <AudioSettingsDialog />
      <ImportRoot />
      <MediaRefsRoot />
      <ContextMenuHost />
    </div>
  );
}

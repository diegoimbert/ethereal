/**
 * App shell: the ONLY place features are registered/mounted. Feature nodes never edit
 * this file; they edit their own `ui/src/features/<name>/` folder, whose `index.tsx`
 * export is mounted into a slot here. Owned by `foundation` (later wiring by `alpha`).
 *
 * Layout (Ableton-like):
 *   ┌ project menu │ transport bar │ recording ┐
 *   │ browser /    │ main view: arrangement ⇄ session (Tab) │
 *   │ plugins      ├─────────────────────────────────────────┤
 *   │              │ detail: devices | piano roll | automation | warp | mixer │
 *   └──────────────┴─────────────────────────────────────────┘
 *
 * Engine access: entries (`ui/src/main.tsx`, `apps/web/src/main.tsx`) wrap `<App />` in
 * `<TransportProvider transport={createDefaultTransport()}>` from `@/transport`; features
 * use `useTransport()` and read the document from `@/state`.
 */
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Button, Panel } from "@/kit";
import { useEditorStore, useProjectStore } from "@/state";
import { ArrangementView } from "@/features/arrangement";
import { AutomationLanes } from "@/features/automation";
import { Browser } from "@/features/browser";
import { DeviceChain } from "@/features/devices";
import { Mixer } from "@/features/mixer";
import { PianoRoll } from "@/features/piano-roll";
import { PluginBrowser } from "@/features/plugins";
import { ProjectMenu } from "@/features/project";
import { RecordingControls } from "@/features/recording";
import { SessionView } from "@/features/session";
import { TransportBar } from "@/features/transport-bar";
import { WarpEditor } from "@/features/warp";
import "./App.css";

interface Slot<Id extends string> {
  id: Id;
  label: string;
  render: () => ReactNode;
}

export type MainViewId = "arrangement" | "session";
export type SidebarTabId = "browser" | "plugins";
export type DetailTabId = "devices" | "piano-roll" | "automation" | "warp" | "mixer";

const MAIN_VIEWS: ReadonlyArray<Slot<MainViewId>> = [
  { id: "arrangement", label: "Arrangement", render: () => <ArrangementView /> },
  { id: "session", label: "Session", render: () => <SessionView /> },
];

const SIDEBAR_TABS: ReadonlyArray<Slot<SidebarTabId>> = [
  { id: "browser", label: "Browser", render: () => <Browser /> },
  { id: "plugins", label: "Plugins", render: () => <PluginBrowser /> },
];

const DETAIL_TABS: ReadonlyArray<Slot<DetailTabId>> = [
  { id: "devices", label: "Devices", render: () => <DeviceChain /> },
  { id: "piano-roll", label: "Piano Roll", render: () => <PianoRoll /> },
  { id: "automation", label: "Automation", render: () => <AutomationLanes /> },
  { id: "warp", label: "Warp", render: () => <WarpEditor /> },
  { id: "mixer", label: "Mixer", render: () => <Mixer /> },
];

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

function Tabs<Id extends string>({
  slots,
  active,
  onSelect,
  label,
}: {
  slots: ReadonlyArray<Slot<Id>>;
  active: Id;
  onSelect: (id: Id) => void;
  label: string;
}) {
  return (
    <div className="eth-shell__tabs" role="tablist" aria-label={label}>
      {slots.map((s) => (
        <Button
          key={s.id}
          size="sm"
          variant="ghost"
          role="tab"
          aria-selected={s.id === active}
          active={s.id === active}
          onClick={() => onSelect(s.id)}
        >
          {s.label}
        </Button>
      ))}
    </div>
  );
}

function renderActive<Id extends string>(slots: ReadonlyArray<Slot<Id>>, id: Id): ReactNode {
  return (slots.find((s) => s.id === id) ?? slots[0]!).render();
}

export function App() {
  const [mainView, setMainView] = useState<MainViewId>("arrangement");
  const [sidebarTab, setSidebarTab] = useState<SidebarTabId>("browser");
  const [detailTab, setDetailTab] = useState<DetailTabId>("devices");
  const [detailOpen, setDetailOpen] = useState(true);

  // Opening a clip (arrangement double-click) focuses its editor in the detail view.
  useEffect(
    () =>
      useEditorStore.subscribe((s, prev) => {
        if (s.request === prev.request || !s.clip) return;
        const clip = useProjectStore.getState().project?.clips[s.clip];
        if (!clip) return;
        setDetailTab(clip.content.type === "Midi" ? "piano-roll" : "warp");
        setDetailOpen(true);
      }),
    [],
  );

  const toggleMainView = useCallback(() => setMainView((v) => (v === "arrangement" ? "session" : "arrangement")), []);

  // Tab toggles Arrangement ⇄ Session (as in Ableton), unless typing in a text field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
      if (isTextEntry(e.target)) return;
      e.preventDefault();
      toggleMainView();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleMainView]);

  return (
    <div className={detailOpen ? "eth-shell" : "eth-shell eth-shell--detail-closed"} data-main-view={mainView}>
      <header className="eth-shell__top" data-slot="top">
        <div data-slot="project">
          <ProjectMenu />
        </div>
        <div className="eth-shell__transport" data-slot="transport-bar">
          <TransportBar />
        </div>
        <div data-slot="recording">
          <RecordingControls />
        </div>
      </header>

      <Panel
        className="eth-shell__sidebar"
        data-slot="sidebar"
        title={<Tabs label="Sidebar" slots={SIDEBAR_TABS} active={sidebarTab} onSelect={setSidebarTab} />}
      >
        {renderActive(SIDEBAR_TABS, sidebarTab)}
      </Panel>

      <Panel
        className="eth-shell__main"
        data-slot="main"
        title={<Tabs label="Main view" slots={MAIN_VIEWS} active={mainView} onSelect={setMainView} />}
        actions={
          <Button size="sm" variant="ghost" onClick={toggleMainView} title="Toggle Arrangement/Session (Tab)">
            ⇄ Tab
          </Button>
        }
      >
        {renderActive(MAIN_VIEWS, mainView)}
      </Panel>

      <Panel
        className="eth-shell__detail"
        data-slot="detail"
        title={<Tabs label="Detail view" slots={DETAIL_TABS} active={detailTab} onSelect={setDetailTab} />}
        actions={
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setDetailOpen((o) => !o)}
            aria-expanded={detailOpen}
            title={detailOpen ? "Hide detail view" : "Show detail view"}
          >
            {detailOpen ? "▾" : "▴"}
          </Button>
        }
      >
        {detailOpen && renderActive(DETAIL_TABS, detailTab)}
      </Panel>
    </div>
  );
}

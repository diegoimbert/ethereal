/**
 * App shell: the ONLY place features are registered/mounted. Feature nodes never edit
 * this file; they edit their own `ui/src/features/<name>/` folder, whose `index.tsx`
 * export is mounted into a slot here. Owned by `foundation` (later wiring by `alpha`).
 *
 * Layout (Ableton-like):
 *   ┌ project menu │ transport bar │ metronome │ recording │ export │ remote │ collab ┐
 *   │ browser /    │ main view: markers + arrangement        │
 *   │ plugins /    ├─────────────────────────────────────────┤
 *   │ midi         │ detail: devices | piano roll | automation | warp | mixer |       │
 *   │              │         tempo | groove | drum rack                               │
 *   └──────────────┴─────────────────────────────────────────┘
 * Roadmap v2 slots (contracts-2, docs/ROADMAP.md) are pre-mounted with placeholders.
 *
 * Engine access: entries (`ui/src/main.tsx`, `apps/web/src/main.tsx`) wrap `<App />` in
 * `<TransportProvider transport={createDefaultTransport()}>` from `@/transport`; features
 * use `useTransport()` and read the document from `@/state`.
 */
import { useEffect, useState, type ReactNode } from "react";
import { Button, Panel } from "@/kit";
import { useEditorStore, useProjectStore } from "@/state";
import { ArrangementView } from "@/features/arrangement";
import { AutomationLanes } from "@/features/automation";
import { Browser } from "@/features/browser";
import { MarkerLane } from "@/features/clip-editing";
import { PresenceBar } from "@/features/collab";
import { DeviceChain } from "@/features/devices";
import { DrumRackView } from "@/features/drum-rack";
import { ExportDialog } from "@/features/export";
import { GroovePanel } from "@/features/groove";
import { MidiLearnPanel } from "@/features/midi-learn";
import { Mixer } from "@/features/mixer";
import { PianoRoll } from "@/features/piano-roll";
import { PluginBrowser } from "@/features/plugins";
import { ProjectMenu } from "@/features/project";
import { RecordingControls } from "@/features/recording";
import { ConnectDialog } from "@/features/remote";
import { MetronomeSettings, TempoEditor } from "@/features/tempo";
import { TransportBar } from "@/features/transport-bar";
import { WarpEditor } from "@/features/warp";
import "./App.css";

interface Slot<Id extends string> {
  id: Id;
  label: string;
  render: () => ReactNode;
}

export type SidebarTabId = "browser" | "plugins" | "midi";
export type DetailTabId = "devices" | "piano-roll" | "automation" | "warp" | "mixer" | "tempo" | "groove" | "drum-rack";

const SIDEBAR_TABS: ReadonlyArray<Slot<SidebarTabId>> = [
  { id: "browser", label: "Browser", render: () => <Browser /> },
  { id: "plugins", label: "Plugins", render: () => <PluginBrowser /> },
  { id: "midi", label: "MIDI", render: () => <MidiLearnPanel /> },
];

const DETAIL_TABS: ReadonlyArray<Slot<DetailTabId>> = [
  { id: "devices", label: "Devices", render: () => <DeviceChain /> },
  { id: "piano-roll", label: "Piano Roll", render: () => <PianoRoll /> },
  { id: "automation", label: "Automation", render: () => <AutomationLanes /> },
  { id: "warp", label: "Warp", render: () => <WarpEditor /> },
  { id: "mixer", label: "Mixer", render: () => <Mixer /> },
  { id: "tempo", label: "Tempo", render: () => <TempoEditor /> },
  { id: "groove", label: "Groove", render: () => <GroovePanel /> },
  { id: "drum-rack", label: "Drum Rack", render: () => <DrumRackView /> },
];

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
          tone="ghost"
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

  return (
    <div className={detailOpen ? "eth-shell" : "eth-shell eth-shell--detail-closed"}>
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
        <div data-slot="collab">
          <PresenceBar />
        </div>
      </header>

      <Panel
        className="eth-shell__sidebar"
        data-slot="sidebar"
        title={<Tabs label="Sidebar" slots={SIDEBAR_TABS} active={sidebarTab} onSelect={setSidebarTab} />}
      >
        {renderActive(SIDEBAR_TABS, sidebarTab)}
      </Panel>

      <Panel className="eth-shell__main" data-slot="main" title="Arrangement">
        <div data-slot="markers">
          <MarkerLane />
        </div>
        <ArrangementView />
      </Panel>

      <Panel
        className="eth-shell__detail"
        data-slot="detail"
        title={<Tabs label="Detail view" slots={DETAIL_TABS} active={detailTab} onSelect={setDetailTab} />}
        actions={
          <Button
            size="sm"
            tone="ghost"
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

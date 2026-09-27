import clsx from "clsx";
import { Command } from "lucide-react";
import { IconButton, MOD_KEY } from "@/kit";
import { Browser } from "@/features/browser";
import { MidiLearnPanel } from "@/features/midi-learn";
import { PluginBrowser } from "@/features/plugins";
import { DevicesPanel } from "./DevicesPanel";
import { useShellStore, type LeftTab } from "./shellStore";
import { LEFT_TABS } from "./tabs";
import "./leftPanels.css";

/**
 * The icon rail on the left edge: each icon opens its panel (or closes it when it is
 * already showing). The last button opens the command palette.
 */
export function LeftRail() {
  const left = useShellStore((s) => s.left);
  const toggle = useShellStore((s) => s.toggleLeft);
  const setPalette = useShellStore((s) => s.setPalette);
  return (
    <nav className="eth-rail" aria-label="Panels">
      {LEFT_TABS.map((t) => (
        <IconButton
          key={t.id}
          size="lg"
          tone="ghost"
          className={clsx("eth-rail__button", left.open && left.tab === t.id && "eth-rail__button--active")}
          label={t.label}
          active={left.open && left.tab === t.id}
          icon={t.icon}
          onClick={() => toggle(t.id)}
        />
      ))}
      <span className="eth-rail__spacer" />
      <IconButton
        size="lg"
        tone="ghost"
        className="eth-rail__button"
        label="Command palette"
        title={`Command palette (${MOD_KEY}K)`}
        icon={<Command />}
        onClick={() => setPalette(true)}
      />
    </nav>
  );
}

/** Content of the left pane for the rail tab (keyed, so each tab keeps its own state). */
export function LeftPanel({ tab }: { tab: LeftTab }) {
  switch (tab) {
    case "library":
      return <Browser key="library" scope="library" />;
    case "project":
      return <Browser key="project" scope="project" />;
    case "plugins":
      return <PluginBrowser />;
    case "midi":
      return <MidiLearnPanel />;
    case "devices":
      return <DevicesPanel />;
  }
}

import { History, Library, SlidersHorizontal, Sparkles } from "lucide-react";
import type { ReactNode } from "react";
import { AutomationLanes } from "@/features/automation";
import { EditedClipControllerLanes } from "@/features/expression";
import { PianoRoll } from "@/features/piano-roll";
import { WarpEditor } from "@/features/warp";
import { contributions, type LeftTabContribution } from "@/ext";
import { t as tr, type MessageKey } from "@/i18n";
import type { BuiltinLeftTab, DrawerTab, LeftTab } from "@/state/shell";

/** Panels of the left rail. */
export const LEFT_TABS: ReadonlyArray<{ id: BuiltinLeftTab; label: MessageKey; icon: ReactNode }> = [
  { id: "library", label: "shell.tab.library", icon: <Library /> },
  { id: "midi", label: "shell.tab.midi", icon: <SlidersHorizontal /> },
  { id: "history", label: "shell.tab.history", icon: <History /> },
  // ai-chat: Claude edits the project through the agent API.
  { id: "ai", label: "shell.tab.ai", icon: <Sparkles /> },
];

/** A rail tab as shown: the app's, or an extension's (`@/ext` "shell.leftTab"). */
export interface RailTab {
  id: LeftTab;
  label(): string;
  icon: ReactNode;
  /** An extension's tab: its panel and visibility. */
  contributed?: LeftTabContribution;
}

/** The rail's tabs: the app's, with the extensions' placed in (`before`, default before "ai"). */
export function railTabs(contributed: ReadonlyArray<LeftTabContribution> = contributions("shell.leftTab")): RailTab[] {
  const out: RailTab[] = LEFT_TABS.map((x) => ({ id: x.id, label: () => tr(x.label), icon: x.icon }));
  for (const c of contributed) {
    const at = out.findIndex((x) => x.id === (c.before ?? "ai"));
    out.splice(at < 0 ? out.length : at, 0, { id: c.id, label: c.label, icon: c.icon, contributed: c });
  }
  return out;
}

/** Editors of the bottom drawer. */
export const DRAWER_TABS: ReadonlyArray<{ id: DrawerTab; label: MessageKey; render: () => ReactNode }> = [
  { id: "piano-roll", label: "shell.tab.piano_roll", render: () => <PianoRoll /> },
  { id: "warp", label: "shell.tab.warp", render: () => <WarpEditor /> },
  {
    id: "automation",
    label: "shell.tab.automation",
    render: () => <AutomationLanes clipLanes={(view, headerWidth) => <EditedClipControllerLanes view={view} headerWidth={headerWidth} />} />,
  },
];

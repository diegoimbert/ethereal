import { History, Library, SlidersHorizontal, Sparkles } from "lucide-react";
import type { ReactNode } from "react";
import { AutomationLanes } from "@/features/automation";
import { PianoRoll } from "@/features/piano-roll";
import { WarpEditor } from "@/features/warp";
import type { MessageKey } from "@/i18n";
import type { DrawerTab, LeftTab } from "@/state/shell";

/** Panels of the left rail. */
export const LEFT_TABS: ReadonlyArray<{ id: LeftTab; label: MessageKey; icon: ReactNode }> = [
  { id: "library", label: "shell.tab.library", icon: <Library /> },
  { id: "midi", label: "shell.tab.midi", icon: <SlidersHorizontal /> },
  { id: "history", label: "shell.tab.history", icon: <History /> },
  // ai-chat: Claude edits the project through the agent API.
  { id: "ai", label: "shell.tab.ai", icon: <Sparkles /> },
];

/** Editors of the bottom drawer. */
export const DRAWER_TABS: ReadonlyArray<{ id: DrawerTab; label: MessageKey; render: () => ReactNode }> = [
  { id: "piano-roll", label: "shell.tab.piano_roll", render: () => <PianoRoll /> },
  { id: "warp", label: "shell.tab.warp", render: () => <WarpEditor /> },
  { id: "automation", label: "shell.tab.automation", render: () => <AutomationLanes /> },
];

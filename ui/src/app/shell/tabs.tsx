import { Boxes, FolderOpen, Library, MessageSquare, Plug, SlidersHorizontal, Sparkles } from "lucide-react";
import type { ReactNode } from "react";
import { AutomationLanes } from "@/features/automation";
import { DrumRackView } from "@/features/drum-rack";
import { GroovePanel } from "@/features/groove";
import { PianoRoll } from "@/features/piano-roll";
import { TempoEditor } from "@/features/tempo";
import { WarpEditor } from "@/features/warp";
import type { DrawerTab, LeftTab } from "./shellStore";

/** Panels of the left rail (`session`: only shown in a collaboration session). */
export const LEFT_TABS: ReadonlyArray<{ id: LeftTab; label: string; icon: ReactNode; session?: boolean }> = [
  { id: "library", label: "Library", icon: <Library /> },
  { id: "project", label: "Project media", icon: <FolderOpen /> },
  { id: "plugins", label: "Plugins", icon: <Plug /> },
  { id: "devices", label: "Devices", icon: <Boxes /> },
  { id: "midi", label: "MIDI mapping", icon: <SlidersHorizontal /> },
  { id: "chat", label: "Chat", icon: <MessageSquare />, session: true },
  // ai-chat: Claude edits the project through the agent API.
  { id: "ai", label: "Ask AI", icon: <Sparkles /> },
];

/** Editors of the bottom drawer. */
export const DRAWER_TABS: ReadonlyArray<{ id: DrawerTab; label: string; render: () => ReactNode }> = [
  { id: "piano-roll", label: "Piano Roll", render: () => <PianoRoll /> },
  { id: "warp", label: "Warp", render: () => <WarpEditor /> },
  { id: "automation", label: "Automation", render: () => <AutomationLanes /> },
  { id: "tempo", label: "Tempo", render: () => <TempoEditor /> },
  { id: "groove", label: "Groove", render: () => <GroovePanel /> },
  { id: "drum-rack", label: "Drum Rack", render: () => <DrumRackView /> },
];


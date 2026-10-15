import clsx from "clsx";
import { Command } from "lucide-react";
import { useT } from "@/i18n";
import { IconButton } from "@/kit";
import { useShortcutLabel } from "@/features/keymap";
import { AiChatPanel, useAiChatShortcut } from "@/features/ai-chat";
import { Browser } from "@/features/browser";
import { MidiLearnPanel } from "@/features/midi-learn";
import { HistoryPanel } from "@/features/undo-history";
import { useShellStore, type LeftTab } from "@/state/shell";
import { LEFT_TABS } from "./tabs";
import "./leftPanels.css";

/**
 * The icon rail on the left edge: each icon opens its panel (or closes it when it is
 * already showing). The last button opens the command palette.
 */
export function LeftRail() {
  const t = useT();
  const left = useShellStore((s) => s.left);
  const toggle = useShellStore((s) => s.toggleLeft);
  const setPalette = useShellStore((s) => s.setPalette);
  const paletteKey = useShortcutLabel("palette.open");
  // ai-chat: Mod+Shift+A opens the AI chat.
  useAiChatShortcut();
  return (
    <nav className="eth-rail" aria-label={t("shell.rail.label")}>
      {LEFT_TABS.map((tab) => (
        <IconButton
          key={tab.id}
          size="lg"
          tone="ghost"
          className={clsx("eth-rail__button", left.open && left.tab === tab.id && "eth-rail__button--active")}
          label={t(tab.label)}
          active={left.open && left.tab === tab.id}
          icon={tab.icon}
          onClick={() => toggle(tab.id)}
        />
      ))}
      <span className="eth-rail__spacer" />
      <IconButton
        size="lg"
        tone="ghost"
        className="eth-rail__button"
        label={t("shell.action.palette_open")}
        title={paletteKey ? t("shell.rail.palette_key", { key: paletteKey }) : t("shell.action.palette_open")}
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
    case "midi":
      return <MidiLearnPanel />;
    case "history":
      return <HistoryPanel />;
    case "ai":
      return <AiChatPanel />;
  }
}

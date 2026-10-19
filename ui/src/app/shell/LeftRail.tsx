import clsx from "clsx";
import { useEffect } from "react";
import { Command } from "lucide-react";
import { useT } from "@/i18n";
import { IconButton } from "@/kit";
import { useShortcutLabel } from "@/features/keymap";
import { AiChatPanel, useAiChatShortcut } from "@/features/ai-chat";
import { Browser } from "@/features/browser";
import { MidiLearnPanel } from "@/features/midi-learn";
import { HistoryPanel } from "@/features/undo-history";
import { contributions, useContributions } from "@/ext";
import { useShellStore, type LeftTab } from "@/state/shell";
import { railTabs, type RailTab } from "./tabs";
import "./leftPanels.css";

/**
 * The icon rail on the left edge: each icon opens its panel (or closes it when it is
 * already showing). The last button opens the command palette. Extensions add tabs
 * (`@/ext` "shell.leftTab"), which may show only at times.
 */
export function LeftRail() {
  const t = useT();
  const setPalette = useShellStore((s) => s.setPalette);
  const paletteKey = useShortcutLabel("palette.open");
  const tabs = railTabs(useContributions("shell.leftTab"));
  // ai-chat: Mod+Shift+A opens the AI chat.
  useAiChatShortcut();
  return (
    <nav className="eth-rail" aria-label={t("shell.rail.label")}>
      {tabs.map((tab) => (tab.contributed?.useVisible ? <ContributedRailButton key={tab.id} tab={tab} /> : <RailButton key={tab.id} tab={tab} />))}
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

function RailButton({ tab }: { tab: RailTab }) {
  const left = useShellStore((s) => s.left);
  const toggle = useShellStore((s) => s.toggleLeft);
  return (
    <IconButton
      size="lg"
      tone="ghost"
      className={clsx("eth-rail__button", left.open && left.tab === tab.id && "eth-rail__button--active")}
      label={tab.label()}
      active={left.open && left.tab === tab.id}
      icon={tab.icon}
      onClick={() => toggle(tab.id)}
    />
  );
}

/** An extension's tab that shows only at times: its pane closes when it stops showing. */
function ContributedRailButton({ tab }: { tab: RailTab }) {
  const visible = tab.contributed!.useVisible!();
  useEffect(() => {
    const { left, setOpen } = useShellStore.getState();
    if (!visible && left.open && left.tab === tab.id) setOpen("left", false);
  }, [visible, tab.id]);
  return visible ? <RailButton tab={tab} /> : null;
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
  const Contributed = contributions("shell.leftTab").find((c) => c.id === tab)?.Component;
  return Contributed ? <Contributed /> : null;
}

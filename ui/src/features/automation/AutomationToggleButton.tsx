import clsx from "clsx";
import type { ReactNode } from "react";
import type { TrackId } from "@/generated";
import { useAutomationToggle } from "./toggle";
import { useT } from "@/i18n";

export interface AutomationToggleButtonProps {
  trackId: TrackId;
  trackName: string;
  className?: string;
  /** Button content (default: a text label). */
  children?: ReactNode;
}

/** Shows / hides a track's automation lanes (`aria-label` "show/hide automation of <track>"). */
export function AutomationToggleButton({ trackId, trackName, className, children }: AutomationToggleButtonProps) {
  const { open, count, toggle } = useAutomationToggle(trackId);
  const t = useT();
  return (
    <button
      type="button"
      className={clsx(className ?? "eth-auto-toggle", count > 0 && "eth-auto-toggle--has")}
      aria-pressed={open}
      aria-expanded={open}
      aria-label={open ? t("automation.toggle.hide", { name: trackName }) : t("automation.toggle.show", { name: trackName })}
      title={t("automation.toggle.title", { count })}
      data-count={count}
      onClick={toggle}
    >
      {children ?? t("automation.toggle.text", { count })}
    </button>
  );
}

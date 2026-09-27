import clsx from "clsx";
import type { ReactNode } from "react";
import type { TrackId } from "@/generated";
import { useAutomationToggle } from "./toggle";

export interface AutomationToggleButtonProps {
  trackId: TrackId;
  trackName: string;
  className?: string;
  /** Button content (default: a text label). */
  children?: ReactNode;
}

/** Shows / hides a track's automation lanes (`aria-label` "Show/Hide automation of <track>"). */
export function AutomationToggleButton({ trackId, trackName, className, children }: AutomationToggleButtonProps) {
  const { open, count, toggle } = useAutomationToggle(trackId);
  return (
    <button
      type="button"
      className={clsx(className ?? "eth-auto-toggle", count > 0 && "eth-auto-toggle--has")}
      aria-pressed={open}
      aria-expanded={open}
      aria-label={`${open ? "Hide" : "Show"} automation of ${trackName}`}
      title={count > 0 ? `Automation (${count} lane${count > 1 ? "s" : ""})` : "Automation"}
      data-count={count}
      onClick={toggle}
    >
      {children ?? `Automation${count > 0 ? ` (${count})` : ""}`}
    </button>
  );
}

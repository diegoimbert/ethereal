import clsx from "clsx";
import { Layers } from "lucide-react";
import type { Track } from "@/generated";
import { useProjectStore } from "@/state";
import { useCompingUi } from "./store";

/**
 * Track-header button showing / hiding the track's take lanes, with the number of takes.
 * Only on tracks that have takes (new lanes come from recording or the header's menu).
 */
export function TakesToggle({ track, className }: { track: Track; className?: string }) {
  const count = useProjectStore((s) => {
    let n = 0;
    for (const l of Object.values(s.project?.take_lanes ?? {})) if (l.track === track.id) n++;
    return n;
  });
  const open = useCompingUi((s) => s.expanded.has(track.id));
  if (count === 0) return null;
  return (
    <button
      type="button"
      className={clsx(className, "eth-takes-toggle")}
      aria-pressed={open}
      aria-label={open ? `Hide takes of ${track.name}` : `Show takes of ${track.name}`}
      title={`${count} take${count === 1 ? "" : "s"} (${open ? "hide" : "show"} take lanes)`}
      onClick={(e) => {
        e.stopPropagation();
        useCompingUi.getState().toggle(track.id);
      }}
    >
      <Layers />
      <span className="eth-takes-toggle__count" aria-hidden>
        {count}
      </span>
    </button>
  );
}

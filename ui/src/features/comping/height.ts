/** Take-lane heights for the arrangement's row layout (v0.2, `comping`). */

import { useMemo } from "react";
import type { TakeLane, Track } from "@/generated";
import { useProjectStore } from "@/state";
import { lanesOf } from "./model";
import { useCompingUi } from "./store";

/** Height of one take lane (px). */
export const TAKE_LANE_HEIGHT = 44;
const NO_LANES: TakeLane[] = [];

/** The take lanes shown under `track` (none when collapsed or without lanes). */
export function useShownLanes(track: Track): TakeLane[] {
  const expanded = useCompingUi((s) => s.expanded.has(track.id));
  const lanes = useProjectStore((s) => s.project?.take_lanes);
  return useMemo(() => (expanded && lanes ? lanesOf({ take_lanes: lanes }, track.id) : NO_LANES), [expanded, lanes, track.id]);
}

/** Height of the take-lane section of each track (fed to the arrangement's row layout). */
export function useTakesHeight(): (track: string) => number {
  const expanded = useCompingUi((s) => s.expanded);
  const lanes = useProjectStore((s) => s.project?.take_lanes);
  return useMemo(() => {
    const counts = new Map<string, number>();
    for (const l of Object.values(lanes ?? {})) if (expanded.has(l.track)) counts.set(l.track, (counts.get(l.track) ?? 0) + 1);
    return (track: string) => (counts.get(track) ?? 0) * TAKE_LANE_HEIGHT;
  }, [expanded, lanes]);
}


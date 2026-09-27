import { useMemo } from "react";
import type { AutomationLane, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { targetKey } from "./params";
import { useAutomationUi } from "./uiStore";

const EMPTY_LANES: Readonly<Record<string, AutomationLane>> = {};

/** Arrangement lanes of a track, by target key. */
export function useTrackLanes(track: TrackId): ReadonlyMap<string, AutomationLane> {
  const lanes = useProjectStore((s) => s.project?.automation_lanes ?? EMPTY_LANES);
  return useMemo(() => {
    const m = new Map<string, AutomationLane>();
    for (const l of Object.values(lanes)) if (l.owner.type === "Track" && l.owner.track === track) m.set(targetKey(l.target), l);
    return m;
  }, [lanes, track]);
}

/**
 * Open/close state of a track's automation lanes, for a toggle button: `count` is the
 * number of automation lanes the track has (to hint that it is automated).
 */
export function useAutomationToggle(trackId: TrackId): { open: boolean; count: number; toggle: () => void } {
  const open = useAutomationUi((s) => s.open.has(trackId));
  const lanes = useTrackLanes(trackId);
  const toggle = () => {
    const initial = lanes.size > 0 ? [...lanes.keys()] : [targetKey({ type: "TrackVolume", track: trackId })];
    useAutomationUi.getState().setOpen(trackId, !open, initial);
  };
  return { open, count: lanes.size, toggle };
}


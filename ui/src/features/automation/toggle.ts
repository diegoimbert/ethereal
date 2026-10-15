import { useMemo } from "react";
import type { AutomationLane, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { targetKey } from "./params";
import { TEMPO_LANE_KEY, useAutomationUi } from "./uiStore";

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
 * number of automation lanes the track has (to hint that it is automated). The first time
 * a track opens it shows its automated lanes (else its volume); the master shows its
 * tempo lane first (and its automated lanes, if any).
 */
export function useAutomationToggle(trackId: TrackId): { open: boolean; count: number; toggle: () => void } {
  const open = useAutomationUi((s) => s.open.has(trackId));
  const lanes = useTrackLanes(trackId);
  const isMaster = useProjectStore((s) => s.project?.tracks[trackId]?.kind === "Master");
  const toggle = () => {
    const initial = isMaster ? [TEMPO_LANE_KEY, ...lanes.keys()] : lanes.size > 0 ? [...lanes.keys()] : [targetKey({ type: "TrackVolume", track: trackId })];
    useAutomationUi.getState().setOpen(trackId, !open, initial);
  };
  return { open, count: lanes.size, toggle };
}

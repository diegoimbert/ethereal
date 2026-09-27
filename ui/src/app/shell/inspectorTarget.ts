import { useMemo } from "react";
import type { ClipId, TrackId } from "@/generated";
import { useArrangementUi } from "@/features/arrangement/state";
import { useSelectedItems } from "@/timeline";

/** What the inspector shows: the selected clips, or the selected track. */
export type InspectorTarget = { kind: "clips"; ids: ClipId[] } | { kind: "track"; id: TrackId };

/** The inspector's subject, or null when nothing is selected (the inspector hides). */
export function useInspectorTarget(): InspectorTarget | null {
  const clips = useSelectedItems("clip");
  const track = useArrangementUi((s) => s.trackFocus);
  // Stable while the selection is (the inspector content is memoized on it).
  return useMemo<InspectorTarget | null>(() => {
    if (clips.size > 0) return { kind: "clips", ids: [...clips] };
    if (track) return { kind: "track", id: track };
    return null;
  }, [clips, track]);
}

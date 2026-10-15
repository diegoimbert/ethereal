import { useEffect } from "react";
import { DEFAULT_ZOOM_LIMITS } from "@/timeline";
import { useProjectStore } from "@/state";
import { arrangementView } from "./uiStore";

/** The widest the arrangement zooms out to, in beats, however short the song (32 bars of 4/4). */
const MIN_SPAN_BEATS = 128;
/** Room past the song's end at the widest zoom (a fraction of the song's length). */
const END_ROOM = 0.5;

/** Where the song ends: its last clip or marker (0 when empty). */
function contentEnd(): number {
  const p = useProjectStore.getState().project;
  if (!p) return 0;
  let end = 0;
  for (const c of Object.values(p.clips)) end = Math.max(end, c.start + c.length);
  for (const m of Object.values(p.markers)) end = Math.max(end, m.position);
  return end;
}

/**
 * Zooming out stops a little past the song's end (at least `MIN_SPAN_BEATS` wide): with
 * nothing past a point, the arrangement doesn't shrink into a sliver of an empty timeline.
 */
export function useZoomOutLimit(): void {
  useEffect(() => {
    let end = -1;
    let width = -1;
    const update = () => {
      const e = contentEnd();
      const w = arrangementView.getState().widthPx;
      if (e === end && w === width) return;
      end = e;
      width = w;
      if (w <= 0) return;
      const span = Math.max(MIN_SPAN_BEATS, e * (1 + END_ROOM));
      arrangementView.getState().setLimits({ minPxPerBeat: Math.max(DEFAULT_ZOOM_LIMITS.minPxPerBeat, w / span) });
    };
    update();
    const offProject = useProjectStore.subscribe(update);
    const offView = arrangementView.subscribe(update);
    return () => {
      offProject();
      offView();
      arrangementView.getState().setLimits({ minPxPerBeat: DEFAULT_ZOOM_LIMITS.minPxPerBeat });
    };
  }, []);
}

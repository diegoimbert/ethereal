/**
 * Lane geometry that follows zoom and scroll without React.
 *
 * Lane content (clips, previews, markers of a lane) is positioned in beats with CSS:
 * `x = (beats - origin) * var(--ppb)`, where `LaneLayer` writes the live zoom into `--ppb`
 * and the scroll into a transform on every view change. So an animated zoom or pan costs one
 * style write per lane per frame instead of re-rendering every clip.
 *
 * Components still need a pixels-per-beat for pixel math (canvases, drag deltas):
 * `useSettledZoom` gives the zoom React renders with. It follows the live zoom once it has
 * been still for `SETTLE_MS`, or at once when the live zoom drifts past `MAX_DRIFT` (so
 * stretched canvases never get too blurry). At rest it is exact.
 */

import { useSyncExternalStore } from "react";
import type { Beats } from "@/generated";
import type { TimelineViewStore } from "@/timeline";

export const SETTLE_MS = 150;
export const MAX_DRIFT = 1.5;

interface Settled {
  value: number;
  listeners: Set<() => void>;
}

const settled = new WeakMap<TimelineViewStore, Settled>();

function settledOf(view: TimelineViewStore): Settled {
  let z = settled.get(view);
  if (z) return z;
  const state: Settled = { value: view.getState().pxPerBeat, listeners: new Set() };
  let timer: ReturnType<typeof setTimeout> | null = null;
  const publish = () => {
    timer = null;
    const v = view.getState().pxPerBeat;
    if (v === state.value) return;
    state.value = v;
    for (const l of state.listeners) l();
  };
  view.subscribe((s) => {
    if (s.pxPerBeat === state.value) return;
    const drift = s.pxPerBeat / state.value;
    if (timer) clearTimeout(timer);
    if (drift > MAX_DRIFT || drift < 1 / MAX_DRIFT) publish();
    else timer = setTimeout(publish, SETTLE_MS);
  });
  settled.set(view, (z = state));
  return z;
}

/** The zoom (px per beat) lanes render with (see the module doc). */
export function useSettledZoom(view: TimelineViewStore): number {
  const z = settledOf(view);
  return useSyncExternalStore(
    (cb) => {
      z.listeners.add(cb);
      return () => z.listeners.delete(cb);
    },
    () => z.value,
  );
}

/** CSS length of a beat offset in a lane layer: `offset * var(--ppb)`. */
export function beatsCss(beats: Beats): string {
  return `calc(${beats} * var(--ppb))`;
}

/** CSS width of a beat length, at least `minPx` wide. */
export function widthCss(beats: Beats, minPx = 0): string {
  return minPx > 0 ? `max(${minPx}px, calc(${beats} * var(--ppb)))` : beatsCss(beats);
}

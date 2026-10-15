/**
 * The settled zoom of a timeline view: the zoom content is laid out with while the live
 * zoom animates. It follows the live zoom once it has been still for `SETTLE_MS`, or at once
 * when the live zoom drifts past `MAX_DRIFT`. In between, views stretch what they drew at
 * the settled zoom with a composited transform (`scaleX(live / settled)`), so an animated
 * zoom doesn't re-lay out or redraw everything on every frame. At rest it is exact.
 *
 * Two tiers: the one styles and canvases follow (`settledZoom`, cheap to apply: a CSS
 * variable, a canvas redraw) keeps the stretch small (`MAX_DRIFT`), the one React renders
 * lanes with (`useSettledZoom`) lets it grow to `RENDER_DRIFT`: a wheel step zooms by ~1.2,
 * so a continuous zoom crossed `MAX_DRIFT` at nearly every step, and re-rendering every lane
 * that often dropped frames. Lane content is placed in beats, so it stays in place either way.
 */

import { startTransition, useEffect, useState } from "react";
import type { TimelineViewStore } from "./viewStore";

export const SETTLE_MS = 150;
/** Largest stretch of lane content before it is laid out again at the live zoom. */
export const MAX_DRIFT = 1.25;
/** Largest drift before lanes re-render (their render window covers zooming out by 2). */
export const RENDER_DRIFT = 2;

interface Settled {
  value: number;
  listeners: Set<() => void>;
}

const settled = new Map<number, WeakMap<TimelineViewStore, Settled>>();

function settledOf(view: TimelineViewStore, maxDrift = MAX_DRIFT): Settled {
  let tier = settled.get(maxDrift);
  if (!tier) settled.set(maxDrift, (tier = new WeakMap()));
  let z = tier.get(view);
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
    if (drift > maxDrift || drift < 1 / maxDrift) publish();
    else timer = setTimeout(publish, SETTLE_MS);
  });
  tier.set(view, (z = state));
  return z;
}

/** The settled zoom now (outside React). */
export function settledZoom(view: TimelineViewStore): number {
  return settledOf(view).value;
}

/** Listen to settled-zoom changes (outside React); returns the unsubscribe. */
export function subscribeSettledZoom(view: TimelineViewStore, listener: () => void): () => void {
  const z = settledOf(view);
  z.listeners.add(listener);
  return () => z.listeners.delete(listener);
}

/**
 * The zoom (px per beat) lanes render with (see the module doc: it may lag `settledZoom`
 * mid-zoom, up to `RENDER_DRIFT`; both agree at rest). Updates are React
 * transitions: re-rendering every lane at a new settled zoom is time-sliced instead of one
 * long task (the lanes already show the new zoom meanwhile: `--ppb` and the stretch are
 * applied outside React, and their canvases are placed in beats).
 */
export function useSettledZoom(view: TimelineViewStore): number {
  const z = settledOf(view, RENDER_DRIFT);
  const [value, setValue] = useState(z.value);
  useEffect(() => {
    const update = () => startTransition(() => setValue(z.value));
    z.listeners.add(update);
    // Settled between the render and this effect.
    update();
    return () => {
      z.listeners.delete(update);
    };
  }, [z]);
  return value;
}

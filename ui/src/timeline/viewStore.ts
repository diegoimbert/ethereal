/**
 * Zoom/scroll store of one timeline view (arrangement, piano roll, automation...).
 *
 * Each view creates its own store with `createTimelineViewStore()` (they don't share zoom)
 * and passes it to the timeline components (`Ruler`, `PlayheadLine`, ...). Read it in React
 * with `useTimelineView(store, selector)` or imperatively with `store.getState()` (canvas).
 * Views that must scroll together (e.g. arrangement lanes + ruler) share one store.
 */

import { useMemo } from "react";
import { createStore, type StoreApi } from "zustand/vanilla";
import { useStore } from "zustand";
import type { BeatRange, Beats } from "@/generated";
import {
  clampViewport,
  DEFAULT_ZOOM_LIMITS,
  revealBeats,
  visibleRange,
  zoomBy,
  zoomTo,
  zoomToRange,
  type TimelineViewport,
  type ZoomLimits,
} from "./viewport";

export interface TimelineViewState extends TimelineViewport {
  /** Width of the content area in px (set by the view, e.g. from a ResizeObserver). */
  widthPx: number;
  limits: ZoomLimits;
  /** Auto-scroll to keep the playhead visible while playing. */
  followPlayhead: boolean;

  setWidth(widthPx: number): void;
  /** Set the viewport (clamped). */
  setViewport(vp: Partial<TimelineViewport>): void;
  /** Scroll so `beats` is at the left edge. */
  scrollTo(beats: Beats): void;
  /** Scroll by a pixel delta (positive = later in time). */
  scrollByPx(dx: number): void;
  /** Set zoom keeping the beat under `anchorPx` fixed (default: the left edge). */
  zoomTo(pxPerBeat: number, anchorPx?: number): void;
  /** Multiply the zoom by `factor` around `anchorPx` (default: the view center). */
  zoomBy(factor: number, anchorPx?: number): void;
  /** Fit a beat range into the view. */
  zoomToRange(range: BeatRange, paddingPx?: number): void;
  /** Scroll the minimum needed for `beats` to be visible with `marginPx` from the edges. */
  reveal(beats: Beats, marginPx?: number): void;
  setFollowPlayhead(follow: boolean): void;
  /** Currently visible beat range. */
  visibleRange(): BeatRange;
}

export type TimelineViewStore = StoreApi<TimelineViewState>;

export interface TimelineViewOptions {
  pxPerBeat?: number;
  scrollBeats?: number;
  widthPx?: number;
  limits?: Partial<ZoomLimits>;
  followPlayhead?: boolean;
}

export function createTimelineViewStore(opts: TimelineViewOptions = {}): TimelineViewStore {
  const limits: ZoomLimits = { ...DEFAULT_ZOOM_LIMITS, ...opts.limits };
  const initial = clampViewport({ pxPerBeat: opts.pxPerBeat ?? 20, scrollBeats: opts.scrollBeats ?? 0 }, limits);

  return createStore<TimelineViewState>()((set, get) => {
    const vp = (): TimelineViewport => {
      const s = get();
      return { pxPerBeat: s.pxPerBeat, scrollBeats: s.scrollBeats };
    };
    const apply = (next: TimelineViewport) => {
      const s = get();
      if (next.pxPerBeat !== s.pxPerBeat || next.scrollBeats !== s.scrollBeats) {
        set({ pxPerBeat: next.pxPerBeat, scrollBeats: next.scrollBeats });
      }
    };
    return {
      ...initial,
      widthPx: opts.widthPx ?? 0,
      limits,
      followPlayhead: opts.followPlayhead ?? true,

      setWidth(widthPx) {
        if (widthPx !== get().widthPx) set({ widthPx });
      },
      setViewport(partial) {
        apply(clampViewport({ ...vp(), ...partial }, get().limits));
      },
      scrollTo(beats) {
        apply(clampViewport({ ...vp(), scrollBeats: beats }, get().limits));
      },
      scrollByPx(dx) {
        const v = vp();
        apply(clampViewport({ ...v, scrollBeats: v.scrollBeats + dx / v.pxPerBeat }, get().limits));
      },
      zoomTo(pxPerBeat, anchorPx = 0) {
        apply(zoomTo(vp(), pxPerBeat, anchorPx, get().limits));
      },
      zoomBy(factor, anchorPx) {
        apply(zoomBy(vp(), factor, anchorPx ?? get().widthPx / 2, get().limits));
      },
      zoomToRange(range, paddingPx = 0) {
        apply(zoomToRange(range, get().widthPx, paddingPx, get().limits));
      },
      reveal(beats, marginPx = 0) {
        apply(revealBeats(vp(), beats, get().widthPx, marginPx, get().limits));
      },
      setFollowPlayhead(followPlayhead) {
        set({ followPlayhead });
      },
      visibleRange() {
        return visibleRange(vp(), get().widthPx);
      },
    };
  });
}

/** Subscribe a component to part of a view store. */
export function useTimelineView<T>(store: TimelineViewStore, selector: (s: TimelineViewState) => T): T {
  return useStore(store, selector);
}

/** The viewport (`pxPerBeat` + `scrollBeats`) of a view store; re-renders on zoom/scroll. */
export function useViewport(store: TimelineViewStore): TimelineViewport {
  const pxPerBeat = useStore(store, (s) => s.pxPerBeat);
  const scrollBeats = useStore(store, (s) => s.scrollBeats);
  return useMemo(() => ({ pxPerBeat, scrollBeats }), [pxPerBeat, scrollBeats]);
}

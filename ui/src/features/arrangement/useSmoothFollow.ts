import { useEffect } from "react";
import { playheadStore } from "@/state";
import { beatsToPx, shiftView, type TimelineViewStore } from "@/timeline";

/** Where the followed playhead sits, as a fraction of the view width from the left edge. */
export const FOLLOW_AT = 0.4;

/**
 * Smooth follow (owner request): while playing with `followPlayhead` on, the playhead moves
 * across the view until it reaches `FOLLOW_AT` of its width; past it, the view scrolls by
 * as much as the playhead advanced since the last frame (`prev`), so the playhead keeps its
 * place on screen (no paging) and a zoom while playing (which moves the view around the
 * pointer) is kept, not undone. Only a playhead outside the view (a loop restart, a locate)
 * makes the view jump, to put it at the follow point. Returns the new scroll (beats), or null
 * to leave the view as it is.
 */
export function followScroll(s: { pxPerBeat: number; scrollBeats: number; widthPx: number }, beats: number, prev: number | null): number | null {
  if (s.widthPx <= 0) return null;
  const x = beatsToPx(beats, s);
  const at = s.widthPx * FOLLOW_AT;
  if (x < 0 || x > s.widthPx) return Math.max(0, beats - at / s.pxPerBeat);
  if (x <= at || prev === null || beats <= prev) return null;
  return s.scrollBeats + (beats - prev);
}

export function useSmoothFollow(view: TimelineViewStore): void {
  useEffect(() => {
    let prev: number | null = null;
    return playheadStore.subscribePlayhead(() => {
      const frame = playheadStore.getPlayhead();
      const s = view.getState();
      if (!frame?.transport.playing || !s.followPlayhead) {
        prev = null;
        return;
      }
      const beats = frame.transport.position;
      const to = followScroll(s, beats, prev);
      const advance = prev !== null && beats > prev ? beats - prev : null;
      prev = beats;
      if (to === null) return;
      // Following: shift with the playhead (a zoom in progress keeps going); else jump.
      if (advance !== null && Math.abs(to - (s.scrollBeats + advance)) < 1e-9) shiftView(view, advance);
      else s.scrollTo(to);
    });
  }, [view]);
}

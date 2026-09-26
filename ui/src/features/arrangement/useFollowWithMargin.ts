import { useEffect } from "react";
import { playheadStore } from "@/state";
import { beatsToPx, type TimelineViewStore } from "@/timeline";

/** Margin kept between the playhead and the right edge before the view pages. */
export function followMargin(widthPx: number): number {
  return Math.min(80, widthPx * 0.1);
}

/**
 * Follow the playhead with a margin: while playing with `followPlayhead` on, once the
 * playhead gets within `followMargin` of the right edge (or leaves the view), page so it
 * sits `followMargin` from the left edge. The Ruler's own follow (which pages only once the
 * playhead is past the edge) then never triggers.
 */
export function followScroll(
  s: { pxPerBeat: number; scrollBeats: number; widthPx: number },
  beats: number,
): number | null {
  if (s.widthPx <= 0) return null;
  const x = beatsToPx(beats, s);
  const margin = followMargin(s.widthPx);
  if (x >= 0 && x <= s.widthPx - margin) return null;
  return Math.max(0, beats - margin / s.pxPerBeat);
}

export function useFollowWithMargin(view: TimelineViewStore): void {
  useEffect(
    () =>
      playheadStore.subscribePlayhead(() => {
        const frame = playheadStore.getPlayhead();
        const s = view.getState();
        if (!frame?.transport.playing || !s.followPlayhead) return;
        const to = followScroll(s, frame.transport.position);
        if (to !== null) s.scrollTo(to);
      }),
    [view],
  );
}

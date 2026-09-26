/**
 * Playhead positioning hooks (`PlayheadLine` is the ready-made line). The playhead moves
 * at ~60 Hz, so these never re-render React: they write `transform` on a DOM node from
 * `playheadStore` / view-store subscriptions.
 */

import { useEffect, useRef, type RefObject } from "react";
import type { Beats } from "@/generated";
import { playheadStore } from "@/state/playhead";
import { beatsToPx } from "./viewport";
import type { TimelineViewStore } from "./viewStore";

/** Maps song beats to the view's time axis (`null` = hide), e.g. clip-relative views. */
export type PlayheadMapping = (songBeats: Beats) => Beats | null;

const identity: PlayheadMapping = (b) => b;

/** Current song position in beats (0 before the first frame). */
export function playheadBeats(): Beats {
  return playheadStore.getPlayhead()?.transport.position ?? 0;
}

/**
 * Keep `ref`'s element translated to the playhead x within `view`. The element should be
 * absolutely positioned at `left: 0`.
 */
export function usePlayheadPosition(
  ref: RefObject<HTMLElement | null>,
  view: TimelineViewStore,
  mapping: PlayheadMapping = identity,
): void {
  const mapRef = useRef(mapping);
  useEffect(() => {
    mapRef.current = mapping;
  });
  useEffect(() => {
    const update = () => {
      const el = ref.current;
      if (!el) return;
      const beats = mapRef.current(playheadBeats());
      const s = view.getState();
      const x = beats === null ? -1 : beatsToPx(beats, s);
      const visible = beats !== null && x >= -1 && (s.widthPx === 0 || x <= s.widthPx + 1);
      el.style.transform = `translateX(${Math.round(x)}px)`;
      el.style.visibility = visible ? "visible" : "hidden";
    };
    update();
    const a = playheadStore.subscribePlayhead(update);
    const b = view.subscribe(update);
    return () => {
      a();
      b();
    };
  }, [ref, view]);
}

/**
 * While playing, page the view when the playhead leaves it (Ableton "follow"), if the
 * view store's `followPlayhead` is on.
 */
export function useFollowPlayhead(view: TimelineViewStore, mapping: PlayheadMapping = identity): void {
  const mapRef = useRef(mapping);
  useEffect(() => {
    mapRef.current = mapping;
  });
  useEffect(
    () =>
      playheadStore.subscribePlayhead(() => {
        const frame = playheadStore.getPlayhead();
        const s = view.getState();
        if (!frame?.transport.playing || !s.followPlayhead || s.widthPx <= 0) return;
        const beats = mapRef.current(frame.transport.position);
        if (beats === null) return;
        const x = beatsToPx(beats, s);
        if (x < 0 || x > s.widthPx) s.scrollTo(beats);
      }),
    [view],
  );
}

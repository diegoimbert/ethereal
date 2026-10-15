import { useState, type PointerEvent as ReactPointerEvent } from "react";
import type { BeatRange, Beats, Command } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { cmd, nextGestureId, useTransport } from "@/transport";
import { useT } from "@/i18n";
import { applyLoopDrag, loopHandleAt, type LoopHandle } from "./loop";
import { beatsToPx, dragBeats } from "./viewport";
import { useViewport, type TimelineViewStore } from "./viewStore";

const LOOP_CURSOR: Record<LoopHandle, string> = { start: "ew-resize", end: "ew-resize", move: "grab" };

export interface LoopBraceProps {
  view: TimelineViewStore;
  /** Snap a raw beat position (`bypass` = Alt). */
  snap: (beats: Beats, bypass: boolean) => Beats;
}

/**
 * The project loop brace in a positioned container whose left edge is the view's x = 0 (the
 * ruler, or the arrangement's loop strip): drag the body to move it, the edges to resize
 * (snapped, Alt: free), double-click toggles the loop. One undo step per drag.
 */
export function LoopBrace({ view, snap }: LoopBraceProps) {
  const t = useT();
  const transport = useTransport();
  const vp = useViewport(view);
  const loopEnabled = useProjectStore((s) => s.project?.settings.loop_enabled ?? false);
  const loopRegion = useProjectStore((s) => s.project?.settings.loop_region ?? null);
  /** Live region while dragging (before the engine echoes it back). */
  const [dragRegion, setDragRegion] = useState<BeatRange | null>(null);
  const region = dragRegion ?? loopRegion;
  if (!region) return null;

  const send = (command: Command, gesture?: number) => {
    transport.send(command, gesture === undefined ? undefined : { gesture }).catch((err: unknown) => {
      console.warn("loop command failed", err);
    });
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !loopRegion) return;
    e.stopPropagation();
    const box = e.currentTarget.getBoundingClientRect();
    const handle = loopHandleAt(e.clientX - box.left, box.width);
    // Keep the hover cursor for the whole drag, even when the pointer leaves the thin brace.
    const root = document.documentElement;
    const prevCursor = root.style.cursor;
    root.style.cursor = LOOP_CURSOR[handle] === "grab" ? "grabbing" : LOOP_CURSOR[handle];
    // Ruler px of the press (the brace sits at the region start), then beats from the press
    // to the pointer through the view of each moment (it may scroll or zoom mid-drag).
    const rulerLeft = box.left - beatsToPx(loopRegion.start, view.getState());
    const beatsMoved = dragBeats(view.getState, e.clientX - rulerLeft);
    const base = loopRegion;
    const gesture = nextGestureId();
    let last: BeatRange = base;
    const move = (ev: PointerEvent) => {
      const delta = beatsMoved(ev.clientX - rulerLeft);
      const next = applyLoopDrag(base, handle, delta, (b) => snap(b, ev.altKey));
      if (next.start !== last.start || next.end !== last.end) {
        last = next;
        setDragRegion(next);
        send(cmd("Transport", { type: "SetLoopRegion", region: next }), gesture);
      }
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      root.style.cursor = prevCursor;
      if (last !== base) send(cmd("Edit", { type: "EndGesture", gesture }));
      setDragRegion(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  /** Resize cursor over the edges (the same zones the drag uses), grab over the body. */
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    const cursor = LOOP_CURSOR[loopHandleAt(e.clientX - box.left, box.width)];
    if (e.currentTarget.style.cursor !== cursor) e.currentTarget.style.cursor = cursor;
  };

  const left = beatsToPx(region.start, vp);
  const width = Math.max(2, (region.end - region.start) * vp.pxPerBeat);
  return (
    <div
      className={loopEnabled ? "eth-ruler__loop" : "eth-ruler__loop eth-ruler__loop--off"}
      style={{ transform: `translateX(${Math.round(left)}px)`, width }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onDoubleClick={() => send(cmd("Transport", { type: "SetLoopEnabled", enabled: !loopEnabled }))}
      data-testid="ruler-loop"
      title={t("timeline.loop_brace.title")}
    />
  );
}

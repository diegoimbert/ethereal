/**
 * Lane header widgets: the value scale (step labels, and the visible value window: wheel to
 * scroll, cmd/ctrl+wheel to zoom, drag to scroll, double-click to reset) and the bottom-edge
 * resize grip (the track header's interaction: drag in `TRACK_HEIGHT_STEP` increments,
 * alt = free, double-click = reset).
 */

import { useEffect, useMemo, useRef, type PointerEvent } from "react";
import type { ParamInfo, TrackId } from "@/generated";
import { setDragCursor } from "@/kit";
import { TRACK_HEIGHT_STEP } from "@/features/arrangement/layout";
import { LANE_PAD, valueToY } from "./geometry";
import { formatNormalized } from "./params";
import { useAutomationUi } from "./uiStore";
import { isFullRange, scrollRange, stepLines, zoomRange, type ValueRange } from "./valueAxis";

/** Wheel zoom per pixel of delta (exponential, like the timeline's). */
const WHEEL_ZOOM = 0.004;

interface ValueScaleProps {
  info: ParamInfo;
  range: ValueRange;
  height: number;
  name: string;
  onRange(range: ValueRange | null): void;
}

export function ValueScale({ info, range, height, name, onRange }: ValueScaleProps) {
  const ref = useRef<HTMLDivElement>(null);
  const live = useRef({ info, range, height, onRange });
  useEffect(() => {
    live.current = { info, range, height, onRange };
  });

  const labels = useMemo(() => {
    const lines = stepLines(info, range, height - 2 * LANE_PAD).filter((l) => l.label !== null);
    if (lines.length > 0) return lines.map((l) => ({ value: l.value, text: l.label!, major: l.major }));
    // Continuous (or too dense to label): the window's ends.
    return [
      { value: range.hi, text: formatNormalized(info, range.hi), major: false },
      { value: range.lo, text: formatNormalized(info, range.lo), major: false },
    ];
  }, [info, range, height]);

  // Native listener: wheel must be able to preventDefault (the arrangement scrolls otherwise).
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const { info: inf, range: r, height: h, onRange: set } = live.current;
      const usable = Math.max(1, h - 2 * LANE_PAD);
      const span = r.hi - r.lo;
      if (e.ctrlKey || e.metaKey) {
        const box = el.getBoundingClientRect();
        const at = r.hi - ((e.clientY - box.top - LANE_PAD) / usable) * span;
        set(zoomRange(r, Math.exp(e.deltaY * WHEEL_ZOOM), at, inf));
      } else {
        set(scrollRange(r, (-e.deltaY / usable) * span, inf));
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const y0 = e.clientY;
    const r0 = live.current.range;
    const usable = Math.max(1, live.current.height - 2 * LANE_PAD);
    setDragCursor("ns-resize");
    const move = (ev: globalThis.PointerEvent) => {
      live.current.onRange(scrollRange(r0, ((ev.clientY - y0) / usable) * (r0.hi - r0.lo), live.current.info));
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", done);
      setDragCursor(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done);
    window.addEventListener("pointercancel", done);
  };

  return (
    <div
      ref={ref}
      className="eth-auto-scale"
      data-zoomed={!isFullRange(range) || undefined}
      role="scrollbar"
      aria-orientation="vertical"
      aria-label={`${name} value range`}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={Math.round(((range.lo + range.hi) / 2) * 100) / 100}
      title="Value range: scroll or drag to move, ⌘/Ctrl-scroll to zoom, double-click to reset"
      onPointerDown={onPointerDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onRange(null);
      }}
    >
      {labels.map((l) => (
        <span
          key={`${l.value}:${l.text}`}
          className="eth-auto-scale__label"
          data-major={l.major || undefined}
          style={{ top: valueToY(l.value, height, range) }}
        >
          {l.text}
        </span>
      ))}
    </div>
  );
}

interface LaneResizeHandleProps {
  trackId: TrackId;
  targetKey: string;
  height: number;
  name: string;
}

/** Bottom edge of a lane's header: drag to resize the lane, double-click to reset it. */
export function LaneResizeHandle({ trackId, targetKey, height, name }: LaneResizeHandleProps) {
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const startY = e.clientY;
    const startH = height;
    const ui = useAutomationUi.getState();
    const move = (ev: globalThis.PointerEvent) => {
      const h = startH + ev.clientY - startY;
      ui.setLaneHeight(trackId, targetKey, ev.altKey ? h : Math.round(h / TRACK_HEIGHT_STEP) * TRACK_HEIGHT_STEP);
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", done);
      setDragCursor(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done);
    window.addEventListener("pointercancel", done);
    setDragCursor("ns-resize");
  };
  return (
    <div
      className="eth-auto-row__resize"
      onPointerDown={onPointerDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        useAutomationUi.getState().setLaneHeight(trackId, targetKey, null);
      }}
      role="separator"
      aria-orientation="horizontal"
      aria-label={`Resize ${name} lane`}
      title="Drag to resize the lane (double-click to reset)"
    />
  );
}

/**
 * One automation lane drawn in SVG: the curve (engine formula), breakpoint handles and
 * the editing interactions.
 *
 * - double-click the background: add a point (time snapped to the grid, alt = no snap);
 * - click a point: select it (shift = add, cmd/ctrl = toggle); drag: move the selection
 *   (snapped; alt = no snap; shift while dragging = lock to the dominant axis);
 * - double-click a point: delete it; Delete/Backspace: delete the selected points;
 * - drag the background: marquee selection; cmd/ctrl+A: select all points of the lane;
 * - alt-drag a segment up/down: bend it (sets `CurveShape::Curve { tension }`).
 *
 * Every drag is one undo gesture. The lane is created with its first point.
 */

import clsx from "clsx";
import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import type { AutomationLane, AutomationOwner, AutomationPoint, AutomationTarget, ParamInfo } from "@/generated";
import { usePointsOfLane } from "@/state";
import {
  DEFAULT_GRID,
  itemSelection,
  marqueeHits,
  resolveGrid,
  selectModeFromEvent,
  snapToGrid,
  useMarquee,
  useSelectedItems,
  useTempoMap,
  useTimelineView,
  useViewport,
  type GridSetting,
  type ItemSelectionStore,
  type TimelineViewStore,
} from "@/timeline";
import { newId, useTransport } from "@/transport";
import { addPointCommand, bendTension, editPointsCommand, moveEdits, removePointsCommand, setCurveCommand, tensionOf } from "./edit";
import { LANE_PAD, lanePath, pointRect, POINT_RADIUS, segmentAt, timeToX, valueToY, xToTime, yToValue, type LaneGeometry } from "./geometry";
import { LaneGesture, sendEdit } from "./gesture";
import { formatNormalized } from "./params";
import "./automation.css";

export interface AutomationLaneViewProps {
  /** The document lane, or `null` when the parameter is shown but has no lane yet. */
  lane: AutomationLane | null;
  owner: AutomationOwner;
  target: AutomationTarget;
  info: ParamInfo;
  /** Horizontal viewport (shared with the ruler / clip lanes). */
  view: TimelineViewStore;
  height: number;
  grid?: GridSetting;
  /** View beats of lane time 0 (clip envelopes: the clip's start). Default 0. */
  offset?: number;
  selection?: ItemSelectionStore;
  /** Accessible name. */
  label?: string;
}

/** Pixels before a press on a point turns into a drag. */
const DRAG_THRESHOLD = 2;
const FALLBACK_WIDTH = 4000;

export function AutomationLaneView({
  lane,
  owner,
  target,
  info,
  view,
  height,
  grid = DEFAULT_GRID,
  offset = 0,
  selection = itemSelection,
  label,
}: AutomationLaneViewProps) {
  const transport = useTransport();
  const tempo = useTempoMap();
  const vp = useViewport(view);
  const width = useTimelineView(view, (s) => s.widthPx) || FALLBACK_WIDTH;
  const points = usePointsOfLane(lane?.id ?? "");
  const selected = useSelectedItems("automationPoint", selection);
  const [readout, setReadout] = useState<{ x: number; y: number; text: string } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  const geom: LaneGeometry = useMemo(() => ({ vp, height, offset }), [vp, height, offset]);
  const path = useMemo(() => lanePath(points, geom, 0, width), [points, geom, width]);

  // Latest values for window-level drag handlers.
  const live = useRef({ points, geom, lane, grid, tempo });
  useLayoutEffect(() => {
    live.current = { points, geom, lane, grid, tempo };
  });

  const snapFn = useCallback(
    (bypass: boolean) => {
      const { grid: g, tempo: t, geom: gm } = live.current;
      const step = bypass ? null : resolveGrid(g, gm.vp.pxPerBeat, t.signatureAt(gm.vp.scrollBeats));
      if (!step) return null;
      const off = gm.offset ?? 0;
      return (time: number) => Math.max(0, snapToGrid(time + off, step, t) - off);
    },
    [],
  );

  const marquee = useMarquee({
    kind: "automationPoint",
    store: selection,
    hitTest: (rect) => marqueeHits(rect, live.current.points.map((p) => ({ id: p.id, rect: pointRect(p, live.current.geom) }))),
  });

  const local = (e: { clientX: number; clientY: number }) => {
    const box = rootRef.current!.getBoundingClientRect();
    return { x: e.clientX - box.left, y: e.clientY - box.top };
  };

  /** Run a window-level pointer drag until release. */
  const trackDrag = (onMove: (e: PointerEvent) => void, onUp: (moved: boolean, e: PointerEvent) => void, start: { x: number; y: number }) => {
    let moved = false;
    const move = (e: PointerEvent) => {
      const p = local(e);
      if (!moved && Math.hypot(p.x - start.x, p.y - start.y) < DRAG_THRESHOLD) return;
      moved = true;
      onMove(e);
    };
    const up = (e: PointerEvent) => {
      done();
      onUp(moved, e);
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
    };
    const cancel = (e: PointerEvent) => {
      done();
      onUp(moved, e);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
  };

  const onPointPointerDown = (e: ReactPointerEvent, point: AutomationPoint) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    rootRef.current?.focus();
    const mode = selectModeFromEvent(e);
    const wasSelected = selection.getState().isSelected("automationPoint", point.id);
    if (mode === "toggle") {
      selection.getState().select("automationPoint", [point.id], "toggle");
      if (wasSelected) return;
    } else if (mode === "add" || !wasSelected) {
      selection.getState().select("automationPoint", [point.id], mode);
    }
    const sel = selection.getState().selected.automationPoint;
    const originals = live.current.points.filter((p) => sel.has(p.id));
    if (!originals.some((p) => p.id === point.id)) return;
    const start = local(e);
    const gesture = new LaneGesture(transport);
    const usable = Math.max(1, live.current.geom.height - 2 * LANE_PAD);
    trackDrag(
      (ev) => {
        const p = local(ev);
        const dx = p.x - start.x;
        const dy = p.y - start.y;
        const lockTime = ev.shiftKey && Math.abs(dy) > Math.abs(dx);
        const lockValue = ev.shiftKey && !lockTime;
        const dt = dx / live.current.geom.vp.pxPerBeat;
        const dv = -dy / usable;
        const edits = moveEdits(originals, point, dt, dv, snapFn(ev.altKey), { lockTime, lockValue });
        gesture.update(editPointsCommand(edits));
        const mine = edits.find((x) => x.id === point.id);
        const t = mine?.time ?? point.time;
        const v = mine?.value ?? point.value;
        setReadout({ x: timeToX(t, live.current.geom), y: valueToY(v, live.current.geom.height), text: formatNormalized(info, v) });
      },
      (moved) => {
        gesture.end();
        setReadout(null);
        // A plain click on a point of a multi-selection selects just that point.
        if (!moved && mode === "replace" && wasSelected) selection.getState().select("automationPoint", [point.id], "replace");
      },
      start,
    );
  };

  const onBackgroundPointerDown = (e: ReactPointerEvent<SVGSVGElement>) => {
    // The lane owns its pointer events (the arrangement has its own clip marquee).
    e.stopPropagation();
    rootRef.current?.focus();
    if (e.button !== 0) return;
    if (e.altKey) {
      const start = local(e);
      const pts = live.current.points;
      const i = segmentAt(pts, xToTime(start.x, live.current.geom));
      if (i >= 0) {
        e.preventDefault();
        const a = pts[i]!;
        const b = pts[i + 1]!;
        const rising = b.value >= a.value;
        const t0 = tensionOf(a.curve);
        const gesture = new LaneGesture(transport);
        trackDrag(
          (ev) => {
            const tension = bendTension(t0, start.y - local(ev).y, rising);
            gesture.update(setCurveCommand([a.id], { type: "Curve", tension }));
          },
          () => gesture.end(),
          start,
        );
        return;
      }
    }
    marquee.onPointerDown(e);
  };

  const onDoubleClick = (e: MouseEvent<SVGSVGElement>) => {
    e.stopPropagation();
    if (e.target !== e.currentTarget) return;
    const p = local(e);
    const snap = snapFn(e.altKey);
    const raw = Math.max(0, xToTime(p.x, live.current.geom));
    const time = snap ? snap(raw) : raw;
    const value = yToValue(p.y, height);
    const id = newId();
    void sendEdit(transport, addPointCommand(live.current.lane, owner, target, { id, time, value, curve: { type: "Linear" } }, newId())).then(
      () => selection.getState().select("automationPoint", [id], "replace"),
    );
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Delete" || e.key === "Backspace") {
      const sel = selection.getState().selected.automationPoint;
      const ids = live.current.points.filter((p) => sel.has(p.id)).map((p) => p.id);
      if (ids.length === 0) return;
      e.preventDefault();
      e.stopPropagation();
      void sendEdit(transport, removePointsCommand(ids));
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      e.stopPropagation();
      selection.getState().select(
        "automationPoint",
        live.current.points.map((p) => p.id),
        "replace",
      );
    }
  };

  const disabled = lane !== null && !lane.enabled;
  const r = marquee.rect;
  return (
    <div
      ref={rootRef}
      className={clsx("eth-auto-lane", disabled && "eth-auto-lane--disabled", lane === null && "eth-auto-lane--empty")}
      style={{ height }}
      tabIndex={0}
      role="group"
      aria-label={label ?? `${info.name} automation`}
      data-lane={lane?.id}
      onKeyDown={onKeyDown}
    >
      <svg
        className="eth-auto-lane__svg"
        width="100%"
        height={height}
        onPointerDown={onBackgroundPointerDown}
        onDoubleClick={onDoubleClick}
        data-testid="automation-lane-svg"
      >
        {path && <path className="eth-auto-lane__line" d={path} />}
        {points.map((p) => {
          const x = timeToX(p.time, geom);
          if (x < -POINT_RADIUS || x > width + POINT_RADIUS) return null;
          const on = selected.has(p.id);
          return (
            <circle
              key={p.id}
              className={clsx("eth-auto-lane__point", on && "eth-auto-lane__point--selected")}
              cx={x}
              cy={valueToY(p.value, height)}
              r={POINT_RADIUS}
              data-point={p.id}
              aria-selected={on}
              onPointerDown={(e) => onPointPointerDown(e, p)}
              onDoubleClick={(e) => {
                e.stopPropagation();
                void sendEdit(transport, removePointsCommand([p.id]));
              }}
            >
              <title>{formatNormalized(info, p.value)}</title>
            </circle>
          );
        })}
        {r && <rect className="eth-auto-lane__marquee" x={r.x0} y={r.y0} width={r.x1 - r.x0} height={r.y1 - r.y0} />}
        {readout && (
          <text className="eth-auto-lane__readout" x={readout.x + 8} y={Math.max(12, readout.y - 6)}>
            {readout.text}
          </text>
        )}
      </svg>
    </div>
  );
}

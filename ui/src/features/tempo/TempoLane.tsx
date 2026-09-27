/**
 * Tempo lane of the tempo editor: the tempo curve (steps and ramps) with one handle per
 * tempo point.
 *
 * - drag a point: time (snapped to the grid; Alt bypasses) and BPM (whole BPM); Shift
 *   locks the dominant axis; the point at beat 0 only changes BPM. One undo step per drag;
 * - double-click the lane: add a point there; double-click a point: remove it;
 * - right-click a point: ramp to the next point on/off, delete;
 * - Delete/Backspace removes the selected point.
 */

import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { TempoPoint } from "@/generated";
import { openContextMenu, setDragCursor } from "@/kit";
import { newId } from "@/transport";
import {
  beatsToPx,
  DEFAULT_GRID,
  pxToBeats,
  resolveGrid,
  snapToGrid,
  useTimelineView,
  useViewport,
  type TempoMap,
  type TimelineViewStore,
} from "@/timeline";
import { sendEdit, TempoGesture, trackDrag, useTempoTransport } from "./gesture";
import {
  addTempoPointCommand,
  bpmRange,
  bpmToY,
  clampBpm,
  editTempoPointCommand,
  formatBpm,
  isAtZero,
  removeTempoPointsCommand,
  tempoPath,
  tempoTimeTaken,
  yToBpm,
  type LaneGeom,
} from "./tempoCommands";

const POINT_RADIUS = 4;
const PAD = 8;

export interface TempoLaneProps {
  view: TimelineViewStore;
  tempo: TempoMap;
  points: TempoPoint[];
  selected: string | null;
  onSelect: (id: string | null) => void;
}

export function TempoLane({ view, tempo, points, selected, onSelect }: TempoLaneProps) {
  const transport = useTempoTransport();
  const vp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const rootRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(120);
  const [readout, setReadout] = useState<{ x: number; y: number; text: string } | null>(null);
  /** BPM range frozen while dragging (so the lane doesn't rescale under the pointer). */
  const [frozen, setFrozen] = useState<LaneGeom["range"] | null>(null);
  const geom: LaneGeom = { height, pad: PAD, range: frozen ?? bpmRange(points) };
  const live = useRef({ points, geom, tempo });
  useLayoutEffect(() => {
    live.current = { points, geom, tempo };
  });

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const set = () => setHeight(Math.max(40, el.clientHeight));
    set();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const toX = (b: number) => beatsToPx(b, vp);
  const local = (e: { clientX: number; clientY: number }) => {
    const box = rootRef.current!.getBoundingClientRect();
    return { x: e.clientX - box.left, y: e.clientY - box.top };
  };
  const snap = (beats: number, bypass: boolean) => {
    if (bypass) return Math.max(0, beats);
    const st = view.getState();
    const step = resolveGrid(DEFAULT_GRID, st.pxPerBeat, live.current.tempo.signatureAt(st.scrollBeats));
    return Math.max(0, snapToGrid(beats, step, live.current.tempo));
  };

  const onPointPointerDown = (e: ReactPointerEvent, point: TempoPoint) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    rootRef.current?.focus();
    onSelect(point.id);
    const start = local(e);
    const range = live.current.geom.range;
    const g: LaneGeom = { ...live.current.geom, range };
    const gesture = new TempoGesture(transport);
    const fixedTime = isAtZero(point);
    let last = { time: point.time, bpm: point.bpm };
    trackDrag(
      e,
      (ev) => {
        if (!frozen) setFrozen(range);
        setDragCursor("grabbing");
        const p = local(ev);
        const dx = p.x - start.x;
        const dy = p.y - start.y;
        const lockTime = fixedTime || (ev.shiftKey && Math.abs(dy) > Math.abs(dx));
        const lockBpm = ev.shiftKey && !lockTime;
        let time = lockTime ? point.time : snap(point.time + dx / view.getState().pxPerBeat, ev.altKey);
        if (tempoTimeTaken(live.current.points, time, point.id)) time = last.time;
        const bpm = lockBpm ? point.bpm : clampBpm(Math.round(yToBpm(bpmToY(point.bpm, g) + dy, g)));
        setReadout({ x: toX(time), y: bpmToY(bpm, g), text: `${formatBpm(bpm)} BPM` });
        if (time === last.time && bpm === last.bpm) return;
        last = { time, bpm };
        gesture.update(
          editTempoPointCommand(point.id, {
            time: time !== point.time ? time : undefined,
            bpm: bpm !== point.bpm ? bpm : undefined,
          }),
        );
      },
      () => {
        setDragCursor(null);
        gesture.end();
        setReadout(null);
        setFrozen(null);
      },
    );
  };

  const onBackgroundPointerDown = (e: ReactPointerEvent) => {
    e.stopPropagation();
    rootRef.current?.focus();
    if (e.button === 0) onSelect(null);
  };

  const onDoubleClick = (e: MouseEvent<SVGSVGElement>) => {
    e.stopPropagation();
    if (e.target !== e.currentTarget) return;
    const p = local(e);
    const time = snap(pxToBeats(p.x, view.getState()), e.altKey);
    if (tempoTimeTaken(live.current.points, time)) return;
    const bpm = clampBpm(Math.round(yToBpm(p.y, live.current.geom)));
    const id = newId();
    void sendEdit(transport, addTempoPointCommand(time, bpm, "Step", id)).then(() => onSelect(id));
  };

  const remove = (point: TempoPoint) => {
    if (isAtZero(point)) return;
    if (selected === point.id) onSelect(null);
    void sendEdit(transport, removeTempoPointsCommand([point.id]));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Delete" && e.key !== "Backspace") return;
    // Keep Delete in the focused lane (bubbling up, it would delete the selected clips).
    e.preventDefault();
    e.stopPropagation();
    const point = live.current.points.find((p) => p.id === selected);
    if (point) remove(point);
  };

  const path = tempoPath(points, toX, geom, widthPx);
  return (
    <div
      ref={rootRef}
      className="eth-tempo-lane"
      tabIndex={0}
      role="group"
      aria-label="Tempo points"
      onKeyDown={onKeyDown}
      data-testid="tempo-lane"
    >
      <svg
        className="eth-tempo-lane__svg"
        width="100%"
        height={height}
        onPointerDown={onBackgroundPointerDown}
        onDoubleClick={onDoubleClick}
        data-testid="tempo-lane-svg"
      >
        {path && <path className="eth-tempo-lane__line" d={path} />}
        {points.map((p) => {
          const x = toX(p.time);
          if (widthPx > 0 && (x < -POINT_RADIUS || x > widthPx + POINT_RADIUS)) return null;
          const on = p.id === selected;
          return (
            <circle
              key={p.id}
              className={on ? "eth-tempo-lane__point eth-tempo-lane__point--selected" : "eth-tempo-lane__point"}
              cx={x}
              cy={bpmToY(p.bpm, geom)}
              r={POINT_RADIUS}
              data-point={p.id}
              aria-selected={on}
              onPointerDown={(e) => onPointPointerDown(e, p)}
              onDoubleClick={(e) => {
                e.stopPropagation();
                remove(p);
              }}
              onContextMenu={(e) => {
                onSelect(p.id);
                openContextMenu(e, [
                  {
                    label: p.curve === "Linear" ? "Hold Tempo (No Ramp)" : "Ramp to Next Point",
                    onSelect: () =>
                      void sendEdit(transport, editTempoPointCommand(p.id, { curve: p.curve === "Linear" ? "Step" : "Linear" })),
                  },
                  "separator",
                  { label: "Delete Tempo Point", shortcut: "⌫", danger: true, disabled: isAtZero(p), onSelect: () => remove(p) },
                ]);
              }}
            >
              <title>{`${formatBpm(p.bpm)} BPM${p.curve === "Linear" ? " (ramp)" : ""}`}</title>
            </circle>
          );
        })}
        {readout && (
          <text className="eth-tempo-lane__readout" x={readout.x + 8} y={Math.max(12, readout.y - 6)}>
            {readout.text}
          </text>
        )}
      </svg>
    </div>
  );
}

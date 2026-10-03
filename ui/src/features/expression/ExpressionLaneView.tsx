/**
 * One clip expression lane (CC, pitch bend or channel pressure) under the piano roll, drawn
 * in SVG on the clip's content timeline (the same x as the notes above it).
 *
 * - Drag on the background: pencil. The stroke replaces the curve under it, one
 *   `ReplaceRange` per pointer move, all with one gesture id (one undo step). The lane is
 *   created by the stroke's first command if the clip has none of this kind yet.
 * - Drag a point: move it (time snaps to the grid, alt = free). Double-click a point:
 *   delete it. Double-click the background: add a point.
 * - Right-click: clear or remove the lane.
 */

import { useMemo, useRef } from "react";
import type { Command, ExpressionKind, ExpressionLane, ExpressionPoint } from "@/generated";
import { openContextMenu } from "@/kit";
import { beatsToPx, pxToBeats, type TimelineViewport } from "@/timeline";
import { cmd, newId, useTransport } from "@/transport";
import { startDrag } from "../piano-roll/drag";
import { curvePath, StrokeSampler, valueToY, yToValue } from "./curve";
import { expressionRange, formatValue, kindLabel, movePoint, strokePoints, withPoint } from "./model";

export interface ExpressionLaneViewProps {
  clip: string;
  kind: ExpressionKind;
  /** The clip's lane of `kind`, or `undefined` when it has none yet (drawing creates it). */
  lane: ExpressionLane | undefined;
  vp: TimelineViewport;
  widthPx: number;
  height: number;
  /** Grid step (beats) points snap to. */
  stepBeats: number;
}

/** Pencil resolution in px: one point per this many pixels at most. */
const PENCIL_PX = 3;
/** Point handles are drawn when visible points are at least this far apart on average. */
const HANDLE_SPACING_PX = 16;
const HANDLE_R = 3.5;
const NO_POINTS: ExpressionPoint[] = [];

export function ExpressionLaneView({ clip, kind, lane, vp, widthPx, height, stepBeats }: ExpressionLaneViewProps) {
  const transport = useTransport();
  const svgRef = useRef<SVGSVGElement>(null);
  const range = expressionRange(kind);
  const points = lane?.points ?? NO_POINTS;
  const label = kindLabel(kind);
  // A stable id for the lane the next stroke may create.
  const pendingId = useMemo(() => newId(), []);
  const laneId = lane?.id ?? pendingId;

  const toX = (t: number) => beatsToPx(t, vp);
  const toTime = (x: number) => pxToBeats(x, vp);
  const toY = (v: number) => valueToY(v, range, height);
  const local = (e: { clientX: number; clientY: number }) => {
    const r = svgRef.current?.getBoundingClientRect();
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) };
  };

  /** `command` on the lane, creating the lane first (same undo step) when needed. */
  const onLane = (command: Command, what: string): Command =>
    lane
      ? command
      : cmd("Edit", {
          type: "Batch",
          label: what,
          commands: [cmd("Expression", { type: "CreateLane", id: laneId, clip, kind }), command],
        });

  const setPoints = (next: ExpressionPoint[]): Command =>
    onLane(cmd("Expression", { type: "SetPoints", lane: laneId, points: next }), `Edit ${label}`);

  const send = (command: Command) => {
    transport.send(command).catch((err: unknown) => console.warn("[expression] command failed:", err));
  };

  const snap = (t: number, free: boolean) => (free || stepBeats <= 0 ? t : Math.round(t / stepBeats) * stepBeats);

  const onBackgroundDown = (e: React.PointerEvent<SVGSVGElement>) => {
    if (e.button !== 0 || (e.target as Element).closest("[data-handle]")) return;
    e.preventDefault();
    const p0 = local(e);
    const sampler = new StrokeSampler(PENCIL_PX / vp.pxPerBeat);
    sampler.add(toTime(p0.x), yToValue(p0.y, range, height));
    startDrag(
      transport,
      e,
      {
        move: (dx, dy) => {
          sampler.add(toTime(p0.x + dx), yToValue(p0.y + dy, range, height));
          const s = strokePoints(sampler.samples(), range, sampler.minGap);
          if (!s) return null;
          return onLane(
            cmd("Expression", { type: "ReplaceRange", lane: laneId, start: s.start, end: s.end, points: s.points }),
            `Draw ${label}`,
          );
        },
      },
      { threshold: 2, cursor: "crosshair" },
    );
  };

  const onHandleDown = (e: React.PointerEvent, index: number) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const orig = points;
    const p = orig[index]!;
    const x0 = toX(p.time);
    const y0 = toY(p.value);
    startDrag(
      transport,
      e,
      {
        move: (dx, dy, m) =>
          setPoints(movePoint(orig, index, snap(toTime(x0 + dx), m.altKey), yToValue(y0 + dy, range, height), range)),
      },
      { threshold: 2, cursor: "grabbing" },
    );
  };

  const onDoubleClick = (e: React.MouseEvent<SVGSVGElement>) => {
    const handle = (e.target as Element).closest("[data-handle]");
    if (handle) {
      const i = Number(handle.getAttribute("data-index"));
      send(setPoints(points.filter((_, k) => k !== i)));
      return;
    }
    const p = local(e);
    send(setPoints(withPoint(points, { time: snap(Math.max(0, toTime(p.x)), e.altKey), value: yToValue(p.y, range, height), curve: { type: "Linear" } })));
  };

  const onContextMenu = (e: React.MouseEvent) => {
    openContextMenu(e, [
      { label: `Clear ${label}`, disabled: !lane || points.length === 0, onSelect: () => lane && send(setPoints([])) },
      {
        label: `Remove ${label} lane`,
        danger: true,
        disabled: !lane,
        onSelect: () => lane && send(cmd("Expression", { type: "RemoveLane", id: lane.id })),
      },
    ]);
  };

  const width = Math.max(0, widthPx);
  const visible = useMemo(() => {
    const t0 = pxToBeats(0, vp);
    const t1 = pxToBeats(width, vp);
    const out: number[] = [];
    for (let i = 0; i < points.length; i++) if (points[i]!.time >= t0 && points[i]!.time <= t1) out.push(i);
    return out;
  }, [points, vp, width]);
  const showHandles = visible.length > 0 && width / visible.length >= HANDLE_SPACING_PX;
  const centre = kind.type === "PitchBend" ? toY(0) : null;
  const d = curvePath(points, toX, toTime, toY, 0, width);

  return (
    <svg
      ref={svgRef}
      className="eth-expr-lane"
      width={width}
      height={height}
      data-testid="expression-lane"
      data-kind={label}
      data-points={points.length}
      aria-label={`${label} lane`}
      onPointerDown={onBackgroundDown}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      {centre !== null && <line className="eth-expr-lane__centre" x1={0} x2={width} y1={centre} y2={centre} />}
      {d && (
        <>
          <path className="eth-expr-lane__fill" d={`${d} L${width},${centre ?? height} L0,${centre ?? height} Z`} />
          <path className="eth-expr-lane__curve" d={d} data-testid="expression-curve" />
        </>
      )}
      {showHandles &&
        visible.map((i) => {
          const p = points[i]!;
          return (
            <circle
              key={i}
              data-handle=""
              data-index={i}
              className="eth-expr-lane__point"
              cx={toX(p.time)}
              cy={toY(p.value)}
              r={HANDLE_R}
              onPointerDown={(e) => onHandleDown(e, i)}
            >
              <title>{formatValue(kind, p.value)}</title>
            </circle>
          );
        })}
      {!d && (
        <text className="eth-expr-lane__hint" x={8} y={height / 2} dominantBaseline="middle">
          Draw to add {label}
        </text>
      )}
    </svg>
  );
}

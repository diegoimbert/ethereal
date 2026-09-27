/**
 * One automation lane drawn in SVG: the curve (engine formula), breakpoint handles, step
 * gridlines and the editing interactions (see README "Automation editing"):
 *
 * - double-click the background: add a point (time snapped to the grid, alt = no snap;
 *   value snapped to the param's steps, cmd/ctrl = whole increments);
 * - click a point: select it (shift = add, cmd/ctrl = toggle); drag: move the selection.
 *   Time snaps to the grid, stepped params (semitones, enums, toggles) snap to their steps;
 *   alt: off the time grid; cmd/ctrl held while dragging: whole value increments of
 *   continuous params (1 dB, 1 %...); shift while dragging: lock to
 *   the dominant axis. A tooltip shows the value and the modifiers while dragging;
 * - double-click a point: delete it; Delete/Backspace: delete the selected points;
 * - drag the background: marquee selection; cmd/ctrl+A: select all points of the lane;
 * - cmd/ctrl+C / X / V / D: copy, cut, paste at the playhead, duplicate (one undo step);
 * - arrows: nudge the selection (up/down: one step or 1 %, shift = fine; left/right: one
 *   grid step);
 * - alt-drag a segment up/down: bend it (sets `CurveShape::Curve { tension }`);
 * - right-click: point / lane menus (cut, copy, paste, duplicate, curve, set value, delete).
 *
 * Every drag is one undo gesture. The lane is created with its first point.
 */

import clsx from "clsx";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import type { AutomationLane, AutomationOwner, AutomationPoint, AutomationPointId, AutomationTarget, CurveShape, ParamInfo } from "@/generated";
import { Button, Dialog, MOD_KEY, NumberField, openContextMenu, type ContextMenuEntry } from "@/kit";
import { usePointsOfLane } from "@/state";
import { paramToNormalized, paramToPlain } from "@/features/devices/paramScale";
import {
  DEFAULT_GRID,
  itemSelection,
  marqueeHits,
  playheadBeats,
  resolveGrid,
  selectModeFromEvent,
  snapToGrid,
  stepLength,
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
import { copyPoints, duplicateAt, hasPointClipboard, pastePoints, pointClipboard, snapshotPoints, type PasteTarget } from "./clipboard";
import { addPointCommand, bendTension, editPointsCommand, moveEdits, removePointsCommand, setCurveCommand, tensionOf } from "./edit";
import { LANE_PAD, lanePath, pointRect, POINT_RADIUS, segmentAt, timeToX, valueToY, xToTime, yToValue, type LaneGeometry } from "./geometry";
import { LaneGesture, sendEdit } from "./gesture";
import { formatNormalized, targetKey as keyOfTarget } from "./params";
import { dragHint, FULL_RANGE, nudgeSize, paramStep, snapValue, stepDragDelta, stepLines, type ValueRange } from "./valueAxis";
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
  /** Visible window of normalized values (default: 0..1). */
  range?: ValueRange;
  selection?: ItemSelectionStore;
  /** Accessible name. */
  label?: string;
}

/** Pixels before a press on a point turns into a drag. */
const DRAG_THRESHOLD = 2;
const FALLBACK_WIDTH = 4000;

interface Tip {
  x: number;
  y: number;
  value: string;
  hint: string;
}

export function AutomationLaneView({
  lane,
  owner,
  target,
  info,
  view,
  height,
  grid = DEFAULT_GRID,
  offset = 0,
  range = FULL_RANGE,
  selection = itemSelection,
  label,
}: AutomationLaneViewProps) {
  const transport = useTransport();
  const tempo = useTempoMap();
  const vp = useViewport(view);
  const width = useTimelineView(view, (s) => s.widthPx) || FALLBACK_WIDTH;
  const points = usePointsOfLane(lane?.id ?? "");
  const selected = useSelectedItems("automationPoint", selection);
  const [tip, setTip] = useState<Tip | null>(null);
  const [editing, setEditing] = useState<{ ids: AutomationPointId[]; plain: number } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const tKey = useMemo(() => keyOfTarget(target), [target]);

  const geom: LaneGeometry = useMemo(() => ({ vp, height, offset, range }), [vp, height, offset, range]);
  const path = useMemo(() => lanePath(points, geom, 0, width), [points, geom, width]);
  const lines = useMemo(() => stepLines(info, range, height - 2 * LANE_PAD), [info, range, height]);

  // Latest values for window-level drag handlers.
  const live = useRef({ points, geom, lane, grid, tempo, info, owner, target, tKey });
  useLayoutEffect(() => {
    live.current = { points, geom, lane, grid, tempo, info, owner, target, tKey };
  });

  const gridStep = useCallback(() => {
    const { grid: g, tempo: t, geom: gm } = live.current;
    return resolveGrid(g, gm.vp.pxPerBeat, t.signatureAt(gm.vp.scrollBeats));
  }, []);

  const snapFn = useCallback(
    (bypass: boolean) => {
      const step = bypass ? null : gridStep();
      if (!step) return null;
      const { tempo: t, geom: gm } = live.current;
      const off = gm.offset ?? 0;
      return (time: number) => Math.max(0, snapToGrid(time + off, step, t) - off);
    },
    [gridStep],
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

  /** Selected points of this lane. */
  const selectedHere = (): AutomationPoint[] => {
    const sel = selection.getState().selected.automationPoint;
    return live.current.points.filter((p) => sel.has(p.id));
  };

  const pasteTarget = (): PasteTarget => {
    const l = live.current;
    return { lane: l.lane, owner: l.owner, target: l.target, info: l.info, targetKey: l.tKey };
  };

  // ---- Clipboard ------------------------------------------------------------------------

  const copy = (): number => copyPoints(selectedHere(), live.current.info, live.current.tKey);
  const cut = () => {
    const pts = selectedHere();
    if (copy() > 0) void sendEdit(transport, removePointsCommand(pts.map((p) => p.id)));
  };
  const pasteAt = (at: number, keepAt = false, clip = pointClipboard()) =>
    pastePoints(transport, clip, pasteTarget(), at, selection, keepAt);
  const duplicate = () => {
    const pts = selectedHere();
    const clip = snapshotPoints(pts, live.current.info, live.current.tKey);
    if (!clip) return;
    const step = gridStep();
    const len = step ? stepLength(step, live.current.tempo.signatureAt(0)) : 1;
    void pasteAt(duplicateAt(pts, len), true, { ...clip }).then(() => undefined);
  };

  // Clipboard events (the desktop app's Edit menu sends these instead of the keys) while a
  // lane has focus: handled here, before the arrangement's document-level clip handlers.
  useEffect(() => {
    const onClipboard = (e: ClipboardEvent) => {
      const root = rootRef.current;
      if (!root || !root.contains(document.activeElement)) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.type === "copy") copy();
      else if (e.type === "cut") cut();
      else void pasteAt(playheadBeats());
    };
    window.addEventListener("copy", onClipboard, true);
    window.addEventListener("cut", onClipboard, true);
    window.addEventListener("paste", onClipboard, true);
    return () => {
      window.removeEventListener("copy", onClipboard, true);
      window.removeEventListener("cut", onClipboard, true);
      window.removeEventListener("paste", onClipboard, true);
    };
  });

  // ---- Pointer --------------------------------------------------------------------------

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
    const originals = selectedHere();
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
        const g = live.current.geom;
        const span = (g.range ?? FULL_RANGE).hi - (g.range ?? FULL_RANGE).lo;
        const dt = dx / g.vp.pxPerBeat;
        // Stepped params: whole steps at a fixed px-per-step; others follow the pointer.
        const dv = stepDragDelta(live.current.info, -dy) ?? (-dy / usable) * span;
        const inf = live.current.info;
        const edits = moveEdits(originals, point, dt, dv, snapFn(ev.altKey), {
          lockTime,
          lockValue,
          // ⌘/Ctrl held mid-drag: whole increments (⌥ stays "off the time grid").
          snapValue: (v) => snapValue(inf, v, ev.metaKey || ev.ctrlKey),
        });
        gesture.update(editPointsCommand(edits));
        const mine = edits.find((x) => x.id === point.id);
        const t = mine?.time ?? point.time;
        const v = mine?.value ?? point.value;
        setTip({ x: timeToX(t, g), y: valueToY(v, g.height, g.range), value: formatNormalized(inf, v), hint: dragHint(inf, MOD_KEY.replace(/\+$/, "")) });
      },
      (moved) => {
        gesture.end();
        setTip(null);
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

  /** Snapped lane time at a lane-local x. */
  const timeAt = (x: number, bypass: boolean) => {
    const snap = snapFn(bypass);
    const raw = Math.max(0, xToTime(x, live.current.geom));
    return snap ? snap(raw) : raw;
  };

  const onDoubleClick = (e: MouseEvent<SVGSVGElement>) => {
    e.stopPropagation();
    if (e.target !== e.currentTarget) return;
    const p = local(e);
    const time = timeAt(p.x, e.altKey);
    const value = snapValue(info, yToValue(p.y, height, range), e.metaKey || e.ctrlKey);
    const id = newId();
    void sendEdit(transport, addPointCommand(live.current.lane, owner, target, { id, time, value, curve: { type: "Linear" } }, newId())).then(
      () => selection.getState().select("automationPoint", [id], "replace"),
    );
  };

  // ---- Keyboard -------------------------------------------------------------------------

  const nudge = (dtSteps: number, dvSteps: number, fine: boolean) => {
    const pts = selectedHere();
    if (pts.length === 0) return;
    const inf = live.current.info;
    const step = gridStep();
    const beat = step ? stepLength(step, live.current.tempo.signatureAt(0)) : 1;
    const dt = dtSteps * (fine ? beat / 4 : beat);
    const dv = dvSteps * nudgeSize(inf, fine);
    const anchor = pts[0]!;
    const stepped = paramStep(inf) !== null;
    const edits = moveEdits(pts, anchor, dt, dv, null, {
      lockTime: dtSteps === 0,
      lockValue: dvSteps === 0,
      snapValue: stepped ? (v) => snapValue(inf, v) : undefined,
    });
    void sendEdit(transport, editPointsCommand(edits));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    const handled = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    if (e.key === "Delete" || e.key === "Backspace") {
      // Always keep Delete in the focused lane, even with nothing to delete: bubbling up,
      // it would delete the arrangement's selected clips.
      handled();
      const ids = selectedHere().map((p) => p.id);
      if (ids.length === 0) return;
      void sendEdit(transport, removePointsCommand(ids));
    } else if (mod && k === "a") {
      handled();
      selection.getState().select(
        "automationPoint",
        live.current.points.map((p) => p.id),
        "replace",
      );
    } else if (mod && !e.shiftKey && !e.altKey && (k === "c" || k === "x" || k === "v" || k === "d")) {
      handled();
      if (k === "c") copy();
      else if (k === "x") cut();
      else if (k === "v") void pasteAt(playheadBeats());
      else duplicate();
    } else if (!mod && (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      if (selectedHere().length === 0) return;
      handled();
      const dir = e.key === "ArrowUp" || e.key === "ArrowRight" ? 1 : -1;
      if (e.key === "ArrowUp" || e.key === "ArrowDown") nudge(0, dir, e.shiftKey);
      else nudge(dir, 0, e.shiftKey);
    }
  };

  // ---- Menus ----------------------------------------------------------------------------

  const setCurve = (ids: AutomationPointId[], curve: CurveShape) => void sendEdit(transport, setCurveCommand(ids, curve));

  const pointMenu = (e: MouseEvent, p: AutomationPoint) => {
    if (!selection.getState().isSelected("automationPoint", p.id)) selection.getState().select("automationPoint", [p.id], "replace");
    const ids = selectedHere().map((x) => x.id);
    const many = ids.length > 1;
    const items: ContextMenuEntry[] = [
      { label: "Cut", shortcut: `${MOD_KEY}X`, onSelect: cut },
      { label: "Copy", shortcut: `${MOD_KEY}C`, onSelect: () => void copy() },
      { label: "Paste at Playhead", shortcut: `${MOD_KEY}V`, disabled: !hasPointClipboard(), onSelect: () => void pasteAt(playheadBeats()) },
      { label: "Duplicate", shortcut: `${MOD_KEY}D`, onSelect: duplicate },
      "separator",
      { label: "Set Value…", onSelect: () => setEditing({ ids, plain: paramToPlain(info, p.value) }) },
      { label: "Linear Curve", onSelect: () => setCurve(ids, { type: "Linear" }) },
      { label: "Step Curve", onSelect: () => setCurve(ids, { type: "Step" }) },
      { label: "Smooth Curve", onSelect: () => setCurve(ids, { type: "Curve", tension: 0.5 }) },
      "separator",
      { label: many ? `Delete ${ids.length} Points` : "Delete Point", shortcut: "⌫", danger: true, onSelect: () => void sendEdit(transport, removePointsCommand(ids)) },
    ];
    openContextMenu(e, items);
  };

  const laneMenu = (e: MouseEvent<SVGSVGElement>) => {
    const at = timeAt(local(e).x, e.altKey);
    const ids = selectedHere().map((x) => x.id);
    openContextMenu(e, [
      { label: "Paste Here", disabled: !hasPointClipboard(), onSelect: () => void pasteAt(at) },
      { label: "Paste at Playhead", shortcut: `${MOD_KEY}V`, disabled: !hasPointClipboard(), onSelect: () => void pasteAt(playheadBeats()) },
      "separator",
      {
        label: "Select All Points",
        shortcut: `${MOD_KEY}A`,
        disabled: live.current.points.length === 0,
        onSelect: () => selection.getState().select("automationPoint", live.current.points.map((p) => p.id), "replace"),
      },
      { label: "Copy", shortcut: `${MOD_KEY}C`, disabled: ids.length === 0, onSelect: () => void copy() },
      {
        label: ids.length > 1 ? `Delete ${ids.length} Points` : "Delete",
        shortcut: "⌫",
        danger: true,
        disabled: ids.length === 0,
        onSelect: () => void sendEdit(transport, removePointsCommand(ids)),
      },
    ]);
  };

  const step = paramStep(info);
  const lo = Math.min(info.min, info.max);
  const hi = Math.max(info.min, info.max);
  const commitValue = () => {
    if (!editing) return;
    const v = snapValue(info, paramToNormalized(info, Math.min(hi, Math.max(lo, editing.plain))));
    void sendEdit(transport, editPointsCommand(editing.ids.map((id) => ({ id, time: null, value: v, curve: null }))));
    setEditing(null);
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
        onContextMenu={laneMenu}
        data-testid="automation-lane-svg"
      >
        {lines.map((l) => {
          const yy = valueToY(l.value, height, range);
          return (
            <line
              key={l.value}
              className={clsx("eth-auto-lane__step", l.major && "eth-auto-lane__step--major")}
              x1={0}
              x2="100%"
              y1={yy}
              y2={yy}
              data-step={l.label ?? undefined}
            />
          );
        })}
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
              cy={valueToY(p.value, height, range)}
              r={POINT_RADIUS}
              data-point={p.id}
              aria-selected={on}
              onPointerDown={(e) => onPointPointerDown(e, p)}
              onDoubleClick={(e) => {
                e.stopPropagation();
                void sendEdit(transport, removePointsCommand([p.id]));
              }}
              onContextMenu={(e) => pointMenu(e, p)}
            >
              <title>{formatNormalized(info, p.value)}</title>
            </circle>
          );
        })}
        {r && <rect className="eth-auto-lane__marquee" x={r.x0} y={r.y0} width={r.x1 - r.x0} height={r.y1 - r.y0} />}
      </svg>
      {tip && (
        <div
          className={clsx("eth-auto-lane__tip", tip.y < height / 2 && "eth-auto-lane__tip--below")}
          style={{ left: tip.x, top: tip.y }}
          role="status"
          data-testid="automation-drag-tip"
        >
          <span className="eth-auto-lane__tip-value">{tip.value}</span>
          <span className="eth-auto-lane__tip-hint">{tip.hint}</span>
        </div>
      )}
      <Dialog
        open={editing !== null}
        onClose={() => setEditing(null)}
        title={`Set ${info.name}`}
        footer={
          <>
            <Button onClick={() => setEditing(null)}>Cancel</Button>
            <Button variant="primary" onClick={commitValue}>
              Set
            </Button>
          </>
        }
      >
        {editing && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              commitValue();
            }}
          >
            <NumberField
              aria-label={`${info.name} value`}
              value={editing.plain}
              min={lo}
              max={hi}
              step={step ?? (hi - lo) / 100}
              precision={step !== null && Number.isInteger(step) ? 0 : 2}
              onChange={(v) => setEditing((s) => (s ? { ...s, plain: v } : s))}
              autoFocus
            />
          </form>
        )}
      </Dialog>
    </div>
  );
}

/**
 * Tempo lane (the master track's first automation lane in the arrangement, see
 * `TempoAutomationLane`): the tempo curve (steps and ramps) with one handle per tempo point.
 * It edits like the automation lanes and the arranger:
 *
 * - a drag on empty space selects the points in a box; Shift+drag selects a time section
 *   (Shift+click extends it); a click deselects and places the playhead;
 * - click a point: select it (Shift adds, Cmd/Ctrl toggles); drag: move the selection, time
 *   on the grid (Alt: free) and BPM (whole BPM; Shift: fine 0.01 steps, like the transport
 *   bar); the point at beat 0 only changes BPM. One undo step per drag; Escape cancels it;
 * - double-click the lane: add a point there; double-click a point: remove it;
 * - Delete, select all, copy / cut / paste (at the selection cursor) / duplicate, on the
 *   selected points or the section (`tempoEdits.ts`);
 * - right-click: point / lane / section menus (cut, copy, paste, duplicate, ramp or hold,
 *   select all, delete).
 */

import { useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { TempoPoint } from "@/generated";
import { openContextMenu, setDragCursor, useScopedDrag, type ContextMenuEntry } from "@/kit";
import { firstMatch, shortcutLabel } from "@/features/keymap";
import { newId } from "@/transport";
import {
  beatsToPx,
  DEFAULT_GRID,
  itemSelection,
  marqueeHits,
  movedEdge,
  precisionDrag,
  pxToBeats,
  resolveGrid,
  selectModeFromEvent,
  snapToGrid,
  stepLength,
  useMarquee,
  useSelectedItems,
  useTimelineView,
  useViewport,
  type Rect,
  type TempoMap,
  type TimelineViewStore,
} from "@/timeline";
import { LaneRangeOverlay } from "@/features/time-edits/LaneRangeOverlay";
import { pasteTarget as editCursor, setPlayStart } from "@/features/time-edits/marker";
import { MARKER_EPS, rangeOf, useTimeSelection, type TimeRangeSelection } from "@/features/time-edits/store";
import { selectTrackEntity } from "@/features/arrangement/actions";
import { sendEdit, TempoGesture, trackDrag, useTempoTransport } from "./gesture";
import {
  addTempoPointCommand,
  bpmRange,
  bpmToY,
  clampBpm,
  formatBpm,
  isAtZero,
  removeTempoPointsCommand,
  tempoPath,
  tempoTimeTaken,
  yToBpm,
  type LaneGeom,
} from "./tempoCommands";
import {
  batch,
  clipOfPoints,
  clipOfRange,
  deletePointsCommand,
  deleteRangeCommand,
  hasTempoClipboard,
  movePointsCommand,
  pasteCommand,
  removableIds,
  setCurvesCommand,
  setTempoClipboard,
  tempoClipboard,
  type TempoClip,
} from "./tempoEdits";
import { useT } from "@/i18n";

const POINT_RADIUS = 4;
const PAD = 8;
/** Shift-drag BPM sensitivity (transport bar: 0.05 BPM per pixel, 0.01 BPM steps). */
const FINE_BPM_PER_PX = 0.05;
/** The tempo lane's key in a time selection (`automationLane`): its own sections. */
export const TEMPO_RANGE_KEY = "tempo";

/** The lane's keymap actions, in lookup order. */
const TEMPO_ACTIONS = ["edit.delete", "edit.deleteModified", "edit.selectAll", "edit.copy", "edit.cut", "edit.paste", "edit.duplicate"] as const;

export interface TempoLaneProps {
  view: TimelineViewStore;
  tempo: TempoMap;
  points: TempoPoint[];
  /** Lane height (px). */
  height: number;
  /** The master track (the lane's sections and the track selected on a press), if any. */
  track?: string | null;
}

export function TempoLane({ view, tempo, points, height, track = null }: TempoLaneProps) {
  const t = useT();
  const windowDrag = useScopedDrag(trackDrag);
  const transport = useTempoTransport();
  const vp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const selected = useSelectedItems("tempoPoint");
  const rootRef = useRef<HTMLDivElement>(null);
  const [readout, setReadout] = useState<{ x: number; y: number; text: string } | null>(null);
  /** BPM range frozen while dragging (so the lane doesn't rescale under the pointer). */
  const [frozen, setFrozen] = useState<LaneGeom["range"] | null>(null);
  const geom: LaneGeom = { height, pad: PAD, range: frozen ?? bpmRange(points) };
  const live = useRef({ points, geom, tempo, track });
  useLayoutEffect(() => {
    live.current = { points, geom, tempo, track };
  });

  const toX = (b: number) => beatsToPx(b, vp);
  const local = (e: { clientX: number; clientY: number }) => {
    const box = rootRef.current!.getBoundingClientRect();
    return { x: e.clientX - box.left, y: e.clientY - box.top };
  };
  const gridStep = () => {
    const st = view.getState();
    return resolveGrid(DEFAULT_GRID, st.pxPerBeat, live.current.tempo.signatureAt(st.scrollBeats));
  };
  const snap = (beats: number, bypass: boolean) => {
    if (bypass) return Math.max(0, beats);
    return Math.max(0, snapToGrid(beats, gridStep(), live.current.tempo));
  };
  const timeAt = (x: number, bypass = false) => snap(pxToBeats(x, view.getState()), bypass);
  const send = (command: ReturnType<typeof batch>) => (command ? sendEdit(transport, command) : Promise.resolve());

  // ---- Selection and section ---------------------------------------------------------------

  const sel = () => itemSelection.getState();
  const selectedIds = () => {
    const ids = sel().selected.tempoPoint;
    return new Set(live.current.points.filter((p) => ids.has(p.id)).map((p) => p.id));
  };
  /** This lane's time section, or null. */
  const laneRange = (): TimeRangeSelection | null => {
    const r = rangeOf(useTimeSelection.getState().selection);
    const t = live.current.track;
    return r && t && r.automationLane === TEMPO_RANGE_KEY && r.tracks[0] === t ? r : null;
  };
  const selectRange = (start: number, end: number) => {
    const t = live.current.track;
    if (!t) return;
    useTimeSelection.getState().setSelection({ start: Math.max(0, start), end: Math.max(0, end), tracks: [t], automationLane: TEMPO_RANGE_KEY });
  };
  const clearRange = () => {
    if (laneRange()) useTimeSelection.getState().setSelection(null);
  };
  const selectAll = () => {
    clearRange();
    sel().select(
      "tempoPoint",
      live.current.points.map((p) => p.id),
      "replace",
    );
  };

  // ---- Edits --------------------------------------------------------------------------------

  const copy = (): boolean => {
    const r = laneRange();
    const clip = r ? clipOfRange(live.current.points, r.start, r.end) : clipOfPoints(live.current.points, selectedIds());
    if (clip) setTempoClipboard(clip);
    return clip !== null;
  };
  const deleteSelection = () => {
    const r = laneRange();
    const { points: pts } = live.current;
    void send(r ? deleteRangeCommand(pts, r.start, r.end) : deletePointsCommand(pts, selectedIds()));
  };
  const cut = () => {
    if (copy()) deleteSelection();
  };
  /** Paste `clip` at `at`; the pasted points (or a pasted section) become the selection. */
  const pasteAt = async (at: number, clip: TempoClip | null = tempoClipboard()) => {
    if (!clip) return;
    const { command, ids } = pasteCommand(live.current.points, clip, at);
    if (!command) return;
    await send(command);
    if (clip.range) selectRange(Math.max(0, at), Math.max(0, at) + clip.span);
    else sel().select("tempoPoint", ids, "replace");
  };
  const duplicate = () => {
    const r = laneRange();
    if (r) return void pasteAt(r.end, clipOfRange(live.current.points, r.start, r.end));
    const ids = selectedIds();
    const clip = clipOfPoints(live.current.points, ids);
    if (!clip) return;
    const first = Math.min(...live.current.points.filter((p) => ids.has(p.id)).map((p) => p.time));
    const step = gridStep();
    const len = step ? stepLength(step, live.current.tempo.signatureAt(first)) : 1;
    void pasteAt(first + clip.span + len, clip);
  };
  const removeOne = (point: TempoPoint) => {
    if (isAtZero(point)) return;
    sel().select("tempoPoint", [point.id], "remove");
    void sendEdit(transport, removeTempoPointsCommand([point.id]));
  };

  // ---- Pointer ------------------------------------------------------------------------------

  const onPointPointerDown = (e: ReactPointerEvent, point: TempoPoint) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    rootRef.current?.focus();
    clearRange();
    const mode = selectModeFromEvent(e);
    const was = sel().isSelected("tempoPoint", point.id);
    if (mode === "toggle") {
      sel().select("tempoPoint", [point.id], "toggle");
      return;
    }
    if (mode === "add") sel().select("tempoPoint", [point.id], "add");
    else if (!was) sel().select("tempoPoint", [point.id], "replace");
    const ids = selectedIds();
    const orig = live.current.points;
    const start = local(e);
    const range = live.current.geom.range;
    const g: LaneGeom = { ...live.current.geom, range };
    const gesture = new TempoGesture(transport);
    const offset = precisionDrag(start);
    let cancelled = false;
    // Escape cancels the drag: the engine takes back what it applied and keeps no undo
    // step (restoring the points by hand left a no-op step behind).
    const onKey = (ev: globalThis.KeyboardEvent) => {
      if (ev.key !== "Escape" || cancelled) return;
      ev.preventDefault();
      ev.stopPropagation();
      cancelled = true;
      gesture.cancel();
      setReadout(null);
      setDragCursor(null);
    };
    window.addEventListener("keydown", onKey, true);
    windowDrag(
      e,
      (ev) => {
        if (cancelled) return;
        if (!frozen) setFrozen(range);
        setDragCursor("grabbing");
        // Alt: precision, 1/10 of the pointer's motion and off the grid; Shift's fine
        // steps win for the BPM, so with Shift + Alt only the time is slowed down.
        const { dx, dy } = offset(local(ev), { x: ev.altKey, y: ev.altKey && !ev.shiftKey });
        const time = isAtZero(point) ? point.time : snap(point.time + dx / view.getState().pxPerBeat, ev.altKey);
        // Shift = fine steps, as the user's tempo drag in the transport bar; Alt keeps
        // hundredths (whole BPM would undo the slower motion).
        const bpm = clampBpm(
          ev.shiftKey
            ? Math.round((point.bpm - dy * FINE_BPM_PER_PX) * 100) / 100
            : ev.altKey
              ? Math.round(yToBpm(bpmToY(point.bpm, g) + dy, g) * 100) / 100
              : Math.round(yToBpm(bpmToY(point.bpm, g) + dy, g)),
        );
        const command = movePointsCommand(orig, ids, time - point.time, bpm - point.bpm);
        if (!command) return;
        setReadout({ x: toX(time), y: bpmToY(bpm, g), text: `${formatBpm(bpm)} bpm` });
        gesture.update(command);
      },
      (dragged) => {
        window.removeEventListener("keydown", onKey, true);
        setDragCursor(null);
        gesture.end();
        setReadout(null);
        setFrozen(null);
        // A plain click on a point of a selection selects just it.
        if (!dragged && !cancelled && mode === "replace" && was) sel().select("tempoPoint", [point.id], "replace");
      },
    );
  };

  const pointBox = (p: TempoPoint): Rect => {
    const x = toX(p.time);
    const y = bpmToY(p.bpm, live.current.geom);
    return { x0: x - POINT_RADIUS, y0: y - POINT_RADIUS, x1: x + POINT_RADIUS, y1: y + POINT_RADIUS };
  };
  const rectTimes = (r: Rect) => [timeAt(r.x0, true), timeAt(r.x1, true)] as const;
  const showSection = (r: Rect) => {
    const [a, b] = rectTimes(r);
    const [s, t] = [snap(a, false), snap(b, false)];
    if (Math.abs(t - s) > MARKER_EPS) selectRange(Math.min(s, t), Math.max(s, t));
  };
  // Like the arranger: a plain drag selects points in a box, Shift+drag a time section.
  const marquee = useMarquee({
    kind: "tempoPoint",
    hitTest: (rect) =>
      marqueeHits(
        rect,
        live.current.points.map((p) => ({ id: p.id, rect: pointBox(p) })),
      ),
    onClick: (p) => {
      clearRange();
      if (transport) setPlayStart(transport, timeAt(p.x));
    },
    section: {
      onPreview: (r) => (r ? showSection(r) : clearRange()),
      onEnd: (r) => {
        showSection(r);
        const range = laneRange();
        if (range && transport) setPlayStart(transport, range.start);
      },
      onShiftClick: (p) => {
        const prev = laneRange();
        if (!prev) return false;
        const at = timeAt(p.x);
        const keep = movedEdge(prev, at) === "start" ? prev.end : prev.start;
        selectRange(Math.min(keep, at), Math.max(keep, at));
        return true;
      },
    },
  });

  const onBackgroundPointerDown = (e: ReactPointerEvent<SVGSVGElement>) => {
    // The lane owns its pointer events (the arrangement has its own clip marquee).
    e.stopPropagation();
    rootRef.current?.focus();
    if (e.button !== 0) return;
    if (track) selectTrackEntity(track, "replace");
    marquee.onPointerDown(e);
  };

  const onDoubleClick = (e: MouseEvent<SVGSVGElement>) => {
    e.stopPropagation();
    if (e.target !== e.currentTarget) return;
    const p = local(e);
    const time = snap(pxToBeats(p.x, view.getState()), e.altKey);
    const { points, tempo } = live.current;
    if (tempoTimeTaken(points, time)) return;
    // Like automation lanes: the tempo there (not the pointer's y), continuing the ramp or
    // hold it splits (a linear ramp split in two is the same ramp), so the map doesn't change.
    const bpm = clampBpm(tempo.bpmAt(time));
    const curve = points.filter((q) => q.time <= time).at(-1)?.curve ?? "Step";
    const id = newId();
    void sendEdit(transport, addTempoPointCommand(time, bpm, curve, id)).then(() => sel().select("tempoPoint", [id], "replace"));
  };

  // ---- Keyboard -----------------------------------------------------------------------------

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // keymap: the arranger's chords (see the registry); kept in the lane (bubbling up, they
    // would edit the arrangement's clips).
    const action = firstMatch(TEMPO_ACTIONS, e);
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    if (action === "edit.delete" || action === "edit.deleteModified") deleteSelection();
    else if (action === "edit.selectAll") selectAll();
    else if (action === "edit.copy") copy();
    else if (action === "edit.cut") cut();
    else if (action === "edit.paste") void pasteAt(editCursor());
    else duplicate();
  };

  // ---- Menus --------------------------------------------------------------------------------

  const clipboardEntries = (canCopy: boolean): ContextMenuEntry[] => [
    { label: t("arrangement.menu.cut"), shortcut: shortcutLabel("edit.cut"), disabled: !canCopy, onSelect: cut },
    { label: t("arrangement.menu.copy"), shortcut: shortcutLabel("edit.copy"), disabled: !canCopy, onSelect: () => void copy() },
    {
      label: t("arrangement.menu.paste"),
      shortcut: shortcutLabel("edit.paste"),
      disabled: !hasTempoClipboard(),
      reason: t("tempo.menu.paste_reason"),
      onSelect: () => void pasteAt(editCursor()),
    },
    { label: t("arrangement.menu.duplicate"), shortcut: shortcutLabel("edit.duplicate"), disabled: !canCopy, onSelect: duplicate },
  ];

  const pointMenu = (e: MouseEvent, p: TempoPoint) => {
    clearRange();
    if (!sel().isSelected("tempoPoint", p.id)) sel().select("tempoPoint", [p.id], "replace");
    const ids = selectedIds();
    const pts = live.current.points.filter((x) => ids.has(x.id));
    const ramped = pts.every((x) => x.curve === "Linear");
    const removable = removableIds(live.current.points, ids);
    openContextMenu(e, [
      ...clipboardEntries(true),
      "separator",
      {
        label: ramped ? t("tempo.menu.hold") : t("tempo.menu.ramp"),
        onSelect: () => void send(setCurvesCommand(live.current.points, ids, ramped ? "Step" : "Linear")),
      },
      "separator",
      {
        label: t("tempo.menu.delete_points", { count: removable.length }),
        shortcut: shortcutLabel("edit.delete"),
        danger: true,
        disabled: removable.length === 0,
        reason: t("tempo.menu.delete_points_reason"),
        onSelect: () => void send(deletePointsCommand(live.current.points, ids)),
      },
    ]);
  };

  const laneMenu = (e: MouseEvent<SVGSVGElement>) => {
    if (e.target !== e.currentTarget) return;
    const r = laneRange();
    const raw = pxToBeats(local(e).x, view.getState());
    if (r && raw >= r.start && raw <= r.end) {
      openContextMenu(e, [
        ...clipboardEntries(true),
        "separator",
        { label: t("arrangement.menu.delete"), shortcut: shortcutLabel("edit.delete"), danger: true, onSelect: deleteSelection },
      ]);
      return;
    }
    const at = timeAt(local(e).x, e.altKey);
    const ids = selectedIds();
    openContextMenu(e, [
      { label: t("automation.menu.paste_here"), disabled: !hasTempoClipboard(), reason: t("tempo.menu.paste_reason"), onSelect: () => void pasteAt(at) },
      {
        label: t("automation.menu.paste_at_cursor"),
        shortcut: shortcutLabel("edit.paste"),
        disabled: !hasTempoClipboard(),
        reason: t("tempo.menu.paste_reason"),
        onSelect: () => void pasteAt(editCursor()),
      },
      "separator",
      { label: t("automation.menu.select_all"), shortcut: shortcutLabel("edit.selectAll"), onSelect: selectAll },
      { label: t("arrangement.menu.copy"), shortcut: shortcutLabel("edit.copy"), disabled: ids.size === 0, onSelect: () => void copy() },
      {
        label: t("arrangement.menu.delete"),
        shortcut: shortcutLabel("edit.delete"),
        danger: true,
        disabled: removableIds(live.current.points, ids).length === 0,
        onSelect: deleteSelection,
      },
    ]);
  };

  const path = tempoPath(points, toX, geom, widthPx);
  const m = marquee.rect;
  return (
    <div
      ref={rootRef}
      className="eth-tempo-lane"
      style={{ height }}
      tabIndex={0}
      role="group"
      aria-label={t("tempo.lane.label")}
      onKeyDown={onKeyDown}
      data-testid="tempo-lane"
      data-keymap-scope="automation"
    >
      {track && <LaneRangeOverlay view={view} track={track} targetKey={TEMPO_RANGE_KEY} height={height} />}
      <svg
        className="eth-tempo-lane__svg"
        width="100%"
        height={height}
        onPointerDown={onBackgroundPointerDown}
        onDoubleClick={onDoubleClick}
        onContextMenu={laneMenu}
        data-testid="tempo-lane-svg"
      >
        {path && <path className="eth-tempo-lane__line" d={path} />}
        {points.map((p) => {
          const x = toX(p.time);
          if (widthPx > 0 && (x < -POINT_RADIUS || x > widthPx + POINT_RADIUS)) return null;
          const on = selected.has(p.id);
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
                removeOne(p);
              }}
              onContextMenu={(e) => {
                e.stopPropagation();
                pointMenu(e, p);
              }}
            >
              <title>{`${formatBpm(p.bpm)} bpm${p.curve === "Linear" ? " (ramp)" : ""}`}</title>
            </circle>
          );
        })}
        {m && <rect className="eth-auto-lane__marquee" x={m.x0} y={m.y0} width={m.x1 - m.x0} height={m.y1 - m.y0} />}
        {readout && (
          <text className="eth-tempo-lane__readout" x={readout.x + 8} y={Math.max(12, readout.y - 6)}>
            {readout.text}
          </text>
        )}
      </svg>
    </div>
  );
}

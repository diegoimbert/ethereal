/**
 * One automation lane drawn in SVG: the curve (engine formula), breakpoint handles, step
 * gridlines and the editing interactions (see README "Automation editing"):
 *
 * - like the arranger: a drag on the background selects the points in a box
 *   (cmd/ctrl-drag toggles them); on track lanes a click selects the track and the lane and
 *   places the insert marker (like the track's empty clip-lane space), and a shift-drag
 *   selects a time range in the lane (shift-click extends it); a shift-drag ON THE LINE inside
 *   a time section (the lane's, or the track row's) moves that section up/down and nothing
 *   outside it (`sectionMovePlan`);
 * - double-click the background: a point is added on the second press at the clicked time
 *   (on the grid, alt = no snap) with the lane's value there, so the lane doesn't change
 *   (an empty lane: the target's current value). Holding and moving past 3 px drags it like
 *   a grabbed point, from that value (one undo step). Double presses are detected on
 *   pointerdown (`createDoublePress`), never on `dblclick`;
 * - click a point: select it (shift = add, cmd/ctrl = toggle); drag: move the selection.
 *   Time snaps to the grid, stepped params (semitones, enums, toggles) snap to their steps;
 *   alt: off the time grid; shift held while dragging: whole value increments of
 *   continuous params (1 dB, 1 %...); cmd/ctrl while dragging: lock to the dominant axis.
 *   A tooltip shows the value and the modifiers while dragging;
 * - double-click a point: delete it; Delete/Backspace: delete the selected points;
 * - cmd/ctrl+A: select all points of the lane;
 * - cmd/ctrl+C / X / V / D: copy, cut, paste at the selection cursor (after a time
 *   selection, at the insert marker; never the playhead), duplicate (one undo step). With a
 *   time range in the lane, they (and Delete) act on the range (`range.ts`);
 * - arrows: nudge the selection (up/down: one step or 1 %, shift = to the next whole
 *   increment; left/right: one grid step, shift = a quarter);
 * - drag the line (not a point) up/down: move that segment's two points (times kept);
 * - alt-drag a segment up/down: bend it (sets `CurveShape::Curve { tension }`);
 * - right-click: point / lane / range menus (cut, copy, paste, duplicate, curve, set value,
 *   delete).
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
import type {
  AutomationLane,
  AutomationOwner,
  AutomationPoint,
  AutomationPointId,
  AutomationTarget,
  Command,
  CurveShape,
  ParamInfo,
  PointEdit,
} from "@/generated";
import { dragEnd, openContextMenu, useDragScope, type ContextMenuEntry } from "@/kit";
import { firstMatch, shortcutLabel } from "@/features/keymap";
import { paramToNormalized, paramToPlain } from "@/domain/paramScale";
import {
  createDoublePress,
  DEFAULT_GRID,
  itemSelection,
  marqueeHits,
  movedEdge,
  precisionDrag,
  PRECISION_RATIO,
  resolveGrid,
  selectModeFromEvent,
  snapToGrid,
  stepLength,
  createTimelineViewStore,
  useMarquee,
  useSelectedItems,
  useTempoMap,
  useTimelineView,
  useViewport,
  type GridSetting,
  type ItemSelectionStore,
  type TempoMap,
  type TimelineViewStore,
} from "@/timeline";
import { cmd, newId, useTransport } from "@/transport";
import { pasteTarget as editCursorTarget, placeInsertMarker, setPlayStart } from "@/features/time-edits/marker";
import { isTimeTrack } from "@/features/time-edits/commands";
import { isMarker, MARKER_EPS, rangeOf, useTimeSelection, type TimeRangeSelection } from "@/features/time-edits/store";
import { runClipAction, selectTrackEntity, type ClipAction } from "@/features/arrangement/actions";
import { clipsFollowing, hasMixedClipboard, isMixedSelection, MOVE_SELECTION, type ClipsFollowing } from "@/features/arrangement/mixedSelection";
import { usePointsOfLane, useProjectStore } from "@/state";
import {
  copyLanes,
  copyPoints,
  copyRange,
  duplicateAt,
  hasPointClipboard,
  multiLaneClipboard,
  pasteLanes,
  pastePoints,
  pointClipboard,
  snapshotLanes,
  rangeEditCommand,
  snapshotPoints,
  snapshotRange,
  type PasteTarget,
  type PointClipboard,
} from "./clipboard";
import { addPointCommand, bendTension, editPointsCommand, moveEdits, removePointsCommand, setCurveCommand, tensionOf } from "./edit";
import { LANE_PAD, lanePath, pointRect, POINT_RADIUS, segmentAt, timeToX, valueToY, xToTime, yToValue, type LaneGeometry } from "./geometry";
import { LaneGesture, sendEdit } from "./gesture";
import { evaluatePoints, splitPointAt } from "./curve";
import { sectionMovePlan, writeRange } from "./range";
import { useAutomationUi } from "./uiStore";
import { flattenCommands, laneOf, registerLane, selectedByLane, type DragMods, type RegisteredLane } from "./laneRegistry";
import { startLaneMarquee } from "./laneMarquee";
import { emptyLaneBaseline, useTargetValue } from "./baseline";
import { formatNormalized, targetKey as keyOfTarget } from "./params";
import { PointValuePopover, type PointValueEdit } from "./PointValuePopover";
import { dragHint, FULL_RANGE, incrementNudge, nudgeSize, paramStep, snapValue, stepDragDelta, stepLines, type ValueRange } from "./valueAxis";
import { automationPointAt, publishAutomationLane } from "@/features/arrangement/gl/layers/automationLanes";
import { LaneRangeOverlay } from "@/features/time-edits/LaneRangeOverlay";
import { useT } from "@/i18n";
import { batchStep, keyedStep, type StepLabel } from "@/features/arrangement/editMath";
import "./automation.css";
import "@/features/time-edits/time-edits.css";

/** The lane's keymap actions (automation scope), in lookup order. */
const AUTOMATION_ACTIONS = [
  "automation.drawMode",
  "edit.delete",
  "edit.deleteModified",
  "edit.selectAll",
  "edit.copy",
  "edit.cut",
  "edit.paste",
  "edit.duplicate",
  "nudge.up",
  "nudge.down",
  "nudge.left",
  "nudge.right",
  "automation.fineUp",
  "automation.fineDown",
  "automation.fineLeft",
  "automation.fineRight",
  "automation.preciseUp",
  "automation.preciseDown",
] as const;

/** Nudge actions -> [time steps, value steps, fine (⇧), precise (⌥: a tenth of a step)]. */
/** Edit keys that act on clips and points together when both are selected (keymap id → clip action). */
const MIXED_ACTIONS: Record<string, ClipAction> = {
  "edit.delete": "delete",
  "edit.deleteModified": "delete",
  "edit.copy": "copy",
  "edit.cut": "cut",
  "edit.paste": "paste",
  "edit.duplicate": "duplicate",
};

const AUTOMATION_NUDGES: Partial<Record<string, readonly [number, number, boolean, boolean?]>> = {
  "nudge.up": [0, 1, false],
  "nudge.down": [0, -1, false],
  "nudge.left": [-1, 0, false],
  "nudge.right": [1, 0, false],
  "automation.fineUp": [0, 1, true],
  "automation.fineDown": [0, -1, true],
  "automation.fineLeft": [-1, 0, true],
  "automation.fineRight": [1, 0, true],
  "automation.preciseUp": [0, 1, false, true],
  "automation.preciseDown": [0, -1, false, true],
};

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
  /**
   * Drawn by the WebGL arranger (`arrangement/gl/layers/automationLanes.ts`): the SVG only
   * takes the pointer (no element per point); the lane publishes what it shows instead.
   * Track lanes only.
   */
  gl?: boolean;
  /**
   * Song beats (the edit cursor, for "paste at cursor") → lane time. Default: identity
   * (track lanes). Clip envelopes map to the content position playing there.
   */
  cursorToLane?: (song: number) => number;
  /** Tempo map of the view's axis (grid snapping; default: the song's). */
  tempo?: TempoMap;
}

interface ClipboardLane {
  root: { readonly current: HTMLElement | null };
  handle(type: string): void;
}

/** Mounted lanes; one capture listener per event type routes to the focused one. */
const clipboardLanes = new Set<ClipboardLane>();
let clipboardInstalled = false;

function registerClipboardLane(lane: ClipboardLane): () => void {
  clipboardLanes.add(lane);
  if (!clipboardInstalled && typeof window !== "undefined") {
    clipboardInstalled = true;
    const route = (e: Event) => {
      for (const l of clipboardLanes) {
        if (!l.root.current?.contains(document.activeElement)) continue;
        e.preventDefault();
        e.stopPropagation();
        l.handle(e.type);
        return;
      }
    };
    for (const type of ["copy", "cut", "paste"]) window.addEventListener(type, route, true);
  }
  return () => {
    clipboardLanes.delete(lane);
  };
}

/** A view that never moves (WebGL lanes don't re-render with the view, see `geomNow`). */
const STILL_VIEW = createTimelineViewStore();

/** Pixels before a press on a point turns into a drag. */
const DRAG_THRESHOLD = 2;
/**
 * Pixels before a point just created by a double-click (held) moves, like a clip drag; on
 * touch, the touch slop. Below it the point keeps exactly the lane's value at its time.
 */
const CREATE_THRESHOLD_PX = 3;
/** Draw mode without a grid (or with ⌥): one point every this many px. */
const FREE_DRAW_PX = 4;
const CREATE_TOUCH_THRESHOLD_PX = 8;
/** How close (px, vertically) to the lane's line a ⇧-press grabs a time section. */
const LINE_HIT_PX = 6;
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
  gl = false,
  cursorToLane,
  tempo: tempoProp,
}: AutomationLaneViewProps) {
  const transport = useTransport();
  const t = useT();
  const songTempo = useTempoMap();
  const tempo = tempoProp ?? songTempo;
  // WebGL: the canvas follows the view itself; the lane doesn't re-render per pan frame and
  // reads the view when it needs it (`geomNow`).
  const vp = useViewport(gl ? STILL_VIEW : view);
  const width = useTimelineView(view, (s) => s.widthPx) || FALLBACK_WIDTH;
  const points = usePointsOfLane(lane?.id ?? "");
  const selected = useSelectedItems("automationPoint", selection);
  const [tip, setTip] = useState<Tip | null>(null);
  const [editing, setEditing] = useState<PointValueEdit | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const dragScope = useDragScope();
  /** Second press of a double-click, seen on `pointerdown` (so a double-click can drag). */
  const [doublePress] = useState(createDoublePress);
  const tKey = useMemo(() => keyOfTarget(target), [target]);
  /** The target's document value (what the knob shows); an empty lane draws it, dashed. */
  const current = useTargetValue(target, info);
  const baseline = emptyLaneBaseline(points.length, current);

  const geom: LaneGeometry = useMemo(() => ({ vp, height, offset, range }), [vp, height, offset, range]);
  const path = useMemo(() => (gl ? "" : lanePath(points, geom, 0, width)), [gl, points, geom, width]);
  const lines = useMemo(() => stepLines(info, range, height - 2 * LANE_PAD), [info, range, height]);

  // Latest values for window-level drag handlers.
  const live = useRef({ points, geom, lane, grid, tempo, info, owner, target, tKey, cursorToLane, current });
  useLayoutEffect(() => {
    live.current = { points, geom, lane, grid, tempo, info, owner, target, tKey, cursorToLane, current };
  });

  /** The lane geometry now (WebGL: at the view's current position). */
  const geomNow = useCallback((): LaneGeometry => {
    const g = live.current.geom;
    if (!gl) return g;
    const s = view.getState();
    return { ...g, vp: { pxPerBeat: s.pxPerBeat, scrollBeats: s.scrollBeats } };
  }, [gl, view]);

  const gridStep = useCallback(() => {
    const { grid: g, tempo: t } = live.current;
    const gm = geomNow();
    return resolveGrid(g, gm.vp.pxPerBeat, t.signatureAt(gm.vp.scrollBeats));
  }, [geomNow]);

  const snapFn = useCallback(
    (bypass: boolean) => {
      const step = bypass ? null : gridStep();
      if (!step) return null;
      const { tempo: t } = live.current;
      const gm = geomNow();
      const off = gm.offset ?? 0;
      return (time: number) => Math.max(0, snapToGrid(time + off, step, t) - off);
    },
    [gridStep, geomNow],
  );

  const marquee = useMarquee({
    kind: "automationPoint",
    store: selection,
    onClick: (p, e) => placeCursor(p.x, e.altKey),
    hitTest: (rect) =>
      marqueeHits(
        rect,
        live.current.points.map((p) => ({ id: p.id, rect: pointRect(p, geomNow()) })),
      ),
  });

  const local = (e: { clientX: number; clientY: number }) => {
    const box = rootRef.current?.getBoundingClientRect() ?? { left: 0, top: 0 };
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

  // ---- Across lanes (track lanes: `laneRegistry.ts`) -------------------------------------

  /** Edits moving some of this lane's points by `dt` beats and `dyPx` px (`RegisteredLane.dragEdits`). */
  const laneDragEdits = (originals: ReadonlyArray<AutomationPoint>, anchor: AutomationPoint, dt: number, dyPx: number, mods: DragMods): PointEdit[] => {
    const g = geomNow();
    const inf = live.current.info;
    const usable = Math.max(1, g.height - 2 * LANE_PAD);
    const span = (g.range ?? FULL_RANGE).hi - (g.range ?? FULL_RANGE).lo;
    // Stepped params: whole steps at a fixed px-per-step; others follow the pointer.
    const dv = stepDragDelta(inf, -dyPx) ?? (-dyPx / usable) * span;
    const moving = new Set(originals.map((p) => p.id));
    return moveEdits(originals, anchor, dt, dv, null, {
      lockTime: mods.lockTime,
      lockValue: mods.lockValue,
      others: live.current.points.filter((p) => !moving.has(p.id)),
      snapValue: (v) => snapValue(inf, v, mods.increments),
    });
  };
  /** Edits nudging some of this lane's points by `dt` beats and `dvSteps` value steps. */
  const laneNudgeEdits = (pts: ReadonlyArray<AutomationPoint>, dt: number, dvSteps: number, fine: boolean, precise: boolean): PointEdit[] => {
    if (pts.length === 0) return [];
    const inf = live.current.info;
    const anchor = pts[0]!;
    const stepped = paramStep(inf) !== null;
    // ⇧↑/⇧↓ on a continuous param: to the next whole increment (1 dB, 1 %...), on round values.
    const increments = fine && dvSteps !== 0 && !stepped;
    // ⌥↑/⌥↓: a tenth of a step (stepped params can't go finer than one step).
    const dv = increments
      ? incrementNudge(inf, anchor.value, dvSteps) - anchor.value
      : dvSteps * nudgeSize(inf, fine) * (precise && !stepped ? PRECISION_RATIO : 1);
    const moving = new Set(pts.map((p) => p.id));
    return moveEdits(pts, anchor, dt, dv, null, {
      others: live.current.points.filter((p) => !moving.has(p.id)),
      lockTime: dt === 0,
      lockValue: dvSteps === 0,
      snapValue: stepped || increments ? (v) => snapValue(inf, v, increments) : undefined,
    });
  };
  const impl = useRef({ dragEdits: laneDragEdits, nudgeEdits: laneNudgeEdits });
  useLayoutEffect(() => {
    impl.current = { dragEdits: laneDragEdits, nudgeEdits: laneNudgeEdits };
  });
  const trackOwner = owner.type === "Track" ? owner.track : null;
  /** This lane as the registry sees it (registered for track lanes only). */
  const selfLane = useMemo<RegisteredLane>(
    () => ({
      view,
      track: trackOwner ?? "",
      key: tKey,
      el: () => rootRef.current,
      points: () => live.current.points,
      geom: geomNow,
      pasteTarget: () => {
        const l = live.current;
        return { lane: l.lane, owner: l.owner, target: l.target, info: l.info, targetKey: l.tKey };
      },
      dragEdits: (...a) => impl.current.dragEdits(...a),
      nudgeEdits: (...a) => impl.current.nudgeEdits(...a),
    }),
    [view, trackOwner, tKey, geomNow],
  );
  useEffect(() => (trackOwner ? registerLane(selfLane) : undefined), [selfLane, trackOwner]);

  /**
   * The selected points of each lane: this lane's first, then (track lanes) those of the
   * other lanes on this timeline, so one gesture moves / deletes / copies all of them.
   */
  const selectedGroups = (): Array<{ lane: RegisteredLane; points: AutomationPoint[] }> => {
    const own = { lane: selfLane, points: selectedHere() };
    const others = trackOwner ? selectedByLane(view, selection.getState().selected.automationPoint).filter((g) => g.lane !== selfLane) : [];
    return own.points.length ? [own, ...others] : others;
  };
  /** The destination of a multi-lane paste: the lane of `track` / `key` shown on this timeline. */
  const laneDest = (track: string, key: string) => laneOf(view, track, key)?.pasteTarget() ?? null;
  /** A snapshot of the selected points of several lanes (null for one lane). */
  const multiSnapshot = () => {
    const groups = selectedGroups();
    if (groups.length < 2) return null;
    return snapshotLanes(groups.map((g) => ({ track: g.lane.track, key: g.lane.key, info: g.lane.pasteTarget().info, points: g.points })));
  };

  // ---- Time range (track lanes: the lane's own time selection) ---------------------------

  /** This lane's time range (a time selection drawn in it), or null. */
  const laneRange = (): TimeRangeSelection | null => {
    const sel = rangeOf(useTimeSelection.getState().selection);
    const l = live.current;
    if (l.owner.type !== "Track" || !sel) return null;
    return sel.automationLane === l.tKey && sel.tracks[0] === l.owner.track ? sel : null;
  };
  const selectRange = (start: number, end: number) => {
    const l = live.current;
    if (l.owner.type !== "Track") return;
    useTimeSelection.getState().setSelection({ start: Math.max(0, start), end: Math.max(0, end), tracks: [l.owner.track], automationLane: l.tKey });
  };
  /** Write `clip` (a range copy; null: delete) over `[at, at + length]`, one undo step. */
  const writeOver = (clip: PointClipboard | null, at: number, length: number, label: StepLabel) =>
    sendEdit(transport, rangeEditCommand(clip, pasteTarget(), live.current.points, at, length, label));
  const rangeEdits = {
    copy: (r: TimeRangeSelection) => copyRange(live.current.points, r.start, r.end, live.current.info, live.current.tKey),
    cut: (r: TimeRangeSelection) => {
      if (rangeEdits.copy(r)) void rangeEdits.delete(r);
    },
    delete: (r: TimeRangeSelection) => writeOver(null, r.start, r.end - r.start, keyedStep("undo.automation.delete_range", "delete automation")),
    /** A copy right after the range (over what is there); the copy becomes the selection. */
    duplicate: (r: TimeRangeSelection) => {
      const clip = snapshotRange(live.current.points, r.start, r.end, live.current.info, live.current.tKey);
      if (!clip) return;
      const len = r.end - r.start;
      void writeOver(clip, r.end, len, keyedStep("undo.automation.duplicate_range", "duplicate automation")).then(() => selectRange(r.end, r.end + len));
    },
  };

  // ---- Clipboard ------------------------------------------------------------------------

  const copy = (): number => {
    const r = laneRange();
    if (r) return rangeEdits.copy(r) ? 1 : 0;
    const groups = selectedGroups();
    if (groups.length > 1) {
      return copyLanes(groups.map((g) => ({ track: g.lane.track, key: g.lane.key, info: g.lane.pasteTarget().info, points: g.points })));
    }
    return copyPoints(selectedHere(), live.current.info, live.current.tKey);
  };
  const cut = () => {
    const r = laneRange();
    if (r) return rangeEdits.cut(r);
    const ids = selectedGroups().flatMap((g) => g.points.map((p) => p.id));
    if (copy() > 0) void sendEdit(transport, removePointsCommand(ids));
  };
  const pasteAt = async (at: number, keepAt = false, given?: PointClipboard) => {
    // The clipboard's: a copy of several lanes puts each lane's points back into their own lane.
    const multi = given ? null : multiLaneClipboard();
    if (multi) return trackOwner ? pasteLanes(transport, multi, laneDest, at, selection, keepAt) : [];
    const clip = given ?? pointClipboard();
    // Clip envelopes: the edit cursor is on the song axis, not the lane's: it stays.
    const created = await pastePoints(transport, clip, pasteTarget(), at, selection, keepAt || live.current.cursorToLane !== undefined);
    // A pasted range becomes the lane's selection (so the next paste lands right after it).
    if (clip?.range && !live.current.cursorToLane) selectRange(at, at + clip.span);
    return created;
  };
  /** The edit cursor in lane time. */
  const cursorAt = () => {
    const song = editCursorTarget();
    return live.current.cursorToLane ? live.current.cursorToLane(song) : song;
  };
  const duplicate = () => {
    const r = laneRange();
    if (r) return rangeEdits.duplicate(r);
    const step = gridStep();
    const len = step ? stepLength(step, live.current.tempo.signatureAt(0)) : 1;
    const multi = multiSnapshot();
    if (multi) {
      const all = selectedGroups().flatMap((g) => g.points);
      void pasteLanes(transport, multi, laneDest, duplicateAt(all, len), selection, true);
      return;
    }
    const pts = selectedHere();
    const clip = snapshotPoints(pts, live.current.info, live.current.tKey);
    if (!clip) return;
    void pasteAt(duplicateAt(pts, len), true, { ...clip }).then(() => undefined);
  };
  /** Delete: the lane's range, else the selected points. */
  const deleteSelection = () => {
    const r = laneRange();
    if (r) return void rangeEdits.delete(r);
    const ids = selectedGroups().flatMap((g) => g.points.map((p) => p.id));
    if (ids.length) void sendEdit(transport, removePointsCommand(ids));
  };

  // Clipboard events (the desktop app's Edit menu sends these instead of the keys) while a
  // lane has focus: routed here by one shared window listener (see `clipboardLanes`),
  // before the arrangement's document-level clip handlers.
  const onClipboard = useRef<(type: string) => void>(() => {});
  useLayoutEffect(() => {
    onClipboard.current = (type) => {
      if (type === "copy") copy();
      else if (type === "cut") cut();
      else void pasteAt(cursorAt());
    };
  });
  useEffect(() => {
    const entry: ClipboardLane = { root: rootRef, handle: (type) => onClipboard.current(type) };
    return registerClipboardLane(entry);
  }, []);

  // ---- Pointer --------------------------------------------------------------------------

  /**
   * Run a window-level pointer drag until release. Unmounting the lane mid-drag (its owner
   * deleted by a collaborator or an undo) ends it like a `pointercancel`.
   */
  const trackDrag = (
    onMove: (e: PointerEvent) => void,
    onUp: (moved: boolean, e: Pick<PointerEvent, "type">) => void,
    start: { x: number; y: number },
    threshold = DRAG_THRESHOLD,
  ) => {
    let moved = false;
    let last: Pick<PointerEvent, "type"> = { type: "pointercancel" };
    const move = (e: PointerEvent) => {
      const p = local(e);
      if (!moved && Math.hypot(p.x - start.x, p.y - start.y) < threshold) return;
      moved = true;
      onMove(e);
    };
    const finish = dragEnd(() => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      onUp(moved, last);
    });
    const end = (e: PointerEvent) => {
      last = e;
      finish();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    dragScope(finish);
  };

  const onPointPointerDown = (e: ReactPointerEvent, point: AutomationPoint) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    rootRef.current?.focus();
    // The second press of a double-click deletes the point.
    if (doublePress(e)) {
      void sendEdit(transport, removePointsCommand([point.id]));
      return;
    }
    // Points and the lane's time range are separate selections: the keys act on one.
    if (laneRange()) useTimeSelection.getState().setSelection(null);
    const mode = selectModeFromEvent(e);
    const wasSelected = selection.getState().isSelected("automationPoint", point.id);
    if (mode === "toggle") {
      selection.getState().select("automationPoint", [point.id], "toggle");
      if (wasSelected) return;
    } else if (mode === "add" || !wasSelected) {
      selection.getState().select("automationPoint", [point.id], mode);
    }
    const groups = selectedGroups();
    if (!groups[0]?.points.some((p) => p.id === point.id)) return;
    const start = local(e);
    const gesture = new LaneGesture(transport);
    // Clips selected with the points (a box across both) move along, in time.
    const follow = owner.type === "Track" ? clipsFollowing() : null;
    const drag = { dt: 0, edits: [] as PointEdit[] };
    trackDrag(
      pointMover(groups, point, gesture, start, follow ? { follow, drag } : undefined),
      (moved) => {
        if (follow && moved) {
          const command = follow.command(drag.dt);
          if (command) gesture.update(batchStep(MOVE_SELECTION, flattenCommands([editPointsCommand(drag.edits), command])), follow.done);
          else follow.done();
        }
        gesture.end();
        setTip(null);
        // A plain click on a point of a multi-selection selects just that point.
        if (!moved && mode === "replace" && wasSelected) selection.getState().select("automationPoint", [point.id], "replace");
      },
      start,
    );
  };

  /**
   * Drag `groups` (the selected points of each lane when the drag started; this lane's
   * first, holding `point`, the grabbed one) from lane-local `from`. One implementation for
   * grabbed points and points just created by a double-click, so both follow the same rules
   * (grid, steps, clamping, axis lock, and a point landing on a neighbour's exact time keeps
   * its side instead of swapping values). The time moves every lane alike (the grabbed point
   * on the grid, the group kept at time >= 0); each lane turns the vertical motion into its
   * own values (`dragEdits`). Returns the pointermove handler; `gesture` carries the edits.
   */
  const pointMover = (
    groups: ReadonlyArray<{ lane: RegisteredLane; points: ReadonlyArray<AutomationPoint> }>,
    point: AutomationPoint,
    gesture: LaneGesture,
    from: { x: number; y: number },
    along?: { follow: ClipsFollowing; drag: { dt: number; edits: PointEdit[] } },
  ) => {
    const offset = precisionDrag(from);
    const minT = Math.min(...groups.flatMap((g) => g.points.map((p) => p.time)), along?.follow.minStart ?? Infinity);
    return (ev: PointerEvent) => {
      // ⌥: precision, 1/10 of the pointer's motion (and off the grid); ⇧'s increments win
      // for the value, so with ⇧ + ⌥ only the time is fine.
      const { dx, dy } = offset(local(ev), { x: ev.altKey, y: ev.altKey && !ev.shiftKey });
      // ⌘/Ctrl: lock to the dominant axis (⇧ is whole value increments, ⌥ precision).
      const lock = ev.metaKey || ev.ctrlKey;
      const lockTime = lock && Math.abs(dy) > Math.abs(dx);
      const lockValue = lock && !lockTime;
      const g = geomNow();
      let dt = lockTime ? 0 : dx / g.vp.pxPerBeat;
      const snap = snapFn(ev.altKey);
      if (!lockTime && snap) dt = snap(point.time + dt) - point.time;
      dt = Math.max(dt, -minT);
      const mods: DragMods = { increments: ev.shiftKey, lockTime, lockValue };
      const edits = groups.flatMap((gr, i) => gr.lane.dragEdits(gr.points, i === 0 && gr.points.includes(point) ? point : gr.points[0]!, dt, dy, mods));
      gesture.update(editPointsCommand(edits));
      if (along) {
        along.drag.dt = dt;
        along.drag.edits = edits;
        along.follow.preview(dt);
      }
      const mine = edits.find((x) => x.id === point.id);
      const t = mine?.time ?? point.time;
      const v = mine?.value ?? point.value;
      const inf = live.current.info;
      setTip({ x: timeToX(t, g), y: valueToY(v, g.height, g.range), value: formatNormalized(inf, v), hint: dragHint(inf) });
    };
  };

  /** WebGL: the point under the pointer (the canvas draws them; no element to hit). */
  const glPointAt = (e: { clientX: number; clientY: number }): AutomationPoint | null => {
    if (!gl) return null;
    const p = local(e);
    const g = geomNow();
    return automationPointAt({ points: live.current.points, offset: g.offset ?? 0, height: g.height, range: g.range ?? FULL_RANGE }, p.x, p.y, g.vp);
  };

  /** Snapped lane time at a lane-local x. */
  const timeAt = (x: number, bypass: boolean) => {
    const snap = snapFn(bypass);
    const raw = Math.max(0, xToTime(x, geomNow()));
    return snap ? snap(raw) : raw;
  };

  const tipAt = (time: number, value: number): Tip => {
    const g = geomNow();
    const inf = live.current.info;
    return { x: timeToX(time, g), y: valueToY(value, g.height, g.range), value: formatNormalized(inf, value), hint: dragHint(inf) };
  };

  /**
   * Second press of a double-click on empty space: the point is created right away (on the
   * press, not the release) at the snapped time with the lane's value there, so adding a
   * point never changes the lane (`splitPointAt`: same value, and a curve that continues the
   * segment it splits). The first point of an empty lane takes the target's current value
   * (the dashed baseline). Holding and moving past a small threshold then drags it exactly
   * like a grabbed point (`pointMover`), from where it was created; a release before that
   * leaves it untouched. Creation and drag are one undo step (one `LaneGesture`).
   */
  const createAndDrag = (e: ReactPointerEvent) => {
    e.preventDefault();
    const l = live.current;
    const press = local(e);
    const id = newId();
    // Switches (a device's on/off) hold their value; everything else ramps.
    const fallback: CurveShape = l.info.unit === "Toggle" ? { type: "Step" } : { type: "Linear" };
    const time = timeAt(press.x, e.altKey);
    // No points and no known current value (the target is gone): the pointer's value.
    const g0 = geomNow();
    const empty = { value: l.current ?? snapValue(l.info, yToValue(press.y, g0.height, g0.range), e.shiftKey), curve: fallback };
    const { value, curve } = splitPointAt(l.points, time, fallback) ?? empty;
    const created: AutomationPoint = { id, lane: l.lane?.id ?? "", time, value, curve };
    const gesture = new LaneGesture(transport);
    gesture.update(addPointCommand(l.lane, l.owner, l.target, { id, time, value, curve }, newId()), () =>
      selection.getState().select("automationPoint", [id], "replace"),
    );
    setTip(tipAt(time, value));
    // Like a grabbed point: moved by the pointer's motion since the press (never to the
    // pointer's y), once past the threshold.
    trackDrag(
      pointMover([{ lane: selfLane, points: [created] }], created, gesture, press),
      () => {
        gesture.end();
        setTip(null);
      },
      press,
      e.pointerType === "touch" ? CREATE_TOUCH_THRESHOLD_PX : CREATE_THRESHOLD_PX,
    );
  };

  /**
   * A ⇧-press on empty space of a track lane: the track (and this lane: it takes the focus)
   * is selected and the other selections cleared; a drag selects a time range in this lane;
   * ⇧ extends the lane's range (or the track's insert marker) to the pointer, or retracts it
   * when pressed inside; with nothing to extend, a click places the insert marker there
   * (snapped; ⌥: free) and, when stopped, the playhead.
   */
  const rangeOrCursor = (e: ReactPointerEvent) => {
    const l = live.current;
    if (l.owner.type !== "Track") return;
    const track = l.owner.track;
    const prev = useTimeSelection.getState().selection;
    const start = local(e);
    const at = timeAt(start.x, e.altKey);
    const extendable = e.shiftKey && prev && prev.tracks[0] === track && (prev.automationLane === l.tKey || isMarker(prev)) ? prev : null;
    // ⇧-click: outside the range extends it to the press, inside retracts the nearer edge to
    // it (`extendOrRetract`). A ⇧-drag selects a new range from the press, like the arranger.
    const anchor = extendable ? (movedEdge(extendable, at) === "start" ? extendable.end : extendable.start) : at;
    selectTrackEntity(track, "replace");
    const show = (from: number, to: number) => {
      if (Math.abs(to - from) > MARKER_EPS) selectRange(Math.min(from, to), Math.max(from, to));
    };
    if (extendable) show(anchor, at);
    trackDrag(
      (ev) => show(at, timeAt(local(ev).x, ev.altKey)),
      (moved, ev) => {
        if (ev.type === "pointercancel") return;
        const r = laneRange();
        if (r && (moved || extendable)) {
          // The selection cursor is the range start: while stopped the playhead goes there.
          setPlayStart(transport, r.start);
          return;
        }
        const kind = useProjectStore.getState().project?.tracks[track]?.kind;
        if (kind && isTimeTrack({ kind })) placeInsertMarker(transport, at, [track]);
        else setPlayStart(transport, at);
      },
      start,
    );
  };

  /**
   * The time section under lane-local `p` when `p` is on the lane's line: the lane's own
   * section, or a section of the track's row (it covers the automation lanes too). Null on an
   * empty lane (no line to grab: its dashed baseline is not in the document).
   */
  const sectionUnder = (p: { x: number; y: number }): { start: number; end: number } | null => {
    const l = live.current;
    if (l.owner.type !== "Track" || l.points.length === 0) return null;
    const sel = rangeOf(useTimeSelection.getState().selection);
    const track = l.owner.track;
    const r = laneRange() ?? (sel && sel.automationLane === undefined && sel.tracks.includes(track) ? sel : null);
    if (!r) return null;
    const g = geomNow();
    const t = xToTime(p.x, g);
    if (t < r.start || t > r.end) return null;
    const v = evaluatePoints(l.points, t)!;
    return Math.abs(valueToY(v, g.height, g.range) - p.y) <= LINE_HIT_PX ? r : null;
  };

  /**
   * ⇧-drag on the line inside a time section: the section moves up/down, the lane outside it
   * doesn't change (`sectionMovePlan`: jumps of two points at its edges, added only once the
   * pointer moved past the threshold, so a ⇧-click changes nothing). The inside moves like a
   * vertical multi-point drag (`moveEdits`: stepped params by whole steps, the group clamped
   * to 0..1; ⌥: precision). One undo step.
   */
  const moveSection = (e: ReactPointerEvent, r: { start: number; end: number }) => {
    const l = live.current;
    const lane = l.lane;
    if (!lane) return;
    const pts = l.points;
    const press = local(e);
    const gesture = new LaneGesture(transport);
    const usable = Math.max(1, geomNow().height - 2 * LANE_PAD);
    const offset = precisionDrag(press);
    // The line's value under the press (the tooltip follows it).
    const grabbed = evaluatePoints(pts, xToTime(press.x, geomNow()))!;
    let plan: ReturnType<typeof sectionMovePlan> | undefined;
    trackDrag(
      (ev) => {
        if (plan === undefined) {
          plan = sectionMovePlan(pts, r.start, r.end, lane.id);
          if (plan?.add.length) gesture.update(cmd("Automation", { type: "AddPoints", lane: lane.id, points: plan.add }));
        }
        if (!plan || plan.moving.length === 0) return;
        const { dy } = offset(local(ev), { x: false, y: ev.altKey });
        const g = geomNow();
        const span = (g.range ?? FULL_RANGE).hi - (g.range ?? FULL_RANGE).lo;
        const inf = live.current.info;
        const dv = stepDragDelta(inf, -dy) ?? (-dy / usable) * span;
        const anchor = plan.moving[0]!;
        const edits = moveEdits(plan.moving, anchor, 0, dv, null, { lockTime: true, snapValue: (v) => snapValue(inf, v, false) });
        gesture.update(editPointsCommand(edits));
        const v = grabbed + (edits[0]?.value ?? anchor.value) - anchor.value;
        setTip({ x: press.x, y: valueToY(v, g.height, g.range), value: formatNormalized(inf, v), hint: "" });
      },
      () => {
        gesture.end();
        setTip(null);
      },
      press,
      CREATE_THRESHOLD_PX,
    );
  };

  /**
   * The points whose line passes under `p` (lane-local px, within `LINE_HIT_PX`): the two
   * ends of a segment, or the first / last point for the hold before / after it. Null off
   * the line.
   */
  const segmentUnder = (p: { x: number; y: number }): AutomationPoint[] | null => {
    const pts = live.current.points;
    if (pts.length === 0) return null;
    const g = geomNow();
    const t = xToTime(p.x, g);
    const v = evaluatePoints(pts, t)!;
    if (Math.abs(valueToY(v, g.height, g.range) - p.y) > LINE_HIT_PX) return null;
    const i = segmentAt(pts, t);
    if (i >= 0) return [pts[i]!, pts[i + 1]!];
    return [t < pts[0]!.time ? pts[0]! : pts[pts.length - 1]!];
  };

  /**
   * A plain drag on the line: the segment under the press moves up/down (its two points'
   * values, times kept), like Ableton's and Bitwig's. Same value rules as a point drag
   * (stepped params by steps, ⇧: whole increments, ⌥: precision), one undo step; the two
   * points become the selection. A click without moving is a click on the lane.
   */
  const moveSegment = (e: ReactPointerEvent, seg: AutomationPoint[], onClick: () => void) => {
    const press = local(e);
    const gesture = new LaneGesture(transport);
    const offset = precisionDrag(press);
    const grabbed = evaluatePoints(live.current.points, xToTime(press.x, geomNow()))!;
    let started = false;
    trackDrag(
      (ev) => {
        if (!started) {
          started = true;
          selection.getState().select(
            "automationPoint",
            seg.map((p) => p.id),
            "replace",
          );
        }
        const { dy } = offset(local(ev), { x: false, y: ev.altKey && !ev.shiftKey });
        const edits = selfLane.dragEdits(seg, seg[0]!, 0, dy, { increments: ev.shiftKey, lockTime: true, lockValue: false });
        gesture.update(editPointsCommand(edits));
        const g = geomNow();
        const inf = live.current.info;
        const v = grabbed + (edits[0]?.value ?? seg[0]!.value) - seg[0]!.value;
        setTip({ x: press.x, y: valueToY(v, g.height, g.range), value: formatNormalized(inf, v), hint: dragHint(inf) });
      },
      (moved) => {
        gesture.end();
        setTip(null);
        if (!moved) onClick();
      },
      press,
    );
  };

  /**
   * Draw mode: the drag writes the automation it passes over, like Ableton's draw mode. With
   * a grid, one step per grid cell at the pointer's value there (cells skipped by a fast
   * drag are filled in between); with ⌥ or no grid, freehand (a linear point every few px).
   * Everything between the first and the last cell drawn is replaced, the lane outside kept
   * (`writeRange`). A click draws one cell. One undo step, live while drawing.
   */
  const draw = (e: ReactPointerEvent) => {
    const l = live.current;
    const original = l.points;
    const g = geomNow();
    const step = e.altKey ? null : gridStep();
    const off = g.offset ?? 0;
    const free = step === null;
    const cell = free ? FREE_DRAW_PX / g.vp.pxPerBeat : stepLength(step, l.tempo.signatureAt(0));
    const curve: CurveShape = free ? { type: "Linear" } : { type: "Step" };
    // Cell index (on the view's grid) → value drawn there.
    const cells = new Map<number, number>();
    const cellOf = (t: number) => Math.floor((t + off) / cell + 1e-9);
    const startOf = (i: number) => i * cell - off;
    const valueAtY = (yy: number) => {
      const gm = geomNow();
      return snapValue(live.current.info, Math.min(1, Math.max(0, yToValue(yy, gm.height, gm.range))), false);
    };
    let prev: { t: number; v: number } | null = null;
    const sample = (p: { x: number; y: number }) => {
      const t = Math.max(0, xToTime(p.x, geomNow()));
      const v = valueAtY(p.y);
      const from = prev ?? { t, v };
      const [i0, i1] = [cellOf(from.t), cellOf(t)];
      const n = Math.abs(i1 - i0);
      for (let k = 0; k <= n; k++) {
        const i = i0 + Math.sign(i1 - i0) * k;
        cells.set(i, n === 0 ? v : from.v + ((v - from.v) * k) / n);
      }
      prev = { t, v };
    };
    const laneId = l.lane?.id ?? newId();
    let created = l.lane !== null;
    const removed = new Set<AutomationPointId>();
    let added: AutomationPointId[] = [];
    const build = (): Command | null => {
      const idx = [...cells.keys()].sort((a, b) => a - b);
      if (idx.length === 0) return null;
      const lo = idx[0]!;
      const hi = idx[idx.length - 1]!;
      const at = Math.max(0, startOf(lo));
      // Steps fill their cell; freehand ends on its last point.
      const end = free ? startOf(hi) : startOf(hi + 1);
      const piece = idx.filter((i) => startOf(i) >= at - 1e-9).map((i) => ({ time: startOf(i) - at, value: cells.get(i)!, curve }));
      const w = writeRange(original, at, Math.max(0, end - at), piece);
      const commands: Command[] = [];
      if (!created) {
        commands.push(cmd("Automation", { type: "CreateLane", id: laneId, owner: live.current.owner, target: live.current.target }));
        created = true;
      }
      // The originals in the range (once) and what the last send added.
      const remove = [...w.remove.filter((id) => !removed.has(id)), ...added];
      for (const id of w.remove) removed.add(id);
      if (remove.length) commands.push(cmd("Automation", { type: "RemovePoints", ids: remove }));
      commands.push(cmd("Automation", { type: "AddPoints", lane: laneId, points: w.add }));
      added = w.add.map((p) => p.id);
      return batchStep(keyedStep("undo.automation.draw", "draw automation"), commands);
    };
    selection.getState().clear("automationPoint");
    const press = local(e);
    const gesture = new LaneGesture(transport);
    const showTip = (p: { x: number; y: number }) => {
      const gm = geomNow();
      const v = valueAtY(p.y);
      setTip({ x: p.x, y: valueToY(v, gm.height, gm.range), value: formatNormalized(live.current.info, v), hint: "" });
    };
    sample(press);
    gesture.updateWith(build);
    showTip(press);
    trackDrag(
      (ev) => {
        const p = local(ev);
        sample(p);
        gesture.updateWith(build);
        showTip(p);
      },
      () => {
        gesture.end();
        setTip(null);
      },
      press,
      0,
    );
  };

  const onBackgroundPointerDown = (e: ReactPointerEvent<SVGSVGElement>) => {
    const hit = e.button === 0 ? glPointAt(e) : null;
    if (hit) {
      onPointPointerDown(e, hit);
      return;
    }
    // The lane owns its pointer events (the arrangement has its own clip marquee).
    e.stopPropagation();
    rootRef.current?.focus();
    if (e.button !== 0) return;
    // Draw mode: the drag draws (points stay grabbable).
    if (useAutomationUi.getState().drawMode && !(e.metaKey || e.ctrlKey || e.shiftKey)) {
      e.preventDefault();
      draw(e);
      return;
    }
    if (doublePress(e)) {
      createAndDrag(e);
      return;
    }
    if (e.altKey) {
      const start = local(e);
      const pts = live.current.points;
      const i = segmentAt(pts, xToTime(start.x, geomNow()));
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
    // ⇧-press on the line inside a time section: move that section up/down.
    if (e.shiftKey && !(e.metaKey || e.ctrlKey)) {
      const r = sectionUnder(local(e));
      if (r) {
        e.preventDefault();
        moveSection(e, r);
        return;
      }
    }
    // Like the arranger: a drag selects points in a box (⌘/Ctrl: toggles them), ⇧+drag
    // selects a time range of the lane (track lanes; ⇧+click extends it); a click places the
    // cursor (track lanes). Clip envelopes: a drag selects points.
    const owner = live.current.owner;
    if (owner.type === "Track" && e.shiftKey) {
      e.preventDefault();
      rangeOrCursor(e);
      return;
    }
    if (owner.type === "Track" && !(e.metaKey || e.ctrlKey)) selectTrackEntity(owner.track, "replace");
    // A plain press on the line: drag that segment up/down (a click stays a click).
    const seg = e.altKey || e.metaKey || e.ctrlKey ? null : segmentUnder(local(e));
    if (seg) {
      e.preventDefault();
      const at = local(e);
      moveSegment(e, seg, () => {
        selection.getState().clear("automationPoint");
        placeCursor(at.x, false);
      });
      return;
    }
    // Track lanes: the box goes on across the other lanes and tracks (`laneMarquee.ts`).
    if (owner.type === "Track" && rootRef.current) {
      e.preventDefault();
      startLaneMarquee(e, { view, lane: rootRef.current, store: selection, onClick: (p, ev) => placeCursor(p.x, ev.altKey) });
      return;
    }
    marquee.onPointerDown(e);
  };

  /** A click on empty space of a track lane: the insert marker there (time tracks), else the playhead. */
  const placeCursor = (x: number, alt: boolean) => {
    const l = live.current;
    if (l.owner.type !== "Track") return;
    const at = timeAt(x, alt);
    const kind = useProjectStore.getState().project?.tracks[l.owner.track]?.kind;
    if (kind && isTimeTrack({ kind })) placeInsertMarker(transport, at, [l.owner.track]);
    else setPlayStart(transport, at);
  };

  // ---- Keyboard -------------------------------------------------------------------------

  /** Nudge the selected points of every lane holding some (one undo step). */
  const nudge = (dtSteps: number, dvSteps: number, fine: boolean, precise = false) => {
    const groups = selectedGroups();
    if (groups.length === 0) return;
    const step = gridStep();
    const beat = step ? stepLength(step, live.current.tempo.signatureAt(0)) : 1;
    let dt = dtSteps * (fine ? beat / 4 : beat);
    // The group keeps its shape: no point goes before time 0.
    dt = Math.max(dt, -Math.min(...groups.flatMap((g) => g.points.map((p) => p.time))));
    const edits = groups.flatMap((g) => g.lane.nudgeEdits(g.points, dt, dvSteps, fine, precise));
    void sendEdit(transport, editPointsCommand(edits));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // keymap: chords from the user's keymap (automation scope; see the registry).
    const action = firstMatch(AUTOMATION_ACTIONS, e);
    const handled = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    // Clips selected with the points (a box across clip and automation lanes), or a copy of
    // both: the arrangement's edit acts on both (`mixedSelection.ts`).
    const mixed = MIXED_ACTIONS[action ?? ""];
    if (mixed && owner.type === "Track" && (isMixedSelection() || (mixed === "paste" && hasMixedClipboard()))) {
      handled();
      void runClipAction(transport, mixed);
      return;
    }
    if (action === "automation.drawMode") {
      handled();
      const ui = useAutomationUi.getState();
      ui.setDrawMode(!ui.drawMode);
      return;
    }
    if (action === "edit.delete" || action === "edit.deleteModified") {
      // Always keep Delete in the focused lane, even with nothing to delete: bubbling up,
      // it would delete the arrangement's selected clips.
      handled();
      deleteSelection();
    } else if (action === "edit.selectAll") {
      handled();
      if (laneRange()) useTimeSelection.getState().setSelection(null);
      selection.getState().select(
        "automationPoint",
        live.current.points.map((p) => p.id),
        "replace",
      );
    } else if (action === "edit.copy" || action === "edit.cut" || action === "edit.paste" || action === "edit.duplicate") {
      handled();
      if (action === "edit.copy") copy();
      else if (action === "edit.cut") cut();
      else if (action === "edit.paste") void pasteAt(cursorAt());
      else duplicate();
    } else if (action && AUTOMATION_NUDGES[action]) {
      if (selectedGroups().length === 0) return;
      handled();
      const [dt, dv, fine, precise] = AUTOMATION_NUDGES[action];
      nudge(dt, dv, fine, precise);
    }
  };

  // ---- Menus ----------------------------------------------------------------------------

  const setCurve = (ids: AutomationPointId[], curve: CurveShape) => void sendEdit(transport, setCurveCommand(ids, curve));

  /** "Set Value…": the popover at the point (kept inside the lane's width). */
  const openValueEdit = (ids: AutomationPointId[], p: AutomationPoint) => {
    const g = geomNow();
    const w = rootRef.current?.clientWidth || width;
    const x = Math.min(Math.max(0, timeToX(p.time, g)), w);
    setEditing({ ids, plain: paramToPlain(info, p.value), x, y: valueToY(p.value, g.height, g.range) });
  };

  const pointMenu = (e: MouseEvent, p: AutomationPoint) => {
    if (!selection.getState().isSelected("automationPoint", p.id)) selection.getState().select("automationPoint", [p.id], "replace");
    const ids = selectedGroups().flatMap((g) => g.points.map((x) => x.id));
    const items: ContextMenuEntry[] = [
      { label: t("arrangement.menu.cut"), shortcut: shortcutLabel("edit.cut"), onSelect: cut },
      { label: t("arrangement.menu.copy"), shortcut: shortcutLabel("edit.copy"), onSelect: () => void copy() },
      { label: t("arrangement.menu.paste"), shortcut: shortcutLabel("edit.paste"), disabled: !hasPointClipboard(), onSelect: () => void pasteAt(cursorAt()) },
      { label: t("arrangement.menu.duplicate"), shortcut: shortcutLabel("edit.duplicate"), onSelect: duplicate },
      "separator",
      { label: t("automation.menu.set_value"), onSelect: () => openValueEdit(ids, p) },
      { label: t("automation.menu.linear"), onSelect: () => setCurve(ids, { type: "Linear" }) },
      { label: t("automation.menu.step"), onSelect: () => setCurve(ids, { type: "Step" }) },
      { label: t("automation.menu.smooth"), onSelect: () => setCurve(ids, { type: "Curve", tension: 0.5 }) },
      "separator",
      {
        label: t("automation.menu.delete_points", { count: ids.length }),
        shortcut: shortcutLabel("edit.delete"),
        danger: true,
        onSelect: () => void sendEdit(transport, removePointsCommand(ids)),
      },
    ];
    openContextMenu(e, items);
  };

  const laneMenu = (e: MouseEvent<SVGSVGElement>) => {
    const hit = glPointAt(e);
    if (hit) {
      pointMenu(e, hit);
      return;
    }
    const at = timeAt(local(e).x, e.altKey);
    const sel = laneRange();
    const raw = xToTime(local(e).x, geomNow());
    if (sel && raw >= sel.start && raw <= sel.end) {
      openContextMenu(e, [
        {
          label: t("arrangement.menu.cut"),
          shortcut: shortcutLabel("edit.cut"),
          disabled: live.current.points.length === 0,
          onSelect: () => rangeEdits.cut(sel),
        },
        {
          label: t("arrangement.menu.copy"),
          shortcut: shortcutLabel("edit.copy"),
          disabled: live.current.points.length === 0,
          onSelect: () => void rangeEdits.copy(sel),
        },
        { label: t("arrangement.menu.paste"), shortcut: shortcutLabel("edit.paste"), disabled: !hasPointClipboard(), onSelect: () => void pasteAt(cursorAt()) },
        {
          label: t("arrangement.menu.duplicate"),
          shortcut: shortcutLabel("edit.duplicate"),
          disabled: live.current.points.length === 0,
          onSelect: () => rangeEdits.duplicate(sel),
        },
        "separator",
        {
          label: t("arrangement.menu.delete"),
          shortcut: shortcutLabel("edit.delete"),
          danger: true,
          disabled: live.current.points.length === 0,
          onSelect: () => void rangeEdits.delete(sel),
        },
      ]);
      return;
    }
    const ids = selectedGroups().flatMap((g) => g.points.map((x) => x.id));
    openContextMenu(e, [
      { label: t("automation.menu.paste_here"), disabled: !hasPointClipboard(), onSelect: () => void pasteAt(at) },
      {
        label: t("automation.menu.paste_at_cursor"),
        shortcut: shortcutLabel("edit.paste"),
        disabled: !hasPointClipboard(),
        onSelect: () => void pasteAt(cursorAt()),
      },
      "separator",
      {
        label: t("automation.menu.select_all"),
        shortcut: shortcutLabel("edit.selectAll"),
        disabled: live.current.points.length === 0,
        onSelect: () =>
          selection.getState().select(
            "automationPoint",
            live.current.points.map((p) => p.id),
            "replace",
          ),
      },
      { label: t("arrangement.menu.copy"), shortcut: shortcutLabel("edit.copy"), disabled: ids.length === 0, onSelect: () => void copy() },
      {
        label: ids.length > 1 ? t("automation.menu.delete_points", { count: ids.length }) : t("arrangement.menu.delete"),
        shortcut: shortcutLabel("edit.delete"),
        danger: true,
        disabled: ids.length === 0,
        onSelect: () => void sendEdit(transport, removePointsCommand(ids)),
      },
    ]);
  };

  /** Sets the points to a typed plain value (already clamped): one undo step. */
  const applyValue = (ids: AutomationPointId[], plain: number) => {
    const v = snapValue(info, paramToNormalized(info, plain));
    void sendEdit(transport, editPointsCommand(ids.map((id) => ({ id, time: null, value: v, curve: null }))));
  };
  const closeValueEdit = () => {
    setEditing(null);
    rootRef.current?.focus();
  };

  const disabled = lane !== null && !lane.enabled;
  const r = marquee.rect;

  // WebGL: publish what the canvas draws for this lane (track lanes only).
  const glKey = useRef({});
  const glTrack = gl && owner.type === "Track" ? owner.track : null;
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!glTrack || !el) return;
    publishAutomationLane(glKey.current, { track: glTrack, el, points, selected, height, range, lines, disabled, offset, baseline });
  }, [glTrack, points, selected, height, range, lines, disabled, offset, baseline]);
  useLayoutEffect(() => {
    const key = glKey.current;
    return () => publishAutomationLane(key, null);
  }, [glTrack]);

  /** WebGL: the point cursor and value tooltip the circles had. */
  const [hoverTip, setHoverTip] = useState<string | null>(null);
  const onGlHover = (e: ReactPointerEvent<SVGSVGElement>) => {
    if (e.buttons !== 0) return;
    const p = glPointAt(e);
    e.currentTarget.style.cursor = p ? "grab" : lineCursor(e);
    const tipText = p ? formatNormalized(info, p.value) : null;
    if (tipText !== hoverTip) setHoverTip(tipText);
  };
  /** Over the line (not a point): the segment drag's cursor. */
  const lineCursor = (e: ReactPointerEvent<SVGSVGElement>) =>
    useAutomationUi.getState().drawMode ? "crosshair" : !e.altKey && !e.shiftKey && segmentUnder(local(e)) ? "ns-resize" : "";
  const onSvgHover = (e: ReactPointerEvent<SVGSVGElement>) => {
    if (e.buttons !== 0) return;
    // The circles set their own cursor.
    const over = e.target instanceof Element && e.target.closest("[data-point]");
    e.currentTarget.style.cursor = over ? "" : lineCursor(e);
  };

  return (
    <div
      ref={rootRef}
      className={clsx("eth-auto-lane", disabled && "eth-auto-lane--disabled", lane === null && "eth-auto-lane--empty", gl && "eth-auto-lane--gl")}
      style={{ height }}
      tabIndex={0}
      role="group"
      aria-label={label ?? t("automation.lane.label", { name: info.name })}
      data-lane={lane?.id}
      onKeyDown={onKeyDown}
      data-keymap-scope="automation"
    >
      {owner.type === "Track" && <LaneRangeOverlay view={view} track={owner.track} targetKey={tKey} height={height} />}
      <svg
        className="eth-auto-lane__svg"
        width="100%"
        height={height}
        onPointerDown={onBackgroundPointerDown}
        // Double-clicks are detected on the second press (`doublePress`), not here.
        onDoubleClick={(e) => e.stopPropagation()}
        onContextMenu={laneMenu}
        onPointerMove={gl ? onGlHover : onSvgHover}
        onPointerLeave={gl ? () => setHoverTip(null) : undefined}
        data-testid="automation-lane-svg"
        data-steps={lines.length}
        data-renderer={gl ? "webgl" : undefined}
        // The empty lane's baseline (normalized value), for tests (WebGL draws it on the canvas).
        data-baseline={baseline ?? undefined}
      >
        {gl && hoverTip && <title>{hoverTip}</title>}
        {!gl &&
          lines.map((l) => {
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
        {!gl && baseline !== null && (
          <line
            className="eth-auto-lane__baseline"
            x1={0}
            x2="100%"
            y1={valueToY(baseline, height, range)}
            y2={valueToY(baseline, height, range)}
            data-testid="automation-baseline"
          />
        )}
        {path && <path className="eth-auto-lane__line" d={path} />}
        {!gl &&
          points.map((p) => {
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
      <PointValuePopover edit={editing} info={info} onApply={applyValue} onDone={closeValueEdit} />
    </div>
  );
}

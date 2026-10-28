/**
 * Reordering tracks by dragging their headers (pointer drag, not HTML5 drag-and-drop).
 *
 * Releasing between two rows moves the track before the row below (into that row's
 * group); over the middle of a group's header it moves the track into the group (at its
 * end). Where a gap could be inside or outside a group (after a group's last child, or the
 * last of nested groups), the pointer's x picks the level (`trackDropTarget`). Groups move with their children, and a group can't go inside itself. Master stays
 * where it is. Return tracks reorder among the returns only (the engine keeps them between
 * the regular tracks and master): over the regular tracks a dragged return shows the zone
 * it can't go in. The view scrolls near its top / bottom edge, and Escape cancels.
 */

import type { PointerEvent as ReactPointerEvent } from "react";
import type { Track, TrackId, TrackKind } from "@/generated";
import { setDragCursor } from "@/kit";
import { size } from "@/theme";
import { setEditActivity as setActivity } from "@/ext/surfaces";
import { cmd } from "@/transport";
import { sendEdit, type ArrangementContextValue } from "./context";
import { holdClipDrag } from "./dragGuard";
import type { Row } from "./layout";
import { useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 4;
/** Within this distance (px) of the view's top / bottom edge, a track drag scrolls it. */
const EDGE_SCROLL_PX = 32;
/** Scroll speed at the very edge (px per frame). */
const EDGE_SCROLL_MAX_PX = 16;

export interface TrackDropTarget {
  parent: TrackId | null;
  before: TrackId | null;
  /** Indicator: a line at `y` (content px), or the group row the track goes into. */
  y: number;
  into: TrackId | null;
  /** The nesting depth it lands at (the line starts at that depth's indent). */
  depth: number;
}

const movable = (t: Track) => t.kind !== "Master";

function isInside(rows: ReadonlyArray<Row>, track: TrackId, ancestor: TrackId): boolean {
  const byId = new Map(rows.map((r) => [r.track.id, r.track]));
  let p = byId.get(track)?.parent ?? null;
  for (let guard = 0; p !== null && guard < 64; guard++) {
    if (p === ancestor) return true;
    p = byId.get(p)?.parent ?? null;
  }
  return false;
}

/** Top (content px) of the returns section: the first return, else master, else the end. */
function returnsTop(rows: ReadonlyArray<Row>): { y: number; snap: number } {
  const first = rows.find((r) => r.track.kind === "Return" || r.track.kind === "Master");
  if (first) return { y: first.y, snap: first.laneHeight / 4 };
  const last = rows[rows.length - 1];
  return { y: last ? last.y + last.height : 0, snap: 0 };
}

/** The zone (content px) a dragged return can't be dropped in: the regular tracks above the returns. */
export function returnBlockedZone(rows: ReadonlyArray<Row>): { top: number; height: number } {
  return { top: 0, height: returnsTop(rows).y };
}

/** Where a return dropped at content y lands among the returns, or null over the regular tracks. */
function returnDropTarget(rows: ReadonlyArray<Row>, y: number, dragged: TrackId): TrackDropTarget | null {
  const top = returnsTop(rows);
  if (y < top.y - top.snap) return null;
  const returns = rows.filter((r) => r.track.kind === "Return");
  let i = returns.findIndex((r) => y < r.y + r.laneHeight / 2);
  if (i < 0) i = returns.length;
  // Right above or below itself: no move.
  if (returns[i]?.track.id === dragged) i++;
  const below = returns[i];
  if (below) return { parent: null, before: below.track.id, y: below.y, into: null, depth: 0 };
  // After the last return (the engine keeps it above master).
  const last = returns[returns.length - 1];
  return { parent: null, before: null, y: last ? last.y + last.height : top.y, into: null, depth: 0 };
}

/** Header card left inset (px) and the width (px) of one nesting level when picking it with the pointer's x. */
const CARD_INSET_PX = parseFloat(size.trackCardInsetLeft);
const INDENT_PX = parseFloat(size.trackIndent);
const DEPTH_STEP_PX = 2 * INDENT_PX;

/** The nesting depth the pointer's x (content px from the header column's left) points at. */
export function depthAtX(x: number): number {
  return Math.max(0, Math.floor((x - CARD_INSET_PX) / DEPTH_STEP_PX));
}

/** Left (content px) of the drop line at `depth`: where a header card at that depth starts. */
export function dropLineLeft(depth: number): number {
  return CARD_INSET_PX + depth * INDENT_PX;
}

/**
 * Where dropping `dragged` at content y lands, or null where it can't go. `kind`: the dragged
 * track's kind when it isn't one of the rows (a track being added from the Add track menu).
 *
 * In a gap where several nesting levels fit (after the last child of a group, possibly the
 * last of several nested groups; right under a group with no visible children), the
 * pointer's `x` (content px from the header column's left) picks one, like a layers panel:
 * further left leaves groups, further right stays inside (one level per `DEPTH_STEP_PX`).
 * Without `x`, the shallowest (the row below's level).
 */
export function trackDropTarget(rows: ReadonlyArray<Row>, y: number, dragged: TrackId, kind?: TrackKind, x?: number): TrackDropTarget | null {
  const draggedKind = kind ?? rows.find((r) => r.track.id === dragged)?.track.kind;
  if (draggedKind === "Return") return returnDropTarget(rows, y, dragged);
  const self = (t: Track) => t.id === dragged || isInside(rows, t.id, dragged);
  // VCA tracks stay top-level (never into a group).
  const topOnly = draggedKind === "Vca";
  // Into a group: over the middle half of its lane.
  const over = rows.find((r) => y >= r.y && y < r.y + r.laneHeight);
  if (over && over.track.kind === "Group" && !self(over.track) && !topOnly) {
    const q = (y - over.y) / over.laneHeight;
    if (q > 0.25 && q < 0.75) return { parent: over.track.id, before: null, y: over.y, into: over.track.id, depth: over.depth + 1 };
  }
  // Otherwise the gap nearest to the pointer: before the first row whose middle is below it.
  let i = rows.findIndex((r) => y < r.y + r.laneHeight / 2);
  if (i < 0) i = rows.length;
  if (topOnly) while (i < rows.length && rows[i]!.track.parent !== null) i++;
  // Skip the dragged track's children: dropping inside them means "no move".
  while (i < rows.length && self(rows[i]!.track) && rows[i]!.track.id !== dragged) i++;
  // Right above itself: the gap it sits in (only another level there is a move).
  const at = rows[i]?.track.id === dragged ? rows[i] : undefined;
  while (at && i < rows.length && self(rows[i]!.track)) i++;
  const next = rows[i];
  const regular = (r: Row | undefined) => (r && r.track.kind !== "Return" && r.track.kind !== "Master" ? r : undefined);
  // Returns and master show last: a gap above them is the end of the regular tracks.
  const below = regular(next);
  // The row above the gap, the dragged track (and its children) left out.
  let a = i - 1;
  while (a >= 0 && self(rows[a]!.track)) a--;
  const above = regular(rows[a]);
  // The levels that fit: from the row below's (a sibling of it) to the row above's, or one
  // deeper when the row above is a group (into it, at its end).
  const minDepth = below?.depth ?? 0;
  const maxDepth = topOnly || !above ? minDepth : Math.max(minDepth, above.depth + (above.track.kind === "Group" ? 1 : 0));
  const depth = x === undefined ? minDepth : Math.min(maxDepth, Math.max(minDepth, depthAtX(x)));
  if (at && (minDepth === maxDepth || depth === at.depth)) return null;
  if (below && depth === below.depth) return { parent: below.track.parent, before: below.track.id, y: below.y, into: null, depth };
  // After the last regular track: end of the top level (the engine keeps it above returns).
  const last = [...rows].reverse().find((r) => regular(r));
  const lineY = below ? below.y : last ? last.y + last.height : 0;
  // Deeper than the row below: at the end of the group at `depth - 1` holding the row above
  // (the row above itself when it's that group).
  const byId = new Map(rows.map((r) => [r.track.id, r]));
  let group: Row | undefined = depth > 0 ? above : undefined;
  while (group && group.depth > depth - 1) group = group.track.parent !== null ? byId.get(group.track.parent) : undefined;
  if (!group || group.track.kind !== "Group") return { parent: null, before: null, y: lineY, into: null, depth: 0 };
  return { parent: group.track.id, before: null, y: lineY, into: null, depth };
}

/** Scroll speed (px per frame, negative = up) for a pointer at client y in a view spanning [top, bottom]. */
export function edgeScrollSpeed(y: number, top: number, bottom: number): number {
  if (bottom - top <= 2 * EDGE_SCROLL_PX) return 0;
  if (y < top + EDGE_SCROLL_PX) return -Math.ceil((EDGE_SCROLL_MAX_PX * Math.min(EDGE_SCROLL_PX, top + EDGE_SCROLL_PX - y)) / EDGE_SCROLL_PX);
  if (y > bottom - EDGE_SCROLL_PX) return Math.ceil((EDGE_SCROLL_MAX_PX * Math.min(EDGE_SCROLL_PX, y - (bottom - EDGE_SCROLL_PX))) / EDGE_SCROLL_PX);
  return 0;
}

export interface TrackDragOptions {
  ctx: ArrangementContextValue;
  /** The dragged track's id (or a placeholder for a track being added). */
  id: TrackId;
  kind: TrackKind;
  startX: number;
  startY: number;
  /** Only vertical movement starts the drag (a header), else any direction (a menu item). */
  vertical: boolean;
  /** Pointer outside the arrangement's tracks: no target (a track being added). */
  insideOnly?: boolean;
  /** The drag started (past the threshold). */
  onStart?(): void;
  onDrop(target: TrackDropTarget): void;
  /** The drag ended without a drop (Escape, pointercancel, no target). */
  onEnd?(dragged: boolean): void;
}

/**
 * A track drag from pointerdown to drop: the drop indicator in `useArrangementUi.trackDrag`,
 * the not-allowed zone for a return over the regular tracks, edge auto-scroll, and Escape
 * (or pointercancel) to cancel without an edit.
 */
export function startTrackDrag(o: TrackDragOptions): void {
  const ui = useArrangementUi.getState;
  const scroller = () => o.ctx.contentRef.current?.parentElement ?? null;
  let target: TrackDropTarget | null = null;
  let active = false;
  let last = { x: o.startX, y: o.startY };
  let frame = 0;

  const update = () => {
    const box = o.ctx.contentRef.current?.getBoundingClientRect();
    const inside = !!box && last.x >= box.left && last.x <= box.right && last.y >= box.top && last.y <= box.bottom;
    if (o.insideOnly && !inside) {
      target = null;
      setDragCursor("grabbing");
      ui().setTrackDrag(null);
      return;
    }
    const rows = o.ctx.rowsRef.current;
    target = trackDropTarget(rows, last.y - (box?.top ?? 0), o.id, o.kind, last.x - (box?.left ?? 0));
    const blocked = !target && o.kind === "Return" ? returnBlockedZone(rows) : undefined;
    setDragCursor(blocked ? "not-allowed" : "grabbing");
    ui().setTrackDrag(
      target ? { track: o.id, y: target.y, into: target.into, depth: target.depth } : { track: o.id, y: null, into: null, ...(blocked ? { blocked } : {}) },
    );
  };
  // Near the view's top / bottom edge the view scrolls (and the target follows).
  const tick = () => {
    frame = 0;
    const el = scroller();
    if (!active || !el) return;
    const box = el.getBoundingClientRect();
    const speed = edgeScrollSpeed(last.y, box.top, box.bottom);
    if (speed === 0) return;
    const before = el.scrollTop;
    el.scrollTop = before + speed;
    if (el.scrollTop === before) return;
    update();
    frame = requestAnimationFrame(tick);
  };

  const move = (ev: PointerEvent) => {
    last = { x: ev.clientX, y: ev.clientY };
    if (!active) {
      const d = o.vertical ? Math.abs(ev.clientY - o.startY) : Math.hypot(ev.clientX - o.startX, ev.clientY - o.startY);
      if (d < DRAG_THRESHOLD_PX) return;
      active = true;
      // Escape cancels (and the arrangement's edit keys wait) only once it's a drag.
      release = holdClipDrag(cancel);
      o.onStart?.();
    }
    update();
    if (!frame) frame = requestAnimationFrame(tick);
  };
  const up = () => {
    const drop = active ? target : null;
    done();
    if (drop) o.onDrop(drop);
    else o.onEnd?.(active);
  };
  const cancel = () => {
    const was = active;
    done();
    o.onEnd?.(was);
  };
  let release = () => {};
  const done = () => {
    release();
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    setDragCursor(null);
    ui().setTrackDrag(null);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    active = false;
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
}

/** Header `onPointerDown`: a vertical drag moves the track (a click still selects it). */
export function onTrackHeaderPointerDown(e: ReactPointerEvent<HTMLElement>, track: Track, ctx: ArrangementContextValue): void {
  if (e.button !== 0 || !movable(track)) return;
  if ((e.target as HTMLElement).closest("button, [role='separator']")) return;
  let started = false;
  startTrackDrag({
    ctx,
    id: track.id,
    kind: track.kind,
    startX: e.clientX,
    startY: e.clientY,
    vertical: true,
    onStart: () => {
      started = true;
      // What the user does, for the extensions (`@/ext` "edit.activity"), cleared at the end.
      setActivity({ kind: "Dragging", target: { type: "Track", track: track.id } });
    },
    onDrop: (target) => {
      setActivity(null);
      void sendEdit(ctx.transport, cmd("Track", { type: "Move", id: track.id, parent: target.parent, before: target.before }));
    },
    onEnd: () => {
      if (started) setActivity(null);
    },
  });
}

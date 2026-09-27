/**
 * Reordering tracks by dragging their headers (pointer drag, not HTML5 drag-and-drop).
 *
 * Releasing between two rows moves the track before the row below (into that row's
 * group); over the middle of a group's header it moves the track into the group (at its
 * end). Groups move with their children. Master and return tracks stay where they are
 * (the engine keeps them at the bottom), and a group can't go inside itself.
 */

import type { PointerEvent as ReactPointerEvent } from "react";
import type { Track, TrackId } from "@/generated";
import { setDragCursor } from "@/kit";
import { setActivity } from "@/features/collab/presence/local";
import { cmd } from "@/transport";
import { sendEdit, type ArrangementContextValue } from "./context";
import type { Row } from "./layout";
import { useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 4;

export interface TrackDropTarget {
  parent: TrackId | null;
  before: TrackId | null;
  /** Indicator: a line at `y` (content px), or the group row the track goes into. */
  y: number;
  into: TrackId | null;
}

const movable = (t: Track) => t.kind !== "Master" && t.kind !== "Return";

function isInside(rows: ReadonlyArray<Row>, track: TrackId, ancestor: TrackId): boolean {
  const byId = new Map(rows.map((r) => [r.track.id, r.track]));
  let p = byId.get(track)?.parent ?? null;
  for (let guard = 0; p !== null && guard < 64; guard++) {
    if (p === ancestor) return true;
    p = byId.get(p)?.parent ?? null;
  }
  return false;
}

/** Where dropping `dragged` at content y lands, or null where it can't go. */
export function trackDropTarget(rows: ReadonlyArray<Row>, y: number, dragged: TrackId): TrackDropTarget | null {
  const self = (t: Track) => t.id === dragged || isInside(rows, t.id, dragged);
  // Into a group: over the middle half of its lane.
  const over = rows.find((r) => y >= r.y && y < r.y + r.laneHeight);
  if (over && over.track.kind === "Group" && !self(over.track)) {
    const q = (y - over.y) / over.laneHeight;
    if (q > 0.25 && q < 0.75) return { parent: over.track.id, before: null, y: over.y, into: over.track.id };
  }
  // Otherwise the gap nearest to the pointer: before the first row whose middle is below it.
  let i = rows.findIndex((r) => y < r.y + r.laneHeight / 2);
  if (i < 0) i = rows.length;
  // Skip the dragged track (and its children): dropping inside them means "no move".
  while (i < rows.length && self(rows[i]!.track) && rows[i]!.track.id !== dragged) i++;
  const below = rows[i];
  if (below && self(below.track)) return null;
  if (!below || !movable(below.track)) {
    // After the last regular track: end of the top level (the engine keeps it above returns).
    const last = [...rows].reverse().find((r) => movable(r.track));
    return { parent: null, before: null, y: last ? last.y + last.height : 0, into: null };
  }
  return { parent: below.track.parent, before: below.track.id, y: below.y, into: null };
}

/** Header `onPointerDown`: a vertical drag moves the track (a click still selects it). */
export function onTrackHeaderPointerDown(e: ReactPointerEvent<HTMLElement>, track: Track, ctx: ArrangementContextValue): void {
  if (e.button !== 0 || !movable(track)) return;
  if ((e.target as HTMLElement).closest("button, [role='separator']")) return;
  const startY = e.clientY;
  const top = () => ctx.contentRef.current?.getBoundingClientRect().top ?? 0;
  let target: TrackDropTarget | null = null;
  let active = false;
  const ui = useArrangementUi.getState;

  const move = (ev: PointerEvent) => {
    if (!active && Math.abs(ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
    if (!active) {
      setDragCursor("grabbing");
      // presence-v2: "Ada · dragging" for the peers, cleared in `done`.
      setActivity({ kind: "Dragging", target: { type: "Track", track: track.id } });
    }
    active = true;
    target = trackDropTarget(ctx.rowsRef.current, ev.clientY - top(), track.id);
    ui().setTrackDrag(target ? { track: track.id, y: target.y, into: target.into } : { track: track.id, y: null, into: null });
  };
  const up = () => {
    done();
    if (!active || !target) return;
    void sendEdit(ctx.transport, cmd("Track", { type: "Move", id: track.id, parent: target.parent, before: target.before }));
  };
  const done = () => {
    setDragCursor(null);
    if (active) setActivity(null);
    ui().setTrackDrag(null);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", done);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", done);
}

/**
 * The arrangement's hooks into time editing, so `ArrangementView` only wires them:
 * - selection and section are separate: a drag over the lanes selects
 *   clips (no section); a Shift-drag makes the time section instead (no clips), shown live
 *   while dragging; Shift can be pressed or released mid-drag to switch; a click clears it;
 * - a Shift-click extends the section (or the insert marker) to the click when outside it,
 *   retracts its nearer edge to the click when inside (`extendOrRetract`), and spans the
 *   clicked track too;
 * - key presses: the time-edit shortcuts run first (see `actions.ts`), Escape clears;
 * - a right-click inside the selection opens its menu (capture, before the lane's/clip's);
 * - header and clip selections replace the time selection.
 */

import { useEffect, useRef, type KeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import type { Beats } from "@/generated";
import { openContextMenu } from "@/kit";
import { extendOrRetract, itemSelection, pxToBeats, resolveGrid, snapToGrid, useTempoMap, type Point, type Rect } from "@/timeline";
import type { EngineTransport } from "@/transport";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { matchesAction } from "@/features/keymap";
import { bindTimeSelection, inTimeSelection, runTimeAction, timeActionForKey, timeSelectionMenu, type TimeActionContext } from "./actions";
import { isTimeTrack, selectionFromRect, type SelectionRow } from "./commands";
import { setPlayStart } from "./marker";
import { clearTimeSelection, useTimeSelection, type TimeRangeSelection } from "./store";

export interface ArrangementTimeEdits {
  /** Content `onPointerDown` (before the marquee starts). */
  onPointerDown(e: ReactPointerEvent): void;
  /** Marquee `onEnd` (a plain drag: clips only). */
  onMarqueeEnd(rect: Rect): void;
  /** Marquee `section` (a Shift-drag: the time section, live). */
  section: { onPreview(rect: Rect | null): void; onEnd(rect: Rect): void };
  /**
   * A Shift-click on the lanes: `select` (the view's track selection, which drops the time
   * selection) runs first, then the extended / retracted selection is set. `false` when there
   * was nothing to extend (a plain click then).
   */
  shiftClick(p: Point, select: () => void): boolean;
  /** Marquee `onClick`. */
  onMarqueeClick(): void;
  /** Arrangement `onKeyDown`: `true` if a time edit took the key. */
  onKeyDown(e: KeyboardEvent): boolean;
  /** Content `onContextMenuCapture`. */
  onContextMenuCapture(e: MouseEvent<HTMLElement>): void;
}

function context(): TimeActionContext {
  return {
    selectedTracks: useArrangementUi.getState().selectedTracks,
    hasSelectedClips: itemSelection.getState().selected.clip.size > 0,
  };
}

export function useArrangementTimeEdits(transport: EngineTransport, rowsRef: RefObject<ReadonlyArray<SelectionRow>>): ArrangementTimeEdits {
  useEffect(
    () =>
      bindTimeSelection({
        subscribe: (fn) => useArrangementUi.subscribe((s, prev) => s.selectedTracks !== prev.selectedTracks && fn(s.selectedTracks)),
      }),
    [],
  );
  const alt = useRef(false);
  const tempo = useTempoMap();
  const toBeats = (px: number) => pxToBeats(px, arrangementView.getState());
  /** Snapped to the arrangement grid, unless Alt was held (like clip drags). */
  const snap = (beats: Beats): Beats => {
    if (alt.current) return beats;
    const s = arrangementView.getState();
    return snapToGrid(beats, resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats)), tempo);
  };
  const fromRect = (rect: Rect) => selectionFromRect(rect, rowsRef.current ?? [], useArrangementUi.getState().headerWidth, (px) => snap(toBeats(px)));
  const shiftClick = (p: Point, select: () => void): boolean => {
    const prev = useTimeSelection.getState().selection;
    // Nothing to extend (or one automation lane's range): a plain click.
    if (!prev || prev.automationLane !== undefined) return false;
    const hw = useArrangementUi.getState().headerWidth;
    const next = extendOrRetract(prev, snap(toBeats(Math.max(0, p.x - hw))));
    const sel: TimeRangeSelection = { ...next, tracks: spanTracks(rowsRef.current ?? [], prev.tracks, p.y) };
    select();
    itemSelection.getState().clear("clip");
    useTimeSelection.getState().setSelection(sel);
    setPlayStart(transport, sel.start);
    return true;
  };
  return {
    shiftClick,
    onPointerDown: (e) => {
      alt.current = e.altKey;
    },
    onMarqueeEnd: () => clearTimeSelection(),
    section: {
      onPreview: (rect) => {
        if (!rect) return clearTimeSelection();
        itemSelection.getState().clear("clip");
        useTimeSelection.getState().setSelection(fromRect(rect));
      },
      onEnd: (rect) => {
        const sel = fromRect(rect);
        useTimeSelection.getState().setSelection(sel);
        // The selection cursor is the range start: while stopped the playhead goes there.
        if (sel) setPlayStart(transport, sel.start);
      },
    },
    onMarqueeClick: clearTimeSelection,
    onKeyDown: (e) => {
      const action = timeActionForKey(e, context());
      if (!action) {
        if (matchesAction("edit.deselect", e)) clearTimeSelection();
        return false;
      }
      e.preventDefault();
      e.stopPropagation();
      void runTimeAction(transport, action, context());
      return true;
    },
    onContextMenuCapture: (e) => {
      const box = e.currentTarget.getBoundingClientRect();
      const hw = useArrangementUi.getState().headerWidth;
      if (inTimeSelection(e.clientX - box.left, e.clientY - box.top, rowsRef.current ?? [], toBeats, hw)) {
        openContextMenu(e, timeSelectionMenu(transport));
      }
    },
  };
}

/**
 * The tracks of a selection on `tracks` after a Shift-click at content `y`: unchanged on one
 * of them (or off the time tracks), else every time track from the selection to the clicked
 * one, in display order.
 */
export function spanTracks(rows: ReadonlyArray<SelectionRow>, tracks: ReadonlyArray<string>, y: number): string[] {
  const order = [...new Set(rows.filter((r) => isTimeTrack(r.track)).map((r) => r.track.id))];
  const clicked = rows.find((r) => y >= r.y && y < r.y + r.height);
  if (!clicked || !isTimeTrack(clicked.track) || tracks.includes(clicked.track.id)) return [...tracks];
  const at = [...tracks, clicked.track.id].map((t) => order.indexOf(t)).filter((i) => i >= 0);
  const span = order.slice(Math.min(...at), Math.max(...at) + 1);
  // Selected tracks not shown as rows (none, normally) stay selected.
  return [...span, ...tracks.filter((t) => !span.includes(t))];
}

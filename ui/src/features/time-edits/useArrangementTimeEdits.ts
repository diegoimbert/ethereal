/**
 * The arrangement's hooks into time editing, so `ArrangementView` only wires them:
 * - the marquee (a drag over the lanes) also makes the time selection; a click clears it;
 * - key presses: the time-edit shortcuts run first (see `actions.ts`), Escape clears;
 * - a right-click inside the selection opens its menu (capture, before the lane's/clip's);
 * - header and clip selections replace the time selection.
 */

import { useEffect, useRef, type KeyboardEvent, type MouseEvent, type PointerEvent, type RefObject } from "react";
import type { Beats } from "@/generated";
import { openContextMenu } from "@/kit";
import { itemSelection, pxToBeats, resolveGrid, snapToGrid, useTempoMap, type Rect } from "@/timeline";
import type { EngineTransport } from "@/transport";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { matchesAction } from "@/features/keymap";
import { bindTimeSelection, inTimeSelection, runTimeAction, timeActionForKey, timeSelectionMenu, type TimeActionContext } from "./actions";
import { selectionFromRect, type SelectionRow } from "./commands";
import { clearTimeSelection, useTimeSelection } from "./store";

export interface ArrangementTimeEdits {
  /** Content `onPointerDown` (before the marquee starts). */
  onPointerDown(e: PointerEvent): void;
  /** Marquee `onEnd`. */
  onMarqueeEnd(rect: Rect): void;
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
  return {
    onPointerDown: (e) => {
      alt.current = e.altKey;
    },
    onMarqueeEnd: (rect) => {
      const hw = useArrangementUi.getState().headerWidth;
      const sel = selectionFromRect(rect, rowsRef.current ?? [], hw, (px) => snap(toBeats(px)));
      useTimeSelection.getState().setSelection(sel);
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

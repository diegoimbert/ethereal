/**
 * Double-click-and-drag on an empty MIDI lane inserts a clip: the second press of the
 * double-click starts the gesture, dragging (without releasing) sets the clip's span, and
 * the release creates it. A double-click without a drag creates a one-bar clip.
 *
 * The span covers the grid cells dragged over (start floored, end ceiled) and can extend
 * left of the press. Hold alt/option to bypass snapping.
 */

import type { PointerEvent as ReactPointerEvent } from "react";
import type { Beats, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { createDoublePress, pxToBeats, resolveGrid, snapToGrid, TempoMap } from "@/timeline";
import { cmd, newId } from "@/transport";
import { sendEdit, type ArrangementContextValue } from "./context";
import { barAround } from "./helpers";
import { arrangementView, useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 3;
/** Shortest clip when snapping is off or bypassed. */
const MIN_LENGTH: Beats = 1 / 16;

export interface InsertSpan {
  start: Beats;
  length: Beats;
}

const isDoublePress = createDoublePress();

/** The span between two beat positions, snapped outward to the grid. */
export function insertSpan(a: Beats, b: Beats, snap: (b: Beats, mode: "floor" | "ceil") => Beats): InsertSpan {
  const start = Math.max(0, snap(Math.min(a, b), "floor"));
  const end = Math.max(snap(Math.max(a, b), "ceil"), start + MIN_LENGTH);
  return { start, length: end - start };
}

/**
 * Lane `onPointerDown` for MIDI tracks. Returns without doing anything unless this is the
 * second press of a double-click on the lane background; then it owns the gesture and
 * reports the live span through `onPreview` (null when it ends).
 */
export function onLaneInsertPointerDown(
  e: ReactPointerEvent<HTMLElement>,
  track: TrackId,
  ctx: ArrangementContextValue,
  onPreview: (span: InsertSpan | null) => void,
): void {
  if (e.button !== 0 || e.target !== e.currentTarget) return;
  if (!isDoublePress(e, track)) return;
  e.stopPropagation();
  e.preventDefault();
  ctx.focus();

  const project = useProjectStore.getState().project;
  if (!project) return;
  const tempo = TempoMap.fromProject(project);
  const view = arrangementView;
  const left = e.currentTarget.getBoundingClientRect().left;
  const beatAt = (clientX: number) => Math.max(0, pxToBeats(clientX - left, view.getState()));
  const anchor = beatAt(e.clientX);
  const startX = e.clientX;
  let active = false;
  let span: InsertSpan | null = null;

  const snapFor = (bypass: boolean) => (b: Beats, mode: "floor" | "ceil") => {
    if (bypass) return b;
    const s = view.getState();
    const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
    return snapToGrid(b, step, tempo, mode);
  };

  const move = (ev: PointerEvent) => {
    if (!active && Math.abs(ev.clientX - startX) < DRAG_THRESHOLD_PX) return;
    active = true;
    span = insertSpan(anchor, beatAt(ev.clientX), snapFor(ev.altKey));
    onPreview(span);
  };

  const up = () => {
    done();
    const s = active && span ? span : barAround(tempo, anchor);
    void sendEdit(
      ctx.transport,
      cmd("Clip", { type: "CreateMidi", id: newId(), track, start: s.start, length: s.length, name: null }),
    ).finally(() => onPreview(null));
  };

  const cancel = () => {
    done();
    onPreview(null);
  };

  const done = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
}

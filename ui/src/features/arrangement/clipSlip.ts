/**
 * The slip drag (see slip.ts for the math): Ctrl+Shift (Cmd+Shift on macOS) and drag a
 * clip's body to move its content under it. Previews live (`useArrangementUi.preview`, so
 * both renderers redraw the waveform and the notes) and commits one edit on release.
 *
 * - The pressed clip, or every selected clip when it is selected (each clamped on its own).
 * - Snapping follows the arrangement grid; hold alt/option to slip freely (as other drags).
 * - Escape (or a pointercancel) drops the preview; nothing is sent (`dragGuard.ts`).
 */

import type { Beats, Clip } from "@/generated";
import { notesOfClip, useProjectStore, useSelectionStore, warpMarkersOfClip } from "@/state";
import { beatsPerBar, dragBeats, itemSelection, resolveGrid, TempoMap } from "@/timeline";
import { setDragCursor } from "@/kit";
import { setEditActivity as setActivity } from "@/ext/surfaces";
import { isArrangementClip, startOf } from "./clipTime";
import type { ClipPointerEvent } from "./clipDrag";
import { sendEdit, type ArrangementContextValue } from "./context";
import { holdClipDrag } from "./dragGuard";
import { audioSlipSource, midiSlipSource, slipCommand, slipPreview, slipRange, slipRate, SLIP_CURSOR, type SlipClip } from "./slip";
import { arrangementView, useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 3;

export function onClipSlipPointerDown(e: ClipPointerEvent, clip: Clip, ctx: ArrangementContextValue): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  ctx.focus();
  const project = useProjectStore.getState().project;
  if (!project) return;

  const sel = itemSelection.getState();
  if (!sel.isSelected("clip", clip.id)) sel.select("clip", [clip.id], "replace");
  useSelectionStore.getState().selectTrack(clip.track);
  const clips: Clip[] = [];
  for (const id of itemSelection.getState().selected.clip) {
    const c = project.clips[id];
    if (c && isArrangementClip(c)) clips.push(c);
  }
  if (!clips.some((c) => c.id === clip.id)) clips.push(clip);

  const tempo = TempoMap.fromProject(project);
  const kind = ctx.transport.kind;
  const slipping: SlipClip[] = clips.map((c) => {
    const markers = warpMarkersOfClip(project, c.id);
    const source =
      c.content.type === "Audio"
        ? audioSlipSource(c, project.media[c.content.media], markers, tempo.bpmAt(startOf(c)), kind)
        : midiSlipSource(c, notesOfClip(project, c.id));
    return { clip: c, range: slipRange(c, source), rate: slipRate(c, markers, kind) };
  });

  const view = arrangementView;
  const startX = e.clientX;
  // Beats from the press to the pointer through the view of each moment (scroll/zoom mid-drag).
  const laneX = (clientX: number) => clientX - (ctx.contentRef.current?.getBoundingClientRect().left ?? 0) - useArrangementUi.getState().headerWidth;
  const beatsMoved = dragBeats(view.getState, laneX(startX));
  let x = startX;
  let active = false;
  let last: ReturnType<typeof slipPreview> | null = null;

  const stepAt = (c: Clip): Beats | null => {
    const step = resolveGrid(useArrangementUi.getState().grid, view.getState().pxPerBeat, tempo.signatureAt(startOf(c)));
    if (!step) return null;
    return step.kind === "beats" ? step.beats : step.bars * beatsPerBar(tempo.signatureAt(startOf(c)));
  };

  const update = (ev: { altKey: boolean }) => {
    setDragCursor(SLIP_CURSOR);
    last = slipPreview(slipping, beatsMoved(laneX(x)), ev.altKey ? () => null : stepAt);
    useArrangementUi.getState().setPreview({ bounds: last, copy: false });
  };

  const move = (ev: PointerEvent) => {
    x = ev.clientX;
    if (!active && Math.abs(x - startX) < DRAG_THRESHOLD_PX) return;
    if (!active) {
      const target = clips.length > 1 ? ({ type: "Selection" } as const) : ({ type: "Clip", clip: clip.id } as const);
      setActivity({ kind: "Resizing", target });
    }
    active = true;
    update(ev);
  };
  const key = (ev: KeyboardEvent) => {
    if (active && ev.key === "Alt") update(ev);
  };
  const up = () => {
    done();
    if (!active || !last) {
      useArrangementUi.getState().setPreview(null);
      return;
    }
    void sendEdit(ctx.transport, slipCommand(clips, last)).finally(() => useArrangementUi.getState().setPreview(null));
  };
  const cancel = () => {
    done();
    useArrangementUi.getState().setPreview(null);
  };
  const done = () => {
    setDragCursor(null);
    if (active) setActivity(null);
    release();
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key);
    window.removeEventListener("keyup", key);
  };
  // Escape cancels the drag (browsers send no pointercancel for it); edit keys wait.
  const release = holdClipDrag(cancel);
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", key);
  window.addEventListener("keyup", key);
}

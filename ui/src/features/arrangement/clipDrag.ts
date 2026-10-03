/**
 * Pointer handling for clips: click selection, then move / resize drags with a live local
 * preview (`useArrangementUi.preview`) committed as one edit on release.
 *
 * - Move: drag the body; vertical movement moves clips between compatible tracks. Below the
 *   last track (the empty canvas above the pinned master) the clips go to new tracks, shown
 *   as ghost lanes meanwhile (see newTrackDrag.ts).
 * - Resize: drag the left/right edge handles (`data-handle="resize-start" | "resize-end"`).
 * - Snapping follows the arrangement grid; hold alt/option to bypass it.
 * - Cmd/ctrl held on release copies instead of moving.
 * - A plain click on an already selected clip selects only it; shift adds; cmd/ctrl toggles.
 * - While stopped, pressing a clip moves the playhead to its start.
 */

import type { PointerEvent as ReactPointerEvent } from "react";
import type { Beats, Clip, MediaRef } from "@/generated";
import { useProjectStore, useSelectionStore, warpMarkersOfClip } from "@/state";
import { itemSelection, resolveGrid, selectModeFromEvent, snapToGrid, TempoMap } from "@/timeline";
import { newId, type EngineTransport } from "@/transport";
import { setDragCursor } from "@/kit";
import { clipSourceMapper } from "@/features/warp/warpMap";
import { setActivity } from "@/features/collab/presence/local";
import { isArrangementClip, mediaLengthInBeats, startOf } from "./clipTime";
import { locateIfStopped } from "./actions";
import { sendEdit, type ArrangementContextValue } from "./context";
import { boundsCommand, dragPreview, moveCommand, type DragMode } from "./editMath";
import { rowIndexAt } from "./layout";
import { inNewTrackZone, newTrackDropCommand, newTrackLanes, onNewLanes, type NewLane } from "./newTrackDrag";
import { arrangementView, useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 3;

/** Media length in content beats of an audio clip (null for MIDI or unknown media). */
export function clipSourceLength(
  clip: Clip,
  media: Readonly<Record<string, MediaRef>>,
  tempo: TempoMap,
  kind: EngineTransport["kind"] = "mock",
): Beats | null {
  if (clip.content.type !== "Audio") return null;
  const m = media[clip.content.media];
  if (!m) return null;
  const project = useProjectStore.getState().project;
  const markers = project ? warpMarkersOfClip(project, clip.id) : [];
  const map = clipSourceMapper(clip.content, markers, tempo.bpmAt(startOf(clip)), clip.offset, kind);
  return mediaLengthInBeats(m, map);
}

/**
 * `keepSelection`: the caller already selected what to drag (a clip cluster): a click
 * without a drag keeps that selection instead of narrowing it to `clip`.
 */
export function onClipPointerDown(
  e: ReactPointerEvent<HTMLElement>,
  clip: Clip,
  ctx: ArrangementContextValue,
  opts: { keepSelection?: boolean } = {},
): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  ctx.focus();
  const handle = (e.target as HTMLElement).closest<HTMLElement>("[data-handle]")?.dataset.handle;
  const mode: DragMode = handle === "resize-start" || handle === "resize-end" ? handle : "move";

  const sel = itemSelection.getState();
  const wasSelected = sel.isSelected("clip", clip.id);
  const selectMode = selectModeFromEvent(e);
  if (!wasSelected) sel.select("clip", [clip.id], selectMode === "replace" ? "replace" : "add");
  useSelectionStore.getState().selectTrack(clip.track);
  locateIfStopped(ctx.transport, startOf(clip));

  const project = useProjectStore.getState().project;
  if (!project) return;
  const clips: Clip[] = [];
  for (const id of itemSelection.getState().selected.clip) {
    const c = project.clips[id];
    if (c && isArrangementClip(c)) clips.push(c);
  }
  if (!clips.some((c) => c.id === clip.id)) clips.push(clip);

  const tempo = TempoMap.fromProject(project);
  const view = arrangementView;
  const rows = ctx.rowsRef.current;
  const contentTop = () => ctx.contentRef.current?.getBoundingClientRect().top ?? 0;
  const rowAt = (clientY: number) =>
    Math.min(rows.length - 1, Math.max(0, rowIndexAt(rows, clientY - contentTop())));
  const startX = e.clientX;
  const startY = e.clientY;
  const startRow = rowAt(startY);
  const { pxPerBeat } = view.getState();
  let active = false;
  let last: ReturnType<typeof dragPreview> | null = null;
  let copy = false;
  // The tracks a drop below the last track would create (ids fixed for the whole drag).
  let lanes: NewLane[] | null = null;
  let below = false;
  /** Pointer in the empty canvas below the last track (not over the pinned master). */
  const belowLastTrack = (clientY: number) => {
    const scroller = ctx.contentRef.current?.parentElement;
    const bottom = scroller ? scroller.getBoundingClientRect().bottom - contentTop() : Infinity;
    return inNewTrackZone(rows, clientY - contentTop(), bottom);
  };

  const snapFor = (bypass: boolean) => {
    if (bypass) return (b: Beats) => b;
    const s = view.getState();
    const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
    return (b: Beats) => snapToGrid(b, step, tempo);
  };

  type Mods = Pick<PointerEvent, "altKey" | "metaKey" | "ctrlKey">;
  let pointer = { x: startX, y: startY };
  /** Preview for the pointer position and the held modifiers (cmd/ctrl: copy). */
  const update = (ev: Mods) => {
    const dx = pointer.x - startX;
    copy = mode === "move" && (ev.metaKey || ev.ctrlKey);
    below = mode === "move" && belowLastTrack(pointer.y);
    setDragCursor(mode === "move" ? (copy ? "copy" : "grabbing") : "ew-resize");
    last = dragPreview(
      {
        clips,
        anchor: clip.id,
        rows,
        snap: snapFor(ev.altKey),
        sourceLength: (c) => clipSourceLength(c, project.media, tempo, ctx.transport.kind),
      },
      mode,
      dx / pxPerBeat,
      mode === "move" && !below ? rowAt(pointer.y) - startRow : 0,
    );
    if (below) {
      lanes ??= newTrackLanes(clips, rows, newId);
      last = onNewLanes(last, clips, lanes);
    }
    useArrangementUi.getState().setPreview(below && lanes ? { bounds: last, copy, newTracks: lanes } : { bounds: last, copy });
  };

  const move = (ev: PointerEvent) => {
    pointer = { x: ev.clientX, y: ev.clientY };
    if (!active && Math.hypot(pointer.x - startX, pointer.y - startY) < DRAG_THRESHOLD_PX) return;
    if (!active) {
      // presence-v2: "Ada · dragging" for the peers, cleared in `done`.
      const target = clips.length > 1 ? ({ type: "Selection" } as const) : ({ type: "Clip", clip: clip.id } as const);
      setActivity({ kind: mode === "move" ? "Dragging" : "Resizing", target });
    }
    active = true;
    update(ev);
  };

  // Pressing or releasing cmd/ctrl (or alt) mid-drag switches copy (or snapping) at once:
  // the originals stay where they were while copying.
  const key = (ev: KeyboardEvent) => {
    if (active && ["Meta", "Control", "Alt"].includes(ev.key)) update(ev);
  };

  const up = (ev: PointerEvent) => {
    done();
    if (!active || !last) {
      useArrangementUi.getState().setPreview(null);
      if (wasSelected && !opts.keepSelection) {
        if (selectMode === "replace") itemSelection.getState().select("clip", [clip.id], "replace");
        else if (selectMode === "toggle") itemSelection.getState().select("clip", [clip.id], "toggle");
      }
      return;
    }
    copy = mode === "move" && (ev.metaKey || ev.ctrlKey);
    const all = Object.values(project.clips);
    const command =
      mode !== "move"
        ? boundsCommand(clips, last)
        : below && lanes
          ? newTrackDropCommand(lanes, clips, last, copy, newId, all)
          : moveCommand(clips, last, copy, newId, all);
    void sendEdit(ctx.transport, command).finally(() => useArrangementUi.getState().setPreview(null));
  };

  const cancel = () => {
    done();
    useArrangementUi.getState().setPreview(null);
  };

  const done = () => {
    setDragCursor(null);
    if (active) setActivity(null);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key);
    window.removeEventListener("keyup", key);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", key);
  window.addEventListener("keyup", key);
}

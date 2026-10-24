/**
 * Pointer handling for clips: click selection, then move / resize drags with a live local
 * preview (`useArrangementUi.preview`) committed as one edit on release.
 *
 * - Move: drag the body; vertical movement moves clips between compatible tracks. Below the
 *   last track (the empty canvas above the pinned master) the clips go to new tracks, shown
 *   as ghost lanes meanwhile (see newTrackDrag.ts).
 * - Resize: drag the left/right edge handles (`data-handle="resize-start" | "resize-end"`).
 * - Stretch: ctrl+shift (cmd+shift on macOS) held on an edge drag scales the content with
 *   the length (it plays faster or slower; `ClipCommand::Stretch`). Pressing or releasing
 *   the modifiers mid-drag switches between resize and stretch.
 * - Snapping follows the arrangement grid; hold alt/option to bypass it.
 * - Cmd/ctrl held on release copies instead of moving.
 * - A plain click on an already selected clip selects only it; shift adds; cmd/ctrl toggles.
 * - While stopped, pressing a clip moves the playhead to its start.
 */

import type { Beats, Clip, MediaRef } from "@/generated";
import { isReadOnly, useProjectStore, useSelectionStore, warpMarkersOfClip } from "@/state";
import { dragBeats, itemSelection, resolveGrid, selectModeFromEvent, snapDelta, snapToGrid, TempoMap } from "@/timeline";
import { newId, type EngineTransport } from "@/transport";
import { setDragCursor } from "@/kit";
import { clipSourceMapper } from "@/domain/warpMap";
import { setEditActivity as setActivity } from "@/ext/surfaces";
import { isArrangementClip, mediaLengthInBeats, startOf } from "./clipTime";
import { placeInsertMarker } from "@/features/time-edits/marker";
import { sendEdit, type ArrangementContextValue } from "./context";
import { holdClipDrag } from "./dragGuard";
import { batchStep, boundsCommand, dragPreview, isStretchModifier, moveCommand, stretchCommand, stretchPreview, type DragMode } from "./editMath";
import { MOVE_SELECTION, pointsFollowing } from "./mixedSelection";
import { LaneGesture } from "@/features/automation/gesture";
import { flattenCommands } from "@/features/automation/laneRegistry";
import { rowIndexAt } from "./layout";
import { inNewTrackZone, newTrackDropCommand, newTrackLanes, onNewLanes, type NewLane } from "./newTrackDrag";
import { arrangementView, useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 3;

/** Media length in content beats of an audio clip (null for MIDI or unknown media). */
export function clipSourceLength(clip: Clip, media: Readonly<Record<string, MediaRef>>, tempo: TempoMap, kind: EngineTransport["kind"] = "mock"): Beats | null {
  if (clip.content.type !== "Audio") return null;
  const m = media[clip.content.media];
  if (!m) return null;
  const project = useProjectStore.getState().project;
  const markers = project ? warpMarkersOfClip(project, clip.id) : [];
  const map = clipSourceMapper(clip.content, markers, tempo.bpmAt(startOf(clip)), clip.offset, kind);
  return mediaLengthInBeats(m, map);
}

/** What a clip press needs from a pointer event (a React or a DOM one). */
export interface ClipPointerEvent {
  button: number;
  clientX: number;
  clientY: number;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  target: EventTarget | null;
  stopPropagation(): void;
  preventDefault(): void;
}

/**
 * `keepSelection`: the caller already selected what to drag (a clip cluster): a click
 * without a drag keeps that selection instead of narrowing it to `clip`. `mode`: the part
 * pressed, when the caller hit-tested it (the WebGL arranger); else it is read from the
 * pressed element's `data-handle`.
 */
export function onClipPointerDown(
  e: ClipPointerEvent,
  clip: Clip,
  ctx: ArrangementContextValue,
  opts: { keepSelection?: boolean; mode?: DragMode } = {},
): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  ctx.focus();
  const handle = opts.mode ?? (e.target instanceof Element ? e.target.closest<HTMLElement>("[data-handle]")?.dataset.handle : undefined);
  const mode: DragMode = handle === "resize-start" || handle === "resize-end" ? handle : "move";

  const sel = itemSelection.getState();
  const wasSelected = sel.isSelected("clip", clip.id);
  const selectMode = selectModeFromEvent(e);
  if (!wasSelected) sel.select("clip", [clip.id], selectMode === "replace" ? "replace" : "add");
  useSelectionStore.getState().selectTrack(clip.track);
  // The clip start becomes the selection: the insert marker on the clip's track (the
  // playhead follows while stopped). A click on the clip's body places it where clicked.
  placeInsertMarker(ctx.transport, startOf(clip), [clip.track]);

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
  const rowAt = (clientY: number) => Math.min(rows.length - 1, Math.max(0, rowIndexAt(rows, clientY - contentTop())));
  const startX = e.clientX;
  const startY = e.clientY;
  const startRow = rowAt(startY);
  // Beats from the press to the pointer, through the view of each moment: the clips stay
  // under the pointer when the view pages (follow playhead), scrolls or zooms mid-drag.
  const laneX = (clientX: number) => clientX - (ctx.contentRef.current?.getBoundingClientRect().left ?? 0) - useArrangementUi.getState().headerWidth;
  const beatsMoved = dragBeats(view.getState, laneX(startX));
  let active = false;
  let last: ReturnType<typeof dragPreview> | null = null;
  let copy = false;
  // The tracks a drop below the last track would create (ids fixed for the whole drag).
  let lanes: NewLane[] | null = null;
  let below = false;
  // Track automation points selected with the clips (a box across both) move along, in
  // time (one gesture: live in their lanes, one undo step with the clips' move).
  const points = mode === "move" ? pointsFollowing() : null;
  const pointsGesture = points ? new LaneGesture(ctx.transport) : null;
  /** The points' shift for the preview `last` (none while copying or creating tracks). */
  const pointsDt = () => {
    const b = last?.get(clip.id);
    if (!points || !b || copy || below) return 0;
    return Math.max(b.start - startOf(clip), -points.minTime);
  };
  /** Pointer in the empty canvas below the last track (not over the pinned master). */
  const belowLastTrack = (clientY: number) => {
    const scroller = ctx.contentRef.current?.parentElement;
    const bottom = scroller ? scroller.getBoundingClientRect().bottom - contentTop() : Infinity;
    return inNewTrackZone(rows, clientY - contentTop(), bottom);
  };

  /** The visible grid step (adaptive: at the current zoom), `null` when off or bypassed. */
  const stepFor = (bypass: boolean) => {
    if (bypass) return null;
    const s = view.getState();
    return resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
  };
  const snapFor = (bypass: boolean) => {
    const step = stepFor(bypass);
    return (b: Beats) => snapToGrid(b, step, tempo);
  };
  // Moves snap relatively: an off-grid clip keeps its offset within the grid.
  const snapMoveFor = (bypass: boolean) => {
    const step = stepFor(bypass);
    return (origin: Beats, delta: Beats) => snapDelta(origin, delta, step, tempo, true);
  };

  type Mods = Pick<PointerEvent, "altKey" | "metaKey" | "ctrlKey" | "shiftKey">;
  let pointer = { x: startX, y: startY };
  let mods: Mods = { altKey: e.altKey, metaKey: e.metaKey, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey };
  let stretch = false;
  /** Preview for the pointer position and the held modifiers (cmd/ctrl: copy; on an edge, cmd/ctrl+shift: stretch). */
  const update = (ev: Mods) => {
    mods = { altKey: ev.altKey, metaKey: ev.metaKey, ctrlKey: ev.ctrlKey, shiftKey: ev.shiftKey };
    copy = mode === "move" && (ev.metaKey || ev.ctrlKey);
    stretch = mode !== "move" && isStretchModifier(ev);
    below = mode === "move" && belowLastTrack(pointer.y);
    setDragCursor(mode === "move" ? (copy ? "copy" : "grabbing") : "ew-resize");
    const input = {
      clips,
      anchor: clip.id,
      rows,
      snap: snapFor(ev.altKey),
      snapMove: snapMoveFor(ev.altKey),
      sourceLength: (c: Clip) => clipSourceLength(c, project.media, tempo, ctx.transport.kind),
    };
    last =
      mode !== "move" && stretch
        ? stretchPreview(input, mode, beatsMoved(laneX(pointer.x)))
        : dragPreview(input, mode, beatsMoved(laneX(pointer.x)), mode === "move" && !below ? rowAt(pointer.y) - startRow : 0);
    if (below) {
      lanes ??= newTrackLanes(clips, rows, newId);
      last = onNewLanes(last, clips, lanes);
    }
    useArrangementUi.getState().setPreview(below && lanes ? { bounds: last, copy, newTracks: lanes } : { bounds: last, copy });
    if (points) pointsGesture!.update(points.command(pointsDt()));
  };

  const move = (ev: PointerEvent) => {
    // A shared project's viewer: the press selects, nothing moves.
    if (isReadOnly()) return;
    pointer = { x: ev.clientX, y: ev.clientY };
    if (!active && Math.hypot(pointer.x - startX, pointer.y - startY) < DRAG_THRESHOLD_PX) return;
    if (!active) {
      // What the user does, for the extensions (`@/ext` "edit.activity"), cleared in `done`.
      const target = clips.length > 1 ? ({ type: "Selection" } as const) : ({ type: "Clip", clip: clip.id } as const);
      setActivity({ kind: mode === "move" ? "Dragging" : "Resizing", target });
    }
    active = true;
    update(ev);
  };

  // Pressing or releasing cmd/ctrl (or alt, shift) mid-drag switches copy (or snapping,
  // stretching) at once: the originals stay where they were while copying.
  const key = (ev: KeyboardEvent) => {
    if (active && ["Meta", "Control", "Alt", "Shift"].includes(ev.key)) update(ev);
  };

  const up = (ev: PointerEvent) => {
    done();
    if (!active || !last) {
      useArrangementUi.getState().setPreview(null);
      pointsGesture?.end();
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
        ? stretch
          ? stretchCommand(clips, last)
          : boundsCommand(clips, last)
        : below && lanes
          ? newTrackDropCommand(lanes, clips, last, copy, newId, all)
          : moveCommand(clips, last, copy, newId, all);
    if (points && pointsGesture) {
      // The points' last edit and the clips' move, in the points' gesture: one undo step.
      const dt = pointsDt();
      const both = flattenCommands([points.command(dt), command]);
      const clear = () => useArrangementUi.getState().setPreview(null);
      if (both.length) pointsGesture.update(batchStep(MOVE_SELECTION, both), clear);
      else clear();
      pointsGesture.end();
      return;
    }
    void sendEdit(ctx.transport, command).finally(() => useArrangementUi.getState().setPreview(null));
  };

  const cancel = () => {
    done();
    useArrangementUi.getState().setPreview(null);
    // The points go back where they were.
    if (points && pointsGesture) {
      pointsGesture.update(points.command(0));
      pointsGesture.end();
    }
  };

  // The view moving under a still pointer (paging while playing) moves the preview too.
  const unsubscribeView = view.subscribe((s, prev) => {
    if (active && (s.scrollBeats !== prev.scrollBeats || s.pxPerBeat !== prev.pxPerBeat)) update(mods);
  });

  const done = () => {
    setDragCursor(null);
    if (active) setActivity(null);
    release();
    unsubscribeView();
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

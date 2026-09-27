/**
 * Pointer handling for clips: click selection, then move / resize drags with a live local
 * preview (`useArrangementUi.preview`) committed as one edit on release.
 *
 * - Move: drag the body; vertical movement moves clips between compatible tracks.
 * - Resize: drag the left/right edge handles (`data-handle="resize-start" | "resize-end"`).
 * - Snapping follows the arrangement grid; hold alt/option to bypass it.
 * - Cmd/ctrl held on release copies instead of moving.
 * - A plain click on an already selected clip selects only it; shift adds; cmd/ctrl toggles.
 * - While stopped, pressing a clip moves the playhead to its start.
 */

import type { PointerEvent as ReactPointerEvent } from "react";
import type { Beats, Clip, MediaRef } from "@/generated";
import { playheadStore, useProjectStore, useSelectionStore, warpMarkersOfClip } from "@/state";
import { itemSelection, resolveGrid, selectModeFromEvent, snapToGrid, TempoMap } from "@/timeline";
import { cmd, newId, type EngineTransport } from "@/transport";
import { setDragCursor } from "@/kit";
import { clipSourceMapper } from "@/features/warp/warpMap";
import { isArrangementClip, mediaLengthInBeats, startOf } from "./clipTime";
import { sendEdit, type ArrangementContextValue } from "./context";
import { boundsCommand, dragPreview, moveCommand, type DragMode } from "./editMath";
import { rowIndexAt } from "./layout";
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

export function onClipPointerDown(e: ReactPointerEvent<HTMLElement>, clip: Clip, ctx: ArrangementContextValue): void {
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
  if (!playheadStore.getPlayhead()?.transport.playing) {
    ctx.transport.send(cmd("Transport", { type: "Locate", position: startOf(clip) })).catch(() => {});
  }

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

  const snapFor = (bypass: boolean) => {
    if (bypass) return (b: Beats) => b;
    const s = view.getState();
    const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
    return (b: Beats) => snapToGrid(b, step, tempo);
  };

  const move = (ev: PointerEvent) => {
    const dx = ev.clientX - startX;
    const dy = ev.clientY - startY;
    if (!active && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
    if (!active) setDragCursor(mode === "move" ? "grabbing" : "ew-resize");
    active = true;
    copy = mode === "move" && (ev.metaKey || ev.ctrlKey);
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
      mode === "move" ? rowAt(ev.clientY) - startRow : 0,
    );
    useArrangementUi.getState().setPreview({ bounds: last, copy });
  };

  const up = (ev: PointerEvent) => {
    done();
    if (!active || !last) {
      useArrangementUi.getState().setPreview(null);
      if (wasSelected) {
        if (selectMode === "replace") itemSelection.getState().select("clip", [clip.id], "replace");
        else if (selectMode === "toggle") itemSelection.getState().select("clip", [clip.id], "toggle");
      }
      return;
    }
    copy = mode === "move" && (ev.metaKey || ev.ctrlKey);
    const command = mode === "move" ? moveCommand(clips, last, copy, newId) : boundsCommand(clips, last);
    void sendEdit(ctx.transport, command).finally(() => useArrangementUi.getState().setPreview(null));
  };

  const cancel = () => {
    done();
    useArrangementUi.getState().setPreview(null);
  };

  const done = () => {
    setDragCursor(null);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
}

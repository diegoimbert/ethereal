/**
 * Nudging the selected clips by one grid step (the design's §6 "Precision"): the clip
 * bar's ◀ ▶, where a drag would be too coarse. The step is the arrangement's grid at the
 * current zoom (a beat when the grid is off); never before the song's start.
 */
import type { Clip, Command, Project } from "@/generated";
import { resolveGrid, stepLength, TempoMap, type GridSetting, type TimelineViewport } from "@/timeline";
import { newId } from "@/transport";
import { dragPreview, moveCommand } from "@/features/arrangement/editMath";

export function nudgeStep(project: Project, grid: GridSetting, vp: TimelineViewport): number {
  const tempo = TempoMap.fromProject(project);
  const sig = tempo.signatureAt(vp.scrollBeats);
  const step = resolveGrid(grid, vp.pxPerBeat, sig);
  return step ? stepLength(step, sig) : 1;
}

export function nudgeCommand(project: Project, clips: ReadonlyArray<Clip>, beats: number): Command | null {
  if (clips.length === 0 || beats === 0) return null;
  const preview = dragPreview({ clips, anchor: clips[0]!.id, rows: [], snap: (b) => b }, "move", beats, 0);
  if ([...preview.values()].every((b, i) => b.start === clips[i]!.start)) return null;
  return moveCommand(clips, preview, false, newId, Object.values(project.clips));
}

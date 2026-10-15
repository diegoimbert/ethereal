/**
 * Group lanes on the arranger: the summary of the group's tracks' clips: merged spans as
 * bars across 40–60 % of the lane's height,
 * in the group's color at half opacity, with `radius.sm` corners.
 */

import type { Beats, Project, TrackId } from "@/generated";
import { radius } from "@/theme";
import { groupSummarySpans } from "../../helpers";
import type { ArrangerLayer } from "../layers";
import { rgbOf } from "../palette";
import { writeRect } from "../pipelines";

const cache = new Map<TrackId, { clips: Project["clips"]; tracks: Project["tracks"]; spans: Array<[Beats, Beats]> }>();

/** The merged spans of a group (memoized on the clips and tracks tables). */
export function summarySpans(project: Pick<Project, "clips" | "tracks">, group: TrackId): Array<[Beats, Beats]> {
  const c = cache.get(group);
  if (c && c.clips === project.clips && c.tracks === project.tracks) return c.spans;
  const spans = groupSummarySpans(project.tracks, project.clips, group);
  cache.set(group, { clips: project.clips, tracks: project.tracks, spans });
  return spans;
}

export function groupSummaryLayer(): ArrangerLayer {
  return {
    id: "group-summary",
    z: "under-clips",
    deps: ({ track, input, row }) => (track.kind === "Group" ? [summarySpans(input.project, track.id), track.color, row.laneHeight, row.takesHeight] : NONE),
    build({ track, input, row }, out) {
      if (track.kind !== "Group") return;
      const main = row.laneHeight - (row.takesHeight ?? 0);
      const r = Math.min(parseFloat(radius.sm) || 0, (main * 0.2) / 2);
      const [cr, cg, cb] = rgbOf(track.color);
      const fill = [cr, cg, cb, 0.5] as const;
      for (const [s, l] of summarySpans(input.project, track.id)) {
        writeRect(out.rects, { x: { b0: s, b1: s + l, minW: 1 }, y: { a0: 0, f0: 0.4, a1: 0, f1: 0.6 }, radius: r, fill });
      }
    },
  };
}

const NONE: ReadonlyArray<unknown> = [];

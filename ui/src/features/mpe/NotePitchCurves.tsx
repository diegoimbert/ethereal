/**
 * Per-note pitch curves drawn inside the piano-roll grid (`mpe`): each note with a `Pitch`
 * expression gets a line from the centre of its row, moving one row per semitone, over the
 * note's span (where an MPE glide goes). Read-only: curves are edited in the lane area
 * below, where pressure and timbre live too.
 */

import { useMemo } from "react";
import type { Note } from "@/generated";
import { beatsToPx, pxToBeats, type TimelineViewport } from "@/timeline";
import { curvePath } from "@/features/expression/curve";
import { useNoteExpressions } from "@/features/expression/hooks";
import "./mpe.css";

export interface NotePitchCurvesProps {
  notes: ReadonlyArray<Note>;
  vp: TimelineViewport;
  keyH: number;
  /** y of the top of a note's row (the piano roll's `pitchToY`). */
  rowY: (pitch: number) => number;
  widthPx: number;
  height: number;
}

export function NotePitchCurves({ notes, vp, keyH, rowY, widthPx, height }: NotePitchCurvesProps) {
  const ids = useMemo(() => notes.map((n) => n.id), [notes]);
  const pitch = useNoteExpressions(ids, "Pitch");
  if (pitch.length === 0) return null;
  const byNote = new Map(notes.map((n) => [n.id, n]));
  return (
    <svg className="eth-mpe-curves" width={Math.max(0, widthPx)} height={height} aria-hidden data-testid="mpe-pitch-curves">
      {pitch.map((e) => {
        const n = byNote.get(e.note);
        if (!n || e.points.length === 0) return null;
        const centre = rowY(n.pitch) + keyH / 2;
        const xa = beatsToPx(n.start, vp);
        const xb = beatsToPx(n.start + n.duration, vp);
        const d = curvePath(
          e.points,
          (t) => beatsToPx(n.start + t, vp),
          (x) => pxToBeats(x, vp) - n.start,
          (v) => centre - v * keyH,
          xa,
          xb,
        );
        return <path key={e.id} className="eth-mpe-curves__pitch" d={d} data-note-id={n.id} data-testid="mpe-pitch-curve" />;
      })}
    </svg>
  );
}

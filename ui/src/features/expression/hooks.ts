import { useShallow } from "zustand/react/shallow";
import type { ClipId, ExpressionLane, NoteExpression, NoteExpressionKind, NoteId } from "@/generated";
import { useProjectStore } from "@/state";

const EMPTY: never[] = [];

/** The clip's expression lanes (any order; one per kind). */
export function useExpressionLanesOf(clip: ClipId): ExpressionLane[] {
  return useProjectStore(
    useShallow((s) => (s.project ? Object.values(s.project.expression_lanes).filter((l) => l.clip === clip) : EMPTY)),
  );
}

/** Note → its expression of `kind`, for the given notes. */
export function useNoteExpressions(notes: ReadonlyArray<NoteId>, kind: NoteExpressionKind): NoteExpression[] {
  return useProjectStore(
    useShallow((s) => {
      if (!s.project) return EMPTY;
      const set = new Set(notes);
      return Object.values(s.project.note_expressions).filter((e) => e.kind === kind && set.has(e.note));
    }),
  );
}

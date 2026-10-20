/**
 * Undo / redo bring the sections back: undoing an edit restores the
 * arrangement's time selection and the piano-roll section as they were when the edit was
 * made; redoing it restores them as they were when it was undone.
 *
 * The engine's history names its top steps (`HistoryState.undo_id` / `redo_id`, unique per
 * session), so each history change is told apart: a new step (its id appears on top of the
 * undo stack: the selections at that moment are the ones before the edit, as the view updates
 * them after the reply), an undo (the old top moves to the redo stack) or a redo (the reverse).
 * The selected notes and clips come back with them. View state only, per window; steps made
 * before the app started restore nothing.
 *
 * A gesture often drops a section before its edit lands (pressing a clip to drag it turns the
 * section into the insert marker; pressing a note clears the piano-roll section): when a new
 * step is made with no section (none, or just a marker), the latest real section since the
 * previous step is the one remembered.
 */

import type { HistoryState } from "@/generated";
import { itemSelection } from "@/timeline";
import { usePianoRollSection, type PianoRollSection } from "@/features/piano-roll/section";
import { useProjectStore } from "@/state";
import { useTimeSelection, type TimeRangeSelection } from "./store";

interface Selections {
  time: TimeRangeSelection | null;
  pianoRoll: PianoRollSection | null;
  /** The selected notes and clips (undoing a duplicate selects the originals again). */
  notes?: ReadonlySet<string>;
  clips?: ReadonlySet<string>;
}

/** A real range (not absent, not a zero-length insert marker). */
const isRange = (s: { start: number; end: number } | null): boolean => s !== null && s.end - s.start > 1e-6;

/** Steps remembered (older ones are forgotten first). */
const MAX_STEPS = 500;

const snapshot = (): Selections => ({
  time: useTimeSelection.getState().selection,
  pianoRoll: usePianoRollSection.getState().section,
  notes: new Set(itemSelection.getState().selected.note),
  clips: new Set(itemSelection.getState().selected.clip),
});

function restore(s: Selections | undefined): void {
  if (!s) return;
  useTimeSelection.getState().setSelection(s.time);
  usePianoRollSection.getState().setSection(s.pianoRoll);
  // Only what still exists after the undo / redo.
  const project = useProjectStore.getState().project;
  if (s.notes && project)
    itemSelection.getState().select(
      "note",
      [...s.notes].filter((id) => project.notes[id]),
      "replace",
    );
  if (s.clips && project)
    itemSelection.getState().select(
      "clip",
      [...s.clips].filter((id) => project.clips[id]),
      "replace",
    );
}

function remember(map: Map<number, Selections>, id: number, s: Selections): void {
  map.delete(id);
  map.set(id, s);
  if (map.size > MAX_STEPS) map.delete(map.keys().next().value!);
}

/** Follow the history: returns the unsubscribe. */
export function bindSelectionHistory(): () => void {
  /** Selections when each step was made (restored by its undo). */
  const before = new Map<number, Selections>();
  /** Selections when each step was undone (restored by its redo). */
  const after = new Map<number, Selections>();
  /** The latest real sections since the last history change (kept when dropped). */
  let held: Selections = { time: null, pianoRoll: null };
  const offTime = useTimeSelection.subscribe((s, p) => {
    if (s.selection !== p.selection && isRange(s.selection)) held = { ...held, time: s.selection };
  });
  const offRoll = usePianoRollSection.subscribe((s, p) => {
    if (s.section !== p.section && isRange(s.section)) held = { ...held, pianoRoll: s.section };
  });
  const offHistory = useProjectStore.subscribe((state, prev) => {
    const h: HistoryState = state.history;
    const p: HistoryState = prev.history;
    if (h === p) return;
    if (p.undo_id !== null && h.redo_id === p.undo_id) {
      remember(after, p.undo_id, snapshot());
      restore(before.get(p.undo_id));
    } else if (p.redo_id !== null && h.undo_id === p.redo_id) {
      restore(after.get(p.redo_id));
    } else if (h.undo_id !== null && h.undo_id !== p.undo_id) {
      const now = snapshot();
      remember(before, h.undo_id, {
        ...now,
        time: isRange(now.time) ? now.time : (held.time ?? now.time),
        pianoRoll: isRange(now.pianoRoll) ? now.pianoRoll : (held.pianoRoll ?? now.pianoRoll),
      });
    }
    // Only real ranges are held.
    const s = snapshot();
    held = { time: isRange(s.time) ? s.time : null, pianoRoll: isRange(s.pianoRoll) ? s.pianoRoll : null };
  });
  return () => {
    offTime();
    offRoll();
    offHistory();
  };
}

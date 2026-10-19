/**
 * The piano roll's notes as text, for screen readers: the note canvases are `aria-hidden`
 * (they draw pixels, not elements), so the notes are listed here (pitch, position, length,
 * velocity) and the note selection is announced. Visually hidden; editing stays on the
 * piano roll's keyboard shortcuts.
 */

import { useMemo } from "react";
import type { Note } from "@/generated";
import { useT } from "@/i18n";
import { formatBarBeat, formatDuration, useSelectedItems, type TempoMap } from "@/timeline";
import { pitchName } from "./geometry";
import { midiVelocity } from "./noteEdits";

export function NoteSummary({ notes, tempo }: { notes: readonly Note[]; tempo: TempoMap }) {
  const t = useT();
  const selected = useSelectedItems("note");
  const sorted = useMemo(() => [...notes].sort((a, b) => a.start - b.start || b.pitch - a.pitch), [notes]);
  const describe = (n: Note) =>
    t("piano_roll.summary.note", {
      pitch: pitchName(n.pitch),
      position: formatBarBeat(n.start, tempo),
      length: formatDuration(n.duration),
      velocity: midiVelocity(n.velocity),
      muted: n.muted ? "true" : "false",
    });
  const picked = sorted.filter((n) => selected.has(n.id));
  return (
    <div className="eth-sr-only" data-testid="piano-roll-note-summary">
      <ul aria-label={t("piano_roll.summary.notes", { count: sorted.length })}>
        {sorted.map((n) => (
          <li key={n.id}>{describe(n)}</li>
        ))}
      </ul>
      <div role="status" aria-live="polite">
        {picked.length === 1
          ? t("piano_roll.summary.selected_one", { note: describe(picked[0]!) })
          : picked.length > 1
            ? t("piano_roll.summary.selected_many", { count: picked.length })
            : ""}
      </div>
    </div>
  );
}

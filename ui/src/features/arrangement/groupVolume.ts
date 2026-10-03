import type { TrackId } from "@/generated";
import { SILENCE_DB } from "@/features/devices/paramScale";
import { dbToFader, faderToDb, MAX_DB } from "@/features/mixer/routing";

/** Volumes (dB) of the tracks a fader drag moves, captured when the drag starts. */
export type VolumeSnapshot = ReadonlyMap<TrackId, number>;

/**
 * New volumes for every track in `start` when the dragged track goes from its start volume to
 * `draggedDb`: each track moves by the same dB amount (the same gain ratio), so the balance
 * between them is kept, clamped to the fader's range. Silent tracks stay silent (a ratio of
 * silence is silence). When the dragged track starts at silence there is no ratio, so every
 * fader moves by the same fader distance instead.
 */
export function groupVolumes(
  start: VolumeSnapshot,
  dragged: TrackId,
  draggedDb: number,
): Map<TrackId, number> {
  const from = start.get(dragged) ?? draggedDb;
  const out = new Map<TrackId, number>();
  if (from <= SILENCE_DB) {
    const delta = dbToFader(draggedDb) - dbToFader(from);
    for (const [id, db] of start)
      out.set(id, faderToDb(clamp01(dbToFader(db) + delta)));
  } else {
    const delta = draggedDb - from;
    for (const [id, db] of start)
      out.set(
        id,
        db <= SILENCE_DB
          ? db
          : Math.min(MAX_DB, Math.max(SILENCE_DB, db + delta)),
      );
  }
  out.set(dragged, draggedDb);
  return out;
}

const clamp01 = (n: number) => Math.min(1, Math.max(0, n));

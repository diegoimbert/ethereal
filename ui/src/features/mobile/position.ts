/**
 * Typed positions and lengths (the design's §6 "Numeric entry"): the clip sheet's fields.
 * Pure. A position is "bar.beat.sixteenth" (1-based, as `formatBarBeat` shows it); a length is
 * "bars.beats.sixteenths" (0-based, as `formatDuration`). Trailing parts may be left out
 * ("3" is bar 3, "0.2" two beats).
 */
import type { Beats } from "@/generated";
import { beatsPerBar, beatUnit, type TempoMap } from "@/timeline";

/** The 1 to 3 numeric parts of `text`, or null. */
function parts(text: string): number[] | null {
  const t = text.trim();
  if (!/^\d+(\.\d+){0,2}$/.test(t)) return null;
  return t.split(".").map(Number);
}

/** A "bar.beat.sixteenth" position (beats), or null when it is not one. */
export function parsePosition(text: string, tempo: TempoMap): Beats | null {
  const p = parts(text);
  if (!p) return null;
  const [bar, beat = 1, six = 1] = p as [number, number?, number?];
  if (bar < 1 || beat < 1 || six < 1) return null;
  const start = tempo.barToBeats(bar);
  const sig = tempo.signatureAt(start);
  const unit = beatUnit(sig);
  if (beat > beatsPerBar(sig) / unit || six > unit / 0.25) return null;
  return start + (beat - 1) * unit + (six - 1) * 0.25;
}

/** A "bars.beats.sixteenths" length (beats), or null when it is not one. */
export function parseLength(text: string, barLength: Beats): Beats | null {
  const p = parts(text);
  if (!p) return null;
  const [bars, beats = 0, six = 0] = p as [number, number?, number?];
  return bars * barLength + beats + six * 0.25;
}

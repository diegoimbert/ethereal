import type { Track } from "@/generated";

/**
 * A volume change alone doesn't re-render the row: the header's fader reads the level
 * from the store itself (`HeaderVolume`), nothing else in the row shows it. A volume drag
 * over many selected tracks otherwise re-rendered every one of their headers and lanes per
 * frame. A patch replaces the whole track (new nested objects too), hence the deep compare.
 */
export function sameTrackExceptVolume(a: Track, b: Track): boolean {
  if (a === b) return true;
  const { mixer: am, ...ar } = a;
  const { mixer: bm, ...br } = b;
  return deepEqual(ar, br) && deepEqual({ ...am, volume: 0 }, { ...bm, volume: 0 });
}

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a);
  if (ka.length !== Object.keys(b).length) return false;
  return ka.every((k) => k in b && deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

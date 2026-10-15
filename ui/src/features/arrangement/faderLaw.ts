/** The track volume / send fader law (track headers, group volume, the inspector). */

import { scaleToNormalized, scaleToPlain, SILENCE_DB } from "@/domain/paramScale";

/** Top of the volume/send fader range (`Decibels` max accepted by the engine). */
export const MAX_DB = 6;

const FADER = { type: "Fader" } as const;

/** Fader position (0..1) → dB, with the protocol's `ParamScale::Fader` law. */
export function faderToDb(n: number, max = MAX_DB): number {
  return scaleToPlain(FADER, SILENCE_DB, max, n);
}

/** dB → fader position (0..1). */
export function dbToFader(db: number, max = MAX_DB): number {
  return scaleToNormalized(FADER, SILENCE_DB, max, db);
}

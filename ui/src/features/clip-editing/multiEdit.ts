/**
 * Editing a field of several audio clips at once (base-150), the Ableton way:
 *
 * - typing or picking a value sets it on every clip (`setAll*`);
 * - a drag, a scrub or an arrow step moves every clip by the same amount, so their
 *   differences are kept (`offset*`, from the values captured when the gesture started).
 *
 * Every builder returns one command for all the clips (an `Edit::Batch` when there are
 * several: one undo step), or null when there is nothing to send.
 */

import type { AudioContent, Clip, ClipId, Command, TextKey, WarpSettings } from "@/generated";
import { cmd } from "@/transport";

export const TRANSPOSE_MAX = 48;
export const GAIN_MIN_DB = -60;
export const GAIN_MAX_DB = 24;

export interface AudioClip {
  clip: Clip;
  audio: AudioContent;
}

/** The audio clips among `clips` (MIDI clips have none of these fields). */
export function audioClips(clips: ReadonlyArray<Clip>): AudioClip[] {
  return clips.flatMap((clip) => (clip.content.type === "Audio" ? [{ clip, audio: clip.content }] : []));
}

/** The common value of `values`, or null when they differ (a "mixed" field). */
export function common<T>(values: ReadonlyArray<T>): T | null {
  if (values.length === 0) return null;
  const first = values[0]!;
  return values.every((v) => v === first) ? first : null;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

function batch(label: string, key: TextKey, commands: Command[]): Command | null {
  if (commands.length === 0) return null;
  if (commands.length === 1) return commands[0]!;
  return cmd("Edit", { type: "Batch", label, key, commands });
}

/** Per-clip base values captured when a drag starts. */
export type Bases = ReadonlyMap<ClipId, number>;

export const transposeBases = (clips: ReadonlyArray<AudioClip>): Bases => new Map(clips.map((c) => [c.clip.id, c.audio.transpose]));
export const gainBases = (clips: ReadonlyArray<AudioClip>): Bases => new Map(clips.map((c) => [c.clip.id, c.audio.gain]));

const transposeCmd = (id: ClipId, semitones: number) => cmd("Clip", { type: "SetTranspose", id, semitones: clamp(semitones, -TRANSPOSE_MAX, TRANSPOSE_MAX) });
const gainCmd = (id: ClipId, gain: number) => cmd("Clip", { type: "SetGain", id, gain: clamp(gain, GAIN_MIN_DB, GAIN_MAX_DB) });

/** Set every clip's transpose to `semitones`. */
export function setAllTranspose(clips: ReadonlyArray<AudioClip>, semitones: number): Command | null {
  return batch(
    "transpose clips", // i18n-ignore
    { id: "undo.clips.transpose", args: {} },
    clips.filter((c) => c.audio.transpose !== semitones).map((c) => transposeCmd(c.clip.id, semitones)),
  );
}

/** Move every clip's transpose by `delta` from its base (clamped per clip). */
export function offsetTranspose(clips: ReadonlyArray<AudioClip>, bases: Bases, delta: number): Command | null {
  return batch(
    "transpose clips", // i18n-ignore
    { id: "undo.clips.transpose", args: {} },
    clips.map((c) => transposeCmd(c.clip.id, (bases.get(c.clip.id) ?? c.audio.transpose) + delta)),
  );
}

/** Set every clip's gain to `gain` dB. */
export function setAllGain(clips: ReadonlyArray<AudioClip>, gain: number): Command | null {
  return batch(
    "set clip gain", // i18n-ignore
    { id: "undo.clip.set_gain", args: {} },
    clips.filter((c) => c.audio.gain !== gain).map((c) => gainCmd(c.clip.id, gain)),
  );
}

/** Move every clip's gain by `delta` dB from its base (clamped per clip). */
export function offsetGain(clips: ReadonlyArray<AudioClip>, bases: Bases, delta: number): Command | null {
  return batch(
    "set clip gain", // i18n-ignore
    { id: "undo.clip.set_gain", args: {} },
    clips.map((c) => gainCmd(c.clip.id, Number(((bases.get(c.clip.id) ?? c.audio.gain) + delta).toFixed(6)))),
  );
}

/** Apply `patch` to every clip's own warp settings (markers and other settings kept). */
export function setAllWarp(clips: ReadonlyArray<AudioClip>, patch: Partial<WarpSettings>): Command | null {
  return batch(
    "set warp", // i18n-ignore
    { id: "undo.warp.set", args: {} },
    clips.map((c) => cmd("Warp", { type: "SetWarp", clip: c.clip.id, warp: { ...c.audio.warp, ...patch } })),
  );
}

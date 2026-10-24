/**
 * Editing a field of several audio clips at once, the Ableton way:
 *
 * - typing or picking a value sets it on every clip (`setAll*`);
 * - a drag, a scrub or an arrow step moves every clip by the same amount, so their
 *   differences are kept (`offset*`, from the values captured when the gesture started).
 *
 * Every builder returns one command for all the clips (an `Edit::Batch` when there are
 * several: one undo step), or null when there is nothing to send.
 */

import type { AudioContent, Clip, ClipId, Command, TextKey, WarpFormants, WarpMode, WarpSettings } from "@/generated";
import type { MessageKey } from "@/i18n";
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

/**
 * A relative edit of one clip's value: `base` (its value when the gesture started) moved by
 * `delta`, clamped to `[lo, hi]` (each clip on its own: one at the edge stops there, the others
 * keep moving). Rounded to 6 decimals so float steps (0.5 dB, ...) stay exact.
 */
export const offsetValue = (base: number, delta: number, lo: number, hi: number): number => clamp(Number((base + delta).toFixed(6)), lo, hi);

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

/**
 * Move every clip's transpose by `delta` from its base (clamped per clip): +2 takes −6 to −4
 * and +3 to +5. Clips already at their new value are left out (a clip pinned at the range's
 * edge sends nothing more while the drag goes on).
 */
export function offsetTranspose(clips: ReadonlyArray<AudioClip>, bases: Bases, delta: number): Command | null {
  return batch(
    "transpose clips", // i18n-ignore
    { id: "undo.clips.transpose", args: {} },
    clips.flatMap((c) => {
      const v = offsetValue(bases.get(c.clip.id) ?? c.audio.transpose, delta, -TRANSPOSE_MAX, TRANSPOSE_MAX);
      return v === c.audio.transpose ? [] : [transposeCmd(c.clip.id, v)];
    }),
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

/** Move every clip's gain by `delta` dB from its base (clamped per clip; unchanged clips left out). */
export function offsetGain(clips: ReadonlyArray<AudioClip>, bases: Bases, delta: number): Command | null {
  return batch(
    "set clip gain", // i18n-ignore
    { id: "undo.clip.set_gain", args: {} },
    clips.flatMap((c) => {
      const v = offsetValue(bases.get(c.clip.id) ?? c.audio.gain, delta, GAIN_MIN_DB, GAIN_MAX_DB);
      return v === c.audio.gain ? [] : [gainCmd(c.clip.id, v)];
    }),
  );
}

/** Complex+'s formant settings when a clip has none (`ether_model::WarpFormants::DEFAULT`). */
export const FORMANTS_DEFAULT: WarpFormants = { amount: 1, shift: 0, envelope: 128 };
export type FormantKey = keyof WarpFormants;
/** Each formant setting's range (`ether_model::WarpFormants`). */
export const FORMANT_RANGE: Readonly<Record<FormantKey, readonly [number, number]>> = {
  amount: [0, 1],
  shift: [-24, 24],
  envelope: [8, 256],
};

export const formantsOf = (c: AudioClip): WarpFormants => c.audio.warp.formants ?? FORMANTS_DEFAULT;
export const formantBases = (clips: ReadonlyArray<AudioClip>, key: FormantKey): Bases => new Map(clips.map((c) => [c.clip.id, formantsOf(c)[key]]));

const formantCmd = (c: AudioClip, key: FormantKey, value: number) =>
  cmd("Warp", { type: "SetWarp", clip: c.clip.id, warp: { ...c.audio.warp, formants: { ...formantsOf(c), [key]: value } } });

/** Set every clip's formant setting `key` to `value` (clamped to its range). */
export function setAllFormant(clips: ReadonlyArray<AudioClip>, key: FormantKey, value: number): Command | null {
  const [lo, hi] = FORMANT_RANGE[key];
  const v = clamp(value, lo, hi);
  return batch(
    "set warp", // i18n-ignore
    { id: "undo.warp.set", args: {} },
    clips.filter((c) => formantsOf(c)[key] !== v).map((c) => formantCmd(c, key, v)),
  );
}

/** Move every clip's formant setting `key` by `delta` from its base (clamped per clip; unchanged clips left out). */
export function offsetFormant(clips: ReadonlyArray<AudioClip>, key: FormantKey, bases: Bases, delta: number): Command | null {
  const [lo, hi] = FORMANT_RANGE[key];
  return batch(
    "set warp", // i18n-ignore
    { id: "undo.warp.set", args: {} },
    clips.flatMap((c) => {
      const now = formantsOf(c)[key];
      const v = offsetValue(bases.get(c.clip.id) ?? now, delta, lo, hi);
      return v === now ? [] : [formantCmd(c, key, v)];
    }),
  );
}

/**
 * The one warp/pitch mode list of the UI: "none" (warp off: the clip plays at the source's
 * native speed) or a warp mode. The model keeps `enabled` and `mode` apart (a clip switched
 * to none keeps its mode for when it is warped again), so an unwarped clip reads as none.
 */
export type WarpChoice = "None" | WarpMode;

export const WARP_CHOICES: ReadonlyArray<{ value: WarpChoice; label: MessageKey }> = [
  { value: "None", label: "warp.mode.none" },
  { value: "Repitch", label: "warp.mode.repitch" },
  { value: "Complex", label: "warp.mode.complex" },
  { value: "ComplexPlus", label: "warp.mode.complex_plus" },
];

/** A clip's choice in the mode list. */
export const warpChoiceOf = (warp: WarpSettings): WarpChoice => (warp.enabled ? warp.mode : "None");

/** The warp settings `patch` that picking `choice` sends (none keeps the mode). */
export const warpChoicePatch = (choice: WarpChoice): Partial<WarpSettings> => (choice === "None" ? { enabled: false } : { enabled: true, mode: choice });

/** Pick `choice` for every clip (one undo step; clips already on it are left out). */
export function setAllWarpChoice(clips: ReadonlyArray<AudioClip>, choice: WarpChoice): Command | null {
  return setAllWarp(
    clips.filter((c) => warpChoiceOf(c.audio.warp) !== choice),
    warpChoicePatch(choice),
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

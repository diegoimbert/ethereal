import type { MusicalScale, ScaleKind, TrackScale } from "@/generated";

export const ROOT_NOTES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;
export const CHROMATIC_SCALE: MusicalScale = { root: 0, kind: "Chromatic" };
/** Semitone intervals above the tonic. Melodic minor uses its ascending form. */
export const SCALES = {
  Chromatic: { label: "Chromatic / None", intervals: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] },
  Major: { label: "Major", intervals: [0, 2, 4, 5, 7, 9, 11] },
  Minor: { label: "Minor", intervals: [0, 2, 3, 5, 7, 8, 10] },
  HarmonicMinor: { label: "Harmonic Minor", intervals: [0, 2, 3, 5, 7, 8, 11] },
  MelodicMinor: { label: "Melodic Minor", intervals: [0, 2, 3, 5, 7, 9, 11] },
  MajorPentatonic: { label: "Major Pentatonic", intervals: [0, 2, 4, 7, 9] },
  MinorPentatonic: { label: "Minor Pentatonic", intervals: [0, 3, 5, 7, 10] },
  Blues: { label: "Blues", intervals: [0, 3, 5, 6, 7, 10] },
  Dorian: { label: "Dorian", intervals: [0, 2, 3, 5, 7, 9, 10] },
  Phrygian: { label: "Phrygian", intervals: [0, 1, 3, 5, 7, 8, 10] },
  Lydian: { label: "Lydian", intervals: [0, 2, 4, 6, 7, 9, 11] },
  Mixolydian: { label: "Mixolydian", intervals: [0, 2, 4, 5, 7, 9, 10] },
  Locrian: { label: "Locrian", intervals: [0, 1, 3, 5, 6, 8, 10] },
  WholeTone: { label: "Whole Tone", intervals: [0, 2, 4, 6, 8, 10] },
} satisfies Record<ScaleKind, { label: string; intervals: readonly number[] }>;

export const SCALE_KINDS = Object.keys(SCALES) as ScaleKind[];
export const getPitchClass = (midiNote: number): number => ((midiNote % 12) + 12) % 12;
/** Pitch classes in scale-degree order, wrapping at B. */
export function getScaleNotes(root: number, scale: ScaleKind): number[] {
  return SCALES[scale].intervals.map((interval) => getPitchClass(root + interval));
}
export function isNoteInScale(midiNote: number, root: number, scale: ScaleKind): boolean {
  return (SCALES[scale].intervals as readonly number[]).includes(getPitchClass(midiNote - root));
}
export function resolveScale(project: MusicalScale, track?: TrackScale): MusicalScale {
  if (track?.type === "Custom") return track.scale;
  return track?.type === "Chromatic" ? CHROMATIC_SCALE : project;
}
/** Short name of a scale, e.g. "C Minor" ("Scale" when chromatic, i.e. no scale). */
export function scaleLabel(scale: MusicalScale): string {
  return scale.kind === "Chromatic" ? "Scale" : `${ROOT_NOTES[scale.root]} ${SCALES[scale.kind].label}`;
}
export function scaleTone(pitch: number, scale: MusicalScale, highlight: boolean): "root" | "in" | "out" | undefined {
  if (!highlight || scale.kind === "Chromatic") return undefined;
  if (getPitchClass(pitch) === scale.root) return "root";
  return isNoteInScale(pitch, scale.root, scale.kind) ? "in" : "out";
}

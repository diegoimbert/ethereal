import type { MusicalScale, ScaleKind, TrackScale } from "@/generated";
import { t, type MessageKey } from "@/i18n";

export const ROOT_NOTES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;
export const CHROMATIC_SCALE: MusicalScale = { root: 0, kind: "Chromatic" };
/** Semitone intervals above the tonic. Melodic minor uses its ascending form. */
export const SCALES = {
  Chromatic: { label: "scale.kind.chromatic", intervals: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] },
  Major: { label: "scale.kind.major", intervals: [0, 2, 4, 5, 7, 9, 11] },
  Minor: { label: "scale.kind.minor", intervals: [0, 2, 3, 5, 7, 8, 10] },
  HarmonicMinor: { label: "scale.kind.harmonic_minor", intervals: [0, 2, 3, 5, 7, 8, 11] },
  MelodicMinor: { label: "scale.kind.melodic_minor", intervals: [0, 2, 3, 5, 7, 9, 11] },
  MajorPentatonic: { label: "scale.kind.major_pentatonic", intervals: [0, 2, 4, 7, 9] },
  MinorPentatonic: { label: "scale.kind.minor_pentatonic", intervals: [0, 3, 5, 7, 10] },
  Blues: { label: "scale.kind.blues", intervals: [0, 3, 5, 6, 7, 10] },
  Dorian: { label: "scale.kind.dorian", intervals: [0, 2, 3, 5, 7, 9, 10] },
  Phrygian: { label: "scale.kind.phrygian", intervals: [0, 1, 3, 5, 7, 8, 10] },
  Lydian: { label: "scale.kind.lydian", intervals: [0, 2, 4, 6, 7, 9, 11] },
  Mixolydian: { label: "scale.kind.mixolydian", intervals: [0, 2, 4, 5, 7, 9, 10] },
  Locrian: { label: "scale.kind.locrian", intervals: [0, 1, 3, 5, 6, 8, 10] },
  WholeTone: { label: "scale.kind.whole_tone", intervals: [0, 2, 4, 6, 8, 10] },
} satisfies Record<ScaleKind, { label: MessageKey; intervals: readonly number[] }>;

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
/** A scale type's name in the current language ("harmonic minor"). */
export function scaleKindName(kind: ScaleKind): string {
  return t(SCALES[kind].label);
}
/** Short name of a scale, e.g. "C minor" ("scale" when chromatic, i.e. no scale). Note names stay as they are. */
export function scaleLabel(scale: MusicalScale): string {
  return scale.kind === "Chromatic" ? t("scale.none") : t("scale.name", { root: ROOT_NOTES[scale.root] ?? "", scale: scaleKindName(scale.kind) });
}
export function scaleTone(pitch: number, scale: MusicalScale, highlight: boolean): "root" | "in" | "out" | undefined {
  if (!highlight || scale.kind === "Chromatic") return undefined;
  if (getPitchClass(pitch) === scale.root) return "root";
  return isNoteInScale(pitch, scale.root, scale.kind) ? "in" : "out";
}

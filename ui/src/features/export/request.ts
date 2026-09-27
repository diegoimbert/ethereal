/** Export form state → `ExportRequest` (pure; unit-tested). */

import type {
  AudioContainer,
  BeatRange,
  BitDepth,
  ExportRange,
  ExportRequest,
  Project,
  Track,
  TrackId,
} from "@/generated";
import { tracksOrdered } from "@/state";

export type RangeChoice = "project" | "loop" | "selection";
export type RateChoice = "engine" | "44100" | "48000" | "88200" | "96000";

export interface ExportForm {
  range: RangeChoice;
  container: AudioContainer;
  bitDepth: BitDepth;
  rate: RateChoice;
  normalize: boolean;
  /** Seconds after the range end (reverb/delay tails), 0..=60. */
  tail: number;
  stems: boolean;
  /** Stem tracks (when `stems`). */
  tracks: ReadonlyArray<TrackId>;
  /** File name without extension; empty = the project name. */
  name: string;
}

export const DEFAULT_FORM: ExportForm = {
  range: "project",
  container: "Wav",
  bitDepth: "Int24",
  rate: "engine",
  normalize: false,
  tail: 0,
  stems: false,
  tracks: [],
  name: "",
};

export const MAX_TAIL_SECONDS = 60;

export const RANGE_OPTIONS: ReadonlyArray<{
  value: RangeChoice;
  label: string;
}> = [
  { value: "project", label: "Whole project" },
  { value: "loop", label: "Loop region" },
  { value: "selection", label: "Time selection" },
];

export const CONTAINER_OPTIONS: ReadonlyArray<{
  value: AudioContainer;
  label: string;
}> = [
  { value: "Wav", label: "WAV" },
  { value: "Flac", label: "FLAC" },
];

export const BIT_DEPTH_OPTIONS: ReadonlyArray<{
  value: BitDepth;
  label: string;
}> = [
  { value: "Int16", label: "16-bit (dithered)" },
  { value: "Int24", label: "24-bit" },
  { value: "Float32", label: "32-bit float" },
];

export const RATE_OPTIONS: ReadonlyArray<{ value: RateChoice; label: string }> =
  [
    { value: "engine", label: "Engine rate" },
    { value: "44100", label: "44.1 kHz" },
    { value: "48000", label: "48 kHz" },
    { value: "88200", label: "88.2 kHz" },
    { value: "96000", label: "96 kHz" },
  ];

/** FLAC has no float samples. */
export function depthAllowed(
  container: AudioContainer,
  depth: BitDepth,
): boolean {
  return !(container === "Flac" && depth === "Float32");
}

/** Tracks that can be exported as stems (everything but master), in display order. */
export function stemCandidates(project: Project): Track[] {
  return tracksOrdered(project).filter((t) => t.kind !== "Master");
}

/** Default stem selection: the audio and MIDI tracks. */
export function defaultStemTracks(project: Project): TrackId[] {
  return stemCandidates(project)
    .filter((t) => t.kind === "Audio" || t.kind === "Midi")
    .map((t) => t.id);
}

export function rangeOf(
  choice: RangeChoice,
  selection: BeatRange | null,
): ExportRange | null {
  switch (choice) {
    case "project":
      return { type: "Project" };
    case "loop":
      return { type: "Loop" };
    case "selection":
      return selection && selection.end > selection.start
        ? { type: "Custom", start: selection.start, end: selection.end }
        : null;
  }
}

/** Why the form can't be exported (`null` = ready). */
export function formProblem(
  form: ExportForm,
  selection: BeatRange | null,
): string | null {
  if (!rangeOf(form.range, selection))
    return "Select a time range in the arrangement first";
  if (!depthAllowed(form.container, form.bitDepth))
    return "FLAC supports 16 or 24 bits only";
  if (form.stems && form.tracks.length === 0)
    return "Choose at least one track for stems";
  return null;
}

export function buildRequest(
  form: ExportForm,
  selection: BeatRange | null,
): ExportRequest {
  const range = rangeOf(form.range, selection);
  if (!range) throw new Error("no range");
  const name = form.name.trim();
  return {
    range,
    format: {
      container: form.container,
      bit_depth: form.bitDepth,
      sample_rate: form.rate === "engine" ? null : Number(form.rate),
    },
    mode: form.stems
      ? { type: "Stems", tracks: [...form.tracks] }
      : { type: "Mix" },
    normalize: form.normalize,
    tail_seconds: Math.min(MAX_TAIL_SECONDS, Math.max(0, form.tail)),
    name: name === "" ? null : name,
  };
}

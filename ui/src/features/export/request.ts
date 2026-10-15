/** Export form state → `ExportRequest` (pure; unit-tested). */

import type { AudioContainer, BeatRange, BitDepth, ExportRange, ExportRequest, Project, Track, TrackId } from "@/generated";
import { tracksOrdered } from "@/state";
import type { MessageKey } from "@/i18n";

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

// The options' labels are message keys (the dialog translates them).
export const RANGE_OPTIONS: ReadonlyArray<{
  value: RangeChoice;
  label: MessageKey;
}> = [
  { value: "project", label: "export_audio.range.project" },
  { value: "loop", label: "export_audio.range.loop" },
  { value: "selection", label: "export_audio.range.selection" },
];

export const CONTAINER_OPTIONS: ReadonlyArray<{
  value: AudioContainer;
  label: MessageKey;
}> = [
  { value: "Wav", label: "export_audio.format.wav" },
  { value: "Flac", label: "export_audio.format.flac" },
  { value: "Mp3", label: "export_audio.format.mp3" },
];

export const BIT_DEPTH_OPTIONS: ReadonlyArray<{
  value: BitDepth;
  label: MessageKey;
}> = [
  { value: "Int16", label: "export_audio.depth.int16" },
  { value: "Int24", label: "export_audio.depth.int24" },
  { value: "Float32", label: "export_audio.depth.float32" },
];

export const RATE_OPTIONS: ReadonlyArray<{ value: RateChoice; label: MessageKey }> = [
  { value: "engine", label: "export_audio.rate.engine" },
  { value: "44100", label: "export_audio.rate.44100" },
  { value: "48000", label: "export_audio.rate.48000" },
  { value: "88200", label: "export_audio.rate.88200" },
  { value: "96000", label: "export_audio.rate.96000" },
];

/** MP3 is lossy: it has no bit depth (the dialog disables the choice). */
export const hasBitDepth = (container: AudioContainer): boolean => container !== "Mp3";

/** MP3 encodes 32, 44.1 and 48 kHz only. */
export function rateAllowed(container: AudioContainer, rate: RateChoice): boolean {
  return container !== "Mp3" || rate === "engine" || rate === "44100" || rate === "48000";
}

/** FLAC has no float samples. */
export function depthAllowed(container: AudioContainer, depth: BitDepth): boolean {
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

export function rangeOf(choice: RangeChoice, selection: BeatRange | null): ExportRange | null {
  switch (choice) {
    case "project":
      return { type: "Project" };
    case "loop":
      return { type: "Loop" };
    case "selection":
      return selection && selection.end > selection.start ? { type: "Custom", start: selection.start, end: selection.end } : null;
  }
}

/** Why the form can't be exported, as a message key (`null` = ready). */
export function formProblem(form: ExportForm, selection: BeatRange | null): MessageKey | null {
  if (!rangeOf(form.range, selection)) return "export_audio.problem.no_selection";
  if (!depthAllowed(form.container, form.bitDepth)) return "export_audio.problem.flac_depth";
  if (!rateAllowed(form.container, form.rate)) return "export_audio.problem.mp3_rate";
  if (form.stems && form.tracks.length === 0) return "export_audio.problem.no_stems";
  return null;
}

export function buildRequest(form: ExportForm, selection: BeatRange | null): ExportRequest {
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
    mode: form.stems ? { type: "Stems", tracks: [...form.tracks] } : { type: "Mix" },
    normalize: form.normalize,
    tail_seconds: Math.min(MAX_TAIL_SECONDS, Math.max(0, form.tail)),
    name: name === "" ? null : name,
  };
}

/** Stem semantics, shown under the stems toggle (see `ether-controller/src/export`). */
export const STEMS_HELP: MessageKey = "export_audio.stems_help";

/** Above this, the web build warns that the files are held in memory. */
export const LARGE_EXPORT_BYTES = 200e6;

/** Beats covered by the form's range (0 when unknown). */
function rangeBeats(form: ExportForm, project: Project, selection: BeatRange | null): number {
  const range = rangeOf(form.range, selection);
  if (!range) return 0;
  switch (range.type) {
    case "Custom":
      return range.end - range.start;
    case "Loop":
      return project.settings.loop_region.end - project.settings.loop_region.start;
    case "Project": {
      let end = 0;
      for (const c of Object.values(project.clips)) end = Math.max(end, c.start + c.length);
      for (const pt of Object.values(project.automation_points)) {
        if (project.automation_lanes[pt.lane]?.owner.type === "Track") end = Math.max(end, pt.time);
      }
      return end;
    }
  }
}

/**
 * Rough size of the export in bytes (all files): the range at the first tempo, the
 * requested rate (48 kHz for the engine rate), stereo; FLAC counted at ~60%, MP3 at
 * 320 kbps.
 */
export function estimateBytes(form: ExportForm, project: Project, selection: BeatRange | null): number {
  const first = Object.values(project.tempo_points).sort((a, b) => a.time - b.time)[0];
  const bpm = first?.bpm ?? 120;
  const seconds = (rangeBeats(form, project, selection) * 60) / bpm + form.tail;
  const rate = form.rate === "engine" ? 48000 : Number(form.rate);
  const bytesPerSample = form.bitDepth === "Int16" ? 2 : form.bitDepth === "Int24" ? 3 : 4;
  const files = form.stems ? form.tracks.length : 1;
  if (form.container === "Mp3") return (seconds * 320_000 * files) / 8;
  const ratio = form.container === "Flac" ? 0.6 : 1;
  return seconds * rate * 2 * bytesPerSample * files * ratio;
}

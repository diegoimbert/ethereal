/**
 * Tempo map editing helpers (pure): commands, sorting, snapping and the tempo lane's
 * geometry. The rules mirror the controller (`crates/ether-controller/src/tempo/mod.rs`):
 * BPM 20..=999, the points at beat 0 can't move or be removed, two points never share a
 * position. A time-signature change may sit anywhere (CONTRACTS.md §11.3): one inside a
 * bar ends that bar early and starts bar 1 of the new signature there.
 */

import type {
  Beats,
  Command,
  MetronomeSound,
  Project,
  TempoCurve,
  TempoPoint,
  TimeSignature,
  TimeSignaturePoint,
} from "@/generated";
import { BEATS_EPSILON, beatsApproxEq } from "@/state/beats";
import { cmd, newId } from "@/transport";
import { snapToGrid, type GridStep } from "@/timeline/grid";
import { beatUnit, TempoMap } from "@/timeline/tempoMap";

export const MIN_BPM = 20;
export const MAX_BPM = 999;
export const MIN_METRONOME_DB = -144;
export const MAX_METRONOME_DB = 6;

export const METRONOME_SOUNDS: ReadonlyArray<{ value: MetronomeSound; label: string }> = [
  { value: "Classic", label: "Classic" },
  { value: "Wood", label: "Wood" },
  { value: "Beep", label: "Beep" },
];

export const DENOMINATORS = [1, 2, 4, 8, 16, 32] as const;

export const clampBpm = (bpm: number): number => Math.min(MAX_BPM, Math.max(MIN_BPM, bpm));

export const isAtZero = (p: { time: Beats }): boolean => beatsApproxEq(p.time, 0);

const byTime = <T extends { time: number; id: string }>(a: T, b: T) => a.time - b.time || (a.id < b.id ? -1 : 1);

export function sortedTempoPoints(p: Pick<Project, "tempo_points">): TempoPoint[] {
  return Object.values(p.tempo_points).sort(byTime);
}

export function sortedSignatures(p: Pick<Project, "time_signatures">): TimeSignaturePoint[] {
  return Object.values(p.time_signatures).sort(byTime);
}

export const formatSignature = (s: TimeSignature): string => `${s.numerator}/${s.denominator}`;

export const formatBpm = (bpm: number): string => (Number.isInteger(bpm) ? String(bpm) : bpm.toFixed(2));

// ─── Commands ───────────────────────────────────────────────────────────────────────────

export function addTempoPointCommand(time: Beats, bpm: number, curve: TempoCurve = "Step", id: string = newId()): Command {
  return cmd("Tempo", { type: "AddTempoPoint", id, time: Math.max(0, time), bpm: clampBpm(bpm), curve });
}

export function editTempoPointCommand(
  id: string,
  change: { time?: Beats; bpm?: number; curve?: TempoCurve },
): Command {
  return cmd("Tempo", {
    type: "EditTempoPoint",
    id,
    time: change.time ?? null,
    bpm: change.bpm !== undefined ? clampBpm(change.bpm) : null,
    curve: change.curve ?? null,
  });
}

export function removeTempoPointsCommand(ids: string[]): Command {
  return cmd("Tempo", { type: "RemoveTempoPoints", ids });
}

export function addSignatureCommand(time: Beats, signature: TimeSignature, id: string = newId()): Command {
  return cmd("Tempo", { type: "AddTimeSignature", id, time, signature });
}

export function editSignatureCommand(id: string, change: { time?: Beats; signature?: TimeSignature }): Command {
  return cmd("Tempo", { type: "EditTimeSignature", id, time: change.time ?? null, signature: change.signature ?? null });
}

export function removeSignaturesCommand(ids: string[]): Command {
  return cmd("Tempo", { type: "RemoveTimeSignatures", ids });
}

export function metronomeSettingsCommand(change: { volume?: number; accent?: boolean; sound?: MetronomeSound }): Command {
  return cmd("Tempo", {
    type: "SetMetronomeSettings",
    volume: change.volume ?? null,
    accent: change.accent ?? null,
    sound: change.sound ?? null,
  });
}

// ─── Placement rules ────────────────────────────────────────────────────────────────────

/** A tempo point other than `except` sits at `time`. */
export function tempoTimeTaken(points: ReadonlyArray<TempoPoint>, time: Beats, except?: string): boolean {
  return points.some((p) => p.id !== except && beatsApproxEq(p.time, time));
}

/**
 * Snap step for placing or moving a time-signature change: the current grid step when it
 * is finer than a beat of `sig` (the signature in effect there), else one beat of `sig`
 * (bars, half notes or an adaptive coarse grid still allow any beat). `null` (grid off)
 * stays off.
 */
export function signatureSnapStep(step: GridStep | null | undefined, sig: TimeSignature): GridStep | null {
  if (step === null) return null;
  const unit = beatUnit(sig);
  if (step?.kind === "beats" && step.beats > 0 && step.beats < unit - BEATS_EPSILON) return step;
  return { kind: "beats", beats: unit };
}

export interface SignatureSnapOptions {
  /** The change being moved (ignored for the grid and the occupied check). */
  except?: string;
  /** The view's resolved grid step (`undefined`: one beat; `null`: grid off). */
  step?: GridStep | null;
  /** Alt held: no snapping. */
  free?: boolean;
}

/**
 * Position for a time-signature change near `raw` (anywhere on the grid, CONTRACTS.md
 * §11.3): snapped to `signatureSnapStep` in the bars of the map without the moved change
 * (`except`), unless `free`. `null` at or before beat 0, or on another change.
 */
export function snapSignatureTime(
  sigs: ReadonlyArray<TimeSignaturePoint>,
  raw: Beats,
  opts: SignatureSnapOptions = {},
): Beats | null {
  const others = sigs.filter((s) => s.id !== opts.except);
  if (others.length === 0 || !Number.isFinite(raw)) return null;
  let t = raw;
  if (!opts.free) {
    const map = new TempoMap([], others);
    t = snapToGrid(raw, signatureSnapStep(opts.step, map.signatureAt(raw)), map);
  }
  if (t <= BEATS_EPSILON) return null;
  if (others.some((s) => beatsApproxEq(s.time, t))) return null;
  return t;
}

// ─── Tempo lane geometry ────────────────────────────────────────────────────────────────

export interface BpmRange {
  min: number;
  max: number;
}

/** Display range of the tempo lane: the points' BPMs with headroom, at least 40 BPM wide. */
export function bpmRange(points: ReadonlyArray<TempoPoint>): BpmRange {
  const bpms = points.length ? points.map((p) => p.bpm) : [120];
  let min = Math.min(...bpms);
  let max = Math.max(...bpms);
  const pad = Math.max(10, (max - min) * 0.15);
  min -= pad;
  max += pad;
  if (max - min < 40) {
    const mid = (min + max) / 2;
    min = mid - 20;
    max = mid + 20;
  }
  return { min: Math.max(MIN_BPM - 5, Math.floor(min)), max: Math.min(MAX_BPM + 5, Math.ceil(max)) };
}

export interface LaneGeom {
  height: number;
  /** Vertical padding (px) above the max and below the min. */
  pad: number;
  range: BpmRange;
}

export function bpmToY(bpm: number, g: LaneGeom): number {
  const usable = Math.max(1, g.height - 2 * g.pad);
  return g.pad + (1 - (bpm - g.range.min) / (g.range.max - g.range.min)) * usable;
}

export function yToBpm(y: number, g: LaneGeom): number {
  const usable = Math.max(1, g.height - 2 * g.pad);
  return g.range.min + (1 - (y - g.pad) / usable) * (g.range.max - g.range.min);
}

/**
 * SVG path of the tempo curve: steps hold until the next point, ramps go straight to it;
 * the last tempo holds to `endX`.
 */
export function tempoPath(points: ReadonlyArray<TempoPoint>, toX: (b: Beats) => number, g: LaneGeom, endX: number): string {
  if (points.length === 0) return "";
  const parts: string[] = [];
  const first = points[0]!;
  parts.push(`M${toX(Math.min(0, first.time)).toFixed(1)},${bpmToY(first.bpm, g).toFixed(1)}`);
  points.forEach((p, i) => {
    const x = toX(p.time);
    const y = bpmToY(p.bpm, g);
    parts.push(`L${x.toFixed(1)},${y.toFixed(1)}`);
    const next = points[i + 1];
    if (!next) {
      parts.push(`L${Math.max(x, endX).toFixed(1)},${y.toFixed(1)}`);
    } else if (p.curve === "Linear") {
      parts.push(`L${toX(next.time).toFixed(1)},${bpmToY(next.bpm, g).toFixed(1)}`);
    } else {
      parts.push(`L${toX(next.time).toFixed(1)},${y.toFixed(1)}`);
    }
  });
  return parts.join(" ");
}

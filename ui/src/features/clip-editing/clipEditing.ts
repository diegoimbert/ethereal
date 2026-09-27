/**
 * Clip editing helpers (pure / command builders): fade geometry for the clip overlay,
 * fade-drag math, and the clip context-menu entries (Reverse, Crossfade, fade curves).
 */

import type { AudioContent, Clip, Command, FadeCurve } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, nextGestureId, type EngineTransport } from "@/transport";
import { fadeGain } from "./fades";

const EPS = 1e-9;
/** Points per fade curve in the overlay path. */
const CURVE_POINTS = 24;

export const audioOf = (clip: Clip): AudioContent | null => (clip.content.type === "Audio" ? clip.content : null);

/**
 * Fade shapes in a `[0, width] × [0, 1]` box (y down: 0 = full level at the top, 1 =
 * silence): `line` is the gain curve, `shade` the region above it (what the fade removes).
 * The fade-out mirrors the fade-in law in time (`fadeGain(curve, 1 - u)`).
 */
export function fadePaths(
  side: "in" | "out",
  fadePx: number,
  width: number,
  curve: FadeCurve,
): { line: string; shade: string } | null {
  if (!(fadePx > 0.5)) return null;
  const f = Math.min(fadePx, width);
  const pts: string[] = [];
  for (let i = 0; i <= CURVE_POINTS; i++) {
    const u = i / CURVE_POINTS;
    const x = side === "in" ? u * f : width - f + u * f;
    const g = fadeGain(curve, side === "in" ? u : 1 - u);
    pts.push(`${x.toFixed(2)},${(1 - g).toFixed(4)}`);
  }
  const line = `M${pts.join("L")}`;
  const shade = side === "in" ? `M0,0${"L" + pts.join("L")}Z` : `M${width},0L${pts.join("L")}Z`;
  return { line, shade };
}

/** Fade length (beats) after dragging a fade handle by `dxPx`; clamped so fades don't cross. */
export function dragFadeLength(
  side: "in" | "out",
  start: number,
  dxPx: number,
  pxPerBeat: number,
  length: number,
  other: number,
): number {
  const d = dxPx / Math.max(EPS, pxPerBeat);
  const v = side === "in" ? start + d : start - d;
  return Math.max(0, Math.min(v, length - other));
}

/** Tension (-1..=1) of a `Curve` fade whose midpoint gain is `g` (0 < g < 1). */
export function tensionForMidGain(g: number): number {
  const c = Math.min(1 - 1e-6, Math.max(1e-6, g));
  // g = 0.5^(4^t)  ⇒  t = log4(ln g / ln 0.5)
  const t = Math.log(Math.log(c) / Math.log(0.5)) / Math.log(4);
  return Math.max(-1, Math.min(1, t));
}

/** Curve after dragging the fade's curve handle vertically by `dy` (fraction of the height, down > 0). */
export function dragCurve(start: FadeCurve, dy: number): FadeCurve {
  const g0 = fadeGain(start, 0.5);
  const t = tensionForMidGain(g0) + dy * 2;
  return { type: "Curve", tension: Math.round(Math.max(-1, Math.min(1, t)) * 100) / 100 };
}

/** Selected audio clips (in start order), always including `clip`. */
export function targetAudioClips(clip: Clip): Clip[] {
  const project = useProjectStore.getState().project;
  if (!project) return [];
  const ids = new Set<string>(itemSelection.getState().selected.clip);
  ids.add(clip.id);
  return [...ids]
    .map((id) => project.clips[id])
    .filter((c): c is Clip => !!c && c.content.type === "Audio")
    .sort((a, b) => a.start - b.start);
}

/**
 * The neighbour `clip` can crossfade with on its track: the next (or previous) audio clip
 * that starts where this one ends or inside it (`ClipCommand::Crossfade` precondition).
 */
export function crossfadeNeighbour(clip: Clip, dir: "next" | "previous"): Clip | null {
  const project = useProjectStore.getState().project;
  if (!project || clip.content.type !== "Audio") return null;
  let best: Clip | null = null;
  for (const o of Object.values(project.clips)) {
    if (o.id === clip.id || o.track !== clip.track || o.content.type !== "Audio") continue;
    const [a, b] = dir === "next" ? [clip, o] : [o, clip];
    const aEnd = a.start + a.length;
    const ok = a.start < b.start - EPS && aEnd >= b.start - 1e-6 && aEnd < b.start + b.length + EPS;
    if (!ok) continue;
    if (!best || (dir === "next" ? o.start < best.start : o.start > best.start)) best = o;
  }
  return best;
}

/** Default crossfade length (beats): half a beat, at most half of either clip. */
export function defaultCrossfadeLength(a: Clip, b: Clip): number {
  return Math.min(0.5, a.length / 2, b.length / 2);
}

export function crossfadeCommand(first: Clip, second: Clip, curve: FadeCurve = { type: "EqualPower" }): Command {
  return cmd("Clip", { type: "Crossfade", first: first.id, second: second.id, length: defaultCrossfadeLength(first, second), curve });
}

function batch(label: string, commands: Command[]): Command | null {
  if (commands.length === 0) return null;
  if (commands.length === 1) return commands[0]!;
  return cmd("Edit", { type: "Batch", label, commands });
}

export function reverseCommand(clips: ReadonlyArray<Clip>, reversed: boolean): Command | null {
  return batch(
    reversed ? "Reverse" : "Unreverse",
    clips.map((c) => cmd("Clip", { type: "SetReversed", id: c.id, reversed })),
  );
}

export function fadeCurvesCommand(clips: ReadonlyArray<Clip>, curve: FadeCurve): Command | null {
  return batch(
    "Set Fade Curves",
    clips.map((c) => cmd("Clip", { type: "SetFadeCurves", id: c.id, fade_in: curve, fade_out: curve })),
  );
}

/** Send one edit as its own undo step. */
export async function sendEdit(transport: EngineTransport, command: Command | null): Promise<void> {
  if (!command) return;
  const gesture = nextGestureId();
  try {
    await transport.send(command, { gesture });
  } catch (err) {
    console.warn("[ethereal] clip edit failed", err);
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

/** Context-menu entries for audio clips (Reverse, Crossfade, fade curves). */
export function clipEditingEntries(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  if (clip.content.type !== "Audio") return [];
  const clips = targetAudioClips(clip);
  const allReversed = clips.length > 0 && clips.every((c) => audioOf(c)?.reversed);
  const equalPower = clips.every((c) => audioOf(c)?.fade_in_curve.type === "EqualPower" && audioOf(c)?.fade_out_curve.type === "EqualPower");
  const next = crossfadeNeighbour(clip, "next");
  const prev = crossfadeNeighbour(clip, "previous");
  const entries: ContextMenuEntry[] = [
    { label: allReversed ? "Unreverse" : "Reverse", onSelect: () => void sendEdit(transport, reverseCommand(clips, !allReversed)) },
    {
      label: equalPower ? "Linear Fades" : "Equal-Power Fades",
      onSelect: () => void sendEdit(transport, fadeCurvesCommand(clips, { type: equalPower ? "Linear" : "EqualPower" })),
    },
  ];
  if (prev) entries.push({ label: "Crossfade with Previous Clip", onSelect: () => void sendEdit(transport, crossfadeCommand(prev, clip)) });
  if (next) entries.push({ label: "Crossfade with Next Clip", onSelect: () => void sendEdit(transport, crossfadeCommand(clip, next)) });
  return entries;
}

/**
 * Insert the clip-editing entries into the arrangement's clip menu, as their own group
 * before the trailing "Delete" group (or at the end).
 */
export function withClipEditingEntries(
  entries: ReadonlyArray<ContextMenuEntry>,
  transport: EngineTransport,
  clip: Clip,
): ContextMenuEntry[] {
  const extra = clipEditingEntries(transport, clip);
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return [...entries, "separator", ...extra];
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}

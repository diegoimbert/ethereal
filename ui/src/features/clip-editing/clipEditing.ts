/**
 * Clip editing helpers (pure / command builders): fade-drag math (the arranger's fade
 * layer, arrangement/gl/layers/clipFades.ts), and the clip context-menu entries (Reverse, Crossfade, fade curves).
 */

import type { AudioContent, Clip, Command, FadeCurve, Project, TextKey } from "@/generated";
import { t } from "@/i18n";
import { shortcutLabel } from "@/features/keymap";
import type { ContextMenuEntry } from "@/kit";
import { clipsOfTrack, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, nextGestureId, type EngineTransport } from "@/transport";
import { fadeGain } from "./fades";

const EPS = 1e-9;

export const audioOf = (clip: Clip): AudioContent | null => (clip.content.type === "Audio" ? clip.content : null);

/** Fade length (beats) after dragging a fade handle by `dxPx`; clamped so fades don't cross. */
export function dragFadeLength(side: "in" | "out", start: number, dxPx: number, pxPerBeat: number, length: number, other: number): number {
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
export function targetClips(clip: Clip): Clip[] {
  const project = useProjectStore.getState().project;
  if (!project) return [];
  const ids = new Set<string>(itemSelection.getState().selected.clip);
  ids.add(clip.id);
  return [...ids]
    .map((id) => project.clips[id])
    .filter((c): c is Clip => !!c)
    .sort((a, b) => a.start - b.start);
}

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

/**
 * `true` if `x` and `y` (same track) overlap as a crossfade: the earlier one ends inside the
 * later one and both fades cover the overlap. Mirror of Rust `clip_editing::is_crossfade`.
 */
export function isCrossfade(x: Clip, y: Clip): boolean {
  const [a, b] = x.start <= y.start ? [x, y] : [y, x];
  if (a.content.type !== "Audio" || b.content.type !== "Audio") return false;
  const aEnd = a.start + a.length;
  const overlap = aEnd - b.start;
  return overlap > 0 && aEnd < b.start + b.length + EPS && a.start < b.start - EPS && overlap <= a.content.fade_out + EPS && overlap <= b.content.fade_in + EPS;
}

/** Two clips touch (the first ends where the second starts) within this many beats. */
const TOUCH = 1e-6;

/**
 * A clip's crossfade neighbour on one side (`out`: the next clip, `in`: the previous one):
 * `crossfaded` when they already overlap as a crossfade (`centre`: the middle of the
 * overlap, `length` its length), else a clip touching this edge that a fade-handle drag can
 * crossfade into (`centre`: the shared edge, `length` 0). `first`/`second` in time order.
 */
export interface CrossfadeSide {
  first: Clip;
  second: Clip;
  other: Clip;
  crossfaded: boolean;
  centre: number;
  length: number;
}

export function crossfadeSide(project: Project, clip: Clip, side: "in" | "out"): CrossfadeSide | null {
  if (clip.content.type !== "Audio") return null;
  const lane = clip.lane ?? null;
  let best: CrossfadeSide | null = null;
  for (const o of clipsOfTrack(project, clip.track)) {
    if (o.id === clip.id || o.content.type !== "Audio" || (o.lane ?? null) !== lane) continue;
    const [first, second] = side === "out" ? [clip, o] : [o, clip];
    if (!(first.start < second.start - EPS)) continue;
    const firstEnd = first.start + first.length;
    let found: CrossfadeSide | null = null;
    if (isCrossfade(first, second)) {
      found = { first, second, other: o, crossfaded: true, centre: (second.start + firstEnd) / 2, length: firstEnd - second.start };
    } else if (Math.abs(firstEnd - second.start) <= TOUCH) {
      found = { first, second, other: o, crossfaded: false, centre: second.start, length: 0 };
    }
    if (found && (!best || (found.crossfaded && !best.crossfaded))) best = found;
  }
  return best;
}

/** Below this on-screen length (px) a crossfade drag removes the crossfade. */
export const CROSSFADE_MIN_PX = 4;

/**
 * Crossfade length (beats) while dragging a fade handle at a clip boundary `centre`: the
 * crossfade stays symmetric around `centre` and its grabbed edge follows the pointer (at
 * beat `pointer`), so the length is twice the pointer's distance to the centre on the
 * dragged side (`before`: the edge left of the centre, else right of it). Snaps to 0
 * (remove) under `CROSSFADE_MIN_PX`.
 */
export function crossfadeDragLength(centre: number, pointer: number, before: boolean, pxPerBeat: number): number {
  const half = before ? centre - pointer : pointer - centre;
  const len = Math.max(0, 2 * half);
  return len * pxPerBeat < CROSSFADE_MIN_PX ? 0 : len;
}

/** `ClipCommand::SetCrossfade` for a crossfade side (`curve` null = keep the curves). */
export function setCrossfadeCommand(x: CrossfadeSide, length: number, curve: FadeCurve | null): Command {
  return cmd("Clip", { type: "SetCrossfade", first: x.first.id, second: x.second.id, at: x.centre, length, curve });
}

/** Default crossfade length (beats): half a beat, at most half of either clip. */
export function defaultCrossfadeLength(a: Clip, b: Clip): number {
  return Math.min(0.5, a.length / 2, b.length / 2);
}

export function crossfadeCommand(first: Clip, second: Clip, curve: FadeCurve = { type: "EqualPower" }): Command {
  return cmd("Clip", { type: "Crossfade", first: first.id, second: second.id, length: defaultCrossfadeLength(first, second), curve });
}

/** Several commands as one undo step: its English label (agents, logs) and its message. */
function batch(label: string, key: TextKey, commands: Command[]): Command | null {
  if (commands.length === 0) return null;
  if (commands.length === 1) return commands[0]!;
  return cmd("Edit", { type: "Batch", label, key, commands });
}

export function reverseCommand(clips: ReadonlyArray<Clip>, reversed: boolean): Command | null {
  return batch(
    reversed ? "reverse clips" : "unreverse clips", // i18n-ignore
    reversed ? { id: "undo.clips.reverse", args: {} } : { id: "undo.clips.unreverse", args: {} },
    clips.map((c) => cmd("Clip", { type: "SetReversed", id: c.id, reversed })),
  );
}

/** `NoteCommand::Reverse` of whole MIDI clips (notes, CC/bend/pressure lanes, per-note expression, envelopes mirrored in time). */
export function reverseMidiCommand(clips: ReadonlyArray<Clip>): Command | null {
  return batch(
    "reverse clips", // i18n-ignore
    { id: "undo.clips.reverse", args: {} },
    clips.filter((c) => c.content.type === "Midi").map((c) => cmd("Note", { type: "Reverse", clip: c.id, notes: null })),
  );
}

/**
 * Mirror just these notes (the piano roll's "Reverse" on a selection): per clip, inside the
 * span of that clip's notes among them (`min start .. max end`); the rest stay. One command
 * per clip, in one undo step. Empty = null (no-op).
 */
export function reverseNotesCommand(notes: ReadonlyArray<{ id: string; clip: string }>): Command | null {
  const byClip = new Map<string, string[]>();
  for (const n of notes) byClip.set(n.clip, [...(byClip.get(n.clip) ?? []), n.id]);
  return batch(
    "reverse notes", // i18n-ignore
    { id: "undo.note.reverse", args: {} },
    [...byClip].map(([clip, ids]) => cmd("Note", { type: "Reverse", clip, notes: ids })),
  );
}

/**
 * The "Reverse" action over any clips: audio clips toggle their reversed flag (unreversing
 * when all of them already are), MIDI clips are mirrored in time. One undo step.
 */
export function reverseClipsCommand(clips: ReadonlyArray<Clip>): Command | null {
  const audio = clips.filter((c) => c.content.type === "Audio");
  const allReversed = audio.length > 0 && audio.every((c) => audioOf(c)?.reversed);
  const parts = [reverseCommand(audio, !allReversed), reverseMidiCommand(clips)].filter((c): c is Command => c !== null);
  const unreverse = allReversed && parts.length === 1;
  return batch(
    unreverse ? "unreverse clips" : "reverse clips", // i18n-ignore
    unreverse ? { id: "undo.clips.unreverse", args: {} } : { id: "undo.clips.reverse", args: {} },
    parts,
  );
}

export function fadeCurvesCommand(clips: ReadonlyArray<Clip>, curve: FadeCurve): Command | null {
  return batch(
    "set fade curves", // i18n-ignore
    { id: "undo.clip.set_fade_curves", args: {} },
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
  // MIDI clips: the one "Reverse" entry (mirrors the notes; audio clips in the selection toggle).
  if (clip.content.type === "Midi") {
    const targets = targetClips(clip);
    return [
      { label: t("clip_edit.menu.reverse"), shortcut: shortcutLabel("clip.reverse"), onSelect: () => void sendEdit(transport, reverseClipsCommand(targets)) },
    ];
  }
  const clips = targetAudioClips(clip);
  const mixed = targetClips(clip);
  const allReversed = clips.length > 0 && clips.every((c) => audioOf(c)?.reversed);
  const equalPower = clips.every((c) => audioOf(c)?.fade_in_curve.type === "EqualPower" && audioOf(c)?.fade_out_curve.type === "EqualPower");
  const project = useProjectStore.getState().project;
  const xin = project ? crossfadeSide(project, clip, "in") : null;
  const xout = project ? crossfadeSide(project, clip, "out") : null;
  const next = xout?.crossfaded ? null : crossfadeNeighbour(clip, "next");
  const prev = xin?.crossfaded ? null : crossfadeNeighbour(clip, "previous");
  const entries: ContextMenuEntry[] = [
    {
      label: allReversed && mixed.length === clips.length ? t("clip_edit.menu.unreverse") : t("clip_edit.menu.reverse"),
      shortcut: shortcutLabel("clip.reverse"),
      onSelect: () => void sendEdit(transport, reverseClipsCommand(mixed)),
    },
    {
      label: equalPower ? t("clip_edit.menu.linear_fades") : t("clip_edit.menu.equal_power_fades"),
      onSelect: () => void sendEdit(transport, fadeCurvesCommand(clips, { type: equalPower ? "Linear" : "EqualPower" })),
    },
  ];
  if (prev) entries.push({ label: t("clip_edit.menu.crossfade_prev"), onSelect: () => void sendEdit(transport, crossfadeCommand(prev, clip)) });
  if (next) entries.push({ label: t("clip_edit.menu.crossfade_next"), onSelect: () => void sendEdit(transport, crossfadeCommand(clip, next)) });
  if (xin?.crossfaded) {
    entries.push({ label: t("clip_edit.menu.remove_crossfade_prev"), onSelect: () => void sendEdit(transport, setCrossfadeCommand(xin, 0, null)) });
  }
  if (xout?.crossfaded) {
    entries.push({ label: t("clip_edit.menu.remove_crossfade_next"), onSelect: () => void sendEdit(transport, setCrossfadeCommand(xout, 0, null)) });
  }
  return entries;
}

/**
 * Insert the clip-editing entries into the arrangement's clip menu, as their own group
 * before the trailing "Delete" group (or at the end).
 */
export function withClipEditingEntries(entries: ReadonlyArray<ContextMenuEntry>, transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  const extra = clipEditingEntries(transport, clip);
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return [...entries, "separator", ...extra];
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}

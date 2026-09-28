/**
 * Take and comp edits sent from the take-lane UI (each one command = one undo step), the
 * take-lane context menus and the next/previous take shortcuts.
 */

import type { Beats, Clip, Command, CompRegion, TakeLane, TakeLaneId, TrackId } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId, nextGestureId, type EngineTransport } from "@/transport";
import { compOf, lanesOf, laneClipsOf, regionAt } from "./model";
import { useCompingUi } from "./store";

/** How long an edit error stays on screen (ms). */
export const ERROR_MS = 5000;
let errorTimer: ReturnType<typeof setTimeout> | null = null;

function showError(track: TrackId, err: unknown): void {
  const message = err instanceof Error ? err.message : typeof err === "object" && err && "message" in err ? String(err.message) : String(err);
  useCompingUi.getState().setError({ track, message });
  if (errorTimer) clearTimeout(errorTimer);
  errorTimer = setTimeout(() => useCompingUi.getState().setError(null), ERROR_MS);
}

/**
 * Send one edit as its own gesture. Failures (a frozen track, a lane deleted by a peer, ...)
 * are shown under the track's take lanes. Resolves `true` on success.
 */
export async function sendTakeEdit(transport: EngineTransport, track: TrackId, command: Command): Promise<boolean> {
  const gesture = nextGestureId();
  try {
    await transport.send(command, { gesture });
    return true;
  } catch (err) {
    showError(track, err);
    return false;
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

const project = () => useProjectStore.getState().project;

/** The swipe: `[start, end)` of `track` now plays from `lane`. Returns the new region id. */
export async function swipeComp(transport: EngineTransport, track: TrackId, lane: TakeLaneId, start: Beats, end: Beats): Promise<string | null> {
  const [a, b] = start <= end ? [start, end] : [end, start];
  const id = newId();
  const ok = await sendTakeEdit(transport, track, cmd("Take", { type: "SetComp", id, split_id: newId(), track, lane, start: Math.max(0, a), end: b }));
  return ok ? id : null;
}

/** Previous (`-1`) or next (`+1`) take for the selected region; `false` when nothing changed. */
export function pickTake(transport: EngineTransport, dir: -1 | 1): boolean {
  const sel = useCompingUi.getState().selected;
  const p = project();
  const region = sel && p?.comp_regions[sel.region];
  if (!p || !sel || !region) return false;
  const lanes = lanesOf(p, region.track);
  const i = lanes.findIndex((l) => l.id === region.lane);
  const next = lanes[i + dir];
  if (i < 0 || !next) return false;
  void swipeComp(transport, region.track, next.id, region.start, region.end).then((id) => {
    if (id) useCompingUi.getState().select({ track: region.track, region: id });
  });
  return true;
}

/** Arrangement key handler: Up/Down pick the previous/next take for the selected region. */
export function handleTakeKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }, transport: EngineTransport): boolean {
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
  if (e.key === "ArrowUp") return pickTake(transport, -1);
  if (e.key === "ArrowDown") return pickTake(transport, 1);
  if (e.key === "Escape" && useCompingUi.getState().selected) {
    useCompingUi.getState().select(null);
    return false;
  }
  return false;
}

/** Click on a take (no drag): this lane plays the region under the pointer, or the clip's range. */
export function pickHere(transport: EngineTransport, track: TrackId, lane: TakeLaneId, at: Beats): void {
  const p = project();
  if (!p) return;
  const region = regionAt(compOf(p, track), at);
  if (region?.lane === lane) {
    useCompingUi.getState().select({ track, region: region.id });
    return;
  }
  const clip = laneClipsOf(p, lane).find((c) => at >= c.start && at < c.start + c.length);
  const range = region ?? (clip ? { start: clip.start, end: clip.start + clip.length } : null);
  if (!range) return;
  void swipeComp(transport, track, lane, range.start, range.end).then((id) => {
    if (id) useCompingUi.getState().select({ track, region: id });
  });
}

export function newLane(transport: EngineTransport, track: TrackId): void {
  useCompingUi.getState().toggle(track, true);
  void sendTakeEdit(transport, track, cmd("Take", { type: "CreateLane", id: newId(), track, name: null, before: null }));
}

export function renameLane(transport: EngineTransport, lane: TakeLane, name: string): void {
  const n = name.trim();
  if (n && n !== lane.name) void sendTakeEdit(transport, lane.track, cmd("Take", { type: "RenameLane", id: lane.id, name: n }));
}

export function toggleAudition(transport: EngineTransport, lane: TakeLane): void {
  const ui = useCompingUi.getState();
  const on = ui.audition.get(lane.track) === lane.id;
  void sendTakeEdit(transport, lane.track, cmd("Take", { type: "Audition", track: lane.track, lane: on ? null : lane.id })).then((ok) => {
    if (ok) useCompingUi.getState().setAudition(lane.track, on ? null : lane.id);
  });
}

export function flattenComp(transport: EngineTransport, track: TrackId, keepLanes: boolean): void {
  void sendTakeEdit(transport, track, cmd("Take", { type: "Flatten", track, seed: newId(), seed_notes: newId(), keep_lanes: keepLanes })).then((ok) => {
    if (ok && !keepLanes) useCompingUi.getState().toggle(track, false);
  });
}

/** Crossfade presets offered on a comp region (seconds). */
export const CROSSFADES: ReadonlyArray<{ label: string; seconds: number }> = [
  { label: "No Crossfade", seconds: 0 },
  { label: "Crossfade 5 ms", seconds: 0.005 },
  { label: "Crossfade 20 ms", seconds: 0.02 },
  { label: "Crossfade 100 ms", seconds: 0.1 },
];

/** Take entries of a track header's context menu. */
export function trackTakeEntries(transport: EngineTransport, track: TrackId): ContextMenuEntry[] {
  const p = project();
  const t = p?.tracks[track];
  if (!p || !t || (t.kind !== "Audio" && t.kind !== "Midi")) return [];
  const lanes = lanesOf(p, track);
  const regions = compOf(p, track);
  const shown = useCompingUi.getState().expanded.has(track);
  return [
    ...(lanes.length > 0 ? [{ label: shown ? "Hide Take Lanes" : "Show Take Lanes", onSelect: () => useCompingUi.getState().toggle(track) }] : []),
    { label: "New Take Lane", onSelect: () => newLane(transport, track) },
    ...(regions.length > 0
      ? [
          { label: "Flatten Comp", onSelect: () => flattenComp(transport, track, true) },
          { label: "Flatten Comp and Delete Takes", danger: true, onSelect: () => flattenComp(transport, track, false) },
        ]
      : []),
  ];
}

/** Context menu of a take lane (`region`: the comp region under the pointer, if any). */
export function laneMenu(
  transport: EngineTransport,
  lane: TakeLane,
  opts: { region: CompRegion | null; clip: Clip | null; rename: () => void },
): ContextMenuEntry[] {
  const p = project();
  if (!p) return [];
  const lanes = lanesOf(p, lane.track);
  const i = lanes.findIndex((l) => l.id === lane.id);
  const auditioning = useCompingUi.getState().audition.get(lane.track) === lane.id;
  const clips = laneClipsOf(p, lane.id);
  const span = clips.length ? { start: Math.min(...clips.map((c) => c.start)), end: Math.max(...clips.map((c) => c.start + c.length)) } : null;
  const edit = (c: Command) => () => void sendTakeEdit(transport, lane.track, c);
  const out: ContextMenuEntry[] = [
    { label: auditioning ? "Stop Auditioning" : "Audition Take", onSelect: () => toggleAudition(transport, lane) },
    {
      label: "Comp Whole Take",
      disabled: !span,
      onSelect: () => span && void swipeComp(transport, lane.track, lane.id, span.start, span.end),
    },
  ];
  if (opts.clip) {
    out.push({
      label: "Promote Clip to Main Lane",
      onSelect: edit(cmd("Take", { type: "MoveToLane", clips: [opts.clip.id], lane: null })),
    });
  }
  if (opts.region?.lane === lane.id) {
    const r = opts.region;
    out.push("separator");
    out.push({ label: "Remove From Comp", onSelect: edit(cmd("Take", { type: "ClearComp", track: lane.track, start: r.start, end: r.end, split_id: newId() })) });
    for (const x of CROSSFADES) {
      out.push({
        label: `${Math.abs(r.crossfade - x.seconds) < 1e-9 ? "✓ " : ""}${x.label}`,
        onSelect: edit(cmd("Take", { type: "SetCrossfade", region: r.id, crossfade: x.seconds })),
      });
    }
  }
  out.push(
    "separator",
    { label: "Rename", onSelect: opts.rename },
    { label: "Move Up", disabled: i <= 0, onSelect: edit(cmd("Take", { type: "MoveLane", id: lane.id, before: lanes[i - 1]?.id ?? null })) },
    {
      label: "Move Down",
      disabled: i < 0 || i >= lanes.length - 1,
      onSelect: edit(cmd("Take", { type: "MoveLane", id: lane.id, before: lanes[i + 2]?.id ?? null })),
    },
    "separator",
    ...trackTakeEntries(transport, lane.track).filter((e) => e !== "separator" && !e.label.endsWith("Take Lanes")),
    "separator",
    { label: "Delete Take", danger: true, onSelect: edit(cmd("Take", { type: "RemoveLane", id: lane.id })) },
  );
  return out;
}

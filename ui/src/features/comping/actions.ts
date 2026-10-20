/**
 * Take and comp edits sent from the take-lane UI (each one command = one undo step), the
 * take-lane context menus and the next/previous take shortcuts.
 */

import type { Beats, Clip, Command, CompRegion, TakeLane, TakeLaneId, TrackId } from "@/generated";
import { matchesAction, type ChordEvent } from "@/features/keymap";
import { useConfirm, type ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, commandErrorText, isCommandFailed, newId, nextGestureId, type EngineTransport } from "@/transport";
import { t } from "@/i18n";
import { compOf, lanesOf, laneClipsOf, regionAt } from "@/domain/comping";
import { useCompingUi } from "./store";

/** How long an edit error stays on screen (ms). */
export const ERROR_MS = 5000;
let errorTimer: ReturnType<typeof setTimeout> | null = null;

function showError(track: TrackId, err: unknown): void {
  const message = isCommandFailed(err)
    ? commandErrorText(err.error)
    : err instanceof Error
      ? err.message
      : typeof err === "object" && err && "message" in err
        ? String(err.message)
        : String(err);
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
export function handleTakeKey(e: ChordEvent, transport: EngineTransport): boolean {
  // keymap: `take.previous` / `take.next` (defaults ↑ / ↓); Escape is `edit.deselect`.
  if (matchesAction("take.previous", e)) return pickTake(transport, -1);
  if (matchesAction("take.next", e)) return pickTake(transport, 1);
  if (matchesAction("edit.deselect", e) && useCompingUi.getState().selected) {
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

/** Flattening and deleting the takes throws the other takes away: ask first. */
export function confirmFlattenAndDeleteTakes(transport: EngineTransport, track: TrackId, lanes: number): void {
  useConfirm.getState().ask({
    title: t("comping.flatten_confirm.title"),
    body: t("comping.flatten_confirm.body", { count: lanes }),
    action: t("comping.flatten_confirm.action"),
    danger: true,
    run: () => flattenComp(transport, track, false),
  });
}

/** Crossfade presets offered on a comp region (seconds). */
export const CROSSFADES: ReadonlyArray<{ seconds: number }> = [{ seconds: 0 }, { seconds: 0.005 }, { seconds: 0.02 }, { seconds: 0.1 }];

/** A crossfade preset's menu label. */
export const crossfadeLabel = (seconds: number): string =>
  seconds === 0 ? t("comping.menu.no_crossfade") : t("comping.menu.crossfade", { ms: Math.round(seconds * 1000) });

/** Take entries of a track header's context menu. */
export function trackTakeEntries(transport: EngineTransport, track: TrackId, withToggle = true): ContextMenuEntry[] {
  const p = project();
  const tr = p?.tracks[track];
  if (!p || !tr || (tr.kind !== "Audio" && tr.kind !== "Midi")) return [];
  const lanes = lanesOf(p, track);
  const regions = compOf(p, track);
  const shown = useCompingUi.getState().expanded.has(track);
  return [
    ...(withToggle && lanes.length > 0
      ? [{ label: shown ? t("comping.menu.hide_lanes") : t("comping.menu.show_lanes"), onSelect: () => useCompingUi.getState().toggle(track) }]
      : []),
    { label: t("comping.menu.new_lane"), onSelect: () => newLane(transport, track) },
    ...(regions.length > 0
      ? [
          { label: t("comping.menu.flatten"), onSelect: () => flattenComp(transport, track, true) },
          { label: t("comping.menu.flatten_delete"), danger: true, onSelect: () => confirmFlattenAndDeleteTakes(transport, track, lanes.length) },
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
    { label: auditioning ? t("comping.menu.stop_audition") : t("comping.menu.audition"), onSelect: () => toggleAudition(transport, lane) },
    {
      label: t("comping.menu.comp_whole"),
      disabled: !span,
      onSelect: () => span && void swipeComp(transport, lane.track, lane.id, span.start, span.end),
    },
  ];
  if (opts.clip) {
    out.push({
      label: t("comping.menu.promote"),
      onSelect: edit(cmd("Take", { type: "MoveToLane", clips: [opts.clip.id], lane: null })),
    });
  }
  if (opts.region?.lane === lane.id) {
    const r = opts.region;
    out.push("separator");
    out.push({
      label: t("comping.menu.remove_from_comp"),
      onSelect: edit(cmd("Take", { type: "ClearComp", track: lane.track, start: r.start, end: r.end, split_id: newId() })),
    });
    for (const x of CROSSFADES) {
      out.push({
        label: `${Math.abs(r.crossfade - x.seconds) < 1e-9 ? "✓ " : ""}${crossfadeLabel(x.seconds)}`,
        onSelect: edit(cmd("Take", { type: "SetCrossfade", region: r.id, crossfade: x.seconds })),
      });
    }
  }
  out.push(
    "separator",
    { label: t("comping.menu.rename"), onSelect: opts.rename },
    { label: t("comping.menu.move_up"), disabled: i <= 0, onSelect: edit(cmd("Take", { type: "MoveLane", id: lane.id, before: lanes[i - 1]?.id ?? null })) },
    {
      label: t("comping.menu.move_down"),
      disabled: i < 0 || i >= lanes.length - 1,
      onSelect: edit(cmd("Take", { type: "MoveLane", id: lane.id, before: lanes[i + 2]?.id ?? null })),
    },
    "separator",
    ...trackTakeEntries(transport, lane.track, false),
    "separator",
    { label: t("comping.menu.delete_take"), danger: true, onSelect: edit(cmd("Take", { type: "RemoveLane", id: lane.id })) },
  );
  return out;
}

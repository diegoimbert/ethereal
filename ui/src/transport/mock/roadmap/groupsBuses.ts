/**
 * Mock of `Track::{GroupSelected, Ungroup, SetVca}` (v0.2, `groups-buses`; CONTRACTS.md
 * §12.10), called from the core track reducer's hook. Mirrors
 * `crates/ether-controller/src/groups/mod.rs`.
 */

import type { Track, TrackCommand, TrackId } from "@/generated";
import { compareOrderKeys, keyBetween } from "@/state/orderKey";
import { fail, type ReducerContext } from "../documentReducer";
import { makeTrack } from "../demoProject";

/** Name of a group created by `GroupSelected { name: null }`. */
export const DEFAULT_GROUP_NAME = "Group";

/** Deletes one track and its cascade (devices, lanes, sends, routings to it). */
export type DeleteTrack = (id: TrackId) => void;

const byOrder = (a: Track, b: Track) => compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id);

function siblings(ctx: ReducerContext, parent: TrackId | null): Track[] {
  return ctx.tx
    .all("Track")
    .filter((t) => t.parent === parent)
    .sort(byOrder);
}

function get(ctx: ReducerContext, id: TrackId): Track {
  return ctx.tx.get("Track", id) ?? fail("NotFound", `track ${id}`);
}

/** What ungrouping `group` would lose (for the `force` check), as a human list. */
export function ungroupLosses(tracks: readonly Track[], devices: number, lanes: number, sends: number, group: TrackId): string[] {
  const lost: string[] = [];
  if (devices > 0) lost.push(`${devices} device(s)`);
  if (lanes > 0) lost.push(`${lanes} automation lane(s)`);
  if (sends > 0) lost.push(`${sends} send(s)`);
  const routed = tracks.filter(
    (t) => (t.output.type === "Track" && t.output.track === group) || (t.input.type === "Track" && t.input.track === group),
  ).length;
  if (routed > 0) lost.push(`${routed} routing(s) to it`);
  return lost;
}

/**
 * Document command (one undo step), dispatched from the track reducer's hook. `Delete` is
 * pre-processed here too (tracks assigned to a deleted VCA are unassigned) and returns
 * `false` so the core reducer carries on; the three groups commands return `true`.
 */
export function groupsTrackCommand(ctx: ReducerContext, c: TrackCommand, deleteTrack: DeleteTrack): boolean {
  switch (c.type) {
    case "GroupSelected":
      groupSelected(ctx, c.ids, c.group, c.name);
      return true;
    case "Ungroup":
      ungroup(ctx, c.group, c.force, deleteTrack);
      return true;
    case "SetVca":
      setVca(ctx, c.id, c.vca);
      return true;
    case "Delete":
      for (const t of ctx.tx.all("Track")) {
        if (t.vca === c.id) ctx.tx.upsert("Track", withVca(t, null));
      }
      return false;
    default:
      return false;
  }
}

function withVca(t: Track, vca: TrackId | null): Track {
  const { vca: _old, ...rest } = t;
  void _old;
  return vca === null ? rest : { ...rest, vca };
}

function groupSelected(ctx: ReducerContext, ids: TrackId[], group: TrackId, name: string | null): void {
  const existing = ctx.tx.get("Track", group);
  if (existing) {
    // Idempotent retry.
    if (existing.kind === "Group" && ids.every((id) => ctx.tx.get("Track", id)?.parent === group)) return;
    fail("InvalidArgument", `track ${group} already exists`);
  }
  if (ids.length === 0) fail("InvalidArgument", "select at least one track to group");
  if (new Set(ids).size !== ids.length) fail("InvalidArgument", "duplicate track in the selection");
  const tracks = ids.map((id) => get(ctx, id));
  for (const t of tracks) {
    if (t.kind === "Master" || t.kind === "Return" || t.kind === "Vca") fail("InvalidArgument", `${t.kind} tracks cannot be grouped`);
  }
  const parent = tracks[0]!.parent;
  if (tracks.some((t) => t.parent !== parent)) fail("InvalidArgument", "grouped tracks must share the same parent (select siblings)");
  const selected = new Set(ids);
  const sibs = siblings(ctx, parent);
  const first = sibs.findIndex((t) => selected.has(t.id));
  const firstTrack = sibs[first]!;
  ctx.tx.upsert(
    "Track",
    makeTrack({
      id: group,
      kind: "Group",
      name: name?.trim() || DEFAULT_GROUP_NAME,
      color: firstTrack.color,
      order: keyBetween(sibs[first - 1]?.order ?? null, firstTrack.order),
      parent,
    }),
  );
  for (const t of sibs) if (selected.has(t.id)) ctx.tx.upsert("Track", { ...t, parent: group });
}

function ungroup(ctx: ReducerContext, group: TrackId, force: boolean, deleteTrack: DeleteTrack): void {
  const g = get(ctx, group);
  if (g.kind !== "Group") fail("InvalidArgument", `track ${group} is not a group`);
  const devices = ctx.tx.all("Device").filter((d) => d.track === group && d.pad === null).length;
  const lanes = ctx.tx
    .all("AutomationLane")
    .filter(
      (l) =>
        (l.owner.type === "Track" && l.owner.track === group) ||
        ((l.target.type === "TrackVolume" || l.target.type === "TrackPan") && l.target.track === group),
    ).length;
  const sends = ctx.tx.all("Send").filter((s) => s.from === group || s.to === group).length;
  const lost = ungroupLosses(ctx.tx.all("Track"), devices, lanes, sends, group);
  if (lost.length > 0 && !force) fail("InvalidState", `ungrouping ${g.name} loses its ${lost.join(", ")}`);
  const sibs = siblings(ctx, g.parent);
  const at = sibs.findIndex((t) => t.id === group);
  const hi = sibs[at + 1]?.order ?? null;
  let lo = sibs[at - 1]?.order ?? null;
  for (const k of siblings(ctx, group)) {
    const order = keyBetween(lo, hi);
    ctx.tx.upsert("Track", { ...k, parent: g.parent, order });
    lo = order;
  }
  deleteTrack(group);
}

function setVca(ctx: ReducerContext, id: TrackId, vca: TrackId | null): void {
  const t = get(ctx, id);
  if ((t.vca ?? null) === vca) return;
  if (t.kind === "Master") fail("InvalidArgument", "the master track cannot be assigned to a VCA");
  if (vca !== null) {
    if (get(ctx, vca).kind !== "Vca") fail("InvalidArgument", `track ${vca} is not a VCA`);
    let cur: TrackId | null = vca;
    let steps = 0;
    while (cur !== null) {
      if (cur === id || steps++ > 10_000) fail("InvalidArgument", "VCA assignment cycle");
      cur = ctx.tx.get("Track", cur)?.vca ?? null;
    }
  }
  ctx.tx.upsert("Track", withVca(t, vca));
}

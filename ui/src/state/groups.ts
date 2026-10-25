/**
 * Group-bys of the normalized tables (notes by clip, clips by track, ...), cached per table
 * object, and their carry-over across patches ([`carryGroups`]). Used by `./selectors` and
 * the project store; no store import here (the store calls `carryGroups`).
 */

import type { AutomationLaneId, AutomationPoint, Clip, ClipId, Device, Note, PatchChange, Project, TrackId, WarpMarker } from "@/generated";
import { isNoteTable, noteValues } from "./noteTable";
import { compareOrderKeys } from "./orderKey";

/** First index of `sorted` whose entry sorts after `e`. */
function upperBound<T>(sorted: ReadonlyArray<T>, e: T, compare: (a: T, b: T) => number): number {
  let lo = 0;
  let hi = sorted.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (compare(sorted[mid]!, e) <= 0) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** A store table (frozen, or the sharded note table): it changes identity when it changes. */
function immutableTable(t: object): boolean {
  return Object.isFrozen(t) || isNoteTable(t);
}

export const byOrder = (a: { order: string; id: string }, b: { order: string; id: string }) =>
  compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id);

export const byTime = (a: { time: number; id: string }, b: { time: number; id: string }) => a.time - b.time || compareOrderKeys(a.id, b.id);

/**
 * Entities of a table grouped by their owner (notes by clip, clips by track, ...), each
 * group sorted. Built once per table object: the store is immutable (immer), so a table
 * changes identity exactly when it changes, and the index is rebuilt on the next read.
 * Lookups are O(1) and return the same array until the table changes, so components
 * re-render only when their own entities did. Mutable tables (the mock engine's working
 * copy) aren't frozen and are grouped fresh on every call instead.
 */
export function groupedBy<T, K>(
  cache: WeakMap<object, Map<K, T[]>>,
  table: Readonly<Record<string, T>>,
  keyOf: (t: T) => K | null,
  compare: (a: T, b: T) => number,
): Map<K, T[]> {
  const hit = cache.get(table);
  if (hit) return hit;
  const groups = new Map<K, T[]>();
  for (const t of (noteValues(table) as T[] | null) ?? Object.values(table)) {
    const k = keyOf(t);
    if (k === null) continue;
    const g = groups.get(k);
    if (g) g.push(t);
    else groups.set(k, [t]);
  }
  for (const g of groups.values()) g.sort(compare);
  if (immutableTable(table)) {
    // Shared between callers: never mutate a returned group (copy it first).
    for (const g of groups.values()) Object.freeze(g);
    cache.set(table, groups);
  }
  return groups;
}

export const NONE: never[] = Object.freeze([]) as never[];
export const notesByClip = new WeakMap<object, Map<ClipId, Note[]>>();
export const clipsByTrack = new WeakMap<object, Map<TrackId, Clip[]>>();
export const devicesByTrack = new WeakMap<object, Map<TrackId, Device[]>>();
export const pointsByLane = new WeakMap<object, Map<AutomationLaneId, AutomationPoint[]>>();
export const markersByClip = new WeakMap<object, Map<ClipId, WarpMarker[]>>();

export const noteOrder = (a: Note, b: Note) => a.start - b.start || a.pitch - b.pitch || compareOrderKeys(a.id, b.id);
export const clipOrder = (a: Clip, b: Clip) => a.start - b.start || compareOrderKeys(a.id, b.id);
export const trackChainKey = (d: Device) => (d.pad === null && d.chain == null ? d.track : null);

/** A group-by that a patch can carry over ([`carryGroups`]): its order is total (ids break ties). */
interface Grouping<T extends { id: string }, K> {
  type: PatchEntityType;
  table: (p: Project) => Readonly<Record<string, T>>;
  cache: WeakMap<object, Map<K, T[]>>;
  keyOf: (t: T) => K | null;
  compare: (a: T, b: T) => number;
}

type PatchEntityType = "Note" | "Clip" | "Device" | "AutomationPoint";

function grouping<T extends { id: string }, K>(g: Grouping<T, K>): Grouping<{ id: string }, unknown> {
  return g as unknown as Grouping<{ id: string }, unknown>;
}

const GROUPINGS: Grouping<{ id: string }, unknown>[] = [
  grouping<Note, ClipId>({ type: "Note", table: (p) => p.notes, cache: notesByClip, keyOf: (n) => n.clip, compare: noteOrder }),
  grouping<Clip, TrackId>({ type: "Clip", table: (p) => p.clips, cache: clipsByTrack, keyOf: (c) => c.track, compare: clipOrder }),
  grouping<Device, TrackId>({ type: "Device", table: (p) => p.devices, cache: devicesByTrack, keyOf: trackChainKey, compare: byOrder }),
  grouping<AutomationPoint, AutomationLaneId>({
    type: "AutomationPoint",
    table: (p) => p.automation_points,
    cache: pointsByLane,
    keyOf: (pt) => pt.lane,
    compare: byTime,
  }),
];

/**
 * After a patch made `next` from `prev`: derive the group-bys already built for `prev`'s
 * tables for `next`'s, regrouping only the owners the patch touched. Untouched groups keep
 * their arrays (identity), so components of unchanged clips/tracks/lanes don't re-render,
 * and a one-note edit costs that clip's notes, not every note of the project. The result
 * is what a full regroup of `next` would build.
 */
export function carryGroups(prev: Project, next: Project, changes: ReadonlyArray<PatchChange>): void {
  for (const g of GROUPINGS) {
    const before = g.table(prev);
    const after = g.table(next);
    if (before === after || !immutableTable(after)) continue;
    const old = g.cache.get(before);
    if (!old) continue;
    const ids = new Set<string>();
    for (const c of changes) {
      if (c.type === "Upsert" && c.entity.type === g.type) ids.add(c.entity.value.id);
      else if (c.type === "Remove" && c.key.type === g.type) ids.add(c.key.id);
      else if (c.type === "DeviceParam" && g.type === "Device") ids.add(c.id);
    }
    // Each touched owner and what the patch puts in it (one pass over the changed ids).
    const owners = new Map<unknown, { id: string }[]>();
    for (const id of ids) {
      const was = before[id];
      const k0 = was === undefined ? null : g.keyOf(was);
      if (k0 !== null && !owners.has(k0)) owners.set(k0, []);
      const now = after[id];
      const k1 = now === undefined ? null : g.keyOf(now);
      if (k1 !== null) {
        const list = owners.get(k1);
        if (list) list.push(now!);
        else owners.set(k1, [now!]);
      }
    }
    const groups = new Map(old);
    for (const [k, added] of owners) {
      const members = (old.get(k) ?? []).filter((e) => !ids.has(e.id));
      if (added.length > 16) {
        members.push(...added);
        members.sort(g.compare);
      } else {
        // A few changes (an edit, a drag step): insert them where they sort, instead of
        // re-sorting the group (a big clip's notes).
        for (const e of added) members.splice(upperBound(members, e, g.compare), 0, e);
      }
      if (members.length === 0) groups.delete(k);
      else groups.set(k, Object.freeze(members) as { id: string }[]);
    }
    g.cache.set(after, groups);
  }
}

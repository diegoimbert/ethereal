/**
 * Pure helpers of the multisampler zone editor (`ether_model::multisampler`): note names
 * (C3 = 60), root keys parsed from sample names, mapping dropped samples across keys,
 * hit-testing and drag edits on the key × velocity map.
 */

import type { MediaId, SampleZone, Zone } from "@/generated";

export const KEYS = 128;
/** Velocity rows (1..=127; 0 counts as 1). */
export const VELS = 127;

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const SEMI: Record<string, number> = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };

/** "C3" for 60 (the zone model's convention: middle C = C3). */
export function noteName(key: number): string {
  return `${NAMES[key % 12]}${Math.floor(key / 12) - 2}`;
}

const clampKey = (k: number) => Math.min(KEYS - 1, Math.max(0, Math.round(k)));
const clampVel = (v: number) => Math.min(VELS, Math.max(1, Math.round(v)));

/**
 * Root key from a sample's file name: a note name (`C3`, `F#2`, `Db-1`; C3 = 60) or a MIDI
 * number of 2-3 digits (`060`, `_60_`, `piano 72.wav`), not glued to other letters/digits. `null` if none.
 */
export function rootFromName(name: string): number | null {
  const base = name.replace(/\.[A-Za-z0-9]{1,5}$/, "");
  const note = /(?:^|[^A-Za-z0-9#])([A-Ga-g])([#b]?)(-?\d)(?![0-9])/.exec(base);
  if (note) {
    const semi = SEMI[note[1]!.toLowerCase()]! + (note[2] === "#" ? 1 : note[2] === "b" ? -1 : 0);
    const key = (Number(note[3]) + 2) * 12 + semi;
    if (key >= 0 && key < KEYS) return key;
  }
  const num = /(?:^|[^0-9A-Za-z])(\d{2,3})(?![0-9])/.exec(base);
  if (num) {
    const key = Number(num[1]);
    if (key < KEYS) return key;
  }
  return null;
}

export function newZone(media: MediaId | null, root: number, keys: Zone): SampleZone {
  return {
    media,
    root_key: root,
    tune_cents: 0,
    keys,
    velocities: { lo: 1, hi: VELS },
    round_robin: 0,
    start: 0,
    end: null,
    looping: false,
    loop_start: 0,
    loop_end: 0,
    loop_crossfade: 0,
    gain: 0,
    pan: 0,
  };
}

export interface DroppedSample {
  media: MediaId;
  name: string;
}

/**
 * Zones for samples dropped on `key`. Roots come from the names (else consecutive keys from
 * `key`, in drop order); several samples split the keyboard between their roots (each zone
 * runs from halfway after the previous root to halfway before the next; the lowest starts
 * at `min(key, its root)`, the highest ends at its root). One sample alone: the whole
 * keyboard when the map is empty, else just its root key.
 */
export function mapDropped(samples: ReadonlyArray<DroppedSample>, key: number, mapEmpty: boolean): SampleZone[] {
  if (samples.length === 0) return [];
  let next = clampKey(key);
  const rooted = samples.map((s) => {
    const parsed = rootFromName(s.name);
    const root = parsed ?? next;
    if (parsed === null) next = Math.min(KEYS - 1, next + 1);
    return { ...s, root };
  });
  if (rooted.length === 1) {
    const r = rooted[0]!;
    return [newZone(r.media, r.root, mapEmpty ? { lo: 0, hi: KEYS - 1 } : { lo: r.root, hi: r.root })];
  }
  rooted.sort((a, b) => a.root - b.root);
  return rooted.map((r, i) => {
    const prev = rooted[i - 1];
    const nxt = rooted[i + 1];
    const lo = prev ? Math.min(r.root, Math.floor((prev.root + r.root) / 2) + 1) : Math.min(clampKey(key), r.root);
    const hi = nxt ? Math.max(lo, Math.floor((r.root + nxt.root) / 2)) : r.root;
    return newZone(r.media, r.root, { lo, hi: Math.max(lo, hi) });
  });
}

// ---- geometry ---------------------------------------------------------------------------

export interface MapSize {
  w: number;
  h: number;
}

export const keyToX = (key: number, s: MapSize) => (key / KEYS) * s.w;
export const xToKey = (x: number, s: MapSize) => clampKey(Math.floor((x / Math.max(1, s.w)) * KEYS));
/** Top of velocity row `v` (127 at the top). */
export const velToY = (v: number, s: MapSize) => (1 - v / VELS) * s.h;
export const yToVel = (y: number, s: MapSize) => clampVel(Math.floor((1 - y / Math.max(1, s.h)) * VELS) + 1);

/** Pixel rectangle of a zone. */
export function zoneRect(z: SampleZone, s: MapSize) {
  const x0 = keyToX(z.keys.lo, s);
  const x1 = keyToX(z.keys.hi + 1, s);
  const y0 = velToY(z.velocities.hi, s);
  const y1 = velToY(Math.max(1, z.velocities.lo) - 1, s);
  return { x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0) };
}

export type Edge = "left" | "right" | "top" | "bottom";
export type Grab = { zone: number; part: Edge | "body" };

/**
 * What a press at (x, y) grabs: an edge of the selected zone first (within `tol` px), else
 * the topmost zone under the pointer (later zones draw on top), else nothing.
 */
export function hitTest(zones: ReadonlyArray<SampleZone>, selected: number | null, x: number, y: number, s: MapSize, tol: number): Grab | null {
  const edgeOf = (i: number): Grab | null => {
    const r = zoneRect(zones[i]!, s);
    const inY = y >= r.y - tol && y <= r.y + r.h + tol;
    const inX = x >= r.x - tol && x <= r.x + r.w + tol;
    if (inY && Math.abs(x - r.x) <= tol) return { zone: i, part: "left" };
    if (inY && Math.abs(x - (r.x + r.w)) <= tol) return { zone: i, part: "right" };
    if (inX && Math.abs(y - r.y) <= tol) return { zone: i, part: "top" };
    if (inX && Math.abs(y - (r.y + r.h)) <= tol) return { zone: i, part: "bottom" };
    return null;
  };
  if (selected !== null && zones[selected]) {
    const e = edgeOf(selected);
    if (e) return e;
  }
  for (let i = zones.length - 1; i >= 0; i--) {
    const r = zoneRect(zones[i]!, s);
    if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) return { zone: i, part: "body" };
  }
  return null;
}

/**
 * Zone `start` after dragging `part` by (dKey, dVel) (keys right, velocities up). Edges stay
 * ordered; a body move shifts both ranges (and the root with the keys) inside the map.
 */
export function dragZone(start: SampleZone, part: Edge | "body", dKey: number, dVel: number): SampleZone {
  const k = start.keys;
  const v = { lo: Math.max(1, start.velocities.lo), hi: start.velocities.hi };
  switch (part) {
    case "left":
      return { ...start, keys: { lo: Math.min(k.hi, clampKey(k.lo + dKey)), hi: k.hi } };
    case "right":
      return { ...start, keys: { lo: k.lo, hi: Math.max(k.lo, clampKey(k.hi + dKey)) } };
    case "top":
      return { ...start, velocities: { lo: v.lo, hi: Math.max(v.lo, clampVel(v.hi + dVel)) } };
    case "bottom":
      return { ...start, velocities: { lo: Math.min(v.hi, clampVel(v.lo + dVel)), hi: v.hi } };
    case "body": {
      const dk = Math.min(KEYS - 1 - k.hi, Math.max(-k.lo, Math.round(dKey)));
      const dv = Math.min(VELS - v.hi, Math.max(1 - v.lo, Math.round(dVel)));
      return {
        ...start,
        root_key: clampKey(start.root_key + dk),
        keys: { lo: k.lo + dk, hi: k.hi + dk },
        velocities: { lo: v.lo + dv, hi: v.hi + dv },
      };
    }
  }
}

/** `zones` with zone `i` replaced. */
export function withZone(zones: ReadonlyArray<SampleZone>, i: number, z: SampleZone): SampleZone[] {
  return zones.map((old, j) => (j === i ? z : old));
}

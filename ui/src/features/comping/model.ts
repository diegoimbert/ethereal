/**
 * Takes and comping, pure model helpers (v0.2, `comping`; CONTRACTS.md §12.2). Shared by the
 * take-lane UI and the MockTransport simulation (`transport/mock/roadmap/comping.ts`), and
 * kept in step with the controller (`crates/ether-controller/src/comping`):
 *
 * - `lanesOf` / `compOf` / `laneClipsOf`: a track's lanes (by order), regions (by start),
 *   a lane's clips (by start).
 * - `swipeRegions`: what a swipe (`Take::SetComp`) or `ClearComp` does to the regions.
 * - `compPieces`: the clip pieces the comp plays (regions trimmed to their lane's clips,
 *   equal-power boundary crossfades on audio tracks).
 * - `deriveId`: `ether_model::derive_id` (collab-safe ids for `Flatten`).
 */

import type { Beats, Clip, ClipId, CompRegion, CompRegionId, Project, TakeLane, TakeLaneId, TrackId } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";

const EPS = BEATS_EPSILON;
/** Default boundary crossfade of a new region (seconds), `DEFAULT_COMP_CROSSFADE`. */
export const DEFAULT_COMP_CROSSFADE = 0.005;
/** Longest boundary crossfade (seconds), `MAX_COMP_CROSSFADE`. */
export const MAX_COMP_CROSSFADE = 0.5;

const byOrder = (a: TakeLane, b: TakeLane) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
const byStart = <T extends { start: number; id: string }>(a: T, b: T) => a.start - b.start || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

export function lanesOf(project: Pick<Project, "take_lanes">, track: TrackId): TakeLane[] {
  return Object.values(project.take_lanes ?? {})
    .filter((l) => l.track === track)
    .sort(byOrder);
}

export function compOf(project: Pick<Project, "comp_regions">, track: TrackId): CompRegion[] {
  return Object.values(project.comp_regions ?? {})
    .filter((r) => r.track === track)
    .sort(byStart);
}

export function laneClipsOf(project: Pick<Project, "clips">, lane: TakeLaneId): Clip[] {
  return Object.values(project.clips)
    .filter((c) => c.lane === lane)
    .sort(byStart);
}

/** Default name of a new lane: "Take N", N one past the highest existing number. */
export function nextTakeName(lanes: ReadonlyArray<TakeLane>): string {
  let n = lanes.length;
  for (const l of lanes) {
    const m = /^Take (\d+)$/.exec(l.name.trim());
    if (m) n = Math.max(n, Number(m[1]));
  }
  return `Take ${n + 1}`;
}

/** One region write of a swipe: `upsert` (new or changed) or `remove`. */
export type RegionEdit = { type: "upsert"; region: CompRegion } | { type: "remove"; id: CompRegionId };

/**
 * The region edits that clear `[start, end)` of `regions` (one track, sorted): regions are
 * trimmed, split (the right part gets `splitId`) or removed. Adjacent regions are not merged.
 */
export function clearRegions(regions: ReadonlyArray<CompRegion>, start: Beats, end: Beats, splitId: CompRegionId): RegionEdit[] {
  const out: RegionEdit[] = [];
  for (const r of regions) {
    if (r.end <= start + EPS || r.start >= end - EPS) continue;
    const left = r.start < start - EPS;
    const right = r.end > end + EPS;
    if (!left && !right) out.push({ type: "remove", id: r.id });
    else if (left && !right) out.push({ type: "upsert", region: { ...r, end: start } });
    else if (!left && right) out.push({ type: "upsert", region: { ...r, start: end } });
    else {
      out.push({ type: "upsert", region: { ...r, end: start } });
      out.push({ type: "upsert", region: { ...r, id: splitId, start: end } });
    }
  }
  return out;
}

/** `Take::SetComp`: the edits that make `[start, end)` of `track` play from `lane`. */
export function swipeRegions(
  regions: ReadonlyArray<CompRegion>,
  s: { id: CompRegionId; splitId: CompRegionId; track: TrackId; lane: TakeLaneId; start: Beats; end: Beats },
): RegionEdit[] {
  return [
    ...clearRegions(regions, s.start, s.end, s.splitId),
    { type: "upsert", region: { id: s.id, track: s.track, lane: s.lane, start: s.start, end: s.end, crossfade: DEFAULT_COMP_CROSSFADE } },
  ];
}

/** Apply region edits to a sorted list (for previews). */
export function applyRegionEdits(regions: ReadonlyArray<CompRegion>, edits: ReadonlyArray<RegionEdit>): CompRegion[] {
  const byId = new Map(regions.map((r) => [r.id, r]));
  for (const e of edits) {
    if (e.type === "remove") byId.delete(e.id);
    else byId.set(e.region.id, e.region);
  }
  return [...byId.values()].sort(byStart);
}

/** One piece of a comp: `[start, end)` of take clip `clip` from content position `offset`. */
export interface CompPiece {
  region: CompRegionId;
  lane: TakeLaneId;
  index: number;
  clip: ClipId;
  start: Beats;
  end: Beats;
  offset: Beats;
  /** Comp fades in beats (equal power); `null` = the clip's own fade. */
  fadeIn: Beats | null;
  fadeOut: Beats | null;
}

/** Content position of `clip` `d` beats after its start (loop unrolled). */
export function contentAt(clip: Clip, d: Beats): Beats {
  const pos = clip.offset + d;
  const { enabled, start: ls, end: le } = clip.looping;
  if (!enabled || le - ls <= EPS || clip.offset >= le || pos < le) return pos;
  const len = le - ls;
  return ls + ((((pos - le) % len) + len) % len);
}

/**
 * The clip pieces `track`'s comp plays (see `ether_model::take`). `bpmAt` converts the
 * crossfades (seconds) to beats at each boundary. Sorted by start.
 */
export function compPieces(project: Pick<Project, "clips" | "comp_regions" | "tracks">, track: TrackId, bpmAt: (beats: Beats) => number = () => 120): CompPiece[] {
  const audio = project.tracks[track]?.kind === "Audio";
  const regions = compOf(project, track);
  const beatsOf = (t: Beats, seconds: number) => (seconds * bpmAt(t)) / 60;
  const out: CompPiece[] = [];
  regions.forEach((r, i) => {
    const prev = i > 0 && Math.abs(regions[i - 1]!.end - r.start) <= EPS;
    const next = regions[i + 1] && Math.abs(regions[i + 1]!.start - r.end) <= EPS ? regions[i + 1]! : null;
    const halfIn = beatsOf(r.start, r.crossfade / 2);
    const halfOut = beatsOf(r.end, (next ?? r).crossfade / 2);
    const extL = audio && prev ? halfIn : 0;
    const extR = audio && next ? halfOut : 0;
    const fadeInEnd = r.start + halfIn;
    const fadeOutStart = r.end - halfOut;
    let index = 0;
    for (const c of laneClipsOf(project, r.lane)) {
      if (c.track !== track) continue;
      const cs = c.start;
      const ce = c.start + c.length;
      if (ce <= r.start + EPS || cs >= r.end - EPS) continue;
      const start = Math.max(cs, r.start - extL);
      const end = Math.min(ce, r.end + extR);
      if (end - start <= EPS) continue;
      let fadeIn = audio && cs <= r.start + EPS ? Math.max(0, fadeInEnd - start) : null;
      let fadeOut = audio && ce >= r.end - EPS ? Math.max(0, end - fadeOutStart) : null;
      const total = (fadeIn ?? 0) + (fadeOut ?? 0);
      if (total > end - start) {
        const k = (end - start) / total;
        fadeIn = fadeIn === null ? null : fadeIn * k;
        fadeOut = fadeOut === null ? null : fadeOut * k;
      }
      out.push({ region: r.id, lane: r.lane, index: index++, clip: c.id, start, end, offset: contentAt(c, start - cs), fadeIn, fadeOut });
    }
  });
  return out.sort((a, b) => a.start - b.start || (a.region < b.region ? -1 : a.region > b.region ? 1 : a.index - b.index));
}

/** The region of `track` at beat `t` (`null` = none). */
export function regionAt(regions: ReadonlyArray<CompRegion>, t: Beats): CompRegion | null {
  return regions.find((r) => t >= r.start - EPS && t < r.end - EPS) ?? null;
}

// ─── derive_id ──────────────────────────────────────────────────────────────────────────

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const M64 = (1n << 64n) - 1n;

function decodeUlid(s: string): bigint {
  let v = 0n;
  for (const ch of s.toUpperCase()) {
    const d = CROCKFORD.indexOf(ch);
    if (d < 0) throw new Error(`not a ULID: ${s}`);
    v = (v << 5n) | BigInt(d);
  }
  return v & ((1n << 128n) - 1n);
}

function encodeUlid(v: bigint): string {
  let out = "";
  for (let i = 0; i < 26; i++) {
    out = CROCKFORD[Number(v & 31n)]! + out;
    v >>= 5n;
  }
  return out;
}

const rotl64 = (x: bigint, k: bigint) => ((x << k) | (x >> (64n - k))) & M64;

/** `ether_model::derive_id(seed, index)`: a deterministic id derived from a seed id. */
export function deriveId(seed: string, index: number): string {
  const u = decodeUlid(seed);
  const timestamp = u >> 80n;
  const random = u & ((1n << 80n) - 1n);
  const lo = random & M64;
  const hi = random >> 64n;
  const i = BigInt(index);
  let x = lo ^ rotl64(hi & M64, 17n) ^ (((i + 1n) * 0x9e3779b97f4a7c15n) & M64);
  x ^= x >> 30n;
  x = (x * 0xbf58476d1ce4e5b9n) & M64;
  x ^= x >> 27n;
  x = (x * 0x94d049bb133111ebn) & M64;
  x ^= x >> 31n;
  const h = (hi ^ rotl64(i, 32n)) & 0xffffn;
  const mixed = (h << 64n) | x;
  return encodeUlid((timestamp << 80n) | mixed);
}

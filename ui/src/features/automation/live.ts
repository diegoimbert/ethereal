/**
 * Live automated values: what the engine applies to a target at the playhead, so knobs,
 * switches and faders follow their automation during playback (and at the stopped
 * position, where the engine evaluates automation too).
 *
 * Evaluated locally from the document and the playhead frames (no engine round trip, no
 * extra IPC), with the engine's rules (`ether_core::automation_rt`, compiled by
 * `ether_controller::compile`):
 * - an enabled arrangement lane (owned by a track) with points drives the target;
 * - an unmuted main-lane clip whose enabled envelope targets it overrides the lane where
 *   the clip plays (the last such clip, by start, wins), evaluated at the clip's content
 *   beat (loops unrolled);
 * - envelopes only apply to targets on the clip's own track;
 * - curve math is `evaluatePoints` (bit-identical to the engine's).
 *
 * Values are normalized (0..1); callers map them through the target's `ParamInfo`.
 */

import { useCallback, useSyncExternalStore } from "react";
import type { AutomationTarget, Clip, DeviceId, Project, TrackId } from "@/generated";
import { compareOrderKeys, playheadStore, pointsOfLane, useProjectStore } from "@/state";
import { contentBeatAt } from "@/features/arrangement/clipTime";
import { DEVICE_ENABLED_PARAM } from "@/features/modulation/model";
import { useModulatedValue } from "@/features/modulation/readback";
import { evaluatePoints, type CurvePoint } from "./curve";
import { targetKey } from "./params";

interface Envelope {
  clip: Clip;
  points: ReadonlyArray<CurvePoint>;
}

interface Source {
  lane: ReadonlyArray<CurvePoint> | null;
  /** Sorted by clip start (then id), like the engine's clip list. */
  envelopes: Envelope[];
}

type Index = Map<string, Source>;

/** The track a target lives on (`null` when its entity is gone). */
function trackOf(project: Project, target: AutomationTarget): TrackId | null {
  switch (target.type) {
    case "TrackVolume":
    case "TrackPan":
      return project.tracks[target.track] ? target.track : null;
    case "SendLevel":
      return project.sends[target.send]?.from ?? null;
    case "DeviceParam":
    case "DeviceEnabled":
      return project.devices[target.device]?.track ?? null;
    case "ClipGain":
    case "ClipTranspose":
      return null; // the clip's own value: no track control shows it
  }
}

function buildIndex(project: Project): Index {
  const index: Index = new Map();
  const source = (key: string): Source => {
    let s = index.get(key);
    if (!s) index.set(key, (s = { lane: null, envelopes: [] }));
    return s;
  };
  // Lanes in id order: the engine keeps the first lane with points per target.
  const lanes = Object.values(project.automation_lanes).sort((a, b) => compareOrderKeys(a.id, b.id));
  for (const lane of lanes) {
    if (!lane.enabled) continue;
    const points = pointsOfLane(project, lane.id);
    if (points.length === 0) continue;
    const track = trackOf(project, lane.target);
    if (track === null) continue;
    if (lane.owner.type === "Track") {
      const s = source(targetKey(lane.target));
      if (!s.lane) s.lane = points;
    } else {
      const clip = project.clips[lane.owner.clip];
      if (!clip || clip.lane != null || clip.track !== track) continue;
      source(targetKey(lane.target)).envelopes.push({ clip, points });
    }
  }
  for (const s of index.values()) {
    if (s.envelopes.length > 1) {
      s.envelopes.sort((a, b) => a.clip.start - b.clip.start || compareOrderKeys(a.clip.id, b.clip.id));
    }
  }
  return index;
}

/** One-entry cache keyed by the tables the index reads (a patch elsewhere keeps it). */
let cached: { tables: unknown[]; index: Index } | null = null;

function indexOf(project: Project): Index {
  const tables = [project.automation_lanes, project.automation_points, project.clips, project.devices, project.sends, project.tracks];
  if (cached && cached.tables.every((t, i) => t === tables[i])) return cached.index;
  const index = buildIndex(project);
  cached = { tables, index };
  return index;
}

const enabledLanes = new WeakMap<object, Set<string>>();

/**
 * Does `target` have an enabled automation lane (arrangement or clip envelope, with or
 * without points)? One set lookup; the set is built once per lanes table.
 */
export function hasEnabledLane(project: Project, target: AutomationTarget): boolean {
  const lanes = project.automation_lanes;
  let set = enabledLanes.get(lanes);
  if (!set) {
    set = new Set(Object.values(lanes).flatMap((l) => (l.enabled ? [targetKey(l.target)] : [])));
    enabledLanes.set(lanes, set);
  }
  return set.has(targetKey(target));
}

/** Normalized value automation gives `target` at timeline beat `beat` (`null`: not automated). */
export function liveValueAt(project: Project, target: AutomationTarget, beat: number): number | null {
  const src = indexOf(project).get(targetKey(target));
  if (!src) return null;
  let out: number | null = null;
  for (const e of src.envelopes) {
    if (e.clip.muted) continue;
    const c = contentBeatAt(e.clip, beat);
    if (c === null) continue;
    const v = evaluatePoints(e.points, c);
    if (v !== null) out = v;
  }
  return out ?? (src.lane ? evaluatePoints(src.lane, beat) : null);
}

/** The position automation is evaluated at (the engine's playhead, 0 before the first frame). */
function currentBeat(): number {
  return playheadStore.getPlayhead()?.transport.position ?? 0;
}

const subscribeBoth = (listener: () => void): (() => void) => {
  const a = playheadStore.subscribePlayhead(listener);
  const b = useProjectStore.subscribe(listener);
  return () => {
    a();
    b();
  };
};

/**
 * Whether `device` runs now: when a macro or modulator switches it (`modulated`), the
 * engine's live switch state (modulation readback); else its "Device On" automation at the
 * playhead when automated (the engine switches it per block, overriding the document's
 * switch), else `device.enabled`.
 */
export function useDeviceOn(device: { id: DeviceId; enabled: boolean }): { on: boolean; automated: boolean; modulated: boolean } {
  const live = useLiveAutomation({ type: "DeviceEnabled", device: device.id });
  const modulated = useProjectStore(
    (s) => !!s.project && Object.values(s.project.mod_mappings).some((m) => m.device === device.id && m.param === DEVICE_ENABLED_PARAM),
  );
  const switched = useModulatedValue(device.id, DEVICE_ENABLED_PARAM, modulated);
  const base = live === null ? device.enabled : live >= 0.5;
  return { on: switched === null ? base : switched >= 0.5, automated: live !== null, modulated };
}

/**
 * The live normalized value of `target` (`null` when no automation drives it). Re-renders
 * only when the value changes, at most at the playhead rate.
 */
export function useLiveAutomation(target: AutomationTarget | null): number | null {
  const key = target ? targetKey(target) : null;
  const get = useCallback(() => {
    if (!target) return null;
    const project = useProjectStore.getState().project;
    return project ? liveValueAt(project, target, currentBeat()) : null;
    // `key` identifies `target`: a new object for the same target keeps the callback.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return useSyncExternalStore(subscribeBoth, get, get);
}

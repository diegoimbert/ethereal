/**
 * Automatable parameters of a track and their `ParamInfo` (normalized ↔ plain mapping and
 * display). Device params come from the engine's descriptors; mixer targets (volume, pan,
 * sends) use the same laws as the mixer UI.
 */

import { useEffect, useMemo, useState } from "react";
import type { AutomationTarget, Device, DeviceDescriptor, ParamInfo, Project, TrackId } from "@/generated";
import { compareOrderKeys, useProjectStore } from "@/state";
import { useTransport, type EngineTransport } from "@/transport";
import { descriptorKey, fetchDescriptor } from "@/features/devices/descriptors";
import { formatParam, paramToPlain, SILENCE_DB } from "@/features/devices/paramScale";

/** Top of the volume/send fader range (same as the mixer). */
export const MAX_DB = 6;

function mixerInfo(name: string, rest: Pick<ParamInfo, "unit" | "min" | "max" | "default" | "scale">): ParamInfo {
  return { id: 0, name, group: "Mixer", labels: null, automatable: true, hidden: false, ...rest };
}

/** Track volume / send level: dB with the fader law, `SILENCE_DB`..`MAX_DB`. */
export const VOLUME_INFO: ParamInfo = mixerInfo("Volume", {
  unit: "Decibels",
  min: SILENCE_DB,
  max: MAX_DB,
  default: 0,
  scale: { type: "Fader" },
});

/** Send level: same law and range as volume, default silence (controller `track_param_info`). */
export const SEND_INFO: ParamInfo = { ...VOLUME_INFO, name: "Send", default: SILENCE_DB };

export const PAN_INFO: ParamInfo = mixerInfo("Pan", { unit: "Pan", min: -1, max: 1, default: 0, scale: { type: "Linear" } });

/** Stable string key of a target (for UI state and React keys). */
export function targetKey(target: AutomationTarget): string {
  switch (target.type) {
    case "TrackVolume":
      return `volume:${target.track}`;
    case "TrackPan":
      return `pan:${target.track}`;
    case "SendLevel":
      return `send:${target.send}`;
    case "DeviceParam":
      return `param:${target.device}:${target.param}`;
  }
}

export function sameTarget(a: AutomationTarget, b: AutomationTarget): boolean {
  return targetKey(a) === targetKey(b);
}

/** An automatable parameter of a track. */
export interface TargetInfo {
  key: string;
  target: AutomationTarget;
  /** Display name ("Volume", "Send → Reverb", "Synth: Cutoff"). */
  name: string;
  /** Menu group ("Mixer" or the device name). */
  group: string;
  info: ParamInfo;
}

/** Descriptors per device id (missing while loading). */
export type DescriptorMap = ReadonlyMap<string, DeviceDescriptor>;

/** The project tables `trackTargets` reads. */
export type TargetTables = Pick<Project, "tracks" | "sends" | "devices">;

/**
 * Every automatable parameter of `track` in menu order: volume, pan, sends, then each
 * device's visible automatable params (devices whose descriptor is loaded).
 */
export function trackTargets(project: TargetTables, track: TrackId, descriptors: DescriptorMap): TargetInfo[] {
  const out: TargetInfo[] = [];
  const add = (target: AutomationTarget, name: string, group: string, info: ParamInfo) =>
    out.push({ key: targetKey(target), target, name, group, info });
  add({ type: "TrackVolume", track }, "Volume", "Mixer", VOLUME_INFO);
  add({ type: "TrackPan", track }, "Pan", "Mixer", PAN_INFO);
  const sends = Object.values(project.sends)
    .filter((s) => s.from === track)
    .sort((a, b) => compareOrderKeys(project.tracks[a.to]?.order ?? "", project.tracks[b.to]?.order ?? ""));
  for (const s of sends) {
    const to = project.tracks[s.to]?.name ?? "Return";
    add({ type: "SendLevel", send: s.id }, `Send → ${to}`, "Mixer", { ...SEND_INFO, name: `Send → ${to}` });
  }
  const devices = Object.values(project.devices)
    .filter((d) => d.track === track)
    .sort((a, b) => compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id));
  for (const d of devices) {
    const desc = descriptors.get(d.id);
    if (!desc) continue;
    for (const t of deviceTargets(d, desc)) out.push(t);
  }
  return out;
}

/** Per descriptor, per `device id + name`: its targets (plugins can have thousands). */
const deviceTargetCache = new WeakMap<DeviceDescriptor, Map<string, TargetInfo[]>>();

/**
 * The visible automatable params of one device as targets, cached: a param value change
 * (a new `Device` object, same descriptor and name) doesn't rebuild thousands of entries.
 */
function deviceTargets(d: Pick<Device, "id" | "name">, desc: DeviceDescriptor): ReadonlyArray<TargetInfo> {
  let byDevice = deviceTargetCache.get(desc);
  if (!byDevice) deviceTargetCache.set(desc, (byDevice = new Map()));
  const k = `${d.id}\u0000${d.name}`;
  let list = byDevice.get(k);
  if (!list) {
    list = [];
    for (const p of desc.params) {
      if (!p.automatable || p.hidden) continue;
      const target: AutomationTarget = { type: "DeviceParam", device: d.id, param: p.id };
      list.push({ key: targetKey(target), target, name: `${d.name}: ${p.name}`, group: d.name, info: p });
    }
    byDevice.set(k, list);
  }
  return list;
}

/** Info for one target, if it (still) exists. */
export function findTarget(targets: ReadonlyArray<TargetInfo>, key: string): TargetInfo | undefined {
  return targets.find((t) => t.key === key);
}

/** Plain value of a normalized automation value, formatted for display ("-6.0 dB"). */
export function formatNormalized(info: ParamInfo, normalized: number): string {
  return formatParam(info, paramToPlain(info, normalized));
}

const NO_DEVICES: Device[] = [];

/** Descriptors of the devices of `track` (fetched from the engine, cached per transport). */
export function useTrackDescriptors(track: TrackId): DescriptorMap {
  const transport = useTransport();
  const devices = useProjectStore((s) => s.project?.devices);
  const list = useMemo(() => (devices ? Object.values(devices).filter((d) => d.track === track) : NO_DEVICES), [devices, track]);
  const keys = list.map((d) => `${d.id}=${descriptorKey(d)}`).join(",");
  const [loaded, setLoaded] = useState<{ transport: EngineTransport; keys: string; map: DescriptorMap } | null>(null);
  useEffect(() => {
    let active = true;
    Promise.all(
      list.map((d) =>
        fetchDescriptor(transport, d).then(
          (desc) => [d.id, desc] as const,
          () => null,
        ),
      ),
    ).then((entries) => {
      if (!active) return;
      const map = new Map<string, DeviceDescriptor>();
      for (const e of entries) if (e) map.set(e[0], e[1]);
      setLoaded({ transport, keys, map });
    });
    return () => {
      active = false;
    };
    // `list` only matters through `keys`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [transport, keys]);
  return loaded && loaded.transport === transport && loaded.keys === keys ? loaded.map : EMPTY_MAP;
}

const EMPTY_MAP: DescriptorMap = new Map();

/** The automatable parameters of `track` (re-computed when the project or descriptors change). */
export function useTrackTargets(track: TrackId): TargetInfo[] {
  const descriptors = useTrackDescriptors(track);
  const tracks = useProjectStore((s) => s.project?.tracks);
  const sends = useProjectStore((s) => s.project?.sends);
  const devices = useProjectStore((s) => s.project?.devices);
  return useMemo(
    () => (tracks && sends && devices ? trackTargets({ tracks, sends, devices }, track, descriptors) : []),
    [tracks, sends, devices, track, descriptors],
  );
}

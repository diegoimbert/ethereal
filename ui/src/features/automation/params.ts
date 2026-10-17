/**
 * Automatable parameters of a track and their `ParamInfo` (normalized ↔ plain mapping and
 * display). Device params come from the engine's descriptors; mixer targets (volume, pan,
 * sends) use the same laws as the track volume fader (`arrangement/faderLaw.ts`).
 */

import { useEffect, useMemo, useState } from "react";
import type { AutomationTarget, Device, DeviceDescriptor, ParamInfo, Project, TrackId } from "@/generated";
import { compareOrderKeys, useProjectStore } from "@/state";
import { useTransport, type EngineTransport } from "@/transport";
import { descriptorKey, fetchDescriptor } from "@/features/devices/descriptors";
import { formatPan, formatParam, labelValue, paramToPlain, SILENCE_DB } from "@/domain/paramScale";
import { getLocale, localizeDescriptor, sourceParam, t as tr, useLocale } from "@/i18n";

/** Top of the volume/send fader range (same as the track volume fader). */
export const MAX_DB = 6;

function mixerInfo(name: string, rest: Pick<ParamInfo, "unit" | "min" | "max" | "default" | "scale">): ParamInfo {
  return { id: 0, name, group: "mixer", labels: null, automatable: true, hidden: false, ...rest };
}

/** Track volume / send level: dB with the fader law, `SILENCE_DB`..`MAX_DB`. */
export const VOLUME_INFO: ParamInfo = mixerInfo("volume", {
  unit: "Decibels",
  min: SILENCE_DB,
  max: MAX_DB,
  default: 0,
  scale: { type: "Fader" },
});

/** Send level: same law and range as volume, default silence (controller `track_param_info`). */
export const SEND_INFO: ParamInfo = { ...VOLUME_INFO, name: "send", default: SILENCE_DB };

export const PAN_INFO: ParamInfo = mixerInfo("pan", { unit: "Pan", min: -1, max: 1, default: 0, scale: { type: "Linear" } });

/**
 * Clip gain envelope of an audio clip (controller `CLIP_GAIN_MAPPING`): dB **added** to the
 * clip's gain, same fader law and range as track volume (the bottom is silence).
 */
export const CLIP_GAIN_INFO: ParamInfo = { ...VOLUME_INFO, name: "clip gain", group: "clip" };

/**
 * Transposition envelope of an audio clip (controller `CLIP_TRANSPOSE_MAPPING`): semitones
 * **added** to the clip's transpose, ±48. The engine takes any value (imported envelopes
 * can be fractional); editing snaps to whole semitones.
 */
export const CLIP_TRANSPOSE_INFO: ParamInfo = {
  id: 0,
  name: "transposition",
  group: "clip",
  unit: "Semitones",
  min: -48,
  max: 48,
  default: 0,
  scale: { type: "Linear" },
  labels: null,
  automatable: true,
  hidden: false,
  step: 1,
};

/** A device's on/off switch (controller `track_param_info`): 0/1, held between points. */
export const DEVICE_ON_INFO: ParamInfo = {
  id: 0,
  name: "device on",
  group: null,
  unit: "Toggle",
  min: 0,
  max: 1,
  default: 1,
  scale: { type: "Linear" },
  labels: ["Off", "On"],
  automatable: true,
  hidden: false,
};

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
    case "DeviceEnabled":
      return `on:${target.device}`;
    case "ClipGain":
      return "clip-gain";
    case "ClipTranspose":
      return "clip-transpose";
  }
}

export function sameTarget(a: AutomationTarget, b: AutomationTarget): boolean {
  return targetKey(a) === targetKey(b);
}

/** An automatable parameter of a track. */
export interface TargetInfo {
  key: string;
  target: AutomationTarget;
  /** Display name ("volume", "send → reverb", "synth: cutoff"), in the current language. */
  name: string;
  /** Menu group ("Mixer" or the device name). */
  group: string;
  info: ParamInfo;
}

/** Whether `t.name` holds a user name (a send's return track, a device), not only app text. */
export function targetNameIsUserText(t: Pick<TargetInfo, "target">): boolean {
  return !["TrackVolume", "TrackPan", "ClipGain", "ClipTranspose"].includes(t.target.type);
}

/** Descriptors per device id (missing while loading). */
export type DescriptorMap = ReadonlyMap<string, DeviceDescriptor>;

/** The project tables `trackTargets` reads. */
export type TargetTables = Pick<Project, "tracks" | "sends" | "devices">;

/**
 * Every automatable parameter of `track` in menu order: volume, pan, sends, then each
 * device's on/off switch and visible automatable params (devices whose descriptor is
 * loaded).
 */
export function trackTargets(project: TargetTables, track: TrackId, descriptors: DescriptorMap): TargetInfo[] {
  const out: TargetInfo[] = [];
  const add = (target: AutomationTarget, name: string, group: string, info: ParamInfo) => out.push({ key: targetKey(target), target, name, group, info });
  const mixer = tr("automation.target.mixer");
  const volume = tr("automation.target.volume");
  const pan = tr("automation.target.pan");
  add({ type: "TrackVolume", track }, volume, mixer, { ...VOLUME_INFO, name: volume });
  add({ type: "TrackPan", track }, pan, mixer, { ...PAN_INFO, name: pan });
  const sends = Object.values(project.sends)
    .filter((s) => s.from === track)
    .sort((a, b) => compareOrderKeys(project.tracks[a.to]?.order ?? "", project.tracks[b.to]?.order ?? ""));
  for (const s of sends) {
    const name = tr("automation.target.send", { to: project.tracks[s.to]?.name ?? tr("automation.target.return") });
    add({ type: "SendLevel", send: s.id }, name, mixer, { ...SEND_INFO, name });
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
  const k = `${d.id}\u0000${d.name}\u0000${getLocale()}`;
  let list = byDevice.get(k);
  if (!list) {
    // The device's on/off switch first.
    const on: AutomationTarget = { type: "DeviceEnabled", device: d.id };
    list = [{ key: targetKey(on), target: on, name: tr("automation.target.device_on", { device: d.name }), group: d.name, info: DEVICE_ON_INFO }];
    for (const p of desc.params) {
      if (!p.automatable || p.hidden) continue;
      const target: AutomationTarget = { type: "DeviceParam", device: d.id, param: p.id };
      list.push({ key: targetKey(target), target, name: tr("automation.target.param", { device: d.name, param: p.name }), group: d.name, info: p });
    }
    byDevice.set(k, list);
  }
  return list;
}

/** Plain value of a normalized automation value, formatted for display ("-6.0 dB"). */
export function formatNormalized(info: ParamInfo, normalized: number): string {
  return formatParam(info, paramToPlain(info, normalized));
}

// ---- Typed values ("Set Value…") ------------------------------------------------------

/** Unit suffixes typed after a number, per unit: suffix (lower case, no spaces) → factor. */
const TYPED_SUFFIXES: Partial<Record<ParamInfo["unit"], Record<string, number>>> = {
  Decibels: { db: 1 },
  Hertz: { hz: 1, k: 1000, khz: 1000 },
  Milliseconds: { ms: 1, s: 1000 },
  Seconds: { s: 1, ms: 0.001 },
  Percent: { "%": 1 },
  Semitones: { st: 1 },
  Cents: { ct: 1 },
  Ratio: { ":1": 1 },
};

/** Unit shown after the "Set Value…" field ("db", "hz"...); empty when the text says it (labels, pan, on/off). */
export function editUnit(info: ParamInfo): string {
  if (info.labels && info.labels.length > 0) return "";
  switch (info.unit) {
    case "Decibels":
      return "db";
    case "Hertz":
      return "hz";
    case "Milliseconds":
      return "ms";
    case "Seconds":
      return "s";
    case "Percent":
      return "%";
    case "Semitones":
      return "st";
    case "Cents":
      return "ct";
    case "Ratio":
      return ": 1";
    default:
      return "";
  }
}

/** Editable text of a plain value, in the param's display unit (`editUnit` shown beside it). */
export function editText(info: ParamInfo, plain: number): string {
  if (info.labels && info.labels.length > 0) return formatParam(info, plain);
  switch (info.unit) {
    case "Pan":
      return formatPan(plain);
    case "Toggle":
      return formatParam(info, plain);
    case "Decibels":
      if (plain <= SILENCE_DB) return "-inf";
      break;
  }
  return String(Number(plain.toFixed(2)));
}

/**
 * Plain value of typed text (the inverse of `editText`, clamped to the param's range), or
 * `null` when it isn't a value. Takes the display forms ("-6 dB", "1.2k", "25L", "C",
 * "-inf", "On", a label) as well as bare numbers in the display unit.
 */
export function parseEditText(info: ParamInfo, text: string): number | null {
  const lo = Math.min(info.min, info.max);
  const hi = Math.max(info.min, info.max);
  const clamp = (v: number) => Math.min(hi, Math.max(lo, v));
  const t = text.trim().toLowerCase().replace(/\s+/g, "");
  if (t === "") return null;
  const labels = info.labels;
  if (labels && labels.length > 0) {
    // The label as shown, or as the engine names it (English).
    const same = (l: string) => l.trim().toLowerCase().replace(/\s+/g, "") === t;
    const english = sourceParam(info).labels ?? labels;
    const i = Math.max(labels.findIndex(same), english.findIndex(same));
    if (i >= 0) return labels.length > 1 ? labelValue(info, i) : info.min;
  }
  if (info.unit === "Toggle") {
    if (t === "on" || t === "true") return hi;
    if (t === "off" || t === "false") return lo;
  }
  if (info.unit === "Pan") {
    if (t === "c" || t === "center" || t === "centre") return clamp(0);
    const m = /^(?:(\d+(?:[.,]\d+)?)([lr])|([lr])(\d+(?:[.,]\d+)?))$/.exec(t);
    if (m) {
      const n = Number((m[1] ?? m[4])!.replace(",", "."));
      const side = m[2] ?? m[3];
      return clamp(((side === "l" ? -1 : 1) * n) / 100);
    }
  }
  if (info.unit === "Decibels" && /^-?(inf|∞)(db)?$/.test(t)) return lo;
  const m = /^([+-]?(?:\d+(?:[.,]\d*)?|[.,]\d+))(.*)$/.exec(t);
  if (!m) return null;
  const n = Number(m[1]!.replace(",", "."));
  const suffix = m[2]!;
  if (!Number.isFinite(n)) return null;
  // Pan: typed in percent (-100 = full left), like its display.
  if (info.unit === "Pan") return suffix === "" || suffix === "%" ? clamp(n / 100) : null;
  const factor = suffix === "" ? 1 : TYPED_SUFFIXES[info.unit]?.[suffix];
  if (factor === undefined) return null;
  return clamp(n * factor);
}

const NO_DEVICES: Device[] = [];

/** Descriptors already fetched, per transport and `descriptorKey` (to answer without waiting). */
const resolved = new WeakMap<EngineTransport, Map<string, DeviceDescriptor>>();

function fetchAndRemember(transport: EngineTransport, device: Device): Promise<DeviceDescriptor> {
  return fetchDescriptor(transport, device).then((desc) => {
    let m = resolved.get(transport);
    if (!m) resolved.set(transport, (m = new Map()));
    m.set(descriptorKey(device), desc);
    return desc;
  });
}

/** The descriptors of `list` if all of them were fetched before, else `null`. */
function knownDescriptors(transport: EngineTransport, list: ReadonlyArray<Device>): DescriptorMap | null {
  if (list.length === 0) return EMPTY_MAP;
  const m = resolved.get(transport);
  if (!m) return null;
  const out = new Map<string, DeviceDescriptor>();
  for (const d of list) {
    const desc = m.get(descriptorKey(d));
    if (!desc) return null;
    out.set(d.id, desc);
  }
  return out;
}

/**
 * Descriptors of the devices of `track` (fetched from the engine, cached per transport).
 * Known ones are returned at once: the automation lanes mount as a track opens, and
 * shouldn't re-render once more when the (cached) descriptors resolve.
 */
export function useTrackDescriptors(track: TrackId): DescriptorMap {
  const transport = useTransport();
  const devices = useProjectStore((s) => s.project?.devices);
  const list = useMemo(() => (devices ? Object.values(devices).filter((d) => d.track === track) : NO_DEVICES), [devices, track]);
  const keys = list.map((d) => `${d.id}=${descriptorKey(d)}`).join(",");
  // eslint-disable-next-line react-hooks/exhaustive-deps -- `list` only matters through `keys`.
  const known = useMemo(() => knownDescriptors(transport, list), [transport, keys]);
  const [loaded, setLoaded] = useState<{ transport: EngineTransport; keys: string; map: DescriptorMap } | null>(null);
  useEffect(() => {
    if (known) return;
    let active = true;
    Promise.all(
      list.map((d) =>
        fetchAndRemember(transport, d).then(
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
  }, [transport, keys, known]);
  if (known) return known;
  return loaded && loaded.transport === transport && loaded.keys === keys ? loaded.map : EMPTY_MAP;
}

const EMPTY_MAP: DescriptorMap = new Map();

/** The automatable parameters of `track` (re-computed when the project or descriptors change), in the current language. */
export function useTrackTargets(track: TrackId): TargetInfo[] {
  const raw = useTrackDescriptors(track);
  const locale = useLocale();
  // eslint-disable-next-line react-hooks/exhaustive-deps -- `locale` only invalidates
  const descriptors = useMemo((): DescriptorMap => new Map([...raw].map(([id, d]) => [id, localizeDescriptor(d)])), [raw, locale]);
  const tracks = useProjectStore((s) => s.project?.tracks);
  const sends = useProjectStore((s) => s.project?.sends);
  const devices = useProjectStore((s) => s.project?.devices);
  return useMemo(
    () => (tracks && sends && devices ? trackTargets({ tracks, sends, devices }, track, descriptors) : []),
    [tracks, sends, devices, track, descriptors],
  );
}

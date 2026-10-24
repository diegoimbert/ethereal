/**
 * Device descriptors (param metadata) fetched from the engine: `Device::GetDescriptor`
 * per device and `Device::ListBuiltin` for the "add device" menu. Never hardcode param
 * ids: they come from here.
 *
 * Cached per transport. Built-ins are cached by type (all instances share a descriptor);
 * plugins by device id and plugin id.
 */

import { useEffect, useMemo, useState } from "react";
import type { BuiltinDevice, BuiltinDeviceType, Device, DeviceDescriptor } from "@/generated";
import { DEFAULT_LFO_CURVE, DEFAULT_SHAPER_CURVES } from "./layout/lfoCurve/lfoCurveMath";
import { localizeDescriptor, useLocale } from "@/i18n";
import { useProjectStore } from "@/state";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { cmd, useConnectionStatus, useTransport, type EngineTransport } from "@/transport";

const descriptorCache = new WeakMap<EngineTransport, Map<string, Promise<DeviceDescriptor>>>();
const builtinListCache = new WeakMap<EngineTransport, Promise<DeviceDescriptor[]>>();

export function descriptorKey(device: Device): string {
  if (device.kind.type === "Builtin") return `builtin:${device.kind.device.type}`;
  // A missing device's descriptor lists its source's parameters (per device: ranges may differ).
  if (device.kind.type === "Missing") return `missing:${device.id}:${device.kind.missing.source_id}`;
  return `plugin:${device.id}:${device.kind.plugin.plugin_id}`;
}

/** `epoch`: a plugin device's descriptor changed on the engine host (`usePluginStore.descriptorsChanged`). */
export function fetchDescriptor(transport: EngineTransport, device: Device, epoch = 0): Promise<DeviceDescriptor> {
  let cache = descriptorCache.get(transport);
  if (!cache) descriptorCache.set(transport, (cache = new Map()));
  const key = `${descriptorKey(device)}#${epoch}`;
  let p = cache.get(key);
  if (!p) {
    p = transport.send(cmd("Device", { type: "GetDescriptor", device: device.id })).then((reply) => {
      if (reply.type !== "Descriptor") throw new Error(`unexpected reply ${reply.type}`);
      return reply.descriptor;
    });
    // Don't cache failures (e.g. a plugin that is still loading).
    p.catch(() => cache.delete(key));
    cache.set(key, p);
  }
  return p;
}

export function fetchBuiltinTypes(transport: EngineTransport): Promise<DeviceDescriptor[]> {
  let p = builtinListCache.get(transport);
  if (!p) {
    p = transport.send(cmd("Device", { type: "ListBuiltin" })).then((reply) => {
      if (reply.type !== "DeviceTypes") throw new Error(`unexpected reply ${reply.type}`);
      return reply.devices;
    });
    p.catch(() => builtinListCache.delete(transport));
    builtinListCache.set(transport, p);
  }
  return p;
}

type Loaded<T> = { key: string; value: T } | { key: string; error: unknown };

/**
 * The descriptor of `device`, or `null` while loading / on error. A built-in's text is in the
 * current language (`localizeDescriptor`; re-renders on a switch).
 */
export function useDescriptor(device: Device): { descriptor: DeviceDescriptor | null; error: unknown } {
  const transport = useTransport();
  // A plugin held as a safe-mode placeholder has no instance (nor descriptor) yet;
  // it is fetched once loaded.
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  // A plugin played on another computer's engine: its descriptor comes from there, maybe later.
  const epoch = usePluginStore((s) => (device.kind.type === "Plugin" ? (s.descriptorEpochs[device.id] ?? 0) : 0));
  const key = `${descriptorKey(device)}#${epoch}`;
  const [loaded, setLoaded] = useState<Loaded<DeviceDescriptor> | null>(null);
  useEffect(() => {
    if (held) return;
    let active = true;
    fetchDescriptor(transport, device, epoch).then(
      (value) => active && setLoaded({ key, value }),
      (error: unknown) => active && setLoaded({ key, error }),
    );
    return () => {
      active = false;
    };
    // `device` only matters through `key` (and its id, captured in the key for plugins).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [transport, key, held]);
  const locale = useLocale();
  const value = loaded && "value" in loaded ? loaded.value : null;
  // eslint-disable-next-line react-hooks/exhaustive-deps -- `locale` only invalidates
  const localized = useMemo(() => value && localizeDescriptor(value), [value, locale]);
  if (held || !loaded || loaded.key !== key) return { descriptor: null, error: null };
  return "value" in loaded ? { descriptor: localized, error: null } : { descriptor: null, error: loaded.error };
}

const NO_TYPES: DeviceDescriptor[] = [];

/** Descriptors of every built-in device type (empty while loading), in the current language. */
export function useBuiltinTypes(): DeviceDescriptor[] {
  const transport = useTransport();
  // Asked again once connected: a transport may refuse what is sent while it connects.
  const connected = useConnectionStatus().status === "connected";
  const [types, setTypes] = useState<{ transport: EngineTransport; list: DeviceDescriptor[] } | null>(null);
  useEffect(() => {
    let active = true;
    fetchBuiltinTypes(transport).then(
      (list) => active && setTypes({ transport, list }),
      (e: unknown) => console.warn("[ethereal] ListBuiltin failed:", e),
    );
    return () => {
      active = false;
    };
  }, [transport, connected]);
  const locale = useLocale();
  const list = types?.transport === transport ? types.list : NO_TYPES;
  // eslint-disable-next-line react-hooks/exhaustive-deps -- `locale` only invalidates
  return useMemo(() => list.map(localizeDescriptor), [list, locale]);
}

/** The `BuiltinDevice` value to insert for a built-in type. */
export function builtinDevice(type: BuiltinDeviceType): BuiltinDevice {
  if (type === "Sampler") return { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } };
  if (type === "MultiSampler") return { type: "MultiSampler", zones: [] };
  if (type === "ExternalInstrument" || type === "ExternalAudioEffect") {
    return { type, routing: { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null } };
  }
  // A new convolution reverb starts on the factory hall (Rust `DEFAULT_CONVOLUTION_IR`).
  if (type === "ConvolutionReverb") return { type, ir: { type: "Factory", id: "hall" } };
  if (type === "VolumeLfo") return { type, curve: DEFAULT_LFO_CURVE };
  if (type === "Shaper") return { type, curves: DEFAULT_SHAPER_CURVES };
  return { type } as BuiltinDevice;
}

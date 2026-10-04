/**
 * Device descriptors (param metadata) fetched from the engine: `Device::GetDescriptor`
 * per device and `Device::ListBuiltin` for the "add device" menu. Never hardcode param
 * ids: they come from here.
 *
 * Cached per transport. Built-ins are cached by type (all instances share a descriptor);
 * plugins by device id and plugin id.
 */

import { useEffect, useState } from "react";
import type { BuiltinDevice, BuiltinDeviceType, Device, DeviceDescriptor } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, useTransport, type EngineTransport } from "@/transport";

const descriptorCache = new WeakMap<EngineTransport, Map<string, Promise<DeviceDescriptor>>>();
const builtinListCache = new WeakMap<EngineTransport, Promise<DeviceDescriptor[]>>();

export function descriptorKey(device: Device): string {
  return device.kind.type === "Builtin"
    ? `builtin:${device.kind.device.type}`
    : `plugin:${device.id}:${device.kind.plugin.plugin_id}`;
}

export function fetchDescriptor(transport: EngineTransport, device: Device): Promise<DeviceDescriptor> {
  let cache = descriptorCache.get(transport);
  if (!cache) descriptorCache.set(transport, (cache = new Map()));
  const key = descriptorKey(device);
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

/** The descriptor of `device`, or `null` while loading / on error. */
export function useDescriptor(device: Device): { descriptor: DeviceDescriptor | null; error: unknown } {
  const transport = useTransport();
  // base-131: a plugin held as a safe-mode placeholder has no instance (nor descriptor) yet;
  // it is fetched once loaded.
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  const key = descriptorKey(device);
  const [loaded, setLoaded] = useState<Loaded<DeviceDescriptor> | null>(null);
  useEffect(() => {
    if (held) return;
    let active = true;
    fetchDescriptor(transport, device).then(
      (value) => active && setLoaded({ key, value }),
      (error: unknown) => active && setLoaded({ key, error }),
    );
    return () => {
      active = false;
    };
    // `device` only matters through `key` (and its id, captured in the key for plugins).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [transport, key, held]);
  if (held || !loaded || loaded.key !== key) return { descriptor: null, error: null };
  return "value" in loaded ? { descriptor: loaded.value, error: null } : { descriptor: null, error: loaded.error };
}

/** Descriptors of every built-in device type (empty while loading). */
export function useBuiltinTypes(): DeviceDescriptor[] {
  const transport = useTransport();
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
  }, [transport]);
  return types?.transport === transport ? types.list : [];
}

/** The `BuiltinDevice` value to insert for a built-in type. */
export function builtinDevice(type: BuiltinDeviceType): BuiltinDevice {
  if (type === "Sampler") return { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } };
  if (type === "MultiSampler") return { type: "MultiSampler", zones: [] };
  if (type === "ExternalInstrument" || type === "ExternalAudioEffect") {
    return { type, routing: { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null } };
  }
  if (type === "ConvolutionReverb") return { type, ir: null };
  return { type } as BuiltinDevice;
}

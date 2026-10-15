/**
 * The plugin's own display text for a param value ("-6.0 dB", "Saw") in the generic device
 * UI (`Plugin::ParamTexts`): CLAP `value_to_text`, VST3 `getParamStringByValue` + units, VST2
 * `effGetParamDisplay` + label. Without one (web host, AU, a plugin with no text, a VST2 value
 * not applied yet) the UI formats the number itself.
 *
 * Requests are batched per device (one command per `BATCH_MS`), throttled per control while
 * the value moves (`THROTTLE_MS`, trailing call included), and texts are cached per value.
 * VST2 only describes the value it holds, which the engine applies a block after the edit,
 * so a missing text is asked again once (`RETRY_MS`).
 */

import { useContext, useEffect, useRef, useState } from "react";
import type { DeviceId, ParamId, PluginParamValue } from "@/generated";
import { cmd, isCommandFailed, TransportContext, type EngineTransport } from "@/transport";

/** Requests of all controls within this window go out as one command per device. */
export const BATCH_MS = 16;
/** At most one request per control per this interval while its value changes. */
export const THROTTLE_MS = 60;
/** A value that had no text is asked again once after this delay. */
export const RETRY_MS = 250;
/** Cached texts kept before the cache is cleared. */
const CACHE_LIMIT = 4096;

interface Pending {
  value: number;
  param: ParamId;
  resolve: (text: string | null) => void;
}

/** Batches and caches `ParamTexts` requests for one transport. */
export class ParamTextSource {
  /** The host has no param texts (web, mock): every request resolves `null`. */
  unsupported = false;
  private readonly cache = new Map<string, string>();
  private readonly pending = new Map<DeviceId, Pending[]>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly transport: EngineTransport) {}

  /** The cached text of `value`, if any. */
  cached(device: DeviceId, param: ParamId, value: number): string | undefined {
    return this.cache.get(key(device, param, value));
  }

  text(device: DeviceId, param: ParamId, value: number): Promise<string | null> {
    if (this.unsupported) return Promise.resolve(null);
    const hit = this.cached(device, param, value);
    if (hit !== undefined) return Promise.resolve(hit);
    return new Promise((resolve) => {
      const list = this.pending.get(device) ?? [];
      list.push({ param, value, resolve });
      this.pending.set(device, list);
      this.timer ??= setTimeout(() => this.flush(), BATCH_MS);
    });
  }

  private flush(): void {
    this.timer = null;
    const batches = [...this.pending];
    this.pending.clear();
    for (const [device, list] of batches) {
      const values: PluginParamValue[] = list.map(({ param, value }) => ({ param, value }));
      this.transport
        .send(cmd("Plugin", { type: "ParamTexts", device, values }))
        .then((reply) => {
          const texts = reply.type === "ParamTexts" ? reply.texts : [];
          list.forEach((p, i) => {
            const text = texts[i] ?? null;
            if (text !== null) this.remember(device, p.param, p.value, text);
            p.resolve(text);
          });
        })
        .catch((e: unknown) => {
          if (isCommandFailed(e, "Unsupported")) this.unsupported = true;
          for (const p of list) p.resolve(null);
        });
    }
  }

  private remember(device: DeviceId, param: ParamId, value: number, text: string): void {
    if (this.cache.size >= CACHE_LIMIT) this.cache.clear();
    this.cache.set(key(device, param, value), text);
  }
}

function key(device: DeviceId, param: ParamId, value: number): string {
  return `${device}|${param}|${value}`;
}

const sources = new WeakMap<EngineTransport, ParamTextSource>();

/** The shared source of `transport`. */
export function paramTextSource(transport: EngineTransport): ParamTextSource {
  let s = sources.get(transport);
  if (!s) {
    s = new ParamTextSource(transport);
    sources.set(transport, s);
  }
  return s;
}

/**
 * The plugin's text for `plain` of `param` (`enabled`: the device is a plugin), or `null`
 * (format the number). While a new value's text is on its way the previous text is kept,
 * so a dragged knob never flickers back to a bare number.
 */
export function usePluginParamText(device: DeviceId, param: ParamId, plain: number, enabled: boolean): string | null {
  const transport = useContext(TransportContext)?.transport ?? null;
  // Hosts without plugin texts (web, mock) reply `Unsupported` once; the source then stops asking.
  const source = enabled && transport ? paramTextSource(transport) : null;
  // The last text fetched (cached values are read during render).
  const [text, setText] = useState<string | null>(null);
  const last = useRef(0);

  useEffect(() => {
    if (!source || source.unsupported || source.cached(device, param, plain) !== undefined) return;
    let live = true;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const fetch = (retry: boolean) => {
      last.current = Date.now();
      void source.text(device, param, plain).then((t) => {
        if (!live) return;
        if (t === null && retry && !source.unsupported) timers.push(setTimeout(() => fetch(false), RETRY_MS));
        else setText(t);
      });
    };
    timers.push(setTimeout(() => fetch(true), Math.max(0, last.current + THROTTLE_MS - Date.now())));
    return () => {
      live = false;
      timers.forEach(clearTimeout);
    };
  }, [source, device, param, plain]);

  if (!source || source.unsupported) return null;
  return source.cached(device, param, plain) ?? text;
}

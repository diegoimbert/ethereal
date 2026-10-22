/**
 * Settings "sandboxed by default" (`Plugin::ListSandboxDefaults`): the plugins the engine
 * host inserts sandboxed. The engine host keeps the list (per machine, next to its plugin
 * list) and applies it itself when a new instance is created, so this is only what settings
 * and the device menu show and edit. `list` stays `null` until loaded, and when the engine
 * host has no plugins (the web replies `Unsupported`): nothing offers the setting then.
 */
import { create } from "zustand";
import type { PluginFormat, SandboxDefault } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

interface SandboxDefaultsState {
  list: SandboxDefault[] | null;
  /** The transport `list` came from (a new connection loads again). */
  from: EngineTransport | null;
  /** The engine host has no such setting (or the load failed). */
  unsupported: boolean;
  /** Load once per transport (`force`: again). */
  load(transport: EngineTransport, force?: boolean): Promise<void>;
  /** Add (`on`) or remove a plugin; the reply is the new list. */
  set(transport: EngineTransport, plugin: { format: PluginFormat; plugin_id: string; name?: string }, on: boolean): Promise<void>;
  reset(): void;
}

const pending = new WeakMap<EngineTransport, Promise<void>>();

export const useSandboxDefaults = create<SandboxDefaultsState>()((set, get) => ({
  list: null,
  from: null,
  unsupported: false,
  load(transport, force = false) {
    if (!force && get().from === transport) return Promise.resolve();
    const inflight = pending.get(transport);
    if (inflight && !force) return inflight;
    const p = transport
      .send(cmd("Plugin", { type: "ListSandboxDefaults" }))
      .then((reply) => {
        if (reply.type === "SandboxDefaults") set({ list: reply.plugins, from: transport, unsupported: false });
        else set({ list: null, from: transport, unsupported: true });
      })
      .catch(() => set({ list: null, from: transport, unsupported: true }))
      .finally(() => pending.delete(transport));
    pending.set(transport, p);
    return p;
  },
  async set(transport, plugin, on) {
    const reply = await transport.send(
      cmd("Plugin", { type: "SetSandboxDefault", format: plugin.format, plugin_id: plugin.plugin_id, name: plugin.name, sandboxed: on }),
    );
    if (reply.type === "SandboxDefaults") set({ list: reply.plugins, from: transport, unsupported: false });
  },
  reset: () => set({ list: null, from: null, unsupported: false }),
}));

/** Whether `plugin` is in `list` (ids compare case-insensitively, per format, like the host). */
export function isSandboxDefault(list: ReadonlyArray<SandboxDefault> | null, plugin: { format: PluginFormat; plugin_id: string }): boolean {
  const id = plugin.plugin_id.toLowerCase();
  return !!list?.some((d) => d.format === plugin.format && d.plugin_id.toLowerCase() === id);
}

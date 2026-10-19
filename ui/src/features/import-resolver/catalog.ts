/**
 * The installed plugins the resolver matches against. The engine's controller has no
 * plugin catalog (the host scans plugins), so the UI passes it with `Resolve::Propose`:
 *
 * - desktop (`tauri`): the scanned list (`Plugin::List`, shared with the plugin store);
 * - mock engine: its fake plugins ([`MOCK_INSTALLED_PLUGINS`]);
 * - another computer's engine plays the project (`usePluginStore.remote`): that computer's list;
 * - web / remote: no plugin host. The e2e suite can stand in an installed list with
 *   `window.__etherResolverPlugins` (set by an init script) to exercise the real engine's
 *   matching; without it the resolver lists audio files only.
 */
import type { Device, PluginDescriptor } from "@/generated";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { MOCK_INSTALLED_PLUGINS } from "@/transport/mock/roadmap/resolver";
import { cmd, type EngineTransport } from "@/transport";

/** e2e seam (see the module comment). */
function seeded(): PluginDescriptor[] | null {
  const w = typeof window === "undefined" ? null : (window as unknown as { __etherResolverPlugins?: unknown }).__etherResolverPlugins;
  return Array.isArray(w) ? (w as PluginDescriptor[]) : null;
}

/** The installed plugins known right now (`null` = this host has no plugin catalog). */
export function catalogNow(transport: EngineTransport | null | undefined): PluginDescriptor[] | null {
  if (!transport) return null;
  const remote = usePluginStore.getState().remote;
  if (remote) return remote.plugins;
  if (transport.kind === "tauri") return usePluginStore.getState().plugins;
  if (transport.kind === "mock") return MOCK_INSTALLED_PLUGINS;
  return seeded();
}

/** The installed plugins, fetched when needed (`null` = no plugin catalog on this host). */
export async function loadCatalog(transport: EngineTransport): Promise<PluginDescriptor[] | null> {
  if (transport.kind === "tauri" && !usePluginStore.getState().remote) {
    const known = usePluginStore.getState().plugins;
    if (known) return known;
    const reply = await transport.send(cmd("Plugin", { type: "List" }));
    if (reply.type !== "Plugins") return [];
    usePluginStore.getState().setPlugins(reply.plugins);
    return reply.plugins;
  }
  return catalogNow(transport);
}

const key = (format: string, id: string) => `${format}:${id.toUpperCase()}`;

/** Is `device` a plugin that isn't installed (per `catalog`)? `false` when unknown. */
export function isMissingPlugin(device: Device, catalog: PluginDescriptor[] | null): boolean {
  if (device.kind.type !== "Plugin" || !catalog) return false;
  const p = device.kind.plugin;
  const k = key(p.format, p.plugin_id);
  return !catalog.some((d) => key(d.format, d.id) === k);
}

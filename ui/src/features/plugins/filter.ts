import type { Command, Device, DeviceId, PluginDescriptor, Track } from "@/generated";
import { cmd } from "@/transport";

/**
 * Plugins matching `query` (every whitespace-separated term must appear in the name,
 * vendor, category or a feature; case-insensitive), sorted by name then vendor.
 */
export function filterPlugins(plugins: readonly PluginDescriptor[], query: string): PluginDescriptor[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  return plugins
    .filter((p) => {
      const hay = [p.name, p.vendor, p.category, ...p.features].join(" ").toLowerCase();
      return terms.every((t) => hay.includes(t));
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.vendor.localeCompare(b.vendor));
}

/** Instruments and note effects only go on MIDI tracks (the engine rejects them elsewhere). */
export function canInsert(plugin: PluginDescriptor, track: Track | undefined): boolean {
  if (!track) return false;
  return plugin.category === "AudioEffect" || track.kind === "Midi";
}

/**
 * `Device::Insert` for `plugin` on `track`: instruments at the start of the chain (their
 * output feeds the effects), everything else at the end.
 */
export function insertCommand(
  plugin: PluginDescriptor,
  track: Track,
  chain: readonly Device[],
  id: DeviceId,
): Command {
  return cmd("Device", {
    type: "Insert",
    id,
    track: track.id,
    device: { type: "Plugin", plugin_id: plugin.id, sandboxed: null },
    before: plugin.category === "Instrument" ? (chain[0]?.id ?? null) : null,
  });
}

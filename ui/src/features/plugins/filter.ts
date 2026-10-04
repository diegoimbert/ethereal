import type { Command, Device, DeviceId, PluginDescriptor, PluginFormat, Track } from "@/generated";
import { cmd } from "@/transport";

/** Display names of the plugin formats, in browser filter order. */
export const FORMAT_LABEL: Record<PluginFormat, string> = {
  Clap: "CLAP",
  Vst3: "VST3",
  Vst2: "VST2",
  Au: "AU",
};

/** The browser's format filter: one format, or every format. */
export type FormatFilter = PluginFormat | "All";

/** Plugin ids are only unique per format: key plugins by both. */
export function pluginKey(p: { format: PluginFormat; id: string }): string {
  return `${p.format}:${p.id}`;
}

/**
 * Plugins of `format` (default: every format) matching `query` (every whitespace-separated
 * term must appear in the name, vendor, category, format or a feature; case-insensitive),
 * sorted by name, then vendor, then format.
 */
export function filterPlugins(
  plugins: readonly PluginDescriptor[],
  query: string,
  format: FormatFilter = "All",
): PluginDescriptor[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  return plugins
    .filter((p) => {
      if (format !== "All" && p.format !== format) return false;
      const hay = [p.name, p.vendor, p.category, FORMAT_LABEL[p.format], ...p.features].join(" ").toLowerCase();
      return terms.every((t) => hay.includes(t));
    })
    .sort(
      (a, b) =>
        a.name.localeCompare(b.name) ||
        a.vendor.localeCompare(b.vendor) ||
        FORMAT_LABEL[a.format].localeCompare(FORMAT_LABEL[b.format]),
    );
}

/** Whether a device's plugin, `(format, plugin_id)`, is in the scanned list. */
export function isInstalled(
  plugins: readonly PluginDescriptor[],
  plugin: { format: PluginFormat; plugin_id: string },
): boolean {
  return plugins.some((p) => p.format === plugin.format && p.id === plugin.plugin_id);
}

/** Instruments and note effects only go on MIDI tracks (the engine rejects them elsewhere). */
export function canInsert(plugin: PluginDescriptor, track: Track | undefined): boolean {
  if (!track) return false;
  return plugin.category === "AudioEffect" || track.kind === "Midi";
}

/**
 * `Device::Insert` for `plugin` on `track` (with its format: ids are only unique per
 * format): instruments at the start of the chain (their output feeds the effects),
 * everything else at the end.
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
    device: { type: "Plugin", plugin_id: plugin.id, format: plugin.format, sandboxed: null },
    before: plugin.category === "Instrument" ? (chain[0]?.id ?? null) : null,
  });
}

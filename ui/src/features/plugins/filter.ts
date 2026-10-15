import type { PluginDescriptor, PluginFormat } from "@/generated";

/** Display names of the plugin formats (shown lowercase, like all the app's own text). */
export const FORMAT_LABEL: Record<PluginFormat, string> = {
  Clap: "CLAP",
  Vst3: "VST3",
  Vst2: "VST2",
  Au: "AU",
};

/** The plugin browser's format toggles, in order. */
export const FORMAT_ORDER: readonly PluginFormat[] = ["Clap", "Vst2", "Vst3", "Au"];

/** The formats this platform hosts: AU on macOS only. */
export function platformFormats(mac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform)): PluginFormat[] {
  return FORMAT_ORDER.filter((f) => f !== "Au" || mac);
}

/** Plugin ids are only unique per format: key plugins by both. */
export function pluginKey(p: { format: PluginFormat; id: string }): string {
  return `${p.format}:${p.id}`;
}

/** Whether a device's plugin, `(format, plugin_id)`, is in the scanned list. */
export function isInstalled(plugins: readonly PluginDescriptor[], plugin: { format: PluginFormat; plugin_id: string }): boolean {
  return plugins.some((p) => p.format === plugin.format && p.id === plugin.plugin_id);
}

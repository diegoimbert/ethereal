import type { PluginDescriptor, PluginFormat } from "@/generated";

/** Display names of the plugin formats (shown lowercase, like all the app's own text). */
export const FORMAT_LABEL: Record<PluginFormat, string> = {
  Clap: "clap",
  Vst3: "vst3",
  Vst2: "vst2",
  Au: "au",
  Webclap: "webclap",
};

/** The plugin browser's format toggles, in order. */
export const FORMAT_ORDER: readonly PluginFormat[] = ["Clap", "Vst2", "Vst3", "Au", "Webclap"];

/**
 * The format toggles this platform may show: AU on macOS only. WebCLAP has none (the web
 * version lists that format only; the desktop never lists it).
 */
export function platformFormats(mac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform)): PluginFormat[] {
  return FORMAT_ORDER.filter((f) => f !== "Webclap" && (f !== "Au" || mac));
}

/** Plugin ids are only unique per format: key plugins by both. */
export function pluginKey(p: { format: PluginFormat; id: string }): string {
  return `${p.format}:${p.id}`;
}

/** Whether a device's plugin, `(format, plugin_id)`, is in the scanned list. */
export function isInstalled(plugins: readonly PluginDescriptor[], plugin: { format: PluginFormat; plugin_id: string }): boolean {
  return plugins.some((p) => p.format === plugin.format && p.id === plugin.plugin_id);
}

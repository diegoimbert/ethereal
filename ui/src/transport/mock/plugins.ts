/**
 * Fake plugins of the mock engine (base-132): not listed by `Plugin::List` (the mock still
 * has no plugin browser), but `Device::Insert` with one of these ids adds a plugin device
 * and `Device::GetDescriptor` describes it. "Mock Mega" has 10,000 params, for the capped
 * device card and the windowed parameter lists (tests, e2e, `?mock` web build).
 */

import type { DeviceDescriptor, ParamInfo, PluginFormat, PluginInstance } from "@/generated";

export const MOCK_MEGA_PLUGIN_ID = "dev.ethereal.mock.mega";
export const MOCK_MEGA_PARAM_COUNT = 10_000;

const KINDS = ["Gain", "Cutoff", "Resonance", "Attack", "Decay", "Release", "Mix", "Detune"] as const;
const GROUPS = ["Oscillator", "Filter", "Envelope", "Modulation", "Effects"] as const;
/** Params `REMOTE_FIRST..+8` are the plugin's own quick controls (CLAP remote-controls page 1). */
export const MOCK_MEGA_REMOTE_FIRST = 4_321;

/** Param `i` of Mock Mega: "Cutoff 42" (group by blocks of 2,000; every 1,000th is hidden). */
export function megaParam(i: number): ParamInfo {
  const kind = KINDS[i % KINDS.length]!;
  const remote = i >= MOCK_MEGA_REMOTE_FIRST && i < MOCK_MEGA_REMOTE_FIRST + 8 ? i - MOCK_MEGA_REMOTE_FIRST : null;
  return {
    id: i,
    name: `${kind} ${i}`,
    group: GROUPS[Math.floor(i / 2_000) % GROUPS.length]!,
    unit: kind === "Gain" ? "Decibels" : kind === "Mix" ? "Percent" : "None",
    min: kind === "Gain" ? -24 : 0,
    max: kind === "Gain" ? 24 : kind === "Mix" ? 100 : 1,
    default: kind === "Mix" ? 100 : 0,
    scale: { type: "Linear" },
    labels: null,
    automatable: i % 7 !== 3,
    hidden: i > 0 && i % 1_000 === 0,
    ...(remote !== null ? { remote } : {}),
  };
}

let mega: DeviceDescriptor | null = null;

function megaDescriptor(): DeviceDescriptor {
  mega ??= {
    device_type: { type: "Plugin", plugin_id: MOCK_MEGA_PLUGIN_ID },
    name: "Mock Mega",
    category: "AudioEffect",
    params: Array.from({ length: MOCK_MEGA_PARAM_COUNT }, (_, i) => megaParam(i)),
    audio_inputs: 2,
    audio_outputs: 2,
    midi_input: false,
    sidechain_inputs: 0,
  };
  return mega;
}

/** Descriptor of a mock plugin, or `null` for an unknown id. */
export function mockPluginDescriptor(pluginId: string): DeviceDescriptor | null {
  return pluginId === MOCK_MEGA_PLUGIN_ID ? megaDescriptor() : null;
}

/** The `PluginInstance` of a new mock plugin device. */
export function mockPluginInstance(pluginId: string, format: PluginFormat): PluginInstance {
  const desc = mockPluginDescriptor(pluginId);
  return {
    format,
    plugin_id: pluginId,
    name: desc?.name ?? pluginId,
    vendor: "Ethereal",
    version: "1.0.0",
    sandboxed: false,
    state: null,
  };
}

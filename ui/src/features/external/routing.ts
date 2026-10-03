/**
 * Pure helpers of the external devices' routing panel (`external-instrument`): channel
 * pickers (mono channels and stereo pairs), MIDI channel options, missing-port detection.
 */

import type { AudioInputChannel, BuiltinDevice, Device, ExternalRouting, HardwarePorts, HwChannels } from "@/generated";

export type ExternalKind = "ExternalInstrument" | "ExternalAudioEffect";

export interface PickerOption {
  value: string;
  label: string;
  group?: string;
}

/** Select value of "nothing routed". */
export const NONE = "none";

/** The routing of an external device, with its kind. */
export function externalOf(device: Device): { kind: ExternalKind; routing: ExternalRouting } | null {
  const k = device.kind;
  if (k.type !== "Builtin") return null;
  const d: BuiltinDevice = k.device;
  if (d.type === "ExternalInstrument" || d.type === "ExternalAudioEffect") return { kind: d.type, routing: d.routing };
  return null;
}

/** Select value of a channel range (`"<first>:<count>"`, or [`NONE`]). */
export function channelsValue(ch: HwChannels | null): string {
  return ch ? `${ch.first}:${ch.count}` : NONE;
}

/** Parse a [`channelsValue`]. */
export function parseChannels(value: string): HwChannels | null {
  if (value === NONE) return null;
  const [first, count] = value.split(":").map(Number);
  if (!Number.isInteger(first) || !Number.isInteger(count)) return null;
  return { first: first!, count: count! };
}

const channelName = (channels: ReadonlyArray<AudioInputChannel>, index: number) =>
  channels.find((c) => c.index === index)?.name ?? `${index + 1}`;

/** Label of a channel range: `In 1`, `In 1/2` (names from the host when known). */
export function channelsLabel(ch: HwChannels, channels: ReadonlyArray<AudioInputChannel>, prefix: string): string {
  const name = (i: number) => {
    const known = channels.find((c) => c.index === i)?.name;
    return known ?? `${prefix} ${i + 1}`;
  };
  if (ch.count < 2) return name(ch.first);
  const a = name(ch.first);
  const b = name(ch.first + 1);
  // "In 1" + "In 2" → "In 1/2"; otherwise both names.
  const common = a.replace(/\d+$/, "");
  return common && b.startsWith(common) && /\d+$/.test(b) ? `${a}/${b.slice(common.length)}` : `${a} + ${b}`;
}

/**
 * Options for a channel picker: "None", then stereo pairs (1/2, 3/4, …) and mono channels.
 * The current value is kept (marked missing) when the host doesn't have it.
 */
export function channelOptions(
  channels: ReadonlyArray<AudioInputChannel>,
  current: HwChannels | null,
  prefix: string,
  missing: string | null,
): PickerOption[] {
  const out: PickerOption[] = [{ value: NONE, label: "None" }];
  const sorted = [...channels].sort((a, b) => a.index - b.index);
  for (let i = 0; i + 1 < sorted.length; i += 2) {
    const ch = { first: sorted[i]!.index, count: 2 };
    if (sorted[i + 1]!.index !== ch.first + 1) continue;
    out.push({ value: channelsValue(ch), label: channelsLabel(ch, channels, prefix), group: "Stereo" });
  }
  for (const c of sorted) {
    out.push({ value: channelsValue({ first: c.index, count: 1 }), label: channelName(channels, c.index), group: "Mono" });
  }
  if (current && !out.some((o) => o.value === channelsValue(current))) {
    const label = channelsLabel(current, [], prefix);
    out.push({ value: channelsValue(current), label: missing ? `${label} (${missing})` : label });
  }
  return out;
}

/** MIDI channel options 1..=16. */
export const MIDI_CHANNELS: PickerOption[] = Array.from({ length: 16 }, (_, i) => ({ value: `${i + 1}`, label: `Ch ${i + 1}` }));

/** MIDI output options; the stored port is kept (marked `missing`, if given) when not listed. */
export function midiOptions(ports: HardwarePorts | null, current: string | null, missing: string | null): PickerOption[] {
  const out: PickerOption[] = [{ value: NONE, label: "None" }];
  for (const p of ports?.midi_outputs ?? []) out.push({ value: p.id, label: p.name });
  if (current !== null && !out.some((o) => o.value === current)) out.push({ value: current, label: missing ? `${current} (${missing})` : current });
  return out;
}

/** Parts of `routing` this host can't reach (shown as missing; the device is silent there). */
export function missingParts(routing: ExternalRouting, ports: HardwarePorts | null): string[] {
  if (!ports) return [];
  const out: string[] = [];
  if (routing.midi_out !== null && !ports.midi_outputs.some((p) => p.id === routing.midi_out)) out.push(`MIDI "${routing.midi_out}"`);
  const has = (list: ReadonlyArray<AudioInputChannel>, ch: HwChannels) =>
    Array.from({ length: ch.count }, (_, i) => ch.first + i).every((i) => list.some((c) => c.index === i));
  if (routing.audio_send && !has(ports.audio_outputs, routing.audio_send)) out.push("audio send");
  if (routing.audio_return && !has(ports.audio_inputs, routing.audio_return)) out.push("audio return");
  return out;
}

/** Whether the routing has what a measurement needs (output side and return). */
export function canMeasure(kind: ExternalKind, routing: ExternalRouting): boolean {
  const out = kind === "ExternalInstrument" ? routing.midi_out !== null : routing.audio_send !== null;
  return out && routing.audio_return !== null;
}

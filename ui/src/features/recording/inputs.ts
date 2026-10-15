// Pure helpers of the recording controls: input choices per track, count-in choices.
import type { InputList, MonitorMode, TrackInput, TrackKind } from "@/generated";
import { channelsLabel } from "@/features/external/routing";
import { t, type MessageKey } from "@/i18n";

export interface InputOption {
  /** Stable `<option>` value (see `inputValue`). */
  value: string;
  label: string;
  /** The label is a device's channel / port name (keeps its case). */
  userText?: boolean;
  input: TrackInput;
}

export const COUNT_IN_CHOICES: readonly number[] = [0, 1, 2, 4];
export const MONITOR_MODES: readonly MonitorMode[] = ["Auto", "In", "Off"];

/** The monitoring modes' names (`MONITOR_MODES`). */
export const MONITOR_LABELS: Record<MonitorMode, MessageKey> = { Auto: "recording.monitor.auto", In: "recording.monitor.in", Off: "recording.monitor.off" };

const allPorts = () => t("recording.input.all_ports");

/** Choices of a MIDI track's channel filter: all channels (`"*"`) or 1–16 (`"0"`–`"15"`, 0-based). */
export function midiChannelOptions(): { value: string; label: string }[] {
  return [
    { value: "*", label: t("recording.input.all_channels") },
    ...Array.from({ length: 16 }, (_, i) => ({ value: `${i}`, label: t("recording.input.channel", { n: i + 1 }) })),
  ];
}

/** `midiChannelOptions` value of a MIDI input's channel filter. */
export function midiChannelValue(input: TrackInput): string {
  return input.type === "Midi" && input.channel !== null ? `${input.channel}` : "*";
}

/** `input` (a MIDI input) listening to the channel of a `midiChannelOptions` value; same port. */
export function withMidiChannel(input: TrackInput, value: string): TrackInput {
  const port = input.type === "Midi" ? input.port : null;
  return { type: "Midi", port, channel: value === "*" ? null : Number(value) };
}

export function countInLabel(bars: number): string {
  if (bars === 0) return t("common.off");
  return t("recording.count_in_bars", { count: bars });
}

/** Serialize a track input to an `<option>` value. */
export function inputValue(input: TrackInput): string {
  switch (input.type) {
    case "None":
      return "none";
    case "Audio":
      return `audio:${input.first}:${input.count}`;
    case "Midi":
      // The channel has its own select (`midiChannelOptions`).
      return `midi:${input.port ?? "*"}`;
    case "Track":
      return `track:${input.track}`;
  }
}

/**
 * Inputs a track can record from: for audio tracks each mono channel and each odd/even
 * stereo pair (1/2, 3/4, …) of the engine's input list (every channel of the interface), for MIDI tracks all ports or one port
 * (keeping the current channel filter).
 * The track's current input is always included, even if the device is gone.
 */
export function inputOptions(kind: TrackKind, inputs: InputList | null, current: TrackInput): InputOption[] {
  const options: InputOption[] = [{ value: "none", label: t("recording.input.none"), input: { type: "None" } }];
  if (kind === "Audio") {
    const channels = inputs?.audio ?? [];
    for (const ch of channels) {
      options.push({ value: `audio:${ch.index}:1`, label: ch.name, userText: true, input: { type: "Audio", first: ch.index, count: 1 } });
    }
    for (let i = 0; i + 1 < channels.length; i += 2) {
      const a = channels[i]!;
      const b = channels[i + 1]!;
      if (b.index !== a.index + 1) continue;
      options.push({
        value: `audio:${a.index}:2`,
        // Ableton-style "1/2", "3/4" for numbered interface inputs.
        label: channelsLabel({ first: a.index, count: 2 }, channels, "in"),
        userText: true,
        input: { type: "Audio", first: a.index, count: 2 },
      });
    }
  } else if (kind === "Midi") {
    const channel = current.type === "Midi" ? current.channel : null;
    options.push({ value: "midi:*", label: allPorts(), input: { type: "Midi", port: null, channel } });
    for (const port of inputs?.midi ?? []) {
      options.push({ value: `midi:${port.id}`, label: port.name, userText: true, input: { type: "Midi", port: port.id, channel } });
    }
  }
  const value = inputValue(current);
  if (!options.some((o) => o.value === value)) {
    options.push({ value, label: describeInput(current), input: current });
  }
  return options;
}

export function describeInput(input: TrackInput): string {
  switch (input.type) {
    case "None":
      return t("recording.input.none");
    case "Audio":
      return input.count > 1 ? `in ${input.first + 1}/${input.first + input.count}` : `in ${input.first + 1}`;
    case "Midi":
      return input.channel === null ? (input.port ?? allPorts()) : t("recording.input.port_channel", { port: input.port ?? allPorts(), n: input.channel + 1 });
    case "Track":
      return t("recording.input.track_output");
  }
}

/** Tracks that can be armed and recorded. */
export function isRecordable(kind: TrackKind): boolean {
  return kind === "Audio" || kind === "Midi";
}

// Pure helpers of the recording controls: input choices per track, count-in choices.
import type { InputList, MonitorMode, TrackInput, TrackKind } from "@/generated";

export interface InputOption {
  /** Stable `<option>` value (see `inputValue`). */
  value: string;
  label: string;
  input: TrackInput;
}

export const COUNT_IN_CHOICES: readonly number[] = [0, 1, 2, 4];
export const MONITOR_MODES: readonly MonitorMode[] = ["Auto", "In", "Off"];

export function countInLabel(bars: number): string {
  if (bars === 0) return "Off";
  return bars === 1 ? "1 bar" : `${bars} bars`;
}

/** Serialize a track input to an `<option>` value. */
export function inputValue(input: TrackInput): string {
  switch (input.type) {
    case "None":
      return "none";
    case "Audio":
      return `audio:${input.first}:${input.count}`;
    case "Midi":
      return `midi:${input.port ?? "*"}:${input.channel ?? "*"}`;
    case "Track":
      return `track:${input.track}`;
  }
}

/**
 * Inputs a track can record from: for audio tracks each mono channel and each adjacent
 * stereo pair of the engine's input list, for MIDI tracks all ports or one port (omni).
 * The track's current input is always included, even if the device is gone.
 */
export function inputOptions(kind: TrackKind, inputs: InputList | null, current: TrackInput): InputOption[] {
  const options: InputOption[] = [{ value: "none", label: "No input", input: { type: "None" } }];
  if (kind === "Audio") {
    const channels = inputs?.audio ?? [];
    for (const ch of channels) {
      options.push({ value: `audio:${ch.index}:1`, label: ch.name, input: { type: "Audio", first: ch.index, count: 1 } });
    }
    for (let i = 0; i + 1 < channels.length; i += 2) {
      const a = channels[i]!;
      const b = channels[i + 1]!;
      if (b.index !== a.index + 1) continue;
      options.push({
        value: `audio:${a.index}:2`,
        label: `${a.name} + ${b.name}`,
        input: { type: "Audio", first: a.index, count: 2 },
      });
    }
  } else if (kind === "Midi") {
    options.push({ value: "midi:*:*", label: "All MIDI inputs", input: { type: "Midi", port: null, channel: null } });
    for (const port of inputs?.midi ?? []) {
      options.push({ value: `midi:${port.id}:*`, label: port.name, input: { type: "Midi", port: port.id, channel: null } });
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
      return "No input";
    case "Audio":
      return input.count > 1 ? `In ${input.first + 1}/${input.first + input.count}` : `In ${input.first + 1}`;
    case "Midi":
      return `${input.port ?? "All MIDI inputs"}${input.channel === null ? "" : ` ch ${input.channel + 1}`}`;
    case "Track":
      return "Track output";
  }
}

/** Tracks that can be armed and recorded. */
export function isRecordable(kind: TrackKind): boolean {
  return kind === "Audio" || kind === "Midi";
}

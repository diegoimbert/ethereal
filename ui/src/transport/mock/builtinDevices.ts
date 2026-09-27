/**
 * Hardcoded descriptors of the built-in devices, for the MockTransport.
 *
 * The real ones come from `ether-devices` (via `Device::ListBuiltin` /
 * `Device::GetDescriptor`); param ids/ranges here are plausible placeholders, NOT a
 * contract: UI code must read descriptors from the engine, never hardcode param ids.
 */

import type { BuiltinDevice, BuiltinDeviceType, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit } from "@/generated";

function param(
  id: number,
  name: string,
  group: string | null,
  unit: ParamUnit,
  min: number,
  max: number,
  def: number,
  scale: ParamScale = { type: "Linear" },
  labels: string[] | null = null,
): ParamInfo {
  return { id, name, group, unit, min, max, default: def, scale, labels, automatable: true, hidden: false };
}

const LOG: ParamScale = { type: "Log" };
const TIME: ParamScale = { type: "Power", exponent: 2 };
const FADER: ParamScale = { type: "Fader" };
const ONOFF = ["Off", "On"];

export const BUILTIN_DESCRIPTORS: Readonly<Record<BuiltinDeviceType, DeviceDescriptor>> = {
  Synth: {
    device_type: { type: "Builtin", device: "Synth" },
    name: "Synth",
    category: "Instrument",
    audio_inputs: 0,
    audio_outputs: 2,
    midi_input: true,
    sidechain_inputs: 0,
    params: [
      param(0, "Waveform", "Oscillator", "None", 0, 3, 1, undefined, ["Sine", "Saw", "Square", "Triangle"]),
      param(1, "Detune", "Oscillator", "Semitones", -12, 12, 0),
      param(2, "Cutoff", "Filter", "Hertz", 20, 20000, 8000, LOG),
      param(3, "Resonance", "Filter", "Percent", 0, 100, 10),
      param(4, "Attack", "Envelope", "Milliseconds", 0, 5000, 5, TIME),
      param(5, "Decay", "Envelope", "Milliseconds", 0, 5000, 200, TIME),
      param(6, "Sustain", "Envelope", "Percent", 0, 100, 70),
      param(7, "Release", "Envelope", "Milliseconds", 0, 10000, 300, TIME),
      param(8, "Volume", "Output", "Decibels", -60, 6, -6, FADER),
    ],
  },
  Sampler: {
    device_type: { type: "Builtin", device: "Sampler" },
    name: "Sampler",
    category: "Instrument",
    audio_inputs: 0,
    audio_outputs: 2,
    midi_input: true,
    sidechain_inputs: 0,
    params: [
      param(0, "Root Key", "Sample", "None", 0, 127, 60),
      param(1, "Transpose", "Sample", "Semitones", -48, 48, 0),
      param(2, "Attack", "Envelope", "Milliseconds", 0, 5000, 1, TIME),
      param(3, "Release", "Envelope", "Milliseconds", 0, 10000, 100, TIME),
      param(4, "Loop", "Sample", "Toggle", 0, 1, 0, undefined, ["Off", "On"]),
      param(5, "Gain", "Output", "Decibels", -60, 12, 0, FADER),
    ],
  },
  Compressor: {
    device_type: { type: "Builtin", device: "Compressor" },
    name: "Compressor",
    category: "AudioEffect",
    audio_inputs: 2,
    audio_outputs: 2,
    midi_input: false,
    sidechain_inputs: 0,
    params: [
      param(0, "Threshold", null, "Decibels", -60, 0, -18),
      param(1, "Ratio", null, "Ratio", 1, 20, 4, LOG),
      param(2, "Attack", null, "Milliseconds", 0.1, 200, 10, LOG),
      param(3, "Release", null, "Milliseconds", 5, 2000, 150, LOG),
      param(4, "Makeup", null, "Decibels", 0, 24, 0),
      param(5, "Mix", null, "Percent", 0, 100, 100),
    ],
  },
  Delay: {
    device_type: { type: "Builtin", device: "Delay" },
    name: "Delay",
    category: "AudioEffect",
    audio_inputs: 2,
    audio_outputs: 2,
    midi_input: false,
    sidechain_inputs: 0,
    params: [
      param(0, "Time", null, "Milliseconds", 1, 2000, 375, LOG),
      param(1, "Sync", null, "Toggle", 0, 1, 1, undefined, ["Off", "On"]),
      param(2, "Feedback", null, "Percent", 0, 95, 35),
      param(3, "Filter", null, "Hertz", 200, 20000, 6000, LOG),
      param(4, "Mix", null, "Percent", 0, 100, 30),
    ],
  },
  // Roadmap v2 effects: mirror `ether-devices/src/{eq,reverb,limiter,utility}.rs` exactly.
  Eq: effect("Eq", "EQ", eqParams()),
  Reverb: effect("Reverb", "Reverb", [
    param(0, "Pre-Delay", "Reverb", "Milliseconds", 0, 250, 20, { type: "Power", exponent: 2 }),
    param(1, "Size", "Reverb", "Percent", 0, 100, 50),
    param(2, "Decay", "Reverb", "Seconds", 0.2, 20, 2, LOG),
    param(3, "Damping", "Reverb", "Percent", 0, 100, 50),
    param(4, "Width", "Output", "Percent", 0, 100, 100),
    param(5, "Mix", "Output", "Percent", 0, 100, 30),
  ]),
  Limiter: effect("Limiter", "Limiter", [
    param(0, "Gain", "Limiter", "Decibels", -12, 24, 0),
    param(1, "Ceiling", "Limiter", "Decibels", -24, 0, -0.3),
    param(2, "Release", "Limiter", "Milliseconds", 1, 1000, 100, LOG),
  ]),
  Utility: effect("Utility", "Utility", [
    param(0, "Gain", "Utility", "Decibels", -36, 36, 0),
    param(1, "Pan", "Utility", "Pan", -1, 1, 0),
    param(2, "Width", "Stereo", "Percent", 0, 200, 100),
    param(3, "Invert L", "Phase", "Toggle", 0, 1, 0, undefined, ONOFF),
    param(4, "Invert R", "Phase", "Toggle", 0, 1, 0, undefined, ONOFF),
    param(5, "Mono", "Stereo", "Toggle", 0, 1, 0, undefined, ONOFF),
  ]),
  DrumRack: placeholder("DrumRack", "Drum Rack", true),
};

/** Stereo audio effect descriptor. */
function effect(device: BuiltinDeviceType, name: string, params: ParamInfo[]): DeviceDescriptor {
  return {
    device_type: { type: "Builtin", device },
    name,
    category: "AudioEffect",
    audio_inputs: 2,
    audio_outputs: 2,
    midi_input: false,
    sidechain_inputs: 0,
    params,
  };
}

/** EQ: 8 bands x (On, Type, Freq, Gain, Q) at ids 5b..5b+4, then Output (40). */
function eqParams(): ParamInfo[] {
  const types = ["Low Cut", "Low Shelf", "Bell", "Notch", "High Shelf", "High Cut"];
  const bands: [boolean, number, number][] = [
    [false, 0, 30],
    [true, 1, 100],
    [true, 2, 250],
    [true, 2, 1000],
    [true, 2, 2500],
    [true, 2, 6000],
    [true, 4, 10000],
    [false, 5, 18000],
  ];
  const params = bands.flatMap(([on, type, freq], b) => {
    const group = `Band ${b + 1}`;
    const id = 5 * b;
    return [
      param(id, "On", group, "Toggle", 0, 1, on ? 1 : 0, undefined, ONOFF),
      param(id + 1, "Type", group, "None", 0, types.length - 1, type, undefined, types),
      param(id + 2, "Freq", group, "Hertz", 20, 20000, freq, LOG),
      param(id + 3, "Gain", group, "Decibels", -24, 24, 0),
      param(id + 4, "Q", group, "None", 0.1, 18, Math.SQRT1_2, LOG),
    ];
  });
  return [...params, param(40, "Output", "Output", "Decibels", -24, 24, 0)];
}

/** Roadmap v2 built-ins not implemented yet: no params, like the Rust placeholders (`ether-devices/src/placeholder.rs`). */
function placeholder(device: BuiltinDeviceType, name: string, instrument: boolean): DeviceDescriptor {
  return {
    device_type: { type: "Builtin", device },
    name,
    category: instrument ? "Instrument" : "AudioEffect",
    audio_inputs: instrument ? 0 : 2,
    audio_outputs: 2,
    midi_input: instrument,
    sidechain_inputs: 0,
    params: [],
  };
}

/** A fresh `BuiltinDevice` of `type` with default data (mirrors Rust `BuiltinDevice::new`). */
export function newBuiltinDevice(type: BuiltinDeviceType): BuiltinDevice {
  return type === "Sampler" ? { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } } : { type };
}

export function builtinDescriptor(device: BuiltinDevice | BuiltinDeviceType): DeviceDescriptor {
  return BUILTIN_DESCRIPTORS[typeof device === "string" ? device : device.type];
}

/** Clamp a plain value to a param's range (and to integer steps for enum params). */
export function clampParam(info: ParamInfo, value: number): number {
  const v = Math.min(info.max, Math.max(info.min, value));
  if (info.labels && info.labels.length > 1) {
    const step = (info.max - info.min) / (info.labels.length - 1);
    return info.min + Math.round((v - info.min) / step) * step;
  }
  return v;
}

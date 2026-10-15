/**
 * Built-in device descriptors for the MockTransport, **generated from Rust**
 * (`ether_devices::descriptor`, layouts included): the v0.1 built-ins in
 * `devices/v01Layouts.json` (`crates/ether-devices/tests/device_ui_layouts.rs`), the EQ and
 * the v0.2 / v0.3 devices and the FM Synth in `devices/*.json`. Regenerate with `UPDATE_MOCK_DESCRIPTORS=1 cargo
 * test -p ether-devices --test device_ui_layouts` (and `--test v02_descriptors`); never
 * edit them by hand. UI code must read descriptors from the engine, never hardcode param ids.
 */

import type { BuiltinDevice, BuiltinDeviceType, DeviceDescriptor, MissingDevice, ParamInfo } from "@/generated";
import { EQ_DESCRIPTOR, FM_SYNTH_DESCRIPTOR, V02_DESCRIPTORS, V03_DESCRIPTORS } from "./devices";
import v01 from "./devices/v01Layouts.json";

type V01DeviceType = "Synth" | "Sampler" | "Compressor" | "Delay" | "Reverb" | "Limiter" | "Utility" | "DrumRack";

// JSON imports widen enum strings to `string`; the file is generated from the typed Rust
// values, so the cast is sound.
const V01 = v01 as unknown as Readonly<Record<V01DeviceType, DeviceDescriptor>>;

// In `BuiltinDeviceType::ALL` order (the add-device menu lists them in this order).
export const BUILTIN_DESCRIPTORS: Readonly<Record<BuiltinDeviceType, DeviceDescriptor>> = {
  Synth: V01.Synth,
  Sampler: V01.Sampler,
  Compressor: V01.Compressor,
  Delay: V01.Delay,
  // Generated from `ether-devices/src/eq.rs` (carries the v0.2 EqCurve layout).
  Eq: EQ_DESCRIPTOR,
  Reverb: V01.Reverb,
  Limiter: V01.Limiter,
  Utility: V01.Utility,
  DrumRack: V01.DrumRack,
  // v0.2 (contracts-3): generated from Rust, one JSON file per device node (`./devices`).
  ...V02_DESCRIPTORS,
  // v0.3 (contracts-4): fx-space and external-instrument, generated from Rust.
  ...V03_DESCRIPTORS,
  // fm-synth, generated from Rust.
  FmSynth: FM_SYNTH_DESCRIPTOR,
};

/** A fresh `BuiltinDevice` of `type` with default data (mirrors Rust `BuiltinDevice::new`). */
export function newBuiltinDevice(type: BuiltinDeviceType): BuiltinDevice {
  if (type === "Sampler") return { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } };
  if (type === "MultiSampler") return { type: "MultiSampler", zones: [] };
  // A new convolution reverb starts on the factory hall (`DEFAULT_CONVOLUTION_IR`, #520).
  if (type === "ConvolutionReverb") return { type: "ConvolutionReverb", ir: { type: "Factory", id: "hall" } };
  if (type === "ExternalInstrument" || type === "ExternalAudioEffect") {
    return { type, routing: { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null } };
  }
  return { type } as BuiltinDevice;
}

export function builtinDescriptor(device: BuiltinDevice | BuiltinDeviceType): DeviceDescriptor {
  return BUILTIN_DESCRIPTORS[typeof device === "string" ? device : device.type];
}

/**
 * A missing device's descriptor (the engine's `missing_descriptor`): its source's
 * parameters, linear over their ranges; no node.
 */
export function missingDescriptor(m: MissingDevice): DeviceDescriptor {
  const io = { Instrument: [0, 2, true], AudioEffect: [2, 2, false], NoteEffect: [0, 0, true] } as const;
  const [audio_inputs, audio_outputs, midi_input] = io[m.role];
  return {
    device_type: { type: "Missing", source_id: m.source_id },
    name: m.description,
    category: m.role,
    params: m.params.map((p) => ({
      id: p.id,
      name: p.name,
      group: null,
      unit: "None",
      min: p.min,
      max: p.max,
      default: p.min,
      scale: { type: "Linear" },
      labels: null,
      automatable: true,
      hidden: false,
    })),
    audio_inputs,
    audio_outputs,
    midi_input,
    sidechain_inputs: 0,
  };
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

/**
 * The Add-device picker's `builtinDevice(type)` (`features/devices/descriptors.ts`, BCR 2 of
 * `external-instrument`) gives every built-in its default data, like Rust
 * `BuiltinDevice::new` (mirrored by the mock's `newBuiltinDevice`): the external devices'
 * routing and the convolution reverb's IR included, so the controller accepts the insert.
 */
import { describe, expect, it } from "vitest";
import type { BuiltinDeviceType } from "@/generated";
import { BUILTIN_DESCRIPTORS } from "@/transport";
import { newBuiltinDevice } from "@/transport/mock/builtinDevices";
import { builtinDevice } from "@/features/devices/descriptors";

const TYPES = Object.keys(BUILTIN_DESCRIPTORS) as BuiltinDeviceType[];

describe("builtinDevice", () => {
  it.each(TYPES)("%s matches BuiltinDevice::new", (type) => {
    expect(builtinDevice(type)).toEqual(newBuiltinDevice(type));
  });

  it("has data for the v0.3 built-ins", () => {
    expect(builtinDevice("ExternalInstrument")).toEqual({
      type: "ExternalInstrument",
      routing: { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null },
    });
    expect(builtinDevice("ExternalAudioEffect")).toMatchObject({ type: "ExternalAudioEffect", routing: { midi_channel: 1 } });
    expect(builtinDevice("ConvolutionReverb")).toEqual({ type: "ConvolutionReverb", ir: null });
  });
});

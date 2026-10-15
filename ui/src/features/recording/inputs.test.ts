import { describe, expect, it } from "vitest";
import type { InputList } from "@/generated";
import { countInLabel, describeInput, inputOptions, inputValue, midiChannelOptions, midiChannelValue, withMidiChannel } from "./inputs";

const inputs: InputList = {
  audio: [
    { index: 0, name: "In 1" },
    { index: 1, name: "In 2" },
    { index: 2, name: "In 3" },
  ],
  midi: [{ id: "Keys", name: "Keys" }],
};

describe("recording inputs", () => {
  it("offers mono channels and adjacent stereo pairs to audio tracks", () => {
    const options = inputOptions("Audio", inputs, { type: "None" });
    expect(options.map((o) => o.value)).toEqual(["none", "audio:0:1", "audio:1:1", "audio:2:1", "audio:0:2"]);
    expect(options.find((o) => o.value === "audio:0:2")!.label).toBe("In 1/2");
  });

  it("offers every input and odd/even pair of a multi-channel interface (1, 2, 1/2, 3/4)", () => {
    const four: InputList = { audio: [0, 1, 2, 3].map((index) => ({ index, name: `${index + 1}` })), midi: [] };
    const options = inputOptions("Audio", four, { type: "None" });
    expect(options.map((o) => o.label)).toEqual(["no input", "1", "2", "3", "4", "1/2", "3/4"]);
    expect(options.at(-1)!.input).toEqual({ type: "Audio", first: 2, count: 2 });
    // Unnumbered channels keep both names.
    const loopback: InputList = {
      audio: [
        { index: 0, name: "Loopback L" },
        { index: 1, name: "Loopback R" },
      ],
      midi: [],
    };
    expect(inputOptions("Audio", loopback, { type: "None" }).at(-1)!.label).toBe("Loopback L + Loopback R");
  });

  it("offers all ports or one port to MIDI tracks, keeping the channel filter", () => {
    const options = inputOptions("Midi", inputs, { type: "Midi", port: null, channel: 1 });
    expect(options.map((o) => o.label)).toEqual(["no input", "all ports", "Keys"]);
    expect(options.at(-1)!.input).toEqual({ type: "Midi", port: "Keys", channel: 1 });
    // The channel is not part of the port select's value.
    expect(inputValue({ type: "Midi", port: "Keys", channel: 1 })).toBe("midi:Keys");
  });

  it("filters a MIDI input by channel (all or 1-16)", () => {
    expect(midiChannelOptions().map((o) => o.label)).toEqual(["all channels", ...Array.from({ length: 16 }, (_, i) => `ch ${i + 1}`)]);
    const keys = { type: "Midi" as const, port: "Keys", channel: null };
    expect(midiChannelValue(keys)).toBe("*");
    expect(withMidiChannel(keys, "1")).toEqual({ type: "Midi", port: "Keys", channel: 1 });
    expect(midiChannelValue(withMidiChannel(keys, "15"))).toBe("15");
    expect(withMidiChannel({ ...keys, channel: 4 }, "*")).toEqual(keys);
  });

  it("keeps the current input when the device is gone", () => {
    const current = { type: "Audio" as const, first: 6, count: 2 };
    const options = inputOptions("Audio", null, current);
    expect(options.at(-1)).toEqual({ value: inputValue(current), label: "in 7/8", input: current });
    expect(describeInput({ type: "Midi", port: "Keys", channel: 9 })).toBe("Keys ch 10");
  });

  it("labels count-in choices", () => {
    expect([0, 1, 4].map(countInLabel)).toEqual(["off", "1 bar", "4 bars"]);
  });
});

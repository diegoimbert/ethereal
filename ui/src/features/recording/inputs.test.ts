import { describe, expect, it } from "vitest";
import type { InputList } from "@/generated";
import { countInLabel, describeInput, inputOptions, inputValue } from "./inputs";

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
    expect(options.find((o) => o.value === "audio:0:2")!.label).toBe("In 1 + In 2");
  });

  it("offers all ports or one port to MIDI tracks", () => {
    const options = inputOptions("Midi", inputs, { type: "Midi", port: null, channel: null });
    expect(options.map((o) => o.label)).toEqual(["No input", "All MIDI inputs", "Keys"]);
  });

  it("keeps the current input when the device is gone", () => {
    const current = { type: "Audio" as const, first: 6, count: 2 };
    const options = inputOptions("Audio", null, current);
    expect(options.at(-1)).toEqual({ value: inputValue(current), label: "In 7/8", input: current });
    expect(describeInput({ type: "Midi", port: "Keys", channel: 9 })).toBe("Keys ch 10");
  });

  it("labels count-in choices", () => {
    expect([0, 1, 4].map(countInLabel)).toEqual(["Off", "1 bar", "4 bars"]);
  });
});

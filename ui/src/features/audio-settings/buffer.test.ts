import { describe, expect, it } from "vitest";
import { bufferChoice } from "./buffer";

describe("bufferChoice", () => {
  it("shows the running size, offering the sizes in the device range", () => {
    expect(bufferChoice(512, 512, [15, 4096], true)).toEqual({ value: 512, options: [32, 64, 128, 256, 512, 1024, 2048], note: null });
    expect(bufferChoice(64, 256, [256, 1024], true)).toEqual({
      value: 256,
      options: [256, 512, 1024],
      note: "the device runs 256 samples (64 asked; it supports 256–1024).",
    });
  });

  it("falls back to the requested size before the first callback", () => {
    expect(bufferChoice(128, 0, null, true)).toMatchObject({ value: 128, note: null });
    expect(bufferChoice(null, undefined, null, true)).toMatchObject({ value: null, note: null });
  });

  it("keeps an odd running size selectable", () => {
    expect(bufferChoice(null, 480, null, true).options).toContain(480);
  });

  it("says when the system sets the buffer (WASAPI shared mode)", () => {
    const c = bufferChoice(128, 480, null, false);
    expect(c.value).toBe(480);
    expect(c.note).toMatch(/set by windows \(wasapi shared mode\): 480 samples per callback/);
  });
});

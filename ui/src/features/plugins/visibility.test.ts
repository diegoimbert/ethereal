// eth-allow-case-file: third-party plugin names (test fixtures).
import { describe, expect, it } from "vitest";
import { plugin } from "./testing";
import { DEFAULT_VISIBILITY, visiblePlugins } from "./visibility";

const names = (ps: ReturnType<typeof plugin>[]) => ps.map((p) => `${p.name}/${p.format}`);

describe("one format per plugin", () => {
  const clap = plugin("com.x.serum", "Serum", "Instrument", "Xfer Records", "Clap");
  const vst3 = plugin("A1", "Serum", "Instrument", "Xfer Records", "Vst3");
  const vst2 = plugin("58667358", "Serum (x64)", "Instrument", "Xfer Records", "Vst2");
  const au = plugin("aumu:XfsX:XfrX", "Serum", "Instrument", "Xfer Records", "Au");
  const fx = plugin("com.x.serumfx", "SerumFX", "AudioEffect", "Xfer Records", "Vst3");

  it("keeps the preferred format of each plugin (names and vendors compared loosely)", () => {
    expect(names(visiblePlugins([vst2, au, vst3, clap, fx], DEFAULT_VISIBILITY))).toEqual(["Serum/Clap", "SerumFX/Vst3"]);
    expect(names(visiblePlugins([vst2, au, vst3, fx], DEFAULT_VISIBILITY))).toEqual(["Serum/Vst3", "SerumFX/Vst3"]);
    expect(names(visiblePlugins([vst2, au, vst3, clap], { ...DEFAULT_VISIBILITY, preferred: "Au" }))).toEqual(["Serum/Au"]);
  });

  it("tells plugins of another vendor or kind apart", () => {
    const other = plugin("o", "Serum", "Instrument", "Someone Else", "Vst3");
    const asFx = plugin("f", "Serum", "AudioEffect", "Xfer Records", "Vst3");
    expect(names(visiblePlugins([clap, other, asFx], DEFAULT_VISIBILITY))).toEqual(["Serum/Clap", "Serum/Vst3", "Serum/Vst3"]);
  });

  it("hide rules apply before choosing: hiding the CLAP version lists the next one", () => {
    const rules = [{ name: "serum", vendor: "xfer records", format: "Clap" as const }];
    expect(names(visiblePlugins([vst2, vst3, clap], { ...DEFAULT_VISIBILITY, rules }))).toEqual(["Serum/Vst3"]);
    expect(names(visiblePlugins([vst2, vst3, clap], { ...DEFAULT_VISIBILITY, rules: [{ ...rules[0]!, format: null }] }))).toEqual([]);
  });
});

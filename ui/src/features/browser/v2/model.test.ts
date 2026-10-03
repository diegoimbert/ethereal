import { describe, expect, it } from "vitest";
import type { BrowserRoot } from "@/generated";
import { ALL_ROOTS, buildQuery, formatDuration, parseTags, placeOfFolder, presetDeviceKey, shortKey } from "./model";

const roots: BrowserRoot[] = [
  { id: "library", name: "Library", kind: "Library", path: null, items: 3 },
  { id: "library/Vocals", name: "Vocals", kind: "Pack", path: null, items: 2 },
];
const filters = { text: "", kind: "all" as const, tags: [], favouritesOnly: false, sort: "Recent" as const };

describe("browser v2 model", () => {
  it("scopes queries to the place (pack folders are library paths)", () => {
    expect(buildQuery({ root: ALL_ROOTS, folder: "" }, filters, roots)).toMatchObject({ roots: [], folder: null, sort: "Recent", limit: 100 });
    expect(buildQuery({ root: "library/Vocals", folder: "Chops" }, { ...filters, text: " hey " }, roots)).toMatchObject({
      roots: ["library/Vocals"],
      folder: "Vocals/Chops",
      text: "hey",
      sort: "Relevance",
    });
    expect(buildQuery({ root: "library", folder: "" }, { ...filters, kind: "Midi" }, roots)).toMatchObject({ kinds: ["Midi"], folder: null });
  });

  it("keeps folders inside the current pack", () => {
    expect(placeOfFolder("library", "Vocals/Chops", { root: "library/Vocals", folder: "" }, roots)).toEqual({ root: "library/Vocals", folder: "Chops" });
    expect(placeOfFolder("library", "Drums", { root: "library/Vocals", folder: "" }, roots)).toEqual({ root: "library", folder: "Drums" });
  });

  it("formats and parses", () => {
    expect(formatDuration(0.6)).toBe("0.6 s");
    expect(formatDuration(12)).toBe("12 s");
    expect(formatDuration(65)).toBe("1:05");
    expect(shortKey("A minor")).toBe("Am");
    expect(shortKey("C# major")).toBe("C#");
    expect(parseTags(" Punchy, dark,,punchy ")).toEqual(["dark", "punchy"]);
    expect(presetDeviceKey({ kind: "Preset", root: "factory", path: "poly-synth/warm-pad" })).toBe("poly-synth");
    expect(presetDeviceKey({ kind: "Preset", root: "user", path: "Presets/compressor/Glue.etherpreset" })).toBe("compressor");
  });
});

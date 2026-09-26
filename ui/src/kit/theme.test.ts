// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { themeEntries, trackColor, TRACK_COLORS } from "./theme";

// Read from disk: vitest stubs CSS imports (even `?raw`) when `test.css` is off.
const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");

describe("theme", () => {
  it("theme.css declares every token from theme.ts with the same value", () => {
    const declared = new Map<string, string>();
    for (const m of css.matchAll(/(--eth-[a-z0-9-]+)\s*:\s*([^;]+);/g)) declared.set(m[1]!, m[2]!.trim());
    for (const [name, value] of themeEntries()) {
      expect(declared.get(name), name).toBe(value);
    }
  });

  it("trackColor wraps around the palette", () => {
    expect(trackColor(0)).toBe(TRACK_COLORS[0]);
    expect(trackColor(TRACK_COLORS.length + 1)).toBe(TRACK_COLORS[1]);
    expect(trackColor(-1)).toBe(TRACK_COLORS[TRACK_COLORS.length - 1]);
  });
});

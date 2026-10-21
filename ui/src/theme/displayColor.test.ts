import { describe, expect, it } from "vitest";
import { displayColor } from "./displayColor";
import { TRACK_COLORS } from "./tokens";

const hex = (s: string) => parseInt(s.slice(1), 16);
const channels = (c: number) => [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff] as const;

describe("displayColor", () => {
  it("shows the palette's colors as they are", () => {
    for (const c of TRACK_COLORS) expect(displayColor(hex(c))).toBe(hex(c));
  });

  it("lifts a very dark imported color into the palette's range, keeping its hue", () => {
    const [r, g, b] = channels(displayColor(0x0a1a5c)); // a very dark blue
    expect(Math.min(r, g, b)).toBeGreaterThan(100);
    expect(b).toBeGreaterThan(r);
    expect(b).toBeGreaterThan(g);
  });

  it("softens a saturated imported color", () => {
    const [r, g, b] = channels(displayColor(0xff0000));
    expect(r).toBeGreaterThan(g);
    expect(g).toBeGreaterThan(80); // no longer pure red
    expect(b).toBeGreaterThan(80);
  });
});

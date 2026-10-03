import { describe, expect, it } from "vitest";
import { contrastRatio, DARK_INK, LIGHT_INK, luminance } from "@/features/arrangement/helpers";
import { avatarStyle } from "./avatar";

const prop = (color: number, name: string) => (avatarStyle(color) as Record<string, string>)[name];

describe("avatarStyle", () => {
  it("fills with the peer colour and picks the readable ink for it", () => {
    expect(prop(0x5cffe8, "--eth-collab-peer")).toBe("#5cffe8");
    // Bright peer colours get dark ink (they used to get the theme background: white in light).
    expect(prop(0x5cffe8, "--eth-collab-peer-ink")).toBe(DARK_INK);
    expect(prop(0xff94a6, "--eth-collab-peer-ink")).toBe(DARK_INK);
    // Dark peer colours get light ink.
    expect(prop(0x1a237e, "--eth-collab-peer-ink")).toBe(LIGHT_INK);
  });

  it("the chosen ink reads (>= 4:1) on a spread of peer colours", () => {
    for (let i = 0; i < 256; i++) {
      // A deterministic spread over the RGB cube.
      const color = (((i * 0x3b) & 0xff) << 16) | (((i * 0x97) & 0xff) << 8) | ((i * 0xd1) & 0xff);
      const ink = prop(color, "--eth-collab-peer-ink")!;
      const ratio = contrastRatio(luminance(color), luminance(parseInt(ink.slice(1), 16)));
      expect(ratio, `#${color.toString(16)} with ${ink}`).toBeGreaterThanOrEqual(4);
    }
  });
});
